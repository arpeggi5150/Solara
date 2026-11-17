//! The updater's side: swap the staged files into the install folder, all or none.
//!
//! 1. Check: every staged file against its new SHA-256, every installed file against its old one
//!    (a file already at its new hash is left alone, so a repeated run finishes the job).
//! 2. Copy each staged file next to its target as `<name>.solara-new`, and check the copy.
//! 3. Commit: move each old file aside to `<name>.solara-old`, then the new one into place.
//!    If any move fails, every file already moved goes back, and the old version is as it was.
//! 4. Delete the `.solara-old` files (one that can't be deleted yet is left for the next run).

use std::path::{Path, PathBuf};

use crate::hash::sha256_file;
use crate::manifest::safe_relative;
use crate::plan::Plan;

const NEW: &str = "solara-new";
const OLD: &str = "solara-old";

/// Swap `plan`'s files in. On `Err`, the install folder holds the old version.
pub fn apply(plan: &Plan) -> Result<(), String> {
    apply_failing_at(plan, None)
}

struct Work {
    path: String,
    target: PathBuf,
    /// The staged file and its checked copy beside the target, when the step writes a file.
    new: Option<(PathBuf, PathBuf, String)>,
    /// The step's file exists now and has to move aside.
    has_old: bool,
}

/// [`apply`], failing on purpose at commit step `fail_at` (tests).
fn apply_failing_at(plan: &Plan, fail_at: Option<usize>) -> Result<(), String> {
    plan.validate()?;
    let mut work = Vec::new();
    for s in &plan.steps {
        let target = plan.install_dir.join(safe_relative(&s.path)?);
        if let (Some(staged), Some(to)) = (&s.staged, &s.to_sha256)
            && &sha256_file(staged)? != to
        {
            return Err(format!("the staged {} doesn't match its checksum, so the update was refused", s.path));
        }
        let current = if target.exists() { Some(sha256_file(&target)?) } else { None };
        if current == s.to_sha256 {
            continue;
        }
        if current != s.from_sha256 {
            return Err(format!("{} isn't the file the update starts from, so the update was refused", s.path));
        }
        let new = match (&s.staged, &s.to_sha256) {
            (Some(staged), Some(to)) => Some((staged.clone(), with_suffix(&target, NEW), to.clone())),
            _ => None,
        };
        work.push(Work { path: s.path.clone(), new, has_old: current.is_some(), target });
    }
    let cleanup_new = |work: &[Work]| {
        for (_, copy, _) in work.iter().filter_map(|w| w.new.as_ref()) {
            let _ = std::fs::remove_file(copy);
        }
    };
    for w in &work {
        let Some((staged, copy, to)) = &w.new else { continue };
        let copied = copy
            .parent()
            .map_or(Ok(()), |dir| std::fs::create_dir_all(dir).map_err(|e| format!("couldn't create {} ({e})", dir.display())))
            .and_then(|()| std::fs::copy(staged, copy).map_err(|e| format!("couldn't write {} ({e})", copy.display())))
            .and_then(|_| keep_permissions(&w.target, copy))
            .and_then(|()| sha256_file(copy));
        match copied {
            Ok(h) if &h == to => {}
            Ok(_) => {
                cleanup_new(&work);
                return Err(format!("the copy of {} doesn't match its checksum, so the update was refused", w.path));
            }
            Err(e) => {
                cleanup_new(&work);
                return Err(format!("{e}; the update was refused"));
            }
        }
    }
    let mut done = 0usize;
    let mut result = Ok(());
    // Where each old file went: `<name>.solara-old`, or a numbered name when an earlier swap's
    // `.solara-old` is still held open (a plugin a running DAW has loaded can be renamed, not deleted).
    let mut aside: Vec<Option<PathBuf>> = vec![None; work.len()];
    for (i, w) in work.iter().enumerate() {
        let step = (|| {
            if fail_at == Some(i) {
                return Err("simulated failure".to_string());
            }
            let old = if w.has_old {
                let old = aside_path(&w.target);
                std::fs::rename(&w.target, &old).map_err(|e| format!("couldn't move {} aside ({e})", w.target.display()))?;
                Some(old)
            } else {
                None
            };
            if let Some((_, copy, _)) = &w.new
                && let Err(e) = std::fs::rename(copy, &w.target)
            {
                if let Some(old) = &old {
                    let _ = std::fs::rename(old, &w.target);
                }
                return Err(format!("couldn't put the new {} in place ({e})", w.target.display()));
            }
            if let Some(slot) = aside.get_mut(i) {
                *slot = old;
            }
            Ok(())
        })();
        if let Err(e) = step {
            result = Err(e);
            break;
        }
        done = i + 1;
    }
    if let Err(e) = result {
        for (w, old) in work.iter().zip(&aside).take(done).rev() {
            if w.new.is_some() {
                let _ = std::fs::remove_file(&w.target);
            }
            if let Some(old) = old {
                let _ = std::fs::rename(old, &w.target);
            }
        }
        cleanup_new(&work);
        return Err(format!("{e}; the old version was put back"));
    }
    for old in aside.iter().flatten() {
        let _ = std::fs::remove_file(old);
    }
    Ok(())
}

/// A free name to move `target` aside to: `<name>.solara-old`, deleting a leftover one first; if a
/// leftover can't be deleted (still loaded somewhere), `<name>.solara-old.1`, `.2`, …
fn aside_path(target: &Path) -> PathBuf {
    let plain = with_suffix(target, OLD);
    for n in 0..100u32 {
        let p = if n == 0 { plain.clone() } else { with_suffix(target, &format!("{OLD}.{n}")) };
        if std::fs::symlink_metadata(&p).is_err() || std::fs::remove_file(&p).is_ok() {
            return p;
        }
    }
    plain
}

/// Give the new file the permissions of the one it replaces (the executable bit on Unix).
fn keep_permissions(target: &Path, copy: &Path) -> Result<(), String> {
    match std::fs::metadata(target) {
        Ok(m) => std::fs::set_permissions(copy, m.permissions()).map_err(|e| format!("couldn't set the permissions of {} ({e})", copy.display())),
        Err(_) => Ok(()),
    }
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut s = path.as_os_str().to_os_string();
    s.push(".");
    s.push(suffix);
    PathBuf::from(s)
}

/// Whether this process can create files in `dir` (else the swap needs an administrator).
pub fn writable(dir: &Path) -> bool {
    let probe = dir.join(format!(".solara-write-test-{}", std::process::id()));
    let ok = std::fs::write(&probe, b"").is_ok();
    let _ = std::fs::remove_file(&probe);
    ok
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hash::sha256_bytes;
    use crate::plan::Step;

    struct Fixture {
        root: PathBuf,
        plan: Plan,
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    /// An install with `a` (changes), `b` (removed), `keep` (untouched), and a new `sub/c`.
    fn fixture(name: &str) -> Fixture {
        let root = std::env::temp_dir().join(format!("solara-swap-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let install = root.join("install");
        let stage = root.join("stage");
        std::fs::create_dir_all(&install).unwrap();
        std::fs::create_dir_all(&stage).unwrap();
        std::fs::write(install.join("a"), b"old a").unwrap();
        std::fs::write(install.join("b"), b"old b").unwrap();
        std::fs::write(install.join("keep"), b"keep").unwrap();
        std::fs::write(stage.join("0"), b"new a").unwrap();
        std::fs::write(stage.join("2"), b"new c").unwrap();
        let plan = Plan {
            app: "solara".into(),
            from_version: "0.1.0".into(),
            to_version: "0.1.1".into(),
            install_dir: install,
            stage_dir: stage.clone(),
            steps: vec![
                Step {
                    path: "a".into(),
                    staged: Some(stage.join("0")),
                    from_sha256: Some(sha256_bytes(b"old a")),
                    to_sha256: Some(sha256_bytes(b"new a")),
                },
                Step { path: "b".into(), staged: None, from_sha256: Some(sha256_bytes(b"old b")), to_sha256: None },
                Step { path: "sub/c".into(), staged: Some(stage.join("2")), from_sha256: None, to_sha256: Some(sha256_bytes(b"new c")) },
            ],
            relaunch: None,
            result_file: root.join("result.json"),
            downloaded_bytes: 0,
        };
        Fixture { root, plan }
    }

    fn snapshot(dir: &Path) -> Vec<(String, Vec<u8>)> {
        let mut out = Vec::new();
        let mut stack = vec![dir.to_path_buf()];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).unwrap().flatten() {
                if e.file_type().unwrap().is_dir() {
                    stack.push(e.path());
                } else {
                    out.push((e.path().strip_prefix(dir).unwrap().to_string_lossy().replace('\\', "/"), std::fs::read(e.path()).unwrap()));
                }
            }
        }
        out.sort();
        out
    }

    #[test]
    fn the_staged_files_go_in_and_nothing_is_left_behind() {
        let f = fixture("ok");
        apply(&f.plan).unwrap();
        assert_eq!(
            snapshot(&f.plan.install_dir),
            vec![("a".into(), b"new a".to_vec()), ("keep".into(), b"keep".to_vec()), ("sub/c".into(), b"new c".to_vec())]
        );
        // A second run finds the work done and changes nothing.
        apply(&f.plan).unwrap();
        assert_eq!(snapshot(&f.plan.install_dir).len(), 3);
    }

    #[cfg(unix)]
    #[test]
    fn a_replaced_file_keeps_its_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let f = fixture("perm");
        std::fs::set_permissions(f.plan.install_dir.join("a"), std::fs::Permissions::from_mode(0o755)).unwrap();
        apply(&f.plan).unwrap();
        assert_eq!(std::fs::metadata(f.plan.install_dir.join("a")).unwrap().permissions().mode() & 0o777, 0o755);
    }

    #[test]
    fn a_failure_part_way_puts_every_file_back() {
        for at in 0..3 {
            let f = fixture(&format!("fail{at}"));
            let before = snapshot(&f.plan.install_dir);
            let e = apply_failing_at(&f.plan, Some(at)).unwrap_err();
            assert!(e.contains("old version was put back"), "{e}");
            assert_eq!(snapshot(&f.plan.install_dir), before, "failing at step {at}");
        }
    }

    #[test]
    fn a_corrupt_staged_file_refuses_the_update_before_anything_moves() {
        let f = fixture("corrupt");
        std::fs::write(f.plan.stage_dir.join("0"), b"new A").unwrap();
        let before = snapshot(&f.plan.install_dir);
        assert!(apply(&f.plan).unwrap_err().contains("doesn't match its checksum"));
        assert_eq!(snapshot(&f.plan.install_dir), before);
    }

    #[test]
    fn an_install_that_is_not_the_starting_version_is_refused() {
        let f = fixture("other");
        std::fs::write(f.plan.install_dir.join("a"), b"someone else's a").unwrap();
        let before = snapshot(&f.plan.install_dir);
        assert!(apply(&f.plan).unwrap_err().contains("isn't the file the update starts from"));
        assert_eq!(snapshot(&f.plan.install_dir), before);
    }

    #[test]
    fn a_plan_staging_outside_its_folder_is_refused() {
        let mut f = fixture("outside");
        f.plan.steps[0].staged = Some(f.root.join("elsewhere"));
        assert!(apply(&f.plan).unwrap_err().contains("outside"));
    }

    #[test]
    fn a_leftover_old_copy_gets_a_numbered_name_instead_of_blocking() {
        let f = fixture("leftover");
        let a = f.plan.install_dir.join("a");
        // A leftover from an earlier swap that can't be deleted: a non-empty folder stands in for
        // a file another program still has open.
        let stuck = with_suffix(&a, OLD);
        std::fs::create_dir_all(stuck.join("held")).unwrap();
        assert_eq!(aside_path(&a), with_suffix(&a, &format!("{OLD}.1")));
        apply(&f.plan).unwrap();
        assert_eq!(std::fs::read(&a).unwrap(), b"new a");
        assert!(stuck.is_dir(), "the stuck leftover is left for later");
        assert!(!with_suffix(&a, &format!("{OLD}.1")).exists(), "this swap's own old copy is deleted");
        // A leftover that can be deleted is reused.
        let _ = std::fs::remove_dir_all(&stuck);
        std::fs::write(&stuck, b"stale").unwrap();
        assert_eq!(aside_path(&a), stuck);
        assert!(!stuck.exists());
    }
}
