//! The look editor: F9 opens a side panel of sliders in six tabs (Time &
//! Sun, Sky & Clouds, Weather, Water, Fog & Light, Colour), each with its
//! everyday settings first and the rest under "Advanced". Each slider is a
//! knob ([`crate::knobs`]); changes apply live, the game keeps running.
//!
//! Presets (Clear Day, Overcast, Golden Hour, Night, Storm, Snow) set several
//! knobs at once, to tweak from. Save writes the level's look file, Revert
//! goes back to it, and "Hold to compare" shows the level as the game made
//! it, with no look (A/B). The status line says which level's look is loaded
//! and whether it has unsaved changes.
//!
//! While [`open`], the pointer is free: the host should ignore its own
//! mouse look (as for [`crate::flycam::active`]).
//!
//! Mouse: drag a slider, click its value (or "reset") to put it back, wheel
//! to scroll. Pad: D-pad up/down a row, left/right the value (or the
//! preset), A apply / toggle, LB/RB the tab, X reset, Y save, B close, hold
//! LT to compare. Keys: arrows, Enter, PageUp/PageDown, Delete, Ctrl+S, hold H.

use bevy::input::gamepad::{Gamepad, GamepadButton};
use bevy::input::mouse::{MouseScrollUnit, MouseWheel};
use bevy::prelude::*;
use bevy::ui::RelativeCursorPosition;
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};

pub struct EditorPlugin;

impl Plugin for EditorPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Editor>().add_systems(Update, (toggle, navigate, mouse, wheel, buttons, refresh).chain());
    }
}

static OPEN: AtomicBool = AtomicBool::new(false);

/// The editor's panel is open (the pointer is the panel's).
pub fn open() -> bool {
    OPEN.load(Ordering::Relaxed)
}

#[derive(Resource, Default)]
pub struct Editor {
    open: bool,
    tab: usize,
    /// The focused line ([`lines`]).
    row: usize,
    dirty: bool,
    message: Option<(String, f32)>,
    repeat: (i32, f32),
    /// Per tab: its Advanced settings shown.
    advanced: [bool; TABS.len()],
    /// The preset the pad's left/right is on.
    preset: usize,
    /// The list's scroll (kept across rebuilds of the same tab), and a line
    /// to bring into view.
    scroll: f32,
    scroll_to: Option<usize>,
    /// The look as saved (level, knobs, there is a file): for "unsaved".
    saved: Option<(String, BTreeMap<String, f32>, bool)>,
}

impl Editor {
    /// Opens (or closes) the panel, on a tab.
    pub fn set_open(&mut self, open: bool, tab: usize) {
        let tab = tab.min(TABS.len() - 1);
        if tab != self.tab {
            self.scroll = 0.0;
        }
        self.open = open;
        self.tab = tab;
        self.dirty = true;
    }

    /// Shows (or hides) the open tab's Advanced settings.
    pub fn set_advanced(&mut self, on: bool) {
        self.advanced[self.tab] = on;
        self.dirty = true;
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Auto {
    No,
    Hour,
    SunAz,
    SunEl,
    Wetness,
}

/// How a value reads.
#[derive(Clone, Copy, PartialEq)]
enum Unit {
    /// A number and its unit, to so many decimals.
    Num(&'static str, usize),
    /// 1 is 100%.
    Percent,
    Hour,
    Deg,
    /// Degrees, or off below 0.
    DegOff,
    Toggle,
    /// -max..max between two words (0 neutral).
    Balance(&'static str, &'static str),
    /// Metres, 0 the game's own.
    FogDistance,
}

struct Def {
    tab: usize,
    /// Under the tab's Advanced.
    adv: bool,
    label: &'static str,
    key: &'static str,
    min: f32,
    max: f32,
    step: f32,
    fallback: f32,
    auto: Auto,
    unit: Unit,
    help: &'static str,
}

struct Tab {
    name: &'static str,
    hint: &'static str,
}

const TABS: [Tab; 6] = [
    Tab { name: "Time & Sun", hint: "What time it is and where the sun sits. Light and sky follow the clock." },
    Tab { name: "Sky & Clouds", hint: "How much cloud there is, how grey it is, and the sky's colour." },
    Tab { name: "Weather", hint: "Rain, snow and wind, and how wet the ground gets." },
    Tab { name: "Water", hint: "Colour, waves, foam and rivers. Turn on Modern water first." },
    Tab { name: "Fog & Light", hint: "Haze in the distance, sun shafts and reflections." },
    Tab { name: "Colour", hint: "The final picture: brightness, warmth, contrast and saturation." },
];

#[allow(clippy::too_many_arguments)]
const fn d(tab: usize, adv: bool, label: &'static str, key: &'static str, min: f32, max: f32, step: f32, fallback: f32, auto: Auto, unit: Unit, help: &'static str) -> Def {
    Def { tab, adv, label, key, min, max, step, fallback, auto, unit, help }
}

const B: bool = false;
const A: bool = true;
const PCT: Unit = Unit::Percent;
const WARM: Unit = Unit::Balance("cooler", "warmer");
const fn n(unit: &'static str, decimals: usize) -> Unit {
    Unit::Num(unit, decimals)
}
use Auto::No as N;

const DEFS: &[Def] = &[
    // Time & Sun
    d(0, B, "Time of day", "time.hour", 0.0, 24.0, 0.25, -1.0, Auto::Hour, Unit::Hour, "The clock. Unset, the level's own sun decides the hour."),
    d(0, B, "Clock speed", "time.speed", 0.0, 600.0, 10.0, 0.0, N, n("x", 0), "How fast time passes: 60 is a game minute every real second. 0 stops the clock."),
    d(0, B, "Sun strength", "sun.brightness", 0.0, 3.0, 0.05, 1.0, N, PCT, "How bright direct sunlight (or moonlight at night) is."),
    d(0, B, "Sun colour", "sun.warmth", -1.0, 1.0, 0.05, 0.0, N, WARM, "Tints the sunlight only: warmer is orange, cooler is blue-white."),
    d(0, B, "Shadow fill (sky light)", "sky.light", 0.0, 3.0, 0.05, 1.0, N, PCT, "Light from the sky that fills shadows. Lower it for deeper, moodier shadows."),
    d(0, A, "Latitude", "time.latitude", -70.0, 70.0, 1.0, 35.0, N, Unit::Deg, "How high the sun climbs at noon: near 0 it passes overhead, far north or south it stays low."),
    d(0, A, "Sun compass bearing", "sun.azimuth", 0.0, 360.0, 2.0, -1.0, Auto::SunAz, Unit::Deg, "Pins the sun to a compass direction (with its height), ignoring the clock."),
    d(0, A, "Sun height", "sun.elevation", 1.0, 90.0, 1.0, -1.0, Auto::SunEl, Unit::Deg, "Pins the sun's height above the horizon (with its bearing), ignoring the clock."),
    // Sky & Clouds
    d(1, B, "Cloud cover", "sky.cover", 0.0, 1.0, 0.02, 0.4, N, PCT, "How much of the sky is cloud."),
    d(1, B, "Cloud greyness", "sky.darkness", 0.0, 1.0, 0.02, 0.3, N, PCT, "How dark the clouds are underneath: low for fair-weather white, high for storm grey."),
    d(1, B, "Cloud drift", "sky.wind_speed", 0.0, 10.0, 0.1, 1.0, N, n("x", 1), "How fast clouds cross the sky. Their direction is Weather > Wind direction."),
    d(1, B, "Sky & horizon colour", "sky.tint", -1.0, 1.0, 0.05, 0.0, N, WARM, "Tints the sky and the fog together."),
    d(1, B, "Sky brightness", "sky.brightness", 0.0, 3.0, 0.05, 1.0, N, PCT, "How bright the sky itself looks."),
    d(1, A, "Dynamic sky", "sky.enabled", 0.0, 1.0, 1.0, 1.0, N, Unit::Toggle, "Off: the game's own sky image, untouched (and none of the clouds below)."),
    d(1, A, "Physical sky colours", "sky.physical", 0.0, 1.0, 1.0, 1.0, N, Unit::Toggle, "On: the sky's colours come from the sun's position (sunsets go orange). Off: from the game's sky image."),
    d(1, A, "Volumetric clouds", "sky.screen_clouds", 0.0, 1.0, 1.0, 1.0, N, Unit::Toggle, "On: 3D clouds traced per pixel (best looking). Off: cheaper clouds painted into the sky."),
    d(1, A, "Cloud thickness", "clouds.density", 0.0, 3.0, 0.05, 1.0, N, PCT, "How dense each cloud is: thin and see-through, or thick and solid."),
    d(1, A, "Cloud wispiness", "clouds.erosion", 0.0, 2.0, 0.05, 1.0, N, PCT, "Fine detail eating into cloud edges."),
    d(1, A, "Cloud quality", "clouds.resolution", 0.125, 1.0, 0.125, 0.5, N, PCT, "The clouds' resolution, as a share of the screen's. Lower is faster."),
    d(1, A, "Cloud height", "sky.altitude", -1000.0, 4000.0, 50.0, 0.0, N, n("m", 0), "Raises or lowers the cloud layer."),
    d(1, A, "Cloud position", "sky.offset", 0.0, 20000.0, 100.0, 0.0, N, n("m", 0), "Slides the cloud pattern, to move a cloud out of (or into) a shot."),
    // Weather
    d(2, B, "Rain", "weather.rain", 0.0, 1.0, 0.02, 0.0, N, PCT, "How hard it rains: drizzle to downpour."),
    d(2, B, "Snow", "weather.snow", 0.0, 1.0, 0.02, 0.0, N, PCT, "How hard it snows."),
    d(2, B, "Wind speed", "weather.wind", 0.0, 30.0, 0.5, 2.0, N, n("m/s", 1), "Slants the rain and snow, roughens the water and pushes the clouds."),
    d(2, B, "Wind direction", "weather.wind_dir", 0.0, 360.0, 5.0, 30.0, N, Unit::Deg, "Where the wind blows toward: rain, clouds and waves all follow it."),
    d(2, B, "Wetness", "weather.wetness", 0.0, 1.0, 0.02, -1.0, Auto::Wetness, PCT, "How wet surfaces look. Unset, it follows the rain (and dries slowly after)."),
    d(2, A, "Rain visibility", "rain.opacity", 0.0, 1.0, 0.02, 0.35, N, PCT, "How see-through the rain streaks are."),
    d(2, A, "Rain brightness", "rain.brightness", 0.0, 4.0, 0.05, 1.0, N, PCT, "How much the rain catches the light."),
    d(2, A, "Rain near the camera", "rain.near", 0.0, 1.0, 0.02, 1.0, N, PCT, "The streaks closest to the camera."),
    d(2, A, "Rain in the distance", "rain.far", 0.0, 1.0, 0.02, 0.7, N, PCT, "The streaks further off, that read as a sheet of rain."),
    d(2, A, "Puddles", "wet.puddles", 0.0, 2.0, 0.05, 1.0, N, PCT, "How much standing water collects on wet ground."),
    d(2, A, "Soaked darkening", "wet.darken", 0.0, 2.0, 0.05, 1.0, N, PCT, "How much darker surfaces get as they soak."),
    // Water
    d(3, B, "Modern water", "water.replace", 0.0, 1.0, 1.0, 0.0, N, Unit::Toggle, "On: the level's water is replaced by modern water (waves, foam, reflections, rivers). Off: the original."),
    d(3, B, "Water colour", "water.tint", -1.0, 1.0, 0.05, 0.0, N, Unit::Balance("bluer", "browner"), "From clear blue to murky green-brown."),
    d(3, B, "Water brightness", "water.brightness", 0.0, 2.0, 0.05, 1.0, N, PCT, "How light or dark the water's own colour is."),
    d(3, B, "Wave height", "water.wave_height", 0.0, 2.0, 0.02, 0.3, N, n("m", 2), "How tall the waves are."),
    d(3, B, "Wave speed", "water.wave_speed", 0.0, 3.0, 0.05, 1.0, N, n("x", 2), "How fast the waves roll."),
    d(3, B, "Foam", "water.foam", 0.0, 3.0, 0.05, 1.0, N, PCT, "White foam where the water meets the shore and objects."),
    d(3, B, "River speed", "water.flow_speed", 0.0, 4.0, 0.05, 1.0, N, n("x", 2), "How fast every river flows."),
    d(3, A, "Murk through depth", "water.absorption", 0.0, 4.0, 0.05, 1.0, N, PCT, "How quickly deep water hides what's below it."),
    d(3, A, "Choppiness", "water.choppiness", 0.0, 1.0, 0.02, 0.4, N, PCT, "Sharp, pointed waves instead of smooth swells."),
    d(3, A, "Ripple size", "water.detail", 0.1, 4.0, 0.05, 1.0, N, n("x", 2), "The scale of the small ripples on top of the waves."),
    d(3, A, "Shore foam width", "water.foam_width", 0.02, 3.0, 0.02, 0.4, N, n("m", 2), "How far shore and object foam reaches out."),
    d(3, A, "Wave-top foam", "water.crest_foam", 0.0, 3.0, 0.05, 0.6, N, PCT, "Whitecaps on the crests of rough water."),
    d(3, A, "Wave-top foam threshold", "water.foam_threshold", 0.0, 1.5, 0.02, 0.5, N, n("", 2), "Higher: only the sharpest crests get whitecaps."),
    d(3, A, "Rain rings", "water.ripples", 0.0, 3.0, 0.05, 1.0, N, PCT, "Rings where raindrops hit the water (times the rain)."),
    d(3, A, "Underwater fog", "water.under_fog", 0.0, 2.0, 0.01, 0.15, N, n("per m", 2), "How thick the water is to see through with the camera under it."),
    d(3, A, "Sky reflection", "water.reflection", 0.0, 3.0, 0.05, 1.0, N, PCT, "How much the water mirrors the sky and the scene."),
    d(3, A, "Sun glitter", "water.glitter", 0.0, 4.0, 0.05, 1.0, N, PCT, "The sparkle of the sun (or moon) on the waves."),
    d(3, A, "Reflection angle", "water.fresnel", 0.1, 3.0, 0.05, 1.0, N, n("x", 2), "Higher: the water reflects even looking steeply down at it."),
    d(3, A, "Flowing rivers", "water.flow", 0.0, 1.0, 1.0, 1.0, N, Unit::Toggle, "Off: rivers stand still."),
    d(3, A, "All rivers flow toward", "water.flow_dir", -1.0, 360.0, 5.0, -1.0, N, Unit::DegOff, "Turns every river one way. Off: each flows its own way."),
    d(3, A, "White water from", "water.rapids", 0.2, 10.0, 0.1, 2.5, N, n("m/s", 1), "Rivers faster than this break into white water (steep slopes count as fast)."),
    d(3, A, "Foam trail length", "water.trail", 0.0, 15.0, 0.25, 4.0, N, n("m", 2), "How far foam trails behind things standing in a river."),
    d(3, A, "Flow brush size", "water.brush_radius", 0.5, 20.0, 0.25, 3.0, N, n("m", 2), "The radius of the Paint flow brush."),
    d(3, A, "Flow brush speed", "water.brush_speed", 0.0, 10.0, 0.1, 1.5, N, n("m/s", 1), "How fast the water you paint flows."),
    d(3, A, "Original water clarity", "water.clarity", 0.0, 2.0, 0.05, 1.0, N, PCT, "With Modern water off: how clear the level's own water is."),
    d(3, A, "Original water waves", "water.waves", 0.0, 1.5, 0.05, 1.0, N, PCT, "With Modern water off: how strong its waves are."),
    // Fog & Light
    d(4, B, "Fog distance", "fog.distance", 0.0, 3000.0, 25.0, 0.0, N, Unit::FogDistance, "How far you see before the haze takes over. Lower is thicker fog; 0 keeps the game's fog."),
    d(4, B, "Fog brightness", "fog.brightness", 0.0, 3.0, 0.05, 1.0, N, PCT, "How bright (lit) the fog is."),
    d(4, B, "Sun shafts", "shafts.strength", 0.0, 4.0, 0.05, 1.0, N, PCT, "Beams of sunlight through the haze, past trees and buildings. 0 turns them off."),
    d(4, B, "Reflections", "ssr.strength", 0.0, 2.0, 0.05, 1.0, N, PCT, "Wet ground and shiny floors mirroring what's on screen."),
    d(4, A, "Fog starts at", "fog.start", 0.0, 1000.0, 5.0, 0.0, N, n("m", 0), "The fog begins this far from the camera."),
    d(4, A, "Sun shaft quality", "shafts.steps", 8.0, 64.0, 2.0, 24.0, N, n("steps", 0), "More is smoother and slower."),
    d(4, A, "Screen reflections", "ssr.enabled", 0.0, 1.0, 1.0, 1.0, N, Unit::Toggle, "Off: no screen-space reflections at all (a little faster)."),
    d(4, A, "Reflection roughness limit", "ssr.roughness", 0.02, 1.0, 0.02, 0.45, N, PCT, "Surfaces rougher than this don't take reflections."),
    d(4, A, "Reflection distance", "ssr.distance", 2.0, 200.0, 1.0, 30.0, N, n("m", 0), "How far reflections reach."),
    // Colour
    d(5, B, "Exposure", "grade.exposure", -3.0, 3.0, 0.05, 0.0, N, n("stops", 2), "Brightens or darkens the whole picture, like a camera's exposure."),
    d(5, B, "Warmth", "grade.warmth", -0.3, 0.3, 0.01, 0.0, N, WARM, "Tints the whole picture: warmer is orange, cooler is blue."),
    d(5, B, "Tint", "grade.tint", -0.3, 0.3, 0.01, 0.0, N, Unit::Balance("greener", "pinker"), "Tints the whole picture between green and magenta."),
    d(5, B, "Contrast", "grade.contrast", 0.5, 2.0, 0.02, 1.0, N, PCT, "The gap between darks and lights."),
    d(5, B, "Saturation", "grade.saturation", 0.0, 2.0, 0.02, 1.0, N, PCT, "How colourful the picture is. 0 is black and white."),
];

/// The presets: knobs each sets (`None`: back to the default). They all set
/// the same everyday knobs, so one after another doesn't leave the last's
/// behind.
const PRESETS: &[(&str, &[(&str, Option<f32>)])] = &[
    ("Clear Day", &[
        ("time.hour", Some(13.0)), ("sun.brightness", Some(1.1)), ("sun.warmth", Some(0.0)), ("sky.light", Some(1.0)),
        ("sky.cover", Some(0.15)), ("sky.darkness", Some(0.15)), ("sky.tint", Some(0.0)), ("sky.brightness", Some(1.0)),
        ("weather.rain", Some(0.0)), ("weather.snow", Some(0.0)), ("weather.wind", Some(2.0)),
        ("fog.distance", None), ("fog.brightness", Some(1.0)), ("shafts.strength", Some(0.6)),
        ("grade.exposure", Some(0.0)), ("grade.warmth", Some(0.0)), ("grade.contrast", Some(1.0)), ("grade.saturation", Some(1.05)),
    ]),
    ("Overcast", &[
        ("time.hour", Some(12.0)), ("sun.brightness", Some(0.35)), ("sun.warmth", Some(-0.1)), ("sky.light", Some(1.3)),
        ("sky.cover", Some(0.78)), ("sky.darkness", Some(0.5)), ("sky.tint", Some(-0.15)), ("sky.brightness", Some(0.85)),
        ("weather.rain", Some(0.0)), ("weather.snow", Some(0.0)), ("weather.wind", Some(5.0)),
        ("fog.distance", Some(900.0)), ("fog.brightness", Some(0.9)), ("shafts.strength", Some(0.3)),
        ("grade.exposure", Some(0.1)), ("grade.warmth", Some(-0.03)), ("grade.contrast", Some(0.95)), ("grade.saturation", Some(0.85)),
    ]),
    ("Golden Hour", &[
        ("time.hour", Some(17.0)), ("sun.brightness", Some(1.2)), ("sun.warmth", Some(0.55)), ("sky.light", Some(0.9)),
        ("sky.cover", Some(0.35)), ("sky.darkness", Some(0.25)), ("sky.tint", Some(0.25)), ("sky.brightness", Some(1.0)),
        ("weather.rain", Some(0.0)), ("weather.snow", Some(0.0)), ("weather.wind", Some(2.0)),
        ("fog.distance", Some(2200.0)), ("fog.brightness", Some(1.1)), ("shafts.strength", Some(1.6)),
        ("grade.exposure", Some(0.0)), ("grade.warmth", Some(0.04)), ("grade.contrast", Some(1.05)), ("grade.saturation", Some(1.1)),
    ]),
    ("Night", &[
        ("time.hour", Some(23.0)), ("sun.brightness", Some(1.0)), ("sun.warmth", Some(0.0)), ("sky.light", Some(0.7)),
        ("sky.cover", Some(0.3)), ("sky.darkness", Some(0.3)), ("sky.tint", Some(-0.2)), ("sky.brightness", Some(1.0)),
        ("weather.rain", Some(0.0)), ("weather.snow", Some(0.0)), ("weather.wind", Some(2.0)),
        ("fog.distance", None), ("fog.brightness", Some(1.0)), ("shafts.strength", Some(0.5)),
        ("grade.exposure", Some(0.6)), ("grade.warmth", Some(-0.05)), ("grade.contrast", Some(1.0)), ("grade.saturation", Some(0.9)),
    ]),
    ("Storm", &[
        ("time.hour", Some(15.0)), ("sun.brightness", Some(0.2)), ("sun.warmth", Some(-0.2)), ("sky.light", Some(1.1)),
        ("sky.cover", Some(1.0)), ("sky.darkness", Some(0.85)), ("sky.tint", Some(-0.3)), ("sky.brightness", Some(0.7)),
        ("weather.rain", Some(0.9)), ("weather.snow", Some(0.0)), ("weather.wind", Some(14.0)),
        ("fog.distance", Some(500.0)), ("fog.brightness", Some(0.7)), ("shafts.strength", Some(0.2)),
        ("grade.exposure", Some(-0.1)), ("grade.warmth", Some(-0.05)), ("grade.contrast", Some(1.1)), ("grade.saturation", Some(0.75)),
    ]),
    ("Snow", &[
        ("time.hour", Some(11.0)), ("sun.brightness", Some(0.5)), ("sun.warmth", Some(-0.15)), ("sky.light", Some(1.3)),
        ("sky.cover", Some(0.8)), ("sky.darkness", Some(0.35)), ("sky.tint", Some(-0.3)), ("sky.brightness", Some(1.0)),
        ("weather.rain", Some(0.0)), ("weather.snow", Some(0.8)), ("weather.wind", Some(4.0)),
        ("fog.distance", Some(450.0)), ("fog.brightness", Some(1.2)), ("shafts.strength", Some(0.3)),
        ("grade.exposure", Some(0.2)), ("grade.warmth", Some(-0.06)), ("grade.contrast", Some(0.95)), ("grade.saturation", Some(0.8)),
    ]),
];

/// Knobs every preset puts back to their defaults (a pinned sun would
/// ignore the preset's clock; wetness follows its rain).
const PRESET_CLEARS: &[&str] = &["sun.azimuth", "sun.elevation", "weather.wetness"];

/// The presets' names, in order.
pub fn preset_names() -> impl Iterator<Item = &'static str> {
    PRESETS.iter().map(|p| p.0)
}

/// Applies preset `i` ([`preset_names`]) as the editor's buttons do.
pub fn apply_preset(i: usize) {
    let Some(i) = (i < PRESETS.len()).then_some(i) else { return };
    for k in PRESET_CLEARS {
        crate::knobs::set(k, None);
    }
    for (k, v) in PRESETS[i].1 {
        crate::knobs::set(k, *v);
    }
}

/// The preset the look is now (every knob it sets as it sets it), if any.
fn active_preset() -> Option<usize> {
    PRESETS.iter().position(|(_, knobs)| {
        knobs.iter().all(|(k, v)| match v {
            Some(v) => crate::knobs::is_set(k) && crate::knobs::get(k, *v) == *v,
            None => !crate::knobs::is_set(k),
        })
    })
}

/// A line of the panel: the presets, a slider, or the Advanced switch.
#[derive(Clone, Copy, PartialEq)]
enum Line {
    Presets,
    Knob(usize),
    Advanced,
}

fn lines(e: &Editor) -> Vec<Line> {
    let mut out = vec![Line::Presets];
    out.extend((0..DEFS.len()).filter(|&i| DEFS[i].tab == e.tab && !DEFS[i].adv).map(Line::Knob));
    if DEFS.iter().any(|d| d.tab == e.tab && d.adv) {
        out.push(Line::Advanced);
        if e.advanced[e.tab] {
            out.extend((0..DEFS.len()).filter(|&i| DEFS[i].tab == e.tab && DEFS[i].adv).map(Line::Knob));
        }
    }
    out
}

fn advanced_count(tab: usize) -> usize {
    DEFS.iter().filter(|d| d.tab == tab && d.adv).count()
}

struct Now {
    hour: f32,
    sun: (f32, f32),
    wetness: f32,
}

fn value(def: &Def, now: &Now) -> (f32, bool) {
    if crate::knobs::is_set(def.key) {
        return (crate::knobs::get(def.key, def.fallback), true);
    }
    let v = match def.auto {
        Auto::Hour => now.hour,
        Auto::SunAz => now.sun.0,
        Auto::SunEl => now.sun.1,
        Auto::Wetness => now.wetness,
        Auto::No => crate::knobs::default_of(def.key).unwrap_or(def.fallback),
    };
    (v, false)
}

fn set(def: &Def, now: &Now, v: f32) {
    let v = if def.unit == Unit::Toggle { if v >= 0.5 { 1.0 } else { 0.0 } } else { v.clamp(def.min, def.max) };
    crate::knobs::set(def.key, Some(v));
    // (The sun's bearing and elevation go together.)
    for (a, b, other) in [(Auto::SunAz, Auto::SunEl, now.sun.1), (Auto::SunEl, Auto::SunAz, now.sun.0)] {
        if def.auto == a {
            if let Some(o) = DEFS.iter().find(|d| d.auto == b) {
                if !crate::knobs::is_set(o.key) {
                    crate::knobs::set(o.key, Some(other));
                }
            }
        }
    }
}

fn format(def: &Def, v: f32, set: bool) -> String {
    let s = match def.unit {
        Unit::Hour => format!("{:02}:{:02}", v.floor() as u32 % 24, (v.fract() * 60.0).round() as u32 % 60),
        Unit::Deg => format!("{v:.0} deg"),
        Unit::DegOff => if v < 0.0 { "Off".into() } else { format!("{v:.0} deg") },
        Unit::Toggle => if v >= 0.5 { "On".into() } else { "Off".into() },
        Unit::Percent => format!("{:.0}%", v * 100.0),
        Unit::FogDistance => if v <= 0.0 { "Game's".into() } else { format!("{v:.0} m") },
        Unit::Balance(neg, pos) => {
            let share = (v / def.max.abs().max(1e-6) * 100.0).round();
            if share == 0.0 { "Neutral".into() } else { format!("{:.0}% {}", share.abs(), if share < 0.0 { neg } else { pos }) }
        }
        Unit::Num(unit, decimals) => {
            let num = format!("{v:.decimals$}");
            match unit {
                "" => num,
                "x" => format!("{num}x"),
                u => format!("{num} {u}"),
            }
        }
    };
    if set || def.auto == Auto::No { s } else { format!("{s} (now)") }
}

fn now_of(tod: &crate::time::TimeOfDay, weather: &crate::weather::Weather) -> Now {
    Now { hour: tod.hours, sun: crate::time::dir_to_angles(tod.sun_dir), wetness: weather.wetness }
}

// --- the panel

#[derive(Component)]
struct Panel;

#[derive(Component, Clone, Copy)]
enum Btn {
    Tab(usize),
    Reset(usize),
    /// A switch (a toggle knob's track).
    Toggle(usize),
    Preset(usize),
    Advanced,
    Save,
    Revert,
    ClearLook,
    Ab,
    Fly,
    Close,
    PaintFlow,
    ClearFlow,
}

#[derive(Component)]
struct Track(usize);

#[derive(Component)]
struct Fill(usize);

#[derive(Component)]
struct ValueText(usize);

/// A slider's "reset" (shown while it's changed).
#[derive(Component)]
struct ResetMark(usize);

/// A line's background, by its index in [`lines`].
#[derive(Component)]
struct RowBg(usize);

#[derive(Component)]
struct PresetBtn(usize);

#[derive(Component)]
struct ScrollList;

#[derive(Component)]
struct Note;

#[derive(Component)]
struct Status;

#[derive(Component)]
struct Help;

const PANEL: Color = Color::srgba(0.055, 0.07, 0.075, 0.86);
const RAISED: Color = Color::srgba(0.12, 0.14, 0.15, 0.95);
const ROW: Color = Color::srgba(0.09, 0.11, 0.115, 0.62);
const ROW_FOCUS: Color = Color::srgba(0.45, 0.85, 0.35, 0.16);
const TRACK: Color = Color::srgba(1.0, 1.0, 1.0, 0.1);
const ACCENT: Color = Color::srgb(0.45, 0.85, 0.35);
const ACCENT_DIM: Color = Color::srgb(0.29, 0.56, 0.23);
const INK: Color = Color::srgb(0.925, 0.917, 0.886);
const INK_DIM: Color = Color::srgb(0.66, 0.67, 0.62);
const INK_MUTE: Color = Color::srgb(0.48, 0.5, 0.46);
const WARN: Color = Color::srgb(0.94, 0.71, 0.24);

/// A slider row's height and the gap under it (fixed, so the list can scroll
/// a focused row into view without asking the layout).
const ROW_H: f32 = 40.0;
const ROW_GAP: f32 = 3.0;

fn text(s: impl Into<String>, size: f32, c: Color) -> impl Bundle {
    (Text::new(s), TextFont { font_size: bevy::text::FontSize::Px(size), ..default() }, TextColor(c))
}

fn button(p: &mut ChildSpawnerCommands, label: &str, b: Btn, accent: bool) {
    p.spawn((Button, b, Node { padding: UiRect::axes(Val::Px(9.0), Val::Px(5.0)), margin: UiRect::all(Val::Px(2.0)), ..default() }, BackgroundColor(if accent { ACCENT_DIM } else { RAISED })))
        .with_children(|c| {
            c.spawn(text(label, 13.0, INK));
        });
}

fn build(commands: &mut Commands, e: &Editor) {
    let list = lines(e);
    commands
        .spawn((
            Panel,
            Node {
                position_type: PositionType::Absolute,
                right: Val::Px(0.0),
                top: Val::Px(0.0),
                width: Val::Px(400.0),
                height: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                padding: UiRect::all(Val::Px(10.0)),
                row_gap: Val::Px(3.0),
                overflow: Overflow::clip(),
                ..default()
            },
            BackgroundColor(PANEL),
            GlobalZIndex(900),
        ))
        .with_children(|p| {
            p.spawn(text("LOOK EDITOR", 16.0, INK));
            p.spawn((Status, text("", 12.0, INK_DIM)));
            // Tabs and the tab's hint.
            p.spawn(Node { flex_wrap: FlexWrap::Wrap, margin: UiRect::top(Val::Px(4.0)), ..default() }).with_children(|r| {
                for (i, tab) in TABS.iter().enumerate() {
                    button(r, tab.name, Btn::Tab(i), i == e.tab);
                }
            });
            p.spawn((text(TABS[e.tab].hint, 12.0, INK_DIM), Node { margin: UiRect::axes(Val::Px(2.0), Val::Px(2.0)), ..default() }));
            // Presets (line 0).
            p.spawn((RowBg(0), RelativeCursorPosition::default(), Node { flex_direction: FlexDirection::Column, padding: UiRect::all(Val::Px(4.0)), ..default() }, BackgroundColor(ROW))).with_children(|row| {
                row.spawn(text("Start from a preset", 12.0, INK_DIM));
                row.spawn(Node { flex_wrap: FlexWrap::Wrap, ..default() }).with_children(|r| {
                    for (i, (name, _)) in PRESETS.iter().enumerate() {
                        r.spawn((
                            Button,
                            Btn::Preset(i),
                            PresetBtn(i),
                            Node { padding: UiRect::axes(Val::Px(8.0), Val::Px(4.0)), margin: UiRect::all(Val::Px(2.0)), border: UiRect::all(Val::Px(1.0)), ..default() },
                            BackgroundColor(RAISED),
                            BorderColor::all(Color::NONE),
                        ))
                            .with_children(|c| {
                                c.spawn(text(*name, 13.0, INK));
                            });
                    }
                });
            });
            // The sliders (scrolling).
            p.spawn((
                ScrollList,
                ScrollPosition(Vec2::new(0.0, e.scroll)),
                Node { flex_direction: FlexDirection::Column, flex_grow: 1.0, flex_basis: Val::Px(0.0), row_gap: Val::Px(ROW_GAP), overflow: Overflow::scroll_y(), margin: UiRect::top(Val::Px(3.0)), ..default() },
            ))
            .with_children(|list_node| {
                for (n, line) in list.iter().enumerate() {
                    match *line {
                        Line::Presets => {}
                        Line::Advanced => {
                            let open = e.advanced[e.tab];
                            list_node
                                .spawn((
                                    RowBg(n),
                                    Button,
                                    Btn::Advanced,
                                    RelativeCursorPosition::default(),
                                    Node { height: Val::Px(ROW_H), min_height: Val::Px(ROW_H), align_items: AlignItems::Center, padding: UiRect::horizontal(Val::Px(6.0)), ..default() },
                                    BackgroundColor(ROW),
                                ))
                                .with_children(|r| {
                                    let label = if open { "Advanced  (hide)".to_owned() } else { format!("Advanced  ({} more settings)", advanced_count(e.tab)) };
                                    r.spawn(text(label, 14.0, ACCENT));
                                });
                        }
                        Line::Knob(i) => {
                            let def = &DEFS[i];
                            list_node
                                .spawn((
                                    RowBg(n),
                                    RelativeCursorPosition::default(),
                                    Node { flex_direction: FlexDirection::Column, height: Val::Px(ROW_H), min_height: Val::Px(ROW_H), padding: UiRect::axes(Val::Px(6.0), Val::Px(4.0)), ..default() },
                                    BackgroundColor(ROW),
                                ))
                                .with_children(|row| {
                                    row.spawn(Node { justify_content: JustifyContent::SpaceBetween, ..default() }).with_children(|l| {
                                        l.spawn(text(def.label, 14.0, INK));
                                        l.spawn(Node { column_gap: Val::Px(8.0), ..default() }).with_children(|v| {
                                            v.spawn((ResetMark(i), Button, Btn::Reset(i), Node { display: Display::None, ..default() })).with_children(|b| {
                                                b.spawn(text("reset", 12.0, INK_MUTE));
                                            });
                                            v.spawn((Button, Btn::Reset(i), Node::default())).with_children(|b| {
                                                b.spawn((ValueText(i), text("", 13.0, INK_DIM)));
                                            });
                                        });
                                    });
                                    // (A toggle is a short switch: click it to flip.)
                                    let toggle = def.unit == Unit::Toggle;
                                    let mut track = row.spawn((
                                        Track(i),
                                        Button,
                                        RelativeCursorPosition::default(),
                                        Node { width: if toggle { Val::Px(44.0) } else { Val::Percent(100.0) }, height: Val::Px(9.0), margin: UiRect::top(Val::Px(5.0)), ..default() },
                                        BackgroundColor(TRACK),
                                    ));
                                    if toggle {
                                        track.insert(Btn::Toggle(i));
                                    }
                                    track
                                    .with_children(|tr| {
                                        tr.spawn((Fill(i), Node { width: Val::Percent(0.0), height: Val::Percent(100.0), ..default() }, BackgroundColor(ACCENT)));
                                    });
                                });
                        }
                    }
                }
            });
            // What the hovered (else focused) slider does.
            p.spawn((Node { min_height: Val::Px(34.0), padding: UiRect::all(Val::Px(6.0)), ..default() }, BackgroundColor(RAISED))).with_children(|h| {
                h.spawn((Help, text("", 12.0, INK)));
            });
            p.spawn((Note, text("", 12.0, WARN)));
            p.spawn(Node { flex_wrap: FlexWrap::Wrap, ..default() }).with_children(|r| {
                button(r, "Save", Btn::Save, true);
                button(r, "Revert", Btn::Revert, false);
                button(r, "Hold to compare with original", Btn::Ab, false);
                button(r, "Clear look", Btn::ClearLook, false);
                button(r, "Fly camera (F8)", Btn::Fly, false);
                button(r, "Close (F9)", Btn::Close, false);
                if e.tab == 3 {
                    button(r, if crate::flow::painting() { "Painting river flow: on" } else { "Paint river flow" }, Btn::PaintFlow, crate::flow::painting());
                    button(r, "Clear painted flow", Btn::ClearFlow, false);
                }
            });
            p.spawn(text(
                "Mouse: drag sliders, click a value to reset it, wheel to scroll. Keys: arrows, Enter, PgUp/PgDn tabs, Del reset, Ctrl+S save, hold H compare. Pad: D-pad, A, LB/RB tabs, X reset, Y save, hold LT compare, B close.",
                11.0,
                INK_MUTE,
            ));
        });
}

fn set_cursor(cursor: &mut Query<&mut CursorOptions, With<PrimaryWindow>>, free: bool) {
    if let Ok(mut c) = cursor.single_mut() {
        c.grab_mode = if free { CursorGrabMode::None } else { CursorGrabMode::Locked };
        c.visible = free;
    }
}

#[allow(clippy::too_many_arguments)]
fn toggle(
    mut commands: Commands,
    mut e: ResMut<Editor>,
    keys: Res<ButtonInput<KeyCode>>,
    pads: Query<&Gamepad>,
    settings: Res<crate::AtmosSettings>,
    panels: Query<Entity, With<Panel>>,
    lists: Query<&ScrollPosition, With<ScrollList>>,
    mut cursor: Query<&mut CursorOptions, With<PrimaryWindow>>,
) {
    let flip = keys.just_pressed(settings.editor_key) || pads.iter().any(|g| g.pressed(GamepadButton::Select) && g.just_pressed(GamepadButton::DPadDown));
    let close = e.open && pads.iter().any(|g| g.just_pressed(GamepadButton::East));
    if flip || close {
        let open = !e.open && !close;
        let tab = e.tab;
        e.set_open(open, tab);
        if !open {
            crate::knobs::set_bypass(false);
        }
        set_cursor(&mut cursor, open);
    }
    let was = OPEN.swap(e.open, Ordering::Relaxed);
    if was && !e.open {
        set_cursor(&mut cursor, false);
    }
    if (!e.open || e.dirty) && !panels.is_empty() {
        for p in &panels {
            commands.entity(p).despawn();
        }
    }
    if e.open && e.dirty {
        e.dirty = false;
        build(&mut commands, &e);
        set_cursor(&mut cursor, true);
    } else if let Ok(s) = lists.single() {
        e.scroll = s.0.y;
    }
}

fn save_now(settings: &crate::AtmosSettings, level: &str, paint: Option<&crate::flow::FlowPaint>) -> String {
    // (The painted flow beside the look: `<level>.flow`.)
    if let Some(paint) = paint {
        if let Err(err) = crate::flow::save(paint, settings, level) {
            return format!("Couldn't save the painted flow: {err}");
        }
    }
    match crate::knobs::save(settings, level) {
        Ok(p) => format!("Saved {}", p.display()),
        Err(err) => format!("Couldn't save: {err}"),
    }
}

/// Saves, and takes the look as saved for the status line.
fn save(e: &mut Editor, settings: &crate::AtmosSettings, level: &str, paint: Option<&crate::flow::FlowPaint>, t: f32) {
    let m = save_now(settings, level, paint);
    if m.starts_with("Saved") {
        e.saved = Some((level.to_owned(), crate::knobs::all(), true));
    }
    e.message = Some((m, t + 4.0));
}

/// Back to the saved look.
fn revert(e: &mut Editor, settings: &crate::AtmosSettings, level: &str, t: f32) {
    crate::knobs::load(settings, level);
    e.saved = None;
    e.message = Some(("Back to the saved look".into(), t + 4.0));
}

/// The focused line's action (pad A, Enter).
fn activate(e: &mut Editor, line: Line, now: &Now, t: f32) {
    match line {
        Line::Presets => {
            apply_preset(e.preset);
            e.message = Some((format!("{}: tweak anything, then Save", PRESETS[e.preset].0), t + 4.0));
        }
        Line::Advanced => {
            let tab = e.tab;
            e.advanced[tab] = !e.advanced[tab];
            e.set_open(true, tab);
        }
        Line::Knob(i) => {
            let def = &DEFS[i];
            if def.unit == Unit::Toggle {
                let (v, _) = value(def, now);
                set(def, now, 1.0 - v.round());
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn navigate(
    mut e: ResMut<Editor>,
    time: Res<Time<Real>>,
    keys: Res<ButtonInput<KeyCode>>,
    pads: Query<&Gamepad>,
    tod: Res<crate::time::TimeOfDay>,
    weather: Res<crate::weather::Weather>,
    settings: Res<crate::AtmosSettings>,
    level: Res<crate::AtmosLevel>,
    paint: Option<Res<crate::flow::FlowPaint>>,
) {
    if !e.open {
        return;
    }
    let pressed = |b: GamepadButton| pads.iter().any(|g| g.just_pressed(b));
    let held = |b: GamepadButton| pads.iter().any(|g| g.pressed(b));
    if pressed(GamepadButton::LeftTrigger) || keys.just_pressed(KeyCode::PageUp) {
        let tab = (e.tab + TABS.len() - 1) % TABS.len();
        e.set_open(true, tab);
        e.row = 0;
        return;
    }
    if pressed(GamepadButton::RightTrigger) || keys.just_pressed(KeyCode::PageDown) {
        let tab = (e.tab + 1) % TABS.len();
        e.set_open(true, tab);
        e.row = 0;
        return;
    }
    let list = lines(&e);
    let row_was = e.row;
    if (pressed(GamepadButton::DPadDown) && !held(GamepadButton::Select)) || keys.just_pressed(KeyCode::ArrowDown) {
        e.row = (e.row + 1) % list.len();
    }
    if pressed(GamepadButton::DPadUp) || keys.just_pressed(KeyCode::ArrowUp) {
        e.row = (e.row + list.len() - 1) % list.len();
    }
    e.row = e.row.min(list.len() - 1);
    if e.row != row_was {
        e.scroll_to = Some(e.row);
    }
    let line = list[e.row];
    let now = now_of(&tod, &weather);
    let t = time.elapsed_secs();
    let dir = (held(GamepadButton::DPadRight) || keys.pressed(KeyCode::ArrowRight)) as i32 - (held(GamepadButton::DPadLeft) || keys.pressed(KeyCode::ArrowLeft)) as i32;
    let step_now = if dir == 0 {
        e.repeat = (0, 0.0);
        false
    } else if e.repeat.0 != dir {
        e.repeat = (dir, t + 0.35);
        true
    } else if t >= e.repeat.1 {
        e.repeat.1 = t + 0.06;
        true
    } else {
        false
    };
    if step_now {
        match line {
            Line::Presets => e.preset = (e.preset as i32 + dir).rem_euclid(PRESETS.len() as i32) as usize,
            Line::Advanced => {
                if e.advanced[e.tab] != (dir > 0) {
                    activate(&mut e, line, &now, t);
                }
            }
            Line::Knob(i) => {
                let def = &DEFS[i];
                let (v, _) = value(def, &now);
                set(def, &now, if def.unit == Unit::Toggle { 1.0 - v.round() } else { v + dir as f32 * def.step });
            }
        }
    }
    if pressed(GamepadButton::South) || keys.just_pressed(KeyCode::Enter) {
        activate(&mut e, line, &now, t);
    }
    if let Line::Knob(i) = line {
        if pressed(GamepadButton::West) || keys.just_pressed(KeyCode::Delete) {
            crate::knobs::set(DEFS[i].key, None);
        }
    }
    if pressed(GamepadButton::North) || (keys.pressed(KeyCode::ControlLeft) && keys.just_pressed(KeyCode::KeyS)) {
        save(&mut e, &settings, &level.0, paint.as_deref(), t);
    }
    let ab = held(GamepadButton::LeftTrigger2) || keys.pressed(KeyCode::KeyH);
    if ab != crate::knobs::bypass() && (ab || !AB_BUTTON.load(Ordering::Relaxed)) {
        crate::knobs::set_bypass(ab);
    }
}

static AB_BUTTON: AtomicBool = AtomicBool::new(false);

fn mouse(e: Res<Editor>, tracks: Query<(&Track, &Interaction, &RelativeCursorPosition)>, tod: Res<crate::time::TimeOfDay>, weather: Res<crate::weather::Weather>) {
    if !e.open {
        return;
    }
    let now = now_of(&tod, &weather);
    for (track, interaction, rel) in &tracks {
        if *interaction != Interaction::Pressed {
            continue;
        }
        let Some(p) = rel.normalized else { continue };
        let def = &DEFS[track.0];
        if def.unit == Unit::Toggle {
            continue;
        }
        let raw = def.min + (def.max - def.min) * (p.x + 0.5).clamp(0.0, 1.0);
        set(def, &now, (raw / def.step).round() * def.step);
    }
}

/// The wheel scrolls the sliders (over the panel), and the focused row is
/// kept in view.
fn wheel(mut e: ResMut<Editor>, mut wheel: MessageReader<MouseWheel>, mut list: Query<(&mut ScrollPosition, &ComputedNode, &RelativeCursorPosition), With<ScrollList>>) {
    let delta: f32 = wheel.read().map(|w| if w.unit == MouseScrollUnit::Line { w.y * ROW_H } else { w.y }).sum();
    if !e.open {
        return;
    }
    let Ok((mut pos, node, over)) = list.single_mut() else { return };
    let view = node.size().y * node.inverse_scale_factor();
    let pitch = ROW_H + ROW_GAP;
    let count = lines(&e).len().saturating_sub(1) as f32;
    let max = (count * pitch - ROW_GAP - view).max(0.0);
    let mut y = pos.0.y;
    if delta != 0.0 && over.cursor_over {
        y -= delta;
    }
    if let Some(row) = e.scroll_to.take() {
        if row == 0 {
            y = 0.0;
        } else {
            let top = (row - 1) as f32 * pitch;
            y = y.min(top).max(top + ROW_H - view);
        }
    }
    let y = y.clamp(0.0, max);
    if y != pos.0.y {
        pos.0.y = y;
        e.scroll = y;
    }
}

#[allow(clippy::too_many_arguments)]
fn buttons(
    mut e: ResMut<Editor>,
    time: Res<Time<Real>>,
    clicked: Query<(&Interaction, &Btn), Changed<Interaction>>,
    all: Query<(&Interaction, &Btn)>,
    settings: Res<crate::AtmosSettings>,
    level: Res<crate::AtmosLevel>,
    tod: Res<crate::time::TimeOfDay>,
    weather: Res<crate::weather::Weather>,
    mut fly: ResMut<crate::flycam::Flycam>,
    mut cursor: Query<&mut CursorOptions, With<PrimaryWindow>>,
    mut paint: Option<ResMut<crate::flow::FlowPaint>>,
) {
    if !e.open {
        return;
    }
    let ab = all.iter().any(|(i, b)| matches!(b, Btn::Ab) && *i == Interaction::Pressed);
    AB_BUTTON.store(ab, Ordering::Relaxed);
    if ab {
        crate::knobs::set_bypass(true);
    } else if clicked.iter().any(|(_, b)| matches!(b, Btn::Ab)) {
        crate::knobs::set_bypass(false);
    }
    let t = time.elapsed_secs();
    let now = now_of(&tod, &weather);
    for (interaction, b) in &clicked {
        if *interaction != Interaction::Pressed {
            continue;
        }
        match *b {
            Btn::Tab(i) => {
                e.set_open(true, i);
                e.row = 0;
            }
            Btn::Reset(i) => crate::knobs::set(DEFS[i].key, None),
            Btn::Toggle(i) => {
                let (v, _) = value(&DEFS[i], &now);
                set(&DEFS[i], &now, 1.0 - v.round());
            }
            Btn::Preset(i) => {
                e.preset = i;
                e.row = 0;
                activate(&mut e, Line::Presets, &now, t);
            }
            Btn::Advanced => {
                e.row = lines(&e).iter().position(|l| *l == Line::Advanced).unwrap_or(0);
                activate(&mut e, Line::Advanced, &now, t);
            }
            Btn::Save => save(&mut e, &settings, &level.0, paint.as_deref(), t),
            Btn::Revert => revert(&mut e, &settings, &level.0, t),
            Btn::ClearLook => {
                crate::knobs::replace(Default::default());
                e.message = Some(("Look cleared: the level as the game made it (Save to keep it so)".into(), t + 4.0));
            }
            Btn::Fly => {
                let tab = e.tab;
                e.set_open(false, tab);
                crate::knobs::set_bypass(false);
                fly.enter();
            }
            Btn::Close => {
                let tab = e.tab;
                e.set_open(false, tab);
                crate::knobs::set_bypass(false);
                set_cursor(&mut cursor, false);
            }
            Btn::Ab => {}
            Btn::PaintFlow => {
                crate::flow::set_painting(!crate::flow::painting());
                let tab = e.tab;
                e.set_open(true, tab);
                e.message = Some((if crate::flow::painting() { "Drag on the water (left button) to paint its flow; right button rubs it out".into() } else { "Flow painting off".into() }, t + 6.0));
            }
            Btn::ClearFlow => {
                if let Some(p) = paint.as_mut() {
                    p.clear();
                }
                e.message = Some(("Painted flow cleared (Save to keep it so)".into(), t + 4.0));
            }
        }
    }
}

/// The line under the pointer, if any.
fn hovered(rows: &Query<(&RowBg, &mut BackgroundColor, &RelativeCursorPosition)>) -> Option<usize> {
    rows.iter().find(|(_, _, rel)| rel.cursor_over).map(|(r, _, _)| r.0)
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn refresh(
    mut e: ResMut<Editor>,
    time: Res<Time<Real>>,
    tod: Res<crate::time::TimeOfDay>,
    weather: Res<crate::weather::Weather>,
    settings: Res<crate::AtmosSettings>,
    level: Res<crate::AtmosLevel>,
    mut values: Query<(&ValueText, &mut Text, &mut TextColor), (Without<Note>, Without<Status>, Without<Help>)>,
    mut fills: Query<(&Fill, &mut Node), Without<ResetMark>>,
    mut marks: Query<(&ResetMark, &mut Node), Without<Fill>>,
    mut rows_bg: Query<(&RowBg, &mut BackgroundColor, &RelativeCursorPosition)>,
    mut presets: Query<(&PresetBtn, &mut BackgroundColor, &mut BorderColor), Without<RowBg>>,
    mut note: Query<&mut Text, (With<Note>, Without<ValueText>, Without<Status>, Without<Help>)>,
    mut status: Query<&mut Text, (With<Status>, Without<ValueText>, Without<Note>, Without<Help>)>,
    mut help: Query<&mut Text, (With<Help>, Without<ValueText>, Without<Note>, Without<Status>)>,
) {
    if !e.open {
        return;
    }
    let now = now_of(&tod, &weather);
    for (v, mut s, mut c) in &mut values {
        let def = &DEFS[v.0];
        let (x, set) = value(def, &now);
        let want = format(def, x, set);
        if s.0 != want {
            s.0 = want;
        }
        let colour = if set { ACCENT } else { INK_DIM };
        if c.0 != colour {
            c.0 = colour;
        }
    }
    for (f, mut node) in &mut fills {
        let def = &DEFS[f.0];
        let (x, _) = value(def, &now);
        node.width = Val::Percent(((x - def.min) / (def.max - def.min)).clamp(0.0, 1.0) * 100.0);
    }
    for (m, mut node) in &mut marks {
        let want = if crate::knobs::is_set(DEFS[m.0].key) { Display::Flex } else { Display::None };
        if node.display != want {
            node.display = want;
        }
    }
    let list = lines(&e);
    let hover = hovered(&rows_bg);
    for (r, mut bg, _) in &mut rows_bg {
        let want = if r.0 == e.row || Some(r.0) == hover { ROW_FOCUS } else { ROW };
        if bg.0 != want {
            bg.0 = want;
        }
    }
    // Filled: the preset the look is now. Outlined: the pad's and keys' pick.
    let active = active_preset();
    for (p, mut bg, mut border) in &mut presets {
        let want = if Some(p.0) == active { ACCENT_DIM } else { RAISED };
        if bg.0 != want {
            bg.0 = want;
        }
        let want = if e.row == 0 && p.0 == e.preset { INK } else { Color::NONE };
        if border.top != want {
            *border = BorderColor::all(want);
        }
    }
    // Help: the hovered line's, else the focused one's.
    let shown = hover.or(Some(e.row)).and_then(|i| list.get(i).copied());
    let want = match shown {
        Some(Line::Knob(i)) => {
            let def = &DEFS[i];
            let d = crate::knobs::default_of(def.key).unwrap_or(def.fallback);
            let default = if def.auto != Auto::No { "the level's own".to_owned() } else { format(def, d, true) };
            format!("{}: {}  (Default: {default}. Knob: {}.)", def.label, def.help, def.key)
        }
        Some(Line::Advanced) => "More settings for this tab. Every one starts at its default.".into(),
        Some(Line::Presets) | None => "A preset sets the time, sky, weather, fog and colour together. Then tweak anything and Save.".into(),
    };
    for mut h in &mut help {
        if h.0 != want {
            h.0 = want.clone();
        }
    }
    // Status: the level, its file and whether there are unsaved changes.
    if e.saved.as_ref().is_none_or(|s| s.0 != level.0) {
        let saved = crate::knobs::read(&settings, &level.0);
        e.saved = Some((level.0.clone(), saved.clone().unwrap_or_default(), saved.is_some()));
    }
    let want = match &e.saved {
        Some((_, saved, file)) => {
            let state = if *saved != crate::knobs::all() {
                "Unsaved changes"
            } else if *file {
                "Saved"
            } else {
                "No saved look yet (as the game made it)"
            };
            format!("Level: {}   |   {}", level.0, state)
        }
        None => format!("Level: {}", level.0),
    };
    for mut s in &mut status {
        if s.0 != want {
            s.0 = want.clone();
        }
    }
    let mut lines_out: Vec<String> = Vec::new();
    if crate::knobs::bypass() {
        lines_out.push("Comparing: the level without its look".into());
    }
    if let Some((m, until)) = e.message.clone() {
        if time.elapsed_secs() < until {
            lines_out.push(m);
        } else {
            e.message = None;
        }
    }
    let want = lines_out.join("\n");
    for mut n in &mut note {
        if n.0 != want {
            n.0 = want.clone();
        }
    }
}
