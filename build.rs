use std::path::Path;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-env-changed=DEVELOPER_DIR");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }

    // ort-sys rc.13 derives the compiler runtime path from `clang
    // --print-search-dirs`. On machines upgraded across Xcode CLT releases,
    // that output can point at a removed versioned directory. xcrun's resource
    // directory is the canonical active toolchain path, so add it as a second,
    // portable search location for clang_rt.osx.
    let Ok(output) = Command::new("xcrun")
        .args(["clang", "--print-resource-dir"])
        .output()
    else {
        return;
    };
    if !output.status.success() {
        return;
    }
    let resource_dir = String::from_utf8_lossy(&output.stdout);
    let runtime_dir = Path::new(resource_dir.trim()).join("lib").join("darwin");
    if runtime_dir.is_dir() {
        println!("cargo:rustc-link-search=native={}", runtime_dir.display());
    }
}
