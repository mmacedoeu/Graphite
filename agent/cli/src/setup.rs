//! `print-config`, `install`, and `doctor` (E-19).
//!
//! The rules these three commands follow:
//!
//! - **`print-config` never writes.** Its payload is the only thing on stdout, so
//!   it can be piped; notes go to stderr.
//! - **`install` is opt-in and reversible.** It writes only with `--yes`, only to
//!   files it can parse, always keeping a timestamped backup, and it never
//!   touches a file it cannot round-trip (see [`crate::clients`]).
//! - **`doctor` proves the server works**, rather than only reading config: it
//!   starts the real binary and completes an MCP `initialize` handshake.

use crate::clients::{self, BodyFormat, Client, MergeOutcome, Plan, Scope, ServerSpec};
use anyhow::{Context, bail};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

/// How many seconds doctor waits for the server to answer `initialize`.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(60);

/// `--format` for `print-config`.
#[derive(Copy, Clone, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum OutputFormat {
	/// The client's own file body.
	Native,
	/// A JSON envelope describing the plan, for agents.
	Json,
	/// Only the client's official command.
	Shell,
	/// Only the destination path.
	Path,
	/// The whole generated guide.
	Markdown,
}

#[derive(clap::Args, Debug)]
pub struct PrintConfigArgs {
	#[arg(long, value_enum, default_value_t = Client::ClaudeCode)]
	pub client: Client,
	#[arg(long, value_enum, default_value_t = Scope::Project)]
	pub scope: Scope,
	#[arg(long, value_enum, default_value_t = OutputFormat::Native)]
	pub format: OutputFormat,
	/// Project directory that owns the project-scoped files.
	#[arg(long)]
	pub project: Option<PathBuf>,
	/// Confinement root written into the configuration.
	#[arg(long)]
	pub root: Option<PathBuf>,
	/// Binary the client should spawn (defaults to this executable).
	#[arg(long)]
	pub binary: Option<String>,
	/// Capability grant to embed, if not the server default.
	#[arg(long)]
	pub capabilities: Option<String>,
}

#[derive(clap::Args, Debug)]
pub struct InstallArgs {
	#[arg(long, value_enum, default_value_t = Client::ClaudeCode)]
	pub client: Client,
	#[arg(long, value_enum, default_value_t = Scope::Project)]
	pub scope: Scope,
	#[arg(long)]
	pub project: Option<PathBuf>,
	#[arg(long)]
	pub root: Option<PathBuf>,
	#[arg(long)]
	pub binary: Option<String>,
	#[arg(long)]
	pub capabilities: Option<String>,
	/// Show what would change, and write nothing.
	#[arg(long)]
	pub dry_run: bool,
	/// Confirm changes to disk.
	#[arg(long)]
	pub yes: bool,
	/// Remove our entry instead of adding it.
	#[arg(long)]
	pub uninstall: bool,
	/// Run the client's own `mcp add` command when we do not write the file.
	#[arg(long)]
	pub run: bool,
}

#[derive(clap::Args, Debug)]
pub struct DoctorArgs {
	#[arg(long)]
	pub root: Option<PathBuf>,
	#[arg(long)]
	pub project: Option<PathBuf>,
	#[arg(long)]
	pub binary: Option<String>,
	/// Machine-readable output.
	#[arg(long)]
	pub json: bool,
	/// Skip starting the server; only inspect configuration and paths.
	#[arg(long)]
	pub no_handshake: bool,
}

/// One diagnostic result.
#[derive(Clone, Debug)]
pub struct Check {
	pub name: String,
	pub ok: bool,
	pub detail: String,
}

impl Check {
	fn new(name: impl Into<String>, ok: bool, detail: impl Into<String>) -> Self {
		Self {
			name: name.into(),
			ok,
			detail: detail.into(),
		}
	}

	fn to_json(&self) -> Value {
		json!({ "name": self.name, "ok": self.ok, "detail": self.detail })
	}
}

/// Read a file if it exists.
pub fn read_optional(path: &Path) -> anyhow::Result<Option<String>> {
	match std::fs::read_to_string(path) {
		Ok(text) => Ok(Some(text)),
		Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
		Err(error) => Err(error).with_context(|| format!("failed to read {}", path.display())),
	}
}

/// Write `body` atomically, copying the previous contents to a timestamped backup
/// first. Returns the backup path when one was made.
pub fn write_with_backup(path: &Path, body: &str) -> anyhow::Result<Option<PathBuf>> {
	let file_name = path.file_name().and_then(|name| name.to_str()).context("destination has no file name")?.to_string();
	if let Some(parent) = path.parent() {
		std::fs::create_dir_all(parent).with_context(|| format!("failed to create {}", parent.display()))?;
	}

	let backup = if path.exists() {
		let stamp = std::time::SystemTime::now()
			.duration_since(std::time::UNIX_EPOCH)
			.map(|elapsed| elapsed.as_millis())
			.unwrap_or_default();
		let backup = path.with_file_name(format!("{file_name}.graphite-agent-backup-{stamp}"));
		std::fs::copy(path, &backup).with_context(|| format!("failed to back up {} to {}", path.display(), backup.display()))?;
		Some(backup)
	} else {
		None
	};

	// Write beside the destination, then rename: a crash never leaves a half file.
	let temporary = path.with_file_name(format!("{file_name}.graphite-agent-tmp"));
	std::fs::write(&temporary, body).with_context(|| format!("failed to write {}", temporary.display()))?;
	std::fs::rename(&temporary, path).with_context(|| format!("failed to replace {}", path.display()))?;
	Ok(backup)
}

/// The user's home directory, for the user-scoped client files.
pub fn home_dir() -> anyhow::Result<PathBuf> {
	for key in ["HOME", "USERPROFILE"] {
		if let Some(value) = std::env::var_os(key)
			&& !value.is_empty()
		{
			return Ok(PathBuf::from(value));
		}
	}
	bail!("cannot determine the home directory: set HOME (or USERPROFILE on Windows)")
}

/// Make a directory absolute, preferring the canonical form when it exists.
pub fn resolve_dir(path: &Path) -> PathBuf {
	if let Ok(canonical) = std::fs::canonicalize(path) {
		return canonical;
	}
	if path.is_absolute() {
		return path.to_path_buf();
	}
	std::env::current_dir().map(|cwd| cwd.join(path)).unwrap_or_else(|_| path.to_path_buf())
}

/// The binary a client should spawn: the explicit override, else this executable.
pub fn resolve_binary(explicit: Option<&str>) -> String {
	if let Some(explicit) = explicit {
		return explicit.to_string();
	}
	std::env::current_exe()
		.ok()
		.map(|path| std::fs::canonicalize(&path).unwrap_or(path))
		.and_then(|path| path.to_str().map(str::to_string))
		.unwrap_or_else(|| "graphite-agent".to_string())
}

fn build_spec(cli_binary: Option<&str>, cli_root: Option<&Path>, project: &Path, capabilities: Option<&str>) -> ServerSpec {
	ServerSpec {
		binary: resolve_binary(cli_binary),
		root: resolve_dir(cli_root.unwrap_or(project)).to_string_lossy().into_owned(),
		capabilities: capabilities.map(str::to_string),
	}
}

fn project_dir(explicit: Option<&Path>) -> PathBuf {
	resolve_dir(explicit.unwrap_or_else(|| Path::new(".")))
}

/// The file body this plan would produce, merged with whatever is already there.
fn preview_body(plan: &Plan) -> anyhow::Result<(String, MergeOutcome)> {
	if plan.body_format == BodyFormat::Toml {
		return Ok((plan.entry.clone(), MergeOutcome::Created));
	}
	let existing = match &plan.path {
		Some(path) if plan.writable => read_optional(path)?,
		_ => None,
	};
	let entry: Value = serde_json::from_str(&plan.entry).context("internal: entry is not JSON")?;
	clients::merge_json(existing.as_deref(), plan.key.context("internal: JSON plan without a key")?, clients::SERVER_NAME, &entry)
}

/// `print-config` — no side effects on the client configuration.
pub fn print_config(args: &PrintConfigArgs) -> anyhow::Result<()> {
	if args.client == Client::All {
		print!("{}", clients::markdown());
		return Ok(());
	}
	if args.format == OutputFormat::Markdown {
		print!("{}", clients::markdown());
		return Ok(());
	}

	let project = project_dir(args.project.as_deref());
	let home = home_dir().unwrap_or_else(|_| project.clone());
	let spec = build_spec(args.binary.as_deref(), args.root.as_deref(), &project, args.capabilities.as_deref());
	let plan = clients::plan(args.client, args.scope, &project, &home, &spec)?;
	let (body, outcome) = preview_body(&plan)?;

	match args.format {
		OutputFormat::Native => {
			if plan.body_format == BodyFormat::Toml {
				print!("{}", plan.entry);
			} else {
				print!("{body}");
			}
			if let Some(shell) = &plan.shell {
				eprintln!("# or use the client's own command:\n# {shell}");
			}
			for step in &plan.steps {
				eprintln!("# {step}");
			}
			if !plan.writable {
				eprintln!("# note: `graphite-agent install` does not edit this client's file.");
			}
		}
		OutputFormat::Json => {
			let envelope = json!({
				"client": args.client.key(),
				"scope": scope_key(plan.scope),
				"path": plan.path.as_ref().map(|path| path.display().to_string()),
				"key": plan.key,
				"format": match plan.body_format { BodyFormat::Json => "json", BodyFormat::Toml => "toml" },
				"body": body,
				"entry": plan.entry,
				"shell": plan.shell,
				"steps": plan.steps,
				"writes_file": plan.writable,
				"outcome": outcome.label(),
			});
			println!("{}", serde_json::to_string_pretty(&envelope)?);
		}
		OutputFormat::Shell => match &plan.shell {
			Some(shell) => println!("{shell}"),
			None => {
				eprintln!("no single command exists for {} {} scope", plan.client.key(), scope_key(plan.scope));
				for step in &plan.steps {
					eprintln!("# {step}");
				}
			}
		},
		OutputFormat::Path => match &plan.path {
			Some(path) => println!("{}", path.display()),
			None => eprintln!("no stable path exists for {} {} scope", plan.client.key(), scope_key(plan.scope)),
		},
		OutputFormat::Markdown => unreachable!("handled above"),
	}
	Ok(())
}

fn scope_key(scope: Scope) -> &'static str {
	match scope {
		Scope::Project => "project",
		Scope::User => "user",
	}
}

/// `install` — merge (or remove) our entry, with a backup, only when confirmed.
pub fn install(args: &InstallArgs) -> anyhow::Result<()> {
	anyhow::ensure!(args.client != Client::All, "`install` configures one client at a time");

	let project = project_dir(args.project.as_deref());
	let home = home_dir().unwrap_or_else(|_| project.clone());
	let spec = build_spec(args.binary.as_deref(), args.root.as_deref(), &project, args.capabilities.as_deref());
	let plan = clients::plan(args.client, args.scope, &project, &home, &spec)?;

	if !plan.writable {
		return install_via_command(args, &plan);
	}

	let path = plan.path.clone().context("internal: writable plan without a path")?;
	let key = plan.key.context("internal: JSON plan without a key")?;
	let existing = read_optional(&path)?;
	let entry: Value = serde_json::from_str(&plan.entry).context("internal: entry is not JSON")?;

	let (body, outcome, verb) = if args.uninstall {
		let Some(existing) = existing else {
			println!("{}: no file to change", path.display());
			return Ok(());
		};
		let (body, removed) = clients::remove_json(&existing, key, clients::SERVER_NAME)?;
		if !removed {
			println!("{}: `{}` was not present", path.display(), clients::SERVER_NAME);
			return Ok(());
		}
		(body, MergeOutcome::Updated, "removed")
	} else {
		let (body, outcome) = clients::merge_json(existing.as_deref(), key, clients::SERVER_NAME, &entry)?;
		(body, outcome, "installed")
	};

	if outcome == MergeOutcome::Unchanged {
		println!("{}: already up to date", path.display());
		return Ok(());
	}

	println!("{}: {}", path.display(), outcome.label());
	if args.dry_run || !args.yes {
		print!("{body}");
		if !args.yes {
			eprintln!("refusing to write without --yes (this was a preview; nothing changed)");
		}
		return Ok(());
	}

	let backup = write_with_backup(&path, &body)?;
	match backup {
		Some(backup) => println!("{verb} into {} (backup: {})", path.display(), backup.display()),
		None => println!("{verb} into {}", path.display()),
	}
	Ok(())
}

/// Clients we never rewrite are configured by their own command.
fn install_via_command(args: &InstallArgs, plan: &Plan) -> anyhow::Result<()> {
	if args.uninstall {
		bail!(
			"`graphite-agent install` never wrote {} configuration, so there is nothing to uninstall; remove it with the client's own command",
			plan.client.key()
		);
	}

	if let Some(path) = &plan.path {
		println!("# file the client reads: {}", path.display());
		println!("{}", plan.entry);
	}
	for step in &plan.steps {
		println!("# {step}");
	}

	let Some(shell) = &plan.shell else {
		println!("# no command exists for this client and scope; follow the steps above.");
		return Ok(());
	};
	println!("# command:\n{shell}");

	if !args.run {
		return Ok(());
	}
	anyhow::ensure!(args.yes, "`--run` executes the client's command; pass --yes to confirm");
	if cfg!(windows) {
		bail!("`--run` needs a POSIX shell; run the command above yourself");
	}

	let status = Command::new("sh").arg("-c").arg(shell).status().with_context(|| format!("failed to run: {shell}"))?;
	anyhow::ensure!(status.success(), "the client's own command failed with {status}");
	Ok(())
}

/// `doctor` — static checks plus a real MCP handshake.
pub fn doctor(args: &DoctorArgs) -> anyhow::Result<()> {
	let project = project_dir(args.project.as_deref());
	let home = home_dir().unwrap_or_else(|_| project.clone());
	let spec = build_spec(args.binary.as_deref(), args.root.as_deref(), &project, None);

	let mut checks = static_checks(&project, &home, &spec);
	if !args.no_handshake {
		checks.push(handshake_check(&spec.binary, Path::new(&spec.root)));
	}

	let failed = checks.iter().filter(|check| !check.ok).count();
	if args.json {
		let payload = json!({
			"ok": failed == 0,
			"failed": failed,
			"checks": checks.iter().map(Check::to_json).collect::<Vec<_>>(),
		});
		println!("{}", serde_json::to_string_pretty(&payload)?);
	} else {
		for check in &checks {
			println!("{} {:<28} {}", if check.ok { "ok  " } else { "FAIL" }, check.name, check.detail);
		}
		println!("\n{} checks, {failed} failed", checks.len());
	}

	if failed > 0 {
		std::process::exit(1);
	}
	Ok(())
}

fn static_checks(project: &Path, home: &Path, spec: &ServerSpec) -> Vec<Check> {
	let mut checks = Vec::new();

	let binary = Path::new(&spec.binary);
	let resolved = if binary.is_absolute() { binary.exists() } else { path_lookup(&spec.binary).is_some() };
	checks.push(Check::new(
		"binary",
		resolved,
		if resolved {
			spec.binary.clone()
		} else {
			format!("`{}` is not on PATH and is not an absolute path; a GUI client will not find it", spec.binary)
		},
	));

	let root = Path::new(&spec.root);
	checks.push(Check::new("root exists", root.is_dir(), format!("{}", root.display())));

	let probe = root.join(".graphite-agent-write-probe");
	let writable = if root.is_dir() {
		let result = std::fs::write(&probe, b"probe").is_ok();
		let _ = std::fs::remove_file(&probe);
		result
	} else {
		false
	};
	checks.push(Check::new(
		"root writable",
		writable,
		if writable { "ok".to_string() } else { format!("cannot write inside {}", root.display()) },
	));

	// The catalog is generated from NODE_METADATA at build time; an empty catalog
	// means the node crates were not linked (see agent/README.md).
	let nodes = graphite_agent_descriptors::node_descriptors_with_identifiers().len();
	checks.push(Check::new("node catalog", nodes > 0, format!("{nodes} node types")));

	for client in clients::CLIENTS {
		for scope in [Scope::Project, Scope::User] {
			let Ok(plan) = clients::plan(client, scope, project, home, spec) else {
				continue;
			};
			let Some(path) = &plan.path else {
				continue;
			};
			// Only report clients that are actually present on this machine.
			if !path.exists() {
				continue;
			}
			let label = format!("{} {}", client.key(), scope_key(scope));
			let text = read_optional(path).unwrap_or(None).unwrap_or_default();
			let install_hint = format!(
				"not configured in {} (run `graphite-agent install --client {} --scope {})",
				path.display(),
				client.key(),
				scope_key(scope)
			);

			// A client file that exists without our entry is the normal "not installed
			// yet" state, not a fault: a developer may use that client for other
			// servers, and failing here would make `doctor` red on a healthy machine.
			// Only the detail differs.
			let detail = if plan.body_format == BodyFormat::Toml {
				if text.contains(&format!("[mcp_servers.{}]", clients::SERVER_NAME)) {
					format!("configured in {}", path.display())
				} else {
					install_hint
				}
			} else {
				let key = plan.key.unwrap_or("mcpServers");
				let entry = serde_json::from_str::<Value>(&text)
					.ok()
					.and_then(|value| value.get(key).and_then(|servers| servers.get(clients::SERVER_NAME)).cloned());
				match entry.as_ref().and_then(|entry| entry.get("command")).and_then(Value::as_str) {
					Some(command) if command == spec.binary => format!("configured in {}", path.display()),
					// A different command is deliberate often enough (a PATH name, or
					// Claude Code's `${VAR:-default}` expansion) that it is reported,
					// not failed.
					Some(command) => format!("configured in {}, spawning `{command}`", path.display()),
					None if entry.is_some() => format!("configured in {} without a command", path.display()),
					None => install_hint,
				}
			};
			checks.push(Check::new(label, true, detail));
		}
	}

	checks
}

fn path_lookup(binary: &str) -> Option<PathBuf> {
	let path = std::env::var_os("PATH")?;
	std::env::split_paths(&path).map(|dir| dir.join(binary)).find(|candidate| candidate.is_file())
}

/// Start the real server and complete an MCP `initialize` handshake.
fn handshake_check(binary: &str, root: &Path) -> Check {
	match handshake(binary, root, HANDSHAKE_TIMEOUT) {
		Ok(report) => {
			let ok = report.tool_count > 0 && report.instructions_present && report.annotated > 0;
			let mut detail = format!("{} tools, instructions {}", report.tool_count, if report.instructions_present { "present" } else { "MISSING" });
			if report.annotated == 0 {
				detail.push_str(", no size annotations");
			} else {
				detail.push_str(&format!(", {} size annotations", report.annotated));
			}
			Check::new("mcp handshake", ok, detail)
		}
		Err(error) => Check::new("mcp handshake", false, format!("{error:#}")),
	}
}

/// What a successful handshake observed.
#[derive(Debug)]
pub struct HandshakeReport {
	pub tool_count: usize,
	pub instructions_present: bool,
	pub annotated: usize,
}

fn handshake(binary: &str, root: &Path, timeout: Duration) -> anyhow::Result<HandshakeReport> {
	let mut child = Command::new(binary)
		.args(["--mode", "headless", "--stdio", "--root"])
		.arg(root)
		.stdin(Stdio::piped())
		.stdout(Stdio::piped())
		.stderr(Stdio::null())
		.spawn()
		.with_context(|| format!("failed to start {binary}"))?;

	let mut stdin = child.stdin.take().context("no stdin")?;
	let stdout = child.stdout.take().context("no stdout")?;
	let (sender, receiver) = mpsc::channel();

	// The reader owns stdout and reports both replies; the main thread keeps the
	// child handle so it can enforce the timeout with a kill.
	std::thread::spawn(move || {
		let mut reader = BufReader::new(stdout);
		let mut replies = Vec::new();
		let requests = [
			json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": { "name": "graphite-agent-doctor", "version": "0" } } }).to_string(),
			json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }).to_string(),
			json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {} }).to_string(),
		];
		for (index, request) in requests.iter().enumerate() {
			if writeln!(stdin, "{request}").and_then(|_| stdin.flush()).is_err() {
				break;
			}
			if index == 1 {
				continue; // a notification has no reply
			}
			let mut line = String::new();
			match reader.read_line(&mut line) {
				Ok(0) | Err(_) => break,
				Ok(_) => replies.push(line),
			}
		}
		let _ = sender.send(replies);
	});

	let replies = match receiver.recv_timeout(timeout) {
		Ok(replies) => replies,
		Err(_) => {
			let _ = child.kill();
			let _ = child.wait();
			bail!("the server did not answer `initialize` within {}s", timeout.as_secs());
		}
	};
	let _ = child.kill();
	let _ = child.wait();

	anyhow::ensure!(replies.len() >= 2, "the server closed the connection before answering `initialize` and `tools/list`");

	let initialize: Value = serde_json::from_str(&replies[0]).context("initialize reply was not JSON")?;
	let tools: Value = serde_json::from_str(&replies[1]).context("tools/list reply was not JSON")?;

	let instructions_present = initialize["result"]["instructions"].as_str().is_some_and(|text| !text.is_empty());
	let tools = tools["result"]["tools"].as_array().cloned().unwrap_or_default();
	let annotated = tools.iter().filter(|tool| tool.get("_meta").is_some()).count();
	let tool_count = tools.len();

	anyhow::ensure!(tool_count > 0, "`tools/list` returned no tools; the node catalog is probably empty");

	Ok(HandshakeReport {
		tool_count,
		instructions_present,
		annotated,
	})
}
