use crate::parser::{self, Doc, Node, Val};
use crate::table::Table;
use std::collections::HashMap;

pub struct Session {
    pub src: String,
    /// 每行起始字节偏移（首元素恒为 0），与 src 同生命周期；供 O(log n) 行号查询
    pub line_starts: Vec<usize>,
    pub doc: Option<Doc>,
    pub tables: HashMap<u64, Table>,
    pub next_table_id: u64,
}

impl Session {
    pub fn new() -> Self {
        Session {
            src: String::new(),
            line_starts: vec![0],
            doc: None,
            tables: HashMap::new(),
            next_table_id: 0,
        }
    }

    /// 解析并缓存；失败时保留旧 doc，返回错误行列
    pub fn load(&mut self, text: String) -> Result<(), parser::ParseError> {
        match parser::parse(&text) {
            Ok(doc) => {
                self.doc = Some(doc);
                self.src = text;
                self.line_starts = build_line_starts(&self.src);
                self.tables.clear();
                Ok(())
            }
            Err(e) => Err(e),
        }
    }

    /// 字节偏移 → 1-based 行号（二分，O(log n)）
    pub fn line_of(&self, offset: usize) -> usize {
        // line_starts[0] 恒为 0，故结果 >= 1；offset 超出末尾时自然钳到最后一行
        self.line_starts.partition_point(|&p| p <= offset)
    }

    pub fn doc(&self) -> &Doc { self.doc.as_ref().expect("doc loaded") }

    /// 无文档时返回 None（命令层据此回 Err，不再 panic）
    pub fn try_doc(&self) -> Option<&Doc> { self.doc.as_ref() }

    pub fn node_kind(n: &Node) -> &'static str {
        match n.val {
            Val::Null => "null", Val::Bool => "bool", Val::Num => "num",
            Val::Str => "str", Val::Obj => "obj", Val::Arr => "arr",
        }
    }
}

/// 供 tauri manage 的全局状态
pub struct AppState(pub std::sync::Mutex<Session>);

fn build_line_starts(text: &str) -> Vec<usize> {
    let mut v = Vec::with_capacity(64);
    v.push(0);
    for (i, b) in text.bytes().enumerate() {
        if b == b'\n' { v.push(i + 1); }
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_of_matches_line_col() {
        let mut s = Session::new();
        s.load("{\n  \"a\": 你好,\n  \"b\": 2\n}".into()).unwrap_err(); // 非法也行，但这里只测行索引 → 用合法文本
        s.load("{\n  \"a\": 1,\n  \"b\": 2\n}".into()).unwrap();
        assert_eq!(s.line_of(0), 1);
        assert_eq!(s.line_of(1), 1); // \n
        assert_eq!(s.line_of(2), 2);
        assert_eq!(s.line_of(14), 3); // "b" 所在行
        assert_eq!(s.line_of(usize::MAX), 4); // 越界钳到末尾空行
        // 与 Doc::line_col 全量对拍
        for off in 0..=s.src.len() {
            assert_eq!(s.line_of(off), Doc::line_col(&s.src, off).0, "offset {off}");
        }
    }
}
