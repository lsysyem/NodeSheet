# 核对 polars 文档（https://docs.pola.rs/api/python/version/2/reference/）里的每个 API 在查看器里有没有对应的节点，
# 没有的写明为什么不做。结果写到 docs/polars-api-coverage.md。
#
# 怎么算“有”：
#   * 节点定义帮助里的 @polars（nodes/*.py：`方式: DataFrame.join`）——这个节点（选这个方式）就是它
#   * nodes/catalog.json 里生成的表达式节点（Expr 方法、命名空间、部分 pl.* 函数）
#   * 下面 RULES 里写的：换个写法能做的（LazyFrame 和 DataFrame 同名、Series 和 Expr 同名…）、不做的和原因
# 文档里有、上面都没说到的 API 会列在最后“没有说明的”，脚本以 1 退出——升级 polars 后跑一遍，新出的 API 都要有个说法。
#
#   python engine/api_coverage.py            抓文档、写 docs/polars-api-coverage.md
#   python engine/api_coverage.py --cache 目录   抓到的页面存在 / 取自这个目录（反复改原因时不用每次都抓）
import concurrent.futures
import fnmatch
import json
import re
import sys
import urllib.parse
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
BASE = "https://docs.pola.rs/api/python/version/2/reference/"
OUT = ROOT / "docs" / "polars-api-coverage.md"

# 文档的各节（顺序照文档首页）。每节的 index 页列全部、子页按用途分，以子页为准
SECTIONS = [
    ("dataframe", "DataFrame"),
    ("lazyframe", "LazyFrame"),
    ("series", "Series"),
    ("expressions", "Expressions"),
    ("selectors", "Selectors"),
    ("datatype_expr", "DataType expressions"),
    ("functions", "Functions"),
    ("datatypes", "Data types"),
    ("schema", "Schema"),
    ("io", "Input/output"),
    ("catalog", "Catalog"),
    ("config", "Config"),
    ("api", "Extending the API"),
    ("plugins", "Plugins"),
    ("sql", "SQL Interface"),
    ("exceptions", "Exceptions"),
    ("testing", "Testing"),
    ("metadata", "Metadata"),
]

HAVE, ALT, NO, TODO = "有节点", "换个写法", "不做", "可以加"

# 生成的表达式节点的说法
GENERATED = "同名的表达式节点（expr 网络里按 Tab 搜名字；`engine/gen_catalog.py` 从 polars 签名生成）"

# 表达式级的“表格 → 一列”的写法：DataFrame 上返回一列（Series）的方法，在 expr 网络里用对应的表达式
ROW_SERIES = "返回的是一列（Series）不是表；在 expr 节点（with_columns / select）的网络里用对应的表达式："

# (API 名的通配, 归类, 说明)。从上往下第一条对上的算数；API 名的写法见 short()
RULES = [
    # ---------------- DataFrame ----------------
    ("DataFrame.group_by_dynamic", HAVE, "`expr` 节点：上下文 group_by，分组后 agg，填“时间窗口”"),
    ("DataFrame.rolling", HAVE, "`expr` 节点：上下文 group_by，分组后 agg，填“滚动窗口”"),
    ("DataFrame.remove", HAVE, "`filter` 节点（方式“去掉满足的”）、`expr` 节点（上下文 filter，打开 remove）"),
    ("DataFrame.match_to_schema", HAVE, "`cast_columns` 节点（方式 match_to_schema）"),
    ("DataFrame.write_ndjson", HAVE, "`write` 节点（JSON，打开 ndjson 或扩展名 .ndjson / .jsonl）"),
    ("DataFrame.limit", ALT, "就是 head：`slice` 节点（取法 head）"),
    ("DataFrame.hstack", ALT, "`concat` 节点（how = horizontal）"),
    ("DataFrame.vstack DataFrame.extend", ALT, "`concat` 节点（how = vertical）"),
    ("DataFrame.select_seq DataFrame.with_columns_seq", ALT,
     "和 select / with_columns 结果一样，只是不并行算（`expr` 节点）；不值得多一个选项"),
    ("DataFrame.insert_column DataFrame.replace_column", ALT, "`expr` 节点（with_columns：同名覆盖、新名追加）；列的位置用 `select` 节点排"),
    ("DataFrame.drop_in_place", ALT, "`select` 节点（方式 drop）"),
    ("DataFrame.clear", ALT, "`slice` 节点（head，n = 0）：只留表结构"),
    ("DataFrame.to_struct", ALT, "expr 网络里 `pl.struct`（col 填 `cs.all()`）"),
    ("DataFrame.sum_horizontal DataFrame.max_horizontal DataFrame.min_horizontal DataFrame.mean_horizontal", ALT,
     ROW_SERIES + "`pl.sum_horizontal` / `pl.max_horizontal` / … 节点"),
    ("DataFrame.fold", ALT, ROW_SERIES + "`pl.fold` 节点"),
    ("DataFrame.hash_rows", ALT, ROW_SERIES + "`pl.struct`（`cs.all()`）→ `hash`"),
    ("DataFrame.is_unique", ALT, ROW_SERIES + "`pl.struct`（`cs.all()`）→ `is_unique`；只留不重复的行用 `unique` 节点（keep = none）"),
    ("DataFrame.n_unique", ALT, "返回一个数：expr 节点 select 里 `pl.struct`（`cs.all()`）→ `n_unique`；每列各自的个数是 `describe` 节点的 n_unique"),
    ("DataFrame.partition_by", NO,
     "结果是好几张表（每组一张），节点只有一个输出；按组算用 `expr` 的 group_by，要每组一张表就各接一个 filter"),
    ("DataFrame.pipe DataFrame.map_rows DataFrame.map_columns", NO,
     "参数是 Python 函数（对整表 / 每行 / 每列调一次）；要写函数就用 `code` 节点，按值算的在 expr 网络里有 `map_elements` / `map_batches` 节点"),
    ("DataFrame.__getitem__ DataFrame.__setitem__", NO, "Python 的 `df[...]` 语法，取列 / 取行的功能 `select` / `slice` / `expr` 都有"),
    ("DataFrame.get_column DataFrame.get_columns DataFrame.get_column_index DataFrame.to_series DataFrame.item "
     "DataFrame.row DataFrame.rows DataFrame.rows_by_key DataFrame.iter_columns DataFrame.iter_rows DataFrame.iter_slices", NO,
     "把数据取成 Python 对象（列、元组、标量、迭代器），不是表；在查看器里就是看表格本身"),
    ("DataFrame.to_dict DataFrame.to_dicts DataFrame.to_arrow DataFrame.to_numpy DataFrame.to_pandas DataFrame.to_jax "
     "DataFrame.to_torch DataFrame.__array__ DataFrame.__arrow_c_stream__", NO,
     "转成别的 Python 库的对象（pandas / numpy / torch…），节点之间传的只有 polars 表；要给别的程序用就 `write` 写成文件"),
    ("DataFrame.to_init_repr", NO, "生成造这张表的 Python 代码；整个节点图导出成代码见表格面板的“Python 脚本”页签"),
    ("DataFrame.columns DataFrame.dtypes DataFrame.schema DataFrame.shape DataFrame.height DataFrame.width DataFrame.flags "
     "DataFrame.collect_schema", NO,
     "表的属性（列名、类型、行列数），表格面板已经显示：表头第二行是类型，路径栏是行×列"),
    ("DataFrame.estimated_size DataFrame.n_chunks DataFrame.is_empty DataFrame.is_sorted", NO,
     "返回一个数或真假（占多少内存、有没有行、排没排好序），不是表；行数在路径栏"),
    ("DataFrame.equals", NO, "返回真假；要比较两张表，用 `join`（how = anti）找出不一样的行"),
    ("DataFrame.glimpse DataFrame.show", NO, "打印到控制台的另一种排版，查看器自己画表格"),
    ("DataFrame.lazy DataFrame.rechunk DataFrame.shrink_to_fit DataFrame.set_sorted DataFrame.clone", NO,
     "不改数据（转 LazyFrame、整理内存、标记已排序、复制）；引擎里的表本来就是 LazyFrame，每步的结果也不会被改"),
    ("DataFrame.plot Series.plot", NO,
     "画图（Altair）：结果是图表不是表，要多带 altair 包，查看器只画表格；画图是另一块功能，要做的话应该是查看器自己的图表面板"),
    ("DataFrame.style", NO, "把表排成 HTML 的样式表格（great_tables 包），用来放进网页 / 笔记本；查看器自己画表格"),
    ("DataFrame.serialize DataFrame.deserialize", NO, "polars 自己的二进制格式；节点图存在工程文件（.rtv）里，数据用 `write` 写成 Parquet / IPC"),

    # ---------------- GroupBy ----------------
    ("GroupBy.agg GroupBy.all GroupBy.len GroupBy.sum GroupBy.mean GroupBy.median GroupBy.min GroupBy.max "
     "GroupBy.first GroupBy.last GroupBy.n_unique GroupBy.quantile GroupBy.head GroupBy.tail", HAVE,
     "`expr` 节点：上下文 group_by，“分组后”选同名的一项"),
    ("GroupBy.having", HAVE, "`expr` 节点 group_by：网络里接“条件”输出"),
    ("GroupBy.map_groups", NO, "参数是 Python 函数，每组调一次；用 `code` 节点（逐组的计算在 agg 网络里写表达式更快）"),
    ("GroupBy.__iter__", NO, "在 Python 里逐组循环，结果不是表"),

    # ---------------- LazyFrame（DataFrame 同名的见 lazy_twin） ----------------
    ("LazyFrame.first", ALT, "`slice` 节点（head，n = 1）"),
    ("LazyFrame.last", ALT, "`slice` 节点（tail，n = 1）"),
    ("LazyFrame.cache", NO, "不改数据；引擎按（代码 + 参数 + 上游）缓存每步的结果"),
    ("LazyFrame.collect LazyFrame.collect_async LazyFrame.collect_batches LazyFrame.execute LazyFrame.sink_batches", NO,
     "执行查询的方式；引擎只 collect 显示着的节点（流式引擎），结果写成 parquet 给表格看"),
    ("LazyFrame.explain", HAVE, "表格面板的“查询计划”页签：显示节点优化后、优化前的计划"),
    ("LazyFrame.show_graph", ALT,
     "“查询计划”页签点“画成树”（explain(format=\"tree\")，同样的树画成文字）；show_graph 出图片要 graphviz，不带"),
    ("LazyFrame.inspect", NO, "执行时把中间结果 print 到控制台；查看器里点那个节点就能看它的结果"),
    ("LazyFrame.map_batches LazyFrame.pipe LazyFrame.pipe_with_schema", NO, "参数是 Python 函数；用 `code` 节点"),
    ("LazyFrame.remote", NO, "交给 Polars Cloud 执行，要云账号"),
    ("LazyFrame.from_lazyframe_resolver", NO, "给写 IO 插件的人用的内部接口"),
    ("LazyFrame.serialize LazyFrame.deserialize", NO, "把查询计划存成 polars 的二进制格式；节点图存在工程文件（.rtv）里"),
    ("LazyFrame.sink_delta LazyFrame.sink_iceberg", NO, "见 IO 里 Delta Lake / Iceberg 那两行：要额外的包，不是一个文件"),
    ("LazyFrame.columns LazyFrame.dtypes LazyFrame.schema LazyFrame.width LazyFrame.collect_schema", NO,
     "表的属性，表格面板已经显示（表头第二行是类型）"),
    ("LazyFrame.lazy LazyFrame.clone", NO, "不改数据"),
    ("LazyFrame.__getitem__", NO, "Python 的 `lf[...]` 切片语法；`slice` 节点"),
    ("pl.QueryOptFlags", NO, "关掉某些查询优化，排查 polars 的问题用"),
    ("pl.Engine pl.InMemoryEngine pl.StreamingEngine pl.RemoteEngine", NO,
     "选执行引擎（collect(engine=…)）；引擎用 polars 2 默认的流式执行，内存里放不下的才有区别"),
    ("pl.GPUEngine", NO, "用 NVIDIA GPU 执行，要装 cudf-polars（几百 MB）和 CUDA；表格查看器的数据量用 CPU 就够"),
    ("InProcessQuery.*", NO, "后台执行（collect(background=True)）的句柄；引擎自己有排队和取消（过期的请求不发）"),
    ("QueryResult.*", NO, "LazyFrame.execute 的结果（还不稳定的新接口）；引擎直接 collect"),
    ("LazyFrameResolver* FilterExpr* pl.ResolvedLazyFrameProps", NO, "给写 IO 插件的人用的内部接口"),

    # ---------------- Series ----------------
    ("Series.list.eval Series.list.filter Series.list.agg Series.arr.eval Series.arr.agg", ALT, "`eval` 节点（双击进去连 element → 结果）"),
    ("Series.cut Series.qcut", NO, "polars 2 里废弃了（Expr 的也是），换成 `bin_intervals` / `bin_quantiles`，这两个有表达式节点"),
    ("Series.to_dummies", ALT, "`to_dummies` 节点"),
    ("Series.describe", ALT, "`describe` 节点"),
    ("Series.rename", ALT, "输出节点的列名（alias），或 `rename` 节点"),
    ("Series.dt.max Series.dt.min", ALT, "时间列直接用 `max` / `min` 节点"),
    ("Series.struct.fields Series.struct.schema", ALT, "结构列的字段和类型在表头第二行（`struct{x: i64, …}`）；改名用 `struct.rename_fields`"),
    ("Series.zip_with", ALT, "就是条件取值：`when` 节点"),
    ("Series.scatter Series.set", ALT, "按位置 / 按条件改值：`when` 节点（按位置的条件写 `pl.int_range(pl.len()) == 3`）"),
    ("Series.new_from_index", ALT, "`get` 节点取第 i 个值：在 with_columns 里一个值会铺满每一行"),
    ("Series.extend", ALT, "`concat` 节点"),
    ("Series.sql", ALT, "`sql` 节点"),
    ("Series.shrink_dtype", NO, "只在 Series 上有：把数值类型缩到装得下的最小类型（省内存）；要指定类型用 `cast` / `cast_columns`"),
    ("Series.to_list Series.to_frame Series.to_arrow Series.to_numpy Series.to_pandas Series.to_jax Series.to_torch "
     "Series.to_init_repr Series.__array__ Series.__arrow_c_stream__", NO, "转成 Python 对象 / 别的库的对象；节点之间传的是 polars 表"),
    ("Series.dtype Series.name Series.shape Series.flags", NO, "一列的属性，表头已经显示（列名、类型）"),
    ("Series.chunk_lengths Series.n_chunks Series.get_chunks Series.estimated_size Series.rechunk Series.shrink_to_fit "
     "Series.clear Series.clone Series.equals", NO, "内存布局 / 复制 / 比较，不改数据或结果不是表"),
    ("Series.__getitem__ Series.__setitem__", NO, "Python 的 `s[...]` 语法；取值用 `get` / `gather` / `slice` 节点"),
    # Series 和 Expr 同名的见 series_twin

    # ---------------- Expressions ----------------
    ("Expr.list.eval Expr.list.filter Expr.list.agg Expr.arr.eval Expr.arr.agg", HAVE, "`eval` 节点"),
    ("Expr.cut Expr.qcut", NO, "polars 2 里废弃了，换成 `bin_intervals` / `bin_quantiles`（有表达式节点）"),
    ("Expr.list.__getitem__", ALT, "就是 `list.get` / `list.slice` 节点"),
    ("Expr.struct.__getitem__", ALT, "就是 `struct.field` 节点"),
    ("Expr.inspect", NO, "执行时把值 print 到控制台；网络里选中哪个节点表格就预览哪个节点的值"),
    ("Expr.pipe", NO, "参数是 Python 函数（拿表达式返回表达式）；直接在网络里接节点，或用 `snippet` 写"),
    ("Expr.deserialize Expr.meta.serialize", NO, "表达式存成 polars 的二进制格式；网络存在工程文件里，复制表达式用参数面板底下那行文字"),
    ("Expr.meta.*", NO,
     "回答“这个表达式是什么样”（输出列名、用到哪些列、是不是标量…），结果是 Python 值不是数据；"
     "引擎用它们做提醒：行数和表对不上（is_length_preserving / is_scalar）、预览带上用到的列（root_names）"),
    ("Expr.ext.* Series.ext.*", NO,
     "扩展类型的列要先在 Python 里 `register_extension_type` 注册一个类型才有，节点图里没地方注册；遇到了在 `code` 节点里处理"),

    # pl.sum("a") 这类顶层函数就是 pl.col("a").sum()
    ("pl.all pl.any pl.approx_n_unique pl.count pl.cum_count pl.cum_sum pl.first pl.head pl.implode pl.last "
     "pl.max pl.mean pl.median pl.min pl.n_unique pl.quantile pl.std pl.sum pl.tail pl.var", ALT,
     "就是 `pl.col(…).同名方法`：col 节点接同名的表达式节点（`pl.all()` = col 填 `cs.all()`）"),
    ("pl.arange", ALT, "`pl.int_range` 的别名，用 `pl.int_range` 节点"),
    ("pl.col pl.functions.col.Col pl.functions.col.Col.__call__", HAVE, "`col` 节点"),
    ("pl.functions.col.Col.__getattr__", ALT, "`pl.col.金额` 是 `pl.col(\"金额\")` 的另一种写法：`col` 节点"),

    # ---------------- Selectors ----------------
    ("cs.expand_selector cs.is_selector", NO, "返回 Python 值（选中哪些列名 / 是不是选择器），不是表达式"),
    ("cs.*", ALT,
     "选择器写在参数里：`col` 节点、select / unique / drop_nulls… 的“列”、cast_columns 的键都可以写 `cs.numeric()` 这样的；"
     "不单做节点，因为选择器是“选哪几列”，不是对值的计算"),

    # ---------------- DataType expressions ----------------
    ("pl.dtype_of pl.self_dtype pl.DataType.to_dtype_expr DataTypeExpr.*", ALT,
     "结果是“类型”不是值，用在要填类型的地方：类型参数（`cast`、`lit`、生成的节点里的 dtype / return_dtype）可以写 `pl.dtype_of(\"列\")`"),

    # ---------------- Functions ----------------
    ("pl.union", ALT, "`concat` 节点（union 不保证行的顺序，其余一样）"),
    ("pl.merge_sorted", ALT, "`concat` 节点（方式 merge_sorted）"),
    ("pl.from_dicts pl.from_records", ALT, "`create` 节点：from_dict（按列写）或 from_repr（贴打印的表格）"),
    ("pl.from_arrow pl.from_dataframe pl.from_numpy pl.from_pandas pl.from_torch", NO,
     "从别的 Python 库的对象建表；节点图里没有这些对象，数据从 `read`（文件）或 `create`（直接写）来"),
    ("pl.json_normalize", NO,
     "输入是 Python 的字典列表（json.load 之后）；JSON 文件用 `read` 读，嵌套的结构用 `explode`（unnest）和 `struct.field` 拆"),
    ("pl.pipe_with_dtype", NO,
     "参数是 Python 函数，一次拿到好几个表达式和类型；单个表达式的 `pipe_with_dtype` 有表达式节点，多个的用 `snippet` 写"),
    ("pl.align_frames", NO, "结果是好几张表；对齐两张表用 `join`（how = full）"),
    ("pl.collect_all pl.collect_all_async pl.explain_all pl.defer", NO, "执行控制（一次执行好几个查询…），引擎自己调度"),
    ("pl.StringCache pl.enable_string_cache pl.disable_string_cache pl.using_string_cache", NO,
     "polars 2 里什么也不做（Categorical 改成了全局的 Categories），只是为了兼容旧代码"),
    ("pl.escape_regex", ALT, "返回的是 Python 字符串；对列里的文字用 `str.escape_regex` 节点"),
    ("pl.set_random_seed", ALT, "`slice` 节点 sample 的 seed 参数（全局种子会影响别的节点，不做）"),
    ("pl.register_extension_type pl.unregister_extension_type pl.get_extension_type pl.BaseExtension", NO,
     "注册扩展类型要写一个 Python 类，节点图里没地方放；不注册的 `Extension(\"名字\", 存储类型)` 在类型参数里能选；见 Expr.ext"),

    # ---------------- Data types ----------------
    ("pl.Utf8", ALT, "就是 `pl.String`（旧名字）"),
    ("pl.Array pl.Binary pl.Boolean pl.Categorical pl.Categories pl.DataType pl.Date pl.Datetime pl.Decimal pl.Duration "
     "pl.Enum pl.Extension pl.Field pl.Float* pl.Int* pl.List pl.Map pl.Null pl.Object pl.String pl.Struct pl.Time pl.UInt* "
     "pl.Unknown", ALT,
     "类型不是操作，写在参数里：类型参数（`cast`、`lit`、生成的节点里的 dtype / return_dtype）旁边的下拉列着全部类型，"
     "带参数的（`Decimal(38, 2)`、`Datetime(\"ms\", \"Asia/Shanghai\")`、`List(Int64)`、`Struct({\"a\": Int64})`…）在输入框里改；"
     "表格级的用 `cast_columns` 的“列: 类型”；列的类型显示在表头第二行"),

    # ---------------- Schema ----------------
    ("pl.Schema pl.Schema.*", NO, "表的结构（列名 → 类型）这个对象本身；表头第二行就是它，要按结构对齐表用 `cast_columns`（match_to_schema）"),

    # ---------------- Input/output ----------------
    ("pl.read_csv", HAVE, "`read` 节点（CSV；gbk / gb18030 编码时用 read_csv，utf8 用 scan_csv）"),
    ("pl.read_parquet pl.read_ipc pl.read_ndjson", ALT, "`read` 节点用的是同格式的 scan_*（懒读，只读用到的列和行），结果一样"),
    ("DataFrame.write_csv DataFrame.write_parquet DataFrame.write_ipc DataFrame.write_ndjson", ALT,
     "`write` 节点用的是同格式的 sink_*（流式写，不用整表进内存），结果一样"),
    ("pl.read_ods", ALT, "`read` 节点（Excel 格式，.ods 文件）：read_excel 的 calamine 引擎也读 ods"),
    ("pl.scan_ndjson", HAVE, "`read` 节点（JSON，打开 ndjson 或扩展名 .ndjson / .jsonl）"),
    ("pl.read_lines pl.scan_lines", HAVE, "`read` 节点（CSV 格式，打开“按行读”）"),
    ("pl.ScanCastOptions", ALT, "`read` 节点的“其他参数”里写 `cast_options=pl.ScanCastOptions(…)`（读一组文件、类型不一致时用）"),
    ("pl.read_ipc_schema pl.read_parquet_schema pl.read_parquet_metadata", NO, "只读文件的结构 / 元信息；打开文件后表头就显示各列类型"),
    ("pl.read_database pl.read_database_uri DataFrame.write_database", NO,
     "要数据库连接（connectorx / ADBC / SQLAlchemy 这些依赖，打包的 Python 里没有），账号密码会存进工程文件；查看器只管文件"),
    ("pl.read_delta pl.scan_delta DataFrame.write_delta", NO, "Delta Lake 要 deltalake 包，是数据湖的目录 + 事务日志，不是一个文件"),
    ("pl.scan_iceberg DataFrame.write_iceberg", NO, "Iceberg 要 pyiceberg 包和一个 catalog 服务，不是一个文件"),
    ("pl.scan_pyarrow_dataset", NO, "要 pyarrow 的 Dataset 对象；一个目录下的多个 parquet，`read` 的路径可以直接写通配符（`数据/*.parquet`，scan_parquet 认），只是文件改了缓存不会自己失效（按路径取修改时间）"),
    ("pl.scan_lance", NO, "Lance 要 pylance 包，数据集是一个目录 + 版本，不是一个文件"),
    ("pl.scan_external_reader io.external_reader.*", NO, "用 Python 写的自定义读取器（FileReaderBuilder 子类）；要用的话在 `code` 节点里写"),
    ("pl.scan_arrow_c_stream", NO, "从实现了 Arrow C 接口的 Python 对象读，节点图里没有这种对象"),
    ("pl.CredentialProvider*", NO, "云存储（S3 / Azure / GCS）的凭据；查看器读本地文件，凭据也不该存进工程文件"),
    ("pl.FileProviderArgs", NO, "PartitionBy 自定义文件名的回调拿到的参数，要写 Python 函数；`write` 的“按列拆分”用 polars 默认的文件名"),

    # ---------------- Catalog / Config / 扩展 / 插件 ----------------
    ("Catalog* catalog.*", NO, "Databricks Unity Catalog，要云账号和服务地址"),
    ("Config.*", NO,
     "控制 print(df) 的排版（行数、列宽、小数位…）和引擎调优；查看器自己画表格，不经过 polars 的打印"),
    ("api.register_*", NO, "给 polars 加自定义命名空间要写 Python 代码；本程序扩展节点的方式是自己写节点类型（nodes/*.py，“节点 → 新建类型”）"),
    ("plugins.* io.plugins.*", NO, "polars 插件是另外编译的 Rust 包 / Python 的 IO 源；要用的话在 `code` 节点里 import"),

    # ---------------- SQL ----------------
    ("pl.SQLContext SQLContext SQLContext.*", HAVE, "`sql` 节点（第 1 个输入注册成 self / df，其余是 df2 ~ df4）"),
    ("pl.sql", ALT, "`sql` 节点；pl.sql 是按 Python 全局变量名找表，节点图里的表只有输入"),
    ("DataFrame.sql LazyFrame.sql", ALT, "`sql` 节点（表名写 self）"),

    # ---------------- Exceptions / Testing / Metadata ----------------
    ("exceptions.*", NO, "错误的类型，不是功能；出错时节点标红、状态栏和参数面板显示 polars 的报错"),
    ("testing.*", NO, "写测试用的断言和随机数据生成；比较两张表用 `join`（anti），造数据用 `create`"),
    ("pl.build_info pl.show_versions pl.get_index_type pl.thread_pool_size", NO, "版本 / 编译信息；引擎启动时检查 polars 版本（要 2.x）"),
]


def fetch(url, cache):
    if cache:
        f = cache / urllib.parse.quote(url[len(BASE):], safe="")
        if f.exists():
            return f.read_text(encoding="utf-8")
    req = urllib.request.Request(url, headers={"User-Agent": "Mozilla/5.0"})
    text = urllib.request.urlopen(req, timeout=60).read().decode("utf-8")
    if cache:
        f.write_text(text, encoding="utf-8")
    return text


def article(html):
    m = re.search(r'<article class="bd-article"[^>]*>(.*?)</article>', html, re.S)
    return m.group(1) if m else html


def docs_apis(cache):
    """文档里的 API：{全名: (节, 页面标题, 页面地址)}。全名照文档，如 polars.DataFrame.join。"""
    pages = [BASE + f"{s}/index.html" if s in ("dataframe", "lazyframe", "series", "expressions", "datatype_expr",
                                                  "schema", "catalog", "sql") else BASE + f"{s}.html"
             for s, _ in SECTIONS]
    seen = {}
    todo = pages
    with concurrent.futures.ThreadPoolExecutor(16) as pool:
        while todo:
            got = list(pool.map(lambda u: (u, fetch(u, cache)), todo))
            todo = []
            for url, html in got:
                seen[url] = html
                # 节里的子页（左边目录里的；不进 api/ 下的单个 API 页）
                for href in re.findall(r'href="([^"#]*\.html)', html):
                    sub = urllib.parse.urljoin(url, href)
                    rest = sub[len(BASE):]
                    if sub.startswith(BASE) and "api/" not in rest and sub not in seen and sub not in todo \
                            and rest.split("/")[0] == url[len(BASE):].split("/")[0] and "/" in rest:
                        todo.append(sub)
    found = {}
    for url, html in seen.items():
        rest = url[len(BASE):]
        section = rest.split("/")[0].removesuffix(".html")
        body = article(html)
        title = re.search(r"<h1>(.*?)<a", body, re.S)
        title = re.sub("<.*?>", "", title.group(1)).strip() if title else section
        names = re.findall(r'href="[^"#]*api/(polars\.[^"#]+)\.html', body)
        # 选择器、部分函数页把 API 直接写在本页（没有单独的 api/ 页）
        names += re.findall(r'<dt class="sig[^"]*" id="(polars\.[\w.]+)"', body)
        # 没有 id、也没有单独 API 页的（Schema 页）：从签名里取名字，方法接在前面的类后面
        owner = None
        for sig in [] if names else re.findall(r'<dt class="sig sig-object py">(.*?)</dt>', body, re.S):
            text = re.sub(r"<[^>]+>", "", sig).strip().removeprefix("class ").strip()
            ident = re.match(r"[\w.]+", text)
            if not ident:
                continue
            ident = ident.group(0)
            if ident.startswith("polars."):
                owner = ident
                names.append(ident)
            elif owner:
                names.append(f"{owner}.{ident}")
        # 只讲一个 API 的页（pl.col）：标题就是它
        if re.fullmatch(r"polars\.[\w.]+", title):
            names.insert(0, title)
        for name in dict.fromkeys(names):
            index = rest.endswith("index.html")
            if name not in found or (not index and found[name][2].endswith("index.html")):
                found[name] = (section, title, url)
    return found


CLASSES = {"DataFrame", "LazyFrame", "Series", "Expr", "GroupBy", "LazyGroupBy", "Config", "SQLContext", "Catalog",
           "DataTypeExpr", "InProcessQuery", "QueryResult", "LazyFrameResolver", "FilterExpr"}
MODULES = {"testing", "exceptions", "api", "catalog", "plugins", "io"}


def short(full):
    """文档全名 → 这里的写法：DataFrame.join、Expr.str.slice、GroupBy.agg、pl.read_csv、pl.Int64、cs.numeric。"""
    n = full.removeprefix("polars.")
    for module, to in [("dataframe.group_by.", ""), ("lazyframe.group_by.", ""), ("lazyframe.in_process.", ""),
                       ("lazyframe.query_result.", ""), ("lazyframe.resolver.", ""), ("lazyframe.engine_config.", ""),
                       ("io.partition.", ""), ("datatypes.", ""), ("selectors.", "cs.")]:
        if n.startswith(module):
            n = to + n[len(module):]
            break
    head = n.split(".")[0]
    if head in CLASSES or head in MODULES or head in ("cs", "pl"):
        return n
    return "pl." + n


def node_refs():
    """节点帮助里的 @polars：{API: [(节点, 方式)]}。"""
    refs = {}
    for path in sorted((ROOT / "nodes").glob("*.py")):
        text = path.read_text(encoding="utf-8")
        m = re.search(r"\n@polars\n(.*?)(?=\n@|\n\"\"\")", text, re.S)
        if not m:
            continue
        for line in m.group(1).splitlines():
            option, _, api = line.rpartition(": ")
            api = api.strip()
            if api.startswith("pl.") and api[3:].split(".")[0] in CLASSES:
                api = api[3:]  # pl.SQLContext → SQLContext（和 short() 一样）
            if api:
                refs.setdefault(api, []).append((path.stem, option.strip()))
    return refs


def generated():
    data = json.loads((ROOT / "nodes" / "catalog.json").read_text(encoding="utf-8"))
    return {t["name"] if t["name"].startswith("pl.") else "Expr." + t["name"] for t in data["types"]}, data["polars"]


# 精选的表达式节点（定义头里 "expr": true）：它们就是盖住同名生成节点的那个
EXPR_NODES = {p.stem for p in (ROOT / "nodes").glob("*.py") if '"expr": true' in p.read_text(encoding="utf-8").split('"""')[1]}


def describe_refs(found):
    parts = []
    for node, option in found:
        parts.append(f"`{node}` 节点" + (f"（{option}）" if option and option != node else ""))
    return "、".join(dict.fromkeys(parts))


def classify(name, refs, gen):
    """→ (归类, 说明)；说不出来的 None。"""
    if name in refs:
        also = "；也有同名的表达式节点" if name in gen and not any(n in EXPR_NODES for n, _ in refs[name]) else ""
        return HAVE, describe_refs(refs[name]) + also
    if name in gen:
        return HAVE, GENERATED
    for patterns, kind, text in RULES:
        if any(fnmatch.fnmatchcase(name, p) for p in patterns.split()):
            return kind, text
    # LazyFrame / LazyGroupBy 和 DataFrame / GroupBy 同名的：引擎里的表本来就是 LazyFrame
    for lazy, eager in (("LazyFrame.", "DataFrame."), ("LazyGroupBy.", "GroupBy.")):
        if name.startswith(lazy):
            twin = classify(eager + name[len(lazy):], refs, gen)
            if twin and twin[0] in (HAVE, ALT):
                return twin
            if twin:
                return twin[0], f"同 {eager}{name[len(lazy):]}：{twin[1]}"
    # Series 的方法 Expr 上都有同名的
    if name.startswith("Series."):
        twin = "Expr." + name[len("Series."):]
        if twin in gen or twin in refs:
            return ALT, "节点之间传的是表不是单列（Series）；对一列做的事在 expr 网络里用同名的表达式节点"
    return None


def main():
    cache = None
    if "--cache" in sys.argv:
        cache = Path(sys.argv[sys.argv.index("--cache") + 1])
        cache.mkdir(parents=True, exist_ok=True)
    apis = docs_apis(cache)
    refs = node_refs()
    gen, version = generated()

    rows = {}  # 节 → 页面 → [(API, 归类, 说明)]
    unexplained = []
    for full, (section, title, url) in apis.items():
        name = short(full)
        got = classify(name, refs, gen)
        if got is None:
            unexplained.append((name, url))
            got = ("没有说明", "")
        rows.setdefault(section, {}).setdefault((title, url), []).append((name, *got))

    # 节点帮助里写了、文档里找不到的 API（写错了或者 polars 改名了）
    names = {short(f) for f in apis}
    stale = sorted(a for a in refs if a not in names and a.replace("Expr.", "Expr.", 1) not in names)

    counts = {}
    for section in rows.values():
        for items in section.values():
            for _, kind, _ in items:
                counts[kind] = counts.get(kind, 0) + 1

    out = [
        "# polars API 对照",
        "",
        f"对照 [polars 2 的 API 文档]({BASE})，每个 API 在查看器里有没有对应的节点；没有的写明为什么。",
        f"`python engine/api_coverage.py` 生成（节点目录是 polars {version} 的），原因写在脚本的 `RULES` 里。",
        "",
        "- **有节点**：有一种节点（或节点的一个方式）就是它",
        "- **换个写法**：没有单独的节点，但用别的节点 / 参数能做到同样的事（说明里写怎么做）",
        "- **不做**：不适合做成节点（说明里写为什么）",
        "- **可以加**：没有不做的理由，只是还没做",
        "",
        "| 有节点 | 换个写法 | 不做 | 可以加 | 共 |",
        "|---:|---:|---:|---:|---:|",
        f"| {counts.get(HAVE, 0)} | {counts.get(ALT, 0)} | {counts.get(NO, 0)} | {counts.get(TODO, 0)} | {sum(counts.values())} |",
        "",
        "SQL 的语句和函数（文档的 SQL Interface → SQL Clauses / SQL Functions）全部能在 `sql` 节点里写，下面不逐个列。",
        "",
    ]
    todo = [(n, t) for s in rows.values() for items in s.values() for n, k, t in items if k == TODO]
    if todo:
        out += ["## 可以加的", ""]
        merged = {}
        for n, t in todo:
            merged.setdefault(t, []).append(n)
        for t, ns in merged.items():
            out.append(f"- {'、'.join(f'`{n}`' for n in ns)}：{t}")
        out.append("")
    if unexplained:
        out += ["## 没有说明的", "", "文档里有、这里还没有说法的（加进 `engine/api_coverage.py` 的 `RULES`）：", ""]
        out += [f"- [`{n}`]({u})" for n, u in unexplained]
        out.append("")
    if stale:
        out += ["## 节点帮助里写了、文档里没有的", ""]
        out += [f"- `{a}`（{describe_refs(refs[a])}）" for a in stale]
        out.append("")

    for key, label in SECTIONS:
        if key not in rows:
            continue
        out += [f"## {label}", ""]
        for (title, url), items in rows[key].items():
            n = len(items)
            out += [f"### [{title}]({url})（{n}）", "", "| API | 归类 | 对应 / 原因 |", "|---|---|---|"]
            # 同一个说法的并成一行
            merged = {}
            for name, kind, text in items:
                merged.setdefault((kind, text), []).append(name)
            order = {HAVE: 0, ALT: 1, TODO: 2, NO: 3}
            for (kind, text), names_ in sorted(merged.items(), key=lambda kv: order.get(kv[0][0], 9)):
                cell = " ".join(f"`{x}`" for x in names_)
                out.append(f"| {cell} | {kind} | {text.replace('|', '&#124;')} |")
            out.append("")

    OUT.parent.mkdir(exist_ok=True)
    OUT.write_text("\n".join(out), encoding="utf-8")
    print(f"{sum(counts.values())} 个 API：" + "，".join(f"{k} {v}" for k, v in counts.items()) + f" → {OUT}")
    for n, u in unexplained:
        print("  没有说明", n, u, file=sys.stderr)
    for a in stale:
        print("  文档里没有", a, file=sys.stderr)
    return 1 if unexplained else 0


sys.exit(main())
