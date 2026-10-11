//! 节点图：每个节点把输入的 DataFrame 变换后传给下游。图本身不懂 polars——
//! 节点的代码在它的类型定义里（`nodes.rs`），这里只负责结构（连线、位置、参数值）和把要执行的部分排成步骤交给引擎。
//!
//! 两级（像 Houdini 的 SOP / VOP）：表格级的 `expr` 节点里面还有一张图（`Node::net`，`Graph::vop` 为真），
//! 里面的节点各算出一个 polars 表达式，从左往右连；输出节点（列 / 键 / 条件 / 排序）的值交给 expr 节点的上下文。
//! group_by 按在哪里算分成三张网络（`Env`）：分组键（整表上算）、聚合、筛选组（都在每组里算）。

use crate::nodes::{self, NodeType, ParamKind, Source, py_str};
use serde_json::{Map, Value, json};
use std::path::Path;
use std::sync::Arc;

mod edit;

pub struct Node {
    pub id: u32,
    /// 节点的类型。一般是注册表里的；code 节点加了参数以后是它自己的一份（`Source::Instance`）
    pub kind: Arc<NodeType>,
    pub title: String,
    /// 各参数填的内容，和 kind.params 一一对应
    pub params: Vec<String>,
    pub inputs: Vec<Option<u32>>,
    pub bypass: bool,
    pub x: f32,
    pub y: f32,
    /// 打开文件时自动建的源节点：参数没动过时，显示它直接用查看器自己的解码结果，不经过引擎
    pub primary: bool,
    pub error: bool,
    /// 上次执行得到的输出列名（参数面板提示用）和类型（中键信息窗）
    pub columns: Vec<String>,
    pub dtypes: Vec<String>,
    /// read：Excel 文件里有哪些 sheet（sheet 下拉框的选项）
    pub sheets: Vec<String>,
    /// expr 节点里面的表达式网络（group_by 时是“聚合”那张），eval 节点里面的网络
    pub net: Option<Box<Graph>>,
    /// expr 节点 group_by 时另外两张网络：分组键（整表上算）、筛选组（having，每组里算）。见 `Env`
    pub key_net: Option<Box<Graph>>,
    pub having_net: Option<Box<Graph>>,
    /// 表达式级节点上次算出来的问题（参数面板里显示）；dtypes 是它输出的类型
    pub note: String,
    /// 表达式级节点的提醒（不算出错）：用不到的输出、group_by 里没聚合成一个值…
    pub warn: String,
    /// 引擎写出来的这个节点的表达式文字（网络 → 文字）：表达式级的是 pl.col("a").str.slice(0, 4)，
    /// expr 节点是 df.with_columns(…)。还没算过、写不出来是空的
    pub text: String,
}

impl Node {
    fn new(id: u32, kind: Arc<NodeType>, title: String) -> Node {
        Node {
            id,
            params: kind.params.iter().map(|p| p.default.clone()).collect(),
            inputs: vec![None; kind.inputs],
            kind,
            title,
            bypass: false,
            x: 0.0,
            y: 0.0,
            primary: false,
            error: false,
            columns: Vec::new(),
            dtypes: Vec::new(),
            sheets: Vec::new(),
            net: None,
            key_net: None,
            having_net: None,
            note: String::new(),
            warn: String::new(),
            text: String::new(),
        }
    }

    pub fn kind(&self) -> &NodeType {
        &self.kind
    }

    /// 带网络的节点里 env 那张网络用哪些输出：expr 节点按上下文（和 group_by 的“分组后”）；
    /// 表达式级的（eval）只有一张，是一个“结果”。
    pub fn env_roles(&self, env: Env) -> &'static [(&'static str, &'static str, bool)] {
        if self.kind.expr {
            return if env == Env::Main { &[("结果", "result", true)] } else { &[] };
        }
        env_roles(self.value("context"), self.value("then"), env)
    }

    /// 当前上下文用到的几张网络（只有一张时是 [Main]）。
    pub fn envs(&self) -> &'static [Env] {
        if self.kind.expr { &[Env::Main] } else { context_envs(self.value("context"), self.value("then")) }
    }

    /// 想看 want 那张网络：当前上下文用得到就是它，否则是第一张。
    pub fn shown_env(&self, want: Env) -> Env {
        let envs = self.envs();
        if envs.contains(&want) { want } else { envs.first().copied().unwrap_or_default() }
    }

    pub fn net_in(&self, env: Env) -> Option<&Graph> {
        match env {
            Env::Main => self.net.as_deref(),
            Env::Keys => self.key_net.as_deref(),
            Env::Having => self.having_net.as_deref(),
        }
    }

    pub fn net_in_mut(&mut self, env: Env) -> Option<&mut Graph> {
        match env {
            Env::Main => self.net.as_deref_mut(),
            Env::Keys => self.key_net.as_deref_mut(),
            Env::Having => self.having_net.as_deref_mut(),
        }
    }

    /// 节点里面的全部网络（当前上下文用不用得到都算）。
    pub fn nets(&self) -> impl Iterator<Item = (Env, &Graph)> {
        [(Env::Main, &self.net), (Env::Keys, &self.key_net), (Env::Having, &self.having_net)]
            .into_iter()
            .filter_map(|(env, net)| Some((env, net.as_deref()?)))
    }

    pub fn nets_mut(&mut self) -> impl Iterator<Item = (Env, &mut Graph)> {
        [(Env::Main, &mut self.net), (Env::Keys, &mut self.key_net), (Env::Having, &mut self.having_net)]
            .into_iter()
            .filter_map(|(env, net)| Some((env, net.as_deref_mut()?)))
    }

    /// 里面网络里的节点（几张网络的 id 不重，按 id 找）。
    pub fn sub_node(&self, id: u32) -> Option<&Node> {
        self.nets().find_map(|(_, g)| g.node(id))
    }

    pub fn sub_node_mut(&mut self, id: u32) -> Option<&mut Node> {
        self.nets_mut().find_map(|(_, g)| g.node_mut(id))
    }

    /// 里面的网络在什么上下文里：expr 节点的上下文，eval 节点的方式（list.eval…）。提示里用。
    pub fn context_label(&self) -> &str {
        if self.kind.expr { self.mode() } else { self.value("context") }
    }

    /// expr 节点：当前上下文用到的网络缺了就建上（group_by 的分组键、筛选组），里面缺的必要输出节点补上
    /// （换了上下文 / 分组后之后）。eval 节点只有一张网络。
    pub fn ensure_outputs(&mut self) {
        if self.net.is_none() {
            return;
        }
        for &env in self.envs() {
            let roles = self.env_roles(env);
            match self.net_in_mut(env) {
                Some(net) => net.ensure_outputs(roles),
                None => self.set_net(env, Graph::new_net(roles, None, env.base())),
            }
        }
    }

    fn set_net(&mut self, env: Env, net: Graph) {
        let slot = match env {
            Env::Main => &mut self.net,
            Env::Keys => &mut self.key_net,
            Env::Having => &mut self.having_net,
        };
        *slot = Some(Box::new(net));
    }

    /// 在界面里换了上下文：缺的网络、输出补上。网络还是新建时的样子（一个 col 接着输出列）而换成了 group_by 时，
    /// 那一列挪去当分组键（分组键那张网络里的 col 还空着的话），聚合的输出列先填 pl.len()（每组几行），
    /// 一换就能跑出有意义的结果。
    pub fn switch_context(&mut self) {
        let fresh = self.value("context") == "group_by" && self.net.as_ref().is_some_and(|net| {
            let [col, out] = &net.nodes[..] else { return false };
            col.kind.name == "col"
                && out.kind.role == "列"
                && out.inputs.first() == Some(&Some(col.id))
                && out.params.iter().zip(&out.kind.params).all(|(v, p)| *v == p.default)
        });
        self.ensure_outputs();
        if !fresh {
            return;
        }
        let Some(net) = &mut self.net else { return };
        let (col, out) = (net.nodes[0].id, net.nodes[1].id);
        let column = net.nodes[0].raw("columns").to_string();
        net.remove(col);
        if let Some(out) = net.node_mut(out) {
            out.set("value", "pl.len()");
            out.x = 40.0;
        }
        net.selected = None;
        let keys = self.key_net.as_mut().and_then(|g| g.nodes.iter_mut().find(|n| n.kind.name == "col" && n.value("columns").is_empty()));
        if let Some(key_col) = keys {
            key_col.set("columns", &column);
        }
    }

    /// expr 节点：env 那张网络里这个角色的输出节点在当前上下文里用不到时，说一声（不算出错）。
    pub fn unused_role(&self, env: Env, role: &str) -> Option<String> {
        if role.is_empty() || self.env_roles(env).iter().any(|(r, ..)| *r == role) {
            return None;
        }
        let context = self.context_label();
        let label = role_name(role);
        Some(match env {
            Env::Main if context == "group_by" && role == "列" => {
                format!("分组后是 {}，用不到{label}（换成 agg 才用）", self.value("then"))
            }
            _ if context == "group_by" => {
                let wanted: Vec<&str> = self.env_roles(env).iter().map(|(r, ..)| role_name(r)).collect();
                format!("{}这张网络只用{}，用不到{label}", env.label(), wanted.join("、"))
            }
            _ => format!("{context} 用不到{label}"),
        })
    }

    /// 第 i 个参数是输入端口、而且连了线（表达式级）：返回上游节点。
    pub fn wired(&self, i: usize) -> Option<u32> {
        self.kind.port_slot(i).and_then(|slot| self.inputs.get(slot).copied().flatten())
    }

    /// 第 i 个参数在当前的“方式”下是否显示：它的显示条件里提到的参数，值都在允许的列表里。
    pub fn visible(&self, i: usize) -> bool {
        let Some(p) = self.kind.params.get(i) else { return false };
        p.show.iter().all(|(other, allowed)| {
            self.kind.param(other).is_none_or(|j| allowed.iter().any(|v| *v == self.params[j]))
        })
    }

    /// 当前的“方式”（第一个 mode 菜单的值）；没有的节点是空串。
    pub fn mode(&self) -> &str {
        self.kind.mode_param().map_or("", |i| self.params[i].as_str())
    }

    /// 叫 name 的参数起作用的值，没 trim 过：不显示的参数按默认值算（和引擎里 ch() 取到的一致）。
    pub fn raw(&self, name: &str) -> &str {
        match self.kind.param(name) {
            Some(i) if self.visible(i) => &self.params[i],
            Some(i) => &self.kind.params[i].default,
            None => "",
        }
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn value(&self, name: &str) -> &str {
        self.raw(name).trim()
    }

    /// 给叫 name 的参数赋值；没有这个参数时返回 false。
    pub fn set(&mut self, name: &str, value: &str) -> bool {
        match self.kind.param(name) {
            Some(i) => {
                self.params[i] = value.to_string();
                true
            }
            None => false,
        }
    }

    /// sheet 参数的下标（只有 read 有）。
    pub fn sheet_param(&self) -> Option<usize> {
        self.kind.params.iter().position(|p| p.kind == ParamKind::Sheet)
    }

    /// 读 Excel 的节点（sheet 参数用得上）。
    pub fn reads_excel(&self) -> bool {
        self.sheet_param().is_some_and(|i| self.visible(i))
    }

    /// 节点的文件参数（读或写）填的路径。
    pub fn file_path(&self) -> Option<&str> {
        let file = self.kind.params.iter().position(|p| matches!(p.kind, ParamKind::File | ParamKind::SaveFile));
        file.map(|i| self.params[i].trim())
    }

    /// 换了文件：带扩展名表的菜单（read / write 的“格式”）跟着扩展名切；认不出的扩展名不动。
    pub fn sync_format(&mut self) {
        let Some(path) = self.file_path() else { return };
        let ext = Path::new(path).extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
        let kind = self.kind.clone();
        for (i, p) in kind.params.iter().enumerate() {
            let matched = p.extensions.iter().find(|(_, exts)| exts.iter().any(|e| *e == ext));
            if let Some((option, _)) = matched {
                self.params[i] = option.clone();
            }
        }
    }

    /// 换一份类型定义（类型被编辑过，或者节点有了自己的定义）：参数按名字对上，新加的用默认值。
    pub fn retype(&mut self, kind: Arc<NodeType>) {
        let old = std::mem::replace(&mut self.kind, kind);
        let mut values = std::mem::take(&mut self.params);
        self.params = (self.kind.params.iter())
            .map(|p| match old.param(&p.name) {
                // 菜单的选项可能变了：原来选的不在了就回到默认
                Some(i) if p.kind != ParamKind::Menu || p.options.contains(&values[i]) => std::mem::take(&mut values[i]),
                _ => p.default.clone(),
            })
            .collect();
        self.inputs.resize(self.kind.inputs, None);
    }

    /// 节点卡片第二行的摘要：读取节点显示文件名，其余显示填了的参数。
    pub fn summary(&self) -> String {
        let def = self.kind();
        let mut parts: Vec<String> = Vec::new();
        if self.net.is_some() && !def.expr {
            // expr 节点：上下文 + 里面接了几个输出
            let context = self.value("context");
            let count = |env: Env, role: &str| self.net_in(env).map_or(0, |g| g.nodes.iter().filter(|n| n.kind.role == role).count());
            // 可选的输出（group_by 的条件）有了才写
            let outputs: Vec<String> = (self.envs().iter())
                .flat_map(|&env| self.env_roles(env).iter().map(move |r| (env, r)))
                .filter(|(env, (role, _, required))| *required || count(*env, role) > 0)
                .map(|(env, (role, ..))| format!("{} 个{}", count(env, role), role_label(context, role)))
                .collect();
            let mut text = format!("{context} · {}", outputs.join("、"));
            if context == "group_by" && self.value("then") != "agg" {
                text += &format!(" · {}", self.value("then"));
            }
            return text;
        }
        for (i, (p, value)) in def.params.iter().zip(&self.params).enumerate() {
            let value = value.trim();
            match p.kind {
                _ if !self.visible(i) || self.wired(i).is_some() => {}
                ParamKind::File | ParamKind::SaveFile => {
                    let name = Path::new(value).file_name().and_then(|n| n.to_str()).unwrap_or(value);
                    parts.push(name.to_string());
                }
                ParamKind::Sheet => parts.push(value.to_string()),
                _ if def.is_io() => {}
                ParamKind::Button => {}
                // 方式：选项多的（下拉）总是写出来，页签只在不是默认那个时写
                ParamKind::Menu if p.mode => {
                    if p.options.len() > 5 || value != p.default {
                        parts.push(value.to_string());
                    }
                }
                ParamKind::Toggle => {
                    if value != p.default {
                        parts.push(p.label.clone());
                    }
                }
                ParamKind::Code => {
                    let line = value.lines().map(str::trim).find(|l| !l.is_empty() && !l.starts_with('#'));
                    parts.extend(line.map(String::from));
                }
                // 没动过的可选参数不占地方
                _ if !p.required && value == p.default => {}
                _ => parts.push(value.to_string()),
            }
        }
        parts.retain(|s| !s.is_empty());
        if parts.is_empty() {
            // 表达式节点的说明太长，节点旁边不写
            return if def.expr { String::new() } else { def.desc.clone() };
        }
        let text = parts.join(" · ");
        match text.char_indices().nth(80) {
            Some((cut, _)) => format!("{}…", &text[..cut]),
            None => text,
        }
    }
}

/// 传给引擎的一个参数：引擎按 kind 把 value 变成 ch() 返回的东西。
#[derive(Debug, PartialEq)]
pub struct StepParam {
    pub name: String,
    pub kind: &'static str,
    pub label: String,
    pub value: String,
    /// 定义里的默认值：网络 → 文字时，和默认一样的关键字参数不写
    pub default: String,
}

/// 一个待执行的节点：code 在 {pl, cs, df ~ df4, inputs, ch, action} 环境里执行。
#[derive(Debug, Default)]
pub struct Step {
    pub id: u32,
    pub code: String,
    pub params: Vec<StepParam>,
    pub inputs: Vec<Option<u32>>,
    /// 节点读的文件的修改时间：文件变了，引擎里的缓存随之失效
    pub stamp: u128,
    /// expr 节点：里面的表达式网络（按依赖排好），代码里用 outputs("角色") 取输出节点的值
    pub net: Vec<Step>,
    /// 表达式级：在每组里算（group_by 的聚合、筛选组两张网络里的节点；推导类型、预览时用）
    pub each: bool,
    /// 表达式级：连了线的端口参数 → 上游节点
    pub ports: Vec<(String, u32)>,
    /// 表达式级的输出节点：角色和它在同角色里的次序（画布上从上到下）
    pub role: String,
    pub order: usize,
    /// 表达式级：还不能算的原因（必填的没填）；引擎把它当成这个节点出错
    pub error: String,
}

/// 编译出错：哪个节点（表达式网络里的节点另给 sub）、为什么。
#[derive(Debug, PartialEq)]
pub struct CompileError {
    pub node: u32,
    pub sub: Option<u32>,
    pub msg: String,
}

fn fail(node: u32, msg: impl Into<String>) -> CompileError {
    CompileError { node, sub: None, msg: msg.into() }
}

/// expr 节点里的网络。一般只有一张（Main）；group_by 按在哪里算分成三张，各自只有一种输出，节点不能跨网络连：
/// 分组键（Keys，整表上算，每行一个值）、聚合（Main，每组里算）、筛选组（Having，having 的条件，每组里算）。
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Env {
    #[default]
    Main,
    Keys,
    Having,
}

impl Env {
    /// 这张网络里节点 id 的起点：同一个 expr 节点的几张网络 id 不重（引擎里拼成一张、按 id 认节点）
    pub fn base(self) -> u32 {
        match self {
            Env::Main => 0,
            Env::Keys => 1_000_000,
            Env::Having => 2_000_000,
        }
    }

    /// group_by 时这张网络叫什么（路径栏的页签）
    pub fn label(self) -> &'static str {
        match self {
            Env::Main => "聚合",
            Env::Keys => "分组键",
            Env::Having => "筛选组",
        }
    }

    /// group_by 时在哪里算
    pub fn scope(self) -> &'static str {
        if self == Env::Keys { "整表" } else { "每组" }
    }

    /// 工程文件里存在节点的哪一项
    fn json_key(self) -> &'static str {
        match self {
            Env::Main => "net",
            Env::Keys => "key_net",
            Env::Having => "having_net",
        }
    }
}

/// 各上下文用到哪几张网络（见 Env）。then 是 group_by 的“分组后”：不是 agg 时用不到聚合那张。
pub fn context_envs(context: &str, then: &str) -> &'static [Env] {
    match context {
        "group_by" if then == "agg" => &[Env::Keys, Env::Main, Env::Having],
        "group_by" => &[Env::Keys, Env::Having],
        _ => &[Env::Main],
    }
}

/// 各上下文里 env 那张网络用哪些输出：(角色, 输出节点的类型, 必要)。必要的缺了会自动补上；
/// group_by 的“条件”是 having（只留满足的分组），要用户自己加。
pub fn env_roles(context: &str, then: &str, env: Env) -> &'static [(&'static str, &'static str, bool)] {
    match (context, env) {
        ("group_by", Env::Keys) => &[("键", "key", true)],
        ("group_by", Env::Main) if then == "agg" => &[("列", "output", true)],
        ("group_by", Env::Having) => &[("条件", "condition", false)],
        ("group_by", _) => &[],
        ("filter", Env::Main) => &[("条件", "condition", true)],
        ("sort", Env::Main) => &[("排序", "sort_key", true)],
        (_, Env::Main) => &[("列", "output", true)],
        _ => &[],
    }
}

/// 输出角色的叫法（提醒里用）。
fn role_name(role: &str) -> &'static str {
    match role {
        "结果" => "结果",
        "条件" => "条件",
        "键" => "分组键",
        "排序" => "排序键",
        _ => "输出列",
    }
}

/// 输出角色在这个上下文里叫什么（摘要里用）。
fn role_label(context: &str, role: &str) -> &'static str {
    match role {
        "条件" => "条件",
        "键" => "分组键",
        "排序" => "排序键",
        _ if context == "group_by" => "聚合",
        _ => "输出列",
    }
}

#[derive(Default)]
pub struct Graph {
    pub nodes: Vec<Node>,
    next_id: u32,
    /// 当前节点（参数面板编辑的那个）
    pub selected: Option<u32>,
    /// 另外选上的节点（多选：Shift 点 / 框选）。选中的全部 = selected + picked，见 `selection`
    pub picked: Vec<u32>,
    pub display: Option<u32>,
    /// 查看器打开的文件，以及源节点当时的参数（用来判断源节点还能不能直接用查看器的数据）
    pub file: String,
    native_params: Vec<(String, String)>,
    /// 选中的连线：(目标节点, 输入槽)
    pub selected_wire: Option<(u32, usize)>,
    /// 表达式网络（expr 节点里面那张图）：节点从左往右流，量尺寸和端口位置的办法不同
    pub vop: bool,
    /// 从工程文件读的时候，因为类型已经没有了而跳过的节点：(节点名, 类型名)
    pub missing: Vec<(String, String)>,
}

/// 节点的尺寸和端口位置（画布坐标，和界面里画的一致）：112×38，上下各 4px 是端口，中间 30px 是节点本身；
/// 输入端口在上沿均匀排开，输出端口在下沿正中。名字画在节点右边，不算在这里面。
const NODE_W: f32 = 112.0;
const NODE_H: f32 = 38.0;
const PORT_IN_Y: f32 = 0.0;
const PORT_OUT_Y: f32 = 38.0;
/// 节点右边名字和摘要大约占的宽度（“适应”时算进去）
const LABEL_W: f32 = 200.0;

fn port_x(inputs: usize, slot: usize) -> f32 {
    NODE_W * (slot + 1) as f32 / (inputs + 1) as f32
}

/// 节点间距（新节点放在选中节点正下方；位置被占了就往右挪，要让出右边的名字）。
pub const NODE_DY: f32 = 70.0;
const NODE_DX: f32 = 300.0;

/// 表达式网络的节点（和界面里画的一致）：160 宽；头 24 高，输出端口在头的右端；
/// 下面每个输入端口一行 18（端口在左边沿）；没有输入的节点只有头。摘要写在节点上面（占 18 高）。
const VOP_W: f32 = 160.0;
const VOP_HEAD: f32 = 24.0;
const VOP_ROW: f32 = 18.0;
const VOP_DX: f32 = 230.0;
const VOP_DY: f32 = 70.0;
const VOP_LABEL_H: f32 = 18.0;

fn vop_h(inputs: usize) -> f32 {
    if inputs == 0 { VOP_HEAD + 6.0 } else { VOP_HEAD + VOP_ROW * inputs as f32 + 4.0 }
}

fn named_params(node: &Node) -> Vec<(String, String)> {
    node.kind.params.iter().zip(&node.params).map(|(p, value)| (p.name.clone(), value.clone())).collect()
}

/// 一个节点存成的 JSON（工程文件、复制粘贴）。用到的非内置类型的定义记进 types（节点私有的定义存在节点自己的 def 里）。
fn node_to_json(n: &Node, types: &mut Map<String, Value>) -> Value {
    let pairs = |items: &[(String, String)]| -> Value {
        Value::Object(items.iter().map(|(name, value)| (name.clone(), json!(value))).collect::<Map<_, _>>())
    };
    let mut v = json!({
        "id": n.id,
        "type": n.kind.name,
        "title": n.title,
        "params": pairs(&named_params(n)),
        "inputs": n.inputs,
        "bypass": n.bypass,
        "x": n.x,
        "y": n.y,
        "primary": n.primary,
    });
    for (env, net) in n.nets() {
        v[env.json_key()] = net.to_json();
    }
    match n.kind.source {
        Source::Builtin => {}
        Source::Instance => v["def"] = json!(n.kind.to_text()),
        Source::User | Source::Project => {
            types.insert(n.kind.name.clone(), json!(n.kind.to_text()));
        }
    }
    v
}

/// v["types"] 里带的类型定义注册上（和现有的一样就不动）。
fn install_types(v: &Value) {
    for (name, text) in v["types"].as_object().into_iter().flatten() {
        let Ok(t) = NodeType::parse(name, text.as_str().unwrap_or(""), Source::Project) else { continue };
        if nodes::find_of(name, t.expr).is_none_or(|current| current.to_text() != t.to_text()) {
            nodes::install(t);
        }
    }
}

/// 从 JSON 恢复一个节点（vop：类型到表达式级的注册表里找）。参数按名字对上，缺的用默认值；连线原样（id 由调用的人对）。
/// 类型已经没有了：Err((节点名, 类型名))；连 id 都没有：None。
fn node_from_json(n: &Value, vop: bool) -> Option<Result<Node, (String, String)>> {
    let id_of = |v: &Value| v.as_u64().map(|id| id as u32);
    let pairs = |v: &Value| -> Vec<(String, String)> {
        let items = v.as_object().into_iter().flatten();
        items.map(|(name, value)| (name.clone(), value.as_str().unwrap_or("").to_string())).collect()
    };
    let name = n["type"].as_str().or(n["kind"].as_str()).unwrap_or("");
    let kind = match n["def"].as_str() {
        Some(text) => NodeType::parse(name, text, Source::Instance).ok().map(Arc::new),
        None => nodes::find_of(name, vop),
    };
    let Some(kind) = kind else {
        let title = n["title"].as_str().unwrap_or(name);
        return Some(Err((title.to_string(), name.to_string())));
    };
    let mut saved = pairs(&n["params"]);
    // 上一版的工程：参数是按位置存的数组。各节点的参数顺序已经变了，只认得出第一个是文件的情况（read / write）
    let first_file = kind.params.first().filter(|p| matches!(p.kind, ParamKind::File | ParamKind::SaveFile));
    if let (Some(old), Some(p)) = (n["params"].as_array(), first_file) {
        saved = vec![(p.name.clone(), old.first().and_then(|v| v.as_str()).unwrap_or("").to_string())];
    }
    // 上一版的开关换成了页签：expr 节点（filter）的 remove、排序键的 descending
    for (kind_name, old, new, value) in [("expr", "remove", "keep", "去掉满足的"), ("sort_key", "descending", "order", "从大到小")] {
        let on = saved.iter().any(|(name, v)| name == old && v == "True");
        if kind.name == kind_name && on && !saved.iter().any(|(name, _)| name == new) {
            saved.push((new.to_string(), value.to_string()));
        }
    }
    let params = (kind.params.iter())
        .map(|p| match saved.iter().find(|(name, _)| *name == p.name) {
            Some((_, value)) => value.clone(),
            None => p.default.clone(),
        })
        .collect();
    let mut inputs: Vec<Option<u32>> = n["inputs"].as_array().into_iter().flatten().map(id_of).collect();
    inputs.resize(kind.inputs, None);
    let title = n["title"].as_str().unwrap_or(&kind.name).to_string();
    let mut node = Node::new(id_of(&n["id"])?, kind, title);
    node.params = params;
    node.inputs = inputs;
    node.bypass = n["bypass"].as_bool().unwrap_or(false);
    node.x = n["x"].as_f64().unwrap_or(0.0) as f32;
    node.y = n["y"].as_f64().unwrap_or(0.0) as f32;
    node.primary = n["primary"].as_bool().unwrap_or(false);
    if node.kind.network {
        let net = Graph::from_json_level(&n["net"], true).unwrap_or_else(|| Graph::new_net(node.env_roles(Env::Main), None, 0));
        node.net = Some(Box::new(net));
        if !node.kind.expr {
            node.key_net = Graph::from_json_level(&n["key_net"], true).map(Box::new);
            node.having_net = Graph::from_json_level(&n["having_net"], true).map(Box::new);
            if node.key_net.is_none() && node.value("context") == "group_by" {
                // 上一版的 group_by 只有一张网络：分组键、having 的条件连同只给它们用的上游各自搬到自己那张网络里
                let net = node.net.as_mut().expect("刚建的");
                let keys = net.split_off("键", Env::Keys.base());
                let having = net.split_off("条件", Env::Having.base());
                node.key_net = Some(Box::new(keys));
                node.having_net = Some(Box::new(having));
            }
            node.ensure_outputs();
        }
    }
    Some(Ok(node))
}

fn mtime(path: &str) -> u128 {
    let modified = std::fs::metadata(Path::new(path)).and_then(|m| m.modified()).ok();
    modified.and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_millis())
}

impl Graph {
    /// 只有一个源节点的新图：read 节点的格式按扩展名选。
    pub fn for_file(path: &Path, sheet: &str) -> Graph {
        Graph::for_file_with(path, sheet, &[])
    }

    /// 查看器自己读不了、read 节点读得了的文件（JSON、Arrow、Avro）：只有一个源节点的图，显示它总是交给引擎读。
    pub fn for_engine_file(path: &Path) -> Graph {
        let mut g = Graph::for_file(path, "");
        g.native_params.clear();
        g
    }

    /// 同 for_file；hints：查看器读这个文件时认出来的读法（CSV 的编码、分隔符，见 DataSource::read_hints），
    /// 填进源节点，引擎才和查看器读得一样（参数没动过，显示它照样用查看器自己的数据）。
    pub fn for_file_with(path: &Path, sheet: &str, hints: &[(&str, String)]) -> Graph {
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
        let mut g = Graph::default();
        g.file = path.to_string_lossy().into_owned();
        let id = g.add(&nodes::find("read").expect("没有 read 节点"));
        let file = g.file.clone();
        let node = g.node_mut(id).unwrap();
        node.primary = true;
        node.set("source", &file);
        node.sync_format();
        if node.reads_excel() {
            node.set("sheet_name", sheet);
        } else if ext == "tsv" {
            node.set("separator", "\\t");
        }
        for (name, value) in hints {
            node.set(name, value);
        }
        let params = named_params(node);
        g.native_params = params;
        g
    }

    /// 存进工程文件的内容：节点、连线、选中 / 显示标记，以及用到的非内置类型的定义（换台机器也能打开）。
    /// 列名、sheet 列表这些打开后再取。
    pub fn to_json(&self) -> Value {
        let pairs = |items: &[(String, String)]| -> Value {
            Value::Object(items.iter().map(|(name, value)| (name.clone(), json!(value))).collect::<Map<_, _>>())
        };
        let mut types = Map::new();
        let nodes: Vec<Value> = self.nodes.iter().map(|n| node_to_json(n, &mut types)).collect();
        json!({
            "file": self.file,
            "native_params": pairs(&self.native_params),
            "next_id": self.next_id,
            "selected": self.selected,
            "display": self.display,
            "types": types,
            "nodes": nodes,
        })
    }

    /// 从工程文件恢复。工程里带的类型定义先注册（和现有的一样就不动）；
    /// 找不到类型的节点跳过（连到它的线断开）；参数按名字对上，缺的用默认值。
    pub fn from_json(v: &Value) -> Option<Graph> {
        Graph::from_json_level(v, false)
    }

    /// vop：这是 expr 节点里面的表达式网络（类型到表达式级的注册表里找）。
    fn from_json_level(v: &Value, vop: bool) -> Option<Graph> {
        let id_of = |v: &Value| v.as_u64().map(|id| id as u32);
        let pairs = |v: &Value| -> Vec<(String, String)> {
            let items = v.as_object().into_iter().flatten();
            items.map(|(name, value)| (name.clone(), value.as_str().unwrap_or("").to_string())).collect()
        };
        install_types(v);
        let mut g = Graph { file: v["file"].as_str().unwrap_or("").to_string(), vop, ..Graph::default() };
        g.native_params = pairs(&v["native_params"]);
        for n in v["nodes"].as_array()? {
            match node_from_json(n, vop) {
                Some(Ok(node)) => g.nodes.push(node),
                Some(Err(missing)) => g.missing.push(missing),
                None => return None,
            }
        }
        // 上一版的工程没按名字存源节点打开时的参数：当它没改过
        if g.native_params.is_empty()
            && let Some(primary) = g.nodes.iter().find(|n| n.primary)
        {
            g.native_params = named_params(primary);
        }
        let ids: Vec<u32> = g.nodes.iter().map(|n| n.id).collect();
        for slot in g.nodes.iter_mut().flat_map(|n| &mut n.inputs) {
            *slot = slot.filter(|id| ids.contains(id));
        }
        let max_id = ids.iter().copied().max().unwrap_or(0);
        g.next_id = id_of(&v["next_id"]).unwrap_or(0).max(max_id);
        g.selected = id_of(&v["selected"]).filter(|id| ids.contains(id));
        g.display = id_of(&v["display"]).filter(|id| ids.contains(id));
        Some(g)
    }

    /// 工程连同数据文件一起挪了位置：读文件的节点按原路径找不到时，改成工程文件旁边的同名文件。
    pub fn relocate(&mut self, dir: &Path) {
        let moved = |path: &str| -> Option<String> {
            let old = Path::new(path.trim());
            if path.trim().is_empty() || old.exists() {
                return None;
            }
            let here = dir.join(old.file_name()?);
            here.exists().then(|| here.to_string_lossy().into_owned())
        };
        for node in &mut self.nodes {
            let kind = node.kind.clone();
            for (i, _) in kind.params.iter().enumerate().filter(|(_, p)| p.kind == ParamKind::File) {
                if let Some(new) = moved(&node.params[i]) {
                    node.params[i] = new;
                }
            }
        }
        if let Some(new) = moved(&self.file) {
            for (_, value) in self.native_params.iter_mut().filter(|(_, value)| *value == self.file) {
                *value = new.clone();
            }
            self.file = new;
        }
    }

    pub fn node(&self, id: u32) -> Option<&Node> {
        self.nodes.iter().find(|n| n.id == id)
    }

    pub fn node_mut(&mut self, id: u32) -> Option<&mut Node> {
        self.nodes.iter_mut().find(|n| n.id == id)
    }

    pub fn index_of(&self, id: u32) -> Option<usize> {
        self.nodes.iter().position(|n| n.id == id)
    }

    pub fn primary(&self) -> Option<&Node> {
        self.nodes.iter().find(|n| n.primary)
    }

    /// 源节点读的还是查看器打开的那个文件、读取参数也没改（sheet 除外，查看器会跟着切）：
    /// 显示它时直接用查看器自己的数据，不用启动引擎。
    pub fn is_native(&self, id: u32) -> bool {
        let Some(node) = self.node(id).filter(|n| n.primary && n.kind.name == "read") else { return false };
        node.kind.params.iter().zip(&node.params).all(|(p, value)| match p.kind {
            ParamKind::Sheet => value.is_empty() || node.sheets.is_empty() || node.sheets.contains(value),
            _ => self.native_params.iter().any(|(name, native)| *name == p.name && native == value),
        })
    }

    /// 新建节点：接在选中节点后面（放在它正下方），并成为选中 + 显示节点。
    pub fn add(&mut self, kind: &Arc<NodeType>) -> u32 {
        self.add_at(kind, None)
    }

    /// 同 add；at：节点放在画布上的这个位置（Tab 菜单在鼠标处建节点）。
    /// 表达式网络里新节点放在选中节点右边，第一个输入接选中节点。
    pub fn add_at(&mut self, kind: &Arc<NodeType>, at: Option<(f32, f32)>) -> u32 {
        self.next_id += 1;
        let id = self.next_id;
        let upstream = self.selected.and_then(|s| self.node(s)).map(|n| (n.id, n.x, n.y));
        let (step_x, step_y) = if self.vop { (VOP_DX, 0.0) } else { (0.0, NODE_DY) };
        let (mut x, mut y) = match (at, upstream) {
            (Some(at), _) => at,
            (None, Some((_, x, y))) => (x + step_x, y + step_y),
            (None, None) if self.vop => (40.0, 40.0 + VOP_DY * self.nodes.len() as f32),
            (None, None) => (24.0, 16.0 + NODE_DY * self.nodes.len() as f32),
        };
        while at.is_none() && self.nodes.iter().any(|n| (n.x - x).abs() < 60.0 && (n.y - y).abs() < 40.0) {
            if self.vop {
                y += VOP_DY;
            } else {
                x += NODE_DX;
            }
        }
        // 名字：类型的短名 + 第一个没人用的序号
        let base = kind.short_name();
        let title = (1..).map(|n| format!("{base}{n}")).find(|t| self.nodes.iter().all(|n| n.title != *t)).unwrap();
        let mut node = Node::new(id, kind.clone(), title);
        (node.x, node.y) = (x, y);
        if let (Some(slot), Some((up, _, _))) = (node.inputs.first_mut(), upstream) {
            *slot = Some(up);
        }
        if kind.network {
            // 里面的“列”先填上游的第一列：with_columns 原样写回这一列，一建好就能跑。
            // expr 节点的分组键那张网络也先建好（同样填第一列），换成 group_by 时直接能用
            let first = upstream.and_then(|(up, ..)| self.node(up)?.columns.first().cloned());
            node.net = Some(Box::new(Graph::new_net(node.env_roles(Env::Main), first.as_deref(), 0)));
            if !kind.expr {
                let roles = env_roles("group_by", "agg", Env::Keys);
                node.key_net = Some(Box::new(Graph::new_net(roles, first.as_deref(), Env::Keys.base())));
                node.ensure_outputs();
            }
        }
        self.nodes.push(node);
        self.selected = Some(id);
        self.picked.clear();
        self.display = Some(id);
        id
    }

    /// 带网络的节点刚建好时里面的网络：一个输入节点接到要的输出节点上。expr 节点里是“列”（column：先填上哪一列），
    /// eval 节点里（输出是“结果”）是 element。没有必要的输出（group_by 的筛选组）时是空的。
    /// 节点的 id 从 base 往后排（见 Env::base）。
    pub fn new_net(roles: &[(&str, &str, bool)], column: Option<&str>, base: u32) -> Graph {
        let mut g = Graph { vop: true, next_id: base, ..Graph::default() };
        if !roles.iter().any(|(.., required)| *required) {
            return g;
        }
        let input = if roles.iter().any(|(role, ..)| *role == "结果") { "element" } else { "col" };
        let col = g.add_at(&nodes::find_expr(input).expect("没有输入节点"), Some((40.0, 40.0)));
        if let (Some(column), Some(node)) = (column, g.node_mut(col)) {
            node.set("columns", &py_str(column));
        }
        g.selected = None;
        g.ensure_outputs(roles);
        if let Some(first) = g.nodes.iter().find(|n| !n.kind.role.is_empty()).map(|n| n.id) {
            g.connect(col, first, 0);
        }
        g.selected = None;
        g.display = None;
        g
    }

    /// 上下文要的输出节点（roles 见 context_roles）缺了就补一个，放在最右边一列、已有节点的下面。
    pub fn ensure_outputs(&mut self, roles: &[(&str, &str, bool)]) {
        let selected = self.selected;
        for (role, type_name, required) in roles {
            if !required || self.nodes.iter().any(|n| n.kind.role == *role) {
                continue;
            }
            let Some(kind) = nodes::find_expr(type_name) else { continue };
            let x = self.nodes.iter().map(|n| n.x).fold(40.0_f32, f32::max) + if self.nodes.is_empty() { 0.0 } else { VOP_DX };
            let x = self.nodes.iter().filter(|n| !n.kind.role.is_empty()).map(|n| n.x).fold(x, f32::min);
            let y = (self.nodes.iter().filter(|n| (n.x - x).abs() < 1.0))
                .map(|n| n.y + vop_h(n.inputs.len()) + 30.0)
                .fold(40.0_f32, f32::max);
            self.selected = None;
            self.add_at(&kind, Some((x, y)));
        }
        self.selected = selected;
        self.display = None;
    }

    /// 上一版的 group_by 网络拆开：role 这种输出连同它的上游搬进一张新网络（id 加上 base，名字、位置照旧）。
    /// 别的输出也用到的上游在这里留一份；没接到任何输出上的节点留在这里。
    fn split_off(&mut self, role: &str, base: u32) -> Graph {
        let outputs = |pick: &dyn Fn(&str) -> bool| -> Vec<u32> {
            let ids: Vec<u32> = self.nodes.iter().filter(|n| !n.kind.role.is_empty() && pick(&n.kind.role)).map(|n| n.id).collect();
            self.with_relatives(&ids, true)
        };
        let moved = outputs(&|r| r == role);
        let kept = outputs(&|r| r != role);
        let mut g = Graph { vop: true, next_id: base, ..Graph::default() };
        let mut types = Map::new();
        for n in self.nodes.iter().filter(|n| moved.contains(&n.id)) {
            let Some(Ok(mut node)) = node_from_json(&node_to_json(n, &mut types), true) else { continue };
            node.id += base;
            for slot in &mut node.inputs {
                *slot = slot.map(|id| id + base);
            }
            g.next_id = g.next_id.max(node.id);
            g.nodes.push(node);
        }
        for id in moved.into_iter().filter(|id| !kept.contains(id)) {
            self.remove(id);
        }
        self.selected = None;
        g
    }

    /// 类型的定义变了（在界面里编辑过、或者被删了换回内置的）：图里用这个类型的节点都换成新的。
    pub fn rebind(&mut self, kind: &Arc<NodeType>) {
        for node in &mut self.nodes {
            let same = node.kind.name == kind.name && node.kind.expr == kind.expr;
            if same && node.kind.source != Source::Instance && !Arc::ptr_eq(&node.kind, kind) {
                node.retype(kind.clone());
            }
            for (_, net) in node.nets_mut() {
                net.rebind(kind);
            }
        }
    }

    /// 删除节点；下游的连线断开，显示标记退回它的上游。
    pub fn remove(&mut self, id: u32) {
        let Some(idx) = self.index_of(id) else { return };
        let node = self.nodes.remove(idx);
        let fallback = node.inputs.first().copied().flatten().or_else(|| self.primary().map(|n| n.id));
        for n in &mut self.nodes {
            for slot in &mut n.inputs {
                if *slot == Some(id) {
                    *slot = None;
                }
            }
        }
        if self.display == Some(id) {
            self.display = fallback;
        }
        if self.selected == Some(id) {
            self.selected = fallback;
        }
        self.picked.retain(|&p| p != id);
        self.selected_wire = None;
    }

    /// 文字 → 网络：引擎拆好的节点（engine.py 的 parse_text）建进这张网络。结果那个节点（root）放在 at，
    /// 它用到的节点一列一列往左排。inputs：snippet 的 a ~ d 原来接着谁。返回 root 的 id。
    pub fn insert_parsed(&mut self, parsed: &Value, at: (f32, f32), inputs: &[(String, Option<u32>)]) -> Result<u32, String> {
        let items = parsed["nodes"].as_array().ok_or("引擎没拆出节点")?;
        let root = parsed["root"].as_u64().map(|i| i as usize).filter(|&i| i < items.len()).ok_or("引擎没说结果是哪个节点")?;
        let depths = parsed_depths(items);
        let deepest = depths.get(root).copied().unwrap_or(0);
        let mut next_y: Vec<f32> = vec![at.1; deepest + 1];
        let mut ids: Vec<Option<u32>> = Vec::with_capacity(items.len());
        for (i, item) in items.iter().enumerate() {
            if let Some(name) = item["input"].as_str() {
                ids.push(inputs.iter().find(|(n, _)| n == name).and_then(|(_, id)| *id));
                continue;
            }
            let name = item["type"].as_str().unwrap_or("");
            let kind = nodes::find_expr(name).ok_or(format!("没有 {name} 这种表达式节点"))?;
            // 离结果越远越靠左；同一列从上往下排
            let column = depths[i].min(deepest);
            let x = at.0 - (deepest - column) as f32 * VOP_DX;
            let y = next_y[column];
            next_y[column] += vop_h(kind.inputs) + 30.0;
            self.selected = None;
            let id = self.add_at(&kind, Some((x, y)));
            let node = self.node_mut(id).expect("刚建的");
            for (param, value) in item["params"].as_object().into_iter().flatten() {
                node.set(param, value.as_str().unwrap_or(""));
            }
            for (param, child) in item["ports"].as_object().into_iter().flatten() {
                let slot = kind.param(param).and_then(|p| kind.port_slot(p));
                let up = child.as_u64().and_then(|c| ids.get(c as usize).copied().flatten());
                if let (Some(slot), Some(up)) = (slot, up) {
                    self.connect(up, id, slot);
                }
            }
            if item.get("net").is_some()
                && let Some(node) = self.node_mut(id).filter(|n| n.net.is_some())
            {
                // eval：里面那张网络照样建，最后接到“结果”上
                let roles = node.env_roles(Env::Main);
                let mut inner = Graph { vop: true, ..Graph::default() };
                let inner_depth = parsed_depths(item["net"]["nodes"].as_array().map_or(&[][..], |v| v));
                let inner_root = item["net"]["root"].as_u64().unwrap_or(0) as usize;
                let x = 40.0 + inner_depth.get(inner_root).copied().unwrap_or(0) as f32 * VOP_DX;
                let top = inner.insert_parsed(&item["net"], (x, 40.0), &[])?;
                inner.ensure_outputs(roles);
                if let Some(result) = inner.nodes.iter_mut().find(|n| !n.kind.role.is_empty()) {
                    (result.x, result.y) = (x + VOP_DX, 40.0);
                    let result = result.id;
                    inner.connect(top, result, 0);
                }
                inner.selected = None;
                node.net = Some(Box::new(inner));
            }
            ids.push(Some(id));
        }
        self.selected = None;
        ids[root].ok_or(String::from("结果是一个没接上的输入"))
    }

    /// Tab 菜单里写的文字建成节点（parsed 见 insert_parsed），结果放在 at。最外面是 .alias("名字")、
    /// 而这张网络用“列”输出（columns：expr 节点里）时，直接建一个叫这个名字的输出列接在后面。返回该选中的节点。
    pub fn insert_text(&mut self, parsed: &Value, at: (f32, f32), columns: bool) -> Result<u32, String> {
        let alias = parsed.get("alias").filter(|a| a.is_object());
        let (Some(alias), true) = (alias, columns) else { return self.insert_parsed(parsed, at, &[]) };
        let root = self.insert_parsed(alias, at, &[])?;
        let kind = nodes::find_expr("output").ok_or("没有输出节点")?;
        self.selected = None;
        let out = self.add_at(&kind, Some((at.0 + VOP_DX, at.1)));
        if let Some(node) = self.node_mut(out) {
            node.set("name", alias["name"].as_str().unwrap_or(""));
        }
        self.connect(root, out, 0);
        Ok(out)
    }

    /// snippet 节点（文字）展开成节点网络：parsed 是引擎拆好的（见 insert_parsed）。原来接 snippet 的改接结果节点。
    pub fn expand_snippet(&mut self, id: u32, parsed: &Value) -> Result<u32, String> {
        let node = self.node(id).ok_or("节点不在了")?;
        let kind = node.kind.clone();
        let inputs: Vec<(String, Option<u32>)> = ["a", "b", "c", "d"]
            .iter()
            .map(|name| (name.to_string(), kind.param(name).and_then(|i| node.wired(i))))
            .collect();
        let at = (node.x, node.y);
        let root = self.insert_parsed(parsed, at, &inputs)?;
        self.replace_uses(id, root);
        self.remove(id);
        self.selected = Some(root);
        Ok(root)
    }

    /// 网络 → 文字：节点连同只给它用的上游节点，换成一个写着它的表达式文字（node.text，引擎写的）的 snippet 节点。
    /// 上游里还有别的节点在用的留着；输出节点不收。返回 snippet 的 id。
    pub fn collapse_to_text(&mut self, id: u32) -> Option<u32> {
        let node = self.node(id).filter(|n| n.kind.role.is_empty() && !n.text.is_empty())?;
        let (text, at) = (node.text.clone(), (node.x, node.y));
        let snippet = nodes::find_expr("snippet")?;
        // 一起收掉的：自己，和所有用到它们的节点都在这里面的上游（输出节点不收）
        let mut gone = vec![id];
        loop {
            let more: Vec<u32> = (self.nodes.iter())
                .filter(|n| !gone.contains(&n.id) && n.kind.role.is_empty())
                .filter(|n| {
                    let users: Vec<&Node> = self.nodes.iter().filter(|u| u.inputs.contains(&Some(n.id))).collect();
                    !users.is_empty() && users.iter().all(|u| gone.contains(&u.id))
                })
                .map(|n| n.id)
                .collect();
            if more.is_empty() {
                break;
            }
            gone.extend(more);
        }
        self.selected = None;
        let new = self.add_at(&snippet, Some(at));
        self.node_mut(new)?.set("expression", &text);
        self.replace_uses(id, new);
        for g in gone {
            self.remove(g);
        }
        self.selected = Some(new);
        Some(new)
    }

    /// 原来接 old 的输入都改接 new。
    fn replace_uses(&mut self, old: u32, new: u32) {
        for slot in self.nodes.iter_mut().filter(|n| n.id != new).flat_map(|n| &mut n.inputs) {
            if *slot == Some(old) {
                *slot = Some(new);
            }
        }
    }

    /// 断开一条连线，节点都留着。
    pub fn disconnect(&mut self, to: u32, slot: usize) -> bool {
        if self.selected_wire == Some((to, slot)) {
            self.selected_wire = None;
        }
        match self.node_mut(to).and_then(|n| n.inputs.get_mut(slot)) {
            Some(input) if input.is_some() => {
                *input = None;
                true
            }
            _ => false,
        }
    }

    /// 节点的宽高（画布坐标，不算右边的名字）。
    fn size(&self, n: &Node) -> (f32, f32) {
        if self.vop { (VOP_W, vop_h(n.inputs.len())) } else { (NODE_W, NODE_H) }
    }

    /// 第 slot 个输入端口的位置。
    fn in_port(&self, n: &Node, slot: usize) -> (f32, f32) {
        if self.vop {
            (n.x, n.y + VOP_HEAD + VOP_ROW * (slot as f32 + 0.5))
        } else {
            (n.x + port_x(n.inputs.len(), slot), n.y + PORT_IN_Y)
        }
    }

    fn out_port(&self, n: &Node) -> (f32, f32) {
        if self.vop { (n.x + VOP_W, n.y + VOP_HEAD / 2.0) } else { (n.x + NODE_W / 2.0, n.y + PORT_OUT_Y) }
    }

    /// (x, y) 处最上面的节点（后加的画在上面）。
    fn node_at(&self, x: f32, y: f32) -> Option<&Node> {
        let hit = |n: &&Node| {
            let (w, h) = self.size(n);
            x >= n.x - 8.0 && x <= n.x + w + 8.0 && y >= n.y - 8.0 && y <= n.y + h + 8.0
        };
        self.nodes.iter().rev().find(hit)
    }

    /// 从输入端口往回拖线，在 (x, y) 松手：落在哪个节点上（它的输出接过来）。
    pub fn output_at(&self, x: f32, y: f32) -> Option<u32> {
        self.node_at(x, y).map(|n| n.id)
    }

    /// 从输出端口拖线到 (x, y) 松手：落在哪个节点的哪个输入上。
    /// 对准了某个端口就是它；落在节点身上则接到第一个空着的输入。
    pub fn input_at(&self, x: f32, y: f32) -> Option<(u32, usize)> {
        let node = self.node_at(x, y).filter(|n| !n.inputs.is_empty())?;
        let count = node.inputs.len();
        if self.vop {
            // 落在哪一行就是哪个端口；落在头上接第一个空着的
            let row = ((y - node.y - VOP_HEAD) / VOP_ROW).floor();
            let first_free = node.inputs.iter().position(|i| i.is_none()).unwrap_or(0);
            let slot = if row >= 0.0 { (row as usize).min(count - 1) } else { first_free };
            return Some((node.id, slot));
        }
        let distance = |slot: usize| (x - node.x - port_x(count, slot)).abs();
        let nearest = (0..count).min_by(|&a, &b| distance(a).total_cmp(&distance(b)))?;
        let on_port = distance(nearest) <= 14.0 && y <= node.y + 14.0;
        let slot = if on_port { nearest } else { node.inputs.iter().position(|i| i.is_none()).unwrap_or(nearest) };
        Some((node.id, slot))
    }

    /// 连线（node 的第 slot 个输入）取样成的折线（画布坐标）：和界面一样的贝塞尔曲线。没连线是 None。
    fn wire_points(&self, node: &Node, slot: usize) -> Option<Vec<(f32, f32)>> {
        let src = node.inputs.get(slot).copied().flatten().and_then(|id| self.node(id))?;
        let (x0, y0) = self.out_port(src);
        let (x3, y3) = self.in_port(node, slot);
        // 控制点：表格级上下弯，表达式级左右弯（和界面里画的一样）
        let (x1, y1, x2, y2) = if self.vop {
            let bend = ((x3 - x0).abs() / 2.0).max(30.0);
            (x0 + bend, y0, x3 - bend, y3)
        } else {
            (x0, y0 + 30.0, x3, y3 - 30.0)
        };
        let point = |t: f32| {
            let u = 1.0 - t;
            let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
            (a * x0 + b * x1 + c * x2 + d * x3, a * y0 + b * y1 + c * y2 + d * y3)
        };
        Some((0..=24).map(|step| point(step as f32 / 24.0)).collect())
    }

    /// (x, y) 附近 tolerance 以内的连线：(目标节点, 输入槽)。
    pub fn wire_at(&self, x: f32, y: f32, tolerance: f32) -> Option<(u32, usize)> {
        let mut best: Option<(f32, (u32, usize))> = None;
        for node in &self.nodes {
            for slot in 0..node.inputs.len() {
                let Some(points) = self.wire_points(node, slot) else { continue };
                for pair in points.windows(2) {
                    let d = segment_distance((x, y), pair[0], pair[1]);
                    if d <= tolerance && best.is_none_or(|(b, _)| d < b) {
                        best = Some((d, (node.id, slot)));
                    }
                }
            }
        }
        best.map(|(_, wire)| wire)
    }

    /// 改节点的名字：去掉首尾空白，不能空、不能和别的节点重名。
    pub fn rename(&mut self, id: u32, name: &str) -> Result<(), String> {
        let name = name.trim();
        if name.is_empty() {
            return Err(String::from("节点名不能是空的"));
        }
        if self.nodes.iter().any(|n| n.id != id && n.title == name) {
            return Err(format!("已经有叫 {name} 的节点了"));
        }
        let node = self.node_mut(id).ok_or_else(|| String::from("节点不存在"))?;
        node.title = name.to_string();
        Ok(())
    }

    /// 节点（连同右边的名字）占的范围：(左, 上, 右, 下)，画布坐标；ids：只算这些（None：全部）。没有节点时为空。
    pub fn bounds_of(&self, ids: Option<&[u32]>) -> Option<(f32, f32, f32, f32)> {
        // 名字和摘要：表格级在节点右边，表达式级在节点上面
        let (right, top) = if self.vop { (0.0, VOP_LABEL_H) } else { (LABEL_W, 0.0) };
        let mut nodes = self.nodes.iter().filter(|n| ids.is_none_or(|ids| ids.contains(&n.id)));
        let first = nodes.next()?;
        let mut b = (first.x, first.y - top, first.x + self.size(first).0 + right, first.y + self.size(first).1);
        for n in nodes {
            let (w, h) = self.size(n);
            b = (b.0.min(n.x), b.1.min(n.y - top), b.2.max(n.x + w + right), b.3.max(n.y + h));
        }
        Some(b)
    }

    /// 把 from 的输出接到 to 的第 slot 个输入；会形成环时拒绝。
    pub fn connect(&mut self, from: u32, to: u32, slot: usize) -> bool {
        if from == to || self.depends_on(from, to) {
            return false;
        }
        match self.node_mut(to).and_then(|n| n.inputs.get_mut(slot)) {
            Some(s) => {
                *s = Some(from);
                true
            }
            None => false,
        }
    }

    /// node 是否（间接）依赖 target。
    fn depends_on(&self, node: u32, target: u32) -> bool {
        let Some(n) = self.node(node) else { return false };
        n.inputs.iter().flatten().any(|&i| i == target || self.depends_on(i, target))
    }

    pub fn clear_errors(&mut self) {
        for n in &mut self.nodes {
            n.error = false;
        }
    }

    /// 参数面板里给人看的代码：节点类型的代码。代码本身就是参数的节点（code）不用再列一遍。
    pub fn code_preview(&self, id: u32) -> String {
        match self.node(id) {
            Some(node) if node.kind.code_param.is_empty() => node.kind.code.trim_end().to_string(),
            _ => String::new(),
        }
    }

    /// 编译 target 及其全部上游（按依赖顺序）。
    pub fn compile(&self, target: u32) -> Result<Vec<Step>, CompileError> {
        let mut steps = Vec::new();
        let mut done = Vec::new();
        self.compile_into(target, &mut steps, &mut done)?;
        Ok(steps)
    }

    fn compile_into(&self, id: u32, steps: &mut Vec<Step>, done: &mut Vec<u32>) -> Result<(), CompileError> {
        if done.contains(&id) {
            return Ok(());
        }
        done.push(id);
        let node = self.node(id).ok_or(fail(id, "节点不存在"))?;
        let def = node.kind();
        for (slot, input) in node.inputs.iter().enumerate() {
            match input {
                Some(up) => self.compile_into(*up, steps, done)?,
                None if slot < def.min_inputs => {
                    return Err(fail(id, format!("第 {} 个输入还没有连接", slot + 1)));
                }
                None => {}
            }
        }
        let inputs = node.inputs.clone();
        // 旁路：输入原样传下去
        if node.bypass && node.inputs.first().is_some_and(|i| i.is_some()) {
            steps.push(Step { id, code: String::from("out = df"), inputs, ..Step::default() });
            return Ok(());
        }
        let (params, stamp) = step_params(node).map_err(|msg| fail(id, msg))?;
        let code = match def.param(&def.code_param) {
            Some(i) => node.params[i].clone(),
            None => def.code.clone(),
        };
        steps.push(Step { id, code, params, inputs, stamp, net: compile_nets(node), ..Step::default() });
        Ok(())
    }

    /// 表达式网络 → 步骤：全部节点按依赖排（没接到输出上的也算，好看它的类型）；
    /// 必填的没填不在这里报错，记在步骤上交给引擎（只有它被用到时才算这个网络出错）。
    fn compile_net(&self) -> Vec<Step> {
        let mut order: Vec<u32> = Vec::new();
        fn visit(g: &Graph, id: u32, order: &mut Vec<u32>) {
            if order.contains(&id) {
                return;
            }
            let Some(node) = g.node(id) else { return };
            for up in node.inputs.iter().flatten() {
                visit(g, *up, order);
            }
            order.push(id);
        }
        for node in &self.nodes {
            visit(self, node.id, &mut order);
        }
        order
            .into_iter()
            .filter_map(|id| self.node(id))
            .map(|node| {
                let (params, error) = match step_params(node) {
                    Ok((params, _)) => (params, String::new()),
                    Err(msg) => (Vec::new(), msg),
                };
                let ports = (0..node.kind.params.len())
                    .filter_map(|i| node.wired(i).map(|up| (node.kind.params[i].name.clone(), up)))
                    .collect();
                let role = node.kind.role.clone();
                let order = self.output_rank(node.id);
                // 节点里面还有网络（eval）：一并编进去，引擎先算里面的，代码里用 outputs("结果") 取
                let net = node.net.as_ref().map(|g| g.compile_net()).unwrap_or_default();
                Step { id: node.id, code: node.kind.code.clone(), params, ports, role, order, error, net, ..Step::default() }
            })
            .collect()
    }

    /// 输出节点在同角色的输出里排第几（从 0 起）：画布上从上到下，同一高度从左到右。
    pub fn output_rank(&self, id: u32) -> usize {
        use std::cmp::Ordering;
        let (Some(i), Some(node)) = (self.index_of(id), self.node(id)) else { return 0 };
        let before = |j: usize, o: &Node| match (o.y, o.x).partial_cmp(&(node.y, node.x)) {
            Some(Ordering::Less) => true,
            Some(Ordering::Equal) => j < i,
            _ => false,
        };
        self.nodes.iter().enumerate().filter(|(j, o)| o.kind.role == node.kind.role && before(*j, o)).count()
    }

    /// 从源节点到 target 导出成能直接用 python 运行的脚本。prelude 是引擎里执行节点代码的那几个函数
    /// （`run_code` 和它用到的参数转换），每个节点一段：`nodes["名字"] = run_code(pl, {代码, 参数}, [输入])`。
    pub fn python_script(&self, target: u32, prelude: &str) -> Result<String, CompileError> {
        let steps = self.compile(target)?;
        let title = |id: u32| self.node(id).map_or(format!("n{id}"), |n| n.title.clone());
        let mut out = format!(
            "# 由 NodeSheet 从节点图生成：从源节点一直算到 {}。\n\
             # 需要 python 和 polars（读 Excel 还要 fastexcel）；运行：python 这个文件.py\n\
             import ast\n\
             import polars as pl\n\n\
             # ---- 执行节点代码的函数（和查看器的引擎一样）：ch(\"参数名\") 取参数，df ~ df4 是输入\n\n",
            title(target)
        );
        out += prelude.trim_end();
        out += "\n\n\nnodes = {}\n";
        for step in &steps {
            let Some(node) = self.node(step.id) else { continue };
            let inputs: Vec<String> =
                step.inputs.iter().map(|i| i.map_or(String::from("None"), |id| format!("nodes[{}]", py_str(&title(id))))).collect();
            out += &format!(
                "\n# ---- {}（{}）\nnodes[{}] = run_code(pl, {}, [{}])\n",
                node.title,
                node.kind.name,
                py_str(&node.title),
                step_py(step, "", Some(node)),
                inputs.join(", ")
            );
        }
        out += &format!("\n\nresult = nodes[{}].collect()\nprint(result)\n", py_str(&title(target)));
        Ok(out)
    }

    /// 节点面板里的按钮被点了：要执行的步骤（最后一步是这个节点自己，引擎带着 action 再执行它一次）。
    pub fn compile_action(&self, id: u32, action: &str) -> Result<Vec<Step>, CompileError> {
        let node = self.node(id).ok_or(fail(id, "节点不存在"))?;
        let button = node.kind.param(action).filter(|&i| node.kind.params[i].kind == ParamKind::Button);
        button.ok_or(fail(id, format!("这个节点没有叫 {action} 的按钮")))?;
        self.compile(id)
    }
}

/// 节点要传给引擎的参数，和它读的文件的修改时间。必填的没填时返回原因。
fn step_params(node: &Node) -> Result<(Vec<StepParam>, u128), String> {
    let def = node.kind();
    let mut params = Vec::new();
    let mut stamp = 0;
    for (i, (p, value)) in def.params.iter().zip(&node.params).enumerate() {
        let visible = node.visible(i);
        if p.required && visible && p.is_blank(value) && node.wired(i).is_none() {
            let what = if p.kind == ParamKind::File { "选择文件" } else if p.port { "连接或填写" } else { "填写" };
            return Err(format!("{what}“{}”后按回车运行", p.label));
        }
        if p.kind == ParamKind::File && visible {
            stamp = stamp.max(mtime(value.trim()));
        }
        if p.kind == ParamKind::Button || p.name == def.code_param {
            continue;
        }
        // 不显示的参数按默认值算：换了方式以后，别的方式里填过的东西不该起作用
        let value = if visible { value.clone() } else { p.default.clone() };
        params.push(StepParam { name: p.name.clone(), kind: p.engine_kind(), label: p.label.clone(), value, default: p.default.clone() });
    }
    Ok((params, stamp))
}

/// 节点里面的网络 → 步骤：当前上下文用到的几张网络接成一张（id 不重），在每组里算的标上 each；
/// 不是这张网络用的输出节点（比如分组键那张里的输出列）去掉角色，不交给上下文。
fn compile_nets(node: &Node) -> Vec<Step> {
    let group = !node.kind.expr && node.value("context") == "group_by";
    let mut steps = Vec::new();
    for &env in node.envs() {
        let Some(g) = node.net_in(env) else { continue };
        let roles = node.env_roles(env);
        for mut s in g.compile_net() {
            if !roles.iter().any(|(role, ..)| *role == s.role) {
                s.role.clear();
            }
            s.each = group && env != Env::Keys;
            steps.push(s);
        }
    }
    steps
}

/// 一个步骤写成 Python 的字典（导出脚本用）：代码、参数，expr 节点还有里面的网络。owner：步骤是这个节点的（找网络里节点的名字）。
fn step_py(step: &Step, indent: &str, owner: Option<&Node>) -> String {
    let params: Vec<String> = step
        .params
        .iter()
        .map(|p| format!("{indent}        {}: [{}, {}, {}],", py_str(&p.name), py_str(p.kind), py_str(&p.value), py_str(&p.label)))
        .collect();
    let params = if params.is_empty() { String::from("{}") } else { format!("{{\n{}\n{indent}    }}", params.join("\n")) };
    // 代码尽量写成三引号的原样文字，好读；里面有三引号或以反斜杠结尾时退回普通字符串
    let code = step.code.trim_end();
    let code = if code.contains("\'\'\'") || code.ends_with('\\') || !indent.is_empty() && code.contains('\n') {
        py_str(code)
    } else if indent.is_empty() {
        format!("r\'\'\'\n{code}\n\'\'\'")
    } else {
        format!("r\'\'\'{code}\'\'\'")
    };
    let mut fields = vec![format!("{indent}    \"id\": {}", step.id), format!("{indent}    \"code\": {code}"), format!("{indent}    \"params\": {params}")];
    if !step.ports.is_empty() {
        let ports: Vec<String> = step.ports.iter().map(|(name, up)| format!("{}: {up}", py_str(name))).collect();
        fields.push(format!("{indent}    \"ports\": {{{}}}", ports.join(", ")));
    }
    if !step.role.is_empty() {
        fields.push(format!("{indent}    \"role\": {}, \"order\": {}", py_str(&step.role), step.order));
    }
    if !step.error.is_empty() {
        fields.push(format!("{indent}    \"error\": {}", py_str(&step.error)));
    }
    if step.each {
        fields.push(format!("{indent}    \"each\": True"));
    }
    if !step.net.is_empty() {
        let title = |id: u32| owner.and_then(|n| n.sub_node(id)).map_or(String::new(), |n| n.title.clone());
        let inner = format!("{indent}        ");
        let items: Vec<String> =
            step.net.iter().map(|s| format!("{inner}# {}\n{inner}{}", title(s.id), step_py(s, &inner, None))).collect();
        fields.push(format!("{indent}    \"net\": [\n{}\n{indent}    ]", items.join(",\n")));
    }
    format!("{indent}{{\n{},\n{indent}}}", fields.join(",\n")).trim_start().to_string()
}

/// 拆好的节点各在第几层：只接常量的是 0，用到别的节点的比它们都大 1（下标小的先建，子节点总在前面）。
fn parsed_depths(items: &[Value]) -> Vec<usize> {
    let mut depths: Vec<usize> = Vec::with_capacity(items.len());
    for item in items {
        let children = item["ports"].as_object().into_iter().flatten().filter_map(|(_, c)| c.as_u64());
        let depth = children.filter_map(|c| depths.get(c as usize).map(|d| d + 1)).max().unwrap_or(0);
        depths.push(depth);
    }
    depths
}

/// 点 p 到线段 ab 的距离。
fn segment_distance(p: (f32, f32), a: (f32, f32), b: (f32, f32)) -> f32 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    let t = if len2 == 0.0 { 0.0 } else { (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / len2).clamp(0.0, 1.0) };
    ((p.0 - a.0 - t * dx).powi(2) + (p.1 - a.1 - t * dy).powi(2)).sqrt()
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::nodes::ParamDef;

    pub fn kind(name: &str) -> Arc<NodeType> {
        nodes::find(name).unwrap_or_else(|| panic!("没有 {name} 节点"))
    }

    fn value_of<'a>(step: &'a Step, name: &str) -> &'a str {
        &step.params.iter().find(|p| p.name == name).unwrap().value
    }

    #[test]
    fn add_chains_after_selected() {
        let mut g = Graph::for_file(Path::new("a.csv"), "");
        let read = g.primary().unwrap().id;
        let f = g.add(&kind("select"));
        assert_eq!(g.node(f).unwrap().inputs, vec![Some(read)]);
        assert_eq!(g.display, Some(f));
        assert_eq!(g.node(f).unwrap().title, "select1");
        // 指定位置：放在那，照样接在选中节点后面
        let s = g.add_at(&kind("unique"), Some((300.0, 40.0)));
        let node = g.node(s).unwrap();
        assert_eq!((node.x, node.y, node.inputs[0]), (300.0, 40.0, Some(f)));
        // 名字不重复：已有 select1 就叫 select2
        let f2 = g.add(&kind("select"));
        assert_eq!(g.node(f2).unwrap().title, "select2");
    }

    #[test]
    fn source_node_follows_the_file_extension() {
        let g = Graph::for_file(Path::new("a.csv"), "");
        assert_eq!((g.nodes[0].mode(), g.nodes[0].value("separator")), ("CSV", ","));
        assert!(!g.nodes[0].reads_excel());
        assert_eq!(Graph::for_file(Path::new("a.tsv"), "").nodes[0].value("separator"), "\\t");
        let mut g = Graph::for_file(Path::new("a.XLSX"), "明细");
        let node = &mut g.nodes[0];
        assert!(node.reads_excel());
        assert_eq!((node.mode(), node.value("sheet_name"), node.value("infer_schema_length")), ("Excel", "明细", "100"));
        // 不显示的参数按默认值算
        node.set("separator", ";");
        assert_eq!(node.value("separator"), ",");
        node.set("source", "b.parquet");
        node.sync_format();
        assert_eq!(node.mode(), "Parquet");
        node.set("source", "b.dat"); // 认不出的扩展名：格式不动
        node.sync_format();
        assert_eq!(node.mode(), "Parquet");
        assert!(!node.set("没有这个参数", "x"));
    }

    #[test]
    fn compile_orders_upstream_first_and_checks_params() {
        let mut g = Graph::for_file(Path::new("a.csv"), "");
        let f = g.add(&kind("select"));
        assert_eq!(g.compile(f).unwrap_err().node, f); // 列没填
        assert!(g.node_mut(f).unwrap().set("columns", "\"a\""));
        let h = g.add(&kind("slice"));
        let steps = g.compile(h).unwrap();
        assert_eq!(steps.iter().map(|s| s.id).collect::<Vec<_>>(), vec![1, f, h]);
        assert_eq!(steps[1].code, kind("select").code);
        assert_eq!(steps[1].params.iter().map(|p| (p.name.as_str(), p.kind, p.value.as_str())).collect::<Vec<_>>(), [
            ("mode", "menu", "select"),
            ("columns", "columns", "\"a\""),
        ]);
        // 不去空白的文本参数传 raw；不显示的参数传默认值
        assert_eq!(steps[0].params.iter().find(|p| p.name == "separator").unwrap().kind, "raw");
        g.node_mut(h).unwrap().set("k", "3");
        assert_eq!((value_of(&steps[2], "mode"), value_of(&g.compile(h).unwrap()[2], "k")), ("head", "10"));
    }

    #[test]
    fn bypass_passes_input_through() {
        let mut g = Graph::for_file(Path::new("a.csv"), "");
        let f = g.add(&kind("select"));
        g.node_mut(f).unwrap().bypass = true;
        let step = &g.compile(f).unwrap()[1];
        assert_eq!((step.code.as_str(), step.params.len()), ("out = df", 0));
    }

    #[test]
    fn code_node_runs_its_own_parameter() {
        let mut g = Graph::for_file(Path::new("a.csv"), "");
        let c = g.add(&kind("code"));
        g.node_mut(c).unwrap().set("code", "out = df.head(3)");
        let step = &g.compile(c).unwrap()[1];
        assert_eq!((step.code.as_str(), step.params.len()), ("out = df.head(3)", 0));
        assert_eq!(g.code_preview(c), "");
        assert_eq!(g.code_preview(1), kind("read").code.trim_end());
    }

    #[test]
    fn buttons_compile_to_an_action() {
        let mut g = Graph::for_file(Path::new("a.csv"), "");
        let w = g.add(&kind("write"));
        // 没填路径也能当直通节点显示；按钮不当参数传
        let steps = g.compile_action(w, "write").unwrap();
        assert_eq!(steps.len(), 2);
        assert!(steps[1].params.iter().all(|p| p.name != "write"));
        assert!(g.compile_action(w, "file").is_err() && g.compile_action(1, "write").is_err());
        g.node_mut(w).unwrap().set("file", "D:/out/结果.ndjson");
        g.node_mut(w).unwrap().sync_format();
        assert_eq!((g.node(w).unwrap().mode(), g.node(w).unwrap().summary().as_str()), ("JSON", "结果.ndjson"));
    }

    #[test]
    fn connect_rejects_cycles_and_remove_unlinks() {
        let mut g = Graph::for_file(Path::new("a.csv"), "");
        let a = g.add(&kind("slice"));
        let b = g.add(&kind("slice"));
        assert!(!g.connect(b, a, 0));
        assert!(!g.connect(a, a, 0));
        g.remove(a);
        assert_eq!(g.node(b).unwrap().inputs, vec![None]);
        assert_eq!(g.compile(b).unwrap_err().node, b);
    }

    #[test]
    fn multi_input_nodes_need_their_inputs() {
        let mut g = Graph::for_file(Path::new("a.xlsx"), "明细");
        let read = g.primary().unwrap().id;
        let j = g.add(&kind("join"));
        assert!(g.compile(j).is_err());
        assert!(g.connect(read, j, 1));
        assert_eq!(g.compile(j).unwrap().len(), 2);
        // concat：前两个输入必须连，后两个随意
        let c = g.add(&kind("concat"));
        assert!(g.compile(c).is_err());
        assert!(g.connect(read, c, 1) && g.connect(read, c, 3));
        assert_eq!(g.compile(c).unwrap().last().unwrap().inputs, vec![Some(j), Some(read), None, Some(read)]);
    }

    #[test]
    fn rename_bounds_and_script() {
        let mut g = Graph::for_file(Path::new("a.csv"), "");
        let read = g.primary().unwrap().id;
        let f = g.add(&kind("select"));
        assert_eq!(g.rename(f, "  只要大单 "), Ok(()));
        assert_eq!(g.node(f).unwrap().title, "只要大单");
        assert!(g.rename(f, "read1").is_err() && g.rename(f, " ").is_err());
        // 范围连着右边的名字
        let (n0, n1) = (&g.nodes[0], &g.nodes[1]);
        let (l, t, r, b) = g.bounds_of(None).unwrap();
        assert_eq!((l, t), (n0.x.min(n1.x), n0.y.min(n1.y)));
        assert!(r >= n1.x + NODE_W + LABEL_W && b == n0.y.max(n1.y) + NODE_H);
        // 脚本：每个节点一段，输入按名字引用；没填条件时导不出
        assert!(g.python_script(f, "def run_code(): pass").is_err());
        g.node_mut(f).unwrap().set("columns", "\"a\"");
        let script = g.python_script(f, "def run_code(): pass").unwrap();
        assert!(script.contains("def run_code(): pass"));
        assert!(script.contains("nodes[\"read1\"] = run_code(pl, {"), "{script}");
        assert!(script.contains("}, [nodes[\"read1\"]])"), "{script}");
        assert!(script.ends_with("result = nodes[\"只要大单\"].collect()\nprint(result)\n"));
        let _ = read;
    }

    #[test]
    fn wires_can_be_hit_dropped_and_removed() {
        let mut g = Graph::for_file(Path::new("a.csv"), "");
        let read = g.primary().unwrap().id;
        let (rx, ry) = (g.nodes[0].x, g.nodes[0].y);
        let h = g.add(&kind("slice"));
        let (hx, hy) = (g.node(h).unwrap().x, g.node(h).unwrap().y);
        // 连线中点附近能点中，离远了点不中
        let mid = ((rx + hx) / 2.0 + NODE_W / 2.0, (ry + PORT_OUT_Y + hy + PORT_IN_Y) / 2.0);
        assert_eq!(g.wire_at(mid.0 + 3.0, mid.1, 6.0), Some((h, 0)));
        assert_eq!(g.wire_at(mid.0 + 40.0, mid.1, 6.0), None);
        g.selected_wire = Some((h, 0));
        assert!(g.disconnect(h, 0));
        assert_eq!((g.selected_wire, g.node(h).unwrap().inputs[0]), (None, None));
        assert!(!g.disconnect(h, 0));
        assert_eq!(g.nodes.len(), 2); // 节点都还在
        // 拖到节点身上：接第一个空输入；对准端口：接那个端口
        let c = g.add(&kind("code"));
        g.node_mut(c).unwrap().inputs[0] = None;
        let (cx, cy) = (g.node(c).unwrap().x, g.node(c).unwrap().y);
        assert_eq!(g.input_at(cx + 100.0, cy + 30.0), Some((c, 0)));
        assert_eq!(g.input_at(cx + port_x(4, 2) + 3.0, cy + 6.0), Some((c, 2)));
        assert_eq!(g.input_at(cx + 500.0, cy), None);
        assert_eq!(g.input_at(rx + 50.0, ry + 30.0), None); // 读取节点没有输入
        assert_eq!(g.output_at(rx + 50.0, ry + 30.0), Some(read));
    }

    /// 一个不在注册表里的类型：head(n)。
    fn top_type(source: Source) -> NodeType {
        let mut t = NodeType::new("my_top");
        t.source = source;
        t.desc = String::from("前 N 行");
        t.params = vec![ParamDef::new("n", ParamKind::Int)];
        t.params[0].default = String::from("5");
        t.code = String::from("out = df.head(ch(\"n\"))\n");
        t
    }

    #[test]
    fn graph_survives_save_and_load() {
        let mut g = Graph::for_file(Path::new("a.xlsx"), "明细");
        let read = g.primary().unwrap().id;
        let f = g.add(&kind("select"));
        g.node_mut(f).unwrap().set("columns", "\"a\"");
        let j = g.add(&kind("join"));
        g.connect(read, j, 1);
        let node = g.node_mut(j).unwrap();
        (node.bypass, node.x, node.y) = (true, 300.0, 120.5);
        g.selected = Some(f);

        let text = serde_json::to_string(&g.to_json()).unwrap();
        let mut back = Graph::from_json(&serde_json::from_str(&text).unwrap()).unwrap();
        assert_eq!(back.file, "a.xlsx");
        assert_eq!((back.selected, back.display), (Some(f), Some(j)));
        assert!(back.is_native(read) && back.primary().is_some());
        let node = back.node(j).unwrap();
        assert_eq!(node.inputs, vec![Some(f), Some(read)]);
        assert_eq!((node.bypass, node.x, node.y, node.title.as_str()), (true, 300.0, 120.5, "join1"));
        assert_eq!(back.node(f).unwrap().params, g.node(f).unwrap().params);
        assert_eq!(back.add(&kind("slice")), j + 1); // 新节点的 id 接着往后排

        // 不认识的类型跳过，连到它的线断开；缺的参数用默认值补，多出来的忽略
        let mut v = g.to_json();
        v["nodes"][1]["type"] = json!("没有这种节点");
        v["nodes"][0]["params"] = json!({"source": "a.xlsx", "早就没有的参数": "x"});
        let back = Graph::from_json(&v).unwrap();
        assert_eq!(back.nodes.len(), 2);
        assert_eq!(back.missing, [(String::from("select1"), String::from("没有这种节点"))]);
        assert_eq!(back.node(j).unwrap().inputs, vec![None, Some(read)]);
        assert_eq!((back.nodes[0].value("source"), back.nodes[0].value("encoding")), ("a.xlsx", "utf8"));
        assert_eq!(back.selected, None);
        assert!(Graph::from_json(&json!({"nodes": 1})).is_none());

        // 上一版的工程：kind + 按位置存的参数；源节点的文件还认得出来
        let old = json!({"file": "a.xlsx", "native_params": ["a.xlsx", "Excel"], "nodes": [
            {"id": 1, "kind": "read", "params": ["a.xlsx", "Excel", "明细"], "inputs": [], "primary": true},
            {"id": 2, "kind": "select", "params": ["select", "\"a\""], "inputs": [1]},
        ]});
        let back = Graph::from_json(&old).unwrap();
        assert_eq!((back.nodes.len(), back.nodes[0].value("source")), (2, "a.xlsx"));
        assert_eq!(back.nodes[1].kind.name, "select");
    }

    #[test]
    fn custom_types_travel_with_the_project() {
        // 节点自己的定义（code 节点加了参数）和用户类型的定义都存进工程
        let mut g = Graph::for_file(Path::new("a.csv"), "");
        let private = g.add(&Arc::new(top_type(Source::Instance)));
        g.node_mut(private).unwrap().set("n", "7");
        let mut shared = top_type(Source::User);
        shared.name = String::from("graph_test_shared_top");
        let shared = g.add(&Arc::new(shared));
        let v = g.to_json();
        assert!(v["nodes"][1]["def"].as_str().unwrap().contains("df.head"));
        assert!(v["nodes"][2]["def"].is_null());
        assert!(v["types"]["graph_test_shared_top"].as_str().unwrap().contains("前 N 行"));

        // 这台机器上没有这个用户类型：打开工程时注册成工程层的类型
        assert!(nodes::find("graph_test_shared_top").is_none());
        let back = Graph::from_json(&v).unwrap();
        assert_eq!(back.nodes.len(), 3);
        assert_eq!((back.node(private).unwrap().kind.source, back.node(private).unwrap().value("n")), (Source::Instance, "7"));
        assert_eq!(back.node(shared).unwrap().kind.source, Source::Project);
        assert_eq!(back.compile(private).unwrap()[1].code, "out = df.head(ch(\"n\"))\n");
        nodes::remove("graph_test_shared_top", Source::Project);
    }

    #[test]
    fn retype_keeps_values_by_name() {
        let mut g = Graph::for_file(Path::new("a.csv"), "");
        let id = g.add(&Arc::new(top_type(Source::User)));
        g.node_mut(id).unwrap().set("n", "7");
        // 类型改了：加了一个菜单、n 还在、输入变成两个
        let mut edited = top_type(Source::User);
        let mut how = ParamDef::new("how", ParamKind::Menu);
        how.options = vec![String::from("a"), String::from("b")];
        how.default = String::from("a");
        edited.params.insert(0, how);
        edited.inputs = 2;
        let edited = Arc::new(edited);
        g.rebind(&edited);
        let node = g.node(id).unwrap();
        assert!(Arc::ptr_eq(&node.kind, &edited));
        assert_eq!((node.params.as_slice(), node.inputs.len()), (&[String::from("a"), String::from("7")][..], 2));
        // 菜单的选项没了：回到默认；别的类型、节点自己的定义不受影响
        g.node_mut(id).unwrap().set("how", "b");
        let mut fewer = (*edited).clone();
        fewer.params[0].options.pop();
        g.rebind(&Arc::new(fewer));
        assert_eq!(g.node(id).unwrap().value("how"), "a");
        assert_eq!(g.nodes[0].kind.name, "read");
    }

    #[test]
    fn source_node_is_native_until_its_options_change() {
        let mut g = Graph::for_file(Path::new("a.xlsx"), "明细");
        g.nodes[0].sheets = vec![String::from("明细"), String::from("汇总")];
        assert!(g.is_native(1));
        g.nodes[0].set("sheet_name", "汇总"); // 换 sheet：查看器跟着切
        assert!(g.is_native(1));
        g.nodes[0].set("has_header", "False");
        assert!(!g.is_native(1));
        g.nodes[0].set("has_header", "True");
        g.nodes[0].set("source", "b.xlsx");
        assert!(!g.is_native(1));
        let f = g.add(&kind("slice"));
        assert!(!g.is_native(f));
    }

    /// 查看器认出来的 CSV 读法（GBK、分号）填进源节点，而且算“没改过”：显示它照样用查看器自己的数据。
    #[test]
    fn source_node_takes_read_hints() {
        let hints = [("encoding", String::from("gb18030")), ("separator", String::from(";"))];
        let g = Graph::for_file_with(Path::new("a.csv"), "", &hints);
        let read = &g.nodes[0];
        assert_eq!((read.value("encoding"), read.raw("separator")), ("gb18030", ";"));
        assert!(g.is_native(read.id));
        // 查看器读不了的格式：源节点交给引擎读
        let g = Graph::for_engine_file(Path::new("a.ndjson"));
        assert_eq!(g.nodes[0].value("format"), "JSON");
        assert!(!g.is_native(g.nodes[0].id));
    }

    #[test]
    fn pick_and_summary() {
        let mut g = Graph::for_file(Path::new("D:/数据/a.xlsx"), "明细");
        assert_eq!(g.nodes[0].summary(), "a.xlsx · 明细");
        let f = g.add(&kind("select"));
        assert_eq!(g.node(f).unwrap().summary(), kind("select").desc);
        g.node_mut(f).unwrap().set("columns", "\"金额\", \"城市\"");
        assert_eq!(g.node(f).unwrap().value("columns"), "\"金额\", \"城市\"");
        assert_eq!(g.node(f).unwrap().summary(), "\"金额\", \"城市\"");
        // 页签不是默认那个才写进摘要；下拉式的“方式”总是写
        g.node_mut(f).unwrap().set("mode", "drop");
        assert_eq!(g.node(f).unwrap().summary(), "drop · \"金额\", \"城市\"");
        let h = g.add(&kind("slice"));
        assert_eq!(g.node(h).unwrap().summary(), "head · 100");
        g.node_mut(h).unwrap().set("mode", "reverse");
        assert_eq!(g.node(h).unwrap().summary(), "reverse");
        // 选列框怎么选：按类型，定义里写了的按写的
        let pick = |t: &str, p: &str| kind(t).params.iter().find(|d| d.name == p).unwrap().pick();
        assert_eq!(pick("select", "columns"), nodes::Pick::Select);
        assert_eq!(pick("join", "on"), nodes::Pick::Select);
        assert_eq!(pick("read", "columns"), nodes::Pick::Names);
        assert_eq!(pick("transpose", "column_names"), nodes::Pick::One);
        assert_eq!(pick("rename", "mapping"), nodes::Pick::One);
        assert_eq!(pick("cast_columns", "schema"), nodes::Pick::Names);
        assert_eq!(pick("join", "predicates"), nodes::Pick::Off);
        assert_eq!(pick("slice", "n"), nodes::Pick::Off);
        assert_eq!(kind("join").params.iter().find(|d| d.name == "right_on").unwrap().pick_from, 1, "右表的键取第二个输入的列");
        assert_eq!(expr_kind("col").params[0].pick(), nodes::Pick::Select);
        assert!(expr_kind("cast").params.iter().all(|d| d.pick() == nodes::Pick::Off), "端口连线就行");
    }

    pub fn expr_kind(name: &str) -> Arc<NodeType> {
        nodes::find_expr(name).unwrap_or_else(|| panic!("没有 {name} 表达式节点"))
    }

    /// expr 节点里面的网络。
    pub fn net_of(g: &mut Graph, id: u32) -> &mut Graph {
        g.node_mut(id).and_then(|n| n.net.as_deref_mut()).expect("不是 expr 节点")
    }

    /// expr 节点里面 env 那张网络（group_by 的分组键 / 筛选组）。
    pub fn env_net(g: &mut Graph, id: u32, env: Env) -> &mut Graph {
        g.node_mut(id).and_then(|n| n.net_in_mut(env)).unwrap_or_else(|| panic!("没有 {env:?} 网络"))
    }

    #[test]
    fn expr_node_starts_with_a_column_wired_to_an_output() {
        let mut g = Graph::for_file(Path::new("a.csv"), "");
        let e = g.add(&kind("expr"));
        assert_eq!(g.node(e).unwrap().summary(), "with_columns · 1 个输出列");
        let net = net_of(&mut g, e);
        assert!(net.vop && net.selected.is_none());
        let names: Vec<&str> = net.nodes.iter().map(|n| n.kind.name.as_str()).collect();
        assert_eq!(names, ["col", "output"]);
        let (col, out) = (net.nodes[0].id, net.nodes[1].id);
        assert_eq!(net.node(out).unwrap().inputs, vec![Some(col)]);
        assert_eq!(net.node(out).unwrap().wired(0), Some(col));
        assert!(net.nodes[1].x > net.nodes[0].x); // 从左往右

        // 编译：expr 节点的步骤带着网络；列还没填 → 那一步记着原因，交给引擎
        let steps = g.compile(e).unwrap();
        let step = steps.last().unwrap();
        assert!(step.net.iter().all(|s| !s.each));
        assert_eq!(step.net.iter().map(|s| s.id).collect::<Vec<_>>(), vec![col, out]);
        assert!(step.net[0].error.contains("列"));
        assert_eq!(step.net[1].ports, vec![(String::from("value"), col)]);
        assert_eq!((step.net[1].role.as_str(), step.net[1].order), ("列", 0));
        // 连了线的必填端口不算没填
        assert!(step.net[1].error.is_empty());

        // 换成 group_by：三张网络——分组键（建节点时就有了：col → 分组键）、聚合（原来那张）、筛选组（空的）
        g.node_mut(e).unwrap().set("context", "group_by");
        g.node_mut(e).unwrap().ensure_outputs();
        assert_eq!(g.node(e).unwrap().envs(), [Env::Keys, Env::Main, Env::Having]);
        assert_eq!(g.node(e).unwrap().summary(), "group_by · 1 个分组键、1 个聚合");
        let keys = env_net(&mut g, e, Env::Keys);
        let names: Vec<&str> = keys.nodes.iter().map(|n| n.kind.name.as_str()).collect();
        assert_eq!(names, ["col", "key"]);
        let (key_col, key) = (keys.nodes[0].id, keys.nodes[1].id);
        assert!(key_col > Env::Keys.base() && key > Env::Keys.base(), "几张网络的 id 不重");
        assert!(env_net(&mut g, e, Env::Having).nodes.is_empty(), "having 的条件是可选的，不自动加");
        g.node_mut(e).unwrap().ensure_outputs(); // 已经有了就不再加
        assert_eq!(net_of(&mut g, e).nodes.len(), 2);
        assert_eq!(env_net(&mut g, e, Env::Keys).nodes.len(), 2);
        // 编译：几张网络接成一张，聚合那张里的在每组里算；分组键那张里的输出列不交给上下文
        let stray = env_net(&mut g, e, Env::Keys).add_at(&expr_kind("output"), Some((400.0, 200.0)));
        let steps = g.compile(e).unwrap();
        let step = |id: u32| steps.last().unwrap().net.iter().find(|s| s.id == id).unwrap();
        assert_eq!((step(key_col).each, step(key).each, step(col).each, step(out).each), (false, false, true, true));
        assert_eq!((step(key).role.as_str(), step(out).role.as_str(), step(stray).role.as_str()), ("键", "列", ""));
        assert!(g.node(e).unwrap().unused_role(Env::Keys, "列").unwrap().contains("分组键这张网络只用分组键"));
        env_net(&mut g, e, Env::Keys).remove(stray);

        // 分组后换成 sum：用不到聚合那张网络（提醒，不删），摘要写上；在每组里算的只剩筛选组
        g.node_mut(e).unwrap().set("then", "sum");
        let node = g.node(e).unwrap();
        assert_eq!(node.envs(), [Env::Keys, Env::Having]);
        assert_eq!(node.shown_env(Env::Main), Env::Keys);
        assert_eq!(node.summary(), "group_by · 1 个分组键 · sum");
        assert!(node.unused_role(Env::Main, "列").unwrap().contains("sum"));
        assert_eq!(node.unused_role(Env::Keys, "键"), None);
        assert!(g.compile(e).unwrap().last().unwrap().net.iter().all(|s| s.id > Env::Keys.base()));
        // 加一个 having 条件：摘要里有了，它在每组里算
        let cond = env_net(&mut g, e, Env::Having).add(&expr_kind("condition"));
        assert!(cond > Env::Having.base());
        assert_eq!(g.node(e).unwrap().summary(), "group_by · 1 个分组键、1 个条件 · sum");
        assert!(g.compile(e).unwrap().last().unwrap().net.iter().find(|s| s.id == cond).unwrap().each);
        g.node_mut(e).unwrap().set("context", "with_columns");
        assert_eq!(g.node(e).unwrap().unused_role(Env::Main, "条件").unwrap(), "with_columns 用不到条件");
        assert_eq!(g.node(e).unwrap().envs(), [Env::Main]);
    }

    #[test]
    fn sort_keys_rank_by_canvas_and_old_toggles_become_tabs() {
        let mut g = Graph::for_file(Path::new("a.csv"), "");
        let e = g.add(&kind("expr"));
        g.node_mut(e).unwrap().set("context", "sort");
        g.node_mut(e).unwrap().ensure_outputs();
        let net = net_of(&mut g, e);
        let first = net.nodes.iter().find(|n| n.kind.role == "排序").unwrap().id;
        net.selected = None;
        let second = net.add_at(&expr_kind("sort_key"), Some((500.0, 900.0)));
        assert_eq!((net.output_rank(first), net.output_rank(second)), (0, 1));
        net.node_mut(second).unwrap().y = -100.0; // 挪到上面：先按它排
        assert_eq!((net.output_rank(first), net.output_rank(second)), (1, 0));
        net.node_mut(second).unwrap().set("order", "从大到小");
        assert_eq!(net.node(second).unwrap().summary(), "从大到小");

        // 上一版存的开关：filter 的 remove、排序键的 descending 换成页签
        let mut v = g.to_json();
        v["nodes"][1]["params"].as_object_mut().unwrap().remove("keep");
        v["nodes"][1]["params"]["remove"] = json!("True");
        let keys = v["nodes"][1]["net"]["nodes"].as_array_mut().unwrap();
        for n in keys.iter_mut().filter(|n| n["type"] == "sort_key") {
            n["params"].as_object_mut().unwrap().remove("order");
            n["params"]["descending"] = json!("True");
        }
        let back = Graph::from_json(&v).unwrap();
        let node = back.node(e).unwrap();
        assert_eq!(node.params[node.kind.param("keep").unwrap()], "去掉满足的");
        assert!(node.net.as_ref().unwrap().nodes.iter().filter(|n| n.kind.role == "排序").all(|n| n.value("order") == "从大到小"));
    }

    #[test]
    fn old_group_by_networks_are_split_when_loaded() {
        // 上一版的 group_by：一张网络里 城市 → 分组键、城市 → agg → 输出列、金额 → sum → 条件，外加一个没接上的节点
        let mut g = Graph::for_file(Path::new("a.csv"), "");
        let e = g.add(&kind("expr"));
        g.node_mut(e).unwrap().set("context", "group_by");
        let node = g.node_mut(e).unwrap();
        (node.key_net, node.having_net) = (None, None);
        let net = net_of(&mut g, e);
        let (city, out) = (net.nodes[0].id, net.nodes[1].id);
        net.selected = None;
        let key = net.add(&expr_kind("key"));
        net.connect(city, key, 0);
        net.selected = None;
        let unique = net.add(&expr_kind("agg"));
        net.connect(city, unique, 0);
        net.connect(unique, out, 0);
        net.selected = None;
        let amount = net.add(&expr_kind("col"));
        net.selected = Some(amount);
        let sum = net.add(&expr_kind("agg"));
        net.selected = None;
        let cond = net.add(&expr_kind("condition"));
        net.connect(sum, cond, 0);
        net.selected = None;
        let loose = net.add(&expr_kind("lit"));
        let v = g.to_json();
        assert!(v["nodes"][1]["key_net"].is_null());

        let back = Graph::from_json(&v).unwrap();
        let node = back.node(e).unwrap();
        let ids = |env: Env| -> Vec<u32> { node.net_in(env).unwrap().nodes.iter().map(|n| n.id).collect() };
        let base = |env: Env, ids: &[u32]| -> Vec<u32> { ids.iter().map(|id| id + env.base()).collect() };
        // 城市两边都用：各留一份；只给分组键 / 条件用的搬走；没接上的留在聚合那张
        assert_eq!(ids(Env::Main), [city, out, unique, loose]);
        assert_eq!(ids(Env::Keys), base(Env::Keys, &[city, key]));
        assert_eq!(ids(Env::Having), base(Env::Having, &[amount, sum, cond]));
        let keys = node.net_in(Env::Keys).unwrap();
        assert_eq!(keys.node(key + Env::Keys.base()).unwrap().inputs, vec![Some(city + Env::Keys.base())]);
        assert_eq!(keys.node(city + Env::Keys.base()).unwrap().title, "col1");
        // 再存再读：照新的格式，不再拆
        let again = Graph::from_json(&back.to_json()).unwrap();
        assert_eq!(again.node(e).unwrap().net_in(Env::Keys).unwrap().nodes.len(), 2);
    }

    #[test]
    fn switching_a_fresh_net_to_group_by_counts_rows_per_key() {
        let mut g = Graph::for_file(Path::new("a.csv"), "");
        let e = g.add(&kind("expr"));
        g.node_mut(e).unwrap().set("context", "group_by");
        g.node_mut(e).unwrap().switch_context();
        let net = net_of(&mut g, e);
        let names: Vec<&str> = net.nodes.iter().map(|n| n.kind.name.as_str()).collect();
        assert_eq!(names, ["output"], "那一列挪去当分组键了");
        assert_eq!(net.nodes[0].inputs, vec![None]);
        assert_eq!(net.nodes[0].value("value"), "pl.len()");
        let keys = env_net(&mut g, e, Env::Keys);
        let (col, key) = (keys.nodes[0].id, keys.nodes[1].id);
        assert_eq!(keys.node(key).unwrap().inputs, vec![Some(col)]);

        // 动过的网络不重新连：只补缺的
        let mut g = Graph::for_file(Path::new("a.csv"), "");
        let e = g.add(&kind("expr"));
        let net = net_of(&mut g, e);
        let out = net.nodes[1].id;
        net.node_mut(out).unwrap().set("name", "新");
        g.node_mut(e).unwrap().set("context", "group_by");
        g.node_mut(e).unwrap().switch_context();
        let net = net_of(&mut g, e);
        assert_eq!(net.node(out).unwrap().inputs, vec![Some(net.nodes[0].id)]);
    }

    #[test]
    fn expr_network_layout_and_wiring() {
        let mut net = Graph { vop: true, ..Graph::default() };
        let col = net.add(&expr_kind("col"));
        // 新节点放在选中节点右边，第一个端口接它
        let cmp = net.add(&expr_kind("compare"));
        let (a, b) = (net.node(col).unwrap(), net.node(cmp).unwrap());
        assert_eq!((b.x, b.y), (a.x + VOP_DX, a.y));
        assert_eq!(b.inputs, vec![Some(col), None]);
        assert_eq!(b.title, "compare1");
        let (x, y) = (b.x, b.y);
        // 生成的节点用短名字起名
        net.selected = None;
        let pad = net.add(&expr_kind("str.pad_start"));
        assert_eq!(net.node(pad).unwrap().title, "pad_start1");
        // 第二行是第二个端口（b）；落在头上接第一个空着的
        assert_eq!(net.input_at(x + 30.0, y + VOP_HEAD + VOP_ROW * 1.5), Some((cmp, 1)));
        assert_eq!(net.input_at(x + 30.0, y + 5.0), Some((cmp, 1)));
        assert_eq!(net.input_at(x + 30.0, y + VOP_HEAD + 3.0), Some((cmp, 0)));
        // 连线：中点附近点得中（左右弯的曲线）
        let (x0, y0) = net.out_port(net.node(col).unwrap());
        let (x3, y3) = net.in_port(net.node(cmp).unwrap(), 0);
        assert_eq!(net.wire_at((x0 + x3) / 2.0, (y0 + y3) / 2.0, 6.0), Some((cmp, 0)));
        // 摘要：连了线的端口不写；没连的写填的值
        net.node_mut(cmp).unwrap().set("b", "100");
        assert_eq!(net.node(cmp).unwrap().summary(), "== · 100");
        // 端口参数：连线优先，填的值是没连时用的
        assert_eq!(net.node(cmp).unwrap().wired(0), Some(col));
        assert_eq!(net.node(cmp).unwrap().wired(2), None);
        let (w, h) = net.size(net.node(cmp).unwrap());
        assert_eq!((w, h), (VOP_W, VOP_HEAD + 2.0 * VOP_ROW + 4.0));
        assert_eq!(net.size(net.node(col).unwrap()).1, VOP_HEAD + 6.0);
    }

    #[test]
    fn eval_nodes_nest_a_network() {
        let mut g = Graph::for_file(Path::new("a.csv"), "");
        let e = g.add(&kind("expr"));
        let net = net_of(&mut g, e);
        net.selected = None;
        let ev = net.add(&expr_kind("eval"));
        let inner = net.node_mut(ev).unwrap().net.as_deref_mut().expect("eval 节点里面有网络");
        assert!(inner.vop);
        let names: Vec<&str> = inner.nodes.iter().map(|n| n.kind.name.as_str()).collect();
        assert_eq!(names, ["element", "result"]);
        let (element, result) = (inner.nodes[0].id, inner.nodes[1].id);
        assert_eq!(inner.node(result).unwrap().inputs, vec![Some(element)]);
        // 摘要照普通节点写（方式是默认的就不写）；“结果”是它要的输出
        let node = net_of(&mut g, e).node(ev).unwrap();
        assert_eq!(node.env_roles(Env::Main), &[("结果", "result", true)]);
        assert_eq!(node.envs(), [Env::Main]);
        assert_eq!(node.context_label(), "list.eval");
        assert_eq!(node.unused_role(Env::Main, "列").unwrap(), "list.eval 用不到输出列");

        // 编译：eval 的步骤带着里面的网络
        let steps = g.compile(e).unwrap();
        let step = steps.last().unwrap().net.iter().find(|s| s.id == ev).unwrap();
        assert_eq!(step.net.iter().map(|s| s.id).collect::<Vec<_>>(), [element, result]);
        assert_eq!(step.net[1].role, "结果");

        // 存进工程再读回来：里面的网络还在
        let back = Graph::from_json(&g.to_json()).unwrap();
        let inner = back.node(e).unwrap().net.as_ref().unwrap().node(ev).unwrap().net.as_ref().unwrap();
        assert_eq!(inner.nodes.len(), 2);
        assert_eq!(inner.node(result).unwrap().inputs, vec![Some(element)]);
    }

    #[test]
    fn parsed_text_builds_nodes_and_collapse_keeps_shared_inputs() {
        // 引擎拆好的 (pl.col("a") + 1) * pl.col("a")：同一个 col 节点用了两次
        let parsed = json!({
            "nodes": [
                {"type": "col", "params": {"columns": "\"a\""}, "ports": {}},
                {"type": "arith", "params": {"op": "+", "b": "1"}, "ports": {"a": 0}},
                {"type": "arith", "params": {"op": "*"}, "ports": {"a": 1, "b": 0}},
            ],
            "root": 2,
        });
        let mut net = Graph { vop: true, ..Graph::default() };
        let root = net.insert_parsed(&parsed, (500.0, 40.0), &[]).unwrap();
        assert_eq!(net.nodes.len(), 3);
        let (col, add) = (net.nodes[0].id, net.nodes[1].id);
        assert_eq!(net.node(root).unwrap().inputs, vec![Some(add), Some(col)]);
        assert_eq!(net.node(add).unwrap().value("b"), "1");
        // 从左往右：col（第 0 层）在 + 左边，+ 在 * 左边，* 在 at
        let x = |id: u32| net.node(id).unwrap().x;
        assert!(x(col) < x(add) && x(add) < x(root) && x(root) == 500.0);

        // 转成文字：+ 和 col 只给它用，一起收掉
        net.node_mut(root).unwrap().text = String::from("(pl.col(\"a\") + 1) * pl.col(\"a\")");
        let output = net.add_at(&expr_kind("output"), Some((800.0, 40.0)));
        net.connect(root, output, 0);
        let mut shared = Graph { vop: true, ..Graph::default() };
        let root2 = shared.insert_parsed(&parsed, (500.0, 40.0), &[]).unwrap();
        let col2 = shared.nodes[0].id;
        let other = shared.add_at(&expr_kind("output"), Some((800.0, 200.0)));
        shared.connect(col2, other, 0);
        shared.node_mut(root2).unwrap().text = String::from("x");

        let snippet = net.collapse_to_text(root).unwrap();
        let names: Vec<&str> = net.nodes.iter().map(|n| n.kind.name.as_str()).collect();
        assert_eq!(names, ["output", "snippet"]);
        assert_eq!(net.node(output).unwrap().inputs, vec![Some(snippet)]);
        // col 还有别人在用：留着
        shared.collapse_to_text(root2).unwrap();
        let names: Vec<&str> = shared.nodes.iter().map(|n| n.kind.name.as_str()).collect();
        assert_eq!(names, ["col", "output", "snippet"]);
        // 输出节点、没有文字的节点不收
        assert_eq!(shared.collapse_to_text(other), None);
    }

    #[test]
    fn output_order_follows_the_canvas_and_networks_are_saved() {
        let mut g = Graph::for_file(Path::new("a.csv"), "");
        let e = g.add(&kind("expr"));
        let net = net_of(&mut g, e);
        let first = net.nodes[1].id;
        net.selected = None;
        let second = net.add_at(&expr_kind("output"), Some((300.0, 10.0))); // 比第一个高
        net.node_mut(second).unwrap().set("value", "pl.lit(1)");
        net.node_mut(net.nodes[0].id).unwrap().set("columns", "\"a\"");
        let steps = g.compile(e).unwrap();
        let order = |id: u32| steps.last().unwrap().net.iter().find(|s| s.id == id).unwrap().order;
        assert_eq!((order(second), order(first)), (0, 1));

        // 存进工程再读回来：网络、连线、参数都在
        let v = g.to_json();
        let back = Graph::from_json(&serde_json::from_str(&serde_json::to_string(&v).unwrap()).unwrap()).unwrap();
        let net = back.node(e).unwrap().net.as_ref().unwrap();
        assert!(net.vop);
        assert_eq!(net.nodes.len(), 3);
        assert_eq!(net.node(first).unwrap().inputs, vec![Some(net.nodes[0].id)]);
        assert_eq!(net.node(second).unwrap().value("value"), "pl.lit(1)");
        assert_eq!(net.nodes[0].value("columns"), "\"a\"");
        // 没存网络的 expr 节点（手改过的工程）：给它一个新的
        let mut v = g.to_json();
        v["nodes"][1]["net"] = Value::Null;
        let back = Graph::from_json(&v).unwrap();
        assert_eq!(back.node(e).unwrap().net.as_ref().unwrap().nodes.len(), 2);

        // 导出的脚本里带着网络
        let script = g.python_script(e, "def run_code(): pass").unwrap();
        assert!(script.contains("\"net\": ["), "{script}");
        assert!(script.contains("\"role\": \"列\", \"order\": 0"), "{script}");
        assert!(script.contains("# col1"), "{script}");
    }
}
