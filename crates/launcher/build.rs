//! Build steps for the launcher binary:
//!   1. Read `product.json` (repo root, the single source of product metadata) and re-export it as
//!      `LAUNCHER_*` compile-time env vars the app reads with `env!`.
//!   2. Windows targets: embed the icon, the version resource and the application manifest
//!      (PerMonitorV2, UTF-8 code page, asInvoker).

use std::env;
use std::path::Path;

struct Product {
    name: String,
    version: String,
    publisher: String,
    description: String,
    copyright: String,
}

fn main() {
    let product = read_product();
    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os == "windows" {
        embed_resources(&product);
    }
    export_env(&product);
}

fn read_product() -> Product {
    let manifest = env::var("CARGO_MANIFEST_DIR").unwrap();
    let path = Path::new(&manifest).join("../../product.json");
    println!("cargo:rerun-if-changed={}", path.display());
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));
    let json: serde_json::Value = serde_json::from_str(&raw)
        .unwrap_or_else(|e| panic!("{} is not valid JSON: {e}", path.display()));
    let field = |key: &str| -> String {
        json.get(key)
            .and_then(|v| v.as_str())
            .unwrap_or_else(|| panic!("product.json is missing the string field \"{key}\""))
            .to_string()
    };
    Product {
        name: field("productName"),
        version: field("version"),
        publisher: field("publisher"),
        description: field("description"),
        copyright: field("copyright"),
    }
}

/// The icon is resource id 1: Explorer shows it and the window picks it up at runtime.
#[cfg(windows)]
fn embed_resources(p: &Product) {
    let manifest = env::var("CARGO_MANIFEST_DIR").unwrap();
    let ico = Path::new(&manifest).join("../../assets/launcher.ico");
    let app_manifest = Path::new(&manifest).join("launcher.exe.manifest");
    println!("cargo:rerun-if-changed={}", ico.display());
    println!("cargo:rerun-if-changed={}", app_manifest.display());

    let mut res = winresource::WindowsResource::new();
    res.set_icon(ico.to_str().expect("icon path is valid UTF-8"));
    res.set_manifest_file(app_manifest.to_str().expect("manifest path is valid UTF-8"));
    res.set("ProductName", &p.name);
    res.set("FileDescription", &p.name);
    res.set("CompanyName", &p.publisher);
    res.set("LegalCopyright", &p.copyright);
    res.set("Comments", &p.description);
    res.set("OriginalFilename", "launcher.exe");
    res.set("InternalName", "launcher");
    res.set("FileVersion", &p.version);
    res.set("ProductVersion", &p.version);
    let packed = packed_version(&p.version);
    res.set_version_info(winresource::VersionInfo::FILEVERSION, packed);
    res.set_version_info(winresource::VersionInfo::PRODUCTVERSION, packed);
    res.compile().expect("failed to embed Windows resources");
}

/// A Windows target built from another host (not supported: resources need the Windows SDK).
#[cfg(not(windows))]
fn embed_resources(_p: &Product) {
    println!("cargo:warning=Windows resources are only embedded when building on Windows");
}

/// Pack "major.minor.patch[.build]" into VS_FIXEDFILEINFO's u64 layout, each field clamped to 16 bits.
#[cfg(windows)]
fn packed_version(version: &str) -> u64 {
    let mut fields = [0u64; 4];
    for (field, part) in fields.iter_mut().zip(version.split('.')) {
        *field = part.parse::<u64>().unwrap_or(0).min(0xFFFF);
    }
    (fields[0] << 48) | (fields[1] << 32) | (fields[2] << 16) | fields[3]
}

fn export_env(p: &Product) {
    println!("cargo:rustc-env=LAUNCHER_PRODUCT_NAME={}", p.name);
    println!("cargo:rustc-env=LAUNCHER_VERSION={}", p.version);
    println!("cargo:rustc-env=LAUNCHER_PUBLISHER={}", p.publisher);
    println!("cargo:rustc-env=LAUNCHER_COPYRIGHT={}", p.copyright);
    println!("cargo:rustc-env=LAUNCHER_DESCRIPTION={}", p.description);
}
