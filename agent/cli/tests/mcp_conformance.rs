//! T2.11 — MCP conformance test for the real `graphite-agent` binary.
//!
//! The test lives in the package that declares the binary so
//! `CARGO_BIN_EXE_graphite-agent` resolves (HIGH-B2).

use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::time::Duration;

/// Rendering may need to initialize a GPU adapter; be generous.
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(300);

/// The §17 curated surface plus the two generated catalog tools, plus the three
/// Phase 4 session tools, plus the four Phase 5 registry tools (§17 additions).
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
	next_id: i64,
}

impl Agent {
	fn spawn(root: &Path, capabilities: &str) -> Self {
		let mut child = Command::new(env!("CARGO_BIN_EXE_graphite-agent"))
			.args(["--stdio", "--root", root.to_str().expect("root is UTF-8"), "--timeout-seconds", "240", "--capabilities", capabilities])
			.stdin(Stdio::piped())
			.stdout(Stdio::piped())
			.stderr(Stdio::inherit())
			.spawn()
			.expect("failed to spawn graphite-agent");

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

	fn send(&mut self, message: &Value) {
		self.stdin.write_all(message.to_string().as_bytes()).expect("write request");
		self.stdin.write_all(b"\n").expect("write newline");
		self.stdin.flush().expect("flush request");
	}

	fn request(&mut self, method: &str, params: Value) -> Value {
		let id = self.next_id;
		self.next_id += 1;
		self.send(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }));
		self.await_id(id)
	}

	fn notify(&mut self, method: &str, params: Value) {
		self.send(&json!({ "jsonrpc": "2.0", "method": method, "params": params }));
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

	fn call_tool(&mut self, name: &str, arguments: Value) -> ToolReply {
		let response = self.request("tools/call", json!({ "name": name, "arguments": arguments }));
		assert!(response.get("error").is_none(), "tools/call `{name}` returned a JSON-RPC error: {response}");
		let result = &response["result"];
		let is_error = result.get("isError").and_then(Value::as_bool).unwrap_or(false);
		let text = result["content"][0]["text"].as_str().unwrap_or_default().to_string();
		let structured = result.get("structuredContent").cloned().unwrap_or(Value::Null);
		ToolReply { is_error, text, structured }
	}

	fn initialize(&mut self) -> Value {
		let response = self.request(
			"initialize",
			json!({ "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": { "name": "conformance", "version": "1" } }),
		);
		self.notify("notifications/initialized", json!({}));
		response
	}
}

impl Drop for Agent {
	fn drop(&mut self) {
		let _ = self.child.kill();
		let _ = self.child.wait();
	}
}

fn temp_root(label: &str) -> PathBuf {
	let root = std::env::temp_dir().join(format!("graphite-agent-conformance-{}-{}", std::process::id(), label));
	let _ = std::fs::remove_dir_all(&root);
	std::fs::create_dir_all(&root).expect("create temp root");
	root
}

#[test]
fn initialize_and_tools_list_match_the_curated_surface() {
	let root = temp_root("tools");
	let mut agent = Agent::spawn(&root, "all");

	let initialize = agent.initialize();
	assert_eq!(initialize["result"]["protocolVersion"], "2025-06-18");
	assert_eq!(initialize["result"]["serverInfo"]["name"], "graphite-agent");
	for capability in ["tools", "resources", "prompts", "logging"] {
		assert!(initialize["result"]["capabilities"].get(capability).is_some(), "missing capability {capability}");
	}
	// E-18: hosts surface `instructions` as server-wide guidance, so the workflow
	// must actually be served rather than left to external documentation.
	let instructions = initialize["result"]["instructions"].as_str().expect("`initialize` must carry `instructions`");
	assert!(instructions.contains("node.list_types"), "instructions must name the discovery tools: {instructions}");

	let listed = agent.request("tools/list", json!({}));
	let tools = listed["result"]["tools"].as_array().expect("tools array").clone();
	let names: BTreeSet<String> = tools.iter().map(|tool| tool["name"].as_str().expect("tool name").to_string()).collect();
	let expected: BTreeSet<String> = EXPECTED_TOOLS.iter().map(|name| name.to_string()).collect();
	assert_eq!(names, expected, "tools/list must equal the §17 curated tools plus node.list_types/node.describe");

	// E-18: the large-output tools carry a result-size annotation, and nothing else
	// does, so the annotation keeps its meaning.
	for name in ["node.list_types", "graph.list_nodes", "render.preview"] {
		let tool = tools.iter().find(|tool| tool["name"] == name).unwrap_or_else(|| panic!("{name} is missing from tools/list"));
		assert!(
			tool["_meta"]["anthropic/maxResultSizeChars"].as_u64().is_some_and(|chars| chars > 0),
			"{name} must carry a `_meta` size annotation"
		);
	}
	let annotated: Vec<&str> = tools.iter().filter(|tool| tool.get("_meta").is_some()).filter_map(|tool| tool["name"].as_str()).collect();
	assert_eq!(annotated.len(), 3, "unexpected annotated tools: {annotated:?}");

	// The generated catalog is non-empty and surfaced through the resource.
	let catalog = agent.request("resources/read", json!({ "uri": "graphite://node-catalog" }));
	let text = catalog["result"]["contents"][0]["text"].as_str().expect("catalog text");
	let catalog: Value = serde_json::from_str(text).expect("catalog JSON");
	assert!(catalog["count"].as_u64().unwrap_or(0) > 0, "catalog entry count must be > 0");
	assert_eq!(catalog["count"].as_u64(), catalog["nodes"].as_object().map(|nodes| nodes.len() as u64));

	let prompts = agent.request("prompts/list", json!({}));
	assert_eq!(prompts["result"]["prompts"].as_array().map(Vec::len), Some(3));

	// T3.6 / gate 7: the generated command catalog is read-only catalog data, never
	// a tool. It is reachable only through `graphite://command-catalog`.
	let resources = agent.request("resources/list", json!({}));
	let uris: BTreeSet<&str> = resources["result"]["resources"]
		.as_array()
		.expect("resources array")
		.iter()
		.filter_map(|resource| resource["uri"].as_str())
		.collect();
	assert!(uris.contains("graphite://command-catalog"), "the command catalog resource must be listed");
	assert!(uris.contains("graphite://node-catalog"), "the node catalog resource must stay listed");

	let command_response = agent.request("resources/read", json!({ "uri": "graphite://command-catalog" }));
	let command_text = command_response["result"]["contents"][0]["text"].as_str().expect("command catalog text");
	let command_catalog: Value = serde_json::from_str(command_text).expect("command catalog JSON");
	let commands = command_catalog["commands"].as_object().expect("commands object");
	assert!(!commands.is_empty(), "the command catalog must not be empty");
	assert_eq!(command_catalog["count"].as_u64(), Some(commands.len() as u64));
	for descriptor in commands.values() {
		let name = descriptor["name"].as_str().expect("command name");
		assert!(name.starts_with("command."), "{name} is not a command name");
		assert!(!names.contains(name), "command descriptor `{name}` must never be an MCP tool (INV-8/E-8)");
	}
}

#[test]
fn authoring_flow_renders_and_exports() {
	let root = temp_root("flow");
	let mut agent = Agent::spawn(&root, "all");
	agent.initialize();

	// document.new
	let created = agent.call_tool("document.new", json!({ "name": "conformance" }));
	assert!(!created.is_error, "document.new failed: {}", created.text);
	let document = created.structured["document_id"].as_u64().expect("document_id");

	// node.list_types — pick two Opacity nodes (Content input #0 is a Graphic, output #0 is a Graphic).
	let types = agent.call_tool("node.list_types", json!({}));
	assert!(!types.is_error, "node.list_types failed: {}", types.text);
	let identifiers: Vec<String> = types.structured["types"]
		.as_array()
		.expect("types array")
		.iter()
		.filter_map(|entry| entry["identifier"].as_str().map(str::to_string))
		.collect();
	assert!(!identifiers.is_empty(), "the node catalog must not be empty");
	let opacity = identifiers
		.iter()
		.find(|identifier| identifier.ends_with("OpacityNode"))
		.or_else(|| identifiers.first())
		.expect("at least one identifier")
		.clone();

	// node.describe exercises the generated descriptor path.
	let described = agent.call_tool("node.describe", json!({ "identifier": opacity }));
	assert!(!described.is_error, "node.describe failed: {}", described.text);

	// graph.add_node ×2
	let first = agent.call_tool("graph.add_node", json!({ "document_id": document, "identifier": opacity, "x": 0, "y": 0 }));
	assert!(!first.is_error, "graph.add_node failed: {}", first.text);
	let first_node = first.structured["node_id"].as_u64().expect("node_id");
	let second = agent.call_tool("graph.add_node", json!({ "document_id": document, "identifier": opacity, "x": 200, "y": 0 }));
	assert!(!second.is_error, "graph.add_node failed: {}", second.text);
	let second_node = second.structured["node_id"].as_u64().expect("node_id");
	assert_ne!(first_node, second_node, "the handler must allocate distinct node ids");

	// graph.connect
	let connected = agent.call_tool(
		"graph.connect",
		json!({ "document_id": document, "from_node": first_node, "from_output": 0, "to_node": second_node, "to_input": 0 }),
	);
	assert!(!connected.is_error, "graph.connect failed: {}", connected.text);

	// render.preview
	let preview = agent.call_tool("render.preview", json!({ "document_id": document, "max_dimension": 64 }));
	if preview.is_error {
		if is_adapter_failure(&preview.text) {
			println!("PARTIAL: render.preview could not initialize a GPU adapter in this environment: {}", preview.text);
		} else {
			panic!("render.preview failed for a non-adapter reason: {}", preview.text);
		}
	} else {
		let image = preview.structured["image_base64_png"].as_str().expect("image_base64_png");
		assert!(!image.is_empty(), "render.preview returned an empty PNG");
		assert!(image.starts_with("iVBORw0KGgo"), "render.preview did not return PNG bytes: {image:.16}");
	}

	// render.export — a confined path is written.
	let exported = agent.call_tool("render.export", json!({ "document_id": document, "path": "nested/preview.png", "format": "png", "scale": 0.5 }));
	if !exported.is_error {
		let written = root.join("nested/preview.png");
		assert!(written.exists(), "render.export did not write {}", written.display());
		assert!(std::fs::metadata(&written).map(|metadata| metadata.len() > 0).unwrap_or(false), "render.export wrote an empty file");
	} else if is_adapter_failure(&exported.text) {
		println!("PARTIAL: render.export could not initialize a GPU adapter in this environment: {}", exported.text);
	} else {
		panic!("render.export failed for a non-adapter reason: {}", exported.text);
	}

	// document.save — the async export reply must be delivered (E-5) and the host
	// writes the `.gdd` bytes under the root.
	let saved = agent.call_tool("document.save", json!({ "document_id": document, "path": "saved.gdd" }));
	assert!(!saved.is_error, "document.save failed: {}", saved.text);
	let saved_path = root.join("saved.gdd");
	assert!(saved_path.exists(), "document.save did not write {}", saved_path.display());
	assert!(std::fs::metadata(&saved_path).map(|metadata| metadata.len() > 0).unwrap_or(false), "document.save wrote an empty file");

	// render.export — traversal is rejected (INV-12).
	let escaped = agent.call_tool("render.export", json!({ "document_id": document, "path": "../escape.png", "format": "png" }));
	assert!(escaped.is_error, "render.export must reject a path outside the root");
	assert!(escaped.text.contains("PathOutsideRoot"), "expected PathOutsideRoot, got {}", escaped.text);

	// history.undo
	let undone = agent.call_tool("history.undo", json!({ "document_id": document }));
	assert!(!undone.is_error, "history.undo failed: {}", undone.text);
	assert_eq!(undone.structured["changed"].as_bool(), Some(true));

	// graph.list_nodes still correlates.
	let nodes = agent.call_tool("graph.list_nodes", json!({ "document_id": document }));
	assert!(!nodes.is_error, "graph.list_nodes failed: {}", nodes.text);
}

#[test]
fn read_only_capabilities_deny_authoring() {
	let root = temp_root("readonly");
	let mut agent = Agent::spawn(&root, "read");
	agent.initialize();

	let created = agent.call_tool("document.new", json!({ "name": "readonly" }));
	// `document.new` needs Persist, which a read-only grant does not include.
	assert!(created.is_error, "document.new must be denied under read-only capabilities");
	assert!(created.text.contains("Unauthorized"), "expected Unauthorized, got {}", created.text);

	let denied = agent.call_tool("graph.add_node", json!({ "document_id": 1, "identifier": "x" }));
	assert!(denied.is_error, "graph.add_node must be denied under read-only capabilities");
	assert!(denied.text.contains("Unauthorized"), "expected Unauthorized, got {}", denied.text);
}

fn is_adapter_failure(text: &str) -> bool {
	let lowered = text.to_ascii_lowercase();
	lowered.contains("adapter")
		|| lowered.contains("gpu")
		|| lowered.contains("wgpu")
		|| lowered.contains("no suitable")
		|| lowered.contains("executor not available")
		|| lowered.contains("vulkan")
		|| lowered.contains("device")
}
