r"""
{
  "desc": "整表统计：各列的摘要 / 空值数 / 合计 / 平均 / 分位数… / 相关系数矩阵（corr）",
  "category": "统计",
  "order": 40,
  "inputs": 1,
  "params": [
    {"name": "mode", "type": "menu", "label": "统计", "mode": true, "options": ["describe", "null_count", "count", "sum", "mean", "median", "min", "max", "std", "var", "product", "n_unique", "quantile", "corr"]},
    {"name": "quantile", "type": "float", "default": "0.5", "required": true, "hint": "0 ~ 1", "show": {"mode": ["quantile"]}}
  ]
}

整张表的统计：每列算一个值（或一组摘要），结果是一张小表。按组统计用 expr 节点（上下文 group_by）。

* describe：每列的个数、空值数、平均、标准差、最小、分位数、最大——拿到新表先看一眼
* null_count：每列有几个空值
* count / sum / mean / median / min / max / std / var / product：每列一个值
* n_unique：每列有几个不同的值
* quantile：每列的分位数
* corr：数值列两两之间的相关系数（皮尔逊），结果是一个方阵

@parameters
mode:
    算哪种统计。
quantile:
    分位数，0.5 = 中位数，0.9 = 90% 分位。

@related
expr, unique

@polars
describe: DataFrame.describe
null_count: DataFrame.null_count
count: DataFrame.count
sum: DataFrame.sum
mean: DataFrame.mean
median: DataFrame.median
min: DataFrame.min
max: DataFrame.max
std: DataFrame.std
var: DataFrame.var
product: DataFrame.product
n_unique: Expr.n_unique
quantile: DataFrame.quantile
corr: DataFrame.corr
"""
mode = ch("mode")
if mode == "n_unique":
    out = df.select(pl.all().n_unique())
elif mode == "corr":
    # 相关系数矩阵：只算数值列
    out = df.select(cs.numeric()).collect().corr()
elif mode == "product":
    out = df.collect().product()
elif mode == "quantile":
    out = df.quantile(ch("quantile"))
else:
    out = getattr(df, mode)()
