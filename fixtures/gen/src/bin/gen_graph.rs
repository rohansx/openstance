//! Generate the synthetic performance graph (not committed).
//!
//! `cargo run --release -p openstance-fixtures --bin gen-graph -- --people 20000
//! --interactions 500000 --seed 42 --out target/synth/heavy`

use std::path::PathBuf;
use std::process::ExitCode;

use openstance_fixtures::graph::{write_graph, GraphParams};

fn main() -> ExitCode {
    let mut p = GraphParams::default();
    let mut out = PathBuf::from("target/synth/heavy");
    let mut args = std::env::args().skip(1);
    while let Some(flag) = args.next() {
        let value = args.next();
        let parsed = match (flag.as_str(), value.as_deref()) {
            ("--people", Some(v)) => v.parse().map(|n| p.people = n).is_ok(),
            ("--interactions", Some(v)) => v.parse().map(|n| p.interactions = n).is_ok(),
            ("--seed", Some(v)) => v.parse().map(|n| p.seed = n).is_ok(),
            ("--out", Some(v)) => {
                out = PathBuf::from(v);
                true
            }
            _ => false,
        };
        if !parsed {
            eprintln!("usage: gen-graph [--people N] [--interactions N] [--seed N] [--out DIR]");
            return ExitCode::from(2);
        }
    }
    match write_graph(&p, &out) {
        Ok(n) => {
            eprintln!(
                "wrote {n} messages for {} people to {}",
                p.people,
                out.display()
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
