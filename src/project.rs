//! 工程文件（.rtv）：节点图 + 界面布局，像 Houdini 的 hip。

use crate::*;

/// 从工程文件读出来、等数据文件加载完再装上的内容。
pub(crate) struct Project {
    /// 工程文件；重新加载一个还没存成工程的图时为空
    pub(crate) path: Option<PathBuf>,
    pub(crate) graph: graph::Graph,
    pub(crate) ui: serde_json::Value,
    /// 重新加载（数据文件外面改过了）：图还是现在这张，撤销历史、进到的那一级网络都留着
    pub(crate) reload: bool,
}

pub(crate) fn is_project(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case(PROJECT_EXT))
}

pub(crate) fn read_project(path: &Path) -> Result<Project, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("无法读取工程文件: {e}"))?;
    let doc: serde_json::Value = serde_json::from_str(&text).map_err(|e| format!("工程文件格式不对: {e}"))?;
    let mut graph = graph::Graph::from_json(&doc["graph"]).ok_or("工程文件里没有节点图")?;
    if let Some(dir) = path.parent() {
        graph.relocate(dir);
    }
    Ok(Project { path: Some(path.to_path_buf()), graph, ui: doc["ui"].clone(), reload: false })
}

/// 存进工程文件的界面状态。
pub(crate) fn ui_state(app: &AppWindow, st: &App) -> serde_json::Value {
    serde_json::json!({
        "node_panel": app.get_node_panel_visible(),
        "node_panel_width": app.get_node_panel_width(),
        "params_height": app.get_params_height(),
        "swap_main": app.get_swap_main(),
        "swap_node": app.get_swap_node(),
        "zoom": app.get_zoom(),
        "pan_x": app.get_pan_x(),
        "pan_y": app.get_pan_y(),
        "font_size": app.get_table_font_size(),
        "theme": st.theme_mode,
        "ui_style": st.ui_style,
        "wrap": st.wrap_cells,
        "row_density": st.row_density,
    })
}

/// 恢复存下的界面状态；缺的项保持现状。personal：个人设置（settings.json）里的才套主题和字号——
/// 工程里存的不管（打开别人的工程不换掉自己的主题）。
pub(crate) fn apply_ui(app: &AppWindow, st: &mut App, ui: &serde_json::Value, personal: bool) {
    let num = |key: &str| ui[key].as_f64().map(|v| v as f32);
    if let Some(v) = ui["node_panel"].as_bool() {
        app.set_node_panel_visible(v);
    }
    if let Some(v) = num("node_panel_width") {
        app.set_node_panel_width(v);
    }
    if let Some(v) = num("params_height") {
        app.set_params_height(v);
    }
    if let Some(v) = ui["swap_main"].as_bool() {
        app.set_swap_main(v);
    }
    if let Some(v) = ui["swap_node"].as_bool() {
        app.set_swap_node(v);
    }
    if let Some(v) = num("zoom") {
        app.set_zoom(v.clamp(0.3, 2.5));
    }
    if let (Some(x), Some(y)) = (num("pan_x"), num("pan_y")) {
        app.set_pan_x(x);
        app.set_pan_y(y);
    }
    if let Some(v) = num("font_size").filter(|_| personal) {
        app.set_table_font_size(v.clamp(9.0, 28.0));
    }
    if let Some(v) = ui["theme"].as_i64().filter(|_| personal) {
        st.theme_mode = (v as i32).clamp(0, 2);
        app.set_theme_mode(st.theme_mode);
        crate::title_bar_theme(app, st.theme_mode);
    }
    if let Some(v) = ui["ui_style"].as_i64().filter(|_| personal) {
        st.ui_style = (v as i32).clamp(0, 3);
        app.set_ui_style(st.ui_style);
    }
    if let Some(v) = ui["wrap"].as_bool() {
        st.wrap_cells = v;
        app.set_settings_wrap(v);
    }
    if let Some(v) = ui["row_density"].as_i64() {
        st.row_density = (v as i32).clamp(0, 2);
        app.set_row_density(st.row_density);
        app.set_row_h_compact(st.row_density == 0);
        app.set_row_h_loose(st.row_density == 2);
    }
}

/// 节点图改过了还没存时问一声（关窗口、退出、打开别的文件之前）。返回 true = 可以接着做（存好了 / 不存了），
/// false = 取消（或者要存、却没存成）。
pub(crate) fn confirm_discard(weak: &slint::Weak<AppWindow>, state: &Arc<Mutex<App>>) -> bool {
    let name = {
        let mut st = state.lock().unwrap();
        undo::checkpoint(&mut st);
        if !undo::dirty(&st) {
            return true;
        }
        st.project.as_ref().and_then(|p| p.file_name()).map(|n| format!("工程“{}”的", n.to_string_lossy()))
    };
    // 对话框挂在主窗口上（模态、居中）
    let mut dialog = rfd::MessageDialog::new();
    if let Some(app) = weak.upgrade() {
        use slint::winit_030::WinitWindowAccessor;
        dialog = app.window().with_winit_window(|w| dialog.clone().set_parent(w)).unwrap_or(dialog);
    }
    // 系统的对话框按钮是“是 / 否 / 取消”：说明里写清楚各是什么
    let answer = dialog
        .set_level(rfd::MessageLevel::Warning)
        .set_title("NodeSheet")
        .set_description(format!(
            "{}节点图改过了，还没有保存。要先保存吗？\n\n是：保存　　否：不保存　　取消：回去接着改",
            name.unwrap_or_default()
        ))
        .set_buttons(rfd::MessageButtons::YesNoCancel)
        .show();
    match answer {
        rfd::MessageDialogResult::Yes => save_project(weak, state, false),
        rfd::MessageDialogResult::No => true,
        _ => false,
    }
}

/// 保存工程（节点图 + 界面布局）。还没存过、或者“另存为”时先问存到哪。返回存成了没有。
pub(crate) fn save_project(weak: &slint::Weak<AppWindow>, state: &Arc<Mutex<App>>, save_as: bool) -> bool {
    let Some(app) = weak.upgrade() else { return false };
    let (known, data) = {
        let st = state.lock().unwrap();
        (st.project.clone(), PathBuf::from(&st.graph.file))
    };
    let path = match known.clone().filter(|_| !save_as) {
        Some(path) => path,
        None => {
            // 默认放在数据文件旁边、和它同名
            let unnamed = || PathBuf::from(format!("未命名.{PROJECT_EXT}"));
            let suggested = known.unwrap_or_else(|| {
                if data.as_os_str().is_empty() { unnamed() } else { data.with_extension(PROJECT_EXT) }
            });
            let name = suggested.file_name().map(PathBuf::from).unwrap_or_else(unnamed);
            let mut dialog = rfd::FileDialog::new()
                .set_title("保存工程")
                .add_filter("NodeSheet 工程", &[PROJECT_EXT])
                .set_file_name(name.to_string_lossy());
            if let Some(dir) = suggested.parent().filter(|d| d.is_dir()) {
                dialog = dialog.set_directory(dir);
            }
            let Some(mut path) = dialog.save_file() else { return false };
            if !is_project(&path) {
                path.as_mut_os_string().push(format!(".{PROJECT_EXT}"));
            }
            path
        }
    };
    let mut st = state.lock().unwrap();
    let doc = serde_json::json!({
        "app": "nodesheet",
        "version": 1,
        "graph": st.graph.to_json(),
        "ui": ui_state(&app, &st),
    });
    // 先写到旁边的临时文件再改名换掉原来的：写到一半出错（磁盘满…）原来的工程还是好的
    let tmp = path.with_extension(format!("{PROJECT_EXT}.tmp"));
    let written = serde_json::to_string_pretty(&doc).map_err(|e| e.to_string());
    let written = written
        .and_then(|text| std::fs::write(&tmp, text).map_err(|e| e.to_string()))
        .and_then(|()| std::fs::rename(&tmp, &path).map_err(|e| e.to_string()));
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    match written {
        Ok(()) => {
            app.set_status(SharedString::from(format!("已保存工程 {}", path.display())));
            st.project = Some(path);
            app.set_project_name(project_name(&st));
            undo::mark_saved(&mut st);
            app.set_modified(false);
            true
        }
        Err(e) => {
            app.set_status(SharedString::from(format!("保存失败: {e}")));
            false
        }
    }
}

/// 窗口标题里的工程名。
pub(crate) fn project_name(st: &App) -> SharedString {
    let name = st.project.as_ref().and_then(|p| p.file_name()).and_then(|n| n.to_str());
    SharedString::from(name.unwrap_or(""))
}
