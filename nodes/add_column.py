r"""
{
  "desc": "增加一列：用界面选常用函数——文本的提取 / 格式、两列运算、日期取部分、条件列、常量 / 行号 / 复制列",
  "category": "列",
  "order": 15,
  "inputs": 1,
  "params": [
    {"name": "mode", "type": "menu", "label": "方式", "mode": true, "options": ["文本", "数学", "日期", "条件", "值"]},
    {"name": "name", "type": "text", "label": "新列名", "required": true},
    {"name": "col", "type": "columns", "label": "列", "pick": "one", "show": {"mode": ["文本", "数学", "日期", "条件"]}},
    {"name": "func_text", "type": "menu", "label": "函数", "options": ["分隔符之前", "分隔符之后", "两分隔符之间", "前 N 字符", "后 N 字符", "取第 N 段", "正则提取", "长度", "大写", "小写", "首字母大写", "去首尾空白", "替换", "补齐", "拼接"], "show": {"mode": ["文本"]}},
    {"name": "func_math", "type": "menu", "label": "函数", "options": ["两列运算", "四舍五入", "绝对值", "平方根", "对数", "向下取整", "向上取整"], "show": {"mode": ["数学"]}},
    {"name": "func_date", "type": "menu", "label": "函数", "options": ["取年", "取季度", "取月", "取日", "取时", "取分", "取秒", "取星期几", "一年中第几天", "转成日期", "加减时间", "截断到"], "show": {"mode": ["日期"]}},
    {"name": "func_val", "type": "menu", "label": "函数", "options": ["常量", "行号", "复制列"], "show": {"mode": ["值"]}},
    {"name": "delim", "type": "text", "label": "分隔符", "strip": false, "show": {"mode": ["文本"], "func_text": ["分隔符之前", "分隔符之后", "两分隔符之间", "取第 N 段"]}},
    {"name": "delim2", "type": "text", "label": "分隔符2", "strip": false, "hint": "后面的那个分隔符，留空 = 同“分隔符”", "show": {"mode": ["文本"], "func_text": ["两分隔符之间"]}},
    {"name": "n", "type": "int", "label": "位数", "required": true, "show": {"mode": ["文本"], "func_text": ["前 N 字符", "后 N 字符"]}},
    {"name": "seg", "type": "int", "label": "第几段", "default": "1", "hint": "1 起", "show": {"mode": ["文本"], "func_text": ["取第 N 段"]}},
    {"name": "pattern", "type": "text", "label": "正则", "required": true, "strip": false, "show": {"mode": ["文本"], "func_text": ["正则提取"]}},
    {"name": "group", "type": "int", "label": "第几组", "default": "1", "hint": "正则的第几个括号；0 = 整个匹配", "show": {"mode": ["文本"], "func_text": ["正则提取"]}},
    {"name": "find", "type": "text", "label": "查找", "required": true, "strip": false, "show": {"mode": ["文本"], "func_text": ["替换"]}},
    {"name": "repl", "type": "text", "label": "替换为", "strip": false, "show": {"mode": ["文本"], "func_text": ["替换"]}},
    {"name": "replace_all", "type": "toggle", "label": "替换全部", "show": {"mode": ["文本"], "func_text": ["替换"]}},
    {"name": "width", "type": "int", "label": "位数", "required": true, "hint": "补齐到几位", "show": {"mode": ["文本"], "func_text": ["补齐"]}},
    {"name": "char", "type": "text", "label": "补的字符", "strip": false, "hint": "留空 = 空格；订单号补 0 常用", "show": {"mode": ["文本"], "func_text": ["补齐"]}},
    {"name": "pad_side", "type": "menu", "label": "补在哪", "options": ["前面", "后面"], "show": {"mode": ["文本"], "func_text": ["补齐"]}},
    {"name": "cols2", "type": "columns", "label": "列", "required": true, "hint": "要拼在一起的列，按这个顺序", "show": {"mode": ["文本"], "func_text": ["拼接"]}},
    {"name": "sep2", "type": "text", "label": "分隔符", "strip": false, "show": {"mode": ["文本"], "func_text": ["拼接"]}},
    {"name": "op", "type": "menu", "label": "运算", "options": ["+", "-", "*", "/", "%", "**"], "show": {"mode": ["数学"], "func_math": ["两列运算"]}},
    {"name": "other", "type": "expr", "label": "对什么", "hint": "另一列 pl.col(\"数量\")，或一个常数 100", "show": {"mode": ["数学"], "func_math": ["两列运算"]}},
    {"name": "decimals", "type": "int", "label": "小数位", "default": "0", "hint": "保留几位小数", "show": {"mode": ["数学"], "func_math": ["四舍五入"]}},
    {"name": "base", "type": "float", "label": "底数", "hint": "留空 = 自然对数（e）", "show": {"mode": ["数学"], "func_math": ["对数"]}},
    {"name": "delta", "type": "int", "label": "数量", "required": true, "hint": "加多少（负数就是往前）", "show": {"mode": ["日期"], "func_date": ["加减时间"]}},
    {"name": "dunit", "type": "menu", "label": "单位", "options": ["天", "周", "月", "季", "年", "时", "分", "秒"], "show": {"mode": ["日期"], "func_date": ["加减时间"]}},
    {"name": "trunc", "type": "menu", "label": "到", "options": ["年", "季度", "月", "日", "时", "分", "秒"], "hint": "截断到哪一格：月 = 月初、年 = 元旦", "show": {"mode": ["日期"], "func_date": ["截断到"]}},
    {"name": "cmp", "type": "menu", "label": "比较", "options": ["等于", "不等于", "大于", "大于等于", "小于", "小于等于", "包含", "不包含", "开头是", "结尾是", "匹配正则", "为空", "不为空"]},
    {"name": "value", "type": "expr", "label": "值", "show": {"mode": ["条件"], "cmp": ["等于", "不等于", "大于", "大于等于", "小于", "小于等于", "包含", "不包含", "开头是", "结尾是", "匹配正则"]}},
    {"name": "then_v", "type": "expr", "label": "满足时", "hint": "条件成立时填什么：\"大单\"、1、pl.col(\"备注\")"},
    {"name": "else_v", "type": "expr", "label": "否则", "hint": "不成立时填什么，留空 = 空值"},
    {"name": "lit", "type": "expr", "label": "值", "show": {"mode": ["值"], "func_val": ["常量"]}},
    {"name": "start", "type": "int", "label": "从几开始", "default": "1", "show": {"mode": ["值"], "func_val": ["行号"]}},
    {"name": "copy", "type": "columns", "label": "列", "pick": "one", "required": true, "show": {"mode": ["值"], "func_val": ["复制列"]}}
  ]
}

增加一列，像 Excel / Power Query 的“添加列”：选好方式，再从“函数”里挑一个常用的，下面只显示它用得到的参数。结果接在表的最后；新列名和已有的相同时覆盖那一列。

* 文本：从文字列里提取一段（按分隔符、按字数、按正则），或改格式（大小写、去空白、替换、补齐），或把几列拼成一列
* 数学：两列（或一列一个常数）做加减乘除，或四舍五入、绝对值、平方根、对数、取整
* 日期：从日期 / 时间列取年月日时分秒、星期几，转成日期，往前往后加时间，截断到月初、年初
* 条件：条件列——这列满足某个条件时填一个值，否则填另一个值
* 值：整列同一个常量、行号，或把一列换个名字复制一份

这些函数要串着做几步，或条件不止一个时，用 expr 节点（上下文 with_columns）里的表达式网络。

@parameters
mode:
    新列按什么算。
name:
    新列的名字。和已有列同名时覆盖那一列。
col:
    对哪一列算（值方式不用）。
func_text:
    文本函数：
    * 分隔符之前 / 之后：如“规格”列按“-”拆，取前半 / 后半
    * 两分隔符之间：夹在两个分隔符中间的一段
    * 前 N 字符 / 后 N 字符：开头或结尾的几个字
    * 取第 N 段：按分隔符拆开取第几段（1 起；分隔符没出现时整段算第 1 段）
    * 正则提取：按正则取第几组（括号），0 = 整个匹配
    * 长度：字符数
    * 大写 / 小写 / 首字母大写 / 去首尾空白
    * 替换：把“查找”换成“替换为”（按字面替换，不是正则）；“替换全部”开着时全换，否则只换第一处
    * 补齐：不够 width 位就在前 / 后面补字符（如订单号前面补 0）
    * 拼接：把选的几列按顺序拼成一个字（要文字列），中间隔“分隔符”
delim:
    分隔符，按字面算（不是正则）。
delim2:
    两分隔符之间用的第二个分隔符，留空 = 同“分隔符”。
n:
    取几位。
seg:
    取第几段，1 起。
pattern:
    正则表达式。
group:
    取正则的第几个括号，0 = 整个匹配。
find:
    要换掉的文字。
repl:
    换成什么，可以是空 = 删掉。
replace_all:
    只换第一处还是全换。
width:
    补齐到几位。
char:
    补上去的字符，留空 = 空格。
pad_side:
    前面 / 后面。
cols2:
    拼接的列，按这里的顺序拼。
sep2:
    列之间的分隔符，留空 = 直接连起来。
func_math:
    数学函数：
    * 两列运算：列 ⊕ 对什么，运算选 + − × ÷ 取余 幂
    * 四舍五入：保留 decimals 位小数
    * 绝对值 / 平方根 / 向下取整 / 向上取整
    * 对数：以“底数”为底，留空 = 自然对数
op:
    两列怎么算。
other:
    参与运算的另一列或常数。
decimals:
    保留几位小数。
base:
    对数的底数，留空 = 自然对数。
func_date:
    日期函数：
    * 取年 / 季度 / 月 / 日 / 时 / 分 / 秒：各取一个数
    * 取星期几：1 = 周一 … 7 = 周日
    * 一年中第几天：1 月 1 日是第 1 天
    * 转成日期：日期时间丢掉时分秒
    * 加减时间：加“数量”个“单位”（负数往前）
    * 截断到：扔掉比它细的部分——月 = 这个月 1 号 0 点
delta:
    加减多少。
dunit:
    加减的单位。
trunc:
    截断到哪一格。
func_val:
    * 常量：整列都是这个值
    * 行号：1 起的序号
    * 复制列：把一列复制一份换个名字（想留着原样改列名时用）
cmp:
    条件列怎么比。文字的比“大于 / 小于”按字典序。
value:
    和列比的值。
then_v:
    条件成立时填的值，留空 = 空值。
else_v:
    条件不成立时填的值，留空 = 空值。
lit:
    整列填的值。
start:
    行号从几开始。
copy:
    复制哪一列。

@inputs
1:
    要加列的表。

@examples
从“规格”里取“-”前面的型号：
{{{
方式      文本
新列名    型号
列        "规格"
函数      分隔符之前
分隔符    -
}}}

金额超过一万标“大单”，否则“普通”：
{{{
方式    条件
新列名  规格
列      "金额"
比较    大于
值      10000
满足时  "大单"
否则    "普通"
}}}

从下单时间取月份，再加 3 天算预计到货：
{{{
方式    日期
新列名  预计到货
列      "下单时间"
函数    加减时间
数量    3
单位    天
}}}

订单号统一补到 10 位，不够的前面补 0：
{{{
方式      文本
新列名    订单号
列        "订单号"
函数      补齐
位数      10
补的字符  0
补在哪    前面
}}}

@related
expr, select, rename, with_row_index

@polars
拼接: pl.concat_str
正则提取: Expr.str.extract
替换: Expr.str.replace
补齐: Expr.str.pad_start
对数: Expr.log
取年: Expr.dt.year
加减时间: Expr.dt.offset_by
截断到: Expr.dt.truncate
条件: pl.when
行号: pl.int_range
"""
import operator


def one(names, what):
    """单列参数 → pl.col(名字)。“列”里选了一列才用得到。"""
    if not names or not isinstance(names[0], str):
        raise ValueError(f"“{what}”要先在“列”里选一列")
    return pl.col(names[0])


def text_of(v, what):
    """文字函数的值：填了数字按它的文字算。"""
    if v is None:
        raise ValueError(f"“{what}”要填一段文字")
    return v if isinstance(v, str) else str(v)


mode = ch("mode")
new_name = ch("name")
if mode == "文本":
    f = ch("func_text")
    if f == "拼接":
        cols = [n for n in ch("cols2") if isinstance(n, str)]
        if not cols:
            raise ValueError("拼接要在“列”里选要拼的列")
        e = pl.concat_str([pl.col(n) for n in cols], separator=ch("sep2"))
    else:
        c = one(ch("col"), f)
        if f == "分隔符之前":
            e = c.str.split(ch("delim")).list.get(0)
        elif f == "分隔符之后":
            e = c.str.split(ch("delim")).list.get(-1)
        elif f == "两分隔符之间":
            e = c.str.split(ch("delim")).list.get(1).str.split(ch("delim2") or ch("delim")).list.get(0)
        elif f == "前 N 字符":
            e = c.str.slice(0, ch("n"))
        elif f == "后 N 字符":
            e = c.str.slice(-ch("n"))
        elif f == "取第 N 段":
            e = c.str.split(ch("delim")).list.get((ch("seg") or 1) - 1)
        elif f == "正则提取":
            e = c.str.extract(ch("pattern"), ch("group") or 0)
        elif f == "长度":
            e = c.str.len_chars()
        elif f == "大写":
            e = c.str.to_uppercase()
        elif f == "小写":
            e = c.str.to_lowercase()
        elif f == "首字母大写":
            e = c.str.to_titlecase()
        elif f == "去首尾空白":
            e = c.str.strip_chars()
        elif f == "替换":
            how = c.str.replace_all if ch("replace_all") else c.str.replace
            e = how(text_of(ch("find"), "查找"), text_of(ch("repl"), "替换为"), literal=True)
        elif f == "补齐":
            side = c.str.pad_end if ch("pad_side") == "后面" else c.str.pad_start
            e = side(ch("width"), ch("char") or " ")
elif mode == "数学":
    f = ch("func_math")
    c = one(ch("col"), f)
    if f == "两列运算":
        v = ch("other")
        if v is None:
            raise ValueError('两列运算要填“对什么”，如 pl.col("数量") 或 100')
        fn = {"+": operator.add, "-": operator.sub, "*": operator.mul,
              "/": operator.truediv, "%": operator.mod, "**": operator.pow}[ch("op")]
        e = fn(c, v)
    elif f == "四舍五入":
        e = c.round(ch("decimals") or 0)
    elif f == "绝对值":
        e = c.abs()
    elif f == "平方根":
        e = c.sqrt()
    elif f == "对数":
        e = c.log(ch("base")) if ch("base") is not None else c.log()
    elif f == "向下取整":
        e = c.floor()
    else:
        e = c.ceil()
elif mode == "日期":
    f = ch("func_date")
    c = one(ch("col"), f)
    if f == "取年":
        e = c.dt.year()
    elif f == "取季度":
        e = c.dt.quarter()
    elif f == "取月":
        e = c.dt.month()
    elif f == "取日":
        e = c.dt.day()
    elif f == "取时":
        e = c.dt.hour()
    elif f == "取分":
        e = c.dt.minute()
    elif f == "取秒":
        e = c.dt.second()
    elif f == "取星期几":
        e = c.dt.weekday()
    elif f == "一年中第几天":
        e = c.dt.ordinal_day()
    elif f == "转成日期":
        e = c.dt.date()
    elif f == "加减时间":
        d = ch("delta")
        if not d:
            raise ValueError("加减时间要填“数量”（负数就是往前）")
        unit = {"天": "d", "周": "w", "月": "mo", "季": "q", "年": "y", "时": "h", "分": "m", "秒": "s"}[ch("dunit")]
        e = c.dt.offset_by(f"{d}{unit}")
    else:
        unit = {"年": "1y", "季度": "1q", "月": "1mo", "日": "1d", "时": "1h", "分": "1m", "秒": "1s"}[ch("trunc")]
        e = c.dt.truncate(unit)
elif mode == "条件":
    c = one(ch("col"), "条件")
    f = ch("cmp")
    if f == "为空":
        cond = c.is_null()
    elif f == "不为空":
        cond = c.is_not_null()
    else:
        v = ch("value")
        if v is None:
            raise ValueError(f'比较选了“{f}”，要填“值”')
        if f == "等于":
            cond = c == v
        elif f == "不等于":
            cond = c != v
        elif f == "大于":
            cond = c > v
        elif f == "大于等于":
            cond = c >= v
        elif f == "小于":
            cond = c < v
        elif f == "小于等于":
            cond = c <= v
        else:
            s = text_of(v, "值")
            if f == "包含":
                cond = c.str.contains(s, literal=True)
            elif f == "不包含":
                cond = ~c.str.contains(s, literal=True)
            elif f == "开头是":
                cond = c.str.starts_with(s)
            elif f == "结尾是":
                cond = c.str.ends_with(s)
            else:
                cond = c.str.contains(s)
    tv, ev = ch("then_v"), ch("else_v")
    w = pl.when(cond).then(pl.lit(tv) if isinstance(tv, str) else tv if tv is not None else pl.lit(None))
    e = w.otherwise(pl.lit(ev) if isinstance(ev, str) else ev) if ev is not None else w
else:
    f = ch("func_val")
    if f == "常量":
        v = ch("lit")
        if v is None:
            raise ValueError('常量要填一个值，如 0、"无"、pl.date(2024, 1, 1)')
        e = v if isinstance(v, pl.Expr) else pl.lit(v)
    elif f == "行号":
        start = ch("start") or 1
        e = pl.int_range(start, pl.len() + start)
    else:
        e = one(ch("copy"), "复制列")

out = df.with_columns(e.alias(new_name))
