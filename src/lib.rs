//! # bevy_atmos
//!
//! Look development for Bevy games: time of day and the sun, a sky with
//! raymarched clouds, sun shafts, distance fog, rain and snow, wet
//! surfaces, water and the colour grade, all driven by live **knobs** that
//! an in-game editor panel (F9) sets and saves per level. A free-flying
//! camera (F8) is included for judging the look.
//!
//! ```no_run
//! use bevy::prelude::*;
//! use bevy_atmos::{AtmosCamera, AtmosPlugin, AtmosSun};
//!
//! fn main() {
//!     App::new()
//!         .add_plugins((DefaultPlugins, AtmosPlugin::new("my_level", "looks")))
//!         .add_systems(Startup, |mut commands: Commands| {
//!             commands.spawn((Camera3d::default(), AtmosCamera));
//!             commands.spawn((DirectionalLight { shadow_maps_enabled: true, ..default() }, AtmosSun));
//!         })
//!         .run();
//! }
//! ```
//!
//! What the host provides:
//! - [`AtmosCamera`] on the camera the look applies to (sky, fog, grade,
//!   shafts, rain round it). It needs `Hdr` for the sky's brightness.
//! - [`AtmosSun`] on the `DirectionalLight` that is the sun.
//! - [`AtmosLevel`], the level's name, which names its saved look file;
//!   change it when the level changes and its look is loaded.
//! - Optional: [`wet::WetMaterial`] (a `StandardMaterial` extension) on
//!   surfaces that may get wet, [`modern_water::AtmosWater`] on the
//!   level's water meshes (replaced by modern water with `water.replace`),
//!   or [`water::WaterMaterial`] as a simple water material,
//!   [`weather::Shelter`] for where rain can't fall.
//!
//! Every value the look uses is a knob ([`knobs`]): `knobs::get(name,
//! default)` anywhere, set live by the editor and loaded from the level's
//! saved look (`<save dir>/<level>.txt`, `name = value` lines).

use bevy::prelude::*;

pub mod air;
mod cloud_noise;
pub mod clouds;
pub mod editor;
pub mod flow;
pub mod flycam;
pub mod grade;
pub mod knobs;
pub mod modern_water;
pub mod sky;
pub mod ssr;
pub mod time;
pub mod water;
pub mod weather;
pub mod wet;

/// The look applies to this camera.
#[derive(Component, Clone, Copy, Default)]
pub struct AtmosCamera;

/// This directional light is the sun.
#[derive(Component, Clone, Copy, Default)]
pub struct AtmosSun;

/// The level being played: its look is saved as `<save dir>/<name>.txt` and
/// loaded when this changes.
#[derive(Resource, Clone, Debug, PartialEq, Eq)]
pub struct AtmosLevel(pub String);

/// Where looks are saved and read.
#[derive(Resource, Clone, Debug)]
pub struct AtmosSettings {
    /// Folder of `<level>.txt` look files.
    pub save_dir: std::path::PathBuf,
    /// Looks built into the game (file name, text), read when the save
    /// folder has none: e.g. `include_str!` of shipped looks.
    pub builtin: &'static [(&'static str, &'static str)],
    /// The editor's key (F9) and the fly camera's (F8).
    pub editor_key: KeyCode,
    pub flycam_key: KeyCode,
}

/// Adds everything: knobs and saved looks, time of day, sky, air, weather,
/// wet surfaces, water, grade, fly camera and the editor.
pub struct AtmosPlugin {
    pub level: String,
    pub save_dir: std::path::PathBuf,
    pub builtin: &'static [(&'static str, &'static str)],
    /// The editor's key (F9 unless set) and the fly camera's (F8).
    pub editor_key: KeyCode,
    pub flycam_key: KeyCode,
}

impl AtmosPlugin {
    pub fn new(level: impl Into<String>, save_dir: impl Into<std::path::PathBuf>) -> Self {
        AtmosPlugin { level: level.into(), save_dir: save_dir.into(), builtin: &[], editor_key: KeyCode::F9, flycam_key: KeyCode::F8 }
    }

    /// Other keys for the editor and the fly camera (when the game uses F9/F8).
    pub fn with_keys(mut self, editor: KeyCode, flycam: KeyCode) -> Self {
        self.editor_key = editor;
        self.flycam_key = flycam;
        self
    }
}

impl Plugin for AtmosPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(AtmosLevel(self.level.clone()))
            .insert_resource(AtmosSettings { save_dir: self.save_dir.clone(), builtin: self.builtin, editor_key: self.editor_key, flycam_key: self.flycam_key })
            .add_plugins((
                knobs::KnobsPlugin,
                ssr::SsrPlugin,
                time::TimePlugin,
                sky::SkyPlugin,
                clouds::CloudsPlugin,
                air::AirPlugin,
                weather::WeatherPlugin,
                wet::WetPlugin,
                water::WaterPlugin,
                modern_water::ModernWaterPlugin,
                flow::FlowPlugin,
                grade::GradePlugin,
                flycam::FlycamPlugin,
                editor::EditorPlugin,
            ));
    }
}

/// Systems that read the look run after this set (time and weather first).
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct LookSet;
