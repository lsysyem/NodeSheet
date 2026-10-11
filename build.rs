use std::path::PathBuf;

fn main() {
    // fluent 样式随 Palette.color-scheme 切换深浅；自绘部分走 ui 里的 Theme 令牌
    let config = slint_build::CompilerConfiguration::new().with_style("fluent".into());
    slint_build::compile_with_config("ui/app-window.slint", config).unwrap();

    // 内置节点：nodes/*.py 打进程序（src/nodes.rs 里 include 这个生成的表）
    println!("cargo:rerun-if-changed=nodes");
    let dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("nodes");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir).expect("缺少 nodes 目录").flatten().map(|e| e.path()).collect();
    files.retain(|p| p.extension().is_some_and(|e| e == "py"));
    files.sort();
    let mut table = String::from("static BUILTIN: &[(&str, &str)] = &[\n");
    for file in &files {
        let name = file.file_stem().unwrap().to_string_lossy();
        table.push_str(&format!("    ({name:?}, include_str!({:?})),\n", file.to_string_lossy()));
    }
    table.push_str("];\n");
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("builtin_nodes.rs");
    std::fs::write(out, table).unwrap();

    // exe 的图标（资源管理器、任务栏）；窗口左上角的图标在 app-window.slint 里设
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rerun-if-changed=assets/app.rc");
        println!("cargo:rerun-if-changed=assets/icon.ico");
        embed_resource::compile("assets/app.rc", embed_resource::NONE).manifest_optional().unwrap();
    }
}
