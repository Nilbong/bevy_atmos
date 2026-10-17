// bevy_atmos's water (`water.rs`): Bevy's standard material with waves
// from the world position and the look's sky reflected by Fresnel.

#import bevy_pbr::{
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{alpha_discard, apply_pbr_lighting, main_pass_post_lighting_processing},
    forward_io::{VertexOutput, FragmentOutput},
    mesh_view_bindings::{view, globals},
}

struct Water {
    // x: waves, y: reflection, z: rain, w: the skybox's brightness.
    params: vec4<f32>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(100) var<uniform> water: Water;
@group(#{MATERIAL_BIND_GROUP}) @binding(101) var sky: texture_cube<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(102) var sky_sampler: sampler;

fn height(p: vec2<f32>, t: f32) -> f32 {
    let k = array<vec2<f32>, 6>(vec2(0.61, 0.27), vec2(-0.43, 0.71), vec2(0.97, -0.33), vec2(0.23, -1.13), vec2(-1.21, -0.52), vec2(0.79, 1.07));
    var h = 0.0;
    for (var i = 0; i < 6; i++) {
        h += sin(6.2831853 * dot(p, k[i]) + 1.4 * sqrt(length(k[i])) * t + f32(i) * 1.7) / (1.0 + f32(i) * 0.35);
    }
    return h;
}

fn hash2(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453);
}

// Rain rings: a ring spreading in each cell, a new place each time round.
fn ripples(p: vec2<f32>, t: f32) -> vec2<f32> {
    var g = vec2(0.0);
    for (var layer = 0; layer < 2; layer++) {
        let q = p * (3.0 + f32(layer) * 1.7) + f32(layer) * 7.3;
        let c = floor(q);
        let phase = fract(t * 1.3 + hash2(c));
        let centre = c + vec2(hash2(c + 3.1), hash2(c + 5.7));
        let d = q - centre;
        let r = length(d);
        let ring = sin((r - phase) * 40.0) * smoothstep(0.12, 0.0, abs(r - phase)) * (1.0 - phase);
        g += normalize(d + 1e-4) * ring;
    }
    return g;
}

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    var pbr_input = pbr_input_from_standard_material(in, is_front);
    pbr_input.material.base_color = alpha_discard(pbr_input.material, pbr_input.material.base_color);
    let t = globals.time;
    let p = in.world_position.xz * 0.35;
    let e = 0.01;
    let h0 = height(p, t);
    let gx = (height(p + vec2(e, 0.0), t) - h0) / e;
    let gz = (height(p + vec2(0.0, e), t) - h0) / e;
    let rings = ripples(in.world_position.xz, t) * water.params.z;
    let slope = vec2(gx, gz) * 0.008 * water.params.x + rings * 0.15;
    let n = normalize(vec3(-slope.x, 1.0, -slope.y));
    pbr_input.N = n;
    var out: FragmentOutput;
    out.color = apply_pbr_lighting(pbr_input);
    // The sky reflected, more at grazing angles.
    let v = normalize(view.world_position - in.world_position.xyz);
    let r = reflect(-v, n);
    let fresnel = 0.02 + 0.98 * pow(1.0 - saturate(dot(n, v)), 5.0);
    let reflected = textureSampleLevel(sky, sky_sampler, vec3(r.x, abs(r.y), r.z), 0.0).rgb * water.params.w * view.exposure;
    out.color = vec4(out.color.rgb + reflected * fresnel * water.params.y, max(out.color.a, fresnel * water.params.y));
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
}
