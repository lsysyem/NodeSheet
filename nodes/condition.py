r"""
{
  "desc": "条件：filter 只留下满足的行；group_by 里是 having，只留下满足的分组（在每组里算，如 pl.len() > 1）。有多个条件时要都满足",
  "category": "输出",
  "order": 30,
  "expr": true,
  "role": "条件",
  "params": [
    {"name": "value", "type": "expr", "label": "条件", "port": true, "required": true, "hint": "没连线时也可以直接写：pl.col(\"金额\") > 100"}
  ]
}

条件：真 / 假的表达式。

* 上下文 filter：只留下条件为真的行
* 上下文 group_by：having——在每组里算，只留下满足的分组，如“行数 > 1”（len → compare）

有几个条件节点时要都满足（且）。要“或”，先用 logic 节点（|）合成一个。

@parameters
value:
    接 compare、logic、is_null、str.contains… 这类得到真假的节点。

@examples
金额大于 100、而且城市是北京：
{{{
col "金额" → compare（> 100）     → 条件
col "城市" → compare（== "北京"） → 条件
}}}

@related
compare, logic, output

@polars
DataFrame.filter
"""
out = ch("value")
