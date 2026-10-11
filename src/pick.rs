//! 参数旁边“列 ▾”弹出的选列框：
//! - 列：表里的列带类型、可以搜索；点一下选上（✓），再点排除（✗），再点取消；
//! - 条件：polars 的全部选择器（cs.*）一条一条加，条与条之间“或 / 且 / 去掉 / 异或”，每条可以取反；
//! - 底下是写成的写法和“现在会选到哪几列”——向引擎要（collect_schema + cs.expand_selector），
//!   所以正则、字母这些和 polars 完全一样。确定时替换参数原来的写法，或者接在后面（开关）。
//!
//! 打开时把参数原来的写法读回来（`read_back`）；读不回来的说一声，确定会替换掉它。
//! 结果：(✓ 的列 ∪ 条件) − ✗ 的列；只勾了列时写成列名（"a", "b" / pl.col("a", "b")）。

use crate::*;
use nodes::{ColInsert, DTYPES, Pick, py_str};
use std::sync::atomic::{AtomicU64, Ordering};

// ---------------------------------------------------------------- 选择器

/// 条件里的文字参数是什么。
#[derive(Clone, Copy, PartialEq, Debug)]
enum TextArg {
    None,
    /// 几段文字，逗号分开，每段一个字符串参数（starts_with…）
    Strings,
    /// 一个字符串（正则）
    Pattern,
    /// 列名，逗号分开（by_name）
    Names,
    /// 第几列：0, 2, -1，连续的 1~3（by_index）
    Indices,
    /// 数据类型：Int64, Datetime("ms")（by_dtype）
    Dtypes,
    /// 时区名（datetime 的时区选“指定”时才有）
    Zone,
}

/// 条件里的下拉。
#[derive(Clone, Copy, PartialEq, Debug)]
enum MenuArg {
    None,
    /// time_unit：任意 / ms / us / ns
    Unit,
    /// time_zone：任意 / 没有时区 / 有时区 / 指定
    Zone,
    /// list / array 的元素是什么（第一个位置参数，一个选择器）
    Inner,
}

/// 一种选择器（cs.name）和它的参数在条件里怎么填。
struct SelDef {
    name: &'static str,
    label: &'static str,
    text: TextArg,
    text_label: &'static str,
    hint: &'static str,
    /// 开关：(关键字参数, 说明, 默认)；关键字空 = 没有
    flags: [(&'static str, &'static str, bool); 2],
    menus: [MenuArg; 2],
    /// array 的 width
    width: bool,
}

const NO_FLAG: (&str, &str, bool) = ("", "", false);
const BASE: SelDef = SelDef {
    name: "",
    label: "",
    text: TextArg::None,
    text_label: "",
    hint: "",
    flags: [NO_FLAG, NO_FLAG],
    menus: [MenuArg::None, MenuArg::None],
    width: false,
};

/// polars 的全部选择器（cs.exclude 就是“去掉”，cs.expand_selector / is_selector 不是选列的）。
const SELECTORS: &[SelDef] = &[
    SelDef { name: "numeric", label: "类型：数字（整数、小数、定点小数）", ..BASE },
    SelDef { name: "integer", label: "类型：整数", ..BASE },
    SelDef { name: "signed_integer", label: "类型：有符号整数（Int8 ~ Int128）", ..BASE },
    SelDef { name: "unsigned_integer", label: "类型：无符号整数（UInt8 ~ UInt128）", ..BASE },
    SelDef { name: "float", label: "类型：小数（Float16 / 32 / 64）", ..BASE },
    SelDef { name: "decimal", label: "类型：定点小数（Decimal）", ..BASE },
    SelDef {
        name: "string",
        label: "类型：文字（String）",
        flags: [("include_categorical", "分类列也算", false), NO_FLAG],
        ..BASE
    },
    SelDef { name: "categorical", label: "类型：分类（Categorical）", ..BASE },
    SelDef { name: "enum", label: "类型：枚举（Enum）", ..BASE },
    SelDef { name: "boolean", label: "类型：真假（Boolean）", ..BASE },
    SelDef { name: "binary", label: "类型：字节（Binary）", ..BASE },
    SelDef { name: "temporal", label: "类型：日期时间类（日期、时间、日期时间、时长）", ..BASE },
    SelDef { name: "date", label: "类型：日期（Date）", ..BASE },
    SelDef { name: "time", label: "类型：时间（Time）", ..BASE },
    SelDef {
        name: "datetime",
        label: "类型：日期时间（Datetime）",
        text: TextArg::Zone,
        text_label: "时区",
        hint: "Asia/Shanghai、UTC",
        menus: [MenuArg::Unit, MenuArg::Zone],
        ..BASE
    },
    SelDef { name: "duration", label: "类型：时长（Duration）", menus: [MenuArg::Unit, MenuArg::None], ..BASE },
    SelDef { name: "nested", label: "类型：嵌套（列表、数组、结构、键值对）", ..BASE },
    SelDef { name: "list", label: "类型：列表（List）", menus: [MenuArg::Inner, MenuArg::None], ..BASE },
    SelDef { name: "array", label: "类型：数组（Array）", menus: [MenuArg::Inner, MenuArg::None], width: true, ..BASE },
    SelDef { name: "struct", label: "类型：结构（Struct）", ..BASE },
    SelDef { name: "map", label: "类型：键值对（Map）", ..BASE },
    SelDef { name: "object", label: "类型：Object", ..BASE },
    SelDef {
        name: "by_dtype",
        label: "类型：是这几种…",
        text: TextArg::Dtypes,
        text_label: "类型",
        hint: "Int64, Datetime(\"ms\")；多个用逗号，右边下拉帮着填",
        ..BASE
    },
    SelDef {
        name: "by_name",
        label: "列名：是这几个…",
        text: TextArg::Names,
        text_label: "列名",
        hint: "不用加引号：姓名, 城市",
        flags: [("require_all", "表里必须都有", true), NO_FLAG],
        ..BASE
    },
    SelDef {
        name: "starts_with",
        label: "列名：开头是…",
        text: TextArg::Strings,
        text_label: "开头",
        hint: "几个都行，逗号分开：价, 金额",
        ..BASE
    },
    SelDef {
        name: "ends_with",
        label: "列名：结尾是…",
        text: TextArg::Strings,
        text_label: "结尾",
        hint: "几个都行，逗号分开：_id, 编号",
        ..BASE
    },
    SelDef {
        name: "contains",
        label: "列名：包含…",
        text: TextArg::Strings,
        text_label: "包含",
        hint: "几个都行，逗号分开：日期, 时间",
        ..BASE
    },
    SelDef {
        name: "matches",
        label: "列名：符合正则…",
        text: TextArg::Pattern,
        text_label: "正则",
        hint: "^\\d{4}年$、(?i)amount",
        ..BASE
    },
    SelDef {
        name: "alpha",
        label: "列名：只有字母（汉字也算）",
        flags: [("ascii_only", "只算 a-z A-Z", false), ("ignore_spaces", "空格不算", false)],
        ..BASE
    },
    SelDef {
        name: "alphanumeric",
        label: "列名：只有字母和数字",
        flags: [("ascii_only", "只算 ASCII", false), ("ignore_spaces", "空格不算", false)],
        ..BASE
    },
    SelDef { name: "digit", label: "列名：只有数字", flags: [("ascii_only", "只算 0-9", false), NO_FLAG], ..BASE },
    SelDef { name: "first", label: "位置：第一列", flags: [("strict", "没有列时报错", true), NO_FLAG], ..BASE },
    SelDef { name: "last", label: "位置：最后一列", flags: [("strict", "没有列时报错", true), NO_FLAG], ..BASE },
    SelDef {
        name: "by_index",
        label: "位置：第几列…",
        text: TextArg::Indices,
        text_label: "第几列",
        hint: "从 0 数，-1 是最后一列：0, 2, -1；连续的写 1~3",
        flags: [("require_all", "必须都有", true), NO_FLAG],
        ..BASE
    },
    SelDef { name: "all", label: "全部列", ..BASE },
    SelDef { name: "empty", label: "一列都不选", ..BASE },
];

/// 条件之间：(写法, 下拉里的字, Python 里的优先级)。
const OPS: [(&str, &str, u8); 4] = [("|", "或者 |", 1), ("&", "并且 &", 3), ("-", "去掉 -", 4), ("^", "只在一边 ^", 2)];
/// 单独一个选择器（cs.x()、~cs.x()）的优先级：比哪个运算都高
const ATOM: u8 = 9;

const UNITS: [&str; 4] = ["任意单位", "ms", "us", "ns"];
const ZONES: [&str; 4] = ["时区任意", "没有时区", "有时区（哪个都行）", "指定时区…"];
const INNERS: [(&str, &str); 11] = [
    ("元素任意", ""),
    ("元素是数字", "cs.numeric()"),
    ("元素是整数", "cs.integer()"),
    ("元素是小数", "cs.float()"),
    ("元素是文字", "cs.string()"),
    ("元素是真假", "cs.boolean()"),
    ("元素是日期时间类", "cs.temporal()"),
    ("元素是日期", "cs.date()"),
    ("元素是列表", "cs.list()"),
    ("元素是数组", "cs.array()"),
    ("元素是结构", "cs.struct()"),
];

fn menu_options(menu: MenuArg) -> Vec<&'static str> {
    match menu {
        MenuArg::None => Vec::new(),
        MenuArg::Unit => UNITS.to_vec(),
        MenuArg::Zone => ZONES.to_vec(),
        MenuArg::Inner => INNERS.iter().map(|(label, _)| *label).collect(),
    }
}

fn menu_label(menu: MenuArg) -> &'static str {
    match menu {
        MenuArg::None => "",
        MenuArg::Unit => "单位",
        MenuArg::Zone => "时区",
        MenuArg::Inner => "元素",
    }
}

/// 一条条件。
#[derive(Clone, PartialEq, Debug)]
pub(crate) struct Row {
    /// 和前面的怎么合（OPS 的下标；第一条不用）
    op: usize,
    /// 取反（~）
    neg: bool,
    /// SELECTORS 的下标
    kind: usize,
    text: String,
    flags: [bool; 2],
    menus: [usize; 2],
    width: String,
}

impl Row {
    fn new(kind: usize) -> Row {
        let def = &SELECTORS[kind];
        Row {
            op: 0,
            neg: false,
            kind,
            text: String::new(),
            flags: [def.flags[0].2, def.flags[1].2],
            menus: [0, 0],
            width: String::new(),
        }
    }

    fn def(&self) -> &'static SelDef {
        &SELECTORS[self.kind]
    }

    /// 文字框现在显不显示（datetime 的时区只在“指定”时要填）。
    fn shows_text(&self) -> bool {
        match self.def().text {
            TextArg::None => false,
            TextArg::Zone => self.menus[1] == 3,
            _ => true,
        }
    }

    /// 写成 cs.xxx(…)；还没填好时是 Err(缺什么)。
    fn code(&self) -> Result<String, String> {
        let def = self.def();
        let mut args: Vec<String> = Vec::new();
        let text = self.text.trim();
        match def.text {
            TextArg::None | TextArg::Zone => {}
            TextArg::Strings | TextArg::Names => {
                let parts = split_list(text);
                if parts.is_empty() {
                    return Err(format!("填上{}", def.text_label));
                }
                args.extend(parts.iter().map(|p| py_str(p)));
            }
            TextArg::Pattern => {
                if text.is_empty() {
                    return Err(String::from("填上正则"));
                }
                args.push(py_str(text));
            }
            TextArg::Indices => {
                let parts = split_list(text);
                if parts.is_empty() {
                    return Err(String::from("填上第几列"));
                }
                for part in parts {
                    args.push(index_code(&part).ok_or(format!("“{part}”不是列的序号（0, 2, -1 或 1~3）"))?);
                }
            }
            TextArg::Dtypes => {
                if text.is_empty() {
                    return Err(String::from("填上类型"));
                }
                args.push(dtype_code(text));
            }
        }
        for (i, menu) in def.menus.iter().enumerate() {
            let at = self.menus[i];
            match menu {
                MenuArg::Inner if at > 0 => args.insert(0, INNERS[at].1.to_string()),
                MenuArg::Unit if at > 0 => args.push(format!("time_unit={}", py_str(UNITS[at]))),
                MenuArg::Zone => match at {
                    1 => args.push(String::from("time_zone=None")),
                    2 => args.push(String::from("time_zone=\"*\"")),
                    3 if text.is_empty() => return Err(String::from("填上时区")),
                    3 => args.push(format!("time_zone={}", py_str(text))),
                    _ => {}
                },
                _ => {}
            }
        }
        if def.width && !self.width.trim().is_empty() {
            let n: u32 = self.width.trim().parse().map_err(|_| String::from("长度要填整数"))?;
            args.push(format!("width={n}"));
        }
        for (i, (kw, _, default)) in def.flags.iter().enumerate() {
            if !kw.is_empty() && self.flags[i] != *default {
                args.push(format!("{kw}={}", if self.flags[i] { "True" } else { "False" }));
            }
        }
        let call = format!("cs.{}({})", def.name, args.join(", "));
        Ok(if self.neg { format!("~{call}") } else { call })
    }
}

/// 逗号分开（中文逗号也算），去掉空的。
fn split_list(text: &str) -> Vec<String> {
    text.split([',', '，']).map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
}

/// 0 → 0，-1 → -1，1~3 → range(1, 4)。
fn index_code(part: &str) -> Option<String> {
    match part.split_once('~') {
        Some((a, b)) => {
            let (a, b): (i64, i64) = (a.trim().parse().ok()?, b.trim().parse().ok()?);
            (b >= a).then(|| format!("range({a}, {})", b + 1))
        }
        None => part.parse::<i64>().ok().map(|n| n.to_string()),
    }
}

/// 类型的写法补上 pl.：List(Int64) → pl.List(pl.Int64)；字符串里的、已经写了 pl. 的不动（engine.py 的 dtype_text）。
pub(crate) fn dtype_code(text: &str) -> String {
    let names: Vec<&str> = DTYPES.iter().map(|(name, _)| *name).chain(["Utf8", "Field"]).collect();
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len() + 8);
    let mut i = 0;
    let mut prev = ' '; // 上一个不是空白的字
    while i < chars.len() {
        let c = chars[i];
        if c == '"' || c == '\'' {
            // 字符串原样抄
            let start = i;
            i += 1;
            while i < chars.len() && chars[i] != c {
                i += if chars[i] == '\\' { 2 } else { 1 };
            }
            i = (i + 1).min(chars.len());
            out.extend(&chars[start..i]);
            prev = c;
            continue;
        }
        if c.is_alphabetic() || c == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();
            if prev != '.' && names.contains(&word.as_str()) {
                out.push_str("pl.");
            }
            out.push_str(&word);
            prev = 'a';
            continue;
        }
        out.push(c);
        if !c.is_whitespace() {
            prev = c;
        }
        i += 1;
    }
    out
}

/// 去掉类型写法里的 pl.：pl.List(pl.Int64) → List(Int64)。
fn dtype_plain(text: &str) -> String {
    let mut out = text.to_string();
    for (name, _) in DTYPES.iter().map(|(n, t)| (*n, *t)).chain([("Utf8", ""), ("Field", "")]) {
        out = out.replace(&format!("pl.{name}"), name);
    }
    out
}

// ---------------------------------------------------------------- 选列框的状态

/// 列从哪来：引擎推这个节点（表格级的图里）的表结构，或者只有节点自己知道的列名（read 读文件，没有上游）。
#[derive(Clone, PartialEq, Debug)]
enum Source {
    Engine(u32),
    Local,
}

#[derive(Clone, PartialEq, Debug)]
enum Matched {
    /// 还没问 / 正在问引擎
    Waiting,
    Done(Vec<String>),
    Failed(String),
}

/// 字典参数的一组：一个键（勾的几列，或一段选择器写法）对一个值。
#[derive(Clone, Debug, PartialEq)]
struct Group {
    /// 勾的列（非空时用它们，selector 空）
    names: Vec<String>,
    /// 选择器的写法（cs.string() | cs.by_name("a")…）
    selector: String,
    /// 值的原文（数据类型的是没加 pl. 的写法）
    value: String,
}

impl Group {
    /// 键的写法（问引擎用）。
    fn key(&self) -> String {
        if self.names.is_empty() {
            return self.selector.clone();
        }
        self.names.iter().map(|n| py_str(n)).collect::<Vec<_>>().join(", ")
    }

    /// 界面上键的显示。
    fn label(&self) -> String {
        if self.names.is_empty() {
            self.selector.clone()
        } else {
            self.names.join(", ")
        }
    }
}

pub(crate) struct PickState {
    /// 改哪个参数：节点所在的图的路径、节点、参数下标
    path: Vec<u32>,
    node: u32,
    param: usize,
    mode: Pick,
    insert: ColInsert,
    dict: bool,
    dict_dtype: bool,
    source: Source,
    /// 表里的列：(列名, 类型的短写法)
    columns: Vec<(String, String)>,
    /// ✓ 的列，按点的先后（select 的列顺序就是它）；✗ 的列
    chosen: Vec<String>,
    out: Vec<String>,
    rows: Vec<Row>,
    search: String,
    /// 键盘移到的那一行（搜索后的列表里）
    cursor: usize,
    append: bool,
    dict_value: String,
    /// 字典参数已入组的键值对（一组一组加：选列、填值、加一组）
    groups: Vec<Group>,
    /// 每组在表上选到哪几列（asks 和它对着：先是各组，最后是还没入组的当前选择）
    group_matched: Vec<Matched>,
    /// 点了哪一组只看它的列（-1 = 全部）
    show_group: i32,
    /// 原来的写法读不回来时的说明
    note: String,
    /// 上游推不出来时的原因
    upstream: String,
    matched: Matched,
    /// 最后一次问引擎的写法
    asked: String,
}

/// 选列框正在等的那次回复；又点了别的就作废
static TICKET: AtomicU64 = AtomicU64::new(0);

impl PickState {
    fn mark(&self, name: &str) -> i32 {
        if self.chosen.iter().any(|c| c == name) {
            1
        } else if self.out.iter().any(|c| c == name) {
            2
        } else {
            0
        }
    }

    /// 写好了的条件（还没填好的不算）。
    fn ready_rows(&self) -> Vec<&Row> {
        self.rows.iter().filter(|r| r.code().is_ok()).collect()
    }

    /// 只是勾了几列（没有条件、没有排除）：写成列名。
    fn plain(&self) -> bool {
        self.ready_rows().is_empty() && self.out.is_empty()
    }

    /// 选择器的写法和它最外层运算的优先级；什么都没选时是 None。
    fn selector(&self) -> Option<(String, u8)> {
        let names = (!self.chosen.is_empty()).then(|| {
            let list: Vec<String> = self.chosen.iter().map(|n| py_str(n)).collect();
            (format!("cs.by_name({})", list.join(", ")), ATOM)
        });
        let rows = self.ready_rows();
        let mut chain: Option<(String, u8)> = None;
        for row in rows {
            let code = row.code().expect("ready_rows 只留写好了的");
            chain = Some(match chain {
                None => (code, ATOM),
                Some((left, top)) => {
                    let (sym, _, prec) = OPS[row.op];
                    let left = if top < prec { format!("({left})") } else { left };
                    (format!("{left} {sym} {code}"), prec)
                }
            });
        }
        // “或”优先级最低：和条件合起来不用加括号
        let base = match (names, chain) {
            (Some((n, _)), Some((r, _))) => Some((format!("{n} | {r}"), 1)),
            (n, r) => n.or(r),
        };
        if self.out.is_empty() {
            return base;
        }
        let (left, top) = base.unwrap_or((String::from("cs.all()"), ATOM));
        let left = if top < 4 { format!("({left})") } else { left };
        let out: Vec<String> = self.out.iter().map(|n| py_str(n)).collect();
        Some((format!("{left} - cs.by_name({})", out.join(", ")), 4))
    }

    /// 选的这些写成参数里的样子（字典是键）。有条件时：“只要列名”的参数写不了选择器，
    /// 用引擎算出来的具体列名；Select 模式本身能写选择器。
    fn keys_text(&self) -> String {
        if self.mode == Pick::One {
            return match (self.chosen.first(), self.insert) {
                (Some(name), ColInsert::Expr) => format!("pl.col({})", py_str(name)),
                (Some(name), _) => py_str(name),
                (None, _) => String::new(),
            };
        }
        if self.plain() {
            let names: Vec<String> = self.chosen.iter().map(|n| py_str(n)).collect();
            if names.is_empty() {
                return String::new();
            }
            return match self.insert {
                ColInsert::Expr if !self.dict => format!("pl.col({})", names.join(", ")),
                _ => names.join(", "),
            };
        }
        self.selector().map(|(code, _)| code).unwrap_or_default()
    }

    /// 还没入组的选择对应的键（写好的样子：列名带引号，选择器原样）。
    /// “只要列名”的参数有条件时：用引擎算出来的具体列名（算好了才有，正在算 / 出错是空的）。
    fn pending_keys(&self) -> Vec<String> {
        if self.keys_text().is_empty() {
            return Vec::new();
        }
        if self.mode == Pick::Names && !self.plain() {
            return match &self.matched {
                Matched::Done(names) => names.iter().map(|n| py_str(n)).collect(),
                _ => Vec::new(),
            };
        }
        if self.mode != Pick::Select || self.plain() {
            return self.chosen.iter().map(|n| py_str(n)).collect();
        }
        self.selector().map(|(code, _)| vec![code]).unwrap_or_default()
    }

    /// 值写成参数里的样子：数据类型补上 pl.；还没填好时是 …（“确定”这时点不了）。
    fn value_text(&self, value: &str) -> String {
        let value = value.trim();
        match value {
            "" => String::from("…"),
            v if self.dict_dtype => dtype_code(v),
            v => v.to_string(),
        }
    }

    /// 确定时写进参数的（不算“接在后面”）。字典是各组一对对键值，再接上还没入组的当前选择。
    fn output(&self) -> String {
        if !self.dict {
            if self.mode == Pick::One || self.plain() {
                return self.keys_text();
            }
            // 有条件：“只要列名”的参数展开成具体列名，Select 写选择器
            if self.mode == Pick::Names {
                return self.pending_keys().join(", ");
            }
            return self.keys_text();
        }
        let mut parts: Vec<String> = Vec::new();
        if !self.append {
            for g in &self.groups {
                if g.value.trim().is_empty() {
                    continue;
                }
                let v = self.value_text(&g.value);
                if g.names.is_empty() {
                    parts.push(format!("{}: {}", g.selector, v));
                } else {
                    parts.extend(g.names.iter().map(|n| format!("{}: {}", py_str(n), v)));
                }
            }
        }
        parts.extend(self.pending_pairs());
        parts.join(", ")
    }

    /// 还没入组的当前选择（勾的列 / 条件）+ 值 → 一对对键值。
    fn pending_pairs(&self) -> Vec<String> {
        let value = self.dict_value.trim();
        if value.is_empty() {
            return Vec::new();
        }
        let v = self.value_text(value);
        self.pending_keys().iter().map(|k| format!("{k}: {v}")).collect()
    }

    /// 能不能点“确定”。
    fn ok_enabled(&self) -> bool {
        if self.dict {
            let selecting = !self.keys_text().is_empty();
            let pending_ready = !selecting
                || (!self.dict_value.trim().is_empty()
                    && match self.mode {
                        Pick::Names if !self.plain() => matches!(&self.matched, Matched::Done(names) if !names.is_empty()),
                        _ => true,
                    });
            return pending_ready
                && self.groups.iter().all(|g| !g.value.trim().is_empty())
                && (!self.groups.is_empty() || selecting);
        }
        if self.mode == Pick::Names && !self.plain() {
            // 条件选的列还没算出来（或一列都选不到）不让确定
            return matches!(&self.matched, Matched::Done(names) if !names.is_empty());
        }
        let empty = self.keys_text().is_empty();
        // 替换时什么都不选 = 清空参数
        !(empty && self.append)
    }

    /// 问引擎的几段写法：字典是每组一段（还没入组的当前选择也算一段）。
    /// 问的是选择器本身（“只要列名”的参数确定时才展开成列名）。
    fn asks(&self) -> Vec<String> {
        if !self.dict {
            return vec![self.keys_text()];
        }
        let mut out: Vec<String> = self.groups.iter().map(|g| g.key()).collect();
        if !self.keys_text().is_empty() {
            out.push(self.keys_text());
        }
        out
    }

    /// 搜索后列出的列：先是写法里有、表里没有的（✓ / ✗ 的），再是表里的。下标对 columns 的是 Some。
    fn listed(&self) -> Vec<(String, String, bool)> {
        let needle = self.search.trim().to_lowercase();
        let hit = |name: &str| needle.is_empty() || name.to_lowercase().contains(&needle);
        let known = |name: &String| self.columns.iter().any(|(c, _)| c == name);
        let extras = self.chosen.iter().chain(&self.out).filter(|n| !known(n) && hit(n));
        let extras = extras.map(|n| (n.clone(), String::new(), true));
        let table = self.columns.iter().filter(|(n, _)| hit(n)).map(|(n, t)| (n.clone(), t.clone(), false));
        extras.chain(table).collect()
    }

    /// 点了一列：没选 → ✓ → ✗ → 没选（只要列名时只在 ✓ 和没选之间换）。
    fn toggle(&mut self, name: &str) {
        match self.mark(name) {
            0 => self.chosen.push(name.to_string()),
            1 => {
                self.chosen.retain(|c| c != name);
                if self.mode != Pick::One {
                    self.out.push(name.to_string());
                }
            }
            _ => self.out.retain(|c| c != name),
        }
    }
}

// ---------------------------------------------------------------- 读回原来的写法

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Str(String),
    Int(i64),
    Word(String),
    Sym(char),
}

/// 写法里的一段：(在原文里的位置, 是什么)。
#[derive(Clone, Debug)]
struct Ex {
    at: (usize, usize),
    kind: Ast,
}

#[derive(Clone, Debug)]
enum Ast {
    Str(String),
    Int(i64),
    /// 名字：cs.numeric、pl.col、True / False / None、range
    Name(String),
    Call(String, Vec<Ex>, Vec<(String, Ex)>),
    List(Vec<Ex>),
    Not(Box<Ex>),
    /// 二元运算：OPS 的下标
    Bin(usize, Box<Ex>, Box<Ex>),
}

/// 拆成记号，带着在原文里的位置（字节）。
fn tokens(text: &str) -> Option<Vec<(Tok, usize, usize)>> {
    let mut out = Vec::new();
    let mut it = text.char_indices().peekable();
    while let Some(&(i, c)) = it.peek() {
        if c.is_whitespace() {
            it.next();
        } else if c == '"' || c == '\'' {
            it.next();
            let mut s = String::new();
            let end;
            loop {
                let (j, d) = it.next()?;
                if d == c {
                    end = j + d.len_utf8();
                    break;
                }
                if d == '\\' {
                    let (_, e) = it.next()?;
                    s.push(match e {
                        'n' => '\n',
                        't' => '\t',
                        'r' => '\r',
                        '0' => '\0',
                        'u' => {
                            let hex: String = (0..4).filter_map(|_| it.next().map(|(_, h)| h)).collect();
                            char::from_u32(u32::from_str_radix(&hex, 16).ok()?)?
                        }
                        other => other,
                    });
                } else {
                    s.push(d);
                }
            }
            out.push((Tok::Str(s), i, end));
        } else if c.is_ascii_digit() {
            let mut end = i;
            while let Some(&(j, d)) = it.peek() {
                if !d.is_ascii_digit() {
                    break;
                }
                end = j + 1;
                it.next();
            }
            out.push((Tok::Int(text[i..end].parse().ok()?), i, end));
        } else if c.is_alphabetic() || c == '_' {
            let mut end = i;
            while let Some(&(j, d)) = it.peek() {
                if !(d.is_alphanumeric() || d == '_' || d == '.') {
                    break;
                }
                end = j + d.len_utf8();
                it.next();
            }
            out.push((Tok::Word(text[i..end].to_string()), i, end));
        } else if "()[]{},=|&-^~:".contains(c) {
            out.push((Tok::Sym(c), i, i + 1));
            it.next();
        } else {
            return None;
        }
    }
    Some(out)
}

struct Parser {
    toks: Vec<(Tok, usize, usize)>,
    at: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.at).map(|(t, ..)| t)
    }

    fn eat(&mut self, c: char) -> bool {
        if self.peek() == Some(&Tok::Sym(c)) {
            self.at += 1;
            true
        } else {
            false
        }
    }

    fn start(&self) -> usize {
        self.toks.get(self.at).map_or(usize::MAX, |t| t.1)
    }

    fn end(&self) -> usize {
        self.toks.get(self.at.wrapping_sub(1)).map_or(0, |t| t.2)
    }

    /// 逗号分开的几项，到 close（或者结尾）为止。
    fn items(&mut self, close: Option<char>) -> Option<Vec<Ex>> {
        let mut items = Vec::new();
        loop {
            if close.is_some_and(|c| self.eat(c)) || (close.is_none() && self.peek().is_none()) {
                return Some(items);
            }
            items.push(self.expr(0)?);
            if !self.eat(',') {
                return match close {
                    Some(c) => self.eat(c).then_some(items),
                    None => self.peek().is_none().then_some(items),
                };
            }
        }
    }

    fn expr(&mut self, min: u8) -> Option<Ex> {
        let mut left = self.unary()?;
        loop {
            let Some(Tok::Sym(c)) = self.peek().cloned() else { return Some(left) };
            let Some(op) = OPS.iter().position(|(sym, ..)| sym.starts_with(c)) else { return Some(left) };
            let prec = OPS[op].2;
            if prec <= min {
                return Some(left);
            }
            self.at += 1;
            let right = self.expr(prec)?;
            left = Ex { at: (left.at.0, right.at.1), kind: Ast::Bin(op, Box::new(left), Box::new(right)) };
        }
    }

    fn unary(&mut self) -> Option<Ex> {
        let start = self.start();
        if self.eat('~') {
            let inner = self.unary()?;
            return Some(Ex { at: (start, inner.at.1), kind: Ast::Not(Box::new(inner)) });
        }
        if self.eat('-') {
            let Some((Tok::Int(n), _, end)) = self.toks.get(self.at).cloned() else { return None };
            self.at += 1;
            return Some(Ex { at: (start, end), kind: Ast::Int(-n) });
        }
        if self.eat('(') {
            let inner = self.expr(0)?;
            return self.eat(')').then(|| Ex { at: (start, self.end()), kind: inner.kind });
        }
        if self.eat('[') {
            let items = self.items(Some(']'))?;
            return Some(Ex { at: (start, self.end()), kind: Ast::List(items) });
        }
        let (tok, _, end) = self.toks.get(self.at).cloned()?;
        self.at += 1;
        let kind = match tok {
            Tok::Str(s) => Ast::Str(s),
            Tok::Int(n) => Ast::Int(n),
            Tok::Word(w) if self.eat('(') => {
                let (mut args, mut kwargs) = (Vec::new(), Vec::new());
                loop {
                    if self.eat(')') {
                        break;
                    }
                    if let (Some(Tok::Word(k)), Some((Tok::Sym('='), ..))) = (self.peek().cloned(), self.toks.get(self.at + 1)) {
                        self.at += 2;
                        kwargs.push((k, self.expr(0)?));
                    } else {
                        args.push(self.expr(0)?);
                    }
                    if !self.eat(',') {
                        if !self.eat(')') {
                            return None;
                        }
                        break;
                    }
                }
                return Some(Ex { at: (start, self.end()), kind: Ast::Call(w, args, kwargs) });
            }
            Tok::Word(w) => Ast::Name(w),
            Tok::Sym(_) => return None,
        };
        Some(Ex { at: (start, end), kind })
    }
}

/// 读回来的：✓ 的列、✗ 的列、条件。
#[derive(Default, Debug, PartialEq)]
struct Parsed {
    chosen: Vec<String>,
    out: Vec<String>,
    rows: Vec<Row>,
}

fn strings(items: &[Ex]) -> Option<Vec<String>> {
    let mut out = Vec::new();
    for item in items {
        match &item.kind {
            Ast::Str(s) => out.push(s.clone()),
            Ast::List(inner) => out.extend(strings(inner)?),
            _ => return None,
        }
    }
    Some(out)
}

/// cs.by_name("a", "b")（只有列名、没有别的参数）→ 列名。
fn by_name(ex: &Ex) -> Option<Vec<String>> {
    match &ex.kind {
        Ast::Call(f, args, kwargs) if f == "cs.by_name" && kwargs.is_empty() => strings(args),
        _ => None,
    }
}

fn is_atom(ex: &Ex) -> bool {
    match &ex.kind {
        Ast::Not(inner) => is_atom(inner),
        Ast::Call(f, ..) => f.starts_with("cs."),
        _ => false,
    }
}

/// 一个选择器 → 一条条件。
fn atom_row(src: &str, ex: &Ex, op: usize) -> Option<Row> {
    let (neg, ex) = match &ex.kind {
        Ast::Not(inner) => (true, inner.as_ref()),
        _ => (false, ex),
    };
    let Ast::Call(f, args, kwargs) = &ex.kind else { return None };
    let kind = SELECTORS.iter().position(|d| f.strip_prefix("cs.") == Some(d.name))?;
    let def = &SELECTORS[kind];
    let mut row = Row::new(kind);
    row.op = op;
    row.neg = neg;
    let mut args: &[Ex] = args;
    if def.menus[0] == MenuArg::Inner && !args.is_empty() {
        let Ast::Call(inner, a, k) = &args[0].kind else { return None };
        if !a.is_empty() || !k.is_empty() {
            return None;
        }
        row.menus[0] = INNERS.iter().position(|(_, code)| code.strip_suffix("()") == Some(inner))?;
        args = &args[1..];
    }
    let joined = |names: Vec<String>| -> Option<String> {
        (!names.iter().any(|n| n.contains([',', '，']))).then(|| names.join(", "))
    };
    match def.text {
        TextArg::None | TextArg::Zone if !args.is_empty() => return None,
        TextArg::None | TextArg::Zone => {}
        TextArg::Strings | TextArg::Names => row.text = joined(strings(args)?)?,
        TextArg::Pattern => match args {
            [Ex { kind: Ast::Str(s), .. }] => row.text = s.clone(),
            _ => return None,
        },
        TextArg::Indices => {
            let mut parts = Vec::new();
            let mut flat: Vec<&Ex> = Vec::new();
            for a in args {
                match &a.kind {
                    Ast::List(inner) => flat.extend(inner),
                    _ => flat.push(a),
                }
            }
            for a in flat {
                parts.push(match &a.kind {
                    Ast::Int(n) => n.to_string(),
                    Ast::Call(r, ab, k) if r == "range" && k.is_empty() => match ab.as_slice() {
                        [Ex { kind: Ast::Int(a), .. }, Ex { kind: Ast::Int(b), .. }] if b > a => format!("{a}~{}", b - 1),
                        _ => return None,
                    },
                    _ => return None,
                });
            }
            row.text = parts.join(", ");
        }
        TextArg::Dtypes => {
            let (Some(first), Some(last)) = (args.first(), args.last()) else { return None };
            row.text = dtype_plain(&src[first.at.0..last.at.1]);
        }
    }
    for (key, value) in kwargs {
        if let Some(i) = def.flags.iter().position(|(kw, ..)| kw == key) {
            row.flags[i] = match &value.kind {
                Ast::Name(b) if b == "True" => true,
                Ast::Name(b) if b == "False" => false,
                _ => return None,
            };
            continue;
        }
        let slot = |menu: MenuArg| def.menus.iter().position(|m| *m == menu);
        match (key.as_str(), &value.kind) {
            ("time_unit", Ast::Str(u)) => row.menus[slot(MenuArg::Unit)?] = UNITS.iter().position(|x| *x == u.as_str())?,
            ("time_zone", Ast::Name(n)) if n == "None" => row.menus[slot(MenuArg::Zone)?] = 1,
            ("time_zone", Ast::Str(z)) if z == "*" => row.menus[slot(MenuArg::Zone)?] = 2,
            ("time_zone", Ast::Str(z)) => {
                row.menus[slot(MenuArg::Zone)?] = 3;
                row.text = z.clone();
            }
            ("width", Ast::Int(n)) if def.width => row.width = n.to_string(),
            _ => return None,
        }
    }
    Some(row)
}

/// a op b op c（左边一层层往里）→ 几条条件。
fn chain(src: &str, ex: &Ex, rows: &mut Vec<Row>) -> Option<()> {
    match &ex.kind {
        Ast::Bin(op, left, right) if is_atom(right) => {
            chain(src, left, rows)?;
            rows.push(atom_row(src, right, *op)?);
            Some(())
        }
        _ if is_atom(ex) => {
            rows.push(atom_row(src, ex, 0)?);
            Some(())
        }
        _ => None,
    }
}

/// 按“或”拆开：a | b | (c & d) → [a, b, c & d]。
fn or_terms(ex: Ex, terms: &mut Vec<Ex>) {
    match ex.kind {
        Ast::Bin(0, left, right) => {
            or_terms(*left, terms);
            or_terms(*right, terms);
        }
        _ => terms.push(ex),
    }
}

/// 参数原来的写法 → 选列框的状态；读不回来是 None。
fn read_back(text: &str, mode: Pick) -> Option<Parsed> {
    let mut parsed = Parsed::default();
    if text.trim().is_empty() {
        return Some(parsed);
    }
    let mut parser = Parser { toks: tokens(text)?, at: 0 };
    let items = parser.items(None)?;
    let mut selectors = Vec::new();
    for item in items {
        match &item.kind {
            Ast::Str(s) => parsed.chosen.push(s.clone()),
            Ast::Call(f, args, kwargs) if f == "pl.col" && kwargs.is_empty() => parsed.chosen.extend(strings(args)?),
            Ast::Call(f, args, kwargs) if f == "pl.all" && args.is_empty() && kwargs.is_empty() => {
                selectors.push(Ex { at: item.at, kind: Ast::Call(String::from("cs.all"), Vec::new(), Vec::new()) });
            }
            _ => selectors.push(item),
        }
    }
    match mode {
        Pick::One => return (parsed.chosen.len() <= 1 && selectors.is_empty()).then_some(parsed),
        Pick::Names => return selectors.is_empty().then_some(parsed),
        _ => {}
    }
    // 只有一个选择器、最外面是“去掉这几列”：那几列是 ✗
    if selectors.len() == 1 && parsed.chosen.is_empty() {
        if let Ast::Bin(2, left, right) = &selectors[0].kind {
            if let Some(names) = by_name(right) {
                parsed.out = names;
                let left = (**left).clone();
                selectors = match &left.kind {
                    Ast::Call(f, a, k) if f == "cs.all" && a.is_empty() && k.is_empty() => Vec::new(),
                    _ => vec![left],
                };
            }
        }
    }
    let mut terms = Vec::new();
    for s in selectors {
        or_terms(s, &mut terms);
    }
    let mut atoms = Vec::new();
    let mut complex = Vec::new();
    for term in terms {
        match by_name(&term) {
            Some(names) => parsed.chosen.extend(names),
            None if is_atom(&term) => atoms.push(term),
            None => complex.push(term),
        }
    }
    // “或”前后可以换：复杂的那一串放最前面，单个的接在后面
    if complex.len() > 1 {
        return None;
    }
    for c in &complex {
        chain(text, c, &mut parsed.rows)?;
    }
    for a in &atoms {
        parsed.rows.push(atom_row(text, a, 0)?);
    }
    Some(parsed)
}

/// 字典参数的写法 → 分组；读不回来是 None。一对键值是一组：键是一个列名（或 pl.col 的几个）或一段选择器，
/// 值的原文留着（数据类型的去掉 pl.，写回去时 dtype_code 再补上）。
fn parse_dict(text: &str, dtype: bool) -> Option<Vec<Group>> {
    if text.trim().is_empty() {
        return Some(Vec::new());
    }
    let mut parser = Parser { toks: tokens(text)?, at: 0 };
    let mut groups = Vec::new();
    while parser.peek().is_some() {
        let key = parser.expr(0)?;
        if !parser.eat(':') {
            return None;
        }
        let value = parser.expr(0)?;
        let vtext = text[value.at.0..value.at.1].trim();
        let value = if dtype { dtype_plain(vtext) } else { vtext.to_string() };
        let group = match &key.kind {
            Ast::Str(s) => Group { names: vec![s.clone()], selector: String::new(), value },
            Ast::Call(f, args, kwargs) if f == "pl.col" && kwargs.is_empty() => {
                Group { names: strings(args)?, selector: String::new(), value }
            }
            _ => Group {
                names: Vec::new(),
                selector: text[key.at.0..key.at.1].trim().to_string(),
                value,
            },
        };
        groups.push(group);
        if !parser.eat(',') {
            break;
        }
    }
    parser.peek().is_none().then_some(groups)
}

// ---------------------------------------------------------------- 打开、推界面、问引擎

/// 参数要不要“列 ▾”按钮：选列框怎么选、列从哪来。
fn source_of(st: &App, path: &[u32], node: u32, param: usize) -> Option<Source> {
    let g = st.graph_at(path)?;
    let n = g.node(node)?;
    let def = n.kind().params.get(param)?;
    if def.pick() == Pick::Off {
        return None;
    }
    match path {
        // 表格级：第几个输入的列；没有输入的（read）用它自己的列
        [] if n.inputs.is_empty() => (!n.columns.is_empty()).then_some(Source::Local),
        [] => n.inputs.get(def.pick_from).copied().flatten().map(Source::Engine),
        // expr 节点的网络里：expr 节点的输入的列；eval 里面只有元素，不选列
        [top] => st.graph.node(*top)?.inputs.first().copied().flatten().map(Source::Engine),
        _ => None,
    }
}

/// 参数面板上这个参数要不要“列 ▾”按钮。
pub(crate) fn available(st: &App, path: &[u32], node: u32, param: usize) -> bool {
    source_of(st, path, node, param).is_some()
}

/// 节点上次运行留下的列名和类型（短写法）：引擎回复之前先列着。
fn known_columns(st: &App, id: u32) -> Vec<(String, String)> {
    let Some(n) = st.graph.node(id) else { return Vec::new() };
    let mut types: Vec<String> = n.dtypes.iter().map(|t| engine::short_dtype(t)).collect();
    if types.is_empty() && n.primary {
        if let Some((_, native)) = &st.native_types {
            types = n.columns.iter().map(|c| native.iter().find(|(name, _)| name == c).map_or(String::new(), |(_, t)| engine::short_dtype(t))).collect();
        }
    }
    n.columns.iter().enumerate().map(|(i, c)| (c.clone(), types.get(i).cloned().unwrap_or_default())).collect()
}

/// 点了参数旁边的“列 ▾”：读回原来的写法，弹出选列框。
fn open(weak: &slint::Weak<AppWindow>, state: &Arc<Mutex<App>>, param: usize) {
    let Some(app) = weak.upgrade() else { return };
    let mut st = state.lock().unwrap();
    let (path, Some(node)) = st.param_target() else { return };
    let Some(source) = source_of(&st, &path, node, param) else { return };
    let Some(n) = st.graph_at(&path).and_then(|g| g.node(node)) else { return };
    let def = n.kind().params[param].clone();
    let value = n.params[param].clone();
    let title = format!("{} · {}", n.title, def.label);
    let columns = match source {
        Source::Engine(up) => known_columns(&st, up),
        Source::Local => n.columns.iter().map(|c| (c.clone(), String::new())).collect(),
    };
    let mode = def.pick();
    let dict = def.kind == ParamKind::Dict;
    let unreadable = || {
        (!value.trim().is_empty())
            .then(|| format!("原来的写法读不回来，确定会替换掉它（或者打开“接在后面”）：{}", value.trim()))
            .unwrap_or_default()
    };
    // 字典读回成分组；读不回来只好接在后面
    let (parsed, groups, note, append) = if dict {
        match parse_dict(&value, def.pick_dtype) {
            Some(groups) => (Parsed::default(), groups, String::new(), false),
            None => (Parsed::default(), Vec::new(), unreadable(), true),
        }
    } else {
        match read_back(&value, mode) {
            Some(parsed) => (parsed, Vec::new(), String::new(), false),
            None => (Parsed::default(), Vec::new(), unreadable(), false),
        }
    };
    let can_append = !dict || !note.is_empty();
    st.pick = Some(PickState {
        path,
        node,
        param,
        mode,
        insert: def.insert(),
        dict,
        dict_dtype: def.pick_dtype,
        source,
        columns,
        chosen: parsed.chosen,
        out: parsed.out,
        rows: parsed.rows,
        search: String::new(),
        cursor: 0,
        append,
        dict_value: String::new(),
        groups,
        group_matched: Vec::new(),
        show_group: -1,
        note,
        upstream: String::new(),
        matched: Matched::Waiting,
        asked: String::from("\u{0}"),
    });
    let cp = app.global::<ColPick>();
    cp.set_title(SharedString::from(title));
    cp.set_mode(match mode {
        Pick::Select => 0,
        Pick::Names => 1,
        _ => 2,
    });
    cp.set_dict(dict);
    cp.set_dict_dtype(def.pick_dtype);
    cp.set_dict_hint(SharedString::from(def.hint.as_str()));
    cp.set_dict_value(SharedString::new());
    cp.set_search(SharedString::new());
    // 位置每次贴着按钮重新开；拽过的大小（box-w/h）留着
    cp.set_box_x(0.0);
    cp.set_box_y(0.0);
    cp.set_can_append(can_append);
    let kinds: Vec<SharedString> = SELECTORS.iter().map(|d| SharedString::from(d.label)).collect();
    cp.set_kinds(ModelRc::new(VecModel::from(kinds)));
    let ops: Vec<SharedString> = OPS.iter().map(|(_, label, _)| SharedString::from(*label)).collect();
    cp.set_ops(ModelRc::new(VecModel::from(ops)));
    let names: Vec<SharedString> = DTYPES.iter().map(|(name, _)| SharedString::from(*name)).collect();
    cp.set_dtype_names(ModelRc::new(VecModel::from(names)));
    cp.set_columns(ModelRc::new(VecModel::<PickCol>::default()));
    cp.set_rows(ModelRc::new(VecModel::<PickRow>::default()));
    push(&app, &st, true);
    cp.set_open(true);
    drop(st);
    ask(weak, state);
}

fn row_item(row: &Row, first: bool) -> PickRow {
    let def = row.def();
    let flag = |i: usize| SharedString::from(def.flags[i].1);
    let menu = |i: usize| -> (SharedString, ModelRc<SharedString>) {
        let options: Vec<SharedString> = menu_options(def.menus[i]).into_iter().map(SharedString::from).collect();
        (SharedString::from(menu_label(def.menus[i])), ModelRc::new(VecModel::from(options)))
    };
    let (menu1_label, menu1_options) = menu(0);
    let (menu2_label, menu2_options) = menu(1);
    PickRow {
        op: row.op as i32,
        neg: row.neg,
        kind: row.kind as i32,
        first,
        has_text: row.shows_text(),
        text_label: SharedString::from(def.text_label),
        text_hint: SharedString::from(def.hint),
        text: SharedString::from(row.text.as_str()),
        dtype_text: def.text == TextArg::Dtypes,
        flag1_label: flag(0),
        flag1: row.flags[0],
        flag2_label: flag(1),
        flag2: row.flags[1],
        menu1_label,
        menu1_options,
        menu1: row.menus[0] as i32,
        menu2_label,
        menu2_options,
        menu2: row.menus[1] as i32,
        has_width: def.width,
        width: SharedString::from(row.width.as_str()),
        note: SharedString::from(row.code().err().unwrap_or_default()),
    }
}

/// 把状态推到界面。lists：条件、分组整个换（加 / 删 / 换种类之后）；打字时只改写法和结果，不动那几行。
fn push(app: &AppWindow, st: &App, lists: bool) {
    let Some(p) = &st.pick else { return };
    let cp = app.global::<ColPick>();
    // 字典：任一组（或点了的那一组）选到的列就是列上的蓝点
    let covered: Vec<String> = if p.dict {
        let mut all: Vec<String> = Vec::new();
        for (i, m) in p.group_matched.iter().enumerate() {
            if p.show_group >= 0 && i as i32 != p.show_group {
                continue;
            }
            if let Matched::Done(names) = m {
                for n in names {
                    if !all.contains(n) {
                        all.push(n.clone());
                    }
                }
            }
        }
        all
    } else {
        Vec::new()
    };
    let matched: &[String] = if p.dict {
        &covered
    } else {
        match &p.matched {
            Matched::Done(names) => names,
            _ => &[],
        }
    };
    let listed = p.listed();
    let items: Vec<PickCol> = (listed.iter().enumerate())
        .map(|(i, (name, dtype, extra))| PickCol {
            name: SharedString::from(name.as_str()),
            dtype: SharedString::from(dtype.as_str()),
            mark: p.mark(name),
            matched: matched.contains(name),
            extra: *extra,
            cursor: i == p.cursor,
        })
        .collect();
    set_model(&cp.get_columns(), items, |m| cp.set_columns(m));
    if lists {
        let items: Vec<PickRow> = p.rows.iter().enumerate().map(|(i, r)| row_item(r, i == 0)).collect();
        set_model(&cp.get_rows(), items, |m| cp.set_rows(m));
        let items: Vec<PickGroup> = p.groups
            .iter()
            .enumerate()
            .map(|(i, g)| {
                let (count, err) = match p.group_matched.get(i) {
                    Some(Matched::Done(names)) => (format!("{} 列", names.len()), String::new()),
                    Some(Matched::Failed(e)) => (String::new(), e.clone()),
                    _ => (String::new(), String::new()),
                };
                PickGroup {
                    key: SharedString::from(g.label()),
                    value: SharedString::from(g.value.as_str()),
                    is_selector: g.names.is_empty(),
                    dtype_text: p.dict_dtype,
                    count: SharedString::from(count),
                    err: SharedString::from(err),
                    active: p.show_group == i as i32,
                }
            })
            .collect();
        set_model(&cp.get_groups(), items, |m| cp.set_groups(m));
    }
    let chosen = p.chosen.len();
    let total = p.columns.len();
    let mut summary = match (p.out.len(), p.rows.len()) {
        _ if p.mode == Pick::One => format!("共 {total} 列，点一列就填好"),
        (0, 0) => format!("共 {total} 列 · 选了 {chosen} 列"),
        (out, 0) => format!("共 {total} 列 · 选了 {chosen} 列 · 排除 {out} 列"),
        (out, rows) => format!("共 {total} 列 · 选了 {chosen} 列 · 排除 {out} 列 · {rows} 条条件"),
    };
    if !p.groups.is_empty() {
        summary += &format!(" · {} 组", p.groups.len());
    }
    cp.set_col_summary(SharedString::from(summary));
    let output = p.output();
    cp.set_code(SharedString::from(output.as_str()));
    let (result, state) = if p.dict {
        dict_result(p)
    } else {
        match (&p.matched, p.upstream.is_empty()) {
            (_, false) => (format!("上游算不出来：{}", p.upstream), 2),
            _ if p.keys_text().is_empty() => (String::from("什么都没选"), 3),
            (Matched::Waiting, _) => (String::from("正在问 polars 会选到哪几列…"), 1),
            (Matched::Failed(e), _) => (e.clone(), 2),
            (Matched::Done(names), _) if names.is_empty() => (String::from("现在一列都选不到"), 2),
            (Matched::Done(names), _) => {
                let shown: Vec<&str> = names.iter().take(12).map(String::as_str).collect();
                let more = if names.len() > 12 { format!(" …（共 {} 列）", names.len()) } else { String::new() };
                (format!("会选到 {} 列：{}{more}", names.len(), shown.join("、")), 0)
            }
        }
    };
    cp.set_result(SharedString::from(result));
    cp.set_result_state(state);
    cp.set_note(SharedString::from(p.note.as_str()));
    cp.set_append(p.append);
    cp.set_can_stage(p.dict && !p.keys_text().is_empty() && !p.dict_value.trim().is_empty());
    cp.set_ok_enabled(p.ok_enabled());
}

/// 字典参数底下“结果”一行：会转几列、哪几列被多个分组选中（后面那组生效）。
fn dict_result(p: &PickState) -> (String, i32) {
    if !p.upstream.is_empty() {
        return (format!("上游算不出来：{}", p.upstream), 2);
    }
    if p.group_matched.iter().any(|m| *m == Matched::Waiting) {
        return (String::from("正在问 polars 会选到哪几列…"), 1);
    }
    if let Some((i, e)) = p.group_matched.iter().enumerate().find_map(|(i, m)| match m {
        Matched::Failed(e) => Some((i, e.clone())),
        _ => None,
    }) {
        let who = if i < p.groups.len() { format!("第 {} 组", i + 1) } else { String::from("还没入组的选择") };
        return (format!("{who}：{e}"), 2);
    }
    let mut counts: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    let mut union: Vec<String> = Vec::new();
    for m in &p.group_matched {
        if let Matched::Done(names) = m {
            for n in names {
                *counts.entry(n.as_str()).or_default() += 1;
                if !union.contains(n) {
                    union.push(n.clone());
                }
            }
        }
    }
    if union.is_empty() {
        return (String::from("现在一列都选不到"), 2);
    }
    let shown: Vec<&str> = union.iter().take(12).map(String::as_str).collect();
    let more = if union.len() > 12 { format!(" …（共 {} 列）", union.len()) } else { String::new() };
    let mut text = format!("会转 {} 列：{}{more}", union.len(), shown.join("、"));
    let mut overlaps: Vec<&str> = counts.iter().filter(|(_, c)| **c > 1).map(|(n, _)| *n).collect();
    if !overlaps.is_empty() {
        overlaps.sort_by_key(|n| union.iter().position(|u| u.as_str() == *n).unwrap_or(usize::MAX));
        let shown: Vec<&str> = overlaps.iter().take(5).copied().collect();
        let more = if overlaps.len() > 5 { format!(" …（共 {} 列）", overlaps.len()) } else { String::new() };
        text += &format!("；⚠ {} 列被多个分组选中（后面那组生效）：{}{more}", overlaps.len(), shown.join("、"));
    }
    (text, 0)
}

/// 模型是 VecModel 就原地换内容（列表滚到哪不跳），不是就新建一个。
fn set_model<T: Clone + 'static>(model: &ModelRc<T>, items: Vec<T>, set: impl FnOnce(ModelRc<T>)) {
    match model.as_any().downcast_ref::<VecModel<T>>() {
        Some(vec) => vec.set_vec(items),
        None => set(ModelRc::new(VecModel::from(items))),
    }
}

/// 写法变了：向引擎要现在会选到哪几列（后台；又变了的话旧的回复作废）。read 节点的列只有名字，在这里对。
fn ask(weak: &slint::Weak<AppWindow>, state: &Arc<Mutex<App>>) {
    let mut st = state.lock().unwrap();
    let st = &mut *st;
    let Some(p) = st.pick.as_mut() else { return };
    let dict = p.dict;
    let texts = p.asks();
    let text = texts.join("\u{1}");
    if text == p.asked {
        return;
    }
    p.asked = text.clone();
    // read 这类没有上游的节点列是自己的；但条件（选择器）要 polars 算，改问引擎推它自己的表结构
    let engine_up = match p.source {
        Source::Engine(up) => Some(up),
        Source::Local => {
            let conditional = p.rows.iter().any(|r| r.code().is_ok()) || !p.out.is_empty();
            conditional.then_some(p.node)
        }
    };
    let Some(source) = engine_up else {
        // 没有上游：只能是列名
        let names: Vec<String> = p.columns.iter().map(|(n, _)| n.clone()).collect();
        let check = |list: &[String]| match list.iter().find(|c| !names.contains(*c)) {
            Some(m) => Matched::Failed(format!("表里没有“{m}”这一列")),
            None => Matched::Done(list.to_vec()),
        };
        if dict {
            p.group_matched = p.groups
                .iter()
                .map(|g| {
                    if g.names.is_empty() {
                        Matched::Failed(String::from("这个参数只能选具体的列"))
                    } else {
                        check(&g.names)
                    }
                })
                .collect();
            if !p.keys_text().is_empty() {
                p.group_matched.push(check(&p.chosen));
            }
        } else {
            p.matched = check(&p.chosen);
        }
        if let Some(app) = weak.upgrade() {
            push(&app, st, false);
        }
        return;
    };
    p.matched = Matched::Waiting;
    if dict {
        p.group_matched = texts.iter().map(|_| Matched::Waiting).collect();
    }
    let ticket = TICKET.fetch_add(1, Ordering::SeqCst) + 1;
    let steps = st.graph.compile(source);
    if let Some(app) = weak.upgrade() {
        push(&app, st, false);
    }
    let steps = match steps {
        Ok(steps) => steps,
        Err(e) => {
            if let Some(p) = st.pick.as_mut() {
                p.upstream = e.msg;
            }
            if let Some(app) = weak.upgrade() {
                push(&app, st, false);
            }
            return;
        }
    };
    let (weak, state) = (weak.clone(), state.clone());
    std::thread::spawn(move || {
        let wanted = move || TICKET.load(Ordering::SeqCst) == ticket;
        let reply = if dict {
            engine::pick_many(&steps, source, &texts, &wanted)
        } else {
            engine::pick(&steps, source, &texts[0], &wanted)
        };
        let Some(reply) = reply else { return };
        let _ = slint::invoke_from_event_loop(move || {
            if !wanted() {
                return;
            }
            let mut st = state.lock().unwrap();
            let Some(p) = st.pick.as_mut() else { return };
            match reply {
                Ok(picked) => {
                    p.upstream.clear();
                    if !picked.columns.is_empty() {
                        p.columns = picked.columns.into_iter().map(|(n, t)| (n, engine::short_dtype(&t))).collect();
                    }
                    if dict {
                        p.group_matched = picked.groups.into_iter().map(|r| match r {
                            Ok(names) => Matched::Done(names),
                            Err(e) => Matched::Failed(e),
                        }).collect();
                    } else {
                        p.matched = match picked.matched {
                            Ok(names) => Matched::Done(names),
                            Err(e) => Matched::Failed(e),
                        };
                    }
                }
                Err(e) => p.upstream = e,
            }
            if let Some(app) = weak.upgrade() {
                push(&app, &st, false);
            }
        });
    });
}

/// 改了状态之后：推界面，再问引擎。
fn update(weak: &slint::Weak<AppWindow>, state: &Arc<Mutex<App>>, rows: bool, f: impl FnOnce(&mut PickState)) {
    {
        let mut st = state.lock().unwrap();
        let Some(p) = st.pick.as_mut() else { return };
        f(p);
        if let Some(app) = weak.upgrade() {
            push(&app, &st, rows);
        }
    }
    ask(weak, state);
}

/// 条件整个换（加 / 删 / 换种类）：推迟到当前回调返回后，不在下拉框自己的回调里销毁它。
fn update_rows(weak: &slint::Weak<AppWindow>, state: &Arc<Mutex<App>>, f: impl FnOnce(&mut PickState) + Send + 'static) {
    let (weak, state) = (weak.clone(), state.clone());
    let _ = slint::invoke_from_event_loop(move || update(&weak, &state, true, f));
}

fn close(weak: &slint::Weak<AppWindow>, state: &Arc<Mutex<App>>) {
    TICKET.fetch_add(1, Ordering::SeqCst);
    state.lock().unwrap().pick = None;
    if let Some(app) = weak.upgrade() {
        app.global::<ColPick>().set_open(false);
    }
}

/// 接在原来的写法后面：前面是逗号、括号、运算符时直接接上，否则加逗号。
fn joined(current: &str, piece: &str) -> String {
    let current = current.trim_end();
    match current.chars().last() {
        None => piece.to_string(),
        Some(c) if ",([{:&|+-*/<>=~".contains(c) => format!("{current} {piece}"),
        Some(_) => format!("{current}, {piece}"),
    }
}

/// 确定：写进参数，重新运行。
fn accept(weak: &slint::Weak<AppWindow>, state: &Arc<Mutex<App>>) {
    {
        let mut st = state.lock().unwrap();
        let Some(p) = st.pick.take() else { return };
        if !p.ok_enabled() {
            st.pick = Some(p);
            return;
        }
        let output = p.output();
        let Some(node) = st.graph_at_mut(&p.path).and_then(|g| g.node_mut(p.node)) else { return };
        let Some(value) = node.params.get_mut(p.param) else { return };
        *value = if p.append { joined(value, &output) } else { output };
        clear_node_error(&mut st, p.node);
    }
    close(weak, state);
    // 参数框要换成新的写法：重建参数面板
    let (weak, state) = (weak.clone(), state.clone());
    let _ = slint::invoke_from_event_loop(move || graph_changed(&weak, &state, true));
}

pub(crate) fn install(app: &AppWindow, state: &Arc<Mutex<App>>) {
    let cp = app.global::<ColPick>();
    {
        let (weak, state) = (app.as_weak(), state.clone());
        cp.on_open_param(move |param| {
            // “按位置”参数（pick = ranges）走范围编辑器，别的走选列框
            let ranges = (|| {
                let st = state.lock().unwrap();
                let (path, node) = st.param_target();
                let node = node?;
                let n = st.graph_at(&path).and_then(|g| g.node(node))?;
                Some(n.kind().params[param.max(0) as usize].pick() == Pick::Ranges)
            })() == Some(true);
            if ranges {
                open_ranges(&weak, &state, param.max(0) as usize);
            } else {
                open(&weak, &state, param.max(0) as usize);
            }
        });
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
        cp.on_search_changed(move |text| {
            update(&weak, &state, false, |p| {
                p.search = text.to_string();
                p.cursor = 0;
            })
        });
    }
    {
        let (weak, state) = (app.as_weak(), state.clone());
        cp.on_col_clicked(move |index| {
            let one = state.lock().unwrap().pick.as_ref().is_some_and(|p| p.mode == Pick::One);
            update(&weak, &state, false, |p| {
                let Some((name, ..)) = p.listed().get(index.max(0) as usize).cloned() else { return };
                p.cursor = index.max(0) as usize;
                if p.mode == Pick::One {
                    p.chosen = vec![name];
                } else {
                    p.toggle(&name);
                }
            });
            // 只要一列：点了就填好
            if one {
                accept(&weak, &state);
            }
        });
    }
    {
        let (weak, state) = (app.as_weak(), state.clone());
        cp.on_move_cursor(move |delta| {
            update(&weak, &state, false, |p| {
                let n = p.listed().len();
                if n > 0 {
                    p.cursor = (p.cursor as i32 + delta).clamp(0, n as i32 - 1) as usize;
                }
            })
        });
    }
    {
        // 搜索框里回车：勾选 / 排除光标那一列（只要一列时直接填好）
        let (weak, state) = (app.as_weak(), state.clone());
        cp.on_enter(move || {
            let cursor = state.lock().unwrap().pick.as_ref().map(|p| p.cursor);
            if let (Some(cursor), Some(app)) = (cursor, weak.upgrade()) {
                app.global::<ColPick>().invoke_col_clicked(cursor as i32);
            }
        });
    }
    {
        // 0 全选 / 1 全不选 / 2 反选：只对搜出来的列
        let (weak, state) = (app.as_weak(), state.clone());
        cp.on_bulk(move |what| {
            update(&weak, &state, false, |p| {
                let names: Vec<String> = p.listed().into_iter().map(|(n, ..)| n).collect();
                for name in names {
                    let mark = p.mark(&name);
                    p.chosen.retain(|c| *c != name);
                    p.out.retain(|c| *c != name);
                    let on = match what {
                        0 => true,
                        2 => mark != 1,
                        _ => false,
                    };
                    if on {
                        p.chosen.push(name);
                    }
                }
            })
        });
    }
    {
        let (weak, state) = (app.as_weak(), state.clone());
        cp.on_add_row(move || {
            update_rows(&weak, &state, |p| {
                // 新的一条默认“类型：数字”，接在后面的默认“或者”
                let kind = SELECTORS.iter().position(|d| d.name == "numeric").unwrap_or(0);
                p.rows.push(Row::new(kind));
            })
        });
    }
    {
        let (weak, state) = (app.as_weak(), state.clone());
        cp.on_remove_row(move |i| {
            update_rows(&weak, &state, move |p| {
                if (i as usize) < p.rows.len() {
                    p.rows.remove(i as usize);
                }
            })
        });
    }
    {
        let (weak, state) = (app.as_weak(), state.clone());
        cp.on_row_kind(move |i, kind| {
            update_rows(&weak, &state, move |p| {
                if let Some(row) = p.rows.get_mut(i as usize) {
                    let (op, neg) = (row.op, row.neg);
                    *row = Row::new((kind.max(0) as usize).min(SELECTORS.len() - 1));
                    row.op = op;
                    row.neg = neg;
                }
            })
        });
    }
    {
        let (weak, state) = (app.as_weak(), state.clone());
        cp.on_row_op(move |i, op| {
            update(&weak, &state, false, |p| {
                if let Some(row) = p.rows.get_mut(i as usize) {
                    row.op = (op.max(0) as usize).min(OPS.len() - 1);
                }
            })
        });
    }
    {
        let (weak, state) = (app.as_weak(), state.clone());
        cp.on_row_neg(move |i, on| {
            update(&weak, &state, false, |p| {
                if let Some(row) = p.rows.get_mut(i as usize) {
                    row.neg = on;
                }
            })
        });
    }
    {
        let (weak, state) = (app.as_weak(), state.clone());
        cp.on_row_text(move |i, text| {
            update(&weak, &state, false, |p| {
                if let Some(row) = p.rows.get_mut(i as usize) {
                    row.text = text.to_string();
                }
            });
            refresh_row_note(&weak, &state, i);
        });
    }
    {
        let (weak, state) = (app.as_weak(), state.clone());
        cp.on_row_width(move |i, text| {
            update(&weak, &state, false, |p| {
                if let Some(row) = p.rows.get_mut(i as usize) {
                    row.width = text.to_string();
                }
            });
            refresh_row_note(&weak, &state, i);
        });
    }
    {
        let (weak, state) = (app.as_weak(), state.clone());
        cp.on_row_flag(move |i, which, on| {
            update(&weak, &state, false, |p| {
                if let Some(row) = p.rows.get_mut(i as usize) {
                    row.flags[(which.max(0) as usize).min(1)] = on;
                }
            })
        });
    }
    {
        // 下拉换了：时区选“指定”时文字框才出来，整行重推
        let (weak, state) = (app.as_weak(), state.clone());
        cp.on_row_menu(move |i, which, at| {
            update_rows(&weak, &state, move |p| {
                if let Some(row) = p.rows.get_mut(i as usize) {
                    row.menus[(which.max(0) as usize).min(1)] = at.max(0) as usize;
                }
            })
        });
    }
    {
        // 类型：下拉选的类型接到文字后面
        let (weak, state) = (app.as_weak(), state.clone());
        cp.on_row_dtype(move |i, at| {
            update_rows(&weak, &state, move |p| {
                let Some((_, template)) = DTYPES.get(at.max(0) as usize) else { return };
                if let Some(row) = p.rows.get_mut(i as usize) {
                    row.text = if row.text.trim().is_empty() { template.to_string() } else { format!("{}, {template}", row.text.trim_end()) };
                }
            })
        });
    }
    {
        let (weak, state) = (app.as_weak(), state.clone());
        cp.on_append_toggled(move |on| update(&weak, &state, false, |p| p.append = on));
    }
    {
        let (weak, state) = (app.as_weak(), state.clone());
        cp.on_dict_value_edited(move |text| update(&weak, &state, false, |p| p.dict_value = text.to_string()));
    }
    {
        let (weak, state) = (app.as_weak(), state.clone());
        cp.on_dict_dtype_picked(move |at| {
            let Some((_, template)) = DTYPES.get(at.max(0) as usize) else { return };
            if let Some(app) = weak.upgrade() {
                app.global::<ColPick>().set_dict_value(SharedString::from(*template));
            }
            update(&weak, &state, false, |p| p.dict_value = template.to_string());
        });
    }
    {
        // 选好的列（或条件）+ 值入组，接着选下一组
        let (weak, state) = (app.as_weak(), state.clone());
        cp.on_add_group(move || {
            update_rows(&weak, &state, |p| {
                let value = p.dict_value.trim().to_string();
                if p.keys_text().is_empty() || value.is_empty() {
                    return;
                }
                // 条件选的（“只要列名”模式）：用引擎算出来的具体列名入组
                let group = if p.mode == Pick::Names && !p.plain() {
                    match &p.matched {
                        Matched::Done(names) if !names.is_empty() => Group { names: names.clone(), selector: String::new(), value },
                        _ => return,
                    }
                } else if p.mode != Pick::Select || p.plain() {
                    Group { names: p.chosen.clone(), selector: String::new(), value }
                } else {
                    Group { names: Vec::new(), selector: p.selector().map(|(c, _)| c).unwrap_or_default(), value }
                };
                p.groups.push(group);
                p.chosen.clear();
                p.out.clear();
                p.rows.clear();
                p.dict_value.clear();
                p.show_group = -1;
            });
            if let Some(app) = weak.upgrade() {
                app.global::<ColPick>().set_dict_value(SharedString::new());
            }
        });
    }
    {
        let (weak, state) = (app.as_weak(), state.clone());
        cp.on_remove_group(move |i| {
            update_rows(&weak, &state, move |p| {
                let i = i as usize;
                if i < p.groups.len() {
                    p.groups.remove(i);
                    if i < p.group_matched.len() {
                        p.group_matched.remove(i);
                    }
                }
                if p.show_group == i as i32 {
                    p.show_group = -1;
                }
            })
        });
    }
    {
        // 点一组：列上的蓝点只看这一组选到的（再点取消）
        let (weak, state) = (app.as_weak(), state.clone());
        cp.on_group_clicked(move |i| {
            update_rows(&weak, &state, move |p| {
                p.show_group = if p.show_group == i { -1 } else { i };
            })
        });
    }
    {
        let (weak, state) = (app.as_weak(), state.clone());
        cp.on_group_value_edited(move |i, text| {
            {
                let mut st = state.lock().unwrap();
                let Some(p) = st.pick.as_mut() else { return };
                if let Some(g) = p.groups.get_mut(i.max(0) as usize) {
                    g.value = text.to_string();
                }
                if let Some(app) = weak.upgrade() {
                    push(&app, &st, false);
                }
            }
            ask(&weak, &state);
        });
    }
    {
        // 类型：下拉选的类型接进那一组的值
        let (weak, state) = (app.as_weak(), state.clone());
        cp.on_group_dtype_picked(move |i, at| {
            let Some((_, template)) = DTYPES.get(at.max(0) as usize) else { return };
            let i = i.max(0) as usize;
            if let Some(app) = weak.upgrade() {
                let cp = app.global::<ColPick>();
                if let Some(mut item) = cp.get_groups().row_data(i) {
                    item.value = SharedString::from(*template);
                    cp.get_groups().set_row_data(i, item);
                }
            }
            update(&weak, &state, false, |p| {
                if let Some(g) = p.groups.get_mut(i) {
                    g.value = template.to_string();
                }
            });
        });
    }

    // ---- 范围编辑器（“按位置”参数）
    let rp = app.global::<RangePick>();
    {
        let (weak, state) = (app.as_weak(), state.clone());
        rp.on_cancel(move || close_ranges(&weak, &state));
    }
    {
        let (weak, state) = (app.as_weak(), state.clone());
        rp.on_accept(move || accept_ranges(&weak, &state));
    }
    {
        let (weak, state) = (app.as_weak(), state.clone());
        rp.on_add_row(move || {
            update_ranges_later(&weak, &state, |p| p.rows.push((RangeSide::Open, RangeSide::Open)));
        });
    }
    {
        let (weak, state) = (app.as_weak(), state.clone());
        rp.on_remove_row(move |i| {
            update_ranges_later(&weak, &state, move |p| {
                if (i as usize) < p.rows.len() {
                    p.rows.remove(i as usize);
                }
            })
        });
    }
    {
        let (weak, state) = (app.as_weak(), state.clone());
        rp.on_row_start(move |i, text| {
            update_ranges(&weak, &state, |p| {
                if let Some(row) = p.rows.get_mut(i.max(0) as usize) {
                    row.0 = range_side(&text);
                }
            })
        });
    }
    {
        let (weak, state) = (app.as_weak(), state.clone());
        rp.on_row_end(move |i, text| {
            update_ranges(&weak, &state, |p| {
                if let Some(row) = p.rows.get_mut(i.max(0) as usize) {
                    row.1 = range_side(&text);
                }
            })
        });
    }
    {
        // 下拉选了列：填进那一端（按它当时的位置算，存列名）
        let (weak, state) = (app.as_weak(), state.clone());
        rp.on_row_picked(move |i, which, at| {
            let name = state.lock().unwrap().ranges.as_ref()
                .and_then(|p| p.columns.get(at.max(0) as usize).cloned());
            let Some(name) = name else { return };
            let side = RangeSide::Name(name);
            update_ranges(&weak, &state, move |p| {
                if let Some(row) = p.rows.get_mut(i.max(0) as usize) {
                    if which == 0 { row.0 = side.clone(); } else { row.1 = side; }
                }
            })
        });
    }
}

/// 打字时那一行“还要填什么”的提示跟着变（只改那一行的提示，不动输入框）。
fn refresh_row_note(weak: &slint::Weak<AppWindow>, state: &Arc<Mutex<App>>, i: i32) {
    let Some(app) = weak.upgrade() else { return };
    let st = state.lock().unwrap();
    let Some(row) = st.pick.as_ref().and_then(|p| p.rows.get(i as usize)) else { return };
    let rows = app.global::<ColPick>().get_rows();
    if let Some(mut item) = rows.row_data(i as usize) {
        let note = SharedString::from(row.code().err().unwrap_or_default());
        if item.note != note {
            item.note = note;
            rows.set_row_data(i as usize, item);
        }
    }
}

// ---------------------------------------------------------------- 范围编辑器（“按位置”参数）

/// 一段的端点：序号（第几列）、列名（按它当时的位置算）、开口（:5 / 1: / :）。
#[derive(Clone, PartialEq, Debug)]
enum RangeSide {
    Num(u32),
    Name(String),
    Open,
}

pub(crate) struct RangeState {
    /// 改哪个参数：节点所在的图的路径、节点、参数下标
    path: Vec<u32>,
    node: u32,
    param: usize,
    /// 上游的列（下拉选；空 = 还没算出来，直接填序号）
    columns: Vec<String>,
    /// 一段段 (起, 止)
    rows: Vec<(RangeSide, RangeSide)>,
}

/// 按分隔符拆开，引号里的不算（和 select.py 的 split_top 一个规则）。
fn split_top(text: &str, sep: char) -> Vec<String> {
    let mut parts = Vec::new();
    let mut buf = String::new();
    let mut quote = ' ';
    let mut esc = false;
    for ch in text.chars() {
        if esc {
            buf.push(ch);
            esc = false;
        } else if quote != ' ' {
            if ch == '\\' {
                esc = true;
            }
            buf.push(ch);
            if ch == quote {
                quote = ' ';
            }
        } else if ch == '"' || ch == '\'' {
            quote = ch;
            buf.push(ch);
        } else if ch == '\\' {
            esc = true;
            buf.push(ch);
        } else if ch == sep {
            parts.push(std::mem::take(&mut buf));
        } else {
            buf.push(ch);
        }
    }
    parts.push(buf);
    parts
}

/// 端点的文字 → 端点：带引号的去引号是列名，纯数字是序号，裸的按列名，空是开口。
fn range_side(text: &str) -> RangeSide {
    let t = text.trim();
    if t.is_empty() {
        return RangeSide::Open;
    }
    if t.len() >= 2
        && let (a, b) = (t.chars().next().unwrap(), t.chars().last().unwrap())
        && (a == '"' || a == '\'') && a == b
    {
        let inner = &t[1..t.len() - 1];
        let mut out = String::new();
        let mut esc = false;
        for ch in inner.chars() {
            if esc {
                out.push(ch);
                esc = false;
            } else if ch == '\\' {
                esc = true;
            } else {
                out.push(ch);
            }
        }
        return RangeSide::Name(out);
    }
    match t.parse::<u32>() {
        Ok(n) => RangeSide::Num(n),
        Err(_) => RangeSide::Name(t.to_string()),
    }
}

impl RangeSide {
    /// 编辑框里显示的：序号是数字，列名不带引号，开口是空。
    fn display(&self) -> String {
        match self {
            RangeSide::Num(n) => n.to_string(),
            RangeSide::Name(name) => name.clone(),
            RangeSide::Open => String::new(),
        }
    }

    /// 写进参数的：序号直接写，列名没有特殊字符时也不带引号，其余加引号转义。
    fn text(&self) -> String {
        match self {
            RangeSide::Open => String::new(),
            RangeSide::Num(n) => n.to_string(),
            RangeSide::Name(name) => {
                let plain = !name.is_empty()
                    && !name.chars().all(|c| c.is_ascii_digit())
                    && !name.chars().any(|c| ",:\"'\\".contains(c) || c.is_whitespace());
                if plain {
                    name.clone()
                } else {
                    format!("\"{}\"", name.replace('\\', "\\\\").replace('"', "\\\""))
                }
            }
        }
    }
}

/// 参数的“按位置”文字 → 一段段；写法不认识（空段、没有冒号）时是 None。
fn parse_ranges(text: &str) -> Option<Vec<(RangeSide, RangeSide)>> {
    if text.trim().is_empty() {
        return Some(Vec::new());
    }
    let mut rows = Vec::new();
    for seg in split_top(text, ',') {
        let seg = seg.trim();
        if seg.is_empty() {
            return None;
        }
        let ends = split_top(seg, ':');
        if ends.len() != 2 {
            return None;
        }
        rows.push((range_side(&ends[0]), range_side(&ends[1])));
    }
    Some(rows)
}

/// 一段段 → 参数的文字（两端都空的段不算）。
fn ranges_text(rows: &[(RangeSide, RangeSide)]) -> String {
    rows.iter()
        .filter(|(a, b)| !matches!((a, b), (RangeSide::Open, RangeSide::Open)))
        .map(|(a, b)| format!("{}:{}", a.text(), b.text()))
        .collect::<Vec<_>>()
        .join(", ")
}

/// 点了参数旁边的“范围”：读回原来的写法，弹出范围编辑器。
fn open_ranges(weak: &slint::Weak<AppWindow>, state: &Arc<Mutex<App>>, param: usize) {
    let Some(app) = weak.upgrade() else { return };
    let mut st = state.lock().unwrap();
    let (path, Some(node)) = st.param_target() else { return };
    let Some(n) = st.graph_at(&path).and_then(|g| g.node(node)) else { return };
    let def = &n.kind().params[param];
    let value = n.params[param].clone();
    let title = format!("{} · {}", n.title, def.label);
    let columns: Vec<String> = match source_of(&st, &path, node, param) {
        Some(Source::Engine(up)) => known_columns(&st, up).into_iter().map(|(c, _)| c).collect(),
        Some(Source::Local) => n.columns.clone(),
        None => Vec::new(),
    };
    let (rows, note) = match parse_ranges(&value) {
        Some(rows) => (rows, String::new()),
        None => (
            Vec::new(),
            (!value.trim().is_empty())
                .then(|| format!("原来的写法读不回来，确定会替换掉它：{}", value.trim()))
                .unwrap_or_default(),
        ),
    };
    // 下拉里带着位置：选了就知道按第几列算
    let items: Vec<SharedString> = columns
        .iter()
        .enumerate()
        .map(|(i, name)| SharedString::from(format!("{name}（第 {} 列）", i + 1)))
        .collect();
    st.ranges = Some(RangeState { path, node, param, columns, rows });
    let rp = app.global::<RangePick>();
    rp.set_title(SharedString::from(title));
    rp.set_box_x(0.0);
    rp.set_box_y(0.0);
    rp.set_columns(ModelRc::new(VecModel::from(items)));
    rp.set_note(SharedString::from(note));
    push_ranges(&app, &st);
    rp.set_open(true);
}

/// 把状态推给界面（各段、写法）。
fn push_ranges(app: &AppWindow, st: &App) {
    let Some(p) = &st.ranges else { return };
    let rp = app.global::<RangePick>();
    let rows: Vec<RangeRow> = p
        .rows
        .iter()
        .map(|(a, b)| RangeRow { start: SharedString::from(a.display()), end: SharedString::from(b.display()) })
        .collect();
    rp.set_rows(ModelRc::new(VecModel::from(rows)));
    rp.set_code(SharedString::from(ranges_text(&p.rows)));
    rp.set_ok_enabled(true);
}

fn update_ranges(weak: &slint::Weak<AppWindow>, state: &Arc<Mutex<App>>, f: impl FnOnce(&mut RangeState)) {
    let mut st = state.lock().unwrap();
    let Some(p) = st.ranges.as_mut() else { return };
    f(p);
    if let Some(app) = weak.upgrade() {
        push_ranges(&app, &st);
    }
}

/// 加 / 删一段：推迟到当前回调返回后，不在回调里销毁它旁边的控件。
fn update_ranges_later(weak: &slint::Weak<AppWindow>, state: &Arc<Mutex<App>>, f: impl FnOnce(&mut RangeState) + Send + 'static) {
    let (weak, state) = (weak.clone(), state.clone());
    let _ = slint::invoke_from_event_loop(move || update_ranges(&weak, &state, f));
}

fn close_ranges(weak: &slint::Weak<AppWindow>, state: &Arc<Mutex<App>>) {
    state.lock().unwrap().ranges = None;
    if let Some(app) = weak.upgrade() {
        app.global::<RangePick>().set_open(false);
    }
}

/// 确定：写进参数，重新运行。
fn accept_ranges(weak: &slint::Weak<AppWindow>, state: &Arc<Mutex<App>>) {
    {
        let mut st = state.lock().unwrap();
        let Some(p) = st.ranges.take() else { return };
        let output = ranges_text(&p.rows);
        let Some(node) = st.graph_at_mut(&p.path).and_then(|g| g.node_mut(p.node)) else { return };
        match node.params.get_mut(p.param) {
            Some(value) => *value = output,
            None => return,
        }
        clear_node_error(&mut st, p.node);
    }
    close_ranges(weak, state);
    // 参数框要换成新的写法：重建参数面板
    let (weak, state) = (weak.clone(), state.clone());
    let _ = slint::invoke_from_event_loop(move || graph_changed(&weak, &state, true));
}

/// 测试用：每种选择器写一条（参数都填上），引擎测试拿去给 polars 认。
#[cfg(test)]
pub(crate) fn every_selector(names: &str) -> Vec<String> {
    (0..SELECTORS.len())
        .map(|kind| {
            let mut row = Row::new(kind);
            row.text = match row.def().text {
                TextArg::Names => names.to_string(),
                TextArg::Indices => String::from("0, -1, 1~2"),
                TextArg::Dtypes => String::from("Int64, List(Int64)"),
                TextArg::Pattern => String::from("^[a-z]$"),
                _ => String::from("a, l"),
            };
            row.menus = [1, 1];
            row.flags = [!row.flags[0], !row.flags[1]];
            if row.def().width {
                row.width = String::from("2");
            }
            row.code().unwrap()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn range_text_roundtrip() {
        let rows = vec![
            (RangeSide::Num(1), RangeSide::Num(3)),
            (RangeSide::Name("ts".into()), RangeSide::Name("收入".into())),
            (RangeSide::Open, RangeSide::Num(5)),
            (RangeSide::Name("a,b".into()), RangeSide::Name("3".into())),
            (RangeSide::Num(2), RangeSide::Open),
        ];
        let text = ranges_text(&rows);
        assert_eq!(text, "1:3, ts:收入, :5, \"a,b\":\"3\", 2:");
        // 写出去的读回来还是原来那段；再写一遍文字不变
        assert_eq!(parse_ranges(&text).unwrap(), rows);
        assert_eq!(ranges_text(&parse_ranges(&text).unwrap()), text);
        // 带引号转义的列名也一样
        let tricky = vec![(RangeSide::Name("a\"b".into()), RangeSide::Name("x:y".into()))];
        let text = ranges_text(&tricky);
        assert_eq!(parse_ranges(&text).unwrap(), tricky);
        // 读不回来的写法是 None（确定会替换）
        assert_eq!(parse_ranges("1:2, oops"), None);
        assert_eq!(parse_ranges("1:2,"), None);
        assert_eq!(parse_ranges(""), Some(vec![]));
    }

    fn state(mode: Pick, insert: ColInsert) -> PickState {
        PickState {
            path: Vec::new(),
            node: 0,
            param: 0,
            mode,
            insert,
            dict: false,
            dict_dtype: false,
            source: Source::Local,
            columns: vec![(String::from("姓名"), String::from("str")), (String::from("年龄"), String::from("i64"))],
            chosen: Vec::new(),
            out: Vec::new(),
            rows: Vec::new(),
            search: String::new(),
            cursor: 0,
            append: false,
            dict_value: String::new(),
            groups: Vec::new(),
            group_matched: Vec::new(),
            show_group: -1,
            note: String::new(),
            upstream: String::new(),
            matched: Matched::Waiting,
            asked: String::new(),
        }
    }

    fn kind(name: &str) -> usize {
        SELECTORS.iter().position(|d| d.name == name).unwrap()
    }

    fn row(name: &str, op: &str) -> Row {
        let mut r = Row::new(kind(name));
        r.op = OPS.iter().position(|(s, ..)| *s == op).unwrap_or(0);
        r
    }

    #[test]
    fn writes_names_and_selectors() {
        let mut p = state(Pick::Select, ColInsert::Name);
        assert_eq!(p.output(), "");
        p.toggle("年龄");
        p.toggle("姓名");
        assert_eq!(p.output(), "\"年龄\", \"姓名\"", "按点的先后");
        p.insert = ColInsert::Expr;
        assert_eq!(p.output(), "pl.col(\"年龄\", \"姓名\")");
        p.insert = ColInsert::Name;
        // 再点一下是排除
        p.toggle("姓名");
        assert_eq!(p.output(), "cs.by_name(\"年龄\") - cs.by_name(\"姓名\")");
        p.chosen.clear();
        assert_eq!(p.output(), "cs.all() - cs.by_name(\"姓名\")");
        p.out.clear();
        // 条件：左边一条条往里算，优先级低的加括号
        p.rows = vec![row("numeric", ""), row("temporal", "|"), row("ends_with", "-")];
        p.rows[2].text = String::from("_id, 编号");
        assert_eq!(p.output(), "(cs.numeric() | cs.temporal()) - cs.ends_with(\"_id\", \"编号\")");
        p.rows[1].neg = true;
        p.rows[2].op = 1; // 并且
        assert_eq!(p.output(), "(cs.numeric() | ~cs.temporal()) & cs.ends_with(\"_id\", \"编号\")");
        p.rows.truncate(2);
        p.rows[1].op = 2; // 去掉：a - b 不用括号
        assert_eq!(p.output(), "cs.numeric() - ~cs.temporal()");
        p.chosen = vec![String::from("姓名")];
        assert_eq!(p.output(), "cs.by_name(\"姓名\") | cs.numeric() - ~cs.temporal()");
        p.out = vec![String::from("年龄")];
        assert_eq!(p.output(), "(cs.by_name(\"姓名\") | cs.numeric() - ~cs.temporal()) - cs.by_name(\"年龄\")");
        // 没填好的条件不算
        p.rows.push(row("matches", "|"));
        assert!(p.rows[2].code().is_err());
        assert_eq!(p.output(), "(cs.by_name(\"姓名\") | cs.numeric() - ~cs.temporal()) - cs.by_name(\"年龄\")");
    }

    #[test]
    fn writes_every_kind_of_argument() {
        let code = |name: &str, f: &dyn Fn(&mut Row)| {
            let mut r = Row::new(kind(name));
            f(&mut r);
            r.code()
        };
        assert_eq!(code("string", &|r| r.flags[0] = true).unwrap(), "cs.string(include_categorical=True)");
        assert_eq!(code("datetime", &|_| {}).unwrap(), "cs.datetime()");
        assert_eq!(code("datetime", &|r| r.menus = [1, 1]).unwrap(), "cs.datetime(time_unit=\"ms\", time_zone=None)");
        assert_eq!(code("datetime", &|r| r.menus = [0, 2]).unwrap(), "cs.datetime(time_zone=\"*\")");
        assert!(code("datetime", &|r| r.menus = [0, 3]).is_err(), "指定时区要填");
        assert_eq!(
            code("datetime", &|r| {
                r.menus = [3, 3];
                r.text = String::from("Asia/Shanghai");
            })
            .unwrap(),
            "cs.datetime(time_unit=\"ns\", time_zone=\"Asia/Shanghai\")"
        );
        assert_eq!(code("duration", &|r| r.menus[0] = 2).unwrap(), "cs.duration(time_unit=\"us\")");
        assert_eq!(code("list", &|r| r.menus[0] = 1).unwrap(), "cs.list(cs.numeric())");
        assert_eq!(
            code("array", &|r| {
                r.menus[0] = 4;
                r.width = String::from("3");
            })
            .unwrap(),
            "cs.array(cs.string(), width=3)"
        );
        assert_eq!(code("by_dtype", &|r| r.text = String::from("Int64, List(String)")).unwrap(), "cs.by_dtype(pl.Int64, pl.List(pl.String))");
        assert_eq!(code("by_name", &|r| { r.text = String::from("a，b"); r.flags[0] = false; }).unwrap(), "cs.by_name(\"a\", \"b\", require_all=False)");
        assert_eq!(code("by_index", &|r| r.text = String::from("0, -1, 2~4")).unwrap(), "cs.by_index(0, -1, range(2, 5))");
        assert!(code("by_index", &|r| r.text = String::from("x")).is_err());
        assert_eq!(code("matches", &|r| r.text = String::from("^\\d+$")).unwrap(), "cs.matches(\"^\\\\d+$\")");
        assert_eq!(code("alpha", &|r| r.flags = [true, true]).unwrap(), "cs.alpha(ascii_only=True, ignore_spaces=True)");
        assert_eq!(code("first", &|r| r.flags[0] = false).unwrap(), "cs.first(strict=False)");
        assert_eq!(dtype_code("Struct({\"Date\": Int64}), pl.Int8"), "pl.Struct({\"Date\": pl.Int64}), pl.Int8");
    }

    #[test]
    fn reads_back_what_it_writes() {
        // 写出来的东西读回来还是同样的状态
        let mut p = state(Pick::Select, ColInsert::Name);
        let cases: Vec<(Vec<&str>, Vec<&str>, Vec<Row>)> = vec![
            (vec!["年龄", "姓名"], vec![], vec![]),
            (vec![], vec!["姓名"], vec![]),
            (vec!["年龄"], vec!["姓名"], vec![]),
            (vec![], vec![], vec![row("numeric", ""), row("temporal", "|"), row("ends_with", "-")]),
            (vec!["x"], vec!["y"], vec![row("numeric", ""), row("string", "&"), row("all", "^")]),
        ];
        for (chosen, out, mut rows) in cases {
            for r in rows.iter_mut() {
                if r.def().text != TextArg::None {
                    r.text = String::from("_id, 编号");
                }
            }
            if let Some(r) = rows.get_mut(1) {
                r.neg = true;
                if r.def().flags[0].0 != "" {
                    r.flags[0] = true;
                }
            }
            p.chosen = chosen.iter().map(|s| s.to_string()).collect();
            p.out = out.iter().map(|s| s.to_string()).collect();
            p.rows = rows;
            let text = p.output();
            let back = read_back(&text, Pick::Select).unwrap_or_else(|| panic!("读不回来：{text}"));
            assert_eq!((back.chosen, back.out, back.rows), (p.chosen.clone(), p.out.clone(), p.rows.clone()), "{text}");
        }
        // 每种选择器的参数都读得回来
        let samples = [
            "cs.datetime(time_unit=\"ns\", time_zone=\"Asia/Shanghai\")",
            "cs.datetime(time_zone=None)",
            "cs.datetime(time_zone=\"*\")",
            "cs.array(cs.string(), width=3)",
            "cs.list(cs.numeric())",
            "cs.by_dtype(pl.Int64, pl.List(pl.String))",
            "cs.by_index(0, -1, range(2, 5))",
            "cs.matches(\"^\\\\d+$\")",
            "cs.alpha(ascii_only=True, ignore_spaces=True)",
            "~cs.first(strict=False)",
            "cs.by_name(\"a\", \"b\", require_all=False)",
        ];
        for s in samples {
            let back = read_back(s, Pick::Select).unwrap_or_else(|| panic!("读不回来：{s}"));
            p.chosen.clear();
            p.out.clear();
            p.rows = back.rows;
            assert_eq!(p.output(), s);
        }
        // 手写的：列名、pl.col、几项并列、外面套括号
        let back = read_back("\"a\", pl.col(\"b\", \"c\"), cs.numeric()", Pick::Select).unwrap();
        assert_eq!(back.chosen, ["a", "b", "c"]);
        assert_eq!(back.rows.len(), 1);
        let back = read_back("cs.string() | (cs.numeric() & ~cs.contains(\"id\"))", Pick::Select).unwrap();
        assert_eq!(back.rows.len(), 3, "复杂的放前面，单个的“或”在后面");
        p.chosen.clear();
        p.out.clear();
        p.rows = back.rows;
        assert_eq!(p.output(), "cs.numeric() & ~cs.contains(\"id\") | cs.string()");
        // 读不回来的
        assert!(read_back("pl.col(\"a\") * 2", Pick::Select).is_none());
        assert!(read_back("(cs.a() | cs.b()) & (cs.c() | cs.d())", Pick::Select).is_none());
        assert!(read_back("cs.numeric()", Pick::Names).is_none(), "只要列名的参数");
        assert_eq!(read_back("\"a\"", Pick::One).unwrap().chosen, ["a"]);
        assert_eq!(read_back("pl.col(\"a\")", Pick::One).unwrap().chosen, ["a"]);
        assert!(read_back("\"a\", \"b\"", Pick::One).is_none());
        assert_eq!(read_back("'x\\'y'", Pick::Names).unwrap().chosen, ["x'y"]);
    }

    #[test]
    fn dict_and_one() {
        let mut p = state(Pick::Select, ColInsert::Name);
        p.dict = true;
        p.dict_dtype = true;
        p.chosen = vec![String::from("a"), String::from("b")];
        assert!(!p.ok_enabled(), "值还没填");
        p.dict_value = String::from("Int64");
        assert_eq!(p.output(), "\"a\": pl.Int64, \"b\": pl.Int64");
        p.rows = vec![row("string", "")];
        assert_eq!(p.output(), "cs.by_name(\"a\", \"b\") | cs.string(): pl.Int64");
        let mut p = state(Pick::One, ColInsert::Expr);
        p.chosen = vec![String::from("年龄")];
        assert_eq!(p.output(), "pl.col(\"年龄\")");
        assert_eq!(joined("\"a\"", "\"b\""), "\"a\", \"b\"");
        assert_eq!(joined("pl.col(\"a\") >", "pl.col(\"b\")"), "pl.col(\"a\") > pl.col(\"b\")");
        // 搜索
        let mut p = state(Pick::Select, ColInsert::Name);
        p.chosen = vec![String::from("不在表里")];
        p.search = String::from("龄");
        let listed: Vec<String> = p.listed().into_iter().map(|(n, ..)| n).collect();
        assert_eq!(listed, ["年龄"]);
        p.search.clear();
        assert_eq!(p.listed()[0], (String::from("不在表里"), String::new(), true));
    }

    #[test]
    fn names_mode_conditions() {
        // “只要列名”的参数也能用条件：问引擎的是选择器，确定时展开成算出来的具体列名
        let mut p = state(Pick::Names, ColInsert::Name);
        p.chosen = vec![String::from("a")];
        p.rows = vec![row("string", "|")];
        assert_eq!(p.keys_text(), "cs.by_name(\"a\") | cs.string()");
        assert_eq!(p.asks(), ["cs.by_name(\"a\") | cs.string()"]);
        assert!(!p.ok_enabled(), "还没算出来不让确定");
        p.matched = Matched::Done(vec![String::from("a"), String::from("b")]);
        assert_eq!(p.output(), "\"a\", \"b\"");
        assert!(p.ok_enabled());
        p.matched = Matched::Done(Vec::new());
        assert!(!p.ok_enabled(), "一列都选不到不让确定");
        // 排除（✗）也可以
        let mut p = state(Pick::Names, ColInsert::Name);
        p.toggle("a");
        p.toggle("a");
        assert_eq!(p.out, ["a"]);
        assert_eq!(p.keys_text(), "cs.all() - cs.by_name(\"a\")");
        p.matched = Matched::Done(vec![String::from("b")]);
        assert_eq!(p.output(), "\"b\"");
        // 字典（schema_overrides 这类）：条件这组用算出来的列名
        let mut p = state(Pick::Names, ColInsert::Name);
        p.dict = true;
        p.dict_dtype = true;
        p.rows = vec![row("string", "")];
        p.dict_value = String::from("Int64");
        p.matched = Matched::Done(vec![String::from("金额"), String::from("单价")]);
        assert_eq!(p.output(), "\"金额\": pl.Int64, \"单价\": pl.Int64");
        assert!(p.ok_enabled());
        assert_eq!(p.pending_keys(), ["\"金额\"", "\"单价\""]);
        p.matched = Matched::Waiting;
        assert!(!p.ok_enabled());
    }

    #[test]
    fn dict_groups() {
        let mut p = state(Pick::Select, ColInsert::Name);
        p.dict = true;
        p.dict_dtype = true;
        // 写出来的分组读回来还是同样的分组
        p.groups = parse_dict("\"数量\": pl.Int64, cs.string(): pl.Float64, \"单价\": Datetime(\"ms\")", true).unwrap();
        assert_eq!(p.groups.len(), 3);
        assert_eq!(p.groups[0].names, ["数量"]);
        assert_eq!(p.groups[1].selector, "cs.string()");
        assert_eq!(p.groups[2].names, ["单价"]);
        assert_eq!(p.groups[2].value, "Datetime(\"ms\")", "值的 pl. 去掉了");
        assert_eq!(p.output(), "\"数量\": pl.Int64, cs.string(): pl.Float64, \"单价\": pl.Datetime(\"ms\")");
        let back = parse_dict(&p.output(), true).unwrap();
        assert_eq!(back, p.groups);
        assert_eq!(p.asks(), ["\"数量\"", "cs.string()", "\"单价\""], "每组问一段");
        // pl.col 的几个列名是一组
        let groups = parse_dict("pl.col(\"a\", \"b\"): pl.Int64", true).unwrap();
        assert_eq!(groups[0].names, ["a", "b"]);
        // 还没入组的当前选择接着写
        p.chosen = vec![String::from("x")];
        p.dict_value = String::from("Int64");
        assert_eq!(p.output(), "\"数量\": pl.Int64, cs.string(): pl.Float64, \"单价\": pl.Datetime(\"ms\"), \"x\": pl.Int64");
        assert!(p.ok_enabled());
        // 值没填的组不让确定
        p.groups[0].value = String::new();
        assert!(!p.ok_enabled());
        assert_eq!(p.asks().len(), 4);
        // 读不回来的
        assert!(parse_dict("\"a\"", true).is_none());
        assert!(parse_dict("\"a\": ", true).is_none(), "值没了：记号拆不出东西");
        assert!(parse_dict("out = 1", true).is_none());
        assert!(parse_dict("", true).unwrap().is_empty());
        assert!(parse_dict("\"a\": 1, \"b\": 2", false).is_some(), "不是类型的值也行（改名的新名字）");
        // 接在后面（读不回来时）：只写还没入组的
        let mut p = state(Pick::Names, ColInsert::Name);
        p.dict = true;
        p.append = true;
        p.chosen = vec![String::from("新列")];
        p.dict_value = String::from("\"新名\"");
        assert_eq!(p.output(), "\"新列\": \"新名\"", "不是类型的值原样写，引号自己带");
    }
}
