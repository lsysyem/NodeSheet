//! 执行节点图：显示节点的结果放进表格；节点面板里的按钮（写出）。

use crate::*;
use std::sync::atomic::{AtomicU64, Ordering};

/// 最新一次运行的编号（同 App.run_ticket）。排队等引擎的后台线程不拿 App 的锁就能知道自己过期了没有。
static LATEST_RUN: AtomicU64 = AtomicU64::new(0);

/// 在跑 / 排着队的运行全部作废（结果回来也不用）。
pub(crate) fn cancel_runs(st: &mut App) {
    st.run_ticket += 1;
    LATEST_RUN.store(st.run_ticket, Ordering::Relaxed);
}

/// 表格切回文件本身的数据（显示标记在源节点上）。返回是否真的换了表（换了要接着 show_current）。
pub(crate) fn restore_native(st: &mut App) -> bool {
    st.node_status.clear();
    st.last_ms = None;
    st.shown = None;
    st.shown_sub.clear();
    st.preview_note.clear();
    let Some((sheets, current)) = st.file_sheets.take() else { return false };
    st.sheets = sheets;
    st.current = current;
    st.reset_sheet_view();
    true
}

/// 记下节点出错：卡片标红，参数面板显示原因。id 为空 = 不是哪个节点的问题（引擎起不来之类）。
/// sub：出错的是 id 里面表达式网络的哪个节点（路径：网络里的节点、eval 里面的节点…；一路都标红，最里面的记下原因）。
pub(crate) fn node_failed(st: &mut App, id: Option<u32>, sub: &[u32], msg: String) {
    let Some(mut node) = id.and_then(|id| st.graph.node_mut(id)) else {
        st.node_error = msg;
        return;
    };
    node.error = true;
    let mut name = node.title.clone();
    for &sub in sub {
        let Some(inner) = node.sub_node_mut(sub) else { break };
        inner.error = true;
        inner.note = msg.clone();
        name = format!("{name} › {}", inner.title);
        node = inner;
    }
    st.node_error = format!("{name}：{msg}");
}

/// 引擎报回来的表达式节点的输出类型、问题和文字（存在 expr 节点里面的网络上，eval 里面的网络一样）。
pub(crate) fn store_subs(st: &mut App, subs: &[(u32, Vec<(u32, engine::SubInfo)>)]) {
    for (id, items) in subs {
        if let Some(expr) = st.graph.node_mut(*id) {
            store_net_infos(expr, items);
        }
    }
}

fn store_net_infos(owner: &mut graph::Node, items: &[(u32, engine::SubInfo)]) {
    // 当前上下文用不到的输出节点：提醒一声（换回去还要用，不删）
    let unused: Vec<(u32, String)> = (owner.nets().flat_map(|(env, net)| net.nodes.iter().map(move |n| (env, n))))
        .filter_map(|(env, n)| Some((n.id, owner.unused_role(env, &n.kind.role)?)))
        .collect();
    for node in owner.nets_mut().flat_map(|(_, net)| &mut net.nodes) {
        let info = items.iter().find(|(sub, _)| *sub == node.id).map(|(_, info)| info);
        match info {
            Some(info) => {
                // 多列（选择器）时重复 n 次：个数就是输出几列
                node.dtypes = vec![info.dtype.clone(); info.count.max(1)];
                node.note = info.error.clone();
                node.error = !info.error.is_empty();
                node.warn = info.warn.clone();
                node.text = info.text.clone();
            }
            None => {
                node.dtypes.clear();
                node.note.clear();
                node.error = false;
                node.warn.clear();
                node.text.clear();
            }
        }
        if let Some((_, msg)) = unused.iter().find(|(sub, _)| *sub == node.id) {
            node.warn = msg.clone();
        }
        if node.net.is_some() {
            store_net_infos(node, info.map_or(&[][..], |info| &info.net));
        }
    }
}

/// 引擎写出来的 expr 节点的文字（整个节点：df.with_columns(…)）。
pub(crate) fn store_texts(st: &mut App, texts: &[(u32, String)]) {
    for (id, text) in texts {
        if let Some(node) = st.graph.node_mut(*id) {
            node.text = text.clone();
        }
    }
}

/// 引擎报回来的各节点输出列名（选列框先列着它们）和类型（中键信息窗）。
pub(crate) fn store_columns(st: &mut App, columns: &[(u32, Vec<String>)], dtypes: &[(u32, Vec<String>)]) {
    for (id, names) in columns {
        if let Some(node) = st.graph.node_mut(*id) {
            node.columns = names.clone();
            node.dtypes = dtypes.iter().find(|(d, _)| d == id).map(|(_, t)| t.clone()).unwrap_or_default();
        }
    }
}

/// 源节点参数的指纹：文件 + 各参数（换 sheet 也算）。返回 (源节点, 指纹)。
pub(crate) fn native_key(st: &App) -> Option<(u32, String)> {
    let node = st.graph.nodes.iter().find(|n| n.primary)?;
    Some((node.id, format!("{}\u{1}{}", st.graph.file, node.params.join("\u{1}"))))
}

/// 表格显示的是查看器自己读的源文件、而它不知道类型时（CSV / Excel 按文本读）：在后台让引擎推出源节点的类型，
/// 表头第二行照它写（就是往下接节点时 polars 拿到的类型）。引擎把读到的表留在缓存里，接着跑下游时不用再读。
/// 没有 Python / 读不出来就算了：不显示类型而已。
pub(crate) fn fetch_native_types(weak: &slint::Weak<AppWindow>, state: &Arc<Mutex<App>>) {
    let mut st = state.lock().unwrap();
    let Some(src) = st.sheets.get(st.current).map(|s| s.source.clone()) else { return };
    let Some((id, key)) = native_key(&st) else { return };
    if st.file_sheets.is_some() || !st.graph.is_native(id) || !src.dtypes().is_empty() || st.native_types_asked == key {
        return;
    }
    let Ok(steps) = st.graph.compile(id) else { return };
    st.native_types_asked = key.clone();
    drop(st);
    let weak = weak.clone();
    let state = state.clone();
    std::thread::spawn(move || {
        // 查看器自己还在渐进解码时先等它解完，别和它抢 CPU / 内存
        while src.is_loading() {
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
        let Ok(ok) = engine::schema(&steps, id) else { return };
        let _ = slint::invoke_from_event_loop(move || {
            let Some(app) = weak.upgrade() else { return };
            let mut st = state.lock().unwrap();
            store_columns(&mut st, &ok.columns, &ok.dtypes);
            let names = ok.columns.iter().find(|(n, _)| *n == id).map(|(_, v)| v.clone()).unwrap_or_default();
            let types = ok.dtypes.iter().find(|(n, _)| *n == id).map(|(_, v)| v.clone()).unwrap_or_default();
            st.native_types = Some((key, names.into_iter().zip(types).collect()));
            update_column_types(&app, &st);
            push_graph(&app, &st, false);
        });
    });
}

/// 执行显示节点及其上游，结果显示到表格。读取参数没动过的源节点直接用查看器自己的数据，不启动引擎。
pub(crate) fn run_graph(weak: &slint::Weak<AppWindow>, state: &Arc<Mutex<App>>) {
    let Some(app) = weak.upgrade() else { return };
    let mut st = state.lock().unwrap();
    // 改了图都会走到这里：记一步撤销（只换了选中的不算）
    undo::checkpoint(&mut st);
    cancel_runs(&mut st); // 在跑的旧请求作废
    st.running = false;
    st.graph.clear_errors();
    st.node_error.clear();
    st.node_output.clear();

    // 进到表达式网络里：表格显示选中的表达式节点的值（路径从 expr 节点里面算起）；没选中时显示网络的主人——
    // 在 expr 节点里是 expr 节点自己的结果，在 eval 里面是 eval 节点的值
    let inside = st.top_node().map(|n| n.id);
    let preview = inside.and_then(|_| {
        let mut path = st.inside[1..].to_vec();
        let net = st.net();
        match net.selected.and_then(|id| net.node(id)) {
            Some(n) => {
                path.push(n.id);
                Some((path, n.title.clone()))
            }
            None if !path.is_empty() => Some((path, st.inside_node()?.title.clone())),
            None => None,
        }
    });
    let Some(display) = inside.or(st.graph.display).filter(|&id| st.graph.node(id).is_some()) else {
        push_graph(&app, &st, false);
        update_busy(&app, &st);
        return;
    };
    if inside.is_none() && st.graph.is_native(display) {
        let restored = restore_native(&mut st);
        push_graph(&app, &st, false);
        update_busy(&app, &st);
        drop(st);
        if restored {
            show_current(weak, state);
        } else {
            fetch_native_types(weak, state);
        }
        return;
    }
    let steps = match st.graph.compile(display) {
        Ok(steps) => steps,
        Err(e) => {
            node_failed(&mut st, Some(e.node), &[], e.msg);
            push_graph(&app, &st, false);
            update_busy(&app, &st);
            return;
        }
    };

    st.running = true;
    let ticket = st.run_ticket;
    let mut title = st.graph.node(display).map(|n| n.title.clone()).unwrap_or_default();
    if let Some((path, sub)) = &preview {
        // eval 里面的节点：一路的名字（expr1 › eval1 › arith1）
        let mut owner = st.graph.node(display);
        for id in &path[..path.len().saturating_sub(1)] {
            owner = owner.and_then(|n| n.sub_node(*id));
            title = format!("{title} › {}", owner.map_or("", |n| n.title.as_str()));
        }
        title = format!("{title} › {sub}");
    }
    let out = engine::work_dir().join(format!("r{ticket}.parquet"));
    push_graph(&app, &st, false);
    app.set_status(SharedString::from(format!("正在运行 {title}…")));
    update_busy(&app, &st);
    drop(st);

    let weak = weak.clone();
    let state = state.clone();
    std::thread::spawn(move || {
        let picked = preview.as_ref().map(|(sub, label)| (sub.as_slice(), label.as_str()));
        let wanted = || LATEST_RUN.load(Ordering::Relaxed) == ticket;
        // 排队时又发了新的运行：这次不用算了（界面那边本来也会丢掉它的结果）
        let Some(ran) = engine::run_if_wanted(&steps, display, picked, &out, &wanted) else { return };
        let result = ran.and_then(|ok| match loader::open_tables(&out) {
            Ok(sheets) => Ok((ok, sheets)),
            Err(e) => Err(engine::RunErr {
                stdout: ok.stdout,
                columns: ok.columns,
                dtypes: ok.dtypes,
                subs: ok.subs,
                ..engine::RunErr::plain(e.to_string())
            }),
        });
        slint::invoke_from_event_loop(move || {
            let Some(app) = weak.upgrade() else { return };
            let mut st = state.lock().unwrap();
            if st.run_ticket != ticket {
                let _ = std::fs::remove_file(&out);
                return;
            }
            st.running = false;
            match &result {
                Ok((ok, _)) => {
                    store_columns(&mut st, &ok.columns, &ok.dtypes);
                    store_subs(&mut st, &ok.subs);
                    store_texts(&mut st, &ok.texts);
                }
                Err(e) => {
                    store_columns(&mut st, &e.columns, &e.dtypes);
                    store_subs(&mut st, &e.subs);
                    store_texts(&mut st, &e.texts);
                }
            }
            match result {
                Ok((ok, mut sheets)) => {
                    if st.file_sheets.is_none() {
                        let current = st.current;
                        st.file_sheets = Some((std::mem::take(&mut st.sheets), current));
                    }
                    if let Some(sheet) = sheets.first_mut() {
                        sheet.name = title.clone();
                    }
                    st.sheets = sheets;
                    st.current = 0;
                    st.reset_sheet_view();
                    st.node_status = format!("{title} · {} 行 · {} ms", fmt_count(ok.rows as usize), ok.ms);
                    st.last_ms = Some(ok.ms);
                    st.shown = Some(display);
                    st.shown_sub = preview.map(|(sub, _)| sub).unwrap_or_default();
                    st.preview_note = ok.note;
                    st.node_output = ok.stdout;
                    st.result_files.push(out);
                    while st.result_files.len() > 2 {
                        let _ = std::fs::remove_file(st.result_files.remove(0));
                    }
                    refresh_view(&app, &mut st);
                }
                Err(e) => {
                    node_failed(&mut st, e.node, &e.sub, e.msg);
                    st.node_output = e.stdout;
                    app.set_status(SharedString::from("运行失败，详见节点面板"));
                    update_busy(&app, &st);
                }
            }
            push_graph(&app, &st, false);
            // 正看着查询计划页：图变了，计划跟着重新要
            if app.get_view_tab() == 2 {
                drop(st);
                push_plan(&weak, &state);
            }
        })
        .expect("事件循环未运行");
    });
}

/// 选中节点面板里的按钮被点了（write 的“写出”）：跑它的上游，再让它带着 action 执行一次。和显示无关，表格不变。
pub(crate) fn run_action(weak: &slint::Weak<AppWindow>, state: &Arc<Mutex<App>>, index: usize) {
    let Some(app) = weak.upgrade() else { return };
    let mut st = state.lock().unwrap();
    let Some(id) = st.graph.selected else { return };
    if st.writing {
        return;
    }
    st.graph.clear_errors();
    st.node_error.clear();
    let Some((action, label, paste)) = (st.graph.node(id).and_then(|n| n.kind.params.get(index)))
        .filter(|p| p.kind == ParamKind::Button)
        .map(|p| (p.name.clone(), p.label.clone(), p.paste.clone()))
    else {
        return;
    };
    // 粘贴剪贴板（create 的 read_clipboard）：文字放进节点的参数，结果跟着参数走（缓存、工程文件都对），再重新运行
    if !paste.is_empty() {
        match clipboard::get_text() {
            Ok(text) => {
                let text = text.replace("\r\n", "\n");
                let rows = text.lines().count();
                if let Some(node) = st.graph.node_mut(id) {
                    node.set(&paste, &text);
                }
                app.set_status(SharedString::from(format!("已粘贴 {rows} 行")));
                drop(st);
                // 参数框要重建（里面的文字换了）：推迟到按钮的回调返回后
                let (weak, state) = (weak.clone(), state.clone());
                let _ = slint::invoke_from_event_loop(move || graph_changed(&weak, &state, true));
            }
            Err(msg) => {
                node_failed(&mut st, Some(id), &[], msg);
                push_graph(&app, &st, false);
            }
        }
        return;
    }
    let steps = match st.graph.compile_action(id, &action) {
        Ok(steps) => steps,
        Err(e) => {
            node_failed(&mut st, Some(e.node), &[], e.msg);
            push_graph(&app, &st, false);
            return;
        }
    };
    // 状态栏里说的是“写出 某文件”；没有文件参数的节点就说节点名
    let node = st.graph.node(id);
    // 文件参数藏起来了（write 写到剪贴板）就不提文件
    let shown_file = |n: &graph::Node| {
        let i = n.kind.params.iter().position(|p| matches!(p.kind, ParamKind::File | ParamKind::SaveFile))?;
        n.visible(i).then(|| n.file_path().map(String::from)).flatten()
    };
    let target = node.and_then(shown_file).filter(|path| !path.is_empty());
    let target = format!("{label} {}", target.or(node.map(|n| n.title.clone())).unwrap_or_default());
    st.writing = true;
    push_graph(&app, &st, false);
    app.set_status(SharedString::from(format!("正在{target}…")));
    update_busy(&app, &st);
    drop(st);

    let weak = weak.clone();
    let state = state.clone();
    std::thread::spawn(move || {
        let result = engine::action(&steps, id, &action);
        slint::invoke_from_event_loop(move || {
            let Some(app) = weak.upgrade() else { return };
            let mut st = state.lock().unwrap();
            st.writing = false;
            match result {
                Ok(ok) => {
                    store_columns(&mut st, &ok.columns, &ok.dtypes);
                    st.node_output = ok.stdout;
                    app.set_status(SharedString::from(format!("已{target} · {} ms", ok.ms)));
                }
                Err(e) => {
                    store_columns(&mut st, &e.columns, &e.dtypes);
                    node_failed(&mut st, e.node, &e.sub, e.msg);
                    st.node_output = e.stdout;
                    app.set_status(SharedString::from(format!("{label}失败，详见节点面板")));
                }
            }
            push_graph(&app, &st, false);
            update_busy(&app, &st);
        })
        .expect("事件循环未运行");
    });
}
