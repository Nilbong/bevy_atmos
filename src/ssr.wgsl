// bevy_atmos's screen-space reflections (`ssr.rs`): the reflected ray
// marched through the depth prepass in screen space; where it passes
// behind a surface, that pixel of the camera's last lit image. Returns
// the colour (exposed, as on screen) and how much to trust it (0: nothing
// found, use the fallback). Smooth surfaces only; fades at the screen's
// edges and toward the ray's end.

#define_import_path bevy_atmos::ssr

#import bevy_pbr::mesh_view_bindings::view
#ifdef DEPTH_PREPASS
#import bevy_pbr::prepass_utils::prepass_depth
#endif

const SSR_MAX_STEPS: f32 = 32.0;

fn ssr_hash(p: vec2<f32>) -> f32 {
    var p3 = fract(vec3(p.xyx) * 0.1031);
    p3 += dot(p3, p3.yzx + 33.33);
    return fract((p3.x + p3.y) * p3.z);
}

fn ssr_depth(uv: vec2<f32>, size: vec2<f32>) -> f32 {
#ifdef DEPTH_PREPASS
    return prepass_depth(vec4(uv * size, 0.0, 0.0), 0u);
#else
    return 0.0;
#endif
}

// `params`: x strength (0 off), y roughness cutoff, z reach (m).
fn screen_reflection(origin: vec3<f32>, r: vec3<f32>, roughness: f32, frag: vec2<f32>, params: vec4<f32>, history: texture_2d<f32>, history_sampler: sampler) -> vec4<f32> {
#ifdef DEPTH_PREPASS
    // (Off, or no frame to reflect yet.)
    if (params.x <= 0.0 || params.w < 0.5) {
        return vec4(0.0);
    }
    let reach = params.z;
    let far = 1.0 - smoothstep(reach, reach * 1.5, distance(origin, view.world_position));
    let smooth_enough = (1.0 - smoothstep(params.y * 0.45, params.y, roughness)) * far;
    if (smooth_enough <= 0.0) {
        return vec4(0.0);
    }
    let near = view.clip_from_view[3][2];
    let start = view.clip_from_world * vec4(origin, 1.0);
    var end = view.clip_from_world * vec4(origin + r * reach, 1.0);
    // Stop the ray at the near plane.
    if (end.w < near) {
        end = mix(start, end, (start.w - near) / max(start.w - end.w, 1e-5));
    }
    let s = start.xyz / start.w;
    let e = end.xyz / end.w;
    let size = view.viewport.zw;
    let s_uv = s.xy * vec2(0.5, -0.5) + 0.5;
    let e_uv = e.xy * vec2(0.5, -0.5) + 0.5;
    // (A step per 8 pixels: the narrowing after a hit finds the edge.)
    let steps = clamp(length((e_uv - s_uv) * size) / 8.0, 8.0, SSR_MAX_STEPS);
    // (A narrow jitter: a full step's sparkled on mirror-flat water.)
    let jitter = 0.5 + (ssr_hash(frag) - 0.5) * 0.3;
    var lo = 0.0;
    var last = s_uv;
    for (var i = 1.0; i <= steps; i += 1.0) {
        let t = (i - 1.0 + jitter) / steps;
        let uv = mix(s_uv, e_uv, t);
        if (any(uv < vec2(0.0)) || any(uv > vec2(1.0))) {
            // (Off the screen's edge: what's at the edge where it left (a
            // wall beside the puddle, most often), faded, not nothing; the
            // bright sky alone read wrong.)
            let edge_c = textureSampleLevel(history, history_sampler, clamp(last, vec2(0.002), vec2(0.998)), 0.0).rgb;
            return vec4(edge_c, 0.55 * smooth_enough * params.x * smoothstep(0.0, 0.3, t));
        }
        last = uv;
        // Depth (reverse Z) runs straight in screen space.
        let ray_z = mix(s.z, e.z, t);
        let scene_z = ssr_depth(uv, size);
        if (scene_z > ray_z) {
            // Behind something: within its assumed thickness, a hit.
            let ray_d = near / max(ray_z, 1e-7);
            let scene_d = near / max(scene_z, 1e-7);
            if (ray_d - scene_d < 0.4 + 0.03 * scene_d) {
                // Narrow it down between the last step and this one.
                var a = lo;
                var b = t;
                for (var k = 0; k < 5; k++) {
                    let m = (a + b) * 0.5;
                    let muv = mix(s_uv, e_uv, m);
                    if (ssr_depth(muv, size) > mix(s.z, e.z, m)) {
                        b = m;
                    } else {
                        a = m;
                    }
                }
                let hit = mix(s_uv, e_uv, b);
                let edge = min(min(hit.x, 1.0 - hit.x), min(hit.y, 1.0 - hit.y));
                let fade = smoothstep(0.0, 0.08, edge) * (1.0 - smoothstep(0.7, 1.0, b)) * smooth_enough;
                // (A few taps round the hit: single samples sparkle on
                // rippled water.)
                let px = 1.0 / size;
                let c = (textureSampleLevel(history, history_sampler, hit, 0.0).rgb * 2.0
                    + textureSampleLevel(history, history_sampler, hit + vec2(px.x, 0.0), 0.0).rgb
                    + textureSampleLevel(history, history_sampler, hit - vec2(px.x, 0.0), 0.0).rgb
                    + textureSampleLevel(history, history_sampler, hit + vec2(0.0, px.y), 0.0).rgb
                    + textureSampleLevel(history, history_sampler, hit - vec2(0.0, px.y), 0.0).rgb) / 6.0;
                return vec4(c, fade * params.x);
            }
        }
        lo = t;
    }
#endif
    return vec4(0.0);
}
