r"""
{
  "desc": "分组键：group_by 按它分组（多个键按画布上从上到下排）",
  "category": "输出",
  "order": 20,
  "expr": true,
  "role": "键",
  "params": [
    {"name": "value", "type": "expr", "label": "值", "port": true, "required": true, "hint": "没连线时也可以直接写：pl.col(\"城市\")"},
    {"name": "name", "type": "text", "label": "名字", "hint": "留空 = 原名"}
  ]
}

分组键：group_by 按它分组。几个分组键就是按几列一起分，结果里分组键在最前面，按画布上从上到下排。

分组键不一定是原来的列，也可以是算出来的值，如 col "日期" → dt.year 按年分组。

@parameters
value:
    接上游节点；没连线时也可以直接写，如 pl.col("城市")。
name:
    分组键在结果里叫什么，留空 = 原名。

@related
output, condition

@polars
DataFrame.group_by
"""
out = ch("value").alias(ch("name")) if ch("name") else ch("value")
