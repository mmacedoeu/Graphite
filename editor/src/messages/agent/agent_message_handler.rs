use crate::messages::portfolio::document::node_graph::document_node_definitions::{DefinitionIdentifier, resolve_document_node_type};
use crate::messages::portfolio::document::query::DocumentQuery;
use crate::messages::portfolio::document::utility_types::network_interface::{InputConnector, OutputConnector};
use crate::messages::prelude::*;
use graph_craft::document::NodeId;
use graph_craft::document::value::TaggedValue;
use graphite_agent_protocol::{AgentOperation, DocumentOperation, QueryId, SnapshotProjection, ToolError, ToolOutcome};
use serde_json::{Value, json};

/// Installed into the editor at construction. The host owns the receiver.
pub type AgentReplySink = futures::channel::mpsc::UnboundedSender<ToolOutcome>;

#[derive(ExtractField)]
pub struct AgentMessageContext<'a> {
	pub portfolio: &'a mut PortfolioMessageHandler,
}

/// Owns no tool modules: it maps curated operations onto existing editor messages
/// and answers typed snapshots through [`DocumentQuery`].
#[derive(Debug, Default, ExtractField)]
pub struct AgentMessageHandler {
	reply_sink: Option<AgentReplySink>,
	/// Ids cancelled by the host. A cancelled id resolves `Cancelled` at most once.
	cancelled: HashSet<QueryId>,
	/// True between an agent-issued `BeginTransaction` and its commit/abort, so
	/// individual mutations are not double-wrapped (INV-5).
	open_transaction: bool,
}

impl AgentMessageHandler {
	pub fn set_reply_sink(&mut self, sink: AgentReplySink) {
		self.reply_sink = Some(sink);
	}

	/// Forward one terminal outcome to the reply sink, translating a cancelled id
	/// into `Cancelled` and removing the id so it can never resolve twice (T1.9/T1.10).
	fn reply(&mut self, id: QueryId, outcome: ToolOutcome) {
		let was_cancelled = self.cancelled.remove(&id);
		let outcome = if was_cancelled {
			ToolOutcome::Err {
				id,
				error: ToolError::Cancelled { id },
			}
		} else {
			outcome
		};

		let Some(sink) = &self.reply_sink else {
			log::warn!("AgentMessageHandler has no reply sink installed; dropping outcome for id {id}");
			return;
		};
		if sink.unbounded_send(outcome).is_err() {
			log::warn!("Agent reply sink is closed; dropping outcome for id {id}");
		}
	}

	fn reply_ok(&mut self, id: QueryId, result: Value) {
		self.reply(id, ToolOutcome::Ok { id, result });
	}

	fn reply_err(&mut self, id: QueryId, error: ToolError) {
		self.reply(id, ToolOutcome::Err { id, error });
	}
}

#[message_handler_data]
impl MessageHandler<AgentMessage, AgentMessageContext<'_>> for AgentMessageHandler {
	fn process_message(&mut self, message: AgentMessage, responses: &mut VecDeque<Message>, context: AgentMessageContext) {
		match message {
			AgentMessage::Cancel { id } => {
				self.cancelled.insert(id);
			}
			AgentMessage::Reply { id, outcome } => {
				self.reply(id, outcome);
			}
			AgentMessage::Execute { id, document, operation } => match self.execute_operation(responses, document, operation) {
				Ok(result) => self.reply_ok(id, result),
				Err(error) => self.reply_err(id, error),
			},
			AgentMessage::Snapshot { id, document, projection } => match snapshot(context.portfolio, document, projection) {
				Ok(result) => self.reply_ok(id, result),
				Err(error) => self.reply_err(id, error),
			},
			AgentMessage::Document { id, operation } => self.document_operation(id, responses, operation),
		}
	}

	advertise_actions!(AgentMessageDiscriminant;
	);
}

impl AgentMessageHandler {
	fn document_operation(&mut self, id: QueryId, responses: &mut VecDeque<Message>, operation: DocumentOperation) {
		match operation {
			DocumentOperation::New { name } => {
				responses.add(PortfolioMessage::NewDocumentWithName { name });
				self.reply_ok(id, json!({ "requested": true }));
			}
			DocumentOperation::Open { path } => {
				// The host canonicalized and prefix-checked the path (INV-12); the editor reads it.
				let path_buf = std::path::PathBuf::from(&path);
				let content = match std::fs::read(&path_buf) {
					Ok(content) => content,
					Err(error) => {
						self.reply_err(
							id,
							ToolError::NotFound {
								what: format!("document file {path}: {error}"),
							},
						);
						return;
					}
				};
				let is_gdd = path_buf.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("gdd"));
				let document_name = path_buf.file_stem().map(|stem| stem.to_string_lossy().to_string());
				if is_gdd {
					responses.add(PortfolioMessage::OpenGddDocument {
						document_name,
						document_path: Some(path_buf),
						content,
					});
				} else {
					responses.add(PortfolioMessage::OpenFile { path: path_buf, content });
				}
				self.reply_ok(id, json!({ "requested": true }));
			}
			DocumentOperation::Close { document } => {
				responses.add(PortfolioMessage::CloseDocument { document_id: DocumentId(document) });
				self.reply_ok(id, json!({ "requested": true }));
			}
			DocumentOperation::ExportGdd { document } => {
				// Async: the document subsystem embeds resources and exports, then emits
				// `AgentMessage::Reply` (INV-15, HIGH-B1). No immediate reply here.
				responses.add(portfolio_document(document, DocumentMessage::ExportGdd { request_id: id }));
			}
			// Forward-compatible: later phases may add protocol variants (additive only, §5).
			_ => {
				self.reply_err(
					id,
					ToolError::InvalidArguments {
						message: "unsupported DocumentOperation variant".to_string(),
					},
				);
			}
		}
	}

	fn execute_operation(&mut self, responses: &mut VecDeque<Message>, document: u64, operation: AgentOperation) -> Result<Value, ToolError> {
		match operation {
			AgentOperation::AddNode { identifier, x, y } => {
				let node_type = definition_identifier(&identifier)?;
				// `NodeId` is hash-derived, not monotonic: allocate it here and reply with
				// this exact id rather than diffing node lists (R2-A).
				let node_id = NodeId::new();
				responses.add(portfolio_document(
					document,
					NodeGraphMessage::CreateNodeFromContextMenu {
						node_id: Some(node_id),
						node_type,
						xy: Some((x as i32, y as i32)),
						add_transaction: true,
					}
					.into(),
				));
				Ok(json!({ "node_id": node_id.0 }))
			}
			AgentOperation::RemoveNode { node_id } => {
				self.wrap_transaction(responses, document);
				responses.add(portfolio_document(
					document,
					NodeGraphMessage::DeleteNodes {
						node_ids: vec![NodeId(node_id)],
						delete_children: true,
					}
					.into(),
				));
				Ok(json!({ "node_id": node_id, "removed": true }))
			}
			AgentOperation::SetInput { node_id, input_index, value_json } => {
				let value: TaggedValue = serde_json::from_str(&value_json).map_err(|error| ToolError::InvalidArguments {
					message: format!("value_json is not a JSON-encoded TaggedValue: {error}"),
				})?;
				self.wrap_transaction(responses, document);
				responses.add(portfolio_document(
					document,
					NodeGraphMessage::SetInputValue {
						node_id: NodeId(node_id),
						input_index: input_index as usize,
						value: Box::new(value),
					}
					.into(),
				));
				Ok(json!({ "node_id": node_id }))
			}
			AgentOperation::Connect {
				from_node,
				from_output,
				to_node,
				to_input,
			} => {
				self.wrap_transaction(responses, document);
				responses.add(portfolio_document(
					document,
					NodeGraphMessage::CreateWire {
						output_connector: OutputConnector::node(NodeId(from_node), from_output as usize),
						input_connector: InputConnector::node_at_index(NodeId(to_node), to_input as usize),
					}
					.into(),
				));
				Ok(json!({ "connected": true }))
			}
			AgentOperation::Disconnect { to_node, to_input } => {
				self.wrap_transaction(responses, document);
				responses.add(portfolio_document(
					document,
					NodeGraphMessage::DisconnectInput {
						input_connector: InputConnector::node_at_index(NodeId(to_node), to_input as usize),
					}
					.into(),
				));
				Ok(json!({ "disconnected": true }))
			}
			AgentOperation::BeginTransaction => {
				responses.add(portfolio_document(document, DocumentMessage::StartTransaction));
				self.open_transaction = true;
				Ok(json!({ "ok": true }))
			}
			AgentOperation::CommitTransaction => {
				responses.add(portfolio_document(document, DocumentMessage::CommitTransaction));
				self.open_transaction = false;
				Ok(json!({ "ok": true }))
			}
			AgentOperation::AbortTransaction => {
				responses.add(portfolio_document(document, DocumentMessage::AbortTransaction));
				self.open_transaction = false;
				Ok(json!({ "ok": true }))
			}
			AgentOperation::Undo => {
				responses.add(portfolio_document(document, DocumentMessage::DocumentHistoryBackward));
				Ok(json!({ "changed": true }))
			}
			AgentOperation::Redo => {
				responses.add(portfolio_document(document, DocumentMessage::DocumentHistoryForward));
				Ok(json!({ "changed": true }))
			}
			// Forward-compatible: later phases may add protocol variants (additive only, §5).
			_ => Err(ToolError::InvalidArguments {
				message: "unsupported AgentOperation variant".to_string(),
			}),
		}
	}

	/// Wrap a mutation in its own transaction so undo is always available (INV-5),
	/// unless the agent already opened one with `BeginTransaction`.
	fn wrap_transaction(&self, responses: &mut VecDeque<Message>, document: u64) {
		if !self.open_transaction {
			responses.add(portfolio_document(document, DocumentMessage::AddTransaction));
		}
	}
}

fn portfolio_document(document: u64, message: DocumentMessage) -> Message {
	PortfolioMessage::DocumentPassMessage {
		document_id: DocumentId(document),
		message,
	}
	.into()
}

fn definition_identifier(identifier: &str) -> Result<DefinitionIdentifier, ToolError> {
	let definition = if identifier.starts_with("PROTONODE:") || identifier.starts_with("NETWORK:") {
		DefinitionIdentifier::from_serialized(identifier)
	} else {
		DefinitionIdentifier::ProtoNode(graphene_std::ProtoNodeIdentifier::with_owned_string(identifier.to_string()))
	};

	if resolve_document_node_type(&definition).is_none() {
		return Err(ToolError::NotFound {
			what: format!("node type {identifier}"),
		});
	}
	Ok(definition)
}

fn snapshot(portfolio: &PortfolioMessageHandler, document: Option<u64>, projection: SnapshotProjection) -> Result<Value, ToolError> {
	match projection {
		SnapshotProjection::DocumentList => portfolio.document_list(),
		// Fully qualified: `PortfolioMessageHandler` has an inherent `active_document()`
		// returning `Option<&DocumentMessageHandler>`, which shadows the trait method.
		SnapshotProjection::ActiveDocument => Ok(json!({ "document_id": DocumentQuery::active_document(portfolio)? })),
		SnapshotProjection::DocumentSummary => portfolio.summary(require_document(document, "DocumentSummary")?),
		SnapshotProjection::NodeList => portfolio.node_list(require_document(document, "NodeList")?),
		SnapshotProjection::Node { node_id } => portfolio.node(require_document(document, "Node")?, node_id),
		SnapshotProjection::Selection => portfolio.selection(require_document(document, "Selection")?),
		// Forward-compatible: later phases may add protocol variants (additive only, §5).
		_ => Err(ToolError::InvalidArguments {
			message: "unsupported SnapshotProjection variant".to_string(),
		}),
	}
}

fn require_document(document: Option<u64>, projection: &str) -> Result<u64, ToolError> {
	document.ok_or_else(|| ToolError::InvalidArguments {
		message: format!("{projection} requires a document id"),
	})
}
