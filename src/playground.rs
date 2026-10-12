//! Main-view management, the off-screen validation sandbox and MCP request handling.

use std::{
    collections::VecDeque,
    io::Cursor,
    sync::{Arc, Mutex},
    time::Duration,
};

use async_channel::{Receiver, Sender};
use bevy::platform::time::Instant;
use bevy::{
    asset::LoadState,
    camera::{RenderTarget, visibility::RenderLayers},
    camera_controller::free_camera::{FreeCamera, FreeCameraState},
    prelude::*,
    render::{
        render_resource::TextureFormat,
        view::screenshot::{Screenshot, ScreenshotCaptured},
    },
};
use serde_json::json;

use crate::{
    bridge::{AppRequest, CompileReport, ObjectStatus, ViewTarget},
    logs,
    material::ToyMaterial,
    pipeline_status::{PipelineStatus, ShaderState},
    project::{Catalog, Manifest, Store},
    scene::{SpawnParams, SpawnedProject, project_radius, spawn_project},
};

const SANDBOX_LAYER: usize = 1;
const SANDBOX_SIZE: UVec2 = UVec2::new(768, 432);
const COMPILE_TIMEOUT: Duration = Duration::from_secs(30);
const SCREENSHOT_TIMEOUT: Duration = Duration::from_secs(5);
/// Frames to wait after everything looks compiled, so the render world has caught up.
const SETTLE_FRAMES: u32 = 3;
/// How long a pipeline may keep failing with a retryable error (unresolved import) before
/// it is reported as an error.
const STUCK_TIMEOUT: Duration = Duration::from_secs(4);

pub struct PlaygroundPlugin {
    pub store: Arc<Store>,
    pub requests: Receiver<AppRequest>,
}

impl Plugin for PlaygroundPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(StoreRes(self.store.clone()))
            .insert_resource(Requests(self.requests.clone()))
            .init_resource::<CatalogRes>()
            .init_resource::<CurrentView>()
            .init_resource::<Validator>()
            .add_systems(Startup, setup)
            .add_systems(
                Update,
                (
                    handle_requests,
                    refresh_catalog,
                    watch_view_manifests,
                    apply_view,
                    run_validator,
                    draw_grid,
                )
                    .chain()
                    .in_set(PlaygroundSystems),
            );
    }
}

#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct PlaygroundSystems;

#[derive(Resource)]
pub struct StoreRes(pub Arc<Store>);

#[derive(Resource)]
struct Requests(Receiver<AppRequest>);

#[derive(Resource, Default)]
pub struct CatalogRes {
    pub catalog: Catalog,
    pub dirty: bool,
    last_scan: Option<Instant>,
}

/// What the main window is showing.
#[derive(Resource, Default)]
pub struct CurrentView {
    pub target: Option<ViewTarget>,
    /// Set to request a (re)spawn; `move_camera` also teleports the camera.
    pub pending: Option<PendingView>,
    pub projects: Vec<ViewProject>,
    pub grid: bool,
    /// Errors from the last spawn, shown in the UI.
    pub errors: Vec<String>,
    /// Incremented every time the view is (re)spawned.
    pub generation: u64,
    manifest_revisions: Vec<(String, Option<u128>)>,
    last_revision_check: Option<Instant>,
}

pub struct PendingView {
    pub target: ViewTarget,
    pub move_camera: bool,
}

pub struct ViewProject {
    pub id: String,
    pub name: String,
    pub root: Entity,
    pub origin: Vec3,
    pub radius: f32,
}

impl CurrentView {
    pub fn open(&mut self, target: ViewTarget) {
        self.pending = Some(PendingView {
            target,
            move_camera: true,
        });
    }

    /// Respawn without moving the camera.
    pub fn reload(&mut self, target: ViewTarget) {
        if self.pending.is_none() {
            self.pending = Some(PendingView {
                target,
                move_camera: false,
            });
        }
    }

    pub fn contains(&self, id: &str) -> bool {
        // `id` may be a group whose shared modules changed.
        self.projects
            .iter()
            .any(|p| p.id == id || p.id.starts_with(&format!("{id}/")))
            || matches!(&self.target, Some(ViewTarget::Group(g)) if id.starts_with(&format!("{g}/")) || id == g)
    }
}

#[derive(Component)]
pub struct MainCamera;

#[derive(Resource)]
struct Sandbox {
    camera: Entity,
    image: Handle<Image>,
}

#[derive(Resource, Default)]
struct Validator {
    queue: VecDeque<(String, bool, Sender<CompileReport>)>,
    active: Option<Job>,
}

struct Job {
    id: String,
    preview: bool,
    reply: Sender<CompileReport>,
    started: Instant,
    log_seq: u64,
    frames: u32,
    settled: u32,
    stuck_since: Option<Instant>,
    spawned: SpawnedProject,
    manifest: Result<Manifest, String>,
    screenshot: Option<(Instant, Arc<Mutex<Option<Result<Vec<u8>, String>>>>)>,
}

fn setup(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    mut view: ResMut<CurrentView>,
    mut catalog: ResMut<CatalogRes>,
    store: Res<StoreRes>,
) {
    commands.spawn((
        Name::new("main camera"),
        MainCamera,
        Camera3d::default(),
        FreeCamera::default(),
        Transform::from_xyz(0.0, 1.5, 6.0).looking_at(Vec3::new(0.0, 1.0, 0.0), Vec3::Y),
        RenderLayers::layer(0),
    ));

    let image = images.add(Image::new_target_texture(
        SANDBOX_SIZE.x,
        SANDBOX_SIZE.y,
        TextureFormat::Rgba8UnormSrgb,
        None,
    ));
    let camera = commands
        .spawn((
            Name::new("sandbox camera"),
            Camera3d::default(),
            Camera {
                order: -1,
                is_active: false,
                ..default()
            },
            RenderTarget::Image(image.clone().into()),
            Transform::from_xyz(0.0, 1.5, 6.0).looking_at(Vec3::new(0.0, 1.0, 0.0), Vec3::Y),
            RenderLayers::layer(SANDBOX_LAYER),
        ))
        .id();
    commands.insert_resource(Sandbox { camera, image });

    // Open the first available project.
    catalog.catalog = store.0.scan();
    catalog.last_scan = Some(Instant::now());
    let first = catalog
        .catalog
        .groups
        .first()
        .map(|g| ViewTarget::Group(g.id.clone()))
        .or_else(|| {
            catalog
                .catalog
                .ungrouped
                .first()
                .map(|p| ViewTarget::Project(p.id.clone()))
        });
    if let Some(target) = first {
        view.open(target);
    }
}

fn refresh_catalog(mut catalog: ResMut<CatalogRes>, store: Res<StoreRes>) {
    let stale = catalog
        .last_scan
        .is_none_or(|t| t.elapsed() > Duration::from_secs(2));
    if catalog.dirty || stale {
        catalog.catalog = store.0.scan();
        catalog.dirty = false;
        catalog.last_scan = Some(Instant::now());
    }
}

/// Picks up manual edits to project.json of projects in the main view.
fn watch_view_manifests(mut view: ResMut<CurrentView>, store: Res<StoreRes>) {
    if view
        .last_revision_check
        .is_some_and(|t| t.elapsed() < Duration::from_millis(700))
    {
        return;
    }
    view.bypass_change_detection().last_revision_check = Some(Instant::now());
    let changed = view
        .manifest_revisions
        .iter()
        .any(|(id, revision)| store.0.manifest_revision(id) != *revision);
    if changed && let Some(target) = view.target.clone() {
        view.reload(target);
    }
}

fn apply_view(
    mut commands: Commands,
    mut view: ResMut<CurrentView>,
    catalog: Res<CatalogRes>,
    store: Res<StoreRes>,
    asset_server: Res<AssetServer>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ToyMaterial>>,
    mut camera: Query<(Entity, &mut Transform, &mut Camera), With<MainCamera>>,
) {
    if view.pending.is_none() {
        return;
    }
    let pending = view.pending.take().unwrap();
    view.generation += 1;
    for project in view.projects.drain(..) {
        commands.entity(project.root).despawn();
    }
    view.errors.clear();
    view.manifest_revisions.clear();

    // (id, origin) of every project to show.
    let mut layout: Vec<(String, Vec3)> = Vec::new();
    match &pending.target {
        ViewTarget::Project(id) => layout.push((id.clone(), Vec3::ZERO)),
        ViewTarget::Group(group) => {
            let Some(entry) = catalog.catalog.group(group) else {
                view.errors.push(format!("group `{group}` not found"));
                view.target = Some(pending.target);
                return;
            };
            let radius = entry
                .projects
                .iter()
                .filter_map(|p| p.manifest.as_ref().ok())
                .map(project_radius)
                .fold(1.0, f32::max);
            let spacing = entry.manifest.spacing.unwrap_or(radius * 2.0 + 2.0);
            let n = entry.projects.len();
            // A row for small groups, otherwise a grid.
            let cols = if n <= 6 {
                n.max(1)
            } else {
                (n as f32).sqrt().ceil() as usize
            };
            let rows = n.div_ceil(cols);
            for (i, p) in entry.projects.iter().enumerate() {
                let (col, row) = (i % cols, i / cols);
                let x = (col as f32 - (cols - 1) as f32 * 0.5) * spacing;
                let z = (row as f32 - (rows - 1) as f32 * 0.5) * spacing;
                layout.push((p.id.clone(), Vec3::new(x, 0.0, z)));
            }
        }
    }

    let mut background = None;
    let mut grid = true;
    let mut camera_spec = None;
    for (id, origin) in &layout {
        view.manifest_revisions
            .push((id.clone(), store.0.manifest_revision(id)));
        let manifest = match store.0.read_manifest(id) {
            Ok(m) => m,
            Err(e) => {
                view.errors.push(format!("{id}: {e}"));
                continue;
            }
        };
        let spawned = spawn_project(
            &mut commands,
            &asset_server,
            &mut meshes,
            &mut materials,
            SpawnParams {
                store: &store.0,
                id,
                manifest: &manifest,
                offset: *origin,
                layers: RenderLayers::layer(0),
                no_culling: false,
            },
        );
        view.errors
            .extend(spawned.errors.iter().map(|e| format!("{id}: {e}")));
        if layout.len() == 1 {
            background = manifest.background;
            grid = manifest.grid.unwrap_or(true);
            camera_spec = manifest.camera.clone();
        }
        view.projects.push(ViewProject {
            id: id.clone(),
            name: if manifest.name.is_empty() {
                id.clone()
            } else {
                manifest.name.clone()
            },
            root: spawned.root.unwrap(),
            origin: *origin,
            radius: project_radius(&manifest),
        });
    }
    view.grid = grid;
    view.target = Some(pending.target.clone());

    let Ok((entity, mut transform, mut cam)) = camera.single_mut() else {
        return;
    };
    cam.clear_color = match background {
        Some(c) => ClearColorConfig::Custom(Color::linear_rgb(c[0], c[1], c[2])),
        None => ClearColorConfig::Default,
    };
    if pending.move_camera {
        *transform = match (&pending.target, camera_spec) {
            (ViewTarget::Project(_), Some(spec)) => {
                Transform::from_translation(Vec3::from_array(spec.position))
                    .looking_at(Vec3::from_array(spec.look_at), Vec3::Y)
            }
            (ViewTarget::Project(_), None) => default_camera(1.0),
            (ViewTarget::Group(_), _) => {
                let extent = layout.iter().map(|(_, o)| o.length()).fold(0.0, f32::max);
                default_camera(extent * 0.9 + 3.0)
            }
        };
        // Re-initialize the controller so it adopts the new orientation.
        commands.entity(entity).insert(FreeCameraState::default());
    }
}

fn default_camera(distance: f32) -> Transform {
    Transform::from_xyz(0.0, 1.5 + distance * 0.25, 5.0 + distance)
        .looking_at(Vec3::new(0.0, 1.0, 0.0), Vec3::Y)
}

fn draw_grid(mut gizmos: Gizmos, view: Res<CurrentView>) {
    if !view.grid {
        return;
    }
    gizmos.grid(
        Isometry3d::from_rotation(Quat::from_rotation_x(std::f32::consts::FRAC_PI_2)),
        UVec2::splat(80),
        Vec2::splat(1.0),
        LinearRgba::gray(0.25),
    );
}

fn handle_requests(
    mut commands: Commands,
    requests: Res<Requests>,
    mut view: ResMut<CurrentView>,
    mut catalog: ResMut<CatalogRes>,
    mut validator: ResMut<Validator>,
    store: Res<StoreRes>,
    asset_server: Res<AssetServer>,
) {
    while let Ok(request) = requests.0.try_recv() {
        match request {
            AppRequest::Compile { id, preview, reply } => {
                validator.queue.push_back((id, preview, reply));
            }
            AppRequest::Open { target, reply } => {
                catalog.catalog = store.0.scan();
                let result = match &target {
                    ViewTarget::Project(id) => catalog
                        .catalog
                        .find(id)
                        .map(|p| format!("opened project `{id}` ({})", p.display_name()))
                        .ok_or_else(|| format!("project `{id}` not found")),
                    ViewTarget::Group(g) => catalog
                        .catalog
                        .group(g)
                        .map(|e| format!("opened group `{g}` with {} projects", e.projects.len()))
                        .ok_or_else(|| format!("group `{g}` not found")),
                };
                if result.is_ok() {
                    view.open(target);
                }
                let _ = reply.try_send(result);
            }
            AppRequest::Screenshot { reply } => {
                commands.spawn(Screenshot::primary_window()).observe(
                    move |captured: On<ScreenshotCaptured>| {
                        let _ = reply.try_send(encode_png(&captured.image));
                    },
                );
            }
            AppRequest::State { reply } => {
                let _ = reply.try_send(state_json(&view));
            }
            AppRequest::ProjectChanged { id } => {
                catalog.dirty = true;
                // Reload already-loaded assets of the project (shaders, textures, glTF).
                for file in store.0.list_files(&id) {
                    asset_server.reload(format!("{id}/{file}"));
                }
                if view.contains(&id)
                    && let Some(target) = view.target.clone()
                {
                    view.reload(target);
                }
            }
            AppRequest::Shutdown => {
                commands.queue(|world: &mut World| {
                    world.write_message(AppExit::Success);
                });
            }
        }
    }
}

fn state_json(view: &CurrentView) -> serde_json::Value {
    let target = match &view.target {
        Some(ViewTarget::Project(id)) => json!({ "project": id }),
        Some(ViewTarget::Group(g)) => json!({ "group": g }),
        None => json!(null),
    };
    let recent: Vec<_> = logs::recent(30)
        .into_iter()
        .map(|e| format!("[{}] {}: {}", e.level, e.target, e.message))
        .collect();
    json!({
        "showing": target,
        "projects_in_view": view.projects.iter().map(|p| json!({
            "id": p.id, "name": p.name, "origin": p.origin.to_array(),
        })).collect::<Vec<_>>(),
        "view_errors": view.errors,
        "recent_log_warnings_and_errors": recent,
    })
}

fn encode_png(image: &Image) -> Result<Vec<u8>, String> {
    let dynamic = image
        .clone()
        .try_into_dynamic()
        .map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    dynamic
        .to_rgba8()
        .write_to(&mut Cursor::new(&mut bytes), image::ImageFormat::Png)
        .map_err(|e| e.to_string())?;
    Ok(bytes)
}

fn run_validator(
    mut commands: Commands,
    mut validator: ResMut<Validator>,
    sandbox: Res<Sandbox>,
    store: Res<StoreRes>,
    asset_server: Res<AssetServer>,
    shaders: Res<Assets<Shader>>,
    status: Res<PipelineStatus>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ToyMaterial>>,
    mut cameras: Query<(&mut Camera, &mut Transform)>,
) {
    let validator = &mut *validator;
    if validator.active.is_none() {
        let Some((id, preview, reply)) = validator.queue.pop_front() else {
            return;
        };
        let manifest = store.0.read_manifest(&id);
        let mut spawned = SpawnedProject::default();
        if let Ok(manifest) = &manifest {
            spawned = spawn_project(
                &mut commands,
                &asset_server,
                &mut meshes,
                &mut materials,
                SpawnParams {
                    store: &store.0,
                    id: &id,
                    manifest,
                    offset: Vec3::ZERO,
                    layers: RenderLayers::layer(SANDBOX_LAYER),
                    no_culling: true,
                },
            );
            if let Ok((mut camera, mut transform)) = cameras.get_mut(sandbox.camera) {
                camera.is_active = true;
                camera.clear_color = match manifest.background {
                    Some(c) => ClearColorConfig::Custom(Color::linear_rgb(c[0], c[1], c[2])),
                    None => ClearColorConfig::Default,
                };
                *transform = match &manifest.camera {
                    Some(spec) => Transform::from_translation(Vec3::from_array(spec.position))
                        .looking_at(Vec3::from_array(spec.look_at), Vec3::Y),
                    None => default_camera(1.0),
                };
            }
        }
        validator.active = Some(Job {
            id,
            preview,
            reply,
            started: Instant::now(),
            log_seq: logs::next_seq(),
            frames: 0,
            settled: 0,
            stuck_since: None,
            spawned,
            manifest,
            screenshot: None,
        });
        return;
    }

    let job = validator.active.as_mut().unwrap();
    job.frames += 1;
    let timed_out = job.started.elapsed() > COMPILE_TIMEOUT;

    // Phase 2: waiting for the preview screenshot.
    if let Some((requested, slot)) = &job.screenshot {
        let result = slot.lock().unwrap().take();
        if result.is_none() && requested.elapsed() < SCREENSHOT_TIMEOUT {
            return;
        }
        let job = validator.active.take().unwrap();
        let mut report = build_report(&job, &store.0, &asset_server, &shaders, &status, false);
        match result {
            Some(Ok(png)) => report.preview_png = Some(png),
            Some(Err(e)) => report.warnings.push(format!("preview failed: {e}")),
            None => report.warnings.push("preview screenshot timed out".into()),
        }
        finish(&mut commands, &sandbox, &mut cameras, job, report);
        return;
    }

    let readiness = if job.frames >= 3 {
        readiness(job, &store.0, &asset_server, &shaders, &status)
    } else {
        Readiness::Waiting
    };
    let ready = match readiness {
        Readiness::Ready => true,
        Readiness::Stuck => {
            job.stuck_since.get_or_insert_with(Instant::now).elapsed() > STUCK_TIMEOUT
        }
        Readiness::Waiting => {
            job.stuck_since = None;
            false
        }
    };
    if ready {
        job.settled += 1;
    } else {
        job.settled = 0;
    }
    if job.settled < SETTLE_FRAMES && !timed_out {
        return;
    }

    let report = build_report(job, &store.0, &asset_server, &shaders, &status, timed_out);
    if job.preview && job.manifest.is_ok() {
        let slot: Arc<Mutex<Option<Result<Vec<u8>, String>>>> = Arc::default();
        let sink = slot.clone();
        commands
            .spawn(Screenshot::image(sandbox.image.clone()))
            .observe(move |captured: On<ScreenshotCaptured>| {
                *sink.lock().unwrap() = Some(encode_png(&captured.image));
            });
        job.screenshot = Some((Instant::now(), slot));
        return;
    }
    let job = validator.active.take().unwrap();
    finish(&mut commands, &sandbox, &mut cameras, job, report);
}

fn finish(
    commands: &mut Commands,
    sandbox: &Sandbox,
    cameras: &mut Query<(&mut Camera, &mut Transform)>,
    job: Job,
    report: CompileReport,
) {
    if let Some(root) = job.spawned.root {
        commands.entity(root).despawn();
    }
    if let Ok((mut camera, _)) = cameras.get_mut(sandbox.camera) {
        camera.is_active = false;
    }
    let _ = job.reply.try_send(report);
}

/// Loaded asset source of a shader matches the file on disk (i.e. reloads have landed).
fn shader_sources_current(
    id: &str,
    store: &Store,
    asset_server: &AssetServer,
    shaders: &Assets<Shader>,
) -> bool {
    store.shader_sources(id).iter().all(|(path, disk)| {
        match asset_server.get_handle::<Shader>(path.clone()) {
            None => true,
            Some(handle) => match asset_server.get_load_state(&handle) {
                Some(LoadState::Failed(_)) => true,
                _ => shaders
                    .get(&handle)
                    .is_some_and(|s| s.source.as_str() == disk),
            },
        }
    })
}

/// Shortens WESL parser errors, which list every acceptable token ("expected `\"&\", \"!\", ...`").
fn compact_error(message: &str) -> String {
    const MARKER: &str = "expected `";
    let mut out = String::with_capacity(message.len());
    let mut rest = message;
    while let Some(start) = rest.find(MARKER) {
        let body_start = start + MARKER.len();
        out.push_str(&rest[..body_start]);
        let body = &rest[body_start..];
        match body.find("`\n").or_else(|| body.rfind('`')) {
            Some(end) if end > 80 => {
                out.push_str("<expression or statement>");
                rest = &body[end..];
            }
            _ => rest = body,
        }
    }
    out.push_str(rest);
    out
}

enum Readiness {
    Waiting,
    /// Everything is loaded but some pipeline keeps failing with a retryable error.
    Stuck,
    Ready,
}

fn readiness(
    job: &Job,
    store: &Store,
    asset_server: &AssetServer,
    shaders: &Assets<Shader>,
    status: &PipelineStatus,
) -> Readiness {
    if job.manifest.is_err() {
        return Readiness::Ready;
    }
    let settled = |state: Option<LoadState>| {
        matches!(state, Some(LoadState::Loaded) | Some(LoadState::Failed(_)))
    };
    let assets_done = job
        .spawned
        .images
        .iter()
        .all(|(_, h)| settled(asset_server.get_load_state(h)))
        && job.spawned.meshes.iter().all(|(_, h)| {
            // Meshes built in code have no load state.
            asset_server
                .get_load_state(h)
                .is_none_or(|s| settled(Some(s)))
        });
    if !assets_done || !shader_sources_current(&job.id, store, asset_server, shaders) {
        return Readiness::Waiting;
    }
    let mut stuck = false;
    for (_, handle) in &job.spawned.entry_shaders {
        if let Some(LoadState::Failed(_)) = asset_server.get_load_state(handle) {
            continue;
        }
        match status.get(handle.id()) {
            Some(ShaderState::Ok | ShaderState::Err(_)) => {}
            Some(ShaderState::Pending(Some(_))) => stuck = true,
            _ => return Readiness::Waiting,
        }
    }
    if stuck {
        Readiness::Stuck
    } else {
        Readiness::Ready
    }
}

/// Explains unresolved imports of a shader: project-relative imports whose file is missing.
fn diagnose_imports(handle: &Handle<Shader>, store: &Store, shaders: &Assets<Shader>) -> String {
    use bevy::shader::ShaderImport;
    let Some(shader) = shaders.get(handle) else {
        return String::new();
    };
    let exists = |path: &str| {
        let rel = path.trim_start_matches('/');
        store.file_exists(&format!("{rel}.wesl")) || store.file_exists(&format!("{rel}.wgsl"))
    };
    let mut missing = Vec::new();
    let mut packages = Vec::new();
    for import in &shader.imports {
        match import {
            // Imports are recorded both as `module` and `module/item`; one of them must exist.
            ShaderImport::AssetPath(path) => {
                let parent = path.rsplit_once('/').map_or("", |(p, _)| p);
                if !exists(path) && !exists(parent) && !missing.contains(path) {
                    missing.push(path.clone());
                }
            }
            ShaderImport::Custom(name) => packages.push(name.clone()),
        }
    }
    // `a/b/c` is reported through `a/b` when both are missing (same import seen at two depths).
    let leaves: Vec<_> = missing
        .iter()
        .filter(|m| !missing.iter().any(|o| m.starts_with(&format!("{o}/"))))
        .map(|m| format!("{}.wesl", m.trim_start_matches('/')))
        .collect();
    // Same for packages: `pkg::module::item` implies `pkg::module`, keep the full forms.
    let packages: Vec<_> = packages
        .iter()
        .filter(|p| !packages.iter().any(|o| o.starts_with(&format!("{p}::"))))
        .cloned()
        .collect();
    if !leaves.is_empty() {
        format!("missing module file(s): {}", leaves.join(", "))
    } else {
        format!(
            "an imported item or module does not exist; check these imports: {}",
            packages.join(", ")
        )
    }
}

fn build_report(
    job: &Job,
    store: &Store,
    asset_server: &AssetServer,
    shaders: &Assets<Shader>,
    status: &PipelineStatus,
    timed_out: bool,
) -> CompileReport {
    let mut report = CompileReport {
        project: job.id.clone(),
        timed_out,
        ..default()
    };
    if let Err(e) = &job.manifest {
        report.errors.push(e.clone());
        return report;
    }
    report.errors.extend(job.spawned.errors.iter().cloned());

    let load_error = |state: Option<LoadState>| match state {
        Some(LoadState::Failed(e)) => Some(e.to_string()),
        _ => None,
    };
    for (label, handle) in &job.spawned.images {
        if let Some(e) = load_error(asset_server.get_load_state(handle)) {
            report.errors.push(format!("texture {label}: {e}"));
        }
    }
    for (label, handle) in &job.spawned.meshes {
        if let Some(e) = load_error(asset_server.get_load_state(handle)) {
            report.errors.push(format!("mesh of `{label}`: {e}"));
        }
    }
    if !shader_sources_current(&job.id, store, asset_server, shaders) {
        report.warnings.push(
            "some shader files were still reloading; results may be stale, compile again".into(),
        );
    }

    let mut seen_errors = Vec::new();
    for (label, handle) in &job.spawned.entry_shaders {
        let shader = handle
            .path()
            .map(|p| p.to_string())
            .unwrap_or_else(|| format!("{:?}", handle.id()));
        let (state, error) = match load_error(asset_server.get_load_state(handle)) {
            Some(e) => ("error", Some(format!("failed to load: {e}"))),
            None => match status.get(handle.id()) {
                Some(ShaderState::Ok) => ("ok", None),
                Some(ShaderState::Err(e)) => ("error", Some(e)),
                Some(ShaderState::Pending(Some(e))) => (
                    "error",
                    Some(format!(
                        "{e} -> {}",
                        diagnose_imports(handle, store, shaders)
                    )),
                ),
                Some(ShaderState::Pending(None)) => ("pending", None),
                None => (
                    "pending",
                    Some("no pipeline was created for this object".into()),
                ),
            },
        };
        if let Some(e) = error.map(|e| compact_error(&e))
            && state != "ok"
            && !seen_errors.contains(&e)
        {
            report
                .errors
                .push(format!("{shader} (object `{label}`): {e}"));
            seen_errors.push(e);
        }
        report.objects.push(ObjectStatus {
            object: label.clone(),
            shader,
            status: state.into(),
        });
    }

    // Anything the renderer logged about this project (or a GPU validation error) while compiling.
    // The pipeline cache's own logs are already covered by the pipeline status above.
    let path_hint = format!("{}/", job.id);
    for entry in logs::since(job.log_seq, |e| {
        e.level == "ERROR"
            && !e.target.contains("pipeline_cache")
            && (e.message.contains(&path_hint) || e.message.contains("rendering error"))
    }) {
        let message = format!("log: {}", compact_error(&entry.message));
        if !report.errors.contains(&message) {
            report.errors.push(message);
        }
    }

    report.ok =
        report.errors.is_empty() && !timed_out && report.objects.iter().all(|o| o.status == "ok");
    if timed_out {
        report.errors.push(format!(
            "timed out after {}s waiting for pipelines",
            COMPILE_TIMEOUT.as_secs()
        ));
    }
    report
}
