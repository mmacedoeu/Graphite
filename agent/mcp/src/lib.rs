//! MCP transport adapter for the Graphite agentic MCP subsystem.
//!
//! Phase 2 implements the newline-delimited JSON-RPC stdio transport (T2.9); Phase 4
//! adds the Streamable HTTP transport over `tiny_http` (T4.8). The adapter depends
//! only on `agent/protocol` + `agent/host` (INV-4) and never touches the `editor`
//! crate.

pub mod http;
pub mod stdio;

pub use http::{HttpConfig, HttpServer, resolve_addr};
pub use stdio::{JsonRpcId, serve_stdio};
