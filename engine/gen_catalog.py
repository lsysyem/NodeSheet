# 从本机装的 polars 生成表达式节点（VOP）的目录：nodes/catalog.json。
#
# 每个返回 Expr 的方法 / 函数变成一种表达式节点：签名里的参数按类型注解变成参数模板——
#   IntoExpr / Expr / Any …  → 输入端口（没连线时用参数框里填的值）
#   Literal[...] 和它的别名 → 下拉菜单；bool → 开关；int / float / str → 数字 / 文字；其余 → Python 表达式
#   Callable（必填的）→ Python 表达式，填 lambda（map_elements / name.map / pl.fold…）
# 节点的代码就是那一次调用：out = ch("input").str.pad_start(ch("length"), **opt(fill_char=ch("fill_char")))
# （opt 去掉没填的关键字参数，让 polars 用它自己的默认值）。
#
# 升级 polars 后重新生成：python engine/gen_catalog.py
import inspect
import json
import re
import sys
import typing
from pathlib import Path

import polars as pl
import polars._typing as ptyping

OUT = Path(__file__).resolve().parent.parent / "nodes" / "catalog.json"

# Expr 自己的方法按用途分组（其余进“其他”）
GROUPS = {
    "运算": "add sub mul truediv floordiv mod pow neg dot",
    "比较": "eq ne gt ge lt le eq_missing ne_missing is_between is_in is_close",
    "逻辑": "and_ or_ xor not_",
    "空值": "fill_null fill_nan is_null is_not_null is_nan is_not_nan drop_nulls drop_nans null_count has_nulls "
    "is_finite is_infinite forward_fill backward_fill interpolate interpolate_by",
    "聚合": "sum mean median min max count len n_unique approx_n_unique first last std var quantile product "
    "nan_max nan_min mode implode arg_max arg_min all any entropy kurtosis skew unique_counts value_counts "
    "item min_by max_by bitwise_and bitwise_or bitwise_xor bitwise_count_ones bitwise_count_zeros",
    "窗口": "over rolling rolling_mean rolling_sum rolling_min rolling_max rolling_std rolling_var rolling_median "
    "rolling_quantile rolling_skew rolling_kurtosis rolling_rank rolling_map rolling_mean_by rolling_sum_by "
    "rolling_min_by rolling_max_by rolling_std_by rolling_var_by rolling_median_by rolling_quantile_by "
    "rolling_rank_by cum_sum cum_prod cum_min cum_max cum_count cumulative_eval shift diff pct_change rank "
    "ewm_mean ewm_std ewm_var ewm_mean_by rle rle_id is_first_distinct is_last_distinct is_unique is_duplicated",
    "数学": "abs sqrt cbrt exp log log10 log1p round round_sig_figs floor ceil clip sign sin cos tan cot "
    "arcsin arccos arctan sinh cosh tanh arcsinh arccosh arctanh degrees radians hist search_sorted peak_max "
    "peak_min hash",
    "类型": "cast reinterpret to_physical shrink_dtype",
    "选取": "head tail slice gather get gather_every filter sort sort_by reverse unique top_k bottom_k top_k_by "
    "bottom_k_by arg_sort arg_unique arg_true sample shuffle limit explode flatten repeat_by extend_constant "
    "append replace replace_strict cut qcut set_sorted",
    "命名": "alias",
    "代码": "map_elements map_batches",
}
GROUP_OF = {m: g for g, names in GROUPS.items() for m in names.split()}

NAMESPACES = {
    "str": "文本 str",
    "dt": "日期 dt",
    "list": "列表 list",
    "arr": "数组 arr",
    "struct": "结构 struct",
    "map": "映射 map",
    "cat": "分类 cat",
    "name": "命名",
    "bin": "二进制 bin",
}

# 顶层函数：只要和 Expr 方法不重复的（pl.sum("a") 就是 pl.col("a").sum()）
FUNCTIONS = (
    "len lit int_range int_ranges list concat_str concat_list concat_arr coalesce struct all_horizontal any_horizontal "
    "sum_horizontal max_horizontal min_horizontal mean_horizontal cum_sum_horizontal date datetime duration time "
    "format arctan2 repeat corr cov rolling_corr rolling_cov business_day_count date_range date_ranges "
    "datetime_range datetime_ranges time_range time_ranges arg_sort_by arg_where nth exclude field element "
    "from_epoch linear_space linear_spaces ones zeros row_index sql_expr fold reduce cum_fold cum_reduce "
    "map_batches map_groups"
).split()

# 要的回调是 Python 对象（不是填得出来的 lambda）、或者只对 Python 对象有意义的，不做成节点
SKIP = set("pipe inspect deserialize from_json register_plugin meta plot".split())

# 精选节点替代了的（nodes/eval.py：里面是一张网络，用 element 节点连；nodes/element.py）
REPLACED = set("list.eval list.filter list.agg arr.eval arr.agg pl.element".split())

# 这些参数的文字不去首尾空白（分隔符、要找的字符…）
RAW_TEXT = set("separator fill_char characters pattern value prefix suffix by delimiter quote_char".split())

TYPE_HINT = "Python 写法：10、\"文字\"、[1, 2]、pl.col(\"列\")"
DTYPE_HINT = "pl.Int64、pl.Float64、pl.String、pl.Date、pl.Datetime…"
DTYPE_PARAM_HINT = "Int64、Float64、String、Datetime(\"ms\")、List(Int64)…；右边下拉列着全部类型"


def first_line(obj):
    doc = inspect.getdoc(obj) or ""
    return doc.strip().split("\n")[0].strip()


def deprecated(obj):
    """整个方法废弃了。参数说明里的 deprecated 只是那个参数废弃了（见 deprecated_params）。"""
    doc = (inspect.getdoc(obj) or "").split("\nParameters\n")[0]
    return ".. deprecated::" in doc or "This functionality is deprecated" in doc


def deprecated_params(obj):
    """文档 Parameters 一节里标了 deprecated 的参数名。"""
    doc = inspect.getdoc(obj) or ""
    if "\nParameters\n" not in doc:
        return set()
    section = doc.split("\nParameters\n", 1)[1]
    section = re.split(r"\n(?:Returns|Raises|Notes|Examples|See Also|Warnings)\n", section)[0]
    found, name = set(), None
    for line in section.splitlines():
        if line and not line[0].isspace() and not line.startswith("-"):
            name = line.split(":")[0].strip().lstrip("*")
        elif name and ".. deprecated::" in line:
            found.add(name)
    return found


def literal_options(annotation):
    """注解里的 Literal 选项（含 polars._typing 里的别名，如 RankMethod）；没有返回 None。"""
    found = re.findall(r"Literal\[([^\]]*)\]", annotation)
    options = []
    for body in found:
        options += [o.strip().strip("'\"") for o in body.split(",") if o.strip()]
    for token in re.findall(r"[A-Za-z_][A-Za-z0-9_]*", annotation):
        alias = getattr(ptyping, token, None)
        if alias is None or token in ("Any", "Expr"):
            continue
        if typing.get_origin(alias) is typing.Literal:
            options += [str(o) for o in typing.get_args(alias)]
    seen = []
    for o in options:
        if o not in seen:
            seen.append(o)
    return seen or None


def union_parts(annotation):
    """顶层的 | 拆开：'int | Sequence[int | str] | None' → ['int', 'Sequence[int | str]', 'None']。"""
    parts, depth, current = [], 0, ""
    for c in annotation:
        if c == "[":
            depth += 1
        elif c == "]":
            depth -= 1
        if c == "|" and depth == 0:
            parts.append(current.strip())
            current = ""
        else:
            current += c
    parts.append(current.strip())
    return [p for p in parts if p]


PORT_TYPES = {"IntoExpr", "IntoExprColumn", "Expr", "Any", "Selector"}


def callback_example(annotation):
    """Callable[[参数…], 返回] → 参数框里的 lambda 示例。"""
    found = re.search(r"Callable\[\[(.*?)\],\s*(.*)\]", annotation)
    args, ret = (found.group(1), found.group(2)) if found else ("", "")
    if args == "str":
        return "lambda name: name.upper()"
    if args.count("Series") == 2:
        return "lambda acc, x: acc + x"
    if args.startswith(("Sequence", "list")):
        return "lambda s: s[0]"
    if args == "Series":
        return "lambda s: s.sum()" if ret == "Any" else "lambda s: s"
    if args == "Expr, DataType":
        return "lambda e, dtype: e.cast(pl.Float64) if dtype.is_integer() else e"
    return "lambda x: x"


def param_def(p, annotation, element=False):
    """签名里的一个参数 → 参数模板（dict，和 nodes/*.py 开头 JSON 里的写法一样）。None = 不收这个参数。"""
    # polars 在命名空间里用 str_ / List_ 这类别名（免得和方法名冲突）
    a = re.sub(r"\b(str|int|float|bool|list|dict|type)_\b", r"\1", annotation or "Any").replace("List_", "list")
    if "Callable" in a:
        if p.default is not inspect.Parameter.empty:
            return None  # 可选的回调（凭据之类）不收
        example = callback_example(a)
        return {"name": p.name, "type": "expr", "required": True, "insert": "none", "hint": f"Python 函数，如 {example}"}
    d = {"name": p.name}
    parts = [x for x in union_parts(a) if x != "None"]
    nullable = len(parts) < len(union_parts(a))
    bare = {x for x in parts if "[" not in x}
    containers = [x for x in parts if "[" in x and not x.startswith("Literal[")]
    port = bool(bare & PORT_TYPES)
    options = literal_options(a)
    # 只收数据类型（PolarsDataType / DataTypeExpr…）：类型参数，下拉列着 polars 的全部类型
    dtype_only = bool(parts) and all("DataType" in x or "IntegerType" in x or x == "type[Any]" for x in parts)
    if dtype_only:
        kind = "dtype"
    elif options and not port:
        kind = "menu"
    elif parts and bare == {"bool"} and not containers:
        kind = "expr" if nullable else "toggle"  # bool | None：留空 = 让 polars 自己决定
    elif not containers and "int" in bare and "float" not in bare and ("str" not in bare or port):
        # int | str | Expr：字符串是列名，没连线时多半填数字
        kind = "int"
    elif not containers and "float" in bare and "str" not in bare:
        kind = "float"
    elif not containers and "str" in bare:
        kind = "text"
    elif port and any("IntoExpr" in x or "str" in x for x in containers):
        kind = "exprs"
    else:
        kind = "expr"
    d["type"] = kind
    if port:
        d["port"] = True
    default = p.default
    if default is inspect.Parameter.empty:
        d["required"] = True
    elif default is None or repr(default).startswith("<"):
        pass  # 留空：不传，用 polars 的默认值
    elif kind == "toggle":
        d["default"] = "True" if default else "False"
    elif kind in ("int", "float"):
        d["default"] = str(default)
    elif kind in ("text", "menu") and isinstance(default, str):
        d["default"] = default
    elif kind == "menu":
        d["default"] = str(default)
    elif isinstance(default, pl.DataType) or (isinstance(default, type) and issubclass(default, pl.DataType)):
        d["default"] = repr(default)  # 类型参数的写法：Int64（写不写 pl. 都行）
    else:
        d["default"] = repr(default)
    if kind == "menu":
        opts = list(options)
        if default is inspect.Parameter.empty:
            # 必填的菜单：默认选第一个，不会空着
            d["default"] = opts[0]
            del d["required"]
        elif default is None or repr(default).startswith("<"):
            opts.insert(0, "")  # 空 = 不传，用 polars 的默认
            d["default"] = ""
        d["options"] = opts
        if d.get("default", opts[0]) not in opts:
            d["default"] = opts[0]
    if kind == "text" and p.name in RAW_TEXT:
        d["strip"] = False
    hint = a.replace("'", "")
    if kind == "dtype":
        hint = DTYPE_PARAM_HINT
    elif "PolarsDataType" in a or "DataType" in a:
        hint = DTYPE_HINT
    elif element:
        hint = "对列表里的每个元素：pl.element() > 1、pl.element().sum()"
    elif kind in ("expr", "exprs"):
        hint = f"{hint}；{TYPE_HINT}"
    d["hint"] = hint
    if kind != "toggle":
        d["insert"] = "expr" if kind in ("expr", "exprs") else "none"
    return d


def build(name, label_path, group, func, self_arg):
    """一个方法 / 函数 → 节点定义。self_arg：第一个参数是输入的表达式（方法）。"""
    sig = inspect.signature(func)
    ret = str(sig.return_annotation).replace("pl.Expr", "Expr")
    # pl.int_range 这类带 eager 的函数注解成 Expr | Series：节点里总是表达式（eager 不传）
    if ret not in ("Expr", "<class 'polars.expr.expr.Expr'>", "Expr | Series", "Series | Expr", "Expr | list[Expr]"):
        return None
    params = []
    if self_arg:
        params.append({"name": "input", "type": "expr", "label": "输入", "port": True, "required": True,
                       "insert": "expr", "hint": "没连线时：pl.col(\"列\")"})
    positional = []  # 直接传（位置参数）
    keyword = []  # 去掉没填的再传
    star = None
    gone = deprecated_params(func)
    items = [p for p in sig.parameters.values() if not (p.name in gone and p.default is not inspect.Parameter.empty)]
    if self_arg:
        items = items[1:]
    var_index = next((i for i, p in enumerate(items) if p.kind == p.VAR_POSITIONAL), None)
    for i, p in enumerate(items):
        if p.kind == p.VAR_KEYWORD or p.name == "eager":
            continue
        annotation = p.annotation if isinstance(p.annotation, str) else getattr(p.annotation, "__name__", "")
        if p.kind == p.VAR_POSITIONAL:
            # *more_exprs：再给三个可选的端口
            names = [f"{p.name}{k}" for k in (1, 2, 3)]
            for k, n in enumerate(names):
                params.append({"name": n, "type": "expr", "label": f"{p.name} {k + 1}", "port": True,
                               "insert": "expr", "hint": f"可选：再加一个；{TYPE_HINT}"})
            star = names
            continue
        # list.eval / arr.agg 这类：表达式里用 pl.element() 指列表的元素
        element = (label_path.split(".")[0] in ("list", "arr") and p.name in ("expr", "predicate")) or name == "cumulative_eval"
        d = param_def(p, annotation, element)
        if d is None:
            continue
        if d["name"] == "input":
            d["name"] = "input_"
        params.append(d)
        before_star = var_index is not None and i < var_index
        if p.kind == p.POSITIONAL_ONLY or before_star:
            positional.append(d["name"])
        elif d.get("required"):
            keyword.append((d["name"], False))
        else:
            keyword.append((d["name"], True))
    args = [f'ch("{n}")' for n in positional]
    if star:
        values = ", ".join(f'ch("{n}")' for n in star)
        args.append(f"*[v for v in ({values}) if v is not None]")
    args += [f'{n}=ch("{n}")' for n, optional in keyword if not optional]
    optional = [f'{n}=ch("{n}")' for n, optional in keyword if optional]
    if optional:
        args.append(f"**opt({', '.join(optional)})")
    target = f'ch("input").{label_path}' if self_arg else f"pl.{label_path}"
    code = f"out = {target}({', '.join(args)})\n"
    return {
        "name": name,
        "desc": first_line(func) or name,
        "category": group,
        "params": params,
        "code": code,
    }


def main():
    entries = []
    skipped = []
    e = pl.col("a")

    def add(name, path, group, func, self_arg):
        if name.split(".")[-1] in SKIP or name in REPLACED or deprecated(func):
            return
        try:
            built = build(name, path, group, func, self_arg)
        except (ValueError, TypeError) as err:
            skipped.append(f"{name}: {err}")
            return
        if built:
            entries.append(built)

    for name in sorted(dir(pl.Expr)):
        if name.startswith("_") or name in NAMESPACES or name in SKIP:
            continue
        func = getattr(pl.Expr, name)
        if callable(func) and not isinstance(func, property):
            add(name, name, GROUP_OF.get(name, "其他"), func, True)
    for ns, group in NAMESPACES.items():
        cls = type(getattr(e, ns))
        for name in sorted(dir(cls)):
            if name.startswith("_"):
                continue
            func = getattr(cls, name)
            if callable(func):
                add(f"{ns}.{name}", f"{ns}.{name}", group, func, True)
    for name in FUNCTIONS:
        func = getattr(pl, name, None)
        if func is not None:
            add(f"pl.{name}", name, "函数 pl", func, False)

    data = {"polars": pl.__version__, "types": entries}
    OUT.write_text(json.dumps(data, ensure_ascii=False, indent=0) + "\n", encoding="utf-8")
    print(f"{len(entries)} 种表达式节点 → {OUT}（polars {pl.__version__}）")
    for s in skipped:
        print("  跳过", s, file=sys.stderr)


main()
