//! The committed fixtures must equal what the generator produces.

use std::path::PathBuf;

#[test]
fn committed_fixtures_match_the_generator() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
    for (rel, want) in openstance_fixtures::files() {
        let got = std::fs::read_to_string(root.join(rel))
            .unwrap_or_else(|e| panic!("fixtures/{rel} missing ({e}); run gen-fixtures"));
        assert!(got == want, "fixtures/{rel} drifted from the generator; run `cargo run -p openstance-fixtures --bin gen-fixtures`");
    }
}
