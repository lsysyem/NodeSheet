r"""
{
  "desc": "选择 / 删除列（select / drop）",
  "category": "列",
  "order": 10,
  "inputs": 1,
  "params": [
    {"name": "mode", "type": "menu", "label": "方式", "mode": true, "options": ["select", "drop"]},
    {"name": "columns", "type": "columns", "label": "列或表达式", "required": true, "hint": "\"医院\", pl.col(\"金额\") * 2；按类型选写 cs.numeric()"}
  ]
}

挑出要的列（select），或删掉不要的列（drop）。

列可以写列名，也可以写选择器：cs.numeric() 全部数值列、cs.string() 全部文字列、cs.starts_with("金额") 名字以“金额”开头的列；选择器能用 | & - ~ 组合。

要算新列、改列的值，用 expr 节点（上下文 select / with_columns）。

@parameters
mode:
    * select：只留这些列，按写的顺序排（也能用来调列的顺序）
    * drop：删掉这些列，其余不动
columns:
    逗号分开。点旁边的“选列”可以勾列、排除列、按条件（选择器）选，确定后写进来。

@examples
留下“医院”和全部数值列：
{{{
方式        select
列或表达式  "医院", cs.numeric()
}}}

@related
rename, cast_columns, expr

@polars
select: DataFrame.select
drop: DataFrame.drop
"""
out = getattr(df, ch("mode"))(*ch("columns"))
