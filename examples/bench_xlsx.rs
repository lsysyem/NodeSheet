//! xlsx 解码耗时基准：cargo run --release --example bench_xlsx -- <file.xlsx>
//!
//! 测量四段耗时：
//! 1. 打开工作簿（zip + sharedStrings + styles）
//! 2. 元信息（<dimension>，不解码单元格）
//! 3. 流式解码前 N 行（首屏）
//! 4. 全量解码 worksheet_range（当前实现的方式）

use std::time::Instant;

use calamine::{Data, Reader, Xlsx};

fn main() {
    inflate_only(&std::env::args().nth(1).unwrap());
    let path = std::env::args().nth(1).expect("用法: bench_xlsx <file.xlsx>");
    let first_n: usize = std::env::args()
        .nth(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(1000);

    let t = Instant::now();
    let file = std::fs::File::open(&path).expect("打开失败");
    let mut wb: Xlsx<std::io::BufReader<std::fs::File>> =
        Xlsx::new(std::io::BufReader::new(file)).expect("打开失败");
    let open_t = t.elapsed();
    let names = wb.sheet_names();
    println!("文件: {path}");
    println!("1) 打开工作簿:            {:>10.1?}", open_t);

    let t = Instant::now();
    let dims = {
        let mut r = wb.worksheet_cells_reader(&names[0]).expect("cells reader");
        r.dimensions()
    };
    println!("2) 元信息 dimension:      {:>10.1?}  ({dims:?})", t.elapsed());

    // 流式解码前 N 行 → 转字符串（模拟首屏）
    let t = Instant::now();
    let (rows, cells) = {
        let mut r = wb.worksheet_cells_reader(&names[0]).expect("cells reader");
        let mut row = 0usize;
        let mut rows: Vec<Vec<String>> = Vec::new();
        let mut cur: Vec<String> = Vec::new();
        let mut last_row = 0u32;
        let mut cells = 0usize;
        while row < first_n {
            match r.next_cell().expect("cell") {
                Some(cell) => {
                    if cell.get_position().0 != last_row {
                        if row > 0 {
                            rows.push(std::mem::take(&mut cur));
                        }
                        row += 1;
                        last_row = cell.get_position().0;
                    }
                    cur.push(fmt(cell.get_value().clone().into()));
                    cells += 1;
                }
                None => break,
            }
        }
        (rows.len(), cells)
    };
    println!(
        "3) 流式解码前 {first_n} 行: {:>10.1?}  ({rows} 行, {cells} 格)",
        t.elapsed()
    );

    stream_all(&mut wb, &names[0], false);
    stream_all(&mut wb, &names[0], true);

    let t = Instant::now();
    let range = wb.worksheet_range(&names[0]).expect("解码失败");
    let full_t = t.elapsed();
    let est_mb = range.height() * range.width() * std::mem::size_of::<Data>()
        / (1024 * 1024);
    println!(
        "4) 全量解码 worksheet_range: {:>9.1?}  ({} 行 × {} 列, 枚举内存约 {est_mb} MB)",
        full_t,
        range.height(),
        range.width()
    );

    // 全量解码后转字符串的开销（window 全表 = 列分析/排序的路径）
    let t = Instant::now();
    let mut n = 0usize;
    for r in 0..range.height() {
        for c in 0..range.width() {
            if range.get((r, c)).map(cell_str).is_some() {
                n += 1;
            }
        }
    }
    println!("5) 全表转字符串:          {:>10.1?}  ({n} 格)", t.elapsed());
}

fn fmt(d: Data) -> String {
    match d {
        Data::Empty => String::new(),
        Data::String(s) => s,
        Data::Float(f) => {
            if f.is_finite() && f.fract() == 0.0 && f.abs() < 1e15 {
                format!("{}", f as i64)
            } else {
                format!("{f}")
            }
        }
        other => format!("{other:?}"),
    }
}

fn cell_str(d: &Data) -> Option<String> {
    Some(fmt(d.clone()))
}

/// 流式全量解码到 Vec<Vec<String>>：optimized=false 为旧写法（clone 两次），true 为直接按 DataRef 转换。
fn stream_all(wb: &mut Xlsx<std::io::BufReader<std::fs::File>>, name: &str, optimized: bool) {
    use calamine::DataRef;
    let t = Instant::now();
    let mut r = wb.worksheet_cells_reader(name).expect("cells reader");
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut cur: Vec<String> = Vec::new();
    let mut last_row = u32::MAX;
    while let Some(cell) = r.next_cell().expect("cell") {
        let (rr, c) = cell.get_position();
        if rr != last_row {
            if last_row != u32::MAX {
                if optimized {
                    let w = cur.len();
                    rows.push(std::mem::replace(&mut cur, Vec::with_capacity(w)));
                } else {
                    rows.push(cur.clone());
                    cur.clear();
                }
            }
            last_row = rr;
        }
        let v = cell.get_value();
        let text = if optimized {
            match v {
                DataRef::Empty => String::new(),
                DataRef::SharedString(s) => (*s).to_owned(),
                DataRef::String(s) => s.clone(),
                other => fmt(Data::from(other.clone())),
            }
        } else if matches!(v, DataRef::Empty) {
            String::new()
        } else {
            fmt(Data::from(v.clone()))
        };
        while cur.len() < c as usize {
            cur.push(String::new());
        }
        cur.push(text);
    }
    println!(
        "6) 流式全量→字符串 ({}): {:>10.1?}  ({} 行)",
        if optimized { "优化" } else { "旧" },
        t.elapsed(),
        rows.len()
    );
}

/// 只解压不解析：下限参考。
pub fn inflate_only(path: &str) {
    use std::io::Read;
    let t = Instant::now();
    let f = std::fs::File::open(path).unwrap();
    let mut z = zip::ZipArchive::new(std::io::BufReader::new(f)).unwrap();
    let mut e = z.by_name("xl/worksheets/sheet1.xml").unwrap();
    let mut buf = vec![0u8; 1 << 20];
    let mut n = 0usize;
    loop {
        let k = e.read(&mut buf).unwrap();
        if k == 0 { break; }
        n += k;
    }
    println!("0) 仅解压 sheet1.xml: {:>10.1?} ({} MB)", t.elapsed(), n >> 20);
}
