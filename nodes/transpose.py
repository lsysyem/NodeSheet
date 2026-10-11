r"""
{
  "desc": "行列转置（transpose），或每 N 行折成一组列（unstack）",
  "category": "变形",
  "order": 30,
  "inputs": 1,
  "params": [
    {"name": "mode", "type": "menu", "label": "方式", "mode": true, "options": ["transpose", "unstack"]},
    {"name": "include_header", "type": "toggle", "default": "True", "hint": "原来的列名作为第一列", "show": {"mode": ["transpose"]}},
    {"name": "header_name", "type": "text", "default": "column", "hint": "那一列叫什么", "show": {"mode": ["transpose"]}},
    {"name": "column_names", "type": "expr", "insert": "name", "hint": "新列名取自哪一列：\"名称\"；留空 = column_0…", "show": {"mode": ["transpose"]}},
    {"name": "step", "type": "int", "default": "2", "required": true, "hint": "每几行一组", "show": {"mode": ["unstack"]}},
    {"name": "how", "type": "menu", "options": ["vertical", "horizontal"], "hint": "vertical：每组竖着排成一列；horizontal：横着排成一行", "show": {"mode": ["unstack"]}},
    {"name": "columns", "type": "columns", "hint": "只折这些列，留空 = 全部", "show": {"mode": ["unstack"]}},
    {"name": "fill_values", "type": "expr", "insert": "none", "hint": "最后一组不满时填什么，留空 = 空值", "show": {"mode": ["unstack"]}}
  ]
}

* transpose：行列互换，第一行变成第一列……原来的列名可以留成第一列，新的列名可以取自某一列
* unstack：每 step 行折成一组，把一长列数据按固定行数分段排成好几列（或好几行）

两种都要先把上游整张表算出来；行很多时转置出来的列也很多，会慢。

@parameters
mode:
    转置还是折叠。
include_header:
    原来的列名放进结果的第一列。
header_name:
    那一列叫什么。
column_names:
    转置后新列的名字取自哪一列，如 "名称"。留空 = column_0、column_1…
step:
    每几行一组。
how:
    vertical：每组竖着排成一列；horizontal：每组横着排成一行。
columns:
    只折这些列，留空 = 全部。
fill_values:
    最后一组不满时填什么，留空 = 空值。

@related
pivot, unpivot

@polars
transpose: DataFrame.transpose
unstack: DataFrame.unstack
"""
# 结果的列数取决于行数，只能先算出来
if ch("mode") == "unstack":
    out = df.collect().unstack(
        step=ch("step"),
        how=ch("how"),
        columns=ch("columns") or None,
        fill_values=ch("fill_values"),
    )
else:
    out = df.collect().transpose(
        include_header=ch("include_header"),
        header_name=ch("header_name") or "column",
        column_names=ch("column_names"),
    )
