//! Mirrors per-shader pipeline compilation state from the render world into the main world.

use std::sync::{Arc, Mutex};

use bevy::{
    platform::collections::HashMap,
    prelude::*,
    render::{
        Render, RenderApp, RenderSystems,
        render_resource::{CachedPipelineState, PipelineCache, PipelineDescriptor},
    },
    shader::ShaderCacheError,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShaderState {
    Ok,
    /// Still compiling; the string holds the last retryable error, if any
    /// (e.g. an import that cannot be resolved yet).
    Pending(Option<String>),
    Err(String),
}

impl ShaderState {
    fn rank(&self) -> u8 {
        match self {
            ShaderState::Ok => 0,
            ShaderState::Pending(_) => 1,
            ShaderState::Err(_) => 2,
        }
    }
}

/// Worst state of any render pipeline using a given shader as vertex or fragment entry point.
#[derive(Resource, Clone, Default)]
pub struct PipelineStatus(pub Arc<Mutex<HashMap<AssetId<Shader>, ShaderState>>>);

impl PipelineStatus {
    pub fn get(&self, id: AssetId<Shader>) -> Option<ShaderState> {
        self.0.lock().unwrap().get(&id).cloned()
    }
}

pub struct PipelineStatusPlugin;

impl Plugin for PipelineStatusPlugin {
    fn build(&self, app: &mut App) {
        let status = PipelineStatus::default();
        app.insert_resource(status.clone());
        if let Some(render_app) = app.get_sub_app_mut(RenderApp) {
            render_app
                .insert_resource(status)
                .add_systems(Render, collect_status.in_set(RenderSystems::Cleanup));
        }
    }
}

/// `last_retry` remembers retryable errors per pipeline: the cache alternates such pipelines
/// between `Err` and `Queued` every frame, and the error should stay visible while it lasts.
fn collect_status(
    cache: Res<PipelineCache>,
    status: Res<PipelineStatus>,
    mut last_retry: Local<HashMap<usize, String>>,
) {
    let mut map: HashMap<AssetId<Shader>, ShaderState> = HashMap::default();
    for (index, pipeline) in cache.pipelines().enumerate() {
        let PipelineDescriptor::RenderPipelineDescriptor(descriptor) = &pipeline.descriptor else {
            continue;
        };
        let state = match &pipeline.state {
            CachedPipelineState::Queued | CachedPipelineState::Creating(_) => {
                ShaderState::Pending(last_retry.get(&index).cloned())
            }
            CachedPipelineState::Ok(_) => {
                last_retry.remove(&index);
                ShaderState::Ok
            }
            CachedPipelineState::Err(
                err @ (ShaderCacheError::ShaderNotLoaded(_)
                | ShaderCacheError::ShaderImportNotYetAvailable),
            ) => {
                last_retry.insert(index, err.to_string());
                ShaderState::Pending(Some(err.to_string()))
            }
            CachedPipelineState::Err(err) => {
                last_retry.remove(&index);
                ShaderState::Err(err.to_string())
            }
        };
        let ids = std::iter::once(descriptor.vertex.shader.id())
            .chain(descriptor.fragment.as_ref().map(|f| f.shader.id()));
        for id in ids {
            let entry = map.entry(id).or_insert(ShaderState::Ok);
            if state.rank() > entry.rank() {
                *entry = state.clone();
            }
        }
    }
    *status.0.lock().unwrap() = map;
}
