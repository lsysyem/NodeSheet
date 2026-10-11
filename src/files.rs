//! 打开文件：后台解码、切 sheet、拖放。

use crate::*;

/// 打开表格文件（新建一张只有源节点的图）或工程文件（恢复存下来的图和界面）。
pub(crate) fn open_file(path: PathBuf, weak: slint::Weak<AppWindow>, state: Arc<Mutex<App>>) {
    // 命令行给的可能是相对路径：节点参数和工程文件里都存绝对路径
    let path = std::path::absolute(&path).unwrap_or(path);
    if !is_project(&path) {
        return load_file(path, None, weak, state);
    }
    match read_project(&path) {
        Ok(project) => load_file(PathBuf::from(&project.graph.file), Some(project), weak, state),
        Err(msg) => {
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(app) = weak.upgrade() {
                    app.set_status(SharedString::from(msg));
                }
            });
        }
    }
}

/// 在后台线程建索引/读元数据，完成后回 UI 线程刷新视图。project：打开的是工程，装上它存的图而不是新建。
pub(crate) fn load_file(path: PathBuf, project: Option<Project>, weak: slint::Weak<AppWindow>, state: Arc<Mutex<App>>) {
    // 工程里可以没有数据文件（只有手动加的读取节点）
    let has_file = !path.as_os_str().is_empty();
    let display_name = path.file_name().and_then(|s| s.to_str()).unwrap_or("").to_string();
    let meta = if has_file { file_meta(&path) } else { String::new() };
    // 和查看器解码文件同时把引擎启动起来，第一次运行节点时不用再等 Python
    engine::warm_up();

    {
        let weak = weak.clone();
        slint::invoke_from_event_loop(move || {
            if let Some(app) = weak.upgrade() {
                app.set_file_name(SharedString::from(display_name));
                app.set_file_meta(SharedString::from(meta));
                app.set_status(SharedString::from("正在加载…"));
                app.set_busy(true);
            }
        })
        .expect("事件循环未运行");
    }

    let progress: loader::ProgressFn = {
        let weak = weak.clone();
        let state = state.clone();
        Arc::new(move |src: &Arc<dyn DataSource>| {
            let src = src.clone();
            let weak = weak.clone();
            let state = state.clone();
            let _ = slint::invoke_from_event_loop(move || {
                let Some(app) = weak.upgrade() else { return };
                let mut st = state.lock().unwrap();
                // 只刷新当前正在看的 sheet；其他 sheet 静默解码
                let Some(cur) = st.sheets.get(st.current) else { return };
                if !Arc::ptr_eq(&cur.source, &src) {
                    return;
                }
                progress_update(&app, &mut st);
            });
        })
    };

    std::thread::spawn(move || {
        let mut project = project;
        let mut result = if has_file { loader::open_tables_with_progress(&path, progress) } else { Ok(Vec::new()) };
        // 查看器自己读不了、read 节点读得了的格式（JSON、Arrow、Avro）：建好源节点交给引擎读
        let engine_read = project.is_none() && matches!(result, Err(loader::LoadError::Unsupported(_))) && engine_reads(&path);
        if engine_read {
            result = Ok(Vec::new());
        }
        let stamp = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
        if let Some(project) = &mut project {
            // 读别的 Excel 文件的节点：sheet 下拉的选项（源节点的取自查看器打开的 sheets）
            let file = project.graph.file.clone();
            for node in &mut project.graph.nodes {
                let path = node.file_path().unwrap_or("").to_string();
                if node.reads_excel() && !(node.primary && path == file) {
                    node.sheets = loader::sheet_names(Path::new(&path));
                }
            }
        }
        slint::invoke_from_event_loop(move || {
            let Some(app) = weak.upgrade() else { return };
            let mut st = state.lock().unwrap();
            let (sheets, error) = match result {
                Ok(sheets) => (sheets, None),
                Err(e) => (Vec::new(), Some(e.to_string())),
            };
            if let (None, Some(e)) = (&project, &error) {
                st.sheets.clear();
                st.current = 0;
                clear_table(&app);
                app.set_status(SharedString::from(e.as_str()));
                update_busy(&app, &st);
                return;
            }
            let restored = project.is_some() || engine_read;
            let reload = project.as_ref().is_some_and(|p| p.reload);
            // 最近打开：工程记工程文件，否则记数据文件
            let recent = project.as_ref().map_or_else(|| has_file.then(|| path.clone()), |p| p.path.clone());
            match project {
                Some(project) => {
                    apply_ui(&app, &mut st, &project.ui, false);
                    st.graph = project.graph;
                    st.project = project.path;
                }
                None if engine_read => {
                    st.graph = graph::Graph::for_engine_file(&path);
                    st.project = None;
                }
                None => {
                    // 新文件 = 只有一个源节点的新图；查看器认出来的读法（CSV 的编码、分隔符）填进源节点
                    let first = sheets.first();
                    let hints = first.map(|s| s.source.read_hints()).unwrap_or_default();
                    st.graph = graph::Graph::for_file_with(&path, first.map_or("", |s| s.name.as_str()), &hints);
                    st.project = None;
                }
            }
            st.file_stamp = stamp;
            app.set_project_name(project_name(&st));
            // 源节点：sheet 下拉的选项取自查看器；表格停在它选的那个 sheet 上
            let names: Vec<String> = sheets.iter().map(|s| s.name.clone()).collect();
            let file = st.graph.file.clone();
            let mut current = 0;
            if let Some(node) = st.graph.nodes.iter_mut().find(|n| n.primary && n.file_path() == Some(&file)) {
                if let Some(si) = node.sheet_param().filter(|_| node.reads_excel()) {
                    current = names.iter().position(|name| *name == node.params[si]).unwrap_or(0);
                }
                node.columns = sheets.get(current).map(|s| s.source.headers().to_vec()).unwrap_or_default();
                node.sheets = names;
            }
            st.file_sheets = None;
            // 重新打开（文件可能改过）：源节点的类型要重新推
            st.native_types = None;
            st.native_types_asked.clear();
            // 重新加载时留在原来那一级网络里（还在的话）
            if !reload || st.inside_node().is_none() {
                st.inside.clear();
            }
            cancel_runs(&mut st);
            st.running = false;
            st.node_status.clear();
            st.node_error.clear();
            st.node_output.clear();
            st.sheets = sheets;
            st.current = current;
            st.reset_sheet_view();
            if !reload {
                // 新打开的图：撤销历史清空，现在的样子算“存过的”
                undo::reset(&mut st);
                if let Some(file) = &recent {
                    settings::add_recent(&app, &mut st, file);
                }
            }
            // 工程里可能带着自己的节点类型：“添加”菜单跟着更新
            push_kinds(&app, &mut st);
            push_graph(&app, &st, true);
            drop(st);
            show_current(&weak, &state);
            if restored {
                // 显示标记可能在别的节点上
                run_graph(&weak, &state);
            }
            if let Some(e) = error {
                app.set_status(SharedString::from(e));
            }
            // 工程里有已经没有了的节点类型（比如旧版的 filter / group_by，现在用 expr 节点）：说一声
            let mut st = state.lock().unwrap();
            let missing = std::mem::take(&mut st.graph.missing);
            if !missing.is_empty() {
                let names: Vec<String> = missing.iter().map(|(title, kind)| format!("{title}（{kind}）")).collect();
                let msg = format!(
                    "工程里有 {} 个节点的类型已经没有了，没能打开：{}。筛选、排序、分组、改值这些现在都在 expr 节点里做",
                    missing.len(),
                    names.join("、")
                );
                app.set_status(SharedString::from(msg.as_str()));
                // 状态栏会被运行结果盖掉；参数面板的提示留到下次运行
                st.node_error = msg;
                push_graph(&app, &st, false);
            }
        })
        .expect("事件循环未运行");
    });
}

/// 查看器自己读不了、read 节点读得了的格式：read 节点“格式”菜单的扩展名表里有，又不是按 CSV 读的
/// （.txt / .log 当 CSV 读多半不对，要在 read 节点里自己选“按行读”）。
pub(crate) fn engine_reads(path: &Path) -> bool {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    let Some(read) = nodes::find("read") else { return false };
    let formats = read.params.iter().flat_map(|p| &p.extensions);
    formats.into_iter().any(|(format, exts)| format != "CSV" && exts.iter().any(|e| *e == ext))
}

/// 文件 → 重新加载（Ctrl+R）：数据文件外面改过了，重新读一遍。节点图、撤销历史、界面都照旧，显示的节点重新运行
/// （文件的修改时间变了，引擎里的缓存自动失效）。
pub(crate) fn reload_file(weak: &slint::Weak<AppWindow>, state: &Arc<Mutex<App>>) {
    let Some(app) = weak.upgrade() else { return };
    let project = {
        let mut st = state.lock().unwrap();
        if st.graph.nodes.is_empty() {
            return;
        }
        undo::checkpoint(&mut st);
        // 图复制一份交给 load_file（读完文件之前界面上还是现在这张）
        let Some(graph) = graph::Graph::from_json(&st.graph.to_json()) else { return };
        Project { path: st.project.clone(), graph, ui: serde_json::Value::Null, reload: true }
    };
    app.set_status(SharedString::from("正在重新加载…"));
    load_file(PathBuf::from(&project.graph.file), Some(project), weak.clone(), state.clone());
}

/// 窗口重新得到焦点：数据文件在外面改过了就在状态栏提醒一声（不自动重新加载，免得打断手上的事）。
pub(crate) fn check_file_changed(weak: &slint::Weak<AppWindow>, state: &Arc<Mutex<App>>) {
    let Some(app) = weak.upgrade() else { return };
    let mut st = state.lock().unwrap();
    let Some(known) = st.file_stamp else { return };
    let now = std::fs::metadata(&st.graph.file).and_then(|m| m.modified()).ok();
    if now.is_some_and(|now| now != known) {
        // 只提醒一次：下次重新加载时记下新的修改时间
        st.file_stamp = now;
        app.set_status(SharedString::from("数据文件在外面改过了 · Ctrl+R 重新加载"));
    }
}

/// 表格切到文件的第 index 个 sheet（源节点的 sheet 参数换了）。
pub(crate) fn switch_sheet(weak: &slint::Weak<AppWindow>, state: &Arc<Mutex<App>>, index: usize) {
    {
        let mut st = state.lock().unwrap();
        if index >= st.sheets.len() {
            return;
        }
        st.current = index;
        st.reset_sheet_view();
        let headers = st.sheets[index].source.headers().to_vec();
        if let Some(node) = st.graph.nodes.iter_mut().find(|n| n.primary) {
            node.columns = headers;
        }
        if let Some(app) = weak.upgrade() {
            push_graph(&app, &st, false);
        }
    }
    show_current(weak, state);
}

/// 表格显示 App.current 这张表；还没解码的 sheet 先在后台解码，完成后再刷新，避免卡 UI。
pub(crate) fn show_current(weak: &slint::Weak<AppWindow>, state: &Arc<Mutex<App>>) {
    let Some(app) = weak.upgrade() else { return };
    let mut st = state.lock().unwrap();
    let Some(src) = st.sheets.get(st.current).map(|s| s.source.clone()) else {
        clear_table(&app);
        update_busy(&app, &st);
        return;
    };
    if src.is_ready() {
        refresh_view(&app, &mut st);
        drop(st);
        fetch_native_types(weak, state);
        return;
    }
    app.set_status(SharedString::from("正在解析 sheet…"));
    app.set_busy(true);
    drop(st);
    let weak = weak.clone();
    let state = state.clone();
    std::thread::spawn(move || {
        let result = src.prepare();
        slint::invoke_from_event_loop(move || {
            let Some(app) = weak.upgrade() else { return };
            let mut st = state.lock().unwrap();
            if !st.sheets.get(st.current).is_some_and(|s| Arc::ptr_eq(&s.source, &src)) {
                return; // 期间用户又切走了
            }
            if let Some(node) = st.graph.nodes.iter_mut().find(|n| n.primary) {
                node.columns = src.headers().to_vec();
            }
            push_graph(&app, &st, false);
            refresh_view(&app, &mut st);
            if let Err(e) = result {
                app.set_status(SharedString::from(format!("解析失败: {e}")));
                return;
            }
            drop(st);
            fetch_native_types(&weak, &state);
        })
        .expect("事件循环未运行");
    });
}

pub(crate) mod drag_drop {
    use super::{open_file, zoom_table, App, AppWindow};
    use crate::project::confirm_discard;
    use slint::ComponentHandle;
    use slint::winit_030::winit::event::{MouseScrollDelta, WindowEvent};
    use slint::winit_030::{EventResult, WinitWindowAccessor};
    use std::cell::RefCell;
    use std::path::PathBuf;
    use std::rc::Rc;
    use std::sync::{Arc, Mutex};

    /// winit 在创建窗口时已把自己注册为 OLE 拖放目标（OleInitialize + RegisterDragDrop），
    /// 从资源管理器拖文件会转成 winit 的 WindowEvent::DroppedFile 事件，在事件过滤器里截获。
    /// 一次拖进几个文件是连着几个事件：先攒起来，这一轮事件处理完再一起处理（要弹对话框问保存，不在事件回调里弹）。
    pub fn install(app: &AppWindow, state: Arc<Mutex<App>>) {
        let weak = app.as_weak();
        let pending: Rc<RefCell<Vec<PathBuf>>> = Rc::default();
        app.window().on_winit_window_event(move |_, event| match event {
            WindowEvent::DroppedFile(path) => {
                if let Some(app) = weak.upgrade() {
                    app.set_drag_hover(false);
                }
                let first = pending.borrow().is_empty();
                pending.borrow_mut().push(path.clone());
                if first {
                    let (weak, state, pending) = (weak.clone(), state.clone(), pending.clone());
                    slint::Timer::single_shot(std::time::Duration::ZERO, move || {
                        let paths = std::mem::take(&mut *pending.borrow_mut());
                        dropped(paths, &weak, &state);
                    });
                }
                EventResult::PreventDefault
            }
            WindowEvent::HoveredFile(_) | WindowEvent::HoveredFileCancelled => {
                if let Some(app) = weak.upgrade() {
                    app.set_drag_hover(matches!(event, WindowEvent::HoveredFile(_)));
                }
                EventResult::Propagate
            }
            // 切回来的时候看看数据文件是不是在外面改过了
            WindowEvent::Focused(true) => {
                super::check_file_changed(&weak, &state);
                EventResult::Propagate
            }
            // Ctrl + 滚轮落在表格上：缩放表格（winit 的事件不带位置，问系统要）
            WindowEvent::MouseWheel { delta, .. } => {
                if !ctrl_held() {
                    return EventResult::Propagate;
                }
                let Some(app) = weak.upgrade() else { return EventResult::Propagate };
                let step = match delta {
                    MouseScrollDelta::LineDelta(_, y) => *y,
                    // 触摸板的高精度滚动：约 40 逻辑像素算一行
                    MouseScrollDelta::PixelDelta(p) => {
                        (p.y / (40.0 * app.window().scale_factor() as f64)) as f32
                    }
                };
                if step == 0.0 || !over_table(&app) {
                    return EventResult::Propagate;
                }
                let mut st = state.lock().unwrap();
                zoom_table(&app, &mut st, app.get_table_font_size() * 1.1f32.powf(step.clamp(-3.0, 3.0)));
                EventResult::PreventDefault
            }
            _ => EventResult::Propagate,
        });
    }

    #[cfg(windows)]
    fn ctrl_held() -> bool {
        #[link(name = "user32")]
        unsafe extern "system" {
            fn GetKeyState(n_virt_key: i32) -> i16;
        }
        // SAFETY: 查当前消息时刻的按键状态
        unsafe { GetKeyState(0x11 /* VK_CONTROL */) as u16 & 0x8000 != 0 }
    }

    /// 鼠标（物理坐标换算过的）落在不在表格区里。
    fn over_table(app: &AppWindow) -> bool {
        let Some((cx, cy)) = cursor_in_window(app) else { return false };
        let (x, y, w, h) = (app.get_table_x(), app.get_table_y(), app.get_table_w(), app.get_table_h());
        w > 0.0 && (x..x + w).contains(&cx) && (y..y + h).contains(&cy)
    }

    /// 拖进来的文件。落在节点画布上（已经打开了文件、在表格级的图里）：每个文件在落点建一个 read 节点
    /// （做 join / concat 用）；否则当主文件打开（节点图改过了先问要不要存，一次只能打开一个）。
    fn dropped(paths: Vec<PathBuf>, weak: &slint::Weak<AppWindow>, state: &Arc<Mutex<App>>) {
        let Some(app) = weak.upgrade() else { return };
        if let Some(at) = canvas_point(&app) {
            if add_read_nodes(&app, &paths, at, state) {
                super::graph_changed(weak, state, true);
                return;
            }
        }
        let Some(path) = paths.first().cloned() else { return };
        if !confirm_discard(weak, state) {
            return;
        }
        open_file(path, weak.clone(), state.clone());
    }

    /// 鼠标现在在节点画布上的哪里（画布坐标，除去了平移、缩放）；不在画布上（或节点面板关着）是 None。
    fn canvas_point(app: &AppWindow) -> Option<(f32, f32)> {
        let (cx, cy) = cursor_in_window(app)?;
        let (x, y, w, h) = (app.get_canvas_x(), app.get_canvas_y(), app.get_canvas_w(), app.get_canvas_h());
        if w <= 0.0 || !(x..x + w).contains(&cx) || !(y..y + h).contains(&cy) {
            return None;
        }
        let zoom = app.get_zoom().max(0.01);
        Some(((cx - x - app.get_pan_x()) / zoom, (cy - y - app.get_pan_y()) / zoom))
    }

    /// 鼠标在窗口里的位置（逻辑像素，和界面的坐标一样）。拖放时界面收不到鼠标事件，问系统要。
    #[cfg(windows)]
    fn cursor_in_window(app: &AppWindow) -> Option<(f32, f32)> {
        #[repr(C)]
        struct Point {
            x: i32,
            y: i32,
        }
        #[link(name = "user32")]
        unsafe extern "system" {
            fn GetCursorPos(point: *mut Point) -> i32;
        }
        let mut p = Point { x: 0, y: 0 };
        // SAFETY: 传进去的是一个 POINT
        if unsafe { GetCursorPos(&mut p) } == 0 {
            return None;
        }
        let (origin, scale) = app.window().with_winit_window(|w| (w.inner_position().ok(), w.scale_factor()))?;
        let origin = origin?;
        Some(((p.x - origin.x) as f32 / scale as f32, (p.y - origin.y) as f32 / scale as f32))
    }

    #[cfg(not(windows))]
    fn cursor_in_window(_app: &AppWindow) -> Option<(f32, f32)> {
        None
    }

    /// 在画布的 at 处给每个 read 节点读得了的文件建一个 read 节点（往下错开），最后一个是选中 + 显示的。
    /// 还没打开文件、正在表达式网络里、一个都读不了时不建，返回 false（照旧当主文件打开）。
    fn add_read_nodes(app: &AppWindow, paths: &[PathBuf], at: (f32, f32), state: &Arc<Mutex<App>>) -> bool {
        let mut st = state.lock().unwrap();
        let Some(read) = crate::nodes::find("read") else { return false };
        let readable: Vec<PathBuf> = paths.iter().filter(|p| super::read_node_reads(p)).cloned().collect();
        if st.graph.nodes.is_empty() || !st.inside.is_empty() || readable.is_empty() {
            return false;
        }
        for (i, path) in readable.iter().enumerate() {
            let path = std::path::absolute(path).unwrap_or_else(|_| path.clone());
            let id = st.graph.add_at(&read, Some((at.0, at.1 + i as f32 * crate::graph::NODE_DY)));
            if let Some(node) = st.graph.node_mut(id) {
                node.set("source", &path.to_string_lossy());
            }
            crate::file_changed(&mut st, id);
        }
        let skipped = paths.len() - readable.len();
        app.set_status(slint::SharedString::from(match skipped {
            0 => format!("加了 {} 个 read 节点", readable.len()),
            n => format!("加了 {} 个 read 节点（{n} 个文件 read 节点读不了，没加）", readable.len()),
        }));
        true
    }
}

/// read 节点读得了这个文件：扩展名在它“格式”菜单的扩展名表里。
pub(crate) fn read_node_reads(path: &Path) -> bool {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    let Some(read) = nodes::find("read") else { return false };
    read.params.iter().flat_map(|p| &p.extensions).any(|(_, exts)| exts.iter().any(|e| *e == ext))
}
