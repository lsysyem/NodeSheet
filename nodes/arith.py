r"""
{
  "desc": "运算：a + b、a - b、a * b、a / b、整除、取余、乘方",
  "category": "运算",
  "order": 1,
  "expr": true,
  "params": [
    {"name": "a", "type": "expr", "port": true, "required": true, "hint": "没连线时：数字，或 pl.col(\"列\")"},
    {"name": "op", "type": "menu", "label": "运算", "mode": true, "search": false, "options": ["+", "-", "*", "/", "//", "%", "**"]},
    {"name": "b", "type": "expr", "port": true, "required": true, "hint": "没连线时：数字，或 pl.col(\"列\")"}
  ]
}

运算：a 和 b 按选的运算符算。a、b 可以接上游节点，也可以直接填数字。

* + - * /：加减乘除，/ 总是得到小数
* //：整除（向下取整）
* %：取余数
* **：乘方，a 的 b 次方

两列文字相加（+）就是拼接；日期减日期得到时长。

@parameters
a:
    左边。
op:
    运算符。
b:
    右边。

@examples
金额 = 单价 × 数量：col "单价" 接 a，col "数量" 接 b，运算选 *。

@related
compare, logic, agg

@polars
+: Expr.add
-: Expr.sub
*: Expr.mul
/: Expr.truediv
//: Expr.floordiv
%: Expr.mod
**: Expr.pow
"""
a, b = ch("a"), ch("b")
if not isinstance(a, pl.Expr):
    a = pl.lit(a)
op = ch("op")
if op == "+":
    out = a + b
elif op == "-":
    out = a - b
elif op == "*":
    out = a * b
elif op == "/":
    out = a / b
elif op == "//":
    out = a // b
elif op == "%":
    out = a % b
else:
    out = a**b
