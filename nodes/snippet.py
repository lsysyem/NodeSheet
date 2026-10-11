r"""
{
  "desc": "表达式文本：直接写 polars 表达式，a ~ d 是接进来的输入",
  "category": "代码",
  "order": 10,
  "expr": true,
  "params": [
    {"name": "a", "type": "expr", "port": true, "hint": "没连线时也可以直接写：pl.col(\"列\")"},
    {"name": "b", "type": "expr", "port": true},
    {"name": "c", "type": "expr", "port": true},
    {"name": "d", "type": "expr", "port": true},
    {"name": "expression", "type": "text", "label": "表达式", "required": true, "insert": "expr", "hint": "a.str.slice(0, 4) + \"-\" + b；也能用 pl、cs"}
  ]
}

直接写一段 polars 表达式。接进来的输入叫 a ~ d，表达式里还能用 pl 和 cs。节点表示不了的写法、或者懒得连节点时用。

参数面板标题栏的“展开成节点”把它拆成表达式节点；反过来，选中一个节点点“转成文字”，会把它连同只给它用的上游收成一个 snippet。

@parameters
a:
    第一个输入，表达式里写 a。
b:
    第二个输入，写 b。
c:
    第三个输入，写 c。
d:
    第四个输入，写 d。
expression:
    polars 表达式，如 a.str.slice(0, 4) + "-" + b；也可以不用输入：pl.col("年") * 100 + pl.col("月")。

@related
col
"""
names = {"pl": pl, "cs": cs, "a": ch("a"), "b": ch("b"), "c": ch("c"), "d": ch("d")}
out = eval(ch("expression"), names)
