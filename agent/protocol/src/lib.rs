//! Transport-agnostic agent contract. No MCP, no editor dependencies.

use futures::Stream;
use serde::{Deserialize, Serialize};
use std::future::Future;
use std::pin::Pin;

/// Monotonic, host-assigned call identity (INV-14). The unit of correlation.
pub type QueryId = u64;

/// Document identity as exposed to agents: the inner `u64` of the editor's
/// `DocumentId(pub u64)`. Converted at the host boundary; never passed raw.
pub type AgentDocumentId = u64;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Capability {
	Read,
	Author,
	Execute,
	Export,
	Persist,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CapabilitySet(pub Vec<Capability>);

impl CapabilitySet {
	pub fn grants(&self, required: Capability) -> bool {
		self.0.contains(&required)
	}
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolDescriptor {
	pub name: String,
	pub description: String,
	pub capability: Capability,
	pub input_schema: serde_json::Value, // JSON Schema draft 2020-12
	pub output_schema: serde_json::Value,
	pub version: u32,
}

/// Adapter-facing request. Carries no id and no capability (INV-6, INV-14).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolRequest {
	pub name: String,
	pub arguments: serde_json::Value,
	pub document: Option<AgentDocumentId>,
}

/// Module-facing call. Constructed only by the host, which assigns id + capability.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
	pub id: QueryId,
	pub name: String,
	pub arguments: serde_json::Value,
	pub capability: Capability,
	pub document: Option<AgentDocumentId>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ToolOutcome {
	Ok { id: QueryId, result: serde_json::Value },
	Err { id: QueryId, error: ToolError },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum ToolError {
	InvalidArguments { message: String },
	Unauthorized { capability: Capability },
	NotFound { what: String },
	Timeout { id: QueryId },
	Cancelled { id: QueryId },
	InvalidGraph { message: String },
	PathOutsideRoot { path: String },
	Internal { message: String },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum AgentEvent {
	Progress { id: QueryId, fraction: f32, message: String },
	DocumentChanged { document: AgentDocumentId },
	Posted { id: QueryId, message: String },
}

/// An accepted, in-flight call. Owned and `'static`, so `cancel` remains
/// reachable while it is outstanding (HIGH-A4 / B-M9).
///
/// NOTE: the future is deliberately NOT `+ Send`. `editor::node_graph_executor::run_node_graph()`
/// holds a `std::sync::MutexGuard` across an `.await`, so it is `!Send` (R9-H1).
/// The host and the MCP adapter must therefore run on ONE dedicated thread with a
/// current-thread runtime (§5.5).
pub struct PendingCall {
	pub id: QueryId,
	pub outcome: Pin<Box<dyn Future<Output = ToolOutcome> + 'static>>,
}

/// The tool-execution boundary. Implementations are `Send + Sync` and use
/// interior mutability (`tokio::sync::Mutex<HostInner>`) so that `call` and
/// `cancel` can be invoked concurrently.
pub trait ToolHost: Send + Sync {
	fn descriptors(&self) -> Vec<ToolDescriptor>;
	fn call(&self, request: ToolRequest) -> PendingCall;
	fn cancel(&self, id: QueryId) -> bool;
	/// Owned stream. Implementations must use `futures::channel::mpsc` (whose
	/// `UnboundedReceiver` implements `Stream`), NOT a tokio broadcast receiver
	/// (which is not a `futures::Stream`; `tokio-stream` is not approved) — R8-H1.
	fn events(&self) -> Pin<Box<dyn Stream<Item = AgentEvent> + Send>>;
}

pub trait ToolModule: Send {
	fn descriptors(&self) -> Vec<ToolDescriptor>;
	/// `'a` must be shared by `self` and `bridge`: the returned future captures
	/// both. With elided lifetimes, `'_` binds to `&mut self` only and no
	/// conforming impl can compile (R6-C1). Do not add `+ Send` (R9-H1).
	fn execute<'a>(&'a mut self, call: ToolCall, bridge: &'a mut dyn EditorBridge) -> Pin<Box<dyn Future<Output = Result<serde_json::Value, ToolError>> + 'a>>;
}

/// The typed query/response boundary over the editor. Implementations submit
/// curated queries and await correlated replies. They must NOT (INV-2, INV-13):
///   - read message-handler state directly,
///   - deserialize and dispatch an arbitrary `Message`.
pub trait EditorBridge: Send {
	/// Submit under a host-allocated id (INV-14).
	fn submit(&mut self, id: QueryId, query: BridgeQuery) -> Result<(), ToolError>;
	fn poll(&mut self, id: QueryId) -> Option<Result<serde_json::Value, ToolError>>;
	fn cancel(&mut self, id: QueryId);
	fn drain_events(&mut self) -> Vec<AgentEvent>;
	/// Advance async editor work. MUST be async (CRIT-A1): the only way to drive
	/// node-graph execution is `run_node_graph().await`. Do not add `+ Send`:
	/// `run_node_graph()` holds a `std::sync::MutexGuard` across `.await` and is
	/// therefore `!Send` (R9-H1).
	fn pump(&mut self) -> Pin<Box<dyn Future<Output = Result<(), ToolError>> + '_>>;
}

/// A curated query (INV-13). `Snapshot.document` is `None` only for the
/// document-independent projections: `DocumentList` and `ActiveDocument`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum BridgeQuery {
	Document { operation: DocumentOperation },
	Operation { document: AgentDocumentId, operation: AgentOperation },
	Snapshot { document: Option<AgentDocumentId>, projection: SnapshotProjection },
}

/// Document lifecycle + persistence (HIGH-A2). Export is async on the editor side (INV-15).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum DocumentOperation {
	New {
		name: String,
	},
	/// `path` MUST already be canonicalized and prefix-checked by the host
	/// against its configured root (INV-12). The editor reads the file itself;
	/// the host does not read bytes for open (the bytes never leave the process,
	/// so no bytes variant is needed). Result: `{ "document_id": <id> }`.
	Open {
		path: String,
	},
	Close {
		document: AgentDocumentId,
	},
	/// Export the document to `.gdd` bytes. Result: `{ "gdd_base64": "..." }`.
	ExportGdd {
		document: AgentDocumentId,
	},
}

/// The ENTIRE graph mutation surface exposed to agents.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum AgentOperation {
	AddNode {
		identifier: String,
		x: f64,
		y: f64,
	},
	RemoveNode {
		node_id: u64,
	},
	/// `value_json` is a JSON-encoded `graph_craft::document::value::TaggedValue`.
	SetInput {
		node_id: u64,
		input_index: u32,
		value_json: String,
	},
	Connect {
		from_node: u64,
		from_output: u32,
		to_node: u64,
		to_input: u32,
	},
	Disconnect {
		to_node: u64,
		to_input: u32,
	},
	BeginTransaction,
	CommitTransaction,
	AbortTransaction,
	Undo,
	Redo,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum SnapshotProjection {
	DocumentList,
	DocumentSummary,
	NodeList,
	Node {
		node_id: u64,
	},
	ActiveDocument,
	/// Currently selected layer/node ids in the document (R7-H1).
	Selection,
}

#[cfg(test)]
mod tests {
	use super::*;

	fn round_trip<T>(value: &T)
	where
		T: Serialize + for<'de> Deserialize<'de> + PartialEq + std::fmt::Debug,
	{
		let json = serde_json::to_string(value).expect("serialize");
		let back: T = serde_json::from_str(&json).expect("deserialize");
		assert_eq!(*value, back, "round trip changed the value");
	}

	#[test]
	fn tool_request_round_trip() {
		round_trip(&ToolRequest {
			name: "graph.add_node".to_string(),
			arguments: serde_json::json!({ "identifier": "graphene_core::raster::OpacityNode" }),
			document: Some(7),
		});
	}

	#[test]
	fn tool_call_round_trip() {
		round_trip(&ToolCall {
			id: 42,
			name: "graph.connect".to_string(),
			arguments: serde_json::json!({ "from_node": 1, "to_node": 2 }),
			capability: Capability::Author,
			document: None,
		});
	}

	#[test]
	fn tool_outcome_round_trip() {
		round_trip(&ToolOutcome::Ok {
			id: 1,
			result: serde_json::json!({ "node_id": 5 }),
		});
		round_trip(&ToolOutcome::Err {
			id: 2,
			error: ToolError::PathOutsideRoot { path: "../escape.png".to_string() },
		});
	}

	#[test]
	fn agent_operation_round_trip() {
		round_trip(&AgentOperation::AddNode {
			identifier: "graphene_core::raster::OpacityNode".to_string(),
			x: 1.5,
			y: -2.5,
		});
		round_trip(&AgentOperation::SetInput {
			node_id: 3,
			input_index: 1,
			value_json: "{\"Value\":1.0}".to_string(),
		});
		round_trip(&AgentOperation::Undo);
	}

	#[test]
	fn document_operation_round_trip() {
		round_trip(&DocumentOperation::New { name: "demo".to_string() });
		round_trip(&DocumentOperation::Open { path: "/tmp/demo.gdd".to_string() });
		round_trip(&DocumentOperation::ExportGdd { document: 9 });
	}

	#[test]
	fn bridge_query_round_trip() {
		round_trip(&BridgeQuery::Document {
			operation: DocumentOperation::Close { document: 4 },
		});
		round_trip(&BridgeQuery::Operation {
			document: 4,
			operation: AgentOperation::RemoveNode { node_id: 8 },
		});
		round_trip(&BridgeQuery::Snapshot {
			document: None,
			projection: SnapshotProjection::DocumentList,
		});
		round_trip(&BridgeQuery::Snapshot {
			document: Some(4),
			projection: SnapshotProjection::Node { node_id: 8 },
		});
	}

	#[test]
	fn capability_set_grants() {
		let set = CapabilitySet(vec![Capability::Read, Capability::Author]);
		assert!(set.grants(Capability::Read));
		assert!(set.grants(Capability::Author));
		assert!(!set.grants(Capability::Export));
		assert!(!CapabilitySet::default().grants(Capability::Read));
	}
}
