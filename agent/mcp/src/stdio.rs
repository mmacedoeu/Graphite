//! Newline-delimited JSON-RPC over stdio (T2.9).
//!
//! **stdout carries JSON-RPC only** (INV-10); diagnostics go to stderr. Requests and
//! notifications are multiplexed with in-flight `tools/call` futures so a
//! `notifications/cancelled` can reach an in-flight call (HIGH-A4 / §5.5).

use futures::FutureExt;
use futures::StreamExt;
use futures::future::Either;
use futures::stream::FuturesUnordered;
use graphite_agent_host::Host;
use graphite_agent_host::modules::command_catalog::command_catalog;
use graphite_agent_host::modules::node_catalog::node_catalog;
use graphite_agent_host::modules::recipe_catalog;
use graphite_agent_protocol::{QueryId, ToolError, ToolHost, ToolOutcome, ToolRequest};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

pub(crate) const PROTOCOL_VERSION: &str = "2025-06-18";
pub(crate) const PARSE_ERROR: i64 = -32700;
pub(crate) const INVALID_REQUEST: i64 = -32600;
pub(crate) const METHOD_NOT_FOUND: i64 = -32601;
pub(crate) const INVALID_PARAMS: i64 = -32602;

/// JSON-RPC ids are numbers or strings; both must correlate back to a `QueryId`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(untagged)]
pub enum JsonRpcId {
	Number(i64),
	String(String),
}

impl JsonRpcId {
	pub(crate) fn from_value(value: &Value) -> Option<Self> {
		match value {
			Value::Number(number) => number.as_i64().map(JsonRpcId::Number),
			Value::String(text) => Some(JsonRpcId::String(text.clone())),
			_ => None,
		}
	}
}

struct PromptArgument {
	name: &'static str,
	description: &'static str,
	required: bool,
}

struct Prompt {
	name: &'static str,
	description: &'static str,
	arguments: &'static [PromptArgument],
	template: &'static str,
}

/// §11.1 — copied verbatim; do not invent additional prompts.
static PROMPTS: &[Prompt] = &[
	Prompt {
		name: "author_procedural_graph",
		description: "Draft a node graph for a procedural pattern.",
		arguments: &[PromptArgument {
			name: "goal",
			description: "What the artwork should depict.",
			required: true,
		}],
		template: "You are authoring a Graphite node graph. Goal: {{goal}}. Use node.list_types and node.describe to discover nodes. Build the graph with graph.add_node, graph.set_input, and graph.connect inside a history.begin/history.commit pair. Render with render.preview and correct any compile errors before finishing.",
	},
	Prompt {
		name: "explain_graph",
		description: "Explain an existing graph in plain language.",
		arguments: &[PromptArgument {
			name: "document_id",
			description: "Document to inspect.",
			required: true,
		}],
		template: "Read document {{document_id}} with graph.list_nodes and graph.get_node. Explain, in plain language, what the graph computes from inputs to the exported output. Name each node's role and describe the data flowing between them.",
	},
	Prompt {
		name: "repair_graph",
		description: "Diagnose and fix a graph that fails to compile or render.",
		arguments: &[PromptArgument {
			name: "document_id",
			description: "Document to repair.",
			required: true,
		}],
		template: "Inspect document {{document_id}}. Call render.preview; if it reports errors, use graph.list_nodes and graph.get_node to locate the broken or unconnected inputs. Repair them with graph.connect and graph.set_input inside a single transaction, then re-render until the preview succeeds.",
	},
];

type Inflight = FuturesUnordered<Pin<Box<dyn Future<Output = (JsonRpcId, ToolOutcome)> + 'static>>>;

/// One stdio connection's state: the id map (§11) and the in-flight call set.
struct Connection {
	host: Arc<dyn ToolHost>,
	/// JSON-RPC id -> host `QueryId`, so `notifications/cancelled` can find the call.
	query_by_request: HashMap<JsonRpcId, QueryId>,
	inflight: Inflight,
}

impl Connection {
	fn new(host: Arc<dyn ToolHost>) -> Self {
		Self {
			host,
			query_by_request: HashMap::new(),
			inflight: FuturesUnordered::new(),
		}
	}

	/// Handle one incoming JSON-RPC message. Immediate replies are appended to `out`;
	/// `tools/call` defers its reply into `inflight`.
	fn dispatch(&mut self, message: Value, out: &mut Vec<Value>) {
		let Some(object) = message.as_object() else {
			out.push(failure(&Value::Null, INVALID_REQUEST, "request must be a JSON object"));
			return;
		};

		let id = object.get("id").cloned();
		let is_notification = id.is_none() || matches!(id, Some(Value::Null));
		let id = id.unwrap_or(Value::Null);
		let params = object.get("params").cloned().unwrap_or(Value::Null);

		let Some(method) = object.get("method").and_then(Value::as_str) else {
			if !is_notification {
				out.push(failure(&id, INVALID_REQUEST, "missing `method`"));
			}
			return;
		};

		let reply = match method {
			"initialize" => Some(success(
				&id,
				json!({
					"protocolVersion": PROTOCOL_VERSION,
					"capabilities": { "tools": {}, "resources": {}, "prompts": {}, "logging": {} },
					"serverInfo": { "name": "graphite-agent", "version": env!("CARGO_PKG_VERSION") },
					"instructions": crate::SERVER_INSTRUCTIONS,
				}),
			)),
			"notifications/initialized" => None,
			"logging/setLevel" => Some(success(&id, json!({}))),
			"tools/list" => Some(success(&id, tools_list(self.host.as_ref()))),
			"tools/call" => {
				self.tools_call(&id, &params, out);
				None
			}
			"notifications/cancelled" => {
				self.cancelled(&params);
				None
			}
			"resources/list" => Some(success(&id, resources_list())),
			"resources/read" => Some(resources_read(&id, &params)),
			"prompts/list" => Some(success(&id, prompts_list())),
			"prompts/get" => Some(prompts_get(&id, &params)),
			_ => {
				if is_notification {
					None
				} else {
					Some(failure(&id, METHOD_NOT_FOUND, format!("unknown method `{method}`")))
				}
			}
		};

		if let Some(reply) = reply {
			out.push(reply);
		}
	}

	fn tools_call(&mut self, id: &Value, params: &Value, out: &mut Vec<Value>) {
		let Some(json_id) = JsonRpcId::from_value(id) else {
			out.push(failure(id, INVALID_PARAMS, "`tools/call` requires a numeric or string id"));
			return;
		};
		let Some(name) = params.get("name").and_then(Value::as_str) else {
			out.push(failure(id, INVALID_PARAMS, "`tools/call` requires a `name` parameter"));
			return;
		};
		if !self.host.descriptors().iter().any(|descriptor| descriptor.name == name) {
			out.push(failure(id, INVALID_PARAMS, format!("unknown tool `{name}`")));
			return;
		}

		let arguments = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
		if !arguments.is_object() {
			out.push(failure(id, INVALID_PARAMS, "`arguments` must be an object"));
			return;
		}

		let pending = self.host.call(ToolRequest {
			name: name.to_string(),
			arguments,
			document: None,
		});
		// INV-14: the host allocated the QueryId; the connection only remembers the mapping.
		self.query_by_request.insert(json_id.clone(), pending.id);
		let outcome = pending.outcome;
		self.inflight.push(Box::pin(async move {
			let outcome = outcome.await;
			(json_id, outcome)
		}));
	}

	fn cancelled(&mut self, params: &Value) {
		let Some(request_id) = params.get("requestId").and_then(JsonRpcId::from_value) else {
			return;
		};
		if let Some(query_id) = self.query_by_request.get(&request_id).copied()
			&& !self.host.cancel(query_id)
		{
			eprintln!("graphite-agent: cancel for request {request_id:?} did not match an in-flight call");
		}
	}
}

fn prompt_json(prompt: &Prompt) -> Value {
	let arguments: Vec<Value> = prompt
		.arguments
		.iter()
		.map(|argument| {
			json!({
				"name": argument.name,
				"description": argument.description,
				"required": argument.required,
			})
		})
		.collect();
	json!({
		"name": prompt.name,
		"description": prompt.description,
		"arguments": arguments,
	})
}

/// `tools/list` result — shared by the stdio and HTTP adapters so there is exactly
/// one registry (INV-8).
pub(crate) fn tools_list(host: &dyn ToolHost) -> Value {
	let tools: Vec<Value> = host
		.descriptors()
		.into_iter()
		.map(|descriptor| {
			let mut entry = json!({
				"name": descriptor.name,
				"description": descriptor.description,
				"inputSchema": descriptor.input_schema,
			});
			// E-18: a per-tool `_meta` object is copied verbatim, which is how a
			// large-output tool raises Claude Code's result-size ceiling.
			if let Some(meta) = descriptor.meta
				&& let Some(object) = entry.as_object_mut()
			{
				object.insert("_meta".to_string(), meta);
			}
			entry
		})
		.collect();
	json!({ "tools": tools })
}

/// `resources/list` result (§11).
pub(crate) fn resources_list() -> Value {
	json!({
		"resources": [
			{
				"uri": "graphite://node-catalog",
				"name": "Node catalog",
				"description": "Every node type generated from NODE_METADATA.",
				"mimeType": "application/json",
			},
			{
				"uri": "graphite://command-catalog",
				"name": "Command catalog",
				"description": "Generated command descriptors (catalog data; never tools, E-8).",
				"mimeType": "application/json",
			},
			{
				"uri": "graphite://recipe-catalog",
				"name": "Recipe catalog",
				"description": "Committed recipes & archetypes corpus; parsed from agent/recipes.json (recipes-and-archetypes plan).",
				"mimeType": "application/json",
			}
		]
	})
}

/// `resources/read` result (§11).
pub(crate) fn resources_read(id: &Value, params: &Value) -> Value {
	match params.get("uri").and_then(Value::as_str) {
		Some("graphite://node-catalog") => success(
			id,
			json!({
				"contents": [
					{
						"uri": "graphite://node-catalog",
						"mimeType": "application/json",
						"text": node_catalog().to_string(),
					}
				]
			}),
		),
		Some("graphite://command-catalog") => success(
			id,
			json!({
				"contents": [
					{
						"uri": "graphite://command-catalog",
						"mimeType": "application/json",
						"text": command_catalog().to_string(),
					}
				]
			}),
		),
		Some("graphite://recipe-catalog") => success(
			id,
			json!({
				"contents": [
					{
						"uri": "graphite://recipe-catalog",
						"mimeType": "application/json",
						"text": recipe_catalog::read_catalog().to_string(),
					}
				]
			}),
		),
		Some(uri) => failure(id, INVALID_PARAMS, format!("unknown resource `{uri}`")),
		None => failure(id, INVALID_PARAMS, "`resources/read` requires a `uri` parameter"),
	}
}

/// `prompts/list` result (§11.1).
pub(crate) fn prompts_list() -> Value {
	let prompts: Vec<Value> = PROMPTS.iter().map(prompt_json).collect();
	json!({ "prompts": prompts })
}

/// `prompts/get` result (§11.1). Arguments are substituted with `{{name}}`.
pub(crate) fn prompts_get(id: &Value, params: &Value) -> Value {
	let Some(name) = params.get("name").and_then(Value::as_str) else {
		return failure(id, INVALID_PARAMS, "`prompts/get` requires a `name` parameter");
	};
	let Some(prompt) = PROMPTS.iter().find(|prompt| prompt.name == name) else {
		return failure(id, INVALID_PARAMS, format!("unknown prompt `{name}`"));
	};

	let arguments = params.get("arguments").and_then(Value::as_object);
	let mut text = prompt.template.to_string();
	for argument in prompt.arguments {
		let value = arguments.and_then(|arguments| arguments.get(argument.name)).and_then(Value::as_str);
		match value {
			Some(value) => text = text.replace(&format!("{{{{{}}}}}", argument.name), value),
			None if argument.required => return failure(id, INVALID_PARAMS, format!("prompt `{name}` requires argument `{}`", argument.name)),
			None => {}
		}
	}

	success(
		id,
		json!({
			"description": prompt.description,
			"messages": [ { "role": "user", "content": { "type": "text", "text": text } } ],
		}),
	)
}

pub(crate) fn success(id: &Value, result: Value) -> Value {
	json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

pub(crate) fn failure(id: &Value, code: i64, message: impl Into<String>) -> Value {
	json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message.into() } })
}

/// §11 success/failure shape: a tool failure is an `isError` result, not a JSON-RPC error.
pub(crate) fn tool_response(id: &JsonRpcId, outcome: ToolOutcome) -> Value {
	match outcome {
		ToolOutcome::Ok { result, .. } => {
			let text = serde_json::to_string(&result).unwrap_or_else(|_| "{}".to_string());
			json!({
				"jsonrpc": "2.0",
				"id": id,
				"result": {
					"content": [ { "type": "text", "text": text } ],
					"structuredContent": result,
					"isError": false,
				}
			})
		}
		ToolOutcome::Err { error, .. } => {
			let text = error_text(&error);
			json!({
				"jsonrpc": "2.0",
				"id": id,
				"result": {
					"content": [ { "type": "text", "text": text } ],
					"isError": true,
				}
			})
		}
	}
}

/// The compact JSON encoding of a [`ToolError`], which keeps the variant name
/// (e.g. `Unauthorized`, `PathOutsideRoot`) in the text content.
fn error_text(error: &ToolError) -> String {
	serde_json::to_string(error).unwrap_or_else(|_| "<unserializable tool error>".to_string())
}

enum Event {
	Line(String),
	Ready(Option<(JsonRpcId, ToolOutcome)>),
}

/// Serve one stdio connection until EOF.
///
/// The host is the concrete [`Host`] so the adapter can drain bridge-originated
/// events (T4.6) on an idle tick without extending the frozen `ToolHost` trait.
pub async fn serve_stdio(host: Arc<Host>) -> std::io::Result<()> {
	let mut lines = BufReader::new(tokio::io::stdin()).lines();
	let stdout = Arc::new(tokio::sync::Mutex::new(tokio::io::stdout()));

	// T4.6: forward debounced editor events as `notifications/message`. This task
	// shares the current-thread runtime with the request loop, so an idle agent
	// still reports a human edit.
	{
		let host = Arc::clone(&host);
		let stdout = Arc::clone(&stdout);
		tokio::spawn(async move {
			let mut events = host.events();
			let mut ticker = tokio::time::interval(Duration::from_millis(20));
			loop {
				ticker.tick().await;
				let _ = host.drain_bridge_events().await;
				while let Some(Some(event)) = events.next().now_or_never() {
					let notification = json!({
						"jsonrpc": "2.0",
						"method": "notifications/message",
						"params": { "level": "info", "data": event },
					});
					let mut guard = stdout.lock().await;
					if write_json(&mut guard, &notification).await.is_err() {
						return;
					}
				}
			}
		});
	}

	let mut connection = Connection::new(Arc::clone(&host) as Arc<dyn ToolHost>);

	loop {
		// Multiplex the next stdin line with completed in-flight calls. Everything is
		// `!Send`, so this all stays on the one dedicated current-thread runtime (§5.5).
		let event = {
			let read = lines.next_line();
			if connection.inflight.is_empty() {
				match read.await? {
					Some(line) => Event::Line(line),
					None => break,
				}
			} else {
				futures::pin_mut!(read);
				let next = connection.inflight.next();
				futures::pin_mut!(next);
				match futures::future::select(read, next).await {
					Either::Left((line, _)) => match line? {
						Some(line) => Event::Line(line),
						None => break,
					},
					Either::Right((ready, _)) => Event::Ready(ready),
				}
			}
		};

		match event {
			Event::Line(line) => {
				if line.trim().is_empty() {
					continue;
				}
				let message: Value = match serde_json::from_str(&line) {
					Ok(message) => message,
					Err(error) => {
						let mut guard = stdout.lock().await;
						write_json(&mut guard, &failure(&Value::Null, PARSE_ERROR, format!("parse error: {error}"))).await?;
						continue;
					}
				};
				let mut responses = Vec::new();
				connection.dispatch(message, &mut responses);
				for response in responses {
					let mut guard = stdout.lock().await;
					write_json(&mut guard, &response).await?;
				}
			}
			Event::Ready(Some((id, outcome))) => {
				connection.query_by_request.remove(&id);
				let mut guard = stdout.lock().await;
				write_json(&mut guard, &tool_response(&id, outcome)).await?;
			}
			Event::Ready(None) => {}
		}
	}

	Ok(())
}

async fn write_json(stdout: &mut tokio::io::Stdout, value: &Value) -> std::io::Result<()> {
	let mut encoded = serde_json::to_vec(value).unwrap_or_else(|_| b"{}".to_vec());
	encoded.push(b'\n');
	stdout.write_all(&encoded).await?;
	stdout.flush().await
}
