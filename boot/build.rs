//! 编译汇编入口 boot.S,链接进内核。

fn main() {
    cc::Build::new()
        .file("src/boot.S")
        .file("src/tramp.S")
        .target("x86_64-mcs.json")
        .flag("-m64")
        .compile("mcs_boot");
    println!("cargo:rerun-if-changed=src/boot.S");
    println!("cargo:rerun-if-changed=src/tramp.S");
    println!("cargo:rustc-link-arg=-Tsrc/linker.ld");
    println!("cargo:rustc-link-arg=-nostdlib");
    println!("cargo:rustc-link-arg=-static");
    println!("cargo:rustc-link-arg=-no-pie");
}