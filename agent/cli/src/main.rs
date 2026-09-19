//! `graphite-agent` — the Graphite agentic MCP binary (T2.10, T4.10).
//!
//! Mode A (headless) and Mode B (attached) are served over stdio and/or the MCP
//! Streamable HTTP adapter. Mode C (peer) arrives in Phase 5.

use anyhow::{Context, bail};
use clap::{Parser, ValueEnum};
use graphite_agent_host::capability::{ALL, ATTACHED_CEILING, PEER_CEILING};
use graphite_agent_host::{AttachedBridge, HeadlessBridge, Host, PeerBridge};
use graphite_agent_mcp::{HttpConfig, HttpServer, serve_stdio};
use graphite_agent_protocol::{Capability, CapabilitySet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
enum Mode {
	Headless,
	Attached,
	Peer,
}

#[derive(Parser, Debug)]
#[command(name = "graphite-agent", version, about = "Graphite agentic MCP server")]
struct Cli {
	/// Session mode.
	#[arg(long, value_enum, default_value_t = Mode::Headless)]
	mode: Mode,

	/// Serve newline-delimited JSON-RPC on stdin/stdout.
	#[arg(long)]
	stdio: bool,

	/// Serve the MCP Streamable HTTP transport on this address (defaults to off).
	#[arg(long)]
	http: Option<String>,

	/// Allow `--http` to bind a non-loopback address. Off by default (T4.8).
	#[arg(long)]
	http_allow_non_loopback: bool,

	/// Confinement root for every file-path tool (INV-12). Required.
	#[arg(long)]
	root: PathBuf,

	/// Working-copy / storage directory (defaults to `<root>/.graphite-agent-working-copy`).
	#[arg(long)]
	storage: Option<PathBuf>,

	/// Unix socket of a live editor session (attached mode).
	#[arg(long)]
	attach: Option<PathBuf>,

	/// `.gdd` document to peer over (peer mode, Phase 5).
	#[arg(long)]
	gdd: Option<PathBuf>,

	/// Per-call timeout in seconds.
	#[arg(long, default_value_t = 30)]
	timeout_seconds: u64,

	/// Comma-separated capability grant set. Defaults to `all` in headless mode and
	/// `read,author` in attached/peer modes (R8-M2).
	#[arg(long)]
	capabilities: Option<String>,
}

fn main() -> anyhow::Result<()> {
	let cli = Cli::parse();

	// T4.7: attached/peer sessions are attenuated to their mode ceiling; a request
	// for a capability outside it is refused as Unauthorized.
	let capabilities = match cli.mode {
		Mode::Headless => CapabilitySet(parse_capabilities(cli.capabilities.as_deref().unwrap_or("all"))?),
		Mode::Attached => {
			let requested = parse_capabilities(cli.capabilities.as_deref().unwrap_or("read,author"))?;
			graphite_agent_host::capability::attenuate(ATTACHED_CEILING, &requested).map_err(|error| anyhow::anyhow!("--capabilities is not permitted in attached mode: {error:?}"))?
		}
		Mode::Peer => {
			let requested = parse_capabilities(cli.capabilities.as_deref().unwrap_or("read,author"))?;
			graphite_agent_host::capability::attenuate(PEER_CEILING, &requested).map_err(|error| anyhow::anyhow!("--capabilities is not permitted in peer mode: {error:?}"))?
		}
	};

	let attach = match cli.mode {
		Mode::Attached => Some(cli.attach.clone().context("--mode attached requires --attach <socket>")?),
		_ => None,
	};
	// T5.7: peer mode opens the `.gdd` working copy named by `--gdd`.
	let gdd = match cli.mode {
		Mode::Peer => Some(cli.gdd.clone().context("--mode peer requires --gdd <path>")?),
		_ => None,
	};

	let root = cli.root.clone();
	let storage = cli.storage.clone();
	let timeout = Duration::from_secs(cli.timeout_seconds.max(1));
	let http = cli.http.clone();
	let http_config = HttpConfig {
		allow_non_loopback: cli.http_allow_non_loopback,
	};

	// §5.5 / R9-H1: the host, its bridges, the modules, and the adapter all hold
	// `!Send` futures, so they run on ONE dedicated thread with a current-thread runtime.
	let worker = std::thread::Builder::new()
		.name("graphite-agent".to_string())
		.spawn(move || -> anyhow::Result<()> {
			let runtime = tokio::runtime::Builder::new_current_thread()
				.enable_all()
				.build()
				.context("failed to build the current-thread runtime")?;
			runtime.block_on(async move {
				let host: Arc<Host> = match gdd {
					Some(path) => {
						let bridge = PeerBridge::open(&path)
							.await
							.map_err(|error| anyhow::anyhow!("failed to open peer document {}: {error:?}", path.display()))?;
						Arc::new(Host::new(Box::new(bridge), capabilities, timeout, root.clone()))
					}
					None => match attach {
						None => {
							let bridge = HeadlessBridge::open(&root, storage.as_deref()).context("failed to construct the headless editor")?;
							Arc::new(Host::new(Box::new(bridge), capabilities, timeout, root.clone()))
						}
						Some(socket) => {
							let bridge = AttachedBridge::connect(&socket).with_context(|| format!("failed to connect to the live editor socket at {}", socket.display()))?;
							Arc::new(Host::new(Box::new(bridge), capabilities, timeout, root.clone()))
						}
					},
				};

				if let Some(address) = http {
					let server = HttpServer::bind(&address, &http_config).map_err(anyhow::Error::msg)?;
					eprintln!("graphite-agent: serving MCP Streamable HTTP on {}", server.local_addr());
					server.serve(host).await;
					Ok(())
				} else {
					serve_stdio(host).await.context("stdio transport failed")
				}
			})
		})
		.context("failed to spawn the agent worker thread")?;

	worker.join().map_err(|_| anyhow::anyhow!("the graphite-agent worker thread panicked"))??;
	Ok(())
}

/// Parse a comma-separated capability spec into a list (order-preserving, deduped).
fn parse_capabilities(spec: &str) -> anyhow::Result<Vec<Capability>> {
	let mut capabilities = Vec::new();
	for entry in spec.split(',').map(str::trim).filter(|entry| !entry.is_empty()) {
		let parsed = match entry.to_ascii_lowercase().as_str() {
			"all" => return Ok(ALL.to_vec()),
			"read" => Capability::Read,
			"author" => Capability::Author,
			"execute" => Capability::Execute,
			"export" => Capability::Export,
			"persist" => Capability::Persist,
			other => bail!("unknown capability `{other}` (expected read, author, execute, export, persist, or all)"),
		};
		if !capabilities.contains(&parsed) {
			capabilities.push(parsed);
		}
	}
	if capabilities.is_empty() {
		bail!("--capabilities must grant at least one capability");
	}
	Ok(capabilities)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn all_is_the_full_grant_set() {
		let capabilities = parse_capabilities("all").expect("parse");
		for capability in [Capability::Read, Capability::Author, Capability::Execute, Capability::Export, Capability::Persist] {
			assert!(capabilities.contains(&capability));
		}
	}

	#[test]
	fn unknown_capabilities_are_rejected() {
		assert!(parse_capabilities("read,teleport").is_err());
	}

	#[test]
	fn attached_mode_attenuates_beyond_the_ceiling() {
		let requested = parse_capabilities("read,author,execute").expect("parse");
		assert!(graphite_agent_host::capability::attenuate(ATTACHED_CEILING, &requested).is_err());
	}
}
