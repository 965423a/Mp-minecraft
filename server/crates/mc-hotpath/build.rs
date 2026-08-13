//! 编译 C 热路径(server/native),链接进 crate。

fn main() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("native");
    cc::Build::new()
        .files([root.join("varint.c"), root.join("bitpack.c")])
        .warnings(true)
        .opt_level(2)
        .compile("mcs_native");
    println!("cargo:rerun-if-changed={}", root.join("varint.c").display());
    println!("cargo:rerun-if-changed={}", root.join("bitpack.c").display());
}