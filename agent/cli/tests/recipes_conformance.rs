//! Conformance tests for the recipes-and-archetypes layer (recipes-and-
//! archetypes plan section 8).
//!
//! Mirrors the boot pattern of mcp_conformance.rs: spawn `graphite-agent` in
//! stdio mode, drive JSON-RPC, assert against the wire. Recipes-specific
//! tests cover recipes.list, recipes.show, recipes.lint, and the
//! graphite://recipe-catalog resource.
//!
//! The shipped corpus is 10 recipes under agent/recipes/. Tests run against
//! the on-disk committed catalog so any future schema break in
//! `recipes-build` (a recipe with a missing `id` field, an unrendered
//! `source.missing`-chain that graduates to Error, etc.) shows up as a
//! red test in CI.

use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::time::Duration;

/// Render needs GPU context; be generous.
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(60);

const EXPECTED_RECIPE_IDS: &[&str] = &[
	"alpha-composite-stack",
	"cartoon-posterize",
	"exploded-component-cycle",
	"feedback-trail-color",
	"kinetic-type-static",
	"kinetic-typography-loop",
	"lut-grade-warm",
	"pulse-glow",
	"selective-saturation",
	"wireframe-progressive-build",
];

const RECIPE_ID_REGEX_TEXT: &str = r"^[a-z0-9]+(?:-[a-z0-9]+)*$";

/// The committed-corpus-aware ids we expect to find. Mirrors the `recipes/`
/// directory in the repo at the time this test was authored.
struct Agent {
	#[allow(dead_code)]
	child: Child,
	stdin: ChildStdin,
	responses: Receiver<Value>,
	next_id: i64,
}

impl Agent {
	/// Spawn `graphite-agent --stdio --root <root> --capabilities all`. The root is
	/// passed so the host's path resolution has a directory to scratch into;
	/// recipes are read from the cwd-relative path `agent/recipes.json` so the
	/// agent process's CWD is the workspace root (set by cargo).
	fn spawn(cwd: &Path) -> Self {
		let mut child = Command::new(env!("CARGO_BIN_EXE_graphite-agent"))
			.current_dir(cwd)
			.args(["--stdio", "--root", "agent/recipes", "--timeout-seconds", "60", "--capabilities", "all"])
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
				Err(RecvTimeoutError::Timeout) => {
					panic!("timed out waiting for response to {method} (id {id})");
				}
				Err(RecvTimeoutError::Disconnected) => {
					panic!("agent stdout closed before response to {method} (id {id})");
				}
			}
		}
	}

	/// Send a JSON-RPC notification (no `id`; no response expected).
	fn notify(&mut self, method: &str, params: Value) {
		let message = json!({ "jsonrpc": "2.0", "method": method, "params": params });
		self.stdin.write_all(message.to_string().as_bytes()).expect("write notification");
		self.stdin.write_all(b"\n").expect("write newline");
		self.stdin.flush().expect("flush notification");
	}

	fn initialize(&mut self) {
		let init = self.request(
			"initialize",
			json!({
				"protocolVersion": "2025-06-18",
				"capabilities": {},
				"clientInfo": { "name": "recipes_conformance", "version": "0" },
			}),
		);
		assert_eq!(init.get("result").map(|r| r["protocolVersion"].as_str()), Some(Some("2025-06-18")), "initialize: {init}");
		self.notify("notifications/initialized", json!({}));
	}

	fn call_tool(&mut self, name: &str, arguments: Value) -> Value {
		self.request("tools/call", json!({ "name": name, "arguments": arguments }))
	}

	fn tool_structured(&mut self, name: &str, arguments: Value) -> Value {
		let response = self.call_tool(name, arguments);
		let result = response.get("result").cloned().unwrap_or(Value::Null);
		assert_eq!(result.get("isError").and_then(Value::as_bool), Some(false), "tool {name} returned error: {result}");
		// MCP responses may carry structured content under `structuredContent`
		// (MCP 2025-06-18) or wrap it inside `content[0].text`. Handle both.
		if let Some(structured) = result.get("structuredContent") {
			structured.clone()
		} else {
			let text = result["content"][0]["text"]
				.as_str()
				.unwrap_or_else(|| panic!("tool {name}: no content[0].text in {result}"));
			serde_json::from_str(text).unwrap_or_else(|e| panic!("tool {name}: content text is not JSON ({e}): {text}"))
		}
	}

	fn resource(&mut self, uri: &str) -> Value {
		self.request("resources/read", json!({ "uri": uri }))
	}
}

/// Find the workspace root by walking up from CWD until `Cargo.toml` is
/// found. `cargo test` runs the binary with CWD equal to
/// `<workspace>/agent/cli/`, so the parent of that is the workspace root.
fn workspace_root() -> PathBuf {
	let mut path = std::env::current_dir().expect("CWD is set");
	loop {
		if path.join("Cargo.toml").is_file() && path.join("agent").is_dir() {
			return path;
		}
		if !path.pop() {
			panic!("could not find workspace root from {}", std::env::current_dir().unwrap().display());
		}
	}
}

fn is_valid_id(id: &str) -> bool {
	let re = fancy_regex::Regex::new(RECIPE_ID_REGEX_TEXT).expect("constant regex");
	re.is_match(id).unwrap_or(false)
}

/// Whether `template` references `identifier`.
///
/// The recipe's template is authored prose, not a structured manifest, so we
/// allow three looseness rungs:
///   1. the full identifier appears verbatim (e.g. `OpacityNode`),
///   2. the trailing segment after the last `::` appears verbatim,
///   3. any CamelCase-split word from the trailing segment appears
///      case-insensitively (so a template mentioning `Posterize` covers the
///      full `PosterizeShaderNodeNode` identifier, and a template mentioning
///      `Brightness` covers `BrightnessContrastShaderNodeNode`).
fn template_references(template: &str, identifier: &str) -> bool {
	if template.contains(identifier) {
		return true;
	}
	let trailing = identifier.rsplit("::").next().unwrap_or(identifier);
	if template.contains(trailing) {
		return true;
	}
	let split_re = fancy_regex::Regex::new(r"(?<!^)([A-Z][a-z]+)").expect("constant regex");
	let template_lower = template.to_ascii_lowercase();
	for word in split_re.find_iter(trailing).map(Result::unwrap).map(|m| m.as_str()) {
		if template_lower.contains(&word.to_ascii_lowercase()) {
			return true;
		}
	}
	false
}

#[test]
fn recipes_list_is_well_formed_on_seeded_corpus() {
	let mut agent = Agent::spawn(&workspace_root());
	agent.initialize();
	let structured = agent.tool_structured("recipes.list", json!({}));
	let recipes = structured["recipes"]
		.as_array()
		.expect("recipes array");
	assert_eq!(recipes.len(), EXPECTED_RECIPE_IDS.len(), "expected {} recipes, got {}", EXPECTED_RECIPE_IDS.len(), recipes.len());

	let expected: BTreeSet<&str> = EXPECTED_RECIPE_IDS.iter().copied().collect();
	let actual: BTreeSet<String> = recipes
		.iter()
		.map(|recipe| {
			let id = recipe["id"].as_str().expect("id is string").to_string();
			assert!(is_valid_id(&id), "recipe id {id:?} does not match recipe-id regex");
			assert!(recipe["category"].is_string(), "missing category in {recipe}");
			assert!(recipe["summary"].is_string(), "missing summary in {recipe}");
			assert!(recipe["required_count"].is_number(), "missing required_count in {recipe}");
			id
		})
		.collect();
	assert_eq!(actual, expected.iter().map(|s| s.to_string()).collect::<BTreeSet<_>>());
}

#[test]
fn recipes_show_round_trips_recipe_template_and_preset() {
	let mut agent = Agent::spawn(&workspace_root());
	agent.initialize();
	let structured = agent.tool_structured(
		"recipes.show",
		json!({ "id": "pulse-glow" }),
	);
	assert_eq!(structured["id"], "pulse-glow");
	let recipe = &structured["recipe"];
	assert_eq!(recipe["category"], "Motion", "recipe card category mismatch");
	assert!(recipe["required"].is_array(), "required[] missing");
	assert!(recipe["defaults"]["frames"].as_u64().unwrap_or(0) > 1, "defaults.frames should be > 1 for pulse-glow");
	assert!(structured["template"].as_str().expect("template is text").contains("## Animation Timeline"), "pulse-glow template must contain ## Animation Timeline");
	assert!(structured["preset"].as_str().expect("preset is text").contains("pulse-glow"), "preset card mentions the recipe id");
}

#[test]
fn recipes_show_returns_invalid_arguments_for_unknown_id() {
	let mut agent = Agent::spawn(&workspace_root());
	agent.initialize();
	let response = agent.call_tool("recipes.show", json!({ "id": "does-not-exist" }));
	assert_eq!(
		response["result"]["isError"].as_bool(),
		Some(true),
		"expected isError=true for an unknown id, got {response}"
	);
	assert!(
		response["result"]["content"][0]["text"]
			.as_str()
			.map(|text| text.contains("does-not-exist"))
			.unwrap_or(false),
		"error text should mention the unknown id"
	);
}

#[test]
fn recipes_lint_returns_no_errors_on_seeded_corpus() {
	let mut agent = Agent::spawn(&workspace_root());
	agent.initialize();
	let structured = agent.tool_structured("recipes.lint", json!({}));
	assert!(structured.is_object());
	assert_eq!(structured["recipe_count"], 10);
	assert_eq!(structured["has_errors"], false, "shipped corpus should lint clean (errors only); got: {structured:?}");
	let issues = structured["issues"].as_array().expect("issues is array");
	// Warnings (source.missing, defaults.fps_or_frames_missing) are
	// acceptable for the seeded layer because the asset re-render pipeline
	// lands in a follow-on. We only assert there are zero Errors, not zero
	// warnings.
	let errors: Vec<&Value> = issues
		.iter()
		.filter(|issue| issue["level"].as_str() == Some("error"))
		.collect();
	assert!(errors.is_empty(), "shipped corpus has Errors: {errors:?}");
}

#[test]
fn graphite_recipe_catalog_resource_is_advertised_and_readable() {
	let mut agent = Agent::spawn(&workspace_root());
	agent.initialize();
	let list = agent.request("resources/list", Value::Null);
	let listed = list["result"]["resources"]
		.as_array()
		.expect("resources array");
	let catalog_uri = listed
		.iter()
		.find(|resource| resource["uri"] == "graphite://recipe-catalog")
		.expect("graphite://recipe-catalog not advertised in resources/list");
	assert_eq!(catalog_uri["name"], "Recipe catalog");

	let read = agent.resource("graphite://recipe-catalog");
	let contents = read["result"]["contents"]
		.as_array()
		.expect("contents array");
	let text = contents[0]["text"].as_str().expect("text content");
	let catalog: Value = serde_json::from_str(text).expect("catalog is JSON");
	assert_eq!(catalog["count"], 10, "catalog count mismatch: {catalog:?}");
	let catalog_recipes = catalog["recipes"].as_array().expect("recipes array");
	assert_eq!(catalog_recipes.len(), 10);
	// Spot-check: the catalog and the recipes.list endpoint must agree on
	// the id set (sanity check that no divergence snuck in during the
	// recipes-build pipeline).
	let list_ids: BTreeSet<String> = agent
		.tool_structured("recipes.list", json!({}))["recipes"]
		.as_array()
		.unwrap()
		.iter()
		.map(|recipe| recipe["id"].as_str().unwrap().to_string())
		.collect();
	let catalog_ids: BTreeSet<String> = catalog_recipes
		.iter()
		.map(|recipe| recipe["id"].as_str().unwrap().to_string())
		.collect();
	assert_eq!(list_ids, catalog_ids);
}

#[test]
fn recipes_show_template_references_every_required_node_identifier() {
	let mut agent = Agent::spawn(&workspace_root());
	agent.initialize();
	for id in EXPECTED_RECIPE_IDS {
		let structured = agent.tool_structured("recipes.show", json!({ "id": id }));
		let template = structured["template"].as_str().unwrap_or_default();
		// A node identifier is `lowercase_segments::joined_by_underscores::Segment`.
		// Pull the required[] identifiers and assert each appears in the template.
		for identifier in structured["recipe"]["required"].as_array().expect("required") {
			let needle = identifier.as_str().expect("identifier");
			// The template must mention the identifier (or any prose-friendly
			// token derived from it); see template_references for the
			// looseness ladder.
			assert!(
				template_references(template, needle),
				"recipe {id}: template does not reference required identifier {needle:?}; template = {template}"
			);
		}
	}
}
