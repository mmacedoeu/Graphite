//! Phase 1 unit tests for the contract primitive (T1.12).

use super::*;
use crate::messages::prelude::*;
use graphite_agent_protocol::{AgentOperation, DocumentOperation, SnapshotProjection, ToolError, ToolOutcome};
use serde_json::json;

/// futures' `try_next` reports "currently empty" as `Err(TryRecvError)`, unlike tokio's.
fn try_recv(receiver: &mut futures::channel::mpsc::UnboundedReceiver<ToolOutcome>) -> Option<ToolOutcome> {
	match receiver.try_next() {
		Ok(Some(outcome)) => Some(outcome),
		Ok(None) | Err(_) => None,
	}
}

fn handler_with_sink() -> (AgentMessageHandler, futures::channel::mpsc::UnboundedReceiver<ToolOutcome>) {
	let (sender, receiver) = futures::channel::mpsc::unbounded();
	let mut handler = AgentMessageHandler::default();
	handler.set_reply_sink(sender);
	(handler, receiver)
}

#[test]
fn execute_replies_with_the_correlated_id() {
	let (mut handler, mut receiver) = handler_with_sink();
	let mut portfolio = PortfolioMessageHandler::default();
	let mut responses = VecDeque::new();

	handler.process_message(
		AgentMessage::Execute {
			id: 7,
			document: 1,
			operation: AgentOperation::Undo,
		},
		&mut responses,
		AgentMessageContext { portfolio: &mut portfolio },
	);

	let outcome = try_recv(&mut receiver).expect("one reply");
	match outcome {
		ToolOutcome::Ok { id, result } => {
			assert_eq!(id, 7, "correlation id must survive the round trip");
			assert_eq!(result.get("changed").and_then(serde_json::Value::as_bool), Some(true));
		}
		other => panic!("expected Ok, got {other:?}"),
	}
	assert!(!responses.is_empty(), "the mutation must be queued");
	assert!(try_recv(&mut receiver).is_none(), "exactly one reply");
}

#[test]
fn unknown_node_type_is_not_found() {
	let (mut handler, mut receiver) = handler_with_sink();
	let mut portfolio = PortfolioMessageHandler::default();
	let mut responses = VecDeque::new();

	handler.process_message(
		AgentMessage::Execute {
			id: 11,
			document: 1,
			operation: AgentOperation::AddNode {
				identifier: "definitely::Not::A::Node".to_string(),
				x: 0.,
				y: 0.,
			},
		},
		&mut responses,
		AgentMessageContext { portfolio: &mut portfolio },
	);

	match try_recv(&mut receiver).expect("one reply") {
		ToolOutcome::Err {
			id: 11,
			error: ToolError::NotFound { .. },
		} => {}
		other => panic!("expected NotFound, got {other:?}"),
	}
	assert!(responses.is_empty(), "an unknown node type must not queue a mutation");
}

#[test]
fn cancelled_id_resolves_cancelled_at_most_once() {
	let (mut handler, mut receiver) = handler_with_sink();
	let mut portfolio = PortfolioMessageHandler::default();

	handler.process_message(AgentMessage::Cancel { id: 3 }, &mut VecDeque::new(), AgentMessageContext { portfolio: &mut portfolio });
	handler.process_message(
		AgentMessage::Reply {
			id: 3,
			outcome: ToolOutcome::Ok { id: 3, result: json!({}) },
		},
		&mut VecDeque::new(),
		AgentMessageContext { portfolio: &mut portfolio },
	);

	match try_recv(&mut receiver).expect("one reply") {
		ToolOutcome::Err {
			id: 3,
			error: ToolError::Cancelled { id: 3 },
		} => {}
		other => panic!("expected Cancelled, got {other:?}"),
	}
	assert!(try_recv(&mut receiver).is_none(), "a cancelled id resolves exactly once");
}

#[test]
fn snapshot_requires_a_document_for_document_scoped_projections() {
	let (mut handler, mut receiver) = handler_with_sink();
	let mut portfolio = PortfolioMessageHandler::default();

	handler.process_message(
		AgentMessage::Snapshot {
			id: 5,
			document: None,
			projection: SnapshotProjection::NodeList,
		},
		&mut VecDeque::new(),
		AgentMessageContext { portfolio: &mut portfolio },
	);

	match try_recv(&mut receiver).expect("one reply") {
		ToolOutcome::Err {
			id: 5,
			error: ToolError::InvalidArguments { .. },
		} => {}
		other => panic!("expected InvalidArguments, got {other:?}"),
	}
}

#[test]
fn document_list_snapshot_works_without_a_document() {
	let (mut handler, mut receiver) = handler_with_sink();
	let mut portfolio = PortfolioMessageHandler::default();

	handler.process_message(
		AgentMessage::Snapshot {
			id: 6,
			document: None,
			projection: SnapshotProjection::DocumentList,
		},
		&mut VecDeque::new(),
		AgentMessageContext { portfolio: &mut portfolio },
	);

	match try_recv(&mut receiver).expect("one reply") {
		ToolOutcome::Ok { id: 6, result } => {
			assert!(result.get("documents").and_then(serde_json::Value::as_array).is_some());
		}
		other => panic!("expected Ok, got {other:?}"),
	}
}

#[test]
fn export_gdd_is_deferred_to_the_document_subsystem() {
	let (mut handler, mut receiver) = handler_with_sink();
	let mut portfolio = PortfolioMessageHandler::default();
	let mut responses = VecDeque::new();

	handler.process_message(
		AgentMessage::Document {
			id: 9,
			operation: DocumentOperation::ExportGdd { document: 4 },
		},
		&mut responses,
		AgentMessageContext { portfolio: &mut portfolio },
	);

	assert!(try_recv(&mut receiver).is_none(), "export must not reply immediately (INV-15)");
	assert_eq!(responses.len(), 1, "exactly one document-pass message is queued");
}

#[test]
fn two_concurrent_ids_stay_correlated() {
	let (mut handler, mut receiver) = handler_with_sink();
	let mut portfolio = PortfolioMessageHandler::default();
	let mut responses = VecDeque::new();

	for (id, operation) in [(1, AgentOperation::Undo), (2, AgentOperation::Redo)] {
		handler.process_message(AgentMessage::Execute { id, document: 1, operation }, &mut responses, AgentMessageContext { portfolio: &mut portfolio });
	}

	for expected in [1, 2] {
		match try_recv(&mut receiver).expect("one reply per call") {
			ToolOutcome::Ok { id, .. } => assert_eq!(id, expected, "replies must keep their own ids"),
			other => panic!("expected Ok, got {other:?}"),
		}
	}
	assert!(try_recv(&mut receiver).is_none());
}

#[test]
fn agent_message_carries_only_curated_protocol_types() {
	// Static check (INV-13): the curated wire enum must never embed an editor `Message`.
	let source = include_str!("agent_message.rs");
	for forbidden in [": Message", "Message)", "DocumentMessage", "PortfolioMessage", "NodeGraphMessage"] {
		assert!(!source.contains(forbidden), "AgentMessage must not carry `{forbidden}` (INV-13)");
	}
}

/// Export round-trip against a real editor: `ExportGdd` produces bytes that
/// `document-format` can re-open (Phase 1 gate 2).
#[cfg(test)]
mod export_round_trip {
	use crate::test_utils::test_prelude::*;
	use graphite_agent_protocol::{DocumentOperation, ToolOutcome};

	fn try_recv(receiver: &mut futures::channel::mpsc::UnboundedReceiver<ToolOutcome>) -> Option<ToolOutcome> {
		match receiver.try_next() {
			Ok(Some(outcome)) => Some(outcome),
			Ok(None) | Err(_) => None,
		}
	}

	#[tokio::test]
	async fn export_gdd_bytes_can_be_reopened() {
		let mut editor = EditorTestUtils::create();
		editor.new_document().await;

		let document = editor.editor.dispatcher.message_handlers.portfolio_message_handler.active_document_id.expect("a document").0;

		let (sender, mut receiver) = futures::channel::mpsc::unbounded();
		editor.editor.set_agent_reply_sink(sender);

		editor
			.handle_message(AgentMessage::Document {
				id: 1,
				operation: DocumentOperation::ExportGdd { document },
			})
			.await;

		let base64_gdd = match try_recv(&mut receiver).expect("the async export must reply") {
			ToolOutcome::Ok { result, .. } => result.get("gdd_base64").and_then(serde_json::Value::as_str).expect("gdd_base64 field").to_string(),
			ToolOutcome::Err { error, .. } => panic!("export failed: {error:?}"),
		};

		use base64::Engine as _;
		let bytes = base64::engine::general_purpose::STANDARD.decode(base64_gdd).expect("valid base64");

		let container = document_container::AnyContainer::Memory(document_container::backends::memory::MemoryBackend::new());
		document_format::Gdd::open_from_archive(bytes.as_ref(), container, document_format::GddV1Layout)
			.await
			.expect("the exported bytes must re-open as a .gdd");
	}

	#[tokio::test]
	async fn selection_and_node_list_snapshots_on_a_real_document() {
		use graphite_agent_protocol::SnapshotProjection;

		let mut editor = EditorTestUtils::create();
		editor.new_document().await;

		let document = editor.editor.dispatcher.message_handlers.portfolio_message_handler.active_document_id.expect("a document").0;
		let (sender, mut receiver) = futures::channel::mpsc::unbounded();
		editor.editor.set_agent_reply_sink(sender);

		for (id, projection) in [(1, SnapshotProjection::Selection), (2, SnapshotProjection::NodeList)] {
			editor
				.handle_message(AgentMessage::Snapshot {
					id,
					document: Some(document),
					projection,
				})
				.await;

			match try_recv(&mut receiver).expect("one reply per snapshot") {
				ToolOutcome::Ok { id: reply_id, result } => {
					assert_eq!(reply_id, id);
					assert_eq!(result.get("document_id").and_then(serde_json::Value::as_u64), Some(document));
					if id == 1 {
						assert!(result.get("selected_node_ids").and_then(serde_json::Value::as_array).is_some(), "Selection must project an id array");
					} else {
						assert!(result.get("nodes").and_then(serde_json::Value::as_array).is_some(), "NodeList must project a node array");
					}
				}
				ToolOutcome::Err { error, .. } => panic!("snapshot {id} failed: {error:?}"),
			}
		}
	}
}
