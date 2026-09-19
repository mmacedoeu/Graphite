//! Graphite agentic MCP host: the `ToolHost` implementation, the curated tool
//! modules, and the mode bridges (headless in Phase 2; attached/peer later).

// Bumped past the default 128 for the same reason `editor/src/lib.rs` is: checking
// that the `Editor`-owning `HeadlessBridge` is `Send` pulls in wgpu/naga trait
// chains that overflow the default trait-resolver recursion limit.
#![recursion_limit = "256"]

pub mod attached;
pub mod capability;
pub mod headless;
pub mod host;
pub mod modules;
pub mod paths;
pub mod peer;

pub use attached::{AttachedBridge, Frame, ScriptedBridgeServer};
pub use headless::HeadlessBridge;
pub use host::{BridgeExt, DocumentChangeDebouncer, Host};
pub use paths::PathRoot;
pub use peer::{PeerBridge, PeerHandle, PeerState};
