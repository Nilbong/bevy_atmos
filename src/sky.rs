//! The sky: a cube map drawn by a compute shader (`sky.wgsl`) into the
//! [`crate::AtmosCamera`]'s `Skybox`. Raymarched clouds over a round
//! earth, lit by the sun (or moon), drifting with the wind; a physical sky
//! (Rayleigh and Mie scattering, stars by night) or the host skybox's own
//! colours behind them. A strip of one face is redrawn a frame (all six at
//! the start, or while the look changes), so drifting clouds cost little.
//!
//! With the screen-space clouds on (`sky.screen_clouds`, `crate::clouds`)
//! there are two cubes: the camera's skybox, the sky without clouds (the
//! clouds are drawn over it per pixel), and a small one with them for
//! reflections and water ([`DynamicSky::image`]).
//!
//! Knobs: `sky.enabled` (1), `sky.physical` (1 with no host skybox, else
//! 0), `sky.cover` (0..1), `sky.darkness`, `sky.altitude` (m),
//! `sky.wind_speed` (drift, x the weather's wind; its direction is
//! `weather.wind_dir`), `sky.offset` (m),
//! `sky.tint` (the sky's and the fog's colour), `sky.brightness`.

use bevy::asset::RenderAssetUsages;
use bevy::core_pipeline::schedule::camera_driver;
use bevy::light::Skybox;
use bevy::prelude::*;
use bevy::render::extract_resource::{ExtractResource, ExtractResourcePlugin};
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_resource::binding_types::{sampler, texture_3d, texture_cube, texture_storage_2d_array, uniform_buffer};
use bevy::render::render_resource::*;
use bevy::render::renderer::{RenderContext, RenderDevice, RenderGraph, RenderQueue};
use bevy::render::texture::GpuImage;
use bevy::render::{Render, RenderApp, RenderStartup, RenderSystems};

pub struct SkyPlugin;

impl Plugin for SkyPlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "sky.wgsl");
        app.add_plugins(ExtractResourcePlugin::<DynamicSky>::default())
            .init_resource::<crate::air::Horizon>()
            .add_systems(Update, (create, update, swap).chain().after(crate::LookSet));
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else { return };
        render_app
            .add_systems(RenderStartup, init_pipeline)
            .add_systems(Render, prepare.in_set(RenderSystems::PrepareBindGroups))
            .add_systems(RenderGraph, draw.in_set(bevy::render::renderer::RenderGraphSystems::Render).before(camera_driver));
    }
}

/// Cube face size.
const SIZE: u32 = 1024;
/// A face is redrawn in this many strips, one a frame.
const STRIPS: u32 = 16;
/// The reflections' cube (the sky with the clouds), drawn by
/// `crate::clouds` from the same clouds the camera sees.
const ENV_SIZE: u32 = crate::clouds::ENV_SIZE;

#[derive(Resource, Clone, ExtractResource)]
pub struct DynamicSky {
    image: Handle<Image>,
    /// The reflections' cube, with clouds (screen-space clouds on).
    env: Handle<Image>,
    screen: bool,
    noise: Handle<Image>,
    classic: Handle<Image>,
    /// The host had its own skybox (its colours behind the clouds).
    has_classic: bool,
    params: SkyParams,
    on: bool,
    faces: u32,
    rows: u32,
    frames: u32,
    /// The look last drawn (redrawn whole when it changes).
    last: [f32; 6],
}

impl DynamicSky {
    /// The sky with its clouds, for reflections and ambient (the sky alone
    /// with the clouds off).
    pub fn image(&self) -> Handle<Image> {
        if self.screen { self.env.clone() } else { self.image.clone() }
    }

    /// The reflections' cube `crate::clouds` draws into.
    pub(crate) fn env(&self) -> Handle<Image> {
        self.env.clone()
    }

    /// What the camera's skybox shows (without clouds when they're drawn
    /// per pixel).
    pub fn background(&self) -> Handle<Image> {
        self.image.clone()
    }

    /// The sky's look now: sun and key light, cover, darkness, wind...
    pub(crate) fn params(&self) -> &SkyParams {
        &self.params
    }

    pub fn on(&self) -> bool {
        self.on
    }
}

#[derive(Clone, Copy, Debug, Default, ShaderType)]
pub(crate) struct SkyParams {
    pub sun_dir: Vec3,
    pub sun_size: f32,
    pub sun_color: Vec3,
    pub coverage: f32,
    pub horizon: Vec3,
    pub fog: f32,
    pub wind: Vec2,
    pub darkness: f32,
    pub face_base: u32,
    pub row_base: u32,
    pub moon_dir: Vec3,
    pub night: f32,
    pub bolt_dir: Vec3,
    pub flash: f32,
    pub key_dir: Vec3,
    pub physical: f32,
    pub key_color: Vec3,
    pub cloud_extra: f32,
    pub time: f32,
    pub bolt_seed: f32,
    pub look: Vec4,
    pub grade: Vec4,
    pub clouds: f32,
}

fn cube(size: u32, fill: &[u8], format: TextureFormat, storage: bool) -> Image {
    let mut image = Image::new_fill(Extent3d { width: size, height: size, depth_or_array_layers: 6 }, TextureDimension::D2, fill, format, RenderAssetUsages::RENDER_WORLD);
    image.texture_view_descriptor = Some(TextureViewDescriptor { dimension: Some(TextureViewDimension::Cube), ..default() });
    if storage {
        image.texture_descriptor.usage = TextureUsages::TEXTURE_BINDING | TextureUsages::STORAGE_BINDING | TextureUsages::COPY_DST;
    }
    image
}

fn create(mut commands: Commands, sky: Option<Res<DynamicSky>>, cameras: Query<Option<&Skybox>, With<crate::AtmosCamera>>, mut images: ResMut<Assets<Image>>) {
    if sky.is_some() {
        return;
    }
    let Some(skybox) = cameras.iter().next() else { return };
    let host = skybox.and_then(|s| s.image.clone());
    let has_classic = host.is_some();
    // (No host skybox: a plain grey-blue one stands behind the physical sky.)
    let classic = host.unwrap_or_else(|| {
        let half = |v: f32| half_bits(v);
        let px: Vec<u8> = [half(0.35), half(0.45), half(0.6), half(1.0)].iter().flat_map(|h| h.to_le_bytes()).collect();
        images.add(cube(1, &px, TextureFormat::Rgba16Float, false))
    });
    commands.insert_resource(DynamicSky {
        image: images.add(cube(SIZE, &[0; 8], TextureFormat::Rgba16Float, true)),
        env: images.add(cube(ENV_SIZE, &[0; 8], TextureFormat::Rgba16Float, true)),
        screen: false,
        noise: images.add(noise_image()),
        classic,
        has_classic,
        params: SkyParams::default(),
        on: false,
        faces: 6,
        rows: SIZE,
        frames: 0,
        last: [f32::NAN; 6],
    });
}

/// f32 to IEEE half (for the placeholder skybox texel).
fn half_bits(v: f32) -> u16 {
    let b = v.to_bits();
    let sign = ((b >> 16) & 0x8000) as u16;
    let exp = ((b >> 23) & 0xff) as i32 - 127 + 15;
    let mant = ((b >> 13) & 0x3ff) as u16;
    if exp <= 0 {
        sign
    } else if exp >= 31 {
        sign | 0x7c00
    } else {
        sign | ((exp as u16) << 10) | mant
    }
}

const NOISE_SIZE: u32 = 128;
const NOISE_CELLS: u32 = 32;

/// Smooth, tiling 3D value noise for the clouds.
fn noise_image() -> Image {
    let hash = |x: u32, y: u32, z: u32| {
        let (x, y, z) = (x % NOISE_CELLS, y % NOISE_CELLS, z % NOISE_CELLS);
        let mut h = x.wrapping_mul(0x8da6_b343) ^ y.wrapping_mul(0xd816_3841) ^ z.wrapping_mul(0xcb1a_b31f);
        h ^= h >> 13;
        h = h.wrapping_mul(0x5bd1_e995);
        h ^= h >> 15;
        (h & 0xffff) as f32 / 65535.0
    };
    let per = (NOISE_SIZE / NOISE_CELLS) as f32;
    let smooth = |t: f32| t * t * (3.0 - 2.0 * t);
    let mut data = Vec::with_capacity((NOISE_SIZE * NOISE_SIZE * NOISE_SIZE) as usize);
    for z in 0..NOISE_SIZE {
        for y in 0..NOISE_SIZE {
            for x in 0..NOISE_SIZE {
                let p = Vec3::new(x as f32, y as f32, z as f32) / per;
                let (i, f) = (p.floor(), p - p.floor());
                let (i, u) = ((i.x as u32, i.y as u32, i.z as u32), Vec3::new(smooth(f.x), smooth(f.y), smooth(f.z)));
                let corner = |dx, dy, dz| hash(i.0 + dx, i.1 + dy, i.2 + dz);
                let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
                let a = lerp(corner(0, 0, 0), corner(1, 0, 0), u.x);
                let b = lerp(corner(0, 1, 0), corner(1, 1, 0), u.x);
                let c = lerp(corner(0, 0, 1), corner(1, 0, 1), u.x);
                let d = lerp(corner(0, 1, 1), corner(1, 1, 1), u.x);
                data.push((lerp(lerp(a, b, u.y), lerp(c, d, u.y), u.z) * 255.0).round() as u8);
            }
        }
    }
    let mut image = Image::new(
        Extent3d { width: NOISE_SIZE, height: NOISE_SIZE, depth_or_array_layers: NOISE_SIZE },
        TextureDimension::D3,
        data,
        TextureFormat::R8Unorm,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = bevy::image::ImageSampler::Descriptor(bevy::image::ImageSamplerDescriptor {
        address_mode_u: bevy::image::ImageAddressMode::Repeat,
        address_mode_v: bevy::image::ImageAddressMode::Repeat,
        address_mode_w: bevy::image::ImageAddressMode::Repeat,
        mag_filter: bevy::image::ImageFilterMode::Linear,
        min_filter: bevy::image::ImageFilterMode::Linear,
        ..default()
    });
    image
}

/// The sky's colour toward the horizon (its units), for the fog.
fn horizon_of(tod: &crate::time::TimeOfDay) -> Vec3 {
    let up = tod.sun_dir.y;
    let dusk = (1.0 - (up.abs() / 0.25).min(1.0)) * (1.0 - tod.night);
    let day = Vec3::new(0.62, 0.7, 0.8).lerp(Vec3::new(0.95, 0.62, 0.42), dusk * 0.7);
    day.lerp(Vec3::new(0.03, 0.04, 0.06), tod.night) * 0.45
}

#[allow(clippy::too_many_arguments)]
fn update(
    mut sky: Option<ResMut<DynamicSky>>,
    time: Res<Time>,
    tod: Res<crate::time::TimeOfDay>,
    weather: Res<crate::weather::Weather>,
    mut horizon_res: ResMut<crate::air::Horizon>,
    mut cycle: Local<f32>,
) {
    let Some(sky) = sky.as_mut() else { return };
    let knob = crate::knobs::get;
    let on = knob("sky.enabled", 1.0) > 0.5;
    if on != sky.on {
        sky.frames = 0;
    }
    sky.on = on;
    let horizon = horizon_of(&tod);
    horizon_res.0 = horizon;
    if !on {
        return;
    }
    let storm = (weather.rain / 0.8).clamp(0.0, 1.0);
    let coverage = knob("sky.cover", 0.4 + 0.5 * storm).clamp(0.0, 1.0);
    let darkness = knob("sky.darkness", 0.3 + 0.5 * storm).clamp(0.0, 1.0);
    let physical = knob("sky.physical", if sky.has_classic { 0.0 } else { 1.0 }) > 0.5;
    let night_key = tod.clocked && tod.night > 0.5;
    // (The clouds, kilometres up, still see a sun a few degrees under the
    // horizon: lit red at dusk while the ground is in shadow.)
    let aloft = { let t = ((tod.sun_dir.y + 0.1) / 0.14).clamp(0.0, 1.0); t * t * (3.0 - 2.0 * t) } * 0.9;
    let (key_dir, key_color) = if night_key { (tod.moon_dir, Vec3::new(0.62, 0.72, 1.0) * 0.2) } else { (tod.sun_dir, tod.sun_color * tod.sun_share.min(1.3).max(if tod.clocked { aloft } else { 0.0 })) };
    // Clouds drift at ~5 m/s (faster in the wind); every strip of a redraw
    // cycle takes the same moment, so strips never tear against each other.
    let speed = 5.0 + weather.wind.length();
    let slice = sky.frames % (6 * STRIPS);
    if slice == 0 {
        *cycle = time.elapsed_secs() * speed;
    }
    let screen = crate::clouds::quality() > 0;
    let look = [coverage, darkness, knob("sky.altitude", 0.0), knob("sky.tint", 0.0), knob("sky.brightness", 1.0), physical as u32 as f32 + 2.0 * screen as u32 as f32];
    let changed = look != sky.last;
    sky.last = look;
    sky.screen = screen;
    let (faces, rows, face_base, row_base) = if sky.frames == 0 || changed {
        (6, SIZE, 0, 0)
    } else {
        (1, SIZE / STRIPS, slice / STRIPS, slice % STRIPS * SIZE / STRIPS)
    };
    sky.faces = faces;
    sky.rows = rows;
    // Clouds drift with the weather's wind (the noise moves against its
    // offset, hence the half turn); `sky.wind_dir` still overrides.
    let dir = knob("sky.wind_dir", knob("weather.wind_dir", 30.0) + 180.0).to_radians();
    let along = *cycle * knob("sky.wind_speed", 1.0).clamp(0.0, 20.0) + knob("sky.offset", 0.0);
    sky.params = SkyParams {
        sun_dir: tod.sun_dir,
        sun_size: 0.0093,
        sun_color: tod.sun_color,
        coverage,
        horizon,
        fog: (crate::knobs::get("fog.distance", 0.0) > 0.0) as u32 as f32,
        wind: Vec2::new(dir.cos(), dir.sin()) * along,
        darkness,
        face_base,
        row_base,
        moon_dir: tod.moon_dir,
        night: tod.night,
        bolt_dir: Vec3::Y,
        flash: 0.0,
        key_dir,
        physical: physical as u32 as f32,
        key_color,
        cloud_extra: (2000.0 * storm + look[2]).clamp(-1500.0, 6000.0),
        time: time.elapsed_secs(),
        bolt_seed: 0.0,
        look: Vec4::new(1.0, 1.0, 1.0, 0.0),
        grade: {
            let warm = look[3].clamp(-1.0, 1.0);
            (Vec3::new(1.0 + 0.25 * warm, 1.0, 1.0 - 0.25 * warm) * look[4].clamp(0.0, 4.0)).extend(1.0)
        },
        // (The clouds are `crate::clouds`', never this cube's.)
        clouds: 0.0,
    };
    sky.frames += 1;
}

/// The camera's skybox shows ours (and the host's again when it's off).
fn swap(mut commands: Commands, sky: Option<Res<DynamicSky>>, mut cameras: Query<(Entity, Option<&mut Skybox>), With<crate::AtmosCamera>>) {
    let Some(sky) = sky else { return };
    let want = if sky.on { sky.background() } else { sky.classic.clone() };
    for (e, skybox) in &mut cameras {
        match skybox {
            Some(mut s) => {
                if s.image.as_ref() != Some(&want) && (!sky.has_classic || s.image.as_ref() == Some(&sky.classic) || s.image.as_ref() == Some(&sky.image)) {
                    s.image = Some(want.clone());
                }
            }
            None => {
                commands.entity(e).insert(Skybox { image: Some(want.clone()), brightness: 1000.0, ..default() });
            }
        }
    }
}

// --- render world

#[derive(Resource)]
struct SkyPipeline {
    layout: BindGroupLayoutDescriptor,
    pipeline: CachedComputePipelineId,
    sampler: Sampler,
}

#[derive(Resource)]
struct SkyBindGroup(Vec<(BindGroup, u32, u32, u32)>);

fn init_pipeline(mut commands: Commands, asset_server: Res<AssetServer>, pipeline_cache: Res<PipelineCache>, device: Res<RenderDevice>) {
    let layout = BindGroupLayoutDescriptor::new(
        "atmos_sky",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::COMPUTE,
            (
                uniform_buffer::<SkyParams>(false),
                texture_cube(TextureSampleType::Float { filterable: true }),
                sampler(SamplerBindingType::Filtering),
                texture_storage_2d_array(TextureFormat::Rgba16Float, StorageTextureAccess::WriteOnly),
                texture_3d(TextureSampleType::Float { filterable: true }),
                sampler(SamplerBindingType::Filtering),
            ),
        ),
    );
    let pipeline = pipeline_cache.queue_compute_pipeline(ComputePipelineDescriptor {
        label: Some("atmos_sky".into()),
        layout: vec![layout.clone()],
        shader: asset_server.load("embedded://bevy_atmos/sky.wgsl"),
        ..default()
    });
    let sampler = device.create_sampler(&SamplerDescriptor {
        label: Some("atmos_sky_classic"),
        mag_filter: FilterMode::Linear,
        min_filter: FilterMode::Linear,
        mipmap_filter: MipmapFilterMode::Linear,
        ..default()
    });
    commands.insert_resource(SkyPipeline { layout, pipeline, sampler });
}

#[allow(clippy::too_many_arguments)]
fn prepare(
    mut commands: Commands,
    sky: Option<Res<DynamicSky>>,
    pipeline: Res<SkyPipeline>,
    images: Res<RenderAssets<GpuImage>>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    pipeline_cache: Res<PipelineCache>,
    mut views: Local<Vec<(AssetId<Image>, TextureView)>>,
) {
    commands.remove_resource::<SkyBindGroup>();
    let Some(sky) = sky.filter(|s| s.on) else { return };
    let (Some(classic), Some(noise)) = (images.get(&sky.classic), images.get(&sky.noise)) else { return };
    // (The reflections' cube is `crate::clouds`', drawn from the clouds.)
    let targets = vec![(&sky.image, sky.params, sky.faces, sky.rows)];
    let mut groups = Vec::new();
    for (image, params, faces, rows) in targets {
        let Some(out) = images.get(image) else { continue };
        if !views.iter().any(|(id, _)| *id == image.id()) {
            let view = out.texture.create_view(&TextureViewDescriptor { dimension: Some(TextureViewDimension::D2Array), ..default() });
            views.push((image.id(), view));
        }
        let Some((_, out_view)) = views.iter().find(|(id, _)| *id == image.id()) else { continue };
        let mut uniform = UniformBuffer::from(params);
        uniform.write_buffer(&device, &queue);
        let group = device.create_bind_group(
            "atmos_sky",
            &pipeline_cache.get_bind_group_layout(&pipeline.layout),
            &BindGroupEntries::sequential((&uniform, &classic.texture_view, &pipeline.sampler, out_view, &noise.texture_view, &noise.sampler)),
        );
        groups.push((group, faces, rows, out.texture.width()));
    }
    commands.insert_resource(SkyBindGroup(groups));
}

fn draw(mut ctx: RenderContext, groups: Option<Res<SkyBindGroup>>, pipeline: Res<SkyPipeline>, pipeline_cache: Res<PipelineCache>) {
    use bevy::render::diagnostic::RecordDiagnostics;
    let Some(groups) = groups else { return };
    let Some(compute) = pipeline_cache.get_compute_pipeline(pipeline.pipeline) else { return };
    let diagnostics = ctx.diagnostic_recorder();
    let diagnostics = diagnostics.as_deref();
    let span = diagnostics.time_span(ctx.command_encoder(), "atmos_sky");
    {
        let mut pass = ctx.command_encoder().begin_compute_pass(&ComputePassDescriptor { label: Some("atmos_sky"), timestamp_writes: None });
        pass.set_pipeline(compute);
        for (group, faces, rows, size) in &groups.0 {
            pass.set_bind_group(0, group, &[]);
            pass.dispatch_workgroups(size.div_ceil(8), rows.div_ceil(8), *faces);
        }
    }
    span.end(ctx.command_encoder());
}
