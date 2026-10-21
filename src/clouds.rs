//! Screen-space volumetric clouds: raymarched per pixel (at a fraction of
//! the view's resolution, `clouds.resolution`) through a cloud layer over a
//! round earth, shaped by tiling Perlin-Worley and Worley noise
//! ([`crate::cloud_noise`], made once at startup) with curl-swirled detail
//! eroding the edges, a coverage map and height profiles by cloud type;
//! lit by light marched toward the sun (Beer-Lambert with the powder term,
//! a two-lobed phase for the silver lining, multiple-scattering octaves)
//! and by the sky round them. Each frame's march is jittered and blended
//! with the last frame's, reprojected, so few steps still converge sharp.
//!
//! These are the only clouds. A compute pass (`clouds.wgsl`) writes them
//! (radiance and transmittance) before the camera draws; a fullscreen quad
//! at the far plane (`cloud_layer.wgsl`) lays them over the sky's pixels.
//! The same pass draws the reflections' and ambient cube from the same
//! clouds (`cube_main`: same model, knobs and lighting, 256 a face, a face a
//! frame) over the sky behind them; the sky's own cube (`crate::sky`) has no
//! clouds.
//!
//! Knobs: `clouds.quality` (0 off: no clouds; 1 low, 2 medium, 3 high: the
//! march's steps and resolution), `clouds.density` (1), `clouds.erosion`
//! (1, detail eating the edges), `clouds.resolution` (of the view; unset:
//! the quality's), plus the sky's `sky.cover`, `sky.darkness`,
//! `sky.altitude`, wind and time of day.

use bevy::asset::RenderAssetUsages;
use bevy::core_pipeline::schedule::camera_driver;
use bevy::light::Skybox;
use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::pbr::{MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::render::extract_resource::{ExtractResource, ExtractResourcePlugin};
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_resource::binding_types::{sampler, texture_2d, texture_3d, texture_cube, texture_storage_2d, texture_storage_2d_array, uniform_buffer};
use bevy::render::render_resource::*;
use bevy::render::renderer::{RenderContext, RenderDevice, RenderGraph, RenderQueue};
use bevy::render::texture::GpuImage;
use bevy::render::{Render, RenderApp, RenderStartup, RenderSystems};
use bevy::shader::ShaderRef;
use std::sync::Mutex;
use std::sync::mpsc::Receiver;

pub struct CloudsPlugin;

impl Plugin for CloudsPlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "clouds.wgsl");
        bevy::asset::embedded_asset!(app, "cloud_layer.wgsl");
        // (The noise on a thread of its own: a second or two.)
        let (send, receive) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name("atmos cloud noise".into())
            .spawn(move || {
                let started = std::time::Instant::now();
                let noise = crate::cloud_noise::make();
                info!("atmos: cloud noise made in {:.1} s", started.elapsed().as_secs_f32());
                send.send(noise).ok();
            })
            .ok();
        app.insert_resource(NoiseJob(Mutex::new(Some(receive))))
            .add_plugins((ExtractResourcePlugin::<ScreenClouds>::default(), MaterialPlugin::<CloudLayer>::default()))
            .add_systems(Update, receive_noise)
            .add_systems(PostUpdate, update.after(bevy::transform::TransformSystems::Propagate).after(bevy::camera::CameraUpdateSystems));
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else { return };
        render_app
            .add_systems(RenderStartup, init_pipeline)
            .add_systems(Render, prepare.in_set(RenderSystems::PrepareBindGroups))
            .add_systems(RenderGraph, draw.in_set(bevy::render::renderer::RenderGraphSystems::Render).before(camera_driver));
    }
}

/// A host drawing its own sky (not `AtmosPlugin`'s): what the clouds need
/// from it, set each frame. With this resource the clouds take it in place
/// of the plugin's sky. The host's sky cube should have no clouds of its own
/// (they're drawn over it per pixel).
#[derive(Resource, Clone, Debug)]
pub struct HostSky {
    /// Draw the clouds.
    pub enabled: bool,
    /// The host's sky behind the clouds (a cube map, the skybox's units):
    /// the clouds' ambient light comes from it.
    pub background: Handle<Image>,
    /// A cube map with clouds for reflections ([`crate::modern_water`]
    /// reflects this when given), else the skybox.
    pub reflections: Option<Handle<Image>>,
    /// Toward the light the clouds take (the sun, or the moon by night),
    /// and its colour and strength (1: the sun at full).
    pub key_dir: Vec3,
    pub key_color: Vec3,
    /// 0 day .. 1 night.
    pub night: f32,
    /// How much of the sky they cover, 0..1, and how dark their undersides.
    pub cover: f32,
    pub darkness: f32,
    /// How far they've drifted (m, world x and z).
    pub drift: Vec2,
    /// How far their tops rise past the usual (m: storm towers).
    pub tops: f32,
    /// 0 fair .. 1 a storm: heavier, darker bases.
    pub storm: f32,
    /// A lightning flash now (0 none, ~1 full) and toward its bolt: it lights
    /// the clouds round it from inside.
    pub flash: f32,
    pub bolt_dir: Vec3,
    /// The horizon's colour (the skybox's units) and whether it's fogged.
    pub horizon: Vec3,
    pub fog: bool,
    /// The sky's grade (rgb multiplier).
    pub grade: Vec3,
}

/// The clouds' quality (`clouds.quality`): 0 off (no clouds at all), 1 low,
/// 2 medium, 3 high: the march's steps and the resolution they're drawn at.
pub fn quality() -> u32 {
    crate::knobs::get("clouds.quality", 2.0).round().clamp(0.0, 3.0) as u32
}

/// Steps through the layer, and the share of the view's resolution (and
/// the most pixels across) the clouds are drawn at, for a quality.
fn quality_settings(q: u32) -> (f32, f32, f32) {
    match q {
        1 => (40.0, 0.3, 640.0),
        3 => (112.0, 0.75, 1440.0),
        _ => (96.0, 0.5, 960.0),
    }
}

/// A quality's look: how much fine detail, how sharp the edges.
fn quality_shape(q: u32) -> (f32, f32) {
    match q {
        1 => (0.0, 1.1),
        3 => (1.0, 4.5),
        _ => (0.85, 2.8),
    }
}

/// The reflections' cube's face size (`crate::sky`'s, drawn here).
pub const ENV_SIZE: u32 = 256;

#[derive(Resource)]
struct NoiseJob(Mutex<Option<Receiver<crate::cloud_noise::CloudNoise>>>);

/// The noise textures, once made.
#[derive(Resource, Clone)]
struct NoiseImages {
    base: Handle<Image>,
    detail: Handle<Image>,
    curl: Handle<Image>,
}

fn receive_noise(mut commands: Commands, job: Res<NoiseJob>, mut images: ResMut<Assets<Image>>) {
    let Ok(mut slot) = job.0.lock() else { return };
    let Some(rx) = slot.as_ref() else { return };
    let Ok(noise) = rx.try_recv() else { return };
    *slot = None;
    let image = |t: crate::cloud_noise::Texels| {
        let dimension = if t.depth > 1 { TextureDimension::D3 } else { TextureDimension::D2 };
        Image::new(Extent3d { width: t.size, height: t.size, depth_or_array_layers: t.depth }, dimension, t.data, TextureFormat::Rgba8Unorm, RenderAssetUsages::RENDER_WORLD)
    };
    commands.insert_resource(NoiseImages { base: images.add(image(noise.base)), detail: images.add(image(noise.detail)), curl: images.add(image(noise.curl)) });
}

#[derive(Clone, Copy, Debug, Default, ShaderType)]
struct CloudParams {
    /// From clip space to the world (the ray through each pixel).
    world_from_clip: Mat4,
    /// Last frame's world to clip (reprojection).
    prev_clip_from_world: Mat4,
    /// xyz: the camera; w: the frame number.
    camera: Vec4,
    /// xyz: toward the key light (the sun, or the moon by night); w: night.
    key_dir: Vec4,
    /// rgb: the key light's colour and strength; w: darkness.
    key_color: Vec4,
    /// xy: how far the clouds have drifted (m); z: cover; w: the tops'
    /// extra height (m, storms and `sky.altitude`).
    wind: Vec4,
    /// x: density, y: erosion, z: 1 with fog, w: 1 physical sky.
    look: Vec4,
    /// rgb: the horizon's colour; w: -.
    horizon: Vec4,
    /// xy: the output's size; z: 1 if last frame's is usable; w: -.
    size: Vec4,
    /// The sky's grade (rgb).
    grade: Vec4,
    /// xyz: toward a lightning bolt; w: its flash now.
    flash: Vec4,
    /// x detail, y edge sharpness, z wispiness, w self-shadowing.
    shape: Vec4,
    /// x: the sky's light on them, y: debug view (1 key only, 2 ambient),
    /// z: seconds (the shapes' slow change).
    tune: Vec4,
    /// x: high cirrus amount, y: mid altocumulus amount (`clouds.cirrus`,
    /// `clouds.alto`), z: how fast the shapes change (`clouds.evolve`).
    layers: Vec4,
}

#[derive(Resource, Clone, ExtractResource)]
struct ScreenClouds {
    output: Handle<Image>,
    history: Handle<Image>,
    noise: NoiseImages,
    background: Handle<Image>,
    params: CloudParams,
    /// The reflections' cube to draw a face of this frame (the plugin's own
    /// sky), and its parameters.
    env: Option<(Handle<Image>, CloudParams)>,
}

/// The composite: the clouds over the sky's pixels.
#[derive(Asset, AsBindGroup, TypePath, Debug, Clone)]
pub struct CloudLayer {
    #[texture(0)]
    #[sampler(1)]
    clouds: Handle<Image>,
    /// x: the skybox's brightness.
    #[uniform(2)]
    look: Vec4,
}

impl Material for CloudLayer {
    fn vertex_shader() -> ShaderRef {
        "embedded://bevy_atmos/cloud_layer.wgsl".into()
    }

    fn fragment_shader() -> ShaderRef {
        "embedded://bevy_atmos/cloud_layer.wgsl".into()
    }

    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Premultiplied
    }

    fn enable_prepass() -> bool {
        false
    }

    fn enable_shadows() -> bool {
        false
    }

    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        descriptor.vertex.buffers = vec![layout.0.get_layout(&[Mesh::ATTRIBUTE_POSITION.at_shader_location(0)])?];
        descriptor.primitive.cull_mode = None;
        Ok(())
    }
}

#[derive(Component)]
struct LayerQuad;

fn target(size: UVec2) -> Image {
    let mut image = Image::new_fill(Extent3d { width: size.x, height: size.y, depth_or_array_layers: 1 }, TextureDimension::D2, &[0; 8], TextureFormat::Rgba16Float, RenderAssetUsages::RENDER_WORLD);
    image.texture_descriptor.usage = TextureUsages::TEXTURE_BINDING | TextureUsages::STORAGE_BINDING | TextureUsages::COPY_DST | TextureUsages::COPY_SRC;
    image.sampler = bevy::image::ImageSampler::linear();
    image
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn update(
    mut commands: Commands,
    sky: Option<Res<crate::sky::DynamicSky>>,
    host: Option<Res<HostSky>>,
    noise: Option<Res<NoiseImages>>,
    clouds: Option<ResMut<ScreenClouds>>,
    cameras: Query<(&Camera, &GlobalTransform, Option<&Skybox>), With<crate::AtmosCamera>>,
    mut quads: Query<(&mut Transform, &mut Visibility, &MeshMaterial3d<CloudLayer>), With<LayerQuad>>,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<CloudLayer>>,
    mut prev: Local<Option<Mat4>>,
    mut frame: Local<u32>,
    mut last_size: Local<UVec2>,
    time: Res<Time>,
) {
    let k = crate::knobs::get;
    let camera = cameras.iter().next();
    // The sky's inputs: a host's own (`HostSky`), else the plugin's sky.
    let inputs = match (&host, &sky) {
        (Some(h), _) => h.enabled.then(|| inputs_of_host(h)),
        (None, Some(s)) => (s.on() && quality() > 0).then(|| inputs_of_sky(s)),
        _ => None,
    };
    let on = inputs.is_some() && noise.is_some() && camera.is_some();
    let plugin_env = sky.as_ref().map(|s| s.env());
    let Some(((camera, at, skybox), (sky, noise))) = camera.zip(inputs.zip(noise)).filter(|_| on) else {
        for (_, mut vis, _) in &mut quads {
            vis.set_if_neq(Visibility::Hidden);
        }
        if clouds.is_some() {
            commands.remove_resource::<ScreenClouds>();
        }
        *prev = None;
        return;
    };
    let view = camera.physical_viewport_size().unwrap_or(UVec2::new(1280, 720)).max(UVec2::ONE);
    // (Half the view's resolution, at most ~960 pixels across: the cost
    // follows the pixel count.)
    let (steps, share, most) = quality_settings(quality().max(1));
    let scale = k("clouds.resolution", (most / view.x as f32).min(share)).clamp(0.125, 1.0);
    let size = (view.as_vec2() * scale).ceil().as_uvec2().max(UVec2::ONE);
    let (output, history) = match &clouds {
        Some(c) => (c.output.clone(), c.history.clone()),
        None => (images.add(target(size)), images.add(target(size))),
    };
    let resized = *last_size != size;
    if resized {
        for h in [&output, &history] {
            if let Some(mut image) = images.get_mut(h) {
                if image.texture_descriptor.size.width != size.x || image.texture_descriptor.size.height != size.y {
                    image.resize(Extent3d { width: size.x, height: size.y, depth_or_array_layers: 1 });
                }
            }
        }
        *last_size = size;
    }
    let view_from_world = at.to_matrix().inverse();
    let clip_from_world = camera.clip_from_view() * view_from_world;
    let p = &sky;
    let params = CloudParams {
        world_from_clip: clip_from_world.inverse(),
        prev_clip_from_world: prev.unwrap_or(clip_from_world),
        camera: at.translation().extend(*frame as f32),
        key_dir: p.key_dir.extend(p.night),
        key_color: p.key_color.extend(p.darkness),
        wind: Vec4::new(p.drift.x, p.drift.y, p.cover, p.tops),
        look: Vec4::new(k("clouds.density", 1.0).clamp(0.0, 4.0), k("clouds.erosion", 1.0).clamp(0.0, 2.0), p.fog as u32 as f32, steps),
        horizon: p.horizon.extend(p.storm),
        size: Vec4::new(size.x as f32, size.y as f32, if prev.is_some() && !resized { 1.0 } else { 0.0 }, 0.0),
        tune: Vec4::new(k("clouds.ambient", 0.45).clamp(0.0, 4.0), k("clouds.debug", 0.0), time.elapsed_secs_wrapped(), 0.0),
        layers: Vec4::new(k("clouds.cirrus", 0.5).clamp(0.0, 1.0), k("clouds.alto", 0.35).clamp(0.0, 1.0), k("clouds.evolve", 1.0).clamp(0.0, 10.0), 0.0),
        grade: p.grade.extend(k("clouds.light", 2.8).clamp(0.0, 4.0)),
        flash: p.bolt_dir.normalize_or(Vec3::Y).extend(p.flash),
        shape: {
            let (detail, sharp) = quality_shape(quality().max(1));
            Vec4::new(detail, sharp, k("clouds.erosion", 1.0).clamp(0.0, 2.0), k("clouds.shadow", 2.0).clamp(0.0, 8.0))
        },
    };
    *prev = Some(clip_from_world);
    *frame = frame.wrapping_add(1);
    // The plugin's own sky: its reflections' cube from these same clouds, a
    // face a frame, at fewer steps.
    let env = plugin_env.filter(|_| host.is_none()).map(|e| {
        let face = (*frame % 6) as f32;
        (e, CloudParams { size: Vec4::new(ENV_SIZE as f32, ENV_SIZE as f32, 0.0, face), look: Vec4::new(params.look.x, params.look.y, params.look.z, (steps * 0.5).max(32.0)), ..params })
    });
    commands.insert_resource(ScreenClouds { output: output.clone(), history, noise: noise.clone(), background: sky.background.clone(), params, env });
    // The composite: a quad far off ahead (it sorts behind the water and
    // the rain), drawn at the far plane by its shader.
    let ahead = at.translation() + at.forward().as_vec3() * 50_000.0;
    let brightness = skybox.map_or(1000.0, |s| s.brightness);
    match quads.single_mut() {
        Ok((mut t, mut vis, handle)) => {
            t.translation = ahead;
            vis.set_if_neq(Visibility::Inherited);
            if let Some(m) = materials.get(&handle.0) {
                // (Resized textures: the material's bind group must be made
                // anew, or it keeps showing the old one, frozen.)
                if m.clouds != output || m.look.x != brightness || resized {
                    if let Some(mut m) = materials.get_mut(&handle.0) {
                        m.clouds = output;
                        m.look.x = brightness;
                    }
                }
            }
        }
        Err(_) => {
            commands.spawn((
                Name::new("atmos clouds"),
                LayerQuad,
                Mesh3d(meshes.add(Rectangle::new(2.0, 2.0))),
                MeshMaterial3d(materials.add(CloudLayer { clouds: output, look: Vec4::new(brightness, 0.0, 0.0, 0.0) })),
                Transform::from_translation(ahead),
                bevy::camera::visibility::NoFrustumCulling,
                bevy::light::NotShadowCaster,
            ));
        }
    }
}

fn inputs_of_host(h: &HostSky) -> HostSky {
    h.clone()
}

/// The plugin's own sky as the clouds' inputs.
fn inputs_of_sky(s: &crate::sky::DynamicSky) -> HostSky {
    let p = s.params();
    HostSky {
        enabled: true,
        background: s.background(),
        reflections: None,
        key_dir: p.key_dir,
        key_color: p.key_color,
        night: p.night,
        cover: p.coverage,
        darkness: p.darkness,
        drift: p.wind,
        tops: p.cloud_extra,
        storm: (p.cloud_extra / 2000.0).clamp(0.0, 1.0),
        flash: p.flash,
        bolt_dir: p.bolt_dir,
        horizon: p.horizon,
        fog: p.fog > 0.5,
        grade: p.grade.truncate(),
    }
}

// --- render world

#[derive(Resource)]
struct CloudPipeline {
    layout: BindGroupLayoutDescriptor,
    pipeline: CachedComputePipelineId,
    cube_layout: BindGroupLayoutDescriptor,
    cube_pipeline: CachedComputePipelineId,
    repeat: Sampler,
    clamp: Sampler,
}

#[derive(Resource)]
struct CloudBindGroup {
    group: BindGroup,
    size: UVec2,
    output: Handle<Image>,
    history: Handle<Image>,
    cube: Option<BindGroup>,
}

fn init_pipeline(mut commands: Commands, asset_server: Res<AssetServer>, pipeline_cache: Res<PipelineCache>, device: Res<RenderDevice>) {
    let layout = BindGroupLayoutDescriptor::new(
        "atmos_clouds",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::COMPUTE,
            (
                uniform_buffer::<CloudParams>(false),
                texture_3d(TextureSampleType::Float { filterable: true }),
                texture_3d(TextureSampleType::Float { filterable: true }),
                texture_2d(TextureSampleType::Float { filterable: true }),
                sampler(SamplerBindingType::Filtering),
                texture_2d(TextureSampleType::Float { filterable: true }),
                sampler(SamplerBindingType::Filtering),
                texture_storage_2d(TextureFormat::Rgba16Float, StorageTextureAccess::WriteOnly),
                texture_cube(TextureSampleType::Float { filterable: true }),
            ),
        ),
    );
    let pipeline = pipeline_cache.queue_compute_pipeline(ComputePipelineDescriptor {
        label: Some("atmos_clouds".into()),
        layout: vec![layout.clone()],
        shader: asset_server.load("embedded://bevy_atmos/clouds.wgsl"),
        entry_point: Some("main".into()),
        ..default()
    });
    // The reflections' cube: the same shader's `cube_main`, the bindings it
    // uses.
    let cube_layout = BindGroupLayoutDescriptor::new(
        "atmos_clouds_cube",
        &BindGroupLayoutEntries::with_indices(
            ShaderStages::COMPUTE,
            (
                (0, uniform_buffer::<CloudParams>(false)),
                (1, texture_3d(TextureSampleType::Float { filterable: true })),
                (2, texture_3d(TextureSampleType::Float { filterable: true })),
                (3, texture_2d(TextureSampleType::Float { filterable: true })),
                (4, sampler(SamplerBindingType::Filtering)),
                (6, sampler(SamplerBindingType::Filtering)),
                (8, texture_cube(TextureSampleType::Float { filterable: true })),
                (9, texture_storage_2d_array(TextureFormat::Rgba16Float, StorageTextureAccess::WriteOnly)),
            ),
        ),
    );
    let cube_pipeline = pipeline_cache.queue_compute_pipeline(ComputePipelineDescriptor {
        label: Some("atmos_clouds_cube".into()),
        layout: vec![cube_layout.clone()],
        shader: asset_server.load("embedded://bevy_atmos/clouds.wgsl"),
        entry_point: Some("cube_main".into()),
        ..default()
    });
    let repeat = device.create_sampler(&SamplerDescriptor {
        label: Some("atmos_clouds_repeat"),
        address_mode_u: AddressMode::Repeat,
        address_mode_v: AddressMode::Repeat,
        address_mode_w: AddressMode::Repeat,
        mag_filter: FilterMode::Linear,
        min_filter: FilterMode::Linear,
        ..default()
    });
    let clamp = device.create_sampler(&SamplerDescriptor { label: Some("atmos_clouds_clamp"), mag_filter: FilterMode::Linear, min_filter: FilterMode::Linear, ..default() });
    commands.insert_resource(CloudPipeline { layout, pipeline, cube_layout, cube_pipeline, repeat, clamp });
}

#[allow(clippy::too_many_arguments)]
fn prepare(
    mut commands: Commands,
    clouds: Option<Res<ScreenClouds>>,
    pipeline: Res<CloudPipeline>,
    images: Res<RenderAssets<GpuImage>>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    pipeline_cache: Res<PipelineCache>,
) {
    commands.remove_resource::<CloudBindGroup>();
    let Some(c) = clouds else { return };
    let get = |h: &Handle<Image>| images.get(h);
    let (Some(out), Some(history), Some(base), Some(detail), Some(curl), Some(background)) =
        (get(&c.output), get(&c.history), get(&c.noise.base), get(&c.noise.detail), get(&c.noise.curl), get(&c.background))
    else {
        return;
    };
    // (Just resized: the textures catch up next frame.)
    let size = UVec2::new(c.params.size.x as u32, c.params.size.y as u32);
    if out.texture.width() != size.x || out.texture.height() != size.y || history.texture.size() != out.texture.size() {
        return;
    }
    let mut uniform = UniformBuffer::from(c.params);
    uniform.write_buffer(&device, &queue);
    let group = device.create_bind_group(
        "atmos_clouds",
        &pipeline_cache.get_bind_group_layout(&pipeline.layout),
        &BindGroupEntries::sequential((
            &uniform,
            &base.texture_view,
            &detail.texture_view,
            &curl.texture_view,
            &pipeline.repeat,
            &history.texture_view,
            &pipeline.clamp,
            &out.texture_view,
            &background.texture_view,
        )),
    );
    let cube = c.env.as_ref().and_then(|(image, params)| {
        let env = images.get(image)?;
        let view = env.texture.create_view(&TextureViewDescriptor { dimension: Some(TextureViewDimension::D2Array), ..default() });
        let mut uniform = UniformBuffer::from(*params);
        uniform.write_buffer(&device, &queue);
        Some(device.create_bind_group(
            "atmos_clouds_cube",
            &pipeline_cache.get_bind_group_layout(&pipeline.cube_layout),
            &BindGroupEntries::with_indices((
                (0, &uniform),
                (1, &base.texture_view),
                (2, &detail.texture_view),
                (3, &curl.texture_view),
                (4, &pipeline.repeat),
                (6, &pipeline.clamp),
                (8, &background.texture_view),
                (9, &view),
            )),
        ))
    });
    commands.insert_resource(CloudBindGroup { group, size, output: c.output.clone(), history: c.history.clone(), cube });
}

fn draw(mut ctx: RenderContext, group: Option<Res<CloudBindGroup>>, pipeline: Res<CloudPipeline>, pipeline_cache: Res<PipelineCache>, images: Res<RenderAssets<GpuImage>>) {
    use bevy::render::diagnostic::RecordDiagnostics;
    let Some(group) = group else { return };
    let Some(compute) = pipeline_cache.get_compute_pipeline(pipeline.pipeline) else { return };
    let (Some(out), Some(history)) = (images.get(&group.output), images.get(&group.history)) else { return };
    let diagnostics = ctx.diagnostic_recorder();
    let diagnostics = diagnostics.as_deref();
    let span = diagnostics.time_span(ctx.command_encoder(), "atmos_clouds");
    {
        let mut pass = ctx.command_encoder().begin_compute_pass(&ComputePassDescriptor { label: Some("atmos_clouds"), timestamp_writes: None });
        pass.set_pipeline(compute);
        pass.set_bind_group(0, &group.group, &[]);
        pass.dispatch_workgroups(group.size.x.div_ceil(8), group.size.y.div_ceil(8), 1);
        // A face of the reflections' cube.
        if let (Some(cube), Some(cube_pipeline)) = (&group.cube, pipeline_cache.get_compute_pipeline(pipeline.cube_pipeline)) {
            pass.set_pipeline(cube_pipeline);
            pass.set_bind_group(0, cube, &[]);
            pass.dispatch_workgroups(ENV_SIZE.div_ceil(8), ENV_SIZE.div_ceil(8), 1);
        }
    }
    // This frame's clouds become the next frame's history.
    ctx.command_encoder().copy_texture_to_texture(out.texture.as_image_copy(), history.texture.as_image_copy(), out.texture.size());
    span.end(ctx.command_encoder());
}
