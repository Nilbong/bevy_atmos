//! Time of day and the sun. The host's sun (its [`crate::AtmosSun`]) is
//! taken as the level's own hour: its direction is fitted onto a day's sun
//! path, so setting the clock (`time.hour`) moves the sun along a path that
//! passes through where the host put it. With no time knob set the host's
//! sun stands as it was.
//!
//! Knobs: `time.hour` (-1: the host's sun), `time.speed` (game seconds per
//! real second), `time.latitude` (degrees), `sun.azimuth` and
//! `sun.elevation` (degrees; both set: the sun there, whatever the hour),
//! `sun.brightness`, `sun.warmth`.

use bevy::prelude::*;

pub struct TimePlugin;

impl Plugin for TimePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<TimeOfDay>().add_systems(Update, drive.in_set(crate::LookSet));
    }
}

/// The time of day and what it gives (read by the sky, air and weather).
#[derive(Resource, Clone, Debug)]
pub struct TimeOfDay {
    /// The clock runs or is set (a time knob is set): night and the moon
    /// apply. Off, the host's sun stands.
    pub clocked: bool,
    pub hours: f32,
    /// The level's own hour (the host's sun on the path).
    pub level_hour: f32,
    /// Toward the sun and moon (Bevy space).
    pub sun_dir: Vec3,
    pub moon_dir: Vec3,
    /// The sun's colour (brightest channel 1).
    pub sun_color: Vec3,
    /// 0 day .. 1 night.
    pub night: f32,
    /// The sun's light against the host's (1 at the level's hour).
    pub sun_share: f32,
}

impl Default for TimeOfDay {
    fn default() -> Self {
        TimeOfDay { clocked: false, hours: 12.0, level_hour: 12.0, sun_dir: Vec3::Y, moon_dir: Vec3::NEG_Y, sun_color: Vec3::ONE, night: 0.0, sun_share: 1.0 }
    }
}

fn smooth(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Toward the sun at `hours` on an equinox day at `latitude` (radians).
pub fn path_dir(hours: f32, latitude: f32) -> Vec3 {
    let h = (hours - 12.0) / 24.0 * std::f32::consts::TAU;
    let (sl, cl) = latitude.sin_cos();
    Vec3::new(-h.sin(), cl * h.cos(), sl * h.cos()).normalize()
}

/// The hour (morning side) at which the path's sun is as high as `dir`, and
/// the turn about the vertical that puts it at `dir` then.
pub fn fit_path(dir: Vec3, latitude: f32) -> (f32, f32) {
    let target = dir.y.clamp(-1.0, 1.0);
    let (mut lo, mut hi) = (6.0f32, 12.0f32);
    for _ in 0..30 {
        let mid = 0.5 * (lo + hi);
        if path_dir(mid, latitude).y < target {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    let hour = 0.5 * (lo + hi);
    let p = path_dir(hour, latitude);
    (hour, f32::atan2(dir.x, dir.z) - f32::atan2(p.x, p.z))
}

/// Compass bearing (0 = -Z, clockwise toward +X) and elevation in degrees,
/// to a direction toward the sun.
pub fn angles_to_dir(bearing: f32, elevation: f32) -> Vec3 {
    let (b, e) = (bearing.to_radians(), elevation.clamp(0.5, 90.0).to_radians());
    Vec3::new(b.sin() * e.cos(), e.sin(), -b.cos() * e.cos()).normalize()
}

pub fn dir_to_angles(d: Vec3) -> (f32, f32) {
    (d.x.atan2(-d.z).to_degrees().rem_euclid(360.0), d.y.clamp(-1.0, 1.0).asin().to_degrees())
}

/// The host's sun as first seen.
#[derive(Component, Clone, Copy)]
pub struct HostSun {
    pub dir: Vec3,
    pub color: LinearRgba,
    pub illuminance: f32,
}

#[allow(clippy::type_complexity)]
fn drive(
    mut commands: Commands,
    time: Res<Time>,
    mut tod: ResMut<TimeOfDay>,
    mut suns: Query<(Entity, &mut DirectionalLight, &mut Transform, Option<&HostSun>), With<crate::AtmosSun>>,
    mut ambient: Option<ResMut<GlobalAmbientLight>>,
    mut ambient_base: Local<Option<f32>>,
) {
    let Ok((e, mut light, mut tf, host)) = suns.single_mut() else { return };
    let host = match host {
        Some(h) => *h,
        None => {
            let h = HostSun { dir: -tf.forward().as_vec3(), color: light.color.to_linear(), illuminance: light.illuminance };
            commands.entity(e).insert(h);
            h
        }
    };
    let latitude = crate::knobs::get("time.latitude", 35.0).clamp(-80.0, 80.0).to_radians();
    let (level_hour, turn) = fit_path(host.dir, latitude);
    tod.level_hour = level_hour;
    let hour_knob = crate::knobs::get("time.hour", -1.0);
    let speed = crate::knobs::get("time.speed", 0.0);
    tod.clocked = hour_knob >= 0.0 || speed != 0.0;
    if hour_knob >= 0.0 {
        tod.hours = hour_knob.rem_euclid(24.0);
    } else if speed != 0.0 {
        tod.hours = (tod.hours + time.delta_secs() * speed / 3600.0).rem_euclid(24.0);
    } else {
        tod.hours = level_hour;
    }
    let on_path = |h: f32| Quat::from_rotation_y(turn) * path_dir(h, latitude);
    let mut sun = if tod.clocked { on_path(tod.hours) } else { host.dir };
    let (az, el) = (crate::knobs::get("sun.azimuth", -1.0), crate::knobs::get("sun.elevation", -1.0));
    if az >= 0.0 && el >= 0.0 {
        sun = angles_to_dir(az, el);
    }
    let moon = {
        let m = Quat::from_rotation_y(turn + 0.3) * path_dir(tod.hours + 12.0, latitude);
        Vec3::new(m.x, m.y + 0.1, m.z).normalize()
    };
    tod.night = if tod.clocked { 1.0 - smooth((sun.y + 0.1) / 0.15) } else { 0.0 };
    // Redder and dimmer low down (air mass), as bright as the host's at its hour.
    let m = 1.0 / sun.y.max(0.035);
    let red = Vec3::new((-0.03 * m).exp(), (-0.07 * m).exp(), (-0.16 * m).exp());
    let host_rgb = Vec3::new(host.color.red, host.color.green, host.color.blue);
    let mut color = if tod.clocked { host_rgb * red } else { host_rgb };
    color /= color.max_element().max(1e-3);
    let at_level = host.dir.y.max(0.05).sqrt();
    tod.sun_share = if tod.clocked { smooth((sun.y + 0.02) / 0.19) * (sun.y.max(0.0).sqrt() / at_level).min(1.3) } else { 1.0 };
    let warmth = crate::knobs::get("sun.warmth", 0.0).clamp(-1.0, 1.0);
    color *= Vec3::new(1.0 + 0.3 * warmth, 1.0, 1.0 - 0.3 * warmth);
    let brightness = crate::knobs::get("sun.brightness", 1.0).clamp(0.0, 10.0);
    // By night the moon lights the level (a tenth of the sun, blue).
    let (key, key_color, lux) = if tod.night > 0.5 && tod.clocked {
        (moon, Vec3::new(0.62, 0.72, 1.0), host.illuminance * 0.1 * tod.night * smooth(moon.y / 0.1))
    } else {
        (sun, color, host.illuminance * tod.sun_share)
    };
    tod.sun_dir = sun;
    tod.moon_dir = moon;
    tod.sun_color = color;
    let lux = lux * brightness;
    if (light.illuminance - lux).abs() > 1e-3 {
        light.illuminance = lux;
    }
    let c = Color::linear_rgb(key_color.x, key_color.y, key_color.z);
    if light.color != c {
        light.color = c;
    }
    let up = if key.y.abs() > 0.99 { Vec3::Z } else { Vec3::Y };
    let rot = Transform::default().looking_to(-Vec3::new(key.x, key.y.max(0.02), key.z).normalize(), up).rotation;
    if tf.rotation != rot {
        tf.rotation = rot;
    }
    // The sky's light with the hour.
    if let Some(a) = ambient.as_mut() {
        let base = *ambient_base.get_or_insert(a.brightness);
        let share = if tod.clocked { (0.25 + 0.75 * (1.0 - tod.night)) * (0.4 + 0.6 * smooth((sun.y + 0.02) / 0.19)) } else { 1.0 };
        let want = base * share * crate::knobs::get("sky.light", 1.0).clamp(0.0, 10.0);
        if (a.brightness - want).abs() > 1e-3 {
            a.brightness = want;
        }
    }
}
