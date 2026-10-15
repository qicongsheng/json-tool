use serde::Serialize;

use crate::parser;

const MAX_CANDIDATES: usize = 50;
const PREVIEW_CHARS: usize = 120;
const MAX_UNESCAPE: usize = 3;
const MAX_SCAN: usize = 10 * 1024 * 1024;

fn skip_ws(b: &[u8], mut i: usize) -> usize {
    while i < b.len() && (b[i] as char).is_ascii_whitespace() {
        i += 1;
    }
    i
}

/// 从 i 起消费一个 JSON 值（字符串/数字/true/false/null/平衡括号结构），返回结束偏移（不含）
/// 字符串感知双引号与宽松单引号（与 parser::parse 的宽松语法一致）
fn consume_value(b: &[u8], i: usize) -> Option<usize> {
    let n = b.len();
    match *b.get(i)? {
        b'"' | b'\'' => {
            let q = b[i];
            let mut j = i + 1;
            while j < n {
                if b[j] == b'\\' { j += 2; continue; }
                if b[j] == q { return Some(j + 1); }
                j += 1;
            }
            None
        }
        b'{' | b'[' => {
            let open = b[i];
            let close = if open == b'{' { b'}' } else { b']' };
            let mut depth = 0usize;
            let mut j = i;
            while j < n {
                if b[j] == b'"' || b[j] == b'\'' { // 跳过字符串
                    let q = b[j];
                    j += 1;
                    while j < n {
                        if b[j] == b'\\' { j += 2; continue; }
                        if b[j] == q { break; }
                        j += 1;
                    }
                } else if b[j] == open {
                    depth += 1;
                } else if b[j] == close {
                    depth -= 1;
                    if depth == 0 { return Some(j + 1); }
                }
                j += 1;
            }
            None
        }
        c if c.is_ascii_digit() || c == b'-' => {
            let mut j = i;
            while j < n && matches!(b[j], b'0'..=b'9' | b'.' | b'e' | b'E' | b'+' | b'-') {
                j += 1;
            }
            Some(j)
        }
        _ => {
            for (word, len) in [(b"true".as_slice(), 4usize), (b"false", 5), (b"null", 4)] {
                if b[i..].starts_with(word) {
                    return Some(i + len);
                }
            }
            None
        }
    }
}

/// 单层转义还原：标准转义序列还原为真实字符；含非法转义/截断的 \u 返回 None。
/// 与 formatter::unwrap_escaped 的字符规则一致，但不要求结果可解析（支持多层迭代）。
fn unescape_once(s: &str) -> Option<String> {
    if !s.contains('\\') {
        return None; // 无转义可还原
    }
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
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
    Some(out)
}

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Detected {
    pub kind: &'static str, // "obj" | "arr"
    pub line: usize,        // 1-based，片段在原文中的起始行
    pub char_len: usize,    // 载入文本字符数
    pub depth: usize,       // 嵌套深度
    pub preview: String,    // 单行预览（还原后）
    pub load_text: String,  // 载入文本（已验证合法）
}

pub fn detect(src: &str) -> Vec<Detected> {
    detect_limited(src, MAX_SCAN)
}

pub fn detect_limited(src: &str, limit: usize) -> Vec<Detected> {
    // 10MB 截断（按 UTF-8 字符边界回退）
    let cut = src.len().min(limit);
    let mut end = cut;
    while end > 0 && !src.is_char_boundary(end) {
        end -= 1;
    }
    let scan_src = &src[..end];

    let (structures, strings) = top_level_spans(scan_src);
    let mut cands: Vec<(usize, Detected)> = Vec::new();

    // struct_spans 供 run 检测防重叠（只含结构 span——顶层字符串 span 是 kv 段的合法组成，
    // 不能进排除表，否则 kv 候选会被误杀）
    let struct_spans: Vec<(usize, usize)> = structures.clone();

    // ① 顶层结构：原文即合法 → 直接候选
    for (start, span_end) in &structures {
        let slice = &scan_src[*start..*span_end];
        let kind = if slice.starts_with('{') { "obj" } else { "arr" };
        if parser::parse(slice).is_ok() {
            if let Some(c) = make_candidate(kind, line_of(src, *start), slice.to_string()) {
                cands.push((*start, c));
            }
        }
    }
    // ② 顶层字符串字面量：内容逐层转义还原（≤3 层），还原出对象/数组即候选
    for (start, span_end) in &strings {
        if span_end - start < 2 {
            continue;
        }
        let inner = &scan_src[start + 1..span_end - 1];
        let mut cur = inner.to_string();
        for _ in 0..MAX_UNESCAPE {
            match unescape_once(&cur) {
                Some(next) => {
                    let t = next.trim();
                    let looks = (t.starts_with('{') && t.ends_with('}'))
                        || (t.starts_with('[') && t.ends_with(']'));
                    if looks {
                        let kind = if t.starts_with('{') { "obj" } else { "arr" };
                        // 解析通过才收候选；形状像但仍有残留转义（双层）→ 继续还原
                        if let Some(c) = make_candidate(kind, line_of(src, *start), t.to_string()) {
                            cands.push((*start, c));
                            break;
                        }
                    }
                    cur = next;
                }
                None => break,
            }
        }
    }
    // ③ 缺括号补全：段起点必须是所在行的第一个非空白字符（防从行中间误起跑），
    //    且起点未被结构占据；跨行消费直到终止符
    let b = scan_src.as_bytes();
    let mut line_start = 0usize;
    while line_start < b.len() {
        let line_end = scan_src[line_start..]
            .find('\n')
            .map(|p| line_start + p)
            .unwrap_or(b.len());
        let first = skip_ws(b, line_start);
        if first < line_end && !struct_spans.iter().any(|&(ts, te)| first >= ts && first < te) {
            if b[first] == b'"' || b[first] == b'\'' {
                // kv 段："k": v, 'k': v …（≥1 对）→ 包 {}。
                // 键只认引号形式（裸键作启发式键会放大日志误报，不参与补全；
                // 裸键结构走 ① 结构扫描 + parse 校验）
                let mut j = first;
                let mut pairs = 0usize;
                loop {
                    j = skip_ws(b, j);
                    let Some(k_end) = consume_value(b, j) else { break };
                    j = skip_ws(b, k_end);
                    if j >= b.len() || b[j] != b':' { break }
                    // 值必须与冒号同行：跨行的残缺值不拼接（防 "a":\n"b":2 误拼成 {"a":"b"}）
                    let colon_line_end = scan_src[j..].find('\n').map(|p| j + p).unwrap_or(b.len());
                    j = skip_ws(b, j + 1);
                    if j >= colon_line_end { break }
                    let Some(v_end) = consume_value(b, j) else { break };
                    pairs += 1;
                    j = v_end;
                    let sep = skip_ws(b, j);
                    if sep < b.len() && b[sep] == b',' { j = sep + 1; } else { break }
                }
                if pairs >= 1 && j > first {
                    if let Some(c) = make_candidate("obj", line_of(src, first),
                                                   format!("{{{}}}", &scan_src[first..j])) {
                        cands.push((first, c));
                    }
                }
            } else {
                // 标量段：v, v, v …（≥2 个）→ 包 []
                let mut j = first;
                let mut items = 0usize;
                loop {
                    j = skip_ws(b, j);
                    let Some(v_end) = consume_value(b, j) else { break };
                    items += 1;
                    j = v_end;
                    let sep = skip_ws(b, j);
                    if sep < b.len() && b[sep] == b',' { j = sep + 1; } else { break }
                }
                if items >= 2 && j > first {
                    if let Some(c) = make_candidate("arr", line_of(src, first),
                                                   format!("[{}]", &scan_src[first..j])) {
                        cands.push((first, c));
                    }
                }
            }
        }
        line_start = line_end + 1;
    }

    // 排序 → 去重叠 → 截断。taken 重建为「已接受候选区间」：
    // 结构 span 已由候选取代、字符串 span 是 kv 段的合法组成，都不参与最终去重
    cands.sort_by_key(|(s, _)| *s);
    let mut out: Vec<Detected> = Vec::new();
    let mut taken: Vec<(usize, usize)> = Vec::new();
    for (s, c) in cands {
        let e = s + c.load_text.len();
        if taken.iter().any(|&(ts, te)| s < te && ts < e) {
            continue;
        }
        taken.push((s, e));
        out.push(c);
        if out.len() >= MAX_CANDIDATES {
            break;
        }
    }
    out
}

fn line_of(src: &str, off: usize) -> usize {
    src[..off].bytes().filter(|&b| b == b'\n').count() + 1
}

fn preview_of(text: &str) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut out: String = flat.chars().take(PREVIEW_CHARS).collect();
    if flat.chars().count() > PREVIEW_CHARS {
        out.push('…');
    }
    out
}

/// 由载入文本构造候选：非法（宽松）JSON 返回 None（保证点击 100% 可载入）；
/// 与 load_text 共用 parser::parse，宽松语法（单引号/裸键/尾逗号）一致可行
fn make_candidate(kind: &'static str, line: usize, load_text: String) -> Option<Detected> {
    let doc = parser::parse(&load_text).ok()?;
    Some(Detected {
        kind,
        line,
        char_len: load_text.chars().count(),
        depth: doc_depth(&doc),
        preview: preview_of(&load_text),
        load_text,
    })
}

/// Doc 树最大嵌套深度（与原 depth_of(&Value) 语义一致：标量 0，容器每层 +1）
fn doc_depth(doc: &parser::Doc) -> usize {
    fn walk(doc: &parser::Doc, id: usize, d: usize) -> usize {
        let node = doc.node(id);
        let mut max = d;
        for &c in &node.children {
            max = max.max(walk(doc, c, d + 1));
        }
        max
    }
    walk(doc, doc.root, 0)
}

/// 字符串感知的顶层 span 扫描：返回 (顶层 {…}/[…] 结构, 顶层字符串字面量)。
/// 字符串内的括号/引号不算结构（`"a{b}"` 不产生结构候选）。
/// 宽松：单引号字符串与双引号同权（与 parser::parse 一致）
fn top_level_spans(src: &str) -> (Vec<(usize, usize)>, Vec<(usize, usize)>) {
    let b = src.as_bytes();
    let (mut structures, mut strings) = (Vec::new(), Vec::new());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'"' | b'\'' => {
                let q = b[i];
                let start = i;
                i += 1;
                while i < b.len() {
                    if b[i] == b'\\' { i += 2; continue; }
                    if b[i] == q { i += 1; break; }
                    i += 1;
                }
                strings.push((start, i.min(b.len())));
            }
            b'{' | b'[' => {
                let open = b[i];
                let close = if open == b'{' { b'}' } else { b']' };
                let start = i;
                let mut depth = 0usize;
                while i < b.len() {
                    if b[i] == b'"' || b[i] == b'\'' { // 跳过字符串字面量（双/单引号）
                        let q = b[i];
                        i += 1;
                        while i < b.len() {
                            if b[i] == b'\\' { i += 2; continue; }
                            if b[i] == q { break; } // 不消费闭合引号，由外层 i += 1 统一消费
                            i += 1;
                        }
                    } else if b[i] == open {
                        depth += 1;
                    } else if b[i] == close {
                        depth -= 1;
                        if depth == 0 { i += 1; break; }
                    }
                    i += 1;
                }
                structures.push((start, i.min(b.len())));
            }
            _ => i += 1,
        }
    }
    (structures, strings)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mixed_log_finds_top_level_structures() {
        let src = "2026-10-13 INFO start\nconfig={\"a\":1,\"nested\":{\"b\":2}}\n…done {\"ok\":true}\n";
        let cands = detect(src);
        assert_eq!(cands.len(), 2);
        assert_eq!(cands[0].kind, "obj");
        assert_eq!(cands[0].line, 2);
        assert_eq!(cands[0].depth, 2); // {"a":1,"nested":{"b":2}} 嵌套 2 层
        assert!(cands[0].preview.contains("\"a\""));
        assert_eq!(cands[1].line, 3);
        assert_eq!(cands[1].load_text, "{\"ok\":true}");
    }

    #[test]
    fn array_structure_detected() {
        let cands = detect("data=[1,2,3,{\"x\":null}]\n");
        assert_eq!(cands.len(), 1);
        assert_eq!(cands[0].kind, "arr");
        assert_eq!(cands[0].load_text, "[1,2,3,{\"x\":null}]");
    }

    #[test]
    fn nested_json_not_listed_separately() {
        // 内层 {"b":2} 不单列（决策：只报顶层）
        let cands = detect("x={\"a\":{\"b\":2}}\n");
        assert_eq!(cands.len(), 1);
        assert_eq!(cands[0].load_text, "{\"a\":{\"b\":2}}");
    }

    #[test]
    fn fake_braces_inside_string_ignored() {
        let cands = detect("note=\"a{b} and \\\"quote\\\" here\"\n");
        assert!(cands.is_empty());
    }

    #[test]
    fn invalid_structure_dropped() {
        let cands = detect("broken={oops,not json}\n");
        assert!(cands.is_empty());
    }

    // ---- 宽松语法（单引号 / 裸键 / 尾逗号）----

    #[test]
    fn lenient_single_quote_structure_detected() {
        let cands = detect("cfg={'a': 'x',}\n");
        assert_eq!(cands.len(), 1);
        assert_eq!(cands[0].kind, "obj");
        assert_eq!(cands[0].load_text, "{'a': 'x',}");
        assert_eq!(cands[0].depth, 1);
    }

    #[test]
    fn lenient_bare_keys_structure_detected() {
        let cands = detect("{a: 1, b: [2,]}");
        assert_eq!(cands.len(), 1);
        assert_eq!(cands[0].depth, 2);
    }

    #[test]
    fn single_quote_kv_segment_completes() {
        // 缺外层花括号的 kv 段：单引号键与双引号同权
        let cands = detect("'k': 'v', 'k2': 2\n");
        assert_eq!(cands.len(), 1);
        assert_eq!(cands[0].load_text, "{'k': 'v', 'k2': 2}");
    }

    #[test]
    fn braces_inside_single_quote_string_not_structure() {
        let cands = detect("note='a{b}' rest\n");
        // 单引号串内 {b} 不构成结构；kv 补全需引号键，`note=` 裸词不触发
        assert!(cands.is_empty());
    }

    #[test]
    fn line_numbers_are_1_based_multiline() {
        let src = "line1\nline2\nline3 {\"n\":1}\n";
        assert_eq!(detect(src)[0].line, 3);
    }

    #[test]
    fn escaped_json_string_detected() {
        // 日志内嵌转义 JSON："{\"a\":1,\"b\":[1,2]}"
        let src = "2026-10-13 INFO msg=\"{\\\"a\\\":1,\\\"b\\\":[1,2]}\"\n";
        let cands = detect(src);
        assert_eq!(cands.len(), 1);
        assert_eq!(cands[0].kind, "obj");
        assert_eq!(cands[0].load_text, "{\"a\":1,\"b\":[1,2]}");
        assert!(cands[0].preview.contains("\"b\""));
    }

    #[test]
    fn double_escaped_json_detected() {
        // 双层转义："{\\\"a\\\":1}" → {"a":1}
        let src = "payload=\"{\\\\\\\"a\\\\\\\":1}\"\n";
        let cands = detect(src);
        assert_eq!(cands.len(), 1);
        assert_eq!(cands[0].load_text, "{\"a\":1}");
    }

    #[test]
    fn escaped_non_json_string_ignored() {
        let cands = detect("msg=\"just plain text\"\n");
        assert!(cands.is_empty());
    }

    #[test]
    fn bare_kv_pairs_wrapped_into_object() {
        // 行首键值对段（无外层大括号）→ 包 {} 载入
        let src = "record:\n\"name\": \"x\", \"age\": 3\nfooter\n";
        let cands = detect(src);
        assert_eq!(cands.len(), 1);
        assert_eq!(cands[0].kind, "obj");
        assert_eq!(cands[0].load_text, "{\"name\": \"x\", \"age\": 3}");
        assert_eq!(cands[0].line, 2);
    }

    #[test]
    fn bare_scalar_run_wrapped_into_array() {
        let src = "values:\n1, 2, 3\nend\n";
        let cands = detect(src);
        assert_eq!(cands.len(), 1);
        assert_eq!(cands[0].kind, "arr");
        assert_eq!(cands[0].load_text, "[1, 2, 3]");
    }

    #[test]
    fn single_scalar_not_a_candidate() {
        // 单个标量不成段（≥2 才候选），且 "x = 1, y = 2" 不从行中间起跑
        assert!(detect("just 42\n").is_empty());
        assert!(detect("x = 1, y = 2\n").is_empty());
    }

    #[test]
    fn invalid_kv_run_dropped() {
        // 第 1 行值残缺（跨行不拼接）→ 无候选；第 2 行是完整 kv 对 → 正常候选
        let cands = detect("\"a\": \n\"b\": 2\n");
        assert_eq!(cands.len(), 1);
        assert_eq!(cands[0].load_text, "{\"b\": 2}");
        assert_eq!(cands[0].line, 2);
    }

    #[test]
    fn candidate_cap_50() {
        let mut src = String::new();
        for i in 0..60 {
            src.push_str(&format!("{{\"i\":{i}}}\n"));
        }
        let cands = detect(&src);
        assert_eq!(cands.len(), 50);
        assert_eq!(cands[49].load_text, "{\"i\":49}");
    }

    #[test]
    fn scan_truncated_at_limit() {
        // 公开入口固定 10MB；内部 detect_limited 可注入小 limit 便于测试
        let mut src = String::new();
        for i in 0..40 {
            src.push_str(&format!("{{\"i\":{i}}}\n"));
        }
        // limit 恰好切在第 21 个候选之前（排除 {"i":20}）
        let cut = src.find("{\"i\":20}").unwrap();
        let cands = detect_limited(&src, cut);
        assert_eq!(cands.len(), 20);
    }

    #[test]
    fn string_before_closing_brace_not_swallowed() {
        // 结构内字符串的下一个字符是 } 时不能被误吞（曾在字符串跳过后多消费 1 字符，
        // 导致 {"a":"X"} 闭合符丢失、span 吞到文尾、parse 失败无候选）
        let cands = detect("log {\"a\":\"X\"} tail\n");
        assert_eq!(cands.len(), 1);
        assert_eq!(cands[0].load_text, "{\"a\":\"X\"}");
    }

    #[test]
    fn log_json_with_quoted_value_before_close_detected() {
        // 日志内嵌 JSON：字符串值后紧跟闭合符（曾因跳字符串多吞 1 字符而漏探测）
        let src = "##LOG## pushContent{\"data\":{\"k\":\"EVENT_DEMO\"}}, resp={\"message\": \"ok\"}\n";
        let cands = detect(src);
        assert_eq!(cands.len(), 2);
        assert_eq!(cands[0].load_text, "{\"data\":{\"k\":\"EVENT_DEMO\"}}");
        assert_eq!(cands[1].load_text, "{\"message\": \"ok\"}");
    }
}


