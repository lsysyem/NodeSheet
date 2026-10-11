r"""
{
  "desc": "输出列：with_columns / select / group_by 的结果里的一列（多个输出按画布上从上到下排）",
  "category": "输出",
  "order": 10,
  "expr": true,
  "role": "列",
  "params": [
    {"name": "value", "type": "expr", "label": "值", "port": true, "required": true, "hint": "没连线时也可以直接写：pl.col(\"单价\") * 2"},
    {"name": "naming", "type": "menu", "label": "列名", "mode": true, "search": false, "options": ["alias", "keep", "prefix", "suffix"]},
    {"name": "name", "type": "text", "label": "名字", "hint": "结果列叫什么；留空 = 表达式自己的名字（通常是用到的第一列，会覆盖它）", "show": {"naming": ["alias"]}},
    {"name": "affix", "type": "text", "label": "加的字", "strip": false, "required": true, "hint": "多列时每列都加：_新", "show": {"naming": ["prefix", "suffix"]}}
  ]
}

输出列：网络算出来的值交给 expr 节点，成为结果里的一列（with_columns / select / group_by 的 agg 用它）。

一个网络可以有好几个输出列，结果里按画布上从上到下排。上游是多列（col 写了几列）时，一个输出就是好几列。

@parameters
value:
    接上游节点；没连线时也可以直接写表达式。
naming:
    * alias：起名字。留空 = 表达式自己的名字（通常是用到的第一列，所以会覆盖那一列）
    * keep：用原来的列名（中间改过名也恢复）
    * prefix / suffix：在原列名前 / 后加字，多列时每列都加
name:
    结果列的名字。
affix:
    要加的字，如 _占比。

@related
key, condition, sort_key, col

@polars
alias: Expr.alias
keep: Expr.name.keep
prefix: Expr.name.prefix
suffix: Expr.name.suffix
"""
value = ch("value")
naming = ch("naming")
if naming == "alias":
    out = value.alias(ch("name")) if ch("name") else value
elif naming == "keep":
    out = value.name.keep()
else:
    out = getattr(value.name, naming)(ch("affix"))
