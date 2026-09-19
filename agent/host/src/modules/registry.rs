//! Phase 5 registry tools (T5.3/T5.4).
//!
//! `registry.apply_delta`, `registry.query`, `registry.merge`, and `history.replay` are the
//! curated Surface C operations. They speak only the typed [`PeerState`] interface
//! (`agent/host/src/peer.rs`) shared with `PeerBridge`; in every other mode the module is
//! constructed without a peer handle and every call is refused.
//!
//! Every accepted `apply_delta`/`merge` runs the semantic validation pass (INV-9): on failure
//! the tool returns [`ToolError::InvalidGraph`] and the session is rolled back by `PeerState`.

use super::{arguments, descriptor, optional_u64, schema_object};
use crate::peer::{PeerHandle, lock_peer};
use document_graph_storage::{Delta, RegistryDelta};
use graphite_agent_protocol::{Capability, EditorBridge, ToolCall, ToolDescriptor, ToolError, ToolModule};
use serde_json::{Value, json};
use std::future::Future;
use std::pin::Pin;

/// The four Phase 5 tools, all served from the host-owned peer handle.
pub struct RegistryModule {
	peer: Option<PeerHandle>,
}

impl RegistryModule {
	pub fn new(peer: Option<PeerHandle>) -> Self {
		Self { peer }
	}

	fn peer(&self) -> Result<&PeerHandle, ToolError> {
		self.peer.as_ref().ok_or_else(|| ToolError::InvalidArguments {
			message: "the registry tools require --mode peer (Surface C)".to_string(),
		})
	}
}

/// Result shapes shared by the delta-writing tools.
fn delta_result_schema(required: &[&str]) -> Value {
	schema_object(required, json!({ "rev": { "type": ["string", "null"] }, "head": { "type": ["string", "null"] } }))
}

fn parse_delta(call: &ToolCall) -> Result<RegistryDelta, ToolError> {
	let value = arguments(call).get("delta").ok_or_else(|| ToolError::InvalidArguments {
		message: "missing argument `delta`".to_string(),
	})?;
	serde_json::from_value(value.clone()).map_err(|error| ToolError::InvalidArguments {
		message: format!("`delta` is not a valid RegistryDelta: {error}"),
	})
}

/// Deltas are exchanged RON-encoded (like the Phase 4 attached framing, T4.1): `Delta` carries
/// a `Rev` (`NonZeroU128`), which JSON's number type cannot represent.
fn parse_deltas(call: &ToolCall) -> Result<Vec<Delta>, ToolError> {
	let encoded = arguments(call).get("deltas").and_then(Value::as_str).ok_or_else(|| ToolError::InvalidArguments {
		message: "missing or non-string argument `deltas` (expected RON-encoded Vec<Delta>)".to_string(),
	})?;
	ron::de::from_str::<Vec<Delta>>(encoded).map_err(|error| ToolError::InvalidArguments {
		message: format!("`deltas` is not a valid RON-encoded Vec<Delta>: {error}"),
	})
}

impl ToolModule for RegistryModule {
	fn descriptors(&self) -> Vec<ToolDescriptor> {
		vec![
			descriptor(
				"registry.apply_delta",
				"Apply one agent-authored CRDT delta to the peer document, then validate the resulting graph.",
				Capability::Author,
				schema_object(
					&["delta"],
					json!({ "delta": { "type": "object", "description": "A JSON-encoded document_graph_storage::RegistryDelta." } }),
				),
				delta_result_schema(&["applied", "rev", "head"]),
			),
			descriptor(
				"registry.query",
				"Read the merged CRDT registry: nodes, networks, peer attributions, and document attributes.",
				Capability::Read,
				schema_object(&[], json!({ "node_id": { "type": "integer", "minimum": 0 } })),
				schema_object(&["node_count", "nodes", "networks", "peer_users", "attributes"], json!({})),
			),
			descriptor(
				"registry.merge",
				"Integrate retired deltas from another peer (RON-encoded Vec<Delta>), then validate the resulting graph.",
				Capability::Author,
				schema_object(
					&["deltas"],
					json!({ "deltas": { "type": "string", "description": "A RON-encoded Vec<document_graph_storage::Delta>." } }),
				),
				delta_result_schema(&["merged", "rev", "head"]),
			),
			descriptor(
				"history.replay",
				"Reconstruct the registry from history and assert it equals the live registry by value.",
				Capability::Read,
				schema_object(&[], json!({})),
				schema_object(&["equal", "node_count", "network_count", "history_length"], json!({})),
			),
		]
	}

	fn execute<'a>(&'a mut self, call: ToolCall, _bridge: &'a mut dyn EditorBridge) -> Pin<Box<dyn Future<Output = Result<Value, ToolError>> + 'a>> {
		Box::pin(async move {
			let handle = self.peer()?.clone();
			let mut state = lock_peer(&handle);
			match call.name.as_str() {
				"registry.apply_delta" => state.apply_delta(parse_delta(&call)?),
				"registry.query" => state.query(optional_u64(&call, "node_id")?),
				"registry.merge" => state.merge(parse_deltas(&call)?),
				"history.replay" => state.replay_check(),
				other => Err(ToolError::NotFound { what: format!("tool {other}") }),
			}
		})
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::peer::PeerBridge;
	use document_graph_storage::PeerId;
	use graphite_agent_protocol::{AgentEvent, BridgeQuery, QueryId};
	use std::path::{Path, PathBuf};

	/// A bridge is required by the `ToolModule` signature; the registry module never uses it.
	struct NullBridge;

	impl EditorBridge for NullBridge {
		fn submit(&mut self, _id: QueryId, _query: BridgeQuery) -> Result<(), ToolError> {
			Ok(())
		}
		fn poll(&mut self, _id: QueryId) -> Option<Result<Value, ToolError>> {
			None
		}
		fn cancel(&mut self, _id: QueryId) {}
		fn drain_events(&mut self) -> Vec<AgentEvent> {
			Vec::new()
		}
		fn pump(&mut self) -> Pin<Box<dyn Future<Output = Result<(), ToolError>> + '_>> {
			Box::pin(async { Ok(()) })
		}
	}

	fn temp_dir(label: &str) -> PathBuf {
		let dir = std::env::temp_dir().join(format!("graphite-agent-registry-{}-{label}", std::process::id()));
		let _ = std::fs::remove_dir_all(&dir);
		std::fs::create_dir_all(&dir).expect("create temp dir");
		dir
	}

	fn write_minimal_gdd(dir: &Path) {
		let manifest = json!({
			"format": "gdd",
			"format_version": 1,
			"editor_version": "test",
			"stdlib_version": "test",
			"document_id": 1,
		});
		std::fs::write(dir.join("manifest.json"), serde_json::to_vec_pretty(&manifest).expect("manifest json")).expect("write manifest");
	}

	fn call(name: &str, arguments: Value) -> ToolCall {
		ToolCall {
			id: 1,
			name: name.to_string(),
			arguments,
			capability: Capability::Author,
			document: None,
		}
	}

	fn block_on<T>(future: impl Future<Output = T>) -> T {
		futures::executor::block_on(future)
	}

	#[test]
	fn registry_tools_are_refused_outside_peer_mode() {
		let mut module = RegistryModule::new(None);
		let mut bridge = NullBridge;
		let outcome = block_on(module.execute(call("registry.query", json!({})), &mut bridge));
		assert!(matches!(outcome, Err(ToolError::InvalidArguments { .. })), "expected InvalidArguments, got {outcome:?}");
	}

	#[test]
	fn descriptor_set_is_the_four_phase_five_tools() {
		let module = RegistryModule::new(None);
		let names: Vec<String> = module.descriptors().into_iter().map(|descriptor| descriptor.name).collect();
		assert_eq!(
			names,
			vec![
				"registry.apply_delta".to_string(),
				"registry.query".to_string(),
				"registry.merge".to_string(),
				"history.replay".to_string(),
			]
		);
	}

	#[test]
	fn apply_query_and_replay_round_trip_through_the_tools() {
		let dir = temp_dir("round-trip");
		write_minimal_gdd(&dir);
		let bridge = block_on(PeerBridge::open_with_peer(&dir, PeerId(41))).expect("open bridge");
		let mut module = RegistryModule::new(Some(bridge.handle()));
		let mut null = NullBridge;

		let applied = block_on(module.execute(
			call(
				"registry.apply_delta",
				json!({ "delta": { "AddNetwork": { "id": 0, "network": { "exports": [], "attributes": {} } } } }),
			),
			&mut null,
		))
		.expect("apply_delta");
		assert_eq!(applied["applied"], json!(true));

		let queried = block_on(module.execute(call("registry.query", json!({})), &mut null)).expect("query");
		assert_eq!(queried["networks"][0]["network_id"], json!(0));
		assert_eq!(queried["peer_id"], json!(41));

		let replay = block_on(module.execute(call("history.replay", json!({})), &mut null)).expect("replay");
		assert_eq!(replay["equal"], json!(true));
	}

	#[test]
	fn invalid_graph_is_rejected_by_the_tool_and_leaves_the_session_unchanged() {
		let dir = temp_dir("tool-invalid");
		write_minimal_gdd(&dir);
		let bridge = block_on(PeerBridge::open_with_peer(&dir, PeerId(42))).expect("open bridge");
		let handle = bridge.handle();
		let mut module = RegistryModule::new(Some(handle.clone()));
		let mut null = NullBridge;

		block_on(module.execute(
			call(
				"registry.apply_delta",
				json!({ "delta": { "AddNetwork": { "id": 0, "network": { "exports": [], "attributes": {} } } } }),
			),
			&mut null,
		))
		.expect("valid apply");

		let before = lock_peer(&handle).head_rev();
		let outcome = block_on(module.execute(
			call(
				"registry.apply_delta",
				json!({ "delta": { "AddNode": { "id": 1, "node": { "implementation": { "ProtoNode": 12345 }, "inputs": [], "attributes": {}, "network": 0 } } } }),
			),
			&mut null,
		));
		assert!(matches!(outcome, Err(ToolError::InvalidGraph { .. })), "expected InvalidGraph, got {outcome:?}");
		assert_eq!(lock_peer(&handle).head_rev(), before);
		assert_eq!(lock_peer(&handle).registry().node_instances.len(), 0);
	}
}
