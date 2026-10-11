r"""
{
  "desc": "条件取值：pl.when(条件).then(值)…otherwise(否则)；值写文字要加引号",
  "category": "条件",
  "order": 1,
  "expr": true,
  "params": [
    {"name": "condition", "type": "expr", "label": "条件 1", "port": true, "required": true, "hint": "pl.col(\"金额\") > 100"},
    {"name": "then", "type": "expr", "label": "值 1", "port": true, "required": true, "hint": "\"大单\"、1、pl.col(\"列\")"},
    {"name": "condition2", "type": "expr", "label": "条件 2", "port": true},
    {"name": "then2", "type": "expr", "label": "值 2", "port": true},
    {"name": "condition3", "type": "expr", "label": "条件 3", "port": true},
    {"name": "then3", "type": "expr", "label": "值 3", "port": true},
    {"name": "otherwise", "type": "expr", "label": "否则", "port": true, "hint": "都不满足时；留空 = 空值"}
  ]
}

按条件取不同的值，像 Excel 的 IF、SQL 的 CASE WHEN：从上往下看，第一个满足的条件取它对应的值，都不满足取“否则”。

值里直接写的文字（"大单"）当文字，不当列名；要取某列的值写 pl.col("列") 或接上游节点。

@parameters
condition:
    第一个条件（真 / 假）。
then:
    条件 1 满足时的值。
condition2:
    可选：前面的都不满足时再看它。
then2:
    条件 2 满足时的值。
condition3:
    可选。
then3:
    条件 3 满足时的值。
otherwise:
    都不满足时的值，留空 = 空值。

@examples
{{{
条件 1  pl.col("金额") > 10000    值 1  "大单"
条件 2  pl.col("金额") > 1000     值 2  "中单"
否则    "小单"
}}}
超过 3 个条件时，把另一个 when 接到“否则”上。

@related
compare, logic

@polars
pl.when
"""


def value(v):
    # then / otherwise 里的字符串 polars 当成列名；这里当文字
    return pl.lit(v) if isinstance(v, str) else v


chain = pl.when(ch("condition")).then(value(ch("then")))
for condition, then in ((ch("condition2"), ch("then2")), (ch("condition3"), ch("then3"))):
    if condition is not None:
        chain = chain.when(condition).then(value(then))
out = chain if ch("otherwise") is None else chain.otherwise(value(ch("otherwise")))
