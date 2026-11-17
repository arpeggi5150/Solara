//! Installing, updating and removing one package.
//!
//! 1. [`unpack`]: unpack each downloaded zip (already checked against the index's SHA-256) into a
//!    staging folder. Every file must be one the index lists, under the package's bundle, with the
//!    listed size and SHA-256; anything else refuses the install.
//! 2. [`plan_install`] / [`plan_remove`]: the file changes under the plugin root, as a
//!    `soundcraft-delta` [`Plan`]: each new file with its SHA-256, each file it replaces or removes
//!    with the SHA-256 it has now.
//! 3. [`apply`]: `soundcraft-delta`'s swap, all or none. If one file can't be put in place, every
//!    file goes back and the old version stays. Empty bundle folders are tidied away afterwards.
//!
//! Only the package's own bundles change: `VST3/<bundle>` and `CLAP/<bundle>` under one root.

use std::io::Read;
use std::path::{Path, PathBuf};

use soundcraft_delta::{Plan, Step, safe_relative, sha256_file, swap};

use crate::index::{Download, FileEntry, Format, MAX_DOWNLOAD, Package};
use crate::record::{InstalledBundle, InstalledPackage, Record};
use crate::roots::{Os, bundle_rel, format_dir};

/// Left by the swap beside a file it replaced when the old one couldn't be deleted yet (a plugin
/// a running program still has loaded); deleted by [`sweep`] later.
const OLD_SUFFIX: &str = ".solara-old";
const NEW_SUFFIX: &str = ".solara-new";
/// Upper bound on files visited in one bundle folder.
const MAX_WALK: usize = 20_000;

/// One unpacked, checked bundle in the staging folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unpacked {
    pub format: Format,
    /// `VST3/Song Sense.vst3`.
    pub bundle: String,
    /// Each file: path relative to the root, the staged copy, its SHA-256.
    pub files: Vec<(String, PathBuf, String)>,
}

/// Unpack the zip at `zip_path` for download `d` into `stage` and check every file against the index.
pub fn unpack(zip_path: &Path, d: &Download, os: Os, stage: &Path) -> Result<Unpacked, String> {
    let name = zip_path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let file = std::fs::File::open(zip_path).map_err(|e| format!("couldn't open {name} ({e})"))?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| format!("{name} isn't a readable zip ({e})"))?;
    if zip.len() > d.files.len().saturating_mul(2).saturating_add(64) {
        return Err(format!("{name} holds more entries than the index lists"));
    }
    let dir = format_dir(os, d.format);
    let mut seen = std::collections::HashSet::new();
    let mut files = Vec::new();
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|e| format!("{name} is damaged ({e})"))?;
        let raw = entry.name().map_err(|e| format!("{name} has an unreadable file name ({e})"))?.into_owned();
        if entry.is_dir() {
            continue;
        }
        let Some(listed) = d.files.iter().find(|f| f.path == raw) else {
            return Err(format!("{name} holds {raw:?}, which the package index doesn't list, so it wasn't installed"));
        };
        if !seen.insert(listed.path.clone()) {
            return Err(format!("{name} holds {raw:?} twice"));
        }
        let rel = safe_relative(&listed.path)?;
        let dest = stage.join(dir).join(rel);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("couldn't create {} ({e})", parent.display()))?;
        }
        let mut out = std::fs::File::create(&dest).map_err(|e| format!("couldn't write {} ({e})", dest.display()))?;
        // Never more than the index says (a zip can claim any size).
        let limit = listed.size.min(MAX_DOWNLOAD).saturating_add(1);
        let n = std::io::copy(&mut (&mut entry).take(limit), &mut out).map_err(|e| format!("couldn't unpack {raw} from {name} ({e})"))?;
        drop(out);
        if n != listed.size || sha256_file(&dest)? != listed.sha256 {
            return Err(format!("{raw} in {name} doesn't match its published checksum, so it wasn't installed"));
        }
        files.push((format!("{dir}/{}", listed.path), dest, listed.sha256.clone()));
    }
    if let Some(missing) = d.files.iter().find(|f| !seen.contains(&f.path)) {
        return Err(format!("{name} is missing {}, so it wasn't installed", missing.path));
    }
    Ok(Unpacked { format: d.format, bundle: bundle_rel(os, d.format, &d.bundle), files })
}

/// A change ready to apply, and what the package looks like afterwards.
#[derive(Clone, Debug)]
pub struct Change {
    pub plan: Plan,
    /// The package's bundles after the change (empty for a removal).
    pub bundles: Vec<InstalledBundle>,
}

/// Install (or update to) `pkg` from its unpacked bundles under `root`. Files the bundles hold now
/// but the new version doesn't are removed; so are the bundles of `previous` (the copy installed
/// under the same root) that the new version no longer has.
pub fn plan_install(pkg: &Package, unpacked: &[Unpacked], root: &Path, stage: &Path, previous: Option<&InstalledPackage>) -> Result<Change, String> {
    let mut steps = Vec::new();
    let mut new_paths = std::collections::HashSet::new();
    let mut bundles = Vec::new();
    for u in unpacked {
        let mut entries = Vec::new();
        for (path, staged, sha) in &u.files {
            new_paths.insert(path.to_ascii_lowercase());
            steps.push(Step {
                path: path.clone(),
                staged: Some(staged.clone()),
                from_sha256: current_hash(root, path)?,
                to_sha256: Some(sha.clone()),
            });
            let size = std::fs::metadata(staged).map(|m| m.len()).unwrap_or(0);
            entries.push(FileEntry { path: path.clone(), size, sha256: sha.clone() });
        }
        bundles.push(InstalledBundle { format: u.format, path: u.bundle.clone(), files: entries });
    }
    let mut old_bundles: Vec<String> = unpacked.iter().map(|u| u.bundle.clone()).collect();
    if let Some(p) = previous.filter(|p| p.root == root) {
        old_bundles.extend(p.bundles.iter().map(|b| b.path.clone()));
    }
    old_bundles.sort();
    old_bundles.dedup();
    for b in &old_bundles {
        for path in walk(root, b)? {
            if !new_paths.contains(&path.to_ascii_lowercase()) {
                steps.push(Step { path: path.clone(), staged: None, from_sha256: current_hash(root, &path)?, to_sha256: None });
            }
        }
    }
    let from = previous.map(|p| p.version.clone()).unwrap_or_default();
    Ok(Change { plan: plan(root, stage, &from, &pkg.version, steps, pkg.download_bytes(&crate::current_platform())), bundles })
}

/// Remove the bundles `bundles` (relative to `root`, `VST3/Song Sense.vst3`) and everything in them.
pub fn plan_remove(root: &Path, bundles: &[String], stage: &Path, version: &str) -> Result<Change, String> {
    let mut steps = Vec::new();
    for b in bundles {
        safe_relative(b)?;
        for path in walk(root, b)? {
            steps.push(Step { path: path.clone(), staged: None, from_sha256: current_hash(root, &path)?, to_sha256: None });
        }
    }
    Ok(Change { plan: plan(root, stage, version, "", steps, 0), bundles: Vec::new() })
}

fn plan(root: &Path, stage: &Path, from: &str, to: &str, steps: Vec<Step>, bytes: u64) -> Plan {
    Plan {
        app: "solplugs".into(),
        from_version: from.into(),
        to_version: to.into(),
        install_dir: root.to_path_buf(),
        stage_dir: stage.to_path_buf(),
        steps,
        relaunch: None,
        result_file: stage.join("result.json"),
        downloaded_bytes: bytes,
    }
}

/// Swap a change in, all or none, then tidy away the bundle folders it emptied.
pub fn apply(plan: &Plan) -> Result<(), String> {
    swap::apply(plan)?;
    tidy(plan);
    Ok(())
}

/// After a swap (here or in an administrator copy of the updater): remove empty folders inside
/// each changed bundle, and the bundle folder itself when it is empty.
pub fn tidy(plan: &Plan) {
    let mut bundles: Vec<&str> = plan.steps.iter().filter_map(|s| bundle_of(&s.path)).collect();
    bundles.sort_unstable();
    bundles.dedup();
    for b in bundles {
        if let Ok(rel) = safe_relative(b) {
            prune_empty(&plan.install_dir.join(rel));
        }
    }
}

/// `VST3/Song Sense.vst3/Contents/x` → `VST3/Song Sense.vst3`.
fn bundle_of(path: &str) -> Option<&str> {
    let mut parts = path.splitn(3, '/');
    let dir = parts.next()?;
    let bundle = parts.next()?;
    Some(&path[..dir.len() + 1 + bundle.len()])
}

/// Delete `dir` and the folders under it that hold no files (a bundle left with files stays).
fn prune_empty(dir: &Path) -> bool {
    if !dir.is_dir() {
        return false;
    }
    let mut empty = true;
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = e.path();
        if e.file_type().is_ok_and(|t| t.is_dir()) {
            empty &= prune_empty(&p);
        } else {
            empty = false;
        }
    }
    empty && std::fs::remove_dir(dir).is_ok()
}

/// Delete what earlier swaps couldn't (`.solara-old` / `.solara-new` copies) inside `bundles`
/// under `root`. One still locked stays for next time.
pub fn sweep(root: &Path, bundles: &[String]) {
    for b in bundles {
        let Ok(rel) = safe_relative(b) else { continue };
        let base = root.join(rel);
        let mut stack = vec![base];
        let mut budget = MAX_WALK;
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
                budget = budget.saturating_sub(1);
                if budget == 0 {
                    return;
                }
                let p = e.path();
                if e.file_type().is_ok_and(|t| t.is_dir()) {
                    stack.push(p);
                } else if p.to_string_lossy().ends_with(OLD_SUFFIX) || p.to_string_lossy().ends_with(NEW_SUFFIX) {
                    let _ = std::fs::remove_file(&p);
                }
            }
        }
    }
}

/// Every file of the bundle `bundle` under `root` (a CLAP bundle can be one file), relative to
/// `root`, `/`-separated. Swap leftovers are skipped ([`sweep`] handles them).
fn walk(root: &Path, bundle: &str) -> Result<Vec<String>, String> {
    let base = root.join(safe_relative(bundle)?);
    let meta = match std::fs::symlink_metadata(&base) {
        Ok(m) => m,
        Err(_) => return Ok(Vec::new()),
    };
    if meta.is_file() {
        return Ok(vec![bundle.to_string()]);
    }
    if !meta.is_dir() {
        return Err(format!("{} isn't a file or a folder, so it was left alone", base.display()));
    }
    let mut out = Vec::new();
    let mut stack = vec![(base, bundle.to_string())];
    while let Some((dir, rel)) = stack.pop() {
        let entries = std::fs::read_dir(&dir).map_err(|e| format!("couldn't read {} ({e})", dir.display()))?;
        for e in entries {
            let e = e.map_err(|e| format!("couldn't read {} ({e})", dir.display()))?;
            let name = e.file_name().to_string_lossy().into_owned();
            let child = format!("{rel}/{name}");
            let t = e.file_type().map_err(|e| e.to_string())?;
            if t.is_symlink() {
                return Err(format!("{} is a link, so the package was left alone", e.path().display()));
            }
            if t.is_dir() {
                stack.push((e.path(), child));
            } else if !name.ends_with(OLD_SUFFIX) && !name.ends_with(NEW_SUFFIX) {
                safe_relative(&child)?;
                out.push(child);
            }
            if out.len() + stack.len() > MAX_WALK {
                return Err(format!("{} holds too many files", root.join(bundle).display()));
            }
        }
    }
    out.sort();
    Ok(out)
}

fn current_hash(root: &Path, path: &str) -> Result<Option<String>, String> {
    let p = root.join(safe_relative(path)?);
    if p.is_file() { sha256_file(&p).map(Some) } else { Ok(None) }
}

/// A package found on this computer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    pub root: PathBuf,
    /// The installed version, when known: from Solara's record, or the index's version when every
    /// file matches it. `None`: some other version, put there by something else.
    pub version: Option<String>,
    /// Its bundles, relative to `root`.
    pub bundles: Vec<String>,
    /// Installed by Solara's package manager (it is in the record).
    pub recorded: bool,
}

/// Look for `pkg` on this computer: first in Solara's record, then in each of `roots` (this
/// account's first), by its bundles' names.
pub fn detect(pkg: &Package, platform: &str, os: Os, roots: &[PathBuf], record: &Record) -> Option<Found> {
    if let Some(p) = record.get(&pkg.id) {
        let bundles: Vec<String> = p.bundles.iter().map(|b| b.path.clone()).filter(|b| exists(&p.root, b)).collect();
        if !bundles.is_empty() {
            return Some(Found { root: p.root.clone(), version: Some(p.version.clone()), bundles, recorded: true });
        }
    }
    let downloads = pkg.downloads_for(platform);
    for root in roots {
        let present: Vec<&Download> = downloads.iter().copied().filter(|d| exists(root, &bundle_rel(os, d.format, &d.bundle))).collect();
        if present.is_empty() {
            continue;
        }
        let current = present.iter().all(|d| {
            d.files.iter().all(|f| {
                let rel = format!("{}/{}", format_dir(os, d.format), f.path);
                matches!(current_hash(root, &rel), Ok(Some(h)) if h == f.sha256)
            })
        });
        return Some(Found {
            root: root.clone(),
            version: current.then(|| pkg.version.clone()),
            bundles: present.iter().map(|d| bundle_rel(os, d.format, &d.bundle)).collect(),
            recorded: false,
        });
    }
    None
}

fn exists(root: &Path, rel: &str) -> bool {
    safe_relative(rel).is_ok_and(|r| root.join(r).exists())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::{INDEX_FORMAT, Index};
    use soundcraft_delta::sha256_bytes;
    use std::io::Write;

    struct Tmp(PathBuf);
    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn tmp(name: &str) -> Tmp {
        let d = std::env::temp_dir().join(format!("solara-pkg-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        Tmp(d)
    }

    /// A zip of `files` and the index download that describes it.
    fn zip_of(dir: &Path, name: &str, format: Format, bundle: &str, files: &[(&str, &[u8])]) -> (PathBuf, Download) {
        let path = dir.join(name);
        let mut w = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        for (p, bytes) in files {
            w.start_file(*p, opts).unwrap();
            w.write_all(bytes).unwrap();
        }
        w.finish().unwrap();
        let zip_bytes = std::fs::read(&path).unwrap();
        let d = Download {
            platform: "windows-x64".into(),
            format,
            url: name.into(),
            size: zip_bytes.len() as u64,
            sha256: sha256_bytes(&zip_bytes),
            bundle: bundle.into(),
            files: files.iter().map(|(p, b)| FileEntry { path: (*p).into(), size: b.len() as u64, sha256: sha256_bytes(b) }).collect(),
        };
        (path, d)
    }

    fn package(version: &str, downloads: Vec<Download>) -> Package {
        Package {
            id: "song-sense".into(),
            name: "Song Sense".into(),
            version: version.into(),
            summary: String::new(),
            vendor: String::new(),
            homepage: String::new(),
            downloads,
        }
    }

    const BIN: &str = "Song Sense.vst3/Contents/x86_64-win/Song Sense.vst3";
    const INFO: &str = "Song Sense.vst3/Contents/Resources/moduleinfo.json";

    fn install(t: &Path, root: &Path, record: &mut Record, pkg: &Package, zips: &[PathBuf], tag: &str) -> Result<(), String> {
        let stage = t.join(format!("stage-{tag}"));
        let mut unpacked = Vec::new();
        for (z, d) in zips.iter().zip(&pkg.downloads) {
            unpacked.push(unpack(z, d, Os::Windows, &stage)?);
        }
        let change = plan_install(pkg, &unpacked, root, &stage, record.get(&pkg.id))?;
        apply(&change.plan)?;
        record.set(InstalledPackage {
            id: pkg.id.clone(),
            name: pkg.name.clone(),
            version: pkg.version.clone(),
            root: root.to_path_buf(),
            bundles: change.bundles,
        });
        Ok(())
    }

    #[test]
    fn installs_updates_and_removes_one_package() {
        let t = tmp("cycle");
        let root = t.0.join("root");
        let mut record = Record::default();
        let (v1, d1) = zip_of(&t.0, "v1.zip", Format::Vst3, "Song Sense.vst3", &[(BIN, b"binary one"), (INFO, b"{\"v\":1}")]);
        let (c1, dc1) = zip_of(&t.0, "c1.zip", Format::Clap, "Song Sense.clap", &[("Song Sense.clap", b"clap one")]);
        let p1 = package("1.0.0", vec![d1, dc1]);
        install(&t.0, &root, &mut record, &p1, &[v1, c1], "1").unwrap();
        assert_eq!(std::fs::read(root.join("VST3").join(BIN)).unwrap(), b"binary one");
        assert_eq!(std::fs::read(root.join("CLAP/Song Sense.clap")).unwrap(), b"clap one");
        let found = detect(&p1, "windows-x64", Os::Windows, std::slice::from_ref(&root), &record).unwrap();
        assert_eq!((found.version.as_deref(), found.recorded, found.bundles.len()), (Some("1.0.0"), true, 2));

        // 1.0.1 changes the binary and drops moduleinfo.json: it is removed, not left behind.
        let (v2, d2) = zip_of(&t.0, "v2.zip", Format::Vst3, "Song Sense.vst3", &[(BIN, b"binary two")]);
        let (c2, dc2) = zip_of(&t.0, "c2.zip", Format::Clap, "Song Sense.clap", &[("Song Sense.clap", b"clap two")]);
        let p2 = package("1.0.1", vec![d2, dc2]);
        install(&t.0, &root, &mut record, &p2, &[v2, c2], "2").unwrap();
        assert_eq!(std::fs::read(root.join("VST3").join(BIN)).unwrap(), b"binary two");
        assert!(!root.join("VST3").join(INFO).exists());
        assert!(!root.join("VST3/Song Sense.vst3/Contents/Resources").exists(), "the emptied folder is tidied away");
        assert_eq!(record.get("song-sense").unwrap().version, "1.0.1");

        let found = detect(&p2, "windows-x64", Os::Windows, std::slice::from_ref(&root), &record).unwrap();
        let change = plan_remove(&root, &found.bundles, &t.0.join("stage-rm"), "1.0.1").unwrap();
        apply(&change.plan).unwrap();
        assert!(!root.join("VST3/Song Sense.vst3").exists());
        assert!(!root.join("CLAP/Song Sense.clap").exists());
        record.remove("song-sense");
        assert_eq!(detect(&p2, "windows-x64", Os::Windows, std::slice::from_ref(&root), &record), None);
    }

    #[test]
    fn a_bad_file_keeps_the_old_version() {
        let t = tmp("bad");
        let root = t.0.join("root");
        let mut record = Record::default();
        let (v1, d1) = zip_of(&t.0, "v1.zip", Format::Vst3, "Song Sense.vst3", &[(BIN, b"binary one")]);
        install(&t.0, &root, &mut record, &package("1.0.0", vec![d1]), &[v1], "1").unwrap();

        // The index says one thing, the zip holds another: refused before anything changes.
        let (v2, mut d2) = zip_of(&t.0, "v2.zip", Format::Vst3, "Song Sense.vst3", &[(BIN, b"binary two")]);
        d2.files[0].sha256 = sha256_bytes(b"something else");
        let err = install(&t.0, &root, &mut record, &package("1.0.1", vec![d2]), std::slice::from_ref(&v2), "2").unwrap_err();
        assert!(err.contains("doesn't match its published checksum"), "{err}");
        assert_eq!(std::fs::read(root.join("VST3").join(BIN)).unwrap(), b"binary one");
        assert_eq!(record.get("song-sense").unwrap().version, "1.0.0");

        // A file the index doesn't list, and a listed file that's missing.
        let (v3, mut d3) = zip_of(&t.0, "v3.zip", Format::Vst3, "Song Sense.vst3", &[(BIN, b"binary two"), (INFO, b"x")]);
        d3.files.truncate(1);
        assert!(install(&t.0, &root, &mut record, &package("1.0.1", vec![d3]), &[v3], "3").unwrap_err().contains("doesn't list"));
        let (v4, mut d4) = zip_of(&t.0, "v4.zip", Format::Vst3, "Song Sense.vst3", &[(BIN, b"binary two")]);
        d4.files.push(FileEntry { path: INFO.into(), size: 1, sha256: sha256_bytes(b"x") });
        assert!(install(&t.0, &root, &mut record, &package("1.0.1", vec![d4]), &[v4], "4").unwrap_err().contains("missing"));
        // Not a zip at all.
        let junk = t.0.join("junk.zip");
        std::fs::write(&junk, b"not a zip").unwrap();
        let (_, d5) = zip_of(&t.0, "v5.zip", Format::Vst3, "Song Sense.vst3", &[(BIN, b"binary two")]);
        assert!(install(&t.0, &root, &mut record, &package("1.0.1", vec![d5]), &[junk], "5").is_err());
        assert_eq!(std::fs::read(root.join("VST3").join(BIN)).unwrap(), b"binary one");
    }

    #[test]
    fn a_zip_cannot_write_outside_its_bundle() {
        let t = tmp("slip");
        for evil in ["../evil.dll", "Song Sense.vst3/../../evil.dll", "/abs.dll", "Song Sense.vst3\\..\\evil.dll"] {
            let (z, mut d) = zip_of(&t.0, "e.zip", Format::Vst3, "Song Sense.vst3", &[(evil, b"x")]);
            // Even an index that lists the path is refused when it parses.
            d.files[0].path = evil.into();
            assert!(unpack(&z, &d, Os::Windows, &t.0.join("stage")).is_err(), "{evil}");
            let mut i = Index { format: INDEX_FORMAT.into(), format_version: 1, name: String::new(), packages: vec![package("1", vec![d])] };
            i.packages[0].downloads[0].url = "https://example.com/e.zip".into();
            assert!(Index::parse(&serde_json::to_vec(&i).unwrap()).is_err(), "{evil}");
        }
        assert!(!t.0.join("evil.dll").exists());
    }

    #[test]
    fn finds_a_copy_installed_by_something_else() {
        let t = tmp("found");
        let (user, system) = (t.0.join("user"), t.0.join("system"));
        let (_, d) = zip_of(&t.0, "v.zip", Format::Vst3, "Song Sense.vst3", &[(BIN, b"binary one")]);
        let pkg = package("1.0.1", vec![d]);
        let roots = vec![user.clone(), system.clone()];
        assert_eq!(detect(&pkg, "windows-x64", Os::Windows, &roots, &Record::default()), None);
        let bin = system.join("VST3").join(BIN);
        std::fs::create_dir_all(bin.parent().unwrap()).unwrap();
        std::fs::write(&bin, b"binary one").unwrap();
        let f = detect(&pkg, "windows-x64", Os::Windows, &roots, &Record::default()).unwrap();
        assert_eq!((f.root, f.version.as_deref(), f.recorded), (system.clone(), Some("1.0.1"), false));
        std::fs::write(&bin, b"an older build").unwrap();
        assert_eq!(detect(&pkg, "windows-x64", Os::Windows, &roots, &Record::default()).unwrap().version, None);
    }

    #[test]
    fn sweep_deletes_swap_leftovers_only() {
        let t = tmp("sweep");
        let b = t.0.join("VST3/A.vst3/Contents");
        std::fs::create_dir_all(&b).unwrap();
        std::fs::write(b.join("A.vst3"), b"keep").unwrap();
        std::fs::write(b.join("A.vst3.solara-old"), b"old").unwrap();
        sweep(&t.0, &["VST3/A.vst3".into()]);
        assert!(b.join("A.vst3").exists());
        assert!(!b.join("A.vst3.solara-old").exists());
        assert_eq!(bundle_of("VST3/A.vst3/Contents/x"), Some("VST3/A.vst3"));
        assert_eq!(bundle_of("CLAP/A.clap"), Some("CLAP/A.clap"));
    }
}
