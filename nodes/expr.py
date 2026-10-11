r"""
{
  "desc": "表达式：选上下文（with_columns / select / filter / group_by / sort），双击进去用表达式节点连",
  "category": "表达式",
  "order": 10,
  "inputs": 1,
  "network": true,
  "params": [
    {"name": "context", "type": "menu", "label": "上下文", "mode": true, "options": ["with_columns", "select", "filter", "group_by", "sort"]},
    {"name": "then", "type": "menu", "label": "分组后", "options": ["agg", "all", "len", "sum", "mean", "median", "min", "max", "first", "last", "n_unique", "quantile", "head", "tail"], "hint": "agg = 网络里的“输出列”在每组里算；其余是 polars 的快捷方法，对分组键以外的每一列都做", "show": {"context": ["group_by"]}},
    {"name": "n", "type": "int", "label": "每组行数", "default": "5", "show": {"context": ["group_by"], "then": ["head", "tail"]}},
    {"name": "quantile", "type": "float", "label": "分位数", "default": "0.5", "hint": "0 ~ 1", "show": {"context": ["group_by"], "then": ["quantile"]}},
    {"name": "interpolation", "type": "menu", "label": "插值", "options": ["nearest", "higher", "lower", "midpoint", "linear", "equiprobable"], "show": {"context": ["group_by"], "then": ["quantile"]}},
    {"name": "len_name", "type": "text", "label": "计数列名", "hint": "留空 = len", "show": {"context": ["group_by"], "then": ["len"]}},
    {"name": "keep", "type": "menu", "label": "留下", "mode": true, "options": ["留下满足的", "去掉满足的"], "hint": "留下满足的（filter）：条件为假、为空的行都去掉；去掉满足的（remove）：只去掉条件为真的，为空的留着", "show": {"context": ["filter"]}},
    {"name": "combine", "type": "menu", "label": "多个条件", "mode": true, "options": ["全部满足", "任一满足"], "hint": "有几个“条件”输出时：全部满足（且）/ 任一满足（或，pl.any_horizontal）", "show": {"context": ["filter"]}},
    {"name": "maintain_order", "type": "toggle", "default": "True", "hint": "分组按第一次出现的顺序排", "show": {"context": ["group_by", "sort"]}},
    {"name": "every", "type": "text", "label": "时间窗口", "hint": "按时间分组（group_by_dynamic，分组后只能 agg）：1d / 1w / 1mo / 1q / 1y；留空 = 普通分组", "show": {"context": ["group_by"], "then": ["agg"]}},
    {"name": "period", "type": "text", "label": "滚动窗口", "hint": "每一行往前看多长（rolling，分组后只能 agg）：3d / 2h / 5i（5 行）；和时间窗口二选一", "show": {"context": ["group_by"], "then": ["agg"]}},
    {"name": "index_column", "type": "columns", "pick": "one", "label": "时间列", "hint": "填了时间窗口或滚动窗口才用：\"日期\"", "show": {"context": ["group_by"], "then": ["agg"]}}
  ]
}

对表做 polars 的“上下文 + 表达式”。外面选上下文；里面（双击或回车进去）是一张表达式网络——像 Houdini 的 VOP：从“列”节点开始，接上各种表达式节点，最后连到输出节点。

== 上下文 ==
* with_columns：加新列或改列，原来的列都在。网络里每个“输出列”是一列，名字和已有的列一样就覆盖它
* select：只要网络算出来的这些列
* filter：只留下满足条件的行。网络里接“条件”输出；有几个条件时选“全部满足”（且）或“任一满足”（或）。选“去掉满足的”就反过来，删掉满足的行
* group_by：分组。在不同地方算的部分分成三张网络，进去后用路径栏上的页签切换（见下面）；“分组后”也可以直接选 sum / mean / len 这些快捷方法，对分组键以外的每一列都做，这时用不到聚合那张网络
* sort：按“排序键”输出排序，每个键单独选从大到小、空值放最后

== group_by 的三张网络 ==
* 分组键 · 整表：接“分组键”输出，在整张表上算，每行一个值，值一样的行分到一组
* 聚合 · 每组：“分组后”选 agg 时用，接“输出列”，在每组里算（求和、计数、取第一个…），每组得到一行
* 筛选组 · 每组：可选，接“条件”输出，只留满足条件的组（having）；空着就是不筛

== 在网络里 ==
* 新建时里面是“col → 输出列”，col 填的是上游的第一列
* 输出节点按画布上从上到下排：几个输出列就是结果里的列序
* 选中网络里的节点，表格显示它算出来的值（连同用到的列）；点空白处显示整个 expr 节点的结果。group_by 里分组键那张网络的节点按整张表显示（和表一样多行），聚合、筛选组里的按组显示（每组一行，没聚合的是一个列表）
* 几张网络之间不能连线：分组键和聚合都要用到同一列时，各放一个 col
* 参数面板底下一行是写成文字的表达式；标题栏的“转成文字”把选中的节点连同上游收成一段文字，“展开成节点”反过来
* Tab 菜单里直接写一段 polars 表达式（如 pl.col("a").str.len_chars()），第一项“按文字建节点”把它拆成节点
* 按 U 回到外面

@parameters
context:
    用哪种上下文，见上面。换了上下文，网络里用不到的输出节点会标黄提醒，不会被删。
then:
    分组后做什么。agg = 用网络里的“输出列”自己算；其余是 polars 分组的快捷方法，对分组键以外的每一列都做，网络里只要分组键。
n:
    head / tail：每组取几行。
quantile:
    每组的分位数，0.5 = 中位数。
interpolation:
    分位数落在两个值之间时怎么取。
len_name:
    计数那一列叫什么。
keep:
    * 留下满足的（filter）：只留条件为真的行，为假、为空的都去掉
    * 去掉满足的（remove）：只去掉条件为真的行，为假、为空的都留着
    所以条件里有空值时，两边加起来不是全表；要把空值算成满足或不满足，在条件后面接 fill_null。
combine:
    有几个“条件”输出时怎么合：全部满足（且），任一满足（或，pl.any_horizontal）。只有一个条件时不起作用。
maintain_order:
    group_by：各组按第一次出现的顺序排（关掉快一些，顺序不定）。sort：值相同的行保持原来的先后。
every:
    按时间窗口分组（group_by_dynamic）：每 1d / 1w / 1mo / 1q / 1y 一组，要填“时间列”。分组键可以不要；有分组键时，每个键各自按时间分。
period:
    滚动窗口（rolling）：每一行往前看这么长算一次，3d = 三天，5i = 5 行。和时间窗口二选一。
index_column:
    时间窗口 / 滚动窗口按哪一列的时间，只用第一个。

@inputs
1:
    要处理的表。网络里“列”节点能用的列就是它的列。

@examples
按医院汇总金额，只要总金额超过一万的医院（上下文 group_by，分组后 agg）：
{{{
分组键：col "医院"                              → 分组键
聚合：  col "金额" → agg（sum）                 → 输出列（名字 总金额）
筛选组：col "金额" → agg（sum）→ compare（> 10000）→ 条件
}}}

@related
select, unique, slice, describe

@polars
with_columns: DataFrame.with_columns
select: DataFrame.select
filter: DataFrame.filter
去掉满足的: DataFrame.remove
任一满足: pl.any_horizontal
group_by: DataFrame.group_by
agg: GroupBy.agg
sort: DataFrame.sort
"""
# outputs("角色")：网络里这种输出节点的值，按画布上从上到下排
context = ch("context")


def need(role, what):
    values = outputs(role)
    if not values:
        raise ValueError(f"{context} 要至少一个“{what}”输出节点：双击进去，按 Tab 搜“{what}”")
    return values


if context == "with_columns":
    out = df.with_columns(need("列", "输出列"))
elif context == "select":
    out = df.select(need("列", "输出列"))
elif context == "filter":
    conditions = need("条件", "条件")
    if ch("combine") == "任一满足" and len(conditions) > 1:
        conditions = [pl.any_horizontal(conditions)]
    out = df.remove(*conditions) if ch("keep") == "去掉满足的" else df.filter(*conditions)
elif context == "group_by":
    keys = outputs("键")
    then = ch("then")
    if ch("every") or ch("period"):
        # group_by_dynamic / rolling 要求时间列有序
        index = (ch("index_column") or [None])[0]
        if index is None:
            raise ValueError("按时间窗口 / 滚动窗口分组要填“时间列”")
        if ch("every"):
            frame = df.sort(index).group_by_dynamic(index, every=ch("every"), group_by=keys or None)
        else:
            frame = df.sort(index).rolling(index, period=ch("period"), group_by=keys or None)
    else:
        frame = df.group_by(need("键", "分组键"), maintain_order=ch("maintain_order"))
    # 网络里的“条件”输出：只留下满足的分组（having，条件在每组里算）
    if outputs("条件"):
        frame = frame.having(*outputs("条件"))
    if then == "agg":
        out = frame.agg(need("列", "输出列"))
    elif then in ("head", "tail"):
        out = getattr(frame, then)(ch("n"))
    elif then == "quantile":
        out = frame.quantile(ch("quantile"), interpolation=ch("interpolation"))
    elif then == "len":
        out = frame.len(name=ch("len_name") or None)
    else:
        out = getattr(frame, then)()
else:
    keys = need("排序", "排序键")
    out = df.sort(
        [k[0] for k in keys],
        descending=[k[1] for k in keys],
        nulls_last=[k[2] for k in keys],
        maintain_order=ch("maintain_order"),
    )
