#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Val { Null, Bool, Num, Str, Obj, Arr }

#[derive(Debug, Clone)]
pub struct Node {
    pub parent: Option<usize>,
    pub children: Vec<usize>,
    pub key: Option<String>,
    pub val: Val,
    pub len: usize,
    pub literal: Option<String>,
    pub start: usize,
    pub end: usize,
}

#[derive(Debug)]
pub struct Doc {
    pub nodes: Vec<Node>,
    pub root: usize,
}

#[derive(Debug)]
pub struct ParseError {
    pub offset: usize,
    pub msg: String,
}

const MAX_DEPTH: usize = 512;

struct Parser<'a> {
    /// 原始输入的字节视图（快路径 ASCII 扫描用）
    b: &'a [u8],
    /// 同一输入的 &str 视图（多字节分支直接按字符解码，保证任意合法 UTF-8 不 panic）
    s: &'a str,
    pos: usize,
    nodes: Vec<Node>,
}

pub fn parse(src: &str) -> Result<Doc, ParseError> {
    let mut p = Parser { b: src.as_bytes(), s: src, pos: 0, nodes: Vec::new() };
    p.skip_ws();
    let root = p.value(None, 0)?;
    p.skip_ws();
    if p.pos != p.b.len() {
        return Err(p.err("unexpected trailing characters".into()));
    }
    Ok(Doc { nodes: p.nodes, root })
}

impl<'a> Parser<'a> {
    fn err(&self, msg: String) -> ParseError { ParseError { offset: self.pos, msg } }
    fn peek(&self) -> Option<u8> { self.b.get(self.pos).copied() }
    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) { self.pos += 1; }
    }
    fn expect(&mut self, c: u8) -> Result<(), ParseError> {
        if self.peek() == Some(c) { self.pos += 1; Ok(()) }
        else { Err(self.err(format!("expected '{}'", c as char))) }
    }

    fn value(&mut self, parent: Option<usize>, depth: usize) -> Result<usize, ParseError> {
        if depth > MAX_DEPTH { return Err(self.err("recursion limit (512) exceeded".into())); }
        let start = self.pos;
        let id = self.nodes.len();
        self.nodes.push(Node {
            parent, children: Vec::new(), key: None, val: Val::Null,
            len: 0, literal: None, start, end: start,
        });
        match self.peek() {
            Some(b'{') => self.object(id, depth)?,
            Some(b'[') => self.array(id, depth)?,
            Some(b'"' | b'\'') => {
                let s = self.string()?;
                self.nodes[id].val = Val::Str;
                self.nodes[id].literal = Some(s);
            }
            // 宽松裸词值：true/false/null 关键字与 Python 风格 None/True/False
            // 先归类，其余无引号连续词按字符串（详见 bare_word）
            Some(c) if c == b'_' || c == b'$' || c.is_ascii_alphabetic() => self.bare_word(id)?,
            Some(c) if c == b'-' || c.is_ascii_digit() => self.number(id)?,
            _ => return Err(self.err("unexpected character".into())),
        }
        self.nodes[id].end = self.pos;
        Ok(id)
    }

    /// 宽松裸词值：无引号的连续标识符词（字符集同裸键 [A-Za-z0-9_$]*）。
    /// true/false/null 关键字及 Python 风格 None/True/False 归类为对应标量，
    /// 其余一律按字符串（如 `BILLING_SUCCESS` → "BILLING_SUCCESS"）。
    /// literal 保留原始写法（树视图原样显示，格式化 span 保真输出）；
    /// 数字开头的输入不进此分支（number 优先且保持严格：`01`/`1.` 仍报错）。
    fn bare_word(&mut self, id: usize) -> Result<(), ParseError> {
        let start = self.pos;
        while matches!(self.peek(), Some(c) if c == b'_' || c == b'$' || c.is_ascii_alphanumeric()) {
            self.pos += 1;
        }
        let word = &self.s[start..self.pos];
        self.nodes[id].val = match word {
            "true" | "True" | "false" | "False" => Val::Bool,
            "null" | "None" => Val::Null,
            _ => Val::Str,
        };
        self.nodes[id].literal = Some(word.to_string());
        Ok(())
    }

    fn number(&mut self, id: usize) -> Result<(), ParseError> {
        let start = self.pos;
        if self.peek() == Some(b'-') { self.pos += 1; }
        // JSON 数字：'-' 后必须至少一位数字，且禁止前导零（0 或 0.digits 合法，01 非法）
        if !matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
            return Err(self.err("invalid number".into()));
        }
        if self.peek() == Some(b'0') {
            self.pos += 1;
            if matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
                return Err(self.err("invalid number".into()));
            }
        } else {
            while matches!(self.peek(), Some(c) if c.is_ascii_digit()) { self.pos += 1; }
        }
        if self.peek() == Some(b'.') {
            self.pos += 1;
            if !matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
                return Err(self.err("invalid number".into()));
            }
            while matches!(self.peek(), Some(c) if c.is_ascii_digit()) { self.pos += 1; }
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.pos += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) { self.pos += 1; }
            if !matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
                return Err(self.err("invalid number".into()));
            }
            while matches!(self.peek(), Some(c) if c.is_ascii_digit()) { self.pos += 1; }
        }
        let text = std::str::from_utf8(&self.b[start..self.pos]).unwrap().to_string();
        self.nodes[id].val = Val::Num;
        self.nodes[id].literal = Some(text);
        Ok(())
    }

    /// 引号参数化的字符串：quote=b'"' 即严格 JSON 字符串（行为不变）；
    /// quote=b'\'' 为宽松单引号（\' 转义为引号本身，其余转义与双引号规则一致，
    /// 内容中的 " 为字面量）。span 恒覆盖原始文本（含原始引号）。
    fn string(&mut self) -> Result<String, ParseError> {
        let quote = match self.peek() {
            Some(c @ (b'"' | b'\'')) => c,
            _ => return Err(self.err("expected string".into())),
        };
        self.pos += 1;
        let mut out = String::new();
        loop {
            // 快速段：连续普通 ASCII 整段拷贝（输入为 &str，该段必然合法 UTF-8）
            let start = self.pos;
            while matches!(self.peek(), Some(c) if c != quote && c != b'\\' && c < 0x80) {
                self.pos += 1;
            }
            if self.pos > start {
                let s = std::str::from_utf8(&self.b[start..self.pos]).unwrap();
                out.push_str(s);
            }
            match self.peek() {
                None => return Err(self.err("unterminated string".into())),
                Some(c) if c == quote => { self.pos += 1; return Ok(out); }
                Some(b'\\') => {
                    self.pos += 1;
                    match self.peek() {
                        Some(c) if c == quote => out.push(quote as char),
                        Some(b'"') => out.push('"'),
                        Some(b'\'') => out.push('\''), // 宽松：\' 还原为单引号（双/单引号串内均合法）
                        Some(b'\\') => out.push('\\'),
                        Some(b'/') => out.push('/'),
                        Some(b'b') => out.push('\u{0008}'),
                        Some(b'f') => out.push('\u{000c}'),
                        Some(b'n') => out.push('\n'),
                        Some(b'r') => out.push('\r'),
                        Some(b't') => out.push('\t'),
                        Some(b'u') => {
                            self.pos += 1;
                            let hi = self.hex4()?;
                            let cp = match hi {
                                // 高位代理：必须紧跟 \uDC00-\uDFFF 低位代理
                                0xD800..0xDC00 => {
                                    if self.b.get(self.pos) == Some(&b'\\') {
                                        self.pos += 1;
                                        self.expect(b'u')?;
                                        let lo = self.hex4()?;
                                        if !(0xDC00..0xE000).contains(&lo) {
                                            return Err(self.err("lone surrogate".into()));
                                        }
                                        0x10000 + ((hi - 0xD800) << 10) + (lo - 0xDC00)
                                    } else {
                                        return Err(self.err("lone surrogate".into()));
                                    }
                                }
                                // 低位代理打头同样是非法的孤立代理
                                0xDC00..0xE000 => return Err(self.err("lone surrogate".into())),
                                _ => hi,
                            };
                            out.push(char::from_u32(cp).unwrap_or('\u{fffd}'));
                            continue; // hex4 已消耗 4 位，跳过公共 +=1
                        }
                        _ => return Err(self.err("invalid escape".into())),
                    }
                    self.pos += 1;
                }
                _ => {
                    // 多字节 UTF-8：pos 只推进过完整字符，self.s[self.pos..] 是合法 &str 切片，
                    // 直接取当前字符解码——不会 panic，也不需要对剩余输入做重复校验（O(n) 总量）
                    let c = self.s[self.pos..].chars().next().unwrap();
                    out.push(c);
                    self.pos += c.len_utf8();
                }
            }
        }
    }

    fn hex4(&mut self) -> Result<u32, ParseError> {
        if self.pos + 4 > self.b.len() { return Err(self.err("bad \\u escape".into())); }
        let s = std::str::from_utf8(&self.b[self.pos..self.pos + 4])
            .map_err(|_| self.err("bad \\u escape".into()))?;
        let n = u32::from_str_radix(s, 16).map_err(|_| self.err("bad \\u escape".into()))?;
        self.pos += 4;
        Ok(n)
    }

    /// 对象键：双/单引号字符串，或宽松裸键（JS 标识符风格 [A-Za-z_$][A-Za-z0-9_$]*，不允许空）
    fn key(&mut self) -> Result<String, ParseError> {
        match self.peek() {
            Some(b'"' | b'\'') => self.string(),
            Some(c) if c == b'_' || c == b'$' || c.is_ascii_alphabetic() => {
                let start = self.pos;
                self.pos += 1;
                while matches!(self.peek(), Some(c) if c == b'_' || c == b'$' || c.is_ascii_alphanumeric()) {
                    self.pos += 1;
                }
                Ok(self.s[start..self.pos].to_string())
            }
            _ => Err(self.err("expected key".into())),
        }
    }

    fn object(&mut self, id: usize, depth: usize) -> Result<(), ParseError> {
        self.expect(b'{')?;
        self.nodes[id].val = Val::Obj;
        self.skip_ws();
        if self.peek() == Some(b'}') { self.pos += 1; return Ok(()); }
        loop {
            self.skip_ws();
            let key_start = self.pos;
            let bare = !matches!(self.peek(), Some(b'"' | b'\''));
            let key = self.key()?;
            if bare {
                self.skip_ws();
                if self.peek() != Some(b':') {
                    // 裸键后无冒号：多半是本想写值（非法字符）而非漏冒号，
                    // 回退到裸键起点报错，保持与严格模式一致的定位
                    self.pos = key_start;
                    return Err(self.err(format!("unexpected character '{}'", self.b[key_start] as char)));
                }
            }
            self.skip_ws();
            self.expect(b':')?;
            self.skip_ws();
            let child = self.value(Some(id), depth + 1)?;
            self.nodes[child].key = Some(key);
            self.nodes[id].children.push(child);
            self.skip_ws();
            match self.peek() {
                Some(b',') => {
                    self.pos += 1;
                    self.skip_ws();
                    // 宽松尾逗号：仅当已至少有一个成员（{,} 仍非法）
                    if self.peek() == Some(b'}') { self.pos += 1; break; }
                }
                Some(b'}') => { self.pos += 1; break; }
                _ => return Err(self.err("expected ',' or '}'".into())),
            }
        }
        self.nodes[id].len = self.nodes[id].children.len();
        Ok(())
    }

    fn array(&mut self, id: usize, depth: usize) -> Result<(), ParseError> {
        self.expect(b'[')?;
        self.nodes[id].val = Val::Arr;
        self.skip_ws();
        if self.peek() == Some(b']') { self.pos += 1; return Ok(()); }
        loop {
            self.skip_ws();
            let child = self.value(Some(id), depth + 1)?;
            let idx = self.nodes[id].children.len();
            self.nodes[child].key = Some(idx.to_string());
            self.nodes[id].children.push(child);
            self.skip_ws();
            match self.peek() {
                Some(b',') => {
                    self.pos += 1;
                    self.skip_ws();
                    // 宽松尾逗号：仅当已至少有一个元素（[, 仍非法）
                    if self.peek() == Some(b']') { self.pos += 1; break; }
                }
                Some(b']') => { self.pos += 1; break; }
                _ => return Err(self.err("expected ',' or ']'".into())),
            }
        }
        self.nodes[id].len = self.nodes[id].children.len();
        Ok(())
    }
}

impl Doc {
    pub fn node(&self, id: usize) -> &Node { &self.nodes[id] }
    pub fn line_col(src: &str, offset: usize) -> (usize, usize) {
        let before = &src[..offset];
        let line = before.bytes().filter(|&b| b == b'\n').count() + 1;
        let col = before.rfind('\n').map(|p| offset - p - 1).unwrap_or(offset) + 1;
        (line, col)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spans_nested() {
        let src = r#"{"a": [1, {"b": "x"}]}"#;
        let doc = parse(src).unwrap();
        let root = doc.node(doc.root);
        assert_eq!(&src[root.start..root.end], src);
        let arr = doc.node(root.children[0]);
        assert_eq!(&src[arr.start..arr.end], r#"[1, {"b": "x"}]"#);
        let obj = doc.node(arr.children[1]);
        let bx = doc.node(obj.children[0]);
        assert_eq!(bx.key.as_deref(), Some("b"));
        assert_eq!(bx.literal.as_deref(), Some("x"));
        assert_eq!(&src[bx.start..bx.end], r#""x""#);
    }

    #[test]
    fn escapes_and_unicode() {
        let src = r#"{"s": "a\"b\n你"}"#;
        let doc = parse(src).unwrap();
        let s = doc.node(doc.node(doc.root).children[0]);
        assert_eq!(s.literal.as_deref(), Some("a\"b\n你"));
        assert_eq!(&src[s.start..s.end], r#""a\"b\n你""#); // span 覆盖原始转义文本
    }

    #[test]
    fn error_position() {
        let src = "{\n  \"a\": 1,\n  bad\n}";
        let e = parse(src).unwrap_err();
        let (line, col) = Doc::line_col(src, e.offset);
        assert_eq!((line, col), (3, 3));
    }

    #[test]
    fn depth_limit() {
        let src = "[".repeat(600) + &"]".repeat(600);
        assert!(parse(&src).is_err());
    }

    fn doc_literal(d: &Doc, id: usize) -> Option<&str> {
        d.node(id).literal.as_deref()
    }

    #[test]
    fn surrogate_pairs_and_escapes() {
        let src = r#"{"e": "\uD83D\uDE00\t\b\f\r"}"#;
        let doc = parse(src).unwrap();
        let s = doc.node(doc.node(doc.root).children[0]);
        assert_eq!(s.literal.as_deref(), Some("😀\t\u{0008}\u{000c}\r"));
    }

    #[test]
    fn lone_surrogate_is_error() {
        assert!(parse(r#""\uD83D""#).is_err());
        assert!(parse(r#""\uD83D\u0041""#).is_err());
        assert!(parse(r#""\uDC00""#).is_err()); // 低位代理打头也非法
    }

    #[test]
    fn bad_unicode_escape_is_error_not_panic() {
        assert!(parse("\"\u{4f60}\"").is_ok());        // 合法 UTF-8 字符串
        assert!(parse("\"\\u你好\"").is_err());          // \\u 后跟多字节字符 → 解析错误，不 panic
        assert!(parse("\"\\u12\"").is_err());            // 不足 4 位 → 错误
    }

    #[test]
    fn consecutive_multibyte_no_panic() {
        for src in ["\"你好\"", "\"中文测试\"", "{\"名字\": \"值\"}", "\"你😀\"", "[\"ab你\", \"😀你c\"]"] {
            let doc = parse(src).unwrap(); // 必须不 panic 且解析成功
            let _ = doc.node(doc.root).end;
        }
        let doc = parse(r#"{"k": "中文value混合123"}"#).unwrap();
        let s = doc.node(doc.node(doc.root).children[0]);
        assert_eq!(s.literal.as_deref(), Some("中文value混合123"));
    }

    #[test]
    fn invalid_numbers_rejected() {
        for bad in ["-", "01", "1.", "1e", "1e+", "0.5."] {
            assert!(parse(bad).is_err(), "should reject {bad}");
        }
        for good in ["0", "-1.5e-3", "123", "0.25"] {
            let d = parse(good).unwrap();
            assert_eq!(doc_literal(&d, d.root).as_deref(), Some(good));
        }
    }

    // ---- 宽松解析：单引号 / 裸键 / 尾逗号 ----

    #[test]
    fn single_quotes_keys_and_values() {
        let src = "{'a': 'x', b: 'it\\'s', \"c\": \"has 'q'\"}";
        let doc = parse(src).unwrap();
        let root = doc.node(doc.root);
        let a = doc.node(root.children[0]);
        assert_eq!(a.key.as_deref(), Some("a"));
        assert_eq!(a.literal.as_deref(), Some("x"));
        assert_eq!(&src[a.start..a.end], "'x'"); // span 覆盖原始单引号文本
        let b = doc.node(root.children[1]);
        assert_eq!(b.key.as_deref(), Some("b"));
        assert_eq!(b.literal.as_deref(), Some("it's")); // \' 转义
        let c = doc.node(root.children[2]);
        assert_eq!(c.literal.as_deref(), Some("has 'q'")); // 双引号串内单引号为字面量
    }

    #[test]
    fn single_quote_string_with_double_quotes_inside() {
        let doc = parse("{'s': 'say \"hi\"'}").unwrap();
        let s = doc.node(doc.node(doc.root).children[0]);
        assert_eq!(s.literal.as_deref(), Some("say \"hi\"")); // 单引号串内 " 为字面量
    }

    #[test]
    fn bare_keys() {
        let doc = parse("{a: 1, _b$: 'x'}").unwrap();
        let root = doc.node(doc.root);
        assert_eq!(doc.node(root.children[0]).key.as_deref(), Some("a"));
        assert_eq!(doc.node(root.children[1]).key.as_deref(), Some("_b$"));
    }

    #[test]
    fn trailing_commas() {
        assert!(parse("{'a': 1,}").is_ok());
        assert!(parse("[1, 2,]").is_ok());
        assert!(parse("[\n  1,\n  2,\n]").is_ok()); // 跨行尾逗号
        let doc = parse("[{'k': [1,],},]").unwrap();
        let root = doc.node(doc.root);
        assert_eq!(root.len, 1);
        let inner = doc.node(root.children[0]);
        assert_eq!(doc.node(inner.children[0]).len, 1);
    }

    #[test]
    fn strict_forms_still_rejected() {
        for bad in ["{,}", "[,]", "{a:}", "{: 1}", "{a-b: 1}", "{\"a\" 1}", "{'a': }", "{'a}", "{a: 1,}extra"] {
            assert!(parse(bad).is_err(), "should reject {bad}");
        }
        // 尾逗号后跟新内容仍报错
        assert!(parse("[1, 2, 3]").is_ok());
        assert!(parse("{'a': 1,, 'b': 2}").is_err());
    }

    #[test]
    fn double_quote_rules_unchanged() {
        // 宽松：双引号串内 \' 也还原为单引号（2026-10-15 放开）
        assert!(parse("{\"a\": 'x'}").is_ok()); // 值用单引号合法（宽松）
        let d = parse("\"\\'\"").unwrap();
        assert_eq!(doc_literal(&d, d.root), Some("'"));
        // 双引号串标准转义不受影响
        let doc = parse("{\"a\": \"x\\ny\"}").unwrap();
        let s = doc.node(doc.node(doc.root).children[0]);
        assert_eq!(s.literal.as_deref(), Some("x\ny"));
    }

    #[test]
    fn escaped_single_quotes_inside_double_quotes() {
        // 用户实测：{"a":"{\'c\':\'b\'}"} —— 转义的内嵌单引号 JSON
        let doc = parse("{\"a\":\"{\\'c\\':\\'b\\'}\"}").unwrap();
        let s = doc.node(doc.node(doc.root).children[0]);
        assert_eq!(s.literal.as_deref(), Some("{'c':'b'}"));
    }

    // ---- 宽松裸词值（无引号连续字符串）----

    #[test]
    fn bare_values_python_literals() {
        // 7.txt 场景：Python 风格 None/True/False 混入 JSON
        let doc = parse(r#"{"a": None, "b": True, "c": False, "d": null}"#).unwrap();
        let root = doc.node(doc.root);
        assert_eq!(doc.node(root.children[0]).val, Val::Null);
        assert_eq!(doc.node(root.children[0]).literal.as_deref(), Some("None")); // literal 原样
        assert_eq!(doc.node(root.children[1]).val, Val::Bool);
        assert_eq!(doc.node(root.children[2]).val, Val::Bool);
        assert_eq!(doc.node(root.children[3]).val, Val::Null);
    }

    #[test]
    fn bare_values_plain_words() {
        let doc = parse(r#"{"status": BILLING_SUCCESS, "list": [None, hello]}"#).unwrap();
        let root = doc.node(doc.root);
        let s = doc.node(root.children[0]);
        assert_eq!(s.val, Val::Str);
        assert_eq!(s.literal.as_deref(), Some("BILLING_SUCCESS"));
        let arr = doc.node(root.children[1]);
        assert_eq!(doc.node(arr.children[0]).val, Val::Null);
        assert_eq!(doc.node(arr.children[1]).literal.as_deref(), Some("hello"));
        // 顶层裸词
        let d = parse("world").unwrap();
        assert_eq!(doc_literal(&d, d.root), Some("world"));
        // 7.txt 原文可解析
        assert!(parse(r#"{"custom_code": None, "data": {"n": 1}}"#).is_ok());
    }

    #[test]
    fn bare_words_lenient_classification() {
        // 关键字前缀的裸词不再报错，按字符串（宽松语义）
        assert_eq!(doc_literal(&parse("tru").unwrap(), 0), Some("tru"));
        // 关键字本体行为不变
        assert_eq!(doc_literal(&parse("true").unwrap(), 0), Some("true"));
        // 数字严格性不受影响（数字开头不进裸词分支）
        for bad in ["-", "01", "1.", "1e", "1e+", "0.5."] {
            assert!(parse(bad).is_err(), "should reject {bad}");
        }
        // 缺值仍报错；裸键后无冒号仍报错
        assert!(parse("{'a': }").is_err());
        assert!(parse("{\"a\": 1, bad}").is_err());
    }
}
