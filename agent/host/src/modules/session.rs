//! Live-session projection tools (T4.5).
//!
//! `session.snapshot`, `session.selection`, and `session.active_document` expose
//! the typed [`SnapshotProjection`] reads over whichever bridge the host owns. In
//! attached mode this is how an agent reads state a human changed, without ever
//! touching message-handler internals (INV-2).

use super::{descriptor, optional_u64, query, required_string, required_u64, schema_object};
use graphite_agent_protocol::{BridgeQuery, Capability, EditorBridge, SnapshotProjection, ToolCall, ToolDescriptor, ToolError, ToolModule};
use serde_json::{Value, json};
use std::future::Future;
use std::pin::Pin;

#[derive(Debug, Default)]
pub struct SessionModule;

/// Map the wire name of a projection (as used by `session.snapshot`) onto the
/// contract enum. `node` additionally requires `node_id`.
fn projection(name: &str, call: &ToolCall) -> Result<SnapshotProjection, ToolError> {
	match name {
		"document_list" => Ok(SnapshotProjection::DocumentList),
		"active_document" => Ok(SnapshotProjection::ActiveDocument),
		"document_summary" => Ok(SnapshotProjection::DocumentSummary),
		"node_list" => Ok(SnapshotProjection::NodeList),
		"node" => Ok(SnapshotProjection::Node {
			node_id: required_u64(call, "node_id")?,
		}),
		"selection" => Ok(SnapshotProjection::Selection),
		other => Err(ToolError::InvalidArguments {
			message: format!("unknown projection `{other}` (expected document_list, active_document, document_summary, node_list, node, or selection)"),
		}),
	}
}

fn projection_schema() -> Value {
	schema_object(
		&["projection"],
		json!({
			"projection": {
				"type": "string",
				"enum": ["document_list", "active_document", "document_summary", "node_list", "node", "selection"],
			},
			"document_id": { "type": "integer", "minimum": 0 },
			"node_id": { "type": "integer", "minimum": 0 },
		}),
	)
}

impl ToolModule for SessionModule {
	fn descriptors(&self) -> Vec<ToolDescriptor> {
		vec![
			descriptor(
				"session.snapshot",
				"Read a typed snapshot projection of the live editor session.",
				Capability::Read,
				projection_schema(),
				schema_object(&[], json!({})),
			),
			descriptor(
				"session.selection",
				"List the currently selected layer/node ids in a document.",
				Capability::Read,
				schema_object(&["document_id"], json!({ "document_id": { "type": "integer", "minimum": 0 } })),
				schema_object(
					&["document_id", "selected_node_ids"],
					json!({ "selected_node_ids": { "type": "array", "items": { "type": "integer" } } }),
				),
			),
			descriptor(
				"session.active_document",
				"Return the id of the document the human is currently editing, if any.",
				Capability::Read,
				schema_object(&[], json!({})),
				schema_object(&["document_id"], json!({ "document_id": { "type": ["integer", "null"] } })),
			),
		]
	}

	fn execute<'a>(&'a mut self, call: ToolCall, bridge: &'a mut dyn EditorBridge) -> Pin<Box<dyn Future<Output = Result<Value, ToolError>> + 'a>> {
		Box::pin(async move {
			let request = match call.name.as_str() {
				"session.snapshot" => {
					let name = required_string(&call, "projection")?;
					let projection = projection(&name, &call)?;
					let document = optional_u64(&call, "document_id")?;
					BridgeQuery::Snapshot { document, projection }
				}
				"session.selection" => BridgeQuery::Snapshot {
					document: Some(required_u64(&call, "document_id")?),
					projection: SnapshotProjection::Selection,
				},
				"session.active_document" => BridgeQuery::Snapshot {
					document: None,
					projection: SnapshotProjection::ActiveDocument,
				},
				other => return Err(ToolError::NotFound { what: format!("tool {other}") }),
			};
			query(bridge, call.id, request).await
		})
	}
}
