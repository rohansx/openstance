//! Graph queries over one person: `who` / `get_person` (P2-CORE-04) and
//! `how_connected` (P2-CORE-05). Read-only; every result uses the shared
//! JSON shapes of `docs/tech-spec/interfaces.md` (1.7 and 1.8).
//!
//! Owned by the "graphq" stream (Phase 2). Calls into `resolve::lookup`
//! (person references), `search::person` and `search::evidence` (shape
//! builders); never writes.

pub mod how;
pub mod who;

pub use how::{how_connected, HowConnectedResponse, HowOptions, NotInGraph, NOT_IN_GRAPH_HINT};
pub use who::get_person;
