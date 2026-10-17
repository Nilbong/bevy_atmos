//! Flowing water (rivers) for [`crate::modern_water`]: a [`WaterFlow`] on
//! a [`crate::modern_water::AtmosWater`] entity makes it a river. Its
//! modern water keeps the mesh's own shape (sloped water follows it) and
//! moves with the flow: flow-mapped ripples and foam (two phases blended,
//! so nothing stretches), slower in the shallows at the banks, white water
//! where it runs fast or steep, foam trailing behind whatever crosses it
//! and calm water just behind.
//!
//! Where the flow comes from:
//! - [`WaterFlow::uv_scroll`]: the original material's texture scroll
//!   (UV units a second); the water flows the way the texture appeared to
//!   move, at the speed it moved over the surface (metres from the mesh's
//!   UV scale, worked out per pixel).
//! - [`WaterFlow::along_uv`]: along the mesh's UV axis (rivers are usually
//!   mapped along their length), at a speed in m/s.
//! - [`WaterFlow::world`]: one direction and speed (m/s, world x and z).
//! - [`WaterFlow::map`]: a flow map over the mesh's UVs (RG: the UV-space
//!   direction, 0.5 still), times a speed.
//! - Painted in the editor (Rivers tab, "Paint flow"): saved per level as
//!   `<save dir>/<level>.flow`, over whatever the mesh says.
//!
//! Knobs: `water.flow` (1 on), `water.flow_speed` (1, times every river),
//! `water.flow_dir` (-1; 0..360 turns every river that way), `water.rapids`
//! (2.5 m/s: white water from there), `water.trail` (4 m of foam behind
//! things), `water.brush_radius` (3 m), `water.brush_speed` (1.5 m/s).

use bevy::picking::mesh_picking::ray_cast::{MeshRayCast, MeshRayCastSettings, RayCastVisibility};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::window::PrimaryWindow;
use std::sync::atomic::{AtomicBool, Ordering};

pub struct FlowPlugin;

impl Plugin for FlowPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, make_paint).add_systems(Update, (load_on_level_change, paint, upload).chain().after(crate::LookSet));
    }
}

/// How a river's water flows (on its `AtmosWater` entity).
#[derive(Component, Clone, Debug)]
pub struct WaterFlow {
    pub source: FlowSource,
}

#[derive(Clone, Debug)]
pub enum FlowSource {
    /// UV units a second along the mesh's UVs (the way the water moves).
    Uv(Vec2),
    /// Along this UV-space direction at this speed (m/s).
    AlongUv(Vec2, f32),
    /// World x, z (m/s).
    World(Vec2),
    /// A flow map over the mesh's UVs (RG: UV-space direction, 0.5 still)
    /// at this speed (m/s).
    Map(Handle<Image>, f32),
}

impl WaterFlow {
    /// From the original material's UV scroll (UV units a second): the
    /// texture appeared to move the other way, and so does the water.
    pub fn uv_scroll(scroll: Vec2) -> Self {
        WaterFlow { source: FlowSource::Uv(-scroll) }
    }

    /// Along the mesh's UV axis `axis` (e.g. `Vec2::Y`: increasing V) at
    /// `speed` m/s.
    pub fn along_uv(axis: Vec2, speed: f32) -> Self {
        WaterFlow { source: FlowSource::AlongUv(axis.normalize_or_zero(), speed) }
    }

    /// One world velocity (x, z, m/s).
    pub fn world(velocity: Vec2) -> Self {
        WaterFlow { source: FlowSource::World(velocity) }
    }

    /// A flow map over the mesh's UVs, times `speed` m/s.
    pub fn map(map: Handle<Image>, speed: f32) -> Self {
        WaterFlow { source: FlowSource::Map(map, speed) }
    }

    /// The material's flow uniform: x mode (1 UV velocity, 2 world, 3
    /// map), yz the vector, w speed.
    pub(crate) fn uniform(&self) -> Vec4 {
        match &self.source {
            FlowSource::Uv(v) => Vec4::new(1.0, v.x, v.y, 1.0),
            FlowSource::AlongUv(v, s) => Vec4::new(4.0, v.x, v.y, *s),
            FlowSource::World(v) => Vec4::new(2.0, v.x, v.y, 1.0),
            FlowSource::Map(_, s) => Vec4::new(3.0, 0.0, 0.0, *s),
        }
    }

    pub(crate) fn map_image(&self) -> Option<Handle<Image>> {
        match &self.source {
            FlowSource::Map(h, _) => Some(h.clone()),
            _ => None,
        }
    }
}

/// The knobs as the material's uniform: x on (speed multiplier, 0 off),
/// y the rapids' speed (m/s), z the trail (m), w the direction override
/// (radians, below 0 none).
pub(crate) fn look() -> Vec4 {
    let k = crate::knobs::get;
    let on = k("water.flow", 1.0) >= 0.5;
    let dir = k("water.flow_dir", -1.0);
    Vec4::new(if on { k("water.flow_speed", 1.0).clamp(0.0, 5.0) } else { 0.0 }, k("water.rapids", 2.5).clamp(0.1, 20.0), k("water.trail", 4.0).clamp(0.0, 20.0), if dir >= 0.0 { dir.to_radians() } else { -1.0 })
}

/// Painting with the flow brush (the editor's "Paint flow").
static PAINTING: AtomicBool = AtomicBool::new(false);

pub fn painting() -> bool {
    PAINTING.load(Ordering::Relaxed)
}

pub fn set_painting(on: bool) {
    PAINTING.store(on, Ordering::Relaxed);
}

/// Painted flow: a grid of world cells (`CELL` m, wrapping every `CELLS`)
/// each a velocity (m/s, x and z) and how much it counts (0 unpainted).
#[derive(Resource)]
pub struct FlowPaint {
    pub image: Handle<Image>,
    cells: std::collections::BTreeMap<(i32, i32), Vec3>,
    dirty: bool,
}

pub const CELL: f32 = 1.0;
pub const CELLS: u32 = 256;

fn make_paint(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    let image = Image::new_fill(Extent3d { width: CELLS, height: CELLS, depth_or_array_layers: 1 }, TextureDimension::D2, &[0; 16], TextureFormat::Rgba32Float, bevy::asset::RenderAssetUsages::default());
    commands.insert_resource(FlowPaint { image: images.add(image), cells: Default::default(), dirty: false });
}

impl FlowPaint {
    pub fn clear(&mut self) {
        self.cells.clear();
        self.dirty = true;
    }

    fn stamp(&mut self, at: Vec3, radius: f32, velocity: Option<Vec2>) {
        let r = (radius / CELL).ceil() as i32;
        let c = ((at.x / CELL).floor() as i32, (at.z / CELL).floor() as i32);
        for dz in -r..=r {
            for dx in -r..=r {
                let cell = (c.0 + dx, c.1 + dz);
                let centre = Vec2::new((cell.0 as f32 + 0.5) * CELL, (cell.1 as f32 + 0.5) * CELL);
                let fall = 1.0 - (centre.distance(Vec2::new(at.x, at.z)) / radius).clamp(0.0, 1.0);
                if fall <= 0.0 {
                    continue;
                }
                match velocity {
                    Some(v) => {
                        let e = self.cells.entry(cell).or_insert(Vec3::ZERO);
                        let w = (fall * 0.6).min(1.0);
                        *e = Vec3::new(e.x + (v.x - e.x) * w, e.y + (v.y - e.y) * w, (e.z + w).min(1.0));
                    }
                    None => {
                        if let Some(e) = self.cells.get_mut(&cell) {
                            e.z = (e.z - fall).max(0.0);
                            if e.z <= 0.0 {
                                self.cells.remove(&cell);
                            }
                        }
                    }
                }
            }
        }
        self.dirty = true;
    }

    fn text(&self, level: &str) -> String {
        let mut text = format!("# {level}'s painted water flow (bevy_atmos): cell x, cell z, m/s x, m/s z, weight; cells {CELL} m\n");
        for ((x, z), v) in &self.cells {
            text.push_str(&format!("{x} {z} {} {} {}\n", v.x, v.y, v.z));
        }
        text
    }
}

/// Saves the painted flow as `<dir>/<level>.flow` (removes it when none).
pub fn save(paint: &FlowPaint, settings: &crate::AtmosSettings, level: &str) -> std::io::Result<()> {
    let path = settings.save_dir.join(format!("{level}.flow"));
    if paint.cells.is_empty() {
        if path.exists() {
            std::fs::remove_file(path)?;
        }
        return Ok(());
    }
    std::fs::create_dir_all(&settings.save_dir)?;
    std::fs::write(path, paint.text(level))
}

#[derive(Default)]
struct Loaded(Option<String>);

fn load_on_level_change(level: Res<crate::AtmosLevel>, settings: Res<crate::AtmosSettings>, paint: Option<ResMut<FlowPaint>>, mut last: Local<Loaded>) {
    let Some(mut paint) = paint else { return };
    if last.0.as_deref() == Some(level.0.as_str()) {
        return;
    }
    last.0 = Some(level.0.clone());
    paint.cells.clear();
    if let Ok(text) = std::fs::read_to_string(settings.save_dir.join(format!("{}.flow", level.0))) {
        for line in text.lines().filter(|l| !l.trim_start().starts_with('#')) {
            let v: Vec<f32> = line.split_whitespace().filter_map(|x| x.parse().ok()).collect();
            if v.len() == 5 && v.iter().all(|x| x.is_finite()) {
                paint.cells.insert((v[0] as i32, v[1] as i32), Vec3::new(v[2], v[3], v[4].clamp(0.0, 1.0)));
            }
        }
    }
    paint.dirty = true;
}

/// The brush: with "Paint flow" on and the editor open, drag on the water
/// (left button) to paint that way; right button rubs it out.
#[allow(clippy::too_many_arguments)]
fn paint(
    mut ray_cast: MeshRayCast,
    paint: Option<ResMut<FlowPaint>>,
    buttons: Res<ButtonInput<MouseButton>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    cameras: Query<(&Camera, &GlobalTransform), With<crate::AtmosCamera>>,
    water: Query<(), With<crate::modern_water::ModernCopy>>,
    mut last: Local<Option<Vec3>>,
) {
    let Some(mut paint) = paint else { return };
    if !painting() || !crate::editor::open() {
        *last = None;
        return;
    }
    let (Ok(window), Some((camera, at))) = (windows.single(), cameras.iter().next()) else { return };
    let Some(cursor) = window.cursor_position() else { return };
    // (Not over the editor's panel, down the right side.)
    if cursor.x > window.width() - 440.0 {
        *last = None;
        return;
    }
    let (left, right) = (buttons.pressed(MouseButton::Left), buttons.pressed(MouseButton::Right));
    if !left && !right {
        *last = None;
        return;
    }
    let Ok(ray) = camera.viewport_to_world(at, cursor) else { return };
    let filter = |e: Entity| water.contains(e);
    let settings = MeshRayCastSettings::default().with_filter(&filter).with_visibility(RayCastVisibility::Any);
    let Some((_, hit)) = ray_cast.cast_ray(ray, &settings).first() else { return };
    let point = hit.point;
    let radius = crate::knobs::get("water.brush_radius", 3.0).clamp(0.3, 30.0);
    if right {
        paint.stamp(point, radius, None);
        *last = Some(point);
        return;
    }
    if let Some(prev) = *last {
        let drag = Vec2::new(point.x - prev.x, point.z - prev.z);
        if drag.length() < 0.15 {
            return;
        }
        let speed = crate::knobs::get("water.brush_speed", 1.5).clamp(0.0, 20.0);
        paint.stamp(point, radius, Some(drag.normalize() * speed));
    }
    *last = Some(point);
}

/// The painted cells into the image the water reads.
fn upload(paint: Option<ResMut<FlowPaint>>, mut images: ResMut<Assets<Image>>) {
    let Some(mut paint) = paint else { return };
    if !paint.dirty {
        return;
    }
    paint.dirty = false;
    let Some(mut image) = images.get_mut(&paint.image) else { return };
    let n = CELLS as i32;
    let mut data = vec![0f32; (n * n * 4) as usize];
    for ((x, z), v) in &paint.cells {
        let i = ((z.rem_euclid(n) * n + x.rem_euclid(n)) * 4) as usize;
        data[i..i + 4].copy_from_slice(&[v.x, v.y, v.z, 0.0]);
    }
    image.data = Some(data.iter().flat_map(|f| f.to_le_bytes()).collect());
}

/// Whether painted flow exists (the water samples it).
pub fn has_paint(paint: &FlowPaint) -> bool {
    !paint.cells.is_empty()
}
