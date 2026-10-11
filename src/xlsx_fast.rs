//! 自研 xlsx 快速解析：zip 流式解压 → 按 `<row` 边界切块 → rayon 并行解析块。
//!
//! 只处理常见的无前缀 SpreadsheetML（Excel/openpyxl/WPS 等的输出）；
//! `probe` 发现异常结构（带命名空间前缀等）时调用方应回退到 calamine。

use memchr::{memchr, memmem};
use rayon::prelude::*;
use std::io::Read;
use std::path::Path;

use crate::loader::Chunk;

pub type Archive = zip::ZipArchive<std::io::BufReader<std::fs::File>>;

pub fn open_archive(path: &Path) -> Result<Archive, String> {
    let f = std::fs::File::open(path).map_err(|e| e.to_string())?;
    zip::ZipArchive::new(std::io::BufReader::with_capacity(1 << 16, f)).map_err(|e| e.to_string())
}

fn read_part(ar: &mut Archive, name: &str) -> Result<Vec<u8>, String> {
    let mut e = ar.by_name(name).map_err(|e| format!("{name}: {e}"))?;
    let mut v = Vec::with_capacity(e.size() as usize);
    e.read_to_end(&mut v).map_err(|e| e.to_string())?;
    Ok(v)
}

// ---------------------------------------------------------------- 基础工具

/// 取标签属性值（要求双引号，前面有空白）。每个单元格要查好几次，模式拼在栈上、不分配。
fn attr<'a>(tag: &'a [u8], name: &str) -> Option<&'a [u8]> {
    const MAX: usize = 32; // 用到的属性名都很短（numFmtId、formatCode…）
    let n = name.len();
    debug_assert!(n + 3 <= MAX);
    let mut buf = [0u8; MAX];
    if n + 3 > MAX {
        return None;
    }
    buf[0] = b' ';
    buf[1..1 + n].copy_from_slice(name.as_bytes());
    buf[1 + n] = b'=';
    buf[2 + n] = b'"';
    let pat = &buf[..n + 3];
    let i = memmem::find(tag, pat)? + pat.len();
    let j = memchr(b'"', &tag[i..])?;
    Some(&tag[i..i + j])
}

/// 解 XML 实体（&lt; &gt; &amp; &quot; &apos; &#N; &#xH;）。
fn unescape(s: &[u8]) -> String {
    let mut out = String::new();
    unescape_into(s, &mut out);
    out
}

/// 解 XML 实体并追加到 out。
fn unescape_into(s: &[u8], out: &mut String) {
    if memchr(b'&', s).is_none() {
        match std::str::from_utf8(s) {
            Ok(t) => out.push_str(t),
            Err(_) => out.push_str(&String::from_utf8_lossy(s)),
        }
        return;
    }
    let text = String::from_utf8_lossy(s);
    let mut rest: &str = &text;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let Some(end) = rest.find(';').filter(|&e| e <= 10) else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let ent = &rest[1..end];
        let ch = match ent {
            "lt" => Some('<'),
            "gt" => Some('>'),
            "amp" => Some('&'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ => ent
                .strip_prefix("#x")
                .or_else(|| ent.strip_prefix("#X"))
                .and_then(|h| u32::from_str_radix(h, 16).ok())
                .or_else(|| ent.strip_prefix('#').and_then(|d| d.parse().ok()))
                .and_then(char::from_u32),
        };
        match ch {
            Some(c) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
}

/// 从 `<si>` / `<is>` 内容里拼出文本：累加所有 `<t>`，跳过拼音 `<rPh>`。
fn extract_text(body: &[u8]) -> String {
    let mut out = String::new();
    extract_text_into(body, &mut out);
    out
}

fn extract_text_into(body: &[u8], out: &mut String) {
    let mut pos = 0;
    while let Some(i) = memchr(b'<', &body[pos..]) {
        let p = pos + i;
        let rest = &body[p..];
        if rest.starts_with(b"<rPh") {
            match memmem::find(rest, b"</rPh>") {
                Some(e) => pos = p + e + 6,
                None => break,
            }
        } else if rest.len() > 2 && rest[1] == b't' && matches!(rest[2], b'>' | b' ' | b'/') {
            let Some(gt) = memchr(b'>', rest) else { break };
            if rest[gt - 1] == b'/' {
                pos = p + gt + 1;
                continue;
            }
            let content = &rest[gt + 1..];
            match memmem::find(content, b"</t>") {
                Some(e) => {
                    unescape_into(&content[..e], out);
                    pos = p + gt + 1 + e + 4;
                }
                None => break,
            }
        } else {
            pos = p + 1;
        }
    }
}

fn col_index(letters: &[u8]) -> Option<usize> {
    if letters.is_empty() {
        return None;
    }
    let mut n = 0usize;
    for &b in letters {
        if !b.is_ascii_alphabetic() {
            break;
        }
        n = n * 26 + (b.to_ascii_uppercase() - b'A' + 1) as usize;
    }
    (n > 0).then(|| n - 1)
}

// ---------------------------------------------------------------- 工作簿结构

/// (sheet 名, zip 内路径)，按工作簿顺序。
pub fn sheet_parts(ar: &mut Archive) -> Result<Vec<(String, String)>, String> {
    let wb = read_part(ar, "xl/workbook.xml")?;
    let rels = read_part(ar, "xl/_rels/workbook.xml.rels")?;

    let mut targets: Vec<(Vec<u8>, String)> = Vec::new();
    let mut pos = 0;
    while let Some(i) = memmem::find(&rels[pos..], b"<Relationship ") {
        let p = pos + i;
        let Some(gt) = memchr(b'>', &rels[p..]) else { break };
        let tag = &rels[p..p + gt];
        if let (Some(id), Some(t)) = (attr(tag, "Id"), attr(tag, "Target")) {
            targets.push((id.to_vec(), unescape(t)));
        }
        pos = p + gt;
    }

    let mut out = Vec::new();
    let mut pos = 0;
    while let Some(i) = memmem::find(&wb[pos..], b"<sheet ") {
        let p = pos + i;
        let Some(gt) = memchr(b'>', &wb[p..]) else { break };
        let tag = &wb[p..p + gt];
        pos = p + gt;
        let (Some(name), Some(rid)) = (attr(tag, "name"), attr(tag, "r:id")) else { continue };
        let Some((_, target)) = targets.iter().find(|(id, _)| id.as_slice() == rid) else { continue };
        let part = match target.strip_prefix('/') {
            Some(abs) => abs.to_owned(),
            None => format!("xl/{target}"),
        };
        if part.contains("/worksheets/") {
            out.push((unescape(name), part));
        }
    }
    Ok(out)
}

/// 首个 sheet 头部探测：确认是无前缀的标准结构。
pub fn probe(ar: &mut Archive, part: &str) -> bool {
    let Ok(mut e) = ar.by_name(part) else { return false };
    let mut head = vec![0u8; 64 * 1024];
    let mut n = 0;
    while n < head.len() {
        match e.read(&mut head[n..]) {
            Ok(0) => break,
            Ok(k) => n += k,
            Err(_) => return false,
        }
    }
    let head = &head[..n];
    // 带前缀（<x:sheetData）或找不到 sheetData 但文件很大 → 不支持
    memmem::find(head, b"<sheetData").is_some() || n < head.len() && memmem::find(head, b"sheetData").is_none()
}

// ---------------------------------------------------------------- 单元格样式（日期）

/// 数字格的格式是哪一类：Excel 把日期存成序列号（45122），显示成什么由样式里的数字格式决定。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NumKind {
    Plain,
    /// 日期、时间、日期时间
    Date,
    /// [h]:mm:ss 这类时长
    Duration,
}

/// 工作簿的样式里和显示有关的部分：每个样式（`<c s="…">` 的下标）是不是日期，以及是不是 1904 纪元。
#[derive(Default)]
pub struct Styles {
    kinds: Vec<NumKind>,
    pub is_1904: bool,
}

impl Styles {
    pub fn kind(&self, style: usize) -> NumKind {
        self.kinds.get(style).copied().unwrap_or(NumKind::Plain)
    }
}

/// 读 styles.xml 的数字格式和 workbook.xml 的 date1904；读不出来就当没有日期样式。
pub fn read_styles(ar: &mut Archive) -> Styles {
    let is_1904 = read_part(ar, "xl/workbook.xml").is_ok_and(|wb| workbook_is_1904(&wb));
    let kinds = read_part(ar, "xl/styles.xml").map(|xml| style_kinds(&xml)).unwrap_or_default();
    Styles { kinds, is_1904 }
}

fn workbook_is_1904(wb: &[u8]) -> bool {
    let Some(i) = memmem::find(wb, b"<workbookPr") else { return false };
    let Some(gt) = memchr(b'>', &wb[i..]) else { return false };
    matches!(attr(&wb[i..i + gt], "date1904"), Some(b"1" | b"true"))
}

/// cellXfs 里每个 xf（按顺序就是样式下标）的数字格式归类；自定义格式（numFmts）盖住同号的内置格式。
fn style_kinds(xml: &[u8]) -> Vec<NumKind> {
    let number = |v: &[u8]| std::str::from_utf8(v).ok()?.trim().parse::<u32>().ok();
    let mut custom: Vec<(u32, NumKind)> = Vec::new();
    for tag in tags(xml, b"<numFmt") {
        if let (Some(id), Some(code)) = (attr(tag, "numFmtId").and_then(number), attr(tag, "formatCode")) {
            custom.push((id, format_kind(&unescape(code))));
        }
    }
    let Some(start) = memmem::find(xml, b"<cellXfs") else { return Vec::new() };
    let end = memmem::find(&xml[start..], b"</cellXfs>").map_or(xml.len(), |e| start + e);
    tags(&xml[start..end], b"<xf")
        .map(|tag| {
            let id = attr(tag, "numFmtId").and_then(number).unwrap_or(0);
            custom.iter().find(|(c, _)| *c == id).map_or_else(|| builtin_kind(id), |(_, kind)| *kind)
        })
        .collect()
}

/// xml 里名字正好是 name 的各个开始标签（`<name ` 到 `>`，不含后面的子元素）。
fn tags<'a>(xml: &'a [u8], name: &'a [u8]) -> impl Iterator<Item = &'a [u8]> + 'a {
    let mut pos = 0;
    std::iter::from_fn(move || {
        loop {
            let p = pos + memmem::find(&xml[pos..], name)?;
            pos = p + name.len();
            // <xf 不算 <xfId 之类别的名字
            if !matches!(xml.get(pos), Some(b' ' | b'>' | b'/' | b'\t' | b'\r' | b'\n')) {
                continue;
            }
            let gt = memchr(b'>', &xml[p..])?;
            pos = p + gt;
            return Some(&xml[p..p + gt]);
        }
    })
}

/// 内置的数字格式：14–22、45、47 是日期时间，46 是时长；27–36、50–58 是中文 / 日文 / 韩文区域的内置日期格式
/// （yyyy"年"m"月"d"日" 这些，Excel 只存编号）。
fn builtin_kind(id: u32) -> NumKind {
    match id {
        14..=22 | 27..=36 | 45 | 47 | 50..=58 => NumKind::Date,
        46 => NumKind::Duration,
        _ => NumKind::Plain,
    }
}

/// 自定义格式码是哪一类：只看第一节（`;` 前面），跳过引号里的字、`\` `_` `*` 后面那个字符、方括号段
/// （[Red]、[$-804]、[>=100]）；有 y / m / d / h / s 就是日期时间，方括号里是 [h] / [mm] / [ss] 是时长。
pub fn format_kind(code: &str) -> NumKind {
    let mut chars = code.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' | '_' | '*' => {
                chars.next();
            }
            '"' => {
                for q in chars.by_ref() {
                    if q == '"' {
                        break;
                    }
                }
            }
            '[' => {
                let inner: String = chars.by_ref().take_while(|&q| q != ']').collect();
                let lower = inner.to_ascii_lowercase();
                let first = lower.chars().next();
                if first.is_some_and(|f| matches!(f, 'h' | 'm' | 's') && lower.chars().all(|q| q == f)) {
                    return NumKind::Duration;
                }
            }
            ';' => break,
            'y' | 'Y' | 'm' | 'M' | 'd' | 'D' | 'h' | 'H' | 's' | 'S' => return NumKind::Date,
            _ => {}
        }
    }
    NumKind::Plain
}

pub fn read_shared_strings(ar: &mut Archive) -> Vec<String> {
    let Ok(data) = read_part(ar, "xl/sharedStrings.xml") else { return Vec::new() };
    let mut ranges: Vec<(usize, usize)> = Vec::new();
    let mut pos = 0;
    while let Some(i) = memmem::find(&data[pos..], b"<si") {
        let p = pos + i;
        let after = data.get(p + 3).copied().unwrap_or(0);
        if !matches!(after, b'>' | b' ' | b'/') {
            pos = p + 3;
            continue;
        }
        let Some(gt) = memchr(b'>', &data[p..]) else { break };
        if data[p + gt - 1] == b'/' {
            ranges.push((p + gt, p + gt)); // <si/> 空串
            pos = p + gt + 1;
            continue;
        }
        let start = p + gt + 1;
        let Some(e) = memmem::find(&data[start..], b"</si>") else { break };
        ranges.push((start, start + e));
        pos = start + e + 5;
    }
    ranges.par_iter().map(|&(s, e)| extract_text(&data[s..e])).collect()
}

// ---------------------------------------------------------------- sheet 数据

/// 数字文本规范化：整数原样，其余按 f64 显示（与旧实现一致）。
#[cfg(test)]
fn number_text(v: &[u8]) -> String {
    let mut s = String::new();
    number_text_into(v, &mut s);
    s
}

fn number_text_into(v: &[u8], out: &mut String) {
    let digits = if v.first() == Some(&b'-') { &v[1..] } else { v };
    let simple = !digits.is_empty()
        && digits.len() <= 15
        && digits.iter().all(u8::is_ascii_digit)
        && (digits.len() == 1 || digits[0] != b'0')
        && v != b"-0";
    if simple {
        // SAFETY: 全是 ASCII 数字/负号
        let s = unsafe { std::str::from_utf8_unchecked(v) };
        if crate::loader::thousands_enabled() {
            if let Ok(i) = s.parse::<i128>() {
                out.push_str(&crate::loader::thousands_sep(&i.to_string()));
                return;
            }
        }
        out.push_str(s);
        return;
    }
    let s = String::from_utf8_lossy(v);
    match s.trim().parse::<f64>() {
        Ok(f) => out.push_str(&crate::loader::format_float(f)),
        Err(_) => out.push_str(&s),
    }
}

/// 把单元格值追加到 out（共享字符串直接拷贝字节，不分配）。数字格的样式是日期时写成日期。
fn cell_value(tag: &[u8], body: &[u8], sst: &[String], styles: &Styles, out: &mut String) {
    let v = || -> Option<&[u8]> {
        let i = memmem::find(body, b"<v")?;
        let gt = memchr(b'>', &body[i..])?;
        if body[i + gt - 1] == b'/' {
            return None;
        }
        let rest = &body[i + gt + 1..];
        let e = memmem::find(rest, b"</v>")?;
        Some(&rest[..e])
    };
    match attr(tag, "t") {
        Some(b"s") => {
            let s = v()
                .and_then(|x| std::str::from_utf8(x).ok()?.trim().parse::<usize>().ok())
                .and_then(|i| sst.get(i));
            if let Some(s) = s {
                out.push_str(s);
            }
        }
        Some(b"inlineStr") => {
            if let Some(i) = memmem::find(body, b"<is") {
                let rest = &body[i..];
                let end = memmem::find(rest, b"</is>").unwrap_or(rest.len());
                extract_text_into(&rest[..end], out);
            }
        }
        Some(b"b") => out.push_str(if v() == Some(b"1") { "TRUE" } else { "FALSE" }),
        Some(b"d") => {
            if let Some(x) = v() {
                crate::loader::push_iso_datetime(&unescape(x), out);
            }
        }
        Some(b"str") | Some(b"e") => {
            if let Some(x) = v() {
                unescape_into(x, out);
            }
        }
        _ => {
            let Some(x) = v() else { return };
            let style = attr(tag, "s").and_then(|s| std::str::from_utf8(s).ok()?.parse::<usize>().ok());
            let kind = style.map_or(NumKind::Plain, |s| styles.kind(s));
            let serial = (kind != NumKind::Plain).then(|| std::str::from_utf8(x).ok()?.trim().parse::<f64>().ok()).flatten();
            match serial {
                Some(f) => crate::loader::write_excel_datetime(f, kind == NumKind::Duration, styles.is_1904, out),
                None => number_text_into(x, out),
            }
        }
    }
}

/// 一块的解析结果：紧凑行数据（块内行号跳空已补空行）+ 首行/末行的 0 基行号。
pub struct Parsed {
    pub chunk: Chunk,
    pub first: Option<u32>, // 首行显式行号（无 r 属性时为 None）
    pub last: Option<u32>,  // 末行行号（能推算时）
}

/// 解析一块（由完整的 `<row>` 组成，可带前后杂项），单元格按列位置排布，直接写入紧凑存储。
pub fn parse_block(block: &[u8], sst: &[String], styles: &Styles) -> Parsed {
    let mut chunk = Chunk::default();
    let mut first: Option<Option<u32>> = None;
    let mut prev: Option<u32> = None;
    let mut pos = 0;
    while let Some(i) = memmem::find(&block[pos..], b"<row") {
        let p = pos + i;
        let next = block.get(p + 4).copied().unwrap_or(0);
        if !matches!(next, b' ' | b'>' | b'/') {
            pos = p + 4;
            continue;
        }
        let Some(gt) = memchr(b'>', &block[p..]) else { break };
        let row_tag = &block[p..p + gt];
        let rnum = attr(row_tag, "r")
            .and_then(|r| std::str::from_utf8(r).ok()?.parse::<u32>().ok())
            .map(|r| r.saturating_sub(1));
        if first.is_none() {
            first = Some(rnum);
        } else if let (Some(pr), Some(r)) = (prev, rnum) {
            // 块内整行空白用空行占位，保持行号一致
            for _ in pr + 1..r {
                chunk.end_row();
            }
        }
        prev = rnum.or(prev.map(|x| x + 1));

        let body_start = p + gt + 1;
        if block[p + gt - 1] == b'/' {
            chunk.end_row();
            pos = body_start;
            continue;
        }
        let body_end = memmem::find(&block[body_start..], b"</row>")
            .map(|e| body_start + e)
            .unwrap_or(block.len());
        let body = &block[body_start..body_end];
        pos = body_end;

        let mut cp = 0;
        while let Some(ci) = memmem::find(&body[cp..], b"<c") {
            let c0 = cp + ci;
            let nb = body.get(c0 + 2).copied().unwrap_or(0);
            if !matches!(nb, b' ' | b'>' | b'/') {
                cp = c0 + 2;
                continue;
            }
            let Some(cgt) = memchr(b'>', &body[c0..]) else { break };
            let tag = &body[c0..c0 + cgt];
            let (inner, next_cp): (&[u8], usize) = if body[c0 + cgt - 1] == b'/' {
                (&[], c0 + cgt + 1)
            } else {
                let inner = &body[c0 + cgt + 1..];
                let end = memmem::find(inner, b"</c>").unwrap_or(inner.len());
                (&inner[..end], c0 + cgt + 1 + end)
            };
            cp = next_cp;
            let have = chunk.open_cells();
            let col = attr(tag, "r").and_then(col_index).unwrap_or(have);
            if col >= have {
                for _ in have..col {
                    chunk.end_cell();
                }
                if !inner.is_empty() {
                    cell_value(tag, inner, sst, styles, chunk.text_mut());
                }
                chunk.end_cell();
            } else {
                // 乱序单元格（罕见）：改写已有的格
                let mut text = String::new();
                cell_value(tag, inner, sst, styles, &mut text);
                chunk.set_open_cell(col, &text);
            }
        }
        chunk.end_row();
    }
    chunk.shrink_to_fit();
    Parsed { chunk, first: first.flatten(), last: prev }
}

/// 按 `<row` 边界切块读取解压后的 sheet XML。
pub struct BlockReader<R: Read> {
    rd: R,
    carry: Vec<u8>,
    eof: bool,
}

impl<R: Read> BlockReader<R> {
    pub fn new(rd: R) -> Self {
        BlockReader { rd, carry: Vec::new(), eof: false }
    }

    /// 取下一块（≥ target 字节，尾部对齐到 `<row` 起点）；读完返回 None。
    pub fn next_block(&mut self, target: usize) -> Option<Vec<u8>> {
        loop {
            if self.eof {
                return if self.carry.is_empty() { None } else { Some(std::mem::take(&mut self.carry)) };
            }
            if self.carry.len() >= target {
                if let Some(cut) = memmem::rfind(&self.carry, b"<row").filter(|&c| c > 0) {
                    let tail = self.carry.split_off(cut);
                    return Some(std::mem::replace(&mut self.carry, tail));
                }
            }
            let old = self.carry.len();
            let want = (target / 4).max(64 * 1024);
            self.carry.resize(old + want, 0);
            let mut got = 0;
            while got < want {
                match self.rd.read(&mut self.carry[old + got..]) {
                    Ok(0) | Err(_) => {
                        self.eof = true;
                        break;
                    }
                    Ok(k) => got += k,
                }
            }
            self.carry.truncate(old + got);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unescape_entities() {
        assert_eq!(unescape("a&lt;b&amp;&#20540;&#x41;&bogus;".as_bytes()), "a<b&值A&bogus;");
    }

    #[test]
    fn text_skips_phonetic() {
        let s = b"<r><t>ab</t></r><rPh sb=\"0\"><t>xx</t></rPh><r><t xml:space=\"preserve\"> c</t></r>";
        assert_eq!(extract_text(s), "ab c");
    }

    #[test]
    fn rows_with_gaps_and_types() {
        let sst = vec!["hello".to_string()];
        let xml = br#"<sheetData><row r="1"><c r="A1" t="s"><v>0</v></c><c r="C1"><v>1.5</v></c></row><row r="3"><c r="B3" t="inlineStr"><is><t>x&amp;y</t></is></c><c r="D3" t="b"><v>1</v></c><c r="E3" s="2"/></row></sheetData>"#;
        let p = parse_block(xml, &sst, &Styles::default());
        assert_eq!((p.first, p.last), (Some(0), Some(2)));
        let c = &p.chunk;
        assert_eq!(c.rows(), 3); // 第 2 行空白占位
        let row = |r: usize| (0..c.row_len(r)).map(|i| c.get(r, i).to_string()).collect::<Vec<_>>();
        assert_eq!(row(0), ["hello", "", "1.5"]);
        assert!(row(1).is_empty());
        assert_eq!(row(2), ["", "x&y", "", "TRUE", ""]);
    }

    #[test]
    fn out_of_order_cells() {
        let xml = br#"<row r="1"><c r="C1"><v>3</v></c><c r="A1"><v>1</v></c></row>"#;
        let p = parse_block(xml, &[], &Styles::default());
        let c = &p.chunk;
        assert_eq!((c.get(0, 0), c.get(0, 1), c.get(0, 2)), ("1", "", "3"));
    }

    #[test]
    fn numbers() {
        assert_eq!(number_text(b"42"), "42");
        assert_eq!(number_text(b"3.0"), "3");
        assert_eq!(number_text(b"1E+2"), "100");
        // 和 Excel 显示的一样最多 15 位有效数字
        assert_eq!(number_text(b"0.30000000000000004"), "0.3");
        assert_eq!(number_text(b"1234.5600000000002"), "1234.56");
    }

    /// 自定义格式码：日期 / 时长 / 普通数字。
    #[test]
    fn format_codes() {
        for code in ["yyyy-mm-dd", "yyyy\"年\"m\"月\"d\"日\"", "[$-804]yyyy/m/d h:mm", "h:mm AM/PM", "mm:ss.0", "[Red]d-mmm"] {
            assert_eq!(format_kind(code), NumKind::Date, "{code}");
        }
        for code in ["[h]:mm:ss", "[mm]:ss", "[Red][h]:mm"] {
            assert_eq!(format_kind(code), NumKind::Duration, "{code}");
        }
        for code in ["General", "G/通用格式", "0.00", "#,##0 \"元\"", "0.00E+00", "[$¥-804]#,##0.00", "0.00_);[Red](0.00)", "@", "\"day\" 0", "0\\d"] {
            assert_eq!(format_kind(code), NumKind::Plain, "{code}");
        }
        assert_eq!((builtin_kind(14), builtin_kind(31), builtin_kind(46), builtin_kind(2)), (NumKind::Date, NumKind::Date, NumKind::Duration, NumKind::Plain));
    }

    /// styles.xml：cellXfs 里第几个 xf 就是样式几；自定义格式盖住同号的内置格式。workbook.xml 的 date1904。
    #[test]
    fn styles_from_xml() {
        let xml = r#"<styleSheet><numFmts count="2"><numFmt numFmtId="164" formatCode="yyyy&quot;年&quot;m&quot;月&quot;"/><numFmt numFmtId="165" formatCode="0.000"/></numFmts>
            <cellStyleXfs count="1"><xf numFmtId="14"/></cellStyleXfs>
            <cellXfs count="5"><xf numFmtId="0" fontId="0"/><xf numFmtId="14" applyNumberFormat="1"/><xf numFmtId="164"><alignment/></xf><xf numFmtId="165"/><xf numFmtId="46"/></cellXfs></styleSheet>"#;
        let kinds = style_kinds(xml.as_bytes());
        assert_eq!(kinds, [NumKind::Plain, NumKind::Date, NumKind::Date, NumKind::Plain, NumKind::Duration]);
        assert!(workbook_is_1904(br#"<workbook><workbookPr date1904="1" defaultThemeVersion="1"/></workbook>"#));
        assert!(!workbook_is_1904(br#"<workbook><workbookPr defaultThemeVersion="1"/></workbook>"#));
    }

    /// 日期样式的数字格写成日期（1900 / 1904 纪元），t="d" 的 ISO 日期把 T 换成空格。
    #[test]
    fn date_cells() {
        let styles = Styles { kinds: vec![NumKind::Plain, NumKind::Date, NumKind::Duration], is_1904: false };
        let xml = br#"<row r="1"><c r="A1" s="1"><v>45122</v></c><c r="B1" s="1"><v>45122.5208333333</v></c><c r="C1" s="1"><v>0.75</v></c><c r="D1" s="2"><v>1.5</v></c><c r="E1" s="0"><v>45122</v></c><c r="F1" t="d"><v>2023-07-15T08:00:00</v></c><c r="G1" s="1"><v>45122.999999999</v></c></row>"#;
        let p = parse_block(xml, &[], &styles);
        let c = &p.chunk;
        let row: Vec<&str> = (0..c.row_len(0)).map(|i| c.get(0, i)).collect();
        assert_eq!(row, ["2023-07-15", "2023-07-15 12:30:00", "18:00:00", "36:00:00", "45122", "2023-07-15 08:00:00", "2023-07-16"]);
        let mac = Styles { kinds: vec![NumKind::Plain, NumKind::Date], is_1904: true };
        let p = parse_block(br#"<row r="1"><c r="A1" s="1"><v>43660</v></c></row>"#, &[], &mac);
        assert_eq!(p.chunk.get(0, 0), "2023-07-15");
    }

    #[test]
    fn block_reader_cuts_on_row() {
        let data = b"<sheetData><row r=\"1\"><c/></row><row r=\"2\"><c/></row><row r=\"3\"><c/></row></sheetData>".to_vec();
        let mut br = BlockReader::new(&data[..]);
        let mut all = 0;
        while let Some(b) = br.next_block(10) {
            all += parse_block(&b, &[], &Styles::default()).chunk.rows();
        }
        assert_eq!(all, 3);
    }
}
