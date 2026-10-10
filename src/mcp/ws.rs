//! Browser transport: the page connects to the Claude Code plugin's local bridge over WebSocket.
//! The bridge forwards JSON-RPC messages from Claude; this side answers them like a server.

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use async_channel::Sender;
use bevy::prelude::Resource;
use wasm_bindgen::{JsCast, prelude::*};
use wasm_bindgen_futures::spawn_local;
use web_sys::{MessageEvent, WebSocket};

use super::McpServer;
use crate::{bridge::AppRequest, project::Store};

pub const DEFAULT_BRIDGE: &str = "ws://127.0.0.1:7880";
const RECONNECT_DELAY: Duration = Duration::from_secs(2);

#[derive(Resource, Clone)]
pub struct BridgeStatus {
    pub url: String,
    connected: Arc<AtomicBool>,
}

impl BridgeStatus {
    pub fn connected(&self) -> bool {
        self.connected.load(Ordering::Relaxed)
    }
}

/// Bridge URL from `?bridge=ws://host:port`, or the default.
fn bridge_url() -> String {
    web_sys::window()
        .and_then(|w| w.location().search().ok())
        .and_then(|search| web_sys::UrlSearchParams::new_with_str(&search).ok())
        .and_then(|params| params.get("bridge"))
        .unwrap_or_else(|| DEFAULT_BRIDGE.to_string())
}

pub fn spawn(store: Arc<Store>, app: Sender<AppRequest>) -> BridgeStatus {
    let status = BridgeStatus {
        url: bridge_url(),
        connected: Arc::default(),
    };
    connect(McpServer::new(store, app), status.clone());
    status
}

fn connect(mcp: McpServer, status: BridgeStatus) {
    let ws = match WebSocket::new(&status.url) {
        Ok(ws) => ws,
        Err(_) => {
            retry_later(mcp, status);
            return;
        }
    };

    let on_open = {
        let status = status.clone();
        Closure::<dyn FnMut()>::new(move || {
            status.connected.store(true, Ordering::Relaxed);
            bevy::log::info!("connected to Claude Code bridge at {}", status.url);
        })
    };
    ws.set_onopen(Some(on_open.as_ref().unchecked_ref()));
    on_open.forget();

    let on_message = {
        let mcp = mcp.clone();
        let socket = ws.clone();
        Closure::<dyn FnMut(MessageEvent)>::new(move |event: MessageEvent| {
            let Some(body) = event.data().as_string() else {
                return;
            };
            let mcp = mcp.clone();
            let socket = socket.clone();
            spawn_local(async move {
                if let Some(response) = mcp.handle_json(&body).await {
                    let _ = socket.send_with_str(&response.to_string());
                }
            });
        })
    };
    ws.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
    on_message.forget();

    // `onclose` also fires after a failed connection attempt.
    let on_close = Closure::once(move || {
        status.connected.store(false, Ordering::Relaxed);
        retry_later(mcp, status);
    });
    ws.set_onclose(Some(on_close.as_ref().unchecked_ref()));
    on_close.forget();
}

fn retry_later(mcp: McpServer, status: BridgeStatus) {
    spawn_local(async move {
        futures_timer::Delay::new(RECONNECT_DELAY).await;
        connect(mcp, status);
    });
}
