//! 代码节点的编辑辅助：语法高亮、补全、签名提示、缩进。都是纯文本分析，不执行代码。
//! polars 的 API 清单（名字 / 签名 / 文档首行）向引擎要一次，取的是本机装的那版 polars。

use serde_json::Value;
use std::collections::HashMap;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

struct Entry {
    name: String,
    sig: String,
    doc: String,
}

static CATALOG: OnceLock<HashMap<String, Vec<Entry>>> = OnceLock::new();
static LOADING: AtomicBool = AtomicBool::new(false);

/// 后台向引擎要 API 清单（只要一次）。引擎起不来时下次再试；没有清单时只补全列名、变量和关键字。
pub fn load_catalog() {
    if CATALOG.get().is_some() || LOADING.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(|| match crate::engine::catalog().map(|v| parse_catalog(&v)) {
        Some(map) if !map.is_empty() => {
            let _ = CATALOG.set(map);
        }
        _ => LOADING.store(false, Ordering::SeqCst),
    });
}

fn parse_catalog(value: &Value) -> HashMap<String, Vec<Entry>> {
    let Some(groups) = value.as_object() else { return HashMap::new() };
    let text = |v: &Value, i: usize| v[i].as_str().unwrap_or("").to_string();
    groups
        .iter()
        .map(|(group, members)| {
            let entries = members.as_array().map(Vec::as_slice).unwrap_or_default();
            let entries = entries.iter().map(|m| Entry { name: text(m, 0), sig: text(m, 1), doc: text(m, 2) });
            (group.clone(), entries.collect())
        })
        .collect()
}

fn group(name: &str) -> &'static [Entry] {
    CATALOG.get().and_then(|c| c.get(name)).map_or(&[], Vec::as_slice)
}

// ---------------------------------------------------------------- 分词

/// 高亮的类别；数值就是颜色层的下标。
#[derive(Clone, Copy, PartialEq, Debug)]
enum Tok {
    Keyword = 1,
    Str = 2,
    Comment = 3,
    Number = 4,
    Call = 5,
}

/// 高亮层数：0 是普通文本，其余对应 `Tok`。
pub const LAYERS: usize = 6;

struct Span {
    start: usize,
    end: usize,
    kind: Tok,
    /// 字符串没有闭合（光标正在里面输入）
    open: bool,
    /// 字符串开头引号的长度
    quote: usize,
}

const PY_KEYWORDS: &[&str] = &[
    "False", "None", "True", "and", "as", "assert", "break", "class", "continue", "def", "del", "elif", "else",
    "except", "finally", "for", "from", "global", "if", "import", "in", "is", "lambda", "not", "or", "pass", "raise",
    "return", "try", "while", "with", "yield",
];

const SQL_KEYWORDS: &[&str] = &[
    "SELECT", "FROM", "WHERE", "GROUP", "BY", "ORDER", "HAVING", "LIMIT", "OFFSET", "AS", "AND", "OR", "NOT", "IN",
    "IS", "NULL", "LIKE", "BETWEEN", "CASE", "WHEN", "THEN", "ELSE", "END", "JOIN", "LEFT", "RIGHT", "INNER", "OUTER",
    "FULL", "CROSS", "ON", "UNION", "ALL", "DISTINCT", "ASC", "DESC", "WITH", "OVER", "PARTITION", "TRUE", "FALSE",
];

fn is_ident(c: char) -> bool {
    c == '_' || c.is_alphanumeric()
}

/// s 末尾的标识符有多少字节。
fn ident_len_back(s: &str) -> usize {
    s.chars().rev().take_while(|&c| is_ident(c)).map(char::len_utf8).sum()
}

fn tokenize(text: &str, sql: bool) -> Vec<Span> {
    let b = text.as_bytes();
    let mut spans = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        let rest = &text[i..];
        if (!sql && c == b'#') || (sql && rest.starts_with("--")) {
            let end = rest.find('\n').map_or(b.len(), |n| i + n);
            spans.push(Span { start: i, end, kind: Tok::Comment, open: false, quote: 0 });
            i = end;
        } else if c == b'"' || c == b'\'' {
            let triple = !sql && b.get(i + 1) == Some(&c) && b.get(i + 2) == Some(&c);
            let quote = if triple { 3 } else { 1 };
            let mut j = i + quote;
            let mut closed = false;
            while j < b.len() {
                if b[j] == b'\\' && !sql {
                    j += 2;
                } else if b[j] == b'\n' && !triple {
                    break;
                } else if b[j] == c && (!triple || (b.get(j + 1) == Some(&c) && b.get(j + 2) == Some(&c))) {
                    j += quote;
                    closed = true;
                    break;
                } else {
                    j += 1;
                }
            }
            let end = j.min(b.len());
            spans.push(Span { start: i, end, kind: Tok::Str, open: !closed, quote });
            i = end;
        } else if c.is_ascii_digit() {
            let len = b[i..].iter().take_while(|&&d| d.is_ascii_alphanumeric() || d == b'.' || d == b'_').count();
            spans.push(Span { start: i, end: i + len, kind: Tok::Number, open: false, quote: 0 });
            i += len;
        } else if c >= 0x80 || c == b'_' || c.is_ascii_alphabetic() {
            let len: usize = rest.chars().take_while(|&ch| is_ident(ch)).map(char::len_utf8).sum();
            if len == 0 {
                i += rest.chars().next().map_or(1, char::len_utf8);
                continue;
            }
            let word = &rest[..len];
            let keyword = if sql {
                SQL_KEYWORDS.contains(&word.to_ascii_uppercase().as_str())
            } else {
                PY_KEYWORDS.contains(&word)
            };
            let kind = if keyword {
                Some(Tok::Keyword)
            } else if rest[len..].trim_start_matches(' ').starts_with('(') {
                Some(Tok::Call)
            } else {
                None
            };
            if let Some(kind) = kind {
                spans.push(Span { start: i, end: i + len, kind, open: false, quote: 0 });
            }
            i += len;
        } else {
            i += 1;
        }
    }
    spans
}

/// 高亮层：每层是整段文本，但只保留属于这一类的字符，其余换成等宽的空白。
/// 各层叠在一起、各用一种颜色画，字符位置和输入框里的完全对齐（等宽字体）。
pub fn layers(text: &str, sql: bool) -> Vec<String> {
    let mut class = vec![0u8; text.len()];
    for span in tokenize(text, sql) {
        class[span.start..span.end].fill(span.kind as u8);
    }
    (0..LAYERS as u8)
        .map(|layer| {
            text.char_indices()
                .map(|(i, ch)| match ch {
                    // 字体里没有制表符的字形（画成方框）：画成空格，等宽字体里一样占一格（粘贴的 Excel 表格是制表符分隔的）
                    '\t' => ' ',
                    _ if class[i] == layer => ch,
                    '\n' | '\r' => ch,
                    _ if crate::is_wide(ch) => '\u{3000}',
                    _ => ' ',
                })
                .collect()
        })
        .collect()
}

// ---------------------------------------------------------------- 补全

pub struct Item {
    /// 名字，也是选中后插入的文本
    pub label: String,
    /// 签名或种类
    pub detail: String,
    pub doc: String,
}

/// 补全结果：选中后用 label 替换 text[start..光标]。
pub struct Found {
    pub start: usize,
    pub items: Vec<Item>,
}

const NAMESPACES: &[&str] = &["str", "dt", "list", "arr", "cat", "struct", "map", "name", "bin"];

/// 参数是文件路径的函数：代码节点里它们的字符串参数补全文件路径。
const PATH_FNS: &[&str] = &[
    "open", "read_csv", "scan_csv", "read_excel", "read_parquet", "scan_parquet", "read_ipc", "scan_ipc",
    "read_ipc_stream", "read_json", "scan_ndjson", "read_ndjson", "read_avro", "scan_lines", "read_delta",
    "sink_parquet", "sink_csv", "sink_ipc", "sink_ndjson", "write_csv", "write_excel", "write_parquet",
    "write_json", "write_ipc", "write_avro",
];

/// 常用的排前面（其余按字母序）。
const COMMON: &[&str] = &[
    "col", "filter", "select", "with_columns", "group_by", "agg", "sort", "join", "alias", "sum", "mean", "len",
    "count", "cast", "when", "then", "otherwise", "lit", "over", "is_in", "is_null", "is_not_null", "fill_null",
    "unique", "head", "rename", "drop", "concat", "min", "max", "median", "n_unique", "first", "last", "round", "str",
    "dt", "contains", "replace", "starts_with", "ends_with", "to_date", "strftime", "year", "month", "collect", "lazy",
    "pivot", "unpivot", "explode", "len_chars", "strip_chars", "to_uppercase", "slice", "shift", "cum_sum", "rank",
    "is_between", "is_duplicated", "drop_nulls", "Int64", "Float64", "String", "Date", "Datetime", "Boolean", "all",
    "any", "exclude", "sum_horizontal", "concat_str", "coalesce", "read_excel", "read_csv", "scan_csv", "scan_parquet",
];

/// 把下标夹到文本范围内并退到字符边界上（界面传来的是字节偏移）。
fn clamp(text: &str, mut i: usize) -> usize {
    i = i.min(text.len());
    while !text.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// b[close] 是闭括号：返回配对的开括号位置。
fn open_bracket(b: &[u8], close: usize) -> Option<usize> {
    let mut depth = 0usize;
    for i in (0..=close).rev() {
        match b[i] {
            b')' | b']' | b'}' => depth += 1,
            b'(' | b'[' | b'{' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

/// 方法链最左边的名字：`df.filter(x).sort("a")` → df。链从括号表达式开始时为 None。
fn chain_root(recv: &str) -> Option<&str> {
    let b = recv.as_bytes();
    let mut i = b.len();
    let mut root = None;
    loop {
        while i > 0 && matches!(b[i - 1], b')' | b']') {
            i = open_bracket(b, i - 1)?;
        }
        let len = ident_len_back(&recv[..i]);
        if len == 0 {
            return root;
        }
        i -= len;
        root = Some(&recv[i..i + len]);
        // 链可以换行写：点号前面的空白跳过
        let head = recv[..i].trim_end();
        if !head.ends_with('.') {
            return root;
        }
        i = head[..head.len() - 1].trim_end().len();
    }
}

/// recv 以一次调用结尾时，被调用的名字：`df.group_by("a")` → group_by。
fn last_call(recv: &str) -> &str {
    let b = recv.as_bytes();
    if b.last() != Some(&b')') {
        return "";
    }
    let Some(open) = open_bracket(b, b.len() - 1) else { return "" };
    let head = &recv[..open];
    &head[head.len() - ident_len_back(head)..]
}

/// 点号前面是什么：决定补全哪一组的成员。recv 不含那个点号。
fn receiver_groups(recv: &str) -> Vec<&'static str> {
    let recv = recv.trim_end();
    let word = &recv[recv.len() - ident_len_back(recv)..];
    let head = recv[..recv.len() - word.len()].trim_end();
    if !word.is_empty() {
        if word.chars().all(|c| c.is_ascii_digit()) {
            return Vec::new(); // 小数点
        }
        if head.ends_with('.') {
            if let Some(ns) = NAMESPACES.iter().find(|&&ns| ns == word) {
                return vec![ns];
            }
        } else if word == "pl" {
            return vec!["pl"];
        }
    }
    let call = last_call(recv);
    if matches!(call, "group_by" | "group_by_dynamic" | "rolling") {
        return vec!["GroupBy"];
    }
    let makes_frame = ["scan_", "read_", "from_", "concat", "LazyFrame", "DataFrame"].iter().any(|p| call.starts_with(p));
    match chain_root(recv) {
        // pl.col(...) 之类：表达式
        Some("pl") if !makes_frame => vec!["Expr"],
        None => vec!["Expr"],
        Some(_) if matches!(call, "collect" | "pivot") => vec!["DataFrame", "LazyFrame"],
        Some(_) => vec!["LazyFrame", "DataFrame"],
    }
}

/// 按前缀筛选、排序候选（label, detail, doc）；全部留下，界面上滚动着看。
/// keep_order：候选本身的顺序有意义（列名、变量），否则常用的在前、其余按长度和字母序。
fn rank(candidates: Vec<(String, String, String)>, prefix: &str, keep_order: bool) -> Vec<Item> {
    let p = prefix.to_lowercase();
    // 只打了一个字母时不做“包含”匹配，太吵
    let loose = !p.is_ascii() || p.len() >= 2;
    let mut scored: Vec<_> = candidates
        .into_iter()
        .enumerate()
        .filter_map(|(index, (label, detail, doc))| {
            let lower = label.to_lowercase();
            let tier = if lower.starts_with(&p) {
                0
            } else if loose && lower.contains(&p) {
                1
            } else {
                return None;
            };
            let key = if keep_order {
                (tier, index, 0, String::new())
            } else {
                let common = COMMON.iter().position(|&c| c == label).unwrap_or(usize::MAX);
                (tier, common, if p.is_empty() { 0 } else { label.len() }, lower)
            };
            Some((key, Item { label, detail, doc }))
        })
        .collect();
    scored.sort_by(|a, b| a.0.cmp(&b.0));
    scored.dedup_by(|a, b| a.1.label == b.1.label);
    let items: Vec<Item> = scored.into_iter().map(|(_, item)| item).collect();
    // 已经打完整了就不再弹
    if items.len() == 1 && items[0].label == prefix { Vec::new() } else { items }
}

/// 头部以一次调用结尾时被调的名字：`pl.read_csv(` → read_csv，不是调用则空。
fn callee_of(head: &str) -> &str {
    let head = head.trim_end();
    if !head.ends_with('(') {
        return "";
    }
    let h = head[..head.len() - 1].trim_end();
    &h[h.len() - ident_len_back(h)..]
}

/// dir 下和 prefix 开头匹配的条目：文件夹在前、按名字排，最多 100 个。文件夹的名字带分隔符（选中后接着往里列）。
fn path_items(dir: &str, prefix: &str) -> Vec<Item> {
    let lower = prefix.to_lowercase();
    let sep = if dir.contains('/') { '/' } else { '\\' };
    let mut dirs: Vec<Item> = Vec::new();
    let mut files: Vec<Item> = Vec::new();
    let Ok(read) = std::fs::read_dir(dir) else { return Vec::new() };
    for entry in read.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.to_lowercase().starts_with(&lower) {
            continue;
        }
        if entry.file_type().is_ok_and(|t| t.is_dir()) {
            dirs.push(Item { label: format!("{name}{sep}"), detail: String::from("文件夹"), doc: String::new() });
        } else {
            files.push(Item { label: name, detail: String::from("文件"), doc: String::new() });
        }
        if dirs.len() + files.len() >= 200 {
            break;
        }
    }
    let by_name = |a: &Item, b: &Item| a.label.to_lowercase().cmp(&b.label.to_lowercase());
    dirs.sort_by(by_name);
    files.sort_by(by_name);
    let mut items = dirs;
    items.extend(files);
    items.truncate(100);
    items
}

/// 光标所在的未闭合调用：(开括号位置, 被调的名字)。
fn enclosing_call(text: &str, cursor: usize) -> Option<(usize, &str)> {
    let b = text.as_bytes();
    let mut depth = 0usize;
    let mut open = None;
    for i in (cursor.saturating_sub(4000)..cursor).rev() {
        match b[i] {
            b')' | b']' | b'}' => depth += 1,
            b'(' | b'[' | b'{' if depth > 0 => depth -= 1,
            b'(' => {
                open = Some(i);
                break;
            }
            b'[' | b'{' => break,
            _ => {}
        }
    }
    let open = open?;
    let head = &text[..open];
    let name = &head[head.len() - ident_len_back(head)..];
    Some((open, name))
}

/// 调用里已经写了的关键字参数（depth 0 的 名字=；== / >= / != 不算）。
fn used_kwargs(call: &str) -> Vec<String> {
    let b = call.as_bytes();
    let mut used = Vec::new();
    let mut depth = 0i32;
    for (i, &c) in b.iter().enumerate() {
        match c {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth -= 1,
            b'=' if depth == 0 && b.get(i + 1) != Some(&b'=') => {
                let mut j = i;
                while j > 0 && b[j - 1].is_ascii_whitespace() {
                    j -= 1;
                }
                let len = ident_len_back(&call[..j]);
                if len > 0 {
                    used.push(call[j - len..j].to_string());
                }
            }
            _ => {}
        }
    }
    used
}

fn push_kwarg(part: &str, out: &mut Vec<(String, String)>) {
    let p = part.trim();
    if p.is_empty() || p == "self" || p == "*" || p == "/" || p.starts_with('*') {
        return;
    }
    let (name, default) = match p.split_once('=') {
        Some((n, d)) => (n.trim(), d.trim()),
        None => (p, ""),
    };
    if name.is_empty() || !name.chars().next().is_some_and(|c| is_ident(c)) {
        return;
    }
    let detail = if default.is_empty() {
        String::from("必填")
    } else {
        let mut d: String = default.chars().take(40).collect();
        if default.chars().count() > 40 {
            d.push('…');
        }
        d
    };
    out.push((name.to_string(), detail));
}

/// 签名里的参数：`(source, *, encoding='utf8')` → [("source", "必填"), ("encoding", "'utf8'")]。
/// self、单独的 * 和 /（位置标记）、*args / **kwargs 不算。
fn kwargs_from_sig(sig: &str) -> Vec<(String, String)> {
    let Some(inner) = sig.strip_prefix('(').and_then(|s| s.strip_suffix(')')) else { return Vec::new() };
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut part = String::new();
    for c in inner.chars() {
        match c {
            '(' | '[' | '{' => {
                depth += 1;
                part.push(c);
            }
            ')' | ']' | '}' => {
                depth -= 1;
                part.push(c);
            }
            ',' if depth == 0 => {
                push_kwarg(&part, &mut out);
                part.clear();
            }
            _ => part.push(c),
        }
    }
    push_kwarg(&part, &mut out);
    out
}

/// 光标处这次调用的关键字参数（去掉已经填了的），从清单里的签名拆出来。
fn kwarg_items(text: &str, open: usize, name: &str, call: &str) -> Vec<Item> {
    if name.is_empty() {
        return Vec::new();
    }
    let head = &text[..open - name.len()];
    let all = ["Expr", "LazyFrame", "DataFrame", "GroupBy", "pl"]
        .into_iter()
        .chain(NAMESPACES.iter().copied());
    let groups: Vec<&'static str> = match head.strip_suffix('.') {
        Some(recv) => receiver_groups(recv).into_iter().chain(all).collect(),
        None => all.collect(),
    };
    let used = used_kwargs(call);
    for g in groups {
        for e in group(g) {
            if e.name == name && !e.sig.is_empty() && e.sig != "(...)" {
                return kwargs_from_sig(&e.sig)
                    .into_iter()
                    .filter(|(n, _)| !used.iter().any(|u| u == n))
                    .map(|(n, d)| Item { label: format!("{n}="), detail: d, doc: String::new() })
                    .collect();
            }
        }
    }
    Vec::new()
}

/// 光标处的补全。columns 是输入表的列名、params 是这个节点定义的参数（都在引号里时补全）；
/// fallback 是路径补全没有写目录时用的目录（当前打开的文件所在的）。
pub fn complete(text: &str, cursor: usize, columns: &[String], params: &[Item], sql: bool, fallback: &str) -> Found {
    let cursor = clamp(text, cursor);
    let before = &text[..cursor];
    let none = Found { start: cursor, items: Vec::new() };
    let column_items = |quoted: bool| -> Vec<(String, String, String)> {
        columns
            .iter()
            .map(|c| {
                let plain = !quoted || (c.chars().all(is_ident) && !c.starts_with(|ch: char| ch.is_ascii_digit()));
                let label = if plain { c.clone() } else { format!("\"{}\"", c.replace('"', "\"\"")) };
                (label, String::from("列"), String::new())
            })
            .collect()
    };

    if let Some(last) = tokenize(before, sql).last().filter(|s| s.end == cursor) {
        match last.kind {
            Tok::Str if last.open => {
                // 引号里：ch 的参数名、路径函数的文件路径，其余补列名。SQL 的单引号是字符串值，不补
                if sql && before[last.start..].starts_with('\'') {
                    return none;
                }
                let start = last.start + last.quote;
                let inside = &before[start..];
                match callee_of(&before[..last.start]) {
                    "ch" => {
                        let items = params.iter().map(|p| (p.label.clone(), p.detail.clone(), p.doc.clone())).collect();
                        return Found { start, items: rank(items, inside, true) };
                    }
                    name if PATH_FNS.contains(&name) => {
                        let sep = inside.rfind(['/', '\\']).map_or(0, |i| i + 1);
                        let (dir, prefix) = inside.split_at(sep);
                        let dir = if dir.is_empty() { fallback } else { dir };
                        return Found { start: start + sep, items: path_items(dir, prefix) };
                    }
                    _ => {}
                }
                return Found { start, items: rank(column_items(false), &before[start..], true) };
            }
            Tok::Str | Tok::Comment | Tok::Number => return none,
            _ => {}
        }
    }

    let start = cursor - ident_len_back(before);
    let prefix = &before[start..];
    let head = &before[..start];
    // 调用的括号里：补这个函数的参数名。空前缀只在括号 / 逗号后面弹（= 后面是值，不弹）；
    // 打了前缀就按前缀筛，一个都没筛中时退回普通补全
    if !sql && !head.ends_with('.') && !prefix.starts_with(|c: char| c.is_ascii_digit()) {
        if let Some((open, callee)) = enclosing_call(before, cursor) {
            let after_arg =
                prefix.is_empty() && { let prev = head.trim_end(); prev.ends_with('(') || prev.ends_with(',') };
            if after_arg || !prefix.is_empty() {
                let items = kwarg_items(before, open, callee, &before[open + 1..]);
                // 刚打完 ( ：全是必填位置参数的（pl.col(name) 这类）不弹，等打了前缀再说
                let auto = !items.iter().all(|i| i.detail == "必填");
                if !items.is_empty() && (auto || !prefix.is_empty()) {
                    if prefix.is_empty() {
                        return Found { start: cursor, items };
                    }
                    let found = rank(items.into_iter().map(|i| (i.label, i.detail, i.doc)).collect(), prefix, true);
                    if !found.is_empty() {
                        return Found { start, items: found };
                    }
                }
            }
        }
    }
    let candidates: Vec<(String, String, String)> = if let Some(recv) = head.strip_suffix('.') {
        if sql {
            return none;
        }
        receiver_groups(recv)
            .into_iter()
            .flat_map(group)
            .map(|e| (e.name.clone(), e.sig.clone(), e.doc.clone()))
            .collect()
    } else if prefix.is_empty() || prefix.starts_with(|c: char| c.is_ascii_digit()) {
        return none;
    } else if sql {
        let keywords = SQL_KEYWORDS.iter().map(|k| (k.to_string(), String::from("关键字"), String::new()));
        let table = (String::from("self"), String::from("输入表"), String::new());
        column_items(true).into_iter().chain([table]).chain(keywords).collect()
    } else {
        let var = |name: &str, what: &str| (name.to_string(), what.to_string(), String::new());
        let mut list = vec![
            var("df", "第 1 个输入（LazyFrame）"),
            var("df2", "第 2 个输入"),
            var("df3", "第 3 个输入"),
            var("df4", "第 4 个输入"),
            var("out", "把结果赋给它"),
            var("pl", "polars"),
            var("ch", "取这个节点的参数"),
            var("inputs", "全部输入的列表"),
        ];
        list.extend(["print", "len", "range", "str", "int", "float", "list", "dict"].map(|n| var(n, "内置函数")));
        list.extend(PY_KEYWORDS.iter().map(|k| var(k, "关键字")));
        list
    };
    let keep_order = !head.ends_with('.');
    Found { start, items: rank(candidates, prefix, keep_order) }
}

/// 把 text[from..to] 换成 insert；返回新文本和新的光标位置。
pub fn replace(text: &str, from: usize, to: usize, insert: &str) -> (String, usize) {
    let (a, b) = (clamp(text, from.min(to)), clamp(text, from.max(to)));
    (format!("{}{insert}{}", &text[..a], &text[b..]), a + insert.len())
}

/// Tab：插入 4 个空格；回车：新行保持缩进，冒号 / 开括号后面多缩一层。
pub fn press(text: &str, anchor: usize, cursor: usize, key: &str) -> (String, usize) {
    let at = clamp(text, anchor.min(cursor));
    let insert = if key == "tab" {
        String::from("    ")
    } else {
        let line = &text[text[..at].rfind('\n').map_or(0, |n| n + 1)..at];
        let indent = &line[..line.len() - line.trim_start_matches([' ', '\t']).len()];
        let deeper = line.trim_end().ends_with([':', '(', '[', '{']);
        format!("\n{indent}{}", if deeper { "    " } else { "" })
    };
    replace(text, anchor, cursor, &insert)
}

/// 光标所在的那次调用的签名和说明（编辑框下面那一行提示）。
pub fn hint(text: &str, cursor: usize) -> String {
    let cursor = clamp(text, cursor);
    let Some((open, name)) = enclosing_call(text, cursor) else { return String::new() };
    if name.is_empty() {
        return String::new();
    }
    let head = &text[..open - name.len()];
    let Some(recv) = head.strip_suffix('.') else { return String::new() };
    let all = ["Expr", "LazyFrame", "DataFrame", "GroupBy", "pl"].into_iter().chain(NAMESPACES.iter().copied());
    let found = receiver_groups(recv).into_iter().chain(all).flat_map(group).find(|e| e.name == name);
    match found {
        Some(e) if e.doc.is_empty() => format!("{name}{}", e.sig),
        Some(e) => format!("{name}{}  —  {}", e.sig, e.doc),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fixture() {
        CATALOG.get_or_init(|| {
            parse_catalog(&json!({
                "pl": [["col", "(name)", "Create an expression"], ["concat", "(items)", ""], ["lit", "(value)", ""]],
                "LazyFrame": [["filter", "(*predicates)", "Filter rows"], ["group_by", "(*by)", ""], ["collect", "()", ""]],
                "DataFrame": [["pivot", "(on)", ""], ["filter", "(*predicates)", "Filter rows"]],
                "GroupBy": [["agg", "(*aggs)", "Aggregate"], ["len", "()", ""]],
                "Expr": [["sum", "()", "Get sum"], ["alias", "(name)", ""], ["str", "", ""], ["is_in", "(other)", ""],
                    ["read_csv", "(source, *, encoding='utf8', n_rows=0)", ""]],
                "str": [["contains", "(pattern)", "Check"], ["len_chars", "()", ""]],
            }))
        });
    }

    fn labels(text: &str) -> Vec<String> {
        labels_with(text, &[])
    }

    fn labels_with(text: &str, params: &[Item]) -> Vec<String> {
        fixture();
        let cols = [String::from("金额"), String::from("医院 名称")];
        complete(text, text.len(), &cols, params, false, "").items.into_iter().map(|i| i.label).collect()
    }

    fn ch_items() -> Vec<Item> {
        vec![
            Item { label: String::from("倍率"), detail: String::from("乘数"), doc: String::from("每行乘的数") },
            Item { label: String::from("城市"), detail: String::from("参数"), doc: String::new() },
        ]
    }

    #[test]
    fn layers_keep_positions() {
        let code = "out = df.filter(pl.col(\"金额\") > 10)  # 注释\nif True: pass";
        let got = layers(code, false);
        assert_eq!(got.len(), LAYERS);
        for layer in &got {
            assert_eq!(layer.chars().count(), code.chars().count());
            assert_eq!(layer.find('\n'), code.find('\n'));
        }
        assert_eq!(got[Tok::Keyword as usize].trim(), "if True: pass".replace(':', " ").trim());
        assert_eq!(got[Tok::Str as usize].trim(), "\"金额\"");
        assert_eq!(got[Tok::Comment as usize].trim(), "# 注释");
        assert_eq!(got[Tok::Number as usize].trim(), "10");
        assert!(got[Tok::Call as usize].contains("filter") && got[Tok::Call as usize].contains("col"));
        assert!(got[0].starts_with("out = df.      (pl.   ( \u{3000}\u{3000} ) >"), "{:?}", got[0]);
        // SQL：关键字不分大小写，-- 是注释
        let sql = layers("select a from self -- x", true);
        assert_eq!(sql[Tok::Keyword as usize].split_whitespace().collect::<Vec<_>>(), ["select", "from"]);
        assert_eq!(sql[Tok::Comment as usize].trim(), "-- x");
    }

    #[test]
    fn completes_by_receiver() {
        assert_eq!(labels("pl."), ["col", "lit", "concat"]);
        assert_eq!(labels("pl.c"), ["col", "concat"]);
        assert_eq!(labels("df."), ["filter", "group_by", "collect", "pivot"]);
        assert_eq!(labels("df.filter(pl.col(\"a\").s"), ["sum", "str"]);
        assert_eq!(labels("pl.col(\"a\").str."), ["contains", "len_chars"]);
        assert_eq!(labels("df.group_by(\"a\")."), ["agg", "len"]);
        assert_eq!(labels("out = (\n    df.filter(x)\n    .gr"), ["group_by"]);
        assert_eq!(labels("df.collect().p"), ["pivot"]);
        assert_eq!(labels("pl.concat([df, df2]).fi"), ["filter"]);
        assert!(labels("x = 1.").is_empty());
        assert!(labels("# df.").is_empty());
    }

    #[test]
    fn completes_columns_variables_and_sql() {
        assert_eq!(labels("pl.col(\""), ["金额", "医院 名称"]);
        assert_eq!(labels("pl.col(\"医"), ["医院 名称"]);
        assert!(labels("pl.col(\"金额").is_empty()); // 已经打完整
        assert!(labels("pl.col(\"金额\"").is_empty());
        assert_eq!(labels("d"), ["df", "df2", "df3", "df4", "dict", "def", "del"]);
        assert!(labels("out").is_empty());
        assert!(labels("").is_empty());
        let cols = [String::from("金额"), String::from("医院 名称")];
        let sql = |text: &str| -> Vec<String> {
            complete(text, text.len(), &cols, &[], true, "").items.into_iter().map(|i| i.label).collect()
        };
        assert_eq!(sql("select 医"), ["\"医院 名称\""]);
        assert_eq!(sql("sel"), ["self", "SELECT"]);
        assert_eq!(sql("select \"金"), ["金额"]);
        assert!(sql("where a = '金").is_empty());
    }

    #[test]
    fn completes_params_and_paths() {
        let ps = ch_items();
        assert_eq!(labels_with("ch(\"", &ps), ["倍率", "城市"]);
        assert_eq!(labels_with("out = df.filter(ch(\"城", &ps), ["城市"]);
        // 已经打完整了就不再弹
        assert!(labels_with("ch(\"城市", &ps).is_empty());
        // ch 之外的字符串还是补列名
        assert_eq!(labels_with("pl.col(\"金", &ps), ["金额"]);
        // 路径函数的字符串参数补文件路径
        let dir = std::env::temp_dir().join(format!("rtv_complete_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("子目录")).unwrap();
        std::fs::write(dir.join("sales.xlsx"), "").unwrap();
        std::fs::write(dir.join("notes.txt"), "").unwrap();
        let d = dir.to_string_lossy().replace('/', "\\");
        let at = |text: String| -> Vec<String> {
            fixture();
            complete(&text, text.len(), &[], &[], false, "").items.into_iter().map(|i| i.label).collect()
        };
        let base = format!("pl.read_csv(\"{d}\\");
        assert_eq!(at(format!("{base}")), ["子目录\\", "notes.txt", "sales.xlsx"]);
        assert_eq!(at(format!("{base}子")), ["子目录\\"]);
        assert_eq!(at(format!("{base}s")), ["sales.xlsx"]);
        // 换了分隔符（正斜杠）照样补，文件夹的名字跟着用 /
        let base = format!("pl.read_csv(\"{}/", dir.to_string_lossy());
        assert_eq!(at(format!("{base}")), ["子目录/", "notes.txt", "sales.xlsx"]);
        // 列名函数里不补路径
        assert!(at(format!("pl.col(\"{d}\\")).is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn completes_kwargs() {
        fixture();
        let labels =
            |text: &str| -> Vec<String> { complete(text, text.len(), &[], &[], false, "").items.into_iter().map(|i| i.label).collect() };
        let details = |text: &str| -> Vec<String> {
            complete(text, text.len(), &[], &[], false, "").items.into_iter().map(|i| i.detail).collect()
        };
        assert_eq!(labels("pl.read_csv("), ["source=", "encoding=", "n_rows="]);
        assert_eq!(details("pl.read_csv("), ["必填", "'utf8'", "0"]);
        assert_eq!(labels("pl.read_csv(e"), ["encoding="]);
        // 已经填过的不列
        assert_eq!(labels("pl.read_csv(encoding=\"utf8\", n"), ["n_rows="]);
        // = 后面是值，不弹
        assert!(labels("pl.read_csv(encoding=").is_empty());
        // 前缀筛不中参数时退回普通补全
        assert_eq!(labels("pl.read_csv(pl."), ["col", "lit", "concat"]);
        // 全是位置参数（*predicates）的打完 ( 不弹
        assert!(labels("df.filter(").is_empty());
        // 必填的也能按前缀补
        assert_eq!(labels("pl.read_csv(s"), ["source="]);
    }

    #[test]
    fn replace_press_and_hint() {
        let text = "df.fi + 1";
        let found = complete(text, 5, &[], &[], false, "");
        assert_eq!(found.start, 3);
        assert_eq!(replace(text, found.start, 5, "filter"), (String::from("df.filter + 1"), 9));
        assert_eq!(press("ab", 1, 1, "tab"), (String::from("a    b"), 5));
        assert_eq!(press("  if x:", 7, 7, "enter"), (String::from("  if x:\n      "), 14));
        assert_eq!(press("  a = 1\nb", 7, 7, "enter"), (String::from("  a = 1\n  \nb"), 10));
        assert_eq!(press("abc", 1, 2, "tab"), (String::from("a    c"), 5)); // 选区被替换
        fixture();
        let at = |text: &str| hint(text, text.len());
        assert_eq!(at("df.filter(pl.col(\"a\") > 1, "), "filter(*predicates)  —  Filter rows");
        assert_eq!(at("df.filter(pl.col("), "col(name)  —  Create an expression");
        assert_eq!(at("pl.col(\"a\").str.contains("), "contains(pattern)  —  Check");
        assert_eq!(at("df.filter(x)"), "");
        assert_eq!(at("print("), "");
    }
}
