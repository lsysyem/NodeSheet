r"""
{
  "desc": "取一部分行（head / tail / slice / gather / gather_every / top_k / bottom_k / sample），或倒过来（reverse）",
  "category": "行",
  "order": 40,
  "inputs": 1,
  "params": [
    {"name": "mode", "type": "menu", "label": "取法", "mode": true, "options": ["head", "tail", "slice", "gather", "gather_every", "top_k", "bottom_k", "sample", "reverse"]},
    {"name": "n", "type": "int", "default": "100", "required": true, "hint": "几行", "show": {"mode": ["head", "tail"]}},
    {"name": "offset", "type": "int", "default": "0", "required": true, "hint": "从第几行开始（负数从末尾数）", "show": {"mode": ["slice"]}},
    {"name": "length", "type": "int", "hint": "取多少行，留空 = 到末尾", "show": {"mode": ["slice"]}},
    {"name": "indices", "type": "exprs", "insert": "none", "required": true, "hint": "第几行（从 0 数，负数从末尾数）：0, 2, -1", "show": {"mode": ["gather"]}},
    {"name": "every", "type": "int", "label": "n", "default": "2", "required": true, "hint": "每隔几行取一行", "show": {"mode": ["gather_every"]}},
    {"name": "every_offset", "type": "int", "label": "offset", "default": "0", "hint": "从第几行开始", "show": {"mode": ["gather_every"]}},
    {"name": "k", "type": "int", "default": "10", "required": true, "hint": "几行", "show": {"mode": ["top_k", "bottom_k"]}},
    {"name": "by", "type": "columns", "required": true, "hint": "按哪列比大小：\"金额\"", "show": {"mode": ["top_k", "bottom_k"]}},
    {"name": "sample_n", "type": "int", "label": "n", "default": "100", "hint": "抽多少行", "show": {"mode": ["sample"]}},
    {"name": "fraction", "type": "float", "hint": "或按比例，如 0.1", "show": {"mode": ["sample"]}},
    {"name": "seed", "type": "int", "hint": "随机种子", "show": {"mode": ["sample"]}}
  ]
}

取一部分行。选好“取法”，下面只显示这种取法用得到的参数。

* head / tail：前 / 后 n 行
* slice：从第 offset 行起取 length 行（从 0 数，负数从末尾数）
* gather：取指定的几行，如第 0、2、最后一行
* gather_every：每隔 n 行取一行（抽稀）
* top_k / bottom_k：按某列最大 / 最小的 k 行，不用先排序
* sample：随机抽 n 行，或一定比例
* reverse：整张表倒过来

@parameters
mode:
    怎么取。
n:
    head / tail 取几行。
offset:
    slice 从第几行开始，从 0 数；-10 = 倒数第 10 行。
length:
    slice 取多少行，留空 = 一直到末尾。
indices:
    要第几行，逗号分开，从 0 数，负数从末尾数。
every:
    每隔几行取一行：2 = 隔一行取一行。
every_offset:
    从第几行开始取。
k:
    top_k / bottom_k 取几行。
by:
    按哪一列（或几列）比大小。
sample_n:
    随机抽几行。
fraction:
    或者按比例抽，0.1 = 抽 10%；填了它就不看 n。
seed:
    随机种子。填一个数，每次抽到的都一样。

@examples
金额最大的 10 笔：
{{{
取法  top_k
k     10
by    "金额"
}}}

@related
unique, expr

@polars
head: DataFrame.head
tail: DataFrame.tail
slice: DataFrame.slice
gather: DataFrame.gather
gather_every: DataFrame.gather_every
top_k: DataFrame.top_k
bottom_k: DataFrame.bottom_k
sample: DataFrame.sample
reverse: DataFrame.reverse
"""
mode = ch("mode")
if mode in ("head", "tail"):
    out = getattr(df, mode)(ch("n"))
elif mode == "slice":
    out = df.slice(ch("offset"), ch("length"))
elif mode == "gather":
    out = df.gather(ch("indices"))
elif mode == "gather_every":
    out = df.gather_every(ch("every"), ch("every_offset") or 0)
elif mode in ("top_k", "bottom_k"):
    out = getattr(df, mode)(ch("k"), by=ch("by"))
elif mode == "reverse":
    out = df.reverse()
elif ch("fraction") is not None:
    # LazyFrame 没有 sample
    out = df.collect().sample(fraction=ch("fraction"), seed=ch("seed"))
elif ch("sample_n") is not None:
    out = df.collect().sample(n=ch("sample_n"), seed=ch("seed"))
else:
    raise ValueError("填写 n 或 fraction")
