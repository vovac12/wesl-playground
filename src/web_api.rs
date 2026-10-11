//! Functions exported to the page's JavaScript (`web/viewer.js`), which owns the MCP transport:
//! a WebSocket to the Claude Code bridge in a browser tab, or MCP Apps host messages when the
//! viewer is embedded in Claude Desktop.

use std::sync::{Arc, Mutex, OnceLock};

use async_channel::Sender;
use bevy::prelude::*;
use wasm_bindgen::prelude::*;

use crate::{bridge::AppRequest, mcp::McpServer, project::Store, ui::ConnectionLabel, vfs::Vfs};

static MCP: OnceLock<McpServer> = OnceLock::new();
static STORE: OnceLock<Arc<Store>> = OnceLock::new();
static APP: OnceLock<Sender<AppRequest>> = OnceLock::new();
static LABEL: Mutex<Option<String>> = Mutex::new(None);

pub fn init(store: Arc<Store>, app: Sender<AppRequest>) {
    let _ = MCP.set(McpServer::new(store.clone(), app.clone()));
    let _ = STORE.set(store);
    let _ = APP.set(app);
}

/// Handles a JSON-RPC message; resolves to the response JSON, or `undefined` for notifications.
#[wasm_bindgen]
pub fn mcp_handle(body: String) -> js_sys::Promise {
    wasm_bindgen_futures::future_to_promise(async move {
        let mcp = MCP.get().ok_or("viewer is not initialized")?;
        Ok(match mcp.handle_json(&body).await {
            Some(response) => JsValue::from_str(&response.to_string()),
            None => JsValue::UNDEFINED,
        })
    })
}

/// Applies a file change that happened outside this viewer; `data = undefined` removes it.
#[wasm_bindgen]
pub fn storage_apply(path: String, data: Option<Vec<u8>>) {
    Vfs::apply_external(&path, data);
    if let (Some(store), Some(app)) = (STORE.get(), APP.get()) {
        let _ = app.try_send(AppRequest::ProjectChanged {
            id: owner(store, &path),
        });
    }
}

/// The project (or group) a file belongs to.
fn owner(store: &Store, path: &str) -> String {
    let parts: Vec<&str> = path.split('/').collect();
    for len in (1..parts.len().min(3)).rev() {
        let candidate = parts[..len].join("/");
        if store.project_exists(&candidate) {
            return candidate;
        }
    }
    parts[0].to_string()
}

/// Stops the app (another viewer took over).
#[wasm_bindgen]
pub fn shutdown() {
    if let Some(app) = APP.get() {
        let _ = app.try_send(AppRequest::Shutdown);
    }
}

/// Text for the connection line of the status bar.
#[wasm_bindgen]
pub fn set_connection_label(text: String) {
    *LABEL.lock().unwrap() = Some(text);
}

pub fn update_connection_label(mut label: ResMut<ConnectionLabel>) {
    if let Some(text) = LABEL.lock().unwrap().take()
        && label.0 != text
    {
        label.0 = text;
    }
}
