//! The typed, sync read boundary over the portfolio (INV-2).
//!
//! Agents never read message-handler fields directly; every read goes through
//! [`DocumentQuery`]. Export is deliberately **not** here: `.gdd` export is async
//! and lives in the document subsystem (INV-15, HIGH-B1).

use crate::messages::portfolio::document::document_message_handler::DocumentMessageHandler;
use crate::messages::portfolio::document::utility_types::misc::DocumentId;
use crate::messages::portfolio::portfolio_message_handler::PortfolioMessageHandler;
use graph_craft::document::{DocumentNode, DocumentNodeImplementation, NodeId};
use graphite_agent_protocol::ToolError;
use serde_json::{Value, json};

/// Sync typed reads only (INV-15: export is NOT here).
pub trait DocumentQuery {
	fn document_list(&self) -> Result<Value, ToolError>;
	fn summary(&self, document: u64) -> Result<Value, ToolError>;
	fn node_list(&self, document: u64) -> Result<Value, ToolError>;
	fn node(&self, document: u64, node_id: u64) -> Result<Value, ToolError>;
	fn active_document(&self) -> Result<Option<u64>, ToolError>;
	/// Selected layer/node ids for the document (R7-H1). Needed by `session.selection`.
	fn selection(&self, document: u64) -> Result<Value, ToolError>;
}

impl PortfolioMessageHandler {
	/// The shared lookup used by every query method. Not part of the trait: it hands
	/// out a borrow of the handler, which stays inside this module.
	fn query_document(&self, document: u64) -> Result<&DocumentMessageHandler, ToolError> {
		self.documents.get(&DocumentId(document)).ok_or_else(|| ToolError::NotFound { what: format!("document {document}") })
	}
}

impl DocumentQuery for PortfolioMessageHandler {
	fn document_list(&self) -> Result<Value, ToolError> {
		let active = self.active_document_id.map(|document_id| document_id.0);
		let mut documents: Vec<_> = self
			.documents
			.iter()
			.map(|(document_id, handler)| {
				json!({
					"document_id": document_id.0,
					"name": handler.name,
					"node_count": handler.network_interface.document_network().nodes.len(),
					"is_active": active == Some(document_id.0),
				})
			})
			.collect();
		documents.sort_by_key(|document| document.get("document_id").and_then(Value::as_u64).unwrap_or(0));
		Ok(json!({ "documents": documents }))
	}

	fn summary(&self, document: u64) -> Result<Value, ToolError> {
		let is_active = self.active_document_id == Some(DocumentId(document));
		let handler = self.query_document(document)?;
		Ok(json!({
			"document_id": document,
			"name": handler.name,
			"node_count": handler.network_interface.document_network().nodes.len(),
			"is_active": is_active,
		}))
	}

	fn node_list(&self, document: u64) -> Result<Value, ToolError> {
		let handler = self.query_document(document)?;
		let mut nodes: Vec<_> = handler.network_interface.document_network().nodes.iter().map(|(node_id, node)| node_json(*node_id, node)).collect();
		nodes.sort_by_key(|node| node.get("node_id").and_then(Value::as_u64).unwrap_or(0));
		Ok(json!({ "document_id": document, "nodes": nodes }))
	}

	fn node(&self, document: u64, node_id: u64) -> Result<Value, ToolError> {
		let handler = self.query_document(document)?;
		let id = NodeId(node_id);
		let node = handler.network_interface.document_network().nodes.get(&id).ok_or_else(|| ToolError::NotFound {
			what: format!("node {node_id} in document {document}"),
		})?;
		Ok(node_json(id, node))
	}

	fn active_document(&self) -> Result<Option<u64>, ToolError> {
		Ok(self.active_document_id.map(|document_id| document_id.0))
	}

	fn selection(&self, document: u64) -> Result<Value, ToolError> {
		let handler = self.query_document(document)?;
		let selected: Vec<u64> = handler.network_interface.selected_nodes().selected_nodes().map(|node_id| node_id.0).collect();
		Ok(json!({ "document_id": document, "selected_node_ids": selected }))
	}
}

fn node_json(node_id: NodeId, node: &DocumentNode) -> Value {
	let identifier = match &node.implementation {
		DocumentNodeImplementation::ProtoNode(proto_node_identifier) => proto_node_identifier.as_str().to_string(),
		DocumentNodeImplementation::Network(_) => "Network".to_string(),
		_ => "Extract".to_string(),
	};
	json!({
		"node_id": node_id.0,
		"identifier": identifier,
		"visible": node.visible,
		"input_count": node.inputs.len(),
	})
}
