r"""用 calamine 的 fork 重新编译 fastexcel（polars 的 Excel 读取引擎）。

polars 的 `engine="calamine"` 其实是调用 fastexcel，而 fastexcel 里的 calamine
是编译进去的。想让 `pl.read_excel` 用上 fork 的 xlsx 并行快速路径，就得拿 fork
重新编译一份 fastexcel，装回 Python 环境（`--install`），打包时也会优先用它。

对 fastexcel 源码做三处改动：
  1. Cargo.toml 加 `[patch.crates-io] calamine = { path = <fork> }`；
  2. `supports_by_ref` 一律返回 false。有 pyarrow 时 polars 走 `load_sheet_eager`
     → `worksheet_range_ref`（逐行 quick-xml，单线程），绕开 fork 的快速路径；
     改成走 `worksheet_range`，代价只是把 by-ref 结果再转成 owned 的那一次拷贝
     （fastexcel 的 eager 路径本来就要转）。
  3. `create_string_array` 改用 Arrow 的 `StringBuilder` 直接写值。原来每格都先
     `to_string()` 成 `Vec<String>` 再交给 arrow 复制一遍，一千万个文本格多出
     一千万次分配；改成一次写进连续的字符串缓冲（同一文件 0.9s → 0.27s）。

用法（在本仓库根目录）：
    python engine/build_fastexcel.py --install
    python engine/build_fastexcel.py --calamine D:\calamine --fastexcel 0.21.0
    python engine/build_fastexcel.py --install --from-wheel engine/wheels/xxx.whl

产物：engine/wheels/fastexcel-<版本>-<abi>.whl（package.ps1 会把它装进随包运行时）。
"""

from __future__ import annotations

import argparse
import os
import re
import subprocess
import sys
import tarfile
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
WHEELS = ROOT / "engine" / "wheels"


def fail(msg: str) -> None:
    print(f"错误：{msg}", file=sys.stderr)
    raise SystemExit(1)


def run(cmd: list[str], **kw) -> subprocess.CompletedProcess:
    print("+", subprocess.list2cmdline(cmd), flush=True)
    return subprocess.run(cmd, check=True, **kw)


def installed_version() -> str:
    out = run([sys.executable, "-c", "import fastexcel; print(fastexcel.__version__)"],
              capture_output=True, text=True)
    return out.stdout.strip()


def download_sdist(version: str, dest: Path) -> Path:
    run([sys.executable, "-m", "pip", "download", f"fastexcel=={version}",
         "--no-binary", ":all:", "--no-deps", "-d", str(dest)])
    files = list(dest.glob("fastexcel-*.tar.gz"))
    if not files:
        fail(f"没有下载到 fastexcel {version} 的源码包")
    return files[0]


def patch_cargo(source: Path, calamine: Path) -> None:
    cargo = source / "Cargo.toml"
    text = cargo.read_text(encoding="utf-8")
    if "[patch.crates-io]" in text:
        fail("fastexcel 的 Cargo.toml 里已经有 [patch.crates-io]，请检查源码")
    # TOML 里用正斜杠，免得 Windows 路径的反斜杠被当成转义
    path = calamine.resolve().as_posix()
    cargo.write_text(text.rstrip() + f'\n\n[patch.crates-io]\ncalamine = {{ path = "{path}" }}\n',
                     encoding="utf-8")


def patch_by_ref(source: Path) -> None:
    """让 xlsx 的 eager 读取走 owned 的 worksheet_range（用上 fork 的快速路径）。"""
    target = source / "src" / "types" / "excelreader" / "mod.rs"
    text = target.read_text(encoding="utf-8")
    pattern = re.compile(
        r"(?P<fn>fn supports_by_ref\(&self\) -> bool \{)(?P<body>.*?)(?P<end>\n    \})",
        re.S,
    )
    m = pattern.search(text)
    if not m or "matches!" not in m.group("body"):
        fail("fastexcel 的 supports_by_ref 和预期的不一样，请检查源码后再改这个脚本")
    patched = (
        m.group("fn")
        + "\n        // 补丁：一律走 owned 的 worksheet_range，用上 calamine fork 的 xlsx 并行快速路径\n"
        + "        false"
        + m.group("end")
    )
    target.write_text(text[:m.start()] + patched + text[m.end():], encoding="utf-8")


def replace_function(text: str, signature: str, indent: str, new: str) -> str:
    """把顶格（或 indent 缩进）的整个函数换成 new（按行找函数结束的孤零零的 }）。"""
    lines = text.splitlines(keepends=True)
    start = next(i for i, line in enumerate(lines) if line.startswith(signature))
    end = next(i for i in range(start + 1, len(lines)) if lines[i].rstrip("\r\n") == indent + "}")
    return "".join(lines[:start]) + new + "".join(lines[end + 1:])


STRING_BUILDER = '''pub(crate) fn create_string_array<CT: CellType + DataType>(
    data: &Range<CT>,
    col: usize,
    row_iter: impl Iterator<Item = usize>,
    whitespace_as_null: bool,
) -> Arc<dyn Array> {
    let capacity = row_iter.size_hint().0;
    let mut builder = StringBuilder::with_capacity(capacity, 0);
    for row in row_iter {
        let Some(cell) = data.get((row, col)) else {
            builder.append_null();
            continue;
        };
        if cell.is_empty() {
            builder.append_null();
            continue;
        }
        if cell.is_string() {
            if let Some(value) = cell.get_string() {
                if whitespace_as_null && value.trim().is_empty() {
                    builder.append_null();
                } else {
                    builder.append_value(value);
                }
                continue;
            }
        }
        match cell_extractors::extract_string(cell) {
            Some(value) => {
                if whitespace_as_null && value.trim().is_empty() {
                    builder.append_null();
                } else {
                    builder.append_value(&value);
                }
            }
            None => builder.append_null(),
        }
    }
    Arc::new(builder.finish())
}
'''

STRING_BUILDER_ERRORS = '''    pub(crate) fn create_string_array_with_errors<CT: CellType + DataType + Debug>(
        data: &Range<CT>,
        col: usize,
        offset: usize,
        limit: usize,
        whitespace_as_null: bool,
    ) -> (Arc<dyn Array>, Vec<CellError>) {
        let mut cell_errors = vec![];
        let mut builder = StringBuilder::with_capacity(limit.saturating_sub(offset), 0);

        for row in offset..limit {
            let Some(cell) = data.get((row, col)) else {
                builder.append_null();
                continue;
            };
            if cell.is_empty() {
                builder.append_null();
                continue;
            }
            if cell.is_string() {
                if let Some(value) = cell.get_string() {
                    if whitespace_as_null && value.trim().is_empty() {
                        builder.append_null();
                    } else {
                        builder.append_value(value);
                    }
                    continue;
                }
            }
            match cell_extractors::extract_string(cell) {
                Some(value) => {
                    if whitespace_as_null && value.trim().is_empty() {
                        builder.append_null();
                    } else {
                        builder.append_value(&value);
                    }
                }
                None => {
                    cell_errors.push(CellError {
                        position: (row, col),
                        row_offset: offset,
                        detail: format!("Expected string but got '{cell:?}'"),
                    });
                    builder.append_null();
                }
            }
        }

        let arr: Arc<dyn Array> = Arc::new(builder.finish());
        (arr, cell_errors)
    }
'''


def patch_string_builder(source: Path) -> None:
    """文本列直接用 Arrow 的 StringBuilder 写，省掉每格一次 String 分配和一次复制。"""
    target = source / "src" / "data" / "python.rs"
    text = target.read_text(encoding="utf-8")
    if "StringBuilder::with_capacity" in text:
        fail("fastexcel 里已经有 StringBuilder 了，检查是不是重复打补丁")
    old_import = "    NullArray, RecordBatch, StringArray, TimestampMillisecondArray,\n"
    if old_import not in text:
        fail("fastexcel 的 arrow_array 导入和预期的不一样，请检查源码后再改这个脚本")
    text = text.replace(
        old_import,
        "    NullArray, RecordBatch, StringArray, TimestampMillisecondArray, builder::StringBuilder,\n",
        1,
    )
    text = replace_function(
        text,
        "pub(crate) fn create_string_array<",
        "",
        STRING_BUILDER,
    )
    text = replace_function(
        text,
        "    pub(crate) fn create_string_array_with_errors<",
        "    ",
        STRING_BUILDER_ERRORS,
    )
    target.write_text(text, encoding="utf-8")


def patch_marker(source: Path) -> None:
    """在包里留个记号，打包时用它确认随包的是打过补丁的 fastexcel。"""
    init = source / "python" / "fastexcel" / "__init__.py"
    if not init.exists():
        fail("找不到 fastexcel 的 __init__.py")
    init.write_text(init.read_text(encoding="utf-8").rstrip()
                    + "\n\n__nodesheet_fast__ = True  # calamine fork 的 xlsx 快速路径\n",
                    encoding="utf-8")


def build(source: Path, out: Path) -> Path:
    out.mkdir(parents=True, exist_ok=True)
    run([sys.executable, "-m", "pip", "wheel", ".", "--no-deps", "-w", str(out)], cwd=source)
    wheels = sorted(out.glob("fastexcel-*.whl"), key=lambda p: p.stat().st_mtime)
    if not wheels:
        fail("编译完了却没找到 wheel")
    return wheels[-1]


def install(wheel: Path) -> None:
    run([sys.executable, "-m", "pip", "install", "--force-reinstall", "--no-deps", str(wheel)])


def main() -> None:
    ap = argparse.ArgumentParser(description="用 calamine fork 重新编译 fastexcel")
    ap.add_argument("--calamine", default=os.environ.get("RTV_CALAMINE", r"D:\calamine"),
                    help="calamine fork 的目录（默认 D:\\calamine，或环境变量 RTV_CALAMINE）")
    ap.add_argument("--fastexcel", default=None, help="fastexcel 版本（默认和当前装的一样）")
    ap.add_argument("--out", default=str(WHEELS), help="wheel 放哪（默认 engine/wheels）")
    ap.add_argument("--install", action="store_true", help="编译完装回当前 Python")
    ap.add_argument("--from-wheel", default=None, help="不编译，直接安装已有的 wheel")
    args = ap.parse_args()

    out = Path(args.out)

    if args.from_wheel:
        wheel = Path(args.from_wheel)
        if not wheel.exists():
            fail(f"找不到 {wheel}")
    elif (existing := sorted(out.glob("fastexcel-*.whl"), key=lambda p: p.stat().st_mtime)):
        # 已经有编译好的就直接用（重编要几分钟）；要重编就先删掉它
        wheel = existing[-1]
        print(f"用已有的 {wheel}（要重新编译请先删掉它）")
    else:
        calamine = Path(args.calamine)
        if not (calamine / "Cargo.toml").exists():
            fail(f"{calamine} 不像 calamine 的源码目录（可以加 --calamine 指定）")
        version = args.fastexcel or installed_version()
        with tempfile.TemporaryDirectory(prefix="fastexcel-") as tmp:
            tmpdir = Path(tmp)
            sdist = download_sdist(version, tmpdir)
            with tarfile.open(sdist) as tar:
                try:
                    tar.extractall(tmpdir, filter="data")  # Python 3.12+
                except TypeError:
                    tar.extractall(tmpdir)
            source = next(p for p in tmpdir.iterdir() if p.is_dir())
            patch_cargo(source, calamine)
            patch_by_ref(source)
            patch_string_builder(source)
            patch_marker(source)
            wheel = build(source, out)

    print(f"wheel: {wheel}")
    if args.install:
        install(wheel)
        run([sys.executable, "-c",
             "import fastexcel; print('fastexcel', fastexcel.__version__, fastexcel.__file__)"])
    else:
        print(f"装到当前 Python：python -m pip install --force-reinstall --no-deps \"{wheel}\"")


if __name__ == "__main__":
    main()
