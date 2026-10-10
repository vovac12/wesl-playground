# Shader playground (Bevy 0.20, WESL)

A running 3D viewer. Shader *projects* live in folders under the viewer's BASE_FOLDER and show
up in the viewer's dropdown. The user can fly around (right mouse = look, WASD/QE = move,
Shift = fast). Projects can be grouped; a whole group can be shown side by side.

## Workflow
1. `create_project` with a manifest and shader files. It compiles the project off-screen and
   returns errors (and a preview PNG when `preview: true`). Fix errors with `update_project`
   until `ok` is true. You can create or edit any project, whether or not it is open in the viewer.
2. Use project ids `group/name` to put related variants in one group (e.g. `portals/swirl_vortex`).
   Ids are lowercase snake_case: `[a-z][a-z0-9_]*`. Give each project a descriptive `name`.
3. Only call `open_in_viewer` when the user wants to see something; it changes what they look at.

## project.json (the `manifest` argument)
```json
{
  "name": "Portal: swirl vortex",
  "description": "Animated swirl, additive glow, no textures",
  "camera": { "position": [0, 1.6, 5], "look_at": [0, 1.5, 0] },
  "background": [0.01, 0.01, 0.02],
  "grid": true,
  "objects": [{
    "name": "portal",
    "mesh": { "shape": "quad", "size": [2.5, 3.5] },
    "fragment_shader": "portal.wesl",
    "vertex_shader": null,
    "textures": ["textures/noise.png"],
    "color": [0.3, 0.6, 1.0, 1.0],
    "params": [[1.0, 0.5, 0.0, 0.0]],
    "alpha_mode": "blend",
    "double_sided": true,
    "position": [0, 1.75, 0], "rotation": [0, 0, 0], "scale": [1, 1, 1]
  }]
}
```
Objects: one entity each. `rotation` is Euler XYZ degrees. Objects without `fragment_shader`
render a UV checker. The ground grid is at y=0, +Y is up, the default camera looks toward -Z.

Mesh shapes: `quad {size:[w,h]}` (XY plane, faces +Z), `plane {size:[w,d], subdivisions}`
(XZ plane, faces +Y), `cube {size:[x,y,z]}`, `sphere {radius, subdivisions}`,
`cylinder {radius, height}`, `capsule {radius, length}`, `cone {radius, height}`,
`torus {minor_radius, major_radius}`, `gltf {path, mesh_index}` (first primitive),
`custom {path}` -> JSON file `{positions:[[x,y,z]..], normals?, uvs?, colors?, indices?}`.
Every built-in shape has positions, normals and UVs.

alpha_mode: `opaque` (default), `blend`, `add`, `premultiplied`, `multiply`, `mask`.
Your fragment output decides the blending (Bevy's own shaders do this premultiply step for you,
custom shaders must do it themselves):
- `blend`: return straight alpha `vec4(rgb, a)` -> `rgb*a + dst*(1-a)`.
- `add` / `premultiplied` (same GPU blend state `src + dst*(1-src.a)`): for pure additive glow
  return `vec4(rgb * mask, 0.0)`; for premultiplied return `vec4(rgb * a, a)`.
- `mask`: fragments with alpha < 0.5 are discarded only if you `discard` them yourself.
Transparent modes don't write depth. Use `double_sided: true` for flat effects that should be
visible from both sides.

## WESL rules (Bevy 0.20 removed naga_oil `#import`/`#ifdef`)
- `import a::b::{c, d};` statements come first and end with `;`.
- Relative imports to files in the same project folder: `import super::common::{my_fn};`
  (file `common.wesl` next to the importing file).
- Conditional compilation: `@if(FLAG)` / `@else` on declarations and statements.
- Everything else is plain WGSL. Use explicit float literals (`1.0`, not `1`) when mixing with f32.
- Fragment entry point must be `fn fragment`, vertex entry point `fn vertex`.
- WESL parse errors point at your file (`--> group/project/file.wesl:4:18`). Type/validation
  errors from naga (`wgsl:L:C`) refer to the composed shader, so locate them by the quoted code.

## Bindings — always import them, never declare your own `@group/@binding`
```wgsl
import shadertoy::shaders::toy::{toy, tex0, samp0, tex1, samp1, tex2, samp2, tex3, samp3, time, camera_position};
```
- `toy.color: vec4<f32>` = object `color`; `toy.params0..params3: vec4<f32>` = object `params[0..3]`.
- `tex0..tex3` / `samp0..samp3` = object `textures` (white 1x1 when absent). Samplers repeat.
- `time() -> f32` seconds since startup, `camera_position() -> vec3<f32>` world space.
- Also available: `import bevy_pbr::render::mesh_view_bindings::{globals, view};`

Noise helpers: `import shadertoy::shaders::noise::{hash21, hash22, hash31, noise2, noise3, fbm2, fbm3, voronoi2, palette, rotate2, TAU};`

## Fragment shader template
```wgsl
import bevy_pbr::render::forward_io::VertexOutput;
import shadertoy::shaders::toy::{toy, time};
import shadertoy::shaders::noise::{fbm2, rotate2};

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    // in.uv (0..1), in.world_position (vec4), in.world_normal, in.position (frag coord)
    let p = in.uv * 2.0 - 1.0;
    let r = length(p);
    let swirl = fbm2(rotate2(p, time() + r * 4.0) * 3.0);
    let ring = smoothstep(1.0, 0.8, r);
    let col = toy.color.rgb * swirl * 2.0;
    return vec4<f32>(col, ring);   // alpha is used with alpha_mode blend
}
```
Output colors are linear HDR-ish (values > 1 look brighter, then tonemapped).

## Vertex shader template (only when you need to displace geometry)
```wgsl
import bevy_pbr::render::{
    forward_io::{Vertex, VertexOutput, decompress_vertex},
    mesh_functions,
    view_transformations::position_world_to_clip,
};
import shadertoy::shaders::toy::time;

@vertex
fn vertex(vertex_in: Vertex) -> VertexOutput {
    let v = decompress_vertex(vertex_in, vertex_in.instance_index);
    let world_from_local = mesh_functions::get_world_from_local(vertex_in.instance_index);
    var p = v.position;
    p += v.normal * sin(p.y * 6.0 + time() * 2.0) * 0.05;
    var out: VertexOutput;
    out.world_position = mesh_functions::mesh_position_local_to_world(world_from_local, vec4<f32>(p, 1.0));
    out.position = position_world_to_clip(out.world_position.xyz);
    out.world_normal = mesh_functions::mesh_normal_local_to_world(v.normal, vertex_in.instance_index);
    out.uv = v.uv;
    return out;
}
```
The fragment shader then receives the same `VertexOutput`.

## Textures and files
- `add_texture` writes a procedural PNG (noise, fbm, voronoi, checker, gradient, radial,
  stripes; tiling) or copies/decodes an existing image. Reference it in `textures`.
- `import_file` copies a binary (e.g. .glb) from a local path or base64.
- Text files (shaders, custom mesh JSON) go through `files` in `create_project` / `update_project`.

## Raymarching tip
For volumetric/SDF looks on a mesh, raymarch inside the fragment shader using
`camera_position()` and `in.world_position.xyz` as ray origin/direction, e.g. on a `cube`
with `double_sided: true` and `alpha_mode: blend`.
