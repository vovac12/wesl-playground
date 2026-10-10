//! Shader playground: a Bevy viewer for WESL shader projects, driven by an embedded MCP server.
//!
//! Environment:
//! - `BASE_FOLDER`: folder holding shader projects (default `./projects`)
//! - `MCP_ADDR`: MCP listen address (default `127.0.0.1:7878`)

#![allow(
    clippy::type_complexity,
    reason = "Bevy queries and systems routinely trip this lint"
)]
#![allow(
    clippy::too_many_arguments,
    reason = "Bevy systems take many parameters"
)]

mod bridge;
mod logs;
mod material;
mod mcp;
mod pipeline_status;
mod playground;
mod project;
mod scene;
mod textures;
mod ui;

use std::{path::PathBuf, sync::Arc};

use bevy::{
    camera_controller::free_camera::FreeCameraPlugin,
    image::{ImageAddressMode, ImageSamplerDescriptor},
    log::LogPlugin,
    prelude::*,
    render::error_handler::{ErrorType, RenderErrorHandler, RenderErrorPolicy},
};

use crate::{
    material::ToyMaterialPlugin,
    pipeline_status::PipelineStatusPlugin,
    playground::PlaygroundPlugin,
    project::{GroupManifest, Manifest, MeshSpec, ObjectSpec, Store},
    ui::UiPlugin,
};

fn main() {
    let base = std::env::var("BASE_FOLDER").unwrap_or_else(|_| "projects".into());
    let base = std::path::absolute(PathBuf::from(base)).expect("resolve BASE_FOLDER");
    std::fs::create_dir_all(&base).expect("create BASE_FOLDER");
    let addr = std::env::var("MCP_ADDR").unwrap_or_else(|_| "127.0.0.1:7878".into());
    let mcp_url = format!("http://{addr}/mcp");

    let store = Arc::new(Store::new(base.clone()));
    seed_examples(&store);
    write_mcp_config(&base, &mcp_url);

    let (tx, rx) = crossbeam_channel::unbounded();

    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(AssetPlugin {
                file_path: base.to_string_lossy().into_owned(),
                // Hot-reload shaders edited by hand as well as through MCP.
                watch_for_changes_override: Some(true),
                ..default()
            })
            // Repeat addressing so shaders can tile and scroll textures freely.
            .set(
                ImagePlugin::default_linear().with_default_sampler(ImageSamplerDescriptor {
                    address_mode_u: ImageAddressMode::Repeat,
                    address_mode_v: ImageAddressMode::Repeat,
                    address_mode_w: ImageAddressMode::Repeat,
                    ..ImageSamplerDescriptor::linear()
                }),
            )
            .set(LogPlugin {
                custom_layer: logs::custom_layer,
                ..default()
            })
            .set(WindowPlugin {
                primary_window: Some(Window {
                    title: format!("Shader Playground - {}", base.display()),
                    ..default()
                }),
                ..default()
            }),
    )
    .add_plugins((
        FreeCameraPlugin,
        ToyMaterialPlugin,
        PipelineStatusPlugin,
        PlaygroundPlugin {
            store: store.clone(),
            requests: rx,
        },
        UiPlugin { mcp_url },
    ))
    // A broken user shader must not take the whole app down: wgpu validation errors are logged
    // (and reported over MCP) instead of quitting.
    .insert_resource(RenderErrorHandler(|error, main_world, _| {
        if error.ty == ErrorType::Validation {
            RenderErrorPolicy::Ignore
        } else {
            bevy::log::error!("Quitting due to {:?} render error", error.ty);
            main_world.write_message(AppExit::error());
            RenderErrorPolicy::StopRendering
        }
    }));

    mcp::spawn(addr, store, tx);
    app.run();
}

/// Lets `claude` started inside BASE_FOLDER pick up the server automatically.
fn write_mcp_config(base: &std::path::Path, url: &str) {
    let path = base.join(".mcp.json");
    if path.exists() {
        return;
    }
    let config = serde_json::json!({
        "mcpServers": { "shadertoy": { "type": "http", "url": url } }
    });
    let _ = std::fs::write(path, serde_json::to_string_pretty(&config).unwrap());
}

/// Creates a small example group when the base folder has no projects.
fn seed_examples(store: &Store) {
    let catalog = store.scan();
    if !catalog.groups.is_empty() || !catalog.ungrouped.is_empty() {
        return;
    }
    let _ = store.write_group(
        "examples",
        &GroupManifest {
            name: "Examples".into(),
            description: "Built-in starter projects".into(),
            spacing: None,
        },
    );

    let rings = "\
import bevy_pbr::render::forward_io::VertexOutput;
import shadertoy::shaders::toy::{toy, time};
import shadertoy::shaders::noise::palette;

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let p = in.uv * 2.0 - 1.0;
    let r = length(p);
    let a = atan2(p.y, p.x);
    let wave = sin(r * 18.0 - time() * toy.params0.x + a * 3.0) * 0.5 + 0.5;
    let col = palette(r + time() * 0.1, vec3(0.5), vec3(0.5), vec3(1.0), vec3(0.0, 0.33, 0.67));
    let alpha = smoothstep(1.0, 0.9, r);
    return vec4<f32>(col * (0.3 + wave * 1.5), alpha);
}
";
    let _ = store.write_file("examples/rings", "rings.wesl", rings.as_bytes());
    let _ = store.write_manifest(
        "examples/rings",
        &Manifest {
            name: "Animated rings".into(),
            description: "UV-space rings on a double-sided quad".into(),
            objects: vec![ObjectSpec {
                name: "rings".into(),
                mesh: MeshSpec::Quad { size: [3.0, 3.0] },
                fragment_shader: Some("rings.wesl".into()),
                params: vec![[4.0, 0.0, 0.0, 0.0]],
                alpha_mode: "blend".into(),
                double_sided: true,
                position: [0.0, 1.6, 0.0],
                ..default()
            }],
            ..default()
        },
    );

    let blob_vertex = "\
import bevy_pbr::render::{
    forward_io::{Vertex, VertexOutput, decompress_vertex},
    mesh_functions,
    view_transformations::position_world_to_clip,
};
import shadertoy::shaders::toy::time;
import shadertoy::shaders::noise::noise3;

@vertex
fn vertex(vertex_in: Vertex) -> VertexOutput {
    let v = decompress_vertex(vertex_in, vertex_in.instance_index);
    let world_from_local = mesh_functions::get_world_from_local(vertex_in.instance_index);
    let n = noise3(v.position * 3.0 + vec3<f32>(0.0, time() * 0.8, 0.0));
    let p = v.position + v.normal * (n - 0.5) * 0.35;
    var out: VertexOutput;
    out.world_position = mesh_functions::mesh_position_local_to_world(world_from_local, vec4<f32>(p, 1.0));
    out.position = position_world_to_clip(out.world_position.xyz);
    out.world_normal = mesh_functions::mesh_normal_local_to_world(v.normal, vertex_in.instance_index);
    out.uv = v.uv;
    return out;
}
";
    let blob_fragment = "\
import bevy_pbr::render::forward_io::VertexOutput;
import shadertoy::shaders::toy::{toy, tex0, samp0, camera_position};

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let n = normalize(in.world_normal);
    let view_dir = normalize(camera_position() - in.world_position.xyz);
    let fresnel = pow(1.0 - max(dot(n, view_dir), 0.0), 3.0);
    let light = max(dot(n, normalize(vec3<f32>(0.4, 1.0, 0.6))), 0.0);
    let tex = textureSample(tex0, samp0, in.uv * 2.0).rgb;
    let base = toy.color.rgb * tex * (0.25 + 0.75 * light);
    return vec4<f32>(base + fresnel * vec3<f32>(0.6, 0.8, 1.5), 1.0);
}
";
    let _ = store.write_file("examples/blob", "blob_vertex.wesl", blob_vertex.as_bytes());
    let _ = store.write_file("examples/blob", "blob.wesl", blob_fragment.as_bytes());
    if let Ok(png) = textures::generate_png(&textures::Procedural {
        kind: "voronoi".into(),
        scale: 6.0,
        color_a: [1.0, 1.0, 1.0, 1.0],
        color_b: [0.2, 0.2, 0.3, 1.0],
        ..default()
    }) {
        let _ = store.write_file("examples/blob", "textures/cells.png", &png);
    }
    let _ = store.write_manifest(
        "examples/blob",
        &Manifest {
            name: "Noise blob".into(),
            description: "Vertex displacement + fresnel fragment shader with a procedural texture"
                .into(),
            objects: vec![ObjectSpec {
                name: "blob".into(),
                mesh: MeshSpec::Sphere {
                    radius: 1.0,
                    subdivisions: 24,
                },
                vertex_shader: Some("blob_vertex.wesl".into()),
                fragment_shader: Some("blob.wesl".into()),
                textures: vec!["textures/cells.png".into()],
                color: [1.0, 0.55, 0.3, 1.0],
                position: [0.0, 1.5, 0.0],
                ..default()
            }],
            ..default()
        },
    );
}
