use fs2::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::HashMap,
    fs::{self, File},
    io::Write,
    path::PathBuf,
};

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct ImageMeta {
    pub id: String,
    pub filename: String,
    pub name: Option<String>,
    pub mime_type: String,
    pub width: u32,
    pub height: u32,
    pub created_at: String,
}
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Envelope {
    pub schema: String,
    pub annotation: Value,
}
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Snapshot {
    #[serde(default)]
    pub pending_deletions: Vec<String>,
    pub images: Vec<ImageMeta>,
    pub annotations: HashMap<String, Vec<Envelope>>,
}
pub struct Storage {
    pub root: PathBuf,
    pub data: Snapshot,
    _lock: File,
}
impl Storage {
    pub fn open(root: PathBuf) -> std::io::Result<Self> {
        fs::create_dir_all(root.join("images"))?;
        let lock = File::options()
            .create(true)
            .truncate(false)
            .write(true)
            .open(root.join("server.lock"))?;
        lock.try_lock_exclusive()?;
        let data = match fs::read(root.join("state.json")) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(std::io::Error::other)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Snapshot::default(),
            Err(e) => return Err(e),
        };
        let mut store = Self {
            root,
            data,
            _lock: lock,
        };
        let result = store.cleanup();
        for failure in result.failures {
            tracing::warn!(
                id = failure.id,
                error = failure.message,
                "Image cleanup pending"
            );
        }
        Ok(store)
    }
    pub fn commit(&mut self, next: Snapshot) -> std::io::Result<()> {
        let bytes = serde_json::to_vec(&next)?;
        let temp = self.root.join("state.json.tmp");
        let mut file = File::create(&temp)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        fs::rename(temp, self.root.join("state.json"))?;
        self.data = next;
        Ok(())
    }
}
