r"""
{
  "desc": "SQL 查询：第 1 个输入叫 self（也叫 df），其余叫 df2 ~ df4",
  "category": "代码",
  "order": 10,
  "inputs": 4,
  "min_inputs": 1,
  "params": [
    {"name": "query", "type": "code", "label": "SQL", "default": "SELECT * FROM self LIMIT 100", "required": true}
  ]
}

用 SQL 查询。第 1 个输入叫 self（也可以叫 df），第 2 ~ 4 个叫 df2、df3、df4，能 JOIN、UNION。polars 的 SQL 支持常见的 SELECT / WHERE / GROUP BY / ORDER BY / 窗口函数和很多函数，不支持的写法会报错。

分组还能一次出好几层小计：GROUP BY ROLLUP("城市", "类") 是“城市 + 类”“城市”“总计”三层，CUBE("城市", "类") 是全部组合，GROUPING SETS (("城市"), ("类"), ()) 自己列。小计行里没参与分组的列是空值。

中文列名、带空格的列名用双引号："金额"；文字值用单引号：'北京'。

@parameters
query:
    SQL 语句。编辑框有补全和高亮。

@inputs
1:
    self / df。
2:
    df2。

@examples
{{{
SELECT "医院", SUM("金额") AS "总金额"
FROM self
WHERE "年份" = 2024
GROUP BY "医院"
ORDER BY "总金额" DESC
}}}

各城市、各类的金额，带小计和总计（小计行的“类”是空值，总计行两列都是空值）：
{{{
SELECT "城市", "类", SUM("金额") AS "金额" FROM self
GROUP BY ROLLUP("城市", "类")
ORDER BY "城市", "类"
}}}

@related
code, expr

@polars
pl.SQLContext
"""
# 连了线的输入都能在 SQL 里用（JOIN / UNION…）
names = ["df", "df2", "df3", "df4"]
frames = {name: frame for name, frame in zip(names, inputs) if frame is not None}
frames["self"] = df
out = pl.SQLContext(frames=frames).execute(ch("query"))
