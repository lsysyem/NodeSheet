# polars API 对照

对照 [polars 2 的 API 文档](https://docs.pola.rs/api/python/version/2/reference/)，每个 API 在查看器里有没有对应的节点；没有的写明为什么。
`python engine/api_coverage.py` 生成（节点目录是 polars 2.0.0 的），原因写在脚本的 `RULES` 里。

- **有节点**：有一种节点（或节点的一个方式）就是它
- **换个写法**：没有单独的节点，但用别的节点 / 参数能做到同样的事（说明里写怎么做）
- **不做**：不适合做成节点（说明里写为什么）
- **可以加**：没有不做的理由，只是还没做

| 有节点 | 换个写法 | 不做 | 可以加 | 共 |
|---:|---:|---:|---:|---:|
| 642 | 545 | 301 | 0 | 1488 |

SQL 的语句和函数（文档的 SQL Interface → SQL Clauses / SQL Functions）全部能在 `sql` 节点里写，下面不逐个列。

## DataFrame

### [Manipulation/selection](https://docs.pola.rs/api/python/version/2/reference/dataframe/modify_select.html)（72）

| API | 归类 | 对应 / 原因 |
|---|---|---|
| `DataFrame.bottom_k` | 有节点 | `slice` 节点（bottom_k） |
| `DataFrame.cast` | 有节点 | `cast_columns` 节点（cast） |
| `DataFrame.drop` | 有节点 | `select` 节点（drop） |
| `DataFrame.drop_nans` | 有节点 | `drop_nulls` 节点（drop_nans） |
| `DataFrame.drop_nulls` | 有节点 | `drop_nulls` 节点 |
| `DataFrame.explode` | 有节点 | `explode` 节点 |
| `DataFrame.fill_nan` | 有节点 | `fill_null` 节点（fill_nan） |
| `DataFrame.fill_null` | 有节点 | `fill_null` 节点 |
| `DataFrame.filter` | 有节点 | `condition` 节点、`expr` 节点（filter）、`filter` 节点（留下满足的） |
| `DataFrame.gather` | 有节点 | `slice` 节点（gather） |
| `DataFrame.gather_every` | 有节点 | `slice` 节点（gather_every） |
| `DataFrame.group_by` | 有节点 | `expr` 节点（group_by）、`key` 节点 |
| `DataFrame.group_by_dynamic` | 有节点 | `expr` 节点：上下文 group_by，分组后 agg，填“时间窗口” |
| `DataFrame.head` | 有节点 | `slice` 节点（head） |
| `DataFrame.interpolate` | 有节点 | `fill_null` 节点（interpolate） |
| `DataFrame.join` | 有节点 | `join` 节点 |
| `DataFrame.join_asof` | 有节点 | `join` 节点（join_asof） |
| `DataFrame.join_where` | 有节点 | `join` 节点（join_where） |
| `DataFrame.match_to_schema` | 有节点 | `cast_columns` 节点（方式 match_to_schema） |
| `DataFrame.merge_sorted` | 有节点 | `concat` 节点（merge_sorted） |
| `DataFrame.pivot` | 有节点 | `pivot` 节点 |
| `DataFrame.remove` | 有节点 | `expr` 节点（去掉满足的）、`filter` 节点（去掉满足的） |
| `DataFrame.rename` | 有节点 | `rename` 节点 |
| `DataFrame.reverse` | 有节点 | `slice` 节点（reverse） |
| `DataFrame.rolling` | 有节点 | `expr` 节点：上下文 group_by，分组后 agg，填“滚动窗口” |
| `DataFrame.sample` | 有节点 | `slice` 节点（sample） |
| `DataFrame.select` | 有节点 | `expr` 节点（select）、`select` 节点 |
| `DataFrame.shift` | 有节点 | `fill_null` 节点（shift） |
| `DataFrame.slice` | 有节点 | `slice` 节点 |
| `DataFrame.sort` | 有节点 | `expr` 节点（sort）、`sort_key` 节点 |
| `DataFrame.tail` | 有节点 | `slice` 节点（tail） |
| `DataFrame.to_dummies` | 有节点 | `to_dummies` 节点 |
| `DataFrame.top_k` | 有节点 | `slice` 节点（top_k） |
| `DataFrame.transpose` | 有节点 | `transpose` 节点 |
| `DataFrame.unique` | 有节点 | `unique` 节点 |
| `DataFrame.unnest` | 有节点 | `explode` 节点（unnest） |
| `DataFrame.unpivot` | 有节点 | `unpivot` 节点 |
| `DataFrame.unstack` | 有节点 | `transpose` 节点（unstack） |
| `DataFrame.update` | 有节点 | `join` 节点（update） |
| `DataFrame.upsample` | 有节点 | `upsample` 节点 |
| `DataFrame.with_columns` | 有节点 | `expr` 节点（with_columns） |
| `DataFrame.with_row_index` | 有节点 | `with_row_index` 节点 |
| `DataFrame.clear` | 换个写法 | `slice` 节点（head，n = 0）：只留表结构 |
| `DataFrame.drop_in_place` | 换个写法 | `select` 节点（方式 drop） |
| `DataFrame.extend` `DataFrame.vstack` | 换个写法 | `concat` 节点（how = vertical） |
| `DataFrame.hstack` | 换个写法 | `concat` 节点（how = horizontal） |
| `DataFrame.insert_column` `DataFrame.replace_column` | 换个写法 | `expr` 节点（with_columns：同名覆盖、新名追加）；列的位置用 `select` 节点排 |
| `DataFrame.limit` | 换个写法 | 就是 head：`slice` 节点（取法 head） |
| `DataFrame.select_seq` `DataFrame.with_columns_seq` | 换个写法 | 和 select / with_columns 结果一样，只是不并行算（`expr` 节点）；不值得多一个选项 |
| `DataFrame.sql` | 换个写法 | `sql` 节点（表名写 self） |
| `DataFrame.clone` `DataFrame.rechunk` `DataFrame.set_sorted` `DataFrame.shrink_to_fit` | 不做 | 不改数据（转 LazyFrame、整理内存、标记已排序、复制）；引擎里的表本来就是 LazyFrame，每步的结果也不会被改 |
| `DataFrame.get_column` `DataFrame.get_column_index` `DataFrame.get_columns` `DataFrame.item` `DataFrame.iter_columns` `DataFrame.iter_rows` `DataFrame.iter_slices` `DataFrame.row` `DataFrame.rows` `DataFrame.rows_by_key` `DataFrame.to_series` | 不做 | 把数据取成 Python 对象（列、元组、标量、迭代器），不是表；在查看器里就是看表格本身 |
| `DataFrame.partition_by` | 不做 | 结果是好几张表（每组一张），节点只有一个输出；按组算用 `expr` 的 group_by，要每组一张表就各接一个 filter |
| `DataFrame.pipe` | 不做 | 参数是 Python 函数（对整表 / 每行 / 每列调一次）；要写函数就用 `code` 节点，按值算的在 expr 网络里有 `map_elements` / `map_batches` 节点 |
| `DataFrame.__getitem__` `DataFrame.__setitem__` | 不做 | Python 的 `df[...]` 语法，取列 / 取行的功能 `select` / `slice` / `expr` 都有 |

### [Miscellaneous](https://docs.pola.rs/api/python/version/2/reference/dataframe/miscellaneous.html)（8）

| API | 归类 | 对应 / 原因 |
|---|---|---|
| `DataFrame.corr` | 有节点 | `describe` 节点（corr） |
| `DataFrame.collect_schema` | 不做 | 表的属性（列名、类型、行列数），表格面板已经显示：表头第二行是类型，路径栏是行×列 |
| `DataFrame.deserialize` `DataFrame.serialize` | 不做 | polars 自己的二进制格式；节点图存在工程文件（.rtv）里，数据用 `write` 写成 Parquet / IPC |
| `DataFrame.equals` | 不做 | 返回真假；要比较两张表，用 `join`（how = anti）找出不一样的行 |
| `DataFrame.lazy` | 不做 | 不改数据（转 LazyFrame、整理内存、标记已排序、复制）；引擎里的表本来就是 LazyFrame，每步的结果也不会被改 |
| `DataFrame.map_columns` `DataFrame.map_rows` | 不做 | 参数是 Python 函数（对整表 / 每行 / 每列调一次）；要写函数就用 `code` 节点，按值算的在 expr 网络里有 `map_elements` / `map_batches` 节点 |

### [Aggregation](https://docs.pola.rs/api/python/version/2/reference/dataframe/aggregation.html)（14）

| API | 归类 | 对应 / 原因 |
|---|---|---|
| `DataFrame.count` | 有节点 | `describe` 节点（count） |
| `DataFrame.max` | 有节点 | `describe` 节点（max） |
| `DataFrame.mean` | 有节点 | `describe` 节点（mean） |
| `DataFrame.median` | 有节点 | `describe` 节点（median） |
| `DataFrame.min` | 有节点 | `describe` 节点（min） |
| `DataFrame.product` | 有节点 | `describe` 节点（product） |
| `DataFrame.quantile` | 有节点 | `describe` 节点（quantile） |
| `DataFrame.std` | 有节点 | `describe` 节点（std） |
| `DataFrame.sum` | 有节点 | `describe` 节点（sum） |
| `DataFrame.var` | 有节点 | `describe` 节点（var） |
| `DataFrame.max_horizontal` `DataFrame.mean_horizontal` `DataFrame.min_horizontal` `DataFrame.sum_horizontal` | 换个写法 | 返回的是一列（Series）不是表；在 expr 节点（with_columns / select）的网络里用对应的表达式：`pl.sum_horizontal` / `pl.max_horizontal` / … 节点 |

### [Descriptive](https://docs.pola.rs/api/python/version/2/reference/dataframe/descriptive.html)（11）

| API | 归类 | 对应 / 原因 |
|---|---|---|
| `DataFrame.describe` | 有节点 | `describe` 节点 |
| `DataFrame.is_duplicated` | 有节点 | `unique` 节点（is_duplicated） |
| `DataFrame.null_count` | 有节点 | `describe` 节点（null_count） |
| `DataFrame.is_unique` | 换个写法 | 返回的是一列（Series）不是表；在 expr 节点（with_columns / select）的网络里用对应的表达式：`pl.struct`（`cs.all()`）→ `is_unique`；只留不重复的行用 `unique` 节点（keep = none） |
| `DataFrame.n_unique` | 换个写法 | 返回一个数：expr 节点 select 里 `pl.struct`（`cs.all()`）→ `n_unique`；每列各自的个数是 `describe` 节点的 n_unique |
| `DataFrame.estimated_size` `DataFrame.is_empty` `DataFrame.is_sorted` `DataFrame.n_chunks` | 不做 | 返回一个数或真假（占多少内存、有没有行、排没排好序），不是表；行数在路径栏 |
| `DataFrame.glimpse` `DataFrame.show` | 不做 | 打印到控制台的另一种排版，查看器自己画表格 |

### [Computation](https://docs.pola.rs/api/python/version/2/reference/dataframe/computation.html)（2）

| API | 归类 | 对应 / 原因 |
|---|---|---|
| `DataFrame.fold` | 换个写法 | 返回的是一列（Series）不是表；在 expr 节点（with_columns / select）的网络里用对应的表达式：`pl.fold` 节点 |
| `DataFrame.hash_rows` | 换个写法 | 返回的是一列（Series）不是表；在 expr 节点（with_columns / select）的网络里用对应的表达式：`pl.struct`（`cs.all()`）→ `hash` |

### [Export](https://docs.pola.rs/api/python/version/2/reference/dataframe/export.html)（11）

| API | 归类 | 对应 / 原因 |
|---|---|---|
| `DataFrame.to_struct` | 换个写法 | expr 网络里 `pl.struct`（col 填 `cs.all()`） |
| `DataFrame.to_arrow` `DataFrame.to_dict` `DataFrame.to_dicts` `DataFrame.to_jax` `DataFrame.to_numpy` `DataFrame.to_pandas` `DataFrame.to_torch` `DataFrame.__array__` `DataFrame.__arrow_c_stream__` | 不做 | 转成别的 Python 库的对象（pandas / numpy / torch…），节点之间传的只有 polars 表；要给别的程序用就 `write` 写成文件 |
| `DataFrame.to_init_repr` | 不做 | 生成造这张表的 Python 代码；整个节点图导出成代码见表格面板的“Python 脚本”页签 |

### [Attributes](https://docs.pola.rs/api/python/version/2/reference/dataframe/attributes.html)（7）

| API | 归类 | 对应 / 原因 |
|---|---|---|
| `DataFrame.columns` `DataFrame.dtypes` `DataFrame.flags` `DataFrame.height` `DataFrame.schema` `DataFrame.shape` `DataFrame.width` | 不做 | 表的属性（列名、类型、行列数），表格面板已经显示：表头第二行是类型，路径栏是行×列 |

### [GroupBy](https://docs.pola.rs/api/python/version/2/reference/dataframe/group_by.html)（17）

| API | 归类 | 对应 / 原因 |
|---|---|---|
| `GroupBy.agg` | 有节点 | `expr` 节点（agg） |
| `GroupBy.all` `GroupBy.first` `GroupBy.head` `GroupBy.last` `GroupBy.len` `GroupBy.max` `GroupBy.mean` `GroupBy.median` `GroupBy.min` `GroupBy.n_unique` `GroupBy.quantile` `GroupBy.sum` `GroupBy.tail` | 有节点 | `expr` 节点：上下文 group_by，“分组后”选同名的一项 |
| `GroupBy.having` | 有节点 | `expr` 节点 group_by：网络里接“条件”输出 |
| `GroupBy.__iter__` | 不做 | 在 Python 里逐组循环，结果不是表 |
| `GroupBy.map_groups` | 不做 | 参数是 Python 函数，每组调一次；用 `code` 节点（逐组的计算在 agg 网络里写表达式更快） |

### [Plot](https://docs.pola.rs/api/python/version/2/reference/dataframe/plot.html)（1）

| API | 归类 | 对应 / 原因 |
|---|---|---|
| `DataFrame.plot` | 不做 | 画图（Altair）：结果是图表不是表，要多带 altair 包，查看器只画表格；画图是另一块功能，要做的话应该是查看器自己的图表面板 |

### [Style](https://docs.pola.rs/api/python/version/2/reference/dataframe/style.html)（1）

| API | 归类 | 对应 / 原因 |
|---|---|---|
| `DataFrame.style` | 不做 | 把表排成 HTML 的样式表格（great_tables 包），用来放进网页 / 笔记本；查看器自己画表格 |

## LazyFrame

### [Miscellaneous](https://docs.pola.rs/api/python/version/2/reference/lazyframe/miscellaneous.html)（16）

| API | 归类 | 对应 / 原因 |
|---|---|---|
| `LazyFrame.collect` `LazyFrame.collect_async` `LazyFrame.collect_batches` `LazyFrame.execute` `LazyFrame.sink_batches` | 不做 | 执行查询的方式；引擎只 collect 显示着的节点（流式引擎），结果写成 parquet 给表格看 |
| `LazyFrame.collect_schema` | 不做 | 表的属性，表格面板已经显示（表头第二行是类型） |
| `LazyFrame.cache` | 不做 | 不改数据；引擎按（代码 + 参数 + 上游）缓存每步的结果 |
| `LazyFrame.deserialize` `LazyFrame.serialize` | 不做 | 把查询计划存成 polars 的二进制格式；节点图存在工程文件（.rtv）里 |
| `LazyFrame.from_lazyframe_resolver` | 不做 | 给写 IO 插件的人用的内部接口 |
| `LazyFrame.lazy` | 不做 | 不改数据 |
| `LazyFrame.map_batches` `LazyFrame.pipe` `LazyFrame.pipe_with_schema` | 不做 | 参数是 Python 函数；用 `code` 节点 |
| `LazyFrame.remote` | 不做 | 交给 Polars Cloud 执行，要云账号 |
| `pl.QueryOptFlags` | 不做 | 关掉某些查询优化，排查 polars 的问题用 |

### [Descriptive](https://docs.pola.rs/api/python/version/2/reference/lazyframe/descriptive.html)（4）

| API | 归类 | 对应 / 原因 |
|---|---|---|
| `LazyFrame.explain` | 有节点 | 表格面板的“查询计划”页签：显示节点优化后、优化前的计划 |
| `LazyFrame.describe` | 有节点 | `describe` 节点 |
| `LazyFrame.show_graph` | 换个写法 | “查询计划”页签点“画成树”（explain(format="tree")，同样的树画成文字）；show_graph 出图片要 graphviz，不带 |
| `LazyFrame.show` | 不做 | 同 DataFrame.show：打印到控制台的另一种排版，查看器自己画表格 |

### [Manipulation/selection](https://docs.pola.rs/api/python/version/2/reference/lazyframe/modify_select.html)（48）

| API | 归类 | 对应 / 原因 |
|---|---|---|
| `LazyFrame.pivot` | 有节点 | `pivot` 节点 |
| `LazyFrame.bottom_k` | 有节点 | `slice` 节点（bottom_k） |
| `LazyFrame.cast` | 有节点 | `cast_columns` 节点（cast） |
| `LazyFrame.drop` | 有节点 | `select` 节点（drop） |
| `LazyFrame.drop_nans` | 有节点 | `drop_nulls` 节点（drop_nans） |
| `LazyFrame.drop_nulls` | 有节点 | `drop_nulls` 节点 |
| `LazyFrame.explode` | 有节点 | `explode` 节点 |
| `LazyFrame.fill_nan` | 有节点 | `fill_null` 节点（fill_nan） |
| `LazyFrame.fill_null` | 有节点 | `fill_null` 节点 |
| `LazyFrame.filter` | 有节点 | `condition` 节点、`expr` 节点（filter） |
| `LazyFrame.gather` | 有节点 | `slice` 节点（gather） |
| `LazyFrame.gather_every` | 有节点 | `slice` 节点（gather_every） |
| `LazyFrame.group_by` | 有节点 | `expr` 节点（group_by）、`key` 节点 |
| `LazyFrame.group_by_dynamic` | 有节点 | `expr` 节点：上下文 group_by，分组后 agg，填“时间窗口” |
| `LazyFrame.head` | 有节点 | `slice` 节点（head） |
| `LazyFrame.interpolate` | 有节点 | `fill_null` 节点（interpolate） |
| `LazyFrame.join` | 有节点 | `join` 节点 |
| `LazyFrame.join_asof` | 有节点 | `join` 节点（join_asof） |
| `LazyFrame.join_where` | 有节点 | `join` 节点（join_where） |
| `LazyFrame.match_to_schema` | 有节点 | `cast_columns` 节点（match_to_schema） |
| `LazyFrame.merge_sorted` | 有节点 | `concat` 节点（merge_sorted） |
| `LazyFrame.remove` | 有节点 | `expr` 节点（去掉满足的） |
| `LazyFrame.rename` | 有节点 | `rename` 节点 |
| `LazyFrame.reverse` | 有节点 | `slice` 节点（reverse） |
| `LazyFrame.rolling` | 有节点 | `expr` 节点：上下文 group_by，分组后 agg，填“滚动窗口” |
| `LazyFrame.select` | 有节点 | `expr` 节点（select）、`select` 节点 |
| `LazyFrame.shift` | 有节点 | `fill_null` 节点（shift） |
| `LazyFrame.slice` | 有节点 | `slice` 节点 |
| `LazyFrame.sort` | 有节点 | `expr` 节点（sort）、`sort_key` 节点 |
| `LazyFrame.tail` | 有节点 | `slice` 节点（tail） |
| `LazyFrame.top_k` | 有节点 | `slice` 节点（top_k） |
| `LazyFrame.unique` | 有节点 | `unique` 节点 |
| `LazyFrame.unnest` | 有节点 | `explode` 节点（unnest） |
| `LazyFrame.unpivot` | 有节点 | `unpivot` 节点 |
| `LazyFrame.update` | 有节点 | `join` 节点（update） |
| `LazyFrame.with_columns` | 有节点 | `expr` 节点（with_columns） |
| `LazyFrame.with_row_index` | 有节点 | `with_row_index` 节点 |
| `LazyFrame.clear` | 换个写法 | `slice` 节点（head，n = 0）：只留表结构 |
| `LazyFrame.first` | 换个写法 | `slice` 节点（head，n = 1） |
| `LazyFrame.last` | 换个写法 | `slice` 节点（tail，n = 1） |
| `LazyFrame.limit` | 换个写法 | 就是 head：`slice` 节点（取法 head） |
| `LazyFrame.select_seq` `LazyFrame.with_columns_seq` | 换个写法 | 和 select / with_columns 结果一样，只是不并行算（`expr` 节点）；不值得多一个选项 |
| `LazyFrame.sql` | 换个写法 | `sql` 节点（表名写 self） |
| `LazyFrame.clone` | 不做 | 不改数据 |
| `LazyFrame.inspect` | 不做 | 执行时把中间结果 print 到控制台；查看器里点那个节点就能看它的结果 |
| `LazyFrame.set_sorted` | 不做 | 同 DataFrame.set_sorted：不改数据（转 LazyFrame、整理内存、标记已排序、复制）；引擎里的表本来就是 LazyFrame，每步的结果也不会被改 |
| `LazyFrame.__getitem__` | 不做 | Python 的 `lf[...]` 切片语法；`slice` 节点 |

### [Aggregation](https://docs.pola.rs/api/python/version/2/reference/lazyframe/aggregation.html)（10）

| API | 归类 | 对应 / 原因 |
|---|---|---|
| `LazyFrame.count` | 有节点 | `describe` 节点（count） |
| `LazyFrame.max` | 有节点 | `describe` 节点（max） |
| `LazyFrame.mean` | 有节点 | `describe` 节点（mean） |
| `LazyFrame.median` | 有节点 | `describe` 节点（median） |
| `LazyFrame.min` | 有节点 | `describe` 节点（min） |
| `LazyFrame.null_count` | 有节点 | `describe` 节点（null_count） |
| `LazyFrame.quantile` | 有节点 | `describe` 节点（quantile） |
| `LazyFrame.std` | 有节点 | `describe` 节点（std） |
| `LazyFrame.sum` | 有节点 | `describe` 节点（sum） |
| `LazyFrame.var` | 有节点 | `describe` 节点（var） |

### [Attributes](https://docs.pola.rs/api/python/version/2/reference/lazyframe/attributes.html)（4）

| API | 归类 | 对应 / 原因 |
|---|---|---|
| `LazyFrame.columns` `LazyFrame.dtypes` `LazyFrame.schema` `LazyFrame.width` | 不做 | 表的属性，表格面板已经显示（表头第二行是类型） |

### [Engines](https://docs.pola.rs/api/python/version/2/reference/lazyframe/engines.html)（5）

| API | 归类 | 对应 / 原因 |
|---|---|---|
| `pl.Engine` `pl.InMemoryEngine` `pl.StreamingEngine` `pl.RemoteEngine` | 不做 | 选执行引擎（collect(engine=…)）；引擎用 polars 2 默认的流式执行，内存里放不下的才有区别 |
| `pl.GPUEngine` | 不做 | 用 NVIDIA GPU 执行，要装 cudf-polars（几百 MB）和 CUDA；表格查看器的数据量用 CPU 就够 |

### [LazyFrameResolver](https://docs.pola.rs/api/python/version/2/reference/lazyframe/resolver.html)（8）

| API | 归类 | 对应 / 原因 |
|---|---|---|
| `LazyFrameResolver` `LazyFrameResolver.lazy` `LazyFrameResolver.schema` `LazyFrameResolver.resolve_lazyframe` `LazyFrameResolver.cse_eq` `FilterExpr` `FilterExpr.pyarrow_expr` `pl.ResolvedLazyFrameProps` | 不做 | 给写 IO 插件的人用的内部接口 |

### [GroupBy](https://docs.pola.rs/api/python/version/2/reference/lazyframe/group_by.html)（16）

| API | 归类 | 对应 / 原因 |
|---|---|---|
| `LazyGroupBy.agg` | 有节点 | `expr` 节点（agg） |
| `LazyGroupBy.all` `LazyGroupBy.first` `LazyGroupBy.head` `LazyGroupBy.last` `LazyGroupBy.len` `LazyGroupBy.max` `LazyGroupBy.mean` `LazyGroupBy.median` `LazyGroupBy.min` `LazyGroupBy.n_unique` `LazyGroupBy.quantile` `LazyGroupBy.sum` `LazyGroupBy.tail` | 有节点 | `expr` 节点：上下文 group_by，“分组后”选同名的一项 |
| `LazyGroupBy.having` | 有节点 | `expr` 节点 group_by：网络里接“条件”输出 |
| `LazyGroupBy.map_groups` | 不做 | 同 GroupBy.map_groups：参数是 Python 函数，每组调一次；用 `code` 节点（逐组的计算在 agg 网络里写表达式更快） |

### [InProcessQuery](https://docs.pola.rs/api/python/version/2/reference/lazyframe/in_process.html)（3）

| API | 归类 | 对应 / 原因 |
|---|---|---|
| `InProcessQuery.cancel` `InProcessQuery.fetch` `InProcessQuery.fetch_blocking` | 不做 | 后台执行（collect(background=True)）的句柄；引擎自己有排队和取消（过期的请求不发） |

### [QueryResult](https://docs.pola.rs/api/python/version/2/reference/lazyframe/query_result.html)（3）

| API | 归类 | 对应 / 原因 |
|---|---|---|
| `QueryResult.head` `QueryResult.n_rows_total` `QueryResult.lazy` | 不做 | LazyFrame.execute 的结果（还不稳定的新接口）；引擎直接 collect |

## Series

### [Export](https://docs.pola.rs/api/python/version/2/reference/series/export.html)（10）

| API | 归类 | 对应 / 原因 |
|---|---|---|
| `Series.to_init_repr` `Series.to_arrow` `Series.to_frame` `Series.to_jax` `Series.to_list` `Series.to_numpy` `Series.to_pandas` `Series.to_torch` `Series.__array__` `Series.__arrow_c_stream__` | 不做 | 转成 Python 对象 / 别的库的对象；节点之间传的是 polars 表 |

### [Computation](https://docs.pola.rs/api/python/version/2/reference/series/computation.html)（89）

| API | 归类 | 对应 / 原因 |
|---|---|---|
| `Series.abs` `Series.approx_n_unique` `Series.arccos` `Series.arccosh` `Series.arcsin` `Series.arcsinh` `Series.arctan` `Series.arctanh` `Series.arg_true` `Series.arg_unique` `Series.bitwise_and` `Series.bitwise_count_ones` `Series.bitwise_count_zeros` `Series.bitwise_leading_ones` `Series.bitwise_leading_zeros` `Series.bitwise_or` `Series.bitwise_trailing_ones` `Series.bitwise_trailing_zeros` `Series.bitwise_xor` `Series.cbrt` `Series.cos` `Series.cosh` `Series.cot` `Series.cum_count` `Series.cum_max` `Series.cum_min` `Series.cum_prod` `Series.cum_sum` `Series.cumulative_eval` `Series.degrees` `Series.diff` `Series.dot` `Series.entropy` `Series.erf` `Series.erfc` `Series.ewm_mean` `Series.ewm_mean_by` `Series.ewm_std` `Series.ewm_sum` `Series.ewm_sum_by` `Series.ewm_var` `Series.exp` `Series.first` `Series.hash` `Series.hist` `Series.index_of` `Series.is_between` `Series.is_close` `Series.kurtosis` `Series.last` `Series.log` `Series.log10` `Series.log1p` `Series.pct_change` `Series.peak_max` `Series.peak_min` `Series.radians` `Series.rank` `Series.replace` `Series.replace_strict` `Series.rolling_kurtosis` `Series.rolling_map` `Series.rolling_max` `Series.rolling_max_by` `Series.rolling_mean` `Series.rolling_mean_by` `Series.rolling_median` `Series.rolling_median_by` `Series.rolling_min` `Series.rolling_min_by` `Series.rolling_quantile` `Series.rolling_quantile_by` `Series.rolling_rank` `Series.rolling_rank_by` `Series.rolling_skew` `Series.rolling_std` `Series.rolling_std_by` `Series.rolling_sum` `Series.rolling_sum_by` `Series.rolling_var` `Series.rolling_var_by` `Series.search_sorted` `Series.sign` `Series.sin` `Series.sinh` `Series.skew` `Series.sqrt` `Series.tan` `Series.tanh` | 换个写法 | 节点之间传的是表不是单列（Series）；对一列做的事在 expr 网络里用同名的表达式节点 |

### [Manipulation/selection](https://docs.pola.rs/api/python/version/2/reference/series/modify_select.html)（62）

| API | 归类 | 对应 / 原因 |
|---|---|---|
| `Series.alias` `Series.append` `Series.arg_sort` `Series.backward_fill` `Series.bin_intervals` `Series.bin_quantiles` `Series.bin_ranks` `Series.bottom_k` `Series.bottom_k_by` `Series.cast` `Series.ceil` `Series.clip` `Series.drop_nans` `Series.drop_nulls` `Series.explode` `Series.extend_constant` `Series.fill_nan` `Series.fill_null` `Series.filter` `Series.floor` `Series.forward_fill` `Series.gather` `Series.gather_every` `Series.head` `Series.interpolate` `Series.interpolate_by` `Series.item` `Series.limit` `Series.repeat_by` `Series.reshape` `Series.reverse` `Series.rle` `Series.rle_id` `Series.round` `Series.round_sig_figs` `Series.sample` `Series.shift` `Series.shuffle` `Series.slice` `Series.sort` `Series.tail` `Series.top_k` `Series.top_k_by` `Series.truncate` `Series.unique` | 换个写法 | 节点之间传的是表不是单列（Series）；对一列做的事在 expr 网络里用同名的表达式节点 |
| `Series.extend` | 换个写法 | `concat` 节点 |
| `Series.new_from_index` | 换个写法 | `get` 节点取第 i 个值：在 with_columns 里一个值会铺满每一行 |
| `Series.rename` | 换个写法 | 输出节点的列名（alias），或 `rename` 节点 |
| `Series.scatter` `Series.set` | 换个写法 | 按位置 / 按条件改值：`when` 节点（按位置的条件写 `pl.int_range(pl.len()) == 3`） |
| `Series.sql` | 换个写法 | `sql` 节点 |
| `Series.to_dummies` | 换个写法 | `to_dummies` 节点 |
| `Series.zip_with` | 换个写法 | 就是条件取值：`when` 节点 |
| `Series.clear` `Series.clone` `Series.rechunk` `Series.shrink_to_fit` | 不做 | 内存布局 / 复制 / 比较，不改数据或结果不是表 |
| `Series.cut` `Series.qcut` | 不做 | polars 2 里废弃了（Expr 的也是），换成 `bin_intervals` / `bin_quantiles`，这两个有表达式节点 |
| `Series.shrink_dtype` | 不做 | 只在 Series 上有：把数值类型缩到装得下的最小类型（省内存）；要指定类型用 `cast` / `cast_columns` |
| `Series.__getitem__` `Series.__setitem__` | 不做 | Python 的 `s[...]` 语法；取值用 `get` / `gather` / `slice` 节点 |

### [Boolean](https://docs.pola.rs/api/python/version/2/reference/series/boolean.html)（3）

| API | 归类 | 对应 / 原因 |
|---|---|---|
| `Series.all` `Series.any` `Series.not_` | 换个写法 | 节点之间传的是表不是单列（Series）；对一列做的事在 expr 网络里用同名的表达式节点 |

### [Aggregation](https://docs.pola.rs/api/python/version/2/reference/series/aggregation.html)（18）

| API | 归类 | 对应 / 原因 |
|---|---|---|
| `Series.arg_max` `Series.arg_min` `Series.count` `Series.implode` `Series.max` `Series.max_by` `Series.mean` `Series.median` `Series.min` `Series.min_by` `Series.mode` `Series.nan_max` `Series.nan_min` `Series.product` `Series.quantile` `Series.std` `Series.sum` `Series.var` | 换个写法 | 节点之间传的是表不是单列（Series）；对一列做的事在 expr 网络里用同名的表达式节点 |

### [Descriptive](https://docs.pola.rs/api/python/version/2/reference/series/descriptive.html)（25）

| API | 归类 | 对应 / 原因 |
|---|---|---|
| `Series.describe` | 换个写法 | `describe` 节点 |
| `Series.has_nulls` `Series.is_duplicated` `Series.is_empty` `Series.is_finite` `Series.is_first_distinct` `Series.is_in` `Series.is_infinite` `Series.is_last_distinct` `Series.is_nan` `Series.is_not_nan` `Series.is_not_null` `Series.is_null` `Series.is_sorted` `Series.is_unique` `Series.len` `Series.lower_bound` `Series.n_unique` `Series.null_count` `Series.unique_counts` `Series.upper_bound` `Series.value_counts` | 换个写法 | 节点之间传的是表不是单列（Series）；对一列做的事在 expr 网络里用同名的表达式节点 |
| `Series.chunk_lengths` `Series.estimated_size` `Series.n_chunks` | 不做 | 内存布局 / 复制 / 比较，不改数据或结果不是表 |

### [Operators](https://docs.pola.rs/api/python/version/2/reference/series/operators.html)（9）

| API | 归类 | 对应 / 原因 |
|---|---|---|
| `Series.eq` `Series.eq_missing` `Series.ge` `Series.gt` `Series.le` `Series.lt` `Series.ne` `Series.ne_missing` `Series.pow` | 换个写法 | 节点之间传的是表不是单列（Series）；对一列做的事在 expr 网络里用同名的表达式节点 |

### [Miscellaneous](https://docs.pola.rs/api/python/version/2/reference/series/miscellaneous.html)（6）

| API | 归类 | 对应 / 原因 |
|---|---|---|
| `Series.map_elements` `Series.reinterpret` `Series.set_sorted` `Series.to_physical` | 换个写法 | 节点之间传的是表不是单列（Series）；对一列做的事在 expr 网络里用同名的表达式节点 |
| `Series.equals` `Series.get_chunks` | 不做 | 内存布局 / 复制 / 比较，不改数据或结果不是表 |

### [Attributes](https://docs.pola.rs/api/python/version/2/reference/series/attributes.html)（4）

| API | 归类 | 对应 / 原因 |
|---|---|---|
| `Series.dtype` `Series.flags` `Series.name` `Series.shape` | 不做 | 一列的属性，表头已经显示（列名、类型） |

### [List](https://docs.pola.rs/api/python/version/2/reference/series/list.html)（44）

| API | 归类 | 对应 / 原因 |
|---|---|---|
| `Series.list.explode` `Series.list.all` `Series.list.any` `Series.list.arg_max` `Series.list.arg_min` `Series.list.concat` `Series.list.contains` `Series.list.count_matches` `Series.list.diff` `Series.list.drop_nulls` `Series.list.first` `Series.list.gather` `Series.list.gather_every` `Series.list.get` `Series.list.head` `Series.list.item` `Series.list.join` `Series.list.last` `Series.list.len` `Series.list.max` `Series.list.mean` `Series.list.median` `Series.list.min` `Series.list.n_unique` `Series.list.reverse` `Series.list.sample` `Series.list.set_difference` `Series.list.set_intersection` `Series.list.set_symmetric_difference` `Series.list.set_union` `Series.list.shift` `Series.list.slice` `Series.list.sort` `Series.list.std` `Series.list.sum` `Series.list.tail` `Series.list.to_array` `Series.list.to_map` `Series.list.to_struct` `Series.list.unique` `Series.list.var` | 换个写法 | 节点之间传的是表不是单列（Series）；对一列做的事在 expr 网络里用同名的表达式节点 |
| `Series.list.agg` `Series.list.eval` `Series.list.filter` | 换个写法 | `eval` 节点（双击进去连 element → 结果） |

### [String](https://docs.pola.rs/api/python/version/2/reference/series/string.html)（47）

| API | 归类 | 对应 / 原因 |
|---|---|---|
| `Series.str.replace` `Series.str.contains` `Series.str.contains_any` `Series.str.count_matches` `Series.str.decode` `Series.str.encode` `Series.str.ends_with` `Series.str.escape_regex` `Series.str.extract` `Series.str.extract_all` `Series.str.extract_groups` `Series.str.extract_many` `Series.str.find` `Series.str.find_many` `Series.str.head` `Series.str.join` `Series.str.json_decode` `Series.str.json_path_match` `Series.str.len_bytes` `Series.str.len_chars` `Series.str.normalize` `Series.str.pad_end` `Series.str.pad_start` `Series.str.replace_all` `Series.str.replace_many` `Series.str.reverse` `Series.str.slice` `Series.str.split` `Series.str.split_exact` `Series.str.splitn` `Series.str.starts_with` `Series.str.strip_chars` `Series.str.strip_chars_start` `Series.str.strip_chars_end` `Series.str.strip_prefix` `Series.str.strip_suffix` `Series.str.strptime` `Series.str.tail` `Series.str.to_date` `Series.str.to_datetime` `Series.str.to_decimal` `Series.str.to_integer` `Series.str.to_lowercase` `Series.str.to_time` `Series.str.to_titlecase` `Series.str.to_uppercase` `Series.str.zfill` | 换个写法 | 节点之间传的是表不是单列（Series）；对一列做的事在 expr 网络里用同名的表达式节点 |

### [Temporal](https://docs.pola.rs/api/python/version/2/reference/series/temporal.html)（47）

| API | 归类 | 对应 / 原因 |
|---|---|---|
| `Series.dt.truncate` `Series.dt.add_business_days` `Series.dt.base_utc_offset` `Series.dt.cast_time_unit` `Series.dt.century` `Series.dt.combine` `Series.dt.convert_time_zone` `Series.dt.date` `Series.dt.day` `Series.dt.days_in_month` `Series.dt.dst_offset` `Series.dt.epoch` `Series.dt.hour` `Series.dt.is_business_day` `Series.dt.is_leap_year` `Series.dt.iso_year` `Series.dt.microsecond` `Series.dt.millennium` `Series.dt.millisecond` `Series.dt.minute` `Series.dt.month` `Series.dt.month_end` `Series.dt.month_start` `Series.dt.nanosecond` `Series.dt.offset_by` `Series.dt.ordinal_day` `Series.dt.quarter` `Series.dt.replace` `Series.dt.replace_time_zone` `Series.dt.round` `Series.dt.second` `Series.dt.strftime` `Series.dt.time` `Series.dt.timestamp` `Series.dt.to_string` `Series.dt.total_days` `Series.dt.total_hours` `Series.dt.total_microseconds` `Series.dt.total_milliseconds` `Series.dt.total_minutes` `Series.dt.total_nanoseconds` `Series.dt.total_seconds` `Series.dt.week` `Series.dt.weekday` `Series.dt.year` | 换个写法 | 节点之间传的是表不是单列（Series）；对一列做的事在 expr 网络里用同名的表达式节点 |
| `Series.dt.max` `Series.dt.min` | 换个写法 | 时间列直接用 `max` / `min` 节点 |

### [Array](https://docs.pola.rs/api/python/version/2/reference/series/array.html)（29）

| API | 归类 | 对应 / 原因 |
|---|---|---|
| `Series.arr.agg` `Series.arr.eval` | 换个写法 | `eval` 节点（双击进去连 element → 结果） |
| `Series.arr.all` `Series.arr.any` `Series.arr.arg_max` `Series.arr.arg_min` `Series.arr.contains` `Series.arr.count_matches` `Series.arr.dot` `Series.arr.explode` `Series.arr.first` `Series.arr.get` `Series.arr.join` `Series.arr.last` `Series.arr.len` `Series.arr.max` `Series.arr.mean` `Series.arr.median` `Series.arr.min` `Series.arr.n_unique` `Series.arr.reverse` `Series.arr.shift` `Series.arr.sort` `Series.arr.std` `Series.arr.sum` `Series.arr.to_list` `Series.arr.to_struct` `Series.arr.unique` `Series.arr.var` | 换个写法 | 节点之间传的是表不是单列（Series）；对一列做的事在 expr 网络里用同名的表达式节点 |

### [Binary](https://docs.pola.rs/api/python/version/2/reference/series/binary.html)（11）

| API | 归类 | 对应 / 原因 |
|---|---|---|
| `Series.bin.contains` `Series.bin.decode` `Series.bin.encode` `Series.bin.ends_with` `Series.bin.get` `Series.bin.head` `Series.bin.reinterpret` `Series.bin.size` `Series.bin.slice` `Series.bin.starts_with` `Series.bin.tail` | 换个写法 | 节点之间传的是表不是单列（Series）；对一列做的事在 expr 网络里用同名的表达式节点 |

### [Categories](https://docs.pola.rs/api/python/version/2/reference/series/categories.html)（6）

| API | 归类 | 对应 / 原因 |
|---|---|---|
| `Series.cat.ends_with` `Series.cat.len_bytes` `Series.cat.len_chars` `Series.cat.starts_with` `Series.cat.physical` `Series.cat.to` | 换个写法 | 节点之间传的是表不是单列（Series）；对一列做的事在 expr 网络里用同名的表达式节点 |

### [Extension Types](https://docs.pola.rs/api/python/version/2/reference/series/extension.html)（2）

| API | 归类 | 对应 / 原因 |
|---|---|---|
| `Series.ext.storage` `Series.ext.to` | 不做 | 扩展类型的列要先在 Python 里 `register_extension_type` 注册一个类型才有，节点图里没地方注册；遇到了在 `code` 节点里处理 |

### [Map](https://docs.pola.rs/api/python/version/2/reference/series/map.html)（6）

| API | 归类 | 对应 / 原因 |
|---|---|---|
| `Series.map.contains_key` `Series.map.entries` `Series.map.get` `Series.map.keys` `Series.map.len` `Series.map.values` | 换个写法 | 节点之间传的是表不是单列（Series）；对一列做的事在 expr 网络里用同名的表达式节点 |

### [Plot](https://docs.pola.rs/api/python/version/2/reference/series/plot.html)（1）

| API | 归类 | 对应 / 原因 |
|---|---|---|
| `Series.plot` | 不做 | 画图（Altair）：结果是图表不是表，要多带 altair 包，查看器只画表格；画图是另一块功能，要做的话应该是查看器自己的图表面板 |

### [Struct](https://docs.pola.rs/api/python/version/2/reference/series/struct.html)（8）

| API | 归类 | 对应 / 原因 |
|---|---|---|
| `Series.struct.eval` `Series.struct.field` `Series.struct.json_encode` `Series.struct.rename_fields` `Series.struct.drop` `Series.struct.unnest` | 换个写法 | 节点之间传的是表不是单列（Series）；对一列做的事在 expr 网络里用同名的表达式节点 |
| `Series.struct.fields` `Series.struct.schema` | 换个写法 | 结构列的字段和类型在表头第二行（`struct{x: i64, …}`）；改名用 `struct.rename_fields` |

## Expressions

### [Functions](https://docs.pola.rs/api/python/version/2/reference/expressions/functions.html)（83）

| API | 归类 | 对应 / 原因 |
|---|---|---|
| `pl.element` | 有节点 | `element` 节点 |
| `pl.select` | 有节点 | `create` 节点（select） |
| `pl.int_range` `pl.len` `Expr.head` `Expr.map_batches` `Expr.map_elements` `Expr.tail` `pl.arg_where` `pl.all_horizontal` `pl.arctan2` `pl.arg_sort_by` `pl.business_day_count` `pl.coalesce` `pl.concat_arr` `pl.concat_list` `pl.concat_str` `pl.corr` `pl.cov` `pl.cum_fold` `pl.cum_reduce` `pl.cum_sum_horizontal` `pl.date` `pl.date_range` `pl.date_ranges` `pl.datetime` `pl.datetime_range` `pl.datetime_ranges` `pl.duration` `pl.exclude` `pl.field` `pl.fold` `pl.format` `pl.from_epoch` `pl.int_ranges` `pl.linear_space` `pl.linear_spaces` `pl.list` `pl.map_batches` `pl.map_groups` `pl.max_horizontal` `pl.mean_horizontal` `pl.min_horizontal` `pl.nth` `pl.ones` `pl.reduce` `pl.repeat` `pl.rolling_corr` `pl.rolling_cov` `pl.row_index` `pl.sql_expr` `pl.struct` `pl.sum_horizontal` `pl.time` `pl.time_range` `pl.time_ranges` `pl.zeros` | 有节点 | 同名的表达式节点（expr 网络里按 Tab 搜名字；`engine/gen_catalog.py` 从 polars 签名生成） |
| `pl.when` | 有节点 | `when` 节点 |
| `pl.any_horizontal` | 有节点 | `expr` 节点（任一满足）；也有同名的表达式节点 |
| `pl.lit` | 有节点 | `lit` 节点 |
| `pl.head` `pl.tail` `pl.all` `pl.quantile` `pl.count` `pl.cum_sum` `pl.any` `pl.approx_n_unique` `pl.cum_count` `pl.first` `pl.implode` `pl.last` `pl.max` `pl.mean` `pl.median` `pl.min` `pl.n_unique` `pl.std` `pl.sum` `pl.var` | 换个写法 | 就是 `pl.col(…).同名方法`：col 节点接同名的表达式节点（`pl.all()` = col 填 `cs.all()`） |
| `pl.sql` | 换个写法 | `sql` 节点；pl.sql 是按 Python 全局变量名找表，节点图里的表只有输入 |
| `pl.arange` | 换个写法 | `pl.int_range` 的别名，用 `pl.int_range` 节点 |
| `pl.pipe_with_dtype` | 不做 | 参数是 Python 函数，一次拿到好几个表达式和类型；单个表达式的 `pipe_with_dtype` 有表达式节点，多个的用 `snippet` 写 |

### [Name](https://docs.pola.rs/api/python/version/2/reference/expressions/name.html)（10）

| API | 归类 | 对应 / 原因 |
|---|---|---|
| `Expr.name.replace` `Expr.name.map` `Expr.name.map_fields` `Expr.name.prefix_fields` `Expr.name.suffix_fields` `Expr.name.to_lowercase` `Expr.name.to_uppercase` | 有节点 | 同名的表达式节点（expr 网络里按 Tab 搜名字；`engine/gen_catalog.py` 从 polars 签名生成） |
| `Expr.name.prefix` | 有节点 | `output` 节点（prefix） |
| `Expr.name.suffix` | 有节点 | `output` 节点（suffix） |
| `Expr.name.keep` | 有节点 | `output` 节点（keep） |

### [Manipulation/selection](https://docs.pola.rs/api/python/version/2/reference/expressions/modify_select.html)（55）

| API | 归类 | 对应 / 原因 |
|---|---|---|
| `Expr.drop_nans` `Expr.append` `Expr.arg_sort` `Expr.arg_true` `Expr.backward_fill` `Expr.bin_intervals` `Expr.bin_quantiles` `Expr.bin_ranks` `Expr.bottom_k` `Expr.bottom_k_by` `Expr.ceil` `Expr.clip` `Expr.drop_nulls` `Expr.explode` `Expr.extend_constant` `Expr.fill_nan` `Expr.fill_null` `Expr.filter` `Expr.floor` `Expr.forward_fill` `Expr.gather` `Expr.gather_every` `Expr.get` `Expr.interpolate` `Expr.interpolate_by` `Expr.item` `Expr.limit` `Expr.lower_bound` `Expr.pipe_with_dtype` `Expr.reinterpret` `Expr.repeat_by` `Expr.replace` `Expr.replace_strict` `Expr.reshape` `Expr.reverse` `Expr.rle` `Expr.rle_id` `Expr.round` `Expr.round_sig_figs` `Expr.sample` `Expr.shift` `Expr.shuffle` `Expr.slice` `Expr.sort` `Expr.sort_by` `Expr.to_physical` `Expr.top_k` `Expr.top_k_by` `Expr.truncate` `Expr.upper_bound` | 有节点 | 同名的表达式节点（expr 网络里按 Tab 搜名字；`engine/gen_catalog.py` 从 polars 签名生成） |
| `Expr.cast` | 有节点 | `cast` 节点 |
| `Expr.cut` `Expr.qcut` | 不做 | polars 2 里废弃了，换成 `bin_intervals` / `bin_quantiles`（有表达式节点） |
| `Expr.inspect` | 不做 | 执行时把值 print 到控制台；网络里选中哪个节点表格就预览哪个节点的值 |
| `Expr.pipe` | 不做 | 参数是 Python 函数（拿表达式返回表达式）；直接在网络里接节点，或用 `snippet` 写 |

### [Computation](https://docs.pola.rs/api/python/version/2/reference/expressions/computation.html)（82）

| API | 归类 | 对应 / 原因 |
|---|---|---|
| `Expr.abs` `Expr.arccos` `Expr.arccosh` `Expr.arcsin` `Expr.arcsinh` `Expr.arctan` `Expr.arctanh` `Expr.arg_unique` `Expr.bitwise_count_ones` `Expr.bitwise_count_zeros` `Expr.bitwise_leading_ones` `Expr.bitwise_leading_zeros` `Expr.bitwise_trailing_ones` `Expr.bitwise_trailing_zeros` `Expr.cbrt` `Expr.cos` `Expr.cosh` `Expr.cot` `Expr.cum_count` `Expr.cum_max` `Expr.cum_min` `Expr.cum_prod` `Expr.cum_sum` `Expr.cumulative_eval` `Expr.degrees` `Expr.diff` `Expr.dot` `Expr.entropy` `Expr.erf` `Expr.erfc` `Expr.ewm_mean` `Expr.ewm_mean_by` `Expr.ewm_std` `Expr.ewm_sum` `Expr.ewm_sum_by` `Expr.ewm_var` `Expr.exp` `Expr.hash` `Expr.hist` `Expr.index_of` `Expr.kurtosis` `Expr.log` `Expr.log10` `Expr.log1p` `Expr.mode` `Expr.pct_change` `Expr.peak_max` `Expr.peak_min` `Expr.radians` `Expr.rank` `Expr.rolling_kurtosis` `Expr.rolling_map` `Expr.rolling_max` `Expr.rolling_max_by` `Expr.rolling_mean` `Expr.rolling_mean_by` `Expr.rolling_median` `Expr.rolling_median_by` `Expr.rolling_min` `Expr.rolling_min_by` `Expr.rolling_quantile` `Expr.rolling_quantile_by` `Expr.rolling_rank` `Expr.rolling_rank_by` `Expr.rolling_skew` `Expr.rolling_std` `Expr.rolling_std_by` `Expr.rolling_sum` `Expr.rolling_sum_by` `Expr.rolling_var` `Expr.rolling_var_by` `Expr.search_sorted` `Expr.sign` `Expr.sin` `Expr.sinh` `Expr.skew` `Expr.sqrt` `Expr.tan` `Expr.tanh` `Expr.unique` `Expr.unique_counts` `Expr.value_counts` | 有节点 | 同名的表达式节点（expr 网络里按 Tab 搜名字；`engine/gen_catalog.py` 从 polars 签名生成） |

### [Operators](https://docs.pola.rs/api/python/version/2/reference/expressions/operators.html)（19）

| API | 归类 | 对应 / 原因 |
|---|---|---|
| `Expr.add` | 有节点 | `arith` 节点（+） |
| `Expr.and_` | 有节点 | `logic` 节点（&） |
| `Expr.eq` | 有节点 | `compare` 节点（==） |
| `Expr.eq_missing` `Expr.ne_missing` `Expr.neg` | 有节点 | 同名的表达式节点（expr 网络里按 Tab 搜名字；`engine/gen_catalog.py` 从 polars 签名生成） |
| `Expr.floordiv` | 有节点 | `arith` 节点（//） |
| `Expr.ge` | 有节点 | `compare` 节点（>=） |
| `Expr.gt` | 有节点 | `compare` 节点（>） |
| `Expr.le` | 有节点 | `compare` 节点（<=） |
| `Expr.lt` | 有节点 | `compare` 节点（<） |
| `Expr.mod` | 有节点 | `arith` 节点（%） |
| `Expr.mul` | 有节点 | `arith` 节点（*） |
| `Expr.ne` | 有节点 | `compare` 节点（!=） |
| `Expr.or_` | 有节点 | `logic` 节点（&#124;） |
| `Expr.pow` | 有节点 | `arith` 节点（**） |
| `Expr.sub` | 有节点 | `arith` 节点（-） |
| `Expr.truediv` | 有节点 | `arith` 节点（/） |
| `Expr.xor` | 有节点 | `logic` 节点（^） |

### [Columns / names](https://docs.pola.rs/api/python/version/2/reference/expressions/columns.html)（2）

| API | 归类 | 对应 / 原因 |
|---|---|---|
| `Expr.alias` | 有节点 | `output` 节点（alias） |
| `Expr.exclude` | 有节点 | 同名的表达式节点（expr 网络里按 Tab 搜名字；`engine/gen_catalog.py` 从 polars 签名生成） |

### [Aggregation](https://docs.pola.rs/api/python/version/2/reference/expressions/aggregation.html)（31）

| API | 归类 | 对应 / 原因 |
|---|---|---|
| `Expr.all` `Expr.any` `Expr.approx_n_unique` `Expr.approx_quantile` `Expr.arg_max` `Expr.arg_min` `Expr.bitwise_and` `Expr.bitwise_or` `Expr.bitwise_xor` `Expr.has_nulls` `Expr.is_empty` `Expr.max_by` `Expr.min_by` `Expr.nan_max` `Expr.nan_min` `Expr.null_count` `Expr.product` `Expr.quantile` | 有节点 | 同名的表达式节点（expr 网络里按 Tab 搜名字；`engine/gen_catalog.py` 从 polars 签名生成） |
| `Expr.count` | 有节点 | `agg` 节点（count） |
| `Expr.first` | 有节点 | `agg` 节点（first） |
| `Expr.implode` | 有节点 | `agg` 节点（implode） |
| `Expr.last` | 有节点 | `agg` 节点（last） |
| `Expr.len` | 有节点 | `agg` 节点（len） |
| `Expr.max` | 有节点 | `agg` 节点（max） |
| `Expr.mean` | 有节点 | `agg` 节点（mean） |
| `Expr.median` | 有节点 | `agg` 节点（median） |
| `Expr.min` | 有节点 | `agg` 节点（min） |
| `Expr.n_unique` | 有节点 | `agg` 节点（n_unique）、`describe` 节点（n_unique） |
| `Expr.std` | 有节点 | `agg` 节点（std） |
| `Expr.sum` | 有节点 | `agg` 节点（sum） |
| `Expr.var` | 有节点 | `agg` 节点（var） |

### [Miscellaneous](https://docs.pola.rs/api/python/version/2/reference/expressions/miscellaneous.html)（2）

| API | 归类 | 对应 / 原因 |
|---|---|---|
| `Expr.set_sorted` | 有节点 | 同名的表达式节点（expr 网络里按 Tab 搜名字；`engine/gen_catalog.py` 从 polars 签名生成） |
| `Expr.deserialize` | 不做 | 表达式存成 polars 的二进制格式；网络存在工程文件里，复制表达式用参数面板底下那行文字 |

### [Boolean](https://docs.pola.rs/api/python/version/2/reference/expressions/boolean.html)（14）

| API | 归类 | 对应 / 原因 |
|---|---|---|
| `Expr.is_between` `Expr.is_close` `Expr.is_duplicated` `Expr.is_finite` `Expr.is_first_distinct` `Expr.is_in` `Expr.is_infinite` `Expr.is_last_distinct` `Expr.is_nan` `Expr.is_not_nan` `Expr.is_not_null` `Expr.is_null` `Expr.is_unique` | 有节点 | 同名的表达式节点（expr 网络里按 Tab 搜名字；`engine/gen_catalog.py` 从 polars 签名生成） |
| `Expr.not_` | 有节点 | `logic` 节点（~） |

### [Window](https://docs.pola.rs/api/python/version/2/reference/expressions/window.html)（2）

| API | 归类 | 对应 / 原因 |
|---|---|---|
| `Expr.over` `Expr.rolling` | 有节点 | 同名的表达式节点（expr 网络里按 Tab 搜名字；`engine/gen_catalog.py` 从 polars 签名生成） |

### [Meta](https://docs.pola.rs/api/python/version/2/reference/expressions/meta.html)（19）

| API | 归类 | 对应 / 原因 |
|---|---|---|
| `Expr.meta.serialize` | 不做 | 表达式存成 polars 的二进制格式；网络存在工程文件里，复制表达式用参数面板底下那行文字 |
| `Expr.meta.as_expression` `Expr.meta.eq` `Expr.meta.has_multiple_outputs` `Expr.meta.is_column` `Expr.meta.is_column_selection` `Expr.meta.is_known_length` `Expr.meta.is_literal` `Expr.meta.is_scalar` `Expr.meta.is_length_preserving` `Expr.meta.is_regex_projection` `Expr.meta.is_row_separable` `Expr.meta.ne` `Expr.meta.output_name` `Expr.meta.pop` `Expr.meta.root_names` `Expr.meta.show_graph` `Expr.meta.tree_format` `Expr.meta.undo_aliases` | 不做 | 回答“这个表达式是什么样”（输出列名、用到哪些列、是不是标量…），结果是 Python 值不是数据；引擎用它们做提醒：行数和表对不上（is_length_preserving / is_scalar）、预览带上用到的列（root_names） |

### [List](https://docs.pola.rs/api/python/version/2/reference/expressions/list.html)（45）

| API | 归类 | 对应 / 原因 |
|---|---|---|
| `Expr.list.explode` `Expr.list.contains` `Expr.list.all` `Expr.list.any` `Expr.list.arg_max` `Expr.list.arg_min` `Expr.list.concat` `Expr.list.count_matches` `Expr.list.diff` `Expr.list.drop_nulls` `Expr.list.first` `Expr.list.gather` `Expr.list.gather_every` `Expr.list.get` `Expr.list.head` `Expr.list.item` `Expr.list.join` `Expr.list.last` `Expr.list.len` `Expr.list.max` `Expr.list.mean` `Expr.list.median` `Expr.list.min` `Expr.list.n_unique` `Expr.list.reverse` `Expr.list.sample` `Expr.list.set_difference` `Expr.list.set_intersection` `Expr.list.set_symmetric_difference` `Expr.list.set_union` `Expr.list.shift` `Expr.list.slice` `Expr.list.sort` `Expr.list.std` `Expr.list.sum` `Expr.list.tail` `Expr.list.to_array` `Expr.list.to_map` `Expr.list.to_struct` `Expr.list.unique` `Expr.list.var` | 有节点 | 同名的表达式节点（expr 网络里按 Tab 搜名字；`engine/gen_catalog.py` 从 polars 签名生成） |
| `Expr.list.agg` | 有节点 | `eval` 节点（list.agg） |
| `Expr.list.eval` | 有节点 | `eval` 节点（list.eval） |
| `Expr.list.filter` | 有节点 | `eval` 节点（list.filter） |
| `Expr.list.__getitem__` | 换个写法 | 就是 `list.get` / `list.slice` 节点 |

### [Array](https://docs.pola.rs/api/python/version/2/reference/expressions/array.html)（29）

| API | 归类 | 对应 / 原因 |
|---|---|---|
| `Expr.arr.contains` `Expr.arr.all` `Expr.arr.any` `Expr.arr.arg_max` `Expr.arr.arg_min` `Expr.arr.count_matches` `Expr.arr.dot` `Expr.arr.explode` `Expr.arr.first` `Expr.arr.get` `Expr.arr.join` `Expr.arr.last` `Expr.arr.len` `Expr.arr.max` `Expr.arr.mean` `Expr.arr.median` `Expr.arr.min` `Expr.arr.n_unique` `Expr.arr.reverse` `Expr.arr.shift` `Expr.arr.sort` `Expr.arr.std` `Expr.arr.sum` `Expr.arr.to_list` `Expr.arr.to_struct` `Expr.arr.unique` `Expr.arr.var` | 有节点 | 同名的表达式节点（expr 网络里按 Tab 搜名字；`engine/gen_catalog.py` 从 polars 签名生成） |
| `Expr.arr.agg` | 有节点 | `eval` 节点（arr.agg） |
| `Expr.arr.eval` | 有节点 | `eval` 节点（arr.eval） |

### [Struct](https://docs.pola.rs/api/python/version/2/reference/expressions/struct.html)（8）

| API | 归类 | 对应 / 原因 |
|---|---|---|
| `Expr.struct.with_fields` `Expr.struct.field` `Expr.struct.unnest` `Expr.struct.json_encode` `Expr.struct.rename_fields` `Expr.struct.eval` `Expr.struct.drop` | 有节点 | 同名的表达式节点（expr 网络里按 Tab 搜名字；`engine/gen_catalog.py` 从 polars 签名生成） |
| `Expr.struct.__getitem__` | 换个写法 | 就是 `struct.field` 节点 |

### [String](https://docs.pola.rs/api/python/version/2/reference/expressions/string.html)（47）

| API | 归类 | 对应 / 原因 |
|---|---|---|
| `Expr.str.replace` `Expr.str.contains` `Expr.str.contains_any` `Expr.str.count_matches` `Expr.str.decode` `Expr.str.encode` `Expr.str.ends_with` `Expr.str.escape_regex` `Expr.str.extract` `Expr.str.extract_all` `Expr.str.extract_groups` `Expr.str.extract_many` `Expr.str.find` `Expr.str.find_many` `Expr.str.head` `Expr.str.join` `Expr.str.json_decode` `Expr.str.json_path_match` `Expr.str.len_bytes` `Expr.str.len_chars` `Expr.str.normalize` `Expr.str.pad_end` `Expr.str.pad_start` `Expr.str.replace_all` `Expr.str.replace_many` `Expr.str.reverse` `Expr.str.slice` `Expr.str.split` `Expr.str.split_exact` `Expr.str.splitn` `Expr.str.starts_with` `Expr.str.strip_chars` `Expr.str.strip_chars_start` `Expr.str.strip_chars_end` `Expr.str.strip_prefix` `Expr.str.strip_suffix` `Expr.str.tail` `Expr.str.to_date` `Expr.str.to_datetime` `Expr.str.to_decimal` `Expr.str.to_integer` `Expr.str.to_lowercase` `Expr.str.to_time` `Expr.str.to_titlecase` `Expr.str.to_uppercase` `Expr.str.zfill` | 有节点 | 同名的表达式节点（expr 网络里按 Tab 搜名字；`engine/gen_catalog.py` 从 polars 签名生成） |
| `Expr.str.strptime` | 有节点 | `cast_columns` 节点（cast）；也有同名的表达式节点 |

### [Temporal](https://docs.pola.rs/api/python/version/2/reference/expressions/temporal.html)（45）

| API | 归类 | 对应 / 原因 |
|---|---|---|
| `Expr.dt.truncate` `Expr.dt.add_business_days` `Expr.dt.base_utc_offset` `Expr.dt.cast_time_unit` `Expr.dt.century` `Expr.dt.combine` `Expr.dt.convert_time_zone` `Expr.dt.date` `Expr.dt.day` `Expr.dt.days_in_month` `Expr.dt.dst_offset` `Expr.dt.epoch` `Expr.dt.hour` `Expr.dt.is_business_day` `Expr.dt.is_leap_year` `Expr.dt.iso_year` `Expr.dt.microsecond` `Expr.dt.millennium` `Expr.dt.millisecond` `Expr.dt.minute` `Expr.dt.month` `Expr.dt.month_end` `Expr.dt.month_start` `Expr.dt.nanosecond` `Expr.dt.offset_by` `Expr.dt.ordinal_day` `Expr.dt.quarter` `Expr.dt.replace` `Expr.dt.replace_time_zone` `Expr.dt.round` `Expr.dt.second` `Expr.dt.strftime` `Expr.dt.time` `Expr.dt.timestamp` `Expr.dt.to_string` `Expr.dt.total_days` `Expr.dt.total_hours` `Expr.dt.total_microseconds` `Expr.dt.total_milliseconds` `Expr.dt.total_minutes` `Expr.dt.total_nanoseconds` `Expr.dt.total_seconds` `Expr.dt.week` `Expr.dt.weekday` `Expr.dt.year` | 有节点 | 同名的表达式节点（expr 网络里按 Tab 搜名字；`engine/gen_catalog.py` 从 polars 签名生成） |

### [Binary](https://docs.pola.rs/api/python/version/2/reference/expressions/binary.html)（11）

| API | 归类 | 对应 / 原因 |
|---|---|---|
| `Expr.bin.contains` `Expr.bin.decode` `Expr.bin.encode` `Expr.bin.ends_with` `Expr.bin.get` `Expr.bin.head` `Expr.bin.reinterpret` `Expr.bin.size` `Expr.bin.slice` `Expr.bin.starts_with` `Expr.bin.tail` | 有节点 | 同名的表达式节点（expr 网络里按 Tab 搜名字；`engine/gen_catalog.py` 从 polars 签名生成） |

### [Categories](https://docs.pola.rs/api/python/version/2/reference/expressions/categories.html)（6）

| API | 归类 | 对应 / 原因 |
|---|---|---|
| `Expr.cat.ends_with` `Expr.cat.len_bytes` `Expr.cat.len_chars` `Expr.cat.starts_with` `Expr.cat.physical` `Expr.cat.to` | 有节点 | 同名的表达式节点（expr 网络里按 Tab 搜名字；`engine/gen_catalog.py` 从 polars 签名生成） |

### [polars.col](https://docs.pola.rs/api/python/version/2/reference/expressions/col.html)（4）

| API | 归类 | 对应 / 原因 |
|---|---|---|
| `pl.col` `pl.functions.col.Col` `pl.functions.col.Col.__call__` | 有节点 | `col` 节点 |
| `pl.functions.col.Col.__getattr__` | 换个写法 | `pl.col.金额` 是 `pl.col("金额")` 的另一种写法：`col` 节点 |

### [Extension Types](https://docs.pola.rs/api/python/version/2/reference/expressions/extension.html)（2）

| API | 归类 | 对应 / 原因 |
|---|---|---|
| `Expr.ext.storage` `Expr.ext.to` | 不做 | 扩展类型的列要先在 Python 里 `register_extension_type` 注册一个类型才有，节点图里没地方注册；遇到了在 `code` 节点里处理 |

### [Map](https://docs.pola.rs/api/python/version/2/reference/expressions/map.html)（6）

| API | 归类 | 对应 / 原因 |
|---|---|---|
| `Expr.map.contains_key` `Expr.map.entries` `Expr.map.get` `Expr.map.keys` `Expr.map.len` `Expr.map.values` | 有节点 | 同名的表达式节点（expr 网络里按 Tab 搜名字；`engine/gen_catalog.py` 从 polars 签名生成） |

## Selectors

### [Selectors](https://docs.pola.rs/api/python/version/2/reference/selectors.html)（40）

| API | 归类 | 对应 / 原因 |
|---|---|---|
| `cs.Selector` `cs.Selector.as_expr` `cs.Selector.exclude` `cs.all` `cs.alpha` `cs.alphanumeric` `cs.array` `cs.binary` `cs.boolean` `cs.by_dtype` `cs.by_index` `cs.by_name` `cs.categorical` `cs.contains` `cs.date` `cs.datetime` `cs.decimal` `cs.digit` `cs.duration` `cs.ends_with` `cs.enum` `cs.exclude` `cs.first` `cs.float` `cs.integer` `cs.last` `cs.list` `cs.map` `cs.matches` `cs.nested` `cs.numeric` `cs.signed_integer` `cs.starts_with` `cs.string` `cs.struct` `cs.temporal` `cs.time` `cs.unsigned_integer` | 换个写法 | 选择器写在参数里：`col` 节点、select / unique / drop_nulls… 的“列”、cast_columns 的键都可以写 `cs.numeric()` 这样的；不单做节点，因为选择器是“选哪几列”，不是对值的计算 |
| `cs.expand_selector` `cs.is_selector` | 不做 | 返回 Python 值（选中哪些列名 / 是不是选择器），不是表达式 |

## DataType expressions

### [Functions](https://docs.pola.rs/api/python/version/2/reference/datatype_expr/dt_functions.html)（3）

| API | 归类 | 对应 / 原因 |
|---|---|---|
| `pl.dtype_of` `pl.self_dtype` `pl.DataType.to_dtype_expr` | 换个写法 | 结果是“类型”不是值，用在要填类型的地方：类型参数（`cast`、`lit`、生成的节点里的 dtype / return_dtype）可以写 `pl.dtype_of("列")` |

### [List](https://docs.pola.rs/api/python/version/2/reference/datatype_expr/dt_list.html)（1）

| API | 归类 | 对应 / 原因 |
|---|---|---|
| `DataTypeExpr.list.inner_dtype` | 换个写法 | 结果是“类型”不是值，用在要填类型的地方：类型参数（`cast`、`lit`、生成的节点里的 dtype / return_dtype）可以写 `pl.dtype_of("列")` |

### [Array](https://docs.pola.rs/api/python/version/2/reference/datatype_expr/dt_array.html)（3）

| API | 归类 | 对应 / 原因 |
|---|---|---|
| `DataTypeExpr.arr.inner_dtype` `DataTypeExpr.arr.width` `DataTypeExpr.arr.shape` | 换个写法 | 结果是“类型”不是值，用在要填类型的地方：类型参数（`cast`、`lit`、生成的节点里的 dtype / return_dtype）可以写 `pl.dtype_of("列")` |

### [Struct](https://docs.pola.rs/api/python/version/2/reference/datatype_expr/dt_struct.html)（2）

| API | 归类 | 对应 / 原因 |
|---|---|---|
| `DataTypeExpr.struct.field_dtype` `DataTypeExpr.struct.field_names` | 换个写法 | 结果是“类型”不是值，用在要填类型的地方：类型参数（`cast`、`lit`、生成的节点里的 dtype / return_dtype）可以写 `pl.dtype_of("列")` |

## Functions

### [Functions](https://docs.pola.rs/api/python/version/2/reference/functions.html)（27）

| API | 归类 | 对应 / 原因 |
|---|---|---|
| `pl.from_repr` | 有节点 | `create` 节点（from_repr） |
| `pl.from_dict` | 有节点 | `create` 节点（from_dict） |
| `pl.concat` | 有节点 | `concat` 节点 |
| `pl.from_dicts` `pl.from_records` | 换个写法 | `create` 节点：from_dict（按列写）或 from_repr（贴打印的表格） |
| `pl.union` | 换个写法 | `concat` 节点（union 不保证行的顺序，其余一样） |
| `pl.merge_sorted` | 换个写法 | `concat` 节点（方式 merge_sorted） |
| `pl.escape_regex` | 换个写法 | 返回的是 Python 字符串；对列里的文字用 `str.escape_regex` 节点 |
| `pl.set_random_seed` | 换个写法 | `slice` 节点 sample 的 seed 参数（全局种子会影响别的节点，不做） |
| `pl.collect_all` `pl.collect_all_async` `pl.defer` `pl.explain_all` | 不做 | 执行控制（一次执行好几个查询…），引擎自己调度 |
| `pl.from_arrow` `pl.from_dataframe` `pl.from_numpy` `pl.from_pandas` `pl.from_torch` | 不做 | 从别的 Python 库的对象建表；节点图里没有这些对象，数据从 `read`（文件）或 `create`（直接写）来 |
| `pl.json_normalize` | 不做 | 输入是 Python 的字典列表（json.load 之后）；JSON 文件用 `read` 读，嵌套的结构用 `explode`（unnest）和 `struct.field` 拆 |
| `pl.align_frames` | 不做 | 结果是好几张表；对齐两张表用 `join`（how = full） |
| `pl.register_extension_type` `pl.unregister_extension_type` `pl.get_extension_type` | 不做 | 注册扩展类型要写一个 Python 类，节点图里没地方放；不注册的 `Extension("名字", 存储类型)` 在类型参数里能选；见 Expr.ext |
| `pl.StringCache` `pl.enable_string_cache` `pl.disable_string_cache` `pl.using_string_cache` | 不做 | polars 2 里什么也不做（Categorical 改成了全局的 Categories），只是为了兼容旧代码 |

## Data types

### [Data types](https://docs.pola.rs/api/python/version/2/reference/datatypes.html)（36）

| API | 归类 | 对应 / 原因 |
|---|---|---|
| `pl.DataType` `pl.Float32` `pl.Float64` `pl.Date` `pl.Int32` `pl.Datetime` `pl.Int64` `pl.Time` `pl.Duration` `pl.Categorical` `pl.UInt32` `pl.Float16` `pl.Decimal` `pl.Int8` `pl.Int16` `pl.Int128` `pl.UInt8` `pl.UInt16` `pl.UInt64` `pl.UInt128` `pl.Array` `pl.List` `pl.Map` `pl.Field` `pl.Struct` `pl.String` `pl.Categories` `pl.Enum` `pl.Binary` `pl.Boolean` `pl.Extension` `pl.Null` `pl.Object` `pl.Unknown` | 换个写法 | 类型不是操作，写在参数里：类型参数（`cast`、`lit`、生成的节点里的 dtype / return_dtype）旁边的下拉列着全部类型，带参数的（`Decimal(38, 2)`、`Datetime("ms", "Asia/Shanghai")`、`List(Int64)`、`Struct({"a": Int64})`…）在输入框里改；表格级的用 `cast_columns` 的“列: 类型”；列的类型显示在表头第二行 |
| `pl.Utf8` | 换个写法 | 就是 `pl.String`（旧名字） |
| `pl.BaseExtension` | 不做 | 注册扩展类型要写一个 Python 类，节点图里没地方放；不注册的 `Extension("名字", 存储类型)` 在类型参数里能选；见 Expr.ext |

## Input/output

### [Input/output](https://docs.pola.rs/api/python/version/2/reference/io.html)（56）

| API | 归类 | 对应 / 原因 |
|---|---|---|
| `DataFrame.write_avro` | 有节点 | `write` 节点（Avro） |
| `DataFrame.write_clipboard` | 有节点 | `write` 节点（剪贴板） |
| `DataFrame.write_excel` | 有节点 | `write` 节点（Excel） |
| `DataFrame.write_ipc_stream` `LazyFrame.sink_ipc` | 有节点 | `write` 节点（IPC） |
| `DataFrame.write_json` `LazyFrame.sink_ndjson` | 有节点 | `write` 节点（JSON） |
| `DataFrame.write_ndjson` | 有节点 | `write` 节点（JSON，打开 ndjson 或扩展名 .ndjson / .jsonl） |
| `pl.read_clipboard` | 有节点 | `create` 节点（read_clipboard） |
| `LazyFrame.sink_csv` | 有节点 | `write` 节点（CSV） |
| `LazyFrame.sink_parquet` | 有节点 | `write` 节点（Parquet） |
| `pl.PartitionBy` | 有节点 | `write` 节点（CSV）、`write` 节点（Parquet）、`write` 节点（IPC） |
| `pl.read_avro` | 有节点 | `read` 节点（Avro） |
| `pl.read_csv` | 有节点 | `read` 节点（CSV；gbk / gb18030 编码时用 read_csv，utf8 用 scan_csv） |
| `pl.scan_csv` | 有节点 | `read` 节点（CSV） |
| `pl.read_excel` | 有节点 | `read` 节点（Excel） |
| `pl.read_ipc_stream` `pl.scan_ipc` | 有节点 | `read` 节点（IPC） |
| `pl.read_json` `pl.scan_ndjson` | 有节点 | `read` 节点（JSON） |
| `pl.read_lines` `pl.scan_lines` | 有节点 | `read` 节点（CSV 格式，打开“按行读”） |
| `pl.scan_parquet` | 有节点 | `read` 节点（Parquet） |
| `DataFrame.write_csv` `DataFrame.write_ipc` `DataFrame.write_parquet` | 换个写法 | `write` 节点用的是同格式的 sink_*（流式写，不用整表进内存），结果一样 |
| `pl.read_ods` | 换个写法 | `read` 节点（Excel 格式，.ods 文件）：read_excel 的 calamine 引擎也读 ods |
| `pl.read_ipc` `pl.read_ndjson` `pl.read_parquet` | 换个写法 | `read` 节点用的是同格式的 scan_*（懒读，只读用到的列和行），结果一样 |
| `pl.ScanCastOptions` | 换个写法 | `read` 节点的“其他参数”里写 `cast_options=pl.ScanCastOptions(…)`（读一组文件、类型不一致时用） |
| `DataFrame.write_database` `pl.read_database` `pl.read_database_uri` | 不做 | 要数据库连接（connectorx / ADBC / SQLAlchemy 这些依赖，打包的 Python 里没有），账号密码会存进工程文件；查看器只管文件 |
| `DataFrame.write_delta` `pl.read_delta` `pl.scan_delta` | 不做 | Delta Lake 要 deltalake 包，是数据湖的目录 + 事务日志，不是一个文件 |
| `DataFrame.write_iceberg` `pl.scan_iceberg` | 不做 | Iceberg 要 pyiceberg 包和一个 catalog 服务，不是一个文件 |
| `LazyFrame.sink_delta` `LazyFrame.sink_iceberg` | 不做 | 见 IO 里 Delta Lake / Iceberg 那两行：要额外的包，不是一个文件 |
| `pl.read_ipc_schema` `pl.read_parquet_metadata` `pl.read_parquet_schema` | 不做 | 只读文件的结构 / 元信息；打开文件后表头就显示各列类型 |
| `pl.scan_lance` | 不做 | Lance 要 pylance 包，数据集是一个目录 + 版本，不是一个文件 |
| `pl.FileProviderArgs` | 不做 | PartitionBy 自定义文件名的回调拿到的参数，要写 Python 函数；`write` 的“按列拆分”用 polars 默认的文件名 |
| `pl.scan_external_reader` `io.external_reader.FileReader` `io.external_reader.FileReaderBuilder` `io.external_reader.ReaderCapabilities` | 不做 | 用 Python 写的自定义读取器（FileReaderBuilder 子类）；要用的话在 `code` 节点里写 |
| `pl.scan_arrow_c_stream` | 不做 | 从实现了 Arrow C 接口的 Python 对象读，节点图里没有这种对象 |
| `pl.scan_pyarrow_dataset` | 不做 | 要 pyarrow 的 Dataset 对象；一个目录下的多个 parquet，`read` 的路径可以直接写通配符（`数据/*.parquet`，scan_parquet 认），只是文件改了缓存不会自己失效（按路径取修改时间） |
| `pl.CredentialProvider` `pl.CredentialProviderAWS` `pl.CredentialProviderAzure` `pl.CredentialProviderGCP` | 不做 | 云存储（S3 / Azure / GCS）的凭据；查看器读本地文件，凭据也不该存进工程文件 |

## Catalog

### [Unity Catalog](https://docs.pola.rs/api/python/version/2/reference/catalog/unity.html)（13）

| API | 归类 | 对应 / 原因 |
|---|---|---|
| `Catalog` `Catalog.list_catalogs` `Catalog.list_namespaces` `Catalog.list_tables` `Catalog.get_table_info` `Catalog.scan_table` `catalog.unity.CatalogInfo` `catalog.unity.ColumnInfo` `catalog.unity.DataSourceFormat` `catalog.unity.NamespaceInfo` `catalog.unity.TableInfo` `catalog.unity.TableInfo.get_polars_schema` `catalog.unity.TableType` | 不做 | Databricks Unity Catalog，要云账号和服务地址 |

## Config

### [Config](https://docs.pola.rs/api/python/version/2/reference/config.html)（31）

| API | 归类 | 对应 / 原因 |
|---|---|---|
| `Config.set_ascii_tables` `Config.set_decimal_separator` `Config.set_thousands_separator` `Config.set_float_precision` `Config.set_fmt_float` `Config.set_fmt_str_lengths` `Config.set_fmt_table_cell_list_len` `Config.set_tbl_cell_alignment` `Config.set_tbl_cell_numeric_alignment` `Config.set_tbl_cols` `Config.set_tbl_column_data_type_inline` `Config.set_tbl_dataframe_shape_below` `Config.set_tbl_formatting` `Config.set_tbl_hide_column_data_types` `Config.set_tbl_hide_column_names` `Config.set_tbl_hide_dtype_separator` `Config.set_tbl_hide_dataframe_shape` `Config.set_tbl_width_chars` `Config.set_trim_decimal_zeros` `Config.set_engine_affinity` `Config.set_streaming_chunk_size` `Config.enable_monitoring` `Config.set_default_credential_provider` `Config.set_tbl_rows` `Config.set_verbose` `Config.load` `Config.load_from_file` `Config.save` `Config.save_to_file` `Config.state` `Config.restore_defaults` | 不做 | 控制 print(df) 的排版（行数、列宽、小数位…）和引擎调优；查看器自己画表格，不经过 polars 的打印 |

## Extending the API

### [Extending the API](https://docs.pola.rs/api/python/version/2/reference/api.html)（4）

| API | 归类 | 对应 / 原因 |
|---|---|---|
| `api.register_expr_namespace` `api.register_dataframe_namespace` `api.register_lazyframe_namespace` `api.register_series_namespace` | 不做 | 给 polars 加自定义命名空间要写 Python 代码；本程序扩展节点的方式是自己写节点类型（nodes/*.py，“节点 → 新建类型”） |

## Plugins

### [Plugins](https://docs.pola.rs/api/python/version/2/reference/plugins.html)（4）

| API | 归类 | 对应 / 原因 |
|---|---|---|
| `plugins.register_plugin_function` `io.plugins.register_io_source` `io.plugins.IOSourceScanFunction` `io.plugins.IOSourceScanFunction.io_source` | 不做 | polars 插件是另外编译的 Rust 包 / Python 的 IO 源；要用的话在 `code` 节点里 import |

## SQL Interface

### [Python API](https://docs.pola.rs/api/python/version/2/reference/sql/python_api.html)（11）

| API | 归类 | 对应 / 原因 |
|---|---|---|
| `SQLContext.execute` `SQLContext.unregister` `SQLContext.execute_global` `SQLContext.register` `SQLContext.register_globals` `SQLContext.register_many` `SQLContext.tables` `SQLContext.__init__` `SQLContext.__enter__` `SQLContext.__exit__` | 有节点 | `sql` 节点（第 1 个输入注册成 self / df，其余是 df2 ~ df4） |
| `SQLContext` | 有节点 | `sql` 节点 |

## Exceptions

### [Exceptions](https://docs.pola.rs/api/python/version/2/reference/exceptions.html)（31）

| API | 归类 | 对应 / 原因 |
|---|---|---|
| `exceptions.PolarsError` `exceptions.AttributeRemovedError` `exceptions.ColumnNotFoundError` `exceptions.ComputeError` `exceptions.DuplicateError` `exceptions.InvalidOperationError` `exceptions.ModuleUpgradeRequiredError` `exceptions.NoDataError` `exceptions.NoRowsReturnedError` `exceptions.OutOfBoundsError` `exceptions.ParameterCollisionError` `exceptions.RowsError` `exceptions.SQLInterfaceError` `exceptions.SQLSyntaxError` `exceptions.SchemaError` `exceptions.SchemaFieldNotFoundError` `exceptions.ShapeError` `exceptions.StringCacheMismatchError` `exceptions.StructFieldNotFoundError` `exceptions.TooManyRowsReturnedError` `exceptions.UnsuitableSQLError` `exceptions.PolarsWarning` `exceptions.CategoricalRemappingWarning` `exceptions.ChronoFormatWarning` `exceptions.CustomUFuncWarning` `exceptions.DataOrientationWarning` `exceptions.MapWithoutReturnDtypeWarning` `exceptions.PerformanceWarning` `exceptions.PolarsInefficientMapWarning` `exceptions.UnstableWarning` `exceptions.PanicException` | 不做 | 错误的类型，不是功能；出错时节点标红、状态栏和参数面板显示 polars 的报错 |

## Testing

### [Testing](https://docs.pola.rs/api/python/version/2/reference/testing.html)（12）

| API | 归类 | 对应 / 原因 |
|---|---|---|
| `testing.assert_frame_equal` `testing.assert_series_equal` `testing.assert_frame_not_equal` `testing.assert_schema_equal` `testing.assert_series_not_equal` `testing.parametric.dataframes` `testing.parametric.dtypes` `testing.parametric.lists` `testing.parametric.series` `testing.parametric.column` `testing.parametric.load_profile` `testing.parametric.set_profile` | 不做 | 写测试用的断言和随机数据生成；比较两张表用 `join`（anti），造数据用 `create` |

## Metadata

### [Metadata](https://docs.pola.rs/api/python/version/2/reference/metadata.html)（4）

| API | 归类 | 对应 / 原因 |
|---|---|---|
| `pl.get_index_type` `pl.build_info` `pl.show_versions` `pl.thread_pool_size` | 不做 | 版本 / 编译信息；引擎启动时检查 polars 版本（要 2.x） |
