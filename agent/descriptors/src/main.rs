//! `graphite-agent-descriptors` — the Phase 0 catalog/inventory binary (T0.5),
//! extended in Phase 3 (T3.6).
//!
//! Subcommands:
//! - `inventory --out <path>`: write `agent/inventory.json` (T0.6).
//! - `coverage`: print node + message-action coverage, including every
//!   non-allowlisted message and its status (T3.6).

use clap::{Parser, Subcommand};
use graphite_agent_descriptors::{inventory, node_catalog_json};
use std::error::Error;
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[clap(name = "graphite-agent-descriptors", version)]
struct App {
	#[clap(subcommand)]
	command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
	/// Write the generated node + allowlist inventory as JSON.
	Inventory {
		/// Output path (for example `agent/inventory.json`).
		#[clap(long)]
		out: PathBuf,
	},
	/// Print node-type and allowlist coverage.
	Coverage,
}

fn main() -> Result<(), Box<dyn Error>> {
	let app = App::parse();

	match app.command {
		Command::Inventory { out } => {
			inventory::write_inventory(&out)?;
			let catalog = node_catalog_json();
			let node_count = catalog.get("count").and_then(serde_json::Value::as_u64).unwrap_or(0);
			eprintln!("wrote {} node descriptors to {}", node_count, out.display());
		}
		Command::Coverage => {
			println!("{}", serde_json::to_string_pretty(&inventory::coverage_json())?);
		}
	}

	Ok(())
}
