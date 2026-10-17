// bevy_atmos's wet surfaces (`wet.rs`): Bevy's standard material, then
// soaked (darker), a water film (glossier) and puddles on flat ground.

#import bevy_pbr::{
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{alpha_discard, apply_pbr_lighting, main_pass_post_lighting_processing},
    forward_io::{VertexOutput, FragmentOutput},
    mesh_view_bindings::view,
}
#import bevy_atmos::ssr::screen_reflection

struct Wet {
    // x: wetness, y: allowed, z: puddles, w: darkening and gloss scale.
    params: vec4<f32>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(100) var<uniform> wet: Wet;
// x: the puddles' scale.
@group(#{MATERIAL_BIND_GROUP}) @binding(101) var<uniform> wet_look: vec4<f32>;
// Screen-space reflections: x strength (0 off), y roughness cutoff, z reach.
@group(#{MATERIAL_BIND_GROUP}) @binding(102) var<uniform> ssr: vec4<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(103) var history: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(104) var history_sampler: sampler;

fn hash2(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453);
}

fn noise2(p: vec2<f32>) -> f32 {
    let c = floor(p);
    let f = p - c;
    let u = f * f * (3.0 - 2.0 * f);
    return mix(mix(hash2(c), hash2(c + vec2(1.0, 0.0)), u.x), mix(hash2(c + vec2(0.0, 1.0)), hash2(c + vec2(1.0, 1.0)), u.x), u.y);
}

// Organic puddle shapes: gradient noise (no grid-aligned edges, unlike
// value noise), its domain warped by a larger one, four octaves.
fn puddle_grad(c: vec2<f32>) -> vec2<f32> {
    var p3 = fract(vec3(c.xyx) * 0.1031);
    p3 += dot(p3, p3.yzx + 33.33);
    let a = fract((p3.x + p3.y) * p3.z) * 6.2831853;
    return vec2(cos(a), sin(a));
}

fn puddle_gnoise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = p - i;
    let u = f * f * f * (f * (f * 6.0 - 15.0) + 10.0);
    let a = dot(puddle_grad(i), f);
    let b = dot(puddle_grad(i + vec2(1.0, 0.0)), f - vec2(1.0, 0.0));
    let c = dot(puddle_grad(i + vec2(0.0, 1.0)), f - vec2(0.0, 1.0));
    let d = dot(puddle_grad(i + vec2(1.0, 1.0)), f - vec2(1.0, 1.0));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

// 0..1, about 0.5 on average.
fn puddle_field(p: vec2<f32>) -> f32 {
    let warp = vec2(puddle_gnoise(p * 0.19 + vec2(3.1, 1.7)), puddle_gnoise(p * 0.19 + vec2(7.7, 4.2))) * 2.2;
    let q = p + warp;
    let n = puddle_gnoise(q * 0.33) * 0.62 + puddle_gnoise(q * 0.87) * 0.26 + puddle_gnoise(q * 2.3) * 0.11 + puddle_gnoise(q * 5.9) * 0.05;
    return clamp(0.5 + n * 0.95, 0.0, 1.0);
}

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    var pbr_input = pbr_input_from_standard_material(in, is_front);
    pbr_input.material.base_color = alpha_discard(pbr_input.material, pbr_input.material.base_color);
    let w = saturate(wet.params.x * wet.params.y);
    if (w > 0.0) {
        let scale = wet.params.w;
        // Soaked: darker (porous surfaces most); a film of water: glossier.
        let film = w * scale;
        let c = pbr_input.material.base_color;
        pbr_input.material.base_color = vec4(c.rgb * mix(1.0, 0.55, saturate(film * 0.85)), c.a);
        pbr_input.material.perceptual_roughness = mix(pbr_input.material.perceptual_roughness, 0.18, saturate(film * 0.75));
        // Puddles: in the hollows of a world-space noise on flat ground,
        // spreading as it gets wetter; still water, mirror-smooth.
        let up = saturate(in.world_normal.y);
        let p = in.world_position.xz;
        let n = puddle_field(p) + (w - 0.6) * 0.45;
        // (The edge a few pixels soft however far, never a hard line.)
        let soft = max(0.025, fwidth(n) * 1.5);
        let puddle = smoothstep(0.65 - soft, 0.65 + soft, n) * smoothstep(0.85, 0.97, up) * saturate(wet.params.z * wet_look.x);
        if (puddle > 0.0) {
            let pc = pbr_input.material.base_color;
            pbr_input.material.base_color = vec4(pc.rgb * mix(1.0, 0.6, puddle), pc.a);
            pbr_input.material.perceptual_roughness = mix(pbr_input.material.perceptual_roughness, 0.03, puddle);
            pbr_input.material.reflectance = max(pbr_input.material.reflectance, vec3(mix(0.5, 1.0, puddle)));
            pbr_input.N = normalize(mix(pbr_input.N, normalize(in.world_normal), puddle));
        }
    }
    var out: FragmentOutput;
    out.color = apply_pbr_lighting(pbr_input);
    // Wet, smooth surfaces reflect what's on screen (water's reflectance,
    // more at grazing angles), over the probe's reflection.
    if (w > 0.0) {
        let v = normalize(view.world_position - pbr_input.world_position.xyz);
        let n = pbr_input.N;
        let rough = pbr_input.material.perceptual_roughness;
        let screen = screen_reflection(pbr_input.world_position.xyz, reflect(-v, n), rough, in.position.xy, ssr, history, history_sampler);
        if (screen.a > 0.0) {
            let f = 0.02 + 0.98 * pow(1.0 - saturate(dot(n, v)), 5.0);
            let amount = saturate(screen.a) * f * (1.0 - rough) * saturate(w * 1.5);
            out.color = vec4(out.color.rgb + screen.rgb * amount, out.color.a);
        }
    }
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
}
