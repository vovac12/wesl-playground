//! MCP server core: JSON-RPC handling and tools, independent of the transport.
//!
//! - Native: Streamable HTTP server (`http.rs`), e.g.
//!   `claude mcp add --transport http shadertoy http://127.0.0.1:7878/mcp`
//! - Browser: the page's JavaScript owns the transport (WebSocket to the plugin's bridge, or
//!   MCP Apps host messages) and hands messages to [`McpServer::handle_json`] through
//!   `web_api::mcp_handle`.

#[cfg(not(target_arch = "wasm32"))]
mod http;

#[cfg(not(target_arch = "wasm32"))]
pub use http::spawn;

use std::{collections::BTreeMap, sync::Arc, time::Duration};

use async_channel::{Receiver, Sender, bounded};
use base64::{Engine, engine::general_purpose::STANDARD as BASE64};
use serde_json::{Value, json};

use crate::{
    bridge::{AppRequest, CompileReport, ViewTarget},
    project::{
        GroupManifest, Manifest, SHARED_DIR, Store, validate_project_id, validate_rel_path,
        validate_slug,
    },
    textures::{Procedural, generate_png},
};

pub const GUIDE: &str = include_str!("../../plugin/server/guide.md");
const TOOLS: &str = include_str!("../../plugin/server/tools.json");
const PROTOCOL_VERSION: &str = "2025-06-18";
const COMPILE_WAIT: Duration = Duration::from_secs(60);
const APP_WAIT: Duration = Duration::from_secs(10);

#[derive(Clone)]
pub struct McpServer {
    store: Arc<Store>,
    app: Sender<AppRequest>,
}

/// Waits for the app's reply, giving up after `timeout`.
async fn wait<T>(rx: Receiver<T>, timeout: Duration) -> Result<T, String> {
    let reply = async {
        rx.recv()
            .await
            .map_err(|_| "viewer is shutting down".to_string())
    };
    let timer = async {
        futures_timer::Delay::new(timeout).await;
        Err(format!(
            "viewer did not respond within {}s (is it busy, minimized or in a background tab?)",
            timeout.as_secs()
        ))
    };
    futures_lite::future::or(reply, timer).await
}

impl McpServer {
    pub fn new(store: Arc<Store>, app: Sender<AppRequest>) -> Self {
        Self { store, app }
    }

    /// Handles a JSON-RPC message or batch; `None` for notifications.
    pub async fn handle_json(&self, body: &str) -> Option<Value> {
        match serde_json::from_str::<Value>(body) {
            Err(e) => Some(rpc_error(Value::Null, -32700, &format!("parse error: {e}"))),
            Ok(Value::Array(batch)) => {
                let mut out = Vec::new();
                for message in batch {
                    out.extend(self.handle_message(message).await);
                }
                (!out.is_empty()).then_some(Value::Array(out))
            }
            Ok(message) => self.handle_message(message).await,
        }
    }

    async fn handle_message(&self, message: Value) -> Option<Value> {
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
                    "instructions": "WESL shader playground: each project is a folder with a project.json and .wesl files. Call `shader_guide` once before writing shaders.",
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
                Ok(match self.call_tool(name, &args).await {
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

    async fn call_tool(&self, name: &str, args: &Value) -> Result<Vec<Value>, String> {
        match name {
            "shader_guide" => Ok(vec![text(GUIDE)]),
            "list_projects" => self.list_projects(),
            "get_project" => self.get_project(args),
            "create_project" => self.create_project(args).await,
            "update_project" => self.update_project(args).await,
            "delete_project" => {
                let id = str_arg(args, "id")?;
                self.store.delete_project(id)?;
                self.notify(id);
                Ok(vec![text(&format!("deleted `{id}`"))])
            }
            "set_group" => self.set_group(args),
            "update_shared_files" => self.update_shared_files(args).await,
            "add_texture" => self.add_texture(args),
            "import_file" => self.import_file(args),
            "compile_project" => {
                let id = str_arg(args, "id")?;
                self.require_project(id)?;
                self.compile(id, bool_arg(args, "preview", true)).await
            }
            "open_in_viewer" => self.open_in_viewer(args).await,
            "screenshot" => {
                let (tx, rx) = bounded(1);
                self.send(AppRequest::Screenshot { reply: tx })?;
                let png = wait(rx, APP_WAIT).await??;
                Ok(vec![image(&png)])
            }
            "viewer_state" => {
                let (tx, rx) = bounded(1);
                self.send(AppRequest::State { reply: tx })?;
                let state = wait(rx, APP_WAIT).await?;
                Ok(vec![text(&serde_json::to_string_pretty(&state).unwrap())])
            }
            other => Err(format!("unknown tool `{other}`")),
        }
    }

    fn send(&self, request: AppRequest) -> Result<(), String> {
        self.app
            .try_send(request)
            .map_err(|_| "viewer is shutting down".to_string())
    }

    fn notify(&self, id: &str) {
        let _ = self
            .app
            .try_send(AppRequest::ProjectChanged { id: id.to_string() });
    }

    fn require_project(&self, id: &str) -> Result<(), String> {
        validate_project_id(id)?;
        if self.store.project_exists(id) {
            Ok(())
        } else {
            Err(format!("project `{id}` does not exist (see list_projects)"))
        }
    }

    async fn compile(&self, id: &str, preview: bool) -> Result<Vec<Value>, String> {
        let (tx, rx) = bounded(1);
        self.send(AppRequest::Compile {
            id: id.to_string(),
            preview,
            reply: tx,
        })?;
        let report: CompileReport = wait(rx, COMPILE_WAIT).await?;
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
            "storage": self.store.vfs.describe(),
            "groups": catalog.groups.iter().map(|g| json!({
                "id": g.id,
                "name": g.manifest.name,
                "description": g.manifest.description,
                "shared": self.store.list_shared_files(&g.id),
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
        let mut value = json!({
            "id": id,
            "manifest": match &manifest { Ok(m) => json!(m), Err(e) => json!({ "error": e }) },
            "files": files,
            "sources": sources,
        });
        if let Some((group, _)) = id.split_once('/') {
            let shared = self.store.list_shared_files(group);
            if !shared.is_empty() {
                value["group_shared_files"] = json!(shared);
                value["group_shared_import"] =
                    json!(format!("import super::super::{SHARED_DIR}::<module>::{{...}};"));
            }
        }
        Ok(vec![text(&serde_json::to_string_pretty(&value).unwrap())])
    }

    async fn create_project(&self, args: &Value) -> Result<Vec<Value>, String> {
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
        self.after_write(id, args).await
    }

    async fn update_project(&self, args: &Value) -> Result<Vec<Value>, String> {
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
        self.after_write(id, args).await
    }

    async fn after_write(&self, id: &str, args: &Value) -> Result<Vec<Value>, String> {
        if bool_arg(args, "compile", true) {
            self.compile(id, bool_arg(args, "preview", false)).await
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

    /// Writes/deletes a group's shared modules, then recompiles every project of the group.
    async fn update_shared_files(&self, args: &Value) -> Result<Vec<Value>, String> {
        let group = str_arg(args, "group")?;
        self.store.check_group(group)?;
        let files = files_arg(args)?;
        if files.is_empty() {
            return Err("`files` is empty".into());
        }
        let mut changed = Vec::new();
        for (path, content) in files {
            match content {
                Some(content) => {
                    changed.push(self.store.write_shared_file(group, &path, content.as_bytes())?)
                }
                None => {
                    self.store.delete_shared_file(group, &path)?;
                    changed.push(format!("{path} (deleted)"));
                }
            }
        }
        self.notify(group);
        let saved = format!("saved {SHARED_DIR}/ in `{group}`: {}", changed.join(", "));
        if !bool_arg(args, "compile", true) {
            return Ok(vec![text(&saved)]);
        }
        let projects: Vec<String> = self
            .store
            .scan()
            .group(group)
            .map(|g| g.projects.iter().map(|p| p.id.clone()).collect())
            .unwrap_or_default();
        let mut results = Vec::new();
        for id in projects {
            let (tx, rx) = bounded(1);
            self.send(AppRequest::Compile {
                id: id.clone(),
                preview: false,
                reply: tx,
            })?;
            let report: CompileReport = wait(rx, COMPILE_WAIT).await?;
            results.push(if report.ok {
                json!({ "project": id, "ok": true })
            } else {
                json!({ "project": id, "ok": false, "timed_out": report.timed_out, "errors": report.errors })
            });
        }
        let value = json!({ "saved": saved, "compiled": results });
        Ok(vec![text(&serde_json::to_string_pretty(&value).unwrap())])
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

    async fn open_in_viewer(&self, args: &Value) -> Result<Vec<Value>, String> {
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
        let msg = wait(rx, APP_WAIT).await??;
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
        read_local_file(src)
    } else {
        Err("provide one of `procedural`, `base64` or `source_path`".into())
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn read_local_file(path: &str) -> Result<Vec<u8>, String> {
    std::fs::read(path).map_err(|e| format!("{path}: {e}"))
}

/// The bridge replaces `source_path` with `base64` before forwarding, since the page
/// cannot read local files.
#[cfg(target_arch = "wasm32")]
fn read_local_file(path: &str) -> Result<Vec<u8>, String> {
    Err(format!(
        "{path}: the browser viewer cannot read local files; pass `base64` instead"
    ))
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
    serde_json::from_str(TOOLS).expect("plugin/server/tools.json is valid JSON")
}
