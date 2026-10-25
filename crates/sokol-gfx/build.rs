//! Compile sokol_gfx + sokol_log from the vendored sources: D3D11 on Windows (MSVC), Metal on
//! macOS (as Objective-C, without ARC: callers wrap frames in an autorelease pool).
//!
//! When cross-checking from another OS (`cargo check --target x86_64-pc-windows-msvc` on a Mac),
//! the C step is skipped: there is no MSVC or Windows SDK to compile against, and `check` doesn't
//! link.

use std::env;

fn main() {
    let c_dir = "../../vendor/sokol-rust/src/sokol/c";
    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap();
    let host = env::var("HOST").unwrap();
    let target = env::var("TARGET").unwrap();
    for f in [
        "sokol_gfx.c",
        "sokol_log.c",
        "sokol_gfx.h",
        "sokol_log.h",
        "sokol_defines.h",
    ] {
        println!("cargo:rerun-if-changed={c_dir}/{f}");
    }
    let host_os = if host.contains("windows") {
        "windows"
    } else if host.contains("apple") {
        "macos"
    } else {
        "other"
    };
    if host != target && host_os != target_os {
        println!("cargo:warning=cross-check for {target}: skipping the sokol C build");
        return;
    }

    let mut build = cc::Build::new();
    build
        .include(c_dir)
        .file(format!("{c_dir}/sokol_gfx.c"))
        .file(format!("{c_dir}/sokol_log.c"))
        .define("IMPL", None)
        .flag_if_supported("-Wno-unused-parameter")
        .flag_if_supported("-Wno-missing-field-initializers")
        .warnings(false);
    if env::var("PROFILE").as_deref() == Ok("release") {
        build.define("NDEBUG", None).opt_level(2);
    }
    match target_os.as_str() {
        "windows" => {
            build.define("SOKOL_D3D11", None);
        }
        "macos" => {
            build
                .define("SOKOL_METAL", None)
                .flag("-x")
                .flag("objective-c");
            println!("cargo:rustc-link-lib=framework=Metal");
            println!("cargo:rustc-link-lib=framework=QuartzCore");
            println!("cargo:rustc-link-lib=framework=Foundation");
        }
        other => panic!("sokol-gfx: unsupported target OS {other}"),
    }
    build.compile("sokol_gfx");
}
