r"""
{
  "desc": "不读文件，直接造一张表：用表达式算（pl.select）/ 写字典（from_dict）/ 贴 polars 打印的表格（from_repr）/ 粘贴从 Excel 复制的表格（read_clipboard）",
  "category": "IO",
  "order": 30,
  "inputs": 0,
  "params": [
    {"name": "mode", "type": "menu", "label": "方式", "mode": true, "options": ["select", "from_dict", "from_repr", "read_clipboard"]},
    {"name": "exprs", "type": "exprs", "label": "列", "required": true, "hint": "pl.int_range(1, 13).alias(\"月\"), pl.date_range(pl.date(2024, 1, 1), pl.date(2024, 12, 31), \"1d\").alias(\"日期\")", "show": {"mode": ["select"]}},
    {"name": "data", "type": "dict", "label": "列名: 值", "required": true, "insert": "none", "hint": "\"城市\": [\"北京\", \"上海\"], \"人数\": [3, 5]", "show": {"mode": ["from_dict"]}},
    {"name": "table", "type": "code", "label": "表格", "required": true, "hint": "把 print(df) 打出来的表格整个贴进来", "show": {"mode": ["from_repr"]}},
    {"name": "paste", "type": "button", "label": "粘贴剪贴板", "paste": "pasted", "show": {"mode": ["read_clipboard"]}},
    {"name": "pasted", "type": "code", "label": "复制的表格", "required": true, "hint": "在 Excel 里复制一块，点“粘贴剪贴板”；也可以直接粘贴进来", "show": {"mode": ["read_clipboard"]}},
    {"name": "has_header", "type": "toggle", "default": "True", "hint": "第一行是表头", "show": {"mode": ["read_clipboard"]}},
    {"name": "separator", "type": "text", "default": "\\t", "strip": false, "hint": "分隔符：Excel、网页复制的是制表符 \\t", "show": {"mode": ["read_clipboard"]}}
  ]
}

不读文件，凭空造一张表：测试数据、日期表、对照表（编码 → 名称）。造好的表可以接到 join 的右边当维表。

* select：用表达式算出每一列（pl.select）。几列的长度要一样，或者是一个值（自动铺满）
* from_dict：写一个字典，键是列名、值是这一列的列表
* from_repr：把 print(df) 打印出来的表格（带边框的那种）整个贴进来，列名和类型都认得出
* read_clipboard：在 Excel / WPS / 网页里复制一块表格，点“粘贴剪贴板”。复制的文字存在节点里（也存进工程文件），之后剪贴板变了不影响它；要换数据再点一次

@parameters
mode:
    怎么造。
exprs:
    一个或多个表达式，逗号分开，每个用 .alias("列名") 起名。
data:
    "列名": [值, …]，几列的值个数要一样。
table:
    polars 打印的表格原样贴进来，包括表头下面的类型那一行。
paste:
    把剪贴板里现在的文字放进“复制的表格”（覆盖原来的），然后重新运行。
pasted:
    复制来的表格：一行一条，列之间是制表符。可以直接改。
has_header:
    第一行是表头；关掉的话列名是 column_1、column_2…
separator:
    列之间的分隔符，制表符写 \t。从 Excel、网页复制的都是制表符。

@examples
2024 年每一天：
{{{
方式  select
列    pl.date_range(pl.date(2024, 1, 1), pl.date(2024, 12, 31), "1d").alias("日期")
}}}
对照表：
{{{
方式      from_dict
列名: 值  "编码": ["A", "B"], "名称": ["门诊", "住院"]
}}}

@related
read, join

@polars
select: pl.select
from_dict: pl.from_dict
from_repr: pl.from_repr
read_clipboard: pl.read_clipboard
"""
mode = ch("mode")
if mode == "from_dict":
    out = pl.from_dict(ch("data"))
elif mode == "from_repr":
    out = pl.from_repr(ch("table"))
elif mode == "read_clipboard":
    # 和 pl.read_clipboard 一样按 CSV 读，只是文字取自节点参数（点“粘贴剪贴板”时放进来的），结果不随剪贴板变
    import io

    separator = ch("separator") or "\t"
    out = pl.read_csv(
        io.StringIO(ch("pasted")),
        separator="\t" if separator == "\\t" else separator,
        has_header=ch("has_header"),
    )
else:
    out = pl.select(ch("exprs"))
