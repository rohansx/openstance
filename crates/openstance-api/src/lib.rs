//! Local HTTP API for openstance (Phase 4). Routes in
//! `docs/tech-spec/interfaces.md` section 3 map onto `openstance-core` calls
//! and return the same JSON as the CLI's `--json` and the MCP tools.
#![warn(missing_docs)]

/// The core crate this API wraps.
pub use openstance_core as core;
/// The MCP server mounted at `/mcp`.
pub use openstance_mcp as mcp;

pub mod error;
pub mod pool;
pub mod token;
