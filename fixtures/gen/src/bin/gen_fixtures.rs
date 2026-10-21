//! Regenerate the committed Phase 1 fixtures under `fixtures/`.
//!
//! `cargo run -p openstance-fixtures --bin gen-fixtures [-- <fixtures dir>]`

use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let root = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".."));
    match openstance_fixtures::generate(&root) {
        Ok(()) => {
            let e = openstance_fixtures::expected();
            eprintln!(
                "wrote fixtures to {}: {} persons ({} + self), {} flagged, {} suggestions, {} bulk, {} identities",
                root.display(),
                e.persons,
                e.persons_excluding_self,
                e.flagged,
                e.suggestions,
                e.bulk,
                e.identities
            );
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("error: writing fixtures to {}: {err}", root.display());
            ExitCode::FAILURE
        }
    }
}
