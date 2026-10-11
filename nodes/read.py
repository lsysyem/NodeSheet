r"""
{
  "desc": "读取文件（Excel / CSV / Parquet / JSON / IPC / Avro）；纯文本、日志可以按行读",
  "category": "IO",
  "order": 10,
  "inputs": 0,
  "params": [
    {"name": "source", "type": "file", "hint": "文件路径", "required": true},
    {"name": "format", "type": "menu", "label": "格式", "mode": true, "search": false, "options": ["Excel", "CSV", "Parquet", "JSON", "IPC", "Avro"], "extensions": {"Excel": ["xlsx", "xlsm", "xls", "xlsb", "ods"], "CSV": ["csv", "tsv", "txt", "log"], "Parquet": ["parquet"], "JSON": ["json", "ndjson", "jsonl"], "IPC": ["arrow", "feather", "ipc", "arrows"], "Avro": ["avro"]}},
    {"name": "sheet_name", "type": "sheet", "file": "source", "show": {"format": ["Excel"]}},
    {"name": "has_header", "type": "toggle", "default": "True", "hint": "首行是表头", "show": {"format": ["Excel", "CSV"]}},
    {"name": "header_row", "type": "int", "hint": "表头在第几行（从 0 数），留空 = 第一行", "show": {"format": ["Excel"]}},
    {"name": "columns", "type": "columns", "pick": "names", "hint": "只读这些列：\"医院\", \"金额\" 或 0, 1", "show": {"format": ["Excel"]}},
    {"name": "schema_overrides", "type": "dict", "pick_dtype": true, "hint": "\"金额\": pl.Float64, \"编码\": pl.String", "show": {"format": ["Excel"]}},
    {"name": "infer_schema_length", "type": "int", "default": "100", "hint": "留空 = 扫描全表", "show": {"format": ["Excel", "JSON"]}},
    {"name": "drop_empty_rows", "type": "toggle", "default": "True", "hint": "丢掉空行", "show": {"format": ["Excel"]}},
    {"name": "drop_empty_cols", "type": "toggle", "default": "True", "hint": "丢掉空列", "show": {"format": ["Excel"]}},
    {"name": "engine", "type": "menu", "options": ["calamine", "openpyxl", "xlsx2csv"], "show": {"format": ["Excel"]}},
    {"name": "lines", "type": "toggle", "default": "False", "hint": "按行读：每行一个字符串，放进 line 列（不拆分隔符；日志、纯文本）", "show": {"format": ["CSV"]}},
    {"name": "separator", "type": "text", "default": ",", "strip": false, "hint": "分隔符，制表符写 \\t", "show": {"format": ["CSV"], "lines": ["False"]}},
    {"name": "encoding", "type": "menu", "options": ["utf8", "utf8-lossy", "gbk", "gb18030"], "show": {"format": ["CSV"]}},
    {"name": "skip_rows", "type": "int", "default": "0", "hint": "跳过开头几行", "show": {"format": ["CSV"]}},
    {"name": "csv_infer_schema_length", "type": "int", "label": "infer_schema_length", "default": "10000", "hint": "留空 = 扫描全表", "show": {"format": ["CSV"], "lines": ["False"]}},
    {"name": "null_values", "type": "exprs", "insert": "none", "hint": "当作空值：\"NA\", \"-\"", "show": {"format": ["CSV"], "lines": ["False"]}},
    {"name": "try_parse_dates", "type": "toggle", "default": "False", "hint": "自动识别日期", "show": {"format": ["CSV"], "lines": ["False"]}},
    {"name": "ndjson", "type": "toggle", "default": "False", "hint": "每行一个 JSON 对象（.ndjson / .jsonl 总是这样读）", "show": {"format": ["JSON"]}},
    {"name": "ipc_stream", "type": "toggle", "label": "流格式", "default": "False", "hint": "Arrow 的流格式（read_ipc_stream）；扩展名 .arrows 总是这样读", "show": {"format": ["IPC"]}},
    {"name": "n_rows", "type": "int", "hint": "只读前 N 行，留空 = 全部", "show": {"format": ["Parquet", "IPC", "Avro"]}},
    {"name": "extra", "type": "kwargs", "label": "其他参数", "hint": "原样追加进括号，如 n_rows=1000"}
  ]
}

读一个文件，作为节点图的源头。打开文件时自动建的源节点就是它；再加一个 read，就能把第二张表接进 join / concat。

换了文件，“格式”按扩展名自动切换。源节点的参数没改过（换 sheet 除外）时，表格直接用查看器自己读的数据，不启动 polars。

* Excel：pl.read_excel，默认用 calamine 引擎，最快
* CSV：pl.scan_csv 懒读，大文件也快；编码选 gbk / gb18030 时整个读进来
* CSV + 按行读：每行一个字符串放进 line 列，不拆分隔符，适合日志、纯文本，之后用表达式节点（str.split、str.extract…）拆
* Parquet / IPC：懒读，只取用到的列
* JSON：一个 JSON 数组用 pl.read_json；每行一个对象（.ndjson / .jsonl）用 pl.scan_ndjson
* IPC 流格式（.arrows）：pl.read_ipc_stream，整个读进来
* Avro：pl.read_avro，整个读进来

@parameters
source:
    要读的文件，点“浏览”选。工程文件里存的是路径，原路径找不到时会找工程旁边的同名文件。
format:
    文件格式。一般不用管：换文件时按扩展名自动切。
sheet_name:
    Excel 的哪个 sheet，留空 = 第一个。
has_header:
    第一行是不是表头。关掉时列名是 column_1、column_2…
header_row:
    表头不在第一行时（上面有标题、说明），填表头在第几行，从 0 数。
columns:
    只读这些列，列名或列号都行。列很多、只用几列时快不少。
schema_overrides:
    指定某些列的类型，不让 polars 猜。编码、身份证号这类该当文字的数字列常用，如 "编码": pl.String。
infer_schema_length:
    看前多少行来猜每列的类型。前面都是整数、后面出现小数时会报错：调大它，或留空（扫全表）。
drop_empty_rows:
    丢掉整行都是空的行。
drop_empty_cols:
    丢掉整列都是空的列。
engine:
    读 Excel 用哪个库。calamine 最快；个别文件读不对时换 openpyxl 试试。
lines:
    按行读：每行一个字符串。日志、格式不规整的文本先这样读进来，再用表达式拆。
separator:
    分隔符，制表符写 \t（.tsv 文件）。
encoding:
    文字编码。中文的老文件多半是 gbk / gb18030；utf8-lossy = 遇到坏字节换成 �，不报错。
skip_rows:
    跳过开头几行（表头之前的说明）。
csv_infer_schema_length:
    看前多少行来猜每列的类型，留空 = 扫全表。
null_values:
    这些写法当成空值，如 "NA", "-", "无"。
try_parse_dates:
    自动把像日期的文字列转成日期。
ndjson:
    每行一个 JSON 对象（扩展名是 .ndjson / .jsonl 时总是这样读）。
ipc_stream:
    按 Arrow 的流格式读（和 .arrow / .feather 的文件格式不同）。扩展名是 .arrows 时总是这样读；流格式要整个读进来，不能只读用到的列。
n_rows:
    只读前 N 行，留空 = 全部。
extra:
    原样追加进 polars 读函数的括号，如 n_rows=1000。能写哪些见下面 polars 的文档。

@examples
读 GBK 编码、分号分隔的 CSV：
{{{
格式       CSV
encoding   gbk
separator  ;
}}}
Excel 的表头在第 3 行（上面两行是标题）：
{{{
header_row  2
}}}

@related
write, create, join, concat

@polars
Excel: pl.read_excel
CSV: pl.scan_csv
Parquet: pl.scan_parquet
JSON: pl.read_json
JSON: pl.scan_ndjson
IPC: pl.scan_ipc
IPC: pl.read_ipc_stream
Avro: pl.read_avro
"""
path, fmt, extra = ch("source"), ch("format"), dict(ch("extra"))
if fmt == "Excel":
    options = dict(extra.pop("read_options", None) or {})
    if ch("header_row") is not None:
        options["header_row"] = ch("header_row")
    out = pl.read_excel(
        path,
        sheet_name=ch("sheet_name") or None,
        has_header=ch("has_header"),
        columns=ch("columns") or None,
        schema_overrides=ch("schema_overrides") or None,
        infer_schema_length=ch("infer_schema_length"),
        drop_empty_rows=ch("drop_empty_rows"),
        drop_empty_cols=ch("drop_empty_cols"),
        engine=ch("engine"),
        read_options=options or None,
        **extra,
    )
elif fmt == "CSV" and ch("lines"):
    encoding = ch("encoding")
    if encoding == "utf8":
        out = pl.scan_lines(path, **extra)
    else:
        # scan_lines 只认 utf8：其他编码用 Python 解码
        with open(path, encoding="utf-8" if encoding == "utf8-lossy" else encoding, errors="replace") as f:
            out = pl.DataFrame({"line": f.read().splitlines()})
    out = out.slice(ch("skip_rows") or 0)
elif fmt == "CSV":
    separator = ch("separator") or ","
    options = dict(
        separator="\t" if separator == "\\t" else separator,
        has_header=ch("has_header"),
        skip_rows=ch("skip_rows") or 0,
        infer_schema_length=ch("csv_infer_schema_length"),
        null_values=ch("null_values") or None,
        try_parse_dates=ch("try_parse_dates"),
        **extra,
    )
    encoding = ch("encoding")
    # scan_csv 只认 utf8；其他编码用 read_csv 整个读进来
    if encoding in ("utf8", "utf8-lossy"):
        out = pl.scan_csv(path, encoding=encoding, **options)
    else:
        out = pl.read_csv(path, encoding=encoding, **options)
elif fmt == "JSON":
    if ch("ndjson") or path.lower().endswith((".ndjson", ".jsonl")):
        out = pl.scan_ndjson(path, infer_schema_length=ch("infer_schema_length"), **extra)
    else:
        out = pl.read_json(path, infer_schema_length=ch("infer_schema_length"), **extra)
elif fmt == "IPC" and (ch("ipc_stream") or path.lower().endswith(".arrows")):
    # 流格式没有 scan：整个读进来
    out = pl.read_ipc_stream(path, n_rows=ch("n_rows"), **extra)
elif fmt == "IPC":
    out = pl.scan_ipc(path, n_rows=ch("n_rows"), **extra)
elif fmt == "Avro":
    out = pl.read_avro(path, n_rows=ch("n_rows"), **extra)
else:
    out = pl.scan_parquet(path, n_rows=ch("n_rows"), **extra)
