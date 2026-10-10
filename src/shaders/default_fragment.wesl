// Fallback fragment shader for objects without `fragment_shader`: tinted UV checker.
import bevy_pbr::render::forward_io::VertexOutput;
import shadertoy::shaders::toy::{toy, tex0, samp0};

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let cells = floor(in.uv * 8.0);
    let checker = 0.6 + 0.4 * f32((i32(cells.x) + i32(cells.y)) & 1);
    let tex = textureSample(tex0, samp0, in.uv);
    return vec4<f32>(vec3<f32>(in.uv, 0.5) * checker, 1.0) * toy.color * tex;
}
