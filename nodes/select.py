r"""
{
  "desc": "选择 / 删除列（select / drop）",
  "category": "列",
  "order": 10,
  "inputs": 1,
  "params": [
    {"name": "mode", "type": "menu", "label": "方式", "mode": true, "options": ["select", "drop"]},
    {"name": "columns", "type": "columns", "label": "列或表达式", "hint": "\"医院\", pl.col(\"金额\") * 2；按类型选写 cs.numeric()"},
    {"name": "ranges", "type": "text", "label": "按位置", "pick": "ranges", "hint": "按第几列选，点“范围”编辑；写成 \"1:2\", \"收入:合计\""}
  ]
}

挑出要的列（select），或删掉不要的列（drop）。“列或表达式”和“按位置”合在一起，按写的顺序排。

列可以写列名，也可以写选择器：cs.numeric() 全部数值列、cs.string() 全部文字列、cs.starts_with("金额") 名字以“金额”开头的列；选择器能用 | & - ~ 组合。

“按位置”按列的顺序选（第几列），点旁边的“范围”编辑：一段一段加，每段的起、止填序号或者从下拉里选一个列名（按它当时的位置算）。直接写也行，一段是 起:止，逗号分开多段——
* 1:5 第 1 到第 5 列（序号 1 起、含两端）；1: 从第 1 列到末尾；:5 从开头到第 5 列
* "收入":"合计" 从“收入”到“合计”这两列之间（含两端，按表里的位置）
* 列名要带引号，纯数字的列名也一样；不带的数字按序号算

要算新列、改列的值，用 expr 节点（上下文 select / with_columns）。

@parameters
mode:
    * select：只留这些列，按写的顺序排（也能用来调列的顺序）
    * drop：删掉这些列，其余不动
columns:
    逗号分开。点旁边的“选列”可以勾列、排除列、按条件（选择器）选，确定后写进来。
ranges:
    按位置的一段段范围，点旁边的“范围”编辑。运行时按当时表的列序展开成具体列名，上游换了列跟着走。

@examples
留下“医院”和全部数值列：
{{{
方式        select
列或表达式  "医院", cs.numeric()
}}}

留下前两列、从“收入”到“合计”的列，再单独留下第 10 列：
{{{
方式        select
按位置      1:2, "收入":"合计", 10:10
}}}

@related
rename, cast_columns, expr

@polars
select: DataFrame.select
drop: DataFrame.drop
"""


import ast


def split_top(text, sep):
    """按 sep 拆开，引号里的不算：'"a:b", c:d' 按, → ['"a:b"', ' c:d']。"""
    parts, buf, quote, esc = [], "", "", False
    for ch in text:
        if esc:
            buf += ch
            esc = False
        elif quote:
            if ch == "\\":
                esc = True
            buf += ch
            if ch == quote:
                quote = ""
        elif ch in "\"'":
            quote = ch
            buf += ch
        elif ch == "\\":
            esc = True
            buf += ch
        elif ch == sep:
            parts.append(buf)
            buf = ""
        else:
            buf += ch
    parts.append(buf)
    return parts


def parse_side(text, seg):
    """范围的一段端点 → ("序号", n) / ("名字", name) / ("开口", None)。"""
    t = text.strip()
    if t and t[0] in "\"'":
        try:
            name = ast.literal_eval(t)
        except Exception:
            raise ValueError(f'范围 "{seg}" 里的 "{t}" 不是合法的列名写法')
        if not isinstance(name, str):
            raise ValueError(f'范围 "{seg}" 里的 "{t}" 应该是列名')
        return ("名字", name)
    if t.isdigit():
        return ("序号", int(t))
    if not t:
        return ("开口", None)
    return ("名字", t)


def parse_ranges(text):
    """“按位置”的文字 → [(原文, 起, 止), …]，起止是 parse_side 的结果。"""
    pairs = []
    for seg in split_top(text, ","):
        seg = seg.strip()
        if not seg:
            raise ValueError("范围里有多余的逗号")
        ends = split_top(seg, ":")
        if len(ends) != 2:
            raise ValueError(f'范围 "{seg}" 要写成 起:止')
        pairs.append((seg, parse_side(ends[0], seg), parse_side(ends[1], seg)))
    return pairs


def bound(side, names, which, seg):
    """端点 → 第几列（1 起）；开口是没有。"""
    kind, v = side
    if kind == "开口":
        return None
    if kind == "序号":
        if not 1 <= v <= len(names):
            raise ValueError(f'范围 "{seg}" 里的第 {v} 列不在表里（共 {len(names)} 列）')
        return v
    if v not in names:
        raise ValueError(f'范围 "{seg}" 里{which}的列 "{v}" 不在表里')
    return names.index(v) + 1


cols = list(ch("columns"))
ranges = ch("ranges").strip()
if ranges:
    names = df.collect_schema().names()
    for seg, a, b in parse_ranges(ranges):
        start, end = bound(a, names, "起", seg), bound(b, names, "止", seg)
        start = start or 1
        end = end or len(names)
        if start > end:
            start, end = end, start
        cols.extend(names[start - 1:end])
if not cols:
    raise ValueError("没有选到列：填“列或表达式”，或点“范围”加位置范围")
out = getattr(df, ch("mode"))(*cols)
