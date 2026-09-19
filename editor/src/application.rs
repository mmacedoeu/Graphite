use crate::dispatcher::Dispatcher;
use crate::messages::prelude::*;
use graph_craft::application_io::PlatformApplicationIo;
use graph_craft::application_io::resource::ResourceStorage;
pub use graphene_std::uuid::*;
use std::sync::{Arc, OnceLock};

pub struct Editor {
	pub dispatcher: Dispatcher,
}

/// Everything [`Editor::new_headless`] needs, so the host constructs it without
/// touching editor internals (M-A8 / B-M7).
#[cfg(not(target_family = "wasm"))]
pub struct HeadlessEditorState {
	/// Use the type `editor/src/application.rs` already imports; `graphene-resource`
	/// is NOT a direct dependency of `editor` (M-B2).
	pub resource_storage: Arc<dyn ResourceStorage>,
	pub working_copy_root: std::path::PathBuf,
	pub uuid_random_seed: u64,
}

#[cfg(not(target_family = "wasm"))]
impl Editor {
	/// Uses `Environment { platform: Platform::Desktop, host: <compile-time host> }`,
	/// `PlatformApplicationIo::default()`, and a real signaling `Wake`.
	/// Install the reply sink via `set_agent_reply_sink` before or after.
	///
	/// The returned `Wake` signals the process-global headless `Notify`, which the
	/// host awaits via `crate::messages::future::headless_wake_notify()` (§5.4, INV-11).
	pub fn new_headless(state: HeadlessEditorState) -> (Self, Wake) {
		let wake = crate::messages::future::headless_wake();
		let editor = Self::new(
			Environment {
				platform: Platform::Desktop,
				host: compile_time_host(),
			},
			state.uuid_random_seed,
			state.resource_storage,
			Some(state.working_copy_root),
			PlatformApplicationIo::default(),
			wake.clone(),
		);
		(editor, wake)
	}
}

#[cfg(not(target_family = "wasm"))]
const fn compile_time_host() -> Host {
	#[cfg(target_os = "windows")]
	{
		Host::Windows
	}
	#[cfg(target_os = "macos")]
	{
		Host::Mac
	}
	#[cfg(not(any(target_os = "windows", target_os = "macos")))]
	{
		Host::Linux
	}
}

impl Editor {
	pub fn new(
		environment: Environment,
		uuid_random_seed: u64,
		resource_storage: Arc<dyn ResourceStorage>,
		working_copy_root: Option<std::path::PathBuf>,
		mut application_io: PlatformApplicationIo,
		wake: Wake,
	) -> Self {
		ENVIRONMENT.set(environment).expect("Editor shoud only be initialized once");
		graphene_std::uuid::set_uuid_seed(uuid_random_seed);

		let mut dispatcher = Dispatcher::new(resource_storage, working_copy_root);
		dispatcher.message_handlers.future_message_handler.set_wake(wake);
		application_io.inject_resource_proxy(dispatcher.message_handlers.resource_storage_message_handler.resources());
		crate::node_graph_executor::replace_application_io(application_io);

		Self { dispatcher }
	}

	#[cfg(test)]
	pub(crate) fn new_local_executor() -> (Self, crate::node_graph_executor::NodeRuntime) {
		let _ = ENVIRONMENT.set(*Editor::environment());
		graphene_std::uuid::set_uuid_seed(0);

		let (mut runtime, executor) = crate::node_graph_executor::NodeGraphExecutor::new_with_local_runtime();
		let editor = Self {
			dispatcher: Dispatcher::with_executor(executor),
		};

		let mut application_io = PlatformApplicationIo::default();
		application_io.inject_resource_proxy(editor.dispatcher.message_handlers.resource_storage_message_handler.resources());
		runtime.replace_application_io(application_io);

		(editor, runtime)
	}

	pub fn handle_message<T: Into<Message>>(&mut self, message: T) -> Vec<FrontendMessage> {
		self.dispatcher.handle_message(message, true);

		std::mem::take(&mut self.dispatcher.responses)
	}

	/// Install the host-owned agent reply sink. Additive: `Editor::new`'s signature is
	/// unchanged, so no out-of-scope caller has to be edited (B-M1).
	pub fn set_agent_reply_sink(&mut self, sink: AgentReplySink) {
		self.dispatcher.message_handlers.agent_message_handler.set_reply_sink(sink);
	}

	pub fn poll_node_graph_evaluation(&mut self, responses: &mut VecDeque<Message>) -> Result<(), String> {
		self.dispatcher.poll_node_graph_evaluation(responses)
	}
}

static ENVIRONMENT: OnceLock<Environment> = OnceLock::new();
impl Editor {
	#[cfg(not(test))]
	pub fn environment() -> &'static Environment {
		ENVIRONMENT.get().expect("Editor environment accessed before initialization")
	}

	#[cfg(test)]
	pub fn environment() -> &'static Environment {
		&Environment {
			platform: Platform::Desktop,
			host: Host::Linux,
		}
	}
}

#[derive(Clone, Copy, Debug)]
pub struct Environment {
	pub platform: Platform,
	pub host: Host,
}
#[derive(Clone, Copy, Debug)]
pub enum Platform {
	Desktop,
	Web,
}
#[derive(Clone, Copy, Debug)]
pub enum Host {
	Windows,
	Mac,
	Linux,
}
impl Environment {
	pub fn is_desktop(&self) -> bool {
		matches!(self.platform, Platform::Desktop)
	}
	pub fn is_web(&self) -> bool {
		matches!(self.platform, Platform::Web)
	}
	pub fn is_windows(&self) -> bool {
		matches!(self.host, Host::Windows)
	}
	pub fn is_mac(&self) -> bool {
		matches!(self.host, Host::Mac)
	}
	pub fn is_linux(&self) -> bool {
		matches!(self.host, Host::Linux)
	}
}

pub const GRAPHITE_RELEASE_SERIES: &str = env!("GRAPHITE_RELEASE_SERIES");
pub const GRAPHITE_GIT_COMMIT_BRANCH: Option<&str> = option_env!("GRAPHITE_GIT_COMMIT_BRANCH");
pub const GRAPHITE_GIT_COMMIT_HASH: &str = env!("GRAPHITE_GIT_COMMIT_HASH");
pub const GRAPHITE_GIT_COMMIT_DATE: &str = env!("GRAPHITE_GIT_COMMIT_DATE");

pub fn commit_info_localized(localized_commit_date: &str) -> String {
	let mut info = String::new();
	info.push_str(&format!("Release Series: {GRAPHITE_RELEASE_SERIES}\n"));
	if let Some(branch) = GRAPHITE_GIT_COMMIT_BRANCH {
		info.push_str(&format!("Branch: {branch}\n"));
	}
	info.push_str(&format!("Commit: {}\n", GRAPHITE_GIT_COMMIT_HASH.get(..8).unwrap_or(GRAPHITE_GIT_COMMIT_HASH)));
	info.push_str(localized_commit_date);
	info
}
