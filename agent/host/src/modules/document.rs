//! Document lifecycle tools (T2.3).
//!
//! `document.save` is `ExportGdd` (async, editor-side, INV-15) followed by a
//! host-side confined write. `document.open` confines the path and then hands it
//! to the editor, which reads the file itself (M-B4) — the host never reads bytes
//! for open. `document.new`/`open` discover the resulting id by diffing two
//! `Snapshot(DocumentList)` reads (R2-L), because `DocumentId`s are random uuids.

use super::{descriptor, optional_string, query, required_string, required_u64, schema_object};
use crate::paths::PathRoot;
use base64::Engine as _;
use graphite_agent_protocol::{BridgeQuery, Capability, DocumentOperation, EditorBridge, SnapshotProjection, ToolCall, ToolDescriptor, ToolError, ToolModule};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

#[derive(Debug)]
pub struct DocumentModule {
	paths: Arc<PathRoot>,
}

impl DocumentModule {
	pub fn new(paths: Arc<PathRoot>) -> Self {
		Self { paths }
	}

	fn list_query() -> BridgeQuery {
		BridgeQuery::Snapshot {
			document: None,
			projection: SnapshotProjection::DocumentList,
		}
	}

	async fn document_ids(bridge: &mut dyn EditorBridge, id: u64) -> Result<BTreeSet<u64>, ToolError> {
		let list = query(bridge, id, Self::list_query()).await?;
		Ok(ids_of(&list))
	}

	/// Pump the editor until the document list contains an id that was not present
	/// in `before`, then return it.
	async fn discover_new_document(bridge: &mut dyn EditorBridge, id: u64, before: &BTreeSet<u64>) -> Result<u64, ToolError> {
		for _ in 0..100 {
			let after = Self::document_ids(bridge, id).await?;
			if let Some(new_id) = after.difference(before).next() {
				return Ok(*new_id);
			}
			tokio::task::yield_now().await;
		}
		Err(ToolError::Internal {
			message: "the editor did not report a new document after the lifecycle operation".to_string(),
		})
	}
}

impl ToolModule for DocumentModule {
	fn descriptors(&self) -> Vec<ToolDescriptor> {
		vec![
			descriptor(
				"document.new",
				"Create a new empty document.",
				Capability::Persist,
				schema_object(&[], json!({ "name": { "type": "string" } })),
				schema_object(&["document_id"], json!({ "document_id": { "type": "integer", "minimum": 0 } })),
			),
			descriptor(
				"document.open",
				"Open a document file under the configured root.",
				Capability::Read,
				schema_object(&["path"], json!({ "path": { "type": "string" } })),
				schema_object(&["document_id"], json!({ "document_id": { "type": "integer", "minimum": 0 } })),
			),
			descriptor(
				"document.save",
				"Export a document to `.gdd` and write it under the configured root.",
				Capability::Persist,
				schema_object(&["document_id"], json!({ "document_id": { "type": "integer", "minimum": 0 }, "path": { "type": "string" } })),
				schema_object(&["saved", "path"], json!({ "saved": { "type": "boolean" }, "path": { "type": "string" } })),
			),
			descriptor(
				"document.close",
				"Close a document.",
				Capability::Persist,
				schema_object(&["document_id"], json!({ "document_id": { "type": "integer", "minimum": 0 } })),
				schema_object(&["closed"], json!({ "closed": { "type": "boolean" } })),
			),
			descriptor(
				"document.list",
				"List the open documents.",
				Capability::Read,
				schema_object(&[], json!({})),
				json!({
					"$schema": super::SCHEMA,
					"type": "object",
					"required": ["documents"],
					"properties": { "documents": { "type": "array", "items": { "type": "object" } } }
				}),
			),
		]
	}

	fn execute<'a>(&'a mut self, call: ToolCall, bridge: &'a mut dyn EditorBridge) -> Pin<Box<dyn Future<Output = Result<Value, ToolError>> + 'a>> {
		Box::pin(async move {
			match call.name.as_str() {
				"document.new" => {
					let name = optional_string(&call, "name")?.unwrap_or_else(|| "Untitled Document".to_string());
					let before = Self::document_ids(bridge, call.id).await?;
					query(
						bridge,
						call.id,
						BridgeQuery::Document {
							operation: DocumentOperation::New { name },
						},
					)
					.await?;
					let document = Self::discover_new_document(bridge, call.id, &before).await?;
					Ok(json!({ "document_id": document }))
				}
				"document.open" => {
					let requested = required_string(&call, "path")?;
					// INV-12: confine before the editor ever sees the path.
					let path = self.paths.resolve(&requested)?;
					let before = Self::document_ids(bridge, call.id).await?;
					query(
						bridge,
						call.id,
						BridgeQuery::Document {
							operation: DocumentOperation::Open {
								path: path.to_string_lossy().to_string(),
							},
						},
					)
					.await?;
					let document = Self::discover_new_document(bridge, call.id, &before).await?;
					Ok(json!({ "document_id": document, "path": path }))
				}
				"document.save" => {
					let document = required_u64(&call, "document_id")?;
					let target = match optional_string(&call, "path")? {
						Some(path) => self.paths.resolve(&path)?,
						None => self.paths.generated(&format!("agent-document-{document}"), "gdd"),
					};
					let exported = query(
						bridge,
						call.id,
						BridgeQuery::Document {
							operation: DocumentOperation::ExportGdd { document },
						},
					)
					.await?;
					let bytes = decode_gdd(&exported)?;
					if let Some(parent) = target.parent() {
						std::fs::create_dir_all(parent).map_err(|error| ToolError::Internal {
							message: format!("failed to create {}: {error}", parent.display()),
						})?;
					}
					std::fs::write(&target, bytes).map_err(|error| ToolError::Internal {
						message: format!("failed to write {}: {error}", target.display()),
					})?;
					Ok(json!({ "saved": true, "path": target, "document_id": document }))
				}
				"document.close" => {
					let document = required_u64(&call, "document_id")?;
					query(
						bridge,
						call.id,
						BridgeQuery::Document {
							operation: DocumentOperation::Close { document },
						},
					)
					.await?;
					Ok(json!({ "closed": true, "document_id": document }))
				}
				"document.list" => query(bridge, call.id, Self::list_query()).await,
				other => Err(ToolError::NotFound { what: format!("tool {other}") }),
			}
		})
	}
}

fn ids_of(list: &Value) -> BTreeSet<u64> {
	list.get("documents")
		.and_then(Value::as_array)
		.into_iter()
		.flatten()
		.filter_map(|document| document.get("document_id").and_then(Value::as_u64))
		.collect()
}

/// Decode the `{ "gdd_base64": "..." }` reply emitted by the editor's async export.
pub(crate) fn decode_gdd(exported: &Value) -> Result<Vec<u8>, ToolError> {
	let encoded = exported.get("gdd_base64").and_then(Value::as_str).ok_or_else(|| ToolError::Internal {
		message: "gdd export reply did not contain gdd_base64".to_string(),
	})?;
	base64::engine::general_purpose::STANDARD.decode(encoded).map_err(|error| ToolError::Internal {
		message: format!("gdd export was not valid base64: {error}"),
	})
}
