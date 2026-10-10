//! Messages between the MCP server and the Bevy app.

use async_channel::Sender;
use serde::Serialize;

pub enum AppRequest {
    /// Spawn the project off-screen, wait for its pipelines and report errors.
    Compile {
        id: String,
        preview: bool,
        reply: Sender<CompileReport>,
    },
    /// Show a project or a whole group in the main view.
    Open {
        target: ViewTarget,
        reply: Sender<Result<String, String>>,
    },
    /// PNG of the main window.
    Screenshot {
        reply: Sender<Result<Vec<u8>, String>>,
    },
    State {
        reply: Sender<serde_json::Value>,
    },
    /// Files of a project (or the project list) changed on disk.
    ProjectChanged {
        id: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ViewTarget {
    Project(String),
    Group(String),
}

#[derive(Serialize, Default, Debug)]
pub struct CompileReport {
    pub project: String,
    /// true when every pipeline compiled and no error was found.
    pub ok: bool,
    pub timed_out: bool,
    /// Manifest, asset and shader errors.
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
    /// Per-object pipeline status.
    pub objects: Vec<ObjectStatus>,
    #[serde(skip)]
    pub preview_png: Option<Vec<u8>>,
}

#[derive(Serialize, Debug, Clone)]
pub struct ObjectStatus {
    pub object: String,
    pub shader: String,
    /// ok | error | pending (details are in `CompileReport::errors`)
    pub status: String,
}
