r"""
{
  "desc": "上下 / 左右拼接 2 ~ 4 张表（concat），或按键归并两张排好序的表（merge_sorted）",
  "category": "多表",
  "order": 20,
  "inputs": 4,
  "min_inputs": 2,
  "params": [
    {"name": "mode", "type": "menu", "label": "方式", "mode": true, "options": ["concat", "merge_sorted"]},
    {"name": "how", "type": "menu", "options": ["vertical", "vertical_relaxed", "diagonal", "diagonal_relaxed", "horizontal", "align"], "show": {"mode": ["concat"]}},
    {"name": "key", "type": "columns", "pick": "one", "required": true, "hint": "两张表都按这一列排好了序：\"日期\"", "show": {"mode": ["merge_sorted"]}}
  ]
}

把 2 ~ 4 张表拼起来（接在输入 1 ~ 4 上，没连的不算）。

* concat：上下或左右拼接，怎么对齐看 how
* merge_sorted：两张按同一列排好序的表合成一张，结果仍然有序（比拼接后再排序快）。两张表的列要一样

@parameters
mode:
    拼接还是归并。
how:
    * vertical：上下拼，列名和类型要完全一样
    * vertical_relaxed：上下拼，类型不同时转成两边都装得下的（整数 + 小数 → 小数）
    * diagonal：上下拼，列不一样也行，缺的列填空值
    * diagonal_relaxed：diagonal，类型也放宽
    * horizontal：左右拼，行数不同时短的补空值；列名不能重复
    * align：按共同的列对齐后合并
key:
    merge_sorted 按哪一列，两张表都要按它从小到大排好。

@inputs
1:
    第一张表（结果里在最上面 / 最左边）。
2:
    第二张表。

@examples
每个月一个文件：每个文件一个 read 节点，都接到 concat 上，how 选 diagonal_relaxed（某几个月多一列也不怕）。

@related
join, read

@polars
concat: pl.concat
merge_sorted: DataFrame.merge_sorted
"""
# inputs：连了线的输入，没连的是 None
frames = [frame for frame in inputs if frame is not None]
if ch("mode") == "merge_sorted":
    # 结果仍按 key 有序；两张表列要一样
    first, second = frames[:2]
    out = first.merge_sorted(second, key=ch("key")[0])
else:
    out = pl.concat(frames, how=ch("how"))
