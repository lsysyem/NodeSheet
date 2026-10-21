# 打包成可以直接发给别人的文件夹 + zip：
#   dist\NodeSheet\NodeSheet.exe
#   dist\NodeSheet\engine\      随包的 Python + polars + fastexcel + xlsxwriter（节点功能用，对方不用装 Python）
# 运行时取自本机的 Python 安装（需要已 pip install -U "polars>=2.0.0" fastexcel xlsxwriter），不联网。
# Excel 读取优先用 engine\wheels 里打过补丁的 fastexcel（engine/build_fastexcel.py 用 calamine fork 编的）。
# 用法：pwsh .\package.ps1
$ErrorActionPreference = "Stop"
Set-Location $PSScriptRoot

$version = (Select-String -Path Cargo.toml -Pattern '^version = "(.+)"').Matches[0].Groups[1].Value
$name = "NodeSheet"
$out = "dist\$name"
$engine = "$out\engine"

cargo build --release
if ($LASTEXITCODE -ne 0) { throw "编译失败" }

# 本机 Python 的位置、版本号，以及 polars / fastexcel / xlsxwriter（写出 Excel 用）装在哪
$info = python -c "import sys, os, polars, fastexcel, xlsxwriter, _polars_runtime_32 as rt; print(sys.base_prefix); print(f'{sys.version_info.major}{sys.version_info.minor}'); [print(os.path.dirname(os.path.dirname(m.__file__))) for m in (polars, rt, fastexcel, xlsxwriter)]"
if ($LASTEXITCODE -ne 0) { throw "本机 Python 里没有 polars / fastexcel / xlsxwriter：pip install -U `"polars>=2.0.0`" fastexcel xlsxwriter" }
# 节点按 polars 2 写的
python -c "import sys, polars; sys.exit(int(polars.__version__.split('.')[0]) < 2)"
if ($LASTEXITCODE -ne 0) { throw "本机 Python 里的 polars 低于 2.0：pip install -U `"polars>=2.0.0`"" }
$py, $tag, $sites = $info[0], $info[1], ($info[2..5] | Select-Object -Unique)

if (Test-Path $out) { Remove-Item $out -Recurse -Force }
New-Item -ItemType Directory -Force "$engine\DLLs", "$engine\Lib\site-packages" | Out-Null
Copy-Item "target\release\nodesheet.exe" "$out\$name.exe"

# 解释器本体
Copy-Item "$py\python.exe", "$py\python3.dll", "$py\python$tag.dll", "$py\vcruntime140*.dll", "$py\LICENSE.txt" $engine
# 扩展模块：不带测试、tkinter
Get-ChildItem "$py\DLLs" -Include *.pyd, *.dll -Recurse |
    Where-Object { $_.Name -notmatch '^(_test|_ctypes_test|_tkinter|tcl|winsound)' } |
    Copy-Item -Destination "$engine\DLLs"
# 标准库：不带测试、GUI、pip 等用不到的大块头
robocopy "$py\Lib" "$engine\Lib" /E /NFL /NDL /NJH /NJS /NP `
    /XD site-packages test tests idlelib tkinter turtledemo ensurepip venv pydoc_data __pycache__ | Out-Null
if ($LASTEXITCODE -ge 8) { throw "复制标准库失败" }
# 第三方包
foreach ($site in $sites) {
    Get-ChildItem $site -Directory |
        Where-Object { $_.Name -match '^(polars|_polars_runtime_32|polars_runtime_32|fastexcel|xlsxwriter)([-.]|$)' } |
        ForEach-Object {
            robocopy $_.FullName "$engine\Lib\site-packages\$($_.Name)" /E /NFL /NDL /NJH /NJS /NP /XD __pycache__ | Out-Null
            if ($LASTEXITCODE -ge 8) { throw "复制 $($_.Name) 失败" }
        }
}
# ._pth：只认包里的路径，不受对方机器上的 Python / 环境变量影响
Set-Content "$engine\python$tag._pth" "DLLs`nLib`nLib\site-packages" -Encoding ascii

# Excel 读取：engine\wheels 里有打过补丁的 fastexcel 就换掉随包的普通版
# （engine/build_fastexcel.py 用 calamine fork 编的，xlsx 并行解析）
$fxWheel = Get-ChildItem "engine\wheels\fastexcel-*.whl" -ErrorAction SilentlyContinue | Select-Object -First 1
if ($fxWheel) {
    $tmp = Join-Path $env:TEMP "nodesheet-fastexcel"
    if (Test-Path $tmp) { Remove-Item $tmp -Recurse -Force }
    Expand-Archive $fxWheel.FullName -DestinationPath $tmp
    Remove-Item "$engine\Lib\site-packages\fastexcel" -Recurse -Force -ErrorAction SilentlyContinue
    Copy-Item "$tmp\fastexcel" "$engine\Lib\site-packages\fastexcel" -Recurse
    Remove-Item $tmp -Recurse -Force
    Write-Host "fastexcel：用打过补丁的 $($fxWheel.Name)"
} else {
    Write-Warning "engine\wheels 里没有打过补丁的 fastexcel：Excel 读取会用普通的 calamine。运行 python engine\build_fastexcel.py --install 生成"
}

# 确认随包的运行时自己能跑
$check = & "$engine\python.exe" -c "import sys, polars, fastexcel, xlsxwriter; print(polars.__version__, fastexcel.__version__); print(all(p.startswith(sys.prefix) for p in sys.path))"
if ($LASTEXITCODE -ne 0 -or $check[1] -ne "True") { throw "随包的 Python 运行时不可用：$check" }
if ($fxWheel) {
    $fast = & "$engine\python.exe" -c "import fastexcel; print(getattr(fastexcel, '__nodesheet_fast__', False))"
    if ($fast -ne "True") { throw "随包的 fastexcel 不是打过补丁的版本：$fast" }
}

Set-Content "$out\使用说明.txt" -Encoding utf8 @"
NodeSheet v$version

双击「$name.exe」运行，或把表格文件拖到窗口里。
支持 xlsx / xlsm / xls / xlsb / ods / parquet / csv / tsv。

界面照 Houdini 的样子：左边表格，右边上面是参数、下面是节点图（Ctrl+E 开关）。
筛选、排序、分组、连接、写 polars 代码都用节点做。
  · 顶上的工具架按分类列着节点，点一下就接在选中节点的下面；也可以点节点图上的「添加」，
    或在节点图上按 Tab 搜索（名字 / polars 方法 / 中文说明）
  · Excel 的 sheet 在 read 节点的 sheet_name 里选
  · 筛选、排序、分组、新增 / 改写列用 expr 节点：先在参数里选上下文（with_columns / select / filter /
    group_by / sort），再双击节点（或选中按回车）进到里面的表达式网络。里面的节点从左往右连，
    每个是一个 polars 表达式（列、常量、运算、比较、条件、聚合，以及 str / dt / list… 的全部方法），
    最后连到输出节点上（输出列 / 分组键 / 条件 / 排序键，多个时按画布上从上到下排）。
    节点头上写着它输出的类型，连线按类型上色；选中网络里的节点，表格显示它算出来的值。
    按 U 或点路径栏上的「节点图」回到外面
  · 节点是一个窄条：右端的旗标点亮（蓝）= 表格显示这个节点的结果；左端的旗标点亮（黄）= 旁路，
    这个节点不做事，输入原样传下去。鼠标停在旗标上有说明
  · 从节点下沿的端口拖到另一个节点上连线；点连线后按 Delete 删连线，选中节点按 Delete 删节点
  · 鼠标中键按住节点：看它输出几行几列、用了多久、各列是什么类型
  · 节点的名字在参数面板顶上改；节点图上的「适应」把所有节点摆进视野，滚轮缩放，双击空白复位
  · write 节点：填好文件、点「写出」把结果存成 Excel / CSV / Parquet
  · 表格上方的「Python 脚本」页签：把整条节点链导出成能直接用 python 运行的脚本，可复制或存成 .py
  · 文件 → 保存（Ctrl+S）：把节点图和界面布局存成 .rtv 工程，下次直接打开
  · 设置里可以换深色 / 浅色主题，把表格和节点面板左右互换、节点图和参数上下互换；
    面板之间的缝可以拖动调大小

自定义节点（像 Houdini 的数字资产）：
  · code 节点里写 polars 代码，用 ch("名字") 取参数；点参数面板上的「参数界面」给它加参数
    （「从代码生成参数」会按代码里的 ch 自动加），拖动参数类型就能排版；
    编辑器会提示代码里用到、但还没有的参数，以及有、但代码没用到的参数
  · 点「存为类型」把它存成新的节点，之后在「添加」菜单和 Tab 搜索里都有
  · 自定义节点存在 %APPDATA%\nodesheet\nodes，一个节点一个 .py 文件，可以直接拷给别人；
    放在程序旁边的 nodes 文件夹里的也会自动加载

表格里：点格子 / 拖出一块 / 点行号、列名选择，Shift + 点扩选，Ctrl+A 全选；
右键或 Ctrl+C 复制（Ctrl+Shift+C 带表头），可以直接粘到 Excel。
表头 / 格子右键还能排序、统计各值的个数、去掉某列、只看某个值的行：自动在节点图里建好节点。
节点图改错了 Ctrl+Z 撤销、Ctrl+Y 重做；改过没存时关窗口会先问要不要保存。
数据文件在外面改过了：文件 → 重新加载（Ctrl+R）。把文件拖到节点图上是加一个 read 节点。

engine 文件夹是节点功能用的 Python + polars（$($check[0])），请和 exe 放在一起，不要删。
不需要安装，整个文件夹可以放在任何位置。系统要求：Windows 10 / 11 64 位。
"@

$zip = "dist\$name-v$version-win64.zip"
if (Test-Path $zip) { Remove-Item $zip }
# 用 .NET 压缩：中文文件名带 UTF-8 标记，任何解压软件都不会乱码（tar -a 写出来的是本地编码）
Add-Type -AssemblyName System.IO.Compression.FileSystem
[IO.Compression.ZipFile]::CreateFromDirectory((Resolve-Path $out), "$PSScriptRoot\$zip", "Optimal", $true)

$size = { param($p) "{0:N0} MB" -f ((Get-ChildItem $p -Recurse -File | Measure-Object Length -Sum).Sum / 1MB) }
"文件夹 $out ：$(& $size $out)"
"压缩包 $zip ：$(& $size $zip)"
