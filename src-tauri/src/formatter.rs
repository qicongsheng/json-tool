use crate::parser::{self, Doc, Node, Val};
use serde::Serialize;
use serde_json::Value;

const MAX_DEPTH: usize = 512;

#[derive(Debug)]
pub struct FormatError {
    pub line: usize,
    pub col: usize,
    pub msg: String,
}

pub fn format(src: &str, indent: usize) -> Result<String, FormatError> {
    // span 保真 + 嵌套展开：只重排空白，标量 token（单引号串、None/裸词、
    // 原始数字与转义写法）逐字保留；字符串值内容本身是合法 JSON（对象/数组）
    // 时展开为真实结构（子树内部同样保真，见 write_node）
    if let Ok(doc) = parser::parse(src) {
        return Ok(format_spans(src, &doc, indent));
    }
    // 整篇转义文档（{\"a\": 1} 形态）：还原后再保真重排
    if let Some(text) = unwrap_escaped(src) {
        if let Ok(doc) = parser::parse(&text) {
            return Ok(format_spans(&text, &doc, indent));
        }
    }
    // 罕见：手写 parser 拒绝但 serde 接受（如孤立代理 → U+FFFD）→ 规范化路径；
    // 两者都失败返回 serde 错误（行列与旧实现一致）
    let v = parse_json(src)?;
    pretty(&v, indent)
}

pub fn minify(src: &str) -> Result<String, FormatError> {
    // 原样路径：span 保真压缩——token 逐字保留，仅去空白
    if let Ok(doc) = parser::parse(src) {
        return Ok(minify_spans(src, &doc));
    }
    match parse_json(src) {
        Ok(v) => serde_json::to_string(&v).map_err(|e| FormatError { line: 0, col: 0, msg: e.to_string() }),
        Err(e) => {
            let text = unwrap_escaped(src).unwrap_or_else(|| src.to_string());
            match parse_json(&text) {
                Ok(v) => serde_json::to_string(&v).map_err(|err| FormatError { line: 0, col: 0, msg: err.to_string() }),
                Err(_) => match parse_lenient(&text) {
                    Some(v) => serde_json::to_string(&v).map_err(|err| FormatError { line: 0, col: 0, msg: err.to_string() }),
                    None => Err(e),
                },
            }
        }
    }
}

/// span 保真序列化：容器结构重新生成（缩进/分隔符），标量节点一律输出
/// 原文切片（含单引号串、None 等裸词、原始数字与转义写法）。
/// 键统一以双引号 JSON 字符串重发（键是结构性字符串，语义不变）。
/// expand=true 时，内容为合法对象/数组的字符串值展开为真实结构。
fn write_node(out: &mut String, src: &str, doc: &Doc, id: usize, indent: Option<usize>, level: usize, expand: bool) {
    let n = doc.node(id);
    match n.val {
        Val::Obj | Val::Arr => {
            let is_obj = n.val == Val::Obj;
            if n.children.is_empty() {
                out.push_str(if is_obj { "{}" } else { "[]" });
                return;
            }
            out.push(if is_obj { '{' } else { '[' });
            for (i, &c) in n.children.iter().enumerate() {
                if let Some(w) = indent {
                    out.push('\n');
                    out.push_str(&" ".repeat(w * (level + 1)));
                }
                if is_obj {
                    let key = doc.node(c).key.clone().unwrap_or_default();
                    out.push_str(&serde_json::to_string(&key).unwrap());
                    out.push_str(if indent.is_some() { ": " } else { ":" });
                }
                write_node(out, src, doc, c, indent, level + 1, expand);
                if i + 1 < n.children.len() { out.push(','); }
            }
            if let Some(w) = indent {
                out.push('\n');
                out.push_str(&" ".repeat(w * level));
            }
            out.push(if is_obj { '}' } else { ']' });
        }
        Val::Str if expand => {
            // 内嵌 JSON 字符串：内容（去首尾空白）是合法对象/数组 → 展开为真实
            // 结构，子树内部同样 span 保真 + 递归展开；展开失败则原样输出 token
            if let Some(inner) = expandable_literal(n) {
                if let Ok(sub) = parser::parse(inner) {
                    write_node(out, inner, &sub, sub.root, indent, level, true);
                    return;
                }
            }
            out.push_str(&src[n.start..n.end]);
        }
        _ => out.push_str(&src[n.start..n.end]),
    }
}

/// 字符串节点的解码内容去首尾空白后形如对象/数组（可尝试展开）
fn expandable_literal(n: &Node) -> Option<&str> {
    let t = n.literal.as_deref()?.trim();
    ((t.starts_with('{') && t.ends_with('}')) || (t.starts_with('[') && t.ends_with(']'))).then_some(t)
}

pub fn format_spans(src: &str, doc: &Doc, indent: usize) -> String {
    let mut out = String::with_capacity(src.len() * 2);
    write_node(&mut out, src, doc, doc.root, Some(indent), 0, true);
    out
}

pub fn minify_spans(src: &str, doc: &Doc) -> String {
    let mut out = String::with_capacity(src.len());
    write_node(&mut out, src, doc, doc.root, None, 0, false);
    out
}

fn parse_json(text: &str) -> Result<Value, FormatError> {
    serde_json::from_str(text).map_err(|e| FormatError {
        line: e.line(),
        col: e.column(),
        msg: e.to_string(),
    })
}

/// 宽松解析回退：手写 parser 支持单引号/裸键/尾逗号，成功则重建 Value
/// （输出永远是标准双引号 JSON，键序/数字精度与严格路径一致）
fn parse_lenient(text: &str) -> Option<Value> {
    let doc = parser::parse(text).ok()?;
    doc_to_value(&doc, doc.root)
}

fn doc_to_value(doc: &Doc, id: usize) -> Option<Value> {
    let n: &Node = doc.node(id);
    Some(match n.val {
        Val::Null => Value::Null,
        Val::Bool => match n.literal.as_deref() {
            // 裸词 literal 保留原始写法（True/False 为 Python 风格）
            Some("true" | "True") => Value::Bool(true),
            Some("false" | "False") => Value::Bool(false),
            _ => return None,
        },
        Val::Num => serde_json::from_str(n.literal.as_deref()?).ok()?,
        Val::Str => Value::String(n.literal.clone()?),
        Val::Obj => {
            let mut m = serde_json::Map::new();
            for &c in &n.children {
                let child = doc.node(c);
                m.insert(child.key.clone()?, doc_to_value(doc, c)?);
            }
            Value::Object(m)
        }
        Val::Arr => {
            let mut a = Vec::with_capacity(n.children.len());
            for &c in &n.children {
                a.push(doc_to_value(doc, c)?);
            }
            Value::Array(a)
        }
    })
}

fn pretty(v: &Value, indent: usize) -> Result<String, FormatError> {
    let out = if indent == 4 {
        let fmt = serde_json::ser::PrettyFormatter::with_indent(b"    ");
        let mut buf = Vec::new();
        let mut ser = serde_json::ser::Serializer::with_formatter(&mut buf, fmt);
        v.serialize(&mut ser).map_err(|e| FormatError { line: 0, col: 0, msg: e.to_string() })?;
        String::from_utf8(buf).unwrap()
    } else {
        serde_json::to_string_pretty(v).map_err(|e| FormatError { line: 0, col: 0, msg: e.to_string() })?
    };
    Ok(out)
}

/// 全文本身是"转义过的 JSON"时还原为正常文本；本身合法或还原后仍非法则返回 None
pub fn unwrap_escaped(src: &str) -> Option<String> {
    if serde_json::from_str::<Value>(src).is_ok() { return None; }
    let mut out = String::with_capacity(src.len());
    let mut chars = src.chars();
    while let Some(c) = chars.next() {
        if c != '\\' { out.push(c); continue; }
        match chars.next()? {
            '"' => out.push('"'),
            '\'' => out.push('\''), // 宽松：\' 还原为单引号（与 parser 转义表一致）
            '\\' => out.push('\\'),
            '/' => out.push('/'),
            'n' => out.push('\n'),
            't' => out.push('\t'),
            'r' => out.push('\r'),
            'u' => {
                let hex: String = chars.by_ref().take(4).collect();
                let n = u32::from_str_radix(&hex, 16).ok()?;
                let cp = if (0xD800..0xDC00).contains(&n) {
                    // 高位代理：必须紧跟 \uDC00-\uDFFF 低位代理
                    if chars.next() != Some('\\') { return None; }
                    if chars.next() != Some('u') { return None; }
                    let hex2: String = chars.by_ref().take(4).collect();
                    let lo = u32::from_str_radix(&hex2, 16).ok()?;
                    if !(0xDC00..0xE000).contains(&lo) { return None; }
                    0x10000 + ((n - 0xD800) << 10) + (lo - 0xDC00)
                } else if (0xDC00..0xE000).contains(&n) {
                    return None; // 孤立低位代理
                } else {
                    n
                };
                out.push(char::from_u32(cp).unwrap_or('\u{fffd}'));
            }
            _ => return None,
        }
    }
    if serde_json::from_str::<Value>(&out).is_ok() { Some(out) } else { None }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    #[test]
    fn format_2_and_4_indent() {
        let src = r#"{"a":[1,2]}"#;
        assert_eq!(format(src, 2).unwrap(), "{\n  \"a\": [\n    1,\n    2\n  ]\n}");
        assert_eq!(format(src, 4).unwrap(), "{\n    \"a\": [\n        1,\n        2\n    ]\n}");
    }

    #[test]
    fn minify_ok() {
        assert_eq!(minify("{ \"a\" : [ 1 , 2 ] }").unwrap(), r#"{"a":[1,2]}"#);
    }

    // ---- 宽松解析（单引号 / 裸键 / 尾逗号 / 裸词值）+ span 保真输出 ----

    #[test]
    fn format_lenient_preserves_tokens() {
        // span 保真：只重排空白，标量 token 原样（单引号串保持单引号；
        // 键为结构性字符串，规范化为双引号）；源尾逗号不保留（分隔符重新生成）
        assert_eq!(
            format("{'a': 'x', b: [1, 2,]}", 2).unwrap(),
            "{\n  \"a\": 'x',\n  \"b\": [\n    1,\n    2\n  ]\n}"
        );
        assert_eq!(minify("{'a': 'x', b: [1, 2,]}").unwrap(), r#"{"a":'x',"b":[1,2]}"#);
    }

    #[test]
    fn format_lenient_escapes() {
        // 单引号串 token 原样：内部转义写法不重写
        assert_eq!(minify("{'s': 'it\\'s \"q\"'}").unwrap(), r#"{"s":'it\'s "q"'}"#);
    }

    #[test]
    fn format_bare_values_verbatim() {
        // 裸词值（Python None / 无引号字符串）原样输出，不改写
        let out = format(r#"{"a": None, "b": BILLING_SUCCESS}"#, 2).unwrap();
        assert_eq!(out, "{\n  \"a\": None,\n  \"b\": BILLING_SUCCESS\n}");
        assert_eq!(minify("{'a': None}").unwrap(), r#"{"a":None}"#);
    }

    #[test]
    fn format_preserves_number_and_escape_source_form() {
        // 数字与 \u 转义写法保真（不再经 serde 规范化）；键序保持源顺序
        let out = minify(r#"{"b": 5.80, "a": "订"}"#).unwrap();
        assert_eq!(out, r#"{"b":5.80,"a":"订"}"#);
    }

    #[test]
    fn format_expands_escaped_single_quote_json() {
        // 用户实测：双引号串内 \' 转义的内嵌单引号 JSON → 加载后格式化自动展开。
        // 展开子树同样 span 保真：值 token `'b'` 保持单引号（宽松 JSON，工具可回载）
        let out = format("{\"a\":\"{\\'c\\':\\'b\\'}\"}", 2).unwrap();
        assert_eq!(out, "{\n  \"a\": {\n    \"c\": 'b'\n  }\n}");
        assert!(parser::parse(&out).is_ok()); // 产物可重新载入
    }

    #[test]
    fn format_lenient_with_unescape_nested() {
        // 宽松外壳 + 字符串值内嵌转义 JSON → 嵌套展开
        let out = format("{'a': '{\"b\": 1}'}", 2).unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["a"], json!({"b": 1}));
    }

    #[test]
    fn format_lenient_error_keeps_strict_error() {
        // 严格与宽松都失败 → 仍返回严格路径的错误行列（与旧行为一致）
        let e = format("{\n  'a': \n}", 2).unwrap_err();
        assert_eq!((e.line, e.col), (2, 3));
        // 格式化结果必须是标准 JSON（可被 serde_json 重新解析）
        let out = format("{'a': 1}", 2).unwrap();
        serde_json::from_str::<Value>(&out).unwrap();
    }

    #[test]
    fn expand_nested_json_string() {
        let src = r#"{"a": "{\"b\": [1, \"x\"]}", "c": "plain"}"#;
        let v: Value = serde_json::from_str(&format(src, 2).unwrap()).unwrap();
        assert_eq!(v["a"], json!({"b": [1, "x"]}));
        assert_eq!(v["c"], "plain"); // 非 JSON 字符串不动
    }

    #[test]
    fn unescape_whole_document() {
        let src = r#"{\"a\": \"{\\\"x\\\": 1}\"}"#; // 双重转义的整篇文档
        let out = format(src, 2).unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["a"], json!({"x": 1}));
    }

    #[test]
    fn unescape_surrogate_pairs() {
        // \uXXXX 在"转义文档"层级出现时也必须正确组合代理对
        // 原始字符串：文档层级含 \" 转义（非法 JSON）→ 走 unwrap 路径，😀 命中代理对组合分支
        let out = unwrap_escaped(r#"{\"e\": \"\ud83d\ude00\"}"#).unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["e"], "😀");
        // 孤立高位代理 → 整个 unwrap 失败（返回 None，调用方回退原文，不产生损坏内容）
        assert!(unwrap_escaped(r#"{\"e\": \"\ud83d\"}"#).is_none());
    }

    #[test]
    fn error_line_col() {
        // `bad` 已是合法宽松裸值，用紧随的多余 token 制造严格/宽松双失败
        let e = format("{\n  \"a\": bad x\n}", 2).unwrap_err();
        assert_eq!((e.line, e.col), (2, 8));
    }

    /// 回归：顶层合法 JSON，resp/req 字段是转义 JSON 字符串（内含 \uXXXX 与中文混排），
    /// 「去转义」格式化后必须展开为真实对象，且 \u 转义还原为中文（合成数据与通用 key）
    #[test]
    fn ff_escaped_fields_expand_to_objects() {
        let src = r#"{"retryTimes":0,"resp":"{\"message\": \"ok\", \"code\": 0, \"data\": {\"result\": \"\\u64cd\\u4f5c\\u6210\\u529f\"}}","req":"{\"list\":[{\"name\":\"示例市\",\"code\":100100,\"district\":\"示例区\",\"district_code\":100101,\"province\":\"示例省\",\"province_code\":100000,\"level\":\"2\",\"create\":1584288000000,\"open_date\":null,\"lat\":\"28.0\",\"lon\":\"112.0\",\"id\":\"001\",\"status\":\"Active\",\"addr\":\"示例省示例市示例区示例路1号\",\"email\":\"user@example.com\",\"mobile\":13800000000,\"owner\":\"测试用户\",\"state\":\"已开店\",\"site\":\"9001\",\"extra\":[{\"type\":2,\"url\":\"https://example.com/qr/abc123\"}],\"meta\":{\"bank\":null,\"tax\":null},\"update\":1791775611390}],\"encode\":\"utf8\",\"service\":\"demo-service\",\"event\":\"DEMO_EVENT\"}"}"#;
        let out = format(src, 2).unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["resp"]["data"]["result"], "操作成功");
        assert_eq!(v["req"]["list"][0]["name"], "示例市");
        assert_eq!(v["req"]["list"][0]["addr"], "示例省示例市示例区示例路1号");
        // 非 JSON 字符串字段保持原样
        assert_eq!(v["retryTimes"], 0);
    }
}
