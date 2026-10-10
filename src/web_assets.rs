//! Browser asset source: serves project files from the in-memory [`Vfs`].

use std::path::{Path, PathBuf};

use bevy::{
    asset::io::{
        AssetReader, AssetReaderError, AssetSourceBuilder, AssetSourceId, PathStream, Reader,
        VecReader,
    },
    prelude::*,
    tasks::futures_lite::stream,
};

use crate::vfs::Vfs;

struct VfsAssetReader;

fn rel(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

impl AssetReader for VfsAssetReader {
    async fn read<'a>(&'a self, path: &'a Path) -> Result<impl Reader + 'a, AssetReaderError> {
        Vfs::read_global(&rel(path))
            .map(VecReader::new)
            .map_err(|_| AssetReaderError::NotFound(path.to_path_buf()))
    }

    async fn read_meta<'a>(&'a self, path: &'a Path) -> Result<impl Reader + 'a, AssetReaderError> {
        Err::<VecReader, _>(AssetReaderError::NotFound(path.to_path_buf()))
    }

    async fn read_directory<'a>(
        &'a self,
        path: &'a Path,
    ) -> Result<Box<PathStream>, AssetReaderError> {
        let dir = rel(path);
        let entries: Vec<PathBuf> = Vfs::list_dir_global(&dir)
            .into_iter()
            .map(|(name, _)| path.join(name))
            .collect();
        Ok(Box::new(stream::iter(entries)))
    }

    async fn is_directory<'a>(&'a self, path: &'a Path) -> Result<bool, AssetReaderError> {
        Ok(Vfs::is_dir_global(&rel(path)))
    }
}

/// Must be added before `DefaultPlugins` so it replaces the default (HTTP) asset source.
pub struct VfsAssetSourcePlugin;

impl Plugin for VfsAssetSourcePlugin {
    fn build(&self, app: &mut App) {
        app.register_asset_source(
            AssetSourceId::Default,
            AssetSourceBuilder::new(|| Box::new(VfsAssetReader)),
        );
    }
}
