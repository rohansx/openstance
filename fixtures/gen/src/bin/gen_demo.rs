//! Regenerate the committed demo dataset under `fixtures/demo/` (P2-DOC-01).
//!
//! `cargo run -p openstance-fixtures --bin gen-demo [-- <out dir>]`

use std::path::PathBuf;
use std::process::ExitCode;

use openstance_fixtures::demo;

fn main() -> ExitCode {
    let dir = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../demo"));
    match demo::generate(&dir) {
        Ok(()) => {
            let people = demo::people();
            let mails = demo::mails(&people);
            eprintln!(
                "wrote {} ({} people, {} messages) to {}",
                demo::EXPORTS.join(", "),
                people.len(),
                mails.len(),
                dir.display()
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error: writing the demo dataset to {}: {e}", dir.display());
            ExitCode::FAILURE
        }
    }
}
