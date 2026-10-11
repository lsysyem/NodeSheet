//! 表格里选中的格子复制到剪贴板。
//!
//! 选择 = 主矩形（界面属性 sel-r0/c0/r1/c1 + 整行 / 整列模式，普通点击 / 拖动 / Shift 照旧）
//! + Ctrl 加选的不连续行 / 列块（存在 App 里，纯回调给界面高亮和状态栏用）。
//! 块的有效性看 sel_extra_gen 对不对得上界面的 sel-gen：开始一次新选择界面就 +1，对不上作废。

use crate::*;

/// 把 x 加进 / 移出区间表（排序、不相交）：在某个区间里就去掉（宽了拆开），否则插入并合并相邻。
pub(crate) fn toggle_range(v: &mut Vec<(i32, i32)>, x: i32) {
    if let Some(i) = v.iter().position(|&(a, b)| a <= x && x <= b) {
        let (a, b) = v[i];
        match (a == x, b == x) {
            (true, true) => {
                v.remove(i);
            }
            (true, false) => v[i] = (a + 1, b),
            (false, true) => v[i] = (a, b - 1),
            (false, false) => {
                v.insert(i + 1, (x + 1, b));
                v[i] = (a, x - 1);
            }
        }
        return;
    }
    let i = v.partition_point(|&(a, _)| a < x);
    v.insert(i, (x, x));
    if i + 1 < v.len() && v[i].1 + 1 == v[i + 1].0 {
        let b = v.remove(i + 1).1;
        v[i].1 = b;
    }
    if i > 0 && v[i - 1].1 + 1 == v[i].0 {
        let b = v.remove(i).1;
        v[i - 1].1 = b;
    }
}

/// 把锚点（按下的那格）所在的块延长到 x（Ctrl 按住拖行号 / 列名）。
/// 块表是排序的，“最后一块”不一定是锚点这块（往上拖时它在前面），不能按位置找。
pub(crate) fn drag_anchor_to(v: &mut Vec<(i32, i32)>, anchor: i32, x: i32) -> bool {
    if let Some(i) = v.iter().position(|&(a, b)| a <= anchor && anchor <= b) {
        let (a, b) = v[i];
        let (lo, hi) = (a.min(x), b.max(x));
        if (lo, hi) == (a, b) {
            return false;
        }
        v[i] = (lo, hi);
    } else {
        // 锚点不在任何块里（理论上不发生）：当作从锚点到 x 的新块
        v.push((anchor.min(x), anchor.max(x)));
    }
    // 延长后可能挨上别的块：重新合并
    *v = merge_ranges(std::mem::take(v));
    true
}

/// 从区间表里挖掉 [a, b]（拆开被跨的块）。
pub(crate) fn remove_span(v: &mut Vec<(i32, i32)>, a: i32, b: i32) {
    let (a, b) = (a.min(b), a.max(b));
    let mut out = Vec::new();
    for (x, y) in std::mem::take(v) {
        if y < a || x > b {
            out.push((x, y));
            continue;
        }
        if x < a {
            out.push((x, a - 1));
        }
        if y > b {
            out.push((b + 1, y));
        }
    }
    *v = out;
}

/// 排序、合并相交 / 相邻的区间。
pub(crate) fn merge_ranges(mut v: Vec<(i32, i32)>) -> Vec<(i32, i32)> {
    v.sort_unstable();
    let mut out: Vec<(i32, i32)> = Vec::with_capacity(v.len());
    for (a, b) in v {
        match out.last_mut() {
            Some(last) if a <= last.1 + 1 => last.1 = last.1.max(b),
            _ => out.push((a, b)),
        }
    }
    out
}

/// Ctrl 加选的块还在不在（gen 对不上就没了）。
fn extras_of(app: &AppWindow, st: &App) -> (Vec<(i32, i32)>, Vec<(i32, i32)>) {
    if st.sel_extra_gen == app.get_sel_gen() {
        (st.sel_extra_rows.clone(), st.sel_extra_cols.clone())
    } else {
        Default::default()
    }
}

/// 主矩形（整行 / 整列模式按整表展开）∪ 加选块里有没有 (r, c)。
fn cell_selected(
    rows_mode: bool,
    cols_mode: bool,
    r0: i32,
    r1: i32,
    c0: i32,
    c1: i32,
    rows: &[(i32, i32)],
    cols: &[(i32, i32)],
    r: i32,
    c: i32,
) -> bool {
    let in_primary =
        (cols_mode || (r0 <= r && r <= r1)) && (rows_mode || (c0 <= c && c <= c1));
    in_primary || rows.iter().any(|&(a, b)| a <= r && r <= b) || cols.iter().any(|&(a, b)| a <= c && c <= b)
}

/// 格子 (r, c) 是不是在加选块的外沿：(上, 下, 左, 右)。
/// 整行块：上下看块的首尾行，左右在表的边上；整列块对称。
/// 跨过这条边的邻居也被选中时不画（相邻块的高亮要连成一片）。
fn extra_edges(
    rows: &[(i32, i32)],
    cols: &[(i32, i32)],
    rows_mode: bool,
    cols_mode: bool,
    r0: i32,
    r1: i32,
    c0: i32,
    c1: i32,
    r: i32,
    c: i32,
    n_rows: usize,
    n_cols: usize,
) -> (bool, bool, bool, bool) {
    let last_row = n_rows.saturating_sub(1) as i32;
    let last_col = n_cols.saturating_sub(1) as i32;
    let sel = |r: i32, c: i32| cell_selected(rows_mode, cols_mode, r0, r1, c0, c1, rows, cols, r, c);
    let mut top = r == 0 && cols.iter().any(|&(a, b)| a <= c && c <= b);
    let mut bottom = r == last_row && cols.iter().any(|&(a, b)| a <= c && c <= b);
    let mut left = c == 0 && rows.iter().any(|&(a, b)| a <= r && r <= b);
    let mut right = c == last_col && rows.iter().any(|&(a, b)| a <= r && r <= b);
    if r >= 0 {
        if rows.iter().any(|&(a, _)| a == r) {
            top = true;
        }
        if rows.iter().any(|&(_, b)| b == r) {
            bottom = true;
        }
    }
    if c >= 0 {
        if cols.iter().any(|&(a, _)| a == c) {
            left = true;
        }
        if cols.iter().any(|&(_, b)| b == c) {
            right = true;
        }
    }
    if top && r > 0 && sel(r - 1, c) {
        top = false;
    }
    if bottom && r < last_row && sel(r + 1, c) {
        bottom = false;
    }
    if left && c > 0 && sel(r, c - 1) {
        left = false;
    }
    if right && c < last_col && sel(r, c + 1) {
        right = false;
    }
    (top, bottom, left, right)
}

/// 整行 / 整列模式下的 Ctrl 加选 / 减选，统一在一张区间表上做：第一次把主矩形并进来、
/// 主矩形置为无效（末尾 < 0），之后增、减都是对这张表的 toggle（自动合并相邻、拆分中间）。
fn pick_in_span(
    app: &AppWindow,
    st: &mut App,
    v: i32,
    lo: i32,
    hi: i32,
    list: impl FnOnce(&mut App) -> &mut Vec<(i32, i32)>,
    set_span: impl FnOnce(&AppWindow, i32, i32),
) {
    let (lo, hi) = (lo.min(hi), lo.max(hi));
    if st.sel_extra_gen != app.get_sel_gen() {
        st.sel_extra_rows.clear();
        st.sel_extra_cols.clear();
        st.sel_extra_gen = app.get_sel_gen();
    }
    let list = list(st);
    if list.is_empty() && hi >= 0 {
        list.push((lo, hi));
        *list = merge_ranges(std::mem::take(list));
        set_span(app, -2, -1);
    }
    toggle_range(list, v);
    let adding = list.iter().any(|&(a, b)| a <= v && v <= b);
    st.pick_anchor = v;
    st.pick_adding = adding;
    app.set_sel_epoch(app.get_sel_epoch() + 1);
}

/// 装上 Ctrl 加选的回调。纯回调里读 sel-epoch / sel-gen 建依赖：块变了（epoch +1）或
/// 拖动改了主矩形（r0/r1 这些）时，高亮和状态栏的绑定会重算。
pub(crate) fn install_sel_extra(app: &AppWindow, state: &Arc<Mutex<App>>) {
    let weak = app.as_weak();
    let st = state.clone();
    app.on_ctrl_pick_row(move |r| {
        let Some(app) = weak.upgrade() else { return };
        if !app.get_sel_rows() || app.get_sel_cols() {
            return;
        }
        let mut st = st.lock().unwrap();
        pick_in_span(
            &app,
            &mut st,
            r,
            app.get_sel_r0(),
            app.get_sel_r1(),
            |st| &mut st.sel_extra_rows,
            |app, a, b| {
                app.set_sel_r0(a);
                app.set_sel_r1(b);
            },
        );
    });
    let weak = app.as_weak();
    let st = state.clone();
    app.on_ctrl_pick_col(move |c| {
        let Some(app) = weak.upgrade() else { return };
        if !app.get_sel_cols() || app.get_sel_rows() {
            return;
        }
        let mut st = st.lock().unwrap();
        pick_in_span(
            &app,
            &mut st,
            c,
            app.get_sel_c0(),
            app.get_sel_c1(),
            |st| &mut st.sel_extra_cols,
            |app, a, b| {
                app.set_sel_c0(a);
                app.set_sel_c1(b);
            },
        );
    });
    let weak = app.as_weak();
    let st = state.clone();
    app.on_ctrl_drag_row_to(move |r| {
        let Some(app) = weak.upgrade() else { return };
        let mut st = st.lock().unwrap();
        if st.sel_extra_gen != app.get_sel_gen() {
            return;
        }
        let anchor = st.pick_anchor;
        let changed = if st.pick_adding {
            drag_anchor_to(&mut st.sel_extra_rows, anchor, r)
        } else {
            let anchor = st.pick_anchor;
            let before = st.sel_extra_rows.clone();
            remove_span(&mut st.sel_extra_rows, anchor.min(r), anchor.max(r));
            before != st.sel_extra_rows
        };
        if changed {
            app.set_sel_epoch(app.get_sel_epoch() + 1);
        }
    });
    let weak = app.as_weak();
    let st = state.clone();
    app.on_ctrl_drag_col_to(move |c| {
        let Some(app) = weak.upgrade() else { return };
        let mut st = st.lock().unwrap();
        if st.sel_extra_gen != app.get_sel_gen() {
            return;
        }
        let anchor = st.pick_anchor;
        let changed = if st.pick_adding {
            drag_anchor_to(&mut st.sel_extra_cols, anchor, c)
        } else {
            let anchor = st.pick_anchor;
            let before = st.sel_extra_cols.clone();
            remove_span(&mut st.sel_extra_cols, anchor.min(c), anchor.max(c));
            before != st.sel_extra_cols
        };
        if changed {
            app.set_sel_epoch(app.get_sel_epoch() + 1);
        }
    });
    let weak = app.as_weak();
    let st = state.clone();
    app.on_row_extra(move |r| {
        let Some(app) = weak.upgrade() else { return false };
        // 读属性 = 给调用中的绑定建依赖
        let _ = (app.get_sel_epoch(), app.get_sel_gen());
        // 界面线程外的锁不等着：拿不到就先当没有，反正马上会重算
        let Ok(st) = st.try_lock() else { return false };
        st.sel_extra_gen == app.get_sel_gen() && st.sel_extra_rows.iter().any(|&(a, b)| a <= r && r <= b)
    });
    let weak = app.as_weak();
    let st = state.clone();
    app.on_col_extra(move |c| {
        let Some(app) = weak.upgrade() else { return false };
        let _ = (app.get_sel_epoch(), app.get_sel_gen());
        let Ok(st) = st.try_lock() else { return false };
        st.sel_extra_gen == app.get_sel_gen() && st.sel_extra_cols.iter().any(|&(a, b)| a <= c && c <= b)
    });
    let weak = app.as_weak();
    let st = state.clone();
    app.on_has_extra_cols(move || {
        let Some(app) = weak.upgrade() else { return false };
        let _ = (app.get_sel_epoch(), app.get_sel_gen());
        let Ok(st) = st.try_lock() else { return false };
        st.sel_extra_gen == app.get_sel_gen() && !st.sel_extra_cols.is_empty()
    });
    // 加选块的描边：格子在外沿的哪个方向（读 sel-epoch / sel-gen 建依赖）
    macro_rules! edge_cb {
        ($on:ident, $idx:tt) => {
            let weak = app.as_weak();
            let st = state.clone();
            app.$on(move |r, c| {
                let Some(app) = weak.upgrade() else { return false };
                let _ = (app.get_sel_epoch(), app.get_sel_gen());
                let Ok(st) = st.try_lock() else { return false };
                if st.sel_extra_gen != app.get_sel_gen() {
                    return false;
                }
                let (n_rows, n_cols) = st
                    .sheets
                    .get(st.current)
                    .map_or((0, 0), |s| (s.source.row_count(), s.source.headers().len()));
                let (r0, r1) = (app.get_sel_r0(), app.get_sel_r1());
                let (c0, c1) = (app.get_sel_c0(), app.get_sel_c1());
                extra_edges(
                    &st.sel_extra_rows,
                    &st.sel_extra_cols,
                    app.get_sel_rows(),
                    app.get_sel_cols(),
                    r0.min(r1),
                    r0.max(r1),
                    c0.min(c1),
                    c0.max(c1),
                    r,
                    c,
                    n_rows,
                    n_cols,
                )
                .$idx
            });
        };
    }
    edge_cb!(on_cell_extra_top, 0);
    edge_cb!(on_cell_extra_bottom, 1);
    edge_cb!(on_cell_extra_left, 2);
    edge_cb!(on_cell_extra_right, 3);
    let weak = app.as_weak();
    let st = state.clone();
    app.on_sel_info_text(move || {
        let Some(app) = weak.upgrade() else { return SharedString::from("") };
        if !app.get_sel_active() {
            return SharedString::from("");
        }
        let (r0, r1, c0, c1) = (app.get_sel_r0(), app.get_sel_r1(), app.get_sel_c0(), app.get_sel_c1());
        let (rows_mode, cols_mode) = (app.get_sel_rows(), app.get_sel_cols());
        let _ = (app.get_sel_gen(), app.get_sel_epoch());
        let Ok(st) = st.try_lock() else { return SharedString::from("") };
        let (extra_rows, extra_cols) = extras_of(&app, &st);
        let (n_rows, n_cols) = st
            .sheets
            .get(st.current)
            .map_or((0, 0), |s| (s.source.row_count(), s.source.headers().len()));
        drop(st);
        let len = |v: Vec<(i32, i32)>| {
            merge_ranges(v)
                .iter()
                .filter(|&(_, b)| *b >= 0)
                .map(|&(a, b)| b.max(0) as usize - a.max(0) as usize + 1)
                .sum::<usize>()
        };
        let (n_rows_sel, n_cols_sel) = if cols_mode {
            let cols = if rows_mode {
                n_cols
            } else {
                let mut cs = extra_cols;
                if c0.max(c1) >= 0 {
                    cs.push((c0.min(c1), c0.max(c1)));
                }
                len(cs)
            };
            (n_rows, cols)
        } else {
            let mut rs = extra_rows;
            if r0.max(r1) >= 0 {
                rs.push((r0.min(r1), r0.max(r1)));
            }
            let rows = len(rs);
            let cols = if rows_mode {
                n_cols
            } else {
                let mut cs = extra_cols;
                if c0.max(c1) >= 0 {
                    cs.push((c0.min(c1), c0.max(c1)));
                }
                len(cs)
            };
            (rows, cols)
        };
        SharedString::from(format!("已选 {n_rows_sel} 行 × {n_cols_sel} 列"))
    });
}

/// 复制出来的一格：含制表符、换行、引号的按 Excel 的习惯用引号包起来。
pub(crate) fn push_cell(out: &mut String, cell: &str) {
    if cell.contains(['\t', '\n', '\r', '"']) {
        out.push('"');
        out.push_str(&cell.replace('"', "\"\""));
        out.push('"');
    } else {
        out.push_str(cell);
    }
}

/// [r0, r1] × cols 这一块的文本：列之间制表符、行之间 \r\n（粘到 Excel 里正好一格一格对上）。
pub(crate) fn selection_text(source: &dyn DataSource, rows: std::ops::Range<usize>, cols: &[usize], with_header: bool) -> String {
    let mut out = String::new();
    if with_header {
        for (i, &c) in cols.iter().enumerate() {
            if i > 0 {
                out.push('\t');
            }
            push_cell(&mut out, source.headers().get(c).map_or("", |h| h.as_str()));
        }
        out.push_str("\r\n");
    }
    source.scan(rows.start, rows.len(), cols, &mut |_, cells| {
        for (i, cell) in cells.iter().enumerate() {
            if i > 0 {
                out.push('\t');
            }
            push_cell(&mut out, cell);
        }
        out.push_str("\r\n");
    });
    // 只有一格时不带换行：粘进输入框不会多出一行
    if rows.len() == 1 && cols.len() == 1 && !with_header {
        out.truncate(out.trim_end_matches("\r\n").len());
    }
    out
}

/// 选中范围分成几段连续的行：(行区间, 这一行的列)。列是 None = 整行所有列。
pub(crate) type SelGroup = (std::ops::Range<usize>, Option<Vec<usize>>);

/// 界面上的选择展开成行分段：主矩形（整行 / 整列模式按整表展开）∪ Ctrl 加选的行 / 列块。
/// 加选行块的行是整行所有列；其余行走“基本列” = 主矩形的列（整列模式）∪ 加选列块。
pub(crate) fn sel_groups(app: &AppWindow, st: &App, n_rows: usize, n_cols: usize) -> Vec<SelGroup> {
    let (extra_rows, extra_cols) = extras_of(app, st);
    sel_groups_impl(
        app.get_sel_rows(),
        app.get_sel_cols(),
        app.get_sel_r0(),
        app.get_sel_r1(),
        app.get_sel_c0(),
        app.get_sel_c1(),
        extra_rows,
        extra_cols,
        n_rows,
        n_cols,
    )
}

fn sel_groups_impl(
    rows_mode: bool,
    cols_mode: bool,
    r0: i32,
    r1: i32,
    c0: i32,
    c1: i32,
    extra_rows: Vec<(i32, i32)>,
    extra_cols: Vec<(i32, i32)>,
    n_rows: usize,
    n_cols: usize,
) -> Vec<SelGroup> {
    if n_rows == 0 || n_cols == 0 {
        return vec![];
    }
    let clamp = |r: (i32, i32)| {
        if r.1 < 0 {
            return 0..0;
        }
        let (a, b) = (r.0.max(0) as usize, r.1.max(0) as usize);
        let (lo, hi) = (a.min(n_rows - 1), b.min(n_rows - 1));
        if lo > hi { lo..lo } else { lo..hi + 1 }
    };
    let cols_of = |v: Vec<(i32, i32)>| {
        let mut out: Vec<usize> = vec![];
        for (a, b) in merge_ranges(v) {
            if b < 0 {
                continue;
            }
            let (a, b) = (a.max(0) as usize, b.max(0) as usize);
            if a < n_cols {
                out.extend(a..b.min(n_cols - 1) + 1);
            }
        }
        out
    };
    if cols_mode {
        // 所有行 × 选中的列；整行也开着（全选）时就是全部列
        let cols = if rows_mode { None } else {
            let mut cs = extra_cols;
            if c0.max(c1) >= 0 {
                cs.push((c0.min(c1), c0.max(c1)));
            }
            Some(cols_of(cs))
        };
        return vec![(0..n_rows, cols)];
    }
    let primary = clamp((r0.min(r1), r0.max(r1)));
    if rows_mode {
        // 整行模式：这些行 × 所有列，加上加选的行块（也是整行）
        let mut rs = extra_rows;
        let mut groups = vec![];
        // 加选块和主矩形可能重叠：一起排序合并（主矩形已 Ctrl 化时是作废的负区间，clamp 会丢掉）
        if r0.max(r1) >= 0 {
            rs.push((r0.min(r1), r0.max(r1)));
        }
        for (a, b) in merge_ranges(rs) {
            let r = clamp((a, b));
            if !r.is_empty() {
                groups.push((r, None));
            }
        }
        return groups;
    }
    // 矩形模式：加选行块的行是整行，其余行取基本列 = 主矩形列 ∪ 加选列块
    let mut base = extra_cols;
    if c0.max(c1) >= 0 {
        base.push((c0.min(c1), c0.max(c1)));
    }
    let base = cols_of(base);
    // 主矩形被加选行块切开：交叉的地方整行，剩下的基本列
    let mut segs: Vec<SelGroup> = vec![];
    let mut rest = Some(primary);
    for (a, b) in merge_ranges(extra_rows) {
        let Some(p) = rest.clone() else {
            let r = clamp((a, b));
            if !r.is_empty() {
                segs.push((r, None));
            }
            continue;
        };
        let e = clamp((a, b));
        let e_end = e.end;
        if e.is_empty() {
            continue;
        }
        if e.end <= p.start || e.start > p.end {
            // 不相交
            if e.end <= p.start {
                segs.push((e, None));
            } else {
                segs.push((p.clone(), Some(base.clone())));
                rest = None;
                segs.push((e, None));
            }
            continue;
        }
        // 相交：P 在块前面的部分是基本列，相交段整行
        if p.start < e.start {
            segs.push((p.start..e.start, Some(base.clone())));
        }
        segs.push((e, None));
        rest = if p.end >= e_end { Some(e_end..p.end) } else { None };
    }
    if let Some(p) = rest {
        if !p.is_empty() {
            segs.push((p, Some(base)));
        }
    }
    segs.retain(|(r, _)| !r.is_empty());
    segs
}

/// 把选中的格子复制到剪贴板。整行 / 整列可能很大，取数放到后台线程。
pub(crate) fn copy_selection(weak: &slint::Weak<AppWindow>, state: &Arc<Mutex<App>>, with_header: bool) {
    let Some(app) = weak.upgrade() else { return };
    if !app.get_sel_active() {
        return;
    }
    let Some(source) = ({
        let st = state.lock().unwrap();
        st.sheets.get(st.current).map(|s| s.source.clone())
    }) else {
        return;
    };
    let (n_rows, n_cols) = (source.row_count(), source.headers().len());
    // 界面上的起点 / 终点 → 夹到表的范围里；Ctrl 加选的行 / 列块并进来
    let groups = {
        let st = state.lock().unwrap();
        sel_groups(&app, &st, n_rows, n_cols)
    };
    let rows: usize = groups.iter().map(|(r, _)| r.len()).sum();
    let cols: usize = groups.iter().map(|(_, c)| c.as_ref().map_or(n_cols, |c| c.len())).max().unwrap_or(0);
    if rows == 0 || cols == 0 {
        return;
    }
    let shape = format!("{} 行 × {} 列", fmt_count(rows), cols);
    if rows * cols > 200_000 {
        app.set_status(SharedString::from(format!("正在复制 {shape}…")));
    }
    let weak = weak.clone();
    std::thread::spawn(move || {
        let mut text = String::new();
        for (i, (rows, cols)) in groups.iter().enumerate() {
            let all: Vec<usize> = (0..n_cols).collect();
            text.push_str(&selection_text(
                source.as_ref(),
                rows.clone(),
                cols.as_ref().unwrap_or(&all),
                with_header && i == 0,
            ));
        }
        // 只有一格时不带换行：粘进输入框不会多出一行
        if rows == 1 && cols == 1 && !with_header {
            text.truncate(text.trim_end_matches("\r\n").len());
        }
        let result = clipboard::set_text(&text);
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(app) = weak.upgrade() {
                app.set_status(SharedString::from(match result {
                    Ok(()) => format!("已复制 {shape}"),
                    Err(e) => format!("复制失败：{e}"),
                }));
            }
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn range_str(groups: &[SelGroup]) -> String {
        groups
            .iter()
            .map(|(r, c)| match c {
                None => format!("{}..{} 全行", r.start, r.end),
                Some(c) => format!("{}..{} 列{:?}", r.start, r.end, c),
            })
            .collect::<Vec<_>>()
            .join(" | ")
    }

    #[test]
    fn toggle_ranges() {
        let mut v = vec![];
        toggle_range(&mut v, 3);
        toggle_range(&mut v, 5);
        toggle_range(&mut v, 4);
        assert_eq!(v, [(3, 5)]);
        toggle_range(&mut v, 4);
        assert_eq!(v, [(3, 3), (5, 5)]);
        toggle_range(&mut v, 3);
        assert_eq!(v, [(5, 5)]);
        // 去掉宽区间中间的
        let mut v = vec![(1, 5)];
        toggle_range(&mut v, 3);
        assert_eq!(v, [(1, 2), (4, 5)]);
    }

    #[test]
    fn merge_sorts_and_joins() {
        assert_eq!(merge_ranges(vec![(5, 6), (1, 3), (4, 9), (20, 21)]), [(1, 9), (20, 21)]);
        assert_eq!(merge_ranges(vec![]), Vec::<(i32, i32)>::new());
    }

    #[test]
    fn groups_rect_plus_extra_rows() {
        // 主矩形 2..6 列 1..4，加选整行 4、8：4 行整行，其余基本列 1..4
        let g = sel_groups_impl(false, false, 2, 5, 1, 3, vec![(4, 4), (8, 8)], vec![], 20, 10);
        assert_eq!(range_str(&g), "2..4 列[1, 2, 3] | 4..5 全行 | 5..6 列[1, 2, 3] | 8..9 全行");
    }

    #[test]
    fn groups_extra_cols() {
        // 整列模式选 1..3，加选列 5、6：所有行 × [1,2,5,6]
        let g = sel_groups_impl(false, true, 0, 0, 1, 2, vec![], vec![(5, 5), (6, 6)], 20, 10);
        assert_eq!(range_str(&g), "0..20 列[1, 2, 5, 6]");
    }

    #[test]
    fn groups_rows_mode() {
        // 整行模式 1..4 加选行 6：都是整行
        let g = sel_groups_impl(true, false, 1, 3, 0, 0, vec![(6, 6)], vec![], 20, 10);
        assert_eq!(range_str(&g), "1..4 全行 | 6..7 全行");
    }

    #[test]
    fn groups_extra_rows_around_primary() {
        // 加选行块盖住主矩形前后：相交段整行
        let g = sel_groups_impl(false, false, 5, 7, 1, 2, vec![(0, 6)], vec![], 20, 10);
        assert_eq!(range_str(&g), "0..7 全行 | 7..8 列[1, 2]");
    }

    #[test]
    fn extra_block_edges() {
        // 整行块 2..4：首行上沿、尾行下沿，左右在表边上（10 列，边是第 9 列）；主矩形不挨着
        let rows = vec![(2, 4)];
        let none = (false, false, 0, 0, false, false); // rows_mode, cols_mode, r0, r1, c0, c1 的空选择
        let (rm, cm, r0, r1, c0, c1) = (false, false, -1, -1, -1, -1);
        assert_eq!(extra_edges(&rows, &[], rm, cm, r0, r1, c0, c1, 2, 5, 20, 10), (true, false, false, false));
        assert_eq!(extra_edges(&rows, &[], rm, cm, r0, r1, c0, c1, 3, 5, 20, 10), (false, false, false, false));
        assert_eq!(extra_edges(&rows, &[], rm, cm, r0, r1, c0, c1, 4, 5, 20, 10), (false, true, false, false));
        assert_eq!(extra_edges(&rows, &[], rm, cm, r0, r1, c0, c1, 2, 0, 20, 10), (true, false, true, false));
        assert_eq!(extra_edges(&rows, &[], rm, cm, r0, r1, c0, c1, 4, 9, 20, 10), (false, true, false, true));
        // 整列块 1..2：列首尾 + 表的上下边（20 行，边是第 19 行）
        let cols = vec![(1, 2)];
        assert_eq!(extra_edges(&[], &cols, rm, cm, r0, r1, c0, c1, 10, 1, 20, 10), (false, false, true, false));
        assert_eq!(extra_edges(&[], &cols, rm, cm, r0, r1, c0, c1, 10, 3, 20, 10), (false, false, false, false));
        assert_eq!(extra_edges(&[], &cols, rm, cm, r0, r1, c0, c1, 0, 1, 20, 10), (true, false, true, false));
        assert_eq!(extra_edges(&[], &cols, rm, cm, r0, r1, c0, c1, 19, 2, 20, 10), (false, true, false, true));
        // 邻居也被选中：不画（整行块 2..4 挨着主矩形整行 0..1，块顶不画线）
        assert_eq!(extra_edges(&rows, &[], true, false, 0, 1, 0, 0, 2, 5, 20, 10), (false, false, false, false));
        // 块尾行的下沿照画（下面的行没选）
        assert_eq!(extra_edges(&rows, &[], true, false, 0, 1, 0, 0, 4, 5, 20, 10), (false, true, false, false));
    }

    #[test]
    fn drag_extends_anchor_block() {
        // 先点了 8，Ctrl 又点了 5：往上拖到 3 只延长 5 的块，不裹进 8
        let mut v = vec![(5, 5), (8, 8)];
        assert!(drag_anchor_to(&mut v, 5, 3));
        assert_eq!(v, [(3, 5), (8, 8)]);
        // 再往下拖回 6：锚点块（3..5）跟着延长，3 还选着
        assert!(drag_anchor_to(&mut v, 5, 6));
        assert_eq!(v, [(3, 6), (8, 8)]);
        // 拖到挨着 8：合并
        assert!(drag_anchor_to(&mut v, 6, 7));
        assert_eq!(v, [(3, 8)]);
    }

    #[test]
    fn remove_span_splits() {
        let mut v = vec![(2, 4), (6, 9)];
        remove_span(&mut v, 3, 3);
        assert_eq!(v, [(2, 2), (4, 4), (6, 9)]);
        remove_span(&mut v, 7, 12);
        assert_eq!(v, [(2, 2), (4, 4), (6, 6)]);
        remove_span(&mut v, 0, 20);
        assert_eq!(v, Vec::<(i32, i32)>::new());
    }

    #[test]
    fn groups_inert_primary() {
        // Ctrl 化之后主矩形是作废的负区间（-2, -1）：只按加选的块算
        let g = sel_groups_impl(true, false, -2, -1, -2, -1, vec![(1, 2), (5, 7)], vec![], 20, 10);
        assert_eq!(range_str(&g), "1..3 全行 | 5..8 全行");
        // 整列模式同理
        let g = sel_groups_impl(false, true, -2, -1, -2, -1, vec![], vec![(0, 0), (3, 4)], 20, 10);
        assert_eq!(range_str(&g), "0..20 列[0, 3, 4]");
    }

    #[test]
    fn groups_clamped() {
        // 负数 / 超界都夹进表里
        let g = sel_groups_impl(false, false, -3, 2, -2, 1, vec![(18, 30)], vec![], 20, 10);
        assert_eq!(range_str(&g), "0..3 列[0, 1] | 18..20 全行");
    }
}
