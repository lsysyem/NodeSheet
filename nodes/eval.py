r"""
{
  "desc": "对列表 / 数组里的每个元素算（list.eval / filter / agg，arr.eval / agg）：双击进去，用 element 节点连到“结果”上",
  "category": "列表 list",
  "order": 1,
  "expr": true,
  "network": true,
  "params": [
    {"name": "input", "type": "expr", "label": "输入", "port": true, "required": true, "hint": "列表列；没连线时：pl.col(\"标签\")"},
    {"name": "method", "type": "menu", "label": "方式", "mode": true, "options": ["list.eval", "list.filter", "list.agg", "arr.eval", "arr.agg"]},
    {"name": "parallel", "type": "toggle", "hint": "多线程算（列表很多时）", "show": {"method": ["list.eval"]}},
    {"name": "as_list", "type": "toggle", "hint": "结果是列表（不定长）而不是数组", "show": {"method": ["arr.eval"]}}
  ]
}

对列表（list）或数组（array）列里的每个元素做运算。双击（或回车）进去是一张小网络：element 节点代表“每个元素”，最后连到“结果”上。

* list.eval：每个元素换成算出来的值，结果还是列表
* list.filter：“结果”是条件，只留下满足的元素
* list.agg：“结果”是聚合，每个列表得到一个值（如 element → agg（sum））
* arr.eval / arr.agg：同上，用于定长的数组（Array）

@parameters
input:
    列表列。
method:
    方式。
parallel:
    列表很多时多线程算。
as_list:
    arr.eval 的结果是不定长的列表，而不是数组。

@examples
“分数”列是列表，每个列表只留及格的：方式 list.filter，网络里 element → compare（>= 60）→ 结果。

@related
element, result

@polars
list.eval: Expr.list.eval
list.filter: Expr.list.filter
list.agg: Expr.list.agg
arr.eval: Expr.arr.eval
arr.agg: Expr.arr.agg
"""
# outputs("结果")：网络里“结果”输出节点的值，是对每个元素（pl.element()）的表达式
results = outputs("结果")
if not results:
    raise ValueError("网络里要一个“结果”输出节点：双击进去，按 Tab 搜“结果”")
ns, method = ch("method").split(".")
target = getattr(ch("input"), ns)
if method == "filter":
    out = target.filter(results[0])
elif method == "agg":
    out = target.agg(results[0])
# 开关没开就不写：写成文字时（网络 → 文字）干净些
elif ns == "list":
    out = target.eval(results[0], parallel=True) if ch("parallel") else target.eval(results[0])
else:
    out = target.eval(results[0], as_list=True) if ch("as_list") else target.eval(results[0])
