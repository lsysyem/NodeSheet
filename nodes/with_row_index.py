r"""
{
  "desc": "加一列行号（with_row_index），放在最前面",
  "category": "列",
  "order": 40,
  "inputs": 1,
  "params": [
    {"name": "name", "type": "text", "default": "index", "hint": "行号列叫什么"},
    {"name": "offset", "type": "int", "default": "0", "hint": "从几开始数"}
  ]
}

在最前面加一列行号：0, 1, 2…。排序、筛选之前加上，能记住每一行原来是第几行。

@parameters
name:
    行号列叫什么。
offset:
    从几开始数，填 1 就是 1, 2, 3…

@related
slice

@polars
DataFrame.with_row_index
"""
out = df.with_row_index(ch("name") or "index", offset=ch("offset") or 0)
