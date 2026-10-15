mod detector;
pub mod formatter;
pub mod parser;
mod state;
pub mod table;

use parser::Val;
use serde::Serialize;
use state::{AppState, Session};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DocInfo {
    size: usize,
    root_id: u64,
    root_kind: &'static str,
    error_line: Option<usize>,
    error_col: Option<usize>,
    error_msg: Option<String>,
    detections: Vec<detector::Detected>, // 仅解析失败时非空
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct NodeDto {
    id: u64,
    key: Option<String>,
    kind: &'static str,
    len: usize,
    literal: Option<String>,
    has_children: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Locate {
    start_line: usize,
    end_line: usize,
    start_col: usize,
    end_col: usize,
}

fn doc_info(s: &Session) -> DocInfo {
    match &s.doc {
        Some(doc) => DocInfo {
            size: s.src.len(),
            root_id: doc.root as u64,
            root_kind: Session::node_kind(doc.node(doc.root)),
            error_line: None, error_col: None, error_msg: None,
            detections: Vec::new(),
        },
        None => DocInfo { size: 0, root_id: 0, root_kind: "obj", error_line: None, error_col: None, error_msg: None, detections: Vec::new() },
    }
}

fn with_doc<R>(state: &AppState, f: impl FnOnce(&Session) -> R) -> R {
    let s = state.0.lock().unwrap_or_else(|e| e.into_inner());
    f(&s)
}

const NO_DOC: &str = "尚未加载 JSON 文档";
const STALE_TABLE: &str = "表格已失效，请重新从树生成表格";

#[tauri::command]
fn load_text(state: tauri::State<AppState>, text: String) -> DocInfo {
    let mut s = state.0.lock().unwrap_or_else(|e| e.into_inner());
    if let Err(e) = s.load(text.clone()) {
        let (line, col) = parser::Doc::line_col(&text, e.offset);
        let size = s.src.len();
        let root_id = s.doc.as_ref().map(|d| d.root as u64).unwrap_or(0);
        return DocInfo {
            size, root_id,
            root_kind: "obj", // 错误分支 root 无意义，占位
            error_line: Some(line), error_col: Some(col),
            error_msg: Some(e.msg),
            detections: detector::detect(&text), // 失败即探测（同一请求内，无额外 IPC）
        };
    }
    doc_info(&s)
}

/// 树节点字面量传输上限：超大字符串（如 5MB 转义 JSON）原样走 IPC + 进 DOM
/// （标签文本节点 + title 属性）是加载/交互卡顿主因之一。树的展示/悬停只看预览，
/// 完整值由「复制Value」走 node_text 另行获取，不受此截断影响。
const LITERAL_MAX_CHARS: usize = 2000;

fn cap_literal(lit: &str) -> String {
    if lit.chars().count() <= LITERAL_MAX_CHARS {
        return lit.to_string();
    }
    let mut out: String = lit.chars().take(LITERAL_MAX_CHARS).collect();
    out.push('…');
    out
}

#[tauri::command]
fn get_children(state: tauri::State<AppState>, node_id: u64, offset: u32, limit: u32) -> Result<Vec<NodeDto>, String> {
    with_doc(&state, |s| {
        let doc = s.try_doc().ok_or_else(|| NO_DOC.to_string())?;
        let children = doc.node(node_id as usize)
            .children
            .get(offset as usize..)
            .unwrap_or(&[]);
        Ok(children.iter()
            .take(limit as usize)
            .map(|&c| {
                let n = doc.node(c);
                NodeDto {
                    id: c as u64,
                    key: n.key.clone(),
                    kind: Session::node_kind(n),
                    len: n.len,
                    literal: if matches!(n.val, Val::Obj | Val::Arr) { None }
                             else { n.literal.as_deref().map(cap_literal) },
                    has_children: !n.children.is_empty(),
                }
            })
            .collect())
    })
}

#[tauri::command]
fn locate(state: tauri::State<AppState>, node_id: u64) -> Result<Locate, String> {
    with_doc(&state, |s| {
        let doc = s.try_doc().ok_or_else(|| NO_DOC.to_string())?;
        let n = doc.node(node_id as usize);
        // 行号走 line_starts 二分索引（O(log n)）；列号仍需行内偏移，量级为单行长度
        let start_line = s.line_of(n.start);
        let end_line = s.line_of(n.end.saturating_sub(1));
        let start_col = col_of(&s.src, n.start);
        let end_col = col_of(&s.src, n.end.saturating_sub(1));
        Ok(Locate { start_line, end_line, start_col, end_col })
    })
}

/// 行内 1-based 列号（从行首扫描，行长度有限，开销可忽略）
fn col_of(src: &str, offset: usize) -> usize {
    src[..offset].rfind('\n').map(|p| offset - p - 1).unwrap_or(offset) + 1
}

/// 表格单元格 → 原文定位：在行节点中找到该字段，返回其原文 span 的行列范围
#[tauri::command]
fn locate_cell(state: tauri::State<AppState>, row_node_id: u64, col: String) -> Result<Locate, String> {
    with_doc(&state, |s| {
        let doc = s.try_doc().ok_or_else(|| NO_DOC.to_string())?;
        let row = doc.nodes.get(row_node_id as usize).ok_or("节点不存在")?;
        let child = row.children.iter().map(|&c| doc.node(c))
            .find(|n| n.key.as_deref() == Some(col.as_str()))
            .ok_or_else(|| format!("该行没有字段 {col}"))?;
        let start_line = s.line_of(child.start);
        let end_line = s.line_of(child.end.saturating_sub(1));
        let start_col = col_of(&s.src, child.start);
        let end_col = col_of(&s.src, child.end.saturating_sub(1));
        Ok(Locate { start_line, end_line, start_col, end_col })
    })
}

/// 表格单元格 → 字段节点 id：供单元格右键「转为表格」下钻（任意类型的嵌套节点都可建单行/N 行表）
#[tauri::command]
fn locate_cell_node(state: tauri::State<AppState>, row_node_id: u64, col: String) -> Result<Option<u64>, String> {
    with_doc(&state, |s| {
        let doc = s.try_doc().ok_or_else(|| NO_DOC.to_string())?;
        let row = doc.nodes.get(row_node_id as usize).ok_or("节点不存在")?;
        Ok(row.children.iter().copied()
            .find(|&c| doc.node(c).key.as_deref() == Some(col.as_str()))
            .map(|c| c as u64))
    })
}

#[tauri::command]
fn node_at_line(state: tauri::State<AppState>, line: u32) -> Result<Option<u64>, String> {
    with_doc(&state, |s| {
        let doc = s.try_doc().ok_or_else(|| NO_DOC.to_string())?;
        // 线性扫最内层包含该行的节点：取 start_line <= line <= end_line 的最小 span
        // 行号经 line_starts 二分索引计算，整体 O(nodes × log lines)
        let line = line as usize;
        let mut best: Option<(usize, usize)> = None;
        for (i, n) in doc.nodes.iter().enumerate() {
            let sl = s.line_of(n.start);
            let el = s.line_of(n.end.saturating_sub(1));
            if sl <= line && line <= el {
                let span = n.end - n.start;
                if best.map(|(_, bs)| span < bs).unwrap_or(true) {
                    best = Some((i, span));
                }
            }
        }
        Ok(best.map(|(i, _)| i as u64))
    })
}

/// 节点 → 根到该节点的 id 链：供前端在树未懒加载到目标深度时重建选中路径
/// （原文点击定位到深层节点时，前端 parentOf 映射尚未注册该节点）
#[tauri::command]
fn node_ancestors(state: tauri::State<AppState>, node_id: u64) -> Result<Vec<u64>, String> {
    with_doc(&state, |s| {
        let doc = s.try_doc().ok_or_else(|| NO_DOC.to_string())?;
        let mut chain: Vec<u64> = Vec::new();
        let mut cur = node_id as usize;
        loop {
            let n = doc.nodes.get(cur).ok_or("节点不存在")?;
            chain.push(cur as u64);
            match n.parent { Some(p) => cur = p, None => break }
        }
        chain.reverse();
        Ok(chain)
    })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FormatOut {
    text: String,
    size: usize,
    root_id: u64,
    root_kind: &'static str,
    error_line: Option<usize>,
    error_col: Option<usize>,
    error_msg: Option<String>,
    already_loaded: bool, // 成功时 state 已载入新文档，前端可跳过 load_text
}

fn format_text_impl(s: &mut Session, indent: usize) -> Result<FormatOut, String> {
    if s.doc.is_none() { return Err(NO_DOC.into()); }
    match formatter::format(&s.src, indent) {
        // 格式化产物一定合法：重载进状态，root_id 用新 doc 的根。
        // load 失败属于异常（产物必合法），不再静默吞掉
        Ok(text) => {
            if let Err(e) = s.load(text.clone()) {
                return Err(e.msg);
            }
            let info = doc_info(&s);
            Ok(FormatOut {
                text, size: info.size, root_id: info.root_id, root_kind: info.root_kind,
                error_line: None, error_col: None, error_msg: None,
                already_loaded: true,
            })
        }
        Err(e) => Ok(FormatOut {
            text: String::new(), size: s.src.len(), root_id: 0, root_kind: "obj",
            error_line: Some(e.line), error_col: Some(e.col), error_msg: Some(e.msg),
            already_loaded: false,
        }),
    }
}

fn minify_text_impl(s: &mut Session) -> Result<FormatOut, String> {
    if s.doc.is_none() { return Err(NO_DOC.into()); }
    match formatter::minify(&s.src) {
        Ok(text) => {
            if let Err(e) = s.load(text.clone()) {
                return Err(e.msg);
            }
            let info = doc_info(&s);
            Ok(FormatOut {
                text, size: info.size, root_id: info.root_id, root_kind: info.root_kind,
                error_line: None, error_col: None, error_msg: None,
                already_loaded: true,
            })
        }
        Err(e) => Ok(FormatOut {
            text: String::new(), size: s.src.len(), root_id: 0, root_kind: "obj",
            error_line: Some(e.line), error_col: Some(e.col), error_msg: Some(e.msg),
            already_loaded: false,
        }),
    }
}

#[tauri::command]
fn format_text(state: tauri::State<AppState>, indent: u32) -> Result<FormatOut, String> {
    let mut s = state.0.lock().unwrap_or_else(|e| e.into_inner());
    format_text_impl(&mut s, indent as usize)
}

#[tauri::command]
fn minify_text(state: tauri::State<AppState>) -> Result<FormatOut, String> {
    let mut s = state.0.lock().unwrap_or_else(|e| e.into_inner());
    minify_text_impl(&mut s)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TableInfo {
    id: u64,
    path: String,
    columns: Vec<String>,
    row_count: usize,
}

#[tauri::command]
fn node_text(state: tauri::State<AppState>, node_id: u64) -> Result<String, String> {
    let s = state.0.lock().unwrap_or_else(|e| e.into_inner());
    let doc = s.try_doc().ok_or_else(|| NO_DOC.to_string())?;
    let n = doc.node(node_id as usize);
    Ok(s.src[n.start..n.end].to_string())
}

#[tauri::command]
fn table_create(state: tauri::State<AppState>, node_id: u64) -> Result<TableInfo, String> {
    let mut s = state.0.lock().unwrap_or_else(|e| e.into_inner());
    let Session { doc, tables, next_table_id, .. } = &mut *s;
    let doc = doc.as_ref().ok_or_else(|| NO_DOC.to_string())?;
    // 任意节点都可建表（数组 = N 行；对象/标量 = 单行），仅节点不存在才是错误
    if doc.nodes.get(node_id as usize).is_none() { return Err("节点不存在".into()); }
    let t = table::Table::create(doc, node_id as usize, next_table_id);
    let info = TableInfo {
        id: t.id, path: t.path.clone(), columns: t.columns.clone(), row_count: t.row_nodes.len(),
    };
    tables.insert(t.id, t);
    Ok(info)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TableReopen {
    info: TableInfo,
    hits: Vec<u64>,
}

/// 编辑重解析后按路径重挂数组表格：path 解析到当前文档的数组节点则建表并立即执行过滤，
/// 返回新表信息 + 命中行索引；路径失效（数组被删/改名）→ Err，前端据此收起面板
#[tauri::command]
fn table_reopen(state: tauri::State<AppState>, path: String, values: Vec<String>, col: Option<String>) -> Result<TableReopen, String> {
    let mut s = state.0.lock().unwrap_or_else(|e| e.into_inner());
    let Session { doc, src, tables, next_table_id, .. } = &mut *s;
    let doc = doc.as_ref().ok_or_else(|| NO_DOC.to_string())?;
    let node_id = table::resolve_path(doc, &path).ok_or_else(|| "文档中已无该数组".to_string())?;
    let t = table::Table::create(doc, node_id, next_table_id);
    let hits = t.filter(doc, src, &values, col.as_ref().map(|c| std::slice::from_ref(c)));
    let info = TableInfo {
        id: t.id, path: t.path.clone(), columns: t.columns.clone(), row_count: t.row_nodes.len(),
    };
    tables.insert(t.id, t);
    Ok(TableReopen { info, hits: hits.into_iter().map(|i| i as u64).collect() })
}

#[tauri::command]
fn get_rows(state: tauri::State<AppState>, table_id: u64, offset: u32, limit: u32) -> Result<Vec<table::Row>, String> {
    let s = state.0.lock().unwrap_or_else(|e| e.into_inner());
    let t = s.tables.get(&table_id).ok_or_else(|| STALE_TABLE.to_string())?;
    Ok(t.rows(s.doc(), &s.src, offset as usize, limit as usize))
}

/// 按全表 0 基行索引直接取行（过滤后只拉命中行，避免全表传输）：
/// 越界索引跳过，输出保持输入顺序；命中集合过大时提示改用导出
#[tauri::command]
fn get_rows_by_indices(state: tauri::State<AppState>, table_id: u64, indices: Vec<u32>) -> Result<Vec<table::Row>, String> {
    let s = state.0.lock().unwrap_or_else(|e| e.into_inner());
    let t = s.tables.get(&table_id).ok_or_else(|| STALE_TABLE.to_string())?;
    if indices.len() > 10_000 {
        return Err("命中行数过多（超过 10000），请缩小过滤范围或使用「导出 Excel」".to_string());
    }
    Ok(t.rows_for_indices(s.doc(), &s.src, &indices.iter().map(|&i| i as usize).collect::<Vec<_>>()))
}

#[tauri::command]
fn table_filter(state: tauri::State<AppState>, table_id: u64, values: Vec<String>, cols: Option<Vec<String>>) -> Result<Vec<u64>, String> {
    let s = state.0.lock().unwrap_or_else(|e| e.into_inner());
    let t = s.tables.get(&table_id).ok_or_else(|| STALE_TABLE.to_string())?;
    Ok(t.filter(s.doc(), &s.src, &values, cols.as_deref()).into_iter().map(|i| i as u64).collect())
}

#[derive(Serialize)]
struct PathValue { path: String, value: String }

#[tauri::command]
fn row_transpose(state: tauri::State<AppState>, table_id: u64, row: u32) -> Result<Vec<PathValue>, String> {
    let s = state.0.lock().unwrap_or_else(|e| e.into_inner());
    let t = s.tables.get(&table_id).ok_or_else(|| STALE_TABLE.to_string())?;
    Ok(t.transpose(s.doc(), row as usize).into_iter()
        .map(|(path, value)| PathValue { path, value })
        .collect())
}

/// 文本按数字优先写入 Excel 单元格（i64 / 有限 f64），其余按字符串
fn xlsx_write_cell(ws: &mut rust_xlsxwriter::Worksheet, row: u32, col: u16, text: &str) -> Result<(), String> {
    let t = text.trim();
    if let Ok(i) = t.parse::<i64>() {
        ws.write_number(row, col, i as f64).map_err(|e| e.to_string())?;
        return Ok(());
    }
    if let Ok(f) = t.parse::<f64>() {
        if f.is_finite() {
            ws.write_number(row, col, f).map_err(|e| e.to_string())?;
            return Ok(());
        }
    }
    ws.write_string(row, col, text).map_err(|e| e.to_string())?;
    Ok(())
}

/// 导出当前表格（按过滤 + 列过滤范围）为 .xlsx：
/// 表头为列名，随后每个命中行一行；嵌套单元格写原始 JSON 片段
#[tauri::command]
fn table_export(state: tauri::State<AppState>, table_id: u64, path: String, values: Vec<String>, cols: Option<Vec<String>>) -> Result<String, String> {
    let s = state.0.lock().unwrap_or_else(|e| e.into_inner());
    let t = s.tables.get(&table_id).ok_or_else(|| STALE_TABLE.to_string())?;
    let doc = s.doc();
    let hit = t.filter(doc, &s.src, &values, cols.as_deref());
    let columns = &t.columns;
    if columns.len() > 16_384 { return Err("列数超出 Excel 上限（16384）".into()); }
    if hit.len() > 1_048_575 { return Err("行数超出 Excel 上限（1048576）".into()); }

    let mut wb = rust_xlsxwriter::Workbook::new();
    let ws = wb.add_worksheet();
    for (c, name) in columns.iter().enumerate() {
        ws.write_string(0, c as u16, name).map_err(|e| e.to_string())?;
    }
    let rows = t.rows_for_indices(doc, &s.src, &hit);
    for (r, row) in rows.iter().enumerate() {
        let row_idx = (r + 1) as u32;
        for (c, cell) in row.cells.iter().enumerate() {
            xlsx_write_cell(ws, row_idx, c as u16, &cell.text)?;
        }
    }
    // 首列筛 + 自适应列宽（首列 ~18，其余按内容长度、上限 30）
    let last_row = rows.len() as u32;
    let last_col = (columns.len().saturating_sub(1)) as u16;
    ws.autofilter(0, 0, last_row, last_col);
    for (c, _name) in columns.iter().enumerate() {
        let width = if c == 0 {
            18.0
        } else {
            let max_len = rows.iter()
                .filter_map(|row| row.cells.get(c).map(|cell| cell.text.chars().count()))
                .chain(std::iter::once(columns[c].chars().count()))
                .max()
                .unwrap_or(8);
            (max_len as f64 + 4.0).clamp(8.0, 30.0)
        };
        ws.set_column_width(c as u16, width);
    }
    wb.save(&path).map_err(|e| format!("写入 Excel 失败：{e}"))?;
    Ok(path)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .manage(AppState(std::sync::Mutex::new(Session::new())))
        .invoke_handler(tauri::generate_handler![
            load_text, get_children, locate, locate_cell, locate_cell_node, node_at_line, node_ancestors, node_text,
            format_text, minify_text,
            table_create, table_reopen, get_rows, get_rows_by_indices, table_filter, row_transpose, table_export
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod command_tests {
    use super::*;

    #[test]
    fn format_text_impl_reload_and_flag() {
        let mut s = Session::new();
        s.load(r#"{"a":1}"#.to_string()).unwrap();
        let out = format_text_impl(&mut s, 2).unwrap();
        assert!(out.already_loaded);
        assert_eq!(out.text, "{\n  \"a\": 1\n}");
        assert_eq!(out.root_kind, "obj");
        // state 已重载为格式化后的文本
        assert_eq!(s.src, "{\n  \"a\": 1\n}");
    }

    #[test]
    fn format_text_impl_lenient_fallback() {
        // 自研 parser 接受字符串内的裸控制字符（原与 serde_json 分歧），
        // 宽松回退后 format 与加载行为一致，不再失败
        // （宽松回退与加载共用同一 parser，「已加载但 format 失败」的场景不复存在）
        let src = "{\"a\":\"x\u{0001}y\"}";
        let mut s = Session::new();
        s.load(src.to_string()).unwrap();
        let out = format_text_impl(&mut s, 2).unwrap();
        assert!(out.error_msg.is_none());
        assert_eq!(s.src, out.text);
    }

    #[test]
    fn format_text_impl_no_doc_err() {
        let mut s = Session::new();
        assert_eq!(format_text_impl(&mut s, 2).err().as_deref(), Some(NO_DOC));
        assert_eq!(minify_text_impl(&mut s).err().as_deref(), Some(NO_DOC));
    }

    #[test]
    fn minify_text_impl_reloads_and_flag() {
        let mut s = Session::new();
        s.load("{\n  \"a\": 1\n}".to_string()).unwrap();
        let out = minify_text_impl(&mut s).unwrap();
        assert!(out.already_loaded);
        assert_eq!(out.text, "{\"a\":1}");
        assert_eq!(s.src, "{\"a\":1}");
    }
}
