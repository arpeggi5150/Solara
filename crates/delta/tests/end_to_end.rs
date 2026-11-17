//! A delta made from two install folders takes the old one to the new one, downloads far less
//! than the new folder, and refuses a corrupt payload while leaving the old folder working.

use std::path::{Path, PathBuf};

use soundcraft_delta::{MANIFEST_NAME, MakeOptions, Manifest, Plan, check_install, make, sha256_file, stage, swap};

struct Dir(PathBuf);
impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A pseudo-random "exe" of `len` bytes; `edit` changes a few bytes in the middle.
fn exe(len: usize, edit: bool) -> Vec<u8> {
    let mut x = 0x2545_f491_4f6c_dd1du64;
    let mut v: Vec<u8> = (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x >> 24) as u8
        })
        .collect();
    if edit {
        v[len / 2..len / 2 + 64].fill(7);
        v.extend_from_slice(b"0.1.1");
    }
    v
}

fn setup(name: &str) -> (Dir, PathBuf, PathBuf, PathBuf) {
    let root = std::env::temp_dir().join(format!("solara-delta-e2e-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let (old, new, out) = (root.join("old"), root.join("new"), root.join("delta"));
    for d in [&old, &new] {
        std::fs::create_dir_all(d).unwrap();
        std::fs::write(d.join("NOTICE"), b"same in both").unwrap();
    }
    std::fs::write(old.join("solara.exe"), exe(3_000_000, false)).unwrap();
    std::fs::write(new.join("solara.exe"), exe(3_000_000, true)).unwrap();
    std::fs::write(old.join("gone.txt"), b"removed in 0.1.1").unwrap();
    std::fs::create_dir_all(new.join("docs")).unwrap();
    std::fs::write(new.join("docs/new.txt"), b"added in 0.1.1").unwrap();
    (Dir(root), old, new, out)
}

fn opts() -> MakeOptions {
    MakeOptions { app: "solara".into(), platform: "windows-x64".into(), from_version: "0.1.0".into(), to_version: "0.1.1".into() }
}

fn plan_for(m: &Manifest, install: &Path, stage_dir: &Path, steps: Vec<soundcraft_delta::Step>) -> Plan {
    Plan {
        app: m.app.clone(),
        from_version: m.from_version.clone(),
        to_version: m.to_version.clone(),
        install_dir: install.to_path_buf(),
        stage_dir: stage_dir.to_path_buf(),
        steps,
        relaunch: None,
        result_file: stage_dir.join("result.json"),
        downloaded_bytes: 0,
    }
}

#[test]
fn a_delta_takes_the_old_folder_to_the_new_one_and_is_much_smaller() {
    let (root, old, new, out) = setup("ok");
    let report = make(&old, &new, &out, &opts()).unwrap();
    assert_eq!((report.changed, report.unchanged), (3, 1));
    assert!(report.payload_bytes * 50 < report.new_folder_bytes, "{report:?}");

    let bytes = std::fs::read(out.join(MANIFEST_NAME)).unwrap();
    let m = Manifest::parse_verified(&bytes, &report.manifest_sha256).unwrap();
    check_install(&m, &old, "solara", "windows-x64", "0.1.0").unwrap();
    assert!(check_install(&m, &old, "solara", "windows-x64", "0.1.1").is_err());
    assert!(check_install(&m, &new, "solara", "windows-x64", "0.1.0").is_err(), "the new folder is not the starting point");

    let stage_dir = root.0.join("stage");
    let steps = stage(&m, &old, &out, &stage_dir).unwrap();
    swap::apply(&plan_for(&m, &old, &stage_dir, steps)).unwrap();
    for f in ["solara.exe", "NOTICE", "docs/new.txt"] {
        assert_eq!(sha256_file(&old.join(f)).unwrap(), sha256_file(&new.join(f)).unwrap(), "{f}");
    }
    assert!(!old.join("gone.txt").exists());
}

#[test]
fn a_corrupt_payload_is_refused_and_the_old_folder_is_untouched() {
    let (root, old, new, out) = setup("corrupt");
    make(&old, &new, &out, &opts()).unwrap();
    let report_bytes = std::fs::read(out.join(MANIFEST_NAME)).unwrap();
    let m: Manifest = serde_json::from_slice(&report_bytes).unwrap();
    let exe_payload = m.files.iter().find(|f| f.path == "solara.exe").and_then(|f| f.payload.clone()).unwrap();
    let p = out.join(&exe_payload.name);
    let mut data = std::fs::read(&p).unwrap();
    data[10] ^= 0xff;
    std::fs::write(&p, data).unwrap();
    let before = sha256_file(&old.join("solara.exe")).unwrap();
    let e = stage(&m, &old, &out, &root.0.join("stage")).unwrap_err();
    assert!(e.contains("doesn't match its checksum"), "{e}");
    assert_eq!(sha256_file(&old.join("solara.exe")).unwrap(), before);
    assert!(old.join("gone.txt").exists());
}
