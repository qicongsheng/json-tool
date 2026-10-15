// test.txt（~9.5MB 单行、内嵌 5MB 转义 JSON）格式化链路分阶段计时
// 运行：cargo test --release --test bench_format -- --ignored --nocapture
use json_tool_lib::formatter;

#[test]
#[ignore]
fn bench_format_stages_test_txt() {
    if cfg!(debug_assertions) {
        println!("跳过：bench 须用 --release 运行");
        return;
    }
    let src = match std::fs::read_to_string("../test.txt") {
        Ok(s) => s,
        Err(_) => { println!("跳过：找不到 ../test.txt"); return; }
    };
    println!("src: {} chars", src.chars().count());

    let t0 = std::time::Instant::now();
    let unwrapped = formatter::unwrap_escaped(&src);
    println!("1. unwrap_escaped(全文尝试): {:?}ms", t0.elapsed().as_millis());

    let text = unwrapped.clone().unwrap_or_else(|| src.clone());
    let t1 = std::time::Instant::now();
    let v: serde_json::Value = serde_json::from_str(&text).unwrap();
    println!("2. serde parse: {}ms", t1.elapsed().as_millis());

    let t2 = std::time::Instant::now();
    let out = formatter::format(&src, 2).unwrap();
    println!("3. format 全程(span 保真+嵌套展开): {}ms → {} chars", t2.elapsed().as_millis(), out.chars().count());
    let _ = v;

    let t4 = std::time::Instant::now();
    let doc = json_tool_lib::parser::parse(&src).unwrap();
    println!("5. parser::parse(span 树, 原文): {}ms, nodes={}", t4.elapsed().as_millis(), doc.nodes.len());

    let t5 = std::time::Instant::now();
    let doc2 = json_tool_lib::parser::parse(&out).unwrap();
    println!("6. parser::parse(span 树, 格式化后): {}ms, nodes={}", t5.elapsed().as_millis(), doc2.nodes.len());

    let t6 = std::time::Instant::now();
    let _ = formatter::minify(&src).unwrap();
    println!("7. minify: {}ms", t6.elapsed().as_millis());
}
