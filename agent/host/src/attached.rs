//! Attached mode (Surface B, T4.4): an [`EditorBridge`] over the desktop's
//! bidirectional local-socket protocol.
//!
//! # Shared framing definition (T4.1, mirrored)
//!
//! The wire protocol is defined by the desktop side in
//! `desktop/src/socket.rs::Frame` and mirrored here, because the desktop and agent
//! processes are separate crates that do not share a type (the `graphite-desktop`
//! crate does not depend on `agent/protocol`, and `agent/host` cannot depend on the
//! desktop). **The two definitions must stay byte-compatible.**
//!
//! ```text
//! frame   = 4-byte big-endian u32 payload length, followed by `payload`
//! payload = RON-encoded `Frame` enum, UTF-8
//!
//! enum Frame {
//!     Request { id: QueryId, query: BridgeQuery },  // agent -> desktop
//!     Response(ToolOutcome),                        // desktop -> agent (correlated by ToolOutcome::id)
//!     Event(AgentEvent),                            // desktop -> agent (unsolicited)
//!     Cancel { id: QueryId },                       // agent -> desktop
//! }
//! ```
//!
//! `Request` carries the host-allocated [`QueryId`] (INV-14) so the desktop can
//! echo it in the correlated `Response`. That id is what makes the framing
//! *correlated*; without it `ToolOutcome::id` would be meaningless to the agent.
//! `Cancel` is the minimal control frame needed to transport the frozen
//! [`EditorBridge::cancel`] method; it maps to the editor's existing
//! `AgentMessage::Cancel { id }` and needs no contract change.
//!
//! The legacy one-way `Message::OpenFiles` RON payload is unchanged on the desktop
//! side and never reaches this crate.

use graphite_agent_protocol::{AgentEvent, BridgeQuery, EditorBridge, QueryId, ToolError, ToolOutcome};
use interprocess::local_socket::{GenericFilePath, ListenerNonblockingMode, ListenerOptions, prelude::*};
use std::collections::HashMap;
use std::future::Future;
use std::io::{ErrorKind, Read, Write};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Upper bound on a single frame body, so a corrupt length prefix cannot make the
/// agent allocate unbounded memory.
const MAX_FRAME_LEN: u32 = 64 * 1024 * 1024;
/// How long a non-blocking-style read waits before yielding back to the pump loop.
const READ_TIMEOUT: Duration = Duration::from_millis(2);
/// How long a write may stall before it is reported as a failure.
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);

/// The bidirectional attached-session frame (T4.1). Mirror of
/// `desktop/src/socket.rs::Frame`; see the module docs for the exact wire format.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum Frame {
	/// Agent -> desktop. `id` is host-allocated (INV-14) and is echoed in the
	/// correlated [`Frame::Response`].
	Request { id: QueryId, query: BridgeQuery },
	/// Desktop -> agent. Correlated by [`ToolOutcome`]'s id.
	Response(ToolOutcome),
	/// Desktop -> agent. Unsolicited editor-state change (e.g. a human edit).
	Event(AgentEvent),
	/// Agent -> desktop. Cancel an in-flight request.
	Cancel { id: QueryId },
}

/// Encode one frame: 4-byte big-endian length + RON body.
fn encode_frame(frame: &Frame) -> Result<Vec<u8>, String> {
	let body = ron::ser::to_string(frame).map_err(|error| format!("failed to encode frame: {error}"))?;
	let body = body.as_bytes();
	let length = u32::try_from(body.len()).map_err(|_| "frame is too large to encode".to_string())?;
	let mut encoded = Vec::with_capacity(4 + body.len());
	encoded.extend_from_slice(&length.to_be_bytes());
	encoded.extend_from_slice(body);
	Ok(encoded)
}

/// Decode one frame body (without the length prefix).
fn decode_frame(body: &[u8]) -> Result<Frame, String> {
	let text = std::str::from_utf8(body).map_err(|error| format!("frame body is not UTF-8: {error}"))?;
	ron::de::from_str(text).map_err(|error| format!("failed to decode frame: {error}"))
}

/// Buffered length-prefixed frame reader, shared by [`AttachedBridge`] and the
/// test-side server so both sides provably use one framing implementation.
#[derive(Default)]
struct FrameReader {
	buffer: Vec<u8>,
}

impl FrameReader {
	fn push(&mut self, bytes: &[u8]) {
		self.buffer.extend_from_slice(bytes);
	}

	/// Pop the next complete frame, if the buffer holds one.
	fn next(&mut self) -> Result<Option<Frame>, String> {
		if self.buffer.len() < 4 {
			return Ok(None);
		}
		let length = u32::from_be_bytes([self.buffer[0], self.buffer[1], self.buffer[2], self.buffer[3]]);
		if length > MAX_FRAME_LEN {
			return Err(format!("frame length {length} exceeds the {MAX_FRAME_LEN}-byte limit"));
		}
		let end = 4 + length as usize;
		if self.buffer.len() < end {
			return Ok(None);
		}
		let frame = decode_frame(&self.buffer[4..end])?;
		self.buffer.drain(..end);
		Ok(Some(frame))
	}
}

/// One side of an attached connection: read with a short receive timeout so the
/// pump loop always returns control to the host's timeout race.
fn configure(stream: &interprocess::local_socket::Stream) -> std::io::Result<()> {
	stream.set_recv_timeout(Some(READ_TIMEOUT))?;
	stream.set_send_timeout(Some(WRITE_TIMEOUT))
}

/// Read every currently available byte into `reader`, returning after the receive
/// timeout elapses. A closed peer is treated as no-more-data.
fn read_available(stream: &mut interprocess::local_socket::Stream, reader: &mut FrameReader) -> Result<(), ToolError> {
	let mut chunk = [0u8; 8192];
	loop {
		match stream.read(&mut chunk) {
			Ok(0) => return Ok(()),
			Ok(count) => reader.push(&chunk[..count]),
			Err(error) if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut | ErrorKind::Interrupted) => return Ok(()),
			Err(error) => {
				return Err(ToolError::Internal {
					message: format!("attached socket read failed: {error}"),
				});
			}
		}
	}
}

/// Write an encoded frame, reporting a stalled write as an internal error.
fn write_frame(stream: &mut interprocess::local_socket::Stream, frame: &Frame) -> Result<(), ToolError> {
	write_frame_io(stream, frame).map_err(|error| ToolError::Internal {
		message: format!("attached socket write failed: {error}"),
	})
}

/// The io-level frame write, shared with the test-side server.
fn write_frame_io(stream: &mut interprocess::local_socket::Stream, frame: &Frame) -> std::io::Result<()> {
	let encoded = encode_frame(frame).map_err(|message| std::io::Error::new(ErrorKind::InvalidData, message))?;
	stream.write_all(&encoded)
}

/// An [`EditorBridge`] over a live desktop editor session (T4.4).
///
/// `submit` writes a [`Frame::Request`]; `poll` reads frames and returns the
/// correlated [`Frame::Response`]; `cancel` writes a [`Frame::Cancel`];
/// `drain_events` returns the buffered [`Frame::Event`]s. Responses and events
/// interleave freely on the socket — every read drains both.
pub struct AttachedBridge {
	stream: interprocess::local_socket::Stream,
	reader: FrameReader,
	responses: HashMap<QueryId, Result<serde_json::Value, ToolError>>,
	events: Vec<AgentEvent>,
}

impl AttachedBridge {
	/// Connect to the desktop's local socket at `socket_path`.
	pub fn connect(socket_path: &Path) -> std::io::Result<Self> {
		let name = socket_path.to_fs_name::<GenericFilePath>()?;
		let stream = <interprocess::local_socket::Stream as interprocess::local_socket::traits::Stream>::connect(name)?;
		configure(&stream)?;
		Ok(Self {
			stream,
			reader: FrameReader::default(),
			responses: HashMap::new(),
			events: Vec::new(),
		})
	}

	/// Read every frame currently available and route it into the buffers.
	fn read_frames(&mut self) -> Result<(), ToolError> {
		read_available(&mut self.stream, &mut self.reader)?;
		while let Some(frame) = self.reader.next().map_err(|message| ToolError::Internal { message })? {
			match frame {
				Frame::Response(outcome) => {
					let (id, result) = match outcome {
						ToolOutcome::Ok { id, result } => (id, Ok(result)),
						ToolOutcome::Err { id, error } => (id, Err(error)),
					};
					self.responses.insert(id, result);
				}
				Frame::Event(event) => self.events.push(event),
				Frame::Request { .. } | Frame::Cancel { .. } => {
					return Err(ToolError::Internal {
						message: "the attached peer sent an agent-to-desktop frame to the agent".to_string(),
					});
				}
			}
		}
		Ok(())
	}
}

impl EditorBridge for AttachedBridge {
	fn submit(&mut self, id: QueryId, query: BridgeQuery) -> Result<(), ToolError> {
		write_frame(&mut self.stream, &Frame::Request { id, query })
	}

	fn poll(&mut self, id: QueryId) -> Option<Result<serde_json::Value, ToolError>> {
		if let Err(error) = self.read_frames() {
			return Some(Err(error));
		}
		self.responses.remove(&id)
	}

	fn cancel(&mut self, id: QueryId) {
		let _ = write_frame(&mut self.stream, &Frame::Cancel { id });
	}

	fn drain_events(&mut self) -> Vec<AgentEvent> {
		let _ = self.read_frames();
		std::mem::take(&mut self.events)
	}

	fn pump(&mut self) -> Pin<Box<dyn Future<Output = Result<(), ToolError>> + '_>> {
		Box::pin(async move {
			self.read_frames()?;
			// The desktop is a separate process; give it a chance to run without
			// busy-spinning the single-threaded host runtime.
			tokio::task::yield_now().await;
			Ok(())
		})
	}
}

// ---------------------------------------------------------------------------
// Test-side server (T4.4, T4.9)
// ---------------------------------------------------------------------------

/// Commands the test drives the scripted server with.
enum ServerCommand {
	/// Simulate a human edit: mutate the live state and emit `count` raw
	/// `DocumentChanged` events.
	HumanEdit {
		document: u64,
		count: usize,
	},
	/// Toggle interleaving a human mutation + event into every request.
	Interleave(bool),
	Shutdown,
}

/// The simulated live-editor state. This is deliberately independent of the real
/// editor: the desktop peer cannot be compiled or run in this environment, so the
/// conformance test exercises [`AttachedBridge`] against a server implementing the
/// same framing and the same `ToolOutcome` shapes.
struct LiveState {
	documents: std::collections::BTreeMap<u64, LiveDocument>,
	active: Option<u64>,
	next_document: u64,
	next_node: u64,
	/// One entry per committed (or standalone) mutation: the pre-mutation node map.
	history: Vec<std::collections::BTreeMap<u64, String>>,
	redo: Vec<std::collections::BTreeMap<u64, String>>,
	/// Set between `BeginTransaction` and `Commit`/`Abort`.
	transaction: Option<std::collections::BTreeMap<u64, String>>,
	interleave: bool,
}

struct LiveDocument {
	name: String,
	nodes: std::collections::BTreeMap<u64, String>,
	selection: Vec<u64>,
}

impl LiveState {
	fn new() -> Self {
		let mut documents = std::collections::BTreeMap::new();
		// Seed one document with a node the agent did not create, so
		// `session.snapshot` can read state the agent did not cause.
		let mut nodes = std::collections::BTreeMap::new();
		nodes.insert(1000u64, "graphene_core::raster::OpacityNode".to_string());
		documents.insert(
			1u64,
			LiveDocument {
				name: "live".to_string(),
				nodes,
				selection: vec![1000],
			},
		);
		Self {
			documents,
			active: Some(1),
			next_document: 2,
			next_node: 2000,
			history: Vec::new(),
			redo: Vec::new(),
			transaction: None,
			interleave: false,
		}
	}

	fn document(&self, document: u64) -> Option<&LiveDocument> {
		self.documents.get(&document)
	}

	fn nodes(&self, document: u64) -> std::collections::BTreeMap<u64, String> {
		self.documents.get(&document).map(|document| document.nodes.clone()).unwrap_or_default()
	}

	/// Apply a mutation, recording history unless a transaction is open.
	fn mutate(&mut self, document: u64, apply: impl FnOnce(&mut LiveDocument)) {
		let before = self.nodes(document);
		if let Some(document) = self.documents.get_mut(&document) {
			apply(document);
		}
		if self.transaction.is_none() {
			self.history.push(before);
			self.redo.clear();
		}
	}

	fn list_json(&self) -> serde_json::Value {
		let documents: Vec<serde_json::Value> = self
			.documents
			.iter()
			.map(|(id, document)| {
				serde_json::json!({
					"document_id": id,
					"name": document.name,
					"node_count": document.nodes.len(),
					"is_active": self.active == Some(*id),
				})
			})
			.collect();
		serde_json::json!({ "documents": documents })
	}

	fn node_json(&self, document: u64, node_id: u64) -> Option<serde_json::Value> {
		let identifier = self.document(document)?.nodes.get(&node_id)?;
		Some(serde_json::json!({
			"node_id": node_id,
			"identifier": identifier,
			"visible": true,
			"input_count": 1,
		}))
	}

	fn node_list_json(&self, document: u64) -> serde_json::Value {
		let nodes: Vec<serde_json::Value> = self
			.document(document)
			.map(|document| {
				document
					.nodes
					.iter()
					.map(|(node_id, identifier)| serde_json::json!({ "node_id": node_id, "identifier": identifier, "visible": true, "input_count": 1 }))
					.collect()
			})
			.unwrap_or_default();
		serde_json::json!({ "document_id": document, "nodes": nodes })
	}

	fn handle(&mut self, query: BridgeQuery) -> Result<serde_json::Value, ToolError> {
		use graphite_agent_protocol::{DocumentOperation, SnapshotProjection};
		match query {
			BridgeQuery::Snapshot { document, projection } => match projection {
				SnapshotProjection::DocumentList => Ok(self.list_json()),
				SnapshotProjection::ActiveDocument => Ok(serde_json::json!({ "document_id": self.active })),
				SnapshotProjection::DocumentSummary => {
					let document = document.ok_or_else(missing_document)?;
					let handler = self.document(document).ok_or_else(|| not_found(document))?;
					Ok(serde_json::json!({
						"document_id": document,
						"name": handler.name,
						"node_count": handler.nodes.len(),
						"is_active": self.active == Some(document),
					}))
				}
				SnapshotProjection::NodeList => Ok(self.node_list_json(document.ok_or_else(missing_document)?)),
				SnapshotProjection::Node { node_id } => {
					let document = document.ok_or_else(missing_document)?;
					self.node_json(document, node_id).ok_or_else(|| ToolError::NotFound {
						what: format!("node {node_id} in document {document}"),
					})
				}
				SnapshotProjection::Selection => {
					let document = document.ok_or_else(missing_document)?;
					let selected = self.document(document).map(|handler| handler.selection.clone()).unwrap_or_default();
					Ok(serde_json::json!({ "document_id": document, "selected_node_ids": selected }))
				}
				_ => Err(ToolError::InvalidArguments {
					message: "unsupported SnapshotProjection variant".to_string(),
				}),
			},
			BridgeQuery::Document { operation } => match operation {
				DocumentOperation::New { name } => {
					let document = self.next_document;
					self.next_document += 1;
					self.documents.insert(
						document,
						LiveDocument {
							name,
							nodes: std::collections::BTreeMap::new(),
							selection: Vec::new(),
						},
					);
					self.active = Some(document);
					Ok(serde_json::json!({ "requested": true, "document_id": document }))
				}
				DocumentOperation::Open { .. } => Ok(serde_json::json!({ "requested": true })),
				DocumentOperation::Close { document } => {
					self.documents.remove(&document);
					if self.active == Some(document) {
						self.active = self.documents.keys().next().copied();
					}
					Ok(serde_json::json!({ "requested": true }))
				}
				DocumentOperation::ExportGdd { .. } => Ok(serde_json::json!({ "gdd_base64": "" })),
				_ => Err(ToolError::InvalidArguments {
					message: "unsupported DocumentOperation variant".to_string(),
				}),
			},
			BridgeQuery::Operation { document, operation } => self.operation(document, operation),
			_ => Err(ToolError::InvalidArguments {
				message: "unsupported BridgeQuery variant".to_string(),
			}),
		}
	}

	fn operation(&mut self, document: u64, operation: graphite_agent_protocol::AgentOperation) -> Result<serde_json::Value, ToolError> {
		use graphite_agent_protocol::AgentOperation;
		match operation {
			AgentOperation::AddNode { identifier, .. } => {
				let node_id = self.next_node;
				self.next_node += 1;
				self.mutate(document, |document| {
					document.nodes.insert(node_id, identifier);
				});
				Ok(serde_json::json!({ "node_id": node_id }))
			}
			AgentOperation::RemoveNode { node_id } => {
				self.mutate(document, |document| {
					document.nodes.remove(&node_id);
				});
				Ok(serde_json::json!({ "node_id": node_id, "removed": true }))
			}
			AgentOperation::SetInput { node_id, .. } => Ok(serde_json::json!({ "node_id": node_id })),
			AgentOperation::Connect { .. } => Ok(serde_json::json!({ "connected": true })),
			AgentOperation::Disconnect { .. } => Ok(serde_json::json!({ "disconnected": true })),
			AgentOperation::BeginTransaction => {
				self.transaction = Some(self.nodes(document));
				Ok(serde_json::json!({ "ok": true }))
			}
			AgentOperation::CommitTransaction => {
				if let Some(before) = self.transaction.take() {
					self.history.push(before);
					self.redo.clear();
				}
				Ok(serde_json::json!({ "ok": true }))
			}
			AgentOperation::AbortTransaction => {
				if let Some(before) = self.transaction.take() {
					let nodes = before;
					if let Some(document) = self.documents.get_mut(&document) {
						document.nodes = nodes;
					}
				}
				Ok(serde_json::json!({ "ok": true }))
			}
			AgentOperation::Undo => {
				if let Some(previous) = self.history.pop() {
					let current = self.nodes(document);
					if let Some(document) = self.documents.get_mut(&document) {
						document.nodes = previous;
					}
					self.redo.push(current);
				}
				Ok(serde_json::json!({ "changed": true }))
			}
			AgentOperation::Redo => {
				if let Some(next) = self.redo.pop() {
					let current = self.nodes(document);
					if let Some(document) = self.documents.get_mut(&document) {
						document.nodes = next;
					}
					self.history.push(current);
				}
				Ok(serde_json::json!({ "changed": true }))
			}
			_ => Err(ToolError::InvalidArguments {
				message: "unsupported AgentOperation variant".to_string(),
			}),
		}
	}
}

fn missing_document() -> ToolError {
	ToolError::InvalidArguments {
		message: "this projection requires a document id".to_string(),
	}
}

fn not_found(document: u64) -> ToolError {
	ToolError::NotFound { what: format!("document {document}") }
}

/// A test-side socket server implementing the mirrored framing, so
/// [`AttachedBridge`] is genuinely exercised without the desktop (T4.4/T4.9).
///
/// It simulates a live editor session: one seeded document containing a
/// human-created node, an undo stack, and the ability to inject unsolicited
/// [`AgentEvent::DocumentChanged`] frames.
pub struct ScriptedBridgeServer {
	socket_path: PathBuf,
	state: Arc<Mutex<LiveState>>,
	commands: std::sync::mpsc::Sender<ServerCommand>,
	thread: Option<std::thread::JoinHandle<()>>,
}

impl ScriptedBridgeServer {
	/// Bind a listener at `socket_path` and serve one agent connection. The
	/// listener is bound before this returns, so the agent may connect immediately.
	pub fn start(socket_path: &Path) -> std::io::Result<Self> {
		// A stale socket file from an earlier run would make `bind` fail.
		let _ = std::fs::remove_file(socket_path);
		let name = socket_path.to_fs_name::<GenericFilePath>()?;
		let listener = ListenerOptions::new().name(name).nonblocking(ListenerNonblockingMode::Accept).try_overwrite(true).create_sync()?;

		let state = Arc::new(Mutex::new(LiveState::new()));
		let (commands, receiver) = std::sync::mpsc::channel();
		let path = socket_path.to_path_buf();

		let thread_state = Arc::clone(&state);
		let thread = std::thread::Builder::new().name("scripted-bridge-server".to_string()).spawn(move || {
			if let Err(error) = serve(listener, &thread_state, &receiver) {
				eprintln!("scripted-bridge-server: {error}");
			}
		})?;

		Ok(Self {
			socket_path: path,
			state,
			commands,
			thread: Some(thread),
		})
	}

	/// The path the agent must be told to attach to.
	pub fn socket_path(&self) -> &Path {
		&self.socket_path
	}

	/// Simulate a human editing `document`: mutate the live state and emit
	/// `count` raw `DocumentChanged` frames as fast as possible (so the agent-side
	/// debounce collapses them into one notification).
	pub fn human_edit(&self, document: u64, count: usize) {
		if let Ok(mut state) = self.state.lock() {
			let node_id = state.next_node;
			state.next_node += 1;
			state.mutate(document, |live| {
				live.nodes.insert(node_id, "human::Edit".to_string());
			});
		}
		let _ = self.commands.send(ServerCommand::HumanEdit { document, count });
	}

	/// Interleave a human mutation plus an event into every subsequent request.
	pub fn interleave_human_mutations(&self, enabled: bool) {
		let _ = self.commands.send(ServerCommand::Interleave(enabled));
	}

	/// The node ids currently in `document` according to the simulated editor.
	pub fn node_ids(&self, document: u64) -> Vec<u64> {
		self.state.lock().map(|state| state.nodes(document).keys().copied().collect()).unwrap_or_default()
	}
}

impl Drop for ScriptedBridgeServer {
	fn drop(&mut self) {
		let _ = self.commands.send(ServerCommand::Shutdown);
		if let Some(thread) = self.thread.take() {
			let _ = thread.join();
		}
	}
}

fn serve(listener: interprocess::local_socket::Listener, state: &Arc<Mutex<LiveState>>, commands: &std::sync::mpsc::Receiver<ServerCommand>) -> std::io::Result<()> {
	let mut stream = loop {
		match listener.accept() {
			Ok(stream) => break stream,
			Err(error) if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::Interrupted) => {
				match commands.try_recv() {
					Ok(ServerCommand::Shutdown) | Err(std::sync::mpsc::TryRecvError::Disconnected) => return Ok(()),
					_ => {}
				}
				std::thread::sleep(Duration::from_millis(1));
			}
			Err(error) => return Err(error),
		}
	};
	configure(&stream)?;

	let mut reader = FrameReader::default();
	loop {
		// Deliver queued commands (injected events, interleave toggles).
		loop {
			match commands.try_recv() {
				Ok(ServerCommand::Shutdown) => return Ok(()),
				Ok(ServerCommand::Interleave(enabled)) => {
					if let Ok(mut state) = state.lock() {
						state.interleave = enabled;
					}
				}
				Ok(ServerCommand::HumanEdit { document, count }) => {
					for _ in 0..count {
						write_frame_io(&mut stream, &Frame::Event(AgentEvent::DocumentChanged { document }))?;
					}
				}
				Err(std::sync::mpsc::TryRecvError::Empty) => break,
				Err(std::sync::mpsc::TryRecvError::Disconnected) => return Ok(()),
			}
		}

		let mut chunk = [0u8; 8192];
		match stream.read(&mut chunk) {
			Ok(0) => return Ok(()),
			Ok(count) => reader.push(&chunk[..count]),
			Err(error) if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut | ErrorKind::Interrupted) => continue,
			Err(error) => return Err(error),
		}

		while let Some(frame) = reader.next().map_err(|message| std::io::Error::new(ErrorKind::InvalidData, message))? {
			let Frame::Request { id, query } = frame else {
				continue;
			};

			// Optionally interleave a human-side mutation + event before replying,
			// so the agent must correlate despite unsolicited frames.
			let mut state_guard = state.lock().expect("scripted server state poisoned");
			if state_guard.interleave {
				let node_id = state_guard.next_node;
				state_guard.next_node += 1;
				let document = state_guard.active.unwrap_or(1);
				state_guard.mutate(document, |live| {
					live.nodes.insert(node_id, "human::Interleaved".to_string());
				});
				drop(state_guard);
				write_frame_io(&mut stream, &Frame::Event(AgentEvent::DocumentChanged { document }))?;
				state_guard = state.lock().expect("scripted server state poisoned");
			}

			let outcome = match state_guard.handle(query) {
				Ok(result) => ToolOutcome::Ok { id, result },
				Err(error) => ToolOutcome::Err { id, error },
			};
			drop(state_guard);
			write_frame_io(&mut stream, &Frame::Response(outcome))?;
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use graphite_agent_protocol::{CapabilitySet, SnapshotProjection, ToolHost, ToolRequest};
	use serde_json::json;
	use std::time::Instant;

	fn temp_socket(label: &str) -> PathBuf {
		// Unix socket paths are length-limited; keep the name short.
		std::env::temp_dir().join(format!("ga-attached-{}-{label}.sock", std::process::id()))
	}

	fn bridge_to(server: &ScriptedBridgeServer) -> AttachedBridge {
		// The listener is already bound before the agent connects.
		AttachedBridge::connect(server.socket_path()).expect("connect to scripted server")
	}

	#[test]
	fn framing_round_trips_through_the_length_prefix() {
		for frame in [
			Frame::Request {
				id: 7,
				query: BridgeQuery::Snapshot {
					document: None,
					projection: SnapshotProjection::ActiveDocument,
				},
			},
			Frame::Response(ToolOutcome::Ok {
				id: 7,
				result: json!({ "document_id": 1 }),
			}),
			Frame::Event(AgentEvent::DocumentChanged { document: 1 }),
			Frame::Cancel { id: 9 },
		] {
			let encoded = encode_frame(&frame).expect("encode");
			let length = u32::from_be_bytes([encoded[0], encoded[1], encoded[2], encoded[3]]);
			assert_eq!(length as usize, encoded.len() - 4);
			let mut reader = FrameReader::default();
			reader.push(&encoded);
			assert_eq!(reader.next().expect("decode"), Some(frame));
			assert!(reader.next().expect("exhausted").is_none());
		}
	}

	#[test]
	fn partial_frames_are_buffered_until_complete() {
		let frame = Frame::Event(AgentEvent::DocumentChanged { document: 3 });
		let encoded = encode_frame(&frame).expect("encode");
		let mut reader = FrameReader::default();
		reader.push(&encoded[..2]);
		assert!(reader.next().expect("incomplete").is_none());
		reader.push(&encoded[2..]);
		assert_eq!(reader.next().expect("complete"), Some(frame));
	}

	#[test]
	fn live_session_read_and_correlation() {
		let server = ScriptedBridgeServer::start(&temp_socket("live")).expect("start server");
		let mut bridge = bridge_to(&server);

		// Read state the agent did not cause: the seeded human node.
		bridge
			.submit(
				1,
				BridgeQuery::Snapshot {
					document: Some(1),
					projection: SnapshotProjection::NodeList,
				},
			)
			.expect("submit");
		let deadline = Instant::now() + Duration::from_secs(5);
		let result = loop {
			bridge.pump_ready();
			if let Some(result) = bridge.poll(1) {
				break result.expect("node list");
			}
			assert!(Instant::now() < deadline, "the scripted server did not answer the snapshot");
			std::thread::sleep(Duration::from_millis(2));
		};
		let nodes = result["nodes"].as_array().expect("nodes").clone();
		assert_eq!(nodes.len(), 1);
		assert_eq!(nodes[0]["node_id"], 1000);
		assert_eq!(nodes[0]["identifier"], "graphene_core::raster::OpacityNode");
	}

	#[test]
	fn human_edits_arrive_as_events_and_are_debounced_by_the_host() {
		let server = ScriptedBridgeServer::start(&temp_socket("events")).expect("start server");
		let mut bridge = bridge_to(&server);
		server.human_edit(1, 3);

		let deadline = Instant::now() + Duration::from_secs(2);
		let mut collected = Vec::new();
		while Instant::now() < deadline && collected.is_empty() {
			collected.extend(bridge.drain_events());
			std::thread::sleep(Duration::from_millis(5));
		}
		assert!(!collected.is_empty(), "the human edit must arrive as at least one event");
		assert!(collected.iter().all(|event| matches!(event, AgentEvent::DocumentChanged { document } if *document == 1)));
	}

	/// A tiny helper so the tests can drive the bridge without the host's loop.
	impl AttachedBridge {
		fn pump_ready(&mut self) {
			let _ = self.read_frames();
		}
	}

	/// The host-side capability gate must refuse a tool the connection lacks.
	#[test]
	fn capability_refusal_is_unauthorized() {
		use crate::host::Host;
		use graphite_agent_protocol::Capability;

		let server = ScriptedBridgeServer::start(&temp_socket("caps")).expect("start server");
		let bridge = bridge_to(&server);
		let host = Host::new(
			Box::new(bridge),
			CapabilitySet(vec![Capability::Read, Capability::Author]),
			Duration::from_secs(5),
			std::env::temp_dir(),
		);
		let pending = host.call(ToolRequest {
			name: "render.preview".to_string(),
			arguments: json!({ "document_id": 1 }),
			document: None,
		});
		let outcome = futures::executor::block_on(pending.outcome);
		assert!(matches!(
			outcome,
			ToolOutcome::Err {
				error: ToolError::Unauthorized { capability: Capability::Execute },
				..
			}
		));
	}
}
