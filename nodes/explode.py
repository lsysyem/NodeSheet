r"""
{
  "desc": "拆开嵌套列：列表拆成多行（explode）/ 结构体拆成多列（unnest）",
  "category": "变形",
  "order": 40,
  "inputs": 1,
  "params": [
    {"name": "mode", "type": "menu", "label": "方式", "mode": true, "options": ["explode", "unnest"]},
    {"name": "columns", "type": "columns", "required": true, "hint": "\"标签\""}
  ]
}

拆开嵌套的列：

* explode：列表列的每个元素一行，其他列跟着重复。如“标签”是 ["a", "b"] 的那一行变成两行
* unnest：结构体（struct）列的每个字段变成一列

常和表达式里的 str.split（文字拆成列表）、struct 一起用。

@parameters
mode:
    拆成行还是拆成列。
columns:
    要拆的列。explode 填多列时，这几列同一行的列表要一样长。

@examples
“标签”是逗号分开的文字：先用 expr 节点把它 str.split(",") 成列表，再接 explode。

@related
expr, unpivot

@polars
explode: DataFrame.explode
unnest: DataFrame.unnest
"""
out = getattr(df, ch("mode"))(ch("columns"))
