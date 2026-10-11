//! 个人设置：主题、字号、换行、面板布局、窗口大小、最近打开的文件。
//! 存在 `%APPDATA%\nodesheet\settings.json`（`RTV_SETTINGS` 可以指到别处：UI 测试时和 `RTV_NODES`
//! 一起指到临时目录，别写进真的用户目录）。工程文件里也存一份布局，打开工程时盖住这里的；
//! 主题和字号只看这里（打开别人的工程不换掉自己的主题）。

use crate::*;
use serde_json::{Value, json};

/// 最近打开的文件最多记几个。
const RECENT_MAX: usize = 10;

fn path() -> PathBuf {
    if let Some(p) = std::env::var_os("RTV_SETTINGS") {
        return PathBuf::from(p);
    }
    let base = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    base.join("nodesheet").join("settings.json")
}

/// 存着的设置；没有、读不了都当空的。
fn load() -> Value {
    let text = std::fs::read_to_string(path()).unwrap_or_default();
    serde_json::from_str::<Value>(&text).ok().filter(Value::is_object).unwrap_or_else(|| json!({}))
}

/// 写回去（先写临时文件再改名，写到一半不会弄坏原来的）。写不了就算了：只是下次不记得。
fn store(v: &Value) {
    let path = path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let tmp = path.with_extension("json.tmp");
    let text = serde_json::to_string_pretty(v).unwrap_or_default();
    if std::fs::write(&tmp, text).is_ok() && std::fs::rename(&tmp, &path).is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
}

/// 启动时（窗口显示之前）：套上存着的设置，“文件”菜单列出最近打开的。
pub(crate) fn apply(app: &AppWindow, st: &mut App) {
    let v = load();
    apply_ui(app, st, &v, true);
    crate::loader::set_display_fmt(crate::loader::DisplayFmt::from_json(&v["display"]));
    let window = &v["window"];
    if let (Some(width), Some(height)) = (window["width"].as_f64(), window["height"].as_f64()) {
        app.window().set_size(slint::LogicalSize::new(width.max(560.0) as f32, height.max(360.0) as f32));
    }
    if window["maximized"].as_bool() == Some(true) {
        app.window().set_maximized(true);
    }
    push_recent(app, st, &v);
}

/// 存下现在的设置（退出时、改了设置菜单时）。画布的缩放、平移是跟着图走的，不存。
pub(crate) fn save(app: &AppWindow, st: &App) {
    let mut v = load();
    let mut ui = ui_state(app, st);
    if let (Some(all), Some(ui)) = (v.as_object_mut(), ui.as_object_mut()) {
        for key in ["zoom", "pan_x", "pan_y"] {
            ui.remove(key);
        }
        all.extend(std::mem::take(ui));
    }
    let window = app.window();
    let mut size = v["window"].clone();
    if !size.is_object() {
        size = json!({});
    }
    // 最大化时记着最大化之前的大小（就是上次存的），不用最大化的大小盖掉它
    if !window.is_maximized() && !window.is_minimized() {
        let logical = window.size().to_logical(window.scale_factor());
        size["width"] = json!(logical.width.round());
        size["height"] = json!(logical.height.round());
    }
    size["maximized"] = json!(window.is_maximized());
    v["window"] = size;
    v["display"] = crate::loader::display_fmt().to_json();
    store(&v);
}

/// 打开了一个文件 / 工程：记进最近打开（最新的在最前，同一个只留一份）。
pub(crate) fn add_recent(app: &AppWindow, st: &mut App, file: &Path) {
    let mut v = load();
    let file = file.to_string_lossy().into_owned();
    let mut list: Vec<String> = recent_list(&v).into_iter().filter(|p| !same_path(p, &file)).collect();
    list.insert(0, file);
    list.truncate(RECENT_MAX);
    v["recent"] = json!(list);
    store(&v);
    push_recent(app, st, &v);
}

fn recent_list(v: &Value) -> Vec<String> {
    let items = v["recent"].as_array().into_iter().flatten();
    items.filter_map(|p| p.as_str().map(String::from)).collect()
}

/// Windows 的路径不分大小写。
fn same_path(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

/// “文件”菜单里的最近打开（文件已经不在了的不列）；App.recent 记着列出来的是哪些，点了按下标找。
fn push_recent(app: &AppWindow, st: &mut App, v: &Value) {
    st.recent = recent_list(v).into_iter().map(PathBuf::from).filter(|p| p.exists()).collect();
    let items: Vec<RecentItem> = st.recent.iter().map(|p| recent_item(p)).collect();
    app.set_recent_files(ModelRc::new(VecModel::from(items)));
}

/// 菜单里的一项：文件名，右边写在哪个文件夹（太长只留后面）。
fn recent_item(path: &Path) -> RecentItem {
    let name = path.file_name().map_or_else(|| path.to_string_lossy(), |n| n.to_string_lossy());
    let folder = path.parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
    let chars: Vec<char> = folder.chars().collect();
    let folder = if chars.len() > 26 { format!("…{}", chars[chars.len() - 25..].iter().collect::<String>()) } else { folder };
    RecentItem { name: SharedString::from(name.as_ref()), folder: SharedString::from(folder) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recent_items_are_short() {
        let item = recent_item(Path::new("D:/很长很长的文件夹名字/第二层也很长很长很长/第三层/销售.xlsx"));
        assert_eq!(item.name.as_str(), "销售.xlsx");
        assert!(item.folder.starts_with('…') && item.folder.chars().count() == 26, "{}", item.folder);
        assert!(same_path("D:\\A\\b.csv", "d:\\a\\B.CSV"));
    }
}
