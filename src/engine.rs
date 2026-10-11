//! 节点执行引擎的进程管理：常驻 Python + polars 子进程，stdin/stdout 逐行 JSON。
//! 优先用程序旁边的 `engine/python.exe`（随包分发），否则用 RTV_PYTHON 或系统里的 python。

use crate::graph::Step;
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

const SCRIPT: &str = include_str!("../engine/engine.py");

pub struct RunOk {
    pub rows: u64,
    pub ms: u64,
    pub stdout: String,
    /// 各节点输出的列名和类型（polars 的叫法，如 Int64、String）
    pub columns: Vec<(u32, Vec<String>)>,
    pub dtypes: Vec<(u32, Vec<String>)>,
    /// expr 节点里各表达式节点的输出类型 / 问题 / 文字
    pub subs: Vec<(u32, Vec<(u32, SubInfo)>)>,
    /// expr 节点整个写成文字（df.with_columns(…)）
    pub texts: Vec<(u32, String)>,
    /// explain 请求的查询计划：（优化后, 优化前）
    pub plan: (String, String),
    /// 预览的说明：filter 里预览真假值时有几行满足
    pub note: String,
    /// 这次是从哪些节点留在引擎里的结果接着算的（以前显示过、collect 过的；见 engine.py 的 kept）
    #[cfg_attr(not(test), allow(dead_code))]
    pub reused: Vec<u32>,
    /// 选列框的请求：{"matched": [列名]} 或 {"error": 原因}（见 engine.py 的 pick_columns）
    pub pick: Value,
}

/// 选列框向引擎要的：表的列名和类型（polars 的叫法），写的那些现在选到哪几列（或者为什么选不了）。
pub struct Picked {
    pub columns: Vec<(String, String)>,
    pub matched: Result<Vec<String>, String>,
    /// 字典参数：每一组（每段写法）各选到哪几列
    pub groups: Vec<Result<Vec<String>, String>>,
}

pub struct RunErr {
    pub node: Option<u32>,
    /// 出错的是 node 里面表达式网络的哪个节点：路径 [网络里的节点, 它里面网络的节点…]，空 = 不是网络里的
    pub sub: Vec<u32>,
    pub msg: String,
    pub stdout: String,
    pub columns: Vec<(u32, Vec<String>)>,
    pub dtypes: Vec<(u32, Vec<String>)>,
    pub subs: Vec<(u32, Vec<(u32, SubInfo)>)>,
    pub texts: Vec<(u32, String)>,
}

/// 表达式节点算出来的东西：输出的类型（多列时是第一列的）和列数；算不出来时是原因。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SubInfo {
    pub dtype: String,
    pub count: usize,
    pub error: String,
    /// 算得出来但值得提醒的（group_by 里没聚合成一个值）
    pub warn: String,
    /// 这个节点的表达式写成文字：pl.col("a").str.slice(0, 4)
    pub text: String,
    /// 节点里面还有网络（eval）：里面各节点的
    pub net: Vec<(u32, SubInfo)>,
}

impl RunErr {
    pub fn plain(msg: impl Into<String>) -> RunErr {
        RunErr {
            node: None,
            sub: Vec::new(),
            msg: msg.into(),
            stdout: String::new(),
            columns: Vec::new(),
            dtypes: Vec::new(),
            subs: Vec::new(),
            texts: Vec::new(),
        }
    }
}

type Pipes = (ChildStdin, BufReader<ChildStdout>);

/// 读写管道：一次只跑一个请求，run 全程持有。
static IO: Mutex<Option<Pipes>> = Mutex::new(None);
/// 子进程句柄单独放，stop 不用等正在跑的请求。
static CHILD: Mutex<Option<Child>> = Mutex::new(None);
/// 引擎是用户点“停止”停的：这时说“已停止”就够了，不用翻 stderr 的日志找原因。
static STOPPED_BY_USER: AtomicBool = AtomicBool::new(false);

/// 引擎的 stderr 写到工作目录里的这个文件：Python 崩溃、内存不够时的原因在这里。
const STDERR_LOG: &str = "engine-stderr.log";

/// 所有进程的临时目录都在这下面，一个进程一个（文件夹名是 pid）。
fn temp_root() -> PathBuf {
    std::env::temp_dir().join("nodesheet")
}

/// 本进程的临时目录（引擎脚本 + 结果文件）。
pub fn work_dir() -> PathBuf {
    temp_root().join(std::process::id().to_string())
}

pub fn cleanup() {
    stop();
    let _ = std::fs::remove_dir_all(work_dir());
}

/// 以前的进程留下的临时目录（被结束、崩溃了的不走 cleanup）：进程已经不在的整个删掉。在后台做。
pub fn sweep_stale() {
    std::thread::spawn(|| {
        let Ok(dirs) = std::fs::read_dir(temp_root()) else { return };
        let me = std::process::id();
        for entry in dirs.flatten() {
            let Some(pid) = entry.file_name().to_str().and_then(|n| n.parse::<u32>().ok()) else { continue };
            if pid != me && !process_alive(pid) {
                let _ = std::fs::remove_dir_all(entry.path());
            }
        }
    });
}

/// 这个 pid 的进程还在不在。打不开、又不是因为没有权限，就是不在了；拿不准的都当它还在（别删别人的东西）。
#[cfg(windows)]
fn process_alive(pid: u32) -> bool {
    use std::ffi::c_void;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut c_void;
        fn GetExitCodeProcess(process: *mut c_void, code: *mut u32) -> i32;
        fn CloseHandle(handle: *mut c_void) -> i32;
        fn GetLastError() -> u32;
    }
    const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
    const STILL_ACTIVE: u32 = 259;
    const ERROR_INVALID_PARAMETER: u32 = 87;
    // SAFETY: 句柄打开了就关掉
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return GetLastError() != ERROR_INVALID_PARAMETER;
        }
        let mut code = 0;
        let ok = GetExitCodeProcess(handle, &mut code) != 0;
        CloseHandle(handle);
        !ok || code == STILL_ACTIVE
    }
}

#[cfg(not(windows))]
fn process_alive(_pid: u32) -> bool {
    true
}

/// msg 后面接上引擎 stderr 日志的最后几行（有的话）：Python 崩溃、导入失败、内存不够时的原因。
fn with_log(msg: String) -> String {
    with_tail(msg, &work_dir().join(STDERR_LOG))
}

fn with_tail(msg: String, log: &Path) -> String {
    let Ok(bytes) = std::fs::read(log) else { return msg };
    let text = String::from_utf8_lossy(&bytes);
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    let tail = lines[lines.len().saturating_sub(15)..].join("\n");
    if tail.is_empty() {
        return msg;
    }
    let tail: String = tail.chars().rev().take(1500).collect::<Vec<_>>().into_iter().rev().collect();
    format!("{msg}。引擎最后的输出：\n{tail}")
}

/// 引擎进程没了（被停止或崩溃）时的报错：不是用户停的就带上它最后的输出。
fn stopped() -> String {
    let msg = String::from("引擎已停止");
    if STOPPED_BY_USER.swap(false, Ordering::Relaxed) { msg } else { with_log(msg) }
}

fn python_path() -> PathBuf {
    let bundled = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("engine").join("python.exe")));
    if let Some(p) = bundled.filter(|p| p.exists()) {
        return p;
    }
    std::env::var_os("RTV_PYTHON").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("python"))
}

fn spawn() -> Result<Pipes, String> {
    const HELP: &str = "把 engine 文件夹放在程序旁边，或安装 Python 后执行 pip install -U \"polars>=2.0.0\" fastexcel";
    let dir = work_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("无法创建临时目录: {e}"))?;
    let script = dir.join("engine.py");
    std::fs::write(&script, SCRIPT).map_err(|e| format!("无法写入引擎脚本: {e}"))?;

    // stderr 进日志文件（每次启动重写）：引擎崩溃时报错里带上它最后的输出
    let stderr = std::fs::File::create(dir.join(STDERR_LOG)).map_or_else(|_| Stdio::null(), Stdio::from);
    let mut cmd = Command::new(python_path());
    cmd.args(["-X", "utf8", "-u"])
        .arg(&script)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(stderr);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let mut child = cmd.spawn().map_err(|e| format!("找不到 Python（{e}）。{HELP}"))?;
    let stdin = child.stdin.take().ok_or("引擎管道不可用")?;
    let mut stdout = BufReader::new(child.stdout.take().ok_or("引擎管道不可用")?);
    *CHILD.lock().unwrap() = Some(child);

    let mut line = String::new();
    let hello = stdout.read_line(&mut line).ok().and_then(|_| serde_json::from_str::<Value>(&line).ok());
    let Some(hello) = hello else {
        // 还没打招呼就没了（Python 起不来、导入崩了）：等它退出，日志写完了再读
        stop_child();
        return Err(with_log(format!("引擎启动失败。{HELP}")));
    };
    if hello["ready"].as_bool() != Some(true) {
        let why = hello["error"].as_str().unwrap_or("");
        stop_child();
        return Err(format!("引擎里没有 polars（{why}）。{HELP}"));
    }
    // 节点（Map、meta 的行数分析、生成的表达式节点…）按 polars 2 写的
    let version = hello["polars"].as_str().unwrap_or("");
    if !polars_supported(version) {
        stop_child();
        return Err(format!("引擎里的 polars 是 {version}，要 2.0 以上。{HELP}"));
    }
    Ok((stdin, stdout))
}

/// polars 的版本号（2.0.0-rc.2、1.44.2）是不是 2 以上。
fn polars_supported(version: &str) -> bool {
    version.split('.').next().and_then(|major| major.parse::<u32>().ok()).is_some_and(|major| major >= 2)
}

fn stop_child() {
    if let Some(mut child) = CHILD.lock().unwrap().take() {
        let _ = child.kill();
        let _ = child.wait();
    }
}

/// 终止引擎进程（正在跑的请求随即以错误返回），下次 run 重新启动。
pub fn stop() {
    if CHILD.lock().unwrap().is_some() {
        STOPPED_BY_USER.store(true, Ordering::Relaxed);
    }
    stop_child();
}

/// 导出 Python 脚本用：引擎里执行节点代码的那几个函数（报错缩短、参数转换、make_env、run_code）。
pub fn script_prelude() -> &'static str {
    let start = SCRIPT.find("def short(e):").unwrap_or(0);
    let end = SCRIPT.find("def catalog(pl):").unwrap_or(SCRIPT.len());
    &SCRIPT[start..end.max(start)]
}

/// polars 的类型名 → 简称，照 polars 打印表格时的叫法：Int64 → i64、String → str、
/// List(Int64) → list[i64]、Array(Int64, shape=(2,)) → array[i64, 2]、Struct({'x': Int64}) → struct{x: i64}、
/// Map(String, Int64) → map[str, i64]、
/// Datetime(time_unit='us', time_zone=None) → datetime[μs]。
pub fn short_dtype(t: &str) -> String {
    let t = t.trim();
    let (base, args) = match t.find('(') {
        Some(i) if t.ends_with(')') => (&t[..i], &t[i + 1..t.len() - 1]),
        _ => (t, ""),
    };
    // 关键字参数：time_unit='us' → ("time_unit", "us")
    let kwarg = |name: &str| -> Option<String> {
        split_top(args).into_iter().find_map(|part| {
            let (k, v) = part.split_once('=')?;
            let v = v.trim().trim_matches(|c| c == '\'' || c == '"');
            (k.trim() == name && v != "None").then(|| v.to_string())
        })
    };
    let unit = |u: String| if u == "us" { String::from("μs") } else { u };
    match base {
        "Int8" | "Int16" | "Int32" | "Int64" | "Int128" => base.replacen("Int", "i", 1),
        "UInt8" | "UInt16" | "UInt32" | "UInt64" | "UInt128" => base.replacen("UInt", "u", 1),
        "Float16" | "Float32" | "Float64" => base.replacen("Float", "f", 1),
        "String" | "Utf8" => String::from("str"),
        "Boolean" => String::from("bool"),
        "Categorical" => String::from("cat"),
        // Extension('名字', Int64) → ext[名字]
        "Extension" | "BaseExtension" => {
            let name = split_top(args).into_iter().next().unwrap_or_default();
            format!("ext[{}]", name.trim().trim_matches(|c| c == '\'' || c == '"'))
        }
        "List" => format!("list[{}]", short_dtype(args)),
        "Map" => {
            let parts: Vec<String> = split_top(args).into_iter().map(short_dtype).collect();
            format!("map[{}]", parts.join(", "))
        }
        "Array" => {
            let parts = split_top(args);
            let inner = parts.first().map_or(String::new(), |p| short_dtype(p));
            let shape = kwarg("shape").unwrap_or_default();
            let shape = shape.trim_matches(|c| c == '(' || c == ')').trim_end_matches(',');
            format!("array[{inner}, {shape}]")
        }
        "Struct" => {
            let fields = split_top(args.trim().trim_start_matches('{').trim_end_matches('}'));
            let mut shown: Vec<String> = (fields.iter().take(STRUCT_FIELDS))
                .filter_map(|f| {
                    let (name, t) = split_field(f)?;
                    Some(format!("{name}: {}", short_dtype(t)))
                })
                .collect();
            if fields.len() > STRUCT_FIELDS {
                shown.push(format!("…共 {} 个", fields.len()));
            }
            format!("struct{{{}}}", shown.join(", "))
        }
        "Datetime" => {
            let mut parts: Vec<String> = kwarg("time_unit").map(unit).into_iter().collect();
            parts.extend(kwarg("time_zone"));
            if parts.is_empty() { String::from("datetime") } else { format!("datetime[{}]", parts.join(", ")) }
        }
        "Duration" => kwarg("time_unit").map(unit).map_or(String::from("duration"), |u| format!("duration[{u}]")),
        "Decimal" => match (kwarg("precision"), kwarg("scale")) {
            (Some(p), Some(s)) => format!("decimal[{p},{s}]"),
            (None, Some(s)) => format!("decimal[*,{s}]"),
            _ => String::from("decimal"),
        },
        other => other.to_lowercase(),
    }
}

/// struct 的类型里最多列出几个字段
const STRUCT_FIELDS: usize = 4;

/// 按顶层的逗号切开（括号、引号里的不算）。
fn split_top(s: &str) -> Vec<&str> {
    let (mut parts, mut depth, mut quote, mut start) = (Vec::new(), 0i32, None, 0);
    for (i, c) in s.char_indices() {
        match (quote, c) {
            (Some(q), _) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '\'' | '"') => quote = Some(c),
            (None, '(' | '[' | '{') => depth += 1,
            (None, ')' | ']' | '}') => depth -= 1,
            (None, ',') if depth == 0 => {
                parts.push(s[start..i].trim());
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push(s[start..].trim());
    parts.retain(|p| !p.is_empty());
    parts
}

/// struct 的一个字段 'x': Int64 → ("x", "Int64")（字段名里可能有冒号，所以先找引号）。
fn split_field(f: &str) -> Option<(&str, &str)> {
    let q = f.chars().next().filter(|c| *c == '\'' || *c == '"')?;
    let end = f[1..].find(q)? + 1;
    let rest = f[end + 1..].trim_start().strip_prefix(':')?;
    Some((&f[1..end], rest.trim()))
}

fn columns_of(resp: &Value) -> Vec<(u32, Vec<String>)> {
    names_by_node(resp, "columns")
}

fn subs_of(resp: &Value) -> Vec<(u32, Vec<(u32, SubInfo)>)> {
    let Some(map) = resp["subs"].as_object() else { return Vec::new() };
    map.iter().filter_map(|(node, items)| Some((node.parse().ok()?, sub_infos(items)))).collect()
}

/// 一张网络里各节点的信息：{id: {"dtype", "n", "error", "warn", "text", "net"}}，net 是节点里面网络的（同样的格式）。
fn sub_infos(items: &Value) -> Vec<(u32, SubInfo)> {
    let text = |v: &Value, key: &str| v[key].as_str().unwrap_or("").to_string();
    (items.as_object().into_iter().flatten())
        .filter_map(|(sub, v)| {
            let info = SubInfo {
                dtype: text(v, "dtype"),
                count: v["n"].as_u64().unwrap_or(0) as usize,
                error: text(v, "error"),
                warn: text(v, "warn"),
                text: text(v, "text"),
                net: sub_infos(&v["net"]),
            };
            Some((sub.parse().ok()?, info))
        })
        .collect()
}

fn texts_of(resp: &Value) -> Vec<(u32, String)> {
    let items = resp["texts"].as_object().into_iter().flatten();
    items.filter_map(|(id, text)| Some((id.parse().ok()?, text.as_str()?.to_string()))).collect()
}

/// 出错的节点在网络里的路径：新的引擎报数组，旧的是一个数。
fn sub_path(v: &Value) -> Vec<u32> {
    match v {
        Value::Array(items) => items.iter().filter_map(|i| i.as_u64().map(|n| n as u32)).collect(),
        _ => v.as_u64().map(|n| vec![n as u32]).unwrap_or_default(),
    }
}

fn names_by_node(resp: &Value, key: &str) -> Vec<(u32, Vec<String>)> {
    let Some(map) = resp[key].as_object() else { return Vec::new() };
    map.iter()
        .filter_map(|(id, names)| {
            let names = names.as_array()?.iter().filter_map(|n| n.as_str().map(String::from)).collect();
            Some((id.parse().ok()?, names))
        })
        .collect()
}

/// 管道能用：引擎还没起来、或者被停止过（子进程没了，管道还留着：没在跑的时候点了“停止”）时重新启动。
fn ensure_running(io: &mut Option<Pipes>) -> Result<&mut Pipes, String> {
    if io.is_some() && CHILD.lock().unwrap().is_none() {
        *io = None;
        STOPPED_BY_USER.store(false, Ordering::Relaxed);
    }
    if io.is_none() {
        *io = Some(spawn()?);
    }
    Ok(io.as_mut().expect("刚启动"))
}

/// 发一个不跑节点图的请求（拆文字、要 API 清单、要文档），返回回复。阻塞，应在后台线程调用。
fn ask(req: Value) -> Result<Value, String> {
    let mut io = IO.lock().unwrap();
    let (stdin, stdout) = ensure_running(&mut io)?;
    let mut line = String::new();
    let sent = writeln!(stdin, "{req}").and_then(|_| stdin.flush());
    match sent.and_then(|_| stdout.read_line(&mut line)) {
        Ok(n) if n > 0 => {}
        _ => {
            *io = None;
            stop_child();
            return Err(stopped());
        }
    }
    serde_json::from_str(&line).map_err(|e| e.to_string())
}

/// 文字 → 网络：把一段 polars 表达式拆成表达式节点（引擎用 Python 的 ast 拆，格式见 engine.py 的 parse_text）。
/// types：表达式级的节点类型和它们的参数（生成的节点按签名对参数）。阻塞。
pub fn parse(text: &str, types: Value) -> Result<Value, String> {
    let mut resp = ask(json!({"parse": text, "types": types}))?;
    match resp["error"].as_str() {
        Some(e) => Err(e.to_string()),
        None => Ok(resp["parsed"].take()),
    }
}

/// 代码补全用的 polars API 清单（{组: [[名字, 签名, 文档首行]]}）。阻塞，应在后台线程调用。
pub fn catalog() -> Option<Value> {
    Some(ask(json!({"catalog": true})).ok()?["catalog"].take())
}

/// 帮助窗口用的 polars 文档：{API: {sig, summary, params, sections} 或 {error}}（格式见 engine.py 的 api_doc）。阻塞。
pub fn doc(apis: &[String]) -> Result<Value, String> {
    let mut resp = ask(json!({"doc": apis}))?;
    match resp["error"].as_str() {
        Some(e) => Err(e.to_string()),
        None => Ok(resp["doc"].take()),
    }
}

/// 执行 steps，把 display 节点的结果写到 out（parquet）。阻塞，应在后台线程调用。
/// preview：只看表达式网络里选中的节点——display（expr 节点）里那个节点（路径：网络里的节点、eval 里面的节点…）
/// 的值连同它用到的列，label 是值那一列的名字。
/// 等到引擎空出来时 wanted() 已经为假（这期间又发了新的运行）就不跑了，返回 None：
/// 连续改了几次参数时，排在中间的那几次不用一个个算完。
pub fn run_if_wanted(
    steps: &[Step],
    display: u32,
    preview: Option<(&[u32], &str)>,
    out: &Path,
    wanted: &dyn Fn() -> bool,
) -> Option<Result<RunOk, RunErr>> {
    let mut req = json!({"out": out.to_string_lossy()});
    if let Some((sub, label)) = preview {
        req["preview"] = json!({"sub": sub, "label": label});
    }
    request_if(steps, display, req, wanted)
}

#[cfg(test)]
pub fn run(steps: &[Step], display: u32, out: &Path) -> Result<RunOk, RunErr> {
    run_if_wanted(steps, display, None, out, &|| true).unwrap()
}

#[cfg(test)]
pub fn preview(steps: &[Step], display: u32, sub: &[u32], label: &str, out: &Path) -> Result<RunOk, RunErr> {
    run_if_wanted(steps, display, Some((sub, label)), out, &|| true).unwrap()
}

/// 在后台先把引擎启动好（Python 启动 + 导入 polars 要大半秒），第一次运行节点时不用等。
/// 已经在跑、或者启动不了（没有 Python）都不管：真要运行时会再试并报错。
pub fn warm_up() {
    std::thread::spawn(|| {
        let mut io = IO.lock().unwrap();
        if io.is_none() {
            *io = spawn().ok();
        }
    });
}

/// 只推出 display 的列名和类型，不写结果（ok.columns / ok.dtypes 里有）。阻塞。
pub fn schema(steps: &[Step], display: u32) -> Result<RunOk, RunErr> {
    request(steps, display, json!({"schema": true}))
}

/// display 的查询计划（ok.plan：优化后、优化前），不执行查询。tree：画成树（像 show_graph），否则是缩进的文字。阻塞。
pub fn explain(steps: &[Step], display: u32, tree: bool) -> Result<RunOk, RunErr> {
    request(steps, display, json!({"explain": if tree { "tree" } else { "plain" }}))
}

/// 选列框：source 的列名、类型，以及 text（参数的写法：列名、选择器、表达式，逗号分隔）在它上面选到哪几列。
/// 只推表结构，不执行查询。等到引擎空出来时 wanted() 为假（又点了别的）就不问了。阻塞。
pub fn pick(steps: &[Step], source: u32, text: &str, wanted: &dyn Fn() -> bool) -> Option<Result<Picked, String>> {
    let picked = pick_many(steps, source, &[text.to_string()], wanted)?;
    Some(picked.map(|mut ok| {
        ok.matched = ok.groups.pop().unwrap_or(Ok(Vec::new()));
        ok
    }))
}

/// 同上，几段一起问（字典参数的一组一个键）；回复里每段各是选到哪几列或出错。阻塞。
pub fn pick_many(steps: &[Step], source: u32, texts: &[String], wanted: &dyn Fn() -> bool) -> Option<Result<Picked, String>> {
    let reply = request_if(steps, source, json!({"pick": texts}), wanted)?;
    Some(reply.map_err(|e| e.msg).map(|ok| {
        let of = |list: &[(u32, Vec<String>)]| list.iter().find(|(id, _)| *id == source).map(|(_, v)| v.clone()).unwrap_or_default();
        let (names, types) = (of(&ok.columns), of(&ok.dtypes));
        let columns = names.into_iter().enumerate().map(|(i, n)| (n, types.get(i).cloned().unwrap_or_default())).collect();
        let groups = ok.pick["matched"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|r| match r.as_str() {
                Some(e) => Err(e.to_string()),
                None => Ok(strings_of(r)),
            })
            .collect();
        Picked { columns, matched: Ok(Vec::new()), groups }
    }))
}

fn strings_of(v: &Value) -> Vec<String> {
    v.as_array().into_iter().flatten().filter_map(|s| s.as_str().map(String::from)).collect()
}

/// 节点面板里的按钮：执行 steps，node 带着 action 再执行一次（write 节点写文件）。阻塞，应在后台线程调用。
pub fn action(steps: &[Step], node: u32, action: &str) -> Result<RunOk, RunErr> {
    request(steps, node, json!({"action": action}))
}

fn step_json(step: &Step) -> Value {
    let params: serde_json::Map<String, Value> =
        step.params.iter().map(|p| (p.name.clone(), json!([p.kind, p.value, p.label, p.default]))).collect();
    let mut v = json!({
        "id": step.id,
        "code": step.code,
        "params": params,
        "inputs": step.inputs,
        "stamp": step.stamp.to_string(),
    });
    if !step.net.is_empty() {
        v["net"] = step.net.iter().map(step_json).collect();
    }
    if step.each {
        v["each"] = json!(true);
    }
    if !step.ports.is_empty() {
        v["ports"] = step.ports.iter().map(|(name, up)| (name.clone(), json!(up))).collect::<serde_json::Map<_, _>>().into();
    }
    if !step.role.is_empty() {
        v["role"] = json!(step.role);
        v["order"] = json!(step.order);
    }
    if !step.error.is_empty() {
        v["error"] = json!(step.error);
    }
    v
}

fn request(steps: &[Step], display: u32, req: Value) -> Result<RunOk, RunErr> {
    request_if(steps, display, req, &|| true).expect("wanted 总是真")
}

fn request_if(steps: &[Step], display: u32, req: Value, wanted: &dyn Fn() -> bool) -> Option<Result<RunOk, RunErr>> {
    let mut io = IO.lock().unwrap();
    if !wanted() {
        return None;
    }
    Some(request_locked(&mut io, steps, display, req))
}

fn request_locked(io: &mut Option<Pipes>, steps: &[Step], display: u32, mut req: Value) -> Result<RunOk, RunErr> {
    let (stdin, stdout) = ensure_running(io).map_err(RunErr::plain)?;

    req["steps"] = steps.iter().map(step_json).collect();
    req["display"] = json!(display);
    let mut line = String::new();
    let sent = writeln!(stdin, "{req}").and_then(|_| stdin.flush());
    let got = sent.and_then(|_| stdout.read_line(&mut line));
    let resp: Option<Value> = match got {
        Ok(n) if n > 0 => serde_json::from_str(&line).ok(),
        _ => None,
    };
    let Some(resp) = resp else {
        // 进程没了（被停止或崩溃）：丢掉管道，下次重启
        *io = None;
        stop_child();
        return Err(RunErr::plain(stopped()));
    };

    let stdout = resp["stdout"].as_str().unwrap_or("").to_string();
    let columns = columns_of(&resp);
    let dtypes = names_by_node(&resp, "dtypes");
    let subs = subs_of(&resp);
    let texts = texts_of(&resp);
    if resp["ok"].as_bool() == Some(true) {
        Ok(RunOk {
            rows: resp["rows"].as_u64().unwrap_or(0),
            ms: resp["ms"].as_u64().unwrap_or(0),
            stdout,
            columns,
            dtypes,
            subs,
            texts,
            plan: (
                resp["plan"]["optimized"].as_str().unwrap_or("").to_string(),
                resp["plan"]["raw"].as_str().unwrap_or("").to_string(),
            ),
            note: resp["note"].as_str().unwrap_or("").to_string(),
            reused: (resp["reused"].as_array().into_iter().flatten()).filter_map(|v| v.as_u64().map(|n| n as u32)).collect(),
            pick: resp["pick"].clone(),
        })
    } else {
        Err(RunErr {
            node: resp["node"].as_u64().map(|n| n as u32),
            sub: sub_path(&resp["sub"]),
            msg: resp["error"].as_str().unwrap_or("未知错误").to_string(),
            stdout,
            columns,
            dtypes,
            subs,
            texts,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::tests::{env_net, expr_kind, kind, net_of};
    use crate::graph::{Env, Graph};
    use crate::loader;
    use crate::nodes::{self, ParamKind};

    /// 在 expr 节点 e 的 env 那张网络里加一个表达式节点（不自动接线）并设好参数。
    fn add_in(g: &mut Graph, e: u32, env: Env, name: &str, set: &[(&str, &str)]) -> u32 {
        let net = env_net(g, e, env);
        net.selected = None;
        let id = net.add(&expr_kind(name));
        for (param, value) in set {
            assert!(net.node_mut(id).unwrap().set(param, value), "{name} 没有参数 {param}");
        }
        net.selected = None;
        id
    }

    /// 加在主网络里（group_by 时是聚合那张）。
    fn add_expr(g: &mut Graph, e: u32, name: &str, set: &[(&str, &str)]) -> u32 {
        add_in(g, e, Env::Main, name, set)
    }

    /// expr 节点 e 里装着 id 的那张网络。
    fn net_with(g: &mut Graph, e: u32, id: u32) -> &mut Graph {
        let node = g.node_mut(e).unwrap();
        node.nets_mut().map(|(_, net)| net).find(|net| net.node(id).is_some()).expect("网络里没有这个节点")
    }

    fn set_expr(g: &mut Graph, e: u32, id: u32, set: &[(&str, &str)]) {
        let node = net_with(g, e, id).node_mut(id).unwrap();
        for (param, value) in set {
            assert!(node.set(param, value), "{} 没有参数 {param}", node.kind.name);
        }
    }

    /// 排队等引擎时又发了新的运行：轮到它时不再执行，也不写结果（不需要 Python 也能测）。
    #[test]
    fn stale_runs_are_skipped() {
        let csv = std::env::current_dir().unwrap().join("testdata").join("people.csv");
        let g = Graph::for_file(&csv, "");
        let id = g.primary().unwrap().id;
        let out = work_dir().join("stale.parquet");
        let _ = std::fs::remove_file(&out);
        assert!(run_if_wanted(&g.compile(id).unwrap(), id, None, &out, &|| false).is_none());
        assert!(!out.exists());
    }

    /// 清理临时目录时认得出哪些进程还在；引擎崩溃时报错里带上 stderr 日志的最后几行。
    #[test]
    fn stale_dirs_and_crash_log() {
        assert!(process_alive(std::process::id()));
        assert!(!process_alive(u32::MAX - 3), "不存在的 pid");
        let log = std::env::temp_dir().join(format!("rtv-log-{}.txt", std::process::id()));
        let lines: Vec<String> = (1..=20).map(|i| format!("第 {i} 行")).collect();
        std::fs::write(&log, lines.join("\n") + "\n\nMemoryError\n").unwrap();
        let msg = with_tail(String::from("引擎已停止"), &log);
        assert!(msg.starts_with("引擎已停止。引擎最后的输出：\n第 7 行") && msg.ends_with("MemoryError"), "{msg}");
        std::fs::write(&log, "  \n").unwrap();
        assert_eq!(with_tail(String::from("引擎已停止"), &log), "引擎已停止");
        let _ = std::fs::remove_file(&log);
        assert_eq!(with_tail(String::from("x"), &log), "x");
    }

    #[test]
    fn short_dtype_shows_nested_types() {
        assert_eq!(short_dtype("Int64"), "i64");
        assert_eq!(short_dtype("UInt32"), "u32");
        assert_eq!(short_dtype("String"), "str");
        assert_eq!(short_dtype("List(Int64)"), "list[i64]");
        assert_eq!(short_dtype("List(List(Float64))"), "list[list[f64]]");
        assert_eq!(short_dtype("Map(String, Int64)"), "map[str, i64]");
        assert!(polars_supported("2.0.0-rc.2") && polars_supported("2.1.0") && polars_supported("10.0.0"));
        assert!(!polars_supported("1.44.2") && !polars_supported(""));
        assert_eq!(short_dtype("Map(String, List(Map(Int32, String)))"), "map[str, list[map[i32, str]]]");
        assert_eq!(short_dtype("Array(Int64, shape=(2,))"), "array[i64, 2]");
        assert_eq!(short_dtype("Array(Int64, shape=(2, 3))"), "array[i64, 2, 3]");
        assert_eq!(short_dtype("Struct({'x': Int64, 'y': String})"), "struct{x: i64, y: str}");
        assert_eq!(short_dtype("List(Struct({'a: b': List(Int8)}))"), "list[struct{a: b: list[i8]}]");
        assert_eq!(
            short_dtype("Struct({'a': Int8, 'b': Int8, 'c': Int8, 'd': Int8, 'e': Int8})"),
            "struct{a: i8, b: i8, c: i8, d: i8, …共 5 个}"
        );
        assert_eq!(short_dtype("Datetime(time_unit='us', time_zone=None)"), "datetime[μs]");
        assert_eq!(short_dtype("Datetime(time_unit='ns', time_zone='Asia/Shanghai')"), "datetime[ns, Asia/Shanghai]");
        assert_eq!(short_dtype("Duration(time_unit='ms')"), "duration[ms]");
        assert_eq!(short_dtype("Decimal(precision=10, scale=2)"), "decimal[10,2]");
        assert_eq!(short_dtype("Categorical"), "cat");
        assert_eq!(short_dtype("Enum(categories=['a'])"), "enum");
        assert_eq!(short_dtype("Date"), "date");
        assert_eq!(short_dtype("Float16"), "f16");
        assert_eq!(short_dtype("Int128"), "i128");
        assert_eq!(short_dtype("UInt128"), "u128");
        assert_eq!(short_dtype("Extension('名字', Int64)"), "ext[名字]");
        assert_eq!(short_dtype("Object"), "object");
        assert_eq!(short_dtype("Null"), "null");
        assert_eq!(short_dtype("Binary"), "binary");
    }

    fn wire(g: &mut Graph, e: u32, from: u32, to: u32, slot: usize) {
        assert!(net_with(g, e, to).connect(from, to, slot), "接不上 {from} → {to}");
    }

    /// 网络里第一个这种类型的节点（主网络先找，再找分组键、筛选组那两张）。
    fn first(g: &mut Graph, e: u32, name: &str) -> u32 {
        let node = g.node(e).unwrap();
        let found = node.nets().flat_map(|(_, net)| &net.nodes).find(|n| n.kind.name == name);
        found.unwrap_or_else(|| panic!("网络里没有 {name}")).id
    }

    /// env 那张网络里第一个这种类型的节点。
    fn first_in(g: &mut Graph, e: u32, env: Env, name: &str) -> u32 {
        let net = env_net(g, e, env);
        net.nodes.iter().find(|n| n.kind.name == name).unwrap_or_else(|| panic!("{env:?} 网络里没有 {name}")).id
    }

    /// expr 节点：上下文设好，缺的输出节点补上。
    fn expr_node(g: &mut Graph, context: &str) -> u32 {
        let e = g.add(&kind("expr"));
        g.node_mut(e).unwrap().set("context", context);
        g.node_mut(e).unwrap().ensure_outputs();
        e
    }

    /// 按年龄筛（> min），再按城市数人数：两个 expr 节点。
    fn filter_then_count(g: &mut Graph, min: &str) -> (u32, u32) {
        let filter = expr_node(g, "filter");
        let age = first(g, filter, "col");
        set_expr(g, filter, age, &[("columns", "\"年龄\"")]);
        let cmp = add_expr(g, filter, "compare", &[("op", ">"), ("b", min)]);
        wire(g, filter, age, cmp, 0);
        let cond = first(g, filter, "condition");
        wire(g, filter, cmp, cond, 0);

        // group_by：分组键那张网络里是 col → 分组键；聚合那张里数每组几行
        let group = expr_node(g, "group_by");
        let key_col = first_in(g, group, Env::Keys, "col");
        set_expr(g, group, key_col, &[("columns", "\"城市\"")]);
        let city = first_in(g, group, Env::Main, "col");
        set_expr(g, group, city, &[("columns", "\"城市\"")]);
        let count = add_expr(g, group, "agg", &[("function", "len")]);
        wire(g, group, city, count, 0);
        let output = first(g, group, "output");
        set_expr(g, group, output, &[("name", "次数")]);
        wire(g, group, count, output, 0);
        (filter, group)
    }

    /// 导出的 Python 脚本能独立运行，结果和引擎一样；各节点的列类型也报回来了。本机没有 Python + polars 时跳过。
    #[test]
    fn exported_script_runs() {
        let csv = std::env::current_dir().unwrap().join("testdata").join("people.csv");
        let mut g = Graph::for_file(&csv, "");
        // 条件和 graph_runs_through_engine 不同：两个测试共用一个引擎，同样的图会命中缓存、少报上游的列
        let (_, group) = filter_then_count(&mut g, "25");

        let out = work_dir().join("script-test.parquet");
        let ok = match run(&g.compile(group).unwrap(), group, &out) {
            Ok(ok) => ok,
            Err(e) if e.node.is_none() => {
                eprintln!("跳过：{}", e.msg);
                return;
            }
            Err(e) => panic!("{}", e.msg),
        };
        let types = &ok.dtypes.iter().find(|(id, _)| *id == group).unwrap().1;
        assert_eq!(types.len(), 2);
        assert_eq!(short_dtype(&types[1]), "u32");

        let script = g.python_script(group, script_prelude()).unwrap();
        let path = work_dir().join("script-test.py");
        std::fs::write(&path, &script).unwrap();
        let output = Command::new(python_path()).arg(&path).env("PYTHONIOENCODING", "utf-8").output().unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(output.status.success(), "{}\n{}", String::from_utf8_lossy(&output.stderr), script);
        assert!(stdout.contains("城市") && stdout.contains(&format!("shape: ({}, 2)", ok.rows)), "{stdout}");
    }

    /// 帮助里的 @polars 都指得到这版 polars 里的 API（生成的节点：名字对得上 Expr / pl 上的方法），文档拆得开。
    fn polars_docs_resolve() {
        let types = nodes::all().into_iter().chain(nodes::all_expr()).filter(|t| t.source == nodes::Source::Builtin);
        let mut apis: Vec<String> = types.flat_map(|t| crate::help::parse(&t.help).polars).map(|(_, api)| api).collect();
        apis.sort();
        apis.dedup();
        assert!(apis.len() > 400);
        let docs = doc(&apis).unwrap();
        for api in &apis {
            assert!(docs[api.as_str()]["error"].is_null(), "{api}：{}", docs[api.as_str()]["error"]);
        }
        let unique = &docs["DataFrame.unique"];
        assert!(unique["sig"].as_str().unwrap().starts_with("unique("));
        assert!(unique["params"].as_array().unwrap().iter().any(|p| p[0] == "keep"));
        assert!(unique["sections"].as_array().unwrap().iter().any(|s| s[0] == "Examples"));
        assert!(docs["DataFrame.nope"].is_null());
    }

    /// 整条链路：图 → 步骤 → 引擎 → parquet → 查看器数据源。本机没有 Python + polars 时跳过。
    #[test]
    fn graph_runs_through_engine() {
        let csv = std::env::current_dir().unwrap().join("testdata").join("people.csv");
        let mut g = Graph::for_file(&csv, "");
        let (filter, group) = filter_then_count(&mut g, "30");

        let out = work_dir().join("test.parquet");
        let ok = match run(&g.compile(group).unwrap(), group, &out) {
            Ok(ok) => ok,
            Err(e) if e.node.is_none() => {
                eprintln!("跳过：{}", e.msg);
                return;
            }
            Err(e) => panic!("{}", e.msg),
        };
        let sheets = loader::open_tables(&out).unwrap();
        let src = &sheets[0].source;
        assert_eq!(src.headers(), ["城市", "次数"]);
        assert_eq!(src.row_count() as u64, ok.rows);
        assert!(src.window(0, 1)[0][1].parse::<u64>().unwrap() > 0);
        assert!(ok.columns.iter().any(|(id, names)| *id == filter && names.len() == 4));
        // 网络里各节点的输出类型：在每组里算的 len 是 u32，没聚合的列在 agg 里是列表
        let info = |subs: &[(u32, Vec<(u32, SubInfo)>)], e: u32, sub: u32| -> SubInfo {
            let items = &subs.iter().find(|(id, _)| *id == e).expect("没报这个 expr 节点").1;
            items.iter().find(|(id, _)| *id == sub).expect("没报这个节点").1.clone()
        };
        let count = first(&mut g, group, "agg");
        assert_eq!(info(&ok.subs, group, count).dtype, "UInt32");
        // 聚合那张网络里的中间节点：标组内的类型（这一组的那几行），不是收成列表以后的
        let city = first_in(&mut g, group, Env::Main, "col");
        assert_eq!(info(&ok.subs, group, city).dtype, "String");
        let key = first(&mut g, group, "key");
        assert_eq!(info(&ok.subs, group, key).dtype, "String");
        assert!(key >= Env::Keys.base(), "分组键那张网络里的 id 和聚合那张的不重");
        let cmp = first(&mut g, filter, "compare");
        assert_eq!(info(&ok.subs, filter, cmp).dtype, "Boolean");
        // filter 里用不上的“输出列”（建节点时默认的那个）也推出了类型
        let unused = first(&mut g, filter, "output");
        assert_eq!(info(&ok.subs, filter, unused).dtype, "Int64");
        // group_by 里没聚合的输出列：算得出来（每组一个列表），但有提醒；聚合了的没有
        let raw = add_expr(&mut g, group, "output", &[("value", "pl.col(\"年龄\")")]);
        let more = run(&g.compile(group).unwrap(), group, &out).ok().unwrap();
        assert_eq!(info(&more.subs, group, raw).dtype, "List(Int64)");
        assert!(info(&more.subs, group, raw).warn.contains("没有聚合"));
        let counted = first(&mut g, group, "output");
        assert_eq!(info(&more.subs, group, counted).warn, "");
        net_of(&mut g, group).remove(raw);

        // 只要类型（查看器自己显示源文件时，表头的类型）：源节点按 polars 读出来的类型，不写结果
        let source = g.primary().unwrap().id;
        let _ = std::fs::remove_file(&out);
        let ok = schema(&g.compile(source).unwrap(), source).ok().unwrap();
        let types = &ok.dtypes.iter().find(|(id, _)| *id == source).unwrap().1;
        assert_eq!(types.iter().map(|t| short_dtype(t)).collect::<Vec<_>>(), ["str", "str", "i64", "f64"]);
        assert!(!out.exists());

        // 预览网络里的节点：它的值连同它用到的列；group_by 里聚合的节点按组显示
        let steps = g.compile(group).unwrap();
        let ok = preview(&steps, group, &[count], "agg1", &out).ok().unwrap();
        assert_eq!(loader::open_tables(&out).unwrap()[0].source.headers(), ["城市", "→ agg1"]);
        assert_eq!(ok.rows, src.row_count() as u64);
        // 分组键那张网络里的节点在整张表上算：和表一样多行
        let key_col = first_in(&mut g, group, Env::Keys, "col");
        let ok = preview(&steps, group, &[key_col], "col1", &out).ok().unwrap();
        assert_eq!(loader::open_tables(&out).unwrap()[0].source.headers(), ["城市", "→ col1"]);
        assert!(ok.rows > src.row_count() as u64, "整表算的分组键：{} 行", ok.rows);
        let steps = g.compile(filter).unwrap();
        preview(&steps, filter, &[cmp], "比较", &out).ok().unwrap();
        assert_eq!(loader::open_tables(&out).unwrap()[0].source.headers(), ["年龄", "→ 比较"]);
        // filter 里预览真假值：路径栏写满足几行；选中“条件”输出时表是只按它筛过的
        let ok = preview(&steps, filter, &[cmp], "比较", &out).ok().unwrap();
        assert!(ok.note.starts_with("满足 ") && ok.note.contains("（共 50）"), "{}", ok.note);
        let cond = first(&mut g, filter, "condition");
        let ok = preview(&steps, filter, &[cond], "条件", &out).ok().unwrap();
        assert_eq!(loader::open_tables(&out).unwrap()[0].source.headers(), ["姓名", "城市", "年龄", "收入"]);
        assert!(ok.note.starts_with(&format!("满足 {} 行", ok.rows)), "{} / {} 行", ok.note, ok.rows);
        // 值的行数和表不一样（unique / value_counts…）：放不到用到的列旁边，只显示值
        let uniq = add_expr(&mut g, filter, "snippet", &[("expression", "pl.col(\"城市\").unique()")]);
        let steps = g.compile(filter).unwrap();
        preview(&steps, filter, &[uniq], "去重", &out).ok().unwrap();
        assert_eq!(loader::open_tables(&out).unwrap()[0].source.headers(), ["→ 去重"]);
        net_of(&mut g, filter).remove(uniq);
        // 只看表达式、不看数据：碰巧没有空值的 drop_nulls 也只显示值
        let dropped = add_expr(&mut g, filter, "snippet", &[("expression", "pl.col(\"城市\").drop_nulls()")]);
        preview(&g.compile(filter).unwrap(), filter, &[dropped], "去空", &out).ok().unwrap();
        assert_eq!(loader::open_tables(&out).unwrap()[0].source.headers(), ["→ 去空"]);
        net_of(&mut g, filter).remove(dropped);
        // 行数和表不一样的值当 filter 的条件：类型推得出来，但有提醒（不用等运行时报错）
        let odd = add_expr(&mut g, filter, "condition", &[("value", "pl.col(\"年龄\").unique() > 1")]);
        let subs = match run(&g.compile(filter).unwrap(), filter, &out) {
            Ok(ok) => ok.subs,
            Err(e) => e.subs,
        };
        assert!(info(&subs, filter, odd).warn.contains("不能当 filter 的条件"), "{:?}", info(&subs, filter, odd));
        assert_eq!(info(&subs, filter, first(&mut g, filter, "condition")).warn, "");
        net_of(&mut g, filter).remove(odd);

        // 出错的节点能被定位：polars 报的错算 expr 节点的；参数填得不对 / 必填的没填，说得出是网络里的哪个节点
        let age = first(&mut g, filter, "col");
        set_expr(&mut g, filter, age, &[("columns", "\"不存在\"")]);
        let err = run(&g.compile(group).unwrap(), group, &out).err().unwrap();
        assert_eq!(err.node, Some(filter));
        assert!(!info(&err.subs, filter, age).error.is_empty(), "列不存在：col 节点的类型推不出来");
        set_expr(&mut g, filter, age, &[("columns", "\"年龄\"")]);
        set_expr(&mut g, filter, cmp, &[("b", "pl.col(")]);
        let err = run(&g.compile(group).unwrap(), group, &out).err().unwrap();
        assert_eq!((err.node, err.sub), (Some(filter), vec![cmp]), "{}", err.msg);
        assert!(err.msg.contains("参数“b”"), "{}", err.msg);
        set_expr(&mut g, filter, cmp, &[("b", "")]);
        let err = run(&g.compile(group).unwrap(), group, &out).err().unwrap();
        assert_eq!((err.node, err.sub), (Some(filter), vec![cmp]), "{}", err.msg);
        assert!(err.msg.contains("“b”"), "{}", err.msg);
        set_expr(&mut g, filter, cmp, &[("b", "30")]);

        // eval 节点（表达式里再套表达式）、文字 ↔ 网络
        eval_inside(&out);
        polars_docs_resolve();
        pick_through_engine();

        // code 节点：print 被收集，out 作为结果
        let code = g.add(&kind("code"));
        g.node_mut(code).unwrap().inputs[0] = Some(1);
        g.node_mut(code).unwrap().set("code", "print(\"hi\")\nout = df.head(3)");
        let ok = run(&g.compile(code).unwrap(), code, &out).ok().unwrap();
        assert_eq!((ok.rows, ok.stdout.trim()), (3, "hi"));

        // write 节点：在图里是直通，点“写出”才写文件，而且每次都写（不走缓存）
        let write = g.add(&kind("write"));
        let target = work_dir().join("写出.csv");
        g.node_mut(write).unwrap().set("file", &target.to_string_lossy());
        g.node_mut(write).unwrap().sync_format();
        let steps = g.compile_action(write, "write").unwrap();
        for _ in 0..2 {
            let _ = std::fs::remove_file(&target);
            assert!(action(&steps, write, "write").is_ok());
            assert_eq!(loader::open_tables(&target).unwrap()[0].source.row_count(), 3);
        }
        assert_eq!(run(&g.compile(write).unwrap(), write, &out).ok().unwrap().rows, 3);
        g.node_mut(write).unwrap().set("file", "Z:/没有这个目录/a.csv");
        let bad = action(&g.compile_action(write, "write").unwrap(), write, "write").err().unwrap();
        assert_eq!(bad.node, Some(write));

        custom_node_with_typed_params();
        csv_hints_read_like_viewer();
        quick_actions_run();
        kept_results_are_reused();
        every_kind_runs_in_polars();
        cleanup();
    }

    /// 显示过的节点的结果留在引擎里：接在它下游的节点从这张表接着算（回复的 reused 里有它），
    /// 查询计划照样从源头写（里面有读 CSV 的那一步）。
    fn kept_results_are_reused() {
        let csv = std::env::current_dir().unwrap().join("testdata").join("people.csv");
        let mut g = Graph::for_file(&csv, "");
        let head = g.add(&kind("slice"));
        g.node_mut(head).unwrap().set("n", "20");
        let pick = g.add(&kind("select"));
        g.node_mut(pick).unwrap().set("columns", "\"姓名\", \"城市\"");
        let out = work_dir().join("kept.parquet");
        let first = run(&g.compile(head).unwrap(), head, &out).map_err(|e| e.msg).unwrap();
        assert!(!first.reused.contains(&head));
        let second = run(&g.compile(pick).unwrap(), pick, &out).map_err(|e| e.msg).unwrap();
        assert_eq!(second.reused, [head]);
        assert_eq!(second.rows, 20);
        assert_eq!(loader::open_tables(&out).unwrap()[0].source.headers(), ["姓名", "城市"]);
        let plan = explain(&g.compile(pick).unwrap(), pick, false).map_err(|e| e.msg).unwrap().plan.0;
        assert!(plan.contains("people.csv"), "计划要从读文件开始：{plan}");
        // 改了上游：留住的结果对不上了，从头算
        g.node_mut(head).unwrap().set("n", "5");
        let third = run(&g.compile(pick).unwrap(), pick, &out).map_err(|e| e.msg).unwrap();
        assert!(third.reused.is_empty() && third.rows == 5, "{:?} / {}", third.reused, third.rows);

        // 从留住的结果里拿的 expr 节点：网络里各节点的类型、整个节点的文字照样报回来
        // （撤销、重新加载以后界面上的图是重建的，这些要重新给）
        let mut g = Graph::for_file(&csv, "");
        let (filter, _) = filter_then_count(&mut g, "40");
        let steps = g.compile(filter).unwrap();
        let first = run(&steps, filter, &out).map_err(|e| e.msg).unwrap();
        let again = run(&steps, filter, &out).map_err(|e| e.msg).unwrap();
        assert_eq!(again.reused, [filter]);
        for ok in [&first, &again] {
            assert!(ok.texts.iter().any(|(id, text)| *id == filter && text.contains("filter")), "{:?}", ok.texts);
            assert!(ok.subs.iter().any(|(id, items)| *id == filter && !items.is_empty()));
        }
    }

    /// 表格右键菜单建出来的节点（quick.rs）都跑得通，结果对。
    fn quick_actions_run() {
        let csv = std::env::current_dir().unwrap().join("testdata").join("people.csv");
        let mut g = Graph::for_file(&csv, "");
        let base = g.primary().unwrap().id;
        let out = work_dir().join("quick.parquet");
        for (what, id) in crate::quick::every_action(&mut g, base) {
            let ok = run(&g.compile(id).unwrap(), id, &out).map_err(|e| e.msg).unwrap_or_else(|e| panic!("{what}：{e}"));
            let table = loader::open_tables(&out).unwrap().remove(0).source;
            let (rows, headers) = (ok.rows, table.headers().to_vec());
            match what {
                "sort" => assert_eq!((rows, table.window(0, 1)[0][3].as_str()), (50, "97215.95"), "{what}"),
                "count" => {
                    assert_eq!(headers, ["城市", "个数"]);
                    assert_eq!(table.window(0, 1)[0], ["深圳", "18"]);
                }
                "only" => assert_eq!(rows, 15),
                "remove" => assert_eq!(rows, 35),
                "close" => assert_eq!(rows, 1),
                // 一行都不剩：表头照样在（以前 0 行的结果打不开，算运行失败）
                "null" => assert_eq!((rows, table.row_count(), headers.len()), (0, 0, 4)),
                "drop" => assert_eq!(headers, ["城市", "年龄"]),
                _ => unreachable!(),
            }
        }
    }

    /// GBK、分号分隔的 CSV：查看器认出来的读法填进源节点，引擎读出来的表头、内容和查看器显示的一样。
    fn csv_hints_read_like_viewer() {
        for name in ["gbk.csv", "semicolon.csv"] {
            let path = std::env::current_dir().unwrap().join("testdata").join(name);
            let src = loader::open_tables(&path).unwrap().remove(0).source;
            let hints = src.read_hints();
            if hints.is_empty() {
                eprintln!("跳过 {name}：系统代码页不是中文的");
                continue;
            }
            let g = Graph::for_file_with(&path, "", &hints);
            let id = g.primary().unwrap().id;
            let out = work_dir().join("hints.parquet");
            let ok = run(&g.compile(id).unwrap(), id, &out).map_err(|e| e.msg).unwrap();
            let got = loader::open_tables(&out).unwrap().remove(0).source;
            assert_eq!(got.headers(), src.headers(), "{name}");
            assert_eq!(ok.rows as usize, src.row_count(), "{name}");
            // 文字列一样（数字列 polars 推成了数字，写法可能不同）
            let last = src.row_count() - 1;
            assert_eq!(got.window(last, 1)[0][0], src.window(last, 1)[0][0], "{name}");
        }
    }

    /// 自定义节点：各种类型的参数经 ch() 变成对应的 Python 值；换了参数值缓存就失效。
    fn custom_node_with_typed_params() {
        let text = r#""""
{"desc": "测试", "inputs": 1, "params": [
  {"name": "n", "type": "int", "default": "2"},
  {"name": "ratio", "type": "float", "default": "1.5"},
  {"name": "flag", "type": "toggle", "default": "True"},
  {"name": "cols", "type": "columns", "default": "\"姓名\", \"年龄\""},
  {"name": "cond", "type": "expr"},
  {"name": "names", "type": "dict", "default": "\"姓名\": \"name\""},
  {"name": "opts", "type": "kwargs", "default": "offset=1"},
  {"name": "sep", "default": " x ", "strip": false},
  {"name": "word", "default": " x "}
]}
"""
assert isinstance(ch("n"), int) and isinstance(ch("ratio"), float) and ch("flag") is True
assert ch("cols") == ["姓名", "年龄"] and ch("names") == {"姓名": "name"} and ch("opts") == {"offset": 1}
assert (ch("sep"), ch("word")) == (" x ", "x") and action is None and inputs[0] is df
assert opt(a=None, b="", c=[], d=0, e=False) == {"d": 0, "e": False}
frame = df if ch("cond") is None else df.filter(ch("cond"))
out = frame.select(ch("cols")).rename(ch("names")).with_row_index(**ch("opts")).head(ch("n"))
"#;
        let t = std::sync::Arc::new(nodes::NodeType::parse("typed", text, nodes::Source::User).unwrap());
        let csv = std::env::current_dir().unwrap().join("testdata").join("people.csv");
        let mut g = Graph::for_file(&csv, "");
        let id = g.add(&t);
        let out = work_dir().join("typed.parquet");
        let rows = |g: &Graph| run(&g.compile(id).unwrap(), id, &out).map(|ok| ok.rows).map_err(|e| e.msg);
        assert_eq!(rows(&g), Ok(2));
        assert_eq!(loader::open_tables(&out).unwrap()[0].source.headers(), ["index", "name", "年龄"]);
        g.node_mut(id).unwrap().set("n", "5");
        g.node_mut(id).unwrap().set("cond", "pl.col(\"年龄\") > 200");
        assert_eq!(rows(&g), Ok(0));
        g.node_mut(id).unwrap().set("n", "两个");
        assert!(rows(&g).unwrap_err().contains("参数“n”"));
        g.node_mut(id).unwrap().set("n", "1");
        g.node_mut(id).unwrap().set("cond", ""); // 留空 = None
        assert_eq!(rows(&g), Ok(1));
    }

    /// 测例：在一张图上不断接新节点，跑一遍，记下跑不通的和测到了哪些（节点, 方式）。
    struct Cases {
        g: Graph,
        out: PathBuf,
        failed: Vec<String>,
        tried: Vec<(String, String)>,
    }

    impl Cases {
        /// 在 up 后面接一个节点（两个输入的第二个接 right），设好参数。
        fn add(&mut self, up: u32, right: u32, name: &str, set: &[(&str, &str)]) -> u32 {
            self.g.selected = Some(up);
            let t = kind(name);
            let id = self.g.add(&t);
            if t.inputs >= 2 {
                self.g.connect(right, id, 1);
            }
            for (param, value) in set {
                assert!(self.g.node_mut(id).unwrap().set(param, value), "{name} 没有参数 {param}");
            }
            if t.network {
                self.g.node_mut(id).unwrap().ensure_outputs();
            }
            self.tried.push((name.to_string(), self.g.node(id).unwrap().mode().to_string()));
            id
        }

        fn run(&mut self, id: u32, what: &str) -> Result<RunOk, String> {
            let result = self.g.compile(id).map_err(|e| e.msg).and_then(|steps| run(&steps, id, &self.out).map_err(|e| e.msg));
            if let Err(msg) = &result {
                self.failed.push(format!("{what}\n    {msg}"));
            }
            result
        }

        fn case(&mut self, up: u32, right: u32, name: &str, set: &[(&str, &str)]) {
            let id = self.add(up, right, name, set);
            let _ = self.run(id, &format!("{name} {set:?}"));
        }

        /// expr 节点：网络里一个表达式节点接到输出上（端口 inputs 依次接这些列），上下文默认 with_columns。
        /// 返回跑出来的结果，跑不通记下。
        fn expr(&mut self, up: u32, context: &str, name: &str, set: &[(&str, &str)], inputs: &[&str]) -> Option<RunOk> {
            let e = self.add(up, up, "expr", &[("context", context)]);
            let g = &mut self.g;
            let col = first(g, e, "col");
            let sub = add_expr(g, e, name, set);
            set_expr(g, e, col, &[("columns", inputs.first().copied().unwrap_or("\"i\""))]);
            if !inputs.is_empty() {
                wire(g, e, col, sub, 0);
            }
            for (slot, column) in inputs.iter().enumerate().skip(1) {
                let more = add_expr(g, e, "col", &[("columns", column)]);
                wire(g, e, more, sub, slot);
            }
            let output = net_of(g, e).nodes.iter().find(|n| !n.kind.role.is_empty()).unwrap().id;
            wire(g, e, sub, output, 0);
            set_expr(g, e, output, &[("naming", "suffix"), ("affix", "_新")]);
            self.tried.push((name.to_string(), net_of(&mut self.g, e).node(sub).unwrap().mode().to_string()));
            let result = self.run(e, &format!("{name} {set:?} ← {inputs:?}")).ok();
            self.g.remove(e);
            result
        }
    }

    /// 每种节点、每个“方式”都拿真的 polars 跑一遍：方法名、参数名写错了在这里现形。
    fn every_kind_runs_in_polars() {
        let options = |name: &str| -> Vec<String> {
            let t = kind(name);
            t.params[t.mode_param().unwrap()].options.clone()
        };
        let mut c = Cases { g: Graph::default(), out: work_dir().join("case.parquet"), failed: Vec::new(), tried: Vec::new() };
        // 跑出来的行数、列数
        let shape = |ok: &RunOk, id: u32| (ok.rows, ok.columns.iter().find(|(n, _)| *n == id).map_or(0, |(_, names)| names.len()));
        // all：各种类型的列都有；plain：只有普通列、按 i 排好序；other：列名和 plain 不重复
        let all = c.g.add(&kind("code"));
        c.g.node_mut(all).unwrap().set(
            "code",
            "out = pl.DataFrame({\"k\": [\"a\", \"b\", \"a\", \"b\"], \"s\": [\" x1 \", \"y22\", None, \"z3\"], \
             \"n\": [1.5, None, 3.25, 4.0], \"i\": [4, 3, 2, 1], \"t\": [\"2024-01-05\", \"2024-02-01\", \"x\", None], \
             \"l\": [[1, 2], [3], [], [4]], \"a\": pl.Series([[1, 2], [3, 4], [5, 6], [7, 8]], dtype=pl.Array(pl.Int64, 2))}\
             ).with_columns(ts=pl.datetime(2024, pl.col(\"i\"), 5, 13, 30), st=pl.struct(x=pl.col(\"i\"), y=pl.col(\"n\")), \
             c=pl.col(\"k\").cast(pl.Categorical), b=pl.col(\"k\").cast(pl.Binary),              mp=pl.col(\"l\").list.eval(pl.struct(key=pl.element().cast(pl.String), value=pl.element())).list.to_map())",
        );
        let plain = c.g.add(&kind("code"));
        c.g.node_mut(plain).unwrap().set("code", "df.select(\"k\", \"i\", \"n\").sort(\"i\")");
        c.g.selected = Some(all);
        let other = c.g.add(&kind("code"));
        c.g.node_mut(other).unwrap().set("code", "df.select(j=pl.col(\"i\"), k2=pl.col(\"k\"), m=pl.col(\"n\")).sort(\"j\")");

        // ---- 行
        c.case(all, all, "unique", &[]);
        c.case(all, all, "unique", &[("subset", "\"k\""), ("keep", "first")]);
        let dup = c.add(all, all, "unique", &[("mode", "is_duplicated"), ("subset", "\"k\"")]);
        assert_eq!(c.run(dup, "unique is_duplicated").map(|ok| ok.rows), Ok(4));
        c.case(plain, plain, "unique", &[("mode", "is_duplicated")]);
        for take in options("slice") {
            let extra: &[(&str, &str)] = match take.as_str() {
                "top_k" | "bottom_k" => &[("by", "\"i\"")],
                "sample" => &[("sample_n", "2"), ("seed", "1")],
                "gather" => &[("indices", "0, -1")],
                _ => &[],
            };
            c.case(all, all, "slice", &[&[("mode", take.as_str())][..], extra].concat());
        }
        c.case(all, all, "slice", &[("mode", "sample"), ("sample_n", ""), ("fraction", "0.5")]);
        c.case(all, all, "drop_nulls", &[]);
        c.case(all, all, "drop_nulls", &[("mode", "drop_nans"), ("subset", "\"n\"")]);
        c.case(all, all, "fill_null", &[("value", "0")]);
        c.case(all, all, "fill_null", &[("strategy", "forward"), ("limit", "1")]);
        c.case(plain, plain, "fill_null", &[("mode", "fill_nan"), ("value", "0")]);
        c.case(plain, plain, "fill_null", &[("mode", "interpolate")]);
        c.case(all, all, "fill_null", &[("mode", "shift"), ("n", "-1")]);
        c.case(plain, plain, "fill_null", &[("mode", "shift"), ("fill_value", "0")]);
        let upsample = c.add(all, all, "upsample", &[("time_column", "\"ts\""), ("every", "1mo")]);
        assert_eq!(c.run(upsample, "upsample").map(|ok| ok.rows), Ok(4));
        c.case(all, all, "upsample", &[("time_column", "\"ts\""), ("every", "1w"), ("group_by", "\"k\"")]);
        // ---- 列
        c.case(all, all, "select", &[("columns", "\"k\", cs.numeric()")]);
        c.case(all, all, "select", &[("mode", "drop"), ("columns", "\"l\", \"st\"")]);
        // 按位置的范围参数：“起:止”（1 起、含两端），列名按当时的位置算，倒着写会调换；和列或表达式合在一起
        let ranged = c.add(all, all, "select", &[("ranges", "1:3, i:ts, 10:12")]);
        if c.run(ranged, "select 范围").is_ok() {
            assert_eq!(loader::open_tables(&c.out).unwrap()[0].source.headers(),
                ["k", "s", "n", "i", "t", "l", "a", "ts", "c", "b", "mp"]);
        }
        let ranged = c.add(all, all, "select", &[("ranges", "\"ts\":\"i\"")]);
        if c.run(ranged, "select 范围倒着写").is_ok() {
            assert_eq!(loader::open_tables(&c.out).unwrap()[0].source.headers(), ["i", "t", "l", "a", "ts"]);
        }
        let ranged = c.add(all, all, "select", &[("columns", "\"st\""), ("ranges", "1:2")]);
        if c.run(ranged, "select 列名 + 范围").is_ok() {
            assert_eq!(loader::open_tables(&c.out).unwrap()[0].source.headers(), ["st", "k", "s"]);
        }
        let ranged = c.add(all, all, "select", &[("mode", "drop"), ("ranges", "1:3")]);
        if c.run(ranged, "drop 范围").is_ok() {
            assert_eq!(loader::open_tables(&c.out).unwrap()[0].source.headers(),
                ["i", "t", "l", "a", "ts", "st", "c", "b", "mp"]);
        }
        c.case(all, all, "rename", &[("mapping", "\"k\": \"K\"")]);
        c.case(all, all, "rename", &[("mode", "前缀"), ("prefix", "p_")]);
        c.case(all, all, "rename", &[("mode", "后缀"), ("suffix", "_s")]);
        c.case(all, all, "rename", &[("mode", "大写")]);
        c.case(all, all, "rename", &[("mode", "小写")]);
        c.case(plain, plain, "cast_columns", &[("dtypes", "\"i\": pl.Float64, cs.string(): pl.Categorical")]);
        let cast = c.add(all, all, "cast_columns", &[("dtypes", "\"s\": pl.Int64, \"t\": pl.Date"), ("strict", "False")]);
        let _ = c.run(cast, "cast_columns strict=False");
        // 对齐成给定的结构：按结构排列，缺的补空列、多的去掉、Int64 → Int64；再试报错的默认
        let schema = "\"i\": pl.Int64, \"k\": pl.String, \"新\": pl.Date";
        let matched = [("mode", "match_to_schema"), ("schema", schema), ("missing_columns", "insert"), ("extra_columns", "ignore")];
        let id = c.add(all, all, "cast_columns", &matched);
        if let Ok(ok) = c.run(id, "cast_columns match_to_schema") {
            let names = ok.columns.iter().find(|(n, _)| *n == id).map(|(_, names)| names.clone()).unwrap_or_default();
            assert_eq!(names, ["i", "k", "新"]);
        }
        let strict = c.add(all, all, "cast_columns", &[("mode", "match_to_schema"), ("schema", schema)]);
        assert!(c.run(strict, "match_to_schema 默认").is_err(), "缺列、多列默认要报错");
        c.failed.pop();
        c.case(all, all, "with_row_index", &[("name", "行"), ("offset", "1")]);
        // ---- 表达式：每种上下文
        expr_contexts(&mut c, all, plain);
        // ---- 统计
        for stat in options("describe") {
            c.case(plain, plain, "describe", &[("mode", stat.as_str())]);
        }
        // ---- 变形
        c.case(plain, plain, "pivot", &[("on", "\"k\""), ("index", "\"i\""), ("values", "\"n\""), ("aggregate_function", "sum")]);
        c.case(plain, plain, "unpivot", &[("on", "\"i\", \"n\""), ("index", "\"k\""), ("variable_name", "项"), ("value_name", "值")]);
        c.case(plain, plain, "unpivot", &[("index", "\"k\"")]);
        c.case(plain, plain, "transpose", &[("header_name", "项")]);
        c.case(plain, plain, "transpose", &[("include_header", "False"), ("column_names", "[\"w\", \"x\", \"y\", \"z\"]")]);
        let unstack = c.add(plain, plain, "transpose", &[("mode", "unstack"), ("step", "2")]);
        assert_eq!(c.run(unstack, "unstack").map(|ok| shape(&ok, unstack)), Ok((2, 6)));
        c.case(plain, plain, "transpose", &[("mode", "unstack"), ("step", "3"), ("how", "horizontal"), ("columns", "\"i\""), ("fill_values", "0")]);
        c.case(all, all, "explode", &[("columns", "\"l\"")]);
        c.case(all, all, "explode", &[("mode", "unnest"), ("columns", "\"st\"")]);
        c.case(plain, plain, "to_dummies", &[("columns", "\"k\""), ("separator", "="), ("drop_first", "True")]);
        c.case(plain, plain, "to_dummies", &[("columns", "\"k\", \"i\"")]);
        // ---- 多表
        for how in ["inner", "left", "right", "full", "semi", "anti"] {
            c.case(plain, other, "join", &[("left_on", "\"i\""), ("right_on", "\"j\""), ("how", how)]);
        }
        c.case(plain, plain, "join", &[("on", "\"k\", \"i\""), ("suffix", "_右")]);
        c.case(plain, other, "join", &[("how", "cross")]);
        let asof = [("mode", "join_asof"), ("left_on", "\"i\""), ("right_on", "\"j\""), ("strategy", "nearest"), ("tolerance", "1")];
        c.case(plain, other, "join", &asof);
        c.case(plain, plain, "join", &[("mode", "join_asof"), ("on", "\"i\""), ("by", "\"k\"")]);
        c.case(plain, other, "join", &[("mode", "join_where"), ("predicates", "pl.col(\"i\") > pl.col(\"j\"), pl.col(\"n\") > 1")]);
        c.case(plain, plain, "join", &[("mode", "update"), ("on", "\"i\""), ("update_how", "full"), ("include_nulls", "True")]);
        for how in ["vertical", "vertical_relaxed", "diagonal", "diagonal_relaxed", "horizontal", "align"] {
            let right = if how == "horizontal" { other } else { plain };
            c.case(plain, right, "concat", &[("how", how)]);
        }
        let merged = c.add(plain, plain, "concat", &[("mode", "merge_sorted"), ("key", "\"i\"")]);
        assert_eq!(c.run(merged, "merge_sorted").map(|ok| ok.rows), Ok(8));
        // ---- 代码
        c.case(all, all, "sql", &[("query", "SELECT k, SUM(i) AS i FROM self GROUP BY k")]);
        let sql = c.add(plain, other, "sql", &[("query", "SELECT a.k, b.m FROM self a JOIN df2 b ON a.i = b.j")]);
        assert_eq!(c.run(sql, "sql 两张表").map(|ok| shape(&ok, sql)), Ok((4, 2)));
        // ---- IO：不读文件造一张表
        let made = c.add(all, all, "create", &[("exprs", "pl.int_range(1, 13).alias(\"月\"), pl.lit(\"a\").alias(\"x\")")]);
        assert_eq!(c.run(made, "create select").map(|ok| shape(&ok, made)), Ok((12, 2)));
        let made = c.add(all, all, "create", &[("mode", "from_dict"), ("data", "\"城市\": [\"北京\", \"上海\"], \"人数\": [3, 5]")]);
        assert_eq!(c.run(made, "create from_dict").map(|ok| ok.rows), Ok(2));
        let table = "shape: (2, 1)
┌─────┐
│ a   │
│ --- │
│ i64 │
╞═════╡
│ 1   │
│ 2   │
└─────┘";
        let made = c.add(all, all, "create", &[("mode", "from_repr"), ("table", table)]);
        assert_eq!(c.run(made, "create from_repr").map(|ok| ok.rows), Ok(2));
        // 从 Excel 复制的（制表符分隔，CRLF 在粘贴时已经换成 \n）
        let pasted = "城市\t人数\n北京\t3\n上海\t5\n";
        let made = c.add(all, all, "create", &[("mode", "read_clipboard"), ("pasted", pasted)]);
        assert_eq!(c.run(made, "create read_clipboard").map(|ok| shape(&ok, made)), Ok((2, 2)));

        // ---- 查询计划：不执行，文字和树两种画法
        let steps = c.g.compile(plain).unwrap();
        match explain(&steps, plain, false) {
            Ok(ok) => assert!(ok.plan.0.contains("SORT") && !ok.plan.1.is_empty(), "{:?}", ok.plan),
            Err(e) => c.failed.push(format!("explain\n    {}", e.msg)),
        }
        match explain(&steps, plain, true) {
            Ok(ok) => assert!(ok.plan.0.contains("SORT"), "{:?}", ok.plan),
            Err(e) => c.failed.push(format!("explain tree\n    {}", e.msg)),
        }

        // ---- 表达式节点：精选的每个方式，和从 polars 生成的全部
        curated_expr_kinds(&mut c, all);
        eval_kinds(&mut c, all);
        generated_expr_kinds(&mut c, all);

        // 每种节点的每个“方式”都跑到了（新加了方式却没加例子，这里会提醒）
        let builtin = |t: &&std::sync::Arc<nodes::NodeType>| t.source == nodes::Source::Builtin;
        let curated = nodes::all_expr().into_iter().filter(|t| t.order < 1000).collect::<Vec<_>>();
        for t in nodes::all().iter().filter(builtin).filter(|t| !t.is_io() && t.name != "code").chain(curated.iter()) {
            let modes = t.mode_param().map_or(vec![String::new()], |i| t.params[i].options.clone());
            for mode in modes {
                assert!(c.tried.contains(&(t.name.clone(), mode.clone())), "{} 的 {mode} 没有测到", t.name);
            }
        }

        // ---- IO：每种格式写出去再读回来
        let files = ["io.xlsx", "io.csv", "io.tsv", "io.parquet", "io.json", "io.ndjson", "io.arrow", "io.arrows", "io.avro"];
        for file in files {
            let target = work_dir().join(file).to_string_lossy().into_owned();
            c.g.selected = Some(plain);
            let write = c.g.add(&kind("write"));
            c.g.node_mut(write).unwrap().set("file", &target);
            c.g.node_mut(write).unwrap().sync_format();
            if file == "io.tsv" {
                c.g.node_mut(write).unwrap().set("separator", "\\t");
            }
            let written = c.g.compile_action(write, "write").map_err(|e| e.msg);
            if let Err(msg) = written.and_then(|steps| action(&steps, write, "write").map_err(|e| e.msg)) {
                c.failed.push(format!("write {file}\n    {msg}"));
                continue;
            }
            // 和打开文件时建的源节点一样配：tsv 的分隔符是制表符
            let mut source = Graph::for_file(std::path::Path::new(&target), "Sheet1");
            source.nodes[0].primary = false;
            match run(&source.compile(1).unwrap(), 1, &c.out) {
                Ok(ok) if ok.rows == 4 && ok.columns[0].1 == ["k", "i", "n"] => {}
                Ok(ok) => c.failed.push(format!("read {file}：{} 行，列 {:?}", ok.rows, ok.columns)),
                Err(e) => c.failed.push(format!("read {file}\n    {}", e.msg)),
            }
        }
        // 按列拆成多个文件（hive 分区）：文件夹是文件名去掉扩展名，读回来用通配符
        for (file, pattern) in [("parts.parquet", "parts/**/*.parquet"), ("parts_csv.csv", "parts_csv/**/*.csv")] {
            let _ = std::fs::remove_dir_all(work_dir().join(file.split('.').next().unwrap()));
            c.g.selected = Some(plain);
            let write = c.g.add(&kind("write"));
            let target = work_dir().join(file).to_string_lossy().into_owned();
            c.g.node_mut(write).unwrap().set("file", &target);
            c.g.node_mut(write).unwrap().sync_format();
            c.g.node_mut(write).unwrap().set("partition_by", "\"k\"");
            let written = c.g.compile_action(write, "write").map_err(|e| e.msg);
            if let Err(msg) = written.and_then(|steps| action(&steps, write, "write").map_err(|e| e.msg)) {
                c.failed.push(format!("write 按列拆分 {file}\n    {msg}"));
                continue;
            }
            let dirs = std::fs::read_dir(work_dir().join(file.split('.').next().unwrap())).map_or(0, |d| d.count());
            assert_eq!(dirs, 2, "{file}：k 有两个值，应该拆成两个文件夹");
            let glob = work_dir().join(pattern).to_string_lossy().into_owned();
            let mut source = Graph::for_file(std::path::Path::new(&glob), "");
            source.nodes[0].primary = false;
            match run(&source.compile(1).unwrap(), 1, &c.out) {
                Ok(ok) if ok.rows == 4 => {}
                Ok(ok) => c.failed.push(format!("read 拆分的 {pattern}：{} 行", ok.rows)),
                Err(e) => c.failed.push(format!("read 拆分的 {pattern}\n    {}", e.msg)),
            }
        }
        // read 的 Excel 选项
        let mut source = Graph::for_file(&work_dir().join("io.xlsx"), "");
        for (param, value) in [("columns", "0, 2"), ("header_row", "1"), ("extra", "raise_if_empty=False")] {
            assert!(source.nodes[0].set(param, value));
        }
        assert!(matches!(source.nodes[0].kind.params[0].kind, ParamKind::File));
        match run(&source.compile(1).unwrap(), 1, &c.out) {
            Ok(ok) if ok.rows == 3 && ok.columns[0].1.len() == 2 => {}
            Ok(ok) => c.failed.push(format!("read xlsx 选项：{} 行，列 {:?}", ok.rows, ok.columns)),
            Err(e) => c.failed.push(format!("read xlsx 选项\n    {}", e.msg)),
        }
        // read 按行读：utf8 走 scan_lines，其他编码 Python 解码；跳过开头几行
        let gbk: &[u8] = &[0xCD, 0xB7, b'\n', 0xBC, 0xD7, b',', 0xD2, 0xD2, b'\n', b'x', b'\n']; // 头 / 甲,乙 / x
        for (file, bytes, encoding) in [("lines.log", "头\n甲,乙\nx\n".as_bytes(), "utf8"), ("lines_gbk.txt", gbk, "gbk")] {
            let path = work_dir().join(file);
            std::fs::write(&path, bytes).unwrap();
            let mut source = Graph::for_file(&path, "");
            source.nodes[0].primary = false;
            for (param, value) in [("lines", "True"), ("encoding", encoding), ("skip_rows", "1")] {
                assert!(source.nodes[0].set(param, value));
            }
            match run(&source.compile(1).unwrap(), 1, &c.out) {
                Ok(ok) if ok.rows == 2 && ok.columns[0].1 == ["line"] => {
                    assert_eq!(loader::open_tables(&c.out).unwrap()[0].source.window(0, 1)[0][0], "甲,乙");
                }
                Ok(ok) => c.failed.push(format!("read 按行读 {file}：{} 行，列 {:?}", ok.rows, ok.columns)),
                Err(e) => c.failed.push(format!("read 按行读 {file}\n    {}", e.msg)),
            }
        }
        assert!(c.failed.is_empty(), "{} 个节点跑不通：\n{}", c.failed.len(), c.failed.join("\n"));
    }

    /// expr 节点的每种上下文：网络里接好输出，跑出来的列 / 行对得上。
    fn expr_contexts(c: &mut Cases, all: u32, plain: u32) {
        let columns = |c: &Cases, ok: &RunOk, id: u32| -> Vec<String> {
            let _ = c;
            ok.columns.iter().find(|(n, _)| *n == id).map(|(_, names)| names.clone()).unwrap_or_default()
        };
        // with_columns：i * 2 放进 j
        let e = c.add(all, all, "expr", &[]);
        let col = first(&mut c.g, e, "col");
        set_expr(&mut c.g, e, col, &[("columns", "\"i\"")]);
        let mul = add_expr(&mut c.g, e, "arith", &[("op", "*"), ("b", "2")]);
        wire(&mut c.g, e, col, mul, 0);
        let output = first(&mut c.g, e, "output");
        wire(&mut c.g, e, mul, output, 0);
        set_expr(&mut c.g, e, output, &[("name", "j")]);
        if let Ok(ok) = c.run(e, "expr with_columns") {
            assert_eq!(columns(c, &ok, e).last().map(String::as_str), Some("j"));
        }
        // select：两个输出，按画布上从上到下排
        let e = c.add(all, all, "expr", &[("context", "select")]);
        let col = first(&mut c.g, e, "col");
        set_expr(&mut c.g, e, col, &[("columns", "\"i\"")]);
        let second = add_expr(&mut c.g, e, "output", &[("value", "pl.col(\"k\")")]);
        net_of(&mut c.g, e).node_mut(second).unwrap().y = -100.0;
        if let Ok(ok) = c.run(e, "expr select") {
            assert_eq!(columns(c, &ok, e), ["k", "i"]);
        }
        // filter / remove：i > 2 的有两行；再加一个 i == 1，“任一满足”时三行，“全部满足”时没有
        for keep in ["留下满足的", "去掉满足的"] {
            let e = c.add(all, all, "expr", &[("context", "filter"), ("keep", keep)]);
            let col = first(&mut c.g, e, "col");
            set_expr(&mut c.g, e, col, &[("columns", "\"i\"")]);
            let cmp = add_expr(&mut c.g, e, "compare", &[("op", ">"), ("b", "2")]);
            wire(&mut c.g, e, col, cmp, 0);
            let cond = first(&mut c.g, e, "condition");
            wire(&mut c.g, e, cmp, cond, 0);
            if let Ok(ok) = c.run(e, &format!("expr filter {keep}")) {
                assert_eq!(ok.rows, 2);
            }
            add_expr(&mut c.g, e, "condition", &[("value", "pl.col(\"i\") == 1")]);
            for (combine, rows) in [("全部满足", 0), ("任一满足", 3)] {
                c.g.node_mut(e).unwrap().set("combine", combine);
                let rows = if keep == "留下满足的" { rows } else { 4 - rows };
                if let Ok(ok) = c.run(e, &format!("expr filter {keep} {combine}")) {
                    assert_eq!(ok.rows, rows, "{keep} {combine}");
                }
            }
        }
        // filter 没有条件：说清楚要加什么
        let e = c.add(all, all, "expr", &[("context", "filter")]);
        let cond = first(&mut c.g, e, "condition");
        net_of(&mut c.g, e).remove(cond);
        let msg = c.run(e, "").err().unwrap_or_default();
        assert!(msg.contains("条件"), "{msg}");
        c.failed.pop();
        // group_by：按 k 求 n 的和；按时间窗口
        for (every, period) in [("", ""), ("2mo", ""), ("", "2mo")] {
            let set = [("context", "group_by"), ("every", every), ("period", period), ("index_column", "\"ts\"")];
            let e = c.add(all, all, "expr", &set);
            let col = first_in(&mut c.g, e, Env::Keys, "col");
            set_expr(&mut c.g, e, col, &[("columns", "\"k\"")]);
            let sum = add_expr(&mut c.g, e, "agg", &[("input", "pl.col(\"n\")")]);
            let output = first(&mut c.g, e, "output");
            wire(&mut c.g, e, sum, output, 0);
            if let Ok(ok) = c.run(e, &format!("expr group_by every={every} period={period}")) {
                let names = columns(c, &ok, e);
                assert!(names.contains(&String::from("k")) && names.contains(&String::from("n")), "{names:?}");
            }
        }
        // group_by 的每种“分组后”（plain：k 有 a / b 两组，各两行），快捷方法用不到网络里的输出列
        let then_options = kind("expr").params.iter().find(|p| p.name == "then").unwrap().options.clone();
        for then in &then_options {
            let e = c.add(plain, plain, "expr", &[("context", "group_by"), ("then", then.as_str()), ("n", "1")]);
            let col = first_in(&mut c.g, e, Env::Keys, "col");
            set_expr(&mut c.g, e, col, &[("columns", "\"k\"")]);
            if then == "agg" {
                let output = first(&mut c.g, e, "output");
                net_of(&mut c.g, e).disconnect(output, 0);
                set_expr(&mut c.g, e, output, &[("value", "pl.col(\"i\").sum()")]);
            }
            if let Ok(ok) = c.run(e, &format!("expr group_by then={then}")) {
                assert_eq!(ok.rows, 2, "group_by {then}");
                assert_eq!(columns(c, &ok, e).first().map(String::as_str), Some("k"), "group_by {then}");
            }
        }
        // having：只留 i 的和大于 4 的组（a：4 + 2 = 6；b：3 + 1 = 4）
        for then in ["agg", "sum"] {
            let e = c.add(plain, plain, "expr", &[("context", "group_by"), ("then", then)]);
            let col = first_in(&mut c.g, e, Env::Keys, "col");
            set_expr(&mut c.g, e, col, &[("columns", "\"k\"")]);
            if then == "agg" {
                let output = first(&mut c.g, e, "output");
                net_of(&mut c.g, e).disconnect(output, 0);
                set_expr(&mut c.g, e, output, &[("value", "pl.col(\"i\").sum()")]);
            }
            add_in(&mut c.g, e, Env::Having, "condition", &[("value", "pl.col(\"i\").sum() > 4")]);
            if let Ok(ok) = c.run(e, &format!("expr group_by having then={then}")) {
                assert_eq!(ok.rows, 1, "having {then}");
            }
        }

        // sort：k 升序、i 降序
        let e = c.add(all, all, "expr", &[("context", "sort")]);
        let key = first(&mut c.g, e, "sort_key");
        set_expr(&mut c.g, e, key, &[("value", "pl.col(\"k\")")]);
        let second = add_expr(&mut c.g, e, "sort_key", &[("value", "pl.col(\"i\")"), ("order", "从大到小")]);
        net_of(&mut c.g, e).node_mut(second).unwrap().y = 500.0;
        let _ = c.run(e, "expr sort");
        for order in ["从小到大", "从大到小"] {
            c.tried.push((String::from("sort_key"), order.to_string()));
        }
    }

    /// eval 节点里面的网络（不是 expr 节点的那张）。
    fn inner_net(g: &mut Graph, e: u32, ev: u32) -> &mut Graph {
        net_of(g, e).node_mut(ev).and_then(|n| n.net.as_deref_mut()).expect("不是 eval 节点")
    }

    /// expr（with_columns）里：col(column) → eval（method）→ 输出；eval 里面：element → inner → 结果。
    /// inner 为空时 element 直接接结果。返回 (expr 节点, eval 节点, eval 里面 inner 那个节点)。
    fn eval_case(c: &mut Cases, up: u32, column: &str, method: &str, inner: Option<(&str, &[(&str, &str)])>) -> (u32, u32, u32) {
        let e = c.add(up, up, "expr", &[]);
        let col = first(&mut c.g, e, "col");
        set_expr(&mut c.g, e, col, &[("columns", column)]);
        let ev = add_expr(&mut c.g, e, "eval", &[("method", method)]);
        wire(&mut c.g, e, col, ev, 0);
        let output = first(&mut c.g, e, "output");
        wire(&mut c.g, e, ev, output, 0);
        set_expr(&mut c.g, e, output, &[("name", "结果")]);
        let net = inner_net(&mut c.g, e, ev);
        let names: Vec<&str> = net.nodes.iter().map(|n| n.kind.name.as_str()).collect();
        assert_eq!(names, ["element", "result"], "eval 里面一开始是 element 接着结果");
        let (element, result) = (net.nodes[0].id, net.nodes[1].id);
        let mut sub = element;
        if let Some((name, set)) = inner {
            net.selected = None;
            sub = net.add(&expr_kind(name));
            for (param, value) in set {
                assert!(net.node_mut(sub).unwrap().set(param, value), "{name} 没有参数 {param}");
            }
            assert!(net.connect(element, sub, 0));
            assert!(net.connect(sub, result, 0));
        }
        c.tried.push((String::from("eval"), method.to_string()));
        (e, ev, sub)
    }

    /// eval 节点的每个方式：列表列 l、数组列 a 里的每个元素。
    fn eval_kinds(c: &mut Cases, all: u32) {
        let double: (&str, &[(&str, &str)]) = ("arith", &[("op", "*"), ("b", "2")]);
        let big: (&str, &[(&str, &str)]) = ("compare", &[("op", ">"), ("b", "1")]);
        let sum: (&str, &[(&str, &str)]) = ("agg", &[("function", "sum")]);
        for (column, method, inner) in [
            ("\"l\"", "list.eval", Some(double)),
            ("\"l\"", "list.filter", Some(big)),
            ("\"l\"", "list.agg", Some(sum)),
            ("\"a\"", "arr.eval", Some(double)),
            ("\"a\"", "arr.agg", Some(sum)),
            ("\"l\"", "list.eval", None),
        ] {
            let (e, ..) = eval_case(c, all, column, method, inner);
            let _ = c.run(e, &format!("eval {method} {inner:?}"));
            c.g.remove(e);
        }
        c.tried.push((String::from("element"), String::new()));
        c.tried.push((String::from("result"), String::new()));
    }

    /// eval 节点里面：类型是每个元素的、预览按路径、出错报到里面那个节点、网络写成文字、文字拆成节点。
    fn eval_inside(out: &std::path::Path) {
        let mut c = Cases { g: Graph::default(), out: out.to_path_buf(), failed: Vec::new(), tried: Vec::new() };
        let src = c.g.add(&kind("code"));
        c.g.node_mut(src).unwrap().set("code", "pl.DataFrame({\"l\": [[1, 2, 3], [4]], \"k\": [\"a\", \"b\"]})");
        let (e, ev, mul) = eval_case(&mut c, src, "\"l\"", "list.eval", Some(("arith", &[("op", "*"), ("b", "10")])));
        let ok = run(&c.g.compile(e).unwrap(), e, out).ok().unwrap();
        let infos = &ok.subs.iter().find(|(id, _)| *id == e).unwrap().1;
        let info = |id: u32| infos.iter().find(|(i, _)| *i == id).unwrap().1.clone();
        assert_eq!(info(ev).dtype, "List(Int64)");
        assert_eq!(info(ev).text, "pl.col(\"l\").list.eval(pl.element() * 10)");
        let inside = info(ev).net;
        let inner = |id: u32| inside.iter().find(|(i, _)| *i == id).unwrap().1.clone();
        assert_eq!((inner(mul).dtype.as_str(), inner(mul).text.as_str()), ("Int64", "pl.element() * 10"), "里面是每个元素的类型");
        let text = &ok.texts.iter().find(|(id, _)| *id == e).unwrap().1;
        assert_eq!(text, "df.with_columns([pl.col(\"l\").list.eval(pl.element() * 10).alias(\"结果\")])");

        // 导出的 Python 脚本：eval 里面的网络一样跑得通
        let script = c.g.python_script(e, script_prelude()).unwrap();
        let path = work_dir().join("script-eval.py");
        std::fs::write(&path, &script).unwrap();
        let output = Command::new(python_path()).arg(&path).env("PYTHONIOENCODING", "utf-8").output().unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(output.status.success(), "{}
{}", String::from_utf8_lossy(&output.stderr), script);
        assert!(stdout.contains("[10, 20, 30]"), "{stdout}");

        // 预览 eval 里面的节点：每行的列表连同用到的列
        let ok = preview(&c.g.compile(e).unwrap(), e, &[ev, mul], "arith1", out).ok().unwrap();
        let shown = loader::open_tables(out).unwrap();
        assert_eq!(shown[0].source.headers(), ["l", "→ arith1"]);
        assert_eq!(ok.rows, 2);
        assert_eq!(shown[0].source.window(0, 1)[0][1], "[10, 20, 30]");

        // 里面的节点出错：路径报到那个节点
        set_expr(&mut c.g, e, ev, &[]);
        inner_net(&mut c.g, e, ev).node_mut(mul).unwrap().set("b", "pl.col(");
        let err = run(&c.g.compile(e).unwrap(), e, out).err().unwrap();
        assert_eq!((err.node, err.sub.clone()), (Some(e), vec![ev, mul]), "{}", err.msg);

        // 文字 → 网络：拆成节点再算，结果一样
        let text = "pl.col(\"l\").list.eval(pl.element() * 10).list.sum().alias(\"和\")";
        let parsed = parse(text, crate::convert::tests::types()).unwrap();
        let e2 = c.add(src, src, "expr", &[]);
        let net = net_of(&mut c.g, e2);
        let old: Vec<u32> = net.nodes.iter().map(|n| n.id).collect();
        for id in old {
            net.remove(id);
        }
        let out_id = net.insert_text(&parsed, (600.0, 40.0), true).unwrap();
        let names: Vec<&str> = net.nodes.iter().map(|n| n.kind.name.as_str()).collect();
        assert_eq!(names, ["col", "eval", "list.sum", "output"]);
        assert_eq!(net.node(out_id).unwrap().value("name"), "和");
        let ok = run(&c.g.compile(e2).unwrap(), e2, out).ok().unwrap();
        let shown = loader::open_tables(out).unwrap();
        assert_eq!(shown[0].source.headers(), ["l", "k", "和"]);
        assert_eq!(shown[0].source.window(0, 2).iter().map(|r| r[2].clone()).collect::<Vec<_>>(), ["60", "40"]);
        // 算完了每个节点都有文字：list.sum 那个转回文字（连同上游收成一个 snippet），再跑一遍还是一样
        let infos = &ok.subs.iter().find(|(id, _)| *id == e2).unwrap().1;
        let net = net_of(&mut c.g, e2);
        let sum = net.nodes.iter().find(|n| n.kind.name == "list.sum").unwrap().id;
        net.node_mut(sum).unwrap().text = infos.iter().find(|(i, _)| *i == sum).unwrap().1.text.clone();
        let snippet = net.collapse_to_text(sum).unwrap();
        let names: Vec<&str> = net.nodes.iter().map(|n| n.kind.name.as_str()).collect();
        assert_eq!(names, ["output", "snippet"], "上游只给它用，一起收掉");
        assert_eq!(net.node(snippet).unwrap().value("expression"), "pl.col(\"l\").list.eval(pl.element() * 10).list.sum()");
        run(&c.g.compile(e2).unwrap(), e2, out).ok().unwrap();
        assert_eq!(loader::open_tables(out).unwrap()[0].source.window(0, 1)[0][2], "60");
        // 生成的节点写成文字：和默认一样的开关（inclusive=False…）不写
        let parsed = parse("pl.col(\"k\").str.split(\"\")", crate::convert::tests::types()).unwrap();
        let e3 = c.add(src, src, "expr", &[]);
        let net = net_of(&mut c.g, e3);
        let output = net.nodes.iter().find(|n| n.kind.role == "列").unwrap().id;
        let split = net.insert_parsed(&parsed, (300.0, 200.0), &[]).unwrap();
        net.connect(split, output, 0);
        let ok = run(&c.g.compile(e3).unwrap(), e3, out).ok().unwrap();
        let infos = &ok.subs.iter().find(|(id, _)| *id == e3).unwrap().1;
        assert_eq!(infos.iter().find(|(i, _)| *i == split).unwrap().1.text, "pl.col(\"k\").str.split(by=\"\")");

        // snippet 再展开成节点
        let parsed = parse(&c.g.node(e2).unwrap().net.as_ref().unwrap().node(snippet).unwrap().value("expression").to_string(), crate::convert::tests::types()).unwrap();
        let net = net_of(&mut c.g, e2);
        let root = net.expand_snippet(snippet, &parsed).unwrap();
        assert_eq!(net.node(root).unwrap().kind.name, "list.sum");
        let output = net.nodes.iter().find(|n| n.kind.role == "列").unwrap();
        assert_eq!(output.inputs, vec![Some(root)], "原来接 snippet 的改接展开出来的结果");
        run(&c.g.compile(e2).unwrap(), e2, out).ok().unwrap();
    }

    /// 选列框：引擎报上游的列和类型，写法选到哪几列；每种选择器 polars 都认。
    fn pick_through_engine() {
        let mut g = Graph::default();
        let src = g.add(&kind("code"));
        g.node_mut(src).unwrap().set(
            "code",
            "out = pl.DataFrame({\"a\": [1], \"b\": [1.5], \"名称\": [\"x\"], \"l\": [[1, 2]], \"日期\": [None]},              schema_overrides={\"日期\": pl.Datetime(\"ms\", \"Asia/Shanghai\")}).with_columns(arr=pl.col(\"l\").list.to_array(2))",
        );
        let steps = g.compile(src).unwrap();
        let ask = |text: &str| pick(&steps, src, text, &|| true).unwrap().unwrap();
        let got = ask("");
        let types: Vec<String> = got.columns.iter().map(|(n, t)| format!("{n} {}", short_dtype(t))).collect();
        assert_eq!(types, ["a i64", "b f64", "名称 str", "l list[i64]", "日期 datetime[ms, Asia/Shanghai]", "arr array[i64, 2]"]);
        assert_eq!(got.matched, Ok(Vec::new()));
        let matched = |text: &str| ask(text).matched;
        let names = |list: &[&str]| Ok(list.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        assert_eq!(matched("\"名称\", \"a\""), names(&["名称", "a"]), "按写的顺序");
        assert_eq!(matched("cs.numeric() - cs.by_name(\"b\")"), names(&["a"]));
        assert_eq!(matched("pl.col(\"a\", \"b\")"), names(&["a", "b"]));
        assert_eq!(matched("cs.all() - cs.by_name(\"a\") & ~cs.nested()"), names(&["b", "名称", "日期"]));
        assert_eq!(matched("cs.datetime(time_zone=\"Asia/Shanghai\")"), names(&["日期"]));
        assert_eq!(matched("cs.alpha(ascii_only=True)"), names(&["a", "b", "l", "arr"]), "汉字不算");
        assert_eq!(matched("cs.by_index(0, -1)"), names(&["a", "arr"]));
        assert_eq!(matched("pl.col(\"b\") * 2"), names(&["b"]), "表达式：用到的列");
        assert!(matched("\"没有\"").unwrap_err().contains("没有"));
        assert!(matched("cs.numeric(").is_err(), "写得不对");
        // 每种选择器（参数都填上）polars 都认：选不到列可以，报错不行
        for code in crate::pick::every_selector("a, l") {
            let got = ask(&code);
            assert!(got.matched.is_ok(), "{code}：{:?}", got.matched);
        }
        // 几段一起问（字典参数的一组一个键）：每段各是各的结果
        let multi = pick_many(&steps, src, &[String::from("\"名称\""), String::from("cs.numeric() - cs.by_name(\"b\")")], &|| true).unwrap().unwrap();
        assert_eq!(multi.groups, vec![Ok(names(&["名称"]).unwrap()), Ok(names(&["a"]).unwrap())]);
        let multi = pick_many(&steps, src, &[String::from("\"名称\""), String::from("\"没有\"")], &|| true).unwrap().unwrap();
        assert_eq!(multi.groups[0], Ok(names(&["名称"]).unwrap()));
        assert!(multi.groups[1].as_ref().err().unwrap().contains("没有"));
        // 上游出错：说得出来
        g.node_mut(src).unwrap().set("code", "out = pl.DataFrame({\"a\": [1]}).select(pl.col(\"x\"))");
        let steps = g.compile(src).unwrap();
        assert!(pick(&steps, src, "", &|| true).unwrap().is_err());
    }

    /// 精选的表达式节点：每个方式都在 with_columns 里跑一遍。
    fn curated_expr_kinds(c: &mut Cases, all: u32) {
        let options = |name: &str| -> Vec<String> {
            let t = expr_kind(name);
            t.mode_param().map_or(Vec::new(), |i| t.params[i].options.clone())
        };
        c.expr(all, "select", "col", &[("columns", "\"k\", cs.numeric()")], &[]);
        c.expr(all, "select", "col", &[("columns", "cs.numeric()")], &[]);
        c.expr(all, "with_columns", "lit", &[("value", "1"), ("dtype", "Float64")], &[]);
        c.expr(all, "with_columns", "lit", &[("value", "\"文字\"")], &[]);
        c.expr(all, "with_columns", "snippet", &[("expression", "a * 2 + b")], &["\"i\"", "\"n\""]);
        for op in options("arith") {
            c.expr(all, "with_columns", "arith", &[("op", op.as_str()), ("b", "2")], &["\"i\""]);
        }
        c.expr(all, "with_columns", "arith", &[("a", "1"), ("b", "2")], &[]);
        for op in options("compare") {
            c.expr(all, "with_columns", "compare", &[("op", op.as_str()), ("b", "2")], &["\"i\""]);
        }
        c.expr(all, "with_columns", "compare", &[("a", "\"a\""), ("b", "pl.col(\"k\")")], &[]);
        for op in options("logic") {
            c.expr(all, "with_columns", "logic", &[("op", op.as_str()), ("a", "pl.col(\"i\") > 1"), ("b", "pl.col(\"n\") > 2")], &[]);
        }
        let when = [("condition", "pl.col(\"i\") > 3"), ("then", "\"大\""), ("condition2", "pl.col(\"i\") > 1"), ("then2", "\"中\"")];
        c.expr(all, "with_columns", "when", &when, &[]);
        c.expr(all, "with_columns", "when", &[&when[..], &[("otherwise", "\"小\"")]].concat(), &[]);
        assert!(expr_kind("cast").params.iter().any(|p| p.kind == ParamKind::Dtype), "表达式级的 cast 应该是精选的（nodes/cast.py）");
        // polars 的全部类型：下拉里填进去的写法都能用，结果表查看器打得开、表头照 polars 的叫法
        for (name, dtype) in nodes::DTYPES {
            let column = match *name {
                // polars 2.0 起文字不能 cast 成日期 / 时间（要用 str.to_date…）
                "Date" | "Datetime" | "Time" => "\"ts\"",
                "Categorical" | "Enum" | "String" | "Binary" => "\"k\"",
                "List" => "\"l\"",
                "Array" => "\"a\"",
                "Struct" => "\"st\"",
                "Map" => "\"mp\"",
                // 什么都 cast 不成 Object：用 lit 造一列
                "Object" => continue,
                _ => "\"i\"",
            };
            let Some(_) = c.expr(all, "with_columns", "cast", &[("dtype", dtype), ("strict", "False")], &[column]) else { continue };
            let shown = loader::open_tables(&c.out).unwrap_or_else(|e| panic!("cast {dtype}：结果打不开 {e:?}"));
            let source = &shown[0].source;
            let expected = match *name {
                "Unknown" => "i64", // cast 成 Unknown 什么都不做
                "Decimal" => "decimal[38,2]",
                "Enum" => "enum",
                "Datetime" => "datetime[μs]",
                "Duration" => "duration[μs]",
                "List" => "list[i64]",
                "Array" => "array[i64, 2]",
                "Struct" => "struct{a: i64, b: str}",
                "Map" => "map[str, i64]",
                "Extension" => "ext[名字]",
                _ => &short_dtype(name),
            };
            // 结果是最后一列（列名后面加了 _新）
            assert_eq!(source.dtypes().last().map(String::as_str), Some(expected), "cast {dtype} 的表头");
            assert_eq!(source.window(0, 4).len(), 4);
        }
        c.expr(all, "with_columns", "lit", &[("value", "{\"a\": 1}"), ("dtype", "Object")], &[]);
        assert_eq!(loader::open_tables(&c.out).unwrap()[0].source.dtypes().last().map(String::as_str), Some("object"));
        c.expr(all, "with_columns", "lit", &[("value", "-(2 ** 100)"), ("dtype", "pl.Int128")], &[]);
        let shown = loader::open_tables(&c.out).unwrap();
        assert_eq!(shown[0].source.dtypes().last().map(String::as_str), Some("i128"));
        assert_eq!(shown[0].source.window(0, 1)[0].last().map(String::as_str), Some("-1267650600228229401496703205376"));
        for function in options("agg") {
            c.expr(all, "with_columns", "agg", &[("function", function.as_str())], &["\"n\""]);
        }
        for naming in options("output") {
            // 输出节点在 expr() 里总是接着的；这里换它自己的方式
            let e = c.add(all, all, "expr", &[("context", "select")]);
            let col = first(&mut c.g, e, "col");
            set_expr(&mut c.g, e, col, &[("columns", "\"i\", \"n\"")]);
            let output = first(&mut c.g, e, "output");
            set_expr(&mut c.g, e, output, &[("naming", naming.as_str()), ("affix", "_x")]);
            c.tried.push((String::from("output"), naming.clone()));
            let what = format!("output {naming}");
            // alias 改不了两列的名字（会重名）：这里该报错
            let ok = c.run(e, &what);
            if naming == "alias" {
                assert!(ok.is_ok() || c.failed.pop().is_some());
            }
            c.g.remove(e);
        }
        for name in ["key", "condition"] {
            c.tried.push((name.to_string(), String::new())); // 在 expr_contexts 里跑过
        }
    }

    /// 从 polars 生成的表达式节点：输入接上类型对得上的列，必填的参数给个样例值，看调用方式对不对。
    /// polars 自己报的错（类型不支持、值不合适）不算；方法不存在、参数名 / 个数不对算生成器的错。
    fn generated_expr_kinds(c: &mut Cases, all: u32) {
        let mut checked = 0;
        let mut worked = 0;
        for t in nodes::all_expr().into_iter().filter(|t| t.order >= 1000) {
            let column = match t.category.as_str() {
                "文本 str" => "\"s\"",
                "日期 dt" => "\"ts\"",
                "列表 list" => "\"l\"",
                "数组 arr" => "\"a\"",
                "结构 struct" => "\"st\"",
                "分类 cat" => "\"c\"",
                "二进制 bin" => "\"b\"",
                "映射 map" => "\"mp\"",
                "比较" | "逻辑" | "选取" | "窗口" | "聚合" => "\"i\"",
                _ => "\"n\"",
            };
            let mut set: Vec<(String, String)> = Vec::new();
            for p in t.params.iter().filter(|p| p.required && p.name != "input" && p.kind != ParamKind::Menu) {
                let sample = match p.kind {
                    // list.eval / arr.agg 这类：表达式里的 pl.element() 是列表的元素
                    _ if p.hint.contains("pl.element()") => "pl.element() > 1",
                    // 回调：用提示里的 lambda 示例
                    _ if p.hint.contains("lambda") => p.hint.split("如 ").nth(1).unwrap_or("lambda x: x"),
                    ParamKind::Int => "1",
                    ParamKind::Float => "0.5",
                    ParamKind::Text => "a",
                    ParamKind::Exprs => "pl.col(\"i\")",
                    ParamKind::Dtype => "Int64",
                    _ => "1",
                };
                set.push((p.name.clone(), sample.to_string()));
            }
            let set: Vec<(&str, &str)> = set.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
            let inputs: &[&str] = if t.param("input").is_some() { &[column] } else { &[] };
            let before = c.failed.len();
            c.expr(all, "select", &t.name, &set, inputs);
            let rejected = c.failed.len() > before;
            if rejected {
                let msg = c.failed.pop().unwrap();
                let detail = msg.split("\n    ").nth(1).unwrap_or("");
                // 生成的调用本身有毛病：方法 / 名字不存在，参数名或个数对不上
                let broken = detail.starts_with("AttributeError")
                    || detail.starts_with("NameError")
                    || detail.starts_with("SyntaxError")
                    || detail.contains("unexpected keyword argument")
                    || detail.contains("positional argument")
                    || detail.starts_with("参数“");
                if broken {
                    c.failed.push(msg);
                }
            }
            checked += 1;
            if c.failed.len() == before && !rejected {
                worked += 1;
            }
        }
        // 回调（填 lambda）真的跑得出来
        let double = [("function", "lambda x: x * 2"), ("return_dtype", "pl.Float64")];
        assert!(c.expr(all, "with_columns", "map_elements", &double, &["\"n\""]).is_some(), "map_elements");
        assert!(c.expr(all, "select", "name.map", &[("function", "lambda name: name.upper()")], &["\"k\""]).is_some(), "name.map");
        let fold = [("acc", "0"), ("function", "lambda acc, x: acc + x"), ("exprs", "pl.col(\"i\"), pl.col(\"n\")")];
        assert!(c.expr(all, "with_columns", "pl.fold", &fold, &[]).is_some(), "pl.fold");
        assert!(c.expr(all, "with_columns", "cumulative_eval", &[("expr", "pl.element().sum()")], &["\"i\""]).is_some());
        assert!(c.expr(all, "with_columns", "to_physical", &[], &["\"c\""]).is_some());
        // 其余的是样例值不合 polars 的意（如时区写成 "a"）
        assert!(checked > 400 && worked > 300, "生成的表达式节点：测了 {checked} 种，只有 {worked} 种跑出了结果");
    }
}
