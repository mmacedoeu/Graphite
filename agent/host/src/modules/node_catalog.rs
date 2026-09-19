//! Catalog tools: `node.list_types` and `node.describe`.
//!
//! Both are generated from `NODE_METADATA` through `graphite-agent-descriptors`
//! (INV-8); the generated `node.type.*` entries are catalog data, never MCP tools
//! (§6, R2-F). These two tools are the only generated entries promoted to tools.

use super::{descriptor, required_string, schema_object};
use graphite_agent_descriptors::{node_catalog_json, node_descriptors_with_identifiers};
use graphite_agent_protocol::{Capability, EditorBridge, ToolCall, ToolDescriptor, ToolError, ToolModule};
use serde_json::{Value, json};
use std::future::Future;
use std::pin::Pin;

#[derive(Debug, Default)]
pub struct NodeCatalogModule;

impl ToolModule for NodeCatalogModule {
	fn descriptors(&self) -> Vec<ToolDescriptor> {
		vec![
			descriptor(
				"node.list_types",
				"List every node type available in the catalog, with its identifier and display name.",
				Capability::Read,
				schema_object(&[], json!({})),
				json!({
					"$schema": super::SCHEMA,
					"type": "object",
					"required": ["types"],
					"properties": { "types": { "type": "array", "items": { "type": "object" } } }
				}),
			),
			descriptor(
				"node.describe",
				"Describe one node type's inputs from the generated catalog.",
				Capability::Read,
				schema_object(&["identifier"], json!({ "identifier": { "type": "string", "description": "Proto node identifier." } })),
				super::ok_object(),
			),
		]
	}

	fn execute<'a>(&'a mut self, call: ToolCall, _bridge: &'a mut dyn EditorBridge) -> Pin<Box<dyn Future<Output = Result<Value, ToolError>> + 'a>> {
		Box::pin(async move {
			match call.name.as_str() {
				"node.list_types" => {
					let types: Vec<Value> = node_descriptors_with_identifiers()
						.into_iter()
						.map(|(identifier, descriptor)| {
							json!({
								"identifier": identifier,
								"name": descriptor.name,
								"description": descriptor.description,
							})
						})
						.collect();
					Ok(json!({ "types": types, "count": types.len() }))
				}
				"node.describe" => {
					let identifier = required_string(&call, "identifier")?;
					let descriptor = node_descriptors_with_identifiers()
						.into_iter()
						.find(|(candidate, _)| candidate == &identifier)
						.map(|(_, descriptor)| descriptor)
						.ok_or_else(|| ToolError::NotFound {
							what: format!("node type {identifier}"),
						})?;
					serde_json::to_value(descriptor).map_err(|error| ToolError::Internal {
						message: format!("failed to serialize node descriptor: {error}"),
					})
				}
				other => Err(ToolError::NotFound { what: format!("tool {other}") }),
			}
		})
	}
}

/// The whole catalog as JSON, used by the `graphite://node-catalog` MCP resource (§11).
pub fn node_catalog() -> Value {
	node_catalog_json()
}
