// bevy_atmos's modern water (`modern_water.rs`): Gerstner waves moving
// a fine grid, layered detail waves in the normals, screen-space
// reflections over the sky by Fresnel, the sun's glitter, colour by
// absorption through the water's depth (the depth prepass), foam on steep
// crests and where it's shallow, rain hitting it, and the surface from
// below. Premultiplied alpha: what's behind shows through by the light the
// water lets through.

#import bevy_pbr::{
    mesh_functions,
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing},
    forward_io::{Vertex, VertexOutput, FragmentOutput},
    mesh_view_bindings::{view, globals},
    view_transformations::{depth_ndc_to_view_z, position_world_to_clip},
}
#import bevy_atmos::ssr::screen_reflection
#ifdef DEPTH_PREPASS
#import bevy_pbr::prepass_utils::prepass_depth
#endif

// x: wave height (m), y: speed, z: choppiness, w: detail scale.
@group(#{MATERIAL_BIND_GROUP}) @binding(100) var<uniform> waves: vec4<f32>;
// xy: wave direction, z: rain ripples, w: 1 with the camera under water.
@group(#{MATERIAL_BIND_GROUP}) @binding(101) var<uniform> wind: vec4<f32>;
// x: sky reflection, y: glitter, z: Fresnel, w: the skybox's brightness.
@group(#{MATERIAL_BIND_GROUP}) @binding(102) var<uniform> light: vec4<f32>;
// x: foam, y: foam width (m), z: rivers' sediment (0 clear .. 1 silty).
@group(#{MATERIAL_BIND_GROUP}) @binding(103) var<uniform> body: vec4<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(104) var<uniform> scatter: vec4<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(105) var<uniform> absorb: vec4<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(106) var<uniform> sun_dir: vec4<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(107) var<uniform> sun_color: vec4<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(108) var sky: texture_cube<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(109) var sky_sampler: sampler;
// x: crest foam, y: foam threshold.
@group(#{MATERIAL_BIND_GROUP}) @binding(110) var<uniform> foam_look: vec4<f32>;
// Screen-space reflections: x strength (0 off), y roughness cutoff, z reach.
@group(#{MATERIAL_BIND_GROUP}) @binding(111) var<uniform> ssr: vec4<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(112) var history: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(113) var history_sampler: sampler;
// Rivers (`flow.rs`): x mode (0 still, 1 UV velocity, 2 world velocity,
// 3 flow map, 4 along a UV axis), yz the vector, w speed (m/s).
@group(#{MATERIAL_BIND_GROUP}) @binding(114) var<uniform> flow: vec4<f32>;
// x speed multiplier (0 off), y rapids speed (m/s), z trail (m), w
// direction override (radians, below 0 none).
@group(#{MATERIAL_BIND_GROUP}) @binding(115) var<uniform> flow_look: vec4<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(116) var flow_map: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(117) var flow_map_sampler: sampler;
// Painted flow: xy m/s, z weight, in world cells (`paint_info`: x cell
// size, y cells a side, z 1 if any).
@group(#{MATERIAL_BIND_GROUP}) @binding(118) var paint: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(119) var<uniform> paint_info: vec4<f32>;
// The host's own water colour (`WaterTint`): rgb linear, w 1 if given.
@group(#{MATERIAL_BIND_GROUP}) @binding(120) var<uniform> tint: vec4<f32>;
// The reflected sky's rotation (a quaternion: the camera's `Skybox`
// rotation when it's the host's sky; identity for the look's own).
@group(#{MATERIAL_BIND_GROUP}) @binding(121) var<uniform> sky_rot: vec4<f32>;
// x: the least depth (m) to colour and foam by (`WaterDepth`: a sheet of
// water just over the ground).
@group(#{MATERIAL_BIND_GROUP}) @binding(122) var<uniform> extra: vec4<f32>;
// x: wave height scale, y: wave length scale (the body's size), z: kind
// (1 puddle, 2 pond, 3 river, 4 sea, 0 unknown), w: 1 to fade by the
// vertex alpha.
@group(#{MATERIAL_BIND_GROUP}) @binding(123) var<uniform> kind: vec4<f32>;

fn puddle() -> bool {
    return abs(kind.z - 1.0) < 0.5;
}

const PI: f32 = 3.14159265;
const GERSTNER: i32 = 5;

fn hash2(p: vec2<f32>) -> f32 {
    var p3 = fract(vec3(p.xyx) * 0.1031);
    p3 += dot(p3, p3.yzx + 33.33);
    return fract((p3.x + p3.y) * p3.z);
}

fn noise2(p: vec2<f32>) -> f32 {
    let c = floor(p);
    let f = p - c;
    let u = f * f * (3.0 - 2.0 * f);
    return mix(mix(hash2(c), hash2(c + vec2(1.0, 0.0)), u.x), mix(hash2(c + vec2(0.0, 1.0)), hash2(c + vec2(1.0, 1.0)), u.x), u.y);
}

// --- foam: a bubble lattice
//
// Worley noise's F2 - F1 is small along the borders between cells: the
// walls between bubbles. Three octaves of it (about 1, 4 and 16 cells a
// metre) summed into a "foaminess" that's high on the walls; the foam's
// coverage sets the threshold, so little coverage leaves thin bright
// filaments along the biggest walls and full coverage packs it dense with
// only the smallest bubbles' holes dark. Octaves too fine for the pixel
// fade to their average (no shimmer).

fn foam_hash2(c: vec2<f32>) -> vec2<f32> {
    var p3 = fract(vec3(c.xyx) * vec3(0.1031, 0.1030, 0.0973));
    p3 += dot(p3, p3.yzx + 33.33);
    return fract((p3.xx + p3.yz) * p3.zy);
}

// (F1, F2) of `p` in cells; the bubbles' centres drift slowly with `t`.
fn worley2(p: vec2<f32>, t: f32) -> vec2<f32> {
    let c = floor(p);
    let f = p - c;
    var f1 = 8.0;
    var f2 = 8.0;
    for (var y = -1; y <= 1; y++) {
        for (var x = -1; x <= 1; x++) {
            let o = vec2(f32(x), f32(y));
            let h = foam_hash2(c + o);
            let centre = o + 0.5 + 0.38 * sin(t * (0.4 + 0.5 * h.yx) + h * 6.2831853);
            let d = length(centre - f);
            if (d < f1) {
                f2 = f1;
                f1 = d;
            } else if (d < f2) {
                f2 = d;
            }
        }
    }
    return vec2(f1, f2);
}

// How much the bubble walls are there (1 on a wall, 0 mid-bubble).
fn walls(p: vec2<f32>, t: f32, width: f32) -> f32 {
    let w = worley2(p, t);
    return 1.0 - smoothstep(0.0, width, w.y - w.x);
}

// Foam at `p` (m) for `coverage` 0..1; `footprint`: metres a pixel.
fn foam_tex(p: vec2<f32>, coverage: f32, footprint: f32, t: f32) -> f32 {
    if (coverage <= 0.002) {
        return 0.0;
    }
    // (Walls thicken as the foam packs denser.)
    let width = mix(0.08, 0.3, coverage);
    // (Warped, so the walls curve instead of running in straight lines.)
    let warp = vec2(noise2(p * 0.9 + vec2(3.1, 7.7)), noise2(p * 0.9 + vec2(11.3, 1.9))) - 0.5;
    let pw = p + warp * 0.9;
    let w1 = walls(pw * 1.0, t * 0.7, width);
    // (Finer octaves fade to their average as they shrink below a pixel.)
    let s2 = smoothstep(0.5, 0.15, footprint * 4.0);
    let s3 = smoothstep(0.5, 0.15, footprint * 16.0);
    var w2 = 0.35;
    var w3 = 0.35;
    if (s2 > 0.0) {
        w2 = mix(0.35, walls(pw * 4.0 + vec2(17.3, 5.1) + warp * 1.5, t * 1.1, width), s2);
    }
    if (s3 > 0.0) {
        w3 = mix(0.35, walls(pw * 16.0 + vec2(41.7, 23.9), t * 1.7, width * 1.2), s3);
    }
    // (Irregular, not a net: how much each size of bubble shows varies
    // across the foam, and the big walls break into gaps.)
    let mixw = noise2(p * 0.23 + vec2(13.1, 4.7));
    let gaps = smoothstep(0.32, 0.58, noise2(p * 1.9 + vec2(2.3, 9.1) + t * 0.05));
    let field = w1 * mix(0.15, 0.5, mixw) * gaps + w2 * 0.36 + w3 * mix(0.45, 0.18, mixw);
    // Coverage sets the threshold: sparse filaments .. packed foam (only a
    // few small holes left when full).
    let threshold = mix(0.62, -0.06, coverage);
    // (Thin foam's edges softer: it dissolves rather than ending in a line.)
    let soft = 0.04 + (1.0 - coverage) * 0.08 + min(footprint * 2.0, 0.2);
    let amount = smoothstep(threshold - soft, threshold + soft * 2.0, field);
    // Translucent where thin, opaque where packed.
    return amount * mix(0.5, 1.0, saturate(field * 1.4 + coverage * 0.4)) * smoothstep(0.0, 0.25, coverage);
}

fn wind_dir() -> vec2<f32> {
    return normalize(wind.xy + vec2(1e-5, 0.0));
}

// The Gerstner waves: wave `i`'s direction (xy), length (m) and height (m).
fn wave(i: i32) -> vec4<f32> {
    let w = wind_dir();
    var spreads = array<f32, 5>(0.0, 0.55, -0.42, 0.95, -0.8);
    var lengths = array<f32, 5>(16.0, 9.7, 6.1, 3.8, 2.3);
    var heights = array<f32, 5>(0.5, 0.3, 0.18, 0.1, 0.055);
    let spread = spreads[i];
    let d = vec2(w.x * cos(spread) - w.y * sin(spread), w.x * sin(spread) + w.y * cos(spread));
    // (Scaled to the body: a pond's short low waves, the sea's swells.)
    return vec4(d, lengths[i] * kind.y, waves.x * heights[i] * kind.x);
}

// Where the surface point that started at `p` (xz) is moved to.
fn gerstner_offset(p: vec2<f32>, t: f32) -> vec3<f32> {
    var o = vec3(0.0);
    for (var i = 0; i < GERSTNER; i++) {
        let w = wave(i);
        let k = 2.0 * PI / w.z;
        let omega = sqrt(9.81 * k) * waves.y;
        // Steepness (choppiness), shared so the crests never loop over,
        // and only as steep as the wave is high (calm water stays round).
        let q = waves.z * min(k * w.w / 0.12, 1.0) / max(k * w.w * f32(GERSTNER), 1e-4);
        let x = k * dot(w.xy, p) - omega * t + f32(i) * 1.93;
        o += vec3(w.x * q * w.w * cos(x), w.w * sin(x), w.y * q * w.w * cos(x));
    }
    return o;
}

// The surface's normal there (xyz) and how much it's squeezed (w: below 1
// on sharp crests, where foam forms).
fn gerstner_normal(p: vec2<f32>, t: f32, footprint: f32) -> vec4<f32> {
    var n = vec3(0.0, 1.0, 0.0);
    var squeeze = 1.0;
    for (var i = 0; i < GERSTNER; i++) {
        let w = wave(i);
        let k = 2.0 * PI / w.z;
        let omega = sqrt(9.81 * k) * waves.y;
        let q = waves.z * min(k * w.w / 0.12, 1.0) / max(k * w.w * f32(GERSTNER), 1e-4);
        let x = k * dot(w.xy, p) - omega * t + f32(i) * 1.93;
        // (Waves shorter than a few pixels fade: they aliased into bands.
        // Their crests are sharper than the wave, by the steepness: by
        // that too, or they striped a river seen along its length.)
        let wa = k * w.w * smoothstep(w.z * 0.5, w.z * 0.12, footprint * (1.5 + 6.0 * q * k * w.w));
        n -= vec3(w.x * wa * cos(x), q * wa * sin(x), w.y * wa * cos(x));
        squeeze -= q * wa * sin(x) * 1.6;
    }
    return vec4(normalize(n), squeeze);
}

// Small waves in the normals only (dh/dx, dh/dz): spread round the wind,
// shorter by the detail scale, fading out where they'd be below a pixel.
fn detail(p: vec2<f32>, t: f32, footprint: f32) -> vec2<f32> {
    let w = wind_dir();
    let sharp = 1.0 + 3.0 * waves.z;
    var g = vec2(0.0);
    var wl = 2.2 / waves.w;
    for (var i = 0; i < 10; i++) {
        // (A river's ripples every way: round the wind they lined up in
        // parallel bands down its length.)
        let spread = (hash2(vec2(f32(i), 3.7)) - 0.5) * select(2.2, 5.5, river());
        let d = vec2(w.x * cos(spread) - w.y * sin(spread), w.x * sin(spread) + w.y * cos(spread));
        let k = 2.0 * PI / wl;
        let omega = sqrt(9.81 * k) * waves.y;
        // (Calm water still has fine ripples; rough water more.)
        // (A puddle lies all but still.)
        // (The sharpened crests are finer than the wave: faded by that.)
        let a = (0.012 + 0.02 * waves.x) * select(1.0, 0.06, puddle()) * wl * smoothstep(wl * 0.5, wl * 0.12, footprint * (0.5 + 0.6 * sharp));
        let x = k * dot(d, p) - omega * t + hash2(vec2(f32(i), 9.1)) * 6.28;
        let e = exp(sharp * (sin(x) - 1.0));
        g += d * (a * k * cos(x) * e);
        wl *= 0.74;
    }
    return g;
}

// Rain hitting the water: for each drop a bright speck and a dimple where
// it lands, then a ring spreading out (xy: the slope it adds, z: the
// splash's whiteness).
fn rain(p: vec2<f32>, t: f32) -> vec3<f32> {
    var g = vec3(0.0);
    for (var layer = 0; layer < 3; layer++) {
        let q = p * (2.6 + f32(layer) * 1.9) + f32(layer) * 7.3;
        let c = floor(q);
        let phase = fract(t * (1.1 + 0.3 * f32(layer)) + hash2(c));
        let centre = c + vec2(hash2(c + 3.1), hash2(c + 5.7)) * 0.8 + 0.1;
        let d = q - centre;
        let r = length(d);
        // The ring: a few crests travelling out, dying away.
        let front = phase * 0.55;
        let ring = sin((r - front) * 45.0) * smoothstep(0.1, 0.0, abs(r - front)) * (1.0 - phase) * (1.0 - phase);
        g += vec3(d / max(r, 1e-3) * ring * 1.6, 0.0);
        // The splash: for the first moments.
        let splash = smoothstep(0.12, 0.0, phase) * smoothstep(0.06, 0.0, r);
        g += vec3(-d / max(r, 1e-3) * splash * 2.0, splash);
    }
    return g;
}

// Drawn on the original mesh (rivers, `OwnMesh` water): its shape, its
// normals, its UVs; no Gerstner displacement.
fn on_mesh() -> bool {
    return flow.x > 0.5;
}

// Flowing (a river; mode 5 is still water on its own mesh).
fn river() -> bool {
    return flow.x > 0.5 && abs(flow.x - 5.0) > 0.5;
}

@vertex
fn vertex(vertex: Vertex) -> VertexOutput {
    var out: VertexOutput;
    let world_from_local = mesh_functions::get_world_from_local(vertex.instance_index);
    var wp = mesh_functions::mesh_position_local_to_world(world_from_local, vec4(vertex.position, 1.0));
    let base = wp.xz;
    out.world_normal = vec3(0.0, 1.0, 0.0);
    if (on_mesh()) {
        // Its own mesh as it is (sloped), its own UVs (a river's flow).
#ifdef VERTEX_NORMALS
        out.world_normal = mesh_functions::mesh_normal_local_to_world(vertex.normal, vertex.instance_index);
#endif
#ifdef VERTEX_UVS_A
        out.uv = vertex.uv;
#endif
    } else {
        // (The waves die down toward the grid's edge, so it meets the shore.)
        var edge = 1.0;
#ifdef VERTEX_UVS_A
        let f = min(vertex.uv, 1.0 - vertex.uv);
        edge = smoothstep(0.0, 0.08, min(f.x, f.y));
        // (The point's place before the waves moved it: its normal's there.)
        out.uv = base;
#endif
        wp = vec4(wp.xyz + gerstner_offset(base, globals.time) * edge, 1.0);
    }
    out.world_position = wp;
    out.position = position_world_to_clip(wp.xyz);
#ifdef VERTEX_COLORS
    // (A host's water may fade its edges with the vertex colour's alpha.)
    out.color = vertex.color;
#endif
#ifdef VERTEX_OUTPUT_INSTANCE_INDEX
    out.instance_index = vertex.instance_index;
#endif
#ifdef VISIBILITY_RANGE_DITHER
    out.visibility_range_dither = mesh_functions::get_visibility_range_dither_level(vertex.instance_index, world_from_local[3]);
#endif
    return out;
}

fn sky_at(dir: vec3<f32>) -> vec3<f32> {
    // (As the skybox looks it up: turned by its rotation.)
    let q = sky_rot;
    let d = dir + 2.0 * cross(q.xyz, cross(q.xyz, dir) + q.w * dir);
    return textureSampleLevel(sky, sky_sampler, d, 0.0).rgb * light.w * view.exposure;
}

// The painted flow at `p` (xy m/s, z weight).
fn painted(p: vec3<f32>) -> vec3<f32> {
    if (paint_info.z < 0.5) {
        return vec3(0.0);
    }
    let n = i32(paint_info.y + 0.5);
    let c = vec2<i32>(floor(p.xz / paint_info.x));
    let s = ((c % vec2<i32>(n)) + vec2<i32>(n)) % vec2<i32>(n);
    return textureLoad(paint, s, 0).xyz;
}

// The scene's view depth at world point `q`'s pixel, and `q`'s own (both
// view z, negative ahead); x > y: something nearer than `q` there.
fn scene_vs(q: vec3<f32>) -> vec2<f32> {
#ifdef DEPTH_PREPASS
    let clip = view.clip_from_world * vec4(q, 1.0);
    if (clip.w <= 0.0) {
        return vec2(-1e9, 0.0);
    }
    let ndc = clip.xyz / clip.w;
    let uv = ndc.xy * vec2(0.5, -0.5) + 0.5;
    if (any(uv < vec2(0.0)) || any(uv > vec2(1.0))) {
        return vec2(-1e9, 0.0);
    }
    let sd = prepass_depth(vec4(uv * view.viewport.zw, 0.0, 0.0), 0u);
    if (sd <= 1e-7) {
        return vec2(-1e9, 0.0);
    }
    return vec2(depth_ndc_to_view_z(sd), depth_ndc_to_view_z(ndc.z));
#else
    return vec2(-1e9, 0.0);
#endif
}

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    var pbr_input = pbr_input_from_standard_material(in, is_front);
    let t = globals.time;
    let wp = in.world_position.xyz;
    let is_river = river();
    let own_mesh = on_mesh();
    var base = wp.xz;
    var uv = vec2(0.0);
#ifdef VERTEX_UVS_A
    uv = in.uv;
    if (!own_mesh) {
        base = in.uv;
    }
#endif
    // (Derivatives up here, in uniform control flow.)
    let footprint = length(fwidth(base));
    let dpx = dpdx(wp);
    let dpy = dpdy(wp);
    let dux = dpdx(uv);
    let duy = dpdy(uv);
    let to_eye = view.world_position - wp;
    let dist = length(to_eye);
    let v = to_eye / dist;
    // (Water faces up: a host mesh built facing down is turned over, or
    // it was drawn as if seen from below.)
    var gn = normalize(in.world_normal);
    if (gn.y < 0.0) {
        gn = -gn;
    }

    // How much water the eye looks through to what's behind, and how deep
    // the water is there (m).
    var ray = 1000.0;
    var depth = 1000.0;
#ifdef DEPTH_PREPASS
    let scene = prepass_depth(in.position, 0u);
    if scene > 1e-7 {
        let along = max(depth_ndc_to_view_z(in.position.z) - depth_ndc_to_view_z(scene), 0.0);
        ray = along * dist / max(-depth_ndc_to_view_z(in.position.z), 1e-3);
        depth = ray * max(abs(dot(v, gn)), 0.02);
    }
#endif

    // The river's flow here (world, m/s, along the surface).
    var fl = vec3(0.0);
    if (is_river) {
        // The surface's direction of increasing u and v (m per UV unit),
        // from the screen derivatives: UV-space flow to metres.
        let det = dux.x * duy.y - dux.y * duy.x;
        var dpdu = vec3(0.0);
        var dpdv = vec3(0.0);
        if (abs(det) > 1e-12) {
            dpdu = (dpx * duy.y - dpy * dux.y) / det;
            dpdv = (dpy * dux.x - dpx * duy.x) / det;
        }
        let mode = i32(flow.x + 0.5);
        if (mode == 1) {
            fl = dpdu * flow.y + dpdv * flow.z;
        } else if (mode == 2) {
            fl = vec3(flow.y, 0.0, flow.z);
        } else if (mode == 3) {
            let m = textureSampleLevel(flow_map, flow_map_sampler, uv, 0.0).rg * 2.0 - 1.0;
            let dir = dpdu * m.x + dpdv * m.y;
            fl = select(vec3(0.0), normalize(dir) * length(m) * flow.w, dot(dir, dir) > 1e-12);
        } else {
            let dir = dpdu * flow.y + dpdv * flow.z;
            fl = select(vec3(0.0), normalize(dir) * flow.w, dot(dir, dir) > 1e-12);
        }
        let pt = painted(wp);
        fl = mix(fl, vec3(pt.x, 0.0, pt.y), saturate(pt.z));
        if (flow_look.w >= 0.0) {
            fl = vec3(cos(flow_look.w), 0.0, sin(flow_look.w)) * length(fl);
        }
        // Slower in the shallows at the banks; the knob's multiplier.
        fl *= flow_look.x * sqrt(smoothstep(0.0, 0.9, depth));
    }
    let speed = length(fl.xz);
    let fdir = select(vec2(1.0, 0.0), fl.xz / max(speed, 1e-4), speed > 1e-4);

    // Foam trailing behind whatever crosses the surface upstream (seen on
    // screen), and calm water just behind it.
    var trail = 0.0;
    var calm = 0.0;
    if (is_river && speed > 0.05 && flow_look.z > 0.0) {
        let back = -normalize(vec3(fl.x, fl.y, fl.z));
        // (Jittered per pixel: fixed steps showed as bands behind a rock.)
        let jit = hash2(floor(in.position.xy) + fract(t) * 37.0);
        for (var k = 0; k < 8; k++) {
            let s = (f32(k) + jit) / 8.0;
            let q = wp + back * flow_look.z * s;
            let z = scene_vs(q);
            if (z.x > z.y && z.x - z.y < 1.5) {
                trail = max(trail, 1.0 - s);
                calm = max(calm, 1.0 - smoothstep(0.0, 0.35, s));
            }
        }
    }

    var g = vec4(0.0, 1.0, 0.0, 1.0);
    var fine = vec2(0.0);
    if (is_river) {
        // Flow-mapped ripples: two phases of the ripples carried along the
        // flow, each fading out as it would stretch, half a cycle apart.
        let cycle = 1.6;
        let ph0 = fract(t / cycle);
        let ph1 = fract(t / cycle + 0.5);
        let w0 = 1.0 - abs(2.0 * ph0 - 1.0);
        let f0 = detail(base - fl.xz * ph0 * cycle, t * 0.3, footprint);
        let f1 = detail(base - fl.xz * ph1 * cycle + vec2(3.7, 1.3), t * 0.3, footprint);
        let rough = (0.5 + speed * 0.35) * (1.0 - 0.7 * calm);
        fine = (f0 * w0 + f1 * (1.0 - w0)) * rough;
    } else {
        g = gerstner_normal(base, t, footprint);
        fine = detail(base, t, footprint);
    }
    let drops = rain(base, t) * wind.z * smoothstep(0.06, 0.015, footprint);
    var n = normalize(vec3(g.x - fine.x - drops.x * 0.1, g.y, g.z - fine.y - drops.y * 0.1));
    if (own_mesh) {
        // On its own (perhaps sloped) surface: the waves' slopes about its
        // own normal.
        let sg = -g.xz / max(g.y, 0.2);
        // (A host's low-poly water mesh has faceted normals: the glitter
        // showed its triangles. Near-flat water shades about straight up,
        // only a real slope keeps the mesh's.)
        let up = vec3(0.0, 1.0, 0.0);
        // (Only a steep fall keeps the mesh's: a host's split per-triangle
        // normals showed as facets in the glitter.)
        let shade_n = normalize(mix(up, gn, smoothstep(0.5, 0.3, gn.y)));
        n = normalize(shade_n + vec3(sg.x - fine.x - drops.x * 0.1, 0.0, sg.y - fine.y - drops.y * 0.1));
    }
    let below = wind.w > 0.5 || dot(v, gn) < 0.0;

    // The water's body lit by the scene's lights (shadows, ambient): its
    // own colour, no specular (the reflections are below).
    // (A river carries silt: its body leans to the colour of its banks'
    // earth and swallows blue first, not a pool's clean turquoise.)
    let sed = select(0.0, saturate(body.z), is_river);
    let silt = vec3(0.55, 0.45, 0.28) * dot(scatter.rgb, vec3(0.2126, 0.7152, 0.0722)) * 1.6;
    let body_rgb = mix(scatter.rgb, silt, sed);
    let absorb_rgb = absorb.rgb * mix(vec3(1.0), vec3(0.6, 0.9, 1.7), sed);
    pbr_input.material.base_color = vec4(body_rgb, 1.0);
    pbr_input.material.reflectance = vec3(0.0);
    pbr_input.material.perceptual_roughness = 1.0;
    pbr_input.N = gn;
    // (Plus the host's own light on it, `WaterAmbient`.)
    let lit = apply_pbr_lighting(pbr_input).rgb + body_rgb * extra.yzw * view.exposure;
    // The light falling on it, for the foam.
    let irradiance = lit.g / max(body_rgb.g, 1e-4);

    var r = reflect(-v, n);
    r = vec3(r.x, abs(r.y), r.z);
    let screen = screen_reflection(wp, r, 0.02, in.position.xy, ssr, history, history_sampler);

    var out: FragmentOutput;
    if below {
        // From below: the sky through Snell's window, else the water's
        // depths mirrored (total internal reflection).
        let nd = -n;
        let rr = refract(-v, nd, 1.33);
        var c = lit * 2.0;
        var a = 1.0;
        if dot(rr, rr) > 0.0 {
            let window = smoothstep(0.0, 0.25, -dot(rr, nd));
            c = mix(c, sky_at(rr) * 0.7, window);
            a = mix(1.0, 0.85, window);
        }
        out.color = main_pass_post_lighting_processing(pbr_input, vec4(c * a, a));
        return out;
    }

    // (Rivers carry silt: they take their colour in far less depth than a
    // still pool, so shallow river water isn't just clear over its bed.)
    let turbidity = select(1.0, 3.0, is_river);
    // (A sheet of water just over the ground: coloured as if deeper.)
    // (A puddle is a film: whatever depth the host gave its water, it
    // colours as a few centimetres, a thin dark tint over its ground.)
    let least = select(extra.x, min(extra.x, 0.05), puddle());
    let ray_c = max(ray, least);
    let depth_c = max(depth, least);
    let trans = exp(-absorb_rgb * turbidity * ray_c);
    // (The wet ground under a puddle darkens: some of it always covered.)
    let through = select(dot(trans, vec3(1.0 / 3.0)), min(dot(trans, vec3(1.0 / 3.0)), 0.35), puddle());

    let ndv = max(dot(n, v), 0.0);
    var fresnel = clamp(0.02 + 0.98 * pow(1.0 - ndv, 5.0 / light.z), 0.0, 1.0);
    // (A puddle: a thin mirror over its ground.)
    if (puddle()) {
        // (Clearly a mirror: a dimmer floor read as a faint smear on the
        // ground, AC1's Damascus puddles.)
        fresnel = max(fresnel, 0.4);
    }
    // (The water's edge fades in, not a hard reflecting line.)
    // (By the coloured depth: a host's sheet of water, a few centimetres
    // over the ground all over, reflects fully; its edge fades by its
    // vertex alpha instead.)
    let edge = smoothstep(0.0, 0.06, depth_c);
    // What's on screen where the reflection lands, over the sky.
    // (Where the screen has nothing (off its edge), a low reflected ray
    // would see walls and the ground's surroundings, not the bright sky:
    // the sky darkens toward the horizon there.)
    let surround = mix(0.35, 1.0, smoothstep(0.02, 0.45, r.y));
    let reflected = mix(sky_at(r) * surround, screen.rgb, saturate(screen.a)) * light.x;

    // The sun's (or moon's) glitter: a very sharp highlight on each facet.
    let l = normalize(sun_dir.xyz);
    let h = normalize(l + v);
    // (Broader where a pixel covers more water: tight highlights there
    // sparkled into facets.)
    let shine = mix(2400.0, 300.0, saturate(footprint * 8.0));
    let ndh = max(dot(n, h), 0.0);
    let d = (shine + 2.0) / (2.0 * PI) * pow(ndh, shine);
    let f_sun = 0.02 + 0.98 * pow(1.0 - max(dot(h, v), 0.0), 5.0);
    // (Capped: a bright sun on a mirror-flat facet would blow out to a
    // white patch; a few times white blooms as glitter.)
    let glint = min(sun_color.rgb * d * f_sun * 0.25 * light.y * view.exposure * step(0.0, l.y) * edge, vec3(6.0));

    // Foam: how much of it there is where (coverage 0..1): where what's
    // behind is close under the surface (by the ray, so a wall gets a thin
    // line, not a band), along shores and round anything through the
    // surface; on crests squeezed sharp enough (rough water); on rivers
    // where they run fast or steep and trailing behind things. Then the
    // bubble lattice at that coverage, carried with the surface (the
    // waves' own coordinates) or the river's flow (two phases, streaked
    // along it).
    // (Rivers and a host's own water sheets lie shallow over their beds:
    // their bank foam only right at the edge.)
    let shore_width = select(body.y, body.y * 0.3, own_mesh);
    let shallow = 1.0 - smoothstep(0.0, shore_width, min(ray_c, depth_c * 2.0));
    let shore = body.x * shallow * shallow;
    let onset = 1.0 - 0.6 * foam_look.y;
    let crest = foam_look.x * smoothstep(onset, onset - 0.3, g.w);
    var white = 0.0;
    if (is_river) {
        // (Steepness only where the water really falls: the banks of a
        // host's river mesh curve up, and turned whole banks white.)
        let steep = smoothstep(0.25, 0.6, 1.0 - saturate(gn.y));
        let fast = speed + steep * 6.0 * flow_look.x;
        white = smoothstep(flow_look.y * 0.7, flow_look.y * 1.5, fast);
        white = max(white, trail * trail * 0.9);
    }
    let coverage = saturate(max(max(shore, crest), white));
    var foam_amount = 0.0;
    if (is_river) {
        let cycle = 1.6;
        let ph0 = fract(t / cycle);
        let ph1 = fract(t / cycle + 0.5);
        let w0 = 1.0 - abs(2.0 * ph0 - 1.0);
        let side = vec2(-fdir.y, fdir.x);
        let q0 = base - fl.xz * ph0 * cycle;
        let q1 = base - fl.xz * ph1 * cycle + vec2(5.1, 2.3);
        // (Streaked along the flow: stretched that way.)
        // (Long streaks along the flow, broken up: patches where it gathers,
        // thin or none elsewhere; two scales, so no regular net shows.)
        let a0 = vec2(dot(q0, fdir) * 0.3, dot(q0, side) * 2.2);
        let a1 = vec2(dot(q1, fdir) * 0.3, dot(q1, side) * 2.2);
        let breakup = smoothstep(0.35, 0.8, noise2(a0 * 0.35 + vec2(3.1, 7.7)) * 0.65 + noise2(base * 0.9) * 0.35);
        let cov_r = saturate(coverage * mix(0.25, 1.0, breakup));
        foam_amount = (foam_tex(a0, cov_r, footprint * 1.4, t) * w0 + foam_tex(a1 * 1.7, cov_r, footprint * 2.0, t) * (1.0 - w0)) * mix(0.35, 1.0, breakup);
    } else {
        // Shore and object foam round about; crest foam in streaks along
        // the way the waves run.
        let q = base + wind.xy * t * 0.05;
        let wd = wind_dir();
        let along = vec2(dot(q, wd) * 0.45, dot(q, vec2(-wd.y, wd.x)) * 1.5);
        // (Patchy: dense in places, thin or none in others, not an even net.)
        let patches = smoothstep(0.25, 0.75, noise2(along * 0.6 + vec2(7.3, 1.1)) * 0.7 + noise2(q * 0.35) * 0.3);
        foam_amount = max(foam_tex(q, saturate(shore), footprint, t), foam_tex(along, saturate(crest * patches * 1.3), footprint * 1.5, t));
    }
    // (No shore foam round a puddle: its whole sheet is "shallow", and the
    // bubble lattice showed as white cracks all over it.)
    let foam = clamp(select(foam_amount, 0.0, puddle()) + drops.z * 0.6, 0.0, 1.0) * smoothstep(0.0, 0.02, depth);
    let foam_rgb = vec3(0.85) * irradiance;

    let water = lit * (vec3(1.0) - trans) * (1.0 - fresnel);
    let rgb = mix(water + reflected * fresnel * edge + glint, foam_rgb, foam);
    var alpha = 1.0 - through * (1.0 - fresnel * edge) * (1.0 - foam);
    var fade = 1.0;
#ifdef VERTEX_COLORS
    if (kind.w > 0.5) {
        // (Broken up by noise: an even fade band read as a cut-out shape.)
        let ragged = noise2(wp.xz * 1.7) * 0.6 + noise2(wp.xz * 5.3) * 0.4;
        // (Gently: on a puddle made of thin pieces, everywhere near an edge,
        // a hard cut showed as cracks.)
        fade = smoothstep(0.0, 1.0, saturate(in.color.a * 1.4 + (ragged - 0.5) * 0.45));
        // (A puddle stays water inside its soft rim: built of thin pieces,
        // it had faded all but away.)
        if (puddle()) {
            fade = max(fade, 0.55 * smoothstep(0.02, 0.15, in.color.a + (ragged - 0.5) * 0.08));
        }
    }
#endif
    // (Premultiplied: faded out, the light it adds fades too.)
    alpha *= fade;
    out.color = main_pass_post_lighting_processing(pbr_input, vec4(rgb * fade, alpha));
    return out;
}
