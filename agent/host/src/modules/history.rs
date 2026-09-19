//! History tools (T2.6). Every mutation the agent makes is wrapped in an editor
//! transaction by the handler (INV-5); these tools let the agent open one
//! explicitly and undo/redo.

use super::{descriptor, query, required_u64, schema_object};
use graphite_agent_protocol::{AgentOperation, BridgeQuery, Capability, EditorBridge, ToolCall, ToolDescriptor, ToolError, ToolModule};
use serde_json::Value;
use std::future::Future;
use std::pin::Pin;

#[derive(Debug, Default)]
pub struct HistoryModule;

impl HistoryModule {
	fn operation_for(name: &str) -> Option<AgentOperation> {
		match name {
			"history.undo" => Some(AgentOperation::Undo),
			"history.redo" => Some(AgentOperation::Redo),
			"history.begin" => Some(AgentOperation::BeginTransaction),
			"history.commit" => Some(AgentOperation::CommitTransaction),
			"history.abort" => Some(AgentOperation::AbortTransaction),
			_ => None,
		}
	}
}

impl ToolModule for HistoryModule {
	fn descriptors(&self) -> Vec<ToolDescriptor> {
		[
			("history.undo", "Undo the most recent change."),
			("history.redo", "Redo the most recently undone change."),
			("history.begin", "Begin a transaction that groups subsequent mutations."),
			("history.commit", "Commit the open transaction."),
			("history.abort", "Abort the open transaction."),
		]
		.into_iter()
		.map(|(name, description)| {
			descriptor(
				name,
				description,
				Capability::Author,
				schema_object(&["document_id"], serde_json::json!({ "document_id": { "type": "integer", "minimum": 0 } })),
				super::ok_object(),
			)
		})
		.collect()
	}

	fn execute<'a>(&'a mut self, call: ToolCall, bridge: &'a mut dyn EditorBridge) -> Pin<Box<dyn Future<Output = Result<Value, ToolError>> + 'a>> {
		Box::pin(async move {
			let Some(operation) = Self::operation_for(&call.name) else {
				return Err(ToolError::NotFound { what: format!("tool {}", call.name) });
			};
			let document = required_u64(&call, "document_id")?;
			query(bridge, call.id, BridgeQuery::Operation { document, operation }).await
		})
	}
}
