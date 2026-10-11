r"""
{
  "desc": "选择 / 删除列（select / drop）",
  "category": "列",
  "order": 10,
  "inputs": 1,
  "params": [
    {"name": "mode", "type": "menu", "label": "方式", "mode": true, "options": ["select", "drop"]},
    {"name": "columns", "type": "columns", "label": "列或表达式", "required": true, "hint": "\"医院\", pl.col(\"金额\") * 2；按位置写 \"1:5\"、\"a:f\"、\"首列:末列\"、\"3\""}
  ]
}

挑出要的列（select），或删掉不要的列（drop）。

列可以写列名，也可以写选择器：cs.numeric() 全部数值列、cs.string() 全部文字列、cs.starts_with("金额") 名字以“金额”开头的列；选择器能用 | & - ~ 组合。

按位置选（第几列）写成带引号的文字：序号 1 起、含两端，可以逗号分开写多段——
* "1:5" 第 1 到第 5 列；"7:10" 第 7 到第 10 列；"1:" 从第 1 列到末尾、":5" 从开头到第 5 列、":" 全部列
* "a:f" 按 Excel 的列标，a = 第 1 列（只认单个字母 a~z，后面的列用序号写）
* "首列:末列" 从叫这个名的列到叫那个名的列（按表里的位置，含两端）
* "3" 或不带引号的 3 单独一列：第 3 列

要算新列、改列的值，用 expr 节点（上下文 select / with_columns）。

@parameters
mode:
    * select：只留这些列，按写的顺序排（也能用来调列的顺序）
    * drop：删掉这些列，其余不动
columns:
    逗号分开。点旁边的“选列”可以勾列、排除列、按条件（选择器）选，确定后写进来；
    按位置的范围（"1:5"、"a:f"、"首列:末列"、"3"）运行时按当时表的列序展开。

@examples
留下“医院”和全部数值列：
{{{
方式        select
列或表达式  "医院", cs.numeric()
}}}

留下前 3 列和从“金额”到“合计”的列，再单独留下第 10 列：
{{{
方式        select
列或表达式  "1:3", "金额:合计", 10
}}}

@related
rename, cast_columns, expr

@polars
select: DataFrame.select
drop: DataFrame.drop
"""
import re

INDEX = re.compile(r"^(-?\d*)\s*:\s*(-?\d*)$")
LETTER = re.compile(r"^[A-Za-z]$")


def expand(item, names):
    """一段参数 → 列名列表：序号、序号范围、Excel 列标范围、名字范围；别的照原样。"""
    text = item.strip() if isinstance(item, str) else item
    if isinstance(item, int) and not isinstance(item, bool) or isinstance(item, str) and text.isdigit():
        n = int(text)
        if not 1 <= n <= len(names):
            raise ValueError(f'第 {n} 列不在表里（共 {len(names)} 列）')
        return [names[n - 1]]
    if isinstance(item, str):
        m = INDEX.match(text)
        if m:
            start = int(m[1]) if m[1] else 1
            end = int(m[2]) if m[2] else len(names)
            if start > end:
                start, end = end, start
            if start < 1 or end > len(names):
                raise ValueError(f'范围 "{text}" 超出了列数（共 {len(names)} 列）')
            return names[start - 1:end]
        if ":" in text:
            left, right = text.split(":", 1)
            a, b = left.strip(), right.strip()
            if LETTER.match(a) and LETTER.match(b):
                start, end = ord(a.lower()) - 96, ord(b.lower()) - 96
            else:
                for name in (a, b):
                    if name not in names:
                        raise ValueError(f'范围 "{text}" 里的列 "{name}" 不在表里')
                start, end = names.index(a) + 1, names.index(b) + 1
            if start > end:
                start, end = end, start
            return names[start - 1:end]
    return [item]


cols = []
names = None
for item in ch("columns"):
    if isinstance(item, int) and not isinstance(item, bool) or isinstance(item, str) and (item.strip().isdigit() or ":" in item):
        if names is None:
            names = df.collect_schema().names()
        cols.extend(expand(item, names))
    else:
        cols.append(item)
out = getattr(df, ch("mode"))(*cols)
