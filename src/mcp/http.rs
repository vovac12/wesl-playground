//! Native transport: Streamable HTTP (JSON responses only) on a background thread.

use std::{sync::Arc, thread};

use async_channel::Sender;
use bevy::tasks::block_on;
use tiny_http::{Header, Method, Request, Response, Server};

use super::McpServer;
use crate::{bridge::AppRequest, project::Store};

pub fn spawn(addr: String, store: Arc<Store>, app: Sender<AppRequest>) {
    let server = match Server::http(&addr) {
        Ok(server) => server,
        Err(e) => {
            bevy::log::error!("MCP server failed to bind {addr}: {e}");
            return;
        }
    };
    bevy::log::info!("MCP server listening on http://{addr}/mcp");
    let mcp = McpServer::new(store, app);
    thread::Builder::new()
        .name("mcp-http".into())
        .spawn(move || {
            for request in server.incoming_requests() {
                let mcp = mcp.clone();
                // Compiles can take a while; don't block other requests.
                thread::spawn(move || handle(&mcp, request));
            }
        })
        .expect("spawn MCP thread");
}

fn handle(mcp: &McpServer, mut request: Request) {
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
    let _ = match block_on(mcp.handle_json(&body)) {
        Some(value) => request.respond(
            Response::from_string(value.to_string())
                .with_header(Header::from_bytes("Content-Type", "application/json").unwrap()),
        ),
        // Notifications and responses get 202 with no body.
        None => request.respond(Response::empty(202)),
    };
}
