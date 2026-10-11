r"""
{
  "desc": "比较：a == b、a != b、a > b、a >= b、a < b、a <= b，得到真假",
  "category": "比较",
  "order": 1,
  "expr": true,
  "params": [
    {"name": "a", "type": "expr", "port": true, "required": true, "hint": "没连线时：pl.col(\"列\")、数字、\"文字\""},
    {"name": "op", "type": "menu", "label": "比较", "mode": true, "search": false, "options": ["==", "!=", ">", ">=", "<", "<="]},
    {"name": "b", "type": "expr", "port": true, "required": true, "hint": "没连线时：数字、\"文字\"、pl.col(\"列\")"}
  ]
}

比较 a 和 b，得到真 / 假，接到“条件”输出、when、logic 上。

文字要加引号："北京"。和空值比较的结果是空值；判断是不是空值用 is_null 节点。

@parameters
a:
    左边。
op:
    == 等于、!= 不等于、> 大于、>= 大于等于、< 小于、<= 小于等于。
b:
    右边。

@related
logic, when, condition

@polars
==: Expr.eq
!=: Expr.ne
>: Expr.gt
>=: Expr.ge
<: Expr.lt
<=: Expr.le
"""
a, b = ch("a"), ch("b")
if not isinstance(a, pl.Expr):
    a = pl.lit(a)
op = ch("op")
if op == "==":
    out = a == b
elif op == "!=":
    out = a != b
elif op == ">":
    out = a > b
elif op == ">=":
    out = a >= b
elif op == "<":
    out = a < b
else:
    out = a <= b
