//! Host-CLI installation surfaces (E-19).
//!
//! Every host CLI differs in only three ways: the file it reads, the wrapper key
//! inside that file, and whether it also has an official `mcp add` command.
//! Everything else — the arguments, the JSON entry, the TOML block, the guide —
//! is derived from one table here, so `print-config`, `install`, and the
//! generated `agent/INSTALL.md` cannot disagree with each other.
//!
//! Two invariants this module enforces:
//!
//! - **We never hand-edit a file we cannot parse.** Clients whose configuration
//!   is TOML (`codex`) or whose file is private state (`claude` user scope) are
//!   never rewritten by us; we print or run their own official command instead.
//! - **Paths are never invented.** Only the documented locations are used, and a
//!   client with no stable path gets manual steps rather than a guess.

use clap::ValueEnum;
use serde_json::{Map, Value, json};
use std::path::{Path, PathBuf};

/// The server name every client configuration uses.
pub const SERVER_NAME: &str = "graphite";

/// Host CLIs we can configure.
#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum Client {
	ClaudeCode,
	Codex,
	/// VS Code's own documentation writes its key as `servers`.
	#[value(name = "vscode", alias = "vs-code")]
	VsCode,
	Cursor,
	Gemini,
	/// Every client, for the generated guide.
	All,
}

/// Every client that can be configured, in guide order.
pub const CLIENTS: [Client; 5] = [Client::ClaudeCode, Client::Codex, Client::VsCode, Client::Cursor, Client::Gemini];

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum Scope {
	Project,
	User,
}

/// Which file format a body is written in.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum BodyFormat {
	Json,
	Toml,
}

impl Client {
	pub fn key(self) -> &'static str {
		match self {
			Client::ClaudeCode => "claude-code",
			Client::Codex => "codex",
			Client::VsCode => "vscode",
			Client::Cursor => "cursor",
			Client::Gemini => "gemini",
			Client::All => "all",
		}
	}

	pub fn label(self) -> &'static str {
		match self {
			Client::ClaudeCode => "Claude Code",
			Client::Codex => "Codex CLI",
			Client::VsCode => "VS Code",
			Client::Cursor => "Cursor",
			Client::Gemini => "Gemini CLI",
			Client::All => "all clients",
		}
	}

	/// A caveat that must appear wherever the client is written about.
	pub fn caveat(self) -> Option<&'static str> {
		match self {
			// The Gemini CLI documentation states it was replaced by Antigravity CLI
			// on 2026-06-18 for unpaid tiers and Google One users.
			Client::Gemini => Some("Superseded upstream by Antigravity CLI; supported here for the users still on it."),
			Client::VsCode => Some("VS Code spells the wrapper key `servers`; every other JSON client uses `mcpServers`."),
			Client::Codex => Some("We never rewrite `config.toml`; the official `codex mcp add` command does."),
			_ => None,
		}
	}
}

/// The arguments and identity of the server we are configuring.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerSpec {
	pub binary: String,
	pub root: String,
	pub capabilities: Option<String>,
}

impl ServerSpec {
	/// The canonical placeholder spec used by the generated guide, so the guide is
	/// byte-identical on every machine.
	pub fn placeholder() -> Self {
		Self {
			binary: "/absolute/path/to/graphite-agent".to_string(),
			root: "/absolute/path/to/art".to_string(),
			capabilities: None,
		}
	}

	pub fn args(&self) -> Vec<String> {
		let mut args = vec!["--mode".to_string(), "headless".to_string(), "--stdio".to_string(), "--root".to_string(), self.root.clone()];
		if let Some(capabilities) = &self.capabilities {
			args.push("--capabilities".to_string());
			args.push(capabilities.clone());
		}
		args
	}

	/// The stdio server entry used by every JSON-config client.
	pub fn json_entry(&self) -> Value {
		json!({
			"type": "stdio",
			"command": self.binary,
			"args": self.args(),
			"env": {},
		})
	}

	/// The `[mcp_servers.graphite]` table for Codex, including the timeouts our
	/// cold start needs: the host builds an `Editor` at startup, which can exceed
	/// Codex's 10-second default.
	pub fn toml_block(&self) -> String {
		let args = self.args().iter().map(|arg| format!("\"{arg}\"")).collect::<Vec<_>>().join(", ");
		format!(
			"[mcp_servers.{SERVER_NAME}]\ncommand = \"{}\"\nargs = [{}]\nstartup_timeout_sec = 60\ntool_timeout_sec = 300\nenabled = true\n",
			self.binary, args
		)
	}

	/// The official imperative commands, for the clients that have them.
	///
	/// Claude Code takes its own options before `--`, and everything after `--` is
	/// passed to the server untouched (its documentation is explicit about this).
	pub fn claude_add_stdio(&self, scope: &str) -> String {
		let args = self.args().iter().map(|arg| shell_quote(arg)).collect::<Vec<_>>().join(" ");
		format!("claude mcp add --scope {scope} --transport stdio {SERVER_NAME} -- {} {}", shell_quote(&self.binary), args)
	}

	pub fn claude_add_json(&self) -> String {
		format!("claude mcp add-json {SERVER_NAME} '{}' --scope user", self.json_entry())
	}

	pub fn codex_add(&self) -> String {
		let args = self.args().iter().map(|arg| shell_quote(arg)).collect::<Vec<_>>().join(" ");
		format!("codex mcp add {SERVER_NAME} -- {} {}", shell_quote(&self.binary), args)
	}
}

/// What should be installed, where, and by which mechanism.
#[derive(Clone, Debug)]
pub struct Plan {
	pub client: Client,
	pub scope: Scope,
	/// The file the client reads. `None` when the client has no stable file path.
	pub path: Option<PathBuf>,
	/// The JSON key holding the server map (JSON clients only).
	pub key: Option<&'static str>,
	pub body_format: BodyFormat,
	/// The entry (JSON) or table (TOML) this plan contributes.
	pub entry: String,
	/// The client's official command, when one exists.
	pub shell: Option<String>,
	/// Manual steps, for clients with neither a writable file nor one command.
	pub steps: Vec<String>,
	/// Whether `install` may edit `path` itself.
	pub writable: bool,
}

/// Build the install plan for one client and scope.
pub fn plan(client: Client, scope: Scope, project: &Path, home: &Path, spec: &ServerSpec) -> anyhow::Result<Plan> {
	anyhow::ensure!(client != Client::All, "`--client all` is only meaningful with `--format markdown`");

	let entry = spec.json_entry();
	let entry_text = serde_json::to_string_pretty(&entry)?;

	let file = |path: PathBuf, key: &'static str, format: BodyFormat, entry: String, writable: bool, shell: Option<String>, steps: Vec<String>| Plan {
		client,
		scope,
		path: Some(path),
		key: Some(key),
		body_format: format,
		entry,
		shell,
		steps,
		writable,
	};

	Ok(match (client, scope) {
		// `.mcp.json` is the portable project file: Claude Code reads it, and VS Code's
		// Agent Host reads a workspace `.mcp.json` natively (its own file uses `servers`).
		(Client::ClaudeCode, Scope::Project) => file(
			project.join(".mcp.json"),
			"mcpServers",
			BodyFormat::Json,
			entry_text,
			true,
			Some(spec.claude_add_stdio("project")),
			vec![],
		),
		// `~/.claude.json` also holds Claude Code's private state and credentials, so we
		// do not rewrite it; its own CLI is the supported writer.
		(Client::ClaudeCode, Scope::User) => Plan {
			client,
			scope,
			path: None,
			key: None,
			body_format: BodyFormat::Json,
			entry: entry_text,
			shell: Some(spec.claude_add_json()),
			steps: vec!["Run the command above; it appends the server to `~/.claude.json` without touching the rest.".to_string()],
			writable: false,
		},
		(Client::Codex, _) => {
			let path = match scope {
				Scope::Project => project.join(".codex").join("config.toml"),
				Scope::User => home.join(".codex").join("config.toml"),
			};
			Plan {
				client,
				scope,
				path: Some(path),
				key: None,
				body_format: BodyFormat::Toml,
				entry: spec.toml_block(),
				shell: Some(spec.codex_add()),
				steps: vec!["Codex has no TOML parser here, so we never rewrite the file: run the command, or paste the table into the file shown.".to_string()],
				writable: false,
			}
		}
		(Client::VsCode, Scope::Project) => file(project.join(".vscode").join("mcp.json"), "servers", BodyFormat::Json, entry_text, true, None, vec![]),
		// The VS Code user file lives under a named profile, so there is no single
		// stable path; the Command Palette flow is the supported route.
		(Client::VsCode, Scope::User) => Plan {
			client,
			scope,
			path: None,
			key: None,
			body_format: BodyFormat::Json,
			entry: entry_text,
			shell: None,
			steps: vec![
				"Run \"MCP: Open User Configuration\" from the Command Palette, or \"MCP: Add Server\".".to_string(),
				"Choose stdio, then enter the command and arguments shown above.".to_string(),
			],
			writable: false,
		},
		(Client::Cursor, Scope::Project) => file(project.join(".cursor").join("mcp.json"), "mcpServers", BodyFormat::Json, entry_text, true, None, vec![]),
		(Client::Cursor, Scope::User) => file(home.join(".cursor").join("mcp.json"), "mcpServers", BodyFormat::Json, entry_text, true, None, vec![]),
		(Client::Gemini, Scope::Project) => file(project.join(".gemini").join("settings.json"), "mcpServers", BodyFormat::Json, entry_text, true, None, vec![]),
		(Client::Gemini, Scope::User) => file(home.join(".gemini").join("settings.json"), "mcpServers", BodyFormat::Json, entry_text, true, None, vec![]),
		(Client::All, _) => unreachable!("rejected above"),
	})
}

/// The outcome of merging our entry into an existing file.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum MergeOutcome {
	Created,
	Updated,
	Unchanged,
}

impl MergeOutcome {
	pub fn label(self) -> &'static str {
		match self {
			MergeOutcome::Created => "created",
			MergeOutcome::Updated => "updated",
			MergeOutcome::Unchanged => "unchanged",
		}
	}
}

/// Merge our entry under `key` in a JSON object, preserving every other key.
///
/// Returns the full new file body, so the caller can show it before writing it.
pub fn merge_json(existing: Option<&str>, key: &str, name: &str, entry: &Value) -> anyhow::Result<(String, MergeOutcome)> {
	let mut root = match existing {
		None => Map::new(),
		Some(text) if text.trim().is_empty() => Map::new(),
		Some(text) => match serde_json::from_str::<Value>(text)? {
			Value::Object(object) => object,
			other => anyhow::bail!("refusing to edit: the existing file is a JSON {} at the top level, not an object", json_kind(&other)),
		},
	};

	let mut servers = match root.get(key) {
		None => Map::new(),
		Some(Value::Object(object)) => object.clone(),
		Some(other) => anyhow::bail!("refusing to edit: the existing `{key}` key is a JSON {}, not an object", json_kind(other)),
	};

	let outcome = match servers.get(name) {
		Some(current) if current == entry => MergeOutcome::Unchanged,
		Some(_) => MergeOutcome::Updated,
		None => MergeOutcome::Created,
	};
	servers.insert(name.to_string(), entry.clone());
	root.insert(key.to_string(), Value::Object(servers));

	let mut body = serde_json::to_string_pretty(&Value::Object(root))?;
	body.push('\n');
	Ok((body, outcome))
}

/// Remove our entry from a JSON object, preserving everything else.
pub fn remove_json(existing: &str, key: &str, name: &str) -> anyhow::Result<(String, bool)> {
	let mut root = match serde_json::from_str::<Value>(existing)? {
		Value::Object(object) => object,
		other => anyhow::bail!("refusing to edit: the existing file is a JSON {} at the top level, not an object", json_kind(&other)),
	};
	let removed = match root.get_mut(key).and_then(Value::as_object_mut) {
		Some(servers) => servers.remove(name).is_some(),
		None => false,
	};
	let mut body = serde_json::to_string_pretty(&Value::Object(root))?;
	body.push('\n');
	Ok((body, removed))
}

fn json_kind(value: &Value) -> &'static str {
	match value {
		Value::Null => "null",
		Value::Bool(_) => "boolean",
		Value::Number(_) => "number",
		Value::String(_) => "string",
		Value::Array(_) => "array",
		Value::Object(_) => "object",
	}
}

/// Quote one shell argument: bare when it is obviously safe, single-quoted otherwise.
fn shell_quote(argument: &str) -> String {
	if !argument.is_empty() && argument.chars().all(|character| character.is_ascii_alphanumeric() || "._/:@=+-".contains(character)) {
		return argument.to_string();
	}
	format!("'{}'", argument.replace('\'', r"'\''"))
}

/// The generated `agent/INSTALL.md`. Deterministic: it uses the placeholder spec
/// and symbolic paths, so it is identical on every machine and a drift test can
/// compare it byte for byte.
pub fn markdown() -> String {
	let spec = ServerSpec::placeholder();
	let project = Path::new("<PROJECT>");
	let home = Path::new("<HOME>");

	let mut out = String::new();
	out.push_str("# Installing the Graphite MCP server\n\n");
	out.push_str("<!-- GENERATED FILE - DO NOT EDIT.\n");
	out.push_str("     Regenerate with: cargo run -p graphite-agent-cli -- print-config --client all --format markdown > agent/INSTALL.md\n");
	out.push_str("     A test compares this file against that command, so hand edits fail the suite. -->\n\n");
	out.push_str("`graphite-agent` speaks MCP over stdio, so every host CLI can run it. The clients differ only in\n");
	out.push_str("where they keep the server list and what they call the wrapper key.\n\n");

	out.push_str("## 1. Get the binary\n\n");
	out.push_str("```sh\ncargo build -p graphite-agent-cli --release\n# or, to put it on PATH:\ncargo install --path agent/cli\n```\n\n");
	out.push_str("Use an absolute path in the configuration if the host is a GUI app that does not inherit your\n");
	out.push_str("shell `PATH` (Cursor and Claude Desktop are the common cases).\n\n");

	out.push_str("## 2. Let the server print its own configuration\n\n");
	out.push_str("```sh\ngraphite-agent print-config --client <client> --scope project\n```\n\n");
	out.push_str("That command writes nothing. Add `--dry-run` to preview, or `--yes` to write, with:\n\n");
	out.push_str("```sh\ngraphite-agent install --client <client> --scope project --yes\ngraphite-agent doctor\n```\n\n");
	out.push_str("`install` merges into any existing file, leaves every other server in place, and keeps a\n");
	out.push_str("timestamped backup of the previous contents before it writes. It is idempotent, and\n");
	out.push_str("`install --uninstall --yes` removes only our entry.\n\n");

	out.push_str("## 3. Per-client configuration\n\n");
	out.push_str("| Client | Wrapper key | Project file | User file |\n|---|---|---|---|\n");
	for client in CLIENTS {
		let project_plan = plan(client, Scope::Project, project, home, &spec).expect("project plan");
		let user_plan = plan(client, Scope::User, project, home, &spec).expect("user plan");
		out.push_str(&format!(
			"| {} | {} | {} | {} |\n",
			client.label(),
			project_plan.key.map(|key| format!("`{key}`")).unwrap_or_else(|| "-".to_string()),
			project_plan.path.as_ref().map(|path| format!("`{}`", path.display())).unwrap_or_else(|| "-".to_string()),
			user_plan.path.as_ref().map(|path| format!("`{}`", path.display())).unwrap_or_else(|| "Command Palette".to_string()),
		));
	}
	out.push('\n');

	for client in CLIENTS {
		out.push_str(&format!("### {}\n\n", client.label()));
		if let Some(caveat) = client.caveat() {
			out.push_str(&format!("> {caveat}\n\n"));
		}
		for (scope, heading) in [(Scope::Project, "Project"), (Scope::User, "User")] {
			let plan = plan(client, scope, project, home, &spec).expect("plan");
			out.push_str(&format!("**{heading} scope**\n\n"));
			if let Some(path) = &plan.path {
				out.push_str(&format!("File: `{}`\n\n", path.display()));
			}
			match plan.body_format {
				// A file we write is shown as the whole file, wrapper key included.
				BodyFormat::Json if plan.writable => {
					let entry: Value = serde_json::from_str(&plan.entry).expect("entry is JSON");
					let key = plan.key.expect("a writable JSON plan always has a key");
					let (body, _) = merge_json(None, key, SERVER_NAME, &entry).expect("a fresh merge cannot fail");
					out.push_str(&format!("```json\n{body}```\n\n"));
				}
				BodyFormat::Json => out.push_str(&format!("```json\n{}\n```\n\n", plan.entry)),
				BodyFormat::Toml => out.push_str(&format!("```toml\n{}```\n\n", plan.entry)),
			}
			if let Some(shell) = &plan.shell {
				out.push_str(&format!("Or use the client's own command:\n\n```sh\n{shell}\n```\n\n"));
			}
			for step in &plan.steps {
				out.push_str(&format!("- {step}\n"));
			}
			if !plan.steps.is_empty() {
				out.push('\n');
			}
			if !plan.writable {
				out.push_str("This file is **not** edited by `graphite-agent install`.\n\n");
			}
		}
	}

	out.push_str("## 4. Replace the paths\n\n");
	out.push_str("- `/absolute/path/to/graphite-agent`: the absolute path to `graphite-agent` (use\n");
	out.push_str("  `command -v graphite-agent`, or the path under `target/release/`). Claude Code also\n");
	out.push_str("  expands `${GRAPHITE_AGENT_BIN:-graphite-agent}` inside a project `.mcp.json`, which is how\n");
	out.push_str("  the committed repo configuration stays machine-independent.\n");
	out.push_str("- `/absolute/path/to/art`: the confinement root. Every file-path tool is refused outside it\n");
	out.push_str("  (INV-12), so open, restore, and export only touch files under this directory.\n\n");

	out.push_str("## 5. Security notes\n\n");
	out.push_str("- A project-scoped `.mcp.json` is committed, so it is code: Claude Code shows a project server\n");
	out.push_str("  as pending until you approve the workspace, and a cloned repository cannot approve itself.\n");
	out.push_str("- `--capabilities` narrows the grant set per client. Attached and peer sessions are already\n");
	out.push_str("  attenuated to their mode ceiling and refuse anything above it.\n");
	out.push_str("- `--root` is the only filesystem boundary. Do not point it at a directory you would not let\n");
	out.push_str("  the agent read and write.\n");
	out.push_str("- Gemini CLI prompts per tool call unless the server is marked trusted; this guide does not\n");
	out.push_str("  set `trust`, because that would silently auto-approve every call.\n\n");

	out.push_str("## 6. When it does not connect\n\n");
	out.push_str("```sh\ngraphite-agent doctor\n```\n\n");
	out.push_str("`doctor` checks the binary is absolute and exists, that the root is a writable directory, that\n");
	out.push_str("the generated node catalog is non-empty, and that each client's configuration mentions us. It\n");
	out.push_str("then performs a real MCP `initialize` handshake by starting the server, unless you pass\n");
	out.push_str("`--no-handshake`. Add `--json` for machine-readable output.\n");

	out
}

#[cfg(test)]
mod tests {
	use super::*;

	fn spec() -> ServerSpec {
		ServerSpec {
			binary: "/usr/local/bin/graphite-agent".to_string(),
			root: "/home/you/art".to_string(),
			capabilities: None,
		}
	}

	#[test]
	fn every_client_has_a_plan_for_both_scopes() {
		for client in CLIENTS {
			for scope in [Scope::Project, Scope::User] {
				let plan = plan(client, scope, Path::new("/project"), Path::new("/home/you"), &spec()).expect("plan");
				assert!(!plan.entry.is_empty(), "{} {scope:?} produced no entry", client.key());
			}
		}
	}

	#[test]
	fn json_clients_use_the_documented_wrapper_key() {
		let spec = spec();
		let vscode = plan(Client::VsCode, Scope::Project, Path::new("/project"), Path::new("/home/you"), &spec).expect("plan");
		assert_eq!(vscode.key, Some("servers"));
		for client in [Client::ClaudeCode, Client::Cursor, Client::Gemini] {
			let plan = plan(client, Scope::Project, Path::new("/project"), Path::new("/home/you"), &spec).expect("plan");
			assert_eq!(plan.key, Some("mcpServers"), "{} uses the wrong key", client.key());
		}
	}

	#[test]
	fn no_client_ever_targets_a_toml_file_for_writing() {
		for client in CLIENTS {
			for scope in [Scope::Project, Scope::User] {
				let plan = plan(client, scope, Path::new("/project"), Path::new("/home/you"), &spec()).expect("plan");
				if plan.body_format == BodyFormat::Toml {
					assert!(!plan.writable, "{} {scope:?} would hand-edit TOML", client.key());
					assert!(plan.shell.is_some(), "{} {scope:?} needs an official command", client.key());
				}
			}
		}
	}

	#[test]
	fn claude_user_scope_never_writes_private_state() {
		let plan = plan(Client::ClaudeCode, Scope::User, Path::new("/project"), Path::new("/home/you"), &spec()).expect("plan");
		assert!(!plan.writable);
		assert!(plan.path.is_none());
		assert!(plan.shell.as_deref().expect("command").starts_with("claude mcp add-json"));
	}

	#[test]
	fn merge_preserves_other_servers_and_is_idempotent() {
		let existing = r#"{"mcpServers":{"other":{"command":"npx","args":["-y","thing"]}}}"#;
		let entry = spec().json_entry();
		let (body, outcome) = merge_json(Some(existing), "mcpServers", SERVER_NAME, &entry).expect("merge");
		assert_eq!(outcome, MergeOutcome::Created);
		let parsed: Value = serde_json::from_str(&body).expect("json");
		assert!(parsed["mcpServers"]["other"]["command"].is_string(), "the existing server was lost");
		assert_eq!(parsed["mcpServers"][SERVER_NAME]["command"], "/usr/local/bin/graphite-agent");

		let (again, outcome) = merge_json(Some(&body), "mcpServers", SERVER_NAME, &entry).expect("merge");
		assert_eq!(outcome, MergeOutcome::Unchanged);
		assert_eq!(again, body);
	}

	#[test]
	fn merge_updates_a_stale_entry_in_place() {
		let stale = json!({ "command": "graphite-agent", "args": ["--root", "/old"] });
		let (body, _) = merge_json(None, "mcpServers", SERVER_NAME, &stale).expect("merge");
		let (body, outcome) = merge_json(Some(&body), "mcpServers", SERVER_NAME, &spec().json_entry()).expect("merge");
		assert_eq!(outcome, MergeOutcome::Updated);
		let parsed: Value = serde_json::from_str(&body).expect("json");
		assert_eq!(parsed["mcpServers"][SERVER_NAME]["args"][4], "/home/you/art", "the stale root must be replaced");
	}

	#[test]
	fn merge_refuses_to_clobber_a_file_it_cannot_parse() {
		assert!(merge_json(Some("[1, 2, 3]"), "mcpServers", SERVER_NAME, &spec().json_entry()).is_err());
		assert!(merge_json(Some(r#"{"mcpServers": 7}"#), "mcpServers", SERVER_NAME, &spec().json_entry()).is_err());
	}

	#[test]
	fn remove_only_drops_our_entry() {
		let existing = format!(r#"{{"mcpServers":{{"keep":{{"command":"x"}},"{SERVER_NAME}":{{"command":"y"}}}}}}"#);
		let (body, removed) = remove_json(&existing, "mcpServers", SERVER_NAME).expect("remove");
		assert!(removed);
		let parsed: Value = serde_json::from_str(&body).expect("json");
		assert!(parsed["mcpServers"]["keep"].is_object());
		assert!(parsed["mcpServers"][SERVER_NAME].is_null());
	}

	#[test]
	fn codex_block_carries_the_cold_start_timeouts() {
		let block = spec().toml_block();
		assert!(block.contains("[mcp_servers.graphite]"));
		assert!(block.contains("startup_timeout_sec = 60"), "the 10s Codex default is too small for our cold start");
		assert!(block.contains("tool_timeout_sec = 300"));
	}

	#[test]
	fn shell_quoting_survives_spaces_and_quotes() {
		assert_eq!(shell_quote("/usr/bin/graphite-agent"), "/usr/bin/graphite-agent");
		assert_eq!(shell_quote("/opt/my tools/agent"), "'/opt/my tools/agent'");
		assert_eq!(shell_quote("it's"), r"'it'\''s'");
	}

	#[test]
	fn markdown_is_deterministic_and_covers_every_client() {
		let first = markdown();
		assert_eq!(first, markdown());
		for client in CLIENTS {
			assert!(first.contains(client.label()), "the guide omits {}", client.key());
		}
		assert!(first.contains("servers"), "VS Code's key must be documented");
		assert!(first.contains("mcpServers"));
		assert!(!first.contains("/home/you"), "the guide must not leak machine paths");
	}
}
