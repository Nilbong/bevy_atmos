//! Wet surfaces: [`WetMaterial`], Bevy's `StandardMaterial` with a
//! [`Wet`] extension. With the weather's wetness, surfaces darken (soaked)
//! and turn glossy (a water film), and flat ground gathers puddles: still,
//! mirror-like water in the hollows of a world-space noise. Each material
//! says how much it may get wet (`allow`: 0 a roofed floor, 1 open ground);
//! `puddles` 0 keeps puddles off it (walls, props).
//!
//! ```ignore
//! materials.add(WetMaterial { base: StandardMaterial::default(), extension: Wet::ground() });
//! ```
//!
//! Knobs: `wet.darken`, `wet.gloss`, `wet.puddles` (scales on the
//! effects, 1 each).

use bevy::pbr::{ExtendedMaterial, MaterialExtension};
use bevy::prelude::*;
use bevy::render::render_resource::AsBindGroup;
use bevy::shader::ShaderRef;

pub struct WetPlugin;

impl Plugin for WetPlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "wet.wgsl");
        app.add_plugins(MaterialPlugin::<WetMaterial>::default()).add_systems(Update, soak.after(crate::LookSet));
    }
}

/// A surface that can get wet.
pub type WetMaterial = ExtendedMaterial<StandardMaterial, Wet>;

#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
pub struct Wet {
    /// x: wetness now (set each frame), y: how much this may get wet
    /// (`allow`), z: puddles on it (0..1), w: darkening and gloss scale.
    #[uniform(100)]
    pub params: Vec4,
    /// x: the puddles' scale (the `wet.puddles` knob; set each frame).
    #[uniform(101)]
    pub look: Vec4,
    /// Screen-space reflections ([`crate::ssr::params`]; set each frame).
    #[uniform(102)]
    pub ssr: Vec4,
    #[texture(103)]
    #[sampler(104)]
    pub history: Option<Handle<Image>>,
}

impl Wet {
    /// Open ground: wets fully, gathers puddles.
    pub fn ground() -> Self {
        Wet { params: Vec4::new(0.0, 1.0, 1.0, 1.0), look: Vec4::ONE, ssr: Vec4::ZERO, history: None }
    }

    /// Walls, props: wets, no puddles.
    pub fn surface() -> Self {
        Wet { params: Vec4::new(0.0, 1.0, 0.0, 1.0), look: Vec4::ONE, ssr: Vec4::ZERO, history: None }
    }

    /// `allow` 0..1 (0: never wet, under a roof).
    pub fn new(allow: f32, puddles: f32) -> Self {
        Wet { params: Vec4::new(0.0, allow.clamp(0.0, 1.0), puddles.clamp(0.0, 1.0), 1.0), look: Vec4::ONE, ssr: Vec4::ZERO, history: None }
    }
}

impl Default for Wet {
    fn default() -> Self {
        Wet::surface()
    }
}

impl MaterialExtension for Wet {
    fn fragment_shader() -> ShaderRef {
        "embedded://bevy_atmos/wet.wgsl".into()
    }
}

/// The weather's wetness into every wet material (when it changes).
fn soak(weather: Res<crate::weather::Weather>, mut materials: ResMut<Assets<WetMaterial>>, mut last: Local<Option<(f32, f32, f32, Vec4, usize)>>) {
    let w = weather.wetness;
    let scale = crate::knobs::get("wet.darken", 1.0).clamp(0.0, 3.0);
    let puddles = crate::knobs::get("wet.puddles", 1.0).clamp(0.0, 3.0);
    let now = (w, scale, puddles, crate::ssr::params(), materials.len());
    if last.is_some_and(|l| (l.0 - now.0).abs() < 0.003 && l.1 == now.1 && l.2 == now.2 && l.3 == now.3 && l.4 == now.4) {
        return;
    }
    *last = Some(now);
    let ids: Vec<_> = materials.ids().collect();
    for id in ids {
        if let Some(mut m) = materials.get_mut(id) {
            m.extension.params.x = w;
            m.extension.params.w = scale;
            m.extension.look.x = puddles;
            m.extension.ssr = now.3;
            if m.extension.history.is_none() {
                m.extension.history = Some(crate::ssr::history());
            }
        }
    }
}
