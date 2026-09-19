//! `print-config`, `install`, and `doctor` (E-19).
//!
//! These run the real binary, because the contract that matters is the command
//! line a user or an agent actually types.

use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn binary() -> &'static str {
	env!("CARGO_BIN_EXE_graphite-agent")
}

/// A fresh, empty directory per test (the process id keeps them disjoint).
fn temp_dir(name: &str) -> PathBuf {
	let base = std::env::temp_dir().join(format!("graphite-agent-setup-{name}-{}", std::process::id()));
	let _ = fs::remove_dir_all(&base);
	fs::create_dir_all(&base).expect("create temp dir");
	base
}

fn run(args: &[&str]) -> Output {
	Command::new(binary()).args(args).output().expect("run graphite-agent")
}

/// Run with an isolated HOME, so `doctor` never reports on the developer's real
/// client files and the test is machine-independent.
fn run_with_home(args: &[&str], home: &Path) -> Output {
	Command::new(binary()).args(args).env("HOME", home).env("USERPROFILE", home).output().expect("run graphite-agent")
}

fn stdout(output: &Output) -> String {
	String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
	String::from_utf8_lossy(&output.stderr).into_owned()
}

fn path_str(path: &Path) -> String {
	path.to_string_lossy().into_owned()
}

#[test]
fn print_config_reports_every_client_as_json() {
	let project = temp_dir("print-all");
	for (client, key) in [("claude-code", "mcpServers"), ("codex", ""), ("vscode", "servers"), ("cursor", "mcpServers"), ("gemini", "mcpServers")] {
		let output = run(&[
			"print-config",
			"--client",
			client,
			"--scope",
			"project",
			"--project",
			&path_str(&project),
			"--root",
			&path_str(&project),
			"--format",
			"json",
		]);
		assert!(output.status.success(), "{client} failed: {}", stderr(&output));
		let envelope: Value = serde_json::from_str(&stdout(&output)).unwrap_or_else(|error| panic!("{client} emitted unparseable JSON: {error}"));
		assert_eq!(envelope["client"], client);
		assert_eq!(envelope["scope"], "project");
		if key.is_empty() {
			assert_eq!(envelope["format"], "toml");
			assert_eq!(envelope["writes_file"], false, "codex must never be written by us");
		} else {
			assert_eq!(envelope["key"], key, "{client} used the wrong wrapper key");
			assert_eq!(envelope["format"], "json");
		}
		assert!(envelope["entry"].is_object() || envelope["entry"].is_string(), "{client} produced no entry");
	}
}

#[test]
fn print_config_never_touches_the_filesystem() {
	let project = temp_dir("print-readonly");
	let output = run(&[
		"print-config",
		"--client",
		"claude-code",
		"--scope",
		"project",
		"--project",
		&path_str(&project),
		"--root",
		&path_str(&project),
	]);
	assert!(output.status.success(), "{}", stderr(&output));
	assert!(!project.join(".mcp.json").exists(), "print-config must not create a file");
	assert!(!stdout(&output).is_empty(), "print-config must print the file body on stdout");
}

#[test]
fn install_refuses_to_write_without_confirmation() {
	let project = temp_dir("install-noconfirm");
	let output = run(&[
		"install",
		"--client",
		"claude-code",
		"--scope",
		"project",
		"--project",
		&path_str(&project),
		"--root",
		&path_str(&project),
	]);
	assert!(output.status.success(), "{}", stderr(&output));
	assert!(!project.join(".mcp.json").exists(), "install wrote a file without --yes");
	assert!(stderr(&output).contains("--yes"), "the refusal must explain how to proceed");
	assert!(!stdout(&output).is_empty(), "the preview must be printed");
}

#[test]
fn install_creates_the_file_and_preserves_other_servers() {
	let project = temp_dir("install-merge");
	let config = project.join(".mcp.json");
	fs::write(&config, r#"{ "mcpServers": { "other": { "command": "npx", "args": ["-y", "thing"] } } }"#).expect("seed config");

	let output = run(&[
		"install",
		"--client",
		"claude-code",
		"--scope",
		"project",
		"--project",
		&path_str(&project),
		"--root",
		&path_str(&project),
		"--yes",
	]);
	assert!(output.status.success(), "{}", stderr(&output));

	let merged: Value = serde_json::from_str(&fs::read_to_string(&config).expect("read config")).expect("json");
	assert_eq!(merged["mcpServers"]["other"]["command"], "npx", "an existing server was lost");
	let entry = &merged["mcpServers"]["graphite"];
	assert!(entry["command"].as_str().expect("command").ends_with("graphite-agent"));
	assert!(Path::new(entry["command"].as_str().expect("command")).is_absolute(), "a GUI client needs an absolute path");
	assert_eq!(entry["args"][0], "--mode");
	assert_eq!(entry["args"][1], "headless");
	assert_eq!(entry["args"][2], "--stdio");
	assert_eq!(entry["args"][3], "--root");

	// No backup on the first write, one backup on the second.
	let backups = |dir: &Path| -> usize {
		fs::read_dir(dir)
			.map(|entries| {
				entries
					.filter_map(Result::ok)
					.filter(|entry| entry.file_name().to_string_lossy().contains("graphite-agent-backup"))
					.count()
			})
			.unwrap_or(0)
	};
	// The file already existed, so the first write must keep a backup.
	assert_eq!(backups(&project), 1, "rewriting an existing file must keep a backup");

	// A stale root updates the entry in place and keeps a backup.
	let other_root = temp_dir("install-merge-root");
	let output = run(&[
		"install",
		"--client",
		"claude-code",
		"--scope",
		"project",
		"--project",
		&path_str(&project),
		"--root",
		&path_str(&other_root),
		"--yes",
	]);
	assert!(output.status.success(), "{}", stderr(&output));
	assert_eq!(backups(&project), 2, "each rewrite must keep its own backup");
	let updated: Value = serde_json::from_str(&fs::read_to_string(&config).expect("read config")).expect("json");
	assert!(
		updated["mcpServers"]["graphite"]["args"][4].as_str().expect("root").contains("install-merge-root"),
		"the stale root was not replaced"
	);
	assert_eq!(updated["mcpServers"]["other"]["command"], "npx");
}

#[test]
fn install_is_idempotent() {
	let project = temp_dir("install-idempotent");
	let args = [
		"install",
		"--client",
		"claude-code",
		"--scope",
		"project",
		"--project",
		&path_str(&project),
		"--root",
		&path_str(&project),
		"--yes",
	];
	assert!(run(&args).status.success());
	let first = fs::read_to_string(project.join(".mcp.json")).expect("config");

	let second = run(&args);
	assert!(second.status.success(), "{}", stderr(&second));
	assert!(stdout(&second).contains("already up to date"), "the second run should be a no-op: {}", stdout(&second));
	assert_eq!(fs::read_to_string(project.join(".mcp.json")).expect("config"), first, "the second run changed the file");
}

#[test]
fn dry_run_previews_without_writing() {
	let project = temp_dir("install-dry-run");
	let output = run(&[
		"install",
		"--client",
		"vscode",
		"--scope",
		"project",
		"--project",
		&path_str(&project),
		"--root",
		&path_str(&project),
		"--dry-run",
		"--yes",
	]);
	assert!(output.status.success(), "{}", stderr(&output));
	assert!(!project.join(".vscode").join("mcp.json").exists(), "--dry-run wrote a file");
	assert!(stdout(&output).contains("servers"), "the preview must show the merged body");
}

#[test]
fn uninstall_removes_only_our_entry() {
	let project = temp_dir("install-uninstall");
	let config = project.join(".mcp.json");
	fs::write(&config, r#"{ "mcpServers": { "other": { "command": "npx" }, "graphite": { "command": "/x/graphite-agent" } } }"#).expect("seed");

	let output = run(&["install", "--client", "claude-code", "--scope", "project", "--project", &path_str(&project), "--uninstall", "--yes"]);
	assert!(output.status.success(), "{}", stderr(&output));
	let merged: Value = serde_json::from_str(&fs::read_to_string(&config).expect("read config")).expect("json");
	assert!(merged["mcpServers"]["graphite"].is_null(), "our entry survived");
	assert_eq!(merged["mcpServers"]["other"]["command"], "npx", "another server was removed");
}

#[test]
fn codex_install_prints_the_official_command_and_writes_nothing() {
	let project = temp_dir("install-codex");
	let output = run(&[
		"install",
		"--client",
		"codex",
		"--scope",
		"project",
		"--project",
		&path_str(&project),
		"--root",
		&path_str(&project),
		"--yes",
	]);
	assert!(output.status.success(), "{}", stderr(&output));
	assert!(!project.join(".codex").join("config.toml").exists(), "we must not hand-edit Codex TOML");
	let text = stdout(&output);
	assert!(text.contains("codex mcp add graphite"), "the official command must be printed: {text}");
	assert!(text.contains("startup_timeout_sec = 60"), "the TOML fallback must keep the cold-start timeout");
}

#[test]
fn user_scope_writes_under_home() {
	let home = temp_dir("install-home");
	let project = temp_dir("install-home-project");
	let output = Command::new(binary())
		.args([
			"install",
			"--client",
			"cursor",
			"--scope",
			"user",
			"--project",
			&path_str(&project),
			"--root",
			&path_str(&project),
			"--yes",
		])
		.env("HOME", &home)
		.env("USERPROFILE", &home)
		.output()
		.expect("run");
	assert!(output.status.success(), "{}", stderr(&output));

	let config = home.join(".cursor").join("mcp.json");
	let merged: Value = serde_json::from_str(&fs::read_to_string(&config).expect("read config")).expect("json");
	assert!(merged["mcpServers"]["graphite"]["command"].is_string());
}

#[test]
fn doctor_reports_checks_as_json() {
	let project = temp_dir("doctor-ok");
	let home = temp_dir("doctor-ok-home");
	let output = run_with_home(
		&[
			"doctor",
			"--no-handshake",
			"--json",
			"--project",
			&path_str(&project),
			"--root",
			&path_str(&project),
			"--binary",
			binary(),
		],
		&home,
	);
	assert!(output.status.success(), "{}", stderr(&output));
	let report: Value = serde_json::from_str(&stdout(&output)).expect("doctor JSON");
	assert_eq!(report["ok"], true, "{report}");
	assert_eq!(report["failed"], 0);
	let names: Vec<&str> = report["checks"].as_array().expect("checks").iter().filter_map(|check| check["name"].as_str()).collect();
	for expected in ["binary", "root exists", "root writable", "node catalog"] {
		assert!(names.contains(&expected), "doctor omitted the `{expected}` check: {names:?}");
	}
	// The catalog check proves NODE_METADATA is linked into this binary.
	let catalog = report["checks"]
		.as_array()
		.expect("checks")
		.iter()
		.find(|check| check["name"] == "node catalog")
		.expect("catalog check");
	assert!(catalog["detail"].as_str().expect("detail").contains("335"), "unexpected catalog: {}", catalog["detail"]);
}

#[test]
fn doctor_fails_and_exits_nonzero_on_a_bad_root() {
	let project = temp_dir("doctor-bad");
	let home = temp_dir("doctor-bad-home");
	let output = run_with_home(
		&[
			"doctor",
			"--no-handshake",
			"--json",
			"--project",
			&path_str(&project),
			"--root",
			&path_str(&project.join("does-not-exist")),
			"--binary",
			binary(),
		],
		&home,
	);
	assert!(!output.status.success(), "doctor must exit non-zero when a check fails");
	let report: Value = serde_json::from_str(&stdout(&output)).expect("doctor JSON");
	assert_eq!(report["ok"], false);
	assert!(report["failed"].as_u64().expect("failed") >= 1);
}

#[test]
fn doctor_handshake_proves_the_server_serves() {
	let project = temp_dir("doctor-handshake");
	let home = temp_dir("doctor-handshake-home");
	let output = run_with_home(&["doctor", "--json", "--project", &path_str(&project), "--root", &path_str(&project), "--binary", binary()], &home);
	assert!(output.status.success(), "{}", stderr(&output));
	let report: Value = serde_json::from_str(&stdout(&output)).expect("doctor JSON");
	let handshake = report["checks"]
		.as_array()
		.expect("checks")
		.iter()
		.find(|check| check["name"] == "mcp handshake")
		.expect("handshake check");
	assert_eq!(handshake["ok"], true, "{handshake}");
	let detail = handshake["detail"].as_str().expect("detail");
	assert!(detail.contains("instructions present"), "instructions were not served: {detail}");
	assert!(!detail.contains("0 tools"), "no tools were served: {detail}");
}

/// The guide is generated, so a hand edit must fail the suite (E-19).
#[test]
fn generated_guide_matches_the_committed_file() {
	let output = run(&["print-config", "--client", "all", "--format", "markdown"]);
	assert!(output.status.success(), "{}", stderr(&output));
	let generated = stdout(&output);
	let committed_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("INSTALL.md");
	let committed = fs::read_to_string(&committed_path).unwrap_or_else(|error| panic!("cannot read {}: {error}", committed_path.display()));
	assert_eq!(
		committed,
		generated,
		"{} is stale. Regenerate it with: cargo run -p graphite-agent-cli -- print-config --client all --format markdown > agent/INSTALL.md",
		committed_path.display()
	);
}
