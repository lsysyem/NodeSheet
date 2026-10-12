# 节点执行引擎：常驻进程，stdin/stdout 各一行一个 JSON。
#
# 请求 {"steps": [步骤…], "display": id, "out": 路径}：执行 display 节点及其上游，结果写成 parquet。
#   步骤 = {"id", "code", "params": {名字: [类型, 填的内容, 标签, 默认值]}, "inputs": [id|null], "stamp"}
#   expr 节点的步骤另有 "net": [表达式节点的步骤…]（按依赖排好；group_by 的分组键 / 聚合 / 筛选组三张网络接成一张，id 不重）；
#   表达式节点的步骤另有 "ports": {参数名: 上游 id}、"role" / "order"（输出节点）、"error"（还不能算的原因）、
#   "each"（group_by 里在每组里算的：聚合、筛选组两张网络里的节点），
#   里面还有网络的（eval）也有 "net"。网络里出错的节点用路径报（"sub": [网络里的节点, 它里面的节点…]）。
# 回复里 "subs" 是各 expr 节点网络里每个节点的类型和文字（里面的网络在 "net" 里），"texts" 是 expr 节点整个写成文字。
# 请求里带 "action": 名字 时不写 out：display 节点带着 action 再执行一次（节点面板里的按钮，如 write 的“写出”），不走缓存。
# 请求里带 "preview": {"node": expr 节点, "sub": 里面的节点（或路径）} 时，写出的是那个表达式节点的值（连同它用到的列）；
#   filter 里预览的是真假值时回复带 "note"（满足几行），选中的是“条件”输出时写出的是只按它筛过的表。
# 请求里带 "schema": true 时什么都不写：只推出 display 节点的列名和类型（查看器自己显示源文件时，表头的类型从这来）。
# 请求里带 "explain": "plain" | "tree" 时也不写：回复的 "plan" 是 display 节点的查询计划 {"optimized", "raw"}（LazyFrame.explain）。
# 请求 {"catalog": true} 返回代码补全用的 polars API 清单。
# 请求 {"doc": ["DataFrame.unique", …]} 返回帮助窗口用的 polars 文档（见 api_doc）。
# 请求 {"parse": 文字, "types": {类型名: [[参数, 类型, 是不是端口], …]}} 把 polars 表达式拆成节点（见 parse_text）。
#
# 节点的代码在这些名字的环境里执行：
#   pl, cs          polars 和 polars.selectors
#   df, df2 ~ df4   第 1 ~ 4 个输入（LazyFrame，没连的是 None）；inputs 是它们的列表
#   ch("名字")      取这个节点的参数，已经按参数类型变好了（见 convert）；表达式节点的端口参数连了线时是上游的表达式
#   action          点了哪个按钮；平时是 None
#   outputs("角色")  expr 节点：网络里这种输出节点的值（列 / 键 / 条件 / 排序），按画布上从上到下
#   as_exprs, map_columns, opt   见下面
# 结果取变量 out，没有则取最后一个表达式的值；表格级是 DataFrame / LazyFrame，表达式级是表达式。
import ast
import contextlib
import io
import json
import os
import sys
import time

# 协议独占原始 stdout；fd 1 改指向空设备，用户代码和原生库的输出不会弄坏协议
proto = os.fdopen(os.dup(sys.stdout.fileno()), "w", encoding="utf-8", newline="\n")
os.dup2(os.open(os.devnull, os.O_WRONLY), 1)

CACHE_KEEP = 8  # 最近多少次请求用过的节点结果保留在缓存里


def send(obj):
    proto.write(json.dumps(obj, ensure_ascii=False) + "\n")
    proto.flush()


def short(e):
    text = f"{type(e).__name__}: {e}".strip()
    # polars 的报错后面常跟整段执行计划，只留前面有用的部分
    text = text.split("\n\nResolved plan until failure")[0]
    return text[:800]


class NodeError(Exception):
    def __init__(self, node, msg, sub=None):
        super().__init__(msg)
        self.node = node
        self.msg = msg
        self.sub = sub


class ParamError(Exception):
    """参数填的内容变不成它的类型。"""


class SubError(Exception):
    """表达式网络里的某个节点算不出来。sub 是它的路径：[网络里的节点, 它里面网络的节点…]。"""

    def __init__(self, sub, msg):
        super().__init__(msg)
        self.sub = sub
        self.msg = msg


# ---- 筛选节点（filter）的条件：界面搭出来的条件树（JSON）→ polars 表达式。
# JSON：{"join": "and" | "or", "items": [条件或分组…]}，分组是同样形状、带 items 的项，
# 条件是 {"col", "op", "value", "type"?}（value 是界面里填的文字，type 是值的类型，不写按文字算）。


def cond_lit(pl, text, kind):
    """条件里的值 → 字面量：数字 / 真假 / 日期 / 时间 / 日期时间 / 文字。"""
    text = text.strip()
    if kind == "number":
        try:
            return int(text)
        except ValueError:
            return float(text)  # 不是整数时让 polars 去报；空值不在这里管
    if kind == "bool":
        return text.lower() in ("true", "1", "yes", "是", "真")
    if kind == "date":
        return pl.lit(text).str.to_date()
    if kind == "time":
        return pl.lit(text).str.to_time()
    if kind == "datetime":
        return pl.lit(text).str.to_datetime()
    return text


def cond_values(pl, c, count=None):
    """条件里的值：是其中一个 → 一串；介于 → 两个（都是逗号分开）。"""
    text = c.get("value") or ""
    kind = c.get("type") or "text"
    parts = [p.strip() for p in text.split(",") if p.strip()]
    if not parts:
        raise ValueError("有一个条件还没填值")
    if count is not None and len(parts) != count:
        raise ValueError(f"“{text}”要填 {count} 个值，用逗号分开")
    return [cond_lit(pl, p, kind) for p in parts]


def cond_test(pl, c):
    """一个条件 → 真假表达式。"""
    name = (c.get("col") or "").strip()
    if not name:
        raise ValueError("有一个条件还没选列")
    col = pl.col(name)
    op = c.get("op") or "eq"
    if op == "is_null":
        return col.is_null()
    if op == "not_null":
        return col.is_not_null()
    if op == "is_true":
        return col == True
    if op == "is_false":
        return col == False
    text = c.get("value") or ""
    if op == "contains":
        return col.str.contains(text, literal=True)
    if op == "ncontains":
        return ~col.str.contains(text, literal=True)
    if op == "starts":
        return col.str.starts_with(text)
    if op == "ends":
        return col.str.ends_with(text)
    if op == "regex":
        return col.str.contains(text)
    if op == "in":
        return col.is_in(cond_values(pl, c))
    if op == "between":
        low, high = cond_values(pl, c, 2)
        return col.is_between(low, high)
    value = cond_lit(pl, text, c.get("type") or "text")
    compare = {"eq": col == value, "ne": col != value, "gt": col > value,
               "ge": col >= value, "lt": col < value, "le": col <= value}
    if op not in compare:
        raise ValueError(f"不认识的操作 {op}")
    return compare[op]


def cond_group(pl, group):
    """一组条件：按 join 合起来（or 用 pl.any_horizontal，和 expr 节点的“任一满足”一致）。"""
    exprs = [e for e in (cond_item(pl, it) for it in group.get("items") or []) if e is not None]
    if not exprs:
        return None
    if len(exprs) == 1:
        return exprs[0]
    if group.get("join") == "or":
        return pl.any_horizontal(exprs)
    import functools
    import operator

    return functools.reduce(operator.and_, exprs)


def cond_item(pl, item):
    """条件树里的一项：有 items 的是分组。"""
    return cond_group(pl, item) if "items" in item else cond_test(pl, item)


def cond_expr(pl, raw):
    """筛选节点的条件（JSON 文字）→ 表达式；没有条件时 None（所有行都留着）。"""
    text = (raw or "").strip()
    if not text:
        return None
    import json

    return cond_group(pl, json.loads(text))


def convert(kind, raw, env):
    """参数填的内容（字符串）→ ch() 返回的值。留空：数字 / 表达式是 None，列表 / 字典是空的。"""
    text = raw.strip()
    if kind in ("raw", "code"):
        return raw
    if kind == "toggle":
        return text == "True"
    if kind == "int":
        return int(text) if text else None
    if kind == "float":
        return float(text) if text else None
    if kind == "expr":
        return eval(text, env) if text else None
    if kind in ("exprs", "columns"):
        return eval("[" + text + "\n]", env) if text else []
    if kind == "dict":
        return eval("{" + text + "\n}", env) if text else {}
    if kind == "kwargs":
        return eval("dict(" + text + "\n)", env) if text else {}
    if kind == "dtype":
        return eval(text, dtype_env(env)) if text else None
    if kind == "conditions":
        # 筛选节点：界面搭的条件树 → 表达式（没有 polars 的环境里也能算出这份 JSON 的形状）
        return cond_expr(env.get("pl") or __import__("polars"), raw)
    return text  # text / menu / file / save_file / sheet


# polars 的数据类型名：类型参数里不写 pl. 也认得（Int64、List(String)、Datetime("ms")）
DTYPE_NAMES = frozenset(
    "Boolean Int8 Int16 Int32 Int64 Int128 UInt8 UInt16 UInt32 UInt64 UInt128 Float16 Float32 Float64 Decimal "
    "String Utf8 Binary Categorical Enum Date Time Datetime Duration List Array Struct Field Map Null Object "
    "Extension BaseExtension Unknown".split()
)


def dtype_env(env):
    """类型参数求值的环境：参数的环境（pl、cs…）加上不带 pl. 的类型名。"""
    pl = env["pl"]
    return {**{n: getattr(pl, n) for n in DTYPE_NAMES if hasattr(pl, n)}, **env}


def make_env(pl):
    """参数里的表达式和节点代码共用的名字。"""

    def as_exprs(items):
        """列名 / 表达式 / 选择器（或它们的列表）→ 表达式列表。"""
        if not isinstance(items, (list, tuple)):
            items = [items]
        return [pl.col(item) if isinstance(item, str) else item for item in items]

    def map_columns(df, columns, fn, suffix=""):
        """对 columns 里的每一列做 fn（表达式 → 表达式）；suffix 留空 = 覆盖原列，否则结果放进“原列名 + suffix”。"""
        results = [fn(e) for e in as_exprs(columns)]
        if suffix:
            results = [e.name.suffix(suffix) for e in results]
        return df.with_columns(results)

    def opt(**kwargs):
        """去掉没填的关键字参数（None / 空文字 / 空列表），让 polars 用它自己的默认值。"""
        return {k: v for k, v in kwargs.items() if v is not None and not (isinstance(v, (str, list)) and len(v) == 0)}

    return {"pl": pl, "cs": pl.selectors, "as_exprs": as_exprs, "map_columns": map_columns, "opt": opt}


def exec_node(pl, step, inputs, action=None, ports=None, extra=None, sym=None):
    """执行一个节点的代码，返回 out。ports：连了线的端口参数 → 上游的值（表达式级）。
    sym：网络 → 文字时用替身（见 Sym）代替 polars，参数按 sym_convert 变，结果是拼出来的代码文字。"""
    env = make_env(pl) if sym is None else sym()
    params = step.get("params") or {}
    ports = ports or {}
    values = {}

    def ch(name):
        if name in ports:
            return ports[name]
        if name not in params:
            raise ParamError(f"这个节点没有叫 {name} 的参数")
        if name not in values:
            kind, raw, label = params[name][:3]
            try:
                values[name] = convert(kind, raw, make_env(pl)) if sym is None else sym_convert(kind, raw)
            except Exception as e:
                raise ParamError(f"参数“{label}”填的内容不对：{short(e)}") from None
        return values[name]

    if sym is not None:
        # 写成文字：和默认值一样的关键字参数也不写（polars 自己的默认和定义里的一样）
        def same_as_default(name):
            p = params.get(name)
            return p is not None and len(p) > 3 and p[1].strip() == p[3].strip()

        loose = env["opt"]
        env["opt"] = lambda **kw: loose(**{k: v for k, v in kw.items() if not same_as_default(k)})
    padded = list(inputs) + [None] * (4 - len(inputs))
    env.update(df=padded[0], df2=padded[1], df3=padded[2], df4=padded[3], inputs=list(inputs), ch=ch, action=action)
    env.update(extra or {})
    tree = ast.parse(step["code"], mode="exec")
    last = None
    if tree.body and isinstance(tree.body[-1], ast.Expr):
        last = ast.Expression(tree.body.pop().value)
    exec(compile(tree, "<node>", "exec"), env)
    value = eval(compile(last, "<node>", "eval"), env) if last is not None else None
    return env.get("out", value)


def run_net(pl, net, inputs, sym=None):
    """表达式网络：按顺序算每个节点的值（多半是一个表达式）。返回 (值, 出错, 里面)，都按节点 id：
    出错是 (源头的路径, 原因)：一个节点出错不影响别的，用到它的节点跟着算不出来，源头还是它；
    里面是节点里面还有网络（eval）时那张网络的 (值, 出错, 里面)。"""
    values, errors, inner = {}, {}, {}
    for s in net:
        sid = s["id"]
        ports = s.get("ports") or {}
        broken = [up for up in ports.values() if up in errors or up not in values]
        if s.get("error"):
            errors[sid] = ((sid,), s["error"])
        elif broken:
            up = broken[0]
            errors[sid] = errors.get(up, ((up,), "这个节点不在网络里"))
        else:
            extra = {}
            if s.get("net"):
                # 里面还有网络：先算里面的，节点代码里用 outputs("结果") 取
                inner[sid] = run_net(pl, s["net"], inputs, sym)
                extra["outputs"] = net_outputs(s["net"], *inner[sid][:2])
            try:
                out = exec_node(pl, s, inputs, ports={k: values[v] for k, v in ports.items()}, extra=extra, sym=sym)
                values[sid] = out
            except SubError as e:
                errors[sid] = ((sid, *e.sub), e.msg)
            except ParamError as e:
                errors[sid] = ((sid,), str(e))
            except Exception as e:
                errors[sid] = ((sid,), short(e))
    return values, errors, inner


def net_outputs(net, values, errors):
    """outputs("角色")：网络里这种输出节点的值，按 order 排。有一个算不出来就报它的源头。"""

    def outputs(role):
        picked = sorted((s for s in net if s.get("role") == role), key=lambda s: s.get("order", 0))
        for s in picked:
            if s["id"] in errors:
                origin, msg = errors[s["id"]]
                raise SubError(list(origin), msg)
        return [values[s["id"]] for s in picked]

    return outputs


def run_code(pl, step, inputs, action=None, report=None):
    """执行表格级节点，返回 LazyFrame。expr 节点先算里面的网络；report 收下网络里各节点的值和错误。"""
    extra = {}
    if "net" in step:
        values, errors, inner = run_net(pl, step["net"], inputs)
        if report is not None:
            report.update(values=values, errors=errors, inner=inner)
        extra["outputs"] = net_outputs(step["net"], values, errors)
    out = exec_node(pl, step, inputs, action, extra=extra)
    if action is not None:
        return out  # 按钮：做了事就行，不要求有结果
    if isinstance(out, pl.DataFrame):
        out = out.lazy()
    if not isinstance(out, pl.LazyFrame):
        raise TypeError("节点需要得到 DataFrame 或 LazyFrame：把结果赋给 out，或让最后一行是结果表达式")
    return out


def catalog(pl):
    """代码补全用的 API 清单：{组: [[名字, 签名, 文档首行]]}，取自本机装的这版 polars。"""
    import inspect

    def members(obj):
        out = []
        for name in dir(obj):
            if name.startswith("_") or name in ("plot", "style"):
                continue
            try:
                m = getattr(obj, name)
            except Exception:
                continue
            sig = doc = ""
            if callable(m):
                try:
                    # 只留参数名和默认值：类型注解太长，补全列表里看不清
                    s = inspect.signature(m)
                    params = [p.replace(annotation=inspect.Parameter.empty) for p in s.parameters.values()]
                    sig = str(s.replace(parameters=params, return_annotation=inspect.Signature.empty))
                except Exception:
                    sig = "(...)"
                doc = (inspect.getdoc(m) or "").strip().split("\n")[0]
            out.append([name, sig[:1500], doc[:200]])
        return out

    lf = pl.LazyFrame({"a": [1]})
    e = pl.col("a")
    groups = {
        "pl": members(pl),
        "LazyFrame": members(lf),
        "DataFrame": members(lf.collect()),
        "GroupBy": members(lf.group_by("a")),
        "Expr": members(e),
    }
    for ns in ("str", "dt", "list", "arr", "cat", "struct", "map", "name", "bin"):
        try:
            groups[ns] = members(getattr(e, ns))
        except Exception:
            pass
    return groups


def rst_prose(lines):
    """numpydoc 里的一段说明 → 段落列表：去掉 RST 标记，段内的行接起来（列表项另起一行）。"""
    import re

    out, para, skip = [], [], None
    for raw in lines + [""]:
        line = raw.strip()
        indent = len(raw) - len(raw.lstrip())
        if skip is not None:
            if line and indent <= skip:
                skip = None
            else:
                continue
        m = re.match(r"\.\. ([\w-]+)::\s*(.*)", line)
        if m:
            kind, arg = m.groups()
            if kind in ("warning", "note", "deprecated"):
                label = {"warning": "Warning:", "note": "Note:", "deprecated": "Deprecated"}[kind]
                para.append(f"{label} {arg}".strip())
            else:
                skip = indent  # versionadded / engine-support 之类：连同缩进的下文都不要
            continue
        if not line:
            if para:
                out.append(" ".join(para).replace("\n ", "\n"))
                para = []
            continue
        if line.startswith(("* ", "- ")) and para:
            para[-1] += "\n"
        para.append(line)
    clean = []
    for p in out:
        p = re.sub(r"`([^`<]+?)\s*<[^>]+>`_", r"\1", p)  # 链接只留文字
        p = re.sub(r":[\w:]+:`~?([^`]+)`", r"\1", p)
        p = p.replace("``", "`")
        p = re.sub(r"`([^`]+)`", r"\1", p)
        clean.append(p)
    return clean


def api_doc(pl, path):
    """polars 的一个 API（DataFrame.unique、Expr.str.pad_start、pl.concat_str）的文档：
    {sig, summary: [段落], params: [[名字, 说明]], sections: [[标题, 文字]]}（See Also / Notes / Examples…）。"""
    import inspect
    import re

    roots = {
        "pl": pl,
        "cs": __import__("polars.selectors").selectors,
        "DataFrame": pl.DataFrame,
        "LazyFrame": pl.LazyFrame,
        "Series": pl.Series,
        "Expr": pl.col("a"),
        "GroupBy": pl.DataFrame({"a": [1]}).group_by("a"),
    }
    head, *rest = path.split(".")
    if head not in roots:
        return {"error": f"不认识的 {head}"}
    obj = roots[head]
    try:
        for name in rest:
            obj = getattr(obj, name)
    except AttributeError:
        return {"error": f"这版 polars（{pl.__version__}）没有 {path}"}
    doc = inspect.getdoc(obj) or ""
    sig = ""
    try:
        s = inspect.signature(obj)
        params = [p.replace(annotation=inspect.Parameter.empty) for p in s.parameters.values() if p.name != "self"]
        sig = rest[-1] + str(s.replace(parameters=params, return_annotation=inspect.Signature.empty))
    except (TypeError, ValueError):
        pass
    # 按 “标题\n-----” 切节
    lines = doc.splitlines()
    sections, title = {"": []}, ""
    i = 0
    while i < len(lines):
        if i + 1 < len(lines) and lines[i].strip() and re.fullmatch(r"-{3,}", lines[i + 1].strip()):
            title = lines[i].strip()
            sections[title] = []
            i += 2
            continue
        sections[title].append(lines[i])
        i += 1
    params = []
    for line in sections.pop("Parameters", []):
        if line and not line[0].isspace():
            params.append([re.match(r"[*\w]*", line).group(), []])
        elif params:
            params[-1][1].append(line[4:] if line.startswith("    ") else line.strip())
    params = [[name, "\n".join(rst_prose(desc))] for name, desc in params]
    out = []
    for title, body in sections.items():
        if title in ("", "Returns", "Raises", "Other Parameters"):
            continue
        if title == "Examples":
            text = "\n".join(body).strip("\n")
            out.append([title, re.sub(r"\s*# doctest:.*", "", text)])
        elif title == "See Also":
            names = [line.split(":")[0].strip() for line in body if line.strip() and not line[0].isspace()]
            out.append([title, ", ".join(names)])
        else:
            out.append([title, "\n\n".join(rst_prose(body))])
    return {"sig": sig, "summary": rst_prose(sections[""]), "params": params, "sections": out}


def as_expr(pl, value):
    """表达式节点的值 → 表达式：排序键是 (表达式, 降序, 空值最后)；普通的 Python 值当常量。"""
    if isinstance(value, tuple) and value:
        value = value[0]
    if isinstance(value, pl.Expr):
        return value
    return pl.lit(value)


def agg_ids(step):
    """group_by 上下文里在每组里算的节点：聚合、筛选组（having）两张网络里的（标着 each）。"""
    return {s["id"] for s in step["net"] if s.get("each")}


def input_of(pl, s, values):
    """eval 节点的“输入”（列表列的表达式）：连了线是上游的值，没连是填的。"""
    ports = s.get("ports") or {}
    if "input" in ports:
        return as_expr(pl, values[ports["input"]])
    kind, raw = (s.get("params") or {})["input"][:2]
    return as_expr(pl, convert(kind, raw, make_env(pl)))


def element_wrap(s, x):
    """eval 节点里面的表达式 → 在外面算得出的表达式：放进 输入.list.eval(…)（数组用 arr.eval，结果都是列表）。"""
    method = ((s.get("params") or {}).get("method") or ["", "list.eval"])[1]
    if method.startswith("arr"):
        return lambda e: x.arr.eval(e, as_list=True)
    return lambda e: x.list.eval(e)


def compose(outer, inner):
    return inner if outer is None else (lambda e: outer(inner(e)))


def net_types(pl, step, lf, run, texts=None):
    """网络里各节点输出的类型：只用输入的表结构推导，不碰数据。run 是 run_net 的结果，texts 是同一张网络用替身算的。
    {id: {"dtype", "n", "text"?, "warn"?, "net"?} 或 {"error"}}：text 是这个节点的表达式写成文字，net 是节点里面网络的（eval）。"""
    try:
        base = pl.LazyFrame(schema=lf.collect_schema()) if lf is not None else pl.LazyFrame()
        reason = ""
    except Exception as e:
        base = None
        reason = short(e)
    params = step.get("params") or {}
    context = params["context"][1] if "context" in params else ""
    rows = ROW_ROLES.get(context)
    return types_of(pl, step["net"], base, reason, run, agg_ids(step), None, 0, texts, rows)


# 上下文里要和表逐行对齐的输出：(角色, 叫法)。值的行数和表不一样（unique / filter / head…）时用不了
ROW_ROLES = {"with_columns": ("列", "放不进 with_columns：要一个值就接聚合，单独看就换 select 上下文"),
             "filter": ("条件", "不能当 filter 的条件"), "sort": ("排序", "不能当排序键"), "group_by": ("键", "不能当分组键")}


def keeps_rows(e):
    """表达式的值和表逐行对齐（行数不变，或是一个值、会铺满每行）；看不出来时 None。只看表达式，不碰数据。
    polars 2 的 meta 分析（标着 unstable）；带 list.eval / arr.eval 的要知道列的类型，会报错 → None。"""
    try:
        return e.meta.is_length_preserving() or e.meta.is_scalar()
    except Exception:
        return None


def types_of(pl, net, base, reason, run, agg, wrap, depth, texts, rows=None):
    """net_types 的一级。wrap：这一级在 eval 里面时，把表达式包成外面算得出的样子（套了 depth 层列表，类型要剥掉 depth 层）。
    rows：这一级的上下文要求和表逐行对齐的输出 (角色, 叫法)，见 ROW_ROLES（eval 里面没有）。"""
    values, errors, inner = run
    info = {}
    for s in net:
        sid = s["id"]
        if sid in errors:
            origin, msg = errors[sid]
            info[sid] = {"error": msg if tuple(origin) == (sid,) else "接进来的节点算不出来"}
        elif base is None:
            info[sid] = {"error": reason}
        else:
            try:
                e = as_expr(pl, values[sid])
                info[sid] = type_of(pl, s, base, e, sid in agg, wrap, depth)
                if rows and s.get("role") == rows[0] and sid not in agg and keeps_rows(e) is False:
                    info[sid]["warn"] = f"行数和表不一样（如 unique、filter、head），{rows[1]}"
            except Exception as e:
                info[sid] = {"error": short(e)}
        if texts is not None and sid in texts[0]:
            text = sym_text(texts[0][sid])
            if text:
                info[sid]["text"] = text
        if sid in inner:
            # eval 节点：里面的节点在每个元素上算——类型是放进 输入.list.eval(…) 以后列表里元素的类型
            inner_base, inner_reason, inner_wrap = base, reason, None
            try:
                inner_wrap = compose(wrap, element_wrap(s, input_of(pl, s, values)))
            except Exception:
                inner_base, inner_reason = None, "eval 的输入算不出来"
            inner_texts = texts[2].get(sid) if texts is not None else None
            info[sid]["net"] = types_of(pl, s["net"], inner_base, inner_reason, inner[sid], set(), inner_wrap, depth + 1, inner_texts)
    return info


def type_of(pl, s, base, e, in_agg, wrap, depth):
    """一个节点的类型。in_agg：在 group_by 的每组里算；wrap / depth 见 types_of。"""
    warn = ""
    if wrap is not None:
        dtypes = list(base.select(wrap(e)).collect_schema().dtypes())
        for _ in range(depth):
            dtypes = [getattr(t, "inner", t) for t in dtypes]
    elif in_agg and s.get("role") == "列":
        # 输出列：结果表里那一列的类型。随便按一个常量分组就能推出来
        schema = base.group_by(pl.lit(0).alias("__组")).agg(e).collect_schema()
        dtypes = list(schema.dtypes())[1:]
        # 不是每组一个值：没聚合，polars 把每组的值收成一个列表
        if dtypes and not e.meta.is_scalar():
            warn = "没有聚合：每组得到一个列表（要一个值就接 agg 节点，如 sum / first）"
    elif in_agg:
        # 每组里的中间节点：组内是这一组的那几行，类型和在整张表上算一样（col 是 f64 不是 list[f64]，
        # 写成列表的话下游会以为要用 list.sum）。整表上算不了的才退回按组推
        try:
            dtypes = list(base.select(e).collect_schema().dtypes())
        except Exception:
            schema = base.group_by(pl.lit(0).alias("__组")).agg(e).collect_schema()
            dtypes = list(schema.dtypes())[1:]
    else:
        dtypes = list(base.select(e).collect_schema().dtypes())
    info = {"dtype": str(dtypes[0]) if dtypes else "", "n": len(dtypes)}
    if warn:
        info["warn"] = warn
    return info


def filter_note(pl, lf, e):
    """filter 里预览真假值：有几行满足、几行是空值。不是真假值（或算不出来）时是空串。"""
    try:
        if list(pl.LazyFrame(schema=lf.collect_schema()).select(e).collect_schema().dtypes()) != [pl.Boolean]:
            return ""
        true, null, total = lf.select(e.sum().alias("t"), e.null_count().alias("z"), pl.len().alias("n")).collect().row(0)
    except Exception:
        return ""
    text = f"满足 {true:,} 行（共 {total:,}）"
    return text + f"，空值 {null:,} 行" if null else text


def readable(pl, df):
    """结果表写成 parquet 前：查看器（arrow-rs）读不了的列换成文字——Int128 / UInt128（列表、结构里的也换），
    Object（parquet 写不了）。返回 (要写的表, parquet 的元信息)：元信息里是各列原来的类型，表头照它写。"""

    def fix(t):
        if t in (pl.Int128, pl.UInt128):
            return pl.String
        if isinstance(t, pl.List):
            return pl.List(fix(t.inner))
        if isinstance(t, pl.Array):
            return pl.Array(fix(t.inner), t.shape)
        if isinstance(t, pl.Struct):
            return pl.Struct({f.name: fix(f.dtype) for f in t.fields})
        if isinstance(t, pl.Map):
            return pl.Map(fix(t.key), fix(t.value))
        return t

    import warnings

    fixes = []
    for name, t in df.schema.items():
        if t == pl.Object:
            with warnings.catch_warnings():
                warnings.simplefilter("ignore")  # polars 劝换成 cast(pl.String)，可 Object 转不了
                fixes.append(pl.col(name).map_elements(str, return_dtype=pl.String, skip_nulls=True))
        elif fix(t) != t:
            fixes.append(pl.col(name).cast(fix(t)))
    meta = {"rtv_dtypes": json.dumps([str(t) for t in df.dtypes], ensure_ascii=False)}
    return (df.with_columns(fixes) if fixes else df), meta


def pick_one(pl, schema, text):
    """一段写法（逗号分隔：列名、序号、选择器、pl.col(…) 这类表达式）在这张表上选到哪几列。
    返回 {"matched": [列名，按选到的顺序]} 或 {"error": 原因}。"""
    text = text.strip()
    if not text:
        return {"matched": []}
    names = schema.names()
    try:
        items = eval("[" + text + "\n]", make_env(pl))
    except Exception as e:
        return {"error": f"写得不对：{short(e)}"}
    out = []
    try:
        for item in items:
            if isinstance(item, str):
                if item not in schema:
                    return {"error": f"表里没有“{item}”这一列"}
                found = [item]
            elif isinstance(item, int) and not isinstance(item, bool):
                if not -len(names) <= item < len(names):
                    return {"error": f"没有第 {item} 列（共 {len(names)} 列，从 0 数）"}
                found = [names[item]]
            else:
                try:
                    found = list(pl.selectors.expand_selector(schema, item, strict=False))
                except Exception:
                    if not isinstance(item, pl.Expr):
                        raise
                    found = [n for n in item.meta.root_names() if n in schema]  # pl.col("a") * 2：用到的列
            out += [n for n in found if n not in out]
    except Exception as e:
        return {"error": short(e)}
    return {"matched": out}


def pick_columns(pl, schema, text):
    """选列框：一段写法，或几段（字典参数的一组一个键）各选到哪几列。
    单段返回 {"matched": […]} 或 {"error": …}；几段返回 {"matched": [每段的结果，出错的是 {"error": …}]}。"""
    if isinstance(text, list):
        results = [pick_one(pl, schema, t) for t in text]
        return {"matched": [r["error"] if "error" in r else r["matched"] for r in results]}
    return pick_one(pl, schema, text)


def preview_frame(pl, step, lf, run, path, label, info=None):
    """表达式节点的值，连同它用到的列：选中网络里的节点时表格显示这个。
    path：[网络里的节点, eval 里面的节点…]；eval 里面的节点包成 输入.list.eval(…) 显示（每行一个列表）。
    info：filter 里预览真假值时写进 note（满足几行）；选中的是“条件”输出时表是只按它筛过的。"""
    values, errors, inner = run
    net, wrap, outer_input = step["net"], None, None
    for depth, sid in enumerate(path[:-1]):
        s = next((s for s in net if s["id"] == sid), None)
        if s is None or sid not in inner:
            raise SubError(list(path[: depth + 1]), "这个节点不在网络里")
        try:
            x = input_of(pl, s, values)
        except Exception as e:
            raise SubError(list(path[: depth + 1]), f"eval 的输入算不出来：{short(e)}")
        outer_input = x if outer_input is None else outer_input
        wrap = compose(wrap, element_wrap(s, x))
        net = s["net"]
        values, errors, inner = inner[sid]
    sub = path[-1]
    if sub in errors:
        origin, msg = errors[sub]
        raise SubError(list(path[:-1]) + list(origin), msg)
    if sub not in values:
        raise SubError(list(path), "这个节点不在网络里")
    e = as_expr(pl, values[sub])
    top = e
    if wrap is not None:
        e, top = wrap(e), outer_input
    params = step.get("params") or {}
    if wrap is None and params.get("context", [None, ""])[1] == "filter":
        note = filter_note(pl, lf, e)
        if note and info is not None:
            info["note"] = note
        role = next((s.get("role") for s in net if s["id"] == sub), "")
        if note and role == "条件":
            # “条件”输出：只按这一个条件筛（照“留下 / 去掉”），看它筛掉了哪些行
            remove = params.get("keep", [None, ""])[1] == "去掉满足的"
            return lf.remove(e) if remove else lf.filter(e)
    columns = lf.collect_schema().names()
    try:
        multi = len(pl.LazyFrame(schema=lf.collect_schema()).select(e).collect_schema()) > 1
    except Exception:
        multi = False
    shown = e.name.prefix("→ ") if multi else e.alias(f"→ {label}")
    if wrap is None and sub in agg_ids(step):
        try:
            keys = net_outputs(step["net"], values, errors)("键")
        except SubError:
            keys = []
        if keys:
            return lf.group_by(keys, maintain_order=True).agg(shown)
        return lf.select(shown)
    try:
        roots = [c for c in dict.fromkeys(top.meta.root_names()) if c in columns]
    except Exception:
        roots = []
    # 值的行数和表不一样（value_counts / unique / filter…）时放不到用到的列旁边：只显示值。
    # eval 里面的节点看外面的输入（输入.list.eval(…) 和输入一样多行）
    keeps = keeps_rows(top)
    if not roots or keeps is False:
        return lf.select(shown)
    frame = lf.select(*[pl.col(c) for c in roots], shown)
    # 看不出来（里面有 list.eval 这类要知道类型才分析得了的）：先试连同用到的列，行数对不上再只要值
    return frame if keeps else PreviewFrame(frame, lf.select(shown))


class PreviewFrame:
    """预览的表：先试连同用到的列一起；行数对不上（ShapeError）就只要值。"""

    def __init__(self, frame, plain):
        self.frame, self.plain = frame, plain

    def collect(self):
        import polars as pl

        try:
            return self.frame.collect()
        except pl.exceptions.ShapeError:
            return self.plain.collect()


# ---- 网络 → 文字：用替身代替 polars 把节点代码再跑一遍，替身记下每次调用，拼成 polars 代码


class Sym:
    """polars 的替身：取属性、调用、运算都得到新的替身，text 是拼出来的代码（pl.col("a").str.slice(0, 4)）。
    loose：最外层是运算（a + b），再往后接 .方法 或做运算时要加括号。"""

    __slots__ = ("text", "loose")

    def __init__(self, text, loose=False):
        self.text = text
        self.loose = loose

    def tight(self):
        return f"({self.text})" if self.loose else self.text

    def __getattr__(self, name):
        if name.startswith("__"):
            raise AttributeError(name)
        return Sym(f"{self.tight()}.{name}")

    def __call__(self, *args, **kwargs):
        parts = [sym_lit(a) for a in args]
        # 没填的关键字参数（None）不写：polars 的默认差不多都是 None
        parts += [f"{k}={sym_lit(v)}" for k, v in kwargs.items() if v is not None]
        return Sym(f"{self.tight()}({', '.join(parts)})")

    def __getitem__(self, key):
        return Sym(f"{self.tight()}[{sym_lit(key)}]")

    def __iter__(self):
        raise TypeError("替身不能展开")

    def __bool__(self):
        return True

    def __hash__(self):
        return id(self)

    def __instancecheck__(self, obj):
        # 节点代码里的 isinstance(x, pl.Expr) / isinstance(x, cs.Selector)：替身就算是
        return isinstance(obj, Sym)

    def __repr__(self):
        return self.text

    def __invert__(self):
        return Sym(f"~{self.tight()}")

    def __neg__(self):
        return Sym(f"-{self.tight()}")


def _binary(op, reflected=False):
    def method(self, other):
        a, b = (other, self) if reflected else (self, other)
        return Sym(f"{sym_operand(a)} {op} {sym_operand(b)}", loose=True)

    return method


for _name, _op in {
    "add": "+", "sub": "-", "mul": "*", "truediv": "/", "floordiv": "//", "mod": "%", "pow": "**",
    "and": "&", "or": "|", "xor": "^",
}.items():
    setattr(Sym, f"__{_name}__", _binary(_op))
    setattr(Sym, f"__r{_name}__", _binary(_op, reflected=True))
for _name, _op in {"eq": "==", "ne": "!=", "gt": ">", "ge": ">=", "lt": "<", "le": "<="}.items():
    setattr(Sym, f"__{_name}__", _binary(_op))


def sym_operand(v):
    return v.tight() if isinstance(v, Sym) else sym_lit(v)


def sym_lit(v):
    """Python 的值写成代码：替身是它的文字，字符串用双引号。"""
    if isinstance(v, Sym):
        return v.text
    if isinstance(v, str):
        return json.dumps(v, ensure_ascii=False)
    if isinstance(v, list):
        return "[" + ", ".join(sym_lit(x) for x in v) + "]"
    if isinstance(v, tuple):
        return "(" + ", ".join(sym_lit(x) for x in v) + ("," if len(v) == 1 else "") + ")"
    if isinstance(v, dict):
        return "{" + ", ".join(f"{sym_lit(k)}: {sym_lit(x)}" for k, x in v.items()) + "}"
    return repr(v)


def sym_text(v):
    """节点的值 → 文字；排序键是 (表达式, 降序, 空值最后)，只写表达式。算不出替身的是空串。"""
    if isinstance(v, tuple) and v:
        v = v[0]
    if isinstance(v, Sym):
        return v.text
    if isinstance(v, (str, int, float, bool)) or v is None:
        return f"pl.lit({sym_lit(v)})"
    return ""


def sym_env():
    """替身环境：pl、cs 是替身，其余和 make_env 一样（opt 去掉没填的关键字参数）。"""
    env = make_env(Sym("pl"))
    env.update(pl=Sym("pl"), cs=Sym("cs"))
    return env


def sym_value(src, node):
    """参数文字里的一段 → 替身环境里的值：字面量照原样，别的（表达式）是写着原文的替身。"""
    try:
        return ast.literal_eval(node)
    except Exception:
        pass
    text = ast.get_source_segment(src, node) or ast.unparse(node)
    tight = isinstance(node, (ast.Call, ast.Attribute, ast.Name, ast.Subscript, ast.Constant))
    return Sym(text, loose=not tight)


def dtype_text(text, prefix):
    """类型的写法加上（prefix）/ 去掉 pl.：List(Int64) ↔ pl.List(pl.Int64)。字符串里的（Struct 的字段名）不动。"""
    import tokenize

    tokens = list(tokenize.generate_tokens(io.StringIO(text).readline))
    starts = [0]
    for line in text.splitlines(keepends=True):
        starts.append(starts[-1] + len(line))

    def at(rc):
        return starts[rc[0] - 1] + rc[1]

    edits = []
    for i, t in enumerate(tokens):
        if t.type != tokenize.NAME or t.string not in DTYPE_NAMES:
            continue
        after_dot = i >= 1 and tokens[i - 1].string == "."
        if prefix and not after_dot:
            edits.append((at(t.start), at(t.start), "pl."))
        elif not prefix and after_dot and i >= 2 and tokens[i - 2].string == "pl":
            edits.append((at(tokens[i - 2].start), at(t.start), ""))
    for a, b, new in reversed(edits):
        text = text[:a] + new + text[b:]
    return text


def sym_convert(kind, raw):
    """参数填的内容 → 替身环境里 ch() 返回的值（和 convert 对应）。"""
    text = raw.strip()
    if kind == "dtype":
        return Sym(dtype_text(text, True)) if text else None
    if kind in ("expr", "exprs", "columns", "dict", "kwargs"):
        if not text:
            return {"expr": None, "dict": {}, "kwargs": {}}.get(kind, [])
        if kind == "expr":
            src = text
            return sym_value(src, ast.parse(src, mode="eval").body)
        if kind == "dict":
            src = "{" + text + "\n}"
            d = ast.parse(src, mode="eval").body
            return {sym_value(src, k): sym_value(src, v) for k, v in zip(d.keys, d.values)}
        if kind == "kwargs":
            src = "dict(" + text + "\n)"
            call = ast.parse(src, mode="eval").body
            return {k.arg: sym_value(src, k.value) for k in call.keywords}
        src = "[" + text + "\n]"
        return [sym_value(src, e) for e in ast.parse(src, mode="eval").body.elts]
    return convert(kind, raw, {})


def node_text(pl, step, run):
    """expr 节点整个写成文字：df.with_columns(…) / df.group_by(…).agg(…)…；写不出来是空串。run 是替身算的网络。"""
    values, errors, _ = run
    try:
        out = exec_node(pl, step, [Sym("df")], extra={"outputs": net_outputs(step["net"], values, errors)}, sym=sym_env)
    except Exception:
        return ""
    return out.text if isinstance(out, Sym) else ""


# ---- 文字 → 网络：用 ast 拆开 polars 表达式，每一步变成一个表达式节点

NAMESPACES = ("str", "dt", "list", "arr", "struct", "map", "cat", "name", "bin")
ARITH = {ast.Add: "+", ast.Sub: "-", ast.Mult: "*", ast.Div: "/", ast.FloorDiv: "//", ast.Mod: "%", ast.Pow: "**"}
LOGIC = {ast.BitAnd: "&", ast.BitOr: "|", ast.BitXor: "^"}
COMPARE = {ast.Eq: "==", ast.NotEq: "!=", ast.Gt: ">", ast.GtE: ">=", ast.Lt: "<", ast.LtE: "<="}
AGG = "sum mean median min max count n_unique first last std var len implode".split()


def dtype_param(src, n):
    """文字里写的类型 → 类型参数的写法（去掉 pl.）：pl.List(pl.Int64) → List(Int64)。不是类型的写法是 None。"""

    def is_dtype(x):
        if isinstance(x, ast.Attribute):
            return isinstance(x.value, ast.Name) and x.value.id == "pl" and x.attr in DTYPE_NAMES
        if isinstance(x, ast.Call):
            return is_dtype(x.func) and all(is_arg(a) for a in x.args) and all(is_arg(k.value) for k in x.keywords)
        return False

    def is_arg(x):
        if isinstance(x, (ast.List, ast.Tuple)):
            return all(is_arg(e) for e in x.elts)
        if isinstance(x, ast.Dict):
            return all(k is not None and is_arg(k) for k in x.keys) and all(is_arg(v) for v in x.values)
        return isinstance(x, ast.Constant) or is_dtype(x)

    seg = ast.get_source_segment(src, n)
    return dtype_text(seg, False) if seg and is_dtype(n) else None
INPUTS = ("a", "b", "c", "d")


def parse_text(text, types):
    """文字表达式 → 节点网络 {"nodes": [...], "root": 下标, "alias"?: 名字}。
    每个节点 {"type", "params": {参数: 填的文字}, "ports": {参数: 接哪个节点（下标）}, "net"?: eval 里面的网络（同样的格式）}；
    {"input": "a"} 是 snippet 节点的输入 a ~ d（接到原来接 a 的节点上）。types：{类型名: [[参数, 类型, 是不是端口], …]}。
    最外面是 .alias("名字") 时另给 alias：在 expr 节点里可以直接建成输出列。认不出的部分变成 snippet 节点（原文照写）。"""
    src = text.strip()
    tree = ast.parse(src, mode="eval").body
    alias = None
    if (
        isinstance(tree, ast.Call)
        and isinstance(tree.func, ast.Attribute)
        and tree.func.attr == "alias"
        and len(tree.args) == 1
        and not tree.keywords
        and isinstance(tree.args[0], ast.Constant)
        and isinstance(tree.args[0].value, str)
    ):
        alias = tree.args[0].value
    b = Builder(src, types)
    root = b.build(tree)
    out = {"nodes": b.nodes, "root": root}
    if alias is not None and isinstance(tree.func.value, ast.AST):
        inner = Builder(src, types)
        out["alias"] = {"name": alias, "nodes": inner.nodes, "root": inner.build(tree.func.value)}
    return out


class Builder:
    def __init__(self, src, types):
        self.src = src
        self.types = types
        self.nodes = []
        self.memo = {}

    def seg(self, node):
        return ast.get_source_segment(self.src, node) or ast.unparse(node)

    def add(self, node, type_name, params=None, ports=None, net=None):
        item = {"type": type_name, "params": params or {}, "ports": ports or {}}
        if net is not None:
            item["net"] = net
        self.nodes.append(item)
        return len(self.nodes) - 1

    def is_expr(self, n):
        """这段是不是表达式（要做成节点）：pl.… / cs.… / a ~ d 开头的调用链，或者含有它们的运算。"""
        if isinstance(n, ast.Name):
            return n.id in ("pl", "cs") or n.id in INPUTS
        if isinstance(n, (ast.Attribute, ast.Subscript)):
            return self.is_expr(n.value)
        if isinstance(n, ast.Call):
            return self.is_expr(n.func)
        if isinstance(n, ast.BinOp):
            return self.is_expr(n.left) or self.is_expr(n.right)
        if isinstance(n, ast.UnaryOp):
            return self.is_expr(n.operand)
        if isinstance(n, ast.Compare):
            return self.is_expr(n.left) or any(self.is_expr(c) for c in n.comparators)
        return False

    def build(self, n):
        key = ast.dump(n)
        if key not in self.memo:
            self.memo[key] = self.make(n)
        return self.memo[key]

    def operand(self, n, params, ports, name):
        """运算的一边：表达式做成节点接到端口上，常量写在参数框里。"""
        if self.is_expr(n):
            ports[name] = self.build(n)
        else:
            params[name] = self.seg(n)

    def make(self, n):
        if isinstance(n, ast.Name) and n.id in INPUTS:
            self.nodes.append({"input": n.id})
            return len(self.nodes) - 1
        if not self.is_expr(n):
            return self.add(n, "lit", {"value": self.seg(n)})
        if isinstance(n, ast.BinOp) and type(n.op) in ARITH:
            params, ports = {"op": ARITH[type(n.op)]}, {}
            self.operand(n.left, params, ports, "a")
            self.operand(n.right, params, ports, "b")
            return self.add(n, "arith", params, ports)
        if isinstance(n, ast.BinOp) and type(n.op) in LOGIC and self.is_expr(n.left) and self.is_expr(n.right):
            return self.add(n, "logic", {"op": LOGIC[type(n.op)]}, {"a": self.build(n.left), "b": self.build(n.right)})
        if isinstance(n, ast.UnaryOp) and isinstance(n.op, ast.Invert):
            return self.add(n, "logic", {"op": "~"}, {"a": self.build(n.operand)})
        if isinstance(n, ast.Compare) and len(n.ops) == 1 and type(n.ops[0]) in COMPARE:
            params, ports = {"op": COMPARE[type(n.ops[0])]}, {}
            self.operand(n.left, params, ports, "a")
            self.operand(n.comparators[0], params, ports, "b")
            return self.add(n, "compare", params, ports)
        if isinstance(n, ast.Call):
            made = self.call(n)
            if made is not None:
                return made
        return self.snippet(n)

    def snippet(self, n):
        """认不出的：原文写进 snippet 节点；里面用到的 a ~ d 照样接上。"""
        names = {x.id for x in ast.walk(n) if isinstance(x, ast.Name) and x.id in INPUTS}
        ports = {name: self.build(ast.Name(id=name, ctx=ast.Load())) for name in sorted(names)}
        return self.add(n, "snippet", {"expression": self.seg(n)}, ports)

    def call(self, n):
        func = n.func
        dotted = dotted_name(func)
        if dotted == "pl.col" and not n.keywords and all(isinstance(a, ast.Constant) for a in n.args):
            if len(n.args) == 1 and n.args[0].value == "":
                return self.add(n, "element")
            return self.add(n, "col", {"columns": ", ".join(self.seg(a) for a in n.args)})
        if dotted is not None and dotted.startswith("cs."):
            return self.add(n, "col", {"columns": self.seg(n)})
        if dotted == "pl.element" and not n.args:
            return self.add(n, "element")
        if dotted == "pl.lit" and len(n.args) == 1:
            params = {"value": self.seg(n.args[0])}
            for k in n.keywords:
                dtype = dtype_param(self.src, k.value)
                if k.arg != "dtype" or dtype is None:
                    return None
                params["dtype"] = dtype
            return self.add(n, "lit", params)
        if isinstance(func, ast.Attribute) and func.attr in ("otherwise", "then"):
            made = self.when(n)
            if made is not None:
                return made
        if dotted is not None and dotted.startswith("pl."):
            return self.generic(n, dotted, None)
        if not isinstance(func, ast.Attribute) or not self.is_expr(func.value):
            return None
        # 方法：x.m(…) 或 x.str.m(…)
        target, method, ns = func.value, func.attr, None
        if isinstance(target, ast.Attribute) and target.attr in NAMESPACES and self.is_expr(target.value):
            ns, target = target.attr, target.value
        if ns is None and method == "cast" and len(n.args) == 1:
            dtype = dtype_param(self.src, n.args[0])
            strict = [k for k in n.keywords if k.arg == "strict"]
            if dtype and len(strict) == len(n.keywords):
                params = {"dtype": dtype}
                if strict:
                    params["strict"] = self.seg(strict[0].value)
                return self.add(n, "cast", params, {"input": self.build(target)})
        if ns is None and method in AGG and not n.args and not n.keywords:
            return self.add(n, "agg", {"function": method}, {"input": self.build(target)})
        if ns in ("list", "arr") and method in ("eval", "filter", "agg") and len(n.args) == 1:
            flags = {"parallel": "list.eval", "as_list": "arr.eval"}
            params = {"method": f"{ns}.{method}"}
            for k in n.keywords:
                if flags.get(k.arg) != params["method"]:
                    return None
                params[k.arg] = self.seg(k.value)
            inner = Builder(self.src, self.types)
            net = {"nodes": inner.nodes, "root": inner.build(n.args[0])}
            return self.add(n, "eval", params, {"input": self.build(target)}, net)
        made = self.generic(n, f"{ns}.{method}" if ns else method, target)
        if made is None:
            # 认不出这一步：前面的部分照样拆成节点，这一步写成 snippet（a 接前面）
            whole, head = self.seg(n), self.seg(target)
            if whole.startswith(head) and not any(isinstance(x, ast.Name) and x.id in INPUTS for x in ast.walk(n)):
                expression = "a" + whole[len(head):]
                return self.add(n, "snippet", {"expression": expression}, {"a": self.build(target)})
        return made

    def generic(self, n, name, target):
        """生成的表达式节点（nodes/catalog.json）：位置参数按签名的顺序，关键字参数按名字。"""
        spec = self.types.get(name)
        if spec is None:
            return None
        params, ports = {}, {}
        slots = list(spec)
        if target is not None:
            if not slots or slots[0][0] != "input":
                return None
            ports["input"] = self.build(target)
            slots = slots[1:]
        by_name = {p[0]: p for p in slots}
        if len(n.args) > len(slots) or any(k.arg not in by_name for k in n.keywords):
            return None
        assigned = list(zip(slots, n.args)) + [(by_name[k.arg], k.value) for k in n.keywords]
        for (pname, kind, port), value in assigned:
            if port and self.is_expr(value):
                ports[pname] = self.build(value)
            else:
                params[pname] = self.param_text(kind, value)
        return self.add(n, name, params, ports)

    def param_text(self, kind, value):
        """填进参数框的文字：文字 / 菜单是字符串本身，列表类的去掉括号，其余照原文。"""
        if kind in ("text", "raw", "menu", "file", "save_file", "sheet") and isinstance(value, ast.Constant) and isinstance(value.value, str):
            return value.value
        if kind in ("exprs", "columns") and isinstance(value, (ast.List, ast.Tuple)):
            return ", ".join(self.seg(e) for e in value.elts)
        if kind == "dtype" and dtype_param(self.src, value) is not None:
            return dtype_param(self.src, value)
        if kind == "dict" and isinstance(value, ast.Dict):
            return ", ".join(f"{self.seg(k)}: {self.seg(v)}" for k, v in zip(value.keys, value.values))
        return self.seg(value)

    def when(self, n):
        """pl.when(c).then(v)[.when(c).then(v)…][.otherwise(o)]，最多三个条件。"""
        otherwise = None
        if n.func.attr == "otherwise":
            if len(n.args) != 1:
                return None
            otherwise, n = n.args[0], n.func.value
        branches = []
        while isinstance(n, ast.Call) and isinstance(n.func, ast.Attribute) and n.func.attr == "then" and len(n.args) == 1:
            then = n.args[0]
            head = n.func.value
            if not (isinstance(head, ast.Call) and isinstance(head.func, ast.Attribute) and head.func.attr == "when" and len(head.args) == 1):
                return None
            branches.insert(0, (head.args[0], then))
            n = head.func.value
            if dotted_name(n) == "pl":
                break
        if dotted_name(n) != "pl" or not branches or len(branches) > 3:
            return None
        params, ports = {}, {}
        names = [("condition", "then"), ("condition2", "then2"), ("condition3", "then3")]
        for (cname, tname), (cond, then) in zip(names, branches):
            self.operand(cond, params, ports, cname)
            self.value(then, params, ports, tname)
        if otherwise is not None:
            self.value(otherwise, params, ports, "otherwise")
        return self.add(n, "when", params, ports)

    def value(self, v, params, ports, name):
        """when 的取值：节点里文字当常量（它会套 pl.lit），所以 polars 里的 "列名" 要写成 pl.col("列名")。"""
        if isinstance(v, ast.Constant) and isinstance(v.value, str):
            params[name] = f"pl.col({self.seg(v)})"
        elif isinstance(v, ast.Call) and dotted_name(v.func) == "pl.lit" and len(v.args) == 1 and not v.keywords:
            params[name] = self.seg(v.args[0])
        else:
            self.operand(v, params, ports, name)


def dotted_name(n):
    """pl.col / cs.numeric 这种点连起来的名字；不是的话 None。"""
    parts = []
    while isinstance(n, ast.Attribute):
        parts.insert(0, n.attr)
        n = n.value
    if isinstance(n, ast.Name):
        return ".".join([n.id, *parts])
    return None


def memory_budget():
    """留住的结果（见 main 里的 kept）最多占多少内存：物理内存的 1/4，最多 4 GB；问不到按 8 GB 的机器算。"""
    total = 8 << 30
    try:
        import ctypes

        class Status(ctypes.Structure):
            _fields_ = [
                ("length", ctypes.c_ulong),
                ("load", ctypes.c_ulong),
                ("total_phys", ctypes.c_ulonglong),
                ("avail_phys", ctypes.c_ulonglong),
                ("total_page", ctypes.c_ulonglong),
                ("avail_page", ctypes.c_ulonglong),
                ("total_virtual", ctypes.c_ulonglong),
                ("avail_virtual", ctypes.c_ulonglong),
                ("avail_extended", ctypes.c_ulonglong),
            ]

        status = Status()
        status.length = ctypes.sizeof(Status)
        if ctypes.windll.kernel32.GlobalMemoryStatusEx(ctypes.byref(status)):
            total = status.total_phys
    except Exception:
        pass
    return min(4 << 30, total // 4)


def subs_json(info):
    """{id: 信息} → JSON（键要是字符串），里面网络的信息一样。"""
    out = {}
    for i, v in info.items():
        v = dict(v)
        if "net" in v:
            v["net"] = subs_json(v["net"])
        out[str(i)] = v
    return out


def main():
    try:
        import polars as pl
    except Exception as e:
        send({"ready": False, "error": short(e)})
        return
    send({"ready": True, "polars": pl.__version__})
    # 库自己的“将来会变”提醒对用户没用，别混进节点的输出里
    import warnings

    warnings.simplefilter("ignore", FutureWarning)
    warnings.simplefilter("ignore", DeprecationWarning)

    # 节点的查询计划：key -> (LazyFrame, 列名, 最近使用的请求序号, 类型, 用到了留住的结果)
    cache = {}
    # 留住的结果：显示的节点 collect 出来的表，key -> (DataFrame, 字节数, 最近使用的请求序号)，先放的在前。
    # 接在它下游的节点、expr 网络的预览、write 的“写出”从这张表接着算，上游不用再算一遍
    # （显示标记往下游挪一格时最常见）。查询计划（explain）不用它：计划要从源头开始写
    kept = {}
    budget = memory_budget()
    # expr 节点算的时候写下的说明（网络里各节点的类型 / 问题 / 文字，整个节点的文字）：key -> (subs, text)。
    # 从缓存、留住的结果里拿的节点不再算一遍，说明照样报回去：界面那边重建了图（撤销、重新加载）也要它们
    described = {}
    # 选列框要的表结构：key -> Schema（节点没变就不用再推一遍）
    schemas = {}
    seq = 0

    def forget(keys):
        """丢掉这些留住的结果。用到了它们的计划也丢掉（计划里引用着那张表，不丢内存就还占着），要用时按原样重建。"""
        if not keys:
            return
        for k in keys:
            del kept[k]
        for k in [k for k, v in cache.items() if v[4]]:
            del cache[k]

    def keep(key, df):
        """留住显示节点的结果：合计超过 budget 时先丢最早留住的；单张超过一半的不留。"""
        size = df.estimated_size()
        if size > budget // 2:
            return
        kept.pop(key, None)
        kept[key] = (df, size, seq)
        total, old = sum(v[1] for v in kept.values()), []
        for k, v in kept.items():
            if total <= budget:
                break
            total -= v[1]
            old.append(k)
        forget(old)

    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        req = json.loads(line)
        if "parse" in req:
            try:
                send({"parsed": parse_text(req["parse"], req.get("types") or {})})
            except SyntaxError as e:
                send({"error": f"写得不对（第 {e.offset or 0} 个字附近）：{e.msg}"})
            except Exception as e:
                send({"error": short(e)})
            continue
        if "doc" in req:
            docs = {}
            for path in req["doc"]:
                try:
                    with contextlib.redirect_stderr(io.StringIO()):
                        docs[path] = api_doc(pl, path)
                except Exception as e:
                    docs[path] = {"error": short(e)}
            send({"doc": docs})
            continue
        if req.get("catalog"):
            try:
                with contextlib.redirect_stderr(io.StringIO()):
                    send({"catalog": catalog(pl)})
            except Exception as e:
                send({"catalog": {}, "error": short(e)})
            continue
        if "pick" not in req:
            seq += 1  # 选列框点一下发一次：不算数，免得把真要用的计划挤出缓存
        started = time.perf_counter()
        steps = {s["id"]: s for s in req["steps"]}
        keys = {}
        columns = {}
        dtypes = {}
        subs = {}  # expr 节点 id -> 网络里各节点的类型 / 错误 / 文字
        texts = {}  # expr 节点 id -> 整个节点写成文字（df.with_columns(…)）
        captured = io.StringIO()
        use_kept = "explain" not in req  # 查询计划要从源头开始写：不用留住的结果
        reused = []  # 这次用了留住的结果的节点
        via_kept = {}  # 节点 id -> 它的计划里有留住的结果（它自己的，或者上游的）

        def key_of(nid):
            # 代码、参数、读的文件、上游、里面的网络，有一样变了就要重算
            if nid not in keys:
                s = steps[nid]
                params = json.dumps(s.get("params") or {}, sort_keys=True, ensure_ascii=False)
                net = json.dumps(s.get("net") or [], sort_keys=True, ensure_ascii=False)
                upstream = tuple(None if i is None else key_of(i) for i in s["inputs"])
                keys[nid] = (s["code"], params, net, s.get("stamp"), upstream)
            return keys[nid]

        def inputs_of(nid):
            return [None if i is None else get(i) for i in steps[nid]["inputs"]]

        def describe_net(nid, inputs, report):
            if "values" in report:
                first = inputs[0] if inputs else None
                run, srun = (report["values"], report["errors"], report["inner"]), sym_run(nid)
                subs[nid] = net_types(pl, steps[nid], first, run, srun)
                texts[nid] = node_text(pl, steps[nid], srun)
                described[key_of(nid)] = (subs[nid], texts[nid])

        def described_again(nid, key):
            if key in described:
                subs[nid], texts[nid] = described[key]

        def sym_run(nid):
            """同一张网络用替身再算一遍：每个节点写成文字。"""
            try:
                return run_net(pl, steps[nid]["net"], [Sym("df")], sym=sym_env)
            except Exception:
                return ({}, {}, {})

        def get(nid):
            key = key_of(nid)
            if use_kept and key in kept:
                df, size, _ = kept.pop(key)
                kept[key] = (df, size, seq)  # 最近用过的挪到最后，最晚被丢
                reused.append(nid)
                via_kept[nid] = True
                columns[nid] = df.columns
                dtypes[nid] = [str(t) for t in df.dtypes]
                described_again(nid, key)
                return df.lazy()
            # 用到了留住的结果的计划不能给 explain 用：explain 另外按原样建一份，存在 ("pure", key) 下
            for k in (key,) if use_kept else (key, ("pure", key)):
                hit = cache.get(k)
                if hit is not None and (use_kept or not hit[4]):
                    cache[k] = (hit[0], hit[1], seq, hit[3], hit[4])
                    columns[nid] = hit[1]
                    dtypes[nid] = hit[3]
                    via_kept[nid] = hit[4]
                    described_again(nid, key)
                    return hit[0]
            inputs = inputs_of(nid)
            uses = any(via_kept.get(i, False) for i in steps[nid]["inputs"] if i is not None)
            report = {}
            try:
                lf = run_code(pl, steps[nid], inputs, report=report)
                schema = lf.collect_schema()
                names = schema.names()
                types = [str(t) for t in schema.dtypes()]
            except NodeError:
                raise
            except SubError as e:
                raise NodeError(nid, e.msg, e.sub)
            except ParamError as e:
                raise NodeError(nid, str(e))
            except Exception as e:
                raise NodeError(nid, short(e))
            finally:
                describe_net(nid, inputs, report)
            # explain 按原样重建的、而普通运行的那份用到了留住的结果：另外存，别把那份换掉
            mine = key if use_kept or not cache.get(key, (None,) * 5)[4] else ("pure", key)
            cache[mine] = (lf, names, seq, types, uses)
            via_kept[nid] = uses
            columns[nid] = names
            dtypes[nid] = types
            return lf

        def preview(target, sub, label, info):
            """表达式网络里选中的节点：它的值（不执行 expr 节点本身）。info 收下预览的说明（note）。"""
            inputs = inputs_of(target)
            lf = inputs[0] if inputs else None
            if lf is None:
                raise NodeError(target, "第 1 个输入还没有连接")
            run = run_net(pl, steps[target]["net"], inputs)
            srun = sym_run(target)
            subs[target] = net_types(pl, steps[target], lf, run, srun)
            texts[target] = node_text(pl, steps[target], srun)
            path = sub if isinstance(sub, list) else [sub]
            try:
                return preview_frame(pl, steps[target], lf, run, path, label, info)
            except SubError as e:
                raise NodeError(target, e.msg, e.sub)

        resp = {"ticket": req.get("ticket")}
        target = req["display"]
        try:
            with contextlib.redirect_stdout(captured), contextlib.redirect_stderr(captured):
                try:
                    if "action" in req:
                        # 按钮：每次都执行，不进缓存
                        run_code(pl, steps[target], inputs_of(target), action=req["action"])
                        rows = cols = 0
                    elif "pick" in req:
                        # 选列框：target 的列名、类型（回复的 columns / dtypes），以及写的这些现在选到哪几列
                        lf = get(target)
                        key = key_of(target)
                        if key not in schemas:
                            schemas[key] = lf.collect_schema()
                        resp["pick"] = pick_columns(pl, schemas[key], req["pick"])
                        rows = cols = 0
                    elif req.get("schema"):
                        # 只要类型：结果进缓存，之后接着往下跑的节点用得上
                        get(target)
                        rows = cols = 0
                    elif "explain" in req:
                        # 查询计划：不执行，polars 说优化后 / 优化前要怎么算（tree = 画成树）
                        lf = get(target).lazy()
                        fmt = "tree" if req["explain"] == "tree" else "plain"
                        resp["plan"] = {"optimized": lf.explain(format=fmt), "raw": lf.explain(format=fmt, optimized=False)}
                        rows = cols = 0
                    else:
                        if req.get("preview"):
                            p = req["preview"]
                            info = {}
                            frame = preview(target, p["sub"], p.get("label") or "结果", info)
                            if info.get("note"):
                                resp["note"] = info["note"]
                        else:
                            frame = get(target)
                        df = frame.collect()
                        shown, meta = readable(pl, df)
                        shown.write_parquet(req["out"], compression="lz4", statistics=False, metadata=meta)
                        del shown
                        rows, cols = df.height, df.width
                        if not req.get("preview"):
                            keep(key_of(target), df)
                        del df
                except NodeError:
                    raise
                except SubError as e:
                    raise NodeError(target, e.msg, e.sub)
                except ParamError as e:
                    raise NodeError(target, str(e))
                except Exception as e:
                    raise NodeError(target, short(e))
            resp.update(ok=True, rows=rows, cols=cols)
        except NodeError as e:
            resp.update(ok=False, node=e.node, sub=e.sub, error=e.msg)
        except Exception as e:
            resp.update(ok=False, node=None, error=short(e))
        for k in [k for k, v in cache.items() if seq - v[2] >= CACHE_KEEP]:
            del cache[k]
        forget([k for k, v in kept.items() if seq - v[2] >= CACHE_KEEP])
        for k in [k for k in described if k not in cache and k not in kept]:
            del described[k]
        for k in [k for k in schemas if k not in cache and k not in kept]:
            del schemas[k]
        resp["reused"] = reused
        resp["columns"] = {str(k): v for k, v in columns.items()}
        resp["dtypes"] = {str(k): v for k, v in dtypes.items()}
        resp["subs"] = {str(k): subs_json(info) for k, info in subs.items()}
        resp["texts"] = {str(k): v for k, v in texts.items() if v}
        resp["stdout"] = captured.getvalue()[-4000:]
        resp["ms"] = int((time.perf_counter() - started) * 1000)
        send(resp)


main()
