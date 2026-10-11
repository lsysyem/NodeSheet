r"""
{
  "desc": "长表转宽表（透视）",
  "category": "变形",
  "order": 10,
  "inputs": 1,
  "params": [
    {"name": "on", "type": "columns", "required": true, "hint": "值变成列名的列：\"月份\""},
    {"name": "index", "type": "columns", "hint": "保持为行的列：\"医院\""},
    {"name": "values", "type": "columns", "hint": "填进格子的列：\"金额\""},
    {"name": "aggregate_function", "type": "menu", "options": ["first", "sum", "mean", "median", "min", "max", "len", "last"]}
  ]
}

长表转宽表（透视表）：on 列里每个不同的值变成一列，格子里填 values 列的值；同一格有多个值时按 aggregate_function 合并。

结果有哪些列取决于数据，所以要先把上游整张表算出来。

@parameters
on:
    它的值变成新的列名，如 "月份"。
index:
    留作行的列，如 "医院"，每个不同的值一行。留空 = on 和 values 以外的全部列。index 和 values 至少填一个。
values:
    填进格子的列，如 "金额"。留空 = on 和 index 以外的全部列。
aggregate_function:
    一格里有多个值时怎么合：first 第一个、sum 求和、mean 平均、len 个数…

@examples
每家医院每月的金额：
{{{
on                  "月份"
index               "医院"
values              "金额"
aggregate_function  sum
}}}

@related
unpivot, transpose, expr

@polars
DataFrame.pivot
"""
# 透视后的列名取决于数据，只能先算出来
out = df.collect().pivot(
    ch("on"),
    index=ch("index") or None,
    values=ch("values") or None,
    aggregate_function=ch("aggregate_function"),
)
