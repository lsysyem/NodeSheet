r"""
{
  "desc": "写 polars 代码；可以给它加参数、存成自己的节点类型",
  "category": "代码",
  "order": 20,
  "inputs": 4,
  "min_inputs": 0,
  "code_param": "code",
  "params": [
    {"name": "code", "type": "code", "label": "代码", "required": true, "default": "# df ~ df4 = 第 1~4 个输入（没连的是 None）\n# ch(\"参数名\") 取这个节点的参数；把结果赋给 out\nout = df\n"}
  ]
}

写一段 polars 代码，什么都能做。输入是 df、df2 ~ df4（LazyFrame，没连的是 None），把结果（DataFrame 或 LazyFrame）赋给 out；也可以让最后一行就是结果。

能用的名字：pl（polars）、cs（polars.selectors）、inputs（全部输入的列表）、ch("参数名")（取这个节点的参数）。print 出来的内容显示在参数面板里。

写顺手了，可以给它加参数（标题栏“参数界面”），再“存为类型”，就成了自己的节点类型，出现在“添加”菜单和 Tab 搜索里——像 Houdini 的数字资产。

@parameters
code:
    节点的代码。编辑框有 polars 的补全、签名提示和高亮，Tab / 回车自动缩进。

@examples
{{{
out = df.with_columns(
    pl.col("姓名").str.slice(0, 1).alias("姓"),
    (pl.col("金额") > 1000).alias("大额"),
)
}}}

@related
sql, expr
"""

