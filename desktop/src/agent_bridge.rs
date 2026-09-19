//! Desktop-side agent bridge (T4.3): relays the editor's reply sink to the socket
//! and dispatches inbound curated `AgentMessage`s into the editor.
//!
//! # Framing (mirror of `agent/host/src/attached.rs::Frame`)
//!
//! The canonical wire definition lives in [`crate::socket::Frame`]; see its doc
//! comment. This module only moves frames:
//!
//! * inbound [`Frame::Request`] / [`Frame::Cancel`] become [`AppEvent::AgentMessage`]
//!   on the winit event loop (the editor is `!Send` and lives there);
//! * outbound [`ToolOutcome`] replies come from the `AgentReplySink` installed on
//!   the editor and are written as [`Frame::Response`];
//! * editor changes signalled by [`AgentBridgeHandle::editor_changed`] are forwarded
//!   as [`Frame::Event`]. The ≤ 1-per-100 ms debounce is authoritative on the agent
//!   side (`graphite-agent-host`), where it is testable (T4.6); this is only the
//!   minimal desktop hook.
//!
//! Message system only: the bridge never deserializes an arbitrary `Message`
//! (INV-2, INV-13).

use crate::event::{AppEvent, AppEventScheduler};
use crate::socket::{Frame, FrameDecoder, write_frame};
use crate::wrapper::AgentMessage;
use crate::wrapper::graphite_agent_protocol::{AgentEvent, BridgeQuery, QueryId, ToolOutcome};
use futures::channel::mpsc::UnboundedReceiver;
use interprocess::local_socket::prelude::*;
use std::io::{ErrorKind, Read};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

/// The per-connection state a single agent connection owns: the editor's reply
/// receiver and the editor-change signal receiver. Both are taken exactly once.
pub(crate) struct AgentBridgeSession {
	pub(crate) replies: UnboundedReceiver<ToolOutcome>,
	pub(crate) changed: mpsc::Receiver<()>,
}

/// Server-side endpoints shared with the socket listener. Exactly one agent
/// connection can own the session at a time.
#[derive(Default)]
pub(crate) struct AgentBridgeEndpoints {
	session: Mutex<Option<AgentBridgeSession>>,
}

impl AgentBridgeEndpoints {
	pub(crate) fn new(replies: UnboundedReceiver<ToolOutcome>, changed: mpsc::Receiver<()>) -> Self {
		Self {
			session: Mutex::new(Some(AgentBridgeSession { replies, changed })),
		}
	}

	fn take_session(&self) -> Option<AgentBridgeSession> {
		self.session.lock().ok()?.take()
	}
}

/// The editor-side handle the app uses to report editor changes (T4.6).
#[derive(Clone)]
pub(crate) struct AgentBridgeHandle {
	changed: mpsc::Sender<()>,
}

impl AgentBridgeHandle {
	pub(crate) fn new(changed: mpsc::Sender<()>) -> Self {
		Self { changed }
	}

	/// Called after the app dispatches an editor message. The signal is coalesced by
	/// the connection loop and debounced by the agent; sending is non-blocking.
	pub(crate) fn editor_changed(&self) {
		let _ = self.changed.send(());
	}
}

/// Serve one agent connection until it closes.
///
/// The stream is put in non-blocking mode so one thread can read requests while
/// writing replies and events.
pub(crate) fn serve_connection(mut connection: interprocess::local_socket::Stream, first_frame: Frame, app_event_scheduler: AppEventScheduler, endpoints: Arc<AgentBridgeEndpoints>) {
	let Some(session) = endpoints.take_session() else {
		tracing::warn!("An agent bridge connection is already active; dropping the new one");
		return;
	};
	if let Err(error) = connection.set_nonblocking(true) {
		tracing::error!("Failed to put the agent bridge socket into non-blocking mode: {error}");
		return;
	}

	let AgentBridgeSession { mut replies, changed } = session;
	let mut decoder = FrameDecoder::default();
	let mut last_document: Option<u64> = None;
	let mut pending_changes = 0usize;

	dispatch_frame(first_frame, &app_event_scheduler, &mut last_document);

	let mut chunk = [0u8; 8192];
	loop {
		// 1. Forward coalesced editor changes as DocumentChanged events, but only once
		//    the session has revealed which document is being edited.
		while changed.try_recv().is_ok() {
			pending_changes += 1;
		}
		if pending_changes > 0 {
			pending_changes = 0;
			if let Some(document) = last_document
				&& let Err(error) = write_frame(&mut connection, &Frame::Event(AgentEvent::DocumentChanged { document }))
			{
				tracing::error!("Failed to write an agent event: {error}");
				break;
			}
		}

		// 2. Forward correlated tool outcomes from the editor's reply sink.
		while let Ok(Some(outcome)) = replies.try_next() {
			if let Err(error) = write_frame(&mut connection, &Frame::Response(outcome)) {
				tracing::error!("Failed to write an agent response: {error}");
				return;
			}
		}

		// 3. Read whatever the agent has sent.
		match connection.read(&mut chunk) {
			Ok(0) => break,
			Ok(count) => decoder.push(&chunk[..count]),
			Err(error) if error.kind() == ErrorKind::WouldBlock => {}
			Err(error) => {
				tracing::error!("Failed to read from the agent bridge socket: {error}");
				break;
			}
		}
		while let Ok(Some(frame)) = decoder.next() {
			dispatch_frame(frame, &app_event_scheduler, &mut last_document);
		}

		std::thread::sleep(Duration::from_millis(2));
	}
}

/// Turn a curated inbound frame into an editor message on the event loop.
fn dispatch_frame(frame: Frame, scheduler: &AppEventScheduler, last_document: &mut Option<u64>) {
	match frame {
		Frame::Request { id, query } => {
			if let Some(document) = document_of(&query) {
				*last_document = Some(document);
			}
			if let Some(message) = agent_message(id, query) {
				scheduler.schedule(AppEvent::AgentMessage(message));
			} else {
				tracing::error!("Dropping an unsupported agent bridge query");
			}
		}
		Frame::Cancel { id } => scheduler.schedule(AppEvent::AgentMessage(AgentMessage::Cancel { id })),
		Frame::Response(_) | Frame::Event(_) => tracing::error!("Received an agent-to-desktop frame from the agent"),
	}
}

/// Map a curated query onto the editor's curated `AgentMessage` (INV-13).
fn agent_message(id: QueryId, query: BridgeQuery) -> Option<AgentMessage> {
	match query {
		BridgeQuery::Document { operation } => Some(AgentMessage::Document { id, operation }),
		BridgeQuery::Operation { document, operation } => Some(AgentMessage::Execute { id, document, operation }),
		BridgeQuery::Snapshot { document, projection } => Some(AgentMessage::Snapshot { id, document, projection }),
		// Forward-compatible: later phases may add protocol variants (additive only, §5).
		_ => None,
	}
}

/// The document a query addresses, if any; used to attribute change events.
fn document_of(query: &BridgeQuery) -> Option<u64> {
	match query {
		BridgeQuery::Operation { document, .. } => Some(*document),
		BridgeQuery::Snapshot { document, .. } => *document,
		BridgeQuery::Document { .. } => None,
		_ => None,
	}
}
