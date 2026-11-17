//! What the app hands the standalone updater (`plan.json`), and what the updater reports back
//! (`last-result.json`).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::hash::is_sha256;
use crate::manifest::safe_relative;

/// The plan's file name in the staging folder.
pub const PLAN_NAME: &str = "plan.json";
/// The updater's report, in the folder that holds the staging folders; the app reads it at its next start.
pub const RESULT_NAME: &str = "last-result.json";

/// Everything the updater needs: which files to swap, with their hashes before and after.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Plan {
    pub app: String,
    pub from_version: String,
    pub to_version: String,
    /// The folder the running copy was started from (absolute).
    pub install_dir: PathBuf,
    /// Where the new files wait (absolute); every `Step::staged` is inside it.
    pub stage_dir: PathBuf,
    pub steps: Vec<Step>,
    /// What to start once the update is in (or refused, so the old version comes back).
    pub relaunch: Option<Relaunch>,
    /// Where the updater writes its [`UpdateResult`].
    pub result_file: PathBuf,
    /// What the update downloaded (manifest and payloads), for the report.
    pub downloaded_bytes: u64,
}

/// One file to change.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Step {
    /// Relative to the install folder, `/`-separated.
    pub path: String,
    /// The new file, inside the staging folder; `None` removes the file.
    pub staged: Option<PathBuf>,
    /// The installed file before; `None` when the update adds it.
    pub from_sha256: Option<String>,
    /// The file after; `None` when the update removes it.
    pub to_sha256: Option<String>,
}

/// The program to start after the update, relative to the install folder, with its arguments.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Relaunch {
    pub exe: String,
    pub args: Vec<String>,
}

/// How an update ended, for the app to show at its next start.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateResult {
    pub ok: bool,
    pub from_version: String,
    pub to_version: String,
    pub message: String,
    pub downloaded_bytes: u64,
}

impl Plan {
    pub fn read(path: &Path) -> Result<Self, String> {
        let bytes = crate::make::read(path)?;
        let plan: Plan = serde_json::from_slice(&bytes).map_err(|e| format!("the update plan is unreadable ({e})"))?;
        plan.validate()?;
        Ok(plan)
    }

    pub fn write(&self, path: &Path) -> Result<(), String> {
        let json = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        crate::make::write(path, &json)
    }

    /// Absolute folders, safe relative paths, staged files inside the staging folder, well-formed hashes.
    pub fn validate(&self) -> Result<(), String> {
        if !self.install_dir.is_absolute() || !self.stage_dir.is_absolute() {
            return Err("the update plan's folders are not absolute".into());
        }
        if self.steps.len() > 10_000 {
            return Err("the update plan has too many steps".into());
        }
        for s in &self.steps {
            safe_relative(&s.path)?;
            let hash_ok = |h: &Option<String>| h.as_deref().is_none_or(is_sha256);
            if !hash_ok(&s.from_sha256) || !hash_ok(&s.to_sha256) || s.staged.is_some() != s.to_sha256.is_some() {
                return Err(format!("the update plan's step for {} is malformed", s.path));
            }
            if s.staged.as_ref().is_some_and(|p| !p.starts_with(&self.stage_dir) || p.components().any(|c| c == std::path::Component::ParentDir)) {
                return Err(format!("the update plan stages {} outside its folder", s.path));
            }
        }
        if let Some(r) = &self.relaunch {
            safe_relative(&r.exe)?;
        }
        Ok(())
    }
}

impl UpdateResult {
    pub fn read(path: &Path) -> Result<Self, String> {
        let bytes = crate::make::read(path)?;
        serde_json::from_slice(&bytes).map_err(|e| format!("the update result is unreadable ({e})"))
    }

    pub fn write(&self, path: &Path) -> Result<(), String> {
        let json = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        crate::make::write(path, &json)
    }
}
