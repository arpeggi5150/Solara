//! Solara's package manager for SolPlugs: browse a package index, then install, update and remove
//! plugins (VST3 and CLAP) without leaving Solara.
//!
//! - [`Index`]: the package repository, one JSON file (`index.json` in the SolPlugs repository).
//!   Each package lists, per platform and plugin format, one zip with its size and SHA-256 and the
//!   SHA-256 of every file inside it.
//! - [`roots`]: where plugins live on this computer (the per-user and the for-everyone folders).
//! - [`Record`]: what Solara installed, and where.
//! - [`install`]: unpack a downloaded zip into a staging folder, check every file, and swap the
//!   package in through `soundcraft-delta`'s all-or-none swap. Any failed check refuses the change
//!   and leaves the installed version as it was.
//!
//! One package is one download: updating a plugin fetches only that plugin's zips, never the rest.
//! Downloading is the caller's job (the app uses its HTTP client), so this crate only reads files.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod index;
pub mod install;
mod record;
pub mod roots;
mod version;

pub use index::{Download, FileEntry, Format, INDEX_FORMAT, INDEX_FORMAT_VERSION, Index, MAX_DOWNLOAD, Package};
pub use record::{InstalledBundle, InstalledPackage, RECORD_NAME, Record};
pub use version::is_newer;

/// The package index Solara reads by default: `index.json` on the SolPlugs repository's main branch.
pub const DEFAULT_INDEX_URL: &str = "https://raw.githubusercontent.com/arpeggi5150/SolPlugs/main/index.json";

/// The platform tag packages are published for, such as `windows-x64`.
pub fn current_platform() -> String {
    soundcraft_delta::current_platform()
}
