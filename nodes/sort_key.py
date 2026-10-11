r"""
{
  "desc": "排序键：sort 按它排（多个键按画布上从上到下，先按上面的排；节点上写着是第几键）",
  "category": "输出",
  "order": 40,
  "expr": true,
  "role": "排序",
  "params": [
    {"name": "value", "type": "expr", "label": "值", "port": true, "required": true, "hint": "没连线时也可以直接写：pl.col(\"金额\")"},
    {"name": "order", "type": "menu", "label": "方向", "mode": true, "options": ["从小到大", "从大到小"]},
    {"name": "nulls_last", "type": "toggle", "label": "空值最后", "hint": "空值排最后（默认在最前）"}
  ]
}

排序键：sort 按它排。几个排序键时先按上面的排，一样的再按下面的排；节点上面写着它是第几键，挪动节点会改变先后。

@parameters
value:
    按什么排：可以是列，也可以是算出来的值（如 str.len_chars 按文字长度排）。
order:
    从小到大（升序）还是从大到小（降序）。文字按字典序，真假值假在前。
nulls_last:
    空值排在最后（默认在最前）。

@related
output

@polars
DataFrame.sort
"""
out = (ch("value"), ch("order") == "从大到小", ch("nulls_last"))
