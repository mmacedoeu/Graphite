//! Headless engine plumbing shared by the CLI and other crates.
//!
//! This module owns the `.gdd`-to-runtime setup (opening an archive, converting the registry to a
//! runtime network, building the platform application I/O and editor API) as well as graph
//! compilation and executor construction.

use crate::export;
use document_container::AnyContainer;
use document_container::backends::memory::MemoryBackend;
use document_format::{Gdd, GddV1Layout};
use futures::executor::block_on;
use graph_craft::application_io::{EditorPreferences, PlatformApplicationIo, PlatformEditorApi};
use graph_craft::document::*;
use graph_craft::graphene_compiler::Compiler;
use graph_craft::proto::ProtoNetwork;
use graphene_std::application_io::{ApplicationIo, NodeGraphUpdateMessage, NodeGraphUpdateSender};
use interpreted_executor::dynamic_executor::DynamicExecutor;
use interpreted_executor::util::wrap_network_in_scope;
use std::error::Error;
use std::sync::Arc;
use std::time::Duration;

/// The `NodeGraphUpdateSender` used by headless consumers.
///
/// It must never write to stdout, which is reserved for machine-readable CLI output (INV-10), so
/// update messages go to stderr.
struct UpdateLogger;

impl NodeGraphUpdateSender for UpdateLogger {
	fn send(&self, message: NodeGraphUpdateMessage) {
		eprintln!("{message:?}");
	}
}

/// Open a `.gdd` document from its archive bytes.
pub async fn open_gdd(gdd_bytes: &[u8]) -> Result<Gdd, Box<dyn Error>> {
	let container = AnyContainer::Memory(MemoryBackend::new());
	let gdd = document_format::Gdd::open_from_archive(gdd_bytes, container, GddV1Layout)
		.await
		.map_err(|error| format!("Failed to open document: {error}"))?;
	Ok(gdd)
}

/// Convert an opened `.gdd` registry into a runtime node network.
pub async fn runtime_network_from_gdd(gdd: &Gdd) -> Result<NodeNetwork, Box<dyn Error>> {
	let declarations = gdd.declarations(gdd).await;
	let (node_network, _metadata) = gdd.registry().to_runtime_with_metadata(&declarations)?;
	Ok(node_network)
}

/// Build the platform application I/O, injecting the `.gdd` resource proxy when present.
pub async fn create_application_io(gdd: Option<&Gdd>) -> PlatformApplicationIo {
	let mut application_io = PlatformApplicationIo::new().await;
	if let Some(gdd) = gdd {
		application_io.inject_resource_proxy(Box::new(gdd.resource_proxy()));
	}
	application_io
}

/// Build the editor API used to compile a runtime network into a proto graph.
pub fn create_editor_api(application_io: Arc<PlatformApplicationIo>) -> Arc<PlatformEditorApi> {
	let preferences = EditorPreferences {
		max_render_region_size: EditorPreferences::default().max_render_region_size,
	};
	Arc::new(PlatformEditorApi {
		application_io: Some(application_io),
		node_graph_message_sender: Box::new(UpdateLogger),
		editor_preferences: Box::new(preferences),
	})
}

/// Compile a runtime node network into a proto graph.
pub fn compile_graph(network: NodeNetwork, editor_api: Arc<PlatformEditorApi>, gdd: Option<&Gdd>) -> Result<ProtoNetwork, Box<dyn Error>> {
	let preprocessor = preprocessor::Preprocessor::new();

	let mut network = wrap_network_in_scope(network, editor_api);

	// A `.gdd` resolves resource hashes from its registry; a legacy `.graphite` has no resource store, so it
	// preprocesses against an empty registry (matching the pre-`.gdd` CLI behavior).
	match gdd {
		Some(gdd) => preprocessor
			.preprocess(&mut network, &|resource_id| gdd.registry().resources.get(&resource_id).and_then(|r| r.hash))
			.expect("Failed to expand network"),
		None => { preprocessor.preprocess(&mut network, &|_| None) }.expect("Failed to expand network"),
	}

	let compiler = Compiler {};
	compiler.compile_single(network).map_err(|x| x.into())
}

/// Build a dynamic executor from a compiled proto graph.
pub fn create_executor(proto_network: ProtoNetwork) -> Result<DynamicExecutor, Box<dyn Error>> {
	let executor = block_on(DynamicExecutor::new(proto_network)).map_err(|errors| errors.iter().map(|e| format!("{e:?}")).reduce(|acc, e| format!("{acc}\n{e}")).unwrap_or_default())?;
	Ok(executor)
}

/// Render a `.gdd` document to PNG bytes, bounded by `max_dimension` in each axis.
///
/// This is the single public rendering entry point for other crates: it owns the whole headless
/// pipeline (archive open, runtime conversion, GPU context, compilation, executor, device polling)
/// so callers never need to touch GPU or document internals. Uses a scale of `1.0` and an opaque
/// (non-transparent) background.
pub async fn render_gdd_to_png(gdd_bytes: &[u8], max_dimension: u32) -> Result<Vec<u8>, Box<dyn Error>> {
	let gdd = open_gdd(gdd_bytes).await?;
	let node_network = runtime_network_from_gdd(&gdd).await?;

	let application_io = Arc::new(create_application_io(Some(&gdd)).await);
	let editor_api = create_editor_api(application_io.clone());
	let proto_graph = compile_graph(node_network, editor_api, Some(&gdd))?;

	let wgpu_executor = application_io.gpu_executor().ok_or("GPU executor not available")?;
	let device = wgpu_executor.context().device.clone();

	// Spawn a thread to poll the GPU device while rendering, mirroring the `Export` subcommand so the
	// render future does not hang.
	std::thread::spawn(move || {
		loop {
			std::thread::sleep(Duration::from_nanos(10));
			device.poll(wgpu::PollType::Poll).unwrap();
		}
	});

	let executor = create_executor(proto_graph)?;

	// The headless library path has no editor-side document metadata (bounds, artboards), so it
	// renders a square viewport whose side is bounded by `max_dimension`.
	let dimension = max_dimension.max(1);
	export::render_document_to_png_bytes(&executor, wgpu_executor, 1.0, (Some(dimension), Some(dimension)), false).await
}
