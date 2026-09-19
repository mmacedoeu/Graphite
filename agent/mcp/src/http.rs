//! MCP Streamable HTTP adapter (T4.8), over `tiny_http`.
//!
//! The adapter is bound to `127.0.0.1` by default and refuses a non-loopback bind
//! unless [`HttpConfig::allow_non_loopback`] is explicitly set. It reuses the same
//! [`Host`] (and therefore the same single tool registry and the same
//! [`ToolHost`] implementation) as the stdio adapter — there is no second registry
//! (INV-8) and no second `Editor` (INV-11).

use crate::stdio::{JsonRpcId, failure, prompts_get, prompts_list, resources_list, resources_read, success, tool_response, tools_list};
use graphite_agent_host::Host;
use graphite_agent_protocol::{ToolHost, ToolRequest};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::time::Duration;

use crate::stdio::{INVALID_PARAMS, INVALID_REQUEST, METHOD_NOT_FOUND, PARSE_ERROR, PROTOCOL_VERSION};

/// HTTP adapter configuration.
#[derive(Clone, Copy, Debug, Default)]
pub struct HttpConfig {
	/// Explicitly opt in to binding a non-loopback address.
	pub allow_non_loopback: bool,
}

/// Validate a bind specification before binding.
///
/// Refuses a non-loopback address unless the config explicitly allows it, which is
/// what keeps the agent surface local by default (T4.8).
pub fn resolve_addr(spec: &str, config: &HttpConfig) -> Result<SocketAddr, String> {
	let addr: SocketAddr = spec.parse().map_err(|error| format!("invalid HTTP bind address `{spec}`: {error} (expected e.g. 127.0.0.1:8765)"))?;
	if !is_loopback(&addr) && !config.allow_non_loopback {
		return Err(format!("refusing to bind non-loopback HTTP address {addr}; pass --http-allow-non-loopback to override"));
	}
	Ok(addr)
}

fn is_loopback(addr: &SocketAddr) -> bool {
	match addr.ip() {
		IpAddr::V4(ip) => ip.is_loopback(),
		IpAddr::V6(ip) => ip.is_loopback(),
	}
}

/// A bound HTTP server. `bind` only validates and binds; [`HttpServer::serve`]
/// drives requests on the caller's runtime.
pub struct HttpServer {
	server: Arc<tiny_http::Server>,
	local_addr: SocketAddr,
	shutdown: Arc<AtomicBool>,
	receiver: mpsc::Receiver<tiny_http::Request>,
	thread: Option<std::thread::JoinHandle<()>>,
}

impl HttpServer {
	/// Bind the configured address and start the accept thread.
	pub fn bind(spec: &str, config: &HttpConfig) -> Result<Self, String> {
		let addr = resolve_addr(spec, config)?;
		let server = tiny_http::Server::http(addr).map_err(|error| format!("failed to bind HTTP address {addr}: {error}"))?;
		// `server_addr` reports the real port, which matters when `:0` was requested.
		let local_addr = server.server_addr().to_ip().unwrap_or(addr);

		let server = Arc::new(server);
		let (sender, receiver) = mpsc::channel();
		let thread_server = Arc::clone(&server);
		let shutdown = Arc::new(AtomicBool::new(false));
		let thread_shutdown = Arc::clone(&shutdown);
		let thread = std::thread::Builder::new()
			.name("graphite-agent-http".to_string())
			.spawn(move || {
				loop {
					if thread_shutdown.load(Ordering::Relaxed) {
						break;
					}
					match thread_server.recv_timeout(Duration::from_millis(50)) {
						Ok(Some(request)) => {
							if sender.send(request).is_err() {
								break;
							}
						}
						Ok(None) => {}
						Err(_) => break,
					}
				}
			})
			.map_err(|error| format!("failed to spawn the HTTP accept thread: {error}"))?;

		Ok(Self {
			server,
			local_addr,
			shutdown,
			receiver,
			thread: Some(thread),
		})
	}

	/// The address actually bound (useful when port `0` was requested).
	pub fn local_addr(&self) -> SocketAddr {
		self.local_addr
	}

	/// Serve requests until the server is dropped or the accept thread stops.
	pub async fn serve(&self, host: Arc<Host>) {
		let mut ids: HashMap<JsonRpcId, graphite_agent_protocol::QueryId> = HashMap::new();
		while !self.shutdown.load(Ordering::Relaxed) {
			match self.receiver.try_recv() {
				Ok(request) => handle_request(&host, request, &mut ids).await,
				Err(mpsc::TryRecvError::Empty) => tokio::time::sleep(Duration::from_millis(1)).await,
				Err(mpsc::TryRecvError::Disconnected) => break,
			}
		}
	}
}

impl Drop for HttpServer {
	fn drop(&mut self) {
		self.shutdown.store(true, Ordering::Relaxed);
		self.server.unblock();
		if let Some(thread) = self.thread.take() {
			let _ = thread.join();
		}
	}
}

fn respond_json(request: tiny_http::Request, status: u16, value: &Value) {
	let body = value.to_string();
	let response = tiny_http::Response::from_string(body)
		.with_status_code(status)
		.with_header(tiny_http::Header::from_bytes("Content-Type", "application/json").expect("static header"));
	let _ = request.respond(response);
}

fn respond_empty(request: tiny_http::Request, status: u16) {
	let _ = request.respond(tiny_http::Response::empty(status));
}

async fn handle_request(host: &Host, mut request: tiny_http::Request, ids: &mut HashMap<JsonRpcId, graphite_agent_protocol::QueryId>) {
	if request.method() != &tiny_http::Method::Post {
		respond_json(request, 405, &failure(&Value::Null, INVALID_REQUEST, "the MCP HTTP adapter accepts POST requests"));
		return;
	}

	let body = {
		let mut body = String::new();
		match request.as_reader().read_to_string(&mut body) {
			Ok(_) => body,
			Err(error) => {
				respond_json(request, 400, &failure(&Value::Null, PARSE_ERROR, format!("failed to read request body: {error}")));
				return;
			}
		}
	};

	let message: Value = match serde_json::from_str(&body) {
		Ok(message) => message,
		Err(error) => {
			respond_json(request, 400, &failure(&Value::Null, PARSE_ERROR, format!("parse error: {error}")));
			return;
		}
	};

	match dispatch(host, &message, ids).await {
		Some(response) => respond_json(request, 200, &response),
		// A notification has no JSON-RPC response; MCP Streamable HTTP answers 202.
		None => respond_empty(request, 202),
	}
}

/// Dispatch one JSON-RPC message against the shared host.
async fn dispatch(host: &Host, message: &Value, ids: &mut HashMap<JsonRpcId, graphite_agent_protocol::QueryId>) -> Option<Value> {
	let object = message.as_object()?;
	let method = object.get("method").and_then(Value::as_str)?;
	let id = object.get("id").cloned();
	let is_notification = id.is_none() || matches!(id, Some(Value::Null));
	let id = id.unwrap_or(Value::Null);
	let params = object.get("params").cloned().unwrap_or(Value::Null);

	match method {
		"initialize" => Some(success(
			&id,
			json!({
				"protocolVersion": PROTOCOL_VERSION,
				"capabilities": { "tools": {}, "resources": {}, "prompts": {}, "logging": {} },
				"serverInfo": { "name": "graphite-agent", "version": env!("CARGO_PKG_VERSION") },
			}),
		)),
		"notifications/initialized" => None,
		"logging/setLevel" => Some(success(&id, json!({}))),
		"tools/list" => Some(success(&id, tools_list(host))),
		"tools/call" => Some(tools_call(host, &id, &params, ids).await),
		"notifications/cancelled" => {
			if let Some(request_id) = params.get("requestId").and_then(JsonRpcId::from_value)
				&& let Some(query_id) = ids.get(&request_id).copied()
			{
				let _ = host.cancel(query_id);
			}
			None
		}
		"resources/list" => Some(success(&id, resources_list())),
		"resources/read" => Some(resources_read(&id, &params)),
		"prompts/list" => Some(success(&id, prompts_list())),
		"prompts/get" => Some(prompts_get(&id, &params)),
		_ if is_notification => None,
		_ => Some(failure(&id, METHOD_NOT_FOUND, format!("unknown method `{method}`"))),
	}
}

async fn tools_call(host: &Host, id: &Value, params: &Value, ids: &mut HashMap<JsonRpcId, graphite_agent_protocol::QueryId>) -> Value {
	let Some(json_id) = JsonRpcId::from_value(id) else {
		return failure(id, INVALID_PARAMS, "`tools/call` requires a numeric or string id");
	};
	let Some(name) = params.get("name").and_then(Value::as_str) else {
		return failure(id, INVALID_PARAMS, "`tools/call` requires a `name` parameter");
	};
	if !host.descriptors().iter().any(|descriptor| descriptor.name == name) {
		return failure(id, INVALID_PARAMS, format!("unknown tool `{name}`"));
	}
	let arguments = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
	if !arguments.is_object() {
		return failure(id, INVALID_PARAMS, "`arguments` must be an object");
	}

	// INV-14: the host allocates the QueryId; the adapter only maps it.
	let pending = host.call(ToolRequest {
		name: name.to_string(),
		arguments,
		document: None,
	});
	ids.insert(json_id.clone(), pending.id);
	let outcome = pending.outcome.await;
	ids.remove(&json_id);
	tool_response(&json_id, outcome)
}

#[cfg(test)]
mod tests {
	use super::*;
	use graphite_agent_protocol::CapabilitySet;
	use std::io::{Read as _, Write as _};
	use std::net::Ipv4Addr;

	fn host() -> Arc<Host> {
		Arc::new(Host::new(
			Box::new(NullBridge),
			CapabilitySet(vec![graphite_agent_protocol::Capability::Read]),
			Duration::from_secs(5),
			std::env::temp_dir(),
		))
	}

	struct NullBridge;

	impl graphite_agent_protocol::EditorBridge for NullBridge {
		fn submit(&mut self, _id: graphite_agent_protocol::QueryId, _query: graphite_agent_protocol::BridgeQuery) -> Result<(), graphite_agent_protocol::ToolError> {
			Ok(())
		}
		fn poll(&mut self, _id: graphite_agent_protocol::QueryId) -> Option<Result<Value, graphite_agent_protocol::ToolError>> {
			None
		}
		fn cancel(&mut self, _id: graphite_agent_protocol::QueryId) {}
		fn drain_events(&mut self) -> Vec<graphite_agent_protocol::AgentEvent> {
			Vec::new()
		}
		fn pump(&mut self) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), graphite_agent_protocol::ToolError>> + '_>> {
			Box::pin(async { Ok(()) })
		}
	}

	#[test]
	fn non_loopback_binds_are_refused_unless_configured() {
		assert!(resolve_addr("0.0.0.0:8765", &HttpConfig::default()).is_err());
		assert!(resolve_addr("127.0.0.1:8765", &HttpConfig::default()).is_ok());
		assert!(resolve_addr("0.0.0.0:8765", &HttpConfig { allow_non_loopback: true }).is_ok());
	}

	/// The bound port `0` must report the real port so the test can reach it.
	fn bound_port(server: &HttpServer) -> u16 {
		let addr = server.local_addr();
		assert!(addr.ip().is_loopback());
		if addr.port() == 0 {
			// Fall back to the listener's own address when `to_ip` is unavailable.
			return 0;
		}
		addr.port()
	}

	#[test]
	fn serves_tools_list_over_loopback() {
		let server = HttpServer::bind("127.0.0.1:0", &HttpConfig::default()).expect("bind");
		let port = bound_port(&server);
		let host = host();

		let handle = std::thread::spawn(move || {
			let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("runtime");
			runtime.block_on(async move {
				server.serve(host).await;
			});
		});

		let body = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list", "params": {} }).to_string();
		let response = http_post(port, &body);
		assert!(response.contains("\"jsonrpc\":\"2.0\""), "unexpected response: {response}");
		assert!(response.contains("session.snapshot"), "tools/list must include the Phase 4 session tools: {response}");
		assert!(response.contains("node.list_types"), "tools/list must include the catalog tools: {response}");

		// The serving thread is detached; the test process exits after the assertions.
		drop(handle);
	}

	/// A minimal HTTP/1.1 POST client, so the test needs no HTTP client dependency.
	fn http_post(port: u16, body: &str) -> String {
		let mut stream = std::net::TcpStream::connect((Ipv4Addr::LOCALHOST, port)).expect("connect");
		let request = format!(
			"POST /mcp HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
			body.len(),
			body
		);
		stream.write_all(request.as_bytes()).expect("write request");
		let mut response = String::new();
		stream.read_to_string(&mut response).expect("read response");
		response
	}
}
