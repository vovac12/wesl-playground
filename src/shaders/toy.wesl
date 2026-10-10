// Material bindings shared by every playground object.
//
//   import shadertoy::shaders::toy::{toy, tex0, samp0, time};
//
// `toy.color` and `toy.params0..3` come from the object's `color` / `params` in project.json.
// `tex0..tex3` are the object's `textures` (a 1x1 white texture when unset).
import bevy_pbr::render::mesh_view_bindings::{globals, view};

struct ToyUniforms {
    color: vec4<f32>,
    params0: vec4<f32>,
    params1: vec4<f32>,
    params2: vec4<f32>,
    params3: vec4<f32>,
};

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> toy: ToyUniforms;
@group(constants::MATERIAL_BIND_GROUP) @binding(1) var tex0: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(2) var samp0: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(3) var tex1: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(4) var samp1: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(5) var tex2: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(6) var samp2: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(7) var tex3: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(8) var samp3: sampler;

/// Seconds since startup.
fn time() -> f32 {
    return globals.time;
}

/// World-space camera position.
fn camera_position() -> vec3<f32> {
    return view.world_position;
}
