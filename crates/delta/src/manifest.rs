//! `delta.json`: what a delta changes, with the SHA-256 of every file before and after and of
//! every payload.

use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::hash::{is_sha256, sha256_bytes};

/// The manifest's file name inside a delta.
pub const MANIFEST_NAME: &str = "delta.json";
/// The `format` every manifest names.
pub const FORMAT: &str = "solara-delta";
/// The manifest layout this code reads and writes.
pub const FORMAT_VERSION: u32 = 1;
/// The largest manifest read.
pub(crate) const MAX_MANIFEST: usize = 1 << 20;
/// The most files one delta may change.
const MAX_FILES: usize = 10_000;

/// A delta from one version of an install folder to the next.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub format: String,
    pub format_version: u32,
    /// The app the delta is for (`solara`).
    pub app: String,
    /// The build it is for, such as `windows-x64` ([`crate::current_platform`]).
    pub platform: String,
    pub from_version: String,
    pub to_version: String,
    /// Only the files that differ; an unchanged file is not listed.
    pub files: Vec<FileChange>,
}

/// One file the delta changes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileChange {
    /// Relative to the install folder, `/`-separated.
    pub path: String,
    pub action: Action,
    /// The installed file the delta starts from; `None` for a file the update adds.
    pub from_sha256: Option<String>,
    /// The file after the update; `None` for a file the update removes.
    pub to_sha256: Option<String>,
    pub to_size: Option<u64>,
    /// What to download for it; `None` for a removal.
    pub payload: Option<Payload>,
}

/// How a file changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Action {
    /// The payload is a deflated bsdiff patch from the old file to the new one.
    Patch,
    /// The payload is the whole new file, deflated (a new file, or one a patch would not shrink).
    Full,
    /// The update deletes the file.
    Remove,
}

/// A downloaded piece of the delta, next to the manifest.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Payload {
    /// Its file name, next to `delta.json`.
    pub name: String,
    pub size: u64,
    pub sha256: String,
}

impl Manifest {
    /// Parse a downloaded manifest, refusing it unless its SHA-256 is `expected_sha256` (as the
    /// release publishes it) and every field is well-formed.
    pub fn parse_verified(bytes: &[u8], expected_sha256: &str) -> Result<Self, String> {
        let expected = expected_sha256.trim().to_ascii_lowercase();
        if !is_sha256(&expected) {
            return Err("the update has no valid published checksum for its manifest, so it was refused".into());
        }
        if bytes.len() > MAX_MANIFEST {
            return Err(format!("the update manifest is larger than {MAX_MANIFEST} bytes, so it was refused"));
        }
        let actual = sha256_bytes(bytes);
        if actual != expected {
            return Err("the update manifest doesn't match its published checksum, so it was refused".into());
        }
        let m: Manifest = serde_json::from_slice(bytes).map_err(|e| format!("the update manifest is unreadable ({e})"))?;
        m.validate()?;
        Ok(m)
    }

    /// Check every field: names, hashes, paths, payload names, and that each action carries
    /// exactly what it needs.
    pub fn validate(&self) -> Result<(), String> {
        if self.format != FORMAT || self.format_version != FORMAT_VERSION {
            return Err(format!("the update is format {} {}, this copy reads {FORMAT} {FORMAT_VERSION}", self.format, self.format_version));
        }
        if self.files.len() > MAX_FILES {
            return Err(format!("the update changes more than {MAX_FILES} files"));
        }
        let mut paths = std::collections::BTreeSet::new();
        let mut payloads = std::collections::BTreeSet::new();
        for f in &self.files {
            safe_relative(&f.path)?;
            if !paths.insert(f.path.to_ascii_lowercase()) {
                return Err(format!("the update lists {} twice", f.path));
            }
            let hash_ok = |h: &Option<String>| h.as_deref().is_none_or(is_sha256);
            if !hash_ok(&f.from_sha256) || !hash_ok(&f.to_sha256) {
                return Err(format!("the update has a malformed checksum for {}", f.path));
            }
            match (f.action, &f.payload) {
                (Action::Remove, None) if f.from_sha256.is_some() && f.to_sha256.is_none() => {}
                (Action::Patch, Some(_)) if f.from_sha256.is_some() && f.to_sha256.is_some() && f.to_size.is_some() => {}
                (Action::Full, Some(_)) if f.to_sha256.is_some() && f.to_size.is_some() => {}
                _ => return Err(format!("the update's entry for {} is incomplete", f.path)),
            }
            if f.to_size.is_some_and(|n| n > crate::MAX_FILE) {
                return Err(format!("the update's {} is larger than {} bytes", f.path, crate::MAX_FILE));
            }
            if let Some(p) = &f.payload {
                if !safe_name(&p.name) || !is_sha256(&p.sha256) || p.size > crate::MAX_FILE {
                    return Err(format!("the update's payload for {} is malformed", f.path));
                }
                if !payloads.insert(p.name.to_ascii_lowercase()) {
                    return Err(format!("the update lists payload {} twice", p.name));
                }
            }
        }
        Ok(())
    }

    /// The bytes all payloads add up to: what an update downloads besides the manifest.
    pub fn payload_bytes(&self) -> u64 {
        self.files.iter().filter_map(|f| f.payload.as_ref()).fold(0u64, |a, p| a.saturating_add(p.size))
    }
}

/// `path` (relative, `/`-separated) as a path under an install folder, or why it can't be one:
/// no absolute paths, drive letters, `..`, `.`, backslashes, colons or empty parts.
pub fn safe_relative(path: &str) -> Result<PathBuf, String> {
    let bad = || format!("the update names an unsafe path ({path:?})");
    if path.is_empty() || path.len() > 240 || path.contains(['\\', ':', '\0']) || path.starts_with('/') {
        return Err(bad());
    }
    let mut out = PathBuf::new();
    for part in path.split('/') {
        if part.is_empty() || part == "." || part == ".." || part.ends_with(['.', ' ']) {
            return Err(bad());
        }
        out.push(part);
    }
    if Path::new(&out).components().all(|c| matches!(c, Component::Normal(_))) { Ok(out) } else { Err(bad()) }
}

/// A payload file name: letters, digits, `.`, `-` and `_`, not starting with a dot.
pub(crate) fn safe_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 200
        && !name.starts_with('.')
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Manifest {
        Manifest {
            format: FORMAT.into(),
            format_version: FORMAT_VERSION,
            app: "solara".into(),
            platform: "windows-x64".into(),
            from_version: "0.1.0".into(),
            to_version: "0.1.1".into(),
            files: vec![FileChange {
                path: "solara.exe".into(),
                action: Action::Patch,
                from_sha256: Some(sha256_bytes(b"old")),
                to_sha256: Some(sha256_bytes(b"new")),
                to_size: Some(3),
                payload: Some(Payload { name: "0-solara.exe.bsdiff.deflate".into(), size: 10, sha256: sha256_bytes(b"p") }),
            }],
        }
    }

    #[test]
    fn a_manifest_is_used_only_with_its_published_checksum() {
        let bytes = serde_json::to_vec(&sample()).unwrap();
        let sha = sha256_bytes(&bytes);
        assert_eq!(Manifest::parse_verified(&bytes, &sha).unwrap(), sample());
        assert_eq!(Manifest::parse_verified(&bytes, &sha.to_ascii_uppercase()).unwrap(), sample());
        let mut bad = bytes.clone();
        if let Some(b) = bad.iter_mut().find(|b| **b == b'1') {
            *b = b'2';
        }
        assert!(Manifest::parse_verified(&bad, &sha).unwrap_err().contains("doesn't match"));
        assert!(Manifest::parse_verified(&bytes, "").unwrap_err().contains("no valid published checksum"));
    }

    #[test]
    fn unsafe_paths_and_names_are_refused() {
        for p in ["", "/abs", "..", "a/../b", "a\\b", "C:/x", "a//b", "./a", "a/", "con.", "x\0"] {
            assert!(safe_relative(p).is_err(), "{p:?}");
        }
        assert_eq!(safe_relative("sub/solara.exe").unwrap(), Path::new("sub").join("solara.exe"));
        let mut m = sample();
        m.files[0].path = "../evil.exe".into();
        assert!(m.validate().is_err());
        let mut m = sample();
        m.files[0].payload.as_mut().unwrap().name = "../p".into();
        assert!(m.validate().is_err());
    }

    #[test]
    fn each_action_must_carry_what_it_needs() {
        let mut m = sample();
        m.files[0].from_sha256 = None;
        assert!(m.validate().is_err(), "a patch needs the old file's hash");
        let mut m = sample();
        m.files[0].action = Action::Remove;
        assert!(m.validate().is_err(), "a removal has no payload");
        let mut m = sample();
        m.files.push(m.files[0].clone());
        assert!(m.validate().unwrap_err().contains("twice"));
        let mut m = sample();
        m.format_version = 2;
        assert!(m.validate().is_err());
    }
}
