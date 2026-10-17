//! Water: [`WaterMaterial`], Bevy's `StandardMaterial` with a [`Water`]
//! extension for water surfaces (a plane, a pool). Waves from the world
//! position (so no two pools move in step, and nothing tiles with the
//! mesh's UVs), the sky (the look's own sky cube) reflected by Fresnel, and
//! the standard lighting's sun glint. Rain ripples it.
//!
//! Knobs: `water.tint` (-1 blue .. 1 brown), `water.brightness`,
//! `water.clarity` (0 opaque .. 2 clear), `water.waves` (0 still .. 1.5),
//! `water.reflection`.

use bevy::pbr::{ExtendedMaterial, MaterialExtension};
use bevy::prelude::*;
use bevy::render::render_resource::AsBindGroup;
use bevy::shader::ShaderRef;

pub struct WaterPlugin;

impl Plugin for WaterPlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "water.wgsl");
        app.add_plugins(MaterialPlugin::<WaterMaterial>::default()).add_systems(Update, look.after(crate::LookSet));
    }
}

/// A water surface. Make it with [`water_material`].
pub type WaterMaterial = ExtendedMaterial<StandardMaterial, Water>;

#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
pub struct Water {
    /// x: wave strength, y: reflection, z: rain, w: the skybox's
    /// brightness (set each frame).
    #[uniform(100)]
    pub params: Vec4,
    /// The sky reflected (the look's sky cube; set each frame).
    #[texture(101, dimension = "cube")]
    #[sampler(102)]
    pub sky: Option<Handle<Image>>,
}

/// A water material of colour `colour` (alpha: how opaque).
pub fn water_material(colour: Color) -> WaterMaterial {
    ExtendedMaterial {
        base: StandardMaterial { base_color: colour, perceptual_roughness: 0.06, reflectance: 0.3, alpha_mode: AlphaMode::Blend, ..default() },
        extension: Water { params: Vec4::new(1.0, 1.0, 0.0, 1000.0), sky: None },
    }
}

impl MaterialExtension for Water {
    fn fragment_shader() -> ShaderRef {
        "embedded://bevy_atmos/water.wgsl".into()
    }
}

#[allow(clippy::too_many_arguments)]
fn look(
    mut materials: ResMut<Assets<WaterMaterial>>,
    sky: Option<Res<crate::sky::DynamicSky>>,
    weather: Res<crate::weather::Weather>,
    cameras: Query<&bevy::light::Skybox, With<crate::AtmosCamera>>,
    mut originals: Local<std::collections::HashMap<AssetId<WaterMaterial>, LinearRgba>>,
    mut last: Local<Option<[f32; 8]>>,
) {
    let k = crate::knobs::get;
    let skybox = cameras.iter().next();
    let now = [
        k("water.tint", 0.0).clamp(-1.0, 1.0),
        k("water.brightness", 1.0).clamp(0.0, 3.0),
        k("water.clarity", 1.0).clamp(0.0, 2.0),
        k("water.waves", 1.0).clamp(0.0, 1.5),
        k("water.reflection", 1.0).clamp(0.0, 3.0),
        (weather.rain * 50.0).round() / 50.0,
        skybox.map_or(1000.0, |s| s.brightness),
        materials.len() as f32,
    ];
    let sky_image = sky.as_ref().map(|s| s.image());
    if *last == Some(now) {
        return;
    }
    *last = Some(now);
    let [tint, bright, clarity, waves, reflection, rain, nits, _] = now;
    let ids: Vec<_> = materials.ids().collect();
    for id in ids {
        let Some(mut m) = materials.get_mut(id) else { continue };
        let base = *originals.entry(id).or_insert(m.base.base_color.to_linear());
        let shade = Vec3::new(1.0 + 0.5 * tint, 1.0 + 0.15 * tint, 1.0 - 0.4 * tint) * bright;
        let alpha = (1.0 - (1.0 - base.alpha) * clarity).clamp(0.0, 1.0);
        m.base.base_color = Color::linear_rgba(base.red * shade.x, base.green * shade.y, base.blue * shade.z, alpha);
        m.extension.params = Vec4::new(waves, reflection, rain, nits);
        m.extension.sky = sky_image.clone();
    }
}
