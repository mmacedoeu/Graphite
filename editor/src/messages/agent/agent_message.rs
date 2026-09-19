use crate::messages::prelude::*;
use graphite_agent_protocol::{AgentOperation, DocumentOperation, QueryId, SnapshotProjection, ToolOutcome};

/// The editor-side agent boundary (INV-13).
///
/// Every variant carries a host-allocated [`QueryId`]; the editor never allocates
/// one (INV-14). The enum is curated: it is not `Message`, and nothing here
/// deserializes and dispatches an arbitrary `Message`.
#[impl_message(Message, Agent)]
#[derive(PartialEq, Clone, Debug, serde::Serialize, serde::Deserialize)]
pub enum AgentMessage {
	/// Lifecycle/persistence. Handled by the document subsystem where async export lives.
	Document {
		id: QueryId,
		operation: DocumentOperation,
	},
	/// Graph/history mutation. Executed by the agent handler via curated messages.
	Execute {
		id: QueryId,
		document: u64,
		operation: AgentOperation,
	},
	/// Typed read. `document: None` is valid only for `DocumentList` and `ActiveDocument`.
	Snapshot {
		id: QueryId,
		document: Option<u64>,
		projection: SnapshotProjection,
	},
	Cancel {
		id: QueryId,
	},
	/// Internal: a subsystem finished async work and reports the correlated result.
	Reply {
		id: QueryId,
		outcome: ToolOutcome,
	},
}
