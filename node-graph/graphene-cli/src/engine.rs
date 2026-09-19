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
use graph_craft::document::NodeNetwork;
use graph_craft::document::value::{RenderOutputType, TaggedValue, UVec2};
use graph_craft::graphene_compiler::Compiler;
use graph_craft::graphene_compiler::Executor;
use graph_craft::proto::ProtoNetwork;
use graphene_std::application_io::{
	ApplicationIo, ExportFormat, NodeGraphUpdateMessage, NodeGraphUpdateSender, RenderConfig, TimingInformation,
};
use graphene_std::core_types::ops::Convert;
use graphene_std::core_types::transform::Footprint;
use graphene_std::raster_types::{CPU, GPU, Raster};
use image::codecs::gif::{GifEncoder, Repeat};
use image::{Delay, Frame, RgbaImage};
use interpreted_executor::dynamic_executor::DynamicExecutor;
use interpreted_executor::util::wrap_network_in_scope;
use std::error::Error;
use std::io::Cursor;
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

/// Render a `.gdd` document to an animated GIF byte buffer at the requested framerate.
///
/// This is the in-memory counterpart to [`crate::export::export_gif`] (file-only). It
/// mirrors [`render_gdd_to_png`] (the only public headless PNG render): same archive
/// open, same runtime conversion, same GPU context, same compilation, same executor
/// construction, same device-polling thread. The only difference is that animation
/// time is advanced per frame via `RenderConfig.time.animation_time`, which any
/// `ExtractAnimationTime` consumer (Animation Time, Quantize Animation Time, and
/// the `render_cache` key derivation) sees as a fresh value per `encode_frame`
/// call. Frames are concatenated into an infinite-looping GIF.
///
/// `frames == 0` and non-finite / non-positive `fps` are rejected with an error.
/// `max_dimension == 0` is clamped to 1 (matching the single-frame path's
/// dimension guarantee). At the default `fps = 30`, `frames = 60`,
/// `max_dimension = 1024`, the encoded buffer stays under ~5 MB; callers (such as
/// the agent host) are responsible for any further size cap and apply an
/// `anthropic/maxResultSizeChars` ceiling on the base64-encoded form.
pub async fn render_gdd_to_gif_bytes(
	gdd_bytes: &[u8],
	fps: f64,
	frames: u32,
	max_dimension: u32,
) -> Result<Vec<u8>, Box<dyn Error>> {
	if frames == 0 {
		return Err("render_gdd_to_gif_bytes: frames must be >= 1".into());
	}
	if !fps.is_finite() || fps <= 0.0 {
		return Err("render_gdd_to_gif_bytes: fps must be a positive finite number".into());
	}
	let dimension = max_dimension.max(1);
	// GIF per-frame delay is in 10ms units; clamp so we never emit a zero-delay
	// frame the browser would render as fast as it can.
	let frame_delay_centis = (100.0 / fps).round().clamp(1.0, u16::MAX as f64) as u16;

	let gdd = open_gdd(gdd_bytes).await?;
	let node_network = runtime_network_from_gdd(&gdd).await?;

	let application_io = Arc::new(create_application_io(Some(&gdd)).await);
	let editor_api = create_editor_api(application_io.clone());
	let proto_graph = compile_graph(node_network, editor_api, Some(&gdd))?;

	let wgpu_executor = application_io.gpu_executor().ok_or("GPU executor not available")?;
	let device = wgpu_executor.context().device.clone();

	// Spawn a thread to poll the GPU device while rendering, mirroring the `Export`
	// subcommand so the render future does not hang.
	std::thread::spawn(move || loop {
		std::thread::sleep(Duration::from_nanos(10));
		device.poll(wgpu::PollType::Poll).unwrap();
	});

	let executor = create_executor(proto_graph)?;

	let mut gif_bytes: Vec<u8> = Vec::new();
	{
		let writer = Cursor::new(&mut gif_bytes);
		let mut encoder = GifEncoder::new_with_speed(writer, 10);
		encoder.set_repeat(Repeat::Infinite)?;

		for frame_idx in 0..frames {
			let animation_time = Duration::from_secs_f64(frame_idx as f64 / fps);
			let mut render_config = RenderConfig {
				scale: 1.0,
				export_format: ExportFormat::Raster,
				for_export: true,
				time: TimingInformation {
					time: animation_time.as_secs_f64(),
					animation_time,
				},
				..Default::default()
			};
			render_config.viewport.resolution = UVec2::new(dimension, dimension);

			let result = (&executor).execute(render_config.into_context()).await?;

			let (data, img_width, img_height) = match result {
				TaggedValue::RenderOutput(output) => match output.data {
					RenderOutputType::Texture(texture) => {
						let gpu_raster = Raster::<GPU>::new_gpu(texture);
						let cpu_raster: Raster<CPU> = gpu_raster.convert(Footprint::BOUNDLESS, wgpu_executor).await;
						cpu_raster.to_flat_u8()
					}
					RenderOutputType::Buffer { data, width, height } => (data, width, height),
					other => {
						return Err(format!("render_gdd_to_gif_bytes: unexpected render output type for GIF frame: {:?}", other).into());
					}
				},
				other => {
					return Err(format!("render_gdd_to_gif_bytes: expected RenderOutput for GIF frame, got: {:?}", other).into());
				}
			};

			let image = RgbaImage::from_raw(img_width, img_height, data)
				.ok_or_else(|| "render_gdd_to_gif_bytes: failed to create frame image from frame buffer")?;
			let frame = Frame::from_parts(
				image,
				0,
				0,
				Delay::from_saturating_duration(Duration::from_millis(frame_delay_centis as u64 * 10)),
			);
			encoder.encode_frame(frame)?;
		}
	}

	Ok(gif_bytes)
}
