r"""
{
  "desc": "连接两张表（join / join_asof 就近匹配 / join_where 不等式 / update 用右表更新）",
  "category": "多表",
  "order": 10,
  "inputs": 2,
  "params": [
    {"name": "mode", "type": "menu", "label": "方式", "mode": true, "options": ["join", "join_asof", "join_where", "update"]},
    {"name": "on", "type": "columns", "hint": "两边同名的键：\"医院\"；或选择器 cs.string()", "show": {"mode": ["join", "join_asof", "update"]}},
    {"name": "left_on", "type": "columns", "hint": "键名不同时：左表的键", "show": {"mode": ["join", "join_asof", "update"]}},
    {"name": "right_on", "type": "columns", "pick_from": 2, "hint": "键名不同时：右表的键", "show": {"mode": ["join", "join_asof", "update"]}},
    {"name": "how", "type": "menu", "options": ["inner", "left", "right", "full", "semi", "anti", "cross"], "show": {"mode": ["join"]}},
    {"name": "strategy", "type": "menu", "options": ["backward", "forward", "nearest"], "show": {"mode": ["join_asof"]}},
    {"name": "by", "type": "columns", "hint": "两边先要相等的列，再在组内就近匹配", "show": {"mode": ["join_asof"]}},
    {"name": "tolerance", "type": "expr", "insert": "none", "hint": "最多差多少：10、\"3d\"", "show": {"mode": ["join_asof"]}},
    {"name": "predicates", "type": "exprs", "pick": "off", "required": true, "hint": "pl.col(\"时间\") >= pl.col(\"开始\"), pl.col(\"时间\") < pl.col(\"结束\")", "show": {"mode": ["join_where"]}},
    {"name": "update_how", "type": "menu", "label": "how", "options": ["left", "inner", "full"], "show": {"mode": ["update"]}},
    {"name": "include_nulls", "type": "toggle", "default": "False", "hint": "右表的空值也覆盖过去", "show": {"mode": ["update"]}},
    {"name": "suffix", "type": "text", "default": "_right", "hint": "右表重名列的后缀", "show": {"mode": ["join", "join_asof", "join_where"]}}
  ]
}

把两张表按键连起来：左表接第 1 个输入，右表接第 2 个输入。

* join：键相等的行配在一起；配不上的怎么办看 how
* join_asof：就近匹配。键（通常是时间）不要求相等，按 strategy 找最近的一行，如每笔成交配上当时最新的报价。两张表都要按键排好序
* join_where：按不等式条件连接，如“时间在开始和结束之间”
* update：用右表的值更新左表：键对上的行，右表不是空值的列覆盖左表

@parameters
mode:
    哪种连接。
on:
    两张表同名的键，可以几个。也可以写选择器：cs.string() 是全部文字列（update 不行）。
left_on:
    键名不一样时，左表用哪几列……
right_on:
    ……右表用哪几列，和 left_on 一一对应。
how:
    * inner：只留两边都有的
    * left：左表全留，右表配不上的填空值（最常用）
    * right：右表全留
    * full：两边都全留
    * semi：只留左表里能在右表配上的行，不加右表的列（“在右表里”）
    * anti：只留左表里在右表配不上的行（“不在右表里”）
    * cross：两两配对（笛卡尔积），不用键
strategy:
    join_asof 找哪一行：backward 不晚于它的最近一行，forward 不早于它的最近一行，nearest 最近的。
by:
    join_asof 先要求这些列相等（如同一只股票），再在组里就近匹配。
tolerance:
    join_asof 最多差多少，超过算配不上：数字，或时间间隔 "3d"。
predicates:
    join_where 的条件，逗号分开，要都满足。
update_how:
    update 时：left 左表全留；inner 只留配上的行；full 右表多出来的行也加进来。
include_nulls:
    update 时右表的空值也覆盖过去（默认空值不覆盖）。
suffix:
    右表和左表重名的列，名字后面加什么。

@inputs
1:
    左表。
2:
    右表。

@examples
订单表（左）配上客户表（右）的客户名称：
{{{
方式  join
on    "客户编号"
how   left
}}}
找出没下过单的客户：客户表接左边、订单表接右边，how 选 anti。

@related
concat, create, read

@polars
join: DataFrame.join
join_asof: DataFrame.join_asof
join_where: DataFrame.join_where
update: DataFrame.update
"""
mode = ch("mode")
suffix = ch("suffix") or "_right"


def keys(single=False):
    on, left, right = ch("on"), ch("left_on"), ch("right_on")
    if on:
        found = {"on": on}
    elif left and right:
        found = {"left_on": left, "right_on": right}
    else:
        raise ValueError("填写 on，或同时填写 left_on 和 right_on")
    # join_asof 只认一个键
    return {name: value[0] for name, value in found.items()} if single else found


if mode == "join_asof":
    out = df.join_asof(
        df2,
        **keys(single=True),
        by=ch("by") or None,
        strategy=ch("strategy"),
        tolerance=ch("tolerance"),
        suffix=suffix,
    )
elif mode == "join_where":
    out = df.join_where(df2, *ch("predicates"), suffix=suffix)
elif mode == "update":
    out = df.update(df2, **keys(), how=ch("update_how"), include_nulls=ch("include_nulls"))
elif ch("how") == "cross":
    out = df.join(df2, how="cross", suffix=suffix)
else:
    out = df.join(df2, **keys(), how=ch("how"), suffix=suffix)
