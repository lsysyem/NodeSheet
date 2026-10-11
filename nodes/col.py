r"""
{
  "desc": "列：pl.col(…)。写多个列名、或者选择器（cs.numeric()），就是多列——后面的节点对每一列都做一遍",
  "category": "输入",
  "order": 10,
  "expr": true,
  "params": [
    {"name": "columns", "type": "columns", "label": "列", "required": true, "hint": "\"金额\"；多列 \"单价\", \"数量\"；按类型 cs.numeric()"}
  ]
}

从表里取列，表达式网络的起点：pl.col("金额")。

写好几个列名、或者用选择器（cs.numeric() 全部数值列、cs.string() 全部文字列、cs.map() 全部映射列、cs.starts_with("金额")…）就是“多列”：后面接的每个节点对这几列各做一遍，输出列也是多列，名字是各自原来的列名（相当于直接改这几列；要另起名字，在输出列上选 suffix 加后缀）。

@parameters
columns:
    列名加引号："金额"。点旁边的“选列”可以勾列、按类型 / 列名 / 位置加条件（选择器），看着会选到哪几列再填进来。

@examples
全部数值列都乘 100（with_columns 里，直接改原列）：
{{{
col（cs.numeric()）→ arith（* 100）→ 输出列（名字留空）
}}}

@related
lit, element, output

@polars
pl.col
"""
items = ch("columns")
names = [c for c in items if isinstance(c, str)]
others = [c for c in items if not isinstance(c, str)]
if not others:
    out = pl.col(*names)
elif len(items) == 1:
    out = others[0]
elif all(isinstance(c, cs.Selector) for c in others):
    out = cs.by_name(*names) if names else others.pop(0)
    for c in others:
        out = out | c
else:
    raise ValueError("写多个的时候只能是列名或选择器（cs.…）")
