//! The app's side: check that a delta fits this copy, check the downloaded payloads, and build
//! the new files in a staging folder. Nothing in the install folder is written.

use std::path::Path;

use crate::codec;
use crate::hash::{sha256_bytes, sha256_file};
use crate::make::{read, write};
use crate::manifest::{Action, Manifest, safe_relative};
use crate::plan::Step;

/// Whether `m` takes this copy (app `app`, `platform`, `running_version`, installed in
/// `install_dir`) to a new version: every file it changes must be exactly the file it starts from.
pub fn check_install(m: &Manifest, install_dir: &Path, app: &str, platform: &str, running_version: &str) -> Result<(), String> {
    if m.app != app {
        return Err(format!("the update is for {}, not {app}", m.app));
    }
    if m.platform != platform {
        return Err(format!("the update is for {}, this copy is {platform}", m.platform));
    }
    if m.from_version != running_version {
        return Err(format!("the update starts from {}, this copy is {running_version}", m.from_version));
    }
    for f in &m.files {
        let path = install_dir.join(safe_relative(&f.path)?);
        match &f.from_sha256 {
            Some(from) => {
                let have = sha256_file(&path).map_err(|e| format!("{e}; this copy doesn't match {} {}", app, m.from_version))?;
                if &have != from {
                    return Err(format!("{} in this copy isn't the {} {} file, so the update was refused", f.path, app, m.from_version));
                }
            }
            None if path.exists() => return Err(format!("the update adds {}, which this copy already has", f.path)),
            None => {}
        }
    }
    Ok(())
}

/// Check every payload in `payload_dir` against the manifest and build each new file in
/// `stage_dir/files`, checked against its own SHA-256. Returns the updater's steps.
pub fn stage(m: &Manifest, install_dir: &Path, payload_dir: &Path, stage_dir: &Path) -> Result<Vec<Step>, String> {
    let files_dir = stage_dir.join("files");
    let mut steps = Vec::with_capacity(m.files.len());
    for (i, f) in m.files.iter().enumerate() {
        let rel = safe_relative(&f.path)?;
        let built = match (f.action, &f.payload) {
            (Action::Remove, _) => None,
            (action, Some(p)) => {
                let payload = read(&payload_dir.join(&p.name))?;
                if payload.len() as u64 != p.size || sha256_bytes(&payload) != p.sha256 {
                    return Err(format!("the downloaded payload for {} doesn't match its checksum, so the update was refused", f.path));
                }
                let max = f.to_size.unwrap_or(0);
                let new = if action == Action::Patch {
                    let old = read(&install_dir.join(&rel))?;
                    if f.from_sha256.as_deref() != Some(sha256_bytes(&old).as_str()) {
                        return Err(format!("{} changed since the update was checked", f.path));
                    }
                    codec::apply_patch(&old, &payload, max)?
                } else {
                    codec::inflate(&payload, max)?
                };
                if Some(new.len() as u64) != f.to_size || f.to_sha256.as_deref() != Some(sha256_bytes(&new).as_str()) {
                    return Err(format!("the rebuilt {} doesn't match its checksum, so the update was refused", f.path));
                }
                let staged = files_dir.join(i.to_string());
                write(&staged, &new)?;
                Some(staged)
            }
            (_, None) => return Err(format!("the update's entry for {} has no payload", f.path)),
        };
        steps.push(Step { path: f.path.clone(), staged: built, from_sha256: f.from_sha256.clone(), to_sha256: f.to_sha256.clone() });
    }
    Ok(steps)
}
