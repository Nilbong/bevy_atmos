//! Weather: rain and snow round the [`crate::AtmosCamera`], wind, and how
//! wet surfaces are (read by [`crate::wet`] and [`crate::water`]).
//!
//! The drops are one draw per kind of many quads placed by the vertex
//! shader (`rain.wgsl`) from the clock and the camera: streaks falling
//! through a box round the camera (wrapping in it, so they stay put in the
//! world), a far layer, splashes, drips off ledges (with a [`Shelter`]),
//! distant curtains, and snowflakes. Hidden while they draw nothing.
//!
//! Knobs: `weather.rain` (0..1), `weather.snow` (0..1), `weather.wind`
//! (m/s), `weather.wind_dir` (degrees), `weather.wetness` (0..1; unset:
//! follows the rain, wetting in a minute and drying in three),
//! `rain.opacity`, `rain.near`, `rain.far`.

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, MeshVertexBufferLayoutRef, PrimitiveTopology};
use bevy::pbr::{MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, Extent3d, RenderPipelineDescriptor, SpecializedMeshPipelineError, TextureDimension, TextureFormat};
use bevy::shader::ShaderRef;

pub struct WeatherPlugin;

impl Plugin for WeatherPlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "rain.wgsl");
        app.init_resource::<Weather>()
            .add_plugins(MaterialPlugin::<RainMaterial>::default())
            .add_systems(Update, weather.in_set(crate::LookSet))
            .add_systems(Update, (spawn, drive).chain().after(crate::LookSet));
    }
}

/// The weather now.
#[derive(Resource, Clone, Debug, Default)]
pub struct Weather {
    pub rain: f32,
    pub snow: f32,
    /// Wind over the ground (m/s, Bevy x and z).
    pub wind: Vec2,
    /// How wet surfaces are, 0..1.
    pub wetness: f32,
}

/// Where rain can't fall: a square grid of heights round the world origin
/// (the highest roof over each cell, metres; below -1e5 = open sky), as an
/// `R32Float` image `cells` a side, each `cell` metres. Without one, rain
/// falls everywhere and nothing drips.
#[derive(Resource, Clone)]
pub struct Shelter {
    pub heights: Handle<Image>,
    pub cell: f32,
    pub cells: u32,
}

fn weather(time: Res<Time>, mut w: ResMut<Weather>) {
    let k = crate::knobs::get;
    w.rain = k("weather.rain", 0.0).clamp(0.0, 1.0);
    w.snow = k("weather.snow", 0.0).clamp(0.0, 1.0);
    let dir = k("weather.wind_dir", 30.0).to_radians();
    let speed = k("weather.wind", 2.0 + 6.0 * w.rain).clamp(0.0, 40.0);
    w.wind = Vec2::new(dir.cos(), dir.sin()) * speed;
    let held = k("weather.wetness", -1.0);
    if held >= 0.0 {
        w.wetness = held.min(1.0);
    } else {
        let rate = if w.rain > w.wetness { 1.0 / 60.0 } else { 1.0 / 180.0 };
        let target = w.rain;
        w.wetness += (target - w.wetness).clamp(-rate * time.delta_secs(), rate * time.delta_secs());
    }
}

const NEAR: u32 = 22000;
const FAR: u32 = 8000;
const SPLASHES: u32 = 4000;
const DRIPS: u32 = 1500;
const CURTAINS: u32 = 72;
const FLAKES: u32 = 72000;
const FAR_FLAKES: u32 = 60000;
const SNOW_FALL: f32 = 1.25;
const SNOW_DRIFT: f32 = 0.8;
const FALL: f32 = 9.0;
/// The drops' light against the horizon's colour (see `drive`).
const RAIN_GAIN: f32 = 200.0;
/// Snowflakes against the rain (bigger and white: less light each).
const SNOW_GAIN: f32 = 0.12;

/// Rain quads: streaks (`kind.x` 0 near, 1 far), splashes (2), drips (3),
/// curtains (4), snowflakes (5 near, 6 far).
#[derive(Asset, AsBindGroup, TypePath, Debug, Clone)]
pub struct RainMaterial {
    #[uniform(0)]
    centre: Vec4,
    #[uniform(1)]
    extent: Vec4,
    #[uniform(2)]
    velocity: Vec4,
    #[uniform(3)]
    light: Vec4,
    #[uniform(4)]
    kind: Vec4,
    #[texture(5, sample_type = "float", filterable = false)]
    heights: Handle<Image>,
    #[uniform(6)]
    key_dir: Vec4,
    #[uniform(7)]
    key_light: Vec4,
    #[uniform(8)]
    lamps: [Vec4; 16],
}

impl Material for RainMaterial {
    fn vertex_shader() -> ShaderRef {
        "embedded://bevy_atmos/rain.wgsl".into()
    }

    fn fragment_shader() -> ShaderRef {
        "embedded://bevy_atmos/rain.wgsl".into()
    }

    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Add
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

fn quads(count: u32) -> Mesh {
    let mut positions = Vec::with_capacity(count as usize * 4);
    let mut indices = Vec::with_capacity(count as usize * 6);
    for i in 0..count {
        for (x, y) in [(-1.0, 0.0), (1.0, 0.0), (1.0, 1.0), (-1.0, 1.0)] {
            positions.push([x, y, i as f32]);
        }
        let b = i * 4;
        indices.extend([b, b + 1, b + 2, b, b + 2, b + 3]);
    }
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD);
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_indices(Indices::U32(indices));
    mesh
}

#[derive(Component)]
struct RainDraw(u32);

fn spawn(
    mut commands: Commands,
    existing: Query<(), With<RainDraw>>,
    shelter: Option<Res<Shelter>>,
    weather: Res<Weather>,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<RainMaterial>>,
) {
    // (Made the first time it rains or snows.)
    if !existing.is_empty() || (weather.rain <= 0.0 && weather.snow <= 0.0) {
        return;
    }
    let (heights, cell, cells) = match shelter {
        Some(s) => (s.heights.clone(), s.cell, s.cells),
        None => {
            let open = Image::new_fill(Extent3d { width: 1, height: 1, depth_or_array_layers: 1 }, TextureDimension::D2, &(-1.0e6f32).to_le_bytes(), TextureFormat::R32Float, RenderAssetUsages::RENDER_WORLD);
            (images.add(open), 1000.0, 1)
        }
    };
    for (kind, count) in [(0, NEAR), (1, FAR), (2, SPLASHES), (3, DRIPS), (4, CURTAINS), (5, FLAKES), (6, FAR_FLAKES)] {
        let material = materials.add(RainMaterial {
            centre: Vec4::ZERO,
            extent: Vec4::ZERO,
            velocity: Vec4::ZERO,
            light: Vec4::ZERO,
            kind: Vec4::new(kind as f32, cell, cells as f32, 0.0),
            heights: heights.clone(),
            key_dir: Vec4::Y,
            key_light: Vec4::ZERO,
            lamps: [Vec4::ZERO; 16],
        });
        let mesh = meshes.add(quads(count));
        commands.spawn((
            Name::new("atmos rain"),
            RainDraw(kind),
            Mesh3d(mesh),
            MeshMaterial3d(material),
            Transform::default(),
            bevy::camera::visibility::NoFrustumCulling,
            bevy::light::NotShadowCaster,
        ));
    }
}

#[allow(clippy::type_complexity)]
fn drive(
    time: Res<Time>,
    weather: Res<Weather>,
    tod: Res<crate::time::TimeOfDay>,
    horizon: Res<crate::air::Horizon>,
    shelter: Option<Res<Shelter>>,
    camera: Query<&GlobalTransform, With<crate::AtmosCamera>>,
    mut draws: Query<(&RainDraw, &MeshMaterial3d<RainMaterial>, &mut Visibility)>,
    mut materials: ResMut<Assets<RainMaterial>>,
) {
    let Some(eye) = camera.iter().next().map(|c| c.translation()) else { return };
    let t = time.elapsed_secs_wrapped();
    let k = crate::knobs::get;
    // (The drop shader's strengths are for light the size of a lit
    // scene's colour on screen; the horizon colour here is a few tenths,
    // which left each streak adding ~0.1% to the image: no rain to see.
    // `RAIN_GAIN` brings a streak to a few percent of what's behind it.)
    let light = horizon.0 * 2.6 * RAIN_GAIN * k("rain.brightness", 1.0).clamp(0.0, 10.0);
    let key_light = tod.sun_color * tod.sun_share.min(1.3) * 0.6 * RAIN_GAIN * (1.0 - tod.night);
    let velocity = Vec3::new(weather.wind.x, -FALL, weather.wind.y);
    let rain = weather.rain * 1.6;
    for (draw, handle, mut vis) in &mut draws {
        let Some(mut m) = materials.get_mut(&handle.0) else { continue };
        // (A shelter that came after the rain did: the host's, or the
        // modern water's.)
        if let Some(s) = &shelter {
            if m.heights != s.heights {
                m.heights = s.heights.clone();
                m.kind.y = s.cell;
                m.kind.z = s.cells as f32;
            }
        }
        let (extent, share) = match draw.0 {
            0 => (Vec3::new(14.0, 9.0, 14.0), (rain / 1.6).min(1.0) * k("rain.near", 1.0).clamp(0.0, 1.0)),
            1 => (Vec3::new(38.0, 16.0, 38.0), (rain / 1.6).powi(2).min(1.0) * k("rain.far", 0.7).clamp(0.0, 1.0)),
            2 => (Vec3::new(11.0, 0.0, 11.0), (rain / 1.6).min(1.0)),
            3 => (Vec3::new(9.0, 0.0, 9.0), if shelter.is_some() { (rain / 1.6).sqrt().min(1.0) } else { 0.0 }),
            4 => (Vec3::new(30.0, 150.0, 0.0), weather.rain * 0.4),
            5 => (Vec3::new(12.0, 8.0, 12.0), weather.snow),
            _ => (Vec3::new(70.0, 25.0, 70.0), weather.snow),
        };
        m.centre = eye.extend(t);
        m.extent = extent.extend(share);
        m.velocity = if draw.0 >= 5 { Vec4::new(weather.wind.x * SNOW_DRIFT, -SNOW_FALL, weather.wind.y * SNOW_DRIFT, 0.0) } else { velocity.extend(0.03) };
        m.light = (light * if draw.0 >= 5 { SNOW_GAIN } else { 1.0 }).extend(0.0);
        m.key_dir = tod.sun_dir.extend(k("rain.opacity", 0.35).max(0.0));
        m.key_light = (key_light * if draw.0 >= 5 { SNOW_GAIN } else { 1.0 }).extend(0.0);
        if draw.0 == 4 {
            m.kind.w = 0.3 + 0.7 * tod.night;
        }
        let on = share > 0.0 && (draw.0 != 4 || m.kind.w > 0.0);
        vis.set_if_neq(if on { Visibility::Inherited } else { Visibility::Hidden });
    }
}
