//! What Solara installed: `packages.json` in Solara's settings folder.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::index::{FileEntry, Format};

/// The record's file name.
pub const RECORD_NAME: &str = "packages.json";
const RECORD_FORMAT: &str = "solara-packages";
const MAX_RECORD_BYTES: u64 = 16 << 20;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    #[serde(default)]
    pub format: String,
    #[serde(default)]
    pub packages: Vec<InstalledPackage>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstalledPackage {
    pub id: String,
    pub name: String,
    pub version: String,
    /// The plugin root it was installed under (absolute).
    pub root: PathBuf,
    pub bundles: Vec<InstalledBundle>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstalledBundle {
    pub format: Format,
    /// Relative to the root, `/`-separated: `VST3/Song Sense.vst3`.
    pub path: String,
    /// Every file installed, relative to the root, with the SHA-256 it was installed with.
    pub files: Vec<FileEntry>,
}

impl Record {
    /// The record at `path`; an empty one when there is none yet. An unreadable record is an error,
    /// so it is never silently replaced.
    pub fn read(path: &Path) -> Result<Self, String> {
        let meta = match std::fs::metadata(path) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Record::default()),
            Err(e) => return Err(format!("couldn't read {} ({e})", path.display())),
        };
        if meta.len() > MAX_RECORD_BYTES {
            return Err(format!("{} is too large", path.display()));
        }
        let bytes = std::fs::read(path).map_err(|e| format!("couldn't read {} ({e})", path.display()))?;
        serde_json::from_slice(&bytes).map_err(|e| format!("{} is unreadable ({e})", path.display()))
    }

    /// Write the record atomically (a temporary file, then a rename).
    pub fn write(&self, path: &Path) -> Result<(), String> {
        let mut r = self.clone();
        r.format = RECORD_FORMAT.into();
        let json = serde_json::to_vec_pretty(&r).map_err(|e| e.to_string())?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("couldn't create {} ({e})", dir.display()))?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, json).map_err(|e| format!("couldn't write {} ({e})", tmp.display()))?;
        std::fs::rename(&tmp, path).map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            format!("couldn't write {} ({e})", path.display())
        })
    }

    pub fn get(&self, id: &str) -> Option<&InstalledPackage> {
        self.packages.iter().find(|p| p.id == id)
    }

    /// Add or replace a package's entry.
    pub fn set(&mut self, p: InstalledPackage) {
        self.packages.retain(|q| q.id != p.id);
        self.packages.push(p);
        self.packages.sort_by(|a, b| a.id.cmp(&b.id));
    }

    pub fn remove(&mut self, id: &str) {
        self.packages.retain(|p| p.id != id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_starts_empty() {
        let dir = std::env::temp_dir().join(format!("solara-record-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join(RECORD_NAME);
        assert_eq!(Record::read(&path).unwrap(), Record::default());
        let mut r = Record::default();
        r.set(InstalledPackage { id: "b".into(), name: "B".into(), version: "1".into(), root: dir.clone(), bundles: vec![] });
        r.set(InstalledPackage { id: "a".into(), name: "A".into(), version: "1".into(), root: dir.clone(), bundles: vec![] });
        r.set(InstalledPackage { id: "b".into(), name: "B".into(), version: "2".into(), root: dir.clone(), bundles: vec![] });
        r.write(&path).unwrap();
        let back = Record::read(&path).unwrap();
        assert_eq!(back.packages.iter().map(|p| (p.id.as_str(), p.version.as_str())).collect::<Vec<_>>(), vec![("a", "1"), ("b", "2")]);
        std::fs::write(&path, b"{broken").unwrap();
        assert!(Record::read(&path).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
