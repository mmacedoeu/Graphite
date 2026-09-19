//! T5.7 — peer-mode conformance test for the real `graphite-agent` binary.
//!
//! Boots the shipped binary in Surface C (peer) mode over a minimal `.gdd` working copy and
//! checks that it answers `initialize` / `tools/list` and serves the Phase 5 `registry.query`
//! tool over stdio (gate 8 / R7-M3, R8-M4).

use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::time::Duration;

const RESPONSE_TIMEOUT: Duration = Duration::from_secs(60);

struct Agent {
	child: Child,
	stdin: ChildStdin,
	responses: Receiver<Value>,
	next_id: i64,
}

impl Agent {
	/// Spawn `graphite-agent --mode peer --gdd <gdd> --root <root> --stdio`.
	fn spawn(root: &Path, gdd: &Path) -> Self {
		let mut child = Command::new(env!("CARGO_BIN_EXE_graphite-agent"))
			.args([
				"--mode",
				"peer",
				"--gdd",
				gdd.to_str().expect("gdd is UTF-8"),
				"--root",
				root.to_str().expect("root is UTF-8"),
				"--stdio",
				"--timeout-seconds",
				"30",
			])
			.stdin(Stdio::piped())
			.stdout(Stdio::piped())
			.stderr(Stdio::inherit())
			.spawn()
			.expect("failed to spawn graphite-agent in peer mode");

		let stdin = child.stdin.take().expect("stdin");
		let stdout = child.stdout.take().expect("stdout");
		let (sender, responses) = channel();
		std::thread::spawn(move || {
			for line in BufReader::new(stdout).lines() {
				let Ok(line) = line else { break };
				if line.trim().is_empty() {
					continue;
				}
				match serde_json::from_str::<Value>(&line) {
					Ok(value) => {
						if sender.send(value).is_err() {
							break;
						}
					}
					Err(error) => panic!("stdout must carry JSON-RPC only (INV-10); got {line:?}: {error}"),
				}
			}
		});

		Self { child, stdin, responses, next_id: 1 }
	}

	fn request(&mut self, method: &str, params: Value) -> Value {
		let id = self.next_id;
		self.next_id += 1;
		let message = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
		self.stdin.write_all(message.to_string().as_bytes()).expect("write request");
		self.stdin.write_all(b"\n").expect("write newline");
		self.stdin.flush().expect("flush request");
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
}

impl Drop for Agent {
	fn drop(&mut self) {
		let _ = self.child.kill();
		let _ = self.child.wait();
	}
}

/// A minimal but valid `.gdd` working copy: a JSON manifest declaring format `gdd`, no
/// registry/history payload (the session loads empty).
fn write_minimal_gdd(dir: &Path) {
	std::fs::create_dir_all(dir).expect("create gdd dir");
	let manifest = json!({
		"format": "gdd",
		"format_version": 1,
		"editor_version": "test",
		"stdlib_version": "test",
		"document_id": 1,
	});
	std::fs::write(dir.join("manifest.json"), serde_json::to_vec_pretty(&manifest).expect("manifest json")).expect("write manifest");
}

fn temp_root(label: &str) -> PathBuf {
	let root = std::env::temp_dir().join(format!("graphite-agent-peer-conformance-{}-{label}", std::process::id()));
	let _ = std::fs::remove_dir_all(&root);
	std::fs::create_dir_all(&root).expect("create temp root");
	root
}

#[test]
fn peer_mode_serves_tools_list_and_registry_query() {
	let root = temp_root("peer");
	let gdd = root.join("document.gdd");
	write_minimal_gdd(&gdd);

	let mut agent = Agent::spawn(&root, &gdd);

	let initialize = agent.request(
		"initialize",
		json!({ "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": { "name": "conformance", "version": "1" } }),
	);
	assert_eq!(initialize["result"]["protocolVersion"], "2025-06-18");
	assert_eq!(initialize["result"]["serverInfo"]["name"], "graphite-agent");

	let listed = agent.request("tools/list", json!({}));
	let names: BTreeSet<String> = listed["result"]["tools"]
		.as_array()
		.expect("tools array")
		.iter()
		.map(|tool| tool["name"].as_str().expect("tool name").to_string())
		.collect();
	for expected in ["registry.apply_delta", "registry.query", "registry.merge", "history.replay"] {
		assert!(names.contains(expected), "peer tools/list must include {expected}");
	}

	// The empty peer document answers a typed registry read.
	let reply = agent.request("tools/call", json!({ "name": "registry.query", "arguments": {} }));
	assert!(reply.get("error").is_none(), "registry.query returned a JSON-RPC error: {reply}");
	let result = &reply["result"];
	assert_eq!(result["isError"], json!(false), "registry.query failed: {result}");
	assert_eq!(result["structuredContent"]["node_count"], json!(0));

	// `history.replay` proves the reconstructed registry equals the live one by value.
	let replay = agent.request("tools/call", json!({ "name": "history.replay", "arguments": {} }));
	assert_eq!(replay["result"]["structuredContent"]["equal"], json!(true), "history.replay failed: {replay}");
}
