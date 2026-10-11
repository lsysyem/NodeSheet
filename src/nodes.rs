//! 节点类型：每种节点是一份定义（元信息 + 参数模板 + polars 代码），不写死在程序里——像 Houdini 的数字资产。
//!
//! 定义文件是一个 `.py`：开头的文档字符串里是 JSON（说明、分类、输入个数、参数模板），后面是节点的代码。
//! 代码在引擎里执行，用 `ch("参数名")` 取参数的值，把结果赋给 `out`（见 `engine/engine.py`）。
//!
//! 定义有三层，同名时后面的盖住前面的：
//! - 内置：仓库里的 `nodes/*.py`，编译时打进程序（`build.rs`）
//! - 用户：`%APPDATA%/nodesheet/nodes/*.py`，启动时扫描；在界面里“存为节点类型”也写到这里
//! - 工程：存在 `.rtv` 里，打开工程时注册，只在这次运行里有效
//!
//! 另外节点实例可以带一份自己的定义（`Source::Instance`，不进注册表）：code 节点加了参数之后就是这样。
//!
//! 节点分两级（像 Houdini 的 SOP / VOP）：表格级的节点传 DataFrame；表达式级的节点（定义里 `"expr": true`）
//! 放在 `expr` 节点的网络里，每个算出一个 polars 表达式，参数里标了 `"port": true` 的同时是输入端口。
//! 表达式级的类型除了 `nodes/*.py` 里精选的，其余由 `engine/gen_catalog.py` 从 polars 的签名生成（`nodes/catalog.json`）。

use serde_json::{Map, Value, json};
use std::path::PathBuf;
use std::sync::{Arc, OnceLock, RwLock};

// static BUILTIN: &[(&str, &str)]：内置节点的（名字, 定义文件内容）
include!(concat!(env!("OUT_DIR"), "/builtin_nodes.rs"));

/// 参数的类型：决定面板里用什么控件，以及 `ch()` 把填的内容变成什么。
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum ParamKind {
    /// 文字，原样传给代码（字符串）
    Text,
    Int,
    Float,
    /// 开关
    Toggle,
    /// 下拉选项；`mode` 的菜单是节点的“方式”，选项少时画成页签
    Menu,
    /// 文件路径：可以点“浏览”选文件
    File,
    /// 要写出的文件路径：点“浏览”弹的是保存对话框
    SaveFile,
    /// Excel 的 sheet：下拉列出文件里的 sheet，留空 = 第一个
    Sheet,
    /// 一个 Python / polars 表达式：`pl.col("a") > 1`、`10`、`"文字"`；留空 = None
    Expr,
    /// polars 的数据类型：`Int64`、`Datetime("ms", "Asia/Shanghai")`、`List(Int64)`（写不写 `pl.` 都行）；
    /// 输入框旁边有全部类型的下拉（`DTYPES`），留空 = None
    Dtype,
    /// 逗号分隔的多个表达式，得到列表
    Exprs,
    /// 逗号分隔的列名（也可以混着写表达式、选择器），得到列表
    Columns,
    /// 键值对 `"a": 1, "b": 2`，得到字典
    Dict,
    /// 关键字参数 `n_rows=10, low_memory=True`，得到字典
    Kwargs,
    /// 多行代码，原样传（字符串）
    Code,
    /// 按钮：点了以后节点的代码带着 `action == 参数名` 执行一次（不走缓存）
    Button,
}

/// 类型在定义文件里的名字，和参数界面编辑器里的说明。
pub const PARAM_KINDS: &[(ParamKind, &str, &str)] = &[
    (ParamKind::Text, "text", "文本"),
    (ParamKind::Int, "int", "整数"),
    (ParamKind::Float, "float", "小数"),
    (ParamKind::Toggle, "toggle", "开关"),
    (ParamKind::Menu, "menu", "下拉菜单"),
    (ParamKind::Columns, "columns", "列名（多个）"),
    (ParamKind::Expr, "expr", "表达式"),
    (ParamKind::Exprs, "exprs", "表达式（多个）"),
    (ParamKind::Dtype, "dtype", "数据类型"),
    (ParamKind::Dict, "dict", "键值对"),
    (ParamKind::Kwargs, "kwargs", "关键字参数"),
    (ParamKind::File, "file", "文件（读）"),
    (ParamKind::SaveFile, "save_file", "文件（写）"),
    (ParamKind::Sheet, "sheet", "Excel 的 sheet"),
    (ParamKind::Code, "code", "多行代码"),
    (ParamKind::Button, "button", "按钮"),
];

impl ParamKind {
    pub fn name(self) -> &'static str {
        PARAM_KINDS.iter().find(|(kind, ..)| *kind == self).map_or("text", |(_, name, _)| name)
    }

    pub fn parse(name: &str) -> Option<ParamKind> {
        PARAM_KINDS.iter().find(|(_, n, _)| *n == name).map(|(kind, ..)| *kind)
    }
}

/// polars 的全部数据类型：(名字, 下拉里选了填进去的写法)。带参数的给一个能直接用的样子，再自己改。
pub const DTYPES: &[(&str, &str)] = &[
    ("Boolean", "Boolean"),
    ("Int8", "Int8"),
    ("Int16", "Int16"),
    ("Int32", "Int32"),
    ("Int64", "Int64"),
    ("Int128", "Int128"),
    ("UInt8", "UInt8"),
    ("UInt16", "UInt16"),
    ("UInt32", "UInt32"),
    ("UInt64", "UInt64"),
    ("UInt128", "UInt128"),
    ("Float16", "Float16"),
    ("Float32", "Float32"),
    ("Float64", "Float64"),
    ("Decimal", "Decimal(38, 2)"),
    ("String", "String"),
    ("Binary", "Binary"),
    ("Categorical", "Categorical"),
    ("Enum", r#"Enum(["a", "b"])"#),
    ("Date", "Date"),
    ("Time", "Time"),
    ("Datetime", r#"Datetime("us")"#),
    ("Duration", r#"Duration("us")"#),
    ("List", "List(Int64)"),
    ("Array", "Array(Int64, 2)"),
    ("Struct", r#"Struct({"a": Int64, "b": String})"#),
    ("Map", "Map(String, Int64)"),
    ("Null", "Null"),
    ("Object", "Object"),
    ("Extension", r#"Extension("名字", Int64)"#),
    ("Unknown", "Unknown"),
];

/// 类型写法里最外层的类型名：`pl.List(Int64)` → List。
pub fn dtype_base(value: &str) -> &str {
    let v = value.trim();
    let v = v.strip_prefix("pl.").unwrap_or(v);
    v.split(|c: char| c == '(' || c.is_whitespace()).next().unwrap_or("")
}

/// 选列框写成什么："列名" 还是 pl.col("列名")。
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum ColInsert {
    None,
    /// "列名"
    Name,
    /// pl.col("列名")
    Expr,
}

/// 参数旁边“列 ▾”弹出的选列框怎么选。
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Pick {
    /// 列名和选择器都行（cs.numeric() - cs.by_name("编号")…），可以多列
    Select,
    /// 只能写列名，可以多列（join 的 on…）
    Names,
    /// 只要一列（transpose 的 column_names…），点一下就填好
    One,
    /// 按位置的范围（select 的“按位置”）：范围编辑器，一段段“起:止”
    Ranges,
    /// 没有选列按钮
    Off,
}

pub const PICKS: &[(Pick, &str, &str)] = &[
    (Pick::Select, "select", "列名或选择器"),
    (Pick::Names, "names", "只要列名"),
    (Pick::One, "one", "只要一列"),
    (Pick::Ranges, "ranges", "按位置的范围"),
    (Pick::Off, "off", "不选列"),
];

/// 参数模板。
#[derive(Clone, PartialEq, Debug)]
pub struct ParamDef {
    /// 代码里 `ch("name")` 用的名字，节点内唯一
    pub name: String,
    /// 面板上显示的名字；定义里没写就是 name
    pub label: String,
    pub kind: ParamKind,
    pub default: String,
    pub hint: String,
    pub required: bool,
    /// Menu 的选项
    pub options: Vec<String>,
    /// Menu：这是节点的“方式”（画成页签 / 写进摘要 / Tab 菜单里搜得到它的选项）
    pub mode: bool,
    /// mode 的选项要不要进 Tab 搜索
    pub search: bool,
    /// Text：传给代码前去掉首尾空白（分隔符这类参数要关掉）
    pub strip: bool,
    /// 选列框写成什么；None = 按类型来
    pub insert: Option<ColInsert>,
    /// 显示条件：这些参数的值都在各自的列表里才显示（不显示的参数按默认值算）
    pub show: Vec<(String, Vec<String>)>,
    /// Menu：选项 → 文件扩展名。节点的文件参数换了文件，菜单跟着扩展名切（read / write 的“格式”）
    pub extensions: Vec<(String, Vec<String>)>,
    /// Sheet：sheet 列表取自哪个文件参数
    pub file: String,
    /// 表达式级节点：这个参数同时是输入端口，连了线就用上游的表达式，没连时用填的值
    pub port: bool,
    /// Button：点了把剪贴板里的文字放进这个参数（查看器自己做，不经过引擎），再重新运行
    pub paste: String,
    /// 选列框怎么选；None = 按类型（`pick()`）
    pub pick: Option<Pick>,
    /// 选列框列的是第几个输入的列（从 0 数；定义文件里从 1 数）
    pub pick_from: usize,
    /// Dict：选列框里每列的值是数据类型（cast_columns 的“列: 类型”），值的输入框旁边有类型下拉
    pub pick_dtype: bool,
}

impl ParamDef {
    pub fn new(name: &str, kind: ParamKind) -> ParamDef {
        ParamDef {
            name: name.to_string(),
            label: name.to_string(),
            kind,
            default: String::from(if kind == ParamKind::Toggle { "False" } else { "" }),
            hint: String::new(),
            required: false,
            options: Vec::new(),
            mode: false,
            search: true,
            strip: true,
            insert: None,
            show: Vec::new(),
            extensions: Vec::new(),
            file: String::new(),
            port: false,
            paste: String::new(),
            pick: None,
            pick_from: 0,
            pick_dtype: false,
        }
    }

    /// 选列框怎么选：没写时按类型——列名（多个）、表达式可以用选择器，只插列名的表达式参数（transpose 的
    /// column_names）只要一列，键值对的键只要列名；表达式级节点的端口连线就行，不弹。
    pub fn pick(&self) -> Pick {
        if self.pick == Some(Pick::Ranges) {
            return Pick::Ranges;
        }
        if self.port || self.insert() == ColInsert::None {
            return Pick::Off;
        }
        self.pick.unwrap_or(match self.kind {
            ParamKind::Columns | ParamKind::Exprs => Pick::Select,
            ParamKind::Expr if self.insert() == ColInsert::Name => Pick::One,
            ParamKind::Expr => Pick::Select,
            ParamKind::Dict => Pick::Names,
            _ => Pick::Off,
        })
    }

    /// 选列框写成什么（"列名" / pl.col("列名")），None = 不选列。
    pub fn insert(&self) -> ColInsert {
        self.insert.unwrap_or(match self.kind {
            ParamKind::Columns | ParamKind::Dict => ColInsert::Name,
            ParamKind::Expr | ParamKind::Exprs => ColInsert::Expr,
            _ => ColInsert::None,
        })
    }

    /// 传给引擎的类型名：不去空白的文本是 raw。
    pub fn engine_kind(&self) -> &'static str {
        if self.kind == ParamKind::Text && !self.strip { "raw" } else { self.kind.name() }
    }

    /// 值算不算没填（必填检查用）。
    pub fn is_blank(&self, value: &str) -> bool {
        if self.strip { value.trim().is_empty() } else { value.is_empty() }
    }

    fn from_json(v: &Value) -> Result<ParamDef, String> {
        let text = |key: &str| v[key].as_str().unwrap_or("").to_string();
        let name = text("name");
        if !is_identifier(&name) {
            return Err(format!("参数名“{name}”不合适：只能用字母、数字、下划线，不能以数字开头"));
        }
        let kind_name = v["type"].as_str().unwrap_or("text");
        let kind = ParamKind::parse(kind_name).ok_or(format!("参数 {name}：不认识的类型 {kind_name}"))?;
        let lists = |v: &Value| -> Vec<(String, Vec<String>)> {
            let pairs = v.as_object().into_iter().flatten();
            pairs.map(|(key, values)| (key.clone(), strings(values))).collect()
        };
        let mut p = ParamDef::new(&name, kind);
        if let Some(label) = v["label"].as_str() {
            p.label = label.to_string();
        }
        p.options = strings(&v["options"]);
        p.default = match v["default"].as_str() {
            Some(default) => default.to_string(),
            // 菜单默认选第一个
            None if kind == ParamKind::Menu => p.options.first().cloned().unwrap_or_default(),
            None => p.default,
        };
        p.hint = text("hint");
        p.required = v["required"].as_bool().unwrap_or(false);
        p.mode = v["mode"].as_bool().unwrap_or(false);
        p.search = v["search"].as_bool().unwrap_or(true);
        p.strip = v["strip"].as_bool().unwrap_or(true);
        p.insert = match v["insert"].as_str() {
            Some("none") => Some(ColInsert::None),
            Some("name") => Some(ColInsert::Name),
            Some("expr") => Some(ColInsert::Expr),
            _ => None,
        };
        p.show = lists(&v["show"]);
        p.extensions = lists(&v["extensions"]);
        p.file = text("file");
        p.port = v["port"].as_bool().unwrap_or(false);
        p.paste = text("paste");
        p.pick = v["pick"].as_str().and_then(|name| PICKS.iter().find(|(_, n, _)| *n == name)).map(|(pick, ..)| *pick);
        p.pick_from = v["pick_from"].as_u64().map_or(0, |n| (n as usize).saturating_sub(1));
        p.pick_dtype = v["pick_dtype"].as_bool().unwrap_or(false);
        Ok(p)
    }

    /// 定义文件里的一行：只写和默认不一样的项，顺序固定。
    fn to_json_text(&self) -> String {
        let blank = ParamDef::new(&self.name, self.kind);
        let pairs = |lists: &[(String, Vec<String>)]| -> Value {
            Value::Object(lists.iter().map(|(key, values)| (key.clone(), json!(values))).collect::<Map<_, _>>())
        };
        let mut items: Vec<(&str, Value)> = vec![("name", json!(self.name))];
        if self.kind != ParamKind::Text {
            items.push(("type", json!(self.kind.name())));
        }
        if self.label != self.name {
            items.push(("label", json!(self.label)));
        }
        let default =
            if self.kind == ParamKind::Menu { self.options.first().cloned().unwrap_or_default() } else { blank.default };
        if self.default != default {
            items.push(("default", json!(self.default)));
        }
        if self.required {
            items.push(("required", json!(true)));
        }
        if !self.strip {
            items.push(("strip", json!(false)));
        }
        if self.port {
            items.push(("port", json!(true)));
        }
        if self.mode {
            items.push(("mode", json!(true)));
        }
        if !self.search {
            items.push(("search", json!(false)));
        }
        if !self.options.is_empty() {
            items.push(("options", json!(self.options)));
        }
        if !self.extensions.is_empty() {
            items.push(("extensions", pairs(&self.extensions)));
        }
        if !self.file.is_empty() {
            items.push(("file", json!(self.file)));
        }
        if !self.paste.is_empty() {
            items.push(("paste", json!(self.paste)));
        }
        if let Some(insert) = self.insert {
            let name = match insert {
                ColInsert::None => "none",
                ColInsert::Name => "name",
                ColInsert::Expr => "expr",
            };
            items.push(("insert", json!(name)));
        }
        if let Some(pick) = self.pick {
            items.push(("pick", json!(PICKS.iter().find(|(p, ..)| *p == pick).map_or("select", |(_, name, _)| name))));
        }
        if self.pick_from > 0 {
            items.push(("pick_from", json!(self.pick_from + 1)));
        }
        if self.pick_dtype {
            items.push(("pick_dtype", json!(true)));
        }
        if !self.hint.is_empty() {
            items.push(("hint", json!(self.hint)));
        }
        if !self.show.is_empty() {
            items.push(("show", pairs(&self.show)));
        }
        object_text(&items)
    }
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array().into_iter().flatten().filter_map(|s| s.as_str().map(String::from)).collect()
}

/// `{"a": 1, "b": 2}`，键按给的顺序排（serde_json 的 Map 会按字母排）。
fn object_text(items: &[(&str, Value)]) -> String {
    let body: Vec<String> = items.iter().map(|(key, value)| format!("{}: {value}", json!(key))).collect();
    format!("{{{}}}", body.join(", "))
}

pub fn is_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_alphabetic() || c == '_') && chars.all(|c| c.is_alphanumeric() || c == '_')
}

/// 定义来自哪一层；同名时大的盖住小的。
#[derive(Clone, Copy, PartialEq, PartialOrd, Debug)]
pub enum Source {
    Builtin,
    User,
    Project,
    /// 某个节点自己的定义，不进注册表
    Instance,
}

/// 节点类型。
#[derive(Clone, PartialEq, Debug)]
pub struct NodeType {
    /// 类型名 = 定义文件名（不带 .py）
    pub name: String,
    pub desc: String,
    pub category: String,
    /// 在分类里排第几（小的在前）
    pub order: i64,
    /// 输入端口数，前 min_inputs 个必须连接
    pub inputs: usize,
    pub min_inputs: usize,
    pub params: Vec<ParamDef>,
    /// 节点的代码（用 ch("名字") 取参数）
    pub code: String,
    /// 代码不在定义里，而是取自这个参数的值（code 节点：代码是用户在面板里写的）
    pub code_param: String,
    pub source: Source,
    /// 表达式级的节点（在 expr 节点的网络里）；输入端口就是标了 port 的参数，inputs 是它们的个数
    pub expr: bool,
    /// 表格级的节点里面有一个表达式网络（expr 节点）
    pub network: bool,
    /// 表达式网络的输出节点：它的值交给上下文的哪一组（列 / 键 / 条件 / 排序）
    pub role: String,
    /// 帮助文档（定义文件开头的文档字符串里、JSON 后面的文字；格式见 `help.rs`）
    pub help: String,
}

const HEADER_MARKS: [&str; 2] = ["r\"\"\"", "\"\"\""];
pub const MAX_INPUTS: usize = 4;
pub const DEFAULT_CATEGORY: &str = "自定义";

impl NodeType {
    pub fn new(name: &str) -> NodeType {
        NodeType {
            name: name.to_string(),
            desc: String::new(),
            category: String::from(DEFAULT_CATEGORY),
            order: 100,
            inputs: 1,
            min_inputs: 1,
            params: Vec::new(),
            code: String::new(),
            code_param: String::new(),
            source: Source::User,
            expr: false,
            network: false,
            role: String::new(),
            help: String::new(),
        }
    }

    /// 解析定义文件。
    pub fn parse(name: &str, text: &str, source: Source) -> Result<NodeType, String> {
        if !is_identifier(name) {
            return Err(format!("类型名“{name}”不合适：只能用字母、数字、下划线，不能以数字开头"));
        }
        let text = text.replace("\r\n", "\n");
        let body = text.trim_start();
        let body = HEADER_MARKS.iter().find_map(|mark| body.strip_prefix(mark));
        let body = body.ok_or("定义文件要以文档字符串开头，里面是 JSON")?;
        let (header, code) = body.split_once("\"\"\"").ok_or("开头的文档字符串没有结束")?;
        // 文档字符串里先是 JSON，后面（可以没有）是帮助文档
        let mut values = serde_json::Deserializer::from_str(header).into_iter::<Value>();
        let v = match values.next() {
            Some(Ok(v)) => v,
            Some(Err(e)) => return Err(format!("开头的 JSON 不对：{e}")),
            None => return Err(String::from("开头的文档字符串里没有 JSON")),
        };
        let help = header[values.byte_offset()..].trim();
        if !v.is_object() {
            return Err(String::from("开头的 JSON 要是一个对象"));
        }
        let mut t = NodeType::new(name);
        t.source = source;
        t.desc = v["desc"].as_str().unwrap_or("").to_string();
        if let Some(category) = v["category"].as_str().filter(|c| !c.trim().is_empty()) {
            t.category = category.trim().to_string();
        }
        t.order = v["order"].as_i64().unwrap_or(100);
        t.inputs = v["inputs"].as_u64().unwrap_or(1) as usize;
        t.min_inputs = v["min_inputs"].as_u64().map_or(t.inputs, |n| n as usize);
        t.code_param = v["code_param"].as_str().unwrap_or("").to_string();
        t.expr = v["expr"].as_bool().unwrap_or(false);
        t.network = v["network"].as_bool().unwrap_or(false);
        t.role = v["role"].as_str().unwrap_or("").to_string();
        t.help = help.replace('\t', "    ");
        for p in v["params"].as_array().into_iter().flatten() {
            t.params.push(ParamDef::from_json(p)?);
        }
        t.sync_ports();
        t.code = code.trim_start_matches('\n').to_string();
        t.check()?;
        Ok(t)
    }

    /// 定义本身有没有毛病（参数重名、显示条件指向不存在的参数…）。
    pub fn check(&self) -> Result<(), String> {
        if !self.expr && (self.inputs > MAX_INPUTS || self.min_inputs > self.inputs) {
            return Err(format!("输入个数不对：最多 {MAX_INPUTS} 个，必连的不能比总数多"));
        }
        for (i, p) in self.params.iter().enumerate() {
            if !is_identifier(&p.name) {
                return Err(format!("参数名“{}”不合适：只能用字母、数字、下划线，不能以数字开头", p.name));
            }
            if self.params[..i].iter().any(|other| other.name == p.name) {
                return Err(format!("有两个参数都叫 {}", p.name));
            }
            if p.kind == ParamKind::Menu && p.options.is_empty() {
                return Err(format!("下拉菜单 {} 还没有选项", p.name));
            }
            for (other, _) in &p.show {
                if !self.params.iter().any(|q| q.name == *other) {
                    return Err(format!("参数 {} 的显示条件里提到了不存在的参数 {other}", p.name));
                }
            }
        }
        if !self.code_param.is_empty() && !self.params.iter().any(|p| p.name == self.code_param) {
            return Err(format!("code_param 指向了不存在的参数 {}", self.code_param));
        }
        if self.help.contains("\"\"\"") {
            return Err(String::from("帮助文档里不能有连着的三个双引号（\"\"\"）"));
        }
        Ok(())
    }

    /// 写成定义文件。
    pub fn to_text(&self) -> String {
        let mut head: Vec<String> = vec![
            format!("  \"desc\": {}", json!(self.desc)),
            format!("  \"category\": {}", json!(self.category)),
            format!("  \"order\": {}", self.order),
        ];
        if self.expr {
            head.push(String::from("  \"expr\": true"));
        } else {
            head.push(format!("  \"inputs\": {}", self.inputs));
        }
        if self.network {
            head.push(String::from("  \"network\": true"));
        }
        if !self.role.is_empty() {
            head.push(format!("  \"role\": {}", json!(self.role)));
        }
        if !self.expr && self.min_inputs != self.inputs {
            head.push(format!("  \"min_inputs\": {}", self.min_inputs));
        }
        if !self.code_param.is_empty() {
            head.push(format!("  \"code_param\": {}", json!(self.code_param)));
        }
        let params: Vec<String> = self.params.iter().map(|p| format!("    {}", p.to_json_text())).collect();
        let params = if params.is_empty() { String::from("[]") } else { format!("[\n{}\n  ]", params.join(",\n")) };
        head.push(format!("  \"params\": {params}"));
        let code = self.code.replace("\r\n", "\n");
        let newline = if code.is_empty() || code.ends_with('\n') { "" } else { "\n" };
        // 帮助文档接在 JSON 后面，空一行
        let help = self.help.replace("\r\n", "\n");
        let help = if help.trim().is_empty() { String::new() } else { format!("\n{}\n", help.trim()) };
        format!("r\"\"\"\n{{\n{}\n}}\n{help}\"\"\"\n{code}{newline}", head.join(",\n"))
    }

    /// 表达式级：输入端口就是标了 port 的参数（按顺序），端口个数跟着参数走。
    pub fn sync_ports(&mut self) {
        if self.expr {
            self.inputs = self.params.iter().filter(|p| p.port).count();
            self.min_inputs = 0;
        }
    }

    /// 参数 i 是第几个输入端口。
    pub fn port_slot(&self, i: usize) -> Option<usize> {
        self.params.get(i).filter(|p| p.port)?;
        Some(self.params[..i].iter().filter(|p| p.port).count())
    }

    /// 起节点名用的短名字：str.pad_start → pad_start、pl.concat_str → concat_str。
    pub fn short_name(&self) -> &str {
        self.name.rsplit('.').next().unwrap_or(&self.name)
    }

    pub fn param(&self, name: &str) -> Option<usize> {
        self.params.iter().position(|p| p.name == name)
    }

    /// 节点的“方式”参数（第一个 mode 菜单）。
    pub fn mode_param(&self) -> Option<usize> {
        self.params.iter().position(|p| p.mode && p.kind == ParamKind::Menu)
    }

    pub fn is_io(&self) -> bool {
        self.category == CATEGORY_ORDER[0]
    }
}

// ---------------------------------------------------------------- 注册表

/// 表格级节点的分类和顺序（对应 polars 文档的章节）；别的分类排在后面。界面里的颜色、图标按这个下标。
pub const CATEGORY_ORDER: [&str; 8] = ["IO", "行", "列", "表达式", "统计", "变形", "多表", "代码"];

/// 表达式级节点的分类和顺序（polars 文档里 Expr 的章节 + 命名空间）。
pub const EXPR_CATEGORY_ORDER: [&str; 24] = [
    "输入", "输出", "运算", "比较", "逻辑", "条件", "空值", "聚合", "窗口", "数学", "类型", "选取", "命名", "文本 str",
    "日期 dt", "列表 list", "数组 arr", "结构 struct", "映射 map", "分类 cat", "二进制 bin", "函数 pl", "代码", "其他",
];

/// 由 polars 签名生成的表达式节点（engine/gen_catalog.py 生成）
const CATALOG: &str = include_str!("../nodes/catalog.json");

/// catalog.json → 表达式级的节点类型。名字和精选的（nodes/*.py）重了的不要，精选的优先。
fn catalog_types(curated: &[Arc<NodeType>]) -> Vec<Arc<NodeType>> {
    let v: Value = serde_json::from_str(CATALOG).expect("nodes/catalog.json 不是 JSON");
    let mut out = Vec::new();
    for (order, item) in v["types"].as_array().into_iter().flatten().enumerate() {
        let name = item["name"].as_str().unwrap_or("");
        if curated.iter().any(|t| t.expr && t.name == name) {
            continue;
        }
        let mut t = NodeType::new(name);
        t.source = Source::Builtin;
        t.expr = true;
        t.desc = item["desc"].as_str().unwrap_or("").to_string();
        t.category = item["category"].as_str().unwrap_or("其他").to_string();
        t.order = 1000 + order as i64;
        t.code = item["code"].as_str().unwrap_or("").to_string();
        // 生成的节点没有自己写的帮助：帮助窗口向引擎要 polars 的文档（str.pad_start → Expr.str.pad_start）
        let api = if name.starts_with("pl.") { name.to_string() } else { format!("Expr.{name}") };
        t.help = format!("@polars\n{api}");
        let params = item["params"].as_array().into_iter().flatten().map(ParamDef::from_json);
        t.params = params.collect::<Result<_, _>>().unwrap_or_else(|e| panic!("catalog.json 里 {name} 的参数有问题：{e}"));
        t.sync_ports();
        out.push(Arc::new(t));
    }
    out
}

static REGISTRY: OnceLock<RwLock<Vec<Arc<NodeType>>>> = OnceLock::new();

fn registry() -> &'static RwLock<Vec<Arc<NodeType>>> {
    REGISTRY.get_or_init(|| {
        let mut types: Vec<Arc<NodeType>> = BUILTIN
            .iter()
            .map(|(name, text)| {
                let parsed = NodeType::parse(name, text, Source::Builtin);
                Arc::new(parsed.unwrap_or_else(|e| panic!("内置节点 {name} 的定义有问题：{e}")))
            })
            .collect();
        let generated = catalog_types(&types);
        types.extend(generated);
        RwLock::new(types)
    })
}

/// 当前生效的表格级类型（同名的只留最上面一层），按分类、order、名字排。
pub fn all() -> Vec<Arc<NodeType>> {
    effective(false)
}

/// 当前生效的表达式级类型。
pub fn all_expr() -> Vec<Arc<NodeType>> {
    effective(true)
}

fn effective(expr: bool) -> Vec<Arc<NodeType>> {
    let types = registry().read().unwrap();
    let mut effective: Vec<Arc<NodeType>> = Vec::new();
    for t in types.iter().filter(|t| t.expr == expr) {
        match effective.iter_mut().find(|e| e.name == t.name) {
            Some(existing) if t.source > existing.source => *existing = t.clone(),
            Some(_) => {}
            None => effective.push(t.clone()),
        }
    }
    let categories = categories_of(&effective, expr);
    let rank = |t: &NodeType| categories.iter().position(|c| *c == t.category).unwrap_or(usize::MAX);
    effective.sort_by(|a, b| (rank(a), a.order, &a.name).cmp(&(rank(b), b.order, &b.name)));
    effective
}

/// 表格级的类型。
pub fn find(name: &str) -> Option<Arc<NodeType>> {
    find_of(name, false)
}

/// 表达式级的类型。
pub fn find_expr(name: &str) -> Option<Arc<NodeType>> {
    find_of(name, true)
}

pub fn find_of(name: &str, expr: bool) -> Option<Arc<NodeType>> {
    let types = registry().read().unwrap();
    let found = types.iter().filter(|t| t.name == name && t.expr == expr);
    found.max_by(|a, b| a.source.partial_cmp(&b.source).unwrap_or(std::cmp::Ordering::Equal)).cloned()
}

/// 内置的那一份（不管有没有被用户的盖住）。
#[cfg(test)]
pub fn builtin(name: &str) -> Option<Arc<NodeType>> {
    builtin_of(name, false)
}

fn builtin_of(name: &str, expr: bool) -> Option<Arc<NodeType>> {
    let types = registry().read().unwrap();
    types.iter().find(|t| t.name == name && t.expr == expr && t.source == Source::Builtin).cloned()
}

/// 定义从哪来的小标签（内置的不标）。节点图的卡片上只标容易误会的两种（canvas）：
/// 盖住了内置的同名节点、节点自己加了参数；参数面板标题栏全都标。
pub fn source_label(t: &NodeType, canvas: bool) -> &'static str {
    match t.source {
        Source::Builtin => "",
        Source::Instance => "私有参数",
        _ if builtin_of(&t.name, t.expr).is_some() => "已改内置",
        _ if canvas => "",
        Source::User => "用户",
        Source::Project => "工程",
    }
}

fn categories_of(types: &[Arc<NodeType>], expr: bool) -> Vec<String> {
    let order: &[&str] = if expr { &EXPR_CATEGORY_ORDER } else { &CATEGORY_ORDER };
    let mut categories: Vec<String> = order.iter().map(|c| c.to_string()).collect();
    for t in types {
        if !categories.contains(&t.category) {
            categories.push(t.category.clone());
        }
    }
    categories
}

/// “添加”菜单里的分类。
pub fn categories() -> Vec<String> {
    categories_of(&all(), false)
}

pub fn expr_categories() -> Vec<String> {
    categories_of(&all_expr(), true)
}

/// 分类的下标（决定图标和颜色）。
pub fn category_index(category: &str) -> usize {
    categories().iter().position(|c| c == category).unwrap_or(CATEGORY_ORDER.len())
}

/// 注册一个类型：同名同层的换掉。
pub fn install(t: NodeType) -> Arc<NodeType> {
    let t = Arc::new(t);
    let mut types = registry().write().unwrap();
    types.retain(|other| !(other.name == t.name && other.source == t.source && other.expr == t.expr));
    types.push(t.clone());
    t
}

pub fn remove(name: &str, source: Source) {
    registry().write().unwrap().retain(|t| !(t.name == name && t.source == source));
}

/// 用户的节点定义放在哪（`RTV_NODES` 可以改）。
pub fn user_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("RTV_NODES") {
        return PathBuf::from(dir);
    }
    let base = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    base.join("nodesheet").join("nodes")
}

/// 扫描程序旁边的 nodes 文件夹和用户目录，注册里面的定义。返回读不了的文件和原因。
pub fn load_user_types() -> Vec<String> {
    let beside_exe = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.join("nodes")));
    let mut errors = Vec::new();
    registry().write().unwrap().retain(|t| t.source != Source::User);
    for dir in beside_exe.into_iter().chain([user_dir()]) {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        let mut files: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
        files.retain(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("py")));
        files.sort();
        for file in files {
            let name = file.file_stem().and_then(|s| s.to_str()).unwrap_or("");
            let parsed = std::fs::read_to_string(&file)
                .map_err(|e| e.to_string())
                .and_then(|text| NodeType::parse(name, &text, Source::User));
            match parsed {
                Ok(t) => {
                    install(t);
                }
                Err(e) => errors.push(format!("{}：{e}", file.display())),
            }
        }
    }
    errors
}

/// 把类型存到用户目录并注册。
pub fn save_user_type(mut t: NodeType) -> Result<Arc<NodeType>, String> {
    t.source = Source::User;
    t.check()?;
    let dir = user_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("无法创建 {}：{e}", dir.display()))?;
    let file = dir.join(format!("{}.py", t.name));
    std::fs::write(&file, t.to_text()).map_err(|e| format!("无法写入 {}：{e}", file.display()))?;
    Ok(install(t))
}

/// 删掉用户目录里的定义（同名的内置节点随之恢复）。
pub fn delete_user_type(name: &str) -> Result<(), String> {
    let file = user_dir().join(format!("{name}.py"));
    if file.exists() {
        std::fs::remove_file(&file).map_err(|e| format!("无法删除 {}：{e}", file.display()))?;
    }
    remove(name, Source::User);
    Ok(())
}

// ---------------------------------------------------------------- 搜索

/// Tab 搜索的一条结果：节点类型，以及（搜到的是“方式”里的某个选项时）那个选项。
#[derive(Clone, PartialEq, Debug)]
pub struct Hit {
    pub kind: Arc<NodeType>,
    pub option: Option<String>,
}

/// 按名字 / 说明搜节点，也搜“方式”里的选项（搜 tail 找到 slice · tail）。空查询列出全部节点。
/// 排序：名字开头匹配 > 名字包含 > 选项匹配 > 说明里提到。
pub fn search(query: &str) -> Vec<Hit> {
    search_in(all(), query, "")
}

/// 表达式级的搜索；prefer：同一档里优先列出这个分类的（选中的节点输出文字时，“文本 str”的排前面）。
pub fn search_expr(query: &str, prefer: &str) -> Vec<Hit> {
    search_in(all_expr(), query, prefer)
}

fn search_in(kinds: Vec<Arc<NodeType>>, query: &str, prefer: &str) -> Vec<Hit> {
    let q = query.trim().to_lowercase();
    let mut hits: Vec<((u8, bool), Hit)> = Vec::new();
    for kind in kinds {
        let name = kind.name.to_lowercase();
        let short = kind.short_name().to_lowercase();
        let other = kind.category != prefer;
        let rank = if name.starts_with(&q) || short.starts_with(&q) {
            0
        } else if name.contains(&q) {
            1
        } else if kind.desc.to_lowercase().contains(&q) || kind.category.to_lowercase().contains(&q) {
            4
        } else {
            u8::MAX
        };
        if rank != u8::MAX {
            hits.push(((rank, other), Hit { kind: kind.clone(), option: None }));
        }
        let mode = kind.mode_param().map(|i| &kind.params[i]).filter(|p| p.search && !q.is_empty());
        for option in mode.into_iter().flat_map(|p| &p.options).filter(|o| **o != kind.name) {
            let lower = option.to_lowercase();
            if lower.contains(&q) {
                let rank = if lower.starts_with(&q) { 2 } else { 3 };
                hits.push(((rank, other), Hit { kind: kind.clone(), option: Some(option.clone()) }));
            }
        }
    }
    // 名字匹配的（开头 / 包含）在前，其中偏好的分类优先；再往后是方式、说明里提到的
    hits.sort_by_key(|((rank, other), _)| (*rank >= 2, *other, *rank));
    hits.into_iter().map(|(_, hit)| hit).collect()
}

/// Python 字符串字面量。
pub fn py_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// 去掉 # 注释（字符串里的 # 不算）。模板代码的注释里常写着 ch("参数名") 的用法，不能当成用到的参数。
fn strip_comments(code: &str) -> String {
    let mut out = String::with_capacity(code.len());
    for line in code.lines() {
        let mut quote: Option<char> = None;
        let mut escaped = false;
        for c in line.chars() {
            match quote {
                Some(q) => {
                    if escaped {
                        escaped = false;
                    } else if c == '\\' {
                        escaped = true;
                    } else if c == q {
                        quote = None;
                    }
                }
                None if c == '#' => break,
                None if c == '"' || c == '\'' => quote = Some(c),
                None => {}
            }
            out.push(c);
        }
        out.push('\n');
    }
    out
}

/// 代码里用到的参数名：`ch("名字")` / `ch('名字')`，按出现顺序、去重；注释里的不算。
pub fn referenced_params(code: &str) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    let code = strip_comments(code);
    let mut rest = code.as_str();
    while let Some(at) = rest.find("ch(") {
        let before = rest[..at].chars().next_back();
        rest = &rest[at + 3..];
        // 前面是标识符的一部分（如 fetch(）就不是 ch
        if before.is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '.') {
            continue;
        }
        let inner = rest.trim_start();
        let Some(quote) = inner.chars().next().filter(|c| *c == '"' || *c == '\'') else { continue };
        if let Some(end) = inner[1..].find(quote) {
            let name = &inner[1..1 + end];
            if is_identifier(name) && !names.iter().any(|n| n == name) {
                names.push(name.to_string());
            }
        }
    }
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn py_str_escapes() {
        assert_eq!(py_str("D:\\a\"b"), "\"D:\\\\a\\\"b\"");
    }

    #[test]
    fn builtin_definitions_are_sound() {
        // 精选的（nodes/*.py，两级都有）和从 polars 生成的表达式节点
        let types: Vec<Arc<NodeType>> = registry().read().unwrap().iter().filter(|t| t.source == Source::Builtin).cloned().collect();
        assert_eq!(types.iter().filter(|t| t.order < 1000).count(), BUILTIN.len());
        // 按分类、order 排：IO 的 read 在最前面；表达式级的“列”在最前面
        let listed = all();
        assert_eq!((listed[0].name.as_str(), listed[1].name.as_str()), ("read", "write"));
        assert_eq!(all_expr()[0].name, "col");
        assert!(categories().starts_with(&CATEGORY_ORDER.map(String::from)));
        assert_eq!(expr_categories(), EXPR_CATEGORY_ORDER.map(String::from), "生成的表达式节点有不认识的分类");
        assert!(all_expr().len() > 400);
        for t in &types {
            let order: &[&str] = if t.expr { &EXPR_CATEGORY_ORDER } else { &CATEGORY_ORDER };
            assert!(!t.desc.is_empty() && order.contains(&t.category.as_str()), "{}", t.name);
            // 输出节点的角色得是上下文认得的；表格级的节点没有端口参数
            assert!(["", "列", "键", "条件", "排序", "结果"].contains(&t.role.as_str()), "{}", t.name);
            assert!(t.expr || t.params.iter().all(|p| !p.port), "{}", t.name);
            assert_eq!(t.inputs, if t.expr { t.params.iter().filter(|p| p.port).count() } else { t.inputs });
            // 代码里用到的参数都得有；定义了的参数代码里都得用到（按钮除外：它是 action）
            let code = if t.code_param.is_empty() { t.code.as_str() } else { "" };
            let used = referenced_params(code);
            for name in &used {
                assert!(t.param(name).is_some(), "{} 的代码用了没定义的参数 {name}", t.name);
            }
            for p in t.params.iter().filter(|p| p.kind != ParamKind::Button && t.code_param.is_empty()) {
                assert!(used.contains(&p.name), "{} 的参数 {} 没被代码用到", t.name, p.name);
            }
            for p in &t.params {
                assert!(p.kind != ParamKind::Menu || p.options.contains(&p.default), "{}.{}", t.name, p.name);
                for (other, values) in &p.show {
                    let target = &t.params[t.param(other).unwrap()];
                    let toggle = [String::from("True"), String::from("False")];
                    let options = if target.kind == ParamKind::Toggle { &toggle[..] } else { &target.options[..] };
                    assert!(values.iter().all(|v| options.contains(v)), "{}.{} 的显示条件里有不存在的选项", t.name, p.name);
                }
                assert!(p.paste.is_empty() || t.param(&p.paste).is_some(), "{}.{} 粘贴到不存在的参数", t.name, p.name);
            }
        }
    }

    #[test]
    fn definition_files_round_trip() {
        for (name, text) in BUILTIN {
            let t = NodeType::parse(name, text, Source::Builtin).unwrap();
            let again = NodeType::parse(name, &t.to_text(), Source::Builtin).unwrap();
            assert_eq!(t, again, "{name}");
        }
        // 手写的：不带 r 前缀、省掉大部分项
        let text = "\"\"\"\n{\"desc\": \"前 N 行\", \"params\": [{\"name\": \"n\", \"type\": \"int\", \"default\": \"5\"}]}\n\"\"\"\n\nout = df.head(ch(\"n\"))\n";
        let t = NodeType::parse("top", text, Source::User).unwrap();
        assert_eq!((t.category.as_str(), t.inputs, t.min_inputs), (DEFAULT_CATEGORY, 1, 1));
        assert_eq!(t.code, "out = df.head(ch(\"n\"))\n");
        assert_eq!((t.params[0].kind, t.params[0].label.as_str(), t.params[0].default.as_str()), (ParamKind::Int, "n", "5"));
        assert!(t.to_text().starts_with("r\"\"\"\n{\n  \"desc\": \"前 N 行\""));
    }

    #[test]
    fn broken_definitions_are_rejected() {
        let parse = |text: &str| NodeType::parse("x", text, Source::User).unwrap_err();
        assert!(parse("out = df").contains("文档字符串"));
        assert!(parse("\"\"\"{oops}\"\"\"").contains("JSON"));
        assert!(parse("\"\"\"{\"params\": [{\"name\": \"a\"}, {\"name\": \"a\"}]}\"\"\"").contains("都叫 a"));
        assert!(parse("\"\"\"{\"params\": [{\"name\": \"1a\"}]}\"\"\"").contains("参数名"));
        assert!(parse("\"\"\"{\"params\": [{\"name\": \"a\", \"type\": \"颜色\"}]}\"\"\"").contains("不认识的类型"));
        assert!(parse("\"\"\"{\"params\": [{\"name\": \"a\", \"type\": \"menu\"}]}\"\"\"").contains("选项"));
        assert!(parse("\"\"\"{\"params\": [{\"name\": \"a\", \"show\": {\"b\": [\"x\"]}}]}\"\"\"").contains("不存在的参数 b"));
        assert!(parse("\"\"\"{\"inputs\": 9}\"\"\"").contains("输入个数"));
        assert!(NodeType::parse("有 空格", "", Source::User).is_err());
    }

    #[test]
    fn layers_shadow_by_name() {
        // 上面一层盖住内置；拿掉后内置恢复
        let mut mine = (*builtin("to_dummies").unwrap()).clone();
        mine.source = Source::Project;
        mine.desc = String::from("我的 one-hot");
        install(mine);
        assert_eq!(find("to_dummies").unwrap().desc, "我的 one-hot");
        assert_ne!(builtin("to_dummies").unwrap().desc, "我的 one-hot");
        assert_eq!(all().iter().filter(|t| t.name == "to_dummies").count(), 1);
        remove("to_dummies", Source::Project);
        assert_eq!(find("to_dummies").unwrap().source, Source::Builtin);
    }

    #[test]
    fn search_finds_kinds_and_their_modes() {
        let names = |query: &str| -> Vec<String> {
            search(query)
                .iter()
                .map(|hit| match &hit.option {
                    Some(option) => format!("{} · {option}", hit.kind.name),
                    None => hit.kind.name.clone(),
                })
                .collect()
        };
        // graph 的测试会临时注册一个 graph_test_ 开头的类型：并行跑时别数进来
        let ours = |name: &str| !name.starts_with("graph_test_");
        assert_eq!(names("").iter().filter(|n| ours(n)).count(), all().iter().filter(|t| ours(&t.name)).count());
        assert_eq!(names("")[0], "read");
        assert_eq!(names("tail")[0], "slice · tail");
        assert_eq!(names("reverse")[0], "slice · reverse");
        assert_eq!(names("select")[0], "select"); // 方式和节点同名的不重复列
        assert_eq!(names("SELECT"), names("select"));
        assert!(names("join").starts_with(&["join".to_string(), "join · join_asof".to_string()]));
        assert!(names("统计").contains(&"describe".to_string()));
        assert!(names("drop").contains(&"select · drop".to_string()));
        assert!(names("表达式").contains(&"expr".to_string()));
        assert!(!names("excel").contains(&"read · Excel".to_string())); // 格式不进搜索
        assert!(names("没有这种东西").is_empty());
        // 表格级的搜不到表达式节点，反过来也一样
        assert!(!names("strip").iter().any(|n| n.contains("strip")));

        let exprs = |query: &str, prefer: &str| -> Vec<String> {
            search_expr(query, prefer).iter().map(|hit| hit.kind.name.clone()).collect()
        };
        assert_eq!(exprs("", "").len(), all_expr().len());
        // 短名字也算开头匹配：搜 strip 找到 str.strip_chars
        assert!(exprs("strip", "").contains(&"str.strip_chars".to_string()));
        assert!(exprs("条件", "").contains(&"when".to_string()));
        // 选中的节点输出文字：同一档里文本的排前面
        assert!(exprs("len", "文本 str")[0].starts_with("str."));
        // 名字里包含也算：偏好的分类排在别的分类的“开头匹配”前面
        assert_eq!(exprs("upper", "文本 str")[0], "str.to_uppercase");
        assert_eq!(exprs("upper", "")[0], "upper_bound");
        assert_eq!(exprs("len", "")[0], "len");
        assert!(!exprs("read", "").contains(&"read".to_string()));
    }

    #[test]
    fn source_labels() {
        let label = |name: &str, source: Source, canvas: bool| {
            let mut t = NodeType::new(name);
            t.source = source;
            source_label(&t, canvas)
        };
        assert_eq!(label("read", Source::Builtin, false), "");
        // 盖住了内置的同名节点：卡片上也标
        assert_eq!(label("read", Source::User, true), "已改内置");
        assert_eq!(label("read", Source::Project, true), "已改内置");
        // 自己的类型：只在参数面板标
        assert_eq!(label("my_sum", Source::User, true), "");
        assert_eq!(label("my_sum", Source::User, false), "用户");
        assert_eq!(label("my_sum", Source::Project, false), "工程");
        assert_eq!(label("code", Source::Instance, true), "私有参数");
    }

    #[test]
    fn referenced_params_are_found_in_code() {
        let code = "x = ch(\"a\") + ch('b_1')\nfetch(\"no\")\ny = ch( \"a\" )\nz = self.ch(\"no\")\nw = ch(name)";
        assert_eq!(referenced_params(code), ["a", "b_1"]);
        // 注释里的不算，字符串里的 # 不是注释
        let code = "# ch(\"名字\") 取参数\nx = ch(\"a\")  # ch(\"c\")\ny = ch('d') + \"#\" + ch(\"e\") + '\\'#' + ch(\"f\")";
        assert_eq!(referenced_params(code), ["a", "d", "e", "f"]);
    }
}
