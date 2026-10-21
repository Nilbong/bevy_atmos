// Ambient occlusion on all of the light (`ssao.direct`): drawn over the
// lit opaque scene with a multiply blend, before the transparents.

#import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput

// Bevy's denoised SSAO (1 open, 0 fully occluded).
@group(0) @binding(0) var ao: texture_2d<f32>;
// x: strength (0..1), y: contrast (the power on the AO).
@group(0) @binding(1) var<uniform> params: vec4<f32>;

@fragment
fn fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let size = vec2<i32>(textureDimensions(ao));
    let p = clamp(vec2<i32>(in.position.xy), vec2(0), size - 1);
    let a = saturate(textureLoad(ao, p, 0).r);
    let k = mix(1.0, pow(a, params.y), saturate(params.x));
    return vec4(k, k, k, 1.0);
}
