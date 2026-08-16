//! 编译汇编入口 boot.S/tramp.S/idt.S 与 C 日志模块(klog),链接进内核。

fn main() {
    let mut build = cc::Build::new();
    build
        .file("src/boot.S")
        .file("src/tramp.S")
        .file("src/idt.S")
        .target("x86_64-mcs.json")
        .flag("-m64");
    if std::env::var("CARGO_FEATURE_KLOG").is_ok() {
        build
            .file("src/klog.c")
            .flag("-Wall")
            .flag("-Wextra")
            .flag("-fno-stack-protector");
    } else {
        build.file("src/klog_off.c");
    }
    build.compile("mcs_boot");
    println!("cargo:rerun-if-changed=src/boot.S");
    println!("cargo:rerun-if-changed=src/tramp.S");
    println!("cargo:rerun-if-changed=src/idt.S");
    println!("cargo:rerun-if-changed=src/klog.c");
    println!("cargo:rerun-if-changed=src/klog.h");
    println!("cargo:rerun-if-changed=src/klog_off.c");
    println!("cargo:rustc-link-arg=-Tsrc/linker.ld");
    println!("cargo:rustc-link-arg=-nostdlib");
    println!("cargo:rustc-link-arg=-static");
    println!("cargo:rustc-link-arg=-no-pie");
}