//! Turning a project manifest into entities.

use bevy::{
    asset::RenderAssetUsages,
    camera::visibility::{NoFrustumCulling, RenderLayers},
    gltf::GltfAssetLabel,
    mesh::{Indices, PrimitiveTopology},
    prelude::*,
};

use crate::{
    material::{ToyMaterial, ToyUniforms, parse_alpha_mode},
    project::{CustomMesh, Manifest, MeshSpec, ObjectSpec, Store},
};

/// Root entity of a spawned project.
#[derive(Component)]
pub struct ProjectRoot;

/// Every asset a spawned project depends on, so callers can track loading and errors.
#[derive(Default)]
pub struct SpawnedProject {
    pub root: Option<Entity>,
    /// (object name, shader handle) of shaders used as pipeline entry points.
    pub entry_shaders: Vec<(String, Handle<Shader>)>,
    pub images: Vec<(String, Handle<Image>)>,
    pub meshes: Vec<(String, Handle<Mesh>)>,
    /// Problems found while building entities (bad manifest values, missing files...).
    pub errors: Vec<String>,
}

pub struct SpawnParams<'a> {
    pub store: &'a Store,
    pub id: &'a str,
    pub manifest: &'a Manifest,
    pub offset: Vec3,
    pub layers: RenderLayers,
    /// Disable frustum culling so every object gets a pipeline even when off-screen.
    pub no_culling: bool,
}

pub fn spawn_project(
    commands: &mut Commands,
    asset_server: &AssetServer,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<ToyMaterial>,
    p: SpawnParams,
) -> SpawnedProject {
    let mut out = SpawnedProject::default();
    out.errors.extend(p.store.missing_files(p.id, p.manifest));
    let root = commands
        .spawn((
            Name::new(format!("project {}", p.id)),
            ProjectRoot,
            Transform::from_translation(p.offset),
            Visibility::default(),
        ))
        .id();
    out.root = Some(root);

    for (index, obj) in p.manifest.objects.iter().enumerate() {
        let label = if obj.name.is_empty() {
            format!("#{index}")
        } else {
            obj.name.clone()
        };
        let mesh = match build_mesh(asset_server, meshes, p.store, p.id, &obj.mesh) {
            Ok(mesh) => mesh,
            Err(e) => {
                out.errors.push(format!("object `{label}`: {e}"));
                continue;
            }
        };
        out.meshes.push((label.clone(), mesh.clone()));
        let material = match build_material(asset_server, p.id, obj, &label, &mut out) {
            Ok(material) => materials.add(material),
            Err(e) => {
                out.errors.push(format!("object `{label}`: {e}"));
                continue;
            }
        };
        let mut entity = commands.spawn((
            Name::new(label),
            Mesh3d(mesh),
            MeshMaterial3d(material),
            Transform {
                translation: Vec3::from_array(obj.position),
                rotation: Quat::from_euler(
                    EulerRot::XYZ,
                    obj.rotation[0].to_radians(),
                    obj.rotation[1].to_radians(),
                    obj.rotation[2].to_radians(),
                ),
                scale: Vec3::from_array(obj.scale),
            },
            p.layers.clone(),
            ChildOf(root),
        ));
        if p.no_culling {
            entity.insert(NoFrustumCulling);
        }
    }
    out
}

fn asset_path(id: &str, rel: &str) -> String {
    format!("{id}/{}", rel.replace('\\', "/"))
}

fn build_material(
    asset_server: &AssetServer,
    id: &str,
    obj: &ObjectSpec,
    label: &str,
    out: &mut SpawnedProject,
) -> Result<ToyMaterial, String> {
    if obj.textures.len() > 4 {
        return Err("at most 4 textures are supported".into());
    }
    let mut textures = obj.textures.iter().map(|t| {
        let handle: Handle<Image> = asset_server.load(asset_path(id, t));
        out.images.push((format!("{label}: {t}"), handle.clone()));
        handle
    });
    let mut shader = |path: &Option<String>| {
        path.as_ref().map(|path| {
            let handle: Handle<Shader> = asset_server.load(asset_path(id, path));
            out.entry_shaders.push((label.to_string(), handle.clone()));
            handle
        })
    };
    let vertex_shader = shader(&obj.vertex_shader);
    let fragment_shader = shader(&obj.fragment_shader);
    let param = |i: usize| {
        obj.params
            .get(i)
            .copied()
            .map(Vec4::from_array)
            .unwrap_or_default()
    };
    Ok(ToyMaterial {
        uniforms: ToyUniforms {
            color: Vec4::from_array(obj.color),
            params0: param(0),
            params1: param(1),
            params2: param(2),
            params3: param(3),
        },
        tex0: textures.next(),
        tex1: textures.next(),
        tex2: textures.next(),
        tex3: textures.next(),
        alpha_mode: parse_alpha_mode(&obj.alpha_mode)?,
        double_sided: obj.double_sided,
        vertex_shader,
        fragment_shader,
    })
}

fn build_mesh(
    asset_server: &AssetServer,
    meshes: &mut Assets<Mesh>,
    store: &Store,
    id: &str,
    spec: &MeshSpec,
) -> Result<Handle<Mesh>, String> {
    let mesh: Mesh = match *spec {
        MeshSpec::Quad { size } => Rectangle::new(size[0], size[1]).into(),
        MeshSpec::Plane { size, subdivisions } => Plane3d::default()
            .mesh()
            .size(size[0], size[1])
            .subdivisions(subdivisions)
            .into(),
        MeshSpec::Cube { size } => Cuboid::new(size[0], size[1], size[2]).into(),
        MeshSpec::Sphere {
            radius,
            subdivisions,
        } => Sphere::new(radius)
            .mesh()
            .ico(subdivisions.min(40))
            .map_err(|e| e.to_string())?,
        MeshSpec::Cylinder { radius, height } => Cylinder::new(radius, height).into(),
        MeshSpec::Capsule { radius, length } => Capsule3d::new(radius, length).into(),
        MeshSpec::Cone { radius, height } => Cone { radius, height }.into(),
        MeshSpec::Torus {
            minor_radius,
            major_radius,
        } => Torus::new(major_radius - minor_radius, major_radius + minor_radius).into(),
        MeshSpec::Gltf {
            ref path,
            mesh_index,
        } => {
            return Ok(asset_server.load(
                GltfAssetLabel::Primitive {
                    mesh: mesh_index,
                    primitive: 0,
                }
                .from_asset(asset_path(id, path)),
            ));
        }
        MeshSpec::Custom { ref path } => {
            let bytes = store.read_file(id, path)?;
            let custom: CustomMesh =
                serde_json::from_slice(&bytes).map_err(|e| format!("{path}: {e}"))?;
            custom_mesh(custom).map_err(|e| format!("{path}: {e}"))?
        }
    };
    Ok(meshes.add(mesh))
}

fn custom_mesh(c: CustomMesh) -> Result<Mesh, String> {
    let n = c.positions.len();
    if n == 0 {
        return Err("`positions` is empty".into());
    }
    let check = |name: &str, len: Option<usize>| match len {
        Some(len) if len != n => Err(format!("`{name}` has {len} entries, expected {n}")),
        _ => Ok(()),
    };
    check("normals", c.normals.as_ref().map(Vec::len))?;
    check("uvs", c.uvs.as_ref().map(Vec::len))?;
    check("colors", c.colors.as_ref().map(Vec::len))?;
    if let Some(indices) = &c.indices {
        if indices.len() % 3 != 0 {
            return Err("`indices` length must be a multiple of 3".into());
        }
        if let Some(bad) = indices.iter().find(|&&i| i as usize >= n) {
            return Err(format!("index {bad} out of range"));
        }
    } else if !n.is_multiple_of(3) {
        return Err("without `indices`, `positions` length must be a multiple of 3".into());
    }

    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, c.positions);
    // The standard mesh pipeline expects UVs; default them to zero.
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_UV_0,
        c.uvs.unwrap_or_else(|| vec![[0.0, 0.0]; n]),
    );
    if let Some(colors) = c.colors {
        mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    }
    if let Some(indices) = c.indices {
        mesh.insert_indices(Indices::U32(indices));
    }
    match c.normals {
        Some(normals) => mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals),
        None => mesh.compute_normals(),
    }
    Ok(mesh)
}

/// Rough footprint of a project, used to space projects in group view.
pub fn project_radius(manifest: &Manifest) -> f32 {
    if let Some(r) = manifest.bounds_radius {
        return r;
    }
    manifest
        .objects
        .iter()
        .map(|o| {
            let extent = match o.mesh {
                MeshSpec::Quad { size } | MeshSpec::Plane { size, .. } => {
                    size[0].max(size[1]) * 0.5
                }
                MeshSpec::Cube { size } => Vec3::from_array(size).length() * 0.5,
                MeshSpec::Sphere { radius, .. } => radius,
                MeshSpec::Cylinder { radius, height } | MeshSpec::Cone { radius, height } => {
                    radius.max(height * 0.5)
                }
                MeshSpec::Capsule { radius, length } => radius + length * 0.5,
                MeshSpec::Torus {
                    minor_radius,
                    major_radius,
                } => minor_radius + major_radius,
                MeshSpec::Gltf { .. } | MeshSpec::Custom { .. } => 1.0,
            };
            let scale = o.scale.iter().fold(0.0f32, |a, b| a.max(b.abs()));
            Vec2::new(o.position[0], o.position[2]).length() + extent * scale
        })
        .fold(1.0, f32::max)
}
