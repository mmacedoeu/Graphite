//! Headless mode bridge (T2.1): owns the single [`Editor`] and translates curated
//! bridge queries into `AgentMessage`s, buffering correlated replies by `QueryId`.

use futures::channel::mpsc::{UnboundedReceiver, unbounded};
use graphite_agent_protocol::{AgentEvent, BridgeQuery, EditorBridge, QueryId, ToolError, ToolOutcome};
use graphite_editor::application::{Editor, HeadlessEditorState};
use graphite_editor::messages::future::FutureMessage;
use graphite_editor::messages::prelude::{AgentMessage, Message, Wake};
use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;

/// Owns the headless editor, the reply receiver, and a per-`QueryId` buffer so an
/// asynchronous reply (`.gdd` export) that arrives after its poll is not lost.
pub struct HeadlessBridge {
	editor: Editor,
	/// Kept so the wake stays alive for the lifetime of the editor's future spawner.
	_wake: Wake,
	replies: UnboundedReceiver<ToolOutcome>,
	buffered: HashMap<QueryId, Result<serde_json::Value, ToolError>>,
}

impl HeadlessBridge {
	/// Construct the process's one `Editor` (INV-11) and install the host-owned
	/// reply sink. The editor's working-copy directory lives at
	/// `<root>/.graphite-agent-working-copy` (T2.1).
	pub fn new(root: &Path) -> Result<Self, std::io::Error> {
		Self::open(root, None)
	}

	/// As [`HeadlessBridge::new`], but `storage` (the `--storage` flag) overrides the
	/// working-copy directory when supplied.
	pub fn open(root: &Path, storage: Option<&Path>) -> Result<Self, std::io::Error> {
		let working_copy_root = storage.map(Path::to_path_buf).unwrap_or_else(|| working_copy_root(root));
		std::fs::create_dir_all(&working_copy_root)?;

		let (sender, receiver) = unbounded();
		let state = HeadlessEditorState {
			resource_storage: Arc::new(graph_craft::application_io::resource::HashMapResourceStorage::new()),
			working_copy_root,
			uuid_random_seed: 0,
		};
		let (mut editor, wake) = Editor::new_headless(state);
		editor.set_agent_reply_sink(sender);

		// T3.6: generate the command catalog from this process's one `Editor`
		// (INV-11) and hand it to the read-only `graphite://command-catalog`
		// resource. Generating it here — rather than from the descriptors crate's
		// own editor — is what keeps a second `Editor` from being constructed.
		crate::modules::command_catalog::install_command_catalog(graphite_agent_descriptors::commands::command_catalog_json_from_actions(&editor.dispatcher.collect_actions()));

		Ok(Self {
			editor,
			_wake: wake,
			replies: receiver,
			buffered: HashMap::new(),
		})
	}

	/// The working-copy directory used by the headless editor.
	pub fn working_copy_root(root: &Path) -> PathBuf {
		working_copy_root(root)
	}

	fn bridge_message(id: QueryId, query: BridgeQuery) -> Result<AgentMessage, ToolError> {
		match query {
			BridgeQuery::Document { operation } => Ok(AgentMessage::Document { id, operation }),
			BridgeQuery::Operation { document, operation } => Ok(AgentMessage::Execute { id, document, operation }),
			BridgeQuery::Snapshot { document, projection } => Ok(AgentMessage::Snapshot { id, document, projection }),
			// Forward-compatible: later phases may add protocol variants (additive only, §5).
			_ => Err(ToolError::InvalidArguments {
				message: "unsupported BridgeQuery variant".to_string(),
			}),
		}
	}

	/// Move every terminal outcome the editor produced into the per-id buffer.
	fn drain_reply_sink(&mut self) {
		// `try_next` yields `Err` while the channel is empty and `Ok(None)` once the
		// sender has dropped; both end the drain.
		while let Ok(Some(outcome)) = self.replies.try_next() {
			self.buffer(outcome);
		}
	}

	fn buffer(&mut self, outcome: ToolOutcome) {
		match outcome {
			ToolOutcome::Ok { id, result } => {
				self.buffered.insert(id, Ok(result));
			}
			ToolOutcome::Err { id, error } => {
				self.buffered.insert(id, Err(error));
			}
		}
	}
}

/// `<root>/.graphite-agent-working-copy`.
fn working_copy_root(root: &Path) -> PathBuf {
	root.join(".graphite-agent-working-copy")
}

impl EditorBridge for HeadlessBridge {
	fn submit(&mut self, id: QueryId, query: BridgeQuery) -> Result<(), ToolError> {
		let message = Self::bridge_message(id, query)?;
		let _ = self.editor.handle_message(message);
		Ok(())
	}

	fn poll(&mut self, id: QueryId) -> Option<Result<serde_json::Value, ToolError>> {
		self.buffered.remove(&id)
	}

	fn cancel(&mut self, id: QueryId) {
		let _ = self.editor.handle_message(AgentMessage::Cancel { id });
	}

	fn drain_events(&mut self) -> Vec<AgentEvent> {
		// Phase 2 has no editor-side event source (document-change events arrive in
		// Phase 4); the host's own event channel is used for anything it emits.
		Vec::new()
	}

	fn pump(&mut self) -> Pin<Box<dyn Future<Output = Result<(), ToolError>> + '_>> {
		Box::pin(async move {
			// §5.4 pump loop, with execution finding E-5: `FutureMessage::Wake` MUST be
			// dispatched at the TOP of every iteration. It drains deferred async results
			// into the dispatcher, which is the only way an async `ExportGdd` reply is
			// ever delivered when there is no pending node-graph work.
			loop {
				let _ = self.editor.handle_message(FutureMessage::Wake);

				let (more, _texture) = graphite_editor::node_graph_executor::run_node_graph().await;
				if !more {
					break;
				}

				let mut messages = VecDeque::new();
				if let Err(error) = self.editor.poll_node_graph_evaluation(&mut messages) {
					// HIGH-A3: normal before any document exists, not a failure.
					if error != "No active document" {
						return Err(ToolError::Internal { message: error });
					}
				}
				if messages.is_empty() {
					break;
				}
				let _ = self.editor.handle_message(Message::Batched {
					messages: messages.into_iter().collect(),
				});
			}

			self.drain_reply_sink();
			Ok(())
		})
	}
}
