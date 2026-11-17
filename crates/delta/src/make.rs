//! Release side: compare two install folders and write the delta between them.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::codec;
use crate::hash::sha256_bytes;
use crate::manifest::{Action, FORMAT, FORMAT_VERSION, FileChange, MANIFEST_NAME, Manifest, Payload, safe_relative};

/// What the delta is for.
#[derive(Clone, Debug)]
pub struct MakeOptions {
    pub app: String,
    pub platform: String,
    pub from_version: String,
    pub to_version: String,
}

/// What [`make`] wrote.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MakeReport {
    /// The SHA-256 of `delta.json`, to publish with it: the app refuses a manifest without it.
    pub manifest_sha256: String,
    pub manifest_bytes: u64,
    /// The payloads together: with `manifest_bytes`, what an update downloads.
    pub payload_bytes: u64,
    /// The whole new folder, uncompressed.
    pub new_folder_bytes: u64,
    pub changed: usize,
    pub unchanged: usize,
}

/// Write the delta from install folder `old` to install folder `new` into `out` (created if
/// missing; it must not already hold a delta).
pub fn make(old: &Path, new: &Path, out: &Path, opts: &MakeOptions) -> Result<MakeReport, String> {
    let old_files = list(old)?;
    let new_files = list(new)?;
    if out.join(MANIFEST_NAME).exists() {
        return Err(format!("{} already holds a delta", out.display()));
    }
    std::fs::create_dir_all(out).map_err(|e| format!("couldn't create {} ({e})", out.display()))?;
    let mut files = Vec::new();
    let mut payload_bytes = 0u64;
    let mut new_folder_bytes = 0u64;
    let mut unchanged = 0usize;
    for (rel, new_path) in &new_files {
        let new_bytes = read(new_path)?;
        new_folder_bytes = new_folder_bytes.saturating_add(new_bytes.len() as u64);
        let to = sha256_bytes(&new_bytes);
        let old_bytes = match old_files.get(rel) {
            Some(p) => Some(read(p)?),
            None => None,
        };
        let from = old_bytes.as_deref().map(sha256_bytes);
        if from.as_deref() == Some(to.as_str()) {
            unchanged += 1;
            continue;
        }
        let full = codec::deflate(&new_bytes)?;
        let patch = match &old_bytes {
            Some(o) => Some(codec::make_patch(o, &new_bytes)?),
            None => None,
        };
        let (action, data, suffix) = match patch {
            Some(p) if p.len() < full.len() => (Action::Patch, p, "bsdiff.deflate"),
            _ => (Action::Full, full, "deflate"),
        };
        let leaf = rel.rsplit('/').next().unwrap_or(rel).replace(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_')), "_");
        let name = format!("{}-{}.{suffix}", files.len(), leaf.trim_start_matches('.'));
        write(&out.join(&name), &data)?;
        payload_bytes = payload_bytes.saturating_add(data.len() as u64);
        files.push(FileChange {
            path: rel.clone(),
            action,
            from_sha256: from,
            to_sha256: Some(to),
            to_size: Some(new_bytes.len() as u64),
            payload: Some(Payload { name, size: data.len() as u64, sha256: sha256_bytes(&data) }),
        });
    }
    for (rel, old_path) in &old_files {
        if !new_files.contains_key(rel) {
            files.push(FileChange {
                path: rel.clone(),
                action: Action::Remove,
                from_sha256: Some(sha256_bytes(&read(old_path)?)),
                to_sha256: None,
                to_size: None,
                payload: None,
            });
        }
    }
    let manifest = Manifest {
        format: FORMAT.into(),
        format_version: FORMAT_VERSION,
        app: opts.app.clone(),
        platform: opts.platform.clone(),
        from_version: opts.from_version.clone(),
        to_version: opts.to_version.clone(),
        files,
    };
    manifest.validate()?;
    let json = serde_json::to_vec_pretty(&manifest).map_err(|e| e.to_string())?;
    write(&out.join(MANIFEST_NAME), &json)?;
    Ok(MakeReport {
        manifest_sha256: sha256_bytes(&json),
        manifest_bytes: json.len() as u64,
        payload_bytes,
        new_folder_bytes,
        changed: manifest.files.len(),
        unchanged,
    })
}

/// Every file under `root`, by its `/`-separated relative path (sorted). Symbolic links and
/// anything that is not a plain file or folder are refused.
fn list(root: &Path) -> Result<BTreeMap<String, PathBuf>, String> {
    let mut out = BTreeMap::new();
    let mut stack = vec![(root.to_path_buf(), String::new())];
    while let Some((dir, prefix)) = stack.pop() {
        let entries = std::fs::read_dir(&dir).map_err(|e| format!("couldn't list {} ({e})", dir.display()))?;
        for entry in entries {
            let entry = entry.map_err(|e| format!("couldn't list {} ({e})", dir.display()))?;
            let name = entry.file_name().into_string().map_err(|n| format!("{} is not a UTF-8 name", Path::new(&n).display()))?;
            let rel = if prefix.is_empty() { name } else { format!("{prefix}/{name}") };
            safe_relative(&rel)?;
            let kind = entry.file_type().map_err(|e| e.to_string())?;
            if kind.is_dir() {
                if out.len() + stack.len() > 10_000 {
                    return Err(format!("{} holds too many files", root.display()));
                }
                stack.push((entry.path(), rel));
            } else if kind.is_file() {
                out.insert(rel, entry.path());
            } else {
                return Err(format!("{} is neither a file nor a folder", entry.path().display()));
            }
        }
    }
    Ok(out)
}

pub(crate) fn read(path: &Path) -> Result<Vec<u8>, String> {
    let len = std::fs::metadata(path).map_err(|e| format!("couldn't read {} ({e})", path.display()))?.len();
    if len > crate::MAX_FILE {
        return Err(format!("{} is larger than {} bytes", path.display(), crate::MAX_FILE));
    }
    std::fs::read(path).map_err(|e| format!("couldn't read {} ({e})", path.display()))
}

pub(crate) fn write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("couldn't create {} ({e})", dir.display()))?;
    }
    std::fs::write(path, bytes).map_err(|e| format!("couldn't write {} ({e})", path.display()))
}
