r"""
{
  "desc": "填空值（fill_null）/ 填 NaN（fill_nan）/ 插值（interpolate），整表上下移（shift）",
  "category": "行",
  "order": 60,
  "inputs": 1,
  "params": [
    {"name": "mode", "type": "menu", "label": "方式", "mode": true, "options": ["fill_null", "fill_nan", "interpolate", "shift"]},
    {"name": "value", "type": "expr", "pick": "one", "insert": "expr", "hint": "填什么：0、\"无\"、pl.col(\"上期\")；只填类型对得上的列", "show": {"mode": ["fill_null", "fill_nan"]}},
    {"name": "strategy", "type": "menu", "options": ["", "forward", "backward", "min", "max", "mean", "zero", "one"], "hint": "不填 value 时按什么填：forward = 用上一个值", "show": {"mode": ["fill_null"]}},
    {"name": "limit", "type": "int", "hint": "forward / backward 最多连着填几个，留空 = 不限", "show": {"mode": ["fill_null"]}},
    {"name": "n", "type": "int", "default": "1", "required": true, "hint": "往下移几行（负数往上）", "show": {"mode": ["shift"]}},
    {"name": "fill_value", "type": "expr", "pick": "one", "insert": "expr", "hint": "移出来的空位填什么，留空 = 空值", "show": {"mode": ["shift"]}}
  ]
}

把空值补上，或者把整张表上下挪几行。对每一列都做；只处理某一列时用 expr 节点（网络里接 fill_null / shift 表达式节点）。

* fill_null：空值填成一个值，或按策略填（用上一个值、平均值…）
* fill_nan：NaN 填成一个值；不填 value 就把 NaN 变成空值
* interpolate：按前后的值线性插值（数值列）
* shift：整表往下（或往上）挪 n 行，空出来的位置填 fill_value

@parameters
mode:
    怎么补。
value:
    填什么。只填类型对得上的列：填 0 时文字列不变。也可以写表达式，如 pl.col("上期")。
strategy:
    不填 value 时按什么填：
    * forward：用上面最近的一个值（Excel 合并单元格读进来常用）
    * backward：用下面最近的一个值
    * min / max / mean：这一列的最小 / 最大 / 平均值
    * zero / one：填 0 / 1
limit:
    forward / backward 时最多连着填几个空值。
n:
    挪几行，正数往下、负数往上。
fill_value:
    挪出来的空位填什么，留空 = 空值。

@examples
Excel 里的合并单元格读进来，下面几行是空的，用上面的值补：
{{{
方式      fill_null
strategy  forward
}}}

@related
drop_nulls, upsample, expr

@polars
fill_null: DataFrame.fill_null
fill_nan: DataFrame.fill_nan
interpolate: DataFrame.interpolate
shift: DataFrame.shift
"""
mode = ch("mode")
if mode == "fill_null":
    if ch("strategy"):
        out = df.fill_null(strategy=ch("strategy"), limit=ch("limit"))
    elif ch("value") is not None:
        out = df.fill_null(ch("value"))
    else:
        raise ValueError("填写 value，或选一个 strategy")
elif mode == "fill_nan":
    # value 留空：NaN 变成空值
    out = df.fill_nan(ch("value"))
elif mode == "interpolate":
    out = df.interpolate()
else:
    out = df.shift(ch("n"), fill_value=ch("fill_value"))
