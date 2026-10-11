//! 表格右键菜单里的快捷操作：在表格正显示的那个节点后面接一个建好的节点并显示——排序、统计各值的个数、
//! 只看 / 去掉等于某个值的行、去掉 / 只留某些列。数据操作照旧都在节点图里，只是不用自己一个个连。

use crate::*;
use graph::{Env, Graph};
use nodes::py_str;

/// 右键菜单的一项被点了。action：sort-asc / sort-desc / count / drop / keep / only / remove；
/// row：右键点的那一行（表头是 -1），col：那一列。
pub(crate) fn quick_action(weak: &slint::Weak<AppWindow>, state: &Arc<Mutex<App>>, action: &str, row: i32, col: i32) {
    let Some(app) = weak.upgrade() else { return };
    let result = {
        let mut st = state.lock().unwrap();
        let columns = picked_columns(&app, &st, col);
        let built = build(&mut st, action, row, col, &columns);
        if built.is_ok() {
            // 可能是从表达式网络里出来的：添加菜单换回表格级的
            push_kinds(&app, &mut st);
        }
        built
    };
    match result {
        Ok(msg) => {
            app.set_status(SharedString::from(msg));
            app.invoke_request_fit();
            graph_changed(weak, state, true);
        }
        Err(msg) => app.set_status(SharedString::from(msg)),
    }
}

/// 去掉 / 只留作用于哪些列：右键点的列在整列选择（主矩形或 Ctrl 加选的列块）里时是选中的那几列，否则就是它。
fn picked_columns(app: &AppWindow, st: &App, col: i32) -> Vec<usize> {
    let mut set: Vec<usize> = vec![];
    if app.get_sel_active() && app.get_sel_cols() && !app.get_sel_rows() {
        let (c0, c1) = (app.get_sel_c0().min(app.get_sel_c1()), app.get_sel_c0().max(app.get_sel_c1()));
        set.extend((c0.max(0)..=c1.max(0)).map(|x| x as usize));
    }
    if st.sel_extra_gen == app.get_sel_gen() {
        for &(a, b) in &st.sel_extra_cols {
            set.extend((a.max(0)..=b.max(0)).map(|x| x as usize));
        }
    }
    match usize::try_from(col) {
        Ok(c) if set.contains(&c) => {
            set.sort_unstable();
            set.dedup();
            set
        }
        _ => usize::try_from(col).into_iter().collect(),
    }
}

/// 建节点。返回状态栏里说的话，或者做不了的原因。
fn build(st: &mut App, action: &str, row: i32, col: i32, columns: &[usize]) -> Result<String, String> {
    if !st.shown_sub.is_empty() {
        return Err(String::from("表格里是表达式网络里某个节点的值：点画布空白处回到整个节点的结果再用"));
    }
    // 表格显示的是哪个节点的结果；显示的是文件本身时是源节点
    let base = (st.shown.or_else(|| st.graph.primary().map(|n| n.id)))
        .filter(|id| st.graph.node(*id).is_some())
        .ok_or("还没有打开文件")?;
    let source = st.sheets.get(st.current).map(|s| s.source.clone()).ok_or("表格是空的")?;
    let headers = source.headers().to_vec();
    let column = usize::try_from(col).ok().and_then(|c| headers.get(c)).cloned().ok_or("没有这一列")?;
    let dtype = column_types(st, &*source).get(col as usize).cloned().unwrap_or_default();
    // 在表达式网络里：新节点建在表格级的图上，先回到外面
    st.inside.clear();
    st.graph.select_only(Some(base));
    match action {
        "sort-asc" | "sort-desc" => {
            let desc = action == "sort-desc";
            sort(&mut st.graph, &column, desc)?;
            Ok(format!("按“{column}”从{}排序", if desc { "大到小" } else { "小到大" }))
        }
        "count" => {
            count(&mut st.graph, &column)?;
            Ok(format!("统计“{column}”各值的个数"))
        }
        "drop" | "keep" => {
            let names: Vec<&str> = columns.iter().filter_map(|&c| headers.get(c)).map(String::as_str).collect();
            select(&mut st.graph, &names, action == "drop")?;
            let list = names.iter().map(|n| format!("“{n}”")).collect::<Vec<_>>().join("、");
            Ok(format!("{}{list}", if action == "drop" { "去掉列 " } else { "只留列 " }))
        }
        "only" | "remove" => {
            let row = usize::try_from(row).map_err(|_| "没有点在格子上")?;
            let value = (source.window(row, 1).first().and_then(|r| r.get(col as usize)).cloned()).unwrap_or_default();
            let test = value_test(&dtype, &value).ok_or_else(|| format!("“{column}”是 {dtype} 列，不能按值筛选"))?;
            filter(&mut st.graph, &column, &test, action == "remove")?;
            let shown = if test == Test::Null { String::from("空值") } else { format!("“{value}”") };
            Ok(format!("{}“{column}”是 {shown} 的行", if action == "remove" { "去掉" } else { "只看" }))
        }
        _ => Err(format!("不认识的操作 {action}")),
    }
}

/// 按值筛选时怎么比：等于（compare ==，值写成 polars 的字面量）、浮点数差不多相等（is_close：表格里显示的是
/// 取整到 15 位有效数字的，和存着的 1234.5600000000002 不完全相等）、是空值（is_null）。
#[derive(Debug, PartialEq)]
enum Test {
    Equal(String),
    Close(String),
    Null,
}

/// 表格里的一格（显示的文字）在这一列的类型下怎么比。列表、结构、带时区的时间这些不好比，返回 None。
fn value_test(dtype: &str, value: &str) -> Option<Test> {
    let text = value.trim();
    // 节点结果里的空值显示成设置里的文字（默认照 polars 写 null）；查看器自己读的 CSV、Excel 里空格子 polars 读成空值
    if text.is_empty() || text == "NULL" || text == crate::loader::null_text() {
        return Some(Test::Null);
    }
    let base = dtype.split(['[', '{']).next().unwrap_or("");
    // 开了千位分隔符时，格子里是 1,234.56 这种，比的时候去掉分隔
    let number = || {
        let t = if crate::loader::thousands_enabled() { text.replace(',', "") } else { String::from(text) };
        t.parse::<f64>().ok().filter(|v| v.is_finite()).map(|_| t)
    };
    match base {
        "f32" | "f64" | "decimal" => number().map(Test::Close),
        b if b.starts_with('i') || b.starts_with('u') => number().map(Test::Equal),
        "bool" => match text.to_ascii_lowercase().as_str() {
            "true" => Some(Test::Equal(String::from("True"))),
            "false" => Some(Test::Equal(String::from("False"))),
            _ => None,
        },
        "date" => date_parts(text).map(|(y, m, d)| Test::Equal(format!("pl.date({y}, {m}, {d})"))),
        // 带时区的写成 2024-01-01 00:00:00+08:00，按文字比对不上：不做
        "datetime" if !dtype.contains(',') => Some(Test::Equal(format!("pl.lit({}).str.to_datetime()", py_str(text)))),
        "str" | "cat" | "enum" => Some(Test::Equal(py_str(value))),
        // 还不知道类型（查看器自己读的 CSV、Excel，引擎还没推出来）：像数字的当数字，否则当文字
        "" => Some(match text.parse::<i64>() {
            Ok(_) => Test::Equal(text.to_string()),
            Err(_) => match number() {
                Some(t) => Test::Close(t),
                None => Test::Equal(py_str(value)),
            },
        }),
        _ => None,
    }
}

/// 2023-07-15 → (2023, 7, 15)。
fn date_parts(text: &str) -> Option<(i32, u32, u32)> {
    let mut parts = text.splitn(3, '-');
    let y = parts.next()?.parse().ok()?;
    let m = parts.next()?.parse().ok()?;
    let d = parts.next()?.parse().ok()?;
    Some((y, m, d))
}

/// 在选中的节点后面接一个 expr 节点，上下文是 context，里面主网络是“col（填这一列）→ 上下文要的输出”。
fn add_expr(g: &mut Graph, context: &str, column: &str, name: &str) -> Result<u32, String> {
    let kind = nodes::find("expr").ok_or("没有 expr 节点")?;
    let id = g.add(&kind);
    let node = g.node_mut(id).expect("刚建的");
    node.set("context", context);
    node.net = Some(Box::new(Graph::new_net(node.env_roles(Env::Main), Some(column), 0)));
    node.ensure_outputs();
    // 名字重了就用默认的（expr1…）
    let _ = g.rename(id, name);
    Ok(id)
}

/// expr 节点 id 里 env 那张网络里第一个这种类型的节点。
fn net_node(g: &mut Graph, id: u32, env: Env, kind: &str) -> Option<u32> {
    let net = g.node(id)?.net_in(env)?;
    net.nodes.iter().find(|n| n.kind.name == kind).map(|n| n.id)
}

fn sort(g: &mut Graph, column: &str, desc: bool) -> Result<u32, String> {
    let id = add_expr(g, "sort", column, &format!("排序_{column}"))?;
    if desc {
        let key = net_node(g, id, Env::Main, "sort_key").ok_or("排序键没建上")?;
        let net = g.node_mut(id).and_then(|n| n.net_in_mut(Env::Main)).expect("刚建的");
        net.node_mut(key).expect("刚找到").set("order", "从大到小");
    }
    Ok(id)
}

/// 统计各值的个数：group_by 这一列（分组后 len），再按个数从大到小排。
fn count(g: &mut Graph, column: &str) -> Result<u32, String> {
    let kind = nodes::find("expr").ok_or("没有 expr 节点")?;
    let id = g.add(&kind);
    let node = g.node_mut(id).expect("刚建的");
    node.set("context", "group_by");
    node.set("then", "len");
    node.set("len_name", "个数");
    // 用不到聚合那张网络（分组后是 len）：空着，免得里面的输出列被提醒“用不到”
    node.net = Some(Box::new(Graph::new_net(&[], None, 0)));
    node.key_net = Some(Box::new(Graph::new_net(node.env_roles(Env::Keys), Some(column), Env::Keys.base())));
    node.ensure_outputs();
    let _ = g.rename(id, &format!("统计_{column}"));
    // 按个数从大到小
    let sorted = sort(g, "个数", true)?;
    let _ = g.rename(sorted, &format!("统计_{column}_排序"));
    Ok(sorted)
}

/// 去掉（drop）/ 只留（select）这些列。
fn select(g: &mut Graph, columns: &[&str], drop: bool) -> Result<u32, String> {
    let kind = nodes::find("select").ok_or("没有 select 节点")?;
    let id = g.add(&kind);
    let node = g.node_mut(id).expect("刚建的");
    node.set("mode", if drop { "drop" } else { "select" });
    node.set("columns", &columns.iter().map(|c| py_str(c)).collect::<Vec<_>>().join(", "));
    let _ = g.rename(id, if drop { "去掉列" } else { "选列" });
    Ok(id)
}

/// 只看（filter）/ 去掉（remove）这一列满足 test 的行：col →（compare / is_close / is_null）→ 条件。
fn filter(g: &mut Graph, column: &str, test: &Test, remove: bool) -> Result<u32, String> {
    let id = add_expr(g, "filter", column, &format!("筛选_{column}"))?;
    if remove {
        g.node_mut(id).expect("刚建的").set("keep", "去掉满足的");
    }
    let col = net_node(g, id, Env::Main, "col").ok_or("列节点没建上")?;
    let cond = net_node(g, id, Env::Main, "condition").ok_or("条件节点没建上")?;
    let (kind, params): (&str, Vec<(&str, &str)>) = match test {
        Test::Equal(v) => ("compare", vec![("op", "=="), ("b", v)]),
        Test::Close(v) => ("is_close", vec![("other", v)]),
        Test::Null => ("is_null", vec![]),
    };
    let kind = nodes::find_expr(kind).ok_or_else(|| format!("没有 {kind} 节点"))?;
    let net = g.node_mut(id).and_then(|n| n.net_in_mut(Env::Main)).expect("刚建的");
    net.selected = None;
    let check = net.add_at(&kind, Some((0.0, 0.0)));
    for (name, value) in params {
        net.node_mut(check).expect("刚建的").set(name, value);
    }
    net.connect(col, check, 0);
    net.connect(check, cond, 0);
    let ids: Vec<u32> = net.nodes.iter().map(|n| n.id).collect();
    net.layout(&ids);
    net.select_only(None);
    Ok(id)
}

/// 引擎测试用：在 base 后面各建一个快捷操作的节点，返回 (说明, 节点)。
#[cfg(test)]
pub(crate) fn every_action(g: &mut Graph, base: u32) -> Vec<(&'static str, u32)> {
    type Build = fn(&mut Graph) -> Result<u32, String>;
    let cases: [(&'static str, Build); 7] = [
        ("sort", |g| sort(g, "收入", true)),
        ("count", |g| count(g, "城市")),
        ("only", |g| filter(g, "城市", &Test::Equal(py_str("北京")), false)),
        ("remove", |g| filter(g, "城市", &Test::Equal(py_str("北京")), true)),
        ("close", |g| filter(g, "收入", &Test::Close(String::from("3476.04")), false)),
        ("null", |g| filter(g, "城市", &Test::Null, false)),
        ("drop", |g| select(g, &["姓名", "收入"], true)),
    ];
    cases
        .into_iter()
        .map(|(what, build)| {
            g.select_only(Some(base));
            (what, build(g).unwrap())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn graph() -> Graph {
        let mut g = Graph::for_file(Path::new("testdata/people.csv"), "");
        g.nodes[0].columns = vec![String::from("姓名"), String::from("城市")];
        g
    }

    #[test]
    fn values_become_literals() {
        assert_eq!(value_test("str", "北京"), Some(Test::Equal(String::from("\"北京\""))));
        assert_eq!(value_test("i64", "30"), Some(Test::Equal(String::from("30"))));
        assert_eq!(value_test("f64", "1234.56"), Some(Test::Close(String::from("1234.56"))));
        assert_eq!(value_test("date", "2023-07-15"), Some(Test::Equal(String::from("pl.date(2023, 7, 15)"))));
        assert_eq!(value_test("bool", "TRUE"), Some(Test::Equal(String::from("True"))));
        assert_eq!(value_test("i64", "NULL"), Some(Test::Null));
        assert_eq!(value_test("", ""), Some(Test::Null));
        assert_eq!(value_test("", "12"), Some(Test::Equal(String::from("12"))));
        assert_eq!(value_test("", "上海"), Some(Test::Equal(String::from("\"上海\""))));
        assert_eq!(value_test("list[i64]", "[1, 2]"), None);
        assert_eq!(value_test("datetime[μs, Asia/Shanghai]", "2024-01-01 00:00:00+08:00"), None);
    }

    #[test]
    fn quick_nodes_are_wired() {
        let mut g = graph();
        g.select_only(Some(1));
        let id = filter(&mut g, "城市", &Test::Equal(String::from("\"北京\"")), true).unwrap();
        let node = g.node(id).unwrap();
        assert_eq!((node.value("context"), node.value("keep"), node.title.as_str()), ("filter", "去掉满足的", "筛选_城市"));
        assert_eq!(node.inputs, [Some(1)]);
        let net = node.net.as_ref().unwrap();
        let names: Vec<&str> = net.nodes.iter().map(|n| n.kind.name.as_str()).collect();
        assert_eq!(names, ["col", "condition", "compare"]);
        let (col, cond, cmp) = (&net.nodes[0], &net.nodes[1], &net.nodes[2]);
        assert_eq!(col.value("columns"), "\"城市\"");
        assert_eq!((cmp.inputs[0], cmp.value("op"), cmp.value("b")), (Some(col.id), "==", "\"北京\""));
        assert_eq!(cond.inputs[0], Some(cmp.id));
        assert_eq!(g.display, Some(id));

        g.select_only(Some(1));
        let sorted = count(&mut g, "城市").unwrap();
        let counted = g.node(sorted).unwrap().inputs[0].unwrap();
        let group = g.node(counted).unwrap();
        assert_eq!((group.value("context"), group.value("then"), group.title.as_str()), ("group_by", "len", "统计_城市"));
        assert_eq!(group.key_net.as_ref().unwrap().nodes[0].value("columns"), "\"城市\"");
        assert!(group.net.as_ref().unwrap().nodes.is_empty());
        let key = &g.node(sorted).unwrap().net.as_ref().unwrap().nodes;
        assert_eq!((key[0].value("columns"), key[1].value("order")), ("\"个数\"", "从大到小"));

        g.select_only(Some(1));
        let s = select(&mut g, &["姓名", "城市"], true).unwrap();
        assert_eq!((g.node(s).unwrap().value("mode"), g.node(s).unwrap().value("columns")), ("drop", "\"姓名\", \"城市\""));
    }
}
