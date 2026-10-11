//! 文字 ↔ 网络：表达式网络里的节点写成文字（引擎用替身把节点代码再跑一遍，写进 `Node::text`），
//! 一段 polars 表达式拆成节点（引擎用 Python 的 ast 拆，`engine::parse`；建进图里是 `Graph::insert_parsed`）。
//! 入口：参数面板标题栏的“转成文字”（节点连同只给它用的上游换成一个 snippet）/“展开成节点”（snippet 拆开），
//! 和表达式网络里 Tab 菜单写的表达式（“按文字建节点”）。

use crate::*;

/// 表达式级的节点类型和参数（引擎拆文字时按它对上生成节点的参数）：{类型名: [[参数, 类型, 是不是端口], …]}。
fn expr_types() -> serde_json::Value {
    let types: serde_json::Map<String, serde_json::Value> = nodes::all_expr()
        .iter()
        .map(|t| {
            let params: Vec<serde_json::Value> =
                t.params.iter().map(|p| serde_json::json!([p.name, p.engine_kind(), p.port])).collect();
            (t.name.clone(), serde_json::Value::from(params))
        })
        .collect();
    serde_json::Value::Object(types)
}

/// 参数面板标题栏上的文字按钮：0 没有；1 转成文字（有引擎写的文字、不是输出节点、不是 snippet）；2 展开成节点（snippet）。
pub(crate) fn text_action_of(node: &graph::Node) -> i32 {
    match node.kind.name.as_str() {
        "snippet" if !node.value("expression").trim().is_empty() => 2,
        "snippet" => 0,
        _ if node.kind.role.is_empty() && !node.text.is_empty() => 1,
        _ => 0,
    }
}

/// 点了标题栏的文字按钮：选中的节点转成文字，或者 snippet 展开成节点。
pub(crate) fn text_action(weak: &slint::Weak<AppWindow>, state: &Arc<Mutex<App>>) {
    let (path, id, text) = {
        let mut st = state.lock().unwrap();
        let path = st.inside.clone();
        let Some(node) = st.net().selected.and_then(|id| st.net().node(id)).filter(|_| !path.is_empty()) else { return };
        let (id, action, text) = (node.id, text_action_of(node), node.value("expression").to_string());
        match action {
            1 => {
                st.net_mut().collapse_to_text(id);
                drop(st);
                graph_changed(weak, state, true);
                return;
            }
            2 => (path, id, text),
            _ => return,
        }
    };
    parse_then(weak, state, text, move |st, parsed| {
        let g = st.graph_at_mut(&path).ok_or("那张网络不在了")?;
        g.expand_snippet(id, parsed).map(|_| ())
    });
}

/// 表达式网络的 Tab 菜单里写的是一段表达式：拆成节点建在 (x, y)。
pub(crate) fn build_from_text(weak: &slint::Weak<AppWindow>, state: &Arc<Mutex<App>>, text: String, at: (f32, f32)) {
    let path = state.lock().unwrap().inside.clone();
    if path.is_empty() {
        return;
    }
    parse_then(weak, state, text, move |st, parsed| {
        // 网络要“列”输出（expr 节点里）时，最外面的 .alias("名字") 直接建成输出列
        let (owner, parent) = path.split_last().ok_or("不在网络里")?;
        let env = if parent.is_empty() { st.env() } else { graph::Env::Main };
        let columns = (st.graph_at(parent).and_then(|g| g.node(*owner)))
            .is_some_and(|n| n.env_roles(env).iter().any(|(role, ..)| *role == "列"));
        let g = st.graph_at_mut(&path).ok_or("那张网络不在了")?;
        let id = g.insert_text(parsed, at, columns)?;
        g.selected = Some(id);
        Ok(())
    });
}

/// 后台让引擎拆文字，拆好了在界面线程里用 apply 建进图里；出错写在参数面板上。
fn parse_then(
    weak: &slint::Weak<AppWindow>,
    state: &Arc<Mutex<App>>,
    text: String,
    apply: impl FnOnce(&mut App, &serde_json::Value) -> Result<(), String> + Send + 'static,
) {
    if let Some(app) = weak.upgrade() {
        app.set_status(SharedString::from("正在把文字拆成节点…"));
    }
    let weak = weak.clone();
    let state = state.clone();
    std::thread::spawn(move || {
        let parsed = engine::parse(&text, expr_types());
        let _ = slint::invoke_from_event_loop(move || {
            let done = {
                let mut st = state.lock().unwrap();
                let done = parsed.and_then(|parsed| apply(&mut st, &parsed));
                if let Err(e) = &done {
                    st.node_error = format!("拆不成节点：{e}");
                }
                done
            };
            match done {
                Ok(()) => {
                    if let Some(app) = weak.upgrade() {
                        app.invoke_request_fit();
                    }
                    graph_changed(&weak, &state, true);
                }
                Err(_) => {
                    if let Some(app) = weak.upgrade() {
                        let st = state.lock().unwrap();
                        push_graph(&app, &st, false);
                        update_busy(&app, &st);
                    }
                }
            }
        });
    });
}

/// Tab 菜单里写的像不像一段表达式（像的话第一项是“按文字建节点”）。
pub(crate) fn looks_like_expr(query: &str) -> bool {
    let q = query.trim();
    q.contains('(') || q.starts_with("pl.") || q.starts_with("cs.")
}

#[cfg(test)]
pub mod tests {
    /// 给引擎拆文字用的节点类型表（测试里直接用）。
    pub fn types() -> serde_json::Value {
        super::expr_types()
    }

    #[test]
    fn expression_like_queries() {
        assert!(super::looks_like_expr("pl.col(\"a\").abs()"));
        assert!(super::looks_like_expr("cs.numeric()"));
        assert!(!super::looks_like_expr("pad"));
        assert!(!super::looks_like_expr("str.slice"));
    }
}
