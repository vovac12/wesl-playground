//! On-disk shader project model.
//!
//! Layout of `BASE_FOLDER`:
//!
//! ```text
//! BASE_FOLDER/
//!   my_project/            <- ungrouped project (has project.json)
//!     project.json
//!     main.wesl
//!   portals/               <- group (no project.json, optional group.json)
//!     group.json
//!     portal_simple/
//!       project.json
//!       portal.wesl
//!       textures/noise.png
//! ```
//!
//! Project ids are `slug` or `group/slug`. Slugs are restricted to `[a-z][a-z0-9_]*` so that
//! every folder is a valid WESL module path component (imports like `super::common` work).

use std::{
    collections::BTreeMap,
    fs,
    path::{Component, Path, PathBuf},
};

use serde::{Deserialize, Serialize};

pub const MANIFEST_FILE: &str = "project.json";
pub const GROUP_FILE: &str = "group.json";

/// Contents of `project.json`.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct Manifest {
    /// Human readable name shown in the UI.
    pub name: String,
    pub description: String,
    /// Initial camera placement (relative to the project origin).
    pub camera: Option<CameraSpec>,
    /// Clear color, linear RGB.
    pub background: Option<[f32; 3]>,
    /// Draw the reference grid on the ground plane (default true).
    pub grid: Option<bool>,
    /// Radius of the project's footprint, used to space projects when a whole group is shown.
    pub bounds_radius: Option<f32>,
    pub objects: Vec<ObjectSpec>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CameraSpec {
    pub position: [f32; 3],
    #[serde(default)]
    pub look_at: [f32; 3],
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct ObjectSpec {
    pub name: String,
    pub mesh: MeshSpec,
    /// Path (relative to the project folder) of a WESL file with an `@vertex fn vertex`.
    /// Omit to use Bevy's standard mesh vertex shader.
    pub vertex_shader: Option<String>,
    /// Path (relative to the project folder) of a WESL file with an `@fragment fn fragment`.
    /// Omit to use the built-in debug shader.
    pub fragment_shader: Option<String>,
    /// Up to 4 texture paths relative to the project folder, bound as `tex0..tex3`.
    pub textures: Vec<String>,
    /// `toy.color`
    pub color: [f32; 4],
    /// `toy.params0..params3`
    pub params: Vec<[f32; 4]>,
    /// opaque | blend | add | multiply | premultiplied | mask
    pub alpha_mode: String,
    /// Disables back-face culling.
    pub double_sided: bool,
    pub position: [f32; 3],
    /// Euler XYZ in degrees.
    pub rotation: [f32; 3],
    pub scale: [f32; 3],
}

impl Default for ObjectSpec {
    fn default() -> Self {
        Self {
            name: String::new(),
            mesh: MeshSpec::default(),
            vertex_shader: None,
            fragment_shader: None,
            textures: Vec::new(),
            color: [1.0; 4],
            params: Vec::new(),
            alpha_mode: "opaque".into(),
            double_sided: false,
            position: [0.0; 3],
            rotation: [0.0; 3],
            scale: [1.0; 3],
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "shape", rename_all = "snake_case")]
pub enum MeshSpec {
    /// Vertical quad in the XY plane facing +Z.
    Quad {
        #[serde(default = "one2")]
        size: [f32; 2],
    },
    /// Horizontal plane in the XZ plane facing +Y.
    Plane {
        #[serde(default = "one2")]
        size: [f32; 2],
        #[serde(default)]
        subdivisions: u32,
    },
    Cube {
        #[serde(default = "one3")]
        size: [f32; 3],
    },
    Sphere {
        #[serde(default = "half")]
        radius: f32,
        #[serde(default = "five")]
        subdivisions: u32,
    },
    Cylinder {
        #[serde(default = "half")]
        radius: f32,
        #[serde(default = "one")]
        height: f32,
    },
    Capsule {
        #[serde(default = "half")]
        radius: f32,
        #[serde(default = "one")]
        length: f32,
    },
    Cone {
        #[serde(default = "half")]
        radius: f32,
        #[serde(default = "one")]
        height: f32,
    },
    Torus {
        #[serde(default = "quarter")]
        minor_radius: f32,
        #[serde(default = "one")]
        major_radius: f32,
    },
    /// A mesh from a glTF/glb file in the project folder (first primitive of `mesh_index`).
    Gltf {
        path: String,
        #[serde(default)]
        mesh_index: usize,
    },
    /// A mesh from a JSON file in the project folder, see [`CustomMesh`].
    Custom { path: String },
}

impl Default for MeshSpec {
    fn default() -> Self {
        MeshSpec::Quad { size: [1.0, 1.0] }
    }
}

fn one() -> f32 {
    1.0
}
fn half() -> f32 {
    0.5
}
fn quarter() -> f32 {
    0.25
}
fn five() -> u32 {
    5
}
fn one2() -> [f32; 2] {
    [1.0; 2]
}
fn one3() -> [f32; 3] {
    [1.0; 3]
}

/// Format of `MeshSpec::Custom` files.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CustomMesh {
    pub positions: Vec<[f32; 3]>,
    #[serde(default)]
    pub normals: Option<Vec<[f32; 3]>>,
    #[serde(default)]
    pub uvs: Option<Vec<[f32; 2]>>,
    #[serde(default)]
    pub colors: Option<Vec<[f32; 4]>>,
    #[serde(default)]
    pub indices: Option<Vec<u32>>,
}

/// Contents of `group.json`.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct GroupManifest {
    pub name: String,
    pub description: String,
    /// Distance between project origins when the whole group is shown together.
    pub spacing: Option<f32>,
}

#[derive(Clone, Debug)]
pub struct ProjectEntry {
    pub id: String,
    pub group: Option<String>,
    pub manifest: Result<Manifest, String>,
}

impl ProjectEntry {
    pub fn display_name(&self) -> String {
        match &self.manifest {
            Ok(m) if !m.name.is_empty() => m.name.clone(),
            _ => self.id.rsplit('/').next().unwrap_or(&self.id).to_string(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct GroupEntry {
    pub id: String,
    pub manifest: GroupManifest,
    pub projects: Vec<ProjectEntry>,
}

impl GroupEntry {
    pub fn display_name(&self) -> String {
        if self.manifest.name.is_empty() {
            self.id.clone()
        } else {
            self.manifest.name.clone()
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Catalog {
    pub groups: Vec<GroupEntry>,
    pub ungrouped: Vec<ProjectEntry>,
}

impl Catalog {
    pub fn find(&self, id: &str) -> Option<&ProjectEntry> {
        self.ungrouped
            .iter()
            .chain(self.groups.iter().flat_map(|g| g.projects.iter()))
            .find(|p| p.id == id)
    }

    pub fn group(&self, id: &str) -> Option<&GroupEntry> {
        self.groups.iter().find(|g| g.id == id)
    }
}

/// Validates a single path segment used as a folder name.
pub fn validate_slug(slug: &str) -> Result<(), String> {
    let mut chars = slug.chars();
    let ok = matches!(chars.next(), Some('a'..='z'))
        && chars.all(|c| matches!(c, 'a'..='z' | '0'..='9' | '_'))
        && slug.len() <= 64;
    if ok {
        Ok(())
    } else {
        Err(format!(
            "invalid name `{slug}`: use lowercase snake_case starting with a letter ([a-z][a-z0-9_]*), it must be a valid WESL module name"
        ))
    }
}

/// Validates `slug` or `group/slug`.
pub fn validate_project_id(id: &str) -> Result<(), String> {
    let parts: Vec<&str> = id.split('/').collect();
    if parts.is_empty() || parts.len() > 2 {
        return Err(format!(
            "invalid project id `{id}`: expected `name` or `group/name`"
        ));
    }
    parts.iter().try_for_each(|p| validate_slug(p))
}

/// Validates a file path inside a project and returns it normalized with `/` separators.
pub fn validate_rel_path(path: &str) -> Result<String, String> {
    let normalized = path.replace('\\', "/");
    let p = Path::new(&normalized);
    if normalized.is_empty() || p.is_absolute() {
        return Err(format!(
            "invalid path `{path}`: must be relative to the project folder"
        ));
    }
    for c in p.components() {
        match c {
            Component::Normal(_) => {}
            _ => {
                return Err(format!(
                    "invalid path `{path}`: `..`, `.` and roots are not allowed"
                ));
            }
        }
    }
    if normalized == MANIFEST_FILE {
        return Err(format!(
            "`{MANIFEST_FILE}` is managed through the manifest tools"
        ));
    }
    // Shader files become WESL modules, so their path components must be valid identifiers.
    if normalized.ends_with(".wesl") || normalized.ends_with(".wgsl") {
        for comp in normalized
            .trim_end_matches(".wesl")
            .trim_end_matches(".wgsl")
            .split('/')
        {
            validate_slug(comp).map_err(|e| format!("shader path `{path}`: {e}"))?;
        }
    }
    Ok(normalized)
}

pub struct Store {
    pub base: PathBuf,
}

impl Store {
    pub fn new(base: PathBuf) -> Self {
        Self { base }
    }

    pub fn project_dir(&self, id: &str) -> PathBuf {
        self.base.join(id)
    }

    pub fn scan(&self) -> Catalog {
        let mut catalog = Catalog::default();
        let Ok(entries) = fs::read_dir(&self.base) else {
            return catalog;
        };
        let mut dirs: Vec<_> = entries
            .flatten()
            .filter(|e| e.path().is_dir())
            .filter_map(|e| e.file_name().into_string().ok())
            .filter(|n| validate_slug(n).is_ok())
            .collect();
        dirs.sort();
        for name in dirs {
            let dir = self.base.join(&name);
            if dir.join(MANIFEST_FILE).exists() {
                catalog.ungrouped.push(self.load_entry(&name, None));
                continue;
            }
            let manifest = fs::read_to_string(dir.join(GROUP_FILE))
                .ok()
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_default();
            let mut projects: Vec<_> = fs::read_dir(&dir)
                .into_iter()
                .flatten()
                .flatten()
                .filter(|e| e.path().join(MANIFEST_FILE).exists())
                .filter_map(|e| e.file_name().into_string().ok())
                .filter(|n| validate_slug(n).is_ok())
                .collect();
            projects.sort();
            let projects = projects
                .into_iter()
                .map(|p| self.load_entry(&format!("{name}/{p}"), Some(name.clone())))
                .collect::<Vec<_>>();
            catalog.groups.push(GroupEntry {
                id: name,
                manifest,
                projects,
            });
        }
        catalog
    }

    fn load_entry(&self, id: &str, group: Option<String>) -> ProjectEntry {
        ProjectEntry {
            id: id.to_string(),
            group,
            manifest: self.read_manifest(id),
        }
    }

    pub fn read_manifest(&self, id: &str) -> Result<Manifest, String> {
        let path = self.project_dir(id).join(MANIFEST_FILE);
        let text = fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        serde_json::from_str(&text).map_err(|e| format!("{MANIFEST_FILE}: {e}"))
    }

    pub fn write_manifest(&self, id: &str, manifest: &Manifest) -> Result<(), String> {
        let dir = self.project_dir(id);
        fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let text = serde_json::to_string_pretty(manifest).map_err(|e| e.to_string())?;
        fs::write(dir.join(MANIFEST_FILE), text).map_err(|e| e.to_string())
    }

    pub fn project_exists(&self, id: &str) -> bool {
        self.project_dir(id).join(MANIFEST_FILE).exists()
    }

    /// Returns an error if `id` is a group folder or would be nested inside a project.
    pub fn check_project_location(&self, id: &str) -> Result<(), String> {
        validate_project_id(id)?;
        if let Some((group, _)) = id.split_once('/') {
            if self.base.join(group).join(MANIFEST_FILE).exists() {
                return Err(format!("`{group}` is a project, not a group"));
            }
        } else if self.base.join(id).is_dir() && !self.project_exists(id) {
            return Err(format!(
                "`{id}` is a group; use `{id}/<name>` for a project inside it"
            ));
        }
        Ok(())
    }

    pub fn write_file(&self, id: &str, rel: &str, bytes: &[u8]) -> Result<String, String> {
        let rel = validate_rel_path(rel)?;
        let path = self.project_dir(id).join(&rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        fs::write(&path, bytes).map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(rel)
    }

    pub fn read_file(&self, id: &str, rel: &str) -> Result<Vec<u8>, String> {
        let rel = validate_rel_path(rel)?;
        fs::read(self.project_dir(id).join(rel)).map_err(|e| e.to_string())
    }

    pub fn delete_file(&self, id: &str, rel: &str) -> Result<(), String> {
        let rel = validate_rel_path(rel)?;
        fs::remove_file(self.project_dir(id).join(rel)).map_err(|e| e.to_string())
    }

    pub fn delete_project(&self, id: &str) -> Result<(), String> {
        validate_project_id(id)?;
        if !self.project_exists(id) {
            return Err(format!("project `{id}` does not exist"));
        }
        fs::remove_dir_all(self.project_dir(id)).map_err(|e| e.to_string())?;
        // Remove an emptied group folder (ignoring group.json).
        if let Some((group, _)) = id.split_once('/') {
            let dir = self.base.join(group);
            let only_meta = fs::read_dir(&dir)
                .map(|it| it.flatten().all(|e| e.file_name() == GROUP_FILE))
                .unwrap_or(false);
            if only_meta {
                let _ = fs::remove_dir_all(dir);
            }
        }
        Ok(())
    }

    pub fn write_group(&self, group: &str, manifest: &GroupManifest) -> Result<(), String> {
        validate_slug(group)?;
        let dir = self.base.join(group);
        if dir.join(MANIFEST_FILE).exists() {
            return Err(format!("`{group}` is a project, not a group"));
        }
        fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let text = serde_json::to_string_pretty(manifest).map_err(|e| e.to_string())?;
        fs::write(dir.join(GROUP_FILE), text).map_err(|e| e.to_string())
    }

    /// Lists project files (relative paths with `/`), excluding the manifest.
    pub fn list_files(&self, id: &str) -> Vec<String> {
        fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) {
            let Ok(entries) = fs::read_dir(dir) else {
                return;
            };
            for e in entries.flatten() {
                let p = e.path();
                if p.is_dir() {
                    walk(root, &p, out);
                } else if let Ok(rel) = p.strip_prefix(root) {
                    let rel = rel.to_string_lossy().replace('\\', "/");
                    if rel != MANIFEST_FILE {
                        out.push(rel);
                    }
                }
            }
        }
        let root = self.project_dir(id);
        let mut out = Vec::new();
        walk(&root, &root, &mut out);
        out.sort();
        out
    }

    /// Every file referenced by the manifest that is missing on disk.
    pub fn missing_files(&self, id: &str, manifest: &Manifest) -> Vec<String> {
        let dir = self.project_dir(id);
        let mut missing = Vec::new();
        for obj in &manifest.objects {
            let mut refs: Vec<&String> = obj.textures.iter().collect();
            refs.extend(obj.vertex_shader.iter());
            refs.extend(obj.fragment_shader.iter());
            match &obj.mesh {
                MeshSpec::Gltf { path, .. } | MeshSpec::Custom { path } => refs.push(path),
                _ => {}
            }
            for r in refs {
                if !dir.join(r).exists() {
                    missing.push(format!("object `{}`: missing file `{r}`", obj.name));
                }
            }
        }
        missing
    }

    /// All `.wesl`/`.wgsl` files of a project with their on-disk sources (asset path -> source).
    pub fn shader_sources(&self, id: &str) -> BTreeMap<String, String> {
        self.list_files(id)
            .into_iter()
            .filter(|f| f.ends_with(".wesl") || f.ends_with(".wgsl"))
            .filter_map(|f| {
                let src = fs::read_to_string(self.project_dir(id).join(&f)).ok()?;
                Some((format!("{id}/{f}"), src))
            })
            .collect()
    }
}
