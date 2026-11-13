use crate::io::{
    file::{new_unwatched_asset_event_debouncer, FilesystemEventHandler},
    memory::Dir,
    AssetSourceEvent, AssetWatcher,
};
use alloc::{
    boxed::Box,
    sync::{Arc, Weak},
    vec::Vec,
};
use bevy_platform::collections::{HashMap, HashSet};
use bevy_platform::sync::{Mutex, PoisonError, RwLock};
use core::time::Duration;
use notify_debouncer_full::{
    notify::{RecommendedWatcher, RecursiveMode},
    Debouncer, RecommendedCache,
};
use std::{
    fs::File,
    io::{BufReader, Read},
    path::{Path, PathBuf},
};
use tracing::warn;

/// A watcher for assets stored in the `embedded` asset source. Embedded assets are assets whose
/// bytes have been embedded into the Rust binary using the [`embedded_asset`](crate::embedded_asset) macro.
/// This watcher will watch for changes to the "source files", read the contents of changed files from the file system
/// and overwrite the initial static bytes of the file embedded in the binary with the new dynamically loaded bytes.
///
/// It watches the folder of each embedded file, one level deep, wherever that folder is, and
/// nothing else. A recursive watch over the asset base path instead would walk every directory
/// below it, symlinks followed: a workspace's `target/` and `.git`, seconds or minutes at startup
/// (bevyengine/bevy#14903), and it would miss a file embedded from outside the base path.
pub struct EmbeddedWatcher {
    _folders: Arc<Mutex<WatchedFolders>>,
}

impl EmbeddedWatcher {
    /// Creates a new `EmbeddedWatcher` that watches for changes to the embedded assets in the given `dir`.
    ///
    /// It watches the folder of every file in `root_paths`, and joins `watchers`, through which
    /// a file embedded later gets its folder watched as well.
    pub fn new(
        dir: Dir,
        root_paths: Arc<RwLock<HashMap<Box<Path>, PathBuf>>>,
        watchers: EmbeddedWatchers,
        sender: async_channel::Sender<AssetSourceEvent>,
        debounce_wait_time: Duration,
    ) -> Self {
        let handler = EmbeddedEventHandler {
            dir,
            sender,
            root_paths: root_paths.clone(),
            last_event: None,
        };
        let debouncer = new_unwatched_asset_event_debouncer(debounce_wait_time, handler).unwrap();
        let folders = Arc::new(Mutex::new(WatchedFolders {
            debouncer,
            folders: HashSet::default(),
        }));
        // Join before reading the files already embedded: a file embedded in between is then
        // seen by one of the two, or both (a folder is watched once).
        watchers.join(&folders);
        {
            let mut folders = folders.lock().unwrap_or_else(PoisonError::into_inner);
            let root_paths = root_paths.read().unwrap_or_else(PoisonError::into_inner);
            for full_path in root_paths.keys() {
                folders.watch_folder_of(full_path);
            }
        }
        Self { _folders: folders }
    }
}

impl AssetWatcher for EmbeddedWatcher {}

/// The folders one [`EmbeddedWatcher`] watches.
pub(crate) struct WatchedFolders {
    debouncer: Debouncer<RecommendedWatcher, RecommendedCache>,
    folders: HashSet<PathBuf>,
}

impl WatchedFolders {
    /// Watches the folder that holds `full_path`, once. A folder that is not there is skipped
    /// without a word: in a shipped build the paths are the build machine's.
    fn watch_folder_of(&mut self, full_path: &Path) {
        let Some(folder) = full_path.parent() else {
            return;
        };
        if !self.folders.insert(folder.to_path_buf()) || !folder.is_dir() {
            return;
        }
        if let Err(error) = self.debouncer.watch(folder, RecursiveMode::NonRecursive) {
            warn!(
                "Cannot watch {} for changes to embedded assets: {error}",
                folder.display()
            );
        }
    }
}

/// Every live [`EmbeddedWatcher`] of one [`EmbeddedAssetRegistry`](crate::io::embedded::EmbeddedAssetRegistry),
/// so that a file embedded after a watcher started gets its folder watched too.
#[derive(Clone, Default)]
pub struct EmbeddedWatchers(Arc<Mutex<Vec<Weak<Mutex<WatchedFolders>>>>>);

impl EmbeddedWatchers {
    fn join(&self, folders: &Arc<Mutex<WatchedFolders>>) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Arc::downgrade(folders));
    }

    /// Has every live watcher watch the folder that holds `full_path`.
    pub(crate) fn watch_folder_of(&self, full_path: &Path) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|folders| {
                let Some(folders) = folders.upgrade() else {
                    return false;
                };
                folders
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .watch_folder_of(full_path);
                true
            });
    }
}

/// A [`FilesystemEventHandler`] that uses [`EmbeddedAssetRegistry`](crate::io::embedded::EmbeddedAssetRegistry) to hot-reload
/// binary-embedded Rust source files. This will read the contents of changed files from the file system and overwrite
/// the initial static bytes from the file embedded in the binary.
pub(crate) struct EmbeddedEventHandler {
    sender: async_channel::Sender<AssetSourceEvent>,
    root_paths: Arc<RwLock<HashMap<Box<Path>, PathBuf>>>,
    dir: Dir,
    last_event: Option<AssetSourceEvent>,
}

impl FilesystemEventHandler for EmbeddedEventHandler {
    fn begin(&mut self) {
        self.last_event = None;
    }

    fn get_path(&self, absolute_path: &Path) -> Option<(PathBuf, bool)> {
        let final_path = self
            .root_paths
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .get(absolute_path)?
            .clone();
        if absolute_path.extension().is_some_and(|e| e == "meta") {
            warn!("Meta file asset hot-reloading is not supported yet: {final_path:?}");
        }
        Some((final_path, false))
    }

    fn handle(&mut self, absolute_paths: &[PathBuf], event: AssetSourceEvent) {
        if self.last_event.as_ref() != Some(&event) {
            if let AssetSourceEvent::ModifiedAsset(path) = &event
                && let Ok(file) = File::open(&absolute_paths[0])
            {
                let mut reader = BufReader::new(file);
                let mut buffer = Vec::new();

                // Read file into vector.
                if reader.read_to_end(&mut buffer).is_ok() {
                    self.dir.insert_asset(path, buffer);
                }
            }
            self.last_event = Some(event.clone());
            self.sender.send_blocking(event).unwrap();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::EmbeddedWatcher;
    use alloc::format;
    use crate::io::{embedded::EmbeddedAssetRegistry, AssetSourceEvent};
    use core::time::Duration;
    use std::{path::Path, time::Instant};

    /// A file embedded after the watcher started, from a folder outside the asset base path,
    /// still reloads when it is edited: the watcher watches the folders of the files it holds.
    #[test]
    fn edit_reloads_a_file_embedded_later_from_outside_the_base_path() {
        let folder =
            std::env::temp_dir().join(format!("bevy_embedded_watcher_{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let file = folder.join("shader.wgsl");
        std::fs::write(&file, "old").unwrap();
        let asset_path = Path::new("some_crate/shader.wgsl");

        let registry = EmbeddedAssetRegistry::default();
        let (sender, receiver) = async_channel::unbounded();
        let _watcher = EmbeddedWatcher::new(
            registry.dir.clone(),
            registry.root_paths.clone(),
            registry.watchers.clone(),
            sender,
            Duration::from_millis(50),
        );
        registry.insert_asset(file.clone(), asset_path, &b"old"[..]);
        std::fs::write(&file, "new").unwrap();

        let deadline = Instant::now() + Duration::from_secs(10);
        let event = loop {
            if let Ok(event) = receiver.try_recv() {
                break Some(event);
            }
            if Instant::now() > deadline {
                break None;
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        std::fs::remove_dir_all(&folder).unwrap();

        assert_eq!(
            event,
            Some(AssetSourceEvent::ModifiedAsset(asset_path.to_path_buf()))
        );
        assert_eq!(registry.dir.get_asset(asset_path).unwrap().value(), b"new");
    }
}
