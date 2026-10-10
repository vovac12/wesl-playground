//! Project file storage.
//!
//! Paths are relative, `/`-separated strings. Natively they live under `BASE_FOLDER`; in the
//! browser they live in memory and are mirrored to IndexedDB by the page (see `web/index.html`).

use std::io;

#[cfg(not(target_arch = "wasm32"))]
pub use native::Vfs;
#[cfg(target_arch = "wasm32")]
pub use web::Vfs;

fn not_found(path: &str) -> io::Error {
    io::Error::new(io::ErrorKind::NotFound, format!("{path}: not found"))
}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::{fs, io, path::PathBuf, time::UNIX_EPOCH};

    pub struct Vfs {
        pub base: PathBuf,
    }

    impl Vfs {
        pub fn new(base: PathBuf) -> Self {
            Self { base }
        }

        pub fn describe(&self) -> String {
            self.base.display().to_string()
        }

        fn path(&self, rel: &str) -> PathBuf {
            self.base.join(rel)
        }

        pub fn read(&self, rel: &str) -> io::Result<Vec<u8>> {
            fs::read(self.path(rel)).map_err(|_| super::not_found(rel))
        }

        pub fn write(&self, rel: &str, bytes: &[u8]) -> io::Result<()> {
            let path = self.path(rel);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(path, bytes)
        }

        pub fn remove_file(&self, rel: &str) -> io::Result<()> {
            fs::remove_file(self.path(rel))
        }

        pub fn remove_dir_all(&self, rel: &str) -> io::Result<()> {
            fs::remove_dir_all(self.path(rel))
        }

        pub fn is_file(&self, rel: &str) -> bool {
            self.path(rel).is_file()
        }

        pub fn is_dir(&self, rel: &str) -> bool {
            self.path(rel).is_dir()
        }

        /// Names of entries directly inside `rel` (`""` is the root): `(name, is_dir)`.
        pub fn list_dir(&self, rel: &str) -> Vec<(String, bool)> {
            let Ok(entries) = fs::read_dir(self.path(rel)) else {
                return Vec::new();
            };
            let mut out: Vec<_> = entries
                .flatten()
                .filter_map(|e| Some((e.file_name().into_string().ok()?, e.path().is_dir())))
                .collect();
            out.sort();
            out
        }

        /// Changes whenever the file changes.
        pub fn revision(&self, rel: &str) -> Option<u128> {
            let modified = fs::metadata(self.path(rel)).ok()?.modified().ok()?;
            Some(modified.duration_since(UNIX_EPOCH).ok()?.as_nanos())
        }
    }
}

#[cfg(target_arch = "wasm32")]
mod web {
    use std::{
        collections::BTreeMap,
        io,
        sync::{
            Mutex,
            atomic::{AtomicU64, Ordering},
        },
    };

    use wasm_bindgen::prelude::*;

    #[wasm_bindgen]
    extern "C" {
        /// Persists a file (implemented by the page).
        #[wasm_bindgen(js_namespace = shadertoyStorage, js_name = put)]
        fn js_put(path: &str, data: &[u8]);
        #[wasm_bindgen(js_namespace = shadertoyStorage, js_name = remove)]
        fn js_remove(path: &str);
        /// Files loaded from IndexedDB before the app started: `[[path, Uint8Array], ...]`.
        #[wasm_bindgen(js_namespace = shadertoyStorage, js_name = initialFiles)]
        fn js_initial_files() -> js_sys::Array;
    }

    struct File {
        bytes: Vec<u8>,
        revision: u64,
    }

    static FILES: Mutex<BTreeMap<String, File>> = Mutex::new(BTreeMap::new());
    static REVISION: AtomicU64 = AtomicU64::new(1);

    pub struct Vfs;

    impl Vfs {
        /// Loads the files the page read from IndexedDB.
        pub fn new() -> Self {
            let mut files = FILES.lock().unwrap();
            for entry in js_initial_files().iter() {
                let entry = js_sys::Array::from(&entry);
                let Some(path) = entry.get(0).as_string() else {
                    continue;
                };
                let bytes = js_sys::Uint8Array::new(&entry.get(1)).to_vec();
                files.insert(path, File { bytes, revision: 0 });
            }
            Self
        }

        pub fn describe(&self) -> String {
            "browser storage (IndexedDB)".into()
        }

        /// Used by the asset reader, which runs outside of any `Vfs` instance.
        pub fn read_global(rel: &str) -> io::Result<Vec<u8>> {
            FILES
                .lock()
                .unwrap()
                .get(rel)
                .map(|f| f.bytes.clone())
                .ok_or_else(|| super::not_found(rel))
        }

        pub fn read(&self, rel: &str) -> io::Result<Vec<u8>> {
            Self::read_global(rel)
        }

        pub fn write(&self, rel: &str, bytes: &[u8]) -> io::Result<()> {
            let revision = REVISION.fetch_add(1, Ordering::Relaxed);
            FILES.lock().unwrap().insert(
                rel.to_string(),
                File {
                    bytes: bytes.to_vec(),
                    revision,
                },
            );
            js_put(rel, bytes);
            Ok(())
        }

        pub fn remove_file(&self, rel: &str) -> io::Result<()> {
            match FILES.lock().unwrap().remove(rel) {
                Some(_) => {
                    js_remove(rel);
                    Ok(())
                }
                None => Err(super::not_found(rel)),
            }
        }

        pub fn remove_dir_all(&self, rel: &str) -> io::Result<()> {
            let prefix = format!("{rel}/");
            let mut files = FILES.lock().unwrap();
            let doomed: Vec<String> = files
                .keys()
                .filter(|k| k.starts_with(&prefix))
                .cloned()
                .collect();
            for path in doomed {
                files.remove(&path);
                js_remove(&path);
            }
            Ok(())
        }

        pub fn is_file(&self, rel: &str) -> bool {
            FILES.lock().unwrap().contains_key(rel)
        }

        pub fn is_dir(&self, rel: &str) -> bool {
            Self::is_dir_global(rel)
        }

        pub fn is_dir_global(rel: &str) -> bool {
            if rel.is_empty() {
                return true;
            }
            let prefix = format!("{rel}/");
            FILES.lock().unwrap().keys().any(|k| k.starts_with(&prefix))
        }

        pub fn list_dir(&self, rel: &str) -> Vec<(String, bool)> {
            Self::list_dir_global(rel)
        }

        pub fn list_dir_global(rel: &str) -> Vec<(String, bool)> {
            let prefix = if rel.is_empty() {
                String::new()
            } else {
                format!("{rel}/")
            };
            let mut out: Vec<(String, bool)> = Vec::new();
            for key in FILES.lock().unwrap().keys() {
                let Some(rest) = key.strip_prefix(&prefix) else {
                    continue;
                };
                let entry = match rest.split_once('/') {
                    Some((dir, _)) => (dir.to_string(), true),
                    None => (rest.to_string(), false),
                };
                if !out.contains(&entry) {
                    out.push(entry);
                }
            }
            out.sort();
            out
        }

        pub fn revision(&self, rel: &str) -> Option<u128> {
            FILES.lock().unwrap().get(rel).map(|f| f.revision as u128)
        }
    }
}
