r"""
{
  "desc": "聚合：一组（group_by 里）或整列（select 里）得到一个值",
  "category": "聚合",
  "order": 1,
  "expr": true,
  "params": [
    {"name": "input", "type": "expr", "label": "输入", "port": true, "required": true, "hint": "没连线时：pl.col(\"列\")"},
    {"name": "function", "type": "menu", "label": "函数", "mode": true, "options": ["sum", "mean", "median", "min", "max", "count", "n_unique", "first", "last", "std", "var", "len", "implode"]}
  ]
}

聚合：很多行变成一个值。

* group_by（分组后 agg）的网络里：每组得到一个值
* select 里：整列得到一个值（结果只有一行）
* with_columns 里：整列的值铺满每一行（如算占比时的分母）

函数：

* sum 求和、mean 平均、median 中位数、min / max 最小 / 最大
* count 非空值的个数、len 行数（含空值）、n_unique 不同的值有几个
* first / last 第一个 / 最后一个
* std / var 标准差 / 方差
* implode 整组收成一个列表

@parameters
input:
    要聚合的值。
function:
    聚合函数。

@examples
每家医院的总金额：expr 节点上下文 group_by，网络里 col "金额" → agg（sum）→ 输出列。
每行金额的占比：col "金额" 接 arith（/）的 a，col "金额" → agg（sum）接同一个 arith 的 b。

@related
expr, over

@polars
sum: Expr.sum
mean: Expr.mean
median: Expr.median
min: Expr.min
max: Expr.max
count: Expr.count
n_unique: Expr.n_unique
first: Expr.first
last: Expr.last
std: Expr.std
var: Expr.var
len: Expr.len
implode: Expr.implode
"""
out = getattr(ch("input"), ch("function"))()
