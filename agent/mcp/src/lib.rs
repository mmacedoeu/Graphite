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

/// The MCP `initialize` result's `instructions` field (E-18).
///
/// Hosts surface this as server-wide guidance: Codex reads it at initialization,
/// and its documentation asks that the first 512 characters stand on their own.
/// This string is therefore front-loaded with the whole workflow, which removes
/// most of the need for an external setup guide.
pub const SERVER_INSTRUCTIONS: &str = "The Graphite MCP server edits Graphite node graphs. \
document.new or document.open returns a document_id. \
Discover nodes with node.list_types and node.describe, \
then build the graph with graph.add_node, graph.set_input, and graph.connect. \
Wrap each logical edit in one history.begin/history.commit pair so a human can undo it atomically. \
Verify with render.preview. Every path must stay inside the server's --root.";

#[cfg(test)]
mod tests {
	use super::*;

	/// Codex's documented requirement: the leading guidance must be self-contained.
	#[test]
	fn instructions_front_load_the_workflow() {
		let head: String = SERVER_INSTRUCTIONS.chars().take(512).collect();
		for expected in ["document.new", "node.list_types", "graph.connect", "history.commit", "render.preview", "--root"] {
			assert!(head.contains(expected), "the first 512 characters must mention {expected}");
		}
		assert!(SERVER_INSTRUCTIONS.chars().count() < 1024, "instructions must stay short");
	}
}
