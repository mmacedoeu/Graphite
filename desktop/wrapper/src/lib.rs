use graph_craft::application_io::PlatformApplicationIo;
use graph_craft::application_io::resource::ResourceStorage;
use graphite_editor::application::{Editor, Environment, Host, Platform};
use graphite_editor::messages::frontend::FrontendMessage;
use graphite_editor::messages::prelude::Wake;

use message_dispatcher::DesktopWrapperMessageDispatcher;
use messages::{DesktopFrontendMessage, DesktopWrapperMessage};
use std::sync::Arc;

pub use graph_craft::application_io::resource::MmapResourceStorage;
// T4.1/T4.3: the desktop crate does not depend on `agent/protocol` directly, so the
// wrapper re-exports the contract crate and the editor's agent message/sink types.
pub use graphite_agent_protocol;
pub use graphite_editor::consts::{DOUBLE_CLICK_MILLISECONDS, FILE_EXTENSION};
pub use graphite_editor::messages::prelude::{AgentMessage, AgentReplySink, Message as EditorMessage};
pub use wgpu_executor::Texture;
pub use wgpu_executor::WgpuBackends;
pub use wgpu_executor::WgpuContext;
pub use wgpu_executor::WgpuContextBuilder;
pub use wgpu_executor::WgpuCurrentSurfaceTexture;
pub use wgpu_executor::WgpuExecutor;
pub use wgpu_executor::WgpuFeatures;
pub use wgpu_executor::WgpuInstance;
pub use wgpu_executor::WgpuSurface;

mod handle_desktop_wrapper_message;
mod intercept_editor_message;
mod intercept_frontend_message;
mod message_dispatcher;
pub mod messages;
pub(crate) mod utils;

pub struct DesktopWrapper {
	editor: Editor,
}

impl DesktopWrapper {
	pub fn new(uuid_random_seed: u64, resource_storage: Arc<dyn ResourceStorage>, working_copy_root: std::path::PathBuf, wgpu_context: WgpuContext, schedule_wake: Wake) -> Self {
		#[cfg(target_os = "windows")]
		let host = Host::Windows;
		#[cfg(target_os = "macos")]
		let host = Host::Mac;
		#[cfg(target_os = "linux")]
		let host = Host::Linux;
		let env = Environment { platform: Platform::Desktop, host };
		let application_io = PlatformApplicationIo::new_with_context(wgpu_context);

		Self {
			editor: Editor::new(env, uuid_random_seed, resource_storage, Some(working_copy_root), application_io, schedule_wake),
		}
	}

	pub fn dispatch(&mut self, message: DesktopWrapperMessage) -> Vec<DesktopFrontendMessage> {
		let mut executor = DesktopWrapperMessageDispatcher::new(&mut self.editor);
		executor.queue_desktop_wrapper_message(message);
		executor.execute()
	}

	/// Install the agent reply sink (T4.3). Only called when the desktop was started
	/// with `--agent-bridge`; without that flag the editor has no agent sink and
	/// behavior is unchanged.
	pub fn set_agent_reply_sink(&mut self, sink: AgentReplySink) {
		self.editor.set_agent_reply_sink(sink);
	}

	/// Dispatch one inbound `AgentMessage` (curated; never an arbitrary `Message`,
	/// INV-13) into the editor and return the frontend messages it produced (T4.3).
	pub fn dispatch_agent_message(&mut self, message: AgentMessage) -> Vec<DesktopFrontendMessage> {
		let mut executor = DesktopWrapperMessageDispatcher::new(&mut self.editor);
		executor.queue_editor_message(message);
		executor.execute()
	}

	pub async fn execute_node_graph() -> NodeGraphExecutionResult {
		let result = graphite_editor::node_graph_executor::run_node_graph().await;
		match result {
			(true, texture) => NodeGraphExecutionResult::HasRun(texture),
			(false, _) => NodeGraphExecutionResult::NotRun,
		}
	}
}

pub enum NodeGraphExecutionResult {
	HasRun(Option<Texture>),
	NotRun,
}

pub fn deserialize_editor_message(data: &[u8]) -> Option<DesktopWrapperMessage> {
	let message = graphite_wasm_wrapper::native_communication::decode_editor_command(data)?;
	Some(DesktopWrapperMessage::FromWeb(message.into()))
}

pub fn serialize_frontend_messages(messages: Vec<FrontendMessage>) -> Option<Vec<u8>> {
	graphite_wasm_wrapper::native_communication::encode_frontend_messages(messages)
}
