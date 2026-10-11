r"""
{
  "desc": "转换类型：cast",
  "category": "类型",
  "order": 1,
  "expr": true,
  "params": [
    {"name": "input", "type": "expr", "label": "输入", "port": true, "required": true, "hint": "没连线时：pl.col(\"列\")"},
    {"name": "dtype", "type": "dtype", "label": "类型", "default": "Float64", "required": true, "hint": "Int64、Decimal(38, 2)、Datetime(\"ms\")、List(String)…；右边下拉列着全部类型"},
    {"name": "strict", "type": "toggle", "default": "True", "hint": "转不了就报错；关掉 = 转不了的变成空"}
  ]
}

转成另一种类型：文字转数字、数字转文字、日期时间转日期…

文字转日期 / 时间用 str.to_date、str.to_datetime、str.to_time 节点（polars 2.0 起 cast 不能直接把文字转成日期；表格级的 cast_columns 会自动换成 str.strptime）。

@parameters
input:
    要转的值。
dtype:
    目标类型，polars 的全部类型都行，右边的下拉帮着填写法（写不写 pl. 都行）：
    * 整数 Int8 ~ Int128、UInt8 ~ UInt128；小数 Float16 / Float32 / Float64；定点小数 Decimal(精度, 小数位)
    * String 文字、Binary 字节、Boolean 真假、Categorical 分类、Enum(["a", "b"]) 固定几个值的分类
    * Date 日期、Time 时间、Datetime("ms", "Asia/Shanghai") 日期时间（单位 ms / us / ns，时区可以不写）、Duration("us") 时长（输入是文字时不行，见上）
    * List(Int64) 列表、Array(Int64, 2) 定长数组、Struct({"a": Int64, "b": String}) 结构、Map(String, Int64) 键值对
    * Null、Object、Extension("名字", Int64)、Unknown
strict:
    转不了（"12元" 转数字）时：开着报错，关掉变成空值。

@related
cast_columns

@polars
Expr.cast
"""
out = ch("input").cast(ch("dtype")) if ch("strict") else ch("input").cast(ch("dtype"), strict=False)
