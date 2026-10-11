r"""
{
  "desc": "元素：pl.element()，eval 节点里面代表列表的每个元素",
  "category": "输入",
  "order": 30,
  "expr": true,
  "params": []
}

代表列表（或数组）里的“每一个元素”：pl.element()。只在 eval 节点里面用——eval 对每个列表的每个元素算一遍里面的网络。

@examples
列表里每个数乘 2：eval（list.eval）里面 element → arith（* 2）→ 结果。

@related
eval, result

@polars
pl.element
"""
out = pl.element()
