//! Knobs: every number the look uses, by name. [`get`] returns the level's
//! look value if it sets one, else the caller's default. The editor sets
//! them ([`set`]); a level's look is a text file of `name = value` lines
//! ([`save`], loaded as [`crate::AtmosLevel`] changes).
//!
//! The store is global (a lock), so any system, or a render-side helper,
//! can read a knob without a resource.

use bevy::prelude::*;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, RwLock};

pub struct KnobsPlugin;

impl Plugin for KnobsPlugin {
    fn build(&self, app: &mut App) {
        // (At startup, before the game's own Startup systems may set knobs;
        // then whenever the level changes.)
        app.init_resource::<LoadedLevel>().add_systems(PreStartup, load_on_level_change).add_systems(PreUpdate, load_on_level_change);
    }
}

#[derive(Default)]
struct Store {
    /// The level's look.
    look: BTreeMap<String, f32>,
    /// Defaults as last asked for (the editor shows them when unset).
    defaults: BTreeMap<String, f32>,
}

static STORE: LazyLock<RwLock<Store>> = LazyLock::new(|| RwLock::new(Store::default()));
static BYPASS: AtomicBool = AtomicBool::new(false);

/// Knob `name`: the look's value, or `default`.
pub fn get(name: &str, default: f32) -> f32 {
    if let Ok(s) = STORE.read() {
        if !BYPASS.load(Ordering::Relaxed) {
            if let Some(v) = s.look.get(name) {
                return *v;
            }
        }
        if s.defaults.get(name) == Some(&default) {
            return default;
        }
    }
    if let Ok(mut s) = STORE.write() {
        s.defaults.insert(name.to_owned(), default);
    }
    default
}

/// Whether the look sets knob `name`.
pub fn is_set(name: &str) -> bool {
    !BYPASS.load(Ordering::Relaxed) && STORE.read().is_ok_and(|s| s.look.contains_key(name))
}

/// Sets knob `name` (`None` unsets it: back to its default).
pub fn set(name: &str, value: Option<f32>) {
    if let Ok(mut s) = STORE.write() {
        match value.filter(|v| v.is_finite()) {
            Some(v) => {
                s.look.insert(name.to_owned(), v);
            }
            None => {
                s.look.remove(name);
            }
        }
    }
}

/// The default knob `name` was last asked for with.
pub fn default_of(name: &str) -> Option<f32> {
    STORE.read().ok().and_then(|s| s.defaults.get(name).copied())
}

/// All set knobs.
pub fn all() -> BTreeMap<String, f32> {
    STORE.read().map(|s| s.look.clone()).unwrap_or_default()
}

/// Replaces the look.
pub fn replace(values: BTreeMap<String, f32>) {
    if let Ok(mut s) = STORE.write() {
        s.look = values.into_iter().filter(|(_, v)| v.is_finite()).collect();
    }
}

/// The editor's A/B: while on, every knob reads its default (the look as
/// the game had it without one).
pub fn set_bypass(on: bool) {
    BYPASS.store(on, Ordering::Relaxed);
}

pub fn bypass() -> bool {
    BYPASS.load(Ordering::Relaxed)
}

/// `name = value` lines (`#` comments).
pub fn parse(text: &str) -> BTreeMap<String, f32> {
    text.lines()
        .filter_map(|l| {
            let l = l.split('#').next()?.trim();
            let (k, v) = l.split_once('=')?;
            let v: f32 = v.trim().parse().ok()?;
            v.is_finite().then(|| (k.trim().to_owned(), v))
        })
        .collect()
}

/// The look as a file's text.
pub fn to_text(level: &str) -> String {
    let mut text = format!("# {level}'s look (bevy_atmos)\n");
    for (k, v) in all() {
        text.push_str(&format!("{k} = {v}\n"));
    }
    text
}

/// Saves the look as `<dir>/<level>.txt`.
pub fn save(settings: &crate::AtmosSettings, level: &str) -> std::io::Result<std::path::PathBuf> {
    std::fs::create_dir_all(&settings.save_dir)?;
    let path = settings.save_dir.join(format!("{level}.txt"));
    std::fs::write(&path, to_text(level))?;
    Ok(path)
}

/// The level's saved look: `<dir>/<level>.txt`, else a built-in look, else
/// `None` (it has none).
pub fn read(settings: &crate::AtmosSettings, level: &str) -> Option<BTreeMap<String, f32>> {
    let name = format!("{level}.txt");
    let text = std::fs::read_to_string(settings.save_dir.join(&name))
        .ok()
        .or_else(|| settings.builtin.iter().find(|(n, _)| *n == name).map(|(_, t)| (*t).to_owned()));
    text.as_deref().map(parse)
}

/// Loads `<dir>/<level>.txt` (else a built-in look, else nothing).
pub fn load(settings: &crate::AtmosSettings, level: &str) -> usize {
    let values = read(settings, level).unwrap_or_default();
    let n = values.len();
    replace(values);
    n
}

/// The level whose look is loaded.
#[derive(Resource, Default)]
struct LoadedLevel(Option<String>);

fn load_on_level_change(level: Res<crate::AtmosLevel>, settings: Res<crate::AtmosSettings>, mut last: ResMut<LoadedLevel>) {
    if last.0.as_deref() == Some(level.0.as_str()) {
        return;
    }
    last.0 = Some(level.0.clone());
    let n = load(&settings, &level.0);
    info!("atmos: {} knobs for {}", n, level.0);
}
