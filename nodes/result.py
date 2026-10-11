r"""
{
  "desc": "结果：eval 节点里每个元素算出来的值（filter 里是留下哪些元素的条件，agg 里是聚合）",
  "category": "输出",
  "order": 50,
  "expr": true,
  "role": "结果",
  "params": [
    {"name": "value", "type": "expr", "label": "值", "port": true, "required": true, "hint": "没连线时也可以直接写：pl.element() * 2"}
  ]
}

eval 节点里网络的结果：对列表的每个元素（element）算出来的值。

* list.eval / arr.eval：每个元素换成这个值
* list.filter：这是条件，只留下为真的元素
* list.agg / arr.agg：这是聚合（如 sum），每个列表得到一个值

@parameters
value:
    接上游节点。

@related
eval, element
"""
out = ch("value")
