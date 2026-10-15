use crate::parser::{Doc, Node, Val};

#[derive(Debug, Clone, serde::Serialize)]
pub struct Cell {
    pub col: String,
    pub text: String,
    pub nested: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Row {
    pub index: usize,
    pub node_id: usize,
    pub cells: Vec<Cell>,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Table {
    pub id: u64,
    pub node_id: usize,
    /// 该数组节点的 JSONPath（`$` / `.key` / `[idx]`），供编辑重解析后按路径重挂表格
    pub path: String,
    pub columns: Vec<String>,
    pub row_nodes: Vec<usize>,
}

pub fn split_tokens(input: &str) -> Vec<String> {
    input.split([',', ' ', '\t', '\n', '\r'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

/// 节点 → JSONPath（`$` / `.key` / `[idx]`）：沿 parent + key 链自底向上收集段再反转。
/// 含 `.` 等特殊字符的键名会生成有歧义的路径（resolve_path 不支持引号转义）——已知局限。
pub fn path_of(doc: &Doc, id: usize) -> String {
    let mut segs: Vec<String> = Vec::new();
    let mut cur = id;
    loop {
        let n = doc.node(cur);
        match n.parent {
            None => break, // 根节点无 key，路径以 `$` 开头
            Some(p) => {
                let k = n.key.clone().unwrap_or_default();
                if doc.node(p).val == Val::Arr {
                    segs.push(format!("[{k}]"));
                } else {
                    segs.push(format!(".{k}"));
                }
                cur = p;
            }
        }
    }
    segs.reverse();
    format!("${}", segs.join(""))
}

/// JSONPath → 节点 id：自根向下逐段解析（`.name` / `[idx]`）。
/// 任一段无匹配（键不存在 / 数组越界 / 节点类型不符）→ None，供 table_reopen 判定「文档中已无该数组」。
pub fn resolve_path(doc: &Doc, path: &str) -> Option<usize> {
    let p = path.trim();
    let b = p.as_bytes();
    if b.first() != Some(&b'$') { return None; }
    let mut cur = doc.root;
    let mut i = 1;
    while i < b.len() {
        match b[i] {
            b'.' => {
                i += 1;
                let start = i;
                // 段终止符 '.' / '[' 均为 ASCII，不会落在多字节 UTF-8 序列内部，字节扫描安全
                while i < b.len() && b[i] != b'.' && b[i] != b'[' { i += 1; }
                let name = &p[start..i];
                let n = doc.node(cur);
                if n.val != Val::Obj { return None; }
                cur = *n.children.iter().find(|&&c| doc.node(c).key.as_deref() == Some(name))?;
            }
            b'[' => {
                i += 1;
                let start = i;
                while i < b.len() && b[i].is_ascii_digit() { i += 1; }
                if start == i || b.get(i) != Some(&b']') { return None; }
                i += 1;
                let idx: usize = p[start..i - 1].parse().ok()?;
                let n = doc.node(cur);
                if n.val != Val::Arr { return None; }
                cur = *n.children.get(idx)?;
            }
            _ => return None,
        }
    }
    Some(cur)
}

impl Table {
    /// 任意节点都可建表（转为表格 = 数组 N 行 / 其他节点单行）：
    /// - Arr：行 = 子元素（现状不变）
    /// - Obj：单行，columns = 子键并集
    /// - 标量（str/num/bool/null）：单行单列 value，cell = 自身 literal
    pub fn create(doc: &Doc, node_id: usize, next_id: &mut u64) -> Table {
        let node = doc.node(node_id);
        let path = path_of(doc, node_id);
        let row_nodes: Vec<usize> = match node.val {
            Val::Arr => node.children.clone(),
            _ => vec![node_id],
        };
        // 列 = 所有行子对象键的并集（保持首次出现顺序）；标量节点/标量行数组（如 ["a","b"]）
        // 没有子键：单标量节点列名用其 key（如 "city"），无 key（根标量）或标量行数组兜底 ["value"]
        // （cell_of 对无子节点的行按自身 literal 取值）
        let mut columns: Vec<String> = Vec::new();
        if !matches!(node.val, Val::Obj | Val::Arr) {
            columns.push(node.key.clone().unwrap_or_else(|| "value".to_string()));
        } else {
            row_nodes.iter().for_each(|&r| {
                doc.node(r).children.iter().for_each(|&c| {
                    let n = doc.node(c);
                    let col = match &n.key {
                        Some(k) => k.clone(),
                        None => "value".to_string(),
                    };
                    if !columns.contains(&col) { columns.push(col); }
                });
            });
            if columns.is_empty() { columns.push("value".to_string()); }
        }
        let id = *next_id;
        *next_id += 1;
        Table { id, node_id, path, columns, row_nodes }
    }

    pub fn rows(&self, doc: &Doc, src: &str, offset: usize, limit: usize) -> Vec<Row> {
        self.row_nodes.iter()
            .skip(offset)
            .take(limit)
            .enumerate()
            .map(|(i, &r)| Row {
                index: offset + i,
                node_id: r,
                cells: self.columns.iter().map(|c| self.cell_of(doc, src, r, c)).collect(),
            })
            .collect()
    }

    /// 按命中行索引（全表 0-based）直接构建行，供导出等场景复用 cell_of 提取逻辑
    pub fn rows_for_indices(&self, doc: &Doc, src: &str, indices: &[usize]) -> Vec<Row> {
        indices.iter()
            .filter_map(|&i| self.row_nodes.get(i).map(|&r| (i, r)))
            .map(|(i, r)| Row {
                index: i,
                node_id: r,
                cells: self.columns.iter().map(|c| self.cell_of(doc, src, r, c)).collect(),
            })
            .collect()
    }

    fn cell_of(&self, doc: &Doc, src: &str, row: usize, col: &str) -> Cell {
        let node = doc.node(row);
        // 标量行（无子节点）的取值列：列名 = 行节点自身 key（单标量节点转表语义，如 "city"），
        // 或兜底 "value"（根标量 / 标量行数组元素——后者 key 是数组下标，不参与匹配）；
        // 列名都不匹配则走下方常规取值（空 cell）
        if (col == "value" || node.key.as_deref() == Some(col)) && node.children.is_empty() {
            return Cell { col: col.into(), text: node.literal.clone().unwrap_or_default(), nested: false };
        }
        let child = node.children.iter()
            .map(|&c| doc.node(c))
            .find(|n| n.key.as_deref() == Some(col));
        match child {
            None => Cell { col: col.into(), text: String::new(), nested: false },
            // 嵌套：直接展示原始 JSON 片段（与原文一致的切片），不再用摘要
            Some(n) if matches!(n.val, Val::Obj | Val::Arr) => Cell {
                col: col.into(),
                text: src[n.start..n.end].trim().to_string(),
                nested: true,
            },
            Some(n) => Cell {
                col: col.into(),
                text: n.literal.clone().unwrap_or_default(),
                nested: false,
            },
        }
    }

    /// 列范围：None = 全部列；Some = 仅集合内的列（列显示筛选 / 单列过滤共用；
    /// 空集合 = 全部列被隐藏 → 无命中）
    pub fn filter(&self, doc: &Doc, src: &str, values: &[String], cols: Option<&[String]>) -> Vec<usize> {
        if values.is_empty() { return (0..self.row_nodes.len()).collect(); }
        self.row_nodes.iter().enumerate()
            .filter(|(_i, r)| {
                self.columns.iter()
                    .filter(|c| cols.map_or(true, |cs| cs.iter().any(|x| x == *c)))
                    .any(|c| self.cell_matches(doc, src, **r, c, values))
            })
            .map(|(i, _)| i)
            .collect()
    }

    fn cell_matches(&self, doc: &Doc, src: &str, row: usize, col: &str, values: &[String]) -> bool {
        let node = doc.node(row);
        let Some(n) = node.children.iter().map(|&c| doc.node(c))
            .find(|n| n.key.as_deref() == Some(col)) else { return false; };
        // 模糊匹配：大小写不敏感的 contains（token "ali" 命中 "Alice"，"付费" 命中 "付费用户"）
        let contains = |l: &str| values.iter().any(|v| l.to_lowercase().contains(&v.to_lowercase()));
        if matches!(n.val, Val::Arr) {
            // 嵌套数组：任一元素值命中，或原始 JSON 片段命中（覆盖对象数组内的 key，与单元格展示的原文一致）
            n.children.iter().any(|&c| doc.node(c).literal.as_deref().is_some_and(contains))
                || contains(&src[n.start..n.end])
        } else if matches!(n.val, Val::Obj) {
            // 嵌套对象：原始 JSON 片段命中（key 与嵌套 value 都参与匹配，与单元格展示的原文一致）
            contains(&src[n.start..n.end])
        } else {
            n.literal.as_deref().is_some_and(contains)
        }
    }

    pub fn transpose(&self, doc: &Doc, row: usize) -> Vec<(String, String)> {
        let mut out = Vec::new();
        fn walk(doc: &Doc, id: usize, path: &str, out: &mut Vec<(String, String)>) {
            let n: &Node = doc.node(id);
            match n.val {
                Val::Obj => n.children.iter().for_each(|&c| {
                    let k = doc.node(c).key.clone().unwrap_or_default();
                    let p = if path.is_empty() { k } else { format!("{path}.{k}") };
                    walk(doc, c, &p, out);
                }),
                Val::Arr => {
                    out.push((path.to_string(), format!("[{}] 项", n.len)));
                    n.children.iter().enumerate().for_each(|(j, &c)| {
                        walk(doc, c, &format!("{path}[{j}]"), out);
                    });
                }
                _ => out.push((path.to_string(), n.literal.clone().unwrap_or_default())),
            }
        }
        let Some(&row_id) = self.row_nodes.get(row) else { return Vec::new(); };
        doc.node(row_id).children.iter().for_each(|&c| {
            let k = doc.node(c).key.clone().unwrap_or_default();
            walk(doc, c, &k, &mut out);
        });
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse;

    fn setup() -> (&'static str, Doc, usize) {
        let src = r#"{"users":[
            {"id":1,"name":"Alice","age":30,"tags":["admin"],"profile":{"level":9}},
            {"id":2,"name":"Bob","age":25,"tags":[],"profile":{"level":3}},
            {"id":3,"name":"Carol","age":35,"tags":["dev"],"profile":{"level":6}}
        ]}"#;
        let doc = parse(src).unwrap();
        let users = doc.node(doc.root).children[0];
        (src, doc, users)
    }

    #[test]
    fn columns_union() {
        let (_src, doc, users) = setup();
        let mut next = 0u64;
        let t = Table::create(&doc, users, &mut next);
        assert_eq!(t.columns, vec!["id", "name", "age", "tags", "profile"]);
        assert_eq!(t.row_nodes.len(), 3);
    }

    #[test]
    fn obj_node_single_row_table() {
        let (src, doc, _users) = setup();
        // 第一个 user 的 profile 对象：$.users[0].profile
        let users = doc.node(doc.root).children[0];
        let row0 = doc.node(users).children[0];
        let profile_id = *doc.node(row0).children.last().unwrap();
        let mut next = 0u64;
        let t = Table::create(&doc, profile_id, &mut next);
        // 对象 = 单行，columns = 其子键
        assert_eq!(t.row_nodes, vec![profile_id]);
        assert_eq!(t.columns, vec!["level"]);
        let rows = t.rows(&doc, src, 0, 10);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].cells.iter().find(|c| c.col == "level").unwrap().text, "9");
        // path 可回解析到同一节点
        assert_eq!(resolve_path(&doc, &t.path), Some(profile_id));
    }

    #[test]
    fn scalar_array_rows_use_value_column() {
        let src = r#"{"tags":["admin","dev"]}"#;
        let doc = parse(src).unwrap();
        let tags = doc.node(doc.root).children[0];
        let mut next = 0u64;
        let t = Table::create(&doc, tags, &mut next);
        // 标量元素数组：行 = 元素，columns 兜底为 ["value"]，cell = 元素字面值
        assert_eq!(t.row_nodes.len(), 2);
        assert_eq!(t.columns, vec!["value"]);
        let rows = t.rows(&doc, src, 0, 10);
        assert_eq!(rows[0].cells[0].text, "admin");
        assert_eq!(rows[1].cells[0].text, "dev");
    }

    #[test]
    fn scalar_node_single_cell_table() {
        let src = r#"{"msg":"查询成功"}"#;
        let doc = parse(src).unwrap();
        let msg = doc.node(doc.root).children[0];
        let mut next = 0u64;
        let t = Table::create(&doc, msg, &mut next);
        // 标量 = 单行单列，列名 = 节点 key（"msg"），cell = 自身 literal
        assert_eq!(t.row_nodes, vec![msg]);
        assert_eq!(t.columns, vec!["msg"]);
        let rows = t.rows(&doc, src, 0, 10);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].cells[0].col, "msg");
        assert_eq!(rows[0].cells[0].text, "查询成功");
        assert!(!rows[0].cells[0].nested);
        assert_eq!(resolve_path(&doc, &t.path), Some(msg));
    }

    #[test]
    fn root_scalar_node_falls_back_to_value_column() {
        // 无 key 的标量节点（根标量）：列名兜底 "value"
        let src = "42";
        let doc = parse(src).unwrap();
        let mut next = 0u64;
        let t = Table::create(&doc, doc.root, &mut next);
        assert_eq!(t.row_nodes, vec![doc.root]);
        assert_eq!(t.columns, vec!["value"]);
        let rows = t.rows(&doc, src, 0, 10);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].cells[0].col, "value");
        assert_eq!(rows[0].cells[0].text, "42");
        assert_eq!(resolve_path(&doc, &t.path), Some(doc.root));
    }

    #[test]
    fn rows_chunked_and_nested() {
        let (src, doc, users) = setup();
        let mut next = 0u64;
        let t = Table::create(&doc, users, &mut next);
        let rows = t.rows(&doc, src, 0, 2);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].cells.iter().find(|c| c.col == "name").unwrap().text, "Alice");
        let tags = rows[0].cells.iter().find(|c| c.col == "tags").unwrap();
        assert!(tags.nested);
        // 嵌套单元格显示原始 JSON 片段（不再用 [n] 摘要）
        assert_eq!(tags.text, r#"["admin"]"#);
        let profile = rows[0].cells.iter().find(|c| c.col == "profile").unwrap();
        assert_eq!(profile.text, r#"{"level":9}"#);
        // 空数组也按原文展示
        let tags1 = rows[1].cells.iter().find(|c| c.col == "tags").unwrap();
        assert_eq!(tags1.text, "[]");
        // offset 分块
        let rows2 = t.rows(&doc, src, 2, 2);
        assert_eq!(rows2.len(), 1);
    }

    #[test]
    fn filter_any_value_any_column() {
        let (src, doc, users) = setup();
        let mut next = 0u64;
        let t = Table::create(&doc, users, &mut next);
        let tokens = split_tokens("Alice, 25\ndev"); // 逗号 + 换行混合
        assert_eq!(tokens, vec!["Alice", "25", "dev"]);
        let hit_all = t.filter(&doc, src, &tokens, None);
        assert_eq!(hit_all, vec![0, 1, 2]); // dev 在 Carol 的嵌套 tags 里
        let hit_name = t.filter(&doc, src, &["Alice".to_string(), "25".to_string()], Some(&["name".to_string()][..]));
        assert_eq!(hit_name, vec![0]); // 30 在 age 列不算
    }

    #[test]
    fn filter_restricted_to_column_subset() {
        // 列显示筛选：只搜指定列集合内的值；空集合 = 全部列被隐藏 → 无命中（区别于 None = 全部列）
        let (src, doc, users) = setup();
        let mut next = 0u64;
        let t = Table::create(&doc, users, &mut next);
        let cols = |cs: &[&str]| cs.iter().map(|c| c.to_string()).collect::<Vec<_>>();
        let name_only = cols(&["name"]);
        // Alice 在 name 列：age 集合不命中，name 集合命中
        assert!(t.filter(&doc, src, &["Alice".to_string()], Some(&cols(&["age"])[..])).is_empty());
        assert_eq!(t.filter(&doc, src, &["Alice".to_string()], Some(&name_only[..])), vec![0]);
        // 25 在 age 列：{name} 集合不命中；空集合无命中
        assert!(t.filter(&doc, src, &["25".to_string()], Some(&name_only[..])).is_empty());
        assert!(t.filter(&doc, src, &["Alice".to_string()], Some(&[][..])).is_empty());
    }

    #[test]
    fn rows_for_indices_order_and_bounds() {
        let (src, doc, users) = setup();
        let mut next = 0u64;
        let t = Table::create(&doc, users, &mut next);
        // 乱序索引保持输入顺序；越界索引（99）跳过不报错
        let rows = t.rows_for_indices(&doc, src, &[2, 0, 99, 1]);
        assert_eq!(rows.iter().map(|r| r.index).collect::<Vec<_>>(), vec![2, 0, 1]);
        let name_of = |r: &Row| r.cells.iter().find(|c| c.col == "name").unwrap().text.clone();
        assert_eq!(name_of(&rows[0]), "Carol");
        assert_eq!(name_of(&rows[1]), "Alice");
        assert_eq!(name_of(&rows[2]), "Bob");
        // 全部越界 → 空结果
        assert!(t.rows_for_indices(&doc, src, &[50, 99]).is_empty());
    }

    #[test]
    fn filter_is_case_insensitive_contains() {
        let (src, doc, users) = setup();
        let mut next = 0u64;
        let t = Table::create(&doc, users, &mut next);
        // 大小写不敏感：alice 命中 Alice
        assert_eq!(t.filter(&doc, src, &["alice".to_string()], Some(&["name".to_string()][..])), vec![0]);
        // 部分子串：ali 命中 Alice；arol 命中 Carol
        assert_eq!(t.filter(&doc, src, &["ali".to_string()], Some(&["name".to_string()][..])), vec![0]);
        assert_eq!(t.filter(&doc, src, &["arol".to_string()], Some(&["name".to_string()][..])), vec![2]);
        // 大写 token 命中小写值
        assert_eq!(t.filter(&doc, src, &["BOB".to_string()], Some(&["name".to_string()][..])), vec![1]);
    }

    #[test]
    fn filter_contains_cjk_substring() {
        let src = r#"{"users":[
            {"name":"付费用户A","level":1},
            {"name":"免费用户B","level":2}
        ]}"#;
        let doc = parse(src).unwrap();
        let users = doc.node(doc.root).children[0];
        let mut next = 0u64;
        let t = Table::create(&doc, users, &mut next);
        // 子串 "付费" 命中 "付费用户A"，但不命中 "免费用户B"
        assert_eq!(t.filter(&doc, src, &["付费".to_string()], Some(&["name".to_string()][..])), vec![0]);
        assert_eq!(t.filter(&doc, src, &["用户".to_string()], Some(&["name".to_string()][..])), vec![0, 1]);
    }

    #[test]
    fn filter_matches_nested_array_elements() {
        let src = r#"{"users":[
            {"name":"A","tags":["dev","ops"]},
            {"name":"B","tags":["qa"]}
        ]}"#;
        let doc = parse(src).unwrap();
        let users = doc.node(doc.root).children[0];
        let mut next = 0u64;
        let t = Table::create(&doc, users, &mut next);
        assert_eq!(t.filter(&doc, src, &["dev".to_string()], None), vec![0]);      // 多元素数组的元素命中
        assert_eq!(t.filter(&doc, src, &["ops".to_string()], None), vec![0]);      // 第二个元素也命中
        assert_eq!(t.filter(&doc, src, &["qa".to_string()], None), vec![1]);
        assert!(t.filter(&doc, src, &["dev ops".to_string()], None).is_empty());    // 拼接文本不匹配（原始片段 `["dev","ops"]` 也不含它）
        assert_eq!(t.filter(&doc, src, &["DEV".to_string()], None), vec![0]);      // 大小写不敏感
        assert_eq!(t.filter(&doc, src, &["ev".to_string()], None), vec![0]);       // 子串命中嵌套数组元素
    }

    #[test]
    fn filter_matches_inside_nested_object() {
        let src = r#"{"users":[{"name":"A","taxpayer":{"address":null,"bank":"hxb"}},{"name":"B"}]}"#;
        let doc = parse(src).unwrap();
        let users = doc.node(doc.root).children[0];
        let mut next = 0u64;
        let t = Table::create(&doc, users, &mut next);
        assert_eq!(t.filter(&doc, src, &["address".to_string()], None), vec![0]); // 对象内的 key 命中
        assert_eq!(t.filter(&doc, src, &["hxb".to_string()], None), vec![0]);     // 对象内的 value 也命中
        assert!(t.filter(&doc, src, &["zzz".to_string()], None).is_empty());
    }

    #[test]
    fn transpose_flattens_paths() {
        let (_src, doc, users) = setup();
        let mut next = 0u64;
        let t = Table::create(&doc, users, &mut next);
        let kv = t.transpose(&doc, 0);
        let get = |k: &str| kv.iter().find(|(p, _)| p == k).unwrap().1.clone();
        assert_eq!(get("name"), "Alice");
        assert_eq!(get("profile.level"), "9");
        assert_eq!(get("tags[0]"), "admin");
    }

    #[test]
    fn path_of_and_resolve_roundtrip() {
        let (src, doc, _users) = setup();
        // 根对象 → $；users 数组 → $.users；数组元素 → $.users[1]；嵌套 → $.users[1].profile
        let root = doc.root;
        let users = doc.node(root).children[0];
        let bob = doc.node(users).children[1];
        let profile = doc.node(bob).children.last().copied().unwrap();
        for id in [root, users, bob, profile] {
            let p = path_of(&doc, id);
            assert_eq!(resolve_path(&doc, &p), Some(id), "path {p} of {src:?}");
        }
        assert_eq!(path_of(&doc, root), "$");
        assert_eq!(path_of(&doc, users), "$.users");
        assert_eq!(path_of(&doc, bob), "$.users[1]");
        assert_eq!(path_of(&doc, profile), "$.users[1].profile");
    }

    #[test]
    fn resolve_path_missing_and_malformed() {
        let (_src, doc, _users) = setup();
        // 键不存在
        assert_eq!(resolve_path(&doc, "$.nope"), None);
        // 数组越界
        assert_eq!(resolve_path(&doc, "$.users[9]"), None);
        // 对象节点上用下标 / 数组节点上用键
        assert_eq!(resolve_path(&doc, "$[0]"), None);
        assert_eq!(resolve_path(&doc, "$.users.name"), None);
        // 非法格式：缺 $ / 空段 / 悬空下标
        assert_eq!(resolve_path(&doc, "users"), None);
        assert_eq!(resolve_path(&doc, "$."), None);
        assert_eq!(resolve_path(&doc, "$.users["), None);
        assert_eq!(resolve_path(&doc, ""), None);
    }
}
