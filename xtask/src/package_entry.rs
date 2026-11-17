//! `cargo xtask package-entry`: write one package's entry in a SolPlugs package index from its
//! release zips (sizes and SHA-256 of each zip and of every file inside it).
//!
//! ```text
//! cargo xtask package-entry --index index.json --id song-sense --name "Song Sense" --version 1.0.1 \
//!   --summary "Song analysis" [--vendor "Zee Creations"] [--homepage https://…] [--platform windows-x64] \
//!   --zip vst3 Song-Sense-1.0.1-VST3.zip https://github.com/…/Song-Sense-1.0.1-VST3.zip \
//!   --zip clap Song-Sense-1.0.1-CLAP.zip https://github.com/…/Song-Sense-1.0.1-CLAP.zip
//! ```
//!
//! The entry replaces the package's old one (or is added), and the whole index is checked as
//! Solara checks it before it is written. Without `--index` the entry is printed.

use std::collections::BTreeSet;
use std::io::Read;
use std::path::Path;

use soundcraft_packages::{Download, FileEntry, Format, INDEX_FORMAT, INDEX_FORMAT_VERSION, Index, Package};

const USAGE: &str = "usage: cargo xtask package-entry [--index FILE] --id ID --name NAME --version V [--summary S] [--vendor V] [--homepage URL] [--platform P] --zip vst3|clap ZIP URL [--zip …]";

pub fn run(args: &[String]) -> Result<(), String> {
    let get = |name: &str| -> Option<String> { args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned() };
    let need = |name: &str| get(name).ok_or_else(|| format!("{name} is required\n\n{USAGE}"));
    let platform = get("--platform").unwrap_or_else(|| "windows-x64".into());
    let mut downloads = Vec::new();
    for (i, a) in args.iter().enumerate() {
        if a != "--zip" {
            continue;
        }
        let (Some(format), Some(zip), Some(url)) = (args.get(i + 1), args.get(i + 2), args.get(i + 3)) else {
            return Err(format!("--zip takes a format, a zip file and its URL\n\n{USAGE}"));
        };
        let format = match format.as_str() {
            "vst3" => Format::Vst3,
            "clap" => Format::Clap,
            other => return Err(format!("unknown format {other} (vst3 or clap)")),
        };
        downloads.push(download(format, Path::new(zip), url, &platform)?);
    }
    if downloads.is_empty() {
        return Err(format!("at least one --zip is required\n\n{USAGE}"));
    }
    let pkg = Package {
        id: need("--id")?,
        name: need("--name")?,
        version: need("--version")?,
        summary: get("--summary").unwrap_or_default(),
        vendor: get("--vendor").unwrap_or_default(),
        homepage: get("--homepage").unwrap_or_default(),
        downloads,
    };
    let Some(path) = get("--index") else {
        let one = Index { format: INDEX_FORMAT.into(), format_version: INDEX_FORMAT_VERSION, name: String::new(), packages: vec![pkg.clone()] };
        one.validate()?;
        println!("{}", serde_json::to_string_pretty(&pkg).map_err(|e| e.to_string())?);
        return Ok(());
    };
    let mut index = match std::fs::read(&path) {
        Ok(bytes) => Index::parse(&bytes)?,
        Err(_) => Index { format: INDEX_FORMAT.into(), format_version: INDEX_FORMAT_VERSION, name: "SolPlugs".into(), packages: Vec::new() },
    };
    match index.packages.iter_mut().find(|p| p.id == pkg.id) {
        // Downloads for other platforms stay; this platform's are replaced.
        Some(old) => {
            let mut kept: Vec<Download> = old.downloads.iter().filter(|d| d.platform != platform).cloned().collect();
            kept.extend(pkg.downloads.clone());
            *old = Package { downloads: kept, ..pkg.clone() };
        }
        None => index.packages.push(pkg.clone()),
    }
    index.validate()?;
    let json = serde_json::to_string_pretty(&index).map_err(|e| e.to_string())?;
    std::fs::write(&path, format!("{json}\n")).map_err(|e| format!("{path}: {e}"))?;
    println!("{} {} written to {path}", pkg.id, pkg.version);
    Ok(())
}

fn download(format: Format, zip_path: &Path, url: &str, platform: &str) -> Result<Download, String> {
    let bytes = std::fs::read(zip_path).map_err(|e| format!("{}: {e}", zip_path.display()))?;
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(&bytes)).map_err(|e| format!("{}: {e}", zip_path.display()))?;
    let mut files = Vec::new();
    let mut bundles = BTreeSet::new();
    for i in 0..zip.len() {
        let mut e = zip.by_index(i).map_err(|e| e.to_string())?;
        if e.is_dir() {
            continue;
        }
        let name = e.name().map_err(|e| e.to_string())?.into_owned();
        let mut data = Vec::new();
        e.read_to_end(&mut data).map_err(|e| format!("{name}: {e}"))?;
        bundles.insert(name.split('/').next().unwrap_or_default().to_string());
        files.push(FileEntry { path: name, size: data.len() as u64, sha256: soundcraft_delta::sha256_bytes(&data) });
    }
    if bundles.len() != 1 {
        return Err(format!("{} must hold exactly one bundle at its top; it holds {bundles:?}", zip_path.display()));
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(Download {
        platform: platform.into(),
        format,
        url: url.into(),
        size: bytes.len() as u64,
        sha256: soundcraft_delta::sha256_bytes(&bytes),
        bundle: bundles.into_iter().next().unwrap_or_default(),
        files,
    })
}
