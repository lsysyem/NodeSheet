use std::fmt;
use std::path::Path;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock, RwLock, Weak};
use std::time::{Duration, Instant};
use serde_json::{Value, json};

/// 解码进度回调：参数是正在解码的源（用于 UI 判断是否当前 sheet）。
pub type ProgressFn = Arc<dyn Fn(&Arc<dyn DataSource>) + Send + Sync>;

/// 逐行回调：(行号, 与请求列一一对应的单元格文本)。
pub type RowFn<'a> = dyn FnMut(usize, &[&str]) + 'a;

/// 一张表（或一个 sheet）的数据源。所有方法按需取数，不要求整个文件都在内存里。
pub trait DataSource: Send + Sync {
    fn headers(&self) -> &[String];
    fn row_count(&self) -> usize;
    /// 返回数据行 [start, start+len) ∩ [0, row_count)。
    fn window(&self, start: usize, len: usize) -> Vec<Vec<String>>;
    /// 解码可能耗时的源（Excel 懒 sheet）是否已完成，可以直接渲染。
    fn is_ready(&self) -> bool {
        true
    }
    /// 预解码（应在后台线程调用），返回失败原因。
    fn prepare(&self) -> Result<(), String> {
        Ok(())
    }
    /// 渐进解码是否仍在进行（UI 状态栏提示“解码中”）。
    fn is_loading(&self) -> bool {
        false
    }
    /// 渐进解码的失败原因。
    fn load_error(&self) -> Option<String> {
        None
    }
    /// 各列的类型（str / i64 / date…，叫法同 polars），表头第二行显示。不知道时为空（Excel / CSV 按文本读）。
    fn dtypes(&self) -> Vec<String> {
        Vec::new()
    }
    /// 查看器自己读这个文件时认出来的读法（CSV 的编码、分隔符）：(read 节点的参数名, 值)。
    /// 打开文件建源节点时填进去，引擎读出来的才和查看器显示的一样。
    fn read_hints(&self) -> Vec<(&'static str, String)> {
        Vec::new()
    }
    /// 供列宽自适应取样的前 n 行（懒源可以用更廉价的方式实现）。
    fn sample(&self, n: usize) -> Vec<Vec<String>> {
        self.window(0, n.min(self.row_count()))
    }

    /// 扫描 [start, start+len)，只取 cols 列，逐行回调 (行号, cols 对应文本)；缺列视为空串。
    /// 行模型按块取数走这里，避免为每格分配 String。
    fn scan(&self, start: usize, len: usize, cols: &[usize], f: &mut RowFn) {
        let mut buf: Vec<&str> = Vec::with_capacity(cols.len());
        for (i, row) in self.window(start, len).iter().enumerate() {
            buf.clear();
            buf.extend(cols.iter().map(|&c| row.get(c).map(String::as_str).unwrap_or("")));
            f(start + i, &buf);
        }
    }
}

/// 一个 sheet / 一张表。
#[derive(Clone)]
pub struct SheetHandle {
    pub name: String,
    pub source: Arc<dyn DataSource>,
}

#[derive(Debug)]
pub enum LoadError {
    Io(std::io::Error),
    Unsupported(String),
    Format(String),
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LoadError::Io(e) => write!(f, "无法读取文件: {e}"),
            LoadError::Unsupported(ext) => {
                write!(f, "不支持的格式: .{ext}（支持 xlsx/xls/xlsm/xlsb/ods/parquet/tsv/csv）")
            }
            LoadError::Format(e) => write!(f, "解析失败: {e}"),
        }
    }
}

pub fn open_tables(path: &Path) -> Result<Vec<SheetHandle>, LoadError> {
    open_tables_with_progress(path, Arc::new(|_| {}))
}

/// Excel 文件里的 sheet 名（不解码数据）；读不出来时为空。
pub fn sheet_names(path: &Path) -> Vec<String> {
    use calamine::Reader;
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    if matches!(ext.as_str(), "xlsx" | "xlsm") {
        let parts = crate::xlsx_fast::open_archive(path).and_then(|mut ar| crate::xlsx_fast::sheet_parts(&mut ar));
        if let Ok(parts) = parts.as_deref() && !parts.is_empty() {
            return parts.iter().map(|(name, _)| name.clone()).collect();
        }
    }
    if !matches!(ext.as_str(), "xlsx" | "xlsm" | "xls" | "xlsb" | "ods") {
        return Vec::new();
    }
    calamine::open_workbook_auto(path).map(|book| book.sheet_names().to_vec()).unwrap_or_default()
}

pub fn open_tables_with_progress(path: &Path, progress: ProgressFn) -> Result<Vec<SheetHandle>, LoadError> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .ok_or_else(|| LoadError::Unsupported(String::from("(无扩展名)")))?;

    match ext.as_str() {
        "xlsx" | "xlsm" => open_xlsx_streaming(path, progress),
        "xls" | "xlsb" | "ods" => {
            let _ = progress;
            open_excel_legacy(path)
        }
        "parquet" => open_parquet(path),
        "csv" | "tsv" => open_csv(path, ext == "tsv"),
        _ => Err(LoadError::Unsupported(ext)),
    }
}

// ---------------------------------------------------------------- 紧凑行存储

/// 一块行数据：所有单元格文本拼在一个 String 里，按偏移切分（每格 4 字节开销，无逐格分配）。
#[derive(Default)]
pub struct Chunk {
    text: String,
    cell_end: Vec<u32>, // 第 i 格的结束字节
    row_end: Vec<u32>,  // 第 r 行的结束格序号
    width: usize,       // 最宽行的格数
}

impl Chunk {
    pub fn rows(&self) -> usize {
        self.row_end.len()
    }

    /// 直接向文本缓冲追加当前格内容，随后调用 end_cell。
    pub fn text_mut(&mut self) -> &mut String {
        &mut self.text
    }

    pub fn end_cell(&mut self) {
        debug_assert!(self.text.len() <= u32::MAX as usize);
        self.cell_end.push(self.text.len() as u32);
    }

    pub fn push_cell(&mut self, s: &str) {
        self.text.push_str(s);
        self.end_cell();
    }

    fn row_start_cell(&self) -> usize {
        self.row_end.last().copied().unwrap_or(0) as usize
    }

    /// 当前未结束行已有的格数。
    pub fn open_cells(&self) -> usize {
        self.cell_end.len() - self.row_start_cell()
    }

    pub fn end_row(&mut self) {
        self.width = self.width.max(self.open_cells());
        self.row_end.push(self.cell_end.len() as u32);
    }

    /// 改写当前未结束行里已存在的第 c 格（乱序单元格，罕见路径）。
    pub fn set_open_cell(&mut self, c: usize, s: &str) {
        let rs = self.row_start_cell();
        let mut cells: Vec<String> = (rs..self.cell_end.len()).map(|i| self.cell(i).to_owned()).collect();
        cells[c] = s.to_owned();
        let text_start = if rs == 0 { 0 } else { self.cell_end[rs - 1] as usize };
        self.text.truncate(text_start);
        self.cell_end.truncate(rs);
        for cell in &cells {
            self.push_cell(cell);
        }
    }

    fn cell(&self, i: usize) -> &str {
        let s = if i == 0 { 0 } else { self.cell_end[i - 1] as usize };
        &self.text[s..self.cell_end[i] as usize]
    }

    fn row_cells(&self, r: usize) -> (usize, usize) {
        let s = if r == 0 { 0 } else { self.row_end[r - 1] as usize };
        (s, self.row_end[r] as usize)
    }

    pub fn row_len(&self, r: usize) -> usize {
        let (s, e) = self.row_cells(r);
        e - s
    }

    pub fn get(&self, r: usize, c: usize) -> &str {
        let (s, e) = self.row_cells(r);
        if s + c < e { self.cell(s + c) } else { "" }
    }

    pub fn row_is_blank(&self, r: usize) -> bool {
        let (s, e) = self.row_cells(r);
        (s..e).all(|i| self.cell(i).is_empty())
    }

    /// n 行空行（空白行占位）。
    /// 收掉增长时多分配的容量（解析完一块就不再追加了）。
    pub fn shrink_to_fit(&mut self) {
        self.text.shrink_to_fit();
        self.cell_end.shrink_to_fit();
        self.row_end.shrink_to_fit();
    }

    pub fn empty_rows(n: usize) -> Chunk {
        Chunk { row_end: vec![0; n], ..Default::default() }
    }

    /// 丢弃前 n 行（表头之前的行），返回剩余部分。
    pub fn skip_rows(self, n: usize) -> Chunk {
        if n == 0 {
            return self;
        }
        let mut out = Chunk::default();
        for r in n..self.rows() {
            let (s, e) = self.row_cells(r);
            for i in s..e {
                out.push_cell(self.cell(i));
            }
            out.end_row();
        }
        out
    }

    fn bytes(&self) -> usize {
        self.text.capacity() + self.cell_end.capacity() * 4 + self.row_end.capacity() * 4
    }
}

/// 多块行数据的只读视图；克隆只复制 Arc。
#[derive(Default, Clone)]
pub struct ChunkStore {
    chunks: Vec<Arc<Chunk>>,
    starts: Vec<usize>,
    rows: usize,
    width: usize,
}

impl ChunkStore {
    pub fn push(&mut self, c: Chunk) {
        if c.rows() == 0 {
            return;
        }
        self.starts.push(self.rows);
        self.rows += c.rows();
        self.width = self.width.max(c.width);
        self.chunks.push(Arc::new(c));
    }

    pub fn rows(&self) -> usize {
        self.rows
    }

    pub fn width(&self) -> usize {
        self.width
    }

    pub fn bytes(&self) -> usize {
        self.chunks.iter().map(|c| c.bytes()).sum()
    }

    fn locate(&self, row: usize) -> (usize, usize) {
        let ci = self.starts.partition_point(|&s| s <= row) - 1;
        (ci, row - self.starts[ci])
    }

    fn scan(&self, start: usize, len: usize, cols: &[usize], f: &mut RowFn) {
        let end = (start + len).min(self.rows);
        if start >= end {
            return;
        }
        let mut buf: Vec<&str> = Vec::with_capacity(cols.len());
        let (mut ci, mut local) = self.locate(start);
        let mut row = start;
        while row < end {
            let ch = &self.chunks[ci];
            let take = (ch.rows() - local).min(end - row);
            for r in local..local + take {
                buf.clear();
                buf.extend(cols.iter().map(|&c| ch.get(r, c)));
                f(row + (r - local), &buf);
            }
            row += take;
            ci += 1;
            local = 0;
        }
    }

    fn window(&self, start: usize, len: usize, width: usize) -> Vec<Vec<String>> {
        let cols: Vec<usize> = (0..width).collect();
        let mut out = Vec::with_capacity(len.min(self.rows.saturating_sub(start)));
        self.scan(start, len, &cols, &mut |_, cells| {
            out.push(cells.iter().map(|s| (*s).to_owned()).collect())
        });
        out
    }
}

// ---------------------------------------------------------------- Excel

/// 把单元格文本写入 out（与 Excel 显示一致：整数值不带 .0）。
fn write_cell(data: &calamine::Data, out: &mut String) {
    use calamine::Data;
    use std::fmt::Write;
    match data {
        Data::Empty => {}
        Data::String(s) => out.push_str(s),
        Data::Float(f) => out.push_str(&format_float(*f)),
        Data::Int(i) => write_int_text(*i as i128, out),
        Data::Bool(b) => out.push_str(if *b { "TRUE" } else { "FALSE" }),
        // calamine 的 ExcelDateTime 打印出来是序列号（45122），照日期写
        Data::DateTime(dt) => {
            let v = dt.as_f64();
            let is_1904 = (0.0..EXCEL_MAX_SERIAL).contains(&v) && excel_1904(dt);
            write_excel_datetime(v, dt.is_duration(), is_1904, out);
        }
        Data::DateTimeIso(s) => push_iso_datetime(s, out),
        Data::DurationIso(s) => out.push_str(s),
        Data::Error(e) => {
            let _ = write!(out, "{e:?}");
        }
    }
}

fn cell_to_string(data: &calamine::Data) -> String {
    let mut s = String::new();
    write_cell(data, &mut s);
    s
}

/// 数字写成文字，和 Excel 的“常规”格式、polars 打印表格一样：整数不带 .0；最多 15 位有效数字
/// （公式算出来的 1234.5600000000002 写成 1234.56）；很大、很小的用指数（1e20、1e-7）。
/// 写法受显示设置（DISPLAY）影响：浮点模式、千位分隔符。
pub(crate) fn format_float(f: f64) -> String {
    format_float_with(f, &DISPLAY.read().unwrap())
}

fn format_float_with(f: f64, cfg: &DisplayFmt) -> String {
    let s = match cfg.float_mode {
        1 => format!("{f}"),
        2 => format_fixed(f, cfg.float_digits.clamp(0, 10) as usize, cfg.trim_zeros),
        _ => format_smart(f),
    };
    if cfg.thousands { thousands_sep(&s) } else { s }
}

/// 智能（默认）：和 Excel 的“常规”格式一样。
fn format_smart(f: f64) -> String {
    if f.is_finite() && f.fract() == 0.0 && f.abs() < 1e15 {
        return format!("{}", f as i64);
    }
    if !f.is_finite() {
        return format!("{f}");
    }
    let plain = |v: f64| (1e-5..1e15).contains(&v.abs());
    if plain(f) {
        // 常见的情况：最短的写法本来就不超过 15 位，不用再取整
        let s = format!("{f}");
        if significant_digits(&s) <= 15 {
            return s;
        }
    }
    let rounded: f64 = format!("{f:.14e}").parse().unwrap_or(f);
    if plain(rounded) { format!("{rounded}") } else { format!("{rounded:e}") }
}

/// 固定小数位数；trim 时去掉末尾的 0（1.50 → 1.5，1.00 → 1）。
fn format_fixed(f: f64, digits: usize, trim: bool) -> String {
    let mut s = format!("{:.*}", digits, f);
    if trim {
        while s.ends_with('0') {
            s.pop();
        }
        if s.ends_with('.') || s == "-" {
            s.pop();
        }
        if s.is_empty() {
            s.push('0');
        }
    }
    s
}

/// 千位分隔符：1234.56 → 1,234.56（只分整数部分；不是纯数字的照原样）。
pub(crate) fn thousands_sep(s: &str) -> String {
    if !s.bytes().all(|b| b.is_ascii_digit() || b == b'.' || b == b'-') {
        return s.to_string();
    }
    let (int_part, frac) = match s.split_once('.') {
        Some((a, b)) => (a, Some(b)),
        None => (s, None),
    };
    let (sign, digits) = int_part.split_at(usize::from(int_part.starts_with('-')));
    let mut out = String::with_capacity(s.len() + digits.len() / 3);
    out.push_str(sign);
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    if let Some(frac) = frac {
        out.push('.');
        out.push_str(frac);
    }
    out
}

/// 整数写成文字；开了千位分隔符就分组（照 polars 打印整数也分组）。
pub(crate) fn write_int_text(i: i128, out: &mut String) {
    use std::fmt::Write;
    if DISPLAY.read().unwrap().thousands {
        out.push_str(&thousands_sep(&i.to_string()));
    } else {
        let _ = write!(out, "{i}");
    }
}

/// 开没开千位分隔符（xlsx 快速路径、列宽取样、按值筛选要用）。
pub(crate) fn thousands_enabled() -> bool {
    DISPLAY.read().unwrap().thousands
}

// ---------------------------------------------------------------- 显示设置

/// 表格显示的格式设置（设置 → 显示格式…，照 polars 的显示项 pl.Config）。
/// 存在 settings.json 的 "display" 下；渲染在后台线程也跑，所以放全局（DISPLAY）。
#[derive(Clone, Debug)]
pub(crate) struct DisplayFmt {
    /// 浮点数：0 智能（和 Excel 常规格式一样）/ 1 完整 / 2 固定小数位
    pub(crate) float_mode: i32,
    /// 固定小数模式的位数
    pub(crate) float_digits: i32,
    /// 整数部分加千位分隔符（1234.56 → 1,234.56）
    pub(crate) thousands: bool,
    /// 固定小数模式去掉小数末尾的 0
    pub(crate) trim_zeros: bool,
    /// 一格最多显示多少字符
    pub(crate) max_cell: usize,
    /// 空值显示的文字（parquet / 节点结果的空值；要 'static，见 with_null）
    pub(crate) null: &'static str,
}

impl Default for DisplayFmt {
    fn default() -> Self {
        DisplayFmt {
            float_mode: 0,
            float_digits: 2,
            thousands: false,
            trim_zeros: false,
            max_cell: CELL_MAX_DEFAULT,
            null: "null",
        }
    }
}

impl DisplayFmt {
    /// 空值文字漏进静态存储（arrow 的 FormatOptions 要 'static）。
    pub(crate) fn with_null(mut self, null: &str) -> Self {
        self.null = Box::leak(null.to_string().into_boxed_str());
        self
    }

    /// settings.json 的 "display"（缺的项用默认）。
    pub(crate) fn from_json(v: &Value) -> Self {
        let d = DisplayFmt::default();
        DisplayFmt {
            float_mode: v["float_mode"].as_i64().map_or(d.float_mode, |m| m.clamp(0, 2) as i32),
            float_digits: v["float_digits"].as_i64().map_or(d.float_digits, |n| n.clamp(0, 10) as i32),
            thousands: v["thousands"].as_bool().unwrap_or(d.thousands),
            trim_zeros: v["trim_zeros"].as_bool().unwrap_or(d.trim_zeros),
            max_cell: v["max_cell"].as_u64().map_or(d.max_cell, |n| n.clamp(10, 100_000) as usize),
            null: v["null"].as_str().map_or(d.null, |s| Box::leak(s.to_string().into_boxed_str())),
        }
    }

    pub(crate) fn to_json(&self) -> Value {
        json!({
            "float_mode": self.float_mode,
            "float_digits": self.float_digits,
            "thousands": self.thousands,
            "trim_zeros": self.trim_zeros,
            "max_cell": self.max_cell,
            "null": self.null,
        })
    }
}

static DISPLAY: RwLock<DisplayFmt> = RwLock::new(DisplayFmt {
    float_mode: 0,
    float_digits: 2,
    thousands: false,
    trim_zeros: false,
    max_cell: CELL_MAX_DEFAULT,
    null: "null",
});

/// 当前显示设置（对话框打开时推进界面、退出时存盘要读）。
pub(crate) fn display_fmt() -> DisplayFmt {
    DISPLAY.read().unwrap().clone()
}

/// 换显示设置（启动时从 settings.json 套上、对话框点“确定”时应用）。
pub(crate) fn set_display_fmt(cfg: DisplayFmt) {
    *DISPLAY.write().unwrap() = cfg;
}

/// 不带指数的数字文字里有几位有效数字（前导的 0 不算）。
fn significant_digits(s: &str) -> usize {
    s.trim_start_matches(['-', '0', '.']).bytes().filter(u8::is_ascii_digit).count()
}

/// Excel 的序列号最大到 9999-12-31。
const EXCEL_MAX_SERIAL: f64 = 2_958_466.0;

/// Excel 的日期时间（序列号：天数 + 一天里的比例）写成文字，照 polars 的写法：整天 2023-07-15，
/// 有时间 2023-07-15 12:30:00（毫秒不是 0 才写 .123），只有时间（整数部分是 0）12:30:00；
/// 时长照 Excel 的 [h]:mm:ss 写（36:00:00）。负数、超出 Excel 范围的照数字写。
pub(crate) fn write_excel_datetime(serial: f64, duration: bool, is_1904: bool, out: &mut String) {
    use calamine::{ExcelDateTime, ExcelDateTimeType};
    use std::fmt::Write;
    const DAY_MS: i64 = 86_400_000;
    if !(0.0..EXCEL_MAX_SERIAL).contains(&serial) {
        out.push_str(&format_float(serial));
        return;
    }
    // 先取整到毫秒再拆成天和一天里的毫秒：x.99999999 进位到第二天，不会写出 24:00:00
    let total = (serial * DAY_MS as f64).round() as i64;
    let (days, ms) = (total / DAY_MS, total % DAY_MS);
    let clock = |out: &mut String, hours: i64| {
        let _ = write!(out, "{hours:02}:{:02}:{:02}", ms / 60_000 % 60, ms / 1000 % 60);
        if ms % 1000 != 0 {
            let _ = write!(out, ".{:03}", ms % 1000);
        }
    };
    if duration {
        clock(out, days * 24 + ms / 3_600_000);
    } else if days == 0 {
        clock(out, ms / 3_600_000);
    } else {
        // 年月日交给 calamine：1904 纪元、1900 年的闰年错误（1900-02-29）它都处理了
        let (y, m, d, ..) = ExcelDateTime::new(days as f64, ExcelDateTimeType::DateTime, is_1904).to_ymd_hms_milli();
        let _ = write!(out, "{y:04}-{m:02}-{d:02}");
        if ms != 0 {
            out.push(' ');
            clock(out, ms / 3_600_000);
        }
    }
}

/// calamine 的 ExcelDateTime 不给 1904 纪元的标记：同一个数按 1900 纪元再算一遍，日期对不上就是 1904 的。
fn excel_1904(dt: &calamine::ExcelDateTime) -> bool {
    let as_1900 = calamine::ExcelDateTime::new(dt.as_f64(), calamine::ExcelDateTimeType::DateTime, false);
    dt.is_datetime() && as_1900.to_ymd_hms_milli() != dt.to_ymd_hms_milli()
}

/// ISO 8601 的日期时间（ods、xlsx 的 t="d"）：日期和时间之间的 T 换成空格，和 polars 的写法一样。
pub(crate) fn push_iso_datetime(s: &str, out: &mut String) {
    match s.as_bytes().get(10) {
        Some(b'T') if s.as_bytes()[..4].iter().all(u8::is_ascii_digit) => {
            out.push_str(&s[..10]);
            // 整天（时间是 00:00:00）只写日期，和序列号日期的写法一致
            if s.len() != 19 || s[11..] != *"00:00:00" {
                out.push(' ');
                out.push_str(&s[11..]);
            }
        }
        _ => out.push_str(s),
    }
}

/// Excel：工作簿句柄共享给所有 sheet，每个 sheet 首次访问时才解码进内存。
type SharedBook = Arc<Mutex<calamine::Sheets<std::io::BufReader<std::fs::File>>>>;

struct ExcelSheet {
    store: ChunkStore,
    headers: Vec<String>,
    width: usize,
}

enum ExcelState {
    /// 尚未解码（持锁期间表示解码中）
    Pending,
    Ready(Arc<ExcelSheet>),
    Failed(String),
}

struct ExcelSource {
    name: String,
    book: SharedBook,
    state: Mutex<ExcelState>,
    headers: OnceLock<Vec<String>>,
}

impl ExcelSource {
    /// 取得解码后的状态；若尚未解码则在本次调用中解码（并发时只有一个线程执行，其余等待）。
    fn ensure_loaded(&self) -> MutexGuard<'_, ExcelState> {
        let mut st = self.state.lock().unwrap();
        if matches!(*st, ExcelState::Pending) {
            *st = match self.decode() {
                Ok(sheet) => ExcelState::Ready(Arc::new(sheet)),
                Err(e) => ExcelState::Failed(e),
            };
        }
        st
    }

    /// 已解码的 sheet 快照（不持锁读取，并行扫描不争锁）。
    fn sheet(&self) -> Option<Arc<ExcelSheet>> {
        match &*self.ensure_loaded() {
            ExcelState::Ready(s) => Some(s.clone()),
            _ => None,
        }
    }

    fn decode(&self) -> Result<ExcelSheet, String> {
        use calamine::Reader;
        use rayon::prelude::*;

        let mut book = self.book.lock().unwrap();
        let range = book
            .worksheet_range(&self.name)
            .map_err(|e| format!("sheet “{}”: {e}", self.name))?;
        drop(book);

        let (width, height) = (range.width(), range.height());
        if width == 0 || height == 0 {
            return Ok(ExcelSheet { store: ChunkStore::default(), headers: Vec::new(), width: 0 });
        }
        // 行间互不依赖，按块并行转成紧凑存储（calamine 的解析本身解不出来，能并行的就是这段转换）
        let rows: Vec<&[calamine::Data]> = range.rows().collect();
        let headers: Vec<String> = rows
            .first()
            .map(|r| r.iter().map(cell_to_string).collect())
            .unwrap_or_default();
        let chunks: Vec<Chunk> = rows[1.min(rows.len())..]
            .par_chunks(65_536)
            .map(|rows| {
                let mut chunk = Chunk::default();
                for row in rows {
                    for cell in *row {
                        write_cell(cell, chunk.text_mut());
                        chunk.end_cell();
                    }
                    chunk.end_row();
                }
                chunk
            })
            .collect();
        let mut store = ChunkStore::default();
        for chunk in chunks {
            store.push(chunk);
        }
        Ok(ExcelSheet { store, headers, width })
    }
}

impl DataSource for ExcelSource {
    fn headers(&self) -> &[String] {
        self.headers.get_or_init(|| self.sheet().map(|s| s.headers.clone()).unwrap_or_default())
    }

    fn row_count(&self) -> usize {
        self.sheet().map_or(0, |s| s.store.rows())
    }

    fn window(&self, start: usize, len: usize) -> Vec<Vec<String>> {
        match self.sheet() {
            Some(s) => s.store.window(start, len, s.width),
            None => Vec::new(),
        }
    }

    fn scan(&self, start: usize, len: usize, cols: &[usize], f: &mut RowFn) {
        if let Some(s) = self.sheet() {
            s.store.scan(start, len, cols, f);
        }
    }

    fn is_ready(&self) -> bool {
        match self.state.try_lock() {
            Ok(st) => matches!(*st, ExcelState::Ready(_) | ExcelState::Failed(_)),
            Err(_) => false, // 解码中
        }
    }

    fn prepare(&self) -> Result<(), String> {
        match &*self.ensure_loaded() {
            ExcelState::Ready(_) => Ok(()),
            ExcelState::Failed(e) => Err(e.clone()),
            ExcelState::Pending => Ok(()),
        }
    }
}

/// xls/xlsb/ods：calamine 不支持流式，保持整 sheet 解码（懒到首次访问）。
fn open_excel_legacy(path: &Path) -> Result<Vec<SheetHandle>, LoadError> {
    use calamine::{open_workbook_auto, Reader};

    let book = open_workbook_auto(path).map_err(|e| LoadError::Format(e.to_string()))?;
    let names: Vec<String> = book.sheet_names().to_vec();
    let book = Arc::new(Mutex::new(book));

    let mut sheets = Vec::new();
    let mut eager_done = false;
    for name in names {
        let source = ExcelSource {
            name: name.clone(),
            book: book.clone(),
            state: Mutex::new(ExcelState::Pending),
            headers: OnceLock::new(),
        };
        if !eager_done {
            // 首个有数据的 sheet 立即解码（用户马上要看，且此处在加载线程上）；其余懒解码
            source.prepare().map_err(LoadError::Format)?;
            let nonempty = source.sheet().is_some_and(|s| s.store.rows() > 0 && s.width > 0);
            if nonempty {
                sheets.push(SheetHandle { name, source: Arc::new(source) });
                eager_done = true;
            }
            // 空 sheet 跳过（空工作簿解析很便宜，继续尝试下一个）
        } else {
            sheets.push(SheetHandle { name, source: Arc::new(source) });
        }
    }

    if sheets.is_empty() {
        return Err(LoadError::Format(String::from("工作簿中没有数据")));
    }
    Ok(sheets)
}

// ---------------------------------------------------------------- xlsx 流式解码

/// 首块行数（决定首屏出现的速度）。
const XLSX_FIRST_CHUNK: usize = 1_000;
/// 后续每块行数（每块解码完通知 UI 更新一次）。
const XLSX_CHUNK_ROWS: usize = 100_000;
/// UI 线程等待首块的超时（首块约几毫秒，超时说明 worker 出了问题）。
const XLSX_UI_WAIT: Duration = Duration::from_secs(5);
/// 后台 prepare 等待首块的超时（多 sheet 排队时可能要等前一个解码完）。
const XLSX_PREPARE_WAIT: Duration = Duration::from_secs(300);

type XlsxReader = calamine::Xlsx<std::io::BufReader<std::fs::File>>;

/// 共享工作簿：只有一个 worker 线程持有 book 锁，按队列逐 sheet 流式解码。
struct XlsxBook {
    engine: XlsxEngine,
    queue: Mutex<Vec<Arc<XlsxSource>>>,
    cv: Condvar,
    progress: ProgressFn,
}

enum XlsxEngine {
    /// 自研并行解析（常规文件）；共享字符串和样式第一次解码时读
    Fast { path: std::path::PathBuf, sst: OnceLock<Vec<String>>, styles: OnceLock<crate::xlsx_fast::Styles> },
    /// calamine 兜底（结构异常的文件）
    Calamine(Mutex<XlsxReader>),
}

struct XlsxSource {
    name: String,
    part: String, // Fast 引擎：sheet 在 zip 内的路径
    book: Arc<XlsxBook>,
    data: Mutex<XlsxData>,
    cv: Condvar,
    headers: OnceLock<Vec<String>>,
    self_ref: OnceLock<Weak<XlsxSource>>,
}

#[derive(Default)]
struct XlsxData {
    headers: Vec<String>,
    store: Arc<ChunkStore>, // 表头之后的数据行；读者持 Arc 快照，追加时写时复制
    first_chunk: bool,      // 表头已确定（或 sheet 已确认无数据）
    notified_rows: usize,   // 上次通知 UI 时的行数
    started: bool,          // 已进入解码队列
    done: bool,
    error: Option<String>,
}

impl XlsxSource {
    /// 首次访问时把 sheet 排入解码队列。
    fn ensure_started(&self) {
        {
            let mut d = self.data.lock().unwrap();
            if d.started || d.done || d.error.is_some() {
                return;
            }
            d.started = true;
        }
        if let Some(this) = self.self_ref.get().and_then(|w| w.upgrade()) {
            self.book.queue.lock().unwrap().push(this);
            self.book.cv.notify_one();
        }
    }

    /// 等待首块解码完成（表头可确定）。返回 false 表示超时。
    fn wait_first(&self, timeout: Duration) -> bool {
        self.ensure_started();
        let deadline = Instant::now() + timeout;
        let mut d = self.data.lock().unwrap();
        while !d.first_chunk {
            let now = Instant::now();
            if now >= deadline {
                return false;
            }
            d = self.cv.wait_timeout(d, deadline - now).unwrap().0;
        }
        true
    }

    /// 已解码数据的快照（锁内只克隆一个 Arc）。
    fn snapshot(&self) -> Option<(Arc<ChunkStore>, usize)> {
        if !self.wait_first(XLSX_UI_WAIT) {
            return None;
        }
        let d = self.data.lock().unwrap();
        Some((d.store.clone(), d.headers.len()))
    }
}

impl DataSource for XlsxSource {
    fn headers(&self) -> &[String] {
        if self.wait_first(XLSX_UI_WAIT) {
            self.headers.get_or_init(|| self.data.lock().unwrap().headers.clone())
        } else {
            // 超时不缓存空表头，之后仍有机会拿到真实表头
            &[]
        }
    }

    fn row_count(&self) -> usize {
        self.snapshot().map_or(0, |(s, _)| s.rows())
    }

    fn window(&self, start: usize, len: usize) -> Vec<Vec<String>> {
        match self.snapshot() {
            Some((s, h)) => s.window(start, len, s.width().max(h)),
            None => Vec::new(),
        }
    }

    fn scan(&self, start: usize, len: usize, cols: &[usize], f: &mut RowFn) {
        if let Some((s, _)) = self.snapshot() {
            s.scan(start, len, cols, f);
        }
    }

    fn is_ready(&self) -> bool {
        self.data.lock().unwrap().first_chunk
    }

    fn is_loading(&self) -> bool {
        let d = self.data.lock().unwrap();
        !d.done && d.error.is_none()
    }

    fn load_error(&self) -> Option<String> {
        self.data.lock().unwrap().error.clone()
    }

    fn prepare(&self) -> Result<(), String> {
        if !self.wait_first(XLSX_PREPARE_WAIT) {
            return Err(String::from("解码超时"));
        }
        match &self.data.lock().unwrap().error {
            Some(e) => Err(e.clone()),
            None => Ok(()),
        }
    }
}

/// worker 主循环：逐个解码队列中的 sheet。
fn xlsx_worker(book: Arc<XlsxBook>) {
    loop {
        let src = {
            let mut q = book.queue.lock().unwrap();
            loop {
                if q.is_empty() {
                    q = book.cv.wait(q).unwrap();
                } else {
                    break q.remove(0);
                }
            }
        };
        let result = xlsx_decode_sheet(&src);
        {
            let mut d = src.data.lock().unwrap();
            d.done = true;
            if let Err(e) = result {
                d.error = Some(e);
            }
            d.first_chunk = true;
        }
        src.cv.notify_all();
        let any: Arc<dyn DataSource> = src.clone();
        (book.progress)(&any);
    }
}

/// 把行列表转成一个 Chunk。
fn rows_to_chunk(rows: &mut Vec<Vec<String>>) -> Chunk {
    let mut c = Chunk::default();
    for row in rows.drain(..) {
        for cell in &row {
            c.push_cell(cell);
        }
        c.end_row();
    }
    c
}

/// calamine 兜底：流式解码一个 sheet，按块追加到共享数据，首块完成即通知 UI。
fn xlsx_decode_sheet(src: &XlsxSource) -> Result<(), String> {
    use calamine::{Data, DataRef};

    let mut book = match &src.book.engine {
        XlsxEngine::Fast { path, sst, styles } => return xlsx_decode_fast(src, path, sst, styles),
        XlsxEngine::Calamine(b) => b.lock().unwrap(),
    };
    let mut reader = book
        .worksheet_cells_reader(&src.name)
        .map_err(|e| format!("sheet “{}”: {e}", src.name))?;

    let mut cur: Vec<String> = Vec::new(); // 正在组装的行
    let mut last_row = 0u32;
    let mut have_row = false;
    let mut header: Option<Vec<String>> = None;
    let mut pending: Vec<Vec<String>> = Vec::new();
    let mut width = 0usize;
    let mut first_sent = false;

    // 收尾当前行：确定表头或作为数据行追加
    macro_rules! finish_row {
        () => {
            if header.is_none() {
                if cur.iter().any(|c| !c.is_empty()) {
                    width = cur.len();
                    header = Some(cur.clone());
                }
                // 表头之前的行（含空行）丢弃
            } else {
                width = width.max(cur.len());
                pending.push(std::mem::take(&mut cur));
            }
        };
    }

    loop {
        let cell = match reader.next_cell() {
            Ok(Some(c)) => c,
            Ok(None) => break,
            Err(e) => return Err(format!("sheet “{}”: {e}", src.name)),
        };
        let (r, c) = cell.get_position();
        if !have_row || r != last_row {
            if have_row {
                finish_row!();
                // 表头之后的整行空白用空行占位，保持行号一致
                if header.is_some() {
                    for _ in (last_row + 1)..r {
                        pending.push(Vec::new());
                    }
                }
            }
            last_row = r;
            have_row = true;
            cur.clear();
        }
        let val = cell.get_value().clone();
        let text = if matches!(val, DataRef::Empty) {
            String::new()
        } else {
            cell_to_string(&Data::from(val))
        };
        while cur.len() < c as usize {
            cur.push(String::new());
        }
        cur.push(text);

        // 首块：表头 + 少量行，尽快让 UI 出现内容；之后按大块通知
        if header.is_some() {
            let target = if first_sent { XLSX_CHUNK_ROWS } else { XLSX_FIRST_CHUNK };
            if pending.len() >= target {
                xlsx_append(src, &header, vec![rows_to_chunk(&mut pending)]);
                first_sent = true;
            }
        }
    }
    if have_row {
        finish_row!();
    }
    let _ = width;
    xlsx_append(src, &header, vec![rows_to_chunk(&mut pending)]);
    Ok(())
}

/// 快速引擎：解压线程流式切块 → rayon 并行解析成紧凑块 → 顺序合并（表头/空行占位）。
/// 解压（单线程、无法并行）与解析重叠进行。
fn xlsx_decode_fast(
    src: &XlsxSource,
    path: &Path,
    sst: &OnceLock<Vec<String>>,
    styles: &OnceLock<crate::xlsx_fast::Styles>,
) -> Result<(), String> {
    use crate::xlsx_fast as xf;
    use rayon::prelude::*;

    let mut ar = xf::open_archive(path)?;
    let sst = sst.get_or_init(|| xf::read_shared_strings(&mut ar));
    let styles = styles.get_or_init(|| xf::read_styles(&mut ar));
    let entry = ar.by_name(&src.part).map_err(|e| format!("sheet “{}”: {e}", src.name))?;

    // 同时在内存里的原始块（4 MB 一块，通道里和正在解析的各 batch 块）：线程再多也不用超过 8，
    // 解压是单线程的，8 个线程解析已经跟得上，多了只是白占内存
    let batch = rayon::current_num_threads().clamp(2, 8);
    let (tx, rx) = std::sync::mpsc::sync_channel::<Vec<u8>>(batch);
    std::thread::scope(|scope| {
        scope.spawn(move || {
            // 首块只取一小块，让首屏尽快出现
            let mut blocks = xf::BlockReader::new(entry);
            let mut size = 128 * 1024;
            while let Some(b) = blocks.next_block(size) {
                size = 4 * 1024 * 1024;
                if tx.send(b).is_err() {
                    break;
                }
            }
        });

        let mut header: Option<Vec<String>> = None;
        let mut last_row: Option<u32> = None;
        // 每轮：阻塞取一块，再把已就绪的块一起取走并行解析
        while let Ok(first) = rx.recv() {
            let mut raw = vec![first];
            while raw.len() < batch {
                match rx.try_recv() {
                    Ok(b) => raw.push(b),
                    Err(_) => break,
                }
            }
            let parsed: Vec<xf::Parsed> = raw.par_iter().map(|b| xf::parse_block(b, sst, styles)).collect();
            drop(raw);
            let mut out: Vec<Chunk> = Vec::new();
            for p in parsed {
                let n = p.chunk.rows();
                if n == 0 {
                    continue;
                }
                let base = p.first.unwrap_or_else(|| last_row.map_or(0, |l| l + 1));
                let last = p.last.unwrap_or(base + n as u32 - 1);
                if header.is_none() {
                    // 表头之前的行（含空行）丢弃
                    if let Some(h) = (0..n).find(|&r| !p.chunk.row_is_blank(r)) {
                        header = Some((0..p.chunk.row_len(h)).map(|c| p.chunk.get(h, c).to_owned()).collect());
                        out.push(p.chunk.skip_rows(h + 1));
                    }
                } else {
                    // 块之间的整行空白用空行占位，保持行号一致（块内由 parse_block 补齐）
                    if let Some(l) = last_row {
                        if base > l + 1 {
                            out.push(Chunk::empty_rows((base - l - 1) as usize));
                        }
                    }
                    out.push(p.chunk);
                }
                last_row = Some(last);
            }
            if header.is_some() {
                xlsx_append(src, &header, out);
            }
        }
        xlsx_append(src, &header, Vec::new());
    });
    Ok(())
}

/// 把新块并入共享数据；通知等待者与 UI。
fn xlsx_append(src: &XlsxSource, header: &Option<Vec<String>>, chunks: Vec<Chunk>) {
    let mut d = src.data.lock().unwrap();
    let first = !d.first_chunk;
    if first {
        if let Some(h) = header {
            d.headers = h.clone();
        }
        d.first_chunk = true;
    }
    if chunks.iter().any(|c| c.rows() > 0) {
        let store = Arc::make_mut(&mut d.store);
        for c in chunks {
            store.push(c);
        }
    }
    // 首块立即通知；之后约每 XLSX_CHUNK_ROWS 行通知一次（完成时由 worker 再通知）
    let rows = d.store.rows();
    let notify = first || rows >= d.notified_rows + XLSX_CHUNK_ROWS;
    if notify {
        d.notified_rows = rows;
    }
    drop(d);
    src.cv.notify_all();
    if notify {
        if let Some(this) = src.self_ref.get().and_then(|w| w.upgrade()) {
            let any: Arc<dyn DataSource> = this;
            (src.book.progress)(&any);
        }
    }
}

/// xlsx/xlsm：打开工作簿，启动 worker 线程流式解码，立即返回句柄。
fn open_xlsx_streaming(path: &Path, progress: ProgressFn) -> Result<Vec<SheetHandle>, LoadError> {
    open_xlsx_with(path, progress, true)
}

pub(crate) fn open_xlsx_with(path: &Path, progress: ProgressFn, allow_fast: bool) -> Result<Vec<SheetHandle>, LoadError> {
    use calamine::Reader;

    // 优先自研快速引擎；结构异常（无 sheet、带命名空间前缀等）回退 calamine
    let fast = allow_fast.then(|| {
        let mut ar = crate::xlsx_fast::open_archive(path).ok()?;
        let parts = crate::xlsx_fast::sheet_parts(&mut ar).ok()?;
        let first = parts.first()?;
        crate::xlsx_fast::probe(&mut ar, &first.1).then_some(parts)
    }).flatten();

    let (engine, entries): (XlsxEngine, Vec<(String, String)>) = match fast {
        Some(parts) => (
            XlsxEngine::Fast { path: path.to_path_buf(), sst: OnceLock::new(), styles: OnceLock::new() },
            parts,
        ),
        None => {
            let file = std::fs::File::open(path).map_err(LoadError::Io)?;
            let book = XlsxReader::new(std::io::BufReader::new(file))
                .map_err(|e| LoadError::Format(e.to_string()))?;
            let names: Vec<(String, String)> =
                book.sheet_names().iter().map(|n| (n.clone(), String::new())).collect();
            (XlsxEngine::Calamine(Mutex::new(book)), names)
        }
    };
    if entries.is_empty() {
        return Err(LoadError::Format(String::from("工作簿中没有数据")));
    }

    let shared = Arc::new(XlsxBook {
        engine,
        queue: Mutex::new(Vec::new()),
        cv: Condvar::new(),
        progress,
    });
    std::thread::spawn({
        let shared = shared.clone();
        move || xlsx_worker(shared)
    });

    let mut sheets = Vec::new();
    for (i, (name, part)) in entries.into_iter().enumerate() {
        let src = Arc::new(XlsxSource {
            name: name.clone(),
            part,
            book: shared.clone(),
            data: Mutex::new(XlsxData::default()),
            cv: Condvar::new(),
            headers: OnceLock::new(),
            self_ref: OnceLock::new(),
        });
        src.self_ref.set(Arc::downgrade(&src)).ok();
        if i == 0 {
            src.ensure_started(); // 首个 sheet 立即开始解码
        }
        sheets.push(SheetHandle { name, source: src });
    }
    Ok(sheets)
}

// ---------------------------------------------------------------- CSV / TSV

/// CSV 的文字编码：UTF-8，或者 Windows 的代码页（中文 Windows 上 Excel“另存为 CSV”默认是 GBK）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum TextEncoding {
    Utf8,
    CodePage(u32),
}

impl TextEncoding {
    /// 前 1 MB 不是 UTF-8（末尾截断的半个字不算）就按系统的 ANSI 代码页。
    fn detect(bytes: &[u8]) -> TextEncoding {
        match std::str::from_utf8(&bytes[..bytes.len().min(1 << 20)]) {
            Err(e) if e.error_len().is_some() => codepage::ansi().map_or(TextEncoding::Utf8, TextEncoding::CodePage),
            _ => TextEncoding::Utf8,
        }
    }

    /// 一个字段的文字：是 UTF-8 的照原样（GBK 文件里的纯 ASCII 字段也是），否则按这个编码解。
    fn text(self, bytes: &[u8]) -> String {
        match (std::str::from_utf8(bytes), self) {
            (Ok(s), _) => s.to_owned(),
            (Err(_), TextEncoding::CodePage(cp)) => codepage::decode(cp, bytes),
            (Err(_), TextEncoding::Utf8) => String::from_utf8_lossy(bytes).into_owned(),
        }
    }

    /// read 节点 encoding 参数的值（polars 用 Python 的 gb18030 解码）；别的代码页节点里没有对应的选项，退回有损 UTF-8。
    fn engine_name(self) -> &'static str {
        match self {
            TextEncoding::Utf8 => "utf8",
            TextEncoding::CodePage(936 | 54936) => "gb18030",
            TextEncoding::CodePage(_) => "utf8-lossy",
        }
    }
}

/// Windows 的代码页转换，直接调 Win32（不引入依赖，同 clipboard.rs）；别的系统没有 ANSI 代码页，按 UTF-8 有损解。
mod codepage {
    #[cfg(windows)]
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetACP() -> u32;
        fn MultiByteToWideChar(cp: u32, flags: u32, src: *const u8, src_len: i32, dst: *mut u16, dst_len: i32) -> i32;
    }

    /// 系统的 ANSI 代码页。936（GBK）换成它的超集 54936（GB18030）；系统设成 UTF-8（65001）时
    /// 不是 UTF-8 的文件最可能还是 GBK 的，也按 GB18030。
    #[cfg(windows)]
    pub fn ansi() -> Option<u32> {
        match unsafe { GetACP() } {
            936 | 65001 => Some(54936),
            cp => Some(cp),
        }
    }

    #[cfg(not(windows))]
    pub fn ansi() -> Option<u32> {
        None
    }

    #[cfg(windows)]
    pub fn decode(cp: u32, bytes: &[u8]) -> String {
        let len = bytes.len().min(i32::MAX as usize) as i32;
        if len == 0 {
            return String::new();
        }
        // SAFETY: 先问要多少个 u16，再按这个长度分配后转换
        unsafe {
            let n = MultiByteToWideChar(cp, 0, bytes.as_ptr(), len, std::ptr::null_mut(), 0);
            if n <= 0 {
                return String::from_utf8_lossy(bytes).into_owned();
            }
            let mut wide = vec![0u16; n as usize];
            let n = MultiByteToWideChar(cp, 0, bytes.as_ptr(), len, wide.as_mut_ptr(), n);
            String::from_utf16_lossy(&wide[..n.max(0) as usize])
        }
    }

    #[cfg(not(windows))]
    pub fn decode(_cp: u32, bytes: &[u8]) -> String {
        String::from_utf8_lossy(bytes).into_owned()
    }
}

/// .csv 的分隔符：第一行里（引号外）没有逗号、却有分号 / 制表符 / 竖线时，用其中最多的那个。
/// 数的是解好码的字（GBK 汉字的第二个字节可能是 0x7C，就是竖线）。
fn sniff_delimiter(map: &[u8], encoding: TextEncoding) -> u8 {
    // 第一行到引号外的第一个换行（GBK 的字节里不会出现换行和引号）
    let mut quoted = false;
    let end = (map.iter())
        .position(|&b| {
            quoted ^= b == b'"';
            b == b'\n' && !quoted
        })
        .unwrap_or(map.len())
        .min(1 << 16);
    let mut counts = [(b',', 0usize), (b';', 0), (b'\t', 0), (b'|', 0)];
    let mut quoted = false;
    for c in encoding.text(&map[..end]).chars() {
        quoted ^= c == '"';
        if let Some(slot) = counts.iter_mut().find(|(d, _)| !quoted && *d as char == c) {
            slot.1 += 1;
        }
    }
    if counts[0].1 > 0 {
        return b',';
    }
    counts[1..].iter().filter(|(_, n)| *n > 0).max_by_key(|(_, n)| *n).map_or(b',', |(d, _)| *d)
}

/// 单条 CSV 记录解析器（csv-core，零拷贝输入，输出缓冲复用）。
struct RecParser {
    rdr: csv_core::Reader,
    out: Vec<u8>,
    ends: Vec<usize>,
    encoding: TextEncoding,
    owned: Vec<String>, // 非 UTF-8 字段按编码解出来的文字
}

impl RecParser {
    fn new(delimiter: u8, encoding: TextEncoding) -> RecParser {
        RecParser {
            rdr: csv_core::ReaderBuilder::new().delimiter(delimiter).build(),
            out: vec![0; 4096],
            ends: vec![0; 64],
            encoding,
            owned: Vec::new(),
        }
    }

    /// 解析 input 开头的一条记录，返回 (消耗字节数, 字段数)；没有记录返回 None。
    fn read(&mut self, input: &[u8]) -> Option<(usize, usize)> {
        use csv_core::ReadRecordResult as R;
        let (mut nin, mut nout, mut nend) = (0, 0, 0);
        loop {
            let (res, i, o, e) =
                self.rdr.read_record(&input[nin..], &mut self.out[nout..], &mut self.ends[nend..]);
            nin += i;
            nout += o;
            nend += e;
            match res {
                R::InputEmpty => {} // 输入已尽：下一轮传空切片让解析器收尾
                R::OutputFull => self.out.resize(self.out.len() * 2, 0),
                R::OutputEndsFull => self.ends.resize(self.ends.len() * 2, 0),
                R::Record => return Some((nin, nend)),
                R::End => return None,
            }
        }
    }

    /// 取刚解析记录的字段（按 cols 选列，缺列为空）交给 f。
    fn emit(&mut self, nfields: usize, cols: Option<&[usize]>, row: usize, f: &mut RowFn) {
        let bounds = |i: usize| (if i == 0 { 0 } else { self.ends[i - 1] }, self.ends[i]);
        // 非 UTF-8 字段（GBK 文件里的中文）按文件的编码解，存入 owned；lossy[i] 指向其下标
        self.owned.clear();
        let mut lossy: Vec<Option<usize>> = Vec::new();
        for i in 0..nfields {
            let (s, e) = bounds(i);
            if std::str::from_utf8(&self.out[s..e]).is_err() {
                lossy.resize(nfields, None);
                lossy[i] = Some(self.owned.len());
                self.owned.push(self.encoding.text(&self.out[s..e]));
            }
        }
        let get = |i: usize| -> &str {
            if i >= nfields {
                return "";
            }
            if let Some(Some(k)) = lossy.get(i) {
                return &self.owned[*k];
            }
            let (s, e) = bounds(i);
            // SAFETY: 上面已逐字段校验过 UTF-8
            unsafe { std::str::from_utf8_unchecked(&self.out[s..e]) }
        };
        let cells: Vec<&str> = match cols {
            Some(cols) => cols.iter().map(|&c| get(c)).collect(),
            None => (0..nfields).map(get).collect(),
        };
        f(row, &cells);
    }
}

/// CSV 的内容。不太大的整个读进内存：文件不被占着，查看器开着时 Excel、记事本还能保存它；
/// 很大的内存映射（映射期间别的程序存不了这个文件，Windows 不让截断、替换映射着的文件）。
enum FileBytes {
    Owned(Vec<u8>),
    Mapped(memmap2::Mmap),
}

impl std::ops::Deref for FileBytes {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        match self {
            FileBytes::Owned(bytes) => bytes,
            FileBytes::Mapped(map) => map,
        }
    }
}

/// 这么大以内的 CSV 读进内存，再大的内存映射。
const CSV_IN_MEMORY: u64 = 256 << 20;

/// CSV：打开时扫一遍记录偏移；之后任意窗口直接在文件内容上解析。
struct CsvSource {
    map: FileBytes,
    delimiter: u8,
    encoding: TextEncoding,
    offsets: Vec<u64>, // offsets[0] 是表头，offsets[i+1] 是数据行 i
    headers: Vec<String>,
    data_rows: usize,
}

impl CsvSource {
    /// 顺序解析数据行 [start, end)。
    fn parse_range(&self, start: usize, end: usize, cols: Option<&[usize]>, f: &mut RowFn) {
        let end = end.min(self.data_rows);
        if start >= end {
            return;
        }
        let mut p = RecParser::new(self.delimiter, self.encoding);
        let mut pos = self.offsets[start + 1] as usize;
        for row in start..end {
            match p.read(&self.map[pos..]) {
                Some((used, n)) => {
                    pos += used;
                    p.emit(n, cols, row, f);
                }
                None => break,
            }
        }
    }
}

impl DataSource for CsvSource {
    fn headers(&self) -> &[String] {
        &self.headers
    }

    fn row_count(&self) -> usize {
        self.data_rows
    }

    fn window(&self, start: usize, len: usize) -> Vec<Vec<String>> {
        let mut out = Vec::with_capacity(len.min(self.data_rows.saturating_sub(start)));
        self.parse_range(start, start + len, None, &mut |_, cells| {
            out.push(cells.iter().map(|s| (*s).to_owned()).collect())
        });
        out
    }

    fn scan(&self, start: usize, len: usize, cols: &[usize], f: &mut RowFn) {
        self.parse_range(start, start + len, Some(cols), f);
    }

    fn read_hints(&self) -> Vec<(&'static str, String)> {
        let mut hints = Vec::new();
        if self.encoding != TextEncoding::Utf8 {
            hints.push(("encoding", self.encoding.engine_name().to_string()));
        }
        match self.delimiter {
            b',' => {}
            b'\t' => hints.push(("separator", String::from("\\t"))),
            d => hints.push(("separator", (d as char).to_string())),
        }
        hints
    }
}

fn open_csv(path: &Path, tsv: bool) -> Result<Vec<SheetHandle>, LoadError> {
    use std::io::Read;
    let mut file = std::fs::File::open(path).map_err(LoadError::Io)?;
    let len = file.metadata().map_err(LoadError::Io)?.len();
    let map = if len <= CSV_IN_MEMORY {
        let mut bytes = Vec::with_capacity(len as usize);
        file.read_to_end(&mut bytes).map_err(LoadError::Io)?;
        FileBytes::Owned(bytes)
    } else {
        // SAFETY: 只读映射；文件在映射期间被其他进程截断属于外部破坏，Windows 上映射期间也禁止截断
        FileBytes::Mapped(unsafe { memmap2::Mmap::map(&file) }.map_err(LoadError::Io)?)
    };
    drop(file);
    let encoding = TextEncoding::detect(&map);
    let delimiter = if tsv { b'\t' } else { sniff_delimiter(&map, encoding) };

    // 扫描一遍记录偏移（csv-core 直接在映射内存上跑，不分配字符串）
    let mut offsets = Vec::new();
    let mut headers: Option<Vec<String>> = None;
    let mut p = RecParser::new(delimiter, encoding);
    let mut pos = 0usize;
    loop {
        let Some((used, n)) = p.read(&map[pos..]) else { break };
        if headers.is_none() {
            headers = Some(
                (0..n)
                    .map(|i| {
                        let s = if i == 0 { 0 } else { p.ends[i - 1] };
                        encoding.text(&p.out[s..p.ends[i]])
                    })
                    .collect(),
            );
        }
        offsets.push(pos as u64);
        pos += used;
    }

    let mut headers = headers.ok_or_else(|| LoadError::Format(String::from("文件为空")))?;
    // 去掉 UTF-8 BOM
    if let Some(first) = headers.first_mut() {
        if first.starts_with('\u{feff}') {
            *first = first.trim_start_matches('\u{feff}').to_string();
        }
    }
    let data_rows = offsets.len().saturating_sub(1);

    let name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("csv")
        .to_string();
    let source = CsvSource { map, delimiter, encoding, offsets, headers, data_rows };
    Ok(vec![SheetHandle { name, source: Arc::new(source) }])
}

// ---------------------------------------------------------------- Parquet

/// Parquet 解码组缓存上限（紧凑存储的字节数）。
const PARQUET_CACHE_BYTES: usize = 512 << 20;

/// 一格最多显示多少字符（默认值）：很长的内容整个画出来，一帧要排版好几秒
/// （group_by 里预览没聚合的列，每格是一个几十万个元素的列表）。
const CELL_MAX_DEFAULT: usize = 500;
/// 列表只列前几个元素，后面写共几个（照 polars 打印表格）。
const LIST_SHOWN: usize = 10;

/// 显示设置里一格最多多少字符。
fn cell_max() -> usize {
    DISPLAY.read().unwrap().max_cell
}

/// 空值显示的文字（设置里定的，默认照 polars 写 null）。
pub(crate) fn null_text() -> &'static str {
    DISPLAY.read().unwrap().null
}

/// 超过设置的字符数就截断写 …（字节长不超过上限的肯定没超字符数，先按字节排除，长的才数字符）。
fn truncate_cell(out: &mut String) {
    let max = cell_max();
    if out.len() <= max {
        return;
    }
    if out.chars().count() > max {
        let end = out.char_indices().nth(max).map_or(out.len(), |(i, _)| i);
        out.truncate(end);
        out.push('…');
    }
}

/// arrow 格式化单元格的选项：时间写成 2023-07-15 12:30:00（arrow 默认带 T），和 polars 打印的一样；
/// 带时区的写成 2023-07-15 12:30:00+08:00；空值写显示设置里的文字。
fn cell_format() -> arrow::util::display::FormatOptions<'static> {
    arrow::util::display::FormatOptions::default()
        .with_null(null_text())
        .with_timestamp_format(Some("%Y-%m-%d %H:%M:%S%.f"))
        .with_timestamp_tz_format(Some("%Y-%m-%d %H:%M:%S%.f%:z"))
}

/// 一格的文字写进 out：列表只写前 LIST_SHOWN 个元素（嵌套的一样），超过显示设置的字符数截断。
fn write_arrow_cell(col: &arrow::array::ArrayRef, fm: &arrow::util::display::ArrayFormatter, row: usize, out: &mut String) {
    use std::fmt::Write;
    if !write_list(col, row, out) && !write_float(col, row, out) && !write_int(col, row, out) {
        let _ = write!(out, "{}", fm.value(row));
    }
    truncate_cell(out);
}

/// col 是浮点列时写第 row 行并返回 true：和 Excel、polars 打印一样最多 15 位有效数字（arrow 写的是
/// 1234.5600000000002 这种最短往返写法）。f32 先按它自己的最短写法变成 f64，0.1f32 不会写成 0.100000001490116。
fn write_float(col: &arrow::array::ArrayRef, row: usize, out: &mut String) -> bool {
    use arrow::array::{Array, AsArray};
    use arrow::datatypes::{DataType, Float32Type, Float64Type};
    let value = match col.data_type() {
        DataType::Float64 => {
            let a = col.as_primitive::<Float64Type>();
            a.is_valid(row).then(|| a.value(row))
        }
        DataType::Float32 => {
            let a = col.as_primitive::<Float32Type>();
            a.is_valid(row).then(|| a.value(row).to_string().parse().unwrap_or(f64::NAN))
        }
        _ => return false,
    };
    match value {
        Some(v) => out.push_str(&format_float(v)),
        None => out.push_str(null_text()),
    }
    true
}

/// col 是整数列时写第 row 行并返回 true（arrow 的写法和这个一样；开了千位分隔符就分组）。
fn write_int(col: &arrow::array::ArrayRef, row: usize, out: &mut String) -> bool {
    use arrow::array::{Array, AsArray};
    use arrow::datatypes::{DataType, Int16Type, Int32Type, Int64Type, Int8Type, UInt16Type, UInt32Type, UInt64Type, UInt8Type};
    macro_rules! prim {
        ($t:ty) => {{
            let a = col.as_primitive::<$t>();
            a.is_valid(row).then(|| a.value(row) as i128)
        }};
    }
    let value = match col.data_type() {
        DataType::Int8 => prim!(Int8Type),
        DataType::Int16 => prim!(Int16Type),
        DataType::Int32 => prim!(Int32Type),
        DataType::Int64 => prim!(Int64Type),
        DataType::UInt8 => prim!(UInt8Type),
        DataType::UInt16 => prim!(UInt16Type),
        DataType::UInt32 => prim!(UInt32Type),
        DataType::UInt64 => prim!(UInt64Type),
        _ => return false,
    };
    match value {
        Some(v) => write_int_text(v, out),
        None => out.push_str(null_text()),
    }
    true
}

/// col 是列表（或映射）列时写第 row 行并返回 true；不是返回 false。列不完时个数写在前面（列宽不够时末尾会被省略）：
/// 共 250000 个：[1, 2, …]
fn write_list(col: &arrow::array::ArrayRef, row: usize, out: &mut String) -> bool {
    use arrow::array::{Array, AsArray};
    use arrow::util::display::ArrayFormatter;
    use std::fmt::Write;
    if let Some(map) = col.as_map_opt() {
        write_map(map, row, out);
        return true;
    }
    let items = if let Some(list) = col.as_list_opt::<i32>() {
        list.is_valid(row).then(|| list.value(row))
    } else if let Some(list) = col.as_list_opt::<i64>() {
        list.is_valid(row).then(|| list.value(row))
    } else if let Some(list) = col.as_fixed_size_list_opt() {
        list.is_valid(row).then(|| list.value(row))
    } else {
        return false;
    };
    let Some(items) = items else {
        out.push_str(null_text());
        return true;
    };
    let shown = items.slice(0, items.len().min(LIST_SHOWN));
    let opts = cell_format();
    let Ok(fm) = ArrayFormatter::try_new(&shown, &opts) else {
        out.push_str("‹无法显示›");
        return true;
    };
    if items.len() > shown.len() {
        let _ = write!(out, "共 {} 个：", items.len());
    }
    out.push('[');
    for i in 0..shown.len() {
        if i > 0 {
            out.push_str(", ");
        }
        if !write_list(&shown, i, out) {
            let _ = write!(out, "{}", fm.value(i));
        }
        if out.len() > cell_max() {
            break;
        }
    }
    if items.len() > shown.len() {
        out.push_str(", …");
    }
    out.push(']');
    true
}

/// 映射（polars 的 Map）的一格：{a: 1, b: 2}，和列表一样只写前 LIST_SHOWN 个条目。
fn write_map(map: &arrow::array::MapArray, row: usize, out: &mut String) {
    use arrow::array::Array;
    use arrow::util::display::ArrayFormatter;
    use std::fmt::Write;
    if !map.is_valid(row) {
        out.push_str(null_text());
        return;
    }
    let entries = map.value(row);
    let n = entries.len().min(LIST_SHOWN);
    let (keys, values) = (entries.column(0).slice(0, n), entries.column(1).slice(0, n));
    let opts = cell_format();
    let (Ok(kf), Ok(vf)) = (ArrayFormatter::try_new(&keys, &opts), ArrayFormatter::try_new(&values, &opts)) else {
        out.push_str("‹无法显示›");
        return;
    };
    if entries.len() > n {
        let _ = write!(out, "共 {} 个：", entries.len());
    }
    out.push('{');
    for i in 0..n {
        if i > 0 {
            out.push_str(", ");
        }
        let _ = write!(out, "{}: ", kf.value(i));
        if !write_list(&values, i, out) {
            let _ = write!(out, "{}", vf.value(i));
        }
        if out.len() > cell_max() {
            break;
        }
    }
    if entries.len() > n {
        out.push_str(", …");
    }
    out.push('}');
}

/// 已解码 row group 的 LRU 缓存（末尾最新）。
#[derive(Default)]
struct GroupCache {
    entries: Vec<(usize, Arc<ChunkStore>, usize)>, // (组号, 数据, 字节数)
    bytes: usize,
}

/// Parquet：只读元数据建 row-group 索引；访问某组时才解码该组（紧凑存储 + LRU 缓存）。
/// 全表扫描按 row group 并行，并只解码所需列。
struct ParquetSource {
    path: std::path::PathBuf,
    meta: parquet::arrow::arrow_reader::ArrowReaderMetadata,
    headers: Vec<String>,
    row_count: usize,
    /// (起始行, 行数) per row group
    groups: Vec<(usize, usize)>,
    cache: Mutex<GroupCache>,
}

impl ParquetSource {
    /// 按组（可选列裁剪、组内偏移/条数）读 record batch，逐行把各列文本写入 bufs 后回调。
    fn read_group(
        &self,
        group: usize,
        cols: Option<&[usize]>,
        offset: usize,
        limit: usize,
        f: &mut dyn FnMut(&[String]),
    ) -> Result<(), String> {
        use arrow::util::display::ArrayFormatter;
        use parquet::arrow::ProjectionMask;
        use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

        let file = std::fs::File::open(&self.path).map_err(|e| e.to_string())?;
        let mut builder = ParquetRecordBatchReaderBuilder::new_with_metadata(file, self.meta.clone())
            .with_row_groups(vec![group]);
        if let Some(cols) = cols {
            let mask = ProjectionMask::roots(self.meta.parquet_schema(), cols.iter().copied());
            builder = builder.with_projection(mask);
        }
        if offset > 0 {
            builder = builder.with_offset(offset);
        }
        let reader = builder.with_limit(limit).build().map_err(|e| e.to_string())?;

        let opts = cell_format();
        let mut bufs: Vec<String> = Vec::new();
        for batch in reader {
            let batch = batch.map_err(|e| e.to_string())?;
            // 某一列显示不了（arrow 不认的类型）只空着这一列，别让整张表都没有数据
            let formatters: Vec<Option<ArrayFormatter>> =
                batch.columns().iter().map(|col| ArrayFormatter::try_new(col, &opts).ok()).collect();
            bufs.resize(formatters.len(), String::new());
            for row_idx in 0..batch.num_rows() {
                for ((b, fm), col) in bufs.iter_mut().zip(&formatters).zip(batch.columns()) {
                    b.clear();
                    match fm {
                        Some(fm) => write_arrow_cell(col, fm, row_idx, b),
                        None => b.push_str("‹无法显示›"),
                    }
                }
                f(&bufs);
            }
        }
        Ok(())
    }

    /// 解码整个 row group（全部列）为紧凑存储。
    fn decode_group(&self, group: usize) -> Result<ChunkStore, String> {
        let mut store = ChunkStore::default();
        let mut chunk = Chunk::default();
        let len = self.groups[group].1;
        self.read_group(group, None, 0, len, &mut |cells| {
            for c in cells {
                chunk.push_cell(c);
            }
            chunk.end_row();
            if chunk.text.len() > 256 << 20 {
                store.push(std::mem::take(&mut chunk));
            }
        })?;
        store.push(chunk);
        Ok(store)
    }

    fn cached(&self, group: usize) -> Option<Arc<ChunkStore>> {
        let mut c = self.cache.lock().unwrap();
        let i = c.entries.iter().position(|e| e.0 == group)?;
        let e = c.entries.remove(i);
        let store = e.1.clone();
        c.entries.push(e);
        Some(store)
    }

    fn group_store(&self, group: usize) -> Option<Arc<ChunkStore>> {
        if let Some(s) = self.cached(group) {
            return Some(s);
        }
        let store = Arc::new(self.decode_group(group).ok()?);
        let bytes = store.bytes();
        let mut c = self.cache.lock().unwrap();
        if !c.entries.iter().any(|e| e.0 == group) {
            c.entries.push((group, store.clone(), bytes));
            c.bytes += bytes;
            while c.bytes > PARQUET_CACHE_BYTES && c.entries.len() > 1 {
                let old = c.entries.remove(0);
                c.bytes -= old.2;
            }
        }
        Some(store)
    }

    fn group_of_row(&self, row: usize) -> usize {
        self.groups
            .partition_point(|&(start, _)| start <= row)
            .saturating_sub(1)
    }
}

/// arrow 的类型 → polars 的简称（表头第二行），叫法和 engine::short_dtype 一样：
/// list[i64]、array[i64, 2]、struct{x: i64, y: str}、map[str, i64]、datetime[μs]…
fn short_dtype(t: &arrow::datatypes::DataType) -> String {
    use arrow::datatypes::{DataType as D, TimeUnit};
    let unit = |u: &TimeUnit| match u {
        TimeUnit::Second => "s",
        TimeUnit::Millisecond => "ms",
        TimeUnit::Microsecond => "μs",
        TimeUnit::Nanosecond => "ns",
    };
    let name = match t {
        D::Boolean => "bool",
        D::Int8 => "i8",
        D::Int16 => "i16",
        D::Int32 => "i32",
        D::Int64 => "i64",
        D::UInt8 => "u8",
        D::UInt16 => "u16",
        D::UInt32 => "u32",
        D::UInt64 => "u64",
        D::Float16 => "f16",
        D::Float32 => "f32",
        D::Float64 => "f64",
        D::Utf8 | D::LargeUtf8 | D::Utf8View => "str",
        D::Date32 | D::Date64 => "date",
        D::Timestamp(u, None) => return format!("datetime[{}]", unit(u)),
        D::Timestamp(u, Some(tz)) => return format!("datetime[{}, {tz}]", unit(u)),
        D::Time32(_) | D::Time64(_) => "time",
        D::Duration(u) => return format!("duration[{}]", unit(u)),
        D::Decimal128(p, s) | D::Decimal256(p, s) => return format!("decimal[{p},{s}]"),
        D::List(f) | D::LargeList(f) | D::ListView(f) | D::LargeListView(f) => {
            return format!("list[{}]", short_dtype(f.data_type()));
        }
        D::FixedSizeList(f, n) => return format!("array[{}, {n}]", short_dtype(f.data_type())),
        // 条目是 struct{key, value}
        D::Map(f, _) => {
            let D::Struct(kv) = f.data_type() else { return String::from("map") };
            let parts: Vec<String> = kv.iter().map(|f| short_dtype(f.data_type())).collect();
            return format!("map[{}]", parts.join(", "));
        }
        D::Struct(fields) => {
            const SHOWN: usize = 4;
            let mut parts: Vec<String> =
                fields.iter().take(SHOWN).map(|f| format!("{}: {}", f.name(), short_dtype(f.data_type()))).collect();
            if fields.len() > SHOWN {
                parts.push(format!("…共 {} 个", fields.len()));
            }
            return format!("struct{{{}}}", parts.join(", "));
        }
        D::Dictionary(..) => "cat",
        D::Binary | D::LargeBinary | D::BinaryView | D::FixedSizeBinary(_) => "binary",
        D::Null => "null",
        other => return format!("{other:?}").to_lowercase(),
    };
    name.to_string()
}

impl DataSource for ParquetSource {
    fn headers(&self) -> &[String] {
        &self.headers
    }

    fn dtypes(&self) -> Vec<String> {
        let fields = self.meta.schema().fields();
        // 引擎写的结果：元信息里是各列在 polars 里的类型（Int128、Object 写进文件时换成了文字）
        let polars: Option<Vec<String>> = (self.meta.metadata().file_metadata().key_value_metadata())
            .and_then(|kv| kv.iter().find(|kv| kv.key == "rtv_dtypes"))
            .and_then(|kv| serde_json::from_str(kv.value.as_deref()?).ok())
            .filter(|types: &Vec<String>| types.len() == fields.len());
        if let Some(types) = polars {
            return types.iter().map(|t| crate::engine::short_dtype(t)).collect();
        }
        (fields.iter())
            .map(|f| {
                // polars 写的 Enum 是字典、扩展类型是存储类型，类型名在字段的元信息里
                let meta = f.metadata();
                if meta.keys().any(|k| k.starts_with("_PL_ENUM_VALUES")) {
                    return String::from("enum");
                }
                match meta.get("ARROW:extension:name") {
                    Some(name) => format!("ext[{name}]"),
                    None => short_dtype(f.data_type()),
                }
            })
            .collect()
    }

    fn row_count(&self) -> usize {
        self.row_count
    }

    fn window(&self, start: usize, len: usize) -> Vec<Vec<String>> {
        let end = (start + len).min(self.row_count);
        let mut out = Vec::with_capacity(end.saturating_sub(start));
        let mut i = start;
        while i < end {
            let g = self.group_of_row(i);
            let Some((g_start, g_len)) = self.groups.get(g).copied() else { break };
            let Some(store) = self.group_store(g) else { break };
            let local = i - g_start;
            let take = (end - i).min(g_len - local);
            // 组内实际行数与元数据不符时 store.window 自动截取
            out.extend(store.window(local, take, self.headers.len()));
            i += take;
        }
        out
    }

    fn scan(&self, start: usize, len: usize, cols: &[usize], f: &mut RowFn) {
        let end = (start + len).min(self.row_count);
        let mut i = start;
        // 列裁剪：只解码 cols 里出现的列（batch 中按列号升序排列）
        let mut uniq: Vec<usize> = cols.iter().copied().filter(|&c| c < self.headers.len()).collect();
        uniq.sort_unstable();
        uniq.dedup();
        let pos: Vec<Option<usize>> = cols.iter().map(|c| uniq.binary_search(c).ok()).collect();
        while i < end {
            let g = self.group_of_row(i);
            let Some((g_start, g_len)) = self.groups.get(g).copied() else { break };
            let local = i - g_start;
            let take = (end - i).min(g_len - local);
            if let Some(store) = self.cached(g) {
                store.scan(local, take, cols, &mut |r, c| f(g_start + r, c));
            } else {
                let mut row = i;
                let _ = self.read_group(g, Some(&uniq), local, take, &mut |bufs| {
                    let cells: Vec<&str> =
                        pos.iter().map(|p| p.and_then(|k| bufs.get(k)).map_or("", |s| s.as_str())).collect();
                    f(row, &cells);
                    row += 1;
                });
            }
            i += take;
        }
    }

    /// 取样只解码首个 row group 的前 n 行，避免为量列宽解码整组
    fn sample(&self, n: usize) -> Vec<Vec<String>> {
        if self.row_count == 0 || n == 0 || self.groups.is_empty() {
            return Vec::new();
        }
        let mut out = Vec::new();
        let take = n.min(self.groups[0].1);
        let _ = self.read_group(0, None, 0, take, &mut |cells| out.push(cells.to_vec()));
        out
    }
}

fn open_parquet(path: &Path) -> Result<Vec<SheetHandle>, LoadError> {
    use parquet::arrow::arrow_reader::{ArrowReaderMetadata, ArrowReaderOptions};

    let file = std::fs::File::open(path).map_err(LoadError::Io)?;
    let meta = ArrowReaderMetadata::load(&file, ArrowReaderOptions::default())
        .map_err(|e| LoadError::Format(e.to_string()))?;

    let row_count = meta.metadata().file_metadata().num_rows().max(0) as usize;
    let mut groups = Vec::new();
    let mut start = 0;
    for rg in meta.metadata().row_groups() {
        let len = rg.num_rows().max(0) as usize;
        groups.push((start, len));
        start += len;
    }

    // 0 行的也照样打开（只有表头）：节点的结果常有一行都不剩的（筛选条件谁都不满足）
    let headers: Vec<String> = meta.schema().fields().iter().map(|f| f.name().clone()).collect();
    if headers.is_empty() {
        return Err(LoadError::Format(String::from("Parquet 文件里没有列")));
    }

    let name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("parquet")
        .to_string();
    let source = ParquetSource {
        path: path.to_path_buf(),
        meta,
        headers,
        row_count,
        groups,
        cache: Mutex::new(GroupCache::default()),
    };
    Ok(vec![SheetHandle { name, source: Arc::new(source) }])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn first_sheet(name: &str) -> (String, Arc<dyn DataSource>) {
        let sheets = open_tables(Path::new(&format!("testdata/{name}"))).expect(name);
        let s = sheets.into_iter().next().unwrap();
        (s.name, s.source)
    }

    fn wait_loaded(src: &Arc<dyn DataSource>) {
        src.prepare().unwrap();
        while src.is_loading() {
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// scan / gather 的结果必须与 window 一致（任意列序、缺列为空）。
    fn check_scan_matches_window(src: &Arc<dyn DataSource>) {
        let n = src.row_count();
        let w = src.headers().len();
        let rows = src.window(0, n);
        assert_eq!(rows.len(), n);
        let cols: Vec<usize> = (0..w).rev().chain([w + 3]).collect();
        let expect = |r: usize| -> Vec<String> {
            cols.iter().map(|&c| rows[r].get(c).cloned().unwrap_or_default()).collect()
        };
        // 分段 scan
        let mut seen = 0;
        for s in (0..n).step_by(7) {
            src.scan(s, 7.min(n - s), &cols, &mut |r, cells| {
                assert_eq!(cells.iter().map(|s| s.to_string()).collect::<Vec<_>>(), expect(r), "scan 行 {r}");
                seen += 1;
            });
        }
        assert_eq!(seen, n);
    }

    #[test]
    fn scan_matches_window_all_sources() {
        for name in ["people.csv", "people.tsv", "events.parquet", "sales.xlsx"] {
            let (_, src) = first_sheet(name);
            wait_loaded(&src);
            check_scan_matches_window(&src);
        }
    }

    #[test]
    fn chunk_store_roundtrip() {
        let mut c = Chunk::default();
        c.push_cell("a");
        c.push_cell("");
        c.push_cell("中文");
        c.end_row();
        c.end_row(); // 空行
        c.push_cell("x");
        c.push_cell("y");
        c.set_open_cell(0, "z");
        c.end_row();
        let mut store = ChunkStore::default();
        store.push(c);
        store.push(Chunk::empty_rows(2));
        assert_eq!(store.rows(), 5);
        assert_eq!(store.width(), 3);
        let w = store.window(0, 10, 3);
        assert_eq!(w[0], ["a", "", "中文"]);
        assert_eq!(w[1], ["", "", ""]);
        assert_eq!(w[2], ["z", "y", ""]);
        assert_eq!(w[4], ["", "", ""]);
        let mut got = Vec::new();
        store.scan(2, 1, &[2, 0], &mut |r, c| got.push((r, c.join("|"))));
        assert_eq!(got, [(2, "|z".to_string())]);
    }

    /// ods（calamine 整表解码路径，xls/xlsb 同一条）：值、日期、布尔、空格的写法。由 testdata/make_ods.py 生成。
    #[test]
    fn ods_decodes() {
        let (_, src) = first_sheet("small.ods");
        wait_loaded(&src);
        assert_eq!(
            src.headers(),
            ["id", "name", "amount", "qty", "city", "active", "score", "remark", "date"]
        );
        assert_eq!(
            src.window(0, 3),
            [
                ["1", "客户00000", "1000", "0", "北京", "TRUE", "0", "", "2023-01-01"],
                ["2", "客户00001", "1003.7", "1", "上海", "FALSE", "0.1429", "备注1", "2023-01-02"],
                ["3", "客户00002", "1007.4", "2", "广州", "TRUE", "0.2857", "备注2", "2023-01-03"],
            ]
        );
    }

    /// 日期、时间、时长照日期写（不是 45122 这种序列号），快速引擎和 calamine 写得一样。
    #[test]
    fn xlsx_dates() {
        let path = Path::new("testdata/dates.xlsx");
        let want = [
            ["2023-07-15", "2023-07-15 12:30:00", "08:05:00", "2023-07-15", "2023-07-15", "36:00:00", "1234.5"],
            ["1999-12-31", "2000-01-01 00:00:01", "23:59:59", "2024-02-29", "1900-03-01", "06:00:00", "0.1"],
        ];
        for fast in [true, false] {
            let src = open_xlsx_with(path, Arc::new(|_| {}), fast).unwrap().remove(0).source;
            wait_loaded(&src);
            assert_eq!(src.headers(), ["日期", "日期时间", "时间", "中文日期", "短日期", "时长", "数字"]);
            assert_eq!(src.window(0, 2), want, "fast = {fast}");
        }
    }

    #[test]
    fn excel_datetime_text() {
        let text = |v: f64, duration: bool, is_1904: bool| {
            let mut s = String::new();
            write_excel_datetime(v, duration, is_1904, &mut s);
            s
        };
        assert_eq!(text(45122.0, false, false), "2023-07-15");
        assert_eq!(text(43660.0, false, true), "2023-07-15");
        assert_eq!(text(45122.0 + 1.5 / 86400.0, false, false), "2023-07-15 00:00:01.500");
        assert_eq!(text(60.0, false, false), "1900-02-29"); // Excel 的 1900 闰年错误
        assert_eq!(text(-1.0, false, false), "-1");
        assert_eq!(text(2.25, true, false), "54:00:00");
        let mut s = String::new();
        push_iso_datetime("2023-07-15T08:00:00", &mut s);
        assert_eq!(s, "2023-07-15 08:00:00");
    }

    /// 数字的写法和 Excel 的“常规”格式一样：最多 15 位有效数字，很大很小的用指数。
    #[test]
    fn float_text_like_excel() {
        for (v, want) in [
            (3.0, "3"),
            (1234.5600000000002, "1234.56"),
            (0.1 + 0.2, "0.3"),
            (-2.9999999999999996, "-3"),
            (123456789.12345679, "123456789.123457"),
            (0.000025, "0.000025"),
            (1e-7, "1e-7"),
            (1.5e20, "1.5e20"),
            (f64::NAN, "NaN"),
        ] {
            assert_eq!(format_float(v), want, "{v}");
        }
    }

    /// 显示设置的三种浮点写法和千位分隔符（照 polars 的 set_fmt_float / set_thousands_separator）。
    #[test]
    fn float_display_settings() {
        let base = DisplayFmt::default();
        let full = DisplayFmt { float_mode: 1, ..base.clone() };
        assert_eq!(format_float_with(1234.5600000000002, &full), "1234.5600000000002");
        assert_eq!(format_float_with(3.0, &full), "3");
        let fixed = DisplayFmt { float_mode: 2, float_digits: 2, ..base.clone() };
        assert_eq!(format_float_with(1234.5, &fixed), "1234.50");
        assert_eq!(format_float_with(3.14159, &fixed), "3.14");
        let trim = DisplayFmt { float_mode: 2, float_digits: 3, trim_zeros: true, ..base.clone() };
        assert_eq!(format_float_with(2.5, &trim), "2.5");
        assert_eq!(format_float_with(2.0, &trim), "2");
        let whole = DisplayFmt { float_mode: 2, float_digits: 0, ..base.clone() };
        assert_eq!(format_float_with(3.7, &whole), "4");
        let th = DisplayFmt { thousands: true, ..base.clone() };
        assert_eq!(format_float_with(1234.56, &th), "1,234.56");
        assert_eq!(format_float_with(-1234567.0, &th), "-1,234,567");
        assert_eq!(format_float_with(1.5e20, &th), "1.5e20");
        assert_eq!(thousands_sep("9876543.21"), "9,876,543.21");
        assert_eq!(thousands_sep("NaN"), "NaN");
        assert_eq!(thousands_sep(""), "");
    }

    /// GBK 的 CSV（中文 Windows 上 Excel“另存为 CSV”的默认）按系统代码页解出中文，引擎也照这样读。
    #[test]
    fn csv_gbk() {
        let (_, src) = first_sheet("gbk.csv");
        assert_eq!(src.row_count(), 3);
        if codepage::ansi() != Some(54936) {
            eprintln!("跳过：系统代码页不是中文的");
            return;
        }
        assert_eq!(src.headers(), ["姓名", "城市", "金额"]);
        assert_eq!(src.window(0, 3), [["张三", "北京", "12.5"], ["李四", "上海", "8"], ["赵六,七", "深圳", "3"]]);
        assert_eq!(src.read_hints(), [("encoding", String::from("gb18030"))]);
        // UTF-8 的照旧
        assert!(first_sheet("people.csv").1.read_hints().is_empty());
    }

    /// 不大的 CSV 读进内存：查看器开着时别的程序还能保存（覆盖、替换）这个文件。
    #[test]
    fn csv_file_is_not_locked() {
        let path = std::env::temp_dir().join(format!("rtv-lock-{}.csv", std::process::id()));
        std::fs::write(&path, "a,b\n1,2\n").unwrap();
        let src = open_tables(&path).unwrap().remove(0).source;
        std::fs::write(&path, "a,b\n3,4\n5,6\n").expect("查看器开着时也能覆盖");
        let new = path.with_extension("new");
        std::fs::write(&new, "x\n").unwrap();
        std::fs::rename(&new, &path).expect("查看器开着时也能替换");
        assert_eq!(src.window(0, 1), [["1", "2"]], "显示的还是打开时读到的");
        let _ = std::fs::remove_file(&path);
    }

    /// 分号分隔的 .csv：第一行没有逗号、有分号，按分号拆。
    #[test]
    fn csv_semicolon() {
        let (_, src) = first_sheet("semicolon.csv");
        assert_eq!(src.headers(), ["名称", "数量", "单价"]);
        assert_eq!(src.window(1, 1), [["香蕉", "5", "2.25"]]);
        assert_eq!(src.read_hints(), [("separator", String::from(";"))]);
        assert_eq!(sniff_delimiter(b"a,b;c\n", TextEncoding::Utf8), b',');
        assert_eq!(sniff_delimiter(b"\"a;b\"\tc\td\n1;2", TextEncoding::Utf8), b'\t');
        assert_eq!(sniff_delimiter(b"only\n", TextEncoding::Utf8), b',');
    }

    /// 快速引擎与 calamine 兜底引擎对同一文件的解码结果必须一致。
    #[test]
    fn xlsx_fast_matches_calamine() {
        for name in ["sales.xlsx", "dates.xlsx", "big500k.xlsx"] {
            let path = Path::new("testdata").join(name);
            if !path.exists() {
                continue;
            }
            let noop: ProgressFn = Arc::new(|_| {});
            let a = open_xlsx_with(&path, noop.clone(), true).unwrap();
            let b = open_xlsx_with(&path, noop, false).unwrap();
            assert_eq!(a.len(), b.len());
            for (x, y) in a.iter().zip(&b) {
                assert_eq!(x.name, y.name);
                x.source.prepare().unwrap();
                y.source.prepare().unwrap();
                while x.source.is_loading() || y.source.is_loading() {
                    std::thread::sleep(Duration::from_millis(20));
                }
                assert_eq!(x.source.headers(), y.source.headers(), "{name} 表头");
                assert_eq!(x.source.row_count(), y.source.row_count(), "{name} 行数");
                let n = x.source.row_count();
                assert_eq!(x.source.window(0, n), y.source.window(0, n), "{name} 内容");
            }
        }
    }

    #[test]
    fn csv_lazy_window() {
        let (_, src) = first_sheet("people.csv");
        assert_eq!(src.headers(), ["姓名", "城市", "年龄", "收入"]);
        assert_eq!(src.row_count(), 50);
        assert_eq!(src.window(0, 2)[0][0], "用户001");
        // 任意位置窗口
        let tail = src.window(48, 10);
        assert_eq!(tail.len(), 2);
        assert_eq!(tail[1][0], "用户050");
    }

    #[test]
    fn tsv_lazy_window() {
        let (_, src) = first_sheet("people.tsv");
        assert_eq!(src.row_count(), 50);
        assert_eq!(src.window(10, 1)[0][0], "用户011");
    }

    #[test]
    fn xlsx_two_sheets_lazy() {
        let sheets = open_tables(Path::new("testdata/sales.xlsx")).unwrap();
        assert_eq!(sheets.len(), 2);
        let s0 = &sheets[0].source;
        assert_eq!(sheets[0].name, "销售数据");
        assert_eq!(s0.headers(), ["产品", "数量", "单价", "备注"]);
        assert_eq!(s0.row_count(), 30);
        assert_eq!(s0.window(0, 1)[0][1], "3"); // 整数显示无 .0
        assert_eq!(s0.window(4, 1)[0][3], ""); // 第5行备注为空
        let s1 = &sheets[1].source;
        assert_eq!(sheets[1].name, "汇总");
        assert_eq!(s1.row_count(), 12);
        assert_eq!(s1.window(11, 5).len(), 1);
    }

    /// 大文件流式解码验证（需先生成 testdata/big1m.xlsx）。
    #[test]
    #[ignore]
    fn xlsx_streaming_big_file() {
        let t = std::time::Instant::now();
        let sheets = open_tables(Path::new("testdata/big1m.xlsx")).unwrap();
        eprintln!("open_tables 返回: {:?}", t.elapsed());
        let src = &sheets[0].source;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !src.is_ready() {
            assert!(std::time::Instant::now() < deadline, "首块超时");
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        eprintln!("首块可用: {:?}  headers={:?}", t.elapsed(), src.headers());
        while src.is_loading() {
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        eprintln!("全部完成: {:?}  {} 行", t.elapsed(), src.row_count());
        assert_eq!(src.row_count(), 1_000_000);
        assert_eq!(src.window(999_999, 1)[0][0], "999999");
    }

    #[test]
    fn parquet_nested_dtypes() {
        // polars 写的（列表、数组、结构、带时区的时间…），表头第二行和 polars 的叫法一致，数据也都显示得出来
        let (_, src) = first_sheet("nested.parquet");
        let row = &src.window(0, 1)[0];
        assert_eq!(row[0], "[1, 2]");
        assert_eq!(row[3], "{x: 1, y: a}");
        assert_eq!(row[4], "2024-01-01 00:00:00+08:00");
        assert_eq!(
            src.dtypes(),
            [
                "list[i64]",
                "list[list[f64]]",
                "array[i64, 2]",
                "struct{x: i64, y: str}",
                "datetime[μs, Asia/Shanghai]",
                "cat",
                "enum",
                "decimal[10,2]",
                "list[struct{x: i64}]",
                "duration[ms]",
                "null",
                "binary",
                "map[str, i64]",
                "map[str, list[i64]]",
            ]
        );
        // 映射（polars 2 的 Map）：条目多时和列表一样只写前几个
        assert_eq!(row[12], "{a: 1, b: 2}");
        assert_eq!(row[13], "共 12 个：{k0: [0, 0], k1: [1, 1], k2: [2, 2], k3: [3, 3], k4: [4, 4], k5: [5, 5], k6: [6, 6], k7: [7, 7], k8: [8, 8], k9: [9, 9], …}");
    }

    /// 很长的列表、很长的文字只显示开头（整格画出来一帧要好几秒）：列表写前几个元素，个数写在前面。
    #[test]
    fn parquet_long_cells_are_cut() {
        use arrow::array::{ArrayRef, Int64Array, ListArray, RecordBatch, StringArray};
        use arrow::datatypes::Int64Type;
        let big: Vec<Option<i64>> = (0..1000).map(Some).collect();
        let lists = ListArray::from_iter_primitive::<Int64Type, _, _>([Some(big), Some(vec![Some(1), Some(2)]), None]);
        let long = "字".repeat(1000);
        let texts = StringArray::from(vec![long.as_str(), "短", "x"]);
        let ids = Int64Array::from(vec![1, 2, 3]);
        let batch = RecordBatch::try_from_iter([
            ("id", Arc::new(ids) as ArrayRef),
            ("值", Arc::new(lists) as ArrayRef),
            ("文字", Arc::new(texts) as ArrayRef),
        ])
        .unwrap();
        let path = std::env::temp_dir().join(format!("rtv-long-{}.parquet", std::process::id()));
        let file = std::fs::File::create(&path).unwrap();
        let mut writer = parquet::arrow::ArrowWriter::try_new(file, batch.schema(), None).unwrap();
        writer.write(&batch).unwrap();
        writer.close().unwrap();

        let src = open_tables(&path).unwrap().remove(0).source;
        let rows = src.window(0, 3);
        assert_eq!(rows[0][1], "共 1000 个：[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, …]");
        assert_eq!(rows[1][1], "[1, 2]");
        assert_eq!(rows[2][1], "null");
        assert!(rows[0][2].ends_with('…') && rows[0][2].chars().count() <= CELL_MAX_DEFAULT + 1, "{}", rows[0][2].chars().count());
        assert_eq!(rows[1][2], "短");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn parquet_lazy_window() {
        let (_, src) = first_sheet("events.parquet");
        assert_eq!(src.headers(), ["id", "名称", "数值", "标签", "日期"]);
        assert_eq!(src.row_count(), 1000);
        let first = src.window(0, 1);
        assert_eq!(first[0][0], "0");
        assert_eq!(first[0][1], "记录_0");
        assert_eq!(first[0][4], "2026-01-01");
        // 任意位置窗口
        let w = src.window(999, 10);
        assert_eq!(w.len(), 1);
        assert_eq!(w[0][1], "记录_999");
        assert_eq!(src.window(7, 1)[0][3], "null"); // 第8行(i=7)标签为 null
        assert_eq!(src.window(1, 1)[0][3], "B");
        // sample 不解码整个文件也应可用
        assert_eq!(src.sample(3).len(), 3);
    }

    #[test]
    fn unsupported_ext() {
        let err = match open_tables(Path::new("testdata/does.not.exist.txt")) {
            Err(e) => e,
            Ok(_) => panic!("应当报不支持格式"),
        };
        assert!(matches!(err, LoadError::Unsupported(_)));
    }
}
