//! Graph authoring tools (T2.5).
//!
//! Mutations go through `BridgeQuery::Operation` (curated `AgentOperation`); reads
//! go through `BridgeQuery::Snapshot` (INV-2, INV-13). `graph.add_node` returns the
//! handler-allocated `node_id` (R2-A) — never an id inferred by diffing lists.

use super::{descriptor, optional_f64, query, required_string, required_u32, required_u64, schema_object};
use graphite_agent_protocol::{AgentOperation, BridgeQuery, Capability, EditorBridge, SnapshotProjection, ToolCall, ToolDescriptor, ToolError, ToolModule};
use serde_json::{Value, json};
use std::future::Future;
use std::pin::Pin;

#[derive(Debug, Default)]
pub struct GraphModule;

impl GraphModule {
	fn operation(document: u64, operation: AgentOperation) -> BridgeQuery {
		BridgeQuery::Operation { document, operation }
	}
}

impl ToolModule for GraphModule {
	fn descriptors(&self) -> Vec<ToolDescriptor> {
		vec![
			descriptor(
				"graph.add_node",
				"Add a node of a given type to a document's graph.",
				Capability::Author,
				schema_object(
					&["document_id", "identifier"],
					json!({
						"document_id": { "type": "integer", "minimum": 0 },
						"identifier": { "type": "string", "description": "Proto node identifier." },
						"x": { "type": "number" },
						"y": { "type": "number" }
					}),
				),
				schema_object(&["node_id"], json!({ "node_id": { "type": "integer", "minimum": 0 } })),
			),
			descriptor(
				"graph.remove_node",
				"Remove a node (and its children) from a document's graph.",
				Capability::Author,
				schema_object(
					&["document_id", "node_id"],
					json!({
						"document_id": { "type": "integer", "minimum": 0 },
						"node_id": { "type": "integer", "minimum": 0 }
					}),
				),
				super::ok_object(),
			),
			descriptor(
				"graph.set_input",
				"Set the value of one of a node's inputs.",
				Capability::Author,
				schema_object(
					&["document_id", "node_id", "input_index", "value"],
					json!({
						"document_id": { "type": "integer", "minimum": 0 },
						"node_id": { "type": "integer", "minimum": 0 },
						"input_index": { "type": "integer", "minimum": 0 },
						"value": { "description": "JSON-encoded TaggedValue for the input." }
					}),
				),
				super::ok_object(),
			),
			descriptor(
				"graph.connect",
				"Connect an output connector to an input connector.",
				Capability::Author,
				schema_object(
					&["document_id", "from_node", "from_output", "to_node", "to_input"],
					json!({
						"document_id": { "type": "integer", "minimum": 0 },
						"from_node": { "type": "integer", "minimum": 0 },
						"from_output": { "type": "integer", "minimum": 0 },
						"to_node": { "type": "integer", "minimum": 0 },
						"to_input": { "type": "integer", "minimum": 0 }
					}),
				),
				super::ok_object(),
			),
			descriptor(
				"graph.disconnect",
				"Disconnect whatever is wired into an input connector.",
				Capability::Author,
				schema_object(
					&["document_id", "to_node", "to_input"],
					json!({
						"document_id": { "type": "integer", "minimum": 0 },
						"to_node": { "type": "integer", "minimum": 0 },
						"to_input": { "type": "integer", "minimum": 0 }
					}),
				),
				super::ok_object(),
			),
			descriptor(
				"graph.list_nodes",
				"List the nodes in a document's graph.",
				Capability::Read,
				schema_object(&["document_id"], json!({ "document_id": { "type": "integer", "minimum": 0 } })),
				json!({
					"$schema": super::SCHEMA,
					"type": "object",
					"required": ["document_id", "nodes"],
					"properties": { "nodes": { "type": "array", "items": { "type": "object" } } }
				}),
			),
			descriptor(
				"graph.get_node",
				"Read one node's identifier and input count.",
				Capability::Read,
				schema_object(
					&["document_id", "node_id"],
					json!({
						"document_id": { "type": "integer", "minimum": 0 },
						"node_id": { "type": "integer", "minimum": 0 }
					}),
				),
				super::ok_object(),
			),
		]
	}

	fn execute<'a>(&'a mut self, call: ToolCall, bridge: &'a mut dyn EditorBridge) -> Pin<Box<dyn Future<Output = Result<Value, ToolError>> + 'a>> {
		Box::pin(async move {
			let document = required_u64(&call, "document_id")?;
			let operation = match call.name.as_str() {
				"graph.add_node" => {
					let identifier = required_string(&call, "identifier")?;
					let x = optional_f64(&call, "x", 0.0)?;
					let y = optional_f64(&call, "y", 0.0)?;
					AgentOperation::AddNode { identifier, x, y }
				}
				"graph.remove_node" => AgentOperation::RemoveNode {
					node_id: required_u64(&call, "node_id")?,
				},
				"graph.set_input" => {
					let node_id = required_u64(&call, "node_id")?;
					let input_index = required_u32(&call, "input_index")?;
					// The argument is an arbitrary JSON value; the editor deserializes it
					// as a `TaggedValue`. Re-encode it as a JSON string for `value_json`.
					let value = call.arguments.get("value").ok_or_else(|| ToolError::InvalidArguments {
						message: "missing argument `value`".to_string(),
					})?;
					let value_json = serde_json::to_string(value).map_err(|error| ToolError::InvalidArguments {
						message: format!("argument `value` is not JSON-serializable: {error}"),
					})?;
					AgentOperation::SetInput { node_id, input_index, value_json }
				}
				"graph.connect" => AgentOperation::Connect {
					from_node: required_u64(&call, "from_node")?,
					from_output: required_u32(&call, "from_output")?,
					to_node: required_u64(&call, "to_node")?,
					to_input: required_u32(&call, "to_input")?,
				},
				"graph.disconnect" => AgentOperation::Disconnect {
					to_node: required_u64(&call, "to_node")?,
					to_input: required_u32(&call, "to_input")?,
				},
				"graph.list_nodes" => {
					return query(
						bridge,
						call.id,
						BridgeQuery::Snapshot {
							document: Some(document),
							projection: SnapshotProjection::NodeList,
						},
					)
					.await;
				}
				"graph.get_node" => {
					let node_id = required_u64(&call, "node_id")?;
					return query(
						bridge,
						call.id,
						BridgeQuery::Snapshot {
							document: Some(document),
							projection: SnapshotProjection::Node { node_id },
						},
					)
					.await;
				}
				other => return Err(ToolError::NotFound { what: format!("tool {other}") }),
			};

			query(bridge, call.id, Self::operation(document, operation)).await
		})
	}
}
