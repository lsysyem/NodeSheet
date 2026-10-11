//! 节点图的撤销 / 重做，和“改过了没存”的标记。
//!
//! 每次改完（运行之前、拖完节点、排列、改名）记一份图的 JSON（`Graph::to_json`，工程文件里存的就是它）；
//! 撤销就是换回上一份。比较用“指纹”：JSON 去掉每一级的选中，只换了选中的节点不算一步。
//! 列名、类型、报错这些运行出来的东西不在 JSON 里，换回去时从当前的图搬过去，再重新运行。

use crate::*;
use serde_json::Value;

/// 最多记几步。
const MAX_STEPS: usize = 100;

/// 某一时刻的节点图，和当时节点面板在哪一级。
#[derive(Clone)]
pub(crate) struct Snap {
    json: Value,
    key: String,
    inside: Vec<u32>,
    env: graph::Env,
}

#[derive(Default)]
pub(crate) struct History {
    undo: Vec<Snap>,
    redo: Vec<Snap>,
    /// 现在的样子（最后一次记下的）
    last: Option<Snap>,
    /// 打开 / 保存时的指纹：和它不一样就是改过了还没存
    saved: String,
}

/// 图的指纹：JSON 去掉每一级图的 selected（只选了别的节点不算改过）。
fn fingerprint(v: &Value) -> String {
    fn strip(v: &mut Value) {
        match v {
            Value::Object(map) => {
                if map.contains_key("nodes") {
                    map.remove("selected");
                }
                map.values_mut().for_each(strip);
            }
            Value::Array(items) => items.iter_mut().for_each(strip),
            _ => {}
        }
    }
    let mut v = v.clone();
    strip(&mut v);
    v.to_string()
}

fn snap(st: &App) -> Snap {
    let json = st.graph.to_json();
    Snap { key: fingerprint(&json), json, inside: st.inside.clone(), env: st.env }
}

/// 打开了文件 / 工程：历史清空，现在的样子就是“存过的”。
pub(crate) fn reset(st: &mut App) {
    let now = snap(st);
    st.history = History { saved: now.key.clone(), last: Some(now), ..History::default() };
}

/// 保存成功了：现在的样子是存过的。
pub(crate) fn mark_saved(st: &mut App) {
    checkpoint(st);
    st.history.saved = st.history.last.as_ref().map_or_else(String::new, |s| s.key.clone());
}

/// 图和上次记下的不一样了就记一步（之前那份进撤销栈，重做栈清空）。
pub(crate) fn checkpoint(st: &mut App) {
    let now = snap(st);
    let history = &mut st.history;
    if history.last.as_ref().is_some_and(|last| last.key == now.key) {
        return;
    }
    if let Some(prev) = history.last.replace(now) {
        history.undo.push(prev);
        if history.undo.len() > MAX_STEPS {
            history.undo.remove(0);
        }
        history.redo.clear();
    }
}

/// 改过了还没存。只有打开文件时自动建的那个源节点、又没有工程文件时不算（只看不改的人不会被问）。
pub(crate) fn dirty(st: &App) -> bool {
    let changed = st.history.last.as_ref().is_some_and(|last| last.key != st.history.saved);
    changed && (st.project.is_some() || st.graph.nodes.len() > 1)
}

/// 撤销（redo 为真时重做）一步。返回做了没有（没有可撤销的就是 false）。
pub(crate) fn step(st: &mut App, redo: bool) -> bool {
    // 还没记下的改动（参数打了字没回车就点了别处）先记成一步：撤销先撤它
    checkpoint(st);
    let target = if redo { st.history.redo.pop() } else { st.history.undo.pop() };
    let Some(target) = target else { return false };
    let Some(current) = st.history.last.take() else { return false };
    if redo {
        st.history.undo.push(current);
    } else {
        st.history.redo.push(current);
    }
    restore(st, &target);
    st.history.last = Some(target);
    true
}

/// 换回某一份图。运行出来的东西（列名、类型、sheet 列表）按 id 从当前的图搬过去；
/// 那时在的那一级网络还在就留在那里，不在了回到表格级。
fn restore(st: &mut App, snap: &Snap) {
    let Some(mut g) = graph::Graph::from_json(&snap.json) else { return };
    carry_runtime(&st.graph, &mut g);
    g.missing.clear();
    let viewer: Vec<String> =
        st.file_sheets.as_ref().map_or(&st.sheets, |(sheets, _)| sheets).iter().map(|s| s.name.clone()).collect();
    for node in &mut g.nodes {
        // 文件换回来了：sheet 下拉的选项也换回来
        if st.graph.node(node.id).is_none_or(|old| old.file_path() != node.file_path()) {
            panel::reload_sheets(node, &snap_file(&snap.json), viewer.clone());
        }
    }
    st.graph = g;
    st.env = snap.env;
    st.inside = snap.inside.clone();
    if st.inside_node().is_none() {
        st.inside.clear();
    }
}

fn snap_file(json: &Value) -> String {
    json["file"].as_str().unwrap_or("").to_string()
}

fn carry_runtime(old: &graph::Graph, new: &mut graph::Graph) {
    for node in &mut new.nodes {
        let Some(prev) = old.node(node.id) else { continue };
        node.columns = prev.columns.clone();
        node.dtypes = prev.dtypes.clone();
        node.sheets = prev.sheets.clone();
        node.text = prev.text.clone();
        for (env, net) in node.nets_mut() {
            if let Some(prev_net) = prev.net_in(env) {
                carry_runtime(prev_net, net);
            }
        }
    }
}

/// Ctrl+Z / Ctrl+Y（菜单的撤销 / 重做）：换回去，同步界面，重新运行。
pub(crate) fn undo_redo(weak: &slint::Weak<AppWindow>, state: &Arc<Mutex<App>>, redo: bool) {
    let Some(app) = weak.upgrade() else { return };
    {
        let mut st = state.lock().unwrap();
        if !step(&mut st, redo) {
            app.set_status(SharedString::from(if redo { "没有可以重做的" } else { "没有可以撤销的" }));
            return;
        }
        // 可能换了一级（添加菜单换成表格级 / 表达式级的节点）
        push_kinds(&app, &mut st);
        push_graph(&app, &st, true);
    }
    run_graph(weak, state);
    // 重新运行会改状态栏（显示文件本身时是“就绪”）：说一声放在后面
    app.set_status(SharedString::from(if redo { "已重做" } else { "已撤销" }));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::tests::kind;

    fn app() -> App {
        let mut st = App::default();
        st.graph = graph::Graph::for_file(Path::new("testdata/people.csv"), "");
        reset(&mut st);
        st
    }

    fn json(st: &App) -> String {
        fingerprint(&st.graph.to_json())
    }

    #[test]
    fn undo_and_redo_steps() {
        let mut st = app();
        let start = json(&st);
        assert!(!dirty(&st), "只有源节点：不算改过");

        let id = st.graph.add(&kind("slice"));
        checkpoint(&mut st);
        let added = json(&st);
        assert!(dirty(&st));

        st.graph.node_mut(id).unwrap().set("n", "3");
        checkpoint(&mut st);
        let set = json(&st);

        // 只换了选中的：不算一步
        st.graph.select_only(Some(1));
        checkpoint(&mut st);
        assert_eq!(st.history.undo.len(), 2);

        // 挪了位置（还没记下）：撤销先撤它
        st.graph.node_mut(id).unwrap().x += 50.0;
        assert!(step(&mut st, false));
        assert_eq!(json(&st), set);
        assert!(step(&mut st, false));
        assert_eq!(json(&st), added);
        assert!(step(&mut st, false));
        assert_eq!(json(&st), start);
        assert!(!step(&mut st, false), "到头了");
        assert!(!dirty(&st));

        assert!(step(&mut st, true));
        assert_eq!(json(&st), added);
        // 撤销以后改了别的：重做的那些作废
        st.graph.node_mut(id).unwrap().set("n", "9");
        checkpoint(&mut st);
        assert!(!step(&mut st, true));
    }

    #[test]
    fn saved_state_is_not_dirty() {
        let mut st = app();
        st.graph.add(&kind("slice"));
        checkpoint(&mut st);
        assert!(dirty(&st));
        mark_saved(&mut st);
        assert!(!dirty(&st));
        // 撤销回到没存时的样子：又算改过了
        assert!(step(&mut st, false));
        st.project = Some(PathBuf::from("a.rtv"));
        assert!(dirty(&st), "有工程文件时只剩源节点也算改过");
    }

    /// 换回去时运行出来的列名留着（选列框不会一下子空了）。
    #[test]
    fn restore_keeps_runtime_info() {
        let mut st = app();
        st.graph.nodes[0].columns = vec![String::from("姓名")];
        st.graph.add(&kind("slice"));
        checkpoint(&mut st);
        assert!(step(&mut st, false));
        assert_eq!(st.graph.nodes[0].columns, ["姓名"]);
    }
}
