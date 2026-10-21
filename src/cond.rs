//! 筛选节点（filter）的条件编辑器：一个条件 = 一列 + 一个操作 + 一个值；条件之间“且 / 或”，可以分组（最多 5 层）。
//!
//! 参数里存的是条件树的 JSON（引擎把它变成 polars 表达式，见 engine.py 的 cond_expr）：
//!   {"join": "and", "items": [{"col": "城市", "op": "contains", "value": "北京"},
//!                             {"join": "or", "items": [{"col": "金额", "op": "gt", "value": "1000", "type": "number"}]}]}
//! 条件：col + op + value（type 是值的类型，文字不写）；分组：join + items。
//!
//! 界面上的条件是一张平表（每行带缩进、分组一行）：条件树一改就整张重排一遍（`push`），所以
//! 加法 / 删法 / 换列这些会换掉行的操作都要推迟到回调返回后（`defer`），不能在控件自己的回调里销毁它。

use crate::nodes::py_str;
use crate::*;
use serde_json::{Value, json};

// ---------------------------------------------------------------- 条件树

/// 一个条件：列 + 操作 + 值。
#[derive(Clone, PartialEq, Debug, Default)]
pub(crate) struct Test {
    pub col: String,
    /// 操作（见 OPS 的 tag）
    pub op: String,
    pub value: String,
    /// 值的类型：text 文字、number 数字、date 日期、time 时间、datetime 日期时间、bool 真假
    pub vtype: String,
}

/// 一组条件：组内怎么合（and = 且 / or = 或）。
#[derive(Clone, PartialEq, Debug)]
pub(crate) struct Group {
    pub join: String,
    pub items: Vec<Item>,
}

impl Default for Group {
    fn default() -> Self {
        Group { join: String::from("and"), items: Vec::new() }
    }
}

#[derive(Clone, PartialEq, Debug)]
pub(crate) enum Item {
    Test(Test),
    Group(Group),
}

/// 分组最多套几层（界面上的缩进到这里为止）。
const MAX_DEPTH: usize = 5;

// ---------------------------------------------------------------- 操作

const TEXT: u8 = 1;
const NUM: u8 = 2;
const TIME: u8 = 4;
const BOOL_KIND: u8 = 8;
/// 分类 / 枚举列：大小、包含这些 polars 都不认（要先用 cast_columns 转成文字）
const CAT: u8 = 16;
const ANY: u8 = TEXT | NUM | TIME | BOOL_KIND | CAT;

/// 值要填几个。
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum Val {
    /// 不用填（为空、为真…）
    None,
    /// 一个
    One,
    /// 两个（逗号分开，小的在前）
    Two,
    /// 一串（逗号分开）
    List,
}

pub(crate) struct OpDef {
    pub tag: &'static str,
    pub label: &'static str,
    pub val: Val,
    /// 适合哪些列（位或）的哪些操作；类型认不出来时全都列出来
    pub kinds: u8,
}

/// 操作表（顺序就是下拉里的顺序；和 engine.py 的 cond_test 一一对应）。
pub(crate) const OPS: &[OpDef] = &[
    OpDef { tag: "eq", label: "等于", val: Val::One, kinds: ANY },
    OpDef { tag: "ne", label: "不等于", val: Val::One, kinds: ANY },
    OpDef { tag: "is_true", label: "为真", val: Val::None, kinds: BOOL_KIND },
    OpDef { tag: "is_false", label: "为假", val: Val::None, kinds: BOOL_KIND },
    OpDef { tag: "contains", label: "包含", val: Val::One, kinds: TEXT },
    OpDef { tag: "ncontains", label: "不包含", val: Val::One, kinds: TEXT },
    OpDef { tag: "starts", label: "开头是", val: Val::One, kinds: TEXT },
    OpDef { tag: "ends", label: "结尾是", val: Val::One, kinds: TEXT },
    OpDef { tag: "regex", label: "匹配正则", val: Val::One, kinds: TEXT },
    OpDef { tag: "gt", label: "大于", val: Val::One, kinds: TEXT | NUM | TIME },
    OpDef { tag: "ge", label: "大于等于", val: Val::One, kinds: TEXT | NUM | TIME },
    OpDef { tag: "lt", label: "小于", val: Val::One, kinds: TEXT | NUM | TIME },
    OpDef { tag: "le", label: "小于等于", val: Val::One, kinds: TEXT | NUM | TIME },
    OpDef { tag: "between", label: "介于（含两端）", val: Val::Two, kinds: NUM | TIME },
    OpDef { tag: "in", label: "是其中之一", val: Val::List, kinds: TEXT | NUM | TIME | CAT },
    OpDef { tag: "is_null", label: "为空", val: Val::None, kinds: ANY },
    OpDef { tag: "not_null", label: "不为空", val: Val::None, kinds: ANY },
];

fn op_def(tag: &str) -> Option<&'static OpDef> {
    OPS.iter().find(|o| o.tag == tag)
}

pub(crate) fn op_label(tag: &str) -> &str {
    op_def(tag).map_or(tag, |o| o.label)
}

fn op_val(tag: &str) -> Val {
    op_def(tag).map_or(Val::One, |o| o.val)
}

/// 值的类型：下拉里的一项（写进 JSON 的值，标签）。文字是默认的，不写进 JSON。
const VALUE_TYPES: [(&str, &str); 6] =
    [("text", "文字"), ("number", "数字"), ("date", "日期"), ("time", "时间"), ("datetime", "日期时间"), ("bool", "真假")];

fn vtype_label(vtype: &str) -> &str {
    VALUE_TYPES.iter().find(|(tag, _)| *tag == vtype).map_or(vtype, |(_, label)| label)
}

/// 类型写法里最外层的类型名：`Datetime(time_unit='us')` → Datetime。
fn base_of(dtype: &str) -> &str {
    dtype.split('(').next().unwrap_or("").trim().trim_start_matches("pl.")
}

/// 一列的类型（polars 的完整写法）→ 属于哪一类；认不出来是 0（什么都列）。
fn bucket_of(dtype: &str) -> u8 {
    let base = base_of(dtype);
    match base {
        "" => 0,
        "String" | "Utf8" => TEXT,
        "Categorical" | "Enum" => CAT,
        "Boolean" => BOOL_KIND,
        "Date" | "Datetime" | "Time" | "Duration" => TIME,
        _ if base.starts_with("Int") || base.starts_with("UInt") || base.starts_with("Float") || base == "Decimal" => NUM,
        _ => 0,
    }
}

/// 值类型 → 它能用的操作（位）。操作下拉按这个筛：先选值按什么类型来，再选做什么。
fn value_mask(vtype: &str) -> u8 {
    match vtype {
        "number" => NUM,
        "date" | "time" | "datetime" => TIME,
        "bool" => BOOL_KIND,
        _ => TEXT,
    }
}

/// 换成这一列时默认选哪个值类型。
fn default_vtype(dtype: &str) -> &'static str {
    let base = base_of(dtype);
    match base {
        "Date" => "date",
        "Time" => "time",
        "Datetime" => "datetime",
        "Boolean" => "bool",
        "" => "text",
        _ if bucket_of(dtype) == NUM => "number",
        _ => "text",
    }
}

/// 这一列的值能按哪种类型填：文字列填文字、数字列填数字、日期列填日期…（认不出来的列全都列）。
/// 不合适的不列出来：文字列上填个日期，比的时候 polars 也不认。
fn vtype_options(dtype: &str) -> &'static [&'static str] {
    const ALL: &[&str] = &["text", "number", "date", "time", "datetime", "bool"];
    match base_of(dtype) {
        "String" | "Utf8" | "Categorical" | "Enum" => &["text"],
        "Boolean" => &["bool"],
        "Date" => &["date"],
        "Time" => &["time"],
        "Datetime" => &["datetime"],
        "" => ALL,
        _ if bucket_of(dtype) == NUM => &["number"],
        _ => ALL,
    }
}

/// 值类型下拉里的选项（tag, 标签）；参数里存着的那个不在里面时补在最后，别让它显示不出来。
fn type_list(dtype: &str, vtype: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> =
        vtype_options(dtype).iter().map(|tag| (tag.to_string(), vtype_label(tag).to_string())).collect();
    let current = if vtype.is_empty() { "text" } else { vtype };
    if !out.iter().any(|(tag, _)| tag == current) {
        out.push((current.to_string(), vtype_label(current).to_string()));
    }
    out
}

/// 这个值类型能选的操作（tag, 标签）：按 OPS 的顺序。
/// 另外分类 / 枚举列没有 str 方法（包含这些跑了也报错），去了这几条。
/// 原来填的操作要是不在里面，补在最后：别让它显示不出来。
fn ops_for(dtype: &str, vtype: &str, op: &str) -> Vec<(String, String)> {
    let mask = value_mask(vtype);
    let mut out: Vec<(String, String)> =
        OPS.iter().filter(|o| o.kinds & mask != 0).map(|o| (o.tag.to_string(), o.label.to_string())).collect();
    if bucket_of(dtype) == CAT {
        out.retain(|(tag, _)| !matches!(tag.as_str(), "contains" | "ncontains" | "starts" | "ends" | "regex"));
    }
    if !op.is_empty() && !out.iter().any(|(tag, _)| tag == op) {
        out.push((op.to_string(), op_label(op).to_string()));
    }
    out
}

/// 这个值类型默认用哪个操作：先按值类型的常规选择（文字 → 包含、数字 / 日期 → 大于、真假 → 为真），
/// 这一列用不了就取第一个能用的。
fn default_op(dtype: &str, vtype: &str) -> String {
    let want = match vtype {
        "number" | "date" | "time" | "datetime" => "gt",
        "bool" => "is_true",
        _ => "contains",
    };
    let ops = ops_for(dtype, vtype, "");
    match ops.iter().find(|(tag, _)| tag == want) {
        Some((tag, _)) => tag.clone(),
        None => ops.first().map_or_else(|| String::from("eq"), |(tag, _)| tag.clone()),
    }
}

/// 按这一列的类型给一个新条件（值类型跟着列走，操作再按它挑一个能用的）。
fn new_test(state: &CondState) -> Test {
    let (col, dtype) = state.columns.first().cloned().unwrap_or_default();
    let vtype = default_vtype(&dtype).to_string();
    let op = default_op(&dtype, &vtype);
    Test { col, op, value: String::new(), vtype }
}

// ---------------------------------------------------------------- JSON

fn to_json(group: &Group) -> Value {
    let items: Vec<Value> = group
        .items
        .iter()
        .map(|item| match item {
            Item::Group(g) => to_json(g),
            Item::Test(t) => {
                let mut v = json!({ "col": t.col, "op": t.op });
                if !t.value.is_empty() {
                    v["value"] = json!(t.value);
                }
                if !t.vtype.is_empty() && t.vtype != "text" {
                    v["type"] = json!(t.vtype);
                }
                v
            }
        })
        .collect();
    json!({ "join": group.join, "items": items })
}

fn from_json(v: &Value) -> Group {
    let mut group = Group { join: v["join"].as_str().unwrap_or("and").to_string(), items: Vec::new() };
    if group.join != "or" {
        group.join = String::from("and");
    }
    for item in v["items"].as_array().into_iter().flatten() {
        if item.get("items").is_some() {
            group.items.push(Item::Group(from_json(item)));
        } else {
            group.items.push(Item::Test(Test {
                col: item["col"].as_str().unwrap_or("").to_string(),
                op: item["op"].as_str().unwrap_or("eq").to_string(),
                value: item["value"].as_str().unwrap_or("").to_string(),
                vtype: item["type"].as_str().unwrap_or("text").to_string(),
            }));
        }
    }
    group
}

/// 参数里存的文字 → 条件树；读不回来时 None（界面上提示一下，从头开始）。
pub(crate) fn parse(text: &str) -> Option<Group> {
    let text = text.trim();
    if text.is_empty() {
        return Some(Group::default());
    }
    let v: Value = serde_json::from_str(text).ok()?;
    v.is_object().then(|| from_json(&v))
}

/// 条件树 → 参数里存的文字；一条条件都没有时是空串（参数留空 = 所有行都留着）。
pub(crate) fn text_of(group: &Group) -> String {
    if count_tests(group) == 0 {
        return String::new();
    }
    to_json(group).to_string()
}

fn count_tests(group: &Group) -> usize {
    group
        .items
        .iter()
        .map(|item| match item {
            Item::Test(_) => 1,
            Item::Group(g) => count_tests(g),
        })
        .sum()
}

// ---------------------------------------------------------------- 摘要和表达式文字

/// 条件的一句话（节点卡片上的摘要、参数面板那一行）：每条写出来，分组只写头两条。
pub(crate) fn summary(text: &str) -> String {
    let Some(group) = parse(text) else { return String::new() };
    if count_tests(&group) == 0 {
        return String::new();
    }
    let joiner = if group.join == "or" { " 或 " } else { " 且 " };
    if group.items.iter().all(|item| matches!(item, Item::Test(_))) {
        let tests: Vec<String> = group.items.iter().filter_map(|i| match i {
            Item::Test(t) => Some(test_summary(t)),
            _ => None,
        }).collect();
        return tests.join(joiner);
    }
    let parts: Vec<String> = group
        .items
        .iter()
        .take(2)
        .map(|item| match item {
            Item::Test(t) => test_summary(t),
            Item::Group(g) => format!(
                "{} {} 个条件的一组",
                if g.join == "or" { "或" } else { "且" },
                count_tests(g)
            ),
        })
        .collect();
    let rest = group.items.len().saturating_sub(parts.len());
    let text = parts.join(joiner);
    match rest {
        0 => text,
        n => format!("{text} 等 {n} 项"),
    }
}

/// 一个条件的说法：城市 包含 北京 / 金额 > 1000 / 邮箱 为空。
pub(crate) fn test_summary(t: &Test) -> String {
    let col = if t.col.trim().is_empty() { "（没选列）" } else { t.col.trim() };
    match t.op.as_str() {
        "eq" => format!("{col} = {}", t.value),
        "ne" => format!("{col} ≠ {}", t.value),
        "gt" => format!("{col} > {}", t.value),
        "ge" => format!("{col} ≥ {}", t.value),
        "lt" => format!("{col} < {}", t.value),
        "le" => format!("{col} ≤ {}", t.value),
        "between" => format!("{col} 介于 {}", t.value),
        "in" => format!("{col} 是其中之一（{}）", t.value),
        _ => match t.value.trim() {
            "" => format!("{col} {}", op_label(&t.op)),
            _ => format!("{col} {} {}", op_label(&t.op), t.value),
        },
    }
}

/// 值写成 polars 里的字面量（按值的类型）。
fn value_text(text: &str, vtype: &str) -> String {
    match vtype {
        "number" => text.to_string(),
        "bool" => String::from(if text.trim().to_lowercase().starts_with('t') { "True" } else { "False" }),
        "date" | "time" | "datetime" => format!("pl.lit({}).str.to_{vtype}()", py_str(text.trim())),
        _ => py_str(text),
    }
}

/// 逗号分开的几个值 → 字面量。
fn values_text(t: &Test) -> Vec<String> {
    t.value.split(',').map(str::trim).filter(|p| !p.is_empty()).map(|p| value_text(p, &t.vtype)).collect()
}

/// 条件树写成 polars 表达式（编辑器底下那一行预览；形状和 engine.py 里搭出来的一致）。
pub(crate) fn expr_text(text: &str) -> String {
    let Some(group) = parse(text) else { return String::new() };
    group_expr(&group)
}

fn group_expr(group: &Group) -> String {
    let exprs: Vec<String> = group
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Test(t) => Some(test_expr(t)),
            Item::Group(g) => {
                let inner = group_expr(g);
                (!inner.is_empty()).then_some(inner)
            }
        })
        .collect();
    match exprs.len() {
        0 => String::new(),
        1 => exprs.into_iter().next().unwrap(),
        _ if group.join == "or" => format!("pl.any_horizontal({})", exprs.join(", ")),
        _ => exprs.iter().map(|e| format!("({e})")).collect::<Vec<_>>().join(" & "),
    }
}

fn test_expr(t: &Test) -> String {
    let col = format!("pl.col({})", py_str(t.col.trim()));
    let value = values_text(t).join(", ");
    match t.op.as_str() {
        "eq" => format!("{col} == {value}"),
        "ne" => format!("{col} != {value}"),
        "gt" => format!("{col} > {value}"),
        "ge" => format!("{col} >= {value}"),
        "lt" => format!("{col} < {value}"),
        "le" => format!("{col} <= {value}"),
        "between" => format!("{col}.is_between({value})"),
        "in" => format!("{col}.is_in([{value}])"),
        "contains" => format!("{col}.str.contains({}, literal=True)", py_str(&t.value)),
        "ncontains" => format!("~{col}.str.contains({}, literal=True)", py_str(&t.value)),
        "starts" => format!("{col}.str.starts_with({})", py_str(&t.value)),
        "ends" => format!("{col}.str.ends_with({})", py_str(&t.value)),
        "regex" => format!("{col}.str.contains({})", py_str(&t.value)),
        "is_null" => format!("{col}.is_null()"),
        "not_null" => format!("{col}.is_not_null()"),
        "is_true" => format!("{col} == True"),
        "is_false" => format!("{col} == False"),
        other => format!("{col}  {other}"),
    }
}

// ---------------------------------------------------------------- 界面状态

/// 编辑器开着时的状态（`App.cond`）。
pub(crate) struct CondState {
    path: Vec<u32>,
    node: u32,
    param: usize,
    root: Group,
    /// 界面上的每一行在树里的位置（改完条件树重排一次）
    refs: Vec<RowRef>,
    /// 上游的列（名字 + polars 的类型写法）
    columns: Vec<(String, String)>,
    /// 还没拿到列：列名自己写
    typing: bool,
    /// 向引擎取列时出错的原因
    upstream: String,
    /// 提示（原来的条件读不回来、分组套太多层…）
    note: String,
    /// 问过引擎没有（列的类型要问引擎，同选列框）
    asked: bool,
}

#[derive(Clone)]
struct RowRef {
    path: Vec<usize>,
}

impl CondState {
    /// 界面上的第 index 行在树里的位置。
    fn path(&self, index: i32) -> Option<Vec<usize>> {
        self.refs.get(usize::try_from(index).ok()?).map(|r| r.path.clone())
    }

    /// 这一列的类型写法（不知道是空串）。
    fn dtype_of(&self, col: &str) -> String {
        self.columns.iter().find(|(name, _)| name == col).map_or(String::new(), |(_, t)| t.clone())
    }

    /// 重排一遍行的位置（改完条件树要调）。
    fn refresh(&mut self) {
        self.refs = rows_of(self).1;
    }
}

/// 树里 path 指的那一项。
fn item_mut<'a>(mut group: &'a mut Group, path: &[usize]) -> Option<&'a mut Item> {
    let (last, rest) = path.split_last()?;
    for i in rest {
        group = match group.items.get_mut(*i)? {
            Item::Group(sub) => sub,
            Item::Test(_) => return None,
        };
    }
    group.items.get_mut(*last)
}

/// 树里 path 指的那一项所在的那一组（删掉它、改这一组的关系用）。
fn parent_mut<'a>(mut group: &'a mut Group, path: &[usize]) -> Option<&'a mut Group> {
    let (last, rest) = path.split_last()?;
    for i in rest {
        group = match group.items.get_mut(*i)? {
            Item::Group(sub) => sub,
            Item::Test(_) => return None,
        };
    }
    (*last < group.items.len()).then_some(group)
}

fn item_at<'a>(mut group: &'a Group, path: &[usize]) -> Option<&'a Item> {
    let (last, rest) = path.split_last()?;
    for i in rest {
        group = match group.items.get(*i)? {
            Item::Group(sub) => sub,
            Item::Test(_) => return None,
        };
    }
    group.items.get(*last)
}

/// path 那一组套了几层（根是 0）。
fn depth_at(state: &CondState, path: &[usize]) -> usize {
    let mut group = &state.root;
    let mut depth = 0;
    for i in path {
        match group.items.get(*i) {
            Some(Item::Group(g)) => {
                group = g;
                depth += 1;
            }
            _ => break,
        }
    }
    depth
}

/// 条件树 → 界面上那一行行（条件一行、分组一行，带缩进）。
fn rows_of(state: &CondState) -> (Vec<CondRow>, Vec<RowRef>) {
    let mut rows = Vec::new();
    let mut refs = Vec::new();
    walk(state, &state.root, 0, &[], &mut rows, &mut refs);
    (rows, refs)
}

fn walk(state: &CondState, group: &Group, depth: usize, prefix: &[usize], rows: &mut Vec<CondRow>, refs: &mut Vec<RowRef>) {
    for (i, item) in group.items.iter().enumerate() {
        let mut path = prefix.to_vec();
        path.push(i);
        let index = rows.len() as i32;
        refs.push(RowRef { path: path.clone() });
        let join = (group.join == "or") as i32;
        let first = i == 0;
        match item {
            Item::Test(t) => {
                let dtype = state.dtype_of(&t.col);
                let current = if t.op.is_empty() { String::from("eq") } else { t.op.clone() };
                let vtype = if t.vtype.is_empty() { "text" } else { t.vtype.as_str() };
                let ops = ops_for(&dtype, vtype, &current);
                let op_index = ops.iter().position(|(tag, _)| *tag == current).map_or(0, |i| i as i32);
                let types = type_list(&dtype, vtype);
                let type_index = types.iter().position(|(tag, _)| tag == vtype).map_or(0, |i| i as i32);
                let valueless = op_val(&current) == Val::None;
                // 真假值用下拉选（真 / 假），别的自己填
                let is_bool = vtype == "bool";
                rows.push(CondRow {
                    kind: 0,
                    depth: depth as i32,
                    index,
                    join,
                    first,
                    group_join: 0,
                    count: SharedString::new(),
                    col: SharedString::from(t.col.as_str()),
                    col_index: state.columns.iter().position(|(name, _)| *name == t.col).map_or(-1, |i| i as i32),
                    op_options: ModelRc::new(VecModel::from(
                        ops.iter().map(|(_, label)| SharedString::from(label.as_str())).collect::<Vec<_>>(),
                    )),
                    op: op_index,
                    value: SharedString::from(t.value.as_str()),
                    value_hint: SharedString::from(match op_val(&current) {
                        Val::None => "",
                        Val::One => "填值",
                        Val::Two => "小的在前，用逗号分开",
                        Val::List => "用逗号分开",
                    }),
                    value_options: ModelRc::new(VecModel::from(
                        if is_bool { vec![SharedString::from("真"), SharedString::from("假")] } else { Vec::new() },
                    )),
                    value_index: if t.value.trim().to_lowercase().starts_with('f') { 1 } else { 0 },
                    valueless,
                    type_options: ModelRc::new(VecModel::from(
                        types.iter().map(|(_, label)| SharedString::from(label.as_str())).collect::<Vec<_>>(),
                    )),
                    type_index,
                });
            }
            Item::Group(g) => {
                rows.push(CondRow {
                    kind: 1,
                    depth: depth as i32,
                    index,
                    join,
                    first,
                    group_join: (g.join == "or") as i32,
                    count: SharedString::from(format!("{} 个条件", g.items.len())),
                    col: SharedString::new(),
                    col_index: -1,
                    op_options: ModelRc::new(VecModel::from(Vec::new())),
                    op: 0,
                    value: SharedString::new(),
                    value_hint: SharedString::new(),
                    value_options: ModelRc::new(VecModel::from(Vec::new())),
                    value_index: 0,
                    valueless: true,
                    type_options: ModelRc::new(VecModel::from(Vec::new())),
                    type_index: 0,
                });
                walk(state, g, depth + 1, &path, rows, refs);
            }
        }
    }
}

/// 下拉里的列名：上游的列，加上条件里用到的（不在上游里的也列出来，不然显示成空）。
fn col_names(state: &CondState) -> Vec<String> {
    fn collect(group: &Group, names: &mut Vec<String>) {
        for item in &group.items {
            match item {
                Item::Test(t) => {
                    if !t.col.trim().is_empty() && !names.iter().any(|n| n == t.col.trim()) {
                        names.push(t.col.trim().to_string());
                    }
                }
                Item::Group(g) => collect(g, names),
            }
        }
    }
    let mut names: Vec<String> = state.columns.iter().map(|(name, _)| name.clone()).collect();
    collect(&state.root, &mut names);
    names
}

fn note_of(state: &CondState) -> String {
    if state.upstream.is_empty() { state.note.clone() } else { state.upstream.clone() }
}

/// 把编辑器的整个状态推进界面（行的内容变了 / 第一次开）。
fn push(app: &AppWindow, st: &App) {
    let Some(state) = st.cond.as_ref() else { return };
    let cp = app.global::<CondPick>();
    let (rows, _) = rows_of(state);
    // 条件表的自然高度：条件一行两排（74px）、分组一行（38px），行之间 4px（和 cond.slint 里的尺寸对上）
    let height = rows.iter().map(|r| if r.kind == 0 { 74.0 } else { 38.0 }).sum::<f32>()
        + 4.0 * rows.len().max(1) as f32
        + 8.0;
    cp.set_list_height(height);
    set_model(&cp.get_rows(), rows, |m| cp.set_rows(m));
    let names: Vec<SharedString> = col_names(state).into_iter().map(SharedString::from).collect();
    set_model(&cp.get_col_names(), names, |m| cp.set_col_names(m));
    cp.set_root_join((state.root.join == "or") as i32);
    cp.set_typing(state.typing);
    cp.set_note(SharedString::from(note_of(state)));
    push_light(app, st);
}

/// 模型是 VecModel 就原地换内容（列表滚到哪不跳），不是就新建一个。
fn set_model<T: Clone + 'static>(model: &ModelRc<T>, items: Vec<T>, set: impl FnOnce(ModelRc<T>)) {
    match model.as_any().downcast_ref::<VecModel<T>>() {
        Some(vec) => vec.set_vec(items),
        None => set(ModelRc::new(VecModel::from(items))),
    }
}

/// 只改预览和摘要：正在输入值的时候用（重建行会让输入框重来）。
fn push_light(app: &AppWindow, st: &App) {
    let Some(state) = st.cond.as_ref() else { return };
    let cp = app.global::<CondPick>();
    // 都按“存进参数里的那份文字”来算：看到的就是真正会跑的东西
    let text = to_json(&state.root).to_string();
    let expr = expr_text(&text);
    cp.set_preview(SharedString::from(match expr.is_empty() {
        true => String::from("（没有条件，所有行都留着）"),
        false => format!("df.filter({expr})"),
    }));
    let summary = summary(&text);
    cp.set_summary(SharedString::from(match summary.is_empty() {
        true => String::from("没有条件 · 所有行都留着"),
        false => format!("{summary} ·（共 {} 个条件）", count_tests(&state.root)),
    }));
}

// ---------------------------------------------------------------- 打开、改、确定

/// 参数面板上点了“编辑”：读回参数里的条件树，弹出编辑器。
pub(crate) fn open(weak: &slint::Weak<AppWindow>, state: &Arc<Mutex<App>>, param: usize) {
    let Some(app) = weak.upgrade() else { return };
    let mut st = state.lock().unwrap();
    let (path, Some(node)) = st.param_target() else { return };
    let Some(n) = st.graph_at(&path).and_then(|g| g.node(node)) else { return };
    let def = n.kind().params[param].clone();
    let value = n.params[param].clone();
    let title = format!("{} · {}", n.title, def.label);
    // 列从第 1 个输入来（表格级的节点；表达式网络里的节点没有列可选，自己写列名）
    let upstream = path.is_empty().then(|| n.inputs.first().copied().flatten()).flatten();
    let columns = upstream.map(|up| columns_of(&st, up)).unwrap_or_default();
    let (root, note) = match parse(&value) {
        Some(group) => (group, String::new()),
        None => (Group::default(), format!("原来的条件读不回来，确定会替换掉：{}", value.trim())),
    };
    let typing = columns.is_empty();
    st.cond = Some(CondState {
        path,
        node,
        param,
        root,
        refs: Vec::new(),
        columns,
        typing,
        upstream: String::new(),
        note,
        asked: false,
    });
    if let Some(c) = st.cond.as_mut() {
        c.refresh();
    }
    let cp = app.global::<CondPick>();
    cp.set_open(true);
    cp.set_title(SharedString::from(title));
    // 位置每次开重新居中；拽过的大小留着
    cp.set_box_x(0.0);
    cp.set_box_y(0.0);
    push(&app, &st);
    drop(st);
    ask(weak, state);
}

/// 上游节点上次运行留下的列名和类型（引擎回复之前先列着）。
fn columns_of(st: &App, up: u32) -> Vec<(String, String)> {
    let Some(n) = st.graph.node(up) else { return Vec::new() };
    let mut types: Vec<String> = n.dtypes.clone();
    if types.is_empty() && n.primary {
        if let Some((_, native)) = &st.native_types {
            types = n
                .columns
                .iter()
                .map(|c| native.iter().find(|(name, _)| name == c).map_or(String::new(), |(_, t)| t.clone()))
                .collect();
        }
    }
    n.columns.iter().enumerate().map(|(i, c)| (c.clone(), types.get(i).cloned().unwrap_or_default())).collect()
}

/// 列的类型问引擎要（和选列框一样：只推上游的表结构，不执行）。拿到的类型决定能选哪些操作。
fn ask(weak: &slint::Weak<AppWindow>, state: &Arc<Mutex<App>>) {
    let (steps, source) = {
        let mut guard = state.lock().unwrap();
        let st = &mut *guard;
        let Some(c) = st.cond.as_mut() else { return };
        if c.asked || c.columns.iter().all(|(_, t)| !t.is_empty()) {
            return; // 问过了，或者上次运行已经知道类型了
        }
        let Some(up) = st.graph.node(c.node).and_then(|n| n.inputs.first().copied().flatten()) else { return };
        c.asked = true;
        match st.graph.compile(up) {
            Ok(steps) => (steps, up),
            Err(e) => {
                c.upstream = e.msg;
                if let Some(app) = weak.upgrade() {
                    push(&app, st);
                }
                return;
            }
        }
    };
    let (weak, state) = (weak.clone(), state.clone());
    std::thread::spawn(move || {
        let reply = engine::pick(&steps, source, "", &|| true);
        let Some(reply) = reply else { return };
        let _ = slint::invoke_from_event_loop(move || {
            let mut st = state.lock().unwrap();
            let Some(c) = st.cond.as_mut() else { return };
            match reply {
                Ok(picked) if !picked.columns.is_empty() => {
                    c.columns = picked.columns;
                    c.typing = false;
                    c.upstream.clear();
                }
                Ok(_) => {}
                Err(e) => c.upstream = e,
            }
            if let Some(app) = weak.upgrade() {
                push(&app, &st);
            }
        });
    });
}

/// 改了状态：重排行再推界面。
fn update(weak: &slint::Weak<AppWindow>, state: &Arc<Mutex<App>>, f: impl FnOnce(&mut CondState)) {
    let mut st = state.lock().unwrap();
    let Some(c) = st.cond.as_mut() else { return };
    f(c);
    c.refresh();
    if let Some(app) = weak.upgrade() {
        push(&app, &st);
    }
}

/// 会换掉行的改动（加 / 删 / 换列）：推迟到回调返回后，不在控件自己的回调里销毁它。
fn defer(weak: &slint::Weak<AppWindow>, state: &Arc<Mutex<App>>, f: impl FnOnce(&mut CondState) + Send + 'static) {
    let (weak, state) = (weak.clone(), state.clone());
    let _ = slint::invoke_from_event_loop(move || update(&weak, &state, f));
}

fn close(weak: &slint::Weak<AppWindow>, state: &Arc<Mutex<App>>) {
    state.lock().unwrap().cond = None;
    if let Some(app) = weak.upgrade() {
        app.global::<CondPick>().set_open(false);
    }
}

/// 确定：条件树写回参数，重新运行。
fn accept(weak: &slint::Weak<AppWindow>, state: &Arc<Mutex<App>>) {
    {
        let mut st = state.lock().unwrap();
        let Some(c) = st.cond.take() else { return };
        let node = st.graph_at_mut(&c.path).and_then(|g| g.node_mut(c.node));
        let Some(value) = node.and_then(|n| n.params.get_mut(c.param)) else { return };
        *value = text_of(&c.root);
        clear_node_error(&mut st, c.node);
    }
    close(weak, state);
    let (weak, state) = (weak.clone(), state.clone());
    let _ = slint::invoke_from_event_loop(move || graph_changed(&weak, &state, true));
}

/// 往这一组里添一项（条件 / 分组）；根用 None。
fn add_item(state: &mut CondState, path: Option<Vec<usize>>, group_item: bool) {
    let test = new_test(state);
    let target = match &path {
        None => &mut state.root,
        Some(path) => match item_mut(&mut state.root, path) {
            Some(Item::Group(g)) => g,
            _ => return,
        },
    };
    if group_item {
        // 新分组里先放一个条件，一加就能填；默认“或”（分组多半是为了写“或”）
        let inner = Group { join: String::from("or"), items: vec![Item::Test(test)] };
        target.items.push(Item::Group(inner));
    } else {
        target.items.push(Item::Test(test));
    }
}

/// 换了一列：值的类型跟着这一列走，操作再按“列 + 值类型”挑一个能用的。
fn retype(test: &mut Test, dtype: &str) {
    test.vtype = default_vtype(dtype).to_string();
    fix_op(test, dtype);
}

/// 值类型 / 列换了之后：值和操作在这一列上还成立吗？不成立就换成这一列能用的。
fn fix_op(test: &mut Test, dtype: &str) {
    if !vtype_options(dtype).contains(&test.vtype.as_str()) {
        test.vtype = default_vtype(dtype).to_string();
    }
    if !ops_for(dtype, &test.vtype, "").iter().any(|(tag, _)| *tag == test.op) {
        test.op = default_op(dtype, &test.vtype);
    }
}

pub(crate) fn install(app: &AppWindow, state: &Arc<Mutex<App>>) {
    let cp = app.global::<CondPick>();
    {
        let (weak, state) = (app.as_weak(), state.clone());
        cp.on_open_param(move |param| open(&weak, &state, param.max(0) as usize));
    }
    {
        let (weak, state) = (app.as_weak(), state.clone());
        cp.on_cancel(move || close(&weak, &state));
    }
    {
        let (weak, state) = (app.as_weak(), state.clone());
        cp.on_accept(move || accept(&weak, &state));
    }
    {
        let (weak, state) = (app.as_weak(), state.clone());
        cp.on_set_root_join(move |join| {
            update(&weak, &state, |c| c.root.join = if join == 1 { String::from("or") } else { String::from("and") });
        });
    }
    {
        let (weak, state) = (app.as_weak(), state.clone());
        cp.on_group_join(move |index, join| {
            let join = if join == 1 { "or" } else { "and" };
            update(&weak, &state, move |c| {
                if let Some(path) = c.path(index) {
                    if let Some(Item::Group(g)) = item_mut(&mut c.root, &path) {
                        g.join = join.to_string();
                    }
                }
            });
        });
    }
    {
        // 和上一个条件的关系：点一下换
        let (weak, state) = (app.as_weak(), state.clone());
        cp.on_row_join(move |index| {
            update(&weak, &state, |c| {
                let Some(path) = c.path(index) else { return };
                if let Some(parent) = parent_mut(&mut c.root, &path) {
                    parent.join = if parent.join == "or" { String::from("and") } else { String::from("or") };
                }
            });
        });
    }
    {
        let (weak, state) = (app.as_weak(), state.clone());
        cp.on_row_col(move |index, col| {
            defer(&weak, &state, move |c| {
                let Some(path) = c.path(index) else { return };
                let Some((name, _)) = c.columns.get(col.max(0) as usize).cloned() else { return };
                let dtype = c.dtype_of(&name);
                if let Some(Item::Test(t)) = item_mut(&mut c.root, &path) {
                    if t.col != name {
                        t.col = name;
                        retype(t, &dtype);
                    }
                }
            });
        });
    }
    {
        // 列名自己写（列列表还没拿到）：和填值一样，打字时不重排行
        let (weak, state) = (app.as_weak(), state.clone());
        cp.on_row_col_text(move |index, text| {
            let text = text.to_string();
            let mut st = state.lock().unwrap();
            let Some(c) = st.cond.as_mut() else { return };
            let Some(path) = c.path(index) else { return };
            if let Some(Item::Test(t)) = item_mut(&mut c.root, &path) {
                t.col = text;
            }
            if let Some(app) = weak.upgrade() {
                push_light(&app, &st);
            }
        });
    }
    {
        let (weak, state) = (app.as_weak(), state.clone());
        cp.on_row_op(move |index, op| {
            defer(&weak, &state, move |c| {
                let Some(path) = c.path(index) else { return };
                let (dtype, vtype, current) = match item_at(&c.root, &path) {
                    Some(Item::Test(t)) => (c.dtype_of(&t.col), t.vtype.clone(), t.op.clone()),
                    _ => return,
                };
                let Some((tag, _)) = ops_for(&dtype, &vtype, &current).get(op.max(0) as usize).cloned() else { return };
                if let Some(Item::Test(t)) = item_mut(&mut c.root, &path) {
                    t.op = tag;
                }
            });
        });
    }
    {
        // 值：打字的时候只更新预览，不重排行（重建行会把输入框重来）
        let (weak, state) = (app.as_weak(), state.clone());
        cp.on_row_value(move |index, text| {
            let text = text.to_string();
            let mut st = state.lock().unwrap();
            let Some(c) = st.cond.as_mut() else { return };
            let Some(path) = c.path(index) else { return };
            if let Some(Item::Test(t)) = item_mut(&mut c.root, &path) {
                t.value = text;
            }
            if let Some(app) = weak.upgrade() {
                push_light(&app, &st);
            }
        });
    }
    {
        // 值的类型换了：能选的操作跟着换（不成立就换成这个类型能用的）
        let (weak, state) = (app.as_weak(), state.clone());
        cp.on_row_type(move |index, kind| {
            defer(&weak, &state, move |c| {
                let Some(path) = c.path(index) else { return };
                let dtype = match item_at(&c.root, &path) {
                    Some(Item::Test(t)) => c.dtype_of(&t.col),
                    _ => return,
                };
                let current = match item_at(&c.root, &path) {
                    Some(Item::Test(t)) => t.vtype.clone(),
                    _ => return,
                };
                let Some((tag, _)) = type_list(&dtype, &current).get(kind.max(0) as usize).cloned() else { return };
                if let Some(Item::Test(t)) = item_mut(&mut c.root, &path) {
                    t.vtype = tag;
                    fix_op(t, &dtype);
                }
            });
        });
    }
    {
        let (weak, state) = (app.as_weak(), state.clone());
        cp.on_row_del(move |index| {
            defer(&weak, &state, move |c| {
                let Some(path) = c.path(index) else { return };
                if let Some(last) = path.last() {
                    if let Some(parent) = parent_mut(&mut c.root, &path) {
                        parent.items.remove(*last);
                    }
                }
            });
        });
    }
    {
        let (weak, state) = (app.as_weak(), state.clone());
        cp.on_row_add(move |index| {
            defer(&weak, &state, move |c| {
                let path = match index {
                    i if i < 0 => None,
                    i => c.path(i),
                };
                if index >= 0 && path.is_none() {
                    return;
                }
                c.note.clear();
                add_item(c, path, false);
            });
        });
    }
    {
        let (weak, state) = (app.as_weak(), state.clone());
        cp.on_group_add(move |index| {
            defer(&weak, &state, move |c| {
                let path = match index {
                    i if i < 0 => None,
                    i => c.path(i),
                };
                if index >= 0 && path.is_none() {
                    return;
                }
                let depth = path.as_ref().map_or(0, |p| depth_at(c, p));
                if depth + 1 > MAX_DEPTH {
                    c.note = format!("分组最多套 {} 层", MAX_DEPTH);
                    return;
                }
                c.note.clear();
                add_item(c, path, true);
            });
        });
    }
    {
        let (weak, state) = (app.as_weak(), state.clone());
        cp.on_clear(move || {
            defer(&weak, &state, |c| {
                c.root.items.clear();
                c.root.join = String::from("and");
                c.note.clear();
            });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(text: &str) -> Group {
        parse(text).unwrap()
    }

    fn state(root: Group) -> CondState {
        let mut state = CondState {
            path: Vec::new(),
            node: 1,
            param: 0,
            root,
            refs: Vec::new(),
            columns: vec![(String::from("a"), String::from("String")), (String::from("b"), String::from("Int64"))],
            typing: false,
            upstream: String::new(),
            note: String::new(),
            asked: true,
        };
        state.refresh();
        state
    }

    #[test]
    fn json_round_trips() {
        // 键按字母排（serde_json 的 Map 就是），读回来再写出去是同一份文字
        let text = r#"{"items":[{"col":"城市","op":"contains","value":"北京"},{"items":[{"col":"金额","op":"gt","type":"number","value":"1000"},{"col":"日期","op":"is_null"}],"join":"and"}],"join":"or"}"#;
        let group = tree(text);
        assert_eq!(to_json(&group).to_string(), text);
        assert_eq!(text_of(&group), text);
        // 空树：参数里存空串
        assert_eq!(text_of(&Group::default()), "");
        assert!(parse("").unwrap().items.is_empty());
        assert!(parse("不是 JSON").is_none());
        assert!(parse("[1, 2]").is_none());
        // 缺的项按默认值兜住，别整个丢掉
        let group = tree(r#"{"join":"or","items":[{"col":"a"}]}"#);
        assert_eq!(group.join, "or");
        assert_eq!(group.items.len(), 1);
        assert_eq!(text_of(&group), r#"{"items":[{"col":"a","op":"eq"}],"join":"or"}"#);
    }

    #[test]
    fn summary_and_expr() {
        let text = text_of(&tree(
            r#"{"join":"and","items":[{"col":"城市","op":"contains","value":"北京"},{"col":"金额","op":"gt","value":"1000","type":"number"}]}"#,
        ));
        assert_eq!(summary(&text), "城市 包含 北京 且 金额 > 1000");
        assert_eq!(summary(""), "");
        let text = text_of(&tree(r#"{"join":"and","items":[{"col":"邮箱","op":"is_null"}]}"#));
        assert_eq!(summary(&text), "邮箱 为空");
        assert_eq!(expr_text(&text), r#"pl.col("邮箱").is_null()"#);
        // 分组：只写头两项
        let text = text_of(&tree(
            r#"{"join":"and","items":[{"join":"or","items":[{"col":"a","op":"eq","value":"1"},{"col":"b","op":"eq","value":"2"}]},{"col":"c","op":"is_null"},{"col":"d","op":"is_null"}]}"#,
        ));
        assert_eq!(summary(&text), "或 2 个条件的一组 且 c 为空 等 1 项");
    }

    #[test]
    fn expr_text_matches_the_shape_of_the_tree() {
        let text = r#"{"join":"and","items":[{"col":"城市","op":"contains","value":"北京"},{"join":"or","items":[{"col":"金额","op":"ge","value":"1000","type":"number"},{"col":"日期","op":"lt","value":"2024-01-01","type":"date"}]}]}"#;
        assert_eq!(
            expr_text(text),
            r#"(pl.col("城市").str.contains("北京", literal=True)) & (pl.any_horizontal(pl.col("金额") >= 1000, pl.col("日期") < pl.lit("2024-01-01").str.to_date()))"#
        );
        assert_eq!(expr_text(""), "");
        // 介于 / 其中之一：逗号分开的几个值
        assert_eq!(
            expr_text(r#"{"join":"and","items":[{"col":"i","op":"between","value":"1, 10","type":"number"}]}"#),
            r#"pl.col("i").is_between(1, 10)"#
        );
        assert_eq!(
            expr_text(r#"{"join":"and","items":[{"col":"k","op":"in","value":"a, b"}]}"#),
            r#"pl.col("k").is_in(["a", "b"])"#
        );
    }

    #[test]
    fn ops_follow_the_value_type_and_the_column() {
        let tags = |dtype: &str, vtype: &str| -> Vec<String> {
            ops_for(dtype, vtype, "").into_iter().map(|(tag, _)| tag).collect()
        };
        // 值类型决定能做什么：文字有包含、数字有大于和介于、真假有为真
        let text = tags("", "text");
        assert!(text.contains(&String::from("contains")) && !text.contains(&String::from("between")));
        let num = tags("", "number");
        assert!(num.contains(&String::from("between")) && num.contains(&String::from("gt")));
        assert!(!num.contains(&String::from("contains")) && !num.contains(&String::from("starts")));
        let time = tags("", "datetime");
        assert!(time.contains(&String::from("between")) && time.contains(&String::from("gt")));
        assert!(!time.contains(&String::from("contains")));
        let bools = tags("", "bool");
        assert!(bools.contains(&String::from("is_true")) && !bools.contains(&String::from("gt")));
        // 列再筛一层：分类 / 枚举列没有 str 方法（包含这些跑了也报错）
        let cat = tags("Categorical", "text");
        assert!(!cat.contains(&String::from("contains")) && !cat.contains(&String::from("regex")));
        assert!(cat.contains(&String::from("eq")) && cat.contains(&String::from("in")));
        // 列认不出来时按值类型来（List / Struct 这些）
        assert_eq!(tags("List(Int64)", "text").len(), text.len());
        // 值类型下拉：只列这一列合得来的
        let types = |dtype: &str| -> Vec<String> {
            vtype_options(dtype).iter().map(|t| t.to_string()).collect()
        };
        assert_eq!(types("String"), ["text"]);
        assert_eq!(types("Categorical"), ["text"]);
        assert_eq!(types("Int64"), ["number"]);
        assert_eq!(types("Datetime(time_unit='us', time_zone=None)"), ["datetime"]);
        assert_eq!(types("Boolean"), ["bool"]);
        assert_eq!(types("").len(), 6);
        // 默认值和默认操作
        assert_eq!(default_vtype("Datetime(time_unit='us', time_zone=None)"), "datetime");
        assert_eq!(default_vtype("Date"), "date");
        assert_eq!(default_vtype("Int64"), "number");
        assert_eq!(default_vtype("String"), "text");
        assert_eq!(default_op("String", "text"), "contains");
        assert_eq!(default_op("Int64", "number"), "gt");
        assert_eq!(default_op("Categorical", "text"), "eq"); // contains 用不了，取第一个能用的
        // 存着的操作不在这一对里也留着（换回来时还在，显示得出）
        assert!(ops_for("Int64", "number", "contains").iter().any(|(tag, _)| tag == "contains"));
    }

    #[test]
    fn rows_carry_the_tree_shape() {
        let mut state = state(tree(
            r#"{"join":"or","items":[{"col":"a","op":"eq","value":"1"},{"join":"and","items":[{"col":"b","op":"gt","value":"2"}]}]}"#,
        ));
        let (rows, _) = rows_of(&state);
        assert_eq!(rows.len(), 3);
        assert_eq!((rows[0].kind, rows[0].depth, rows[0].first, rows[0].join), (0, 0, true, 1));
        assert_eq!((rows[1].kind, rows[1].depth, rows[1].first), (1, 0, false));
        assert_eq!((rows[2].kind, rows[2].depth, rows[2].first), (0, 1, true));
        assert_eq!(state.path(2), Some(vec![1, 0]));
        assert_eq!(rows[0].type_index, 0); // 文字
        // 换成数字列：值的类型和操作跟着换（文字列上的“包含”在数字列上不成立）
        let path = state.path(0).unwrap();
        let dtype = state.dtype_of("b");
        if let Some(Item::Test(t)) = item_mut(&mut state.root, &path) {
            t.col = String::from("b");
            t.op = String::from("contains"); // 原来这行是照着文字列填的
            retype(t, &dtype);
        }
        state.refresh();
        let (rows, _) = rows_of(&state);
        assert_eq!(rows[0].type_index, 0);
        assert_eq!(state_test(&state, 0), "gt"); // contains 换成了这一列能用的
        assert_eq!(state_type(&state, 0), "number");
        // 增删
        add_item(&mut state, None, false);
        assert_eq!(state.root.items.len(), 3);
        add_item(&mut state, Some(vec![1]), true);
        assert_eq!(state.root.items.len(), 3);
        match state.root.items.get(1).unwrap() {
            Item::Group(g) => assert_eq!(g.items.len(), 2),
            _ => panic!("第 2 项应该是那个分组"),
        }
        state.refresh();
        let path = state.path(3).unwrap();
        assert_eq!(path, vec![1, 1]);
        if let Some(last) = path.last() {
            parent_mut(&mut state.root, &path).unwrap().items.remove(*last);
        }
        match state.root.items.get(1).unwrap() {
            Item::Group(g) => assert_eq!(g.items.len(), 1),
            _ => panic!("第 2 项应该是那个分组"),
        }
        // 空值操作不用填值
        let mut test = Test { col: String::from("a"), op: String::from("is_null"), value: String::new(), vtype: String::from("text") };
        retype(&mut test, "String");
        assert_eq!(test.op, "is_null");
        assert_eq!(ops_for("String", "text", "is_null")[0].0, "eq");
    }

    /// 第 index 行现在的操作（测试里看用）。
    fn state_test(state: &CondState, index: i32) -> String {
        match state.path(index).and_then(|p| item_at(&state.root, &p)) {
            Some(Item::Test(t)) => t.op.clone(),
            _ => String::new(),
        }
    }

    /// 第 index 行现在的值类型。
    fn state_type(state: &CondState, index: i32) -> String {
        match state.path(index).and_then(|p| item_at(&state.root, &p)) {
            Some(Item::Test(t)) => t.vtype.clone(),
            _ => String::new(),
        }
    }
}
