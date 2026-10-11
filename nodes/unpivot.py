r"""
{
  "desc": "宽表转长表（逆透视）",
  "category": "变形",
  "order": 20,
  "inputs": 1,
  "params": [
    {"name": "on", "type": "columns", "hint": "要拆成行的列，留空 = index 以外的全部"},
    {"name": "index", "type": "columns", "hint": "保持不动的列：\"医院\""},
    {"name": "variable_name", "type": "text", "hint": "列名那一列叫什么，默认 variable"},
    {"name": "value_name", "type": "text", "hint": "值那一列叫什么，默认 value"}
  ]
}

宽表转长表（逆透视）：把好几列压成两列——一列是原来的列名，一列是值。比如 1月 ~ 12月 十二列变成“月份”“金额”两列，行数变成 12 倍。

@parameters
on:
    要压成行的列，如 cs.ends_with("月")。留空 = index 以外的全部列。
index:
    保持不动、每行重复的列，如 "医院"。
variable_name:
    放原列名的那一列叫什么，默认 variable。
value_name:
    放值的那一列叫什么，默认 value。

@examples
{{{
index          "医院"
variable_name  月份
value_name     金额
}}}

@related
pivot, transpose

@polars
DataFrame.unpivot
"""
out = df.unpivot(
    ch("on") or None,
    index=ch("index") or None,
    variable_name=ch("variable_name") or None,
    value_name=ch("value_name") or None,
)
