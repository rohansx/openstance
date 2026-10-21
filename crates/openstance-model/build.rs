//! macOS link fix for the `fastembed` feature.
//!
//! The ONNX Runtime prebuilt that `ort` downloads calls `@available(...)`,
//! which compiles to `___isPlatformVersionAtLeast`. That symbol lives in
//! clang's compiler-rt (`libclang_rt.osx.a`), which rustc does not link
//! (it passes `-nodefaultlibs`). Debug builds happen to link, but release
//! builds with fat LTO fail with "Undefined symbols ...
//! ___isPlatformVersionAtLeast". Adding compiler-rt as a non-bundled static
//! library lets the linker take just the missing members from it.

use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=CC");
    let fastembed = std::env::var_os("CARGO_FEATURE_FASTEMBED").is_some();
    let macos = std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos");
    if !(fastembed && macos) {
        return;
    }
    if let Some(dir) = compiler_rt_dir() {
        println!("cargo:rustc-link-search=native={}", dir.display());
        println!("cargo:rustc-link-lib=static:-bundle=clang_rt.osx");
    } else {
        println!(
            "cargo:warning=libclang_rt.osx.a not found; release builds with fastembed may fail to link"
        );
    }
}

/// The directory holding `libclang_rt.osx.a`, asked of the C compiler.
fn compiler_rt_dir() -> Option<PathBuf> {
    let cc = std::env::var("CC").unwrap_or_else(|_| "cc".to_owned());
    let out = Command::new(cc)
        .arg("-print-file-name=libclang_rt.osx.a")
        .output()
        .ok()?;
    let path = PathBuf::from(String::from_utf8(out.stdout).ok()?.trim());
    // An unknown file comes back as the bare name.
    if !path.is_absolute() || !path.is_file() {
        return None;
    }
    path.parent().map(PathBuf::from)
}
