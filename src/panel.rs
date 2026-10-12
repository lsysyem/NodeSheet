//! 节点面板：把节点图同步到界面（画布 + 参数面板），以及面板上各种操作的回调。
//! 画布上显示的是 `App::net()`：表格级的节点图，或者进到某个 expr 节点里面的表达式网络。

use crate::*;

/// 下拉参数的选项和当前选中的下标。sheet 的第一项表示“不指定”。
pub(crate) fn param_options(node: &graph::Node, def: &nodes::ParamDef, value: &str) -> (Vec<String>, i32) {
    let is_sheet = def.kind == ParamKind::Sheet;
    let mut options: Vec<String> = match def.kind {
        ParamKind::Menu => def.options.clone(),
        ParamKind::Sheet => {
            std::iter::once(String::from("（第一个 sheet）")).chain(node.sheets.iter().cloned()).collect()
        }
        // 类型：下拉只是帮着填写法，值在旁边的输入框里（可以改成 Datetime("ms")、List(String)…）
        ParamKind::Dtype => {
            let blank = def.default.trim().is_empty();
            let options: Vec<String> = (blank.then(String::new).into_iter())
                .chain(nodes::DTYPES.iter().map(|(name, _)| name.to_string()))
                .collect();
            let base = nodes::dtype_base(value);
            let index = options.iter().position(|o| o == base).map_or(-1, |i| i as i32);
            return (options, index);
        }
        _ => return (Vec::new(), -1),
    };
    if is_sheet && value.is_empty() {
        return (options, 0);
    }
    match options.iter().skip(is_sheet as usize).position(|o| o == value) {
        Some(i) => (options, (i + is_sheet as usize) as i32),
        None => {
            // 手填的、列表里没有的值也要能显示
            options.push(value.to_string());
            let last = options.len() as i32 - 1;
            (options, last)
        }
    }
}

/// 节点的文件换了：格式页签跟着扩展名走；读 Excel 的节点重新取 sheet 列表，原来选的 sheet 不在新文件里就清掉。
pub(crate) fn file_changed(st: &mut App, id: u32) {
    let file = st.graph.file.clone();
    let viewer: Vec<String> =
        st.file_sheets.as_ref().map_or(&st.sheets, |(sheets, _)| sheets).iter().map(|s| s.name.clone()).collect();
    let Some(node) = st.graph.node_mut(id) else { return };
    node.sync_format();
    reload_sheets(node, &file, viewer);
}

/// 重新取节点所指文件的 sheet 列表（viewer：查看器打开的文件的 sheets，源节点直接用它）。
pub(crate) fn reload_sheets(node: &mut graph::Node, file: &str, viewer: Vec<String>) {
    let Some(si) = node.sheet_param() else { return };
    let path = node.file_path().unwrap_or("").to_string();
    node.sheets = if !node.reads_excel() {
        Vec::new()
    } else if node.primary && path == file {
        viewer
    } else {
        loader::sheet_names(Path::new(&path))
    };
    if !node.sheets.contains(&node.params[si]) {
        node.params[si].clear();
    }
}

/// polars 的类型 → 连线、类型标签的颜色组：0 不知道 / 1 数字 / 2 文字 / 3 真假 / 4 日期时间 / 5 列表结构 / 6 出错。
fn type_group(node: &graph::Node) -> i32 {
    if node.error {
        return 6;
    }
    let Some(t) = node.dtypes.first() else { return 0 };
    let base = t.split('(').next().unwrap_or(t);
    match base {
        _ if base.starts_with("Int") || base.starts_with("UInt") || base.starts_with("Float") || base == "Decimal" => 1,
        "String" | "Utf8" | "Categorical" | "Enum" => 2,
        "Boolean" => 3,
        "Date" | "Datetime" | "Time" | "Duration" => 4,
        "List" | "Array" | "Struct" | "Map" => 5,
        _ => 0,
    }
}

/// 表达式节点头上的类型标签：str / i64 / list[i64]…，多列时加上列数。
fn type_label(node: &graph::Node) -> String {
    let Some(t) = node.dtypes.first() else { return String::new() };
    let short = engine::short_dtype(t);
    match node.dtypes.len() {
        0 | 1 => short,
        n => format!("{short} ×{n}"),
    }
}

/// 表达式节点上一行一个的输入端口：名字，没连线时后面写着填的值。
fn port_items(g: &graph::Graph) -> Vec<PortItem> {
    let mut items = Vec::new();
    for (index, node) in g.nodes.iter().enumerate() {
        for (i, p) in node.kind.params.iter().enumerate() {
            let Some(slot) = node.kind.port_slot(i) else { continue };
            let wired = node.wired(i).is_some();
            let value = if wired { "" } else { node.params[i].trim() };
            items.push(PortItem {
                node: index as i32,
                slot: slot as i32,
                label: SharedString::from(p.label.as_str()),
                value: SharedString::from(value.lines().next().unwrap_or("")),
                wired,
            });
        }
    }
    items
}

/// 把节点图同步到界面。节点数没变时逐个原地更新（拖动中的节点不会被重建）；
/// with_params 为真时才重建参数编辑框（否则会打断正在输入的光标）。
pub(crate) fn push_graph(app: &AppWindow, st: &App, with_params: bool) {
    let g = st.net();
    let vop = g.vop;
    let items: Vec<NodeItem> = g
        .nodes
        .iter()
        .map(|n| NodeItem {
            title: SharedString::from(n.title.as_str()),
            x: n.x,
            y: n.y,
            inputs: n.inputs.len() as i32,
            selected: g.is_selected(n.id),
            display: !vop && g.display == Some(n.id),
            bypass: n.bypass,
            error: n.error,
            cat: if vop { 3 } else { nodes::category_index(&n.kind.category) as i32 },
            // 表达式节点算不出来：右边直接写原因；有提醒时写提醒
            summary: SharedString::from(match () {
                _ if vop && n.error && !n.note.is_empty() => n.note.clone(),
                _ if vop && !n.error && !n.warn.is_empty() => n.warn.clone(),
                // 排序键：写出是第几键（先后按画布上从上到下，挪动节点会变）
                _ if vop && n.kind.role == "排序" => {
                    let rank = format!("第 {} 键", g.output_rank(n.id) + 1);
                    match n.summary() {
                        rest if rest.is_empty() => rank,
                        rest => format!("{rank} · {rest}"),
                    }
                }
                _ => n.summary(),
            }),
            warn: vop && !n.error && !n.warn.is_empty(),
            badge: SharedString::from(nodes::source_label(&n.kind, true)),
            dtype: SharedString::from(type_label(n)),
            tgroup: type_group(n),
            output: !n.kind.role.is_empty(),
        })
        .collect();
    let wires = wire_items(st);

    let switched = app.get_vop() != vop;
    app.set_vop(vop);
    let model = app.get_nodes();
    match model.as_any().downcast_ref::<VecModel<NodeItem>>().filter(|_| !switched) {
        // 原地改：多了补在后面、少了从后面去掉，正按着的节点不会被重建（Alt 拖复制时节点数在拖动中变了）。
        // 节点数变了时先清掉连线和端口：它们里面的下标可能越界
        Some(shown) => {
            let resized = shown.row_count() != items.len();
            if resized {
                app.set_wires(ModelRc::new(VecModel::from(Vec::new())));
                app.set_vop_ports(ModelRc::new(VecModel::from(Vec::new())));
            } else {
                app.set_wires(ModelRc::new(VecModel::from(wires.clone())));
            }
            for (i, item) in items.into_iter().enumerate() {
                if i >= shown.row_count() {
                    shown.push(item);
                } else if shown.row_data(i).as_ref() != Some(&item) {
                    shown.set_row_data(i, item);
                }
            }
            while shown.row_count() > g.nodes.len() {
                shown.remove(shown.row_count() - 1);
            }
            if resized {
                app.set_wires(ModelRc::new(VecModel::from(wires)));
            }
        }
        None => {
            app.set_wires(ModelRc::new(VecModel::from(Vec::new())));
            app.set_vop_ports(ModelRc::new(VecModel::from(Vec::new())));
            app.set_nodes(ModelRc::new(VecModel::from(items)));
            app.set_wires(ModelRc::new(VecModel::from(wires)));
        }
    }
    let ports = if vop { port_items(g) } else { Vec::new() };
    let shown_ports = app.get_vop_ports();
    if shown_ports.row_count() != ports.len() || shown_ports.iter().zip(&ports).any(|(a, b)| a != *b) {
        app.set_vop_ports(ModelRc::new(VecModel::from(ports)));
    }

    app.set_wire_selected(g.selected_wire.is_some());
    // 窗口标题前的 ●：节点图改过了还没存
    app.set_modified(undo::dirty(st));
    // 路径栏：一路进到了哪些节点里面（expr1 › eval1，每一级都能点回去），最里面那一级的上下文。
    // group_by 的 expr 节点里有几张网络：在它那一级是页签，再往里时名字后面写着是哪一张
    let inside = st.top_node();
    let env = st.env();
    let envs = inside.map_or(&[][..], |n| n.envs());
    let titles: Vec<String> = (1..=st.inside.len())
        .filter_map(|depth| st.graph_at(&st.inside[..depth - 1])?.node(st.inside[depth - 1]))
        .enumerate()
        .map(|(depth, n)| match depth {
            0 if envs.len() > 1 && st.inside.len() > 1 => format!("{} · {}", n.title, env.label()),
            _ => n.title.clone(),
        })
        .collect();
    let crumbs: Vec<SharedString> = titles.into_iter().map(SharedString::from).collect();
    if app.get_net_crumbs().iter().ne(crumbs.iter().cloned()) {
        app.set_net_crumbs(ModelRc::new(VecModel::from(crumbs)));
    }
    app.set_net_context(SharedString::from(st.inside_node().map_or("", |n| n.context_label())));
    let tabs: Vec<SharedString> = match envs {
        [_, _, ..] if st.inside.len() == 1 => envs.iter().map(|e| SharedString::from(format!("{} · {}", e.label(), e.scope()))).collect(),
        _ => Vec::new(),
    };
    if app.get_net_envs().iter().ne(tabs.iter().cloned()) {
        app.set_net_envs(ModelRc::new(VecModel::from(tabs)));
    }
    app.set_net_env(envs.iter().position(|e| *e == env).unwrap_or(0) as i32);
    app.set_net_hint(SharedString::from(match env {
        _ if envs.len() < 2 || st.inside.len() != 1 => "",
        graph::Env::Keys => "分组键是空的 · 按 Tab 加节点接到“分组键”输出：在整张表上算，每行一个值，值一样的行分到一组",
        graph::Env::Main => "聚合是空的 · 按 Tab 加节点接到“输出列”：在每组里算，接一个聚合（sum / mean / len…）得到每组一个值",
        graph::Env::Having => "筛选组是空的，所有的组都留着 · 要只留一部分组：按 Tab 加节点接到“条件”输出（having：在每组里算，如 sum > 100）",
    }));

    // 表格的路径栏：表格里是哪个节点的结果、算了多久（文件本身的数据算源节点的）
    let shown = match (&st.file_sheets, st.shown) {
        (Some(_), Some(id)) => st.graph.node(id),
        _ => st.graph.primary(),
    };
    // 预览网络里的节点：路径上一级一级的名字
    let mut subs: Vec<&graph::Node> = Vec::new();
    for &id in &st.shown_sub {
        let owner = subs.last().copied().or(shown);
        match owner.and_then(|n| n.sub_node(id)) {
            Some(n) => subs.push(n),
            None => break,
        }
    }
    let shown_sub = subs.last().copied();
    let view_title = match shown {
        Some(n) => std::iter::once(n).chain(subs.iter().copied()).map(|n| n.title.as_str()).collect::<Vec<_>>().join(" › "),
        None => String::new(),
    };
    app.set_view_title(SharedString::from(view_title));
    let view_summary = match shown_sub {
        // filter 里预览真假值：有几行满足（引擎数的）
        Some(sub) if !st.preview_note.is_empty() => format!("预览：{} · {}", sub.kind.name, st.preview_note),
        Some(sub) => format!("预览：{}（连同它用到的列）", sub.kind.name),
        None => shown.map(|n| n.summary()).unwrap_or_default(),
    };
    app.set_view_summary(SharedString::from(view_summary));
    app.set_view_time(SharedString::from(match st.last_ms {
        Some(ms) if ms < 1000 => format!("{ms} ms"),
        Some(ms) => format!("{:.1} s", ms as f64 / 1000.0),
        None => String::new(),
    }));

    // 正看着 Python 脚本页：图变了，脚本跟着重新生成
    if app.get_view_tab() == 1 {
        push_script(app, st);
    }

    // 参数面板：选中的节点；表达式网络里没选中节点时是 expr 节点自己
    let (path, target) = st.param_target();
    let in_net = !path.is_empty();
    let pg = st.graph_at(&path).unwrap_or(&st.graph);
    let selected = target.and_then(|id| pg.node(id));
    type_editor::push_entry(app, selected.filter(|_| !vop));
    app.set_node_vop(vop);
    // 选中的是这一级图里的节点（不是网络的主人自己）：里面有网络就能进去，表达式节点能转成文字 / 展开
    let own = path == st.inside;
    app.set_node_net(own && selected.is_some_and(|n| n.net.is_some()));
    app.set_node_text_action(if own && in_net { selected.map_or(0, text_action_of) } else { 0 });
    match selected {
        Some(node) => {
            let def = node.kind();
            app.set_node_title(SharedString::from(node.title.as_str()));
            app.set_node_desc(SharedString::from(def.desc.as_str()));
            app.set_node_source(SharedString::from(nodes::source_label(&node.kind, false)));
            app.set_node_cat(if in_net { 3 } else { nodes::category_index(&def.category) as i32 });
            // 有引擎写的文字（网络 → 文字）：底下一行是这个节点的表达式，展开后再列节点的代码
            let code = pg.code_preview(node.id);
            let (label, head, code) = match node.text.as_str() {
                "" => ("代码", code_head(&code), code),
                text if code.is_empty() => ("表达式", text.to_string(), text.to_string()),
                text => ("表达式", text.to_string(), format!("{text}

# ---- 节点的代码（ch(\"参数\") 取参数）
{code}")),
            };
            app.set_node_code_label(SharedString::from(label));
            app.set_node_code_head(SharedString::from(head));
            app.set_node_code(SharedString::from(code));
            app.set_node_kind(SharedString::from(def.name.as_str()));
            if with_params {
                // 只列当前页签里的参数；id 是它在节点参数表里的下标（回调里用）
                let params: Vec<ParamItem> = def
                    .params
                    .iter()
                    .zip(&node.params)
                    .enumerate()
                    .filter(|(i, _)| node.visible(*i))
                    .map(|(i, (p, value))| {
                        let (options, index) = param_options(node, p, value);
                        // 菜单里的空选项（不指定，用 polars 的默认）给它个名字
                        let options: Vec<SharedString> = (options.iter())
                            .map(|o| SharedString::from(if o.is_empty() && matches!(p.kind, ParamKind::Menu | ParamKind::Dtype) { "（默认）" } else { o }))
                            .collect();
                        // 页签和按钮自己占一行，不带标签；按钮上的字就是参数名。选项多的“方式”画成带标签的下拉
                        let tabs = p.kind == ParamKind::Menu && p.mode && p.options.len() <= 5;
                        let button = p.kind == ParamKind::Button;
                        let bare = tabs || button;
                        // 表达式节点的端口连了线：不能填，写着从哪接来的
                        let wired = node.wired(i).and_then(|up| pg.node(up));
                        let value = match wired {
                            Some(up) => format!("← {}", up.title),
                            None if button => p.label.clone(),
                            // 筛选条件：这一行写条件的一句话（点旁边的“编辑”打开条件编辑器）
                            None if p.kind == ParamKind::Conditions => cond::summary(value),
                            None => value.clone(),
                        };
                        ParamItem {
                            id: i as i32,
                            label: SharedString::from(if bare { "" } else { p.label.as_str() }),
                            value: SharedString::from(value),
                            hint: SharedString::from(p.hint.as_str()),
                            kind: match p.kind {
                                _ if wired.is_some() => 7,
                                ParamKind::Code => 1,
                                ParamKind::Toggle => 2,
                                ParamKind::Menu if tabs => 5,
                                ParamKind::Menu | ParamKind::Sheet => 3,
                                ParamKind::File | ParamKind::SaveFile => 4,
                                ParamKind::Button => 6,
                                ParamKind::Dtype => 8,
                                ParamKind::Conditions => 9,
                                _ => 0,
                            },
                            options: ModelRc::new(VecModel::from(options)),
                            index,
                            // 表达式、路径这些长内容：标签放在上面，编辑框占满一行
                            stacked: bare
                                || matches!(p.kind, ParamKind::Code | ParamKind::File | ParamKind::SaveFile)
                                || matches!(p.kind, ParamKind::Dict | ParamKind::Kwargs)
                                || (!in_net && p.insert() != nodes::ColInsert::None),
                            pick: !button && wired.is_none() && pick::available(st, &path, node.id, i),
                            pick_ranges: !button && wired.is_none() && p.pick() == nodes::Pick::Ranges,
                        }
                    })
                    .collect();
                app.set_node_params(ModelRc::new(VecModel::from(params)));
                app.set_ac_items(ModelRc::new(VecModel::from(Vec::new())));
                app.set_code_hint(SharedString::new());
                // 显示着的那个代码框（create 有两个：from_repr 的表格、read_clipboard 粘贴的表格）
                let code = (def.params.iter().zip(&node.params).enumerate())
                    .find(|(i, (p, _))| p.kind == ParamKind::Code && node.visible(*i))
                    .map(|(_, pair)| pair);
                if let Some((_, value)) = code {
                    complete::load_catalog();
                    set_code_layers(app, value, def.name == "sql");
                }
            }
        }
        None => {
            app.set_node_title(SharedString::new());
            app.set_node_code(SharedString::new());
            app.set_node_params(ModelRc::new(VecModel::from(Vec::new())));
        }
    }
    // 表达式网络里选中的节点算不出来：说它自己的原因
    let note = selected.filter(|_| in_net).map_or("", |n| n.note.as_str());
    app.set_node_error(SharedString::from(if note.is_empty() { st.node_error.as_str() } else { note }));
    let warn = selected.filter(|n| in_net && !n.error).map_or("", |n| n.warn.as_str());
    app.set_node_warn(SharedString::from(warn));
    app.set_node_output(SharedString::from(st.node_output.trim_end()));
    app.set_node_running(st.running);
}

/// 画布上的连线（下标是 nodes 里的）：选中的、正拖着的节点压着的（松手就插进去）。
fn wire_items(st: &App) -> Vec<WireItem> {
    let g = st.net();
    let hot = st.node_drag.as_ref().and_then(|d| d.hot);
    let mut items = Vec::new();
    for (dst, n) in g.nodes.iter().enumerate() {
        for (slot, src) in n.inputs.iter().enumerate() {
            let Some(src) = src.and_then(|id| g.index_of(id)) else { continue };
            let wire = Some((n.id, slot));
            items.push(WireItem {
                src: src as i32,
                dst: dst as i32,
                slot: slot as i32,
                selected: g.selected_wire == wire,
                hot: hot == wire,
            });
        }
    }
    items
}

/// 代码预览收起时的一行：第一行不是注释的代码，后面注明一共几行。
fn code_head(code: &str) -> String {
    let lines: Vec<&str> = code.lines().filter(|l| !l.trim().is_empty()).collect();
    let first = lines.iter().find(|l| !l.trim_start().starts_with('#')).or(lines.first()).map_or("", |l| l.trim());
    match lines.len() {
        0 | 1 => first.to_string(),
        n => format!("{first}    …共 {n} 行"),
    }
}

/// 帮助窗口顶上“这个节点”的情况：输出的行列和用时（表格正显示它时才知道行数）、输入、各列类型。
pub(crate) fn node_info(st: &App, g: &graph::Graph, node: &graph::Node) -> String {
    if g.vop {
        // 表达式节点：输出类型、接了什么
        let mut lines = vec![format!("这个节点：{}", node.title)];
        lines.push(match (node.note.as_str(), type_label(node)) {
            ("", t) if t.is_empty() => String::from("输出：还没算过"),
            ("", t) => format!("输出：{t}"),
            (note, _) => format!("算不出来：{note}"),
        });
        if !node.warn.is_empty() && !node.error {
            lines.push(format!("提醒：{}", node.warn));
        }
        let wired: Vec<String> = (0..node.kind.params.len())
            .filter_map(|i| {
                let up = g.node(node.wired(i)?)?;
                Some(format!("{} ← {}", node.kind.params[i].label, up.title))
            })
            .collect();
        if !wired.is_empty() {
            lines.push(format!("输入：{}", wired.join("、")));
        }
        if !node.kind.role.is_empty() {
            lines.push(format!("输出节点：交给上下文的“{}”", node.kind.role));
        }
        return lines.join("\n");
    }
    let mut lines = vec![format!("这个节点：{}", node.title)];
    let shown = g.display == Some(node.id);
    let rows = if shown { st.sheets.get(st.current).map(|s| s.source.row_count()) } else { None };
    let cols = if node.columns.is_empty() && shown {
        st.sheets.get(st.current).map_or(0, |s| s.source.headers().len())
    } else {
        node.columns.len()
    };
    let mut output = match (rows, cols) {
        (Some(rows), cols) => format!("输出：{} 行 × {cols} 列", fmt_count(rows)),
        (None, 0) => String::from("输出：还没运行过（让表格显示它，或运行它下游的节点）"),
        (None, cols) => format!("输出：{cols} 列"),
    };
    if let (true, Some(ms)) = (shown, st.last_ms) {
        output += &format!(" · 用时 {ms} ms");
    }
    lines.push(output);
    if !node.inputs.is_empty() {
        let inputs: Vec<String> =
            node.inputs.iter().map(|i| i.and_then(|id| g.node(id)).map_or(String::from("—"), |n| n.title.clone())).collect();
        lines.push(format!("输入：{}", inputs.join("、")));
    }
    if node.bypass {
        lines.push(String::from("已旁路：输入原样传给下游"));
    }
    if node.net.is_some() {
        lines.push(String::from("双击（或回车）进去看表达式网络"));
    }
    if !node.columns.is_empty() {
        const SHOW: usize = 12;
        let mut items: Vec<String> = node
            .columns
            .iter()
            .enumerate()
            .take(SHOW)
            .map(|(i, name)| match node.dtypes.get(i) {
                Some(t) => format!("{name} {}", engine::short_dtype(t)),
                None => name.clone(),
            })
            .collect();
        if node.columns.len() > SHOW {
            items.push(format!("…另 {} 列", node.columns.len() - SHOW));
        }
        lines.push(format!("列：{}", items.join("、")));
    }
    lines.join("\n")
}

/// Python 脚本页：从源节点到显示节点的脚本。
fn push_script(app: &AppWindow, st: &App) {
    let g = &st.graph;
    let target = st.top_node().map(|n| n.id).or(g.display);
    let text = match target {
        None => String::from("# 还没有节点图：先打开一个表格文件"),
        Some(id) => match g.python_script(id, engine::script_prelude()) {
            Ok(script) => script,
            Err(e) => {
                let name = g.node(e.node).map_or(String::new(), |n| n.title.clone());
                format!("# 还导出不了：{name}：{}", e.msg)
            }
        },
    };
    let layers: Vec<SharedString> = complete::layers(&text, false).iter().map(SharedString::from).collect();
    app.set_script_layers(ModelRc::new(VecModel::from(layers)));
    app.set_script_text(SharedString::from(text));
}

/// 查询计划页：向引擎要显示节点的 explain（优化后、优化前），在后台要，回来时还看着这一页才放上去。
pub(crate) fn push_plan(weak: &slint::Weak<AppWindow>, state: &Arc<Mutex<App>>) {
    let Some(app) = weak.upgrade() else { return };
    let st = state.lock().unwrap();
    let g = &st.graph;
    let target = st.top_node().map(|n| n.id).or(g.display);
    let steps = match target.map(|id| g.compile(id)) {
        None => Err(String::from("还没有节点图：先打开一个表格文件")),
        Some(Ok(steps)) => Ok(steps),
        Some(Err(e)) => Err(format!("{}：{}", g.node(e.node).map_or(String::new(), |n| n.title.clone()), e.msg)),
    };
    let tree = app.get_plan_tree();
    drop(st);
    let steps = match steps {
        Ok(steps) => steps,
        Err(msg) => return set_plan(&app, &msg),
    };
    set_plan(&app, "正在生成查询计划…");
    let target = target.unwrap_or_default();
    let weak = weak.clone();
    std::thread::spawn(move || {
        let text = match engine::explain(&steps, target, tree) {
            Ok(ok) => format!("── 优化后 ──\n{}\n\n── 优化前 ──\n{}", ok.plan.0.trim_end(), ok.plan.1.trim_end()),
            Err(e) => format!("生成不了查询计划：{}", e.msg),
        };
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(app) = weak.upgrade().filter(|app| app.get_view_tab() == 2 && app.get_plan_tree() == tree) {
                set_plan(&app, &text);
            }
        });
    });
}

/// 计划是纯文字：只有一层（不高亮）。
fn set_plan(app: &AppWindow, text: &str) {
    app.set_plan_layers(ModelRc::new(VecModel::from(vec![SharedString::from(text)])));
    app.set_plan_text(SharedString::from(text));
}

pub(crate) fn set_code_layers(app: &AppWindow, code: &str, sql: bool) {
    let layers: Vec<SharedString> = complete::layers(code, sql).iter().map(SharedString::from).collect();
    app.set_code_layers(ModelRc::new(VecModel::from(layers)));
}

/// 代码编辑框的内容或光标变了：重算高亮、补全和签名提示。
pub(crate) fn code_assist(app: &AppWindow, st: &mut App, text: &str, cursor: usize) {
    let g = st.net();
    let Some(node) = g.selected.and_then(|id| g.node(id)) else { return };
    let sql = node.kind().name == "sql";
    // 引号里补全的列名：所有接进来的输入的列
    let mut columns: Vec<String> = Vec::new();
    for up in node.inputs.iter().flatten().filter_map(|&id| g.node(id)) {
        for name in &up.columns {
            if !columns.contains(name) {
                columns.push(name.clone());
            }
        }
    }
    set_code_layers(app, text, sql);
    // ch("…") 里补全的参数名：这个节点定义的全部参数
    let params: Vec<complete::Item> = node
        .kind()
        .params
        .iter()
        .map(|p| complete::Item {
            label: p.name.clone(),
            detail: if p.label.is_empty() || p.label == p.name { String::from("参数") } else { p.label.clone() },
            doc: p.hint.clone(),
        })
        .collect();
    // 路径函数里没写目录时：从当前打开的文件所在目录开始列
    let fallback = std::path::Path::new(&st.graph.file)
        .parent()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    let found = complete::complete(text, cursor, &columns, &params, sql, &fallback);
    let items: Vec<CompletionItem> = found
        .items
        .iter()
        .map(|i| CompletionItem {
            label: SharedString::from(i.label.as_str()),
            detail: SharedString::from(i.detail.as_str()),
            doc: SharedString::from(i.doc.as_str()),
        })
        .collect();
    app.set_ac_items(ModelRc::new(VecModel::from(items)));
    app.set_ac_index(0);
    app.set_code_hint(SharedString::from(if sql { String::new() } else { complete::hint(text, cursor) }));
    st.completions = found.items;
    st.completion_start = found.start;
}

/// Delete：选中了连线就只删连线；否则删选中的全部节点（源节点不能删），接它们的下游改接它们的上游。
pub(crate) fn delete_selection(weak: &slint::Weak<AppWindow>, state: &Arc<Mutex<App>>) {
    {
        let mut st = state.lock().unwrap();
        let g = st.net_mut();
        if let Some((to, slot)) = g.selected_wire {
            g.disconnect(to, slot);
        } else {
            let ids = g.selection();
            let (primary, rest): (Vec<u32>, Vec<u32>) = ids.into_iter().partition(|&id| g.node(id).is_some_and(|n| n.primary));
            for &id in &rest {
                g.delete(id);
            }
            // 表达式网络里删了节点：没选中的了，表格回到 expr 节点的结果
            if g.vop && !rest.is_empty() {
                g.select_only(None);
            }
            if !primary.is_empty() {
                st.node_error = String::from("源节点对应当前打开的文件，不能删除");
            }
            if rest.is_empty() {
                if let Some(app) = weak.upgrade() {
                    push_graph(&app, &st, false);
                }
                return;
            }
        }
    }
    graph_changed(weak, state, true);
}

/// 把 from 的输出接到 to 的第 slot 个输入，然后重新运行。
pub(crate) fn connect_nodes(weak: &slint::Weak<AppWindow>, state: &Arc<Mutex<App>>, from: u32, to: u32, slot: usize) {
    {
        let mut st = state.lock().unwrap();
        let g = st.net_mut();
        g.selected_wire = None;
        if !g.connect(from, to, slot) {
            st.node_error = String::from(if from == to { "不能接到自己身上" } else { "不能这样连：会形成环" });
            if let Some(app) = weak.upgrade() {
                push_graph(&app, &st, false);
            }
            return;
        }
    }
    // 端口连上线以后，参数面板里那一项变成“← 来自…”
    graph_changed(weak, state, true);
}

/// 改了图的结构或参数后：同步界面并重新运行。
pub(crate) fn graph_changed(weak: &slint::Weak<AppWindow>, state: &Arc<Mutex<App>>, with_params: bool) {
    if let Some(app) = weak.upgrade() {
        push_graph(&app, &state.lock().unwrap(), with_params);
    }
    run_graph(weak, state);
}

/// 正在改参数面板上那个节点的参数：它上次运行留下的报错不再作数。
pub(crate) fn clear_node_error(st: &mut App, id: u32) {
    if let Some(node) = st.param_graph().0.node_mut(id).filter(|n| n.error) {
        node.error = false;
        node.note.clear();
        st.node_error.clear();
    }
}

/// 进到带网络的节点里面（index < 0：选中的那个）：表格级的 expr 节点，或网络里的 eval 节点。别的节点什么也不做。
pub(crate) fn enter_node(weak: &slint::Weak<AppWindow>, state: &Arc<Mutex<App>>, index: i32) {
    {
        let mut st = state.lock().unwrap();
        let g = st.net();
        let id = match usize::try_from(index) {
            Ok(i) => g.nodes.get(i).map(|n| n.id),
            Err(_) => g.selected,
        };
        let Some(node) = id.and_then(|id| g.node(id)).filter(|n| n.net.is_some()) else { return };
        let id = node.id;
        st.net_mut().selected = Some(id);
        st.inside.push(id);
        let Some(app) = weak.upgrade() else { return };
        push_kinds(&app, &mut st);
        app.set_add_cat(0);
    }
    fit_after_switch(weak, state);
    graph_changed(weak, state, true);
}

/// 回到上一级（levels 级；usize::MAX = 一直回到表格级）。出来以后选中的是刚才进去的那个节点。
pub(crate) fn go_up_levels(weak: &slint::Weak<AppWindow>, state: &Arc<Mutex<App>>, levels: usize) {
    {
        let mut st = state.lock().unwrap();
        if st.inside.is_empty() {
            return;
        }
        let keep = st.inside.len().saturating_sub(levels);
        let left = st.inside[keep];
        st.inside.truncate(keep);
        st.net_mut().selected = Some(left);
        let Some(app) = weak.upgrade() else { return };
        push_kinds(&app, &mut st);
        app.set_add_cat(0);
    }
    fit_after_switch(weak, state);
    graph_changed(weak, state, true);
}

pub(crate) fn go_up(weak: &slint::Weak<AppWindow>, state: &Arc<Mutex<App>>) {
    go_up_levels(weak, state, 1);
}

/// 换了一级：画布摆到能看见全部节点（界面那边量好画布大小再回调 fit-view）。
fn fit_after_switch(weak: &slint::Weak<AppWindow>, state: &Arc<Mutex<App>>) {
    if let Some(app) = weak.upgrade() {
        push_graph(&app, &state.lock().unwrap(), true);
        app.invoke_request_fit();
    }
}

/// 给选中节点的文件参数选一个文件：read 是打开对话框，write 是保存对话框。
pub(crate) fn browse_param(weak: &slint::Weak<AppWindow>, state: &Arc<Mutex<App>>, index: usize) {
    // formats：节点的格式菜单（带扩展名表的那个）里的各个格式和它们的扩展名，当前选中的排最前
    let (saving, formats, current) = {
        let st = state.lock().unwrap();
        let Some(node) = st.graph.selected.and_then(|id| st.graph.node(id)).filter(|_| st.inside.is_empty()) else {
            return;
        };
        let (Some(def), Some(value)) = (node.kind().params.get(index), node.params.get(index)) else { return };
        let menu = node.kind().params.iter().position(|p| !p.extensions.is_empty());
        let mut formats = menu.map_or(Vec::new(), |i| node.kind().params[i].extensions.clone());
        if let Some(i) = menu {
            // 定义文件里的扩展名表按字母排过：照菜单选项的顺序排回来，选中的那个提到最前
            let options = &node.kind().params[i].options;
            formats.sort_by_key(|(name, _)| (*name != node.params[i], options.iter().position(|o| o == name)));
        }
        (def.kind == ParamKind::SaveFile, formats, PathBuf::from(value.trim()))
    };
    let mut dialog = rfd::FileDialog::new();
    if let Some(dir) = current.parent().filter(|d| d.is_dir()) {
        dialog = dialog.set_directory(dir);
    }
    let picked = if saving {
        // 当前格式排在最前面（对话框默认选它）；每种格式只给第一个扩展名，存出来的文件名才确定
        for (name, extensions) in &formats {
            dialog = dialog.add_filter(name, &extensions[..extensions.len().min(1)]);
        }
        if let Some(name) = current.file_name().and_then(|n| n.to_str()) {
            dialog = dialog.set_file_name(name);
        }
        dialog.set_title("写出到").save_file()
    } else {
        let all: Vec<&String> = formats.iter().flat_map(|(_, extensions)| extensions).collect();
        if !all.is_empty() {
            dialog = dialog.add_filter("表格文件", &all);
        }
        dialog.set_title("选择文件").add_filter("所有文件", &["*"]).pick_file()
    };
    let Some(path) = picked else { return };
    {
        let mut st = state.lock().unwrap();
        let Some(id) = st.graph.selected else { return };
        match st.graph.node_mut(id).and_then(|n| n.params.get_mut(index)) {
            Some(value) => *value = path.to_string_lossy().into_owned(),
            None => return,
        }
        file_changed(&mut st, id);
    }
    graph_changed(weak, state, true);
}

fn kind_item(kind: &NodeType, cat: i32) -> KindItem {
    KindItem {
        name: SharedString::from(kind.name.as_str()),
        desc: SharedString::from(kind.desc.as_str()),
        cat,
    }
}

/// “添加”菜单里的节点类型和分类：当前这一级的（表格级 / 表达式级），取自注册表（用户加了 / 改了节点类型之后要重新推一次）；
/// 工具架总是表格级的。
pub(crate) fn push_kinds(app: &AppWindow, st: &mut App) {
    st.shelf_kinds = nodes::all();
    let shelf_cats = nodes::categories();
    let items: Vec<KindItem> =
        st.shelf_kinds.iter().map(|k| kind_item(k, shelf_cats.iter().position(|c| *c == k.category).unwrap_or(0) as i32)).collect();
    app.set_shelf_kinds(ModelRc::new(VecModel::from(items)));
    let cats: Vec<SharedString> = shelf_cats.iter().map(SharedString::from).collect();
    app.set_shelf_cats(ModelRc::new(VecModel::from(cats)));

    let vop = st.inside_node().is_some();
    st.kinds = if vop { nodes::all_expr() } else { st.shelf_kinds.clone() };
    let cats = if vop { nodes::expr_categories() } else { shelf_cats };
    let items: Vec<KindItem> =
        st.kinds.iter().map(|k| kind_item(k, cats.iter().position(|c| *c == k.category).unwrap_or(0) as i32)).collect();
    app.set_node_kinds(ModelRc::new(VecModel::from(items)));
    let icons: Vec<i32> = (0..cats.len() as i32).map(|i| if vop { 3 } else { i }).collect();
    app.set_node_cat_icons(ModelRc::new(VecModel::from(icons)));
    let cats: Vec<SharedString> = cats.iter().map(SharedString::from).collect();
    app.set_node_cats(ModelRc::new(VecModel::from(cats)));
}

/// 刚建的节点第一个参数是要读的文件：直接让用户选。
fn browse_new_node(weak: &slint::Weak<AppWindow>, state: &Arc<Mutex<App>>, kind: &NodeType) {
    if !kind.expr && kind.params.first().is_some_and(|p| p.kind == ParamKind::File) {
        browse_param(weak, state, 0);
    }
}

/// 选中节点输出的类型对应哪个命名空间（Tab 菜单里把它排前面）。
fn preferred_category(st: &App) -> &'static str {
    let g = st.net();
    // group_by 在每组里算的网络（聚合、筛选组）里：没选中节点、或选中的是输出节点时，多半是要接一个聚合
    let grouping = st.inside.len() == 1
        && st.inside_node().is_some_and(|n| n.value("context") == "group_by")
        && st.env() != graph::Env::Keys;
    let Some(node) = g.selected.and_then(|id| g.node(id)) else { return if grouping { "聚合" } else { "" } };
    if grouping && !node.kind.role.is_empty() {
        return "聚合";
    }
    match type_group(node) {
        2 => "文本 str",
        4 => "日期 dt",
        5 if node.dtypes.first().is_some_and(|t| t.starts_with("Struct")) => "结构 struct",
        5 if node.dtypes.first().is_some_and(|t| t.starts_with("Array")) => "数组 arr",
        5 if node.dtypes.first().is_some_and(|t| t.starts_with("Map")) => "映射 map",
        5 => "列表 list",
        1 => "数学",
        _ => "",
    }
}

pub(crate) fn install_graph_callbacks(app: &AppWindow, state: &Arc<Mutex<App>>) {
    push_kinds(app, &mut state.lock().unwrap());

    let changed = graph_changed;

    {
        // Tab 菜单里输入了什么：列出匹配的节点（以及页签里的方式，如 slice · tail）
        let weak = app.as_weak();
        let state = state.clone();
        app.on_search_nodes(move |query| {
            let mut st = state.lock().unwrap();
            let hits = if st.inside_node().is_some() {
                nodes::search_expr(&query, preferred_category(&st))
            } else {
                nodes::search(&query)
            };
            let vop = st.inside_node().is_some();
            let mut items: Vec<KindItem> = hits
                .iter()
                .map(|hit| {
                    let cat = if vop { 3 } else { nodes::category_index(&hit.kind.category) as i32 };
                    let mut item = kind_item(&hit.kind, cat);
                    if let Some(option) = &hit.option {
                        item.name = SharedString::from(format!("{} · {option}", hit.kind.name));
                    }
                    item
                })
                .collect();
            // 网络里写的是一段表达式（pl.col("a").str.len_chars()）：第一项按这段文字建节点
            st.search_text = (vop && looks_like_expr(&query)).then(|| query.trim().to_string());
            if st.search_text.is_some() {
                let desc = SharedString::from("拆成节点：每一步一个表达式节点，认不出的部分写成 snippet");
                items.insert(0, KindItem { name: SharedString::from(format!("按文字建节点：{}", query.trim())), desc, cat: 3 });
            }
            st.search_hits = hits;
            if let Some(app) = weak.upgrade() {
                app.set_search_results(ModelRc::new(VecModel::from(items)));
            }
        });
    }
    {
        // 添加第 index 条搜索结果，放在画布的 (x, y)
        let weak = app.as_weak();
        let state = state.clone();
        app.on_add_found(move |index, x, y| {
            let mut index = index.max(0) as usize;
            let text = state.lock().unwrap().search_text.take();
            if let Some(text) = text {
                if index == 0 {
                    build_from_text(&weak, &state, text, (x, y));
                    return;
                }
                index -= 1;
            }
            let kind = {
                let mut st = state.lock().unwrap();
                let Some(hit) = st.search_hits.get(index).cloned() else { return };
                let g = st.net_mut();
                let wire = g.selected_wire.take();
                let id = g.add_at(&hit.kind, Some((x, y)));
                // 选着连线按 Tab：新节点插进这条线
                if let Some(wire) = wire {
                    g.insert_on_wire(id, wire);
                }
                // 搜到的是某个方式：建好就切过去
                if let (Some(option), Some(i), Some(node)) = (hit.option, hit.kind.mode_param(), g.node_mut(id)) {
                    node.params[i] = option;
                }
                hit.kind
            };
            changed(&weak, &state, true);
            browse_new_node(&weak, &state, &kind);
        });
    }

    {
        let weak = app.as_weak();
        let state = state.clone();
        app.on_add_node(move |kind| {
            let kind = {
                let mut st = state.lock().unwrap();
                let Some(kind) = st.kinds.get(kind.max(0) as usize).cloned() else { return };
                st.net_mut().add(&kind);
                kind
            };
            changed(&weak, &state, true);
            browse_new_node(&weak, &state, &kind);
        });
    }
    {
        // 工具架：总是表格级的节点；在表达式网络里点了就先回到表格级，接在 expr 节点后面
        let weak = app.as_weak();
        let state = state.clone();
        app.on_shelf_add(move |kind| {
            let inside = !state.lock().unwrap().inside.is_empty();
            if inside {
                go_up_levels(&weak, &state, usize::MAX);
            }
            let kind = {
                let mut st = state.lock().unwrap();
                let Some(kind) = st.shelf_kinds.get(kind.max(0) as usize).cloned() else { return };
                st.graph.add(&kind);
                kind
            };
            changed(&weak, &state, true);
            browse_new_node(&weak, &state, &kind);
        });
    }
    install_canvas_callbacks(app, state);
    {
        let weak = app.as_weak();
        let state = state.clone();
        app.on_set_display(move |index| {
            {
                let mut st = state.lock().unwrap();
                if !st.inside.is_empty() {
                    return;
                }
                let Some(id) = st.graph.nodes.get(index.max(0) as usize).map(|n| n.id) else { return };
                st.graph.display = Some(id);
            }
            changed(&weak, &state, false);
        });
    }
    {
        let weak = app.as_weak();
        let state = state.clone();
        app.on_toggle_bypass(move |index| {
            {
                let mut st = state.lock().unwrap();
                if !st.inside.is_empty() {
                    return;
                }
                let Some(node) = st.graph.nodes.get_mut(index.max(0) as usize) else { return };
                node.bypass = !node.bypass;
            }
            changed(&weak, &state, false);
        });
    }
    {
        let weak = app.as_weak();
        let state = state.clone();
        app.on_delete_selection(move || delete_selection(&weak, &state));
    }
    {
        let weak = app.as_weak();
        let state = state.clone();
        app.on_connect_nodes(move |src, dst, slot| {
            let ids = {
                let st = state.lock().unwrap();
                let g = st.net();
                let id_at = |i: i32| g.nodes.get(i.max(0) as usize).map(|n| n.id);
                id_at(src).zip(id_at(dst))
            };
            if let Some((src, dst)) = ids {
                connect_nodes(&weak, &state, src, dst, slot.max(0) as usize);
            }
        });
    }
    {
        // 拖线松手：slot < 0 是从 node 的输出端口拖出来的，否则是从它的第 slot 个输入往回拖的
        let weak = app.as_weak();
        let state = state.clone();
        app.on_wire_dropped(move |node, slot, x, y| {
            let link = {
                let st = state.lock().unwrap();
                let g = st.net();
                let Some(id) = g.nodes.get(node.max(0) as usize).map(|n| n.id) else { return };
                match usize::try_from(slot) {
                    Err(_) => g.input_at(x, y).map(|(to, slot)| (id, to, slot)),
                    Ok(slot) => g.output_at(x, y).map(|from| (from, id, slot)),
                }
            };
            if let Some((from, to, slot)) = link {
                connect_nodes(&weak, &state, from, to, slot);
            }
        });
    }
    {
        // 点画布空白处：点中连线就选中它；没点中 = 取消多选（表达式网络里全不选：表格回到 expr 节点的结果）。
        // keep：右键（只看点没点中连线，不动选中的节点）
        let weak = app.as_weak();
        let state = state.clone();
        app.on_canvas_clicked(move |x, y, tolerance, keep| {
            let rerun = {
                let mut st = state.lock().unwrap();
                let g = st.net_mut();
                let before = (g.selected, g.picked.clone(), g.selected_wire);
                let hit = g.wire_at(x, y, tolerance);
                if hit.is_none() && !keep {
                    g.select_only(if g.vop { None } else { g.selected });
                }
                g.selected_wire = hit;
                if (g.selected, &g.picked, g.selected_wire) == (before.0, &before.1, before.2) {
                    return;
                }
                let changed = g.selected != before.0;
                let vop = g.vop;
                if let Some(app) = weak.upgrade() {
                    push_graph(&app, &st, changed);
                }
                vop && changed
            };
            if rerun {
                run_graph(&weak, &state);
            }
        });
    }
    {
        let weak = app.as_weak();
        let state = state.clone();
        app.on_disconnect_input(move |dst, slot| {
            {
                let mut st = state.lock().unwrap();
                let g = st.net_mut();
                let Some(id) = g.nodes.get(dst.max(0) as usize).map(|n| n.id) else { return };
                if !g.disconnect(id, slot.max(0) as usize) {
                    return;
                }
            }
            changed(&weak, &state, true);
        });
    }
    {
        let weak = app.as_weak();
        let state = state.clone();
        app.on_code_edited(move |index, text, cursor| {
            let Some(app) = weak.upgrade() else { return };
            let mut st = state.lock().unwrap();
            let (g, Some(id)) = st.param_graph() else { return };
            if let Some(value) = g.node_mut(id).and_then(|n| n.params.get_mut(index.max(0) as usize)) {
                *value = text.to_string();
            }
            clear_node_error(&mut st, id);
            push_graph(&app, &st, false);
            code_assist(&app, &mut st, &text, cursor.max(0) as usize);
        });
    }
    {
        // 选中第 pick 个补全：返回替换后的文本和光标位置
        let state = state.clone();
        app.on_code_accept(move |text, cursor, pick| {
            let st = state.lock().unwrap();
            let (text, cursor) = match st.completions.get(pick.max(0) as usize) {
                Some(item) => complete::replace(&text, st.completion_start, cursor.max(0) as usize, &item.label),
                None => (text.to_string(), cursor.max(0) as usize),
            };
            CodeEdit { text: SharedString::from(text), cursor: cursor as i32 }
        });
    }
    app.on_code_key(move |text, anchor, cursor, key| {
        let (text, cursor) = complete::press(&text, anchor.max(0) as usize, cursor.max(0) as usize, &key);
        CodeEdit { text: SharedString::from(text), cursor: cursor as i32 }
    });
    {
        let weak = app.as_weak();
        let state = state.clone();
        app.on_param_edited(move |index, text| {
            let mut st = state.lock().unwrap();
            let (g, Some(id)) = st.param_graph() else { return };
            if let Some(value) = g.node_mut(id).and_then(|n| n.params.get_mut(index.max(0) as usize)) {
                *value = text.to_string();
            }
            clear_node_error(&mut st, id);
            // 节点卡片上的摘要、代码预览跟着变；不重建参数编辑框
            if let Some(app) = weak.upgrade() {
                push_graph(&app, &st, false);
            }
        });
    }
    {
        let weak = app.as_weak();
        let state = state.clone();
        app.on_param_accepted(move |index| {
            let mut st = state.lock().unwrap();
            let (path, Some(id)) = st.param_target() else { return };
            let is_file = path.is_empty()
                && (st.graph.node(id).and_then(|n| n.kind().params.get(index.max(0) as usize)))
                    .is_some_and(|p| matches!(p.kind, ParamKind::File | ParamKind::SaveFile));
            if is_file {
                file_changed(&mut st, id);
            }
            drop(st);
            // 换了文件要重建参数框（格式页签、sheet 下拉的选项变了）：推迟到回调返回后，不在编辑框自己的回调里销毁它
            let (weak, state) = (weak.clone(), state.clone());
            let _ = slint::invoke_from_event_loop(move || graph_changed(&weak, &state, is_file));
        });
    }
    {
        let weak = app.as_weak();
        let state = state.clone();
        app.on_param_toggled(move |index, on| {
            {
                let mut st = state.lock().unwrap();
                let (g, Some(id)) = st.param_graph() else { return };
                match g.node_mut(id).and_then(|n| n.params.get_mut(index.max(0) as usize)) {
                    Some(value) => *value = String::from(if on { "True" } else { "False" }),
                    None => return,
                }
            }
            changed(&weak, &state, false);
        });
    }
    {
        let weak = app.as_weak();
        let state = state.clone();
        app.on_param_picked(move |index, option| {
            let index = index.max(0) as usize;
            let mut switch_to = None;
            let is_tabs;
            let rebuild;
            {
                let mut guard = state.lock().unwrap();
                let st = &mut *guard;
                let (path, Some(id)) = st.param_target() else { return };
                let in_net = !path.is_empty();
                let (g, _) = st.param_graph();
                let Some(node) = g.node_mut(id) else { return };
                let (Some(def), Some(value)) = (node.kind().params.get(index), node.params.get(index)) else {
                    return;
                };
                let (options, _) = param_options(node, def, value);
                let Some(mut picked) = options.get(option.max(0) as usize).cloned() else { return };
                // 类型的下拉选的是类型名，填进去的是它的写法（带参数的类型给个样子）
                if def.kind == ParamKind::Dtype {
                    picked = (nodes::DTYPES.iter()).find(|(name, _)| *name == picked).map_or(picked, |(_, t)| t.to_string());
                }
                let is_sheet = def.kind == ParamKind::Sheet;
                // “方式”换了，显示哪些参数也跟着变
                is_tabs = def.mode;
                // 类型：旁边的输入框换成新的写法
                rebuild = is_tabs || def.kind == ParamKind::Dtype;
                let is_context = node.kind.network && (def.name == "context" || def.name == "then");
                if is_sheet && option == 0 {
                    picked.clear();
                }
                node.params[index] = picked;
                // expr 节点换了上下文 / 分组后：里面缺的输出节点补上
                if is_context {
                    node.switch_context();
                }
                if !in_net {
                    let Some(node) = st.graph.nodes.iter_mut().find(|n| n.id == id) else { return };
                    // 源节点换 sheet：查看器跟着切（表格正显示别的节点时，只记下文件该停在哪个 sheet）
                    if is_sheet && node.primary && node.file_path() == Some(&st.graph.file) {
                        let picked = &node.params[index];
                        let target =
                            if picked.is_empty() { Some(0) } else { node.sheets.iter().position(|s| s == picked) };
                        match (&mut st.file_sheets, target) {
                            (Some((_, current)), Some(i)) => *current = i,
                            (None, Some(i)) => switch_to = Some(i),
                            _ => {}
                        }
                    }
                    if is_tabs {
                        let viewer: Vec<String> = st
                            .file_sheets
                            .as_ref()
                            .map_or(&st.sheets, |(sheets, _)| sheets)
                            .iter()
                            .map(|s| s.name.clone())
                            .collect();
                        reload_sheets(node, &st.graph.file, viewer);
                    }
                }
            }
            if let Some(i) = switch_to {
                switch_sheet(&weak, &state, i);
            }
            if rebuild {
                // 换页签要重建参数框：推迟到回调返回后，不在页签自己的回调里销毁它
                let (weak, state) = (weak.clone(), state.clone());
                let _ = slint::invoke_from_event_loop(move || graph_changed(&weak, &state, true));
            } else {
                changed(&weak, &state, false);
            }
        });
    }
    {
        let weak = app.as_weak();
        let state = state.clone();
        app.on_param_browse(move |index| browse_param(&weak, &state, index.max(0) as usize));
    }
    {
        // 参数面板里的按钮（write 节点的“写出”）
        let weak = app.as_weak();
        let state = state.clone();
        app.on_param_action(move |index| {
            if state.lock().unwrap().inside.is_empty() {
                run_action(&weak, &state, index.max(0) as usize);
            }
        });
    }
    {
        let weak = app.as_weak();
        let state = state.clone();
        app.on_run_graph(move || run_graph(&weak, &state));
    }
    {
        let weak = app.as_weak();
        let state = state.clone();
        app.on_rename_node(move |name| {
            let Some(app) = weak.upgrade() else { return };
            let mut st = state.lock().unwrap();
            let (g, Some(id)) = st.param_graph() else { return };
            if g.node(id).is_some_and(|n| n.title == name.trim()) {
                return;
            }
            match g.rename(id, &name) {
                Ok(()) => {
                    app.set_status(SharedString::from(format!("已改名为 {}", name.trim())));
                    undo::checkpoint(&mut st);
                }
                Err(e) => app.set_status(SharedString::from(e)),
            }
            // 输入框在自己的回调里，推迟到回调返回后再刷新（改名失败时框里换回原名）
            drop(st);
            let (weak, state) = (weak.clone(), state.clone());
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(app) = weak.upgrade() {
                    push_graph(&app, &state.lock().unwrap(), false);
                }
            });
        });
    }
    {
        let weak = app.as_weak();
        let state = state.clone();
        // 推迟到这次鼠标事件处理完：双击时松开鼠标的那一下（“节点挪到了哪”）还属于表格级的图
        app.on_enter_node(move |index| {
            let (weak, state) = (weak.clone(), state.clone());
            let _ = slint::invoke_from_event_loop(move || enter_node(&weak, &state, index));
        });
    }
    {
        let weak = app.as_weak();
        let state = state.clone();
        app.on_text_action({
            let weak = weak.clone();
            let state = state.clone();
            move || text_action(&weak, &state)
        });
        app.on_go_top({
            let weak = weak.clone();
            let state = state.clone();
            move || {
                let (weak, state) = (weak.clone(), state.clone());
                let _ = slint::invoke_from_event_loop(move || go_up_levels(&weak, &state, usize::MAX));
            }
        });
        app.on_pick_env({
            let weak = weak.clone();
            let state = state.clone();
            move |index| {
                {
                    let mut st = state.lock().unwrap();
                    let Some(&env) = st.top_node().and_then(|n| n.envs().get(index.max(0) as usize)) else { return };
                    st.env = env;
                }
                fit_after_switch(&weak, &state);
                graph_changed(&weak, &state, true);
            }
        });
        app.on_go_level({
            let weak = weak.clone();
            let state = state.clone();
            move |depth| {
                let (weak, state) = (weak.clone(), state.clone());
                let _ = slint::invoke_from_event_loop(move || {
                    let levels = state.lock().unwrap().inside.len().saturating_sub(depth.max(0) as usize);
                    if levels > 0 {
                        go_up_levels(&weak, &state, levels);
                    }
                });
            }
        });
        app.on_go_up(move || {
            let (weak, state) = (weak.clone(), state.clone());
            let _ = slint::invoke_from_event_loop(move || go_up(&weak, &state));
        });
    }
    {
        let weak = app.as_weak();
        let state = state.clone();
        // selected：只看选中的节点（G），没选中的时看全部（H）
        app.on_fit_view(move |width, height, selected| {
            let Some(app) = weak.upgrade() else { return };
            let st = state.lock().unwrap();
            let g = st.net();
            let picked = g.selection();
            let ids = (selected && !picked.is_empty()).then_some(&picked[..]);
            let Some((left, top, right, bottom)) = g.bounds_of(ids) else {
                app.set_zoom(1.0);
                app.set_pan_x(0.0);
                app.set_pan_y(0.0);
                return;
            };
            const MARGIN: f32 = 24.0;
            let zoom = ((width - 2.0 * MARGIN) / (right - left)).min((height - 2.0 * MARGIN) / (bottom - top)).clamp(0.3, 1.2);
            app.set_zoom(zoom);
            app.set_pan_x((width - (right - left) * zoom) / 2.0 - left * zoom);
            app.set_pan_y((height - (bottom - top) * zoom) / 2.0 - top * zoom);
        });
    }
    {
        let weak = app.as_weak();
        app.on_copy_text(move |text| {
            let Some(app) = weak.upgrade() else { return };
            app.set_status(SharedString::from(match clipboard::set_text(&text) {
                Ok(()) => format!("已复制 {} 行", text.lines().count()),
                Err(e) => format!("复制失败：{e}"),
            }));
        });
    }
    {
        let weak = app.as_weak();
        let state = state.clone();
        app.on_show_script(move || {
            if let Some(app) = weak.upgrade() {
                push_script(&app, &state.lock().unwrap());
            }
        });
    }
    {
        let weak = app.as_weak();
        let state = state.clone();
        app.on_show_plan(move || push_plan(&weak, &state));
    }
    {
        let weak = app.as_weak();
        let state = state.clone();
        app.on_save_script(move || {
            let Some(app) = weak.upgrade() else { return };
            let st = state.lock().unwrap();
            let name = st.graph.display.and_then(|id| st.graph.node(id)).map_or(String::from("节点图"), |n| n.title.clone());
            let text = app.get_script_text().to_string();
            drop(st);
            let Some(path) = rfd::FileDialog::new()
                .add_filter("Python 脚本", &["py"])
                .set_file_name(format!("{name}.py"))
                .set_title("存成 Python 脚本")
                .save_file()
            else {
                return;
            };
            app.set_status(SharedString::from(match std::fs::write(&path, text) {
                Ok(()) => format!("已存到 {}", path.display()),
                Err(e) => format!("存不了：{e}"),
            }));
        });
    }
    {
        let weak = app.as_weak();
        let state = state.clone();
        app.on_stop_graph(move || {
            engine::stop();
            let mut st = state.lock().unwrap();
            cancel_runs(&mut st);
            st.running = false;
            st.writing = false;
            if let Some(app) = weak.upgrade() {
                push_graph(&app, &st, false);
                update_busy(&app, &st);
            }
        });
    }
}

/// 画布上按着的节点：按下时鼠标在哪（画布坐标）、按着哪些修饰键；拖起来以后是一起挪的节点和它们原来的位置。
pub(crate) struct NodeDrag {
    id: u32,
    press: (f32, f32),
    shift: bool,
    ctrl: bool,
    alt: bool,
    started: bool,
    /// 一起挪的节点和它们原来的位置
    moving: Vec<(u32, f32, f32)>,
    /// Alt 拖：挪的是刚复制出来的那份
    copied: bool,
    /// 只拖着一个没连线的节点、压在这条线上：松手就插进去
    hot: Option<(u32, usize)>,
}

/// 这些节点的位置改到界面的节点模型里（拖动中只改这几行，不整个同步）。
fn push_positions(app: &AppWindow, g: &graph::Graph, ids: impl Iterator<Item = u32>) {
    let model = app.get_nodes();
    for id in ids {
        let (Some(i), Some(node)) = (g.index_of(id), g.node(id)) else { continue };
        if let Some(mut item) = model.row_data(i).filter(|item| item.x != node.x || item.y != node.y) {
            (item.x, item.y) = (node.x, node.y);
            model.set_row_data(i, item);
        }
    }
}

/// 画布上的鼠标和键盘操作（照 Houdini 的网络编辑器）：点选 / Shift 加选 / Ctrl 减选、拖动（Shift 连同上游、
/// Ctrl 连同下游、Alt 复制一份）、拖到线上插入、框选、Y 剪线，以及快捷键和右键菜单的命令。
fn install_canvas_callbacks(app: &AppWindow, state: &Arc<Mutex<App>>) {
    {
        // 按下节点：Shift 加选；Ctrl 先不动（没拖动就松手 = 减选）；否则没选中的只选它，已选中的成为当前节点（好一起拖）
        let weak = app.as_weak();
        let state = state.clone();
        app.on_node_press(move |index, shift, ctrl, alt, x, y| {
            let rerun = {
                let mut st = state.lock().unwrap();
                let g = st.net_mut();
                let Some(id) = g.nodes.get(index.max(0) as usize).map(|n| n.id) else { return };
                let before = g.selected;
                if shift || (!ctrl && g.is_selected(id)) {
                    g.select_add(id);
                } else if !ctrl {
                    g.select_only(Some(id));
                }
                g.selected_wire = None;
                let changed = g.selected != before;
                let vop = g.vop;
                let press = (x, y);
                st.node_drag =
                    Some(NodeDrag { id, press, shift, ctrl, alt, started: false, moving: Vec::new(), copied: false, hot: None });
                if let Some(app) = weak.upgrade() {
                    push_graph(&app, &st, changed);
                }
                // 表达式网络里换了选中的节点：表格换成它的值
                vop && changed
            };
            if rerun {
                run_graph(&weak, &state);
            }
        });
    }
    {
        // 拖着节点：(x, y) 是鼠标现在在画布上的位置
        let weak = app.as_weak();
        let state = state.clone();
        app.on_node_drag(move |x, y| {
            let Some(app) = weak.upgrade() else { return };
            let mut guard = state.lock().unwrap();
            let st = &mut *guard;
            let Some(mut drag) = st.node_drag.take() else { return };
            let g = st.net_mut();
            let mut resized = false;
            if !drag.started {
                drag.started = true;
                let mut ids = if g.is_selected(drag.id) { g.selection() } else { vec![drag.id] };
                if drag.shift {
                    ids = g.with_relatives(&ids, true);
                }
                if drag.ctrl {
                    ids = g.with_relatives(&ids, false);
                }
                if drag.alt {
                    ids = g.duplicate(&ids);
                    drag.copied = !ids.is_empty();
                    resized = drag.copied;
                }
                drag.moving = ids.iter().filter_map(|&id| g.node(id).map(|n| (id, n.x, n.y))).collect();
            }
            let (dx, dy) = (x - drag.press.0, y - drag.press.1);
            for &(id, x0, y0) in &drag.moving {
                if let Some(node) = g.node_mut(id) {
                    (node.x, node.y) = (x0 + dx, y0 + dy);
                }
            }
            let hot = match drag.moving[..] {
                [(id, ..)] => g.wire_under(id),
                _ => None,
            };
            let hot_changed = hot != drag.hot;
            drag.hot = hot;
            let moving: Vec<u32> = drag.moving.iter().map(|m| m.0).collect();
            st.node_drag = Some(drag);
            if resized {
                push_graph(&app, st, true);
                return;
            }
            push_positions(&app, st.net(), moving.into_iter());
            if hot_changed {
                app.set_wires(ModelRc::new(VecModel::from(wire_items(st))));
            }
        });
    }
    {
        // 松开节点：没拖动 = 点选（Ctrl 减选，普通的点只留它一个）；拖过了 = 位置定下来，压在线上的插进去
        let weak = app.as_weak();
        let state = state.clone();
        app.on_node_release(move || {
            let Some(app) = weak.upgrade() else { return };
            let rerun = {
                let mut st = state.lock().unwrap();
                let Some(drag) = st.node_drag.take() else { return };
                let g = st.net_mut();
                let vop = g.vop;
                if !drag.started {
                    let before = g.selected;
                    if drag.ctrl {
                        g.select_remove(drag.id);
                    } else if !drag.shift {
                        g.select_only(Some(drag.id));
                    }
                    let changed = g.selected != before;
                    push_graph(&app, &st, changed);
                    vop && changed
                } else {
                    let inserted = match (drag.hot, &drag.moving[..]) {
                        (Some(wire), [(id, ..)]) => g.insert_on_wire(*id, wire),
                        _ => false,
                    };
                    // 输出节点的先后按画布上的位置：挪了位置，结果的列顺序可能变了
                    let moved_output =
                        vop && drag.moving.iter().any(|(id, ..)| g.node(*id).is_some_and(|n| !n.kind.role.is_empty()));
                    // 挪完了：记一步撤销（不重新运行的时候 run_graph 不会记）
                    undo::checkpoint(&mut st);
                    push_graph(&app, &st, inserted || drag.copied);
                    inserted || moved_output || (vop && drag.copied)
                }
            };
            if rerun {
                run_graph(&weak, &state);
            }
        });
    }
    {
        // 框选（画布坐标的两个角）：mode 0 换成框里的 / 1 加上（Shift）/ 2 去掉（Ctrl）
        let weak = app.as_weak();
        let state = state.clone();
        app.on_box_select(move |x0, y0, x1, y1, mode| {
            let rerun = {
                let mut st = state.lock().unwrap();
                let g = st.net_mut();
                let before = g.selected;
                g.select_box((x0, y0, x1, y1), mode);
                let changed = g.selected != before;
                let vop = g.vop;
                if let Some(app) = weak.upgrade() {
                    push_graph(&app, &st, changed);
                }
                vop && changed
            };
            if rerun {
                run_graph(&weak, &state);
            }
        });
    }
    {
        // 按着 Y 拖：一路记下轨迹，松手时剪断和它相交的连线
        let state = state.clone();
        app.on_cut_point(move |x, y| state.lock().unwrap().cut.push((x, y)));
    }
    {
        let weak = app.as_weak();
        let state = state.clone();
        app.on_cut_end(move || {
            let cut = {
                let mut st = state.lock().unwrap();
                let line = std::mem::take(&mut st.cut);
                st.net_mut().cut_wires(&line)
            };
            if cut > 0 {
                if let Some(app) = weak.upgrade() {
                    app.set_status(SharedString::from(format!("剪断了 {cut} 条连线")));
                }
                graph_changed(&weak, &state, true);
            }
        });
    }
    {
        // 中键按住节点：它的信息（输出的行列、用时、输入、各列类型）
        let state = state.clone();
        app.on_node_info(move |index| {
            let st = state.lock().unwrap();
            let g = st.net();
            SharedString::from(g.nodes.get(index.max(0) as usize).map_or(String::new(), |n| node_info(&st, g, n)))
        });
    }
    {
        // 快捷键和右键菜单的命令；(x, y) 是鼠标在画布上的位置（粘贴到那里）
        let weak = app.as_weak();
        let state = state.clone();
        app.on_net_command(move |command, x, y| net_command(&weak, &state, &command, (x, y)));
    }
}

/// 快捷键和右键菜单的命令（command 见各分支）。at：鼠标在画布上的位置。
fn net_command(weak: &slint::Weak<AppWindow>, state: &Arc<Mutex<App>>, command: &str, at: (f32, f32)) {
    let Some(app) = weak.upgrade() else { return };
    if command == "delete" {
        delete_selection(weak, state);
        return;
    }
    // 做完以后：结构变了 = 同步界面并重新运行；只换了选中的 = 同步界面（表达式网络里预览跟着换）
    enum After {
        Nothing,
        Select,
        Changed(bool),
    }
    let mut st = state.lock().unwrap();
    let vop = st.net().vop;
    let selection = st.net().selection();
    let before = st.net().selected;
    let after = match command {
        // Q / B：旁路选中的节点（跟着当前节点切换，全部设成一样）
        "bypass" if !vop => {
            let on = !st.graph.selected.and_then(|id| st.graph.node(id)).is_some_and(|n| n.bypass);
            for &id in &selection {
                if let Some(node) = st.graph.node_mut(id) {
                    node.bypass = on;
                }
            }
            After::Changed(false)
        }
        // R：表格显示当前节点
        "display" if !vop => match st.graph.selected {
            Some(id) if st.graph.display != Some(id) => {
                st.graph.display = Some(id);
                After::Changed(false)
            }
            _ => After::Nothing,
        },
        // PgUp / PgDn：沿着链选上游 / 下游
        "up" | "down" => match st.net().walk(command == "down") {
            Some(id) => {
                st.net_mut().select_only(Some(id));
                After::Select
            }
            None => After::Nothing,
        },
        "select-all" => {
            st.net_mut().select_all();
            After::Select
        }
        "copy" | "cut" => {
            st.node_clip = st.net().copy_nodes(&selection).or(st.node_clip.take());
            let n = selection.len();
            app.set_status(SharedString::from(match (n, command) {
                (0, _) => String::from("没有选中的节点"),
                (_, "cut") => format!("已剪切 {n} 个节点 · Ctrl+V 粘贴"),
                _ => format!("已复制 {n} 个节点 · Ctrl+V 粘贴"),
            }));
            if command == "cut" && n > 0 {
                drop(st);
                delete_selection(weak, state);
                return;
            }
            After::Nothing
        }
        "paste" => match st.node_clip.clone() {
            Some(clip) if clip["vop"].as_bool() == Some(vop) => {
                let new = st.net_mut().paste(&clip, Some(at), false);
                app.set_status(SharedString::from(format!("已粘贴 {} 个节点", new.len())));
                After::Changed(true)
            }
            Some(_) => {
                app.set_status(SharedString::from(if vop {
                    "复制的是表格级的节点，不能贴进表达式网络"
                } else {
                    "复制的是表达式节点，只能贴进表达式网络"
                }));
                After::Nothing
            }
            None => After::Nothing,
        },
        // 复制一份：放在原来的右下方，外面接进来的线照旧接着
        "duplicate" if !selection.is_empty() => {
            let g = st.net_mut();
            for id in g.duplicate(&selection) {
                if let Some(node) = g.node_mut(id) {
                    (node.x, node.y) = (node.x + 40.0, node.y + 40.0);
                }
            }
            After::Changed(true)
        }
        // L：排列选中的（选了不止一个时），否则排列全部
        "layout" | "layout-all" => {
            let g = st.net_mut();
            let ids = if command == "layout" && selection.len() > 1 { selection } else { g.nodes.iter().map(|n| n.id).collect() };
            g.layout(&ids);
            // 表达式网络：输出节点的先后没变，还是重跑一次，提醒（用不到的输出…）跟着刷新
            if vop { After::Changed(false) } else { After::Select }
        }
        _ => After::Nothing,
    };
    match after {
        After::Nothing => {}
        After::Select => {
            // 表格级的排列（L）不重新运行：挪了位置在这里记一步撤销；只换了选中的不算
            undo::checkpoint(&mut st);
            let changed = st.net().selected != before;
            push_graph(&app, &st, changed);
            drop(st);
            if vop && changed {
                run_graph(weak, state);
            }
        }
        After::Changed(with_params) => {
            drop(st);
            graph_changed(weak, state, with_params);
        }
    }
}
