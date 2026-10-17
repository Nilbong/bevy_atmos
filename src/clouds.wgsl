// bevy_atmos's screen-space clouds (`clouds.rs`): per output pixel, a
// ray from the camera through a cloud layer over a round earth, marched
// through tiling noise (Perlin-Worley shapes, curl-swirled Worley detail
// eroding the edges, a coverage map, height profiles by cloud type), lit
// by light marched toward the key light (Beer-Lambert, powder, two-lobed
// phase, multiple-scattering octaves) and by the sky round them. Jittered
// each frame and blended with last frame's, reprojected.
//
// Out: rgb the clouds' light (the sky cube's units, premultiplied), a the
// transmittance (1: clear sky).

struct CloudParams {
    world_from_clip: mat4x4<f32>,
    prev_clip_from_world: mat4x4<f32>,
    // xyz: the camera; w: the frame number.
    camera: vec4<f32>,
    // xyz: toward the key light; w: night.
    key_dir: vec4<f32>,
    // rgb: the key light's colour; w: darkness.
    key_color: vec4<f32>,
    // xy: drift (m); z: cover; w: extra height of the tops (m).
    wind: vec4<f32>,
    // x: density, y: erosion, z: fog, w: physical sky.
    look: vec4<f32>,
    horizon: vec4<f32>,
    // xy: output size; z: history usable.
    size: vec4<f32>,
    grade: vec4<f32>,
    // xyz: toward a lightning bolt; w: its flash now.
    flash: vec4<f32>,
}

@group(0) @binding(0) var<uniform> cp: CloudParams;
@group(0) @binding(1) var base_noise: texture_3d<f32>;
@group(0) @binding(2) var detail_noise: texture_3d<f32>;
@group(0) @binding(3) var curl_noise: texture_2d<f32>;
@group(0) @binding(4) var repeat_sampler: sampler;
@group(0) @binding(5) var history: texture_2d<f32>;
@group(0) @binding(6) var clamp_sampler: sampler;
@group(0) @binding(7) var output: texture_storage_2d<rgba16float, write>;
@group(0) @binding(8) var background: texture_cube<f32>;

const PI: f32 = 3.14159265;
const EARTH: f32 = 6360000.0;
const CLOUD_BASE: f32 = 1500.0;
const CLOUD_TOP: f32 = 4000.0;
// The noise's tiles (m): the shapes, the detail, the curl, the weather.
const BASE_TILE: f32 = 5500.0;
const DETAIL_TILE: f32 = 650.0;
const CURL_TILE: f32 = 3000.0;
const WEATHER_TILE: f32 = 18000.0;
// Extinction per metre at density 1.
const SIGMA: f32 = 0.035;
const STEPS: i32 = 80;
const LIGHT_STEPS: i32 = 6;
// Furthest a ray goes through the layer, and before it (m).
const MAX_SPAN: f32 = 35000.0;
const MAX_START: f32 = 80000.0;

fn remap(v: f32, lo: f32, hi: f32, nlo: f32, nhi: f32) -> f32 {
    return nlo + (v - lo) / max(hi - lo, 1e-5) * (nhi - nlo);
}

fn hg(c: f32, g: f32) -> f32 {
    let d = 1.0 + g * g - 2.0 * g * c;
    return (1.0 - g * g) / (4.0 * PI * d * sqrt(d));
}

fn top() -> f32 {
    return max(CLOUD_TOP + cp.wind.w, CLOUD_BASE + 600.0);
}

// Height through the layer (0 base .. 1 top) of `p` (from the earth's
// centre at the origin's foot: `p.y` up from the ground).
fn height_frac(p: vec3<f32>) -> f32 {
    let r = length(vec3(p.x, p.y + EARTH, p.z)) - EARTH;
    return (r - CLOUD_BASE) / (top() - CLOUD_BASE);
}

// The weather at `xz`: x coverage noise, y cloud type (0 stratus .. 1
// cumulus).
fn weather(xz: vec2<f32>) -> vec2<f32> {
    let w = textureSampleLevel(base_noise, repeat_sampler, vec3(xz / WEATHER_TILE, 0.31), 0.0);
    let w2 = textureSampleLevel(base_noise, repeat_sampler, vec3(xz / WEATHER_TILE * 0.7 + 0.5, 0.73), 0.0);
    return vec2(w.g * 0.7 + w.b * 0.3, w2.r);
}

// How much cloud a type has at height `h` (0..1).
fn profile(h: f32, kind: f32) -> f32 {
    let stratus = saturate(remap(h, 0.0, 0.08, 0.0, 1.0)) * saturate(remap(h, 0.15, 0.3, 1.0, 0.0));
    let strato = saturate(remap(h, 0.0, 0.12, 0.0, 1.0)) * saturate(remap(h, 0.35, 0.6, 1.0, 0.0));
    let cumulus = saturate(remap(h, 0.0, 0.1, 0.0, 1.0)) * saturate(remap(h, 0.45, 0.95, 1.0, 0.0));
    return mix(mix(stratus, strato, saturate(kind * 2.0)), cumulus, saturate(kind * 2.0 - 1.0));
}

// Cloud density at `p` (m, world, the ground at y 0); `cheap` skips the
// detail (the light march, empty space).
fn density(p: vec3<f32>, cheap: bool) -> f32 {
    let h = height_frac(p);
    if (h <= 0.0 || h >= 1.0) {
        return 0.0;
    }
    let q = p + vec3(cp.wind.x, 0.0, cp.wind.y);
    let w = weather(q.xz);
    let cover = cp.wind.z;
    // Coverage: more of the weather noise passes as the cover rises.
    // (The Worley weather noise bunches round 0.48, 0.36..0.6 for 80% of
    // it: spread over 0..1 first, so `cover` is about the share of the sky
    // covered.)
    let wn = saturate((w.x - 0.3) / 0.36);
    // (Ramping up fast past the threshold: a soft ramp left most covered
    // ground thin, and the shapes' own threshold below it fewer still.)
    let cov = saturate(remap(wn, 1.0 - cover * 1.2, 1.0 - cover * 1.2 + 0.25, 0.0, 1.0));
    if (cov <= 0.0) {
        return 0.0;
    }
    // More cumulus with more cover and in storms, flatter when sparse.
    let kind = saturate(w.y * 0.6 + 0.25 + cover * 0.35);
    // The tops lean downwind.
    let lean = q + vec3(1.0, 0.0, 0.6) * h * 400.0;
    let b = textureSampleLevel(base_noise, repeat_sampler, lean / BASE_TILE, 0.0);
    let low = b.g * 0.625 + b.b * 0.25 + b.a * 0.125;
    var c = saturate(remap(b.r, low - 1.0, 1.0, 0.0, 1.0));
    c *= profile(h, kind);
    c = saturate(remap(c, 1.0 - cov * mix(0.75, 1.0, cover), 1.0, 0.0, 1.0)) * mix(0.6, 1.0, cov);
    if (cheap || c <= 0.0) {
        return c * cp.look.x;
    }
    // Detail: swirled by the curl noise, wispy underneath and billowy on
    // top, eating into the thin edges (the cores stay).
    let curl = textureSampleLevel(curl_noise, repeat_sampler, q.xz / CURL_TILE, 0.0).xy * 2.0 - 1.0;
    let qd = q + vec3(curl.x, 0.0, curl.y) * (1.0 - h) * 220.0;
    let d = textureSampleLevel(detail_noise, repeat_sampler, qd / DETAIL_TILE, 0.0).rgb;
    let dfbm = d.r * 0.625 + d.g * 0.25 + d.b * 0.125;
    let modifier = mix(dfbm, 1.0 - dfbm, saturate(h * 6.0));
    c = saturate(remap(c, modifier * 0.45 * cp.look.y, 1.0, 0.0, 1.0));
    // (A steep ramp: dense cores, crisp edges, not a haze round them.)
    return saturate(c * 3.0) * cp.look.x;
}

// Where the ray from height `cam_h` along `d` is in the layer: (start,
// end), end <= start for none.
fn sphere(cam_h: f32, d: vec3<f32>, r: f32) -> vec2<f32> {
    let o = vec3(0.0, EARTH + cam_h, 0.0);
    let b = dot(o, d);
    let c = dot(o, o) - r * r;
    let disc = b * b - c;
    if (disc < 0.0) {
        return vec2(-1.0, -1.0);
    }
    let s = sqrt(disc);
    return vec2(-b - s, -b + s);
}

fn layer(cam_h: f32, d: vec3<f32>) -> vec2<f32> {
    let rb = EARTH + CLOUD_BASE;
    let rt = EARTH + top();
    let ib = sphere(cam_h, d, rb);
    let it = sphere(cam_h, d, rt);
    let ground = sphere(cam_h, d, EARTH);
    var t0 = 0.0;
    var t1 = 0.0;
    if (cam_h < CLOUD_BASE) {
        // Below: from the base up through the top (none toward the ground).
        if (ground.x > 0.0) {
            return vec2(0.0, -1.0);
        }
        t0 = ib.y;
        t1 = it.y;
    } else if (cam_h < top()) {
        // Inside: to the top, or down to the base.
        t0 = 0.0;
        t1 = select(it.y, ib.x, ib.x > 0.0);
    } else {
        // Above: down from the top to the base (or out the far side).
        if (it.x < 0.0) {
            return vec2(0.0, -1.0);
        }
        t0 = it.x;
        t1 = select(it.y, ib.x, ib.x > 0.0);
    }
    t0 = max(t0, 0.0);
    t1 = min(t1, t0 + MAX_SPAN);
    if (t0 > MAX_START) {
        return vec2(0.0, -1.0);
    }
    return vec2(t0, t1);
}

// Interleaved gradient noise, turning each frame.
fn jitter(px: vec2<f32>, frame: f32) -> f32 {
    let p = px + vec2(5.588238, 5.588238) * (frame % 64.0);
    return fract(52.9829189 * fract(dot(p, vec2(0.06711056, 0.00583715))));
}

fn background_at(d: vec3<f32>) -> vec3<f32> {
    return textureSampleLevel(background, clamp_sampler, d, 0.0).rgb;
}

@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<u32>(cp.size.xy);
    if (gid.x >= size.x || gid.y >= size.y) {
        return;
    }
    let px = vec2<f32>(gid.xy) + 0.5;
    let uv = px / cp.size.xy;
    let ndc = vec2(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
    let far = cp.world_from_clip * vec4(ndc, 1e-4, 1.0);
    let near = cp.world_from_clip * vec4(ndc, 1.0, 1.0);
    let d = normalize(far.xyz / far.w - near.xyz / near.w);
    // Last frame's, where the same direction was then (the clouds are far:
    // a direction is enough).
    var old = vec4(0.0);
    var has_old = false;
    if (cp.size.z > 0.5) {
        let prev = cp.prev_clip_from_world * vec4(d, 0.0);
        if (prev.w > 1e-5) {
            let puv = vec2(prev.x / prev.w * 0.5 + 0.5, 0.5 - prev.y / prev.w * 0.5);
            if (all(puv > vec2(0.0)) && all(puv < vec2(1.0))) {
                old = textureSampleLevel(history, clamp_sampler, puv, 0.0);
                has_old = true;
            }
        }
    }
    // A quarter of the pixels march each frame (a 2x2 pattern in turn);
    // the rest carry last frame's over.
    let turn = (gid.x % 2u) + (gid.y % 2u) * 2u;
    if (has_old && turn != u32(cp.camera.w) % 4u) {
        textureStore(output, vec2<i32>(gid.xy), old);
        return;
    }
    let cam = cp.camera.xyz;
    let cam_h = max(cam.y, 0.0);
    let span = layer(cam_h, d);

    var color = vec3(0.0);
    var trans = 1.0;
    if (span.y > span.x) {
        let key = normalize(cp.key_dir.xyz);
        let cs = dot(d, key);
        // Silver lining forward, some back-scatter, an even part.
        let phase = mix(mix(hg(cs, 0.8), hg(cs, -0.3), 0.3), 1.0 / (4.0 * PI), 0.25);
        let darkness = cp.key_color.w;
        // (A storm: the key light veiled, the bases darker.)
        let storm = cp.horizon.w;
        let key_light = cp.key_color.rgb * mix(7.0, 3.0, darkness) * (1.0 - 0.35 * storm);
        // Where the bolt leaves the cloud base: lightning lights the cloud
        // round it from inside.
        let bolt_dir = normalize(cp.flash.xyz);
        let bolt_at = vec3(cam.x, max(cam.y, 0.0), cam.z) + bolt_dir * (CLOUD_BASE + 300.0) / max(bolt_dir.y, 0.12);
        // The sky round the clouds: overhead lights their tops, the
        // horizon their sides, darker underneath.
        let zenith = background_at(vec3(0.0, 1.0, 0.0));
        let zenith_lum = dot(zenith, vec3(0.2126, 0.7152, 0.0722));
        let around = (background_at(vec3(1.0, 0.15, 0.0)) + background_at(vec3(-1.0, 0.15, 0.0)) + background_at(vec3(0.0, 0.15, 1.0)) + background_at(vec3(0.0, 0.15, -1.0))) * 0.25;
        // A dark sky's sun is low or veiled (dusk, a storm, the night's
        // moon): its light on the clouds as weak as the sky is dark.
        let key_lit = key_light * clamp(zenith_lum / 0.35, 0.12, 1.0);
        let steps = STEPS;
        let dt = (span.y - span.x) / f32(steps);
        let j = jitter(px, cp.camera.w);
        let origin = vec3(cam.x, cam_h, cam.z);
        var t_sum = 0.0;
        var weight = 0.0;
        for (var i = 0; i < steps; i++) {
            if (trans < 0.01) {
                break;
            }
            let t = span.x + (f32(i) + j) * dt;
            let p = origin + d * t;
            if (density(p, true) <= 0.0) {
                continue;
            }
            let dens = density(p, false);
            if (dens <= 0.0) {
                continue;
            }
            let h = saturate(height_frac(p));
            // Toward the key light: the optical depth of the cloud there.
            var od = 0.0;
            var lstep = 50.0;
            var lt = 0.0;
            for (var k = 0; k < LIGHT_STEPS; k++) {
                lt += lstep * (0.5 + 0.5 * j);
                od += density(p + key * lt, k > 1) * lstep;
                lt += lstep * (0.5 - 0.5 * j);
                lstep *= 1.7;
            }
            // (Light inside a cloud scatters many times: it gets further
            // than its extinction says, so tops glow and only deep bases
            // go dark.)
            od *= SIGMA * 0.35;
            // Beer-Lambert with the powder term (dark where light enters
            // a thick body edge-on), and octaves for light scattered many
            // times (brighter, softer).
            let powder = mix(1.0 - exp(-od * 2.0 - dens * 0.5), 1.0, 0.5 + 0.5 * cs);
            var energy = 0.0;
            var a = 1.0;
            var b = 1.0;
            var c = 1.0;
            for (var o = 0; o < 3; o++) {
                energy += a * exp(-od * b) * mix(1.0 / (4.0 * PI), phase, c);
                a *= 0.6;
                b *= 0.3;
                c *= 0.5;
            }
            // (By night the clouds keep a faint glow of their own, the
            // city's and the stars', so a night deck shows its masses.)
            let night_glow = vec3(0.42, 0.46, 0.55) * 0.55 * 0.45 * cp.key_dir.w * mix(0.6, 1.0, h);
            let ambient = mix(around * 0.3, zenith * 0.6 + around * 0.2, h) * (1.0 - 0.55 * darkness) * (1.0 - 0.45 * storm * (1.0 - h)) + night_glow;
            let lightning = vec3(0.8, 0.85, 1.0) * cp.flash.w * 9.0 * exp(-distance(p, bolt_at) / 1600.0) * max(dot(zenith, vec3(0.2126, 0.7152, 0.0722)), 0.05);
            let light = key_lit * energy * powder * 2.5 + ambient + lightning;
            let sigma = SIGMA * dens;
            let step_t = exp(-sigma * dt);
            // (Energy-conserving: the light scattered over the step.)
            color += trans * light * (1.0 - step_t);
            t_sum += t * trans * (1.0 - step_t);
            weight += trans * (1.0 - step_t);
            trans *= step_t;
        }
        // Far clouds fade into the horizon's haze, and toward the horizon.
        if (weight > 0.0) {
            let dist = t_sum / weight;
            let haze = 1.0 - exp(-dist / 45000.0);
            color = mix(color, around * (1.0 - trans), haze * 0.85);
        }
        if (cam_h < CLOUD_BASE) {
            let fade = smoothstep(0.0, 0.035, d.y);
            color *= fade;
            trans = mix(1.0, trans, fade);
        }
        color *= cp.grade.rgb;
    }
    var now = vec4(color, trans);
    if (has_old) {
        now = mix(old, now, 0.35);
    }
    textureStore(output, vec2<i32>(gid.xy), now);
}
