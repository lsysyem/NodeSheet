r"""
{
  "desc": "重命名列（映射 / 加前后缀 / 大小写）",
  "category": "列",
  "order": 20,
  "inputs": 1,
  "params": [
    {"name": "mode", "type": "menu", "label": "方式", "mode": true, "options": ["映射", "前缀", "后缀", "大写", "小写"]},
    {"name": "mapping", "type": "dict", "pick": "one", "label": "旧名: 新名", "required": true, "hint": "\"旧名\": \"新名\", \"a\": \"b\"", "show": {"mode": ["映射"]}},
    {"name": "prefix", "type": "text", "required": true, "strip": false, "hint": "加在每个列名前面", "show": {"mode": ["前缀"]}},
    {"name": "suffix", "type": "text", "required": true, "strip": false, "hint": "加在每个列名后面", "show": {"mode": ["后缀"]}}
  ]
}

改列名：一对一改，或者给每列加前缀 / 后缀，或者统一大小写。

@parameters
mode:
    * 映射：按“旧名: 新名”一个个改，没写到的列不变
    * 前缀 / 后缀：每个列名前面 / 后面加同样的字（join 前区分两张表的列常用）
    * 大写 / 小写：英文列名统一大小写
mapping:
    "旧名": "新名"，逗号分开写多对。
prefix:
    加在前面的字，空格也算。
suffix:
    加在后面的字，空格也算。

@related
select, expr

@polars
DataFrame.rename
"""
mode = ch("mode")
if mode == "前缀":
    out = df.rename(lambda name: ch("prefix") + name)
elif mode == "后缀":
    out = df.rename(lambda name: name + ch("suffix"))
elif mode == "大写":
    out = df.rename(str.upper)
elif mode == "小写":
    out = df.rename(str.lower)
else:
    out = df.rename(ch("mapping"))
