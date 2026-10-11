r"""
{
  "desc": "改列的类型（cast）：文字存的数字转成数字、整数转小数…；或把表对齐成给定的结构（match_to_schema）",
  "category": "列",
  "order": 30,
  "inputs": 1,
  "params": [
    {"name": "mode", "type": "menu", "label": "方式", "mode": true, "options": ["cast", "match_to_schema"]},
    {"name": "dtypes", "type": "dict", "pick": "select", "pick_dtype": true, "label": "列: 类型", "required": true, "hint": "\"数量\": pl.Int64, \"单价\": pl.Float64；不同列转不同类型：一组一组加；按类型选：cs.string(): pl.Float64；全部：cs.all(): pl.String", "show": {"mode": ["cast"]}},
    {"name": "strict", "type": "toggle", "default": "True", "hint": "转不了就报错；关掉 = 转不了的变成空值", "show": {"mode": ["cast"]}},
    {"name": "schema", "type": "dict", "pick_dtype": true, "label": "目标结构", "required": true, "hint": "全部列按顺序写：\"日期\": pl.Date, \"金额\": pl.Float64, \"时间\": pl.Datetime(\"us\")", "show": {"mode": ["match_to_schema"]}},
    {"name": "missing_columns", "type": "menu", "label": "缺的列", "options": ["raise", "insert"], "hint": "raise = 报错；insert = 补一列空值", "show": {"mode": ["match_to_schema"]}},
    {"name": "extra_columns", "type": "menu", "label": "多的列", "options": ["raise", "ignore"], "hint": "raise = 报错；ignore = 去掉", "show": {"mode": ["match_to_schema"]}},
    {"name": "integer_cast", "type": "menu", "options": ["forbid", "upcast"], "hint": "整数类型不一样：forbid = 报错；upcast = 往大转（Int32 → Int64）", "show": {"mode": ["match_to_schema"]}},
    {"name": "float_cast", "type": "menu", "options": ["forbid", "upcast"], "hint": "小数类型不一样：forbid = 报错；upcast = 往大转（Float32 → Float64）", "show": {"mode": ["match_to_schema"]}},
    {"name": "missing_struct_fields", "type": "menu", "options": ["raise", "insert"], "hint": "结构列里缺的字段：报错 / 补空值", "show": {"mode": ["match_to_schema"]}},
    {"name": "extra_struct_fields", "type": "menu", "options": ["raise", "ignore"], "hint": "结构列里多的字段：报错 / 去掉", "show": {"mode": ["match_to_schema"]}}
  ]
}

改列的类型，或者把整张表对齐成给定的结构。

* cast：改指定列的类型。最常见的是文字存的数字转成数字，或数字转成文字（编码前面的 0 不丢）
* match_to_schema：给出整张表应有的列和类型，表按它排列列的顺序；缺的列、多的列、整数 / 小数位数不一样时怎么办，各自选“报错”还是“补上 / 去掉 / 往大转”。几个文件合并（concat）前统一格式、或检查一张表是不是预期的样子时用

只转一部分值、或要先处理再转（去掉千分位的逗号再转数字），用 expr 节点里的 cast 表达式节点。

@parameters
mode:
    cast 改指定列的类型；match_to_schema 把整张表对齐成给定的结构。
dtypes:
    "列名": 类型，逗号分开。类型写 pl.Int64（整数）、pl.Float64（小数）、pl.String（文字）、pl.Date（日期）、pl.Datetime、pl.Boolean…
    不同列转不同的类型：选列框里一组一组加（选列、填类型、加一组）。写法里排后面的组说了算：两根选择器都选到同一列时，转成后面那组填的类型。
    键也可以是选择器：cs.string(): pl.Float64 把全部文字列转成小数。
    文字列转 pl.Date / pl.Datetime / pl.Time 时按 str.strptime 自动认格式（polars 2.0 起 cast 不能直接把文字转成日期）；格式特殊的用 expr 节点里的 str.to_date。
strict:
    转不了的值（如“12元”转数字）怎么办：开着就报错，关掉就变成空值。
schema:
    整张表应有的样子："列名": 类型，按想要的顺序写全部列。类型要写完整：时间写 pl.Datetime("us")，不能只写 pl.Datetime。
    它不做别的转换（文字不会变成数字），类型对不上就报错：要转换先用 cast 方式。
missing_columns:
    表里没有、结构里有的列：raise 报错；insert 补一列空值。
extra_columns:
    表里有、结构里没有的列：raise 报错；ignore 去掉。
integer_cast:
    整数列位数不一样（表里 Int32、结构里 Int64）：forbid 报错；upcast 转成结构里的（只能往大转）。
float_cast:
    小数列位数不一样：forbid 报错；upcast 转成结构里的（只能往大转）。
missing_struct_fields:
    结构（struct）列里缺的字段：raise 报错；insert 补空值。
extra_struct_fields:
    结构（struct）列里多的字段：raise 报错；ignore 去掉。

@examples
cast：
{{{
"数量": pl.Int64, "单价": pl.Float64, "编码": pl.String
}}}
match_to_schema：几个月的报表列不全、顺序不一，合并前统一成这四列（缺的列 insert、多的列 ignore）：
{{{
"日期": pl.Date, "医院": pl.String, "数量": pl.Int64, "金额": pl.Float64
}}}

@related
cast, select, concat, expr

@polars
cast: DataFrame.cast
cast: Expr.str.strptime
match_to_schema: LazyFrame.match_to_schema
"""
if ch("mode") == "match_to_schema":
    # polars 标着 unstable：以后的版本可能会改
    out = df.match_to_schema(
        ch("schema"),
        missing_columns=ch("missing_columns"),
        missing_struct_fields=ch("missing_struct_fields"),
        extra_columns=ch("extra_columns"),
        extra_struct_fields=ch("extra_struct_fields"),
        integer_cast=ch("integer_cast"),
        float_cast=ch("float_cast"),
    )
else:
    schema = df.collect_schema()
    # 键可以是列名、选择器或类型：先展开成 列名 → 类型
    casts = {}
    for key, dtype in ch("dtypes").items():
        if isinstance(key, str):
            casts[key] = dtype
        else:
            selector = key if cs.is_selector(key) else cs.by_dtype(key)
            casts.update(dict.fromkeys(cs.expand_selector(schema, selector), dtype))

    def to_temporal(name, dtype):
        base = getattr(dtype, "base_type", None)
        return schema.get(name) == pl.String and base is not None and base() in (pl.Date, pl.Datetime, pl.Time)

    # polars 2.0 起文字不能直接 cast 成日期 / 时间：这些列用 str.strptime（格式自动认）
    parsed = {name: dtype for name, dtype in casts.items() if to_temporal(name, dtype)}
    out = df.with_columns(pl.col(name).str.strptime(dtype, strict=ch("strict")) for name, dtype in parsed.items())
    out = out.cast({name: dtype for name, dtype in casts.items() if name not in parsed}, strict=ch("strict"))
