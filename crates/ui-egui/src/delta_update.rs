//! Delta updates in the app: download only what changed, check it, stage the new files, hand over
//! to the standalone updater and quit.
//!
//! Update checks stay off ([`crate::updates_ui::UPDATE_CHECKS_ENABLED`]), so nothing here runs by
//! itself: an update starts only when asked through the control channel
//! (`app.delta_update {"manifest": <url or path of delta.json>, "sha256": <its SHA-256>}`). The app
//! never writes into its own install folder; the updater does, after the app has exited.
//! See `docs/delta-updates.md`.

use crate::SoundApp;

#[cfg(not(target_arch = "wasm32"))]
pub(crate) use native::*;

#[cfg(target_arch = "wasm32")]
#[derive(Default)]
pub(crate) struct Runtime;

#[cfg(target_arch = "wasm32")]
pub(crate) fn start(_app: &mut SoundApp, _p: &serde_json::Value) -> Result<serde_json::Value, String> {
    Err("updates are not supported in the web browser".into())
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn status(_app: &SoundApp) -> serde_json::Value {
    serde_json::json!({"phase": "unsupported"})
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn tick(_app: &mut SoundApp, _ctx: &egui::Context) {}

#[cfg(target_arch = "wasm32")]
pub(crate) fn show(_app: &mut SoundApp, _ctx: &egui::Context) {}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::mpsc::{Receiver, TryRecvError};

    use serde_json::{Value, json};
    use soundcraft_delta::{MANIFEST_NAME, Manifest, PLAN_NAME, Plan, RESULT_NAME, Relaunch, UpdateResult, check_install, current_platform, stage};

    use super::SoundApp;

    const APP: &str = "solara";
    const VERSION: &str = env!("CARGO_PKG_VERSION");
    /// The updater's name next to the app: shipped as `solara-updater.exe`; a cargo build names it
    /// `soundcraft-updater`.
    const UPDATER_NAMES: &[&str] = &["solara-updater.exe", "soundcraft-updater.exe", "solara-updater", "soundcraft-updater"];

    #[derive(Default)]
    pub(crate) struct Runtime {
        job: Option<Job>,
        phase: Phase,
        /// How the last update went, read once at start from the updater's report.
        notice: Option<UpdateResult>,
        notice_read: bool,
    }

    struct Job {
        rx: Receiver<Result<Prepared, String>>,
        bytes: Arc<AtomicU64>,
        total: Arc<AtomicU64>,
    }

    #[derive(Clone, Debug, Default, PartialEq)]
    enum Phase {
        #[default]
        Idle,
        Downloading,
        /// The updater is running and the app is quitting.
        Handing {
            to: String,
            bytes: u64,
        },
        Failed(String),
    }

    struct Prepared {
        updater: PathBuf,
        plan_path: PathBuf,
        plan: Plan,
    }

    /// What one update needs to know about this copy.
    struct Setup {
        source: String,
        sha256: String,
        install_dir: PathBuf,
        exe_name: String,
        updater: PathBuf,
        update_dir: PathBuf,
        relaunch_args: Vec<String>,
    }

    /// `app.delta_update`: start downloading and staging the delta that `p.manifest` names.
    pub(crate) fn start(app: &mut SoundApp, p: &Value) -> Result<Value, String> {
        let source = p.get("manifest").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).ok_or("`manifest` is required")?;
        let sha256 = p.get("sha256").and_then(Value::as_str).ok_or("`sha256` (the manifest's published SHA-256) is required")?;
        if app.delta_rt.job.is_some() || matches!(app.delta_rt.phase, Phase::Handing { .. }) {
            return Err("an update is already under way".into());
        }
        if app.has_unsaved_changes() || app.engine.transport.recording {
            return Err("save the session (and stop recording) before updating".into());
        }
        let update_dir = app.update_dir.clone().ok_or("this copy has no folder for updates")?;
        let exe = std::env::current_exe().map_err(|e| format!("couldn't find this program ({e})"))?;
        let install_dir = exe.parent().map(Path::to_path_buf).ok_or("couldn't find this program's folder")?;
        let exe_name = exe.file_name().and_then(|n| n.to_str()).map(str::to_string).ok_or("this program's name isn't UTF-8")?;
        let updater = UPDATER_NAMES
            .iter()
            .map(|n| install_dir.join(n))
            .find(|p| p.is_file())
            .ok_or_else(|| format!("the updater (solara-updater) isn't in {}", install_dir.display()))?;
        let relaunch_args = app.engine.path.clone().into_iter().collect();
        let setup = Setup { source: source.to_string(), sha256: sha256.to_string(), install_dir, exe_name, updater, update_dir, relaunch_args };
        let (tx, rx) = std::sync::mpsc::channel();
        let bytes = Arc::new(AtomicU64::new(0));
        let total = Arc::new(AtomicU64::new(0));
        let (b, t) = (bytes.clone(), total.clone());
        std::thread::Builder::new()
            .name("delta-update".into())
            .spawn(move || {
                let _ = tx.send(prepare(&setup, &b, &t));
            })
            .map_err(|e| format!("couldn't start the download ({e})"))?;
        app.delta_rt.job = Some(Job { rx, bytes, total });
        app.delta_rt.phase = Phase::Downloading;
        log::info!("delta update: downloading {source}");
        Ok(json!({"started": true}))
    }

    /// `app.delta_update.status`.
    pub(crate) fn status(app: &SoundApp) -> Value {
        let rt = &app.delta_rt;
        let (done, total) = rt.job.as_ref().map_or((0, 0), |j| (j.bytes.load(Ordering::Relaxed), j.total.load(Ordering::Relaxed)));
        let mut v = match &rt.phase {
            Phase::Idle => json!({"phase": "idle"}),
            Phase::Downloading => json!({"phase": "downloading", "downloaded_bytes": done, "total_bytes": total}),
            Phase::Handing { to, bytes } => json!({"phase": "installing", "to_version": to, "downloaded_bytes": bytes}),
            Phase::Failed(e) => json!({"phase": "failed", "error": e}),
        };
        v["version"] = json!(VERSION);
        if let Some(n) = &rt.notice {
            v["last_result"] = json!({"ok": n.ok, "from_version": n.from_version, "to_version": n.to_version, "message": n.message, "downloaded_bytes": n.downloaded_bytes});
        }
        v
    }

    /// Fetch the manifest and payloads, check them and this copy, build the new files, copy the
    /// updater beside them and write its plan. Runs off the UI thread.
    fn prepare(s: &Setup, bytes: &AtomicU64, total: &AtomicU64) -> Result<Prepared, String> {
        let http = craft_update::Http::new(&format!("Solara/{VERSION}"))?;
        let remote = s.source.starts_with("https://") || s.source.starts_with("http://");
        let manifest_bytes = if remote {
            http.get_bytes(&s.source, 1 << 20)?.ok_or_else(|| format!("{} wasn't found", s.source))?
        } else {
            std::fs::read(&s.source).map_err(|e| format!("couldn't read {} ({e})", s.source))?
        };
        let m = Manifest::parse_verified(&manifest_bytes, &s.sha256)?;
        check_install(&m, &s.install_dir, APP, &current_platform(), VERSION)?;
        let manifest_len = manifest_bytes.len() as u64;
        total.store(manifest_len.saturating_add(m.payload_bytes()), Ordering::Relaxed);
        bytes.store(manifest_len, Ordering::Relaxed);

        let stage_dir =
            s.update_dir.join(format!("{}-{}", m.to_version.replace(|c: char| !c.is_ascii_alphanumeric() && c != '.', "_"), std::process::id()));
        let _ = std::fs::remove_dir_all(&stage_dir);
        let download = stage_dir.join("download");
        std::fs::create_dir_all(&download).map_err(|e| format!("couldn't create {} ({e})", download.display()))?;
        let base = s
            .source
            .strip_suffix(MANIFEST_NAME)
            .map(str::to_string)
            .unwrap_or_else(|| s.source.rsplit_once(['/', '\\']).map(|(dir, _)| format!("{dir}/")).unwrap_or_default());
        let stop = craft_update::Stop::default();
        let mut downloaded = manifest_len;
        for p in m.files.iter().filter_map(|f| f.payload.as_ref()) {
            let dest = download.join(&p.name);
            let from = format!("{base}{}", p.name);
            if remote {
                let before = downloaded;
                http.download(&from, &dest, p.size, &p.sha256, &stop, &mut |t| {
                    if let craft_update::http::Transfer::Bytes { done, .. } = t {
                        bytes.store(before.saturating_add(done), Ordering::Relaxed);
                    }
                })?;
            } else {
                std::fs::copy(&from, &dest).map_err(|e| format!("couldn't read {from} ({e})"))?;
            }
            downloaded = downloaded.saturating_add(p.size);
            bytes.store(downloaded, Ordering::Relaxed);
        }
        let steps = stage(&m, &s.install_dir, &download, &stage_dir)?;
        let _ = std::fs::remove_dir_all(&download);
        let updater = stage_dir.join(s.updater.file_name().unwrap_or_default());
        std::fs::copy(&s.updater, &updater).map_err(|e| format!("couldn't copy the updater ({e})"))?;
        let plan = Plan {
            app: APP.into(),
            from_version: m.from_version.clone(),
            to_version: m.to_version.clone(),
            install_dir: s.install_dir.clone(),
            stage_dir: stage_dir.clone(),
            steps,
            relaunch: Some(Relaunch { exe: s.exe_name.clone(), args: s.relaunch_args.clone() }),
            result_file: s.update_dir.join(RESULT_NAME),
            downloaded_bytes: downloaded,
        };
        let plan_path = stage_dir.join(PLAN_NAME);
        plan.write(&plan_path)?;
        log::info!("delta update: {} -> {} staged, {downloaded} bytes downloaded", plan.from_version, plan.to_version);
        Ok(Prepared { updater, plan_path, plan })
    }

    /// Poll the background job; once the update is staged, start the updater and quit.
    pub(crate) fn tick(app: &mut SoundApp, _ctx: &egui::Context) {
        if !app.delta_rt.notice_read {
            app.delta_rt.notice_read = true;
            if let Some(path) = app.update_dir.as_ref().map(|d| d.join(RESULT_NAME))
                && path.is_file()
            {
                match UpdateResult::read(&path) {
                    Ok(r) => {
                        log::info!("last update: {}", r.message);
                        app.delta_rt.notice = Some(r);
                    }
                    Err(e) => log::warn!("{e}"),
                }
                let _ = std::fs::remove_file(&path);
                // The staging folders of finished updates: the updater can't delete its own copy.
                for dir in std::fs::read_dir(path.parent().unwrap_or(Path::new(""))).into_iter().flatten().flatten() {
                    if dir.file_type().is_ok_and(|t| t.is_dir()) {
                        let _ = std::fs::remove_dir_all(dir.path());
                    }
                }
            }
        }
        let Some(job) = &app.delta_rt.job else { return };
        let done = match job.rx.try_recv() {
            Ok(r) => r,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err("the download stopped unexpectedly".into()),
        };
        app.delta_rt.job = None;
        let launched = done.and_then(|p| {
            // Checked again here: the session may have changed while the update downloaded.
            if app.has_unsaved_changes() || app.engine.transport.recording {
                return Err("the session changed while the update downloaded; save it and try again".into());
            }
            std::process::Command::new(&p.updater)
                .arg("--plan")
                .arg(&p.plan_path)
                .arg("--wait-pid")
                .arg(std::process::id().to_string())
                .current_dir(&p.plan.stage_dir)
                .spawn()
                .map_err(|e| format!("couldn't start the updater ({e})"))?;
            Ok(p.plan)
        });
        match launched {
            Ok(plan) => {
                log::info!("delta update: updater started, quitting for {}", plan.to_version);
                app.delta_rt.phase = Phase::Handing { to: plan.to_version, bytes: plan.downloaded_bytes };
                app.quit_requested = true;
            }
            Err(e) => {
                log::warn!("delta update refused: {e}");
                app.delta_rt.phase = Phase::Failed(e);
            }
        }
    }

    /// The update's progress or refusal, and at start how the last update went.
    pub(crate) fn show(app: &mut SoundApp, ctx: &egui::Context) {
        let rt = &mut app.delta_rt;
        let text = match (&rt.phase, &rt.notice) {
            (Phase::Downloading, _) => {
                let (d, t) = rt.job.as_ref().map_or((0, 0), |j| (j.bytes.load(Ordering::Relaxed), j.total.load(Ordering::Relaxed)));
                ctx.request_repaint_after(std::time::Duration::from_millis(200));
                format!("Downloading the update: {} of {}", kb(d), kb(t))
            }
            (Phase::Handing { to, .. }, _) => format!("Installing Solara {to}. Solara restarts when it's done."),
            (Phase::Failed(e), _) => format!("The update was refused: {e}. Solara {VERSION} keeps working."),
            (Phase::Idle, Some(n)) => n.message.clone(),
            (Phase::Idle, None) => return,
        };
        let mut open = true;
        egui::Window::new("Solara Update")
            .open(&mut open)
            .resizable(false)
            .collapsible(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .show(ctx, |ui| {
                ui.set_max_width(420.0);
                ui.label(text);
            });
        if !open {
            if matches!(rt.phase, Phase::Failed(_)) {
                rt.phase = Phase::Idle;
            }
            rt.notice = None;
        }
    }

    fn kb(bytes: u64) -> String {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    }
}
