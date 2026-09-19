//! The curated tool modules. Each module owns a disjoint slice of the §17 tool
//! list and answers through [`graphite_agent_protocol::EditorBridge`].

pub mod command_catalog;
pub mod document;
pub mod graph;
pub mod history;
pub mod node_catalog;
pub mod registry;
pub mod render;
pub mod session;

use graphite_agent_protocol::{BridgeQuery, Capability, EditorBridge, QueryId, ToolCall, ToolDescriptor, ToolError};
use serde_json::{Value, json};

/// The only JSON Schema dialect used by the curated descriptors (§6).
pub(crate) const SCHEMA: &str = "https://json-schema.org/draft/2020-12/schema";

/// Build a curated descriptor at version 1 (§5.6).
pub(crate) fn descriptor(name: &str, description: &str, capability: Capability, input_schema: Value, output_schema: Value) -> ToolDescriptor {
	ToolDescriptor {
		name: name.to_string(),
		description: description.to_string(),
		capability,
		input_schema,
		output_schema,
		version: 1,
		meta: None,
	}
}

/// The documented hard ceiling for the `anthropic/maxResultSizeChars` annotation.
pub(crate) const MAX_RESULT_SIZE_CHARS_CEILING: u32 = 500_000;

/// The one size annotation we emit. Claude Code persists an oversized *text* tool
/// result to disk and replaces it with a file reference above ~10k tokens (25k by
/// default); `_meta["anthropic/maxResultSizeChars"]` raises that ceiling for one
/// tool. The value is clamped to the host's documented hard limit rather than
/// rejected, so a wrong constant degrades instead of panicking (E-18).
pub(crate) fn max_result_size_chars(chars: u32) -> Value {
	json!({ "anthropic/maxResultSizeChars": chars.min(MAX_RESULT_SIZE_CHARS_CEILING) })
}

/// Names whose text result can exceed a host's default result cap, with the
/// ceiling each one asks for. Centralized here so the tools stay the single
/// source of truth: the modules build the descriptors and this table only
/// annotates them.
fn size_annotation(tool: &str) -> Option<u32> {
	match tool {
		// The whole generated node catalog: 335 entries with descriptions.
		"node.list_types" => Some(200_000),
		// A large graph's node dump.
		"graph.list_nodes" => Some(200_000),
		// A base64 PNG, and the only tool that can plausibly reach the ceiling.
		"render.preview" => Some(MAX_RESULT_SIZE_CHARS_CEILING),
		_ => None,
	}
}

/// Attach the size annotation to a curated descriptor, if it needs one (E-18).
pub fn annotate(descriptor: ToolDescriptor) -> ToolDescriptor {
	match size_annotation(&descriptor.name) {
		Some(chars) => {
			let mut descriptor = descriptor;
			descriptor.meta = Some(max_result_size_chars(chars));
			descriptor
		}
		None => descriptor,
	}
}

/// Every tool name that carries a size annotation. Used by the conformance tests to
/// prove each annotated name is a real `tools/list` entry.
pub fn annotated_tool_names() -> Vec<String> {
	["node.list_types", "graph.list_nodes", "render.preview"].into_iter().map(str::to_string).collect()
}

/// Submit one curated query under the host-allocated call id and drive the editor
/// (pump + poll) until its correlated reply arrives.
///
/// The id is reused for every sub-query of a single tool call: the tool call is the
/// correlation unit (INV-14 allocates ids per tool call, not per bridge query), and
/// sub-queries are strictly sequential — each reply is drained before the next
/// submit. A call that never replies is resolved by the host's timeout/cancel race.
pub(crate) async fn query(bridge: &mut dyn EditorBridge, id: QueryId, query: BridgeQuery) -> Result<Value, ToolError> {
	bridge.submit(id, query)?;
	loop {
		bridge.pump().await?;
		if let Some(result) = bridge.poll(id) {
			return result;
		}
		// Be a good citizen on the single-threaded runtime: the async work that
		// produces the reply may be running on the editor's own spawner runtime.
		tokio::task::yield_now().await;
	}
}

pub(crate) fn schema_object(required: &[&str], properties: Value) -> Value {
	json!({
		"$schema": SCHEMA,
		"type": "object",
		"additionalProperties": false,
		"required": required,
		"properties": properties,
	})
}

pub(crate) fn ok_object() -> Value {
	json!({ "$schema": SCHEMA, "type": "object", "additionalProperties": true })
}

pub(crate) fn arguments(call: &ToolCall) -> &serde_json::Map<String, Value> {
	call.arguments.as_object().unwrap_or(&EMPTY_MAP)
}

static EMPTY_MAP: std::sync::LazyLock<serde_json::Map<String, Value>> = std::sync::LazyLock::new(serde_json::Map::new);

pub(crate) fn required_u64(call: &ToolCall, key: &str) -> Result<u64, ToolError> {
	arguments(call).get(key).and_then(Value::as_u64).ok_or_else(|| ToolError::InvalidArguments {
		message: format!("missing or non-integer argument `{key}`"),
	})
}

pub(crate) fn optional_u64(call: &ToolCall, key: &str) -> Result<Option<u64>, ToolError> {
	match arguments(call).get(key) {
		None | Some(Value::Null) => Ok(None),
		Some(value) => value.as_u64().map(Some).ok_or_else(|| ToolError::InvalidArguments {
			message: format!("argument `{key}` must be a non-negative integer"),
		}),
	}
}

pub(crate) fn required_u32(call: &ToolCall, key: &str) -> Result<u32, ToolError> {
	arguments(call)
		.get(key)
		.and_then(Value::as_u64)
		.and_then(|value| u32::try_from(value).ok())
		.ok_or_else(|| ToolError::InvalidArguments {
			message: format!("missing or non-integer argument `{key}`"),
		})
}

pub(crate) fn optional_u32(call: &ToolCall, key: &str, default: u32) -> Result<u32, ToolError> {
	match arguments(call).get(key) {
		None | Some(Value::Null) => Ok(default),
		Some(value) => value.as_u64().and_then(|value| u32::try_from(value).ok()).ok_or_else(|| ToolError::InvalidArguments {
			message: format!("argument `{key}` must be a non-negative integer"),
		}),
	}
}

pub(crate) fn required_string(call: &ToolCall, key: &str) -> Result<String, ToolError> {
	arguments(call).get(key).and_then(Value::as_str).map(str::to_string).ok_or_else(|| ToolError::InvalidArguments {
		message: format!("missing or non-string argument `{key}`"),
	})
}

pub(crate) fn optional_string(call: &ToolCall, key: &str) -> Result<Option<String>, ToolError> {
	match arguments(call).get(key) {
		None | Some(Value::Null) => Ok(None),
		Some(value) => value.as_str().map(|text| Some(text.to_string())).ok_or_else(|| ToolError::InvalidArguments {
			message: format!("argument `{key}` must be a string"),
		}),
	}
}

pub(crate) fn optional_f64(call: &ToolCall, key: &str, default: f64) -> Result<f64, ToolError> {
	match arguments(call).get(key) {
		None | Some(Value::Null) => Ok(default),
		Some(value) => value.as_f64().ok_or_else(|| ToolError::InvalidArguments {
			message: format!("argument `{key}` must be a number"),
		}),
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn plain(name: &str) -> ToolDescriptor {
		descriptor(name, "description", Capability::Read, json!({}), json!({}))
	}

	#[test]
	fn only_listed_tools_receive_an_annotation() {
		assert!(annotate(plain("document.new")).meta.is_none(), "an unrelated tool gained a `_meta` entry");
		let annotated = annotate(plain("render.preview"));
		assert_eq!(annotated.meta.expect("meta")["anthropic/maxResultSizeChars"], MAX_RESULT_SIZE_CHARS_CEILING);
	}

	#[test]
	fn every_annotated_name_resolves_to_a_ceiling() {
		for name in annotated_tool_names() {
			assert!(size_annotation(&name).is_some(), "{name} resolves to no annotation");
		}
	}

	#[test]
	fn a_ceiling_above_the_host_limit_is_clamped_not_rejected() {
		assert_eq!(max_result_size_chars(u32::MAX)["anthropic/maxResultSizeChars"], MAX_RESULT_SIZE_CHARS_CEILING);
	}
}
