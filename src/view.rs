//! 表格视图：懒加载的行模型、列宽、状态栏。数据怎么来的（文件 / 节点结果）这里不关心。

use crate::*;

/// 列宽自适应的取样行数。
pub(crate) const WIDTH_SAMPLE_ROWS: usize = 500;
/// 行缓存上限（超过后整体清空，重新按需生成可见行）。
pub(crate) const ROW_CACHE_LIMIT: usize = 8192;
/// 行模型未命中时一次取的对齐块行数。
pub(crate) const ROW_BLOCK: usize = if cfg!(test) { 32 } else { 64 };
/// 工程文件的扩展名：节点图 + 界面布局（像 Houdini 的 hip）。
pub(crate) const PROJECT_EXT: &str = "rtv";

pub(crate) fn clear_table(app: &AppWindow) {
    app.set_rows(ModelRc::new(VecModel::from(Vec::new())));
    app.set_columns(ModelRc::new(VecModel::from(Vec::new())));
    app.set_stats(SharedString::new());
}

/// 表格行模型：只在界面需要时按块向数据源取数。
pub(crate) struct LazyRowsModel {
    source: Arc<dyn DataSource>,
    visible: Vec<usize>,
    /// 建模型时的表头列数（渐进解码中若变化则需整体刷新）
    n_headers: usize,
    /// 已通知给 UI 的行数（渐进解码时由 grow 增长）
    count: Cell<usize>,
    cache: RefCell<HashMap<usize, RowData>>,
    tracker: ModelNotify,
}

impl LazyRowsModel {
    fn new(source: Arc<dyn DataSource>, visible: Vec<usize>) -> Self {
        Self {
            n_headers: source.headers().len(),
            count: Cell::new(source.row_count()),
            source,
            visible,
            cache: RefCell::new(HashMap::new()),
            tracker: ModelNotify::default(),
        }
    }

    /// 渐进解码新增了行：只追加通知，不替换模型（保留缓存与滚动位置）。
    fn grow(&self) {
        let old = self.count.get();
        let new = self.source.row_count();
        if new > old {
            self.count.set(new);
            self.tracker.row_added(old, new - old);
        }
    }

    fn make_row(&self, row: usize, cells: &[&str]) -> RowData {
        let items: Vec<StandardListViewItem> = cells.iter().map(|&c| StandardListViewItem::from(c)).collect();
        RowData {
            num: SharedString::from((row + 1).to_string()),
            cells: ModelRc::new(VecModel::from(items)),
        }
    }
}

/// 0 行时的占位行（保证横向滚动范围与表头一致）。
pub(crate) fn blank_row(n_cols: usize) -> RowData {
    let items: Vec<StandardListViewItem> = (0..n_cols).map(|_| StandardListViewItem::default()).collect();
    RowData { num: SharedString::new(), cells: ModelRc::new(VecModel::from(items)) }
}

impl Model for LazyRowsModel {
    type Data = RowData;

    fn row_count(&self) -> usize {
        self.count.get()
    }

    fn row_data(&self, row: usize) -> Option<Self::Data> {
        let count = self.count.get();
        if row >= count {
            return None;
        }
        if let Some(hit) = self.cache.borrow().get(&row) {
            return Some(hit.clone());
        }
        let block = row - row % ROW_BLOCK;
        let len = ROW_BLOCK.min(count - block);
        let mut fetched = Vec::with_capacity(len);
        self.source.scan(block, len, &self.visible, &mut |r, cells| {
            fetched.push((r, self.make_row(r, cells)));
        });
        let mut cache = self.cache.borrow_mut();
        if cache.len() + fetched.len() > ROW_CACHE_LIMIT {
            cache.clear();
        }
        cache.extend(fetched);
        Some(cache.get(&row).cloned().unwrap_or_else(|| blank_row(self.visible.len())))
    }

    fn set_row_data(&self, _row: usize, _data: Self::Data) {}

    fn model_tracker(&self) -> &dyn ModelTracker {
        &self.tracker
    }

    fn as_any(&self) -> &dyn core::any::Any {
        self
    }
}

pub(crate) fn refresh_view(app: &AppWindow, st: &mut App) {
    if st.sheets.is_empty() {
        return;
    }
    let index = st.current.min(st.sheets.len() - 1);
    st.current = index;
    let source = st.sheets[index].source.clone();
    let n_cols = source.headers().len();

    // 行模型（懒加载）；0 行时垫一行空行，保证横向滚动范围与表头一致
    if source.row_count() == 0 && n_cols > 0 {
        app.set_rows(ModelRc::new(VecModel::from(vec![blank_row(n_cols)])));
    } else {
        app.set_rows(ModelRc::new(Rc::new(LazyRowsModel::new(source.clone(), (0..n_cols).collect()))));
    }

    // 列模型：保留用户拖出来的宽度，缺的才按内容取样自适应
    let old_widths = read_current_widths(app);
    if old_widths.len() == st.col_widths.len() {
        st.col_widths = old_widths;
    }
    let dtypes = column_types(st, &*source);
    if st.col_widths.len() != n_cols || st.col_numeric.len() != n_cols {
        // 换了一张表：原来选的格子不作数了（sel-gen +1 连 Ctrl 加选的块一起作废）
        app.set_sel_active(false);
        app.set_sel_gen(app.get_sel_gen() + 1);
        let sample = source.sample(WIDTH_SAMPLE_ROWS);
        // 字号缩放时按内容算的列宽跟着变（8px 是 13px 字号下半角字符的宽）
        let k = app.get_table_font_size() / 13.0;
        st.col_widths.clear();
        st.col_numeric.clear();
        for c in 0..n_cols {
            let cells = || sample.iter().filter_map(|line| line.get(c));
            let head = source.headers().get(c).map_or(0.0, |h| visual_width(h));
            // 表头第二行的类型（字号 -2.5px 的等宽字，约 0.75 个字宽）也要放得下：list[struct{…}] 这种很长
            let dtype = dtypes.get(c).map_or(0.0, |t| t.chars().count() as f32 * 0.75 * k);
            let chars = cells().map(|cell| visual_width(cell)).fold(head.max(dtype), f32::max);
            st.col_widths.push((chars * 8.0 * k + 36.0).clamp(72.0, 420.0));
            // 非空格 ≥90% 可解析为数值即视为数值列（右对齐）
            let (mut filled, mut nums) = (0usize, 0usize);
            for cell in cells().map(|s| s.trim()).filter(|s| !s.is_empty()) {
                filled += 1;
                // 开了千位分隔符时，带分隔的也算数值列
                nums += (cell.parse::<f64>().is_ok()
                    || (crate::loader::thousands_enabled() && cell.replace(',', "").parse::<f64>().is_ok()))
                    as usize;
            }
            st.col_numeric.push(filled > 0 && nums * 10 >= filled * 9);
        }
    }

    let columns: Vec<ColumnSpec> = (0..n_cols)
        .map(|c| ColumnSpec {
            title: SharedString::from(source.headers()[c].as_str()),
            width: st.col_widths[c],
            numeric: st.col_numeric[c],
            dtype: SharedString::from(dtypes.get(c).map_or("", |t| t.as_str())),
        })
        .collect();
    app.set_columns(ModelRc::new(VecModel::from(columns)));
    app.set_settings_wrap(st.wrap_cells);

    app.set_theme_mode(st.theme_mode);
    update_status(app, st);
}

/// 表头第二行的类型：数据源自己知道的（parquet、节点结果）；查看器自己读的源文件用引擎推出来的源节点类型
/// （见 run::fetch_native_types），按列名对上，列名都对不上而列数一样时按位置。
pub(crate) fn column_types(st: &App, source: &dyn DataSource) -> Vec<String> {
    let own = source.dtypes();
    if !own.is_empty() || st.file_sheets.is_some() {
        return own;
    }
    let Some((key, types)) = &st.native_types else { return own };
    if native_key(st).is_none_or(|(_, k)| k != *key) {
        return own;
    }
    let headers = source.headers();
    let by_name: Vec<String> = (headers.iter())
        .map(|h| types.iter().find(|(name, _)| name == h).map_or(String::new(), |(_, t)| engine::short_dtype(t)))
        .collect();
    if by_name.iter().all(String::is_empty) && headers.len() == types.len() {
        return types.iter().map(|(_, t)| engine::short_dtype(t)).collect();
    }
    by_name
}

/// 只换表头的类型（引擎后来才推出来的）：列宽、选区、滚动都不动。
pub(crate) fn update_column_types(app: &AppWindow, st: &App) {
    let Some(source) = st.sheets.get(st.current).map(|s| s.source.clone()) else { return };
    let types = column_types(st, &*source);
    let model = app.get_columns();
    for (i, t) in types.into_iter().enumerate().take(model.row_count()) {
        if let Some(mut col) = model.row_data(i) {
            if col.dtype.as_str() != t {
                col.dtype = SharedString::from(t);
                model.set_row_data(i, col);
            }
        }
    }
}

pub(crate) fn update_busy(app: &AppWindow, st: &App) {
    let loading = st.sheets.get(st.current).is_some_and(|s| s.source.is_loading());
    app.set_busy(loading || st.running || st.writing);
}

/// 千分位格式化：1234567 → "1,234,567"。
pub(crate) fn fmt_count(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, ch) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// 状态栏元信息：格式 · 文件大小。
pub(crate) fn file_meta(path: &std::path::Path) -> String {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_uppercase();
    let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0) as f64;
    let size = if size >= 1024.0 * 1024.0 * 1024.0 {
        format!("{:.1} GB", size / (1024.0 * 1024.0 * 1024.0))
    } else if size >= 1024.0 * 1024.0 {
        format!("{:.1} MB", size / (1024.0 * 1024.0))
    } else {
        format!("{:.0} KB", (size / 1024.0).ceil())
    };
    if ext.is_empty() { size } else { format!("{ext} · {size}") }
}

/// 行号列宽度：按最大行号位数。
pub(crate) fn gutter_width(rows: usize) -> f32 {
    let digits = rows.max(1).to_string().len() as f32;
    (digits * 7.5 + 22.0).max(44.0)
}

/// 渐进解码进度：当前行模型是同源懒模型时只追加行，否则整体刷新。
pub(crate) fn progress_update(app: &AppWindow, st: &mut App) {
    let base = st.sheets[st.current].source.clone();
    let rows = app.get_rows();
    match rows.as_any().downcast_ref::<LazyRowsModel>() {
        Some(m) if Arc::ptr_eq(&m.source, &base) && m.n_headers == base.headers().len() => {
            m.grow();
            update_status(app, st);
        }
        _ => refresh_view(app, st),
    }
}

/// 状态栏：左侧消息（解码进度/失败/节点耗时），右侧统计（行×列）。
pub(crate) fn update_status(app: &AppWindow, st: &App) {
    let source = st.sheets[st.current].source.clone();
    let status = if let Some(e) = source.load_error() {
        format!("解析失败: {e}")
    } else if source.is_loading() {
        format!("解码中… 已 {} 行", fmt_count(source.row_count()))
    } else if !st.node_status.is_empty() {
        st.node_status.clone()
    } else {
        String::from("就绪")
    };
    app.set_status(SharedString::from(status));
    app.set_stats(SharedString::from(format!(
        "{} 行 × {} 列",
        fmt_count(source.row_count()),
        source.headers().len()
    )));
    app.set_gutter_width(gutter_width(source.row_count()));
    update_busy(app, st);
}

/// 表格缩放（Ctrl + 滚轮 / 菜单重置）：改字号，行高、表头跟着变；列宽按比例缩放。
/// 字号经 ui_state 存进设置（project.rs 的 font_size），退出时记下来。
pub(crate) fn zoom_table(app: &AppWindow, st: &mut App, to: f32) {
    let old = app.get_table_font_size();
    let new = to.clamp(9.0, 28.0);
    let ratio = new / old;
    if (ratio - 1.0).abs() < 0.001 {
        return;
    }
    for w in &mut st.col_widths {
        *w = (*w * ratio).max(24.0);
    }
    app.set_table_font_size(new);
    let columns = app.get_columns();
    for i in 0..columns.row_count() {
        if let Some(mut col) = columns.row_data(i) {
            col.width = (col.width * ratio).max(24.0);
            columns.set_row_data(i, col);
        }
    }
}

/// 读回 UI 当前列宽（用户可能拖拽过）。
pub(crate) fn read_current_widths(app: &AppWindow) -> Vec<f32> {
    let model = app.get_columns();
    (0..model.row_count()).filter_map(|i| model.row_data(i)).map(|c| c.width).collect()
}

/// 估算字符串的显示宽度（CJK/全角字符按 2 计），单位约为半角字符数。
pub(crate) fn visual_width(s: &str) -> f32 {
    s.chars()
        .map(|c| if is_wide(c) { 2.0 } else { 1.0 })
        .sum::<f32>()
        .min(50.0) // 超长单元格不把列撑爆
}

pub(crate) fn is_wide(c: char) -> bool {
    matches!(c as u32,
        0x1100..=0x115F
        | 0x2E80..=0xA4CF
        | 0xAC00..=0xD7A3
        | 0xF900..=0xFAFF
        | 0xFE30..=0xFE6F
        | 0xFF00..=0xFF60
        | 0xFFE0..=0xFFE6
        | 0x1F300..=0x1FAFF
        | 0x30000..=0x3FFFD)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn csv_source() -> Arc<dyn DataSource> {
        let sheets = loader::open_tables(Path::new("testdata/people.csv")).unwrap();
        sheets[0].source.clone()
    }

    /// 行模型按块取数的结果与 window 一致（含块尾、越界）。
    #[test]
    fn lazy_rows_model_blocks() {
        let src = csv_source();
        let visible = vec![2, 0];
        let m = LazyRowsModel::new(src.clone(), visible.clone());
        assert_eq!(m.row_count(), src.row_count());
        let rows = src.window(0, src.row_count());
        for r in [0, 1, 31, 32, 33, src.row_count() - 1] {
            let got = m.row_data(r).unwrap();
            assert_eq!(got.num.as_str(), (r + 1).to_string());
            let got = got.cells;
            let texts: Vec<String> = (0..got.row_count()).map(|i| got.row_data(i).unwrap().text.to_string()).collect();
            let want: Vec<String> = visible.iter().map(|&c| rows[r][c].clone()).collect();
            assert_eq!(texts, want, "row {r}");
        }
        assert!(m.row_data(src.row_count()).is_none());
        m.grow(); // 行数未变：不应越界通知
        assert_eq!(m.row_count(), src.row_count());
    }

    /// 复制出来的文本：制表符分列、\r\n 分行，特殊字符加引号，单个格子不带换行。
    #[test]
    fn selection_text_is_excel_friendly() {
        let src = csv_source();
        let rows = src.window(0, 3);
        let block = selection_text(src.as_ref(), 1..3, &[0, 2], false);
        assert_eq!(block, format!("{}\t{}\r\n{}\t{}\r\n", rows[1][0], rows[1][2], rows[2][0], rows[2][2]));
        let headed = selection_text(src.as_ref(), 0..1, &[1], true);
        assert_eq!(headed, format!("{}\r\n{}\r\n", src.headers()[1], rows[0][1]));
        assert_eq!(selection_text(src.as_ref(), 2..3, &[3], false), rows[2][3]);
        assert_eq!(selection_text(src.as_ref(), 0..0, &[0], false), "");
        let mut cell = String::new();
        push_cell(&mut cell, "a\tb \"c\"");
        assert_eq!(cell, "\"a\tb \"\"c\"\"\"");
    }
}

/// 端到端性能基准（需 testdata/big1m.xlsx、big.csv、big.parquet）：
/// cargo test --release bench_ops -- --ignored --nocapture --test-threads=1
#[cfg(test)]
mod bench {
    use super::*;
    use std::time::Instant;

    #[repr(C)]
    #[derive(Default)]
    struct Pmc {
        cb: u32,
        page_faults: u32,
        peak_ws: usize,
        ws: usize,
        qpp: usize,
        qp: usize,
        qnpp: usize,
        qnp: usize,
        private: usize,
        peak_private: usize,
    }
    unsafe extern "system" {
        fn GetCurrentProcess() -> isize;
        fn K32GetProcessMemoryInfo(h: isize, p: *mut Pmc, cb: u32) -> i32;
    }
    fn private_mb() -> f64 {
        let mut p = Pmc { cb: std::mem::size_of::<Pmc>() as u32, ..Default::default() };
        unsafe { K32GetProcessMemoryInfo(GetCurrentProcess(), &mut p, p.cb) };
        p.private as f64 / 1048576.0
    }

    fn run(file: &str) {
        let path = Path::new("testdata").join(file);
        if !path.exists() {
            eprintln!("跳过 {file}（不存在）");
            return;
        }
        let mem0 = private_mb();
        let t = Instant::now();
        let sheets = loader::open_tables(&path).unwrap();
        let src = sheets[0].source.clone();
        src.prepare().unwrap();
        while src.is_loading() {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let n = src.row_count();
        eprintln!("== {file}: {n} 行 × {} 列", src.headers().len());
        eprintln!("  打开+全量解码   {:>9.1?}   数据内存 {:.0} MB", t.elapsed(), private_mb() - mem0);
        // xls/xlsb/ods 和 xlsx 兜底走的都是 calamine 整表解码这条路径
        if file.ends_with(".xlsx") {
            let t = Instant::now();
            let sheets = loader::open_xlsx_with(&path, std::sync::Arc::new(|_| {}), false).unwrap();
            let src = sheets[0].source.clone();
            src.prepare().unwrap();
            while src.is_loading() {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            let n = src.row_count();
            eprintln!("  calamine 路径   {:>9.1?}   {} 行", t.elapsed(), n);
        }
    }

    #[test]
    #[ignore]
    fn bench_ops() {
        for f in ["big1m.xlsx", "big.csv", "big.parquet", "big200k.ods"] {
            run(f);
        }
    }


}
