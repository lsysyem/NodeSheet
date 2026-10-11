r"""
{
  "desc": "写出文件（Excel / CSV / Parquet / JSON / IPC / Avro）或复制到剪贴板：点“写出”才写；可以按列拆成多个文件",
  "category": "IO",
  "order": 20,
  "inputs": 1,
  "params": [
    {"name": "file", "type": "save_file", "hint": "写到哪个文件", "show": {"format": ["Excel", "CSV", "Parquet", "JSON", "IPC", "Avro"]}},
    {"name": "write", "type": "button", "label": "写出"},
    {"name": "format", "type": "menu", "label": "格式", "mode": true, "search": false, "options": ["Excel", "CSV", "Parquet", "JSON", "IPC", "Avro", "剪贴板"], "extensions": {"Excel": ["xlsx"], "CSV": ["csv", "tsv", "txt"], "Parquet": ["parquet"], "JSON": ["json", "ndjson", "jsonl"], "IPC": ["arrow", "feather", "ipc", "arrows"], "Avro": ["avro"]}},
    {"name": "worksheet", "type": "text", "default": "Sheet1", "hint": "sheet 名", "show": {"format": ["Excel"]}},
    {"name": "include_header", "type": "toggle", "default": "True", "hint": "写表头", "show": {"format": ["Excel", "CSV", "剪贴板"]}},
    {"name": "autofit", "type": "toggle", "default": "False", "hint": "列宽按内容自适应", "show": {"format": ["Excel"]}},
    {"name": "separator", "type": "text", "default": ",", "strip": false, "hint": "分隔符，制表符写 \\t", "show": {"format": ["CSV"]}},
    {"name": "include_bom", "type": "toggle", "default": "False", "hint": "带 BOM（Excel 直接打开中文不乱码）", "show": {"format": ["CSV"]}},
    {"name": "null_value", "type": "text", "hint": "空值写成什么，留空 = 空", "show": {"format": ["CSV"]}},
    {"name": "compression", "type": "menu", "options": ["zstd", "lz4", "snappy", "gzip", "uncompressed"], "show": {"format": ["Parquet"]}},
    {"name": "ipc_stream", "type": "toggle", "label": "流格式", "default": "False", "hint": "Arrow 的流格式（write_ipc_stream）；扩展名 .arrows 总是这样写", "show": {"format": ["IPC"]}},
    {"name": "ipc_compression", "type": "menu", "label": "compression", "options": ["uncompressed", "lz4", "zstd"], "show": {"format": ["IPC"]}},
    {"name": "ndjson", "type": "toggle", "default": "False", "hint": "每行一个 JSON 对象（.ndjson / .jsonl 总是这样写）", "show": {"format": ["JSON"]}},
    {"name": "avro_compression", "type": "menu", "label": "compression", "options": ["uncompressed", "snappy", "deflate"], "show": {"format": ["Avro"]}},
    {"name": "partition_by", "type": "columns", "pick": "names", "label": "按列拆分", "hint": "按这些列的值拆成多个文件，放进和文件同名的文件夹：结果/城市=北京/00000000.parquet；留空 = 写一个文件", "show": {"format": ["CSV", "Parquet", "JSON", "IPC"]}},
    {"name": "extra", "type": "kwargs", "label": "其他参数", "hint": "原样追加进括号，如 float_precision=2"}
  ]
}

把输入的表写成文件，或者复制到剪贴板。平时它只把输入原样传给下游、不写文件；点“写出”按钮才真的写——改参数不会反复写盘。

CSV、Parquet、IPC 和每行一个对象的 JSON 是流式写的（LazyFrame.sink_*）：一边算一边写，表比内存大也写得出来。Excel、JSON 数组、Avro、IPC 流格式和剪贴板要先把整张表算出来再写。

写 Excel 要 Python 里装了 xlsxwriter。CSV 想用 Excel 双击打开、中文不乱码，打开 include_bom。

@parameters
file:
    写到哪个文件。点“浏览”弹保存对话框；换了扩展名，“格式”跟着切。
write:
    点了才写：把上游整张表算出来，写进文件（或剪贴板）。
format:
    文件格式，按扩展名自动切。“剪贴板”：写成制表符分隔的文字放进剪贴板，可以直接粘贴进 Excel。
worksheet:
    Excel 里 sheet 的名字。
include_header:
    写表头。
autofit:
    按内容调 Excel 的列宽（慢一点）。
separator:
    分隔符，制表符写 \t。
include_bom:
    文件开头写 UTF-8 的 BOM。Excel 打开 CSV 时靠它认出 UTF-8，中文才不乱码。
null_value:
    空值写成什么，如 NULL；留空就空着。
compression:
    Parquet 的压缩方式。zstd 又小又快，一般不用改。
ipc_stream:
    写成 Arrow 的流格式（和 .arrow / .feather 的文件格式不同，主要在程序之间传数据用）。扩展名是 .arrows 时总是这样写。不能和“按列拆分”一起用。
ipc_compression:
    IPC（Arrow）文件的压缩方式。
ndjson:
    每行一个 JSON 对象（扩展名是 .ndjson / .jsonl 时总是这样写）。
avro_compression:
    Avro 文件的压缩方式。
partition_by:
    按这些列的值拆成多个文件（hive 分区）：文件路径去掉扩展名当文件夹，每个值一个子文件夹，如 结果/城市=北京/00000000.parquet。读回来用 read 节点，路径写 结果/**/*.parquet。JSON 拆分时每行一个对象。
extra:
    原样追加进 polars 写函数的括号，如 float_precision=2。

@inputs
1:
    要写出的表；它也原样传给下游。

@related
read

@polars
Excel: DataFrame.write_excel
CSV: LazyFrame.sink_csv
Parquet: LazyFrame.sink_parquet
JSON: DataFrame.write_json
JSON: LazyFrame.sink_ndjson
IPC: LazyFrame.sink_ipc
IPC: DataFrame.write_ipc_stream
CSV: pl.PartitionBy
Parquet: pl.PartitionBy
IPC: pl.PartitionBy
Avro: DataFrame.write_avro
剪贴板: DataFrame.write_clipboard
"""
# 在图里只是把输入原样传下去；点了“写出”按钮（action == "write"）才写文件
if action == "write":
    import pathlib

    path, fmt, extra = ch("file"), ch("format"), ch("extra")
    if not path and fmt != "剪贴板":
        raise ValueError("先填写要写到哪个文件")
    lower = (path or "").lower()
    ndjson = fmt == "JSON" and (ch("ndjson") or lower.endswith((".ndjson", ".jsonl")) or bool(ch("partition_by")))
    stream = fmt == "IPC" and (ch("ipc_stream") or lower.endswith(".arrows"))
    parts = ch("partition_by") if fmt in ("CSV", "Parquet", "JSON", "IPC") else []
    if parts and stream:
        raise ValueError("IPC 流格式不能按列拆分：关掉“流格式”，或换成 .arrow")
    # 按列拆分：文件路径去掉扩展名当文件夹
    target = pl.PartitionBy(str(pathlib.Path(path).with_suffix("")), key=parts) if parts else path
    lf = df.lazy()
    if fmt == "CSV":
        separator = ch("separator") or ","
        lf.sink_csv(
            target,
            separator="\t" if separator == "\\t" else separator,
            include_header=ch("include_header"),
            include_bom=ch("include_bom"),
            null_value=ch("null_value") or None,
            **extra,
        )
    elif fmt == "Parquet":
        lf.sink_parquet(target, compression=ch("compression"), **extra)
    elif fmt == "IPC" and not stream:
        lf.sink_ipc(target, compression=ch("ipc_compression"), **extra)
    elif ndjson:
        lf.sink_ndjson(target, **extra)
    else:
        # 这几种要先把整张表算出来
        data = lf.collect()
        if fmt == "Excel":
            data.write_excel(
                path,
                worksheet=ch("worksheet") or "Sheet1",
                include_header=ch("include_header"),
                autofit=ch("autofit"),
                **extra,
            )
        elif fmt == "JSON":
            data.write_json(path, **extra)
        elif fmt == "IPC":
            data.write_ipc_stream(path, compression=ch("ipc_compression"), **extra)
        elif fmt == "Avro":
            data.write_avro(path, compression=ch("avro_compression"), **extra)
        else:
            data.write_clipboard(include_header=ch("include_header"), **extra)
out = df
