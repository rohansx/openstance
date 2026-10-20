//! MCP server for openstance (Phase 2). Maps the tool schemas in
//! `docs/tech-spec/interfaces.md` section 2 onto `openstance-core` calls.
//! Empty in Phase 1; it exists so the workspace layout and dependency
//! direction (`openstance-mcp -> openstance-core`, never `-> openstance-api`)
//! are in place from the start.
#![warn(missing_docs)]

/// The core crate this server wraps (re-exported for the binary's wiring).
pub use openstance_core as core;
