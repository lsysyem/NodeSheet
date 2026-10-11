r"""
{
  "desc": "常量：pl.lit(…)，每行都是这个值",
  "category": "输入",
  "order": 20,
  "expr": true,
  "params": [
    {"name": "value", "type": "expr", "label": "值", "required": true, "insert": "none", "hint": "100、\"文字\"、True、None"},
    {"name": "dtype", "type": "dtype", "label": "类型", "hint": "留空 = 按值猜；Int32、Decimal(38, 2)、Datetime(\"ms\")…"}
  ]
}

常量：每一行都是同一个值，pl.lit(…)。常用来加一列固定的值，或者和别的表达式运算。

运算节点（arith / compare）没连线的端口直接填数字就行，不一定要 lit。

@parameters
value:
    100、3.14、"文字"（要加引号）、True、None。
dtype:
    指定类型，留空 = 按值猜。polars 的全部类型都行（写法见 cast 节点），右边的下拉帮着填。

@examples
加一列“来源”，每行都是“门诊”：lit（"门诊"）→ 输出列（名字 来源）。

@related
col, arith

@polars
pl.lit
"""
out = pl.lit(ch("value"), dtype=ch("dtype"))
