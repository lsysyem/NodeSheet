r"""
{
  "desc": "类别列展开成 0 / 1 列（one-hot）",
  "category": "变形",
  "order": 50,
  "inputs": 1,
  "params": [
    {"name": "columns", "type": "columns", "hint": "留空 = 所有列"},
    {"name": "separator", "type": "text", "default": "_", "strip": false, "hint": "列名和值之间夹什么"},
    {"name": "drop_first", "type": "toggle", "default": "False", "hint": "每个类别少一列"}
  ]
}

把类别列展开成 0 / 1 列（独热编码，one-hot）：列里每个不同的值一列，是这个值的行为 1，其余为 0。做回归、机器学习的特征时常用。

要先把上游整张表算出来（有哪些列取决于数据）。

@parameters
columns:
    要展开的列。留空 = 全部列（数值列也会展开，一般要填）。
separator:
    新列名是“原列名 + 它 + 值”，如 性别_男。
drop_first:
    每个类别少一列（第一个值不单独成列），避免回归里的共线性。

@related
pivot

@polars
DataFrame.to_dummies
"""
out = df.collect().to_dummies(ch("columns") or None, separator=ch("separator"), drop_first=ch("drop_first"))
