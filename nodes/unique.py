r"""
{
  "desc": "去重（unique），或只留下重复的行（is_duplicated）",
  "category": "行",
  "order": 30,
  "inputs": 1,
  "params": [
    {"name": "mode", "type": "menu", "label": "方式", "mode": true, "options": ["unique", "is_duplicated"]},
    {"name": "subset", "type": "columns", "hint": "依据列，留空 = 整行比较"},
    {"name": "keep", "type": "menu", "options": ["any", "first", "last", "none"], "hint": "重复的留哪一行；none = 重复的一行都不留", "show": {"mode": ["unique"]}}
  ]
}

去掉重复的行；或者反过来，只看重复的行（查重）。

* unique：同样的行只留一行。“依据列”相同就算重复，留哪一行看 keep；结果保持原来的顺序
* is_duplicated：只留下重复了的行（每一份都留），用来查哪些数据重复了

@parameters
mode:
    去重还是查重。
subset:
    按哪几列判断重复，留空 = 整行都一样才算。如按 "身份证号" 去重。
keep:
    重复的几行留哪一行：
    * any：随便一行（最快）
    * first：第一次出现的
    * last：最后一次出现的（如留最新的一条记录）
    * none：重复的一行都不留，只剩从没重复过的

@examples
每个客户只留最后一次下单（上游先按时间排好序）：
{{{
方式    unique
subset  "客户"
keep    last
}}}

@related
slice, drop_nulls, expr

@polars
unique: DataFrame.unique
is_duplicated: DataFrame.is_duplicated
"""
if ch("mode") == "is_duplicated":
    # 依据列的值出现了不止一次的行，全部留下
    subset = ch("subset")
    out = df.filter(pl.struct(subset or pl.all()).is_duplicated())
else:
    out = df.unique(ch("subset") or None, keep=ch("keep"), maintain_order=True)
