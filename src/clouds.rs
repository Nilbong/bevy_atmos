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
//! A compute pass (`clouds.wgsl`) writes the clouds (radiance and
//! transmittance) before the camera draws; a fullscreen quad at the far
//! plane (`cloud_layer.wgsl`) lays them over the sky's pixels only. The
//! sky's cube then has no clouds of its own (`crate::sky`), and a small
//! cube with them stays for reflections.
//!
//! Knobs: `sky.screen_clouds` (1; 0 the cube's own clouds), `clouds.density`
//! (1), `clouds.erosion` (1, detail eating the edges), `clouds.resolution`
//! (of the view: half, at most ~960 pixels across), plus the sky's `sky.cover`, `sky.darkness`,
//! `sky.altitude`, wind and time of day.

use bevy::asset::RenderAssetUsages;
use bevy::core_pipeline::schedule::camera_driver;
use bevy::light::Skybox;
use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::pbr::{MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::render::extract_resource::{ExtractResource, ExtractResourcePlugin};
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_resource::binding_types::{sampler, texture_2d, texture_3d, texture_cube, texture_storage_2d, uniform_buffer};
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
}

#[derive(Resource, Clone, ExtractResource)]
struct ScreenClouds {
    output: Handle<Image>,
    history: Handle<Image>,
    noise: NoiseImages,
    background: Handle<Image>,
    params: CloudParams,
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
) {
    let k = crate::knobs::get;
    let camera = cameras.iter().next();
    // The sky's inputs: a host's own (`HostSky`), else the plugin's sky.
    let inputs = match (&host, &sky) {
        (Some(h), _) => h.enabled.then(|| inputs_of_host(h)),
        (None, Some(s)) => (s.on() && crate::sky::screen_clouds()).then(|| inputs_of_sky(s)),
        _ => None,
    };
    let on = inputs.is_some() && noise.is_some() && camera.is_some();
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
    let scale = k("clouds.resolution", (960.0 / view.x as f32).min(0.5)).clamp(0.125, 1.0);
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
        look: Vec4::new(k("clouds.density", 1.0).clamp(0.0, 4.0), k("clouds.erosion", 1.0).clamp(0.0, 2.0), p.fog as u32 as f32, 0.0),
        horizon: p.horizon.extend(p.storm),
        size: Vec4::new(size.x as f32, size.y as f32, if prev.is_some() && !resized { 1.0 } else { 0.0 }, 0.0),
        grade: p.grade.extend(1.0),
        flash: p.bolt_dir.normalize_or(Vec3::Y).extend(p.flash),
    };
    *prev = Some(clip_from_world);
    *frame = frame.wrapping_add(1);
    commands.insert_resource(ScreenClouds { output: output.clone(), history, noise: noise.clone(), background: sky.background.clone(), params });
    // The composite: a quad far off ahead (it sorts behind the water and
    // the rain), drawn at the far plane by its shader.
    let ahead = at.translation() + at.forward().as_vec3() * 50_000.0;
    let brightness = skybox.map_or(1000.0, |s| s.brightness);
    match quads.single_mut() {
        Ok((mut t, mut vis, handle)) => {
            t.translation = ahead;
            vis.set_if_neq(Visibility::Inherited);
            if let Some(m) = materials.get(&handle.0) {
                if m.clouds != output || m.look.x != brightness {
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
    repeat: Sampler,
    clamp: Sampler,
}

#[derive(Resource)]
struct CloudBindGroup {
    group: BindGroup,
    size: UVec2,
    output: Handle<Image>,
    history: Handle<Image>,
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
    commands.insert_resource(CloudPipeline { layout, pipeline, repeat, clamp });
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
    commands.insert_resource(CloudBindGroup { group, size, output: c.output.clone(), history: c.history.clone() });
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
    }
    // This frame's clouds become the next frame's history.
    ctx.command_encoder().copy_texture_to_texture(out.texture.as_image_copy(), history.texture.as_image_copy(), out.texture.size());
    span.end(ctx.command_encoder());
}
