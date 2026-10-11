r"""
{
  "desc": "逻辑：且（&）、或（|）、异或（^）、非（~，只用 a）",
  "category": "逻辑",
  "order": 1,
  "expr": true,
  "params": [
    {"name": "a", "type": "expr", "port": true, "required": true},
    {"name": "op", "type": "menu", "label": "逻辑", "mode": true, "search": false, "options": ["&", "|", "^", "~"]},
    {"name": "b", "type": "expr", "port": true, "hint": "“非”不用"}
  ]
}

真假值的组合：& 且（两个都真才真）、| 或（有一个真就真）、^ 异或（正好一个真）、~ 非（只用 a，真假反过来）。

@parameters
a:
    真假值。
op:
    逻辑运算。
b:
    另一个真假值；“非”不用。

@related
compare, condition, when

@polars
&: Expr.and_
|: Expr.or_
^: Expr.xor
~: Expr.not_
"""
a, b = ch("a"), ch("b")
op = ch("op")
if op == "~":
    out = ~a
elif b is None:
    raise ValueError("b 还没有连接")
elif op == "&":
    out = a & b
elif op == "|":
    out = a | b
else:
    out = a ^ b
