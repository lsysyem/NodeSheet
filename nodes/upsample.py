r"""
{
  "desc": "按时间补齐缺的行（upsample）：每隔 every 一行，补出来的行其他列是空值",
  "category": "行",
  "order": 70,
  "inputs": 1,
  "params": [
    {"name": "time_column", "type": "columns", "pick": "one", "required": true, "hint": "时间列：\"日期\""},
    {"name": "every", "type": "text", "default": "1d", "required": true, "hint": "间隔：1d / 1w / 1mo / 1h / 30m"},
    {"name": "group_by", "type": "columns", "pick": "names", "hint": "每组各自补齐：\"医院\""},
    {"name": "maintain_order", "type": "toggle", "default": "True", "hint": "分组按第一次出现的顺序排"}
  ]
}

按时间把缺的行补出来：时间列每隔 every 一行，原来没有的时间新增一行，其他列是空值。之后常接 fill_null 把值补上。

时间列要是日期或时间类型。节点会先按（分组列、时间列）排好序，要先把上游整张表算出来。

@parameters
time_column:
    时间列，只用第一个。
every:
    间隔：1d 一天、1w 一周、1mo 一个月、1q 一季度、1y 一年、1h 一小时、30m 三十分钟。
group_by:
    每组各自补齐，如每家医院的日期各自连续。
maintain_order:
    结果里各组按第一次出现的顺序排。

@examples
每天的销售额，没卖的日子补成 0：upsample（every = 1d）→ fill_null（value = 0）。

@related
fill_null, expr

@polars
DataFrame.upsample
"""
# upsample 要求时间列在组内有序，只有 DataFrame 有
time = ch("time_column")[0]
groups = ch("group_by")
out = df.sort([*groups, time]).collect().upsample(
    time,
    every=ch("every"),
    group_by=groups or None,
    maintain_order=ch("maintain_order"),
)
