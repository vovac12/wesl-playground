//! Minimal MCP server (Streamable HTTP transport, JSON responses only) running inside the app.
//!
//! Register with Claude Code:
//! `claude mcp add --transport http shadertoy http://127.0.0.1:7878/mcp`

use std::{collections::BTreeMap, sync::Arc, thread, time::Duration};

use base64::{Engine, engine::general_purpose::STANDARD as BASE64};
use crossbeam_channel::{Sender, bounded};
use serde_json::{Value, json};
use tiny_http::{Header, Method, Request, Response, Server};

use crate::{
    bridge::{AppRequest, CompileReport, ViewTarget},
    project::{
        GroupManifest, Manifest, Store, validate_project_id, validate_rel_path, validate_slug,
    },
    textures::{Procedural, generate_png},
};

pub const GUIDE: &str = include_str!("guide.md");
const PROTOCOL_VERSION: &str = "2025-06-18";
const COMPILE_WAIT: Duration = Duration::from_secs(60);
const APP_WAIT: Duration = Duration::from_secs(10);

#[derive(Clone)]
pub struct McpServer {
    store: Arc<Store>,
    app: Sender<AppRequest>,
}

pub fn spawn(addr: String, store: Arc<Store>, app: Sender<AppRequest>) {
    let server = match Server::http(&addr) {
        Ok(server) => server,
        Err(e) => {
            bevy::log::error!("MCP server failed to bind {addr}: {e}");
            return;
        }
    };
    bevy::log::info!("MCP server listening on http://{addr}/mcp");
    let mcp = McpServer { store, app };
    thread::Builder::new()
        .name("mcp-http".into())
        .spawn(move || {
            for request in server.incoming_requests() {
                let mcp = mcp.clone();
                // Compiles can take a while; don't block other requests.
                thread::spawn(move || mcp.handle_http(request));
            }
        })
        .expect("spawn MCP thread");
}

fn json_header() -> Header {
    Header::from_bytes("Content-Type", "application/json").unwrap()
}

impl McpServer {
    fn handle_http(&self, mut request: Request) {
        if request.url().split('?').next() != Some("/mcp") {
            let _ = request.respond(Response::from_string("not found").with_status_code(404));
            return;
        }
        match request.method() {
            Method::Post => {}
            Method::Delete => {
                let _ = request.respond(Response::empty(200));
                return;
            }
            // No server-initiated streams.
            _ => {
                let _ = request.respond(Response::empty(405));
                return;
            }
        }
        let mut body = String::new();
        if request.as_reader().read_to_string(&mut body).is_err() {
            let _ = request.respond(Response::empty(400));
            return;
        }
        let response = match serde_json::from_str::<Value>(&body) {
            Err(e) => Some(rpc_error(Value::Null, -32700, &format!("parse error: {e}"))),
            Ok(Value::Array(batch)) => {
                let out: Vec<Value> = batch
                    .into_iter()
                    .filter_map(|m| self.handle_message(m))
                    .collect();
                (!out.is_empty()).then_some(Value::Array(out))
            }
            Ok(message) => self.handle_message(message),
        };
        let _ = match response {
            Some(value) => {
                request.respond(Response::from_string(value.to_string()).with_header(json_header()))
            }
            // Notifications and responses get 202 with no body.
            None => request.respond(Response::empty(202)),
        };
    }

    fn handle_message(&self, message: Value) -> Option<Value> {
        let id = message.get("id").cloned();
        let method = message
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let params = message.get("params").cloned().unwrap_or(json!({}));
        let id = id?; // notification
        let result = match method {
            "initialize" => {
                let version = params
                    .get("protocolVersion")
                    .and_then(Value::as_str)
                    .unwrap_or(PROTOCOL_VERSION);
                Ok(json!({
                    "protocolVersion": version,
                    "capabilities": { "tools": { "listChanged": false } },
                    "serverInfo": { "name": "shadertoy", "version": env!("CARGO_PKG_VERSION") },
                    "instructions": GUIDE,
                }))
            }
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({ "tools": tool_list() })),
            "tools/call" => {
                let name = params
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let args = params.get("arguments").cloned().unwrap_or(json!({}));
                Ok(match self.call_tool(name, &args) {
                    Ok(content) => json!({ "content": content, "isError": false }),
                    Err(e) => json!({ "content": [text(&e)], "isError": true }),
                })
            }
            other => Err((-32601, format!("method not found: {other}"))),
        };
        Some(match result {
            Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
            Err((code, msg)) => rpc_error(id, code, &msg),
        })
    }

    fn call_tool(&self, name: &str, args: &Value) -> Result<Vec<Value>, String> {
        match name {
            "shader_guide" => Ok(vec![text(GUIDE)]),
            "list_projects" => self.list_projects(),
            "get_project" => self.get_project(args),
            "create_project" => self.create_project(args),
            "update_project" => self.update_project(args),
            "delete_project" => {
                let id = str_arg(args, "id")?;
                self.store.delete_project(id)?;
                self.notify(id);
                Ok(vec![text(&format!("deleted `{id}`"))])
            }
            "set_group" => self.set_group(args),
            "add_texture" => self.add_texture(args),
            "import_file" => self.import_file(args),
            "compile_project" => {
                let id = str_arg(args, "id")?;
                self.require_project(id)?;
                self.compile(id, bool_arg(args, "preview", true))
            }
            "open_in_viewer" => self.open_in_viewer(args),
            "screenshot" => {
                let (tx, rx) = bounded(1);
                self.send(AppRequest::Screenshot { reply: tx })?;
                let png = rx
                    .recv_timeout(APP_WAIT)
                    .map_err(|_| "viewer did not respond")??;
                Ok(vec![image(&png)])
            }
            "viewer_state" => {
                let (tx, rx) = bounded(1);
                self.send(AppRequest::State { reply: tx })?;
                let state = rx
                    .recv_timeout(APP_WAIT)
                    .map_err(|_| "viewer did not respond")?;
                Ok(vec![text(&serde_json::to_string_pretty(&state).unwrap())])
            }
            other => Err(format!("unknown tool `{other}`")),
        }
    }

    fn send(&self, request: AppRequest) -> Result<(), String> {
        self.app
            .send(request)
            .map_err(|_| "viewer is shutting down".to_string())
    }

    fn notify(&self, id: &str) {
        let _ = self
            .app
            .send(AppRequest::ProjectChanged { id: id.to_string() });
    }

    fn require_project(&self, id: &str) -> Result<(), String> {
        validate_project_id(id)?;
        if self.store.project_exists(id) {
            Ok(())
        } else {
            Err(format!("project `{id}` does not exist (see list_projects)"))
        }
    }

    fn compile(&self, id: &str, preview: bool) -> Result<Vec<Value>, String> {
        let (tx, rx) = bounded(1);
        self.send(AppRequest::Compile {
            id: id.to_string(),
            preview,
            reply: tx,
        })?;
        let report: CompileReport = rx
            .recv_timeout(COMPILE_WAIT)
            .map_err(|_| "viewer did not finish compiling in time (is it busy or minimized?)")?;
        let mut out = vec![text(&serde_json::to_string_pretty(&report).unwrap())];
        if let Some(png) = &report.preview_png {
            out.push(image(png));
        }
        Ok(out)
    }

    fn list_projects(&self) -> Result<Vec<Value>, String> {
        let catalog = self.store.scan();
        let project = |p: &crate::project::ProjectEntry| match &p.manifest {
            Ok(m) => json!({
                "id": p.id, "name": m.name, "description": m.description,
                "objects": m.objects.len(),
            }),
            Err(e) => json!({ "id": p.id, "manifest_error": e }),
        };
        let value = json!({
            "base_folder": self.store.base.display().to_string(),
            "groups": catalog.groups.iter().map(|g| json!({
                "id": g.id,
                "name": g.manifest.name,
                "description": g.manifest.description,
                "projects": g.projects.iter().map(project).collect::<Vec<_>>(),
            })).collect::<Vec<_>>(),
            "ungrouped": catalog.ungrouped.iter().map(project).collect::<Vec<_>>(),
        });
        Ok(vec![text(&serde_json::to_string_pretty(&value).unwrap())])
    }

    fn get_project(&self, args: &Value) -> Result<Vec<Value>, String> {
        let id = str_arg(args, "id")?;
        self.require_project(id)?;
        let manifest = self.store.read_manifest(id);
        let files = self.store.list_files(id);
        let mut sources = BTreeMap::new();
        if bool_arg(args, "include_sources", true) {
            for f in &files {
                let is_text = [".wesl", ".wgsl", ".json", ".txt", ".md"]
                    .iter()
                    .any(|e| f.ends_with(e));
                if is_text && let Ok(bytes) = self.store.read_file(id, f) {
                    sources.insert(f.clone(), String::from_utf8_lossy(&bytes).into_owned());
                }
            }
        }
        let value = json!({
            "id": id,
            "manifest": match &manifest { Ok(m) => json!(m), Err(e) => json!({ "error": e }) },
            "files": files,
            "sources": sources,
        });
        Ok(vec![text(&serde_json::to_string_pretty(&value).unwrap())])
    }

    fn create_project(&self, args: &Value) -> Result<Vec<Value>, String> {
        let id = str_arg(args, "id")?;
        self.store.check_project_location(id)?;
        let manifest = manifest_arg(args)?.ok_or("`manifest` is required")?;
        if self.store.project_exists(id) {
            if !bool_arg(args, "overwrite", false) {
                return Err(format!(
                    "project `{id}` already exists; use update_project or pass overwrite: true"
                ));
            }
            self.store.delete_project(id)?;
        }
        if let Some((group, _)) = id.split_once('/')
            && let Some(group_name) = args.get("group_name").and_then(Value::as_str)
        {
            self.store.write_group(
                group,
                &GroupManifest {
                    name: group_name.to_string(),
                    ..Default::default()
                },
            )?;
        }
        let files = files_arg(args)?;
        for (path, content) in &files {
            if let Some(content) = content {
                self.store.write_file(id, path, content.as_bytes())?;
            }
        }
        self.store.write_manifest(id, &manifest)?;
        self.notify(id);
        self.after_write(id, args)
    }

    fn update_project(&self, args: &Value) -> Result<Vec<Value>, String> {
        let id = str_arg(args, "id")?;
        self.require_project(id)?;
        for (path, content) in files_arg(args)? {
            match content {
                Some(content) => {
                    self.store.write_file(id, &path, content.as_bytes())?;
                }
                None => self.store.delete_file(id, &path)?,
            }
        }
        if let Some(manifest) = manifest_arg(args)? {
            self.store.write_manifest(id, &manifest)?;
        }
        self.notify(id);
        self.after_write(id, args)
    }

    fn after_write(&self, id: &str, args: &Value) -> Result<Vec<Value>, String> {
        if bool_arg(args, "compile", true) {
            self.compile(id, bool_arg(args, "preview", false))
        } else {
            Ok(vec![text(&format!("saved `{id}` (not compiled)"))])
        }
    }

    fn set_group(&self, args: &Value) -> Result<Vec<Value>, String> {
        let group = str_arg(args, "group")?;
        validate_slug(group)?;
        let manifest = GroupManifest {
            name: args
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            description: args
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            spacing: args
                .get("spacing")
                .and_then(Value::as_f64)
                .map(|v| v as f32),
        };
        self.store.write_group(group, &manifest)?;
        self.notify(group);
        Ok(vec![text(&format!("group `{group}` saved"))])
    }

    fn add_texture(&self, args: &Value) -> Result<Vec<Value>, String> {
        let id = str_arg(args, "project")?;
        self.require_project(id)?;
        let path = validate_rel_path(str_arg(args, "path")?)?;
        let bytes = if let Some(p) = args.get("procedural") {
            if !path.ends_with(".png") {
                return Err("procedural textures are PNG; use a `.png` path".into());
            }
            let p: Procedural = serde_json::from_value(p.clone()).map_err(|e| e.to_string())?;
            generate_png(&p)?
        } else {
            binary_source(args)?
        };
        let size = bytes.len();
        self.store.write_file(id, &path, &bytes)?;
        self.notify(id);
        Ok(vec![text(&format!(
            "wrote `{path}` ({size} bytes) to `{id}`; reference it in an object's `textures`"
        ))])
    }

    fn import_file(&self, args: &Value) -> Result<Vec<Value>, String> {
        let id = str_arg(args, "project")?;
        self.require_project(id)?;
        let path = str_arg(args, "path")?;
        let bytes = binary_source(args)?;
        let rel = self.store.write_file(id, path, &bytes)?;
        self.notify(id);
        Ok(vec![text(&format!(
            "wrote `{rel}` ({} bytes) to `{id}`",
            bytes.len()
        ))])
    }

    fn open_in_viewer(&self, args: &Value) -> Result<Vec<Value>, String> {
        let target = match (
            args.get("project").and_then(Value::as_str),
            args.get("group").and_then(Value::as_str),
        ) {
            (Some(p), None) => ViewTarget::Project(p.to_string()),
            (None, Some(g)) => ViewTarget::Group(g.to_string()),
            _ => return Err("pass exactly one of `project` or `group`".into()),
        };
        let (tx, rx) = bounded(1);
        self.send(AppRequest::Open { target, reply: tx })?;
        let msg = rx
            .recv_timeout(APP_WAIT)
            .map_err(|_| "viewer did not respond")??;
        Ok(vec![text(&msg)])
    }
}

fn binary_source(args: &Value) -> Result<Vec<u8>, String> {
    if let Some(b64) = args.get("base64").and_then(Value::as_str) {
        let b64 = b64.split_once("base64,").map_or(b64, |(_, data)| data);
        BASE64
            .decode(b64.trim())
            .map_err(|e| format!("invalid base64: {e}"))
    } else if let Some(src) = args.get("source_path").and_then(Value::as_str) {
        std::fs::read(src).map_err(|e| format!("{src}: {e}"))
    } else {
        Err("provide one of `procedural`, `base64` or `source_path`".into())
    }
}

fn manifest_arg(args: &Value) -> Result<Option<Manifest>, String> {
    match args.get("manifest") {
        None | Some(Value::Null) => Ok(None),
        // Some clients send nested objects as JSON strings.
        Some(Value::String(s)) => serde_json::from_str(s)
            .map(Some)
            .map_err(|e| format!("manifest: {e}")),
        Some(v) => serde_json::from_value(v.clone())
            .map(Some)
            .map_err(|e| format!("manifest: {e}")),
    }
}

fn files_arg(args: &Value) -> Result<Vec<(String, Option<String>)>, String> {
    let files = match args.get("files") {
        None | Some(Value::Null) => return Ok(Vec::new()),
        Some(Value::String(s)) => serde_json::from_str(s).map_err(|e| format!("files: {e}"))?,
        Some(v) => v.clone(),
    };
    let Value::Object(map) = files else {
        return Err("`files` must be an object of { \"path\": \"content\" | null }".into());
    };
    map.into_iter()
        .map(|(path, content)| {
            let path = validate_rel_path(&path)?;
            match content {
                Value::String(s) => Ok((path, Some(s))),
                Value::Null => Ok((path, None)),
                _ => Err(format!("file `{path}`: content must be a string or null")),
            }
        })
        .collect()
}

fn str_arg<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    args.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("missing string argument `{key}`"))
}

fn bool_arg(args: &Value, key: &str, default: bool) -> bool {
    args.get(key).and_then(Value::as_bool).unwrap_or(default)
}

fn text(s: &str) -> Value {
    json!({ "type": "text", "text": s })
}

fn image(png: &[u8]) -> Value {
    json!({ "type": "image", "data": BASE64.encode(png), "mimeType": "image/png" })
}

fn rpc_error(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

fn tool_list() -> Value {
    let id = json!({ "type": "string", "description": "Project id: `name` or `group/name`, lowercase snake_case" });
    let manifest = json!({
        "type": "object",
        "description": "Full project.json content (see shader_guide): name, description, camera, background, grid, objects[]",
    });
    let files = json!({
        "type": "object",
        "description": "Map of relative path -> file text (e.g. {\"portal.wesl\": \"...\"}). In update_project, null deletes the file.",
        "additionalProperties": { "type": ["string", "null"] },
    });
    json!([
        {
            "name": "shader_guide",
            "description": "Returns the playground guide: manifest format, WESL rules, available bindings, fragment/vertex templates. Read it before writing shaders.",
            "inputSchema": { "type": "object", "properties": {} },
        },
        {
            "name": "list_projects",
            "description": "List all groups and projects in the base folder.",
            "inputSchema": { "type": "object", "properties": {} },
        },
        {
            "name": "get_project",
            "description": "Get a project's manifest, file list and text file sources.",
            "inputSchema": {
                "type": "object",
                "properties": { "id": id, "include_sources": { "type": "boolean", "default": true } },
                "required": ["id"],
            },
        },
        {
            "name": "create_project",
            "description": "Create a project (manifest + shader/text files), then compile it off-screen and return errors. Works independently of what the viewer shows. Put variants in one group with ids like `portals/v1_swirl`.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": id,
                    "manifest": manifest,
                    "files": files,
                    "group_name": { "type": "string", "description": "Optional display name for the group (when id has a group)" },
                    "overwrite": { "type": "boolean", "default": false, "description": "Replace an existing project entirely" },
                    "compile": { "type": "boolean", "default": true },
                    "preview": { "type": "boolean", "default": false, "description": "Also return a rendered preview image" },
                },
                "required": ["id", "manifest"],
            },
        },
        {
            "name": "update_project",
            "description": "Write/delete files and/or replace the manifest of an existing project, then compile it and return errors.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": id,
                    "manifest": manifest,
                    "files": files,
                    "compile": { "type": "boolean", "default": true },
                    "preview": { "type": "boolean", "default": false },
                },
                "required": ["id"],
            },
        },
        {
            "name": "delete_project",
            "description": "Delete a project folder.",
            "inputSchema": { "type": "object", "properties": { "id": id }, "required": ["id"] },
        },
        {
            "name": "set_group",
            "description": "Create or update a group's display name, description and spacing (distance between projects when the whole group is shown).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "group": { "type": "string" },
                    "name": { "type": "string" },
                    "description": { "type": "string" },
                    "spacing": { "type": "number" },
                },
                "required": ["group"],
            },
        },
        {
            "name": "add_texture",
            "description": "Add a texture to a project: procedural PNG, base64 image data, or a copy of a local image file.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "project": id,
                    "path": { "type": "string", "description": "e.g. textures/noise.png" },
                    "procedural": {
                        "type": "object",
                        "properties": {
                            "kind": { "type": "string", "enum": ["noise", "fbm", "voronoi", "checker", "gradient", "radial", "stripes"] },
                            "size": { "type": "integer", "default": 256 },
                            "scale": { "type": "number", "default": 8 },
                            "seed": { "type": "integer", "default": 0 },
                            "color_a": { "type": "array", "items": { "type": "number" }, "description": "linear RGBA at t=0" },
                            "color_b": { "type": "array", "items": { "type": "number" }, "description": "linear RGBA at t=1" },
                        },
                    },
                    "base64": { "type": "string" },
                    "source_path": { "type": "string", "description": "Absolute path of a local file to copy" },
                },
                "required": ["project", "path"],
            },
        },
        {
            "name": "import_file",
            "description": "Copy a binary file (e.g. .glb mesh, image) into a project from a local path or base64.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "project": id,
                    "path": { "type": "string" },
                    "base64": { "type": "string" },
                    "source_path": { "type": "string" },
                },
                "required": ["project", "path"],
            },
        },
        {
            "name": "compile_project",
            "description": "Compile a project off-screen (does not change the viewer) and return shader/asset errors plus an optional preview image.",
            "inputSchema": {
                "type": "object",
                "properties": { "id": id, "preview": { "type": "boolean", "default": true } },
                "required": ["id"],
            },
        },
        {
            "name": "open_in_viewer",
            "description": "Show a project, or a whole group side by side, in the user's viewer window. Changes what the user sees; use when asked.",
            "inputSchema": {
                "type": "object",
                "properties": { "project": { "type": "string" }, "group": { "type": "string" } },
            },
        },
        {
            "name": "screenshot",
            "description": "Capture the viewer window as the user currently sees it.",
            "inputSchema": { "type": "object", "properties": {} },
        },
        {
            "name": "viewer_state",
            "description": "What the viewer is showing, its errors, and recent warnings/errors from the log.",
            "inputSchema": { "type": "object", "properties": {} },
        },
    ])
}
