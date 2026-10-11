r"""
{
  "desc": "删除含空值 / NaN 的行（drop_nulls / drop_nans）",
  "category": "行",
  "order": 50,
  "inputs": 1,
  "params": [
    {"name": "mode", "type": "menu", "label": "方式", "mode": true, "options": ["drop_nulls", "drop_nans"]},
    {"name": "subset", "type": "columns", "hint": "只看这些列，留空 = 所有列"}
  ]
}

删掉有空值（null）或 NaN 的行。

空值是“没有数据”，什么类型的列都可能有；NaN 是小数运算出来的“不是数”（如 0 / 0），只有小数列有。两者不一样：drop_nulls 不删 NaN。

@parameters
mode:
    * drop_nulls：删有空值的行
    * drop_nans：删有 NaN 的行
subset:
    只看这几列，留空 = 任何一列有就删。

@examples
没填手机号的不要：
{{{
subset  "手机号"
}}}

@related
fill_null, unique

@polars
drop_nulls: DataFrame.drop_nulls
drop_nans: DataFrame.drop_nans
"""
out = getattr(df, ch("mode"))(ch("subset") or None)
