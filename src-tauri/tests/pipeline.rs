// 直接调用各模块函数验证 load → get_children → locate → filter 主链路
use json_tool_lib::{parser, table};

#[test]
fn pipeline() {
    let src = r#"{"users":[{"name":"Alice","age":30},{"name":"Bob","age":25}]}"#;
    let doc = parser::parse(src).unwrap();
    let users_id = doc.node(doc.root).children[0]; // "users" 数组节点
    let mut next = 0u64;
    let t = table::Table::create(&doc, users_id, &mut next);
    assert_eq!(t.columns, vec!["name", "age"]);
    assert_eq!(t.filter(&doc, src, &["Bob".to_string()], None), vec![1]);
    let (line, _) = parser::Doc::line_col(src, doc.node(t.row_nodes[1]).start);
    assert_eq!(line, 1); // 示例为单行
}
