#[derive(clap::Parser)]
#[clap(name = "graphite", version)]
pub struct Cli {
	#[arg(help = "Files to open on startup")]
	pub files: Vec<std::path::PathBuf>,

	#[arg(long, action = clap::ArgAction::SetTrue, help = "Disable hardware accelerated UI rendering")]
	pub disable_ui_acceleration: bool,

	/// Serve the bidirectional agent bridge on the desktop local socket (T4.2).
	/// Off by default; without it the desktop behaves exactly as before.
	#[arg(long, action = clap::ArgAction::SetTrue, help = "Serve the agent bridge on the desktop socket")]
	pub agent_bridge: bool,
}
