//! 节点类型编辑器（像 Houdini 的 Type Properties / Edit Parameter Interface）。
//!
//! 编辑的是一份草稿，点“应用 / 保存”才生效。四种用法：
//! - 改某个节点自己的参数界面（code 节点加参数，像 spare 参数）：结果是这个节点私有的类型
//! - 改注册表里的类型（内置 / 用户 / 工程）：存到用户目录，图里用它的节点跟着换
//! - 把 code 节点存成新的节点类型：节点随之变成新类型的实例
//! - 从空白新建一个节点类型
//!
//! 界面在 `ui/type-editor.slint`，状态都在全局 `TypeEdit` 上。

use crate::*;
use nodes::{ColInsert, PARAM_KINDS, ParamDef, Source};
use slint::ComponentHandle as _;

/// 在编辑什么。
#[derive(Clone)]
pub(crate) enum Target {
    /// 这个节点自己的参数界面
    Node(u32),
    /// 注册表里叫这个名字的类型
    Type(String),
    /// 把这个 code 节点存成新的节点类型
    SaveAs(u32),
    /// 从空白新建
    New,
}

pub(crate) struct Editor {
    target: Target,
    draft: NodeType,
    /// 草稿里有、但参数表里不列出来的参数（code 节点的代码参数）
    hidden: String,
    /// 参数表里选中的第几个（参数表的下标，不是草稿的）
    selected: Option<usize>,
    /// 左边“来自节点”：图里别的节点的参数
    sources: Vec<ParamDef>,
    /// 改节点自己的参数界面时，节点上写的代码（代码参数藏起来不在编辑器里改）
    node_code: String,
}

impl Editor {
    /// 参数表里列出的参数在草稿里的下标。
    fn shown(&self) -> Vec<usize> {
        (0..self.draft.params.len()).filter(|&i| self.draft.params[i].name != self.hidden).collect()
    }

    /// 拿来和参数对照的代码：code 节点看节点上写的，其余看草稿里的。
    fn code(&self) -> &str {
        if self.hidden.is_empty() { &self.draft.code } else { &self.node_code }
    }

    /// 代码里 ch("…") 用到的参数名。
    fn refs(&self) -> Vec<String> {
        nodes::referenced_params(self.code())
    }

    fn selected_param(&mut self) -> Option<&mut ParamDef> {
        let i = *self.shown().get(self.selected?)?;
        self.draft.params.get_mut(i)
    }

    /// 参数名重了就加数字：n → n_2 → n_3…
    fn unique_name(&self, base: &str) -> String {
        let taken = |name: &str| self.draft.params.iter().any(|p| p.name == name);
        if !taken(base) {
            return base.to_string();
        }
        (2..).map(|k| format!("{base}_{k}")).find(|name| !taken(name)).unwrap()
    }

    /// 插到参数表的第 at 个位置（草稿里放在对应的地方；放到最后时放在最后一个列出的参数后面）。
    fn insert(&mut self, at: usize, p: ParamDef) {
        let shown = self.shown();
        let pos = match shown.get(at) {
            Some(&i) => i,
            None => shown.last().map_or(0, |&i| i + 1),
        };
        self.draft.params.insert(pos, p);
        self.selected = Some(at.min(shown.len()));
    }
}

fn kind_index(kind: ParamKind) -> i32 {
    PARAM_KINDS.iter().position(|(k, ..)| *k == kind).unwrap_or(0) as i32
}

fn kind_label(kind: ParamKind) -> String {
    PARAM_KINDS.iter().find(|(k, ..)| *k == kind).map_or(String::new(), |(_, name, label)| format!("{label} · {name}"))
}

fn list(values: &str) -> Vec<String> {
    values.split(',').map(|v| v.trim().to_string()).filter(|v| !v.is_empty()).collect()
}

/// code 节点的代码用到了第几个输入：df → 1，df3 → 3。
fn inputs_used(code: &str) -> usize {
    let used = |name: &str| {
        code.match_indices(name).any(|(at, _)| {
            let before = code[..at].chars().next_back();
            let after = code[at + name.len()..].chars().next();
            let ident = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || c == '_');
            !ident(before) && !ident(after)
        })
    };
    ["df4", "df3", "df2", "df"].iter().position(|name| used(name)).map_or(1, |i| 4 - i)
}

// ---------------------------------------------------------------- 推到界面

fn push_all(app: &AppWindow, ed: &Editor) {
    let te = app.global::<TypeEdit>();
    let (title, note, apply, show_type) = match &ed.target {
        Target::Node(_) => (
            String::from("参数界面"),
            String::from("只改这个节点（像 Houdini 的 spare 参数）；要在别处复用，回参数面板点“存为类型”"),
            "应用",
            false,
        ),
        Target::Type(name) => {
            let note = match nodes::find(name).map(|t| t.source) {
                Some(Source::Builtin) => String::from("内置节点：保存后存到用户目录、盖住内置的（删掉用户的那份就恢复）"),
                Some(Source::Project) => String::from("这个类型来自工程文件；保存后存到用户目录，以后都能用"),
                _ => format!("存在 {}", nodes::user_dir().join(format!("{name}.py")).display()),
            };
            (format!("节点类型：{name}"), note, "保存", true)
        }
        Target::SaveAs(_) => (
            String::from("存为节点类型"),
            format!("存到 {}，之后在“添加”菜单和 Tab 搜索里都有", nodes::user_dir().display()),
            "保存",
            true,
        ),
        Target::New => (
            String::from("新建节点类型"),
            format!("存到 {}，之后在“添加”菜单和 Tab 搜索里都有", nodes::user_dir().display()),
            "保存",
            true,
        ),
    };
    let can_delete = matches!(&ed.target, Target::Type(name) if nodes::find(name).is_some_and(|t| t.source == Source::User));
    te.set_title(SharedString::from(title));
    te.set_note(SharedString::from(note));
    te.set_apply_label(SharedString::from(apply));
    te.set_show_type(show_type);
    te.set_show_code(show_type);
    // 新建 / 存为类型先填名字；改已有的先看参数
    te.set_tab(if matches!(ed.target, Target::SaveAs(_) | Target::New) { 0 } else { 1 });
    te.set_can_delete(can_delete);
    te.set_error(SharedString::new());
    te.set_message(SharedString::new());
    let d = &ed.draft;
    te.set_type_detail(ModelRc::new(VecModel::from(vec![EdType {
        name: SharedString::from(d.name.as_str()),
        category: SharedString::from(d.category.as_str()),
        desc: SharedString::from(d.desc.as_str()),
        inputs: d.inputs as i32,
        min_inputs: d.min_inputs as i32,
        code: SharedString::from(d.code.as_str()),
        help: SharedString::from(d.help.as_str()),
    }])));
    set_code_layers(app, &d.code);
    let kinds: Vec<SharedString> = PARAM_KINDS.iter().map(|(kind, ..)| SharedString::from(kind_label(*kind))).collect();
    te.set_kind_names(ModelRc::new(VecModel::from(kinds)));
    let sources: Vec<EdSource> = ed
        .sources
        .iter()
        .map(|p| EdSource {
            label: SharedString::from(p.label.as_str()),
            detail: SharedString::from(format!("{} · {}", p.name, kind_label(p.kind))),
        })
        .collect();
    te.set_sources(ModelRc::new(VecModel::from(sources)));
    push_params(app, ed);
    push_detail(app, ed);
    te.set_open(true);
}

fn set_code_layers(app: &AppWindow, code: &str) {
    let layers: Vec<SharedString> = complete::layers(code, false).iter().map(SharedString::from).collect();
    app.global::<TypeEdit>().set_code_layers(ModelRc::new(VecModel::from(layers)));
}

/// 参数没被代码用到（按钮不算：按钮是看 action，不用 ch 取）。
fn unused(p: &ParamDef, refs: &[String]) -> bool {
    p.kind != ParamKind::Button && !refs.contains(&p.name)
}

fn row(p: &ParamDef, refs: &[String]) -> EdParam {
    EdParam {
        name: SharedString::from(p.name.as_str()),
        label: SharedString::from(p.label.as_str()),
        kind: SharedString::from(kind_label(p.kind)),
        kind_index: kind_index(p.kind),
        unused: unused(p, refs),
        mode: p.mode,
        required: p.required,
        cond: SharedString::from(p.show.first().map_or(String::new(), |(name, values)| format!("{name} = {}", values.join(" / ")))),
    }
}

/// 参数表（整个换）。
fn push_params(app: &AppWindow, ed: &Editor) {
    let te = app.global::<TypeEdit>();
    let refs = ed.refs();
    let rows: Vec<EdParam> = ed.shown().iter().map(|&i| row(&ed.draft.params[i], &refs)).collect();
    te.set_params(ModelRc::new(VecModel::from(rows)));
    te.set_selected(ed.selected.map_or(-1, |s| s as i32));
    push_refs(app, ed);
}

/// 只改参数表的一行（正在属性里打字时：不整个换，列表不会跳）。
fn update_row(app: &AppWindow, ed: &Editor) {
    let te = app.global::<TypeEdit>();
    let (Some(at), Some(&i)) = (ed.selected, ed.selected.and_then(|s| ed.shown().get(s).copied()).as_ref()) else { return };
    let rows = te.get_params();
    if at < rows.row_count() {
        rows.set_row_data(at, row(&ed.draft.params[i], &ed.refs()));
    }
    push_refs(app, ed);
}

/// 代码和参数的对照：代码用到、还没有的参数；有、但代码没用到的参数。
struct Refs {
    /// 代码里用到的，按出现顺序：（名字, 有没有这个参数）
    used: Vec<(String, bool)>,
    /// 列出来的参数里代码没用到的
    unused: Vec<String>,
}

fn compare(ed: &Editor) -> Refs {
    let refs = ed.refs();
    let used = refs.iter().map(|name| (name.clone(), ed.draft.param(name).is_some())).collect();
    let unused =
        ed.shown().iter().map(|&i| &ed.draft.params[i]).filter(|p| unused(p, &refs)).map(|p| p.name.clone()).collect();
    Refs { used, unused }
}

/// 把对照推到界面：参数页底下的提示、代码页右边的清单、参数表每行的“代码没用到”。
fn push_refs(app: &AppWindow, ed: &Editor) {
    let te = app.global::<TypeEdit>();
    let r = compare(ed);
    let missing: Vec<String> = r.used.iter().filter(|(_, has)| !has).map(|(name, _)| format!("ch(\"{name}\")")).collect();
    te.set_missing(SharedString::from(missing.join("、")));
    te.set_unused(SharedString::from(r.unused.join("、")));
    let mut items: Vec<EdRef> = r
        .used
        .iter()
        .map(|(name, has)| EdRef { name: SharedString::from(name.as_str()), state: if *has { 0 } else { 1 } })
        .collect();
    items.extend(r.unused.iter().map(|name| EdRef { name: SharedString::from(name.as_str()), state: 2 }));
    te.set_code_refs(ModelRc::new(VecModel::from(items)));
    // 参数表的行：只改变了的（不整个换，列表不会跳）
    let rows = te.get_params();
    for (at, mut item) in rows.iter().enumerate() {
        let flag = r.unused.iter().any(|name| name == item.name.as_str());
        if item.unused != flag {
            item.unused = flag;
            rows.set_row_data(at, item);
        }
    }
}

/// 选中参数的属性（整个换：输入框重新取值）。
fn push_detail(app: &AppWindow, ed: &Editor) {
    let te = app.global::<TypeEdit>();
    let shown = ed.shown();
    let selected = ed.selected.and_then(|s| shown.get(s)).map(|&i| &ed.draft.params[i]);
    // 显示条件可以看的参数：别的列出来的参数
    let others: Vec<&ParamDef> = shown.iter().map(|&i| &ed.draft.params[i]).filter(|p| Some(*p) != selected).collect();
    let mut choices = vec![SharedString::from("（总是显示）")];
    choices.extend(others.iter().map(|p| SharedString::from(format!("{}（{}）", p.name, p.label))));
    te.set_show_choices(ModelRc::new(VecModel::from(choices)));
    let detail: Vec<EdDetail> = selected
        .map(|p| {
            let (show_param, show_values) = match p.show.first() {
                Some((name, values)) => {
                    let at = others.iter().position(|o| o.name == *name).map_or(0, |i| i + 1);
                    (at as i32, values.join(", "))
                }
                None => (0, String::new()),
            };
            EdDetail {
                name: SharedString::from(p.name.as_str()),
                label: SharedString::from(p.label.as_str()),
                kind: kind_index(p.kind),
                default: SharedString::from(p.default.as_str()),
                hint: SharedString::from(p.hint.as_str()),
                required: p.required,
                strip: p.strip,
                options: SharedString::from(p.options.join(", ")),
                option_list: ModelRc::new(VecModel::from(p.options.iter().map(SharedString::from).collect::<Vec<_>>())),
                mode: p.mode,
                show_param,
                show_values: SharedString::from(show_values),
                insert: match p.insert {
                    None => 0,
                    Some(ColInsert::Name) => 1,
                    Some(ColInsert::Expr) => 2,
                    Some(ColInsert::None) => 3,
                },
                is_menu: p.kind == ParamKind::Menu,
                is_text: p.kind == ParamKind::Text,
                pick: p.pick.map_or(0, |pick| nodes::PICKS.iter().position(|(k, ..)| *k == pick).map_or(0, |i| i as i32 + 1)),
                pick_from: SharedString::from((p.pick_from + 1).to_string()),
                pick_dtype: p.pick_dtype,
                is_dict: p.kind == ParamKind::Dict,
            }
        })
        .into_iter()
        .collect();
    te.set_detail(ModelRc::new(VecModel::from(detail)));
}

fn close(app: &AppWindow, st: &mut App) {
    st.editor = None;
    app.global::<TypeEdit>().set_open(false);
}

fn show_error(app: &AppWindow, msg: impl Into<String>) {
    app.global::<TypeEdit>().set_error(SharedString::from(msg.into()));
}

// ---------------------------------------------------------------- 打开

/// 打开编辑器。
pub(crate) fn open(app: &AppWindow, st: &mut App, target: Target) {
    let g = &st.graph;
    let node_of = |id: u32| g.node(id);
    let mut hidden = String::new();
    let mut node_code = String::new();
    let draft = match &target {
        Target::Node(id) => {
            let Some(node) = node_of(*id) else { return };
            hidden = node.kind.code_param.clone();
            node_code = node.raw(&hidden).to_string();
            let mut draft = (*node.kind).clone();
            draft.source = Source::Instance;
            draft
        }
        Target::Type(name) => {
            let Some(kind) = nodes::find(name) else { return };
            (*kind).clone()
        }
        Target::SaveAs(id) => {
            let Some(node) = node_of(*id) else { return };
            let code = node.raw(&node.kind.code_param).to_string();
            let mut draft = NodeType::new("");
            draft.params = node.kind.params.iter().filter(|p| p.name != node.kind.code_param).cloned().collect();
            draft.inputs = inputs_used(&code);
            draft.min_inputs = draft.inputs.min(1);
            // 参数的默认值换成这个节点现在填的
            for p in &mut draft.params {
                p.default = node.raw(&p.name).to_string();
            }
            draft.code = code;
            draft
        }
        Target::New => {
            let mut draft = NodeType::new("");
            draft.code = String::from("# df = 第 1 个输入；ch(\"名字\") 取参数\nout = df\n");
            draft
        }
    };
    // “来自节点”：图里其他节点的参数（不含代码参数、按钮）
    let editing = match &target {
        Target::Node(id) | Target::SaveAs(id) => Some(*id),
        _ => None,
    };
    let mut sources = Vec::new();
    for node in g.nodes.iter().filter(|n| Some(n.id) != editing) {
        for p in node.kind.params.iter().filter(|p| p.name != node.kind.code_param && p.kind != ParamKind::Button) {
            let mut p = p.clone();
            p.label = format!("{} · {}", node.title, p.label);
            sources.push(p);
        }
    }
    let ed = Editor { target, draft, hidden, selected: None, sources, node_code };
    let mut ed = ed;
    ed.selected = if ed.shown().is_empty() { None } else { Some(0) };
    push_all(app, &ed);
    st.editor = Some(ed);
}

/// 选中的节点：自己的参数界面（code 节点 / 已经有私有定义的）或它的类型。
fn edit_selected(app: &AppWindow, st: &mut App) {
    let Some(node) = st.graph.selected.and_then(|id| st.graph.node(id)) else {
        app.set_status(SharedString::from("先在节点图里选中一个节点"));
        return;
    };
    let target = if node.kind.source == Source::Instance || !node.kind.code_param.is_empty() {
        Target::Node(node.id)
    } else {
        Target::Type(node.kind.name.clone())
    };
    open(app, st, target);
}

/// 节点面板标题栏上的按钮：选中节点变了时更新。
pub(crate) fn push_entry(app: &AppWindow, node: Option<&graph::Node>) {
    let te = app.global::<TypeEdit>();
    let own = node.is_some_and(|n| n.kind.source == Source::Instance || !n.kind.code_param.is_empty());
    te.set_edit_label(SharedString::from(if own { "参数界面" } else { "类型属性" }));
    te.set_can_save(node.is_some_and(|n| !n.kind.code_param.is_empty()));
}

// ---------------------------------------------------------------- 应用

/// 注册表里的类型换了一份定义：图里用它的节点都换过去（改了名的，原名的节点也换）。
fn rebind_all(st: &mut App, old_name: &str, kind: &Arc<NodeType>) {
    for node in &mut st.graph.nodes {
        if node.kind.source != Source::Instance && (node.kind.name == old_name || node.kind.name == kind.name) {
            node.retype(kind.clone());
        }
    }
}

fn check_new_name(draft: &NodeType, old: Option<&str>) -> Result<(), String> {
    if draft.name.trim().is_empty() {
        return Err(String::from("给类型起个名字（字母、数字、下划线）"));
    }
    if !nodes::is_identifier(&draft.name) {
        return Err(format!("类型名“{}”不合适：只能用字母、数字、下划线，不能以数字开头", draft.name));
    }
    if Some(draft.name.as_str()) != old && nodes::find(&draft.name).is_some() {
        return Err(format!("已经有叫 {} 的节点类型了，换个名字", draft.name));
    }
    Ok(())
}

/// 点了“应用 / 保存”。成功返回 true（编辑器关掉、节点图要刷新）。
fn apply(app: &AppWindow, st: &mut App) -> bool {
    let Some((draft, target)) = st.editor.as_ref().map(|ed| (ed.draft.clone(), ed.target.clone())) else { return false };
    if let Err(e) = draft.check() {
        show_error(app, e);
        return false;
    }
    let result: Result<String, String> = match &target {
        Target::Node(id) => {
            let mut draft = draft;
            draft.source = Source::Instance;
            match st.graph.node_mut(*id) {
                Some(node) => {
                    node.retype(Arc::new(draft));
                    Ok(format!("已更新 {} 的参数界面", node.title))
                }
                None => Err(String::from("这个节点已经不在了")),
            }
        }
        Target::Type(old) => {
            let old = old.clone();
            check_new_name(&draft, Some(&old)).and_then(|_| {
                let was_user = nodes::find(&old).is_some_and(|t| t.source == Source::User);
                let kind = nodes::save_user_type(draft)?;
                if kind.name != old && was_user {
                    nodes::delete_user_type(&old)?;
                }
                rebind_all(st, &old, &kind);
                Ok(format!("已保存节点类型 {}", kind.name))
            })
        }
        Target::SaveAs(id) => {
            let id = *id;
            check_new_name(&draft, None).and_then(|_| {
                let kind = nodes::save_user_type(draft)?;
                if let Some(node) = st.graph.node_mut(id) {
                    node.retype(kind.clone());
                }
                Ok(format!("已存成节点类型 {}（分类“{}”）", kind.name, kind.category))
            })
        }
        Target::New => check_new_name(&draft, None).and_then(|_| {
            let kind = nodes::save_user_type(draft)?;
            Ok(format!("已新建节点类型 {}：在“添加”菜单的“{}”里，或按 Tab 搜", kind.name, kind.category))
        }),
    };
    match result {
        Ok(msg) => {
            close(app, st);
            push_kinds(app, st);
            app.set_status(SharedString::from(msg));
            true
        }
        Err(e) => {
            show_error(app, e);
            false
        }
    }
}

/// 改了一项。key：t.xxx 是类型本身，p.xxx 是选中的参数。返回要不要重建属性表（推迟做：控件还在自己的回调里）。
fn set_field(ed: &mut Editor, key: &str, value: &str) -> bool {
    let number = || value.trim().parse::<usize>().unwrap_or(0).min(nodes::MAX_INPUTS);
    match key {
        "t.name" => ed.draft.name = value.trim().to_string(),
        "t.category" => ed.draft.category = value.trim().to_string(),
        "t.desc" => ed.draft.desc = value.to_string(),
        "t.help" => ed.draft.help = value.to_string(),
        "t.inputs" => {
            ed.draft.inputs = number();
            ed.draft.min_inputs = ed.draft.min_inputs.min(ed.draft.inputs);
        }
        "t.min_inputs" => ed.draft.min_inputs = number().min(ed.draft.inputs),
        _ => {}
    }
    if !key.starts_with("p.") {
        return false;
    }
    // 显示条件可选的参数（和 push_detail 里的顺序一致）
    let shown = ed.shown();
    let me = ed.selected.and_then(|s| shown.get(s).copied());
    let others: Vec<String> =
        shown.iter().filter(|&&i| Some(i) != me).map(|&i| ed.draft.params[i].name.clone()).collect();
    let Some(p) = ed.selected_param() else { return false };
    let mut rebuild = false;
    match key {
        "p.name" => {
            let old = std::mem::replace(&mut p.name, value.trim().to_string());
            let new = p.name.clone();
            // 别的参数的显示条件里提到它的，跟着改名
            for other in &mut ed.draft.params {
                for (name, _) in &mut other.show {
                    if *name == old {
                        *name = new.clone();
                    }
                }
            }
            if ed.draft.code_param == old {
                ed.draft.code_param = new;
            }
        }
        "p.label" => p.label = value.to_string(),
        "p.default" => p.default = value.to_string(),
        "p.hint" => p.hint = value.to_string(),
        "p.options" => {
            p.options = list(value);
            if !p.options.contains(&p.default) {
                p.default = p.options.first().cloned().unwrap_or_default();
            }
        }
        "p.required" => p.required = value == "True",
        "p.strip" => p.strip = value == "True",
        "p.mode" => p.mode = value == "True",
        "p.kind" => {
            let kind = PARAM_KINDS.get(value.parse::<usize>().unwrap_or(0)).map_or(ParamKind::Text, |(k, ..)| *k);
            if kind != p.kind {
                p.kind = kind;
                if kind == ParamKind::Menu && p.options.is_empty() {
                    p.options = vec![String::from("a"), String::from("b")];
                }
                if kind == ParamKind::Menu && !p.options.contains(&p.default) {
                    p.default = p.options[0].clone();
                }
                if kind == ParamKind::Toggle && !matches!(p.default.as_str(), "True" | "False") {
                    p.default = String::from("False");
                }
                rebuild = true;
            }
        }
        "p.show_param" => {
            let at = value.parse::<usize>().unwrap_or(0);
            let values = p.show.first().map(|(_, v)| v.clone()).unwrap_or_default();
            p.show = match others.get(at.wrapping_sub(1)) {
                Some(name) if at > 0 => vec![(name.clone(), values)],
                _ => Vec::new(),
            };
            rebuild = true;
        }
        "p.show_values" => {
            if let Some((_, values)) = p.show.first_mut() {
                *values = list(value);
            }
        }
        "p.pick" => {
            p.pick = value.parse::<usize>().ok().and_then(|i| i.checked_sub(1)).and_then(|i| nodes::PICKS.get(i)).map(|(k, ..)| *k);
            rebuild = true; // 页签的高亮跟着换
        }
        "p.pick_from" => p.pick_from = value.trim().parse::<usize>().map_or(0, |n| n.saturating_sub(1)),
        "p.pick_dtype" => p.pick_dtype = value == "True",
        "p.insert" => {
            p.insert = match value {
                "1" => Some(ColInsert::Name),
                "2" => Some(ColInsert::Expr),
                "3" => Some(ColInsert::None),
                _ => None,
            };
            rebuild = true;
        }
        _ => {}
    }
    rebuild
}

// ---------------------------------------------------------------- 回调

pub(crate) fn install(app: &AppWindow, state: &Arc<Mutex<App>>) {
    let te = app.global::<TypeEdit>();
    // 回调里都是：锁住状态，改草稿，推界面
    let with = |weak: slint::Weak<AppWindow>, state: Arc<Mutex<App>>, f: fn(&AppWindow, &mut App)| {
        move || {
            if let Some(app) = weak.upgrade() {
                f(&app, &mut state.lock().unwrap());
            }
        }
    };
    te.on_edit_node(with(app.as_weak(), state.clone(), edit_selected));
    // 帮助页签的“预览”：按草稿打开帮助窗口
    te.on_preview_help(with(app.as_weak(), state.clone(), |app, st| {
        let Some(draft) = st.editor.as_ref().map(|ed| Arc::new(ed.draft.clone())) else { return };
        help::show(app, st, help::Page::of_kind(&draft), false);
    }));
    te.on_save_node(with(app.as_weak(), state.clone(), |app, st| {
        match st.graph.selected.and_then(|id| st.graph.node(id)).filter(|n| !n.kind.code_param.is_empty()) {
            Some(node) => {
                let id = node.id;
                open(app, st, Target::SaveAs(id));
            }
            None => app.set_status(SharedString::from("选中一个 code 节点，才能把它存成节点类型")),
        }
    }));
    te.on_new_type(with(app.as_weak(), state.clone(), |app, st| open(app, st, Target::New)));
    te.on_open_folder(|| {
        let dir = nodes::user_dir();
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::process::Command::new("explorer").arg(&dir).spawn();
    });
    {
        let weak = app.as_weak();
        let state = state.clone();
        te.on_reload_types(move || {
            {
                let mut guard = state.lock().unwrap();
                let st = &mut *guard;
                let broken = nodes::load_user_types();
                // 图里的节点换成重新读到的定义（被删掉的用户类型换回内置的；都没有了就留着原来那份）
                let names: Vec<String> = st.graph.nodes.iter().map(|n| n.kind.name.clone()).collect();
                for name in names {
                    if let Some(kind) = nodes::find(&name) {
                        st.graph.rebind(&kind);
                    }
                }
                if let Some(app) = weak.upgrade() {
                    push_kinds(&app, st);
                    app.set_status(SharedString::from(match broken.first() {
                        Some(first) => format!("有 {} 个节点定义读不了：{first}", broken.len()),
                        None => format!("已重新加载节点类型（共 {} 种）", st.kinds.len()),
                    }));
                }
            }
            graph_changed(&weak, &state, true);
        });
    }

    // ---- 参数表
    let edit = |weak: slint::Weak<AppWindow>, state: Arc<Mutex<App>>, f: fn(&mut Editor, usize, usize), detail: bool| {
        move |a: i32, b: i32| {
            let Some(app) = weak.upgrade() else { return };
            let mut st = state.lock().unwrap();
            let Some(ed) = st.editor.as_mut() else { return };
            f(ed, a.max(0) as usize, b.max(0) as usize);
            push_params(&app, ed);
            if detail {
                push_detail(&app, ed);
            }
        }
    };
    te.on_add(edit(
        app.as_weak(),
        state.clone(),
        |ed, kind, at| {
            let kind = PARAM_KINDS.get(kind).map_or(ParamKind::Text, |(k, ..)| *k);
            let name = ed.unique_name(&format!("{}1", kind.name()).replace("_file1", "1"));
            let mut p = ParamDef::new(&name, kind);
            if kind == ParamKind::Menu {
                p.options = vec![String::from("a"), String::from("b")];
                p.default = String::from("a");
            }
            ed.insert(at, p);
        },
        true,
    ));
    te.on_copy(edit(
        app.as_weak(),
        state.clone(),
        |ed, source, at| {
            let Some(mut p) = ed.sources.get(source).cloned() else { return };
            // 来源节点上的标签带着节点名，复制过来去掉；显示条件提到的参数这边不一定有，不带过来
            p.label = p.label.split(" · ").last().unwrap_or(&p.name).to_string();
            p.show.clear();
            p.name = ed.unique_name(&p.name);
            ed.insert(at, p);
        },
        true,
    ));
    te.on_move(edit(
        app.as_weak(),
        state.clone(),
        |ed, from, to| {
            let slots = ed.shown();
            if from >= slots.len() {
                return;
            }
            let mut order: Vec<ParamDef> = slots.iter().map(|&i| ed.draft.params[i].clone()).collect();
            let moved = order.remove(from);
            let to = if to > from { to - 1 } else { to }.min(order.len());
            order.insert(to, moved);
            // 排好的放回原来那些位置（藏起来的代码参数不动）
            for (slot, p) in slots.into_iter().zip(order) {
                ed.draft.params[slot] = p;
            }
            ed.selected = Some(to);
        },
        true,
    ));
    {
        let weak = app.as_weak();
        let state = state.clone();
        te.on_remove(move |at| {
            let Some(app) = weak.upgrade() else { return };
            let mut st = state.lock().unwrap();
            let Some(ed) = st.editor.as_mut() else { return };
            let Some(&i) = ed.shown().get(at.max(0) as usize) else { return };
            let removed = ed.draft.params.remove(i);
            // 别的参数的显示条件里提到它的，条件去掉
            for p in &mut ed.draft.params {
                p.show.retain(|(name, _)| *name != removed.name);
            }
            let count = ed.shown().len();
            ed.selected = ed.selected.filter(|_| count > 0).map(|s| s.min(count - 1));
            push_params(&app, ed);
            push_detail(&app, ed);
        });
    }
    {
        let weak = app.as_weak();
        let state = state.clone();
        te.on_select(move |at| {
            let Some(app) = weak.upgrade() else { return };
            let mut st = state.lock().unwrap();
            let Some(ed) = st.editor.as_mut() else { return };
            ed.selected = Some(at.max(0) as usize);
            app.global::<TypeEdit>().set_selected(at);
            push_detail(&app, ed);
        });
    }
    {
        let weak = app.as_weak();
        let state = state.clone();
        te.on_field(move |key, value| {
            let Some(app) = weak.upgrade() else { return };
            let rebuild = {
                let mut st = state.lock().unwrap();
                let Some(ed) = st.editor.as_mut() else { return };
                app.global::<TypeEdit>().set_error(SharedString::new());
                let rebuild = set_field(ed, &key, &value);
                update_row(&app, ed);
                rebuild
            };
            if rebuild {
                // 属性表要重建（类型变了 / 显示条件变了）：推迟到回调返回后，不在控件自己的回调里销毁它
                let (weak, state) = (weak.clone(), state.clone());
                let _ = slint::invoke_from_event_loop(move || {
                    if let (Some(app), Some(ed)) = (weak.upgrade(), state.lock().unwrap().editor.as_ref()) {
                        push_detail(&app, ed);
                    }
                });
            }
        });
    }
    {
        let weak = app.as_weak();
        let state = state.clone();
        te.on_scan(move || {
            let Some(app) = weak.upgrade() else { return };
            let mut guard = state.lock().unwrap();
            let st = &mut *guard;
            let Some(ed) = st.editor.as_mut() else { return };
            let missing: Vec<String> = ed.refs().into_iter().filter(|name| ed.draft.param(name).is_none()).collect();
            for name in &missing {
                // 表达式最通用：数字、"文字"、pl.col(...) 都能填；以后可以在右边改类型
                let at = ed.shown().len();
                ed.insert(at, ParamDef::new(name, ParamKind::Expr));
            }
            push_params(&app, ed);
            push_detail(&app, ed);
            app.global::<TypeEdit>().set_message(SharedString::from(match missing.len() {
                0 => String::from("代码里 ch(\"…\") 用到的参数都已经有了"),
                n => format!("加了 {n} 个参数：{}（类型先当表达式，可以在右边改）", missing.join("、")),
            }));
        });
    }
    {
        let weak = app.as_weak();
        let state = state.clone();
        te.on_code_edited(move |text, _cursor| {
            let Some(app) = weak.upgrade() else { return };
            let mut st = state.lock().unwrap();
            if let Some(ed) = st.editor.as_mut() {
                ed.draft.code = text.to_string();
                set_code_layers(&app, &text);
                push_refs(&app, ed);
            }
        });
    }
    te.on_code_key(|text, anchor, cursor, key| {
        let (text, cursor) = complete::press(&text, anchor.max(0) as usize, cursor.max(0) as usize, &key);
        CodeEdit { text: SharedString::from(text), cursor: cursor as i32 }
    });
    {
        let weak = app.as_weak();
        let state = state.clone();
        te.on_apply(move || {
            let applied = match weak.upgrade() {
                Some(app) => apply(&app, &mut state.lock().unwrap()),
                None => false,
            };
            if applied {
                graph_changed(&weak, &state, true);
            }
        });
    }
    te.on_cancel(with(app.as_weak(), state.clone(), close));
    {
        let weak = app.as_weak();
        let state = state.clone();
        te.on_delete_type(move || {
            {
                let Some(app) = weak.upgrade() else { return };
                let mut st = state.lock().unwrap();
                let Some(Target::Type(name)) = st.editor.as_ref().map(|ed| &ed.target) else { return };
                let name = name.clone();
                if let Err(e) = nodes::delete_user_type(&name) {
                    show_error(&app, e);
                    return;
                }
                // 有同名的内置节点：换回内置的；没有的话图里的节点留着原来那份定义
                if let Some(kind) = nodes::find(&name) {
                    st.graph.rebind(&kind);
                }
                close(&app, &mut st);
                push_kinds(&app, &mut st);
                app.set_status(SharedString::from(format!("已删除节点类型 {name}")));
            }
            graph_changed(&weak, &state, true);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn editor(params: &[(&str, ParamKind)], hidden: &str) -> Editor {
        let mut draft = NodeType::new("t");
        draft.params = params.iter().map(|(name, kind)| ParamDef::new(name, *kind)).collect();
        Editor {
            target: Target::New,
            draft,
            hidden: hidden.to_string(),
            selected: None,
            sources: Vec::new(),
            node_code: String::new(),
        }
    }

    fn names(ed: &Editor) -> Vec<&str> {
        ed.draft.params.iter().map(|p| p.name.as_str()).collect()
    }

    #[test]
    fn hidden_code_param_stays_out_of_the_list() {
        let mut ed = editor(&[("code", ParamKind::Code)], "code");
        assert!(ed.shown().is_empty());
        // 加到最后：放在代码参数前面（面板里参数在上、代码在下）
        ed.insert(0, ParamDef::new("n", ParamKind::Int));
        ed.insert(1, ParamDef::new("m", ParamKind::Int));
        assert_eq!(names(&ed), ["n", "m", "code"]);
        assert_eq!(ed.selected, Some(1));
        assert_eq!(ed.unique_name("n"), "n_2");
        assert_eq!(ed.unique_name("k"), "k");
    }

    #[test]
    fn fields_edit_the_selected_param() {
        let mut ed = editor(&[("mode", ParamKind::Menu), ("a", ParamKind::Text)], "");
        ed.draft.params[0].options = vec![String::from("x"), String::from("y")];
        ed.draft.params[1].show = vec![(String::from("mode"), vec![String::from("x")])];
        ed.selected = Some(0);
        // 改名：显示条件里提到它的跟着改
        set_field(&mut ed, "p.name", "how");
        assert_eq!(ed.draft.params[1].show[0].0, "how");
        set_field(&mut ed, "p.options", "p, q, r");
        assert_eq!((ed.draft.params[0].options.len(), ed.draft.params[0].default.as_str()), (3, "p"));
        // 类型改成开关：默认值变成 False；要重建属性表
        assert!(set_field(&mut ed, "p.kind", &kind_index(ParamKind::Toggle).to_string()));
        assert_eq!(ed.draft.params[0].default, "False");
        // 第二个参数：显示条件看第 1 个“别的参数”
        ed.selected = Some(1);
        assert!(set_field(&mut ed, "p.show_param", "0"));
        assert!(ed.draft.params[1].show.is_empty());
        set_field(&mut ed, "p.show_param", "1");
        set_field(&mut ed, "p.show_values", "True");
        assert_eq!(ed.draft.params[1].show, [(String::from("how"), vec![String::from("True")])]);
        set_field(&mut ed, "t.inputs", "3");
        set_field(&mut ed, "t.min_inputs", "9");
        assert_eq!((ed.draft.inputs, ed.draft.min_inputs), (3, 3));
        ed.draft.check().unwrap();
    }

    #[test]
    fn code_and_params_are_compared() {
        let mut ed = editor(&[("n", ParamKind::Int), ("go", ParamKind::Button), ("extra", ParamKind::Text)], "");
        ed.draft.code = String::from("out = df.head(ch(\"n\")).with_columns(ch(\"tz\"))");
        let r = compare(&ed);
        assert_eq!(r.used, [(String::from("n"), true), (String::from("tz"), false)]);
        // 按钮不用 ch 取，不算没用到
        assert_eq!(r.unused, ["extra"]);
        // code 节点：对照节点上写的代码，代码参数本身不算
        let mut ed = editor(&[("a", ParamKind::Expr), ("code", ParamKind::Code)], "code");
        ed.node_code = String::from("out = df.filter(ch(\"a\"))");
        let r = compare(&ed);
        assert_eq!((r.used, r.unused), (vec![(String::from("a"), true)], Vec::<String>::new()));
    }

    #[test]
    fn inputs_are_guessed_from_the_code() {
        assert_eq!(inputs_used("out = df.head()"), 1);
        assert_eq!(inputs_used("out = df.join(df3, on='k')"), 3);
        assert_eq!(inputs_used("out = pl.DataFrame({'a': [1]})"), 1);
        assert_eq!(inputs_used("my_df2 = 1\nout = df"), 1);
    }
}
