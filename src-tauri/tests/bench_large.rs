// 生成 ~50MB JSON：10 万行 x 20 字段；断言解析与过滤在 2s 内
// 性能目标按 release 构建定义（debug 下解析器慢 ~4 倍，数字无意义）
// 运行：cargo test --release --test bench_large -- --ignored --nocapture
use json_tool_lib::{parser, table};

#[test]
#[ignore]
fn bench_50mb_parse_and_filter() {
    if cfg!(debug_assertions) {
        println!("跳过：bench 须用 --release 运行（cargo test --release --test bench_large -- --ignored --nocapture）");
        return;
    }
    let mut src = String::with_capacity(60_000_000);
    src.push_str(r#"{"users":["#);
    for i in 0..100_000 {
        if i > 0 { src.push(','); }
        src.push_str(&format!(
            r#"{{"id":{i},"name":"user{i}","age":{},"email":"u{i}@x.com","city":"c{}","tags":["t{}"],"profile":{{"level":{},"active":true}}"#,
            i % 80, i % 7, i % 5, i % 10));
        // 补足 20 字段 / ~50MB（brief 注释要求 10 万行 x 20 字段、总量 >40MB）
        for j in 0..13 {
            src.push_str(&format!(r#","f{j}":"pad{i}-{j}-0123456789""#));
        }
        src.push('}');
    }
    src.push_str("]}");
    assert!(src.len() > 40_000_000, "generated JSON too small: {} bytes", src.len());

    let t0 = std::time::Instant::now();
    let doc = parser::parse(&src).unwrap();
    let parse_ms = t0.elapsed().as_millis();
    println!("parse: {parse_ms}ms");

    let users = doc.node(doc.root).children[0]; // "users" 数组节点
    let mut next = 0u64;
    let t = table::Table::create(&doc, users, &mut next);
    let t1 = std::time::Instant::now();
    let hit = t.filter(&doc, &src, &["user12345".to_string(), "user99999".to_string()], None);
    let filter_ms = t1.elapsed().as_millis();
    println!("filter: {filter_ms}ms, hit: {hit:?}");
    assert_eq!(hit, vec![12345, 99999]);
    assert!(parse_ms < 2000, "parse too slow: {parse_ms}ms");
    assert!(filter_ms < 2000, "filter too slow: {filter_ms}ms");
}
