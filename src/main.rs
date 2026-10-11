#![cfg_attr(all(target_os = "windows", not(debug_assertions)), windows_subsystem = "windows")]

//! 模块分工：
//! - 数据：`loader`（打开文件、按需取数）、`xlsx_fast`
//! - 节点：`nodes`（节点类型的定义和注册表）、`graph`（节点图的结构）、`engine`（Python + polars 子进程）
//! - 界面胶水：`view`（表格）、`selection`（选择 / 复制）、`files`（打开文件、重新加载、拖放）、`project`（工程文件）、
//!   `panel`（节点面板）、`type_editor`（节点类型 / 参数界面编辑器）、`run`（执行节点图）、`complete`（代码编辑辅助）、
//!   `help`（节点的帮助窗口）、`undo`（撤销 / 重做、改过了没存）、`settings`（个人设置、最近打开）、
//!   `quick`（表格右键菜单建节点）
//!
//! 界面胶水的模块都 `use crate::*`，互相直接调用；状态集中在 `App`。

mod clipboard;
mod complete;
mod convert;
mod engine;
mod files;
mod graph;
mod help;
mod loader;
mod nodes;
mod panel;
mod pick;
mod project;
mod quick;
mod run;
mod selection;
mod settings;
mod type_editor;
mod undo;
mod view;
mod xlsx_fast;

use convert::*;
use files::*;
use loader::{DataSource, SheetHandle};
use nodes::{NodeType, ParamKind};
use panel::*;
use project::*;
use run::*;
use selection::*;
use view::*;
use slint::language::StandardListViewItem;
use slint::{Model, ModelNotify, ModelRc, ModelTracker, SharedString, VecModel};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex};

slint::include_modules!();

#[derive(Default)]
struct App {
    /// 表格正在显示的表：文件的各个 sheet，或某个节点的结果（只有一张）
    sheets: Vec<SheetHandle>,
    current: usize,
    /// 设置：表格数据单元格自动换行
    wrap_cells: bool,
    /// 各列宽度（含用户拖拽结果）和是否数值列（取样判定，右对齐用）；换表时清空
    col_widths: Vec<f32>,
    col_numeric: Vec<bool>,
    /// 主题：0 跟随系统 / 1 浅色 / 2 深色
    theme_mode: i32,
    /// 节点图
    graph: graph::Graph,
    /// 节点面板正显示哪一级：空 = 表格级的节点图；[expr] = 那个 expr 节点里面的表达式网络；
    /// 再往后是网络里带网络的节点（eval）的 id，一级一级进去
    inside: Vec<u32>,
    /// expr 节点里看的是哪张网络（group_by 分成分组键 / 聚合 / 筛选组三张，见 graph::Env）。
    /// 当前上下文用不到它时看第一张（`env()`）
    env: graph::Env,
    /// 表格正显示节点结果时，文件本身的 sheets 和当前 sheet 暂存在这里
    file_sheets: Option<(Vec<SheetHandle>, usize)>,
    /// 查看器自己显示源文件、又不知道类型时（CSV / Excel 按文本读）：引擎推出来的源节点各列的 (列名, 类型)，
    /// 连同当时源节点参数的指纹（见 run::native_key；换了 sheet / 改了参数就不作数）。asked：已经去要过的指纹
    native_types: Option<(String, Vec<(String, String)>)>,
    native_types_asked: String,
    /// 当前工程存在哪（“保存”直接写回这里）；打开的是表格文件、还没存过时为空
    project: Option<PathBuf>,
    run_ticket: u64,
    running: bool,
    /// 写出节点正在写文件
    writing: bool,
    /// 状态栏左侧：显示节点结果时的说明（节点名 · 耗时）
    node_status: String,
    /// 表格里现在是哪个节点的结果、算了多久（表格路径栏）；显示的是文件本身时为空。
    /// 显示标记换了但还没跑出来（或跑失败了）时，表格里还是上一次的结果，路径栏照实说
    shown: Option<u32>,
    /// 表格里是 shown 里面某个表达式节点的值（预览）
    shown_sub: Vec<u32>,
    /// 预览的说明（filter 里预览真假值：满足几行），表格路径栏里写
    preview_note: String,
    last_ms: Option<u64>,
    node_error: String,
    node_output: String,
    /// 引擎写出的结果文件（只留最近两个）
    result_files: Vec<PathBuf>,
    /// 代码编辑框当前弹出的补全：选中后替换 [completion_start, 光标)
    completions: Vec<complete::Item>,
    completion_start: usize,
    /// 开着的选列框（参数旁边的“选列”）
    pick: Option<pick::PickState>,
    /// Tab 菜单当前列出的搜索结果
    search_hits: Vec<nodes::Hit>,
    /// Tab 菜单里写的是一段表达式：第一项是“按文字建节点”，建的就是它
    search_text: Option<String>,
    /// “添加”菜单里列出的节点类型（界面按下标报回来）：当前这一级的（进到表达式网络里是表达式节点）
    kinds: Vec<Arc<NodeType>>,
    /// 工具架上的节点类型：总是表格级的
    shelf_kinds: Vec<Arc<NodeType>>,
    /// 节点类型编辑器正在编辑的草稿
    editor: Option<type_editor::Editor>,
    /// 帮助窗口正显示的一页，和“后退”能回到的页
    help: Option<help::Page>,
    help_back: Vec<help::Page>,
    /// 画布上正按着 / 拖着的节点（见 panel::NodeDrag）
    node_drag: Option<NodeDrag>,
    /// 按着 Y 划出来的剪线轨迹（画布坐标），松手时剪断和它相交的连线
    cut: Vec<(f32, f32)>,
    /// 复制的节点（Graph::copy_nodes 的 JSON）：Ctrl+V 贴到当前这一级
    node_clip: Option<serde_json::Value>,
    /// 节点图的撤销 / 重做，和改过了没存（见 undo.rs）
    history: undo::History,
    /// “文件”菜单里列出来的最近打开的文件（菜单按下标报回来）
    recent: Vec<PathBuf>,
    /// 打开数据文件时它的修改时间：窗口重新得到焦点时比一比，外面改过了就提醒重新加载
    file_stamp: Option<std::time::SystemTime>,
    /// Ctrl 加选的不连续行 / 列块（排序、不相交的区间表，见 selection.rs）。
    /// sel_extra_gen 记的是哪个 sel-gen 时改的：界面开始一次新选择（sel-gen +1）就对不上、块作废
    sel_extra_rows: Vec<(i32, i32)>,
    sel_extra_cols: Vec<(i32, i32)>,
    sel_extra_gen: i32,
    /// Ctrl 按住的拖动：锚点 + 方向（true = 加选）。按下时定，拖动经过的行 / 列统一应用
    pick_anchor: i32,
    pick_adding: bool,
}

impl App {
    /// 换 sheet / 换文件 / 换节点结果时重置列宽等显示状态。
    fn reset_sheet_view(&mut self) {
        self.col_widths.clear();
        self.col_numeric.clear();
    }

    /// path 那一级的图：空 = 表格级的；[expr, eval…] 一级一级往里（expr 节点里是 env 那张网络）。有一级不在了是 None。
    fn graph_at(&self, path: &[u32]) -> Option<&graph::Graph> {
        let mut g = &self.graph;
        for (depth, id) in path.iter().enumerate() {
            let node = g.node(*id)?;
            g = if depth == 0 { node.net_in(node.shown_env(self.env))? } else { node.net.as_deref()? };
        }
        Some(g)
    }

    fn graph_at_mut(&mut self, path: &[u32]) -> Option<&mut graph::Graph> {
        let want = self.env;
        let mut g = &mut self.graph;
        for (depth, id) in path.iter().enumerate() {
            let node = g.node_mut(*id)?;
            g = if depth == 0 { node.net_in_mut(node.shown_env(want))? } else { node.net.as_deref_mut()? };
        }
        Some(g)
    }

    /// 正看着 expr 节点里的哪张网络（不在网络里时是 Main）。
    fn env(&self) -> graph::Env {
        self.top_node().map_or(graph::Env::Main, |n| n.shown_env(self.env))
    }

    /// 节点面板正显示的那张图：表格级的，或者进到的那一级表达式网络。
    fn net(&self) -> &graph::Graph {
        self.graph_at(&self.inside).unwrap_or(&self.graph)
    }

    fn net_mut(&mut self) -> &mut graph::Graph {
        if self.graph_at(&self.inside).is_none() {
            return &mut self.graph;
        }
        let path = self.inside.clone();
        self.graph_at_mut(&path).expect("刚查过")
    }

    /// 进去的那个节点（当前这张网络的主人：expr 节点，或网络里的 eval 节点），还在的话。
    fn inside_node(&self) -> Option<&graph::Node> {
        let (last, parent) = self.inside.split_last()?;
        self.graph_at(parent)?.node(*last).filter(|n| n.net.is_some())
    }

    /// 最外面的 expr 节点（表格级的图里那个）：表格显示它的结果、选列框列的是它的输入的列。
    fn top_node(&self) -> Option<&graph::Node> {
        self.inside.first().and_then(|id| self.graph.node(*id)).filter(|n| n.net.is_some())
    }

    /// 参数面板正在编辑哪个节点：(它所在的图的路径, id)；路径不空 = 表达式级的节点。
    /// 网络里没选中节点时是网络的主人自己（像 Houdini：VOP 里空选时参数面板显示外面那个节点）。
    fn param_target(&self) -> (Vec<u32>, Option<u32>) {
        if self.inside_node().is_none() {
            return (Vec::new(), self.graph.selected);
        }
        match self.net().selected {
            Some(sub) => (self.inside.clone(), Some(sub)),
            None => (self.inside[..self.inside.len() - 1].to_vec(), self.inside.last().copied()),
        }
    }

    /// 参数面板正在编辑的节点所在的图，和它的 id。
    fn param_graph(&mut self) -> (&mut graph::Graph, Option<u32>) {
        let (path, id) = self.param_target();
        if self.graph_at(&path).is_none() {
            return (&mut self.graph, None);
        }
        (self.graph_at_mut(&path).expect("刚查过"), id)
    }
}

/// 窗口标题栏（系统画的）跟着主题变深浅：跟随系统时交还给系统。
pub(crate) fn title_bar_theme(app: &AppWindow, mode: i32) {
    use slint::winit_030::{WinitWindowAccessor, winit::window::Theme};
    let theme = match mode {
        1 => Some(Theme::Light),
        2 => Some(Theme::Dark),
        _ => None,
    };
    app.window().with_winit_window(|w| w.set_theme(theme));
}

fn main() {
    // 以前被结束 / 崩溃的进程留下的临时目录
    engine::sweep_stale();
    let app = AppWindow::new().expect("无法创建窗口");
    // 默认深色（Houdini 的样子）；上次的主题、布局、窗口大小、最近打开的文件在个人设置里
    let state = Arc::new(Mutex::new(App { theme_mode: 2, ..App::default() }));
    app.set_theme_mode(2);
    settings::apply(&app, &mut state.lock().unwrap());

    {
        let weak = app.as_weak();
        let state = state.clone();
        app.on_open_file(move || {
            // JSON / Arrow / Avro 查看器自己读不了，打开时交给引擎（files::engine_reads）
            let path = rfd::FileDialog::new()
                .add_filter(
                    "表格文件 / 工程",
                    &[
                        "xlsx", "xls", "xlsm", "xlsb", "ods", "parquet", "tsv", "csv", "json", "ndjson", "jsonl", "arrow",
                        "feather", "ipc", "arrows", "avro", PROJECT_EXT,
                    ],
                )
                .add_filter("工程文件", &[PROJECT_EXT])
                .set_title("打开表格文件或工程")
                .pick_file();
            if let Some(path) = path.filter(|_| confirm_discard(&weak, &state)) {
                open_file(path, weak.clone(), state.clone());
            }
        });
    }

    {
        let weak = app.as_weak();
        let state = state.clone();
        app.on_quit(move || {
            if confirm_discard(&weak, &state) {
                if let Some(app) = weak.upgrade() {
                    settings::save(&app, &state.lock().unwrap());
                }
                engine::cleanup();
                std::process::exit(0);
            }
        });
    }
    {
        // 点窗口右上角的 ✕：节点图改过了先问要不要存；关之前记下窗口大小、布局
        let weak = app.as_weak();
        let state = state.clone();
        app.window().on_close_requested(move || {
            if !confirm_discard(&weak, &state) {
                return slint::CloseRequestResponse::KeepWindowShown;
            }
            if let Some(app) = weak.upgrade() {
                settings::save(&app, &state.lock().unwrap());
            }
            slint::CloseRequestResponse::HideWindow
        });
    }
    {
        let weak = app.as_weak();
        let state = state.clone();
        app.on_open_recent(move |index| {
            let path = state.lock().unwrap().recent.get(index.max(0) as usize).cloned();
            if let Some(path) = path.filter(|_| confirm_discard(&weak, &state)) {
                open_file(path, weak.clone(), state.clone());
            }
        });
    }
    {
        let weak = app.as_weak();
        let state = state.clone();
        app.on_reload_file(move || reload_file(&weak, &state));
    }

    {
        let weak = app.as_weak();
        let state = state.clone();
        app.on_save_project(move |save_as| {
            save_project(&weak, &state, save_as);
        });
    }
    {
        let weak = app.as_weak();
        let state = state.clone();
        app.on_undo(move |redo| undo::undo_redo(&weak, &state, redo));
    }

    {
        let weak = app.as_weak();
        let state = state.clone();
        app.on_set_theme(move |mode| {
            let mut st = state.lock().unwrap();
            st.theme_mode = mode.clamp(0, 2);
            if let Some(app) = weak.upgrade() {
                app.set_theme_mode(st.theme_mode);
                title_bar_theme(&app, st.theme_mode);
                settings::save(&app, &st);
            }
        });
    }
    {
        // 主题模式从界面读（这时 Rust 那边可能正拿着状态锁）
        let weak = app.as_weak();
        app.on_dark_changed(move |_| {
            if let Some(app) = weak.upgrade() {
                title_bar_theme(&app, app.get_theme_mode());
            }
        });
    }

    {
        let weak = app.as_weak();
        let state = state.clone();
        app.on_toggle_wrap(move || {
            let mut st = state.lock().unwrap();
            st.wrap_cells = !st.wrap_cells;
            if let Some(app) = weak.upgrade() {
                app.set_settings_wrap(st.wrap_cells);
                settings::save(&app, &st);
            }
        });
    }

    {
        // 设置 → 显示格式：打开对话框前把当前的值推进去
        let weak = app.as_weak();
        app.on_show_display_fmt(move || {
            let cfg = loader::display_fmt();
            if let Some(app) = weak.upgrade() {
                app.set_fmt_float_mode(cfg.float_mode);
                app.set_fmt_float_digits(cfg.float_digits.to_string().into());
                app.set_fmt_thousands(cfg.thousands);
                app.set_fmt_trim(cfg.trim_zeros);
                app.set_fmt_max_cell(cfg.max_cell.to_string().into());
                app.set_fmt_null(cfg.null.into());
                app.set_fmt_visible(true);
            }
        });
    }
    {
        // 对话框点“确定”：应用、存盘、按新的格式重新取样列宽
        let weak = app.as_weak();
        let state = state.clone();
        app.on_apply_display_fmt(move || {
            let Some(app) = weak.upgrade() else { return };
            let mut st = state.lock().unwrap();
            let cfg = loader::DisplayFmt {
                float_mode: app.get_fmt_float_mode().clamp(0, 2),
                float_digits: app.get_fmt_float_digits().trim().parse().unwrap_or(2).clamp(0, 10),
                thousands: app.get_fmt_thousands(),
                trim_zeros: app.get_fmt_trim(),
                max_cell: app.get_fmt_max_cell().trim().parse().unwrap_or(500).clamp(10, 100_000),
                null: "",
            }
            .with_null(app.get_fmt_null().as_str());
            loader::set_display_fmt(cfg);
            settings::save(&app, &st);
            st.col_widths.clear();
            refresh_view(&app, &mut st);
        });
    }

    {
        // 表格缩放重置（设置菜单 / Ctrl+0）；滚轮缩放在 files::drag_drop 的 winit 事件过滤器里
        let weak = app.as_weak();
        let state = state.clone();
        app.on_reset_zoom(move || {
            let mut st = state.lock().unwrap();
            if let Some(app) = weak.upgrade() {
                zoom_table(&app, &mut st, 13.0);
                settings::save(&app, &st);
            }
        });
    }
    {
        let weak = app.as_weak();
        app.on_table_zoom_pct(move || {
            weak.upgrade().map_or(100, |app| (app.get_table_font_size() / 13.0 * 100.0).round() as i32)
        });
    }

    {
        let weak = app.as_weak();
        let state = state.clone();
        app.on_autofit_columns(move || {
            let Some(app) = weak.upgrade() else { return };
            let mut st = state.lock().unwrap();
            // 丢掉记忆的列宽（含界面上拖出来的），按内容重新取样
            st.col_widths.clear();
            app.set_columns(ModelRc::new(VecModel::from(Vec::new())));
            refresh_view(&app, &mut st);
        });
    }

    {
        // 表格内容坐标 x 落在第几列（拖动选择时界面来问）
        let weak = app.as_weak();
        app.on_col_at(move |x| {
            let Some(app) = weak.upgrade() else { return 0 };
            let columns = app.get_columns();
            let mut right = 0.0;
            for i in 0..columns.row_count() {
                right += columns.row_data(i).map_or(0.0, |c| c.width);
                if x < right {
                    return i as i32;
                }
            }
            columns.row_count().saturating_sub(1) as i32
        });
    }

    {
        let weak = app.as_weak();
        let state = state.clone();
        app.on_copy_selection(move |with_header| copy_selection(&weak, &state, with_header));
    }
    {
        // Ctrl 加选不连续的行 / 列：块存 Rust，高亮和状态栏走纯回调（读 sel-epoch / sel-gen 建依赖）
        selection::install_sel_extra(&app, &state);
    }
    {
        // 表格右键菜单：在显示的节点后面建好排序 / 筛选 / 统计 / 选列的节点
        let weak = app.as_weak();
        let state = state.clone();
        app.on_quick_action(move |action, row, col| quick::quick_action(&weak, &state, &action, row, col));
    }

    // 用户自己的节点类型（程序旁边的 nodes 文件夹、用户目录）；定义有毛病的说一声，其余照常用
    let broken = nodes::load_user_types();
    if let Some(first) = broken.first() {
        app.set_status(SharedString::from(format!("有 {} 个节点定义读不了：{first}", broken.len())));
    }
    install_graph_callbacks(&app, &state);
    type_editor::install(&app, &state);
    help::install(&app, &state);
    pick::install(&app, &state);

    // 事件过滤器要在事件循环启动后注册（文件拖放走 winit 的 OLE 事件）
    {
        let weak = app.as_weak();
        let state = state.clone();
        slint::invoke_from_event_loop(move || {
            if let Some(app) = weak.upgrade() {
                title_bar_theme(&app, state.lock().unwrap().theme_mode);
                drag_drop::install(&app, state);
            }
        })
        .expect("事件循环未运行");
    }

    if let Some(path) = std::env::args().nth(1) {
        open_file(PathBuf::from(path), app.as_weak(), state);
    }

    app.run().unwrap();
    engine::cleanup();
}
