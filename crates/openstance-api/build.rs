//! Detect a built web UI. `ui/dist/index.html` present at build time sets
//! `cfg(ui_dist)`, and the `ui` feature then embeds the directory. Without
//! it (CI without node, `cargo install` from a fresh checkout) the build
//! still succeeds and `/` serves a placeholder telling the user to run
//! `npm run build` in `ui/`.

use std::path::Path;

fn main() {
    println!("cargo::rustc-check-cfg=cfg(ui_dist)");
    let dist = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ui/dist");
    println!("cargo::rerun-if-changed={}", dist.display());
    println!(
        "cargo::rerun-if-changed={}",
        dist.join("index.html").display()
    );
    if dist.join("index.html").is_file() {
        println!("cargo::rustc-cfg=ui_dist");
    }
}
