//! The package index: the JSON file that lists every package Solara can install.
//!
//! ```json
//! {
//!   "format": "solplugs-index",
//!   "format_version": 1,
//!   "name": "SolPlugs",
//!   "packages": [{
//!     "id": "song-sense",
//!     "name": "Song Sense",
//!     "version": "1.0.1",
//!     "summary": "Song analysis: key, tempo and sections",
//!     "vendor": "Zee Creations",
//!     "homepage": "https://github.com/arpeggi5150/Song-Sense-Releases",
//!     "downloads": [{
//!       "platform": "windows-x64",
//!       "format": "vst3",
//!       "url": "https://github.com/.../Song-Sense-1.0.1-VST3.zip",
//!       "size": 2379793,
//!       "sha256": "4bf3…",
//!       "bundle": "Song Sense.vst3",
//!       "files": [{"path": "Song Sense.vst3/Contents/x86_64-win/Song Sense.vst3", "size": 5364736, "sha256": "…"}]
//!     }]
//!   }]
//! }
//! ```
//!
//! Everything in an index is hostile input: [`Index::parse`] checks names, paths, sizes and
//! hashes before anything uses them. A download's `url` is `https://…`, or a path relative to the
//! index (for an index read from disk, such as a test repository).

use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use soundcraft_delta::safe_relative;

/// `format` of an index file.
pub const INDEX_FORMAT: &str = "solplugs-index";
/// The index format version this build reads.
pub const INDEX_FORMAT_VERSION: u32 = 1;
/// The largest zip (and the largest unpacked package) Solara downloads.
pub const MAX_DOWNLOAD: u64 = 1 << 30;
/// The largest index file Solara reads.
pub const MAX_INDEX_BYTES: usize = 4 << 20;
const MAX_PACKAGES: usize = 1000;
const MAX_FILES: usize = 2000;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Index {
    pub format: String,
    pub format_version: u32,
    #[serde(default)]
    pub name: String,
    pub packages: Vec<Package>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Package {
    /// Stable id: lowercase letters, digits and `-`.
    pub id: String,
    pub name: String,
    pub version: String,
    /// One short line for the browser.
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub vendor: String,
    /// Where the package comes from (its own releases page), shown in the browser.
    #[serde(default)]
    pub homepage: String,
    pub downloads: Vec<Download>,
}

/// A plugin format a package ships in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    Vst3,
    Clap,
}

impl Format {
    pub fn label(self) -> &'static str {
        match self {
            Format::Vst3 => "VST3",
            Format::Clap => "CLAP",
        }
    }

    /// The bundle's file extension.
    pub fn extension(self) -> &'static str {
        match self {
            Format::Vst3 => "vst3",
            Format::Clap => "clap",
        }
    }
}

/// One zip: a package's plugin in one format, for one platform.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Download {
    pub platform: String,
    pub format: Format,
    pub url: String,
    pub size: u64,
    pub sha256: String,
    /// The bundle at the top of the zip, such as `Song Sense.vst3` (a folder for VST3, a file or a
    /// folder for CLAP). It is installed into the format's plugin folder under this name.
    pub bundle: String,
    /// Every file in the zip, each under `bundle`, with its size and SHA-256.
    pub files: Vec<FileEntry>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileEntry {
    /// `/`-separated, starting with the bundle name.
    pub path: String,
    pub size: u64,
    pub sha256: String,
}

impl Index {
    /// Parse and check an index file.
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > MAX_INDEX_BYTES {
            return Err("the package index is too large".into());
        }
        let index: Index = serde_json::from_slice(bytes).map_err(|e| format!("the package index is unreadable ({e})"))?;
        index.validate()?;
        Ok(index)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.format != INDEX_FORMAT {
            return Err(format!("this isn't a SolPlugs package index (format {:?})", self.format));
        }
        if self.format_version != INDEX_FORMAT_VERSION {
            return Err(format!("the package index is format version {}; this Solara reads version {INDEX_FORMAT_VERSION}", self.format_version));
        }
        if self.packages.len() > MAX_PACKAGES {
            return Err("the package index lists too many packages".into());
        }
        let mut ids = HashSet::new();
        for p in &self.packages {
            p.validate()?;
            if !ids.insert(p.id.as_str()) {
                return Err(format!("the package index lists {} twice", p.id));
            }
        }
        Ok(())
    }

    pub fn package(&self, id: &str) -> Option<&Package> {
        self.packages.iter().find(|p| p.id == id)
    }
}

impl Package {
    fn validate(&self) -> Result<(), String> {
        let id_ok = !self.id.is_empty()
            && self.id.len() <= 64
            && self.id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            && !self.id.starts_with('-');
        if !id_ok {
            return Err(format!("the package index has a bad package id ({:?})", self.id));
        }
        let text_ok = |s: &str, max: usize| s.len() <= max && !s.chars().any(char::is_control);
        if self.name.trim().is_empty() || !text_ok(&self.name, 80) || !text_ok(&self.summary, 300) || !text_ok(&self.vendor, 80) {
            return Err(format!("package {} has a bad name, summary or vendor", self.id));
        }
        if !self.homepage.is_empty() && (!self.homepage.starts_with("https://") || !text_ok(&self.homepage, 300)) {
            return Err(format!("package {} has a bad homepage", self.id));
        }
        let version_ok = !self.version.is_empty()
            && self.version.len() <= 40
            && self.version.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'+'));
        if !version_ok {
            return Err(format!("package {} has a bad version ({:?})", self.id, self.version));
        }
        if self.downloads.len() > 32 {
            return Err(format!("package {} lists too many downloads", self.id));
        }
        let mut seen = HashSet::new();
        for d in &self.downloads {
            d.validate(&self.id)?;
            if !seen.insert((d.platform.as_str(), d.format)) {
                return Err(format!("package {} lists {} for {} twice", self.id, d.format.label(), d.platform));
            }
        }
        Ok(())
    }

    /// This package's downloads for `platform`, VST3 first.
    pub fn downloads_for(&self, platform: &str) -> Vec<&Download> {
        let mut v: Vec<&Download> = self.downloads.iter().filter(|d| d.platform == platform).collect();
        v.sort_by_key(|d| d.format);
        v
    }

    /// The bytes one install of this package downloads on `platform`.
    pub fn download_bytes(&self, platform: &str) -> u64 {
        self.downloads_for(platform).iter().fold(0u64, |n, d| n.saturating_add(d.size))
    }
}

impl Download {
    fn validate(&self, id: &str) -> Result<(), String> {
        let bad = |what: &str| format!("package {id}'s {} download has a bad {what}", self.format.label());
        let platform_ok =
            !self.platform.is_empty() && self.platform.len() <= 32 && self.platform.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-');
        if !platform_ok {
            return Err(bad("platform"));
        }
        if !url_ok(&self.url) {
            return Err(bad("url"));
        }
        if self.size == 0 || self.size > MAX_DOWNLOAD {
            return Err(bad("size"));
        }
        if !is_sha256(&self.sha256) {
            return Err(bad("SHA-256"));
        }
        // The bundle is one plain name with the format's extension: it never leaves the plugin folder.
        let ext_ok = self.bundle.rsplit_once('.').is_some_and(|(stem, ext)| !stem.is_empty() && ext.eq_ignore_ascii_case(self.format.extension()));
        if self.bundle.contains('/') || safe_relative(&self.bundle).is_err() || !ext_ok {
            return Err(bad("bundle name"));
        }
        if self.files.is_empty() || self.files.len() > MAX_FILES {
            return Err(bad("file list"));
        }
        let mut total = 0u64;
        let mut paths = HashSet::new();
        for f in &self.files {
            let under = f.path == self.bundle || f.path.strip_prefix(self.bundle.as_str()).is_some_and(|rest| rest.starts_with('/'));
            if safe_relative(&f.path).is_err() || !under || !is_sha256(&f.sha256) || f.size > MAX_DOWNLOAD {
                return Err(format!("package {id} lists a bad file ({:?})", f.path));
            }
            if !paths.insert(f.path.to_ascii_lowercase()) {
                return Err(format!("package {id} lists {} twice", f.path));
            }
            total = total.saturating_add(f.size);
        }
        if total > MAX_DOWNLOAD {
            return Err(bad("unpacked size"));
        }
        Ok(())
    }

    /// Where to fetch the zip: its `https://` URL, or its path next to the index at `index_source`.
    pub fn resolve(&self, index_source: &str) -> String {
        if self.url.starts_with("https://") {
            return self.url.clone();
        }
        match index_source.rsplit_once(['/', '\\']) {
            Some((dir, _)) => format!("{dir}/{}", self.url),
            None => self.url.clone(),
        }
    }
}

fn url_ok(url: &str) -> bool {
    if url.len() > 2000 || url.chars().any(|c| c.is_control() || c == ' ') {
        return false;
    }
    if let Some(rest) = url.strip_prefix("https://") {
        return !rest.is_empty() && !rest.starts_with('/');
    }
    // A relative path next to the index: no scheme, no way out of the index's folder.
    !url.contains(':') && safe_relative(url).is_ok()
}

pub(crate) fn is_sha256(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) const H: &str = "4bf377740b6562d1d84f538b4f0f9dde0d782a6f9c49d477a52f6e63cf1ce63d";

    pub(crate) fn sample() -> Index {
        Index {
            format: INDEX_FORMAT.into(),
            format_version: 1,
            name: "SolPlugs".into(),
            packages: vec![Package {
                id: "song-sense".into(),
                name: "Song Sense".into(),
                version: "1.0.1".into(),
                summary: "Song analysis".into(),
                vendor: "Zee Creations".into(),
                homepage: "https://github.com/arpeggi5150/Song-Sense-Releases".into(),
                downloads: vec![
                    Download {
                        platform: "windows-x64".into(),
                        format: Format::Clap,
                        url: "https://example.com/c.zip".into(),
                        size: 10,
                        sha256: H.into(),
                        bundle: "Song Sense.clap".into(),
                        files: vec![FileEntry { path: "Song Sense.clap".into(), size: 3, sha256: H.into() }],
                    },
                    Download {
                        platform: "windows-x64".into(),
                        format: Format::Vst3,
                        url: "zips/v.zip".into(),
                        size: 10,
                        sha256: H.into(),
                        bundle: "Song Sense.vst3".into(),
                        files: vec![FileEntry { path: "Song Sense.vst3/Contents/x86_64-win/Song Sense.vst3".into(), size: 3, sha256: H.into() }],
                    },
                ],
            }],
        }
    }

    fn rejects(f: impl FnOnce(&mut Index)) {
        let mut i = sample();
        f(&mut i);
        let json = serde_json::to_vec(&i).unwrap();
        assert!(Index::parse(&json).is_err(), "accepted {i:?}");
    }

    #[test]
    fn reads_a_good_index() {
        let json = serde_json::to_vec(&sample()).unwrap();
        let i = Index::parse(&json).unwrap();
        let p = i.package("song-sense").unwrap();
        let d = p.downloads_for("windows-x64");
        assert_eq!(d.iter().map(|d| d.format).collect::<Vec<_>>(), vec![Format::Vst3, Format::Clap]);
        assert_eq!(p.download_bytes("windows-x64"), 20);
        assert!(p.downloads_for("linux-x64").is_empty());
    }

    #[test]
    fn rejects_hostile_entries() {
        rejects(|i| i.format = "other".into());
        rejects(|i| i.format_version = 2);
        rejects(|i| i.packages[0].id = "../x".into());
        rejects(|i| i.packages[0].id = "Song".into());
        rejects(|i| i.packages[0].version = "1.0 beta".into());
        rejects(|i| i.packages[0].name = String::new());
        rejects(|i| i.packages[0].homepage = "http://plain".into());
        rejects(|i| i.packages.push(i.packages[0].clone()));
        rejects(|i| i.packages[0].downloads[0].url = "http://example.com/c.zip".into());
        rejects(|i| i.packages[0].downloads[0].url = "file:///c.zip".into());
        rejects(|i| i.packages[0].downloads[0].url = "../c.zip".into());
        rejects(|i| i.packages[0].downloads[0].url = "C:/c.zip".into());
        rejects(|i| i.packages[0].downloads[0].sha256 = H.to_uppercase());
        rejects(|i| i.packages[0].downloads[0].size = 0);
        rejects(|i| i.packages[0].downloads[0].size = MAX_DOWNLOAD + 1);
        rejects(|i| i.packages[0].downloads[0].bundle = "../Song Sense.clap".into());
        rejects(|i| i.packages[0].downloads[0].bundle = "x/Song Sense.clap".into());
        rejects(|i| i.packages[0].downloads[0].bundle = "Song Sense.vst3".into());
        rejects(|i| i.packages[0].downloads[0].files[0].path = "Other.clap".into());
        rejects(|i| i.packages[0].downloads[1].files[0].path = "Song Sense.vst3/../../x".into());
        rejects(|i| i.packages[0].downloads[1].files[0].path = "Song Sense.vst3x/a".into());
        rejects(|i| i.packages[0].downloads[0].files.clear());
        rejects(|i| {
            let f = i.packages[0].downloads[1].files[0].clone();
            i.packages[0].downloads[1].files.push(FileEntry { path: f.path.to_uppercase().replace("SONG SENSE.VST3", "Song Sense.vst3"), ..f });
        });
        rejects(|i| i.packages[0].downloads[1].format = Format::Clap);
        assert!(Index::parse(b"not json").is_err());
        assert!(Index::parse(&vec![b' '; MAX_INDEX_BYTES + 1]).is_err());
    }

    #[test]
    fn resolves_relative_downloads_next_to_the_index() {
        let i = sample();
        let d = &i.packages[0].downloads;
        assert_eq!(d[0].resolve("C:\\repo\\index.json"), "https://example.com/c.zip");
        assert_eq!(d[1].resolve("C:\\repo\\index.json"), "C:\\repo/zips/v.zip");
        assert_eq!(d[1].resolve("https://raw.example.com/main/index.json"), "https://raw.example.com/main/zips/v.zip");
    }
}
