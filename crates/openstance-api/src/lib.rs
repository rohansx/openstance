//! Local HTTP API for openstance (Phase 4). Routes in
//! `docs/tech-spec/interfaces.md` section 3 map onto `openstance-core` calls
//! and return the same JSON as the CLI. Empty in Phase 1.
#![warn(missing_docs)]

/// The core crate this API wraps.
pub use openstance_core as core;
/// The MCP server mounted at `/mcp` (Phase 4).
pub use openstance_mcp as mcp;
