use crate::agent_bridge::{self, AgentBridgeEndpoints};
use crate::consts::APP_SOCKET_FILE_NAME;
use crate::event::{AppEvent, AppEventScheduler};
use crate::wrapper::graphite_agent_protocol::{AgentEvent, BridgeQuery, QueryId, ToolOutcome};
use interprocess::local_socket::{GenericFilePath, GenericNamespaced, ListenerNonblockingMode, ListenerOptions, Name, prelude::*};
use std::io::{ErrorKind, Read, Write};
use std::sync::Arc;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

// TODO: Needs to be integrated/replaced with the action system.
// TODO: At that point this should just wrap the action, meaning all actions bindable by the user can also be accessed via the socket.
#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) enum Message {
	OpenFiles(Vec<std::path::PathBuf>),
}

/// Upper bound on a single agent frame body, so a corrupt length prefix cannot make
/// the desktop allocate unbounded memory.
const MAX_FRAME_LEN: u32 = 64 * 1024 * 1024;

/// Bidirectional attached-session frame (T4.1).
///
/// # Wire format (canonical definition — `agent/host/src/attached.rs::Frame` mirrors it)
///
/// ```text
/// frame   = 4-byte big-endian u32 payload length, followed by `payload`
/// payload = RON-encoded `Frame` enum, UTF-8
///
/// enum Frame {
///     Request { id: QueryId, query: BridgeQuery },  // agent -> desktop
///     Response(ToolOutcome),                        // desktop -> agent (correlated by ToolOutcome::id)
///     Event(AgentEvent),                            // desktop -> agent (unsolicited)
///     Cancel { id: QueryId },                       // agent -> desktop
/// }
/// ```
///
/// `Request` carries the host-allocated `QueryId` (INV-14) so the desktop echoes it
/// in the correlated `Response`; that id is what makes the framing *correlated*.
/// `Cancel` is the minimal control frame needed to transport the frozen
/// `EditorBridge::cancel` method.
///
/// The legacy one-way [`Message::OpenFiles`] RON payload is unchanged: its first
/// bytes are `Open`, which can never be a valid frame length (the high byte of a
/// length is zero for every realistic frame), so the two protocols coexist on the
/// same socket.
#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) enum Frame {
	Request { id: QueryId, query: BridgeQuery },
	Response(ToolOutcome),
	Event(AgentEvent),
	Cancel { id: QueryId },
}

fn decode_frame(body: &[u8]) -> std::io::Result<Frame> {
	let text = std::str::from_utf8(body).map_err(|error| std::io::Error::new(ErrorKind::InvalidData, error))?;
	ron::de::from_str(text).map_err(|error| std::io::Error::new(ErrorKind::InvalidData, error))
}

fn encode_frame(frame: &Frame) -> std::io::Result<Vec<u8>> {
	let body = ron::ser::to_string(frame).map_err(|error| std::io::Error::new(ErrorKind::InvalidData, error))?;
	let length = u32::try_from(body.len()).map_err(|_| std::io::Error::new(ErrorKind::InvalidData, "frame is too large to encode"))?;
	let mut encoded = Vec::with_capacity(4 + body.len());
	encoded.extend_from_slice(&length.to_be_bytes());
	encoded.extend_from_slice(body.as_bytes());
	Ok(encoded)
}

/// Write one frame: 4-byte big-endian length + RON body.
pub(crate) fn write_frame<W: Write>(writer: &mut W, frame: &Frame) -> std::io::Result<()> {
	writer.write_all(&encode_frame(frame)?)
}

/// Read a frame body of `length` bytes (the header was already consumed).
pub(crate) fn read_frame_payload<R: Read>(reader: &mut R, length: u32) -> std::io::Result<Option<Frame>> {
	if length > MAX_FRAME_LEN {
		return Err(std::io::Error::new(ErrorKind::InvalidData, "agent frame exceeds the size limit"));
	}
	let mut body = vec![0u8; length as usize];
	match reader.read_exact(&mut body) {
		Ok(()) => Ok(Some(decode_frame(&body)?)),
		Err(error) if error.kind() == ErrorKind::UnexpectedEof => Ok(None),
		Err(error) => Err(error),
	}
}

/// Buffered length-prefixed frame reader, for non-blocking connections.
#[derive(Default)]
pub(crate) struct FrameDecoder {
	buffer: Vec<u8>,
}

impl FrameDecoder {
	pub(crate) fn push(&mut self, bytes: &[u8]) {
		self.buffer.extend_from_slice(bytes);
	}

	pub(crate) fn next(&mut self) -> std::io::Result<Option<Frame>> {
		if self.buffer.len() < 4 {
			return Ok(None);
		}
		let length = u32::from_be_bytes([self.buffer[0], self.buffer[1], self.buffer[2], self.buffer[3]]);
		if length > MAX_FRAME_LEN {
			return Err(std::io::Error::new(ErrorKind::InvalidData, "agent frame exceeds the size limit"));
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

fn handle_message(message: Message, app_event_scheduler: &AppEventScheduler) {
	match message {
		Message::OpenFiles(paths) => {
			app_event_scheduler.schedule(AppEvent::OpenFiles(paths));
		}
	}
}

pub(crate) fn send(message: Message) -> std::io::Result<()> {
	let data = ron::ser::to_string(&message).map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
	let mut connection = interprocess::local_socket::Stream::connect(socket_name())?;
	connection.write_all(data.as_bytes())
}

/// Handle one accepted connection.
///
/// Legacy behavior is fully preserved: an [`Message::OpenFiles`] sender writes a bare
/// RON payload whose first bytes are `Open`. Anything else is the new framed agent
/// protocol, which requires `--agent-bridge`.
fn handle_connection(mut connection: interprocess::local_socket::Stream, app_event_scheduler: AppEventScheduler, agent_bridge: Option<Arc<AgentBridgeEndpoints>>) {
	let mut header = [0u8; 4];
	if let Err(error) = connection.read_exact(&mut header) {
		tracing::error!("Failed to read the socket message header: {}", error);
		return;
	}

	// Legacy one-way `OpenFiles` payload (unchanged).
	if &header == b"Open" {
		let mut data = String::from("Open");
		if let Err(error) = connection.read_to_string(&mut data) {
			tracing::error!("Failed to read socket message: {}", error);
			return;
		}
		match ron::de::from_str(&data) {
			Ok(message) => handle_message(message, &app_event_scheduler),
			Err(error) => tracing::error!("Failed to deserialize socket message: {}", error),
		}
		return;
	}

	let Some(agent_bridge) = agent_bridge else {
		tracing::warn!("Received an agent frame but the agent bridge is disabled (--agent-bridge)");
		return;
	};
	let length = u32::from_be_bytes(header);
	match read_frame_payload(&mut connection, length) {
		Ok(Some(frame)) => agent_bridge::serve_connection(connection, frame, app_event_scheduler, agent_bridge),
		Ok(None) => {}
		Err(error) => tracing::error!("Failed to read the first agent frame: {}", error),
	}
}

pub(crate) struct SocketHandle {
	thread: Option<thread::JoinHandle<()>>,
	shutdown_sender: mpsc::Sender<()>,
}
impl Drop for SocketHandle {
	fn drop(&mut self) {
		let _ = self.shutdown_sender.send(());
		let _ = self.thread.take().expect("SocketHandle can only be dropped once").join();
	}
}

pub(crate) fn start(app_event_scheduler: AppEventScheduler, agent_bridge: Option<Arc<AgentBridgeEndpoints>>) -> SocketHandle {
	let (shutdown_sender, shutdown_receiver) = mpsc::channel();

	let thread = thread::Builder::new()
		.name("socket".to_string())
		.spawn(move || run(app_event_scheduler, shutdown_receiver, agent_bridge))
		.expect("Failed to spawn socket thread");

	SocketHandle {
		shutdown_sender,
		thread: Some(thread),
	}
}

fn run(app_event_scheduler: AppEventScheduler, shutdown_receiver: mpsc::Receiver<()>, agent_bridge: Option<Arc<AgentBridgeEndpoints>>) {
	let listener = match ListenerOptions::new()
		.name(socket_name())
		.nonblocking(ListenerNonblockingMode::Accept)
		.try_overwrite(true)
		.max_spin_time(Duration::from_millis(100))
		.create_sync()
	{
		Ok(listener) => listener,
		Err(error) => {
			tracing::error!("Failed to bind socket: {}", error);
			return;
		}
	};

	let max_backoff = Duration::from_millis(100);
	let mut backoff = Duration::ZERO;

	loop {
		if backoff.is_zero() {
			match shutdown_receiver.try_recv() {
				Ok(()) | Err(mpsc::TryRecvError::Disconnected) => break,
				Err(mpsc::TryRecvError::Empty) => {}
			}
			backoff = Duration::from_nanos(1);
		} else {
			match shutdown_receiver.recv_timeout(backoff) {
				Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
				Err(mpsc::RecvTimeoutError::Timeout) => {}
			}
			backoff = (backoff * 2).min(max_backoff);
		}

		match listener.accept() {
			Ok(connection) => {
				backoff = Duration::ZERO;

				let app_event_scheduler = app_event_scheduler.clone();
				let agent_bridge = agent_bridge.clone();
				let spawn_result = thread::Builder::new().name("socket-connection".to_string()).spawn(move || {
					handle_connection(connection, app_event_scheduler, agent_bridge);
				});
				if let Err(error) = spawn_result {
					tracing::error!("Failed to spawn socket connection thread: {}", error);
				}
			}
			Err(error) if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::Interrupted) => {}
			Err(error) => {
				tracing::error!("Failed to accept socket connection: {}", error);
			}
		}
	}
}

fn socket_name() -> Name<'static> {
	if cfg!(target_os = "windows") {
		let user = std::env::var("USERNAME").unwrap_or_default();
		let name = format!("{user}-{app}-{APP_SOCKET_FILE_NAME}", app = crate::consts::APP_NAME);
		name.to_ns_name::<GenericNamespaced>().expect("valid named pipe name")
	} else {
		crate::dirs::app_data_dir().join(APP_SOCKET_FILE_NAME).to_fs_name::<GenericFilePath>().expect("valid socket path")
	}
}
