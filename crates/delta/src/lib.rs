//! Solara delta updates: an update downloads and installs only the files that changed.
//!
//! A delta takes one install folder from one version to the next. It is a hash manifest,
//! `delta.json` ([`Manifest`]), plus one payload file per changed file: a binary patch of the old
//! file (bsdiff, then deflate) or, when that is not smaller, the whole new file (deflate). Unchanged
//! files are not in it at all.
//!
//! The steps, and who runs them:
//!
//! 1. Release side, [`make()`]: compare the old and new install folders and write the delta.
//! 2. The app, [`Manifest::parse_verified`] and [`stage()`]: check the manifest against its published
//!    SHA-256, check that the installed files are the ones the delta starts from, check every
//!    downloaded payload against its SHA-256, and build the new files in a staging folder, checked
//!    against their SHA-256 too. It writes a [`Plan`] and never touches its own install folder.
//! 3. The standalone updater, [`wait::wait_for_exit`] and [`swap::apply`]: wait for the app (and
//!    anything else running from the install folder) to exit, check every staged file again, and
//!    swap them in, rolling every file back if one step fails.
//!
//! Any failed check refuses the update and leaves the installed version as it was.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod codec;
mod hash;
mod make;
mod manifest;
mod plan;
mod stage;
pub mod swap;
pub mod wait;

pub use hash::{sha256_bytes, sha256_file};
pub use make::{MakeOptions, MakeReport, make};
pub use manifest::{Action, FORMAT, FORMAT_VERSION, FileChange, MANIFEST_NAME, Manifest, Payload, safe_relative};
pub use plan::{PLAN_NAME, Plan, RESULT_NAME, Relaunch, Step, UpdateResult};
pub use stage::{check_install, stage};

/// The platform tag a delta is made for, such as `windows-x64`: an update only applies to a copy
/// built for the same one.
pub fn current_platform() -> String {
    let os = std::env::consts::OS;
    let arch = match std::env::consts::ARCH {
        "x86_64" => "x64",
        "x86" => "x86",
        "aarch64" => "arm64",
        other => other,
    };
    format!("{os}-{arch}")
}

/// The largest file a delta reads or writes whole (the app is well under it).
pub const MAX_FILE: u64 = 1 << 30;
