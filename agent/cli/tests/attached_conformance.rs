//! T4.9 — attached-mode conformance test (Surface B).
//!
//! The desktop peer cannot be compiled or run in this environment, so this test
//! starts the test-side server [`ScriptedBridgeServer`] (which implements the same
//! mirrored framing as `desktop/src/socket.rs`) and drives the shipped
//! `graphite-agent --mode attached` binary against it.
//!
//! Assertions (phase 4 gates 3–7, 11):
//! * live-session read of state the agent did not cause;
//! * atomic undo of an agent transaction;
//! * correlated responses under an interleaved human-side mutation;
//! * one debounced `DocumentChanged` notification;
//! * `Unauthorized` on a capability-refused call.

use graphite_agent_host::ScriptedBridgeServer;
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const RESPONSE_TIMEOUT: Duration = Duration::from_secs(60);

/// The §17 curated tools plus the two catalog tools plus the three Phase 4 session
/// tools plus the four Phase 5 registry tools, matching `mcp_conformance.rs`.
const EXPECTED_TOOLS: &[&str] = &[
	"document.new",
	"document.open",
	"document.save",
	"document.close",
	"document.list",
	"node.list_types",
	"node.describe",
	"graph.add_node",
	"graph.remove_node",
	"graph.set_input",
	"graph.connect",
	"graph.disconnect",
	"graph.list_nodes",
	"graph.get_node",
	"history.undo",
	"history.redo",
	"history.begin",
	"history.commit",
	"history.abort",
	"render.preview",
	"render.export",
	"session.snapshot",
	"session.selection",
	"session.active_document",
	"registry.apply_delta",
	"registry.query",
	"registry.merge",
	"history.replay",
];

struct ToolReply {
	is_error: bool,
	text: String,
	structured: Value,
}

struct Agent {
	child: Child,
	stdin: ChildStdin,
	responses: Receiver<Value>,
	notifications: Arc<Mutex<Vec<Value>>>,
	next_id: AtomicU64,
}

impl Agent {
	fn spawn(root: &Path, socket: &Path) -> Self {
		let mut child = Command::new(env!("CARGO_BIN_EXE_graphite-agent"))
			.args([
				"--mode",
				"attached",
				"--attach",
				socket.to_str().expect("socket path is UTF-8"),
				"--root",
				root.to_str().expect("root is UTF-8"),
				"--stdio",
				"--timeout-seconds",
				"20",
			])
			.stdin(Stdio::piped())
			.stdout(Stdio::piped())
			.stderr(Stdio::inherit())
			.spawn()
			.expect("failed to spawn graphite-agent in attached mode");

		let stdin = child.stdin.take().expect("stdin");
		let stdout = child.stdout.take().expect("stdout");
		let (sender, responses) = channel();
		let notifications = Arc::new(Mutex::new(Vec::new()));
		let thread_notifications = Arc::clone(&notifications);
		std::thread::spawn(move || {
			for line in BufReader::new(stdout).lines() {
				let Ok(line) = line else { break };
				if line.trim().is_empty() {
					continue;
				}
				let Ok(value) = serde_json::from_str::<Value>(&line) else {
					panic!("stdout must carry JSON-RPC only (INV-10); got {line:?}");
				};
				let is_notification = value.get("id").is_none() && value.get("method").is_some();
				if is_notification {
					thread_notifications.lock().expect("notifications").push(value);
				} else if sender.send(value).is_err() {
					break;
				}
			}
		});

		Self {
			child,
			stdin,
			responses,
			notifications,
			next_id: AtomicU64::new(1),
		}
	}

	fn request(&mut self, method: &str, params: Value) -> Value {
		let id = self.next_id.fetch_add(1, Ordering::Relaxed) as i64;
		self.send(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }));
		self.await_id(id)
	}

	fn notify(&mut self, method: &str, params: Value) {
		self.send(&json!({ "jsonrpc": "2.0", "method": method, "params": params }));
	}

	fn send(&mut self, message: &Value) {
		self.stdin.write_all(message.to_string().as_bytes()).expect("write request");
		self.stdin.write_all(b"\n").expect("write newline");
		self.stdin.flush().expect("flush request");
	}

	fn await_id(&mut self, id: i64) -> Value {
		loop {
			match self.responses.recv_timeout(RESPONSE_TIMEOUT) {
				Ok(value) => {
					if value.get("id").and_then(Value::as_i64) == Some(id) {
						return value;
					}
				}
				Err(RecvTimeoutError::Timeout) => panic!("timed out waiting for JSON-RPC response id {id}"),
				Err(RecvTimeoutError::Disconnected) => panic!("graphite-agent stdout closed while waiting for id {id}"),
			}
		}
	}

	fn initialize(&mut self) {
		let response = self.request(
			"initialize",
			json!({ "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": { "name": "attached-conformance", "version": "1" } }),
		);
		assert_eq!(response["result"]["protocolVersion"], "2025-06-18");
		self.notify("notifications/initialized", json!({}));
	}

	fn call_tool(&mut self, name: &str, arguments: Value) -> ToolReply {
		let response = self.request("tools/call", json!({ "name": name, "arguments": arguments }));
		assert!(response.get("error").is_none(), "tools/call `{name}` returned a JSON-RPC error: {response}");
		let result = &response["result"];
		let is_error = result.get("isError").and_then(Value::as_bool).unwrap_or(false);
		let text = result["content"][0]["text"].as_str().unwrap_or_default().to_string();
		let structured = result.get("structuredContent").cloned().unwrap_or(Value::Null);
		ToolReply { is_error, text, structured }
	}

	/// All `DocumentChanged` notifications received so far.
	fn document_changed_notifications(&self) -> Vec<Value> {
		self.notifications
			.lock()
			.expect("notifications")
			.iter()
			.filter(|notification| notification["method"] == "notifications/message")
			.filter_map(|notification| notification["params"]["data"].get("DocumentChanged").cloned())
			.collect()
	}

	/// Wait until at least one `DocumentChanged` notification has arrived.
	fn wait_for_document_changed(&self, timeout: Duration) -> Vec<Value> {
		let deadline = Instant::now() + timeout;
		loop {
			let seen = self.document_changed_notifications();
			if !seen.is_empty() || Instant::now() >= deadline {
				return seen;
			}
			std::thread::sleep(Duration::from_millis(10));
		}
	}
}

impl Drop for Agent {
	fn drop(&mut self) {
		let _ = self.child.kill();
		let _ = self.child.wait();
	}
}

fn temp_root(label: &str) -> PathBuf {
	let root = std::env::temp_dir().join(format!("graphite-attached-conformance-{}-{label}", std::process::id()));
	let _ = std::fs::remove_dir_all(&root);
	std::fs::create_dir_all(&root).expect("create temp root");
	root
}

fn temp_socket(label: &str) -> PathBuf {
	// Unix socket paths are length-limited; keep the name short.
	std::env::temp_dir().join(format!("ga-att-{}-{label}.sock", std::process::id()))
}

fn node_ids(reply: &ToolReply) -> BTreeSet<u64> {
	reply.structured["nodes"].as_array().into_iter().flatten().filter_map(|node| node["node_id"].as_u64()).collect()
}

#[test]
fn shipped_binary_lists_tools_and_reads_the_live_session() {
	let root = temp_root("list");
	let server = ScriptedBridgeServer::start(&temp_socket("list")).expect("start scripted server");
	let mut agent = Agent::spawn(&root, server.socket_path());
	agent.initialize();

	let listed = agent.request("tools/list", json!({}));
	let names: BTreeSet<String> = listed["result"]["tools"]
		.as_array()
		.expect("tools array")
		.iter()
		.map(|tool| tool["name"].as_str().expect("tool name").to_string())
		.collect();
	let expected: BTreeSet<String> = EXPECTED_TOOLS.iter().map(|name| name.to_string()).collect();
	assert_eq!(names, expected, "attached tools/list must equal the 28-tool surface");

	// Gate 3: read state the agent did not cause (the seeded human node 1000).
	let snapshot = agent.call_tool("session.snapshot", json!({ "projection": "node_list", "document_id": 1 }));
	assert!(!snapshot.is_error, "session.snapshot failed: {}", snapshot.text);
	assert_eq!(node_ids(&snapshot), BTreeSet::from([1000]), "the live session must report the human-created node");

	let active = agent.call_tool("session.active_document", json!({}));
	assert!(!active.is_error, "session.active_document failed: {}", active.text);
	assert_eq!(active.structured["document_id"].as_u64(), Some(1));

	let selection = agent.call_tool("session.selection", json!({ "document_id": 1 }));
	assert!(!selection.is_error, "session.selection failed: {}", selection.text);
	assert_eq!(selection.structured["selected_node_ids"], json!([1000]));
}

#[test]
fn agent_transaction_undoes_as_one_atomic_step() {
	let root = temp_root("undo");
	let server = ScriptedBridgeServer::start(&temp_socket("undo")).expect("start scripted server");
	let mut agent = Agent::spawn(&root, server.socket_path());
	agent.initialize();

	assert!(!agent.call_tool("history.begin", json!({ "document_id": 1 })).is_error);
	let added = agent.call_tool("graph.add_node", json!({ "document_id": 1, "identifier": "graphene_core::raster::OpacityNode", "x": 0, "y": 0 }));
	assert!(!added.is_error, "graph.add_node failed: {}", added.text);
	let node_id = added.structured["node_id"].as_u64().expect("node_id");
	assert!(!agent.call_tool("history.commit", json!({ "document_id": 1 })).is_error);

	// Gate 3: the agent-caused mutation is visible through the live projection.
	let before_undo = agent.call_tool("session.snapshot", json!({ "projection": "node_list", "document_id": 1 }));
	assert!(node_ids(&before_undo).contains(&node_id), "the committed node must be visible through session.snapshot");
	assert!(node_ids(&before_undo).contains(&1000), "the human node must survive");
	assert_eq!(node_ids(&before_undo), server.node_ids(1).into_iter().collect::<BTreeSet<u64>>());

	let undone = agent.call_tool("history.undo", json!({ "document_id": 1 }));
	assert!(!undone.is_error, "history.undo failed: {}", undone.text);
	let after_undo = agent.call_tool("session.snapshot", json!({ "projection": "node_list", "document_id": 1 }));
	assert!(!node_ids(&after_undo).contains(&node_id), "one undo must remove the whole transaction");
	assert!(node_ids(&after_undo).contains(&1000), "undo must not touch the human node");
}

#[test]
fn responses_stay_correlated_under_an_interleaved_human_mutation() {
	let root = temp_root("interleave");
	let server = ScriptedBridgeServer::start(&temp_socket("interleave")).expect("start scripted server");
	let mut agent = Agent::spawn(&root, server.socket_path());
	agent.initialize();

	server.interleave_human_mutations(true);

	// Each request gets a human mutation + DocumentChanged event interleaved before
	// its response; every response must still correlate by id and be well-formed.
	for _ in 0..3 {
		let reply = agent.call_tool("graph.list_nodes", json!({ "document_id": 1 }));
		assert!(!reply.is_error, "graph.list_nodes failed under interleaving: {}", reply.text);
		let ids = node_ids(&reply);
		assert!(ids.contains(&1000), "state must not be corrupted: {ids:?}");
	}

	// After interleaving, one more read must still correlate and show the mutations.
	let reply = agent.call_tool("graph.list_nodes", json!({ "document_id": 1 }));
	assert!(!reply.is_error, "final graph.list_nodes failed: {}", reply.text);
	assert_eq!(node_ids(&reply), server.node_ids(1).into_iter().collect(), "the agent and the editor must agree");
}

#[test]
fn human_edits_arrive_as_one_debounced_document_changed() {
	let root = temp_root("events");
	let server = ScriptedBridgeServer::start(&temp_socket("events")).expect("start scripted server");
	let mut agent = Agent::spawn(&root, server.socket_path());
	agent.initialize();

	// A burst of three raw events for one document must collapse to one notification.
	server.human_edit(1, 3);
	let seen = agent.wait_for_document_changed(Duration::from_secs(5));
	assert!(!seen.is_empty(), "a human edit must produce a DocumentChanged notification");
	// Give a couple of debounce windows for any incorrectly forwarded duplicates.
	std::thread::sleep(Duration::from_millis(250));
	let seen = agent.document_changed_notifications();
	assert_eq!(seen.len(), 1, "the burst must debounce to exactly one DocumentChanged, got {seen:?}");
	assert_eq!(seen[0]["document"].as_u64(), Some(1));
}

#[test]
fn capability_refused_call_is_unauthorized() {
	let root = temp_root("caps");
	let server = ScriptedBridgeServer::start(&temp_socket("caps")).expect("start scripted server");
	let mut agent = Agent::spawn(&root, server.socket_path());
	agent.initialize();

	// The attached default grant set is `read,author`, so an Execute tool is refused
	// by the host before it ever reaches the socket.
	let denied = agent.call_tool("render.preview", json!({ "document_id": 1 }));
	assert!(denied.is_error, "render.preview must be denied under read,author");
	assert!(denied.text.contains("Unauthorized"), "expected Unauthorized, got {}", denied.text);

	// A read tool is still allowed, proving the refusal is capability-specific.
	let allowed = agent.call_tool("session.active_document", json!({}));
	assert!(!allowed.is_error, "session.active_document must be allowed: {}", allowed.text);
}
