//! 节点的帮助文档（像 Houdini 的帮助卡）：中键点节点、F1、参数面板头上的“?”打开帮助窗口。
//!
//! 帮助写在定义文件开头的文档字符串里、JSON 的后面，是一种很小的标记（照 Houdini 的 wiki 写法）：
//! - 开头几段是概述；空行分段；`== 标题 ==` 是小标题；`* ` 开头是列表项；`{{{` 到 `}}}` 之间是代码
//! - `@parameters`：顶格写 `参数名:` 起一条，下面缩进的几行是它的说明
//! - `@inputs`：同样，`1:`、`2:`… 是第几个输入
//! - `@examples`：例子（写法同概述）
//! - `@related`：相关的节点类型名（逗号或空白分隔），帮助窗口里点了跳过去
//! - `@polars`：对应的 polars API（`DataFrame.unique`），帮助窗口向引擎要它的文档（英文，本机装的那版）；
//!   写成 `选项: DataFrame.xxx` 的只在节点的某个菜单（方式）选着这一项时用
//!
//! 参数一节总是列全节点的参数（标签、名字、类型、默认值、显示条件）：有说明的写说明，没有的用参数的 hint；
//! polars 文档里同名参数的说明附在后面。从 polars 生成的表达式节点没有自己写的帮助，只有 `@polars`。

use crate::*;
use serde_json::Value;
use std::sync::OnceLock;

#[derive(Clone, Debug, PartialEq)]
pub enum Block {
    Heading(String),
    Para(String),
    Bullet(String),
    Code(String),
}

#[derive(Debug, Default)]
pub struct Help {
    pub overview: Vec<Block>,
    pub params: Vec<(String, Vec<Block>)>,
    pub inputs: Vec<(String, Vec<Block>)>,
    pub examples: Vec<Block>,
    pub related: Vec<String>,
    /// (菜单选项，空 = 总是用, API)
    pub polars: Vec<(String, String)>,
}

fn is_code_fence(line: &str) -> bool {
    let t = line.trim();
    t == "{{{" || t == "}}}" || t.starts_with("```")
}

/// 解析帮助文字。
pub fn parse(text: &str) -> Help {
    // 先按 @节 切开（代码块里的 @ 不算）
    let mut sections: Vec<(String, Vec<&str>)> = vec![(String::new(), Vec::new())];
    let mut in_code = false;
    for line in text.lines() {
        if is_code_fence(line) {
            in_code = !in_code;
        }
        match line.strip_prefix('@').filter(|_| !in_code) {
            Some(name) => sections.push((name.trim().to_lowercase(), Vec::new())),
            None => sections.last_mut().unwrap().1.push(line),
        }
    }
    let mut help = Help::default();
    for (name, lines) in sections {
        match name.as_str() {
            "" => help.overview.extend(blocks(&lines)),
            "parameters" | "参数" => help.params.extend(entries(&lines)),
            "inputs" | "输入" => help.inputs.extend(entries(&lines)),
            "examples" | "例子" => help.examples.extend(blocks(&lines)),
            "related" | "相关" => {
                let words = lines.iter().flat_map(|l| l.split([',', '，', '、', ' ']));
                help.related.extend(words.map(str::trim).filter(|w| !w.is_empty()).map(String::from));
            }
            "polars" => {
                for line in lines.iter().map(|l| l.trim()).filter(|l| !l.is_empty()) {
                    let (key, api) = line.split_once([':', '：']).unwrap_or(("", line));
                    help.polars.push((key.trim().to_string(), api.trim().to_string()));
                }
            }
            // 不认识的节：当成概述里的一个小标题
            other => {
                help.overview.push(Block::Heading(other.to_string()));
                help.overview.extend(blocks(&lines));
            }
        }
    }
    help
}

/// `名字:` 顶格起一条，下面缩进的是说明。第一条之前的文字是这一节的开场白（名字为空）。
fn entries(lines: &[&str]) -> Vec<(String, Vec<Block>)> {
    let mut out: Vec<(String, Vec<&str>)> = Vec::new();
    for line in lines {
        let t = line.trim_end();
        let top = !line.starts_with([' ', '\t']);
        let key = t.strip_suffix(':').or_else(|| t.strip_suffix('：')).filter(|k| top && !k.trim().is_empty());
        match key {
            Some(key) => out.push((key.trim().to_string(), Vec::new())),
            None if out.is_empty() => out.push((String::new(), vec![line])),
            None => out.last_mut().unwrap().1.push(line),
        }
    }
    out.into_iter().map(|(key, lines)| (key, blocks(&lines))).filter(|(k, b)| !k.is_empty() || !b.is_empty()).collect()
}

/// 去掉共同的缩进。
fn dedent<'a>(lines: &[&'a str]) -> Vec<&'a str> {
    let indent = lines.iter().filter(|l| !l.trim().is_empty()).map(|l| l.len() - l.trim_start().len()).min().unwrap_or(0);
    lines.iter().map(|l| if l.len() >= indent { &l[indent..] } else { l.trim_start() }).collect()
}

fn is_cjk(c: char) -> bool {
    matches!(c as u32, 0x2E80..=0x9FFF | 0xF900..=0xFAFF | 0xFF00..=0xFFEF)
}

/// 段落里的几行接成一行：中文之间不加空格。
fn join_line(para: &mut String, line: &str) {
    let cjk = para.chars().last().is_some_and(is_cjk) || line.chars().next().is_some_and(is_cjk);
    if !para.is_empty() && !cjk {
        para.push(' ');
    }
    para.push_str(line);
}

fn blocks(lines: &[&str]) -> Vec<Block> {
    let lines = dedent(lines);
    let mut out = Vec::new();
    let mut para = String::new();
    let mut bullet = false;
    let mut code: Option<Vec<&str>> = None;
    let flush = |out: &mut Vec<Block>, para: &mut String, bullet: bool| {
        if !para.is_empty() {
            let text = std::mem::take(para);
            out.push(if bullet { Block::Bullet(text) } else { Block::Para(text) });
        }
    };
    for line in lines {
        if let Some(c) = &mut code {
            if is_code_fence(line) {
                out.push(Block::Code(dedent(c).join("\n").trim_end().to_string()));
                code = None;
            } else {
                c.push(line);
            }
            continue;
        }
        let t = line.trim();
        if is_code_fence(line) {
            flush(&mut out, &mut para, bullet);
            code = Some(Vec::new());
        } else if t.is_empty() {
            flush(&mut out, &mut para, bullet);
        } else if t.len() > 4 && t.starts_with("==") && t.ends_with("==") {
            flush(&mut out, &mut para, bullet);
            out.push(Block::Heading(t.trim_matches('=').trim().to_string()));
        } else if let Some(item) = t.strip_prefix("* ").or_else(|| t.strip_prefix("- ")) {
            flush(&mut out, &mut para, bullet);
            bullet = true;
            para.push_str(item.trim());
        } else {
            if para.is_empty() {
                bullet = false;
            }
            join_line(&mut para, t);
        }
    }
    if let Some(c) = code {
        out.push(Block::Code(dedent(&c).join("\n").trim_end().to_string()));
    }
    flush(&mut out, &mut para, bullet);
    out
}

// ---------------------------------------------------------------- polars 的文档（向引擎要，存起来）

enum Doc {
    Loading,
    Ready(Value),
    Failed(String),
}

static DOCS: OnceLock<Mutex<HashMap<String, Doc>>> = OnceLock::new();

fn docs() -> &'static Mutex<HashMap<String, Doc>> {
    DOCS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 还没要过（或上次没要到）的 API：在后台向引擎要，要到了调 done（在事件循环里）。
fn fetch(apis: &[String], done: impl FnOnce() + Send + 'static) {
    let missing: Vec<String> = {
        let mut map = docs().lock().unwrap();
        let missing: Vec<String> =
            apis.iter().filter(|a| matches!(map.get(*a), None | Some(Doc::Failed(_)))).cloned().collect();
        for api in &missing {
            map.insert(api.clone(), Doc::Loading);
        }
        missing
    };
    if missing.is_empty() {
        return;
    }
    std::thread::spawn(move || {
        let result = engine::doc(&missing);
        {
            let mut map = docs().lock().unwrap();
            for api in &missing {
                let doc = match &result {
                    Ok(found) => match &found[api.as_str()] {
                        Value::Object(o) if o.contains_key("error") => Doc::Failed(o["error"].as_str().unwrap_or("").to_string()),
                        Value::Object(_) => Doc::Ready(found[api.as_str()].clone()),
                        _ => Doc::Failed(String::from("引擎没有回这一项")),
                    },
                    Err(e) => Doc::Failed(e.clone()),
                };
                map.insert(api.clone(), doc);
            }
        }
        let _ = slint::invoke_from_event_loop(done);
    });
}

// ---------------------------------------------------------------- 一页帮助

/// 帮助窗口里的一页：某个节点类型；从节点打开时带着这个节点的情况和它参数现在的值。
#[derive(Clone)]
pub struct Page {
    pub kind: Arc<NodeType>,
    /// 这个节点的情况（输出的行列、输入、各列类型），从类型打开时为空
    pub info: String,
    /// 参数的值（和 kind.params 一一对应；从类型打开时是默认值）：`@polars` 里带选项的条目按显示着的菜单挑
    pub values: Vec<String>,
}

impl Page {
    pub fn of_kind(kind: &Arc<NodeType>) -> Page {
        Page { kind: kind.clone(), info: String::new(), values: kind.params.iter().map(|p| p.default.clone()).collect() }
    }

    pub fn of_node(node: &graph::Node, info: String) -> Page {
        Page { kind: node.kind.clone(), info, values: node.params.clone() }
    }

    /// 从 Tab 菜单里搜到的某个方式打开：“方式”换成它。
    pub fn with_mode(mut self, option: &str) -> Page {
        if let Some(i) = self.kind.mode_param() {
            self.values[i] = option.to_string();
        }
        self
    }

    /// 显示着的菜单现在的值（显示条件同 graph::Node::visible）。
    fn menus(&self) -> Vec<&str> {
        let params = &self.kind.params;
        let shown = |p: &nodes::ParamDef| {
            p.show.iter().all(|(other, allowed)| self.kind.param(other).is_none_or(|j| allowed.contains(&self.values[j])))
        };
        (params.iter().zip(&self.values)).filter(|(p, _)| p.kind == ParamKind::Menu && shown(p)).map(|(_, v)| v.as_str()).collect()
    }

    /// 这一页要看的 polars API。
    pub fn apis(&self) -> Vec<String> {
        let help = parse(&self.kind.help);
        let menus = self.menus();
        let mut out: Vec<String> = Vec::new();
        for (key, api) in help.polars {
            if (key.is_empty() || menus.contains(&key.as_str())) && !out.contains(&api) {
                out.push(api);
            }
        }
        out
    }
}

// 帮助窗口里一行的种类（ui/theme.slint 的 HelpBlock）
const PARA: i32 = 0;
const SECTION: i32 = 1;
const HEADING: i32 = 2;
const BULLET: i32 = 3;
const CODE: i32 = 4;
const ENTRY: i32 = 5;
const LINK: i32 = 6;
const INFO: i32 = 7;
const NOTE: i32 = 8;

fn row(kind: i32, text: &str, label: &str, indent: bool) -> HelpBlock {
    HelpBlock { kind, text: SharedString::from(text), label: SharedString::from(label), indent }
}

fn push_blocks(out: &mut Vec<HelpBlock>, blocks: &[Block], indent: bool) {
    for b in blocks {
        out.push(match b {
            Block::Heading(t) => row(HEADING, t, "", indent),
            Block::Para(t) => row(PARA, t, "", indent),
            Block::Bullet(t) => row(BULLET, t, "", indent),
            Block::Code(t) => row(CODE, t, "", indent),
        });
    }
}

/// 参数的类型说明（参数界面编辑器里的叫法）。
fn kind_label(kind: ParamKind) -> &'static str {
    nodes::PARAM_KINDS.iter().find(|(k, ..)| *k == kind).map_or("", |(_, _, label)| label)
}

/// 参数条目的第二部分：名字 · 类型 · 默认值 · 必填 · 端口 · 显示条件。
fn param_line(t: &NodeType, p: &nodes::ParamDef) -> String {
    let mut parts: Vec<String> = Vec::new();
    if p.label != p.name {
        parts.push(p.name.clone());
    }
    parts.push(String::from(kind_label(p.kind)));
    if p.port {
        parts.push(String::from("也是输入端口"));
    }
    if p.required {
        parts.push(String::from("必填"));
    }
    if !p.default.is_empty() && p.kind != ParamKind::Button && !p.default.contains('\n') {
        parts.push(format!("默认 {}", p.default));
    }
    for (other, values) in &p.show {
        let label = t.param(other).map_or(other.as_str(), |i| t.params[i].label.as_str());
        parts.push(format!("{label} 是 {} 时才有", values.join(" / ")));
    }
    parts.join(" · ")
}

/// 节点的参数是不是 polars 文档里的那个参数（`*more_exprs` 生成了端口 more_exprs1 ~ 3）。
fn same_param(param: &str, polars: &str) -> bool {
    let polars = polars.trim_start_matches('*');
    param.strip_prefix(polars).is_some_and(|rest| rest.chars().all(|c| c.is_ascii_digit()))
}

/// polars 文档里参数的说明：[[名字, 说明]]。
fn polars_params(doc: &Value) -> Vec<(String, String)> {
    let items = doc["params"].as_array().map(Vec::as_slice).unwrap_or_default();
    items
        .iter()
        .filter_map(|p| Some((p[0].as_str()?.to_string(), p[1].as_str().unwrap_or("").to_string())))
        .collect()
}

/// 一页帮助 → 帮助窗口里的一行行。
pub fn render(page: &Page) -> Vec<HelpBlock> {
    let t = &page.kind;
    let help = parse(&t.help);
    let apis = page.apis();
    let map = docs().lock().unwrap();
    let ready: Vec<&Value> = apis.iter().filter_map(|a| match map.get(a) {
        Some(Doc::Ready(v)) => Some(v),
        _ => None,
    }).collect();
    // polars 文档里的参数说明，按名字找（节点参数和 polars 同名的才对得上）
    let polars_desc = |name: &str| -> Option<String> {
        ready.iter().flat_map(|d| polars_params(d)).find(|(n, _)| same_param(name, n)).map(|(_, d)| d)
    };
    let mut out: Vec<HelpBlock> = Vec::new();
    if !page.info.is_empty() {
        out.push(row(INFO, &page.info, "", false));
    }
    push_blocks(&mut out, &help.overview, false);
    if help.overview.is_empty() && !t.desc.is_empty() && apis.is_empty() {
        out.push(row(NOTE, "这个节点类型还没有写帮助（类型属性 → 帮助 页签里可以写）", "", false));
    }

    // 输入：表格级节点是 DataFrame（df ~ df4）；表达式节点的输入就是标了端口的参数，在参数一节里说
    if (!t.expr && t.inputs > 0) || !help.inputs.is_empty() {
        out.push(row(SECTION, "输入", "", false));
        let written = |i: usize| help.inputs.iter().find(|(k, _)| k.parse::<usize>().ok() == Some(i + 1));
        for (_, blocks) in help.inputs.iter().filter(|(k, _)| k.is_empty()) {
            push_blocks(&mut out, blocks, false);
        }
        for i in 0..t.inputs.max(help.inputs.iter().filter_map(|(k, _)| k.parse::<usize>().ok()).max().unwrap_or(0)) {
            let var = if i == 0 { String::from("df") } else { format!("df{}", i + 1) };
            let need = if i < t.min_inputs { "必须连" } else { "可以不连" };
            out.push(row(ENTRY, &format!("{need} · 代码里是 {var}"), &format!("输入 {}", i + 1), false));
            if let Some((_, blocks)) = written(i) {
                push_blocks(&mut out, blocks, true);
            }
        }
        // 写成名字的（左表 / 右表…）
        for (key, blocks) in help.inputs.iter().filter(|(k, _)| !k.is_empty() && k.parse::<usize>().is_err()) {
            out.push(row(ENTRY, "", key, false));
            push_blocks(&mut out, blocks, true);
        }
    }

    // 参数：全部列出来（code 节点的代码参数也是）
    if !t.params.is_empty() {
        out.push(row(SECTION, "参数", "", false));
        for (_, blocks) in help.params.iter().filter(|(k, _)| k.is_empty()) {
            push_blocks(&mut out, blocks, false);
        }
        for p in &t.params {
            out.push(row(ENTRY, &param_line(t, p), &p.label, false));
            // 先按名字找，再按标签找（几个参数的标签可以一样，如 slice 的三个 n）
            let written = (help.params.iter().find(|(k, _)| *k == p.name))
                .or_else(|| help.params.iter().find(|(k, _)| *k == p.label));
            match written {
                Some((_, blocks)) => push_blocks(&mut out, blocks, true),
                None if !p.hint.is_empty() => out.push(row(NOTE, &p.hint, "", true)),
                None => {}
            }
            if let Some(desc) = polars_desc(&p.name).filter(|d| !d.is_empty()) {
                out.push(row(NOTE, &format!("polars：{desc}"), "", true));
            }
            if p.kind == ParamKind::Menu && written.is_none() {
                let options: Vec<&str> = p.options.iter().map(|o| if o.is_empty() { "（不填）" } else { o.as_str() }).collect();
                out.push(row(NOTE, &format!("选项：{}", options.join(" / ")), "", true));
            }
            if p.kind == ParamKind::Dtype && written.is_none() {
                let names: Vec<&str> = nodes::DTYPES.iter().map(|(name, _)| *name).collect();
                out.push(row(NOTE, &format!("polars 的数据类型：{}", names.join(" / ")), "", true));
            }
        }
    }

    if !help.examples.is_empty() {
        out.push(row(SECTION, "例子", "", false));
        push_blocks(&mut out, &help.examples, false);
    }

    // polars 的文档：签名、说明、节点里没有的参数、例子
    for api in &apis {
        out.push(row(SECTION, &format!("polars 文档 · {api}"), "", false));
        match map.get(api) {
            Some(Doc::Ready(doc)) => {
                if let Some(sig) = doc["sig"].as_str().filter(|s| !s.is_empty()) {
                    out.push(row(CODE, sig, "", false));
                }
                let summary = doc["summary"].as_array().map(Vec::as_slice).unwrap_or_default();
                for para in summary.iter().filter_map(|p| p.as_str()) {
                    out.push(row(PARA, para, "", false));
                }
                let others: Vec<(String, String)> = polars_params(doc)
                    .into_iter()
                    .filter(|(name, _)| !t.params.iter().any(|p| same_param(&p.name, name)))
                    .collect();
                if !others.is_empty() {
                    out.push(row(HEADING, "其他参数", "", false));
                    for (name, desc) in others {
                        out.push(row(ENTRY, "", &name, false));
                        if !desc.is_empty() {
                            out.push(row(NOTE, &desc, "", true));
                        }
                    }
                }
                let sections = doc["sections"].as_array().map(Vec::as_slice).unwrap_or_default();
                for s in sections {
                    let (Some(title), Some(text)) = (s[0].as_str(), s[1].as_str()) else { continue };
                    out.push(row(HEADING, title, "", false));
                    out.push(row(if title == "Examples" { CODE } else { PARA }, text, "", false));
                }
            }
            Some(Doc::Failed(e)) => out.push(row(NOTE, &format!("读不到 polars 的文档：{e}"), "", false)),
            _ => out.push(row(NOTE, "正在向引擎要 polars 的文档…", "", false)),
        }
    }

    // 相关节点：同一级的优先
    let related: Vec<Arc<NodeType>> = help
        .related
        .iter()
        .filter_map(|name| if t.expr { nodes::find_expr(name).or_else(|| nodes::find(name)) } else { nodes::find(name).or_else(|| nodes::find_expr(name)) })
        .collect();
    if !related.is_empty() {
        out.push(row(SECTION, "相关节点", "", false));
        for k in related {
            out.push(row(LINK, &k.desc, &k.name, false));
        }
    }
    out
}

/// 帮助窗口头上的标签：用户 / 工程 / 已改内置 / 私有参数；生成的写“polars 生成”。
fn badge(t: &NodeType) -> &'static str {
    match nodes::source_label(t, false) {
        "" if t.expr && t.order >= 1000 && t.source == nodes::Source::Builtin => "polars 生成",
        label => label,
    }
}

/// 换帮助窗口里的内容：改原来那个模型（整个换掉模型时，滚动区域的高度不跟着重算，滚不到底）。
fn set_blocks(app: &AppWindow, rows: Vec<HelpBlock>) {
    let model = app.get_help_blocks();
    match model.as_any().downcast_ref::<VecModel<HelpBlock>>() {
        Some(vec) => vec.set_vec(rows),
        None => app.set_help_blocks(ModelRc::new(VecModel::from(rows))),
    }
}

/// 把这一页推到帮助窗口（并打开它）。back：把当前页记进“后退”（窗口关着时从头记）。
pub(crate) fn show(app: &AppWindow, st: &mut App, page: Page, back: bool) {
    let current = st.help.take();
    if !app.get_help_visible() {
        st.help_back.clear();
    } else if let Some(current) = current.filter(|_| back) {
        st.help_back.push(current);
    }
    let t = page.kind.clone();
    app.set_help_title(SharedString::from(t.name.as_str()));
    app.set_help_desc(SharedString::from(t.desc.as_str()));
    let category = if t.expr { format!("表达式节点 · {}", t.category) } else { format!("表格节点 · {}", t.category) };
    app.set_help_category(SharedString::from(category));
    app.set_help_cat(if t.expr { 3 } else { nodes::category_index(&t.category) as i32 });
    app.set_help_badge(SharedString::from(badge(&t)));
    set_blocks(app, render(&page));
    app.set_help_can_back(!st.help_back.is_empty());
    app.set_help_page(app.get_help_page() + 1);
    app.set_help_visible(true);
    let apis = page.apis();
    st.help = Some(page);
    // polars 的文档到了：还看着同一页就重画（不换滚动位置）
    let weak = app.as_weak();
    let shown = t.clone();
    let state = STATE.get().cloned();
    fetch(&apis, move || {
        let (Some(app), Some(state)) = (weak.upgrade(), state) else { return };
        let st = state.lock().unwrap();
        if let Some(page) = st.help.as_ref().filter(|p| Arc::ptr_eq(&p.kind, &shown)) {
            set_blocks(&app, render(page));
        }
    });
}

/// 后台线程回来时找状态用（帮助窗口只有一个）
static STATE: OnceLock<Arc<Mutex<App>>> = OnceLock::new();

/// 节点 → 一页帮助（带着它的情况）。
fn node_page(st: &App, g: &graph::Graph, id: u32) -> Option<Page> {
    let node = g.node(id)?;
    Some(Page::of_node(node, node_info(st, g, node)))
}

pub(crate) fn install(app: &AppWindow, state: &Arc<Mutex<App>>) {
    let _ = STATE.set(state.clone());
    {
        // F1 / 参数面板右上角的“帮助” / 右键菜单：参数面板正编辑的节点
        let weak = app.as_weak();
        let state = state.clone();
        app.on_selected_help(move || {
            let Some(app) = weak.upgrade() else { return };
            let mut st = state.lock().unwrap();
            let (path, id) = st.param_target();
            let page = id.zip(st.graph_at(&path)).and_then(|(id, g)| node_page(&st, g, id));
            if let Some(page) = page {
                show(&app, &mut st, page, true);
            }
        });
    }
    {
        // 中键点了菜单里的节点种类：0 Tab 搜索结果 / 1 “添加”菜单 / 2 工具架
        let weak = app.as_weak();
        let state = state.clone();
        app.on_kind_help(move |source, index| {
            let Some(app) = weak.upgrade() else { return };
            let mut st = state.lock().unwrap();
            let index = index.max(0) as usize;
            let page = match source {
                0 => {
                    // Tab 菜单的第一项可能是“按文字建节点”
                    let index = if st.search_text.is_some() { index.checked_sub(1) } else { Some(index) };
                    index.and_then(|i| st.search_hits.get(i)).map(|hit| match &hit.option {
                        Some(option) => Page::of_kind(&hit.kind).with_mode(option),
                        None => Page::of_kind(&hit.kind),
                    })
                }
                1 => st.kinds.get(index).map(Page::of_kind),
                _ => st.shelf_kinds.get(index).map(Page::of_kind),
            };
            if let Some(page) = page {
                show(&app, &mut st, page, true);
            }
        });
    }
    {
        // 点了“相关节点”
        let weak = app.as_weak();
        let state = state.clone();
        app.on_help_link(move |name| {
            let Some(app) = weak.upgrade() else { return };
            let mut st = state.lock().unwrap();
            let expr = st.help.as_ref().is_some_and(|p| p.kind.expr);
            let found = if expr { nodes::find_expr(&name).or_else(|| nodes::find(&name)) } else { nodes::find(&name).or_else(|| nodes::find_expr(&name)) };
            if let Some(kind) = found {
                show(&app, &mut st, Page::of_kind(&kind), true);
            }
        });
    }
    {
        let weak = app.as_weak();
        let state = state.clone();
        app.on_help_back(move || {
            let Some(app) = weak.upgrade() else { return };
            let mut st = state.lock().unwrap();
            if let Some(page) = st.help_back.pop() {
                show(&app, &mut st, page, false);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markup_is_parsed() {
        let text = "去重：同样的行只留一行。\n第二行接上。\n\n== 小标题 ==\n* 一项\n  接着写\n* two\n{{{\n  df.unique()\n@not_a_section\n}}}\n\n@parameters\n开场白\nsubset:\n    按哪些列\n\n    第二段\nkeep：\n    留哪一行\n@inputs\n1:\n    要去重的表\n@related\nselect, drop_nulls、slice\n@polars\nDataFrame.unique\nis_duplicated: DataFrame.is_duplicated\n";
        let h = parse(text);
        assert_eq!(
            h.overview,
            vec![
                Block::Para(String::from("去重：同样的行只留一行。第二行接上。")),
                Block::Heading(String::from("小标题")),
                Block::Bullet(String::from("一项接着写")),
                Block::Bullet(String::from("two")),
                Block::Code(String::from("  df.unique()\n@not_a_section")),
            ]
        );
        assert_eq!(h.params.len(), 3);
        assert_eq!(h.params[0], (String::new(), vec![Block::Para(String::from("开场白"))]));
        assert_eq!(h.params[1].0, "subset");
        assert_eq!(h.params[1].1, vec![Block::Para(String::from("按哪些列")), Block::Para(String::from("第二段"))]);
        assert_eq!(h.params[2].0, "keep");
        assert_eq!(h.inputs, vec![(String::from("1"), vec![Block::Para(String::from("要去重的表"))])]);
        assert_eq!(h.related, ["select", "drop_nulls", "slice"]);
        assert_eq!(h.polars, [(String::new(), String::from("DataFrame.unique")), (String::from("is_duplicated"), String::from("DataFrame.is_duplicated"))]);
        // 英文行之间加空格
        assert_eq!(blocks(&["a b", "c"]), vec![Block::Para(String::from("a b c"))]);
    }

    #[test]
    fn polars_entries_follow_the_menus() {
        let kind = nodes::find("unique").unwrap();
        let mut page = Page::of_kind(&kind);
        assert_eq!(page.apis(), ["DataFrame.unique"]);
        page = page.with_mode("is_duplicated");
        assert_eq!(page.apis(), ["DataFrame.is_duplicated"]);
        // 只看显示着的菜单：expr 节点的“分组后”只在 group_by 时算数
        let expr = nodes::find("expr").unwrap();
        assert_eq!(Page::of_kind(&expr).apis(), ["DataFrame.with_columns"]);
        assert_eq!(Page::of_kind(&expr).with_mode("group_by").apis(), ["DataFrame.group_by", "GroupBy.agg"]);
        // 生成的表达式节点：只有 polars 的文档
        let generated = nodes::find_expr("str.pad_start").unwrap();
        assert_eq!(Page::of_kind(&generated).apis(), ["Expr.str.pad_start"]);
    }

    #[test]
    fn builtin_help_is_sound() {
        // 只看内置的：别的测试会临时注册工程 / 用户层的类型（并行跑时这里看得见）
        let curated = (nodes::all().into_iter().chain(nodes::all_expr()))
            .filter(|t| t.order < 1000 && t.source == nodes::Source::Builtin);
        for t in curated {
            let h = parse(&t.help);
            let name = &t.name;
            assert!(!h.overview.is_empty(), "{name} 还没有写帮助");
            for (key, _) in &h.params {
                assert!(key.is_empty() || t.param(key).is_some(), "{name} 的帮助里说明了不存在的参数 {key}");
            }
            for p in &t.params {
                let written = h.params.iter().any(|(k, _)| *k == p.name);
                assert!(written || !p.hint.is_empty(), "{name}.{} 既没有说明也没有提示", p.name);
            }
            for (key, _) in &h.inputs {
                assert!(key.parse::<usize>().is_ok_and(|i| (1..=t.inputs).contains(&i)), "{name} 的帮助里说明了不存在的输入 {key}");
            }
            for other in &h.related {
                assert!(nodes::find(other).or_else(|| nodes::find_expr(other)).is_some(), "{name} 的相关节点 {other} 不存在");
            }
            // @polars 的选项都是某个菜单里有的
            for (key, _) in h.polars.iter().filter(|(k, _)| !k.is_empty()) {
                let menu = t.params.iter().any(|p| p.kind == ParamKind::Menu && p.options.contains(key));
                assert!(menu, "{name} 的 @polars 里的 {key} 不是哪个菜单的选项");
            }
        }
    }

    #[test]
    fn every_param_is_listed() {
        for kind in nodes::all().iter().chain(nodes::all_expr().iter().take(60)) {
            let rows = render(&Page::of_kind(kind));
            for p in &kind.params {
                assert!(rows.iter().any(|r| r.kind == ENTRY && r.label == p.label.as_str()), "{}.{}", kind.name, p.name);
            }
        }
    }
}
