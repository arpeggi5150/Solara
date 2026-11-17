//! The SolPlugs package browser: Solara's plugin store (Window > SolPlugs Packages…).
//!
//! It reads the package index (`soundcraft_packages::DEFAULT_INDEX_URL`, or the `SOLARA_PACKAGE_INDEX`
//! environment variable, or the `index` given to `app.packages.refresh`), lists every package with
//! its version and a short line, and installs, updates and removes them. It checks the index each
//! time the window opens, and on Refresh.
//!
//! One package is one download: an update fetches only that plugin's zips. Every zip is checked
//! against the index's SHA-256 as it downloads, and every file in it against its own SHA-256 when
//! it is unpacked; then the files are swapped in all or none. If any check fails, the installed
//! version stays and the window says why. A copy installed for everyone (in Common Files) is
//! changed through the updater, as an administrator. Control channel: `app.packages`,
//! `app.packages.refresh`, `app.packages.install`, `app.packages.update`, `app.packages.remove`.

#[cfg(target_arch = "wasm32")]
use crate::SoundApp;

#[cfg(not(target_arch = "wasm32"))]
pub(crate) use native::*;

#[cfg(target_arch = "wasm32")]
#[derive(Default)]
pub(crate) struct Runtime;

#[cfg(target_arch = "wasm32")]
pub(crate) fn command(_app: &mut SoundApp, _method: &str, _p: &serde_json::Value) -> Result<serde_json::Value, String> {
    Err("SolPlugs packages are not supported in the web browser".into())
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn opened(_app: &mut SoundApp) {}

#[cfg(target_arch = "wasm32")]
pub(crate) fn tick(_app: &mut SoundApp, _ctx: &egui::Context) {}

#[cfg(target_arch = "wasm32")]
pub(crate) fn show(app: &mut SoundApp, ctx: &egui::Context) {
    let mut open = app.ui.show_packages;
    egui::Window::new("SolPlugs Packages").open(&mut open).show(ctx, |ui| {
        ui.label("SolPlugs packages install plugins on a computer; the web version can't.");
    });
    app.ui.show_packages = open;
}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::mpsc::{Receiver, TryRecvError};

    use egui::{RichText, vec2};
    use serde_json::{Value, json};
    use soundcraft_packages::install::{self, Found};
    use soundcraft_packages::roots::{self, Env, Os};
    use soundcraft_packages::{DEFAULT_INDEX_URL, Index, InstalledPackage, Package, RECORD_NAME, Record, current_platform, is_newer};

    use crate::SoundApp;

    const VERSION: &str = env!("CARGO_PKG_VERSION");

    #[derive(Default)]
    pub(crate) struct Runtime {
        /// Where the index comes from; empty: the default.
        source: String,
        index: Option<Index>,
        /// The source the shown index was read from (relative downloads resolve against it).
        index_source: String,
        error: Option<String>,
        fetch: Option<Receiver<Result<(Index, String), String>>>,
        job: Option<Job>,
        /// The last outcome per package id, shown under its row.
        notes: HashMap<String, Note>,
        record: Option<Record>,
        found: HashMap<String, Option<Found>>,
        /// Tests: install here instead of this account's plugin folder.
        pub(crate) root_override: Option<PathBuf>,
        swept: bool,
        checked: Option<std::time::Instant>,
    }

    #[derive(Clone, Debug)]
    struct Note {
        ok: bool,
        text: String,
        /// What the change downloaded.
        downloaded: u64,
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Action {
        Install,
        Update,
        Remove,
    }

    impl Action {
        fn verb(self) -> &'static str {
            match self {
                Action::Install => "install",
                Action::Update => "update",
                Action::Remove => "remove",
            }
        }
    }

    struct Job {
        id: String,
        action: Action,
        rx: Receiver<Result<Done, String>>,
        bytes: Arc<AtomicU64>,
        total: Arc<AtomicU64>,
    }

    /// A finished job: the package's new record entry (`None` once removed) and what it downloaded.
    struct Done {
        entry: Option<InstalledPackage>,
        downloaded: u64,
    }

    /// Everything a job needs, owned, so it can run off the UI thread.
    struct Work {
        action: Action,
        pkg: Package,
        index_source: String,
        root: PathBuf,
        found: Option<Found>,
        previous: Option<InstalledPackage>,
        dir: PathBuf,
        updater: Option<PathBuf>,
    }

    fn source(rt: &Runtime) -> String {
        if !rt.source.is_empty() {
            return rt.source.clone();
        }
        std::env::var("SOLARA_PACKAGE_INDEX").ok().filter(|s| !s.trim().is_empty()).unwrap_or_else(|| DEFAULT_INDEX_URL.to_string())
    }

    fn package_dir(app: &SoundApp) -> Option<PathBuf> {
        app.package_dir.clone()
    }

    fn roots_env(rt: &Runtime) -> (Vec<PathBuf>, Option<PathBuf>) {
        if let Some(r) = &rt.root_override {
            return (vec![r.clone()], Some(r.clone()));
        }
        let env = Env::current();
        (roots::all_roots(Os::current(), &env), roots::user_root(Os::current(), &env))
    }

    /// The window was opened: check the index again.
    pub(crate) fn opened(app: &mut SoundApp) {
        refresh(app);
    }

    fn refresh(app: &mut SoundApp) {
        if app.packages_rt.fetch.is_some() {
            return;
        }
        let src = source(&app.packages_rt);
        let (tx, rx) = std::sync::mpsc::channel();
        let spawned = std::thread::Builder::new().name("package-index".into()).spawn(move || {
            let _ = tx.send(fetch_index(&src).map(|i| (i, src)));
        });
        match spawned {
            Ok(_) => {
                app.packages_rt.fetch = Some(rx);
                app.packages_rt.error = None;
            }
            Err(e) => app.packages_rt.error = Some(format!("couldn't check the package index ({e})")),
        }
    }

    fn fetch_index(src: &str) -> Result<Index, String> {
        let bytes = if is_remote(src) {
            let http = craft_update::Http::new(&format!("Solara/{VERSION}"))?;
            http.get_bytes(src, 4 << 20)?.ok_or_else(|| "the package index wasn't found".to_string())?
        } else {
            std::fs::read(src).map_err(|e| format!("couldn't read {src} ({e})"))?
        };
        Index::parse(&bytes)
    }

    fn is_remote(s: &str) -> bool {
        s.starts_with("https://") || s.starts_with("http://")
    }

    /// Re-read the record and look for every package on this computer.
    fn rescan_found(app: &mut SoundApp) {
        let record = match package_dir(app).map(|d| Record::read(&d.join(RECORD_NAME))) {
            Some(Ok(r)) => r,
            Some(Err(e)) => {
                log::warn!("packages: {e}");
                app.packages_rt.error = Some(e);
                Record::default()
            }
            None => Record::default(),
        };
        let (roots, _) = roots_env(&app.packages_rt);
        let platform = current_platform();
        let mut found = HashMap::new();
        if let Some(index) = &app.packages_rt.index {
            for p in &index.packages {
                let f = install::detect(p, &platform, Os::current(), &roots, &record);
                if !app.packages_rt.swept
                    && let Some(f) = &f
                {
                    install::sweep(&f.root, &f.bundles);
                }
                found.insert(p.id.clone(), f);
            }
            app.packages_rt.swept = true;
        }
        app.packages_rt.found = found;
        app.packages_rt.record = Some(record);
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum State {
        /// No download for this computer.
        Unavailable,
        NotInstalled,
        Installed,
        UpdateAvailable,
        /// Found, but not a version the index knows (put there by something else).
        OtherVersion,
    }

    impl State {
        fn id(self) -> &'static str {
            match self {
                State::Unavailable => "unavailable",
                State::NotInstalled => "not_installed",
                State::Installed => "installed",
                State::UpdateAvailable => "update_available",
                State::OtherVersion => "other_version",
            }
        }
    }

    fn state(rt: &Runtime, p: &Package) -> State {
        match rt.found.get(&p.id).and_then(Option::as_ref) {
            Some(f) => match &f.version {
                Some(v) if is_newer(&p.version, v) => State::UpdateAvailable,
                Some(_) => State::Installed,
                None => State::OtherVersion,
            },
            None if p.downloads_for(&current_platform()).is_empty() => State::Unavailable,
            None => State::NotInstalled,
        }
    }

    /// The control channel's `app.packages*` methods.
    pub(crate) fn command(app: &mut SoundApp, method: &str, p: &Value) -> Result<Value, String> {
        match method {
            "app.packages" => Ok(status(app)),
            "app.packages.refresh" => {
                if let Some(src) = p.get("index").and_then(Value::as_str) {
                    app.packages_rt.source = src.trim().to_string();
                }
                refresh(app);
                Ok(json!({"started": true, "index": source(&app.packages_rt)}))
            }
            "app.packages.install" | "app.packages.update" | "app.packages.remove" => {
                let id = p.get("id").and_then(Value::as_str).ok_or("`id` (the package id) is required")?;
                let action = match method {
                    "app.packages.remove" => Action::Remove,
                    "app.packages.update" => Action::Update,
                    _ => Action::Install,
                };
                start(app, id, action)?;
                Ok(json!({"started": true}))
            }
            other => Err(format!("unknown method `{other}`")),
        }
    }

    fn status(app: &SoundApp) -> Value {
        let rt = &app.packages_rt;
        let packages: Vec<Value> = rt
            .index
            .iter()
            .flat_map(|i| i.packages.iter())
            .map(|p| {
                let f = rt.found.get(&p.id).and_then(Option::as_ref);
                json!({
                    "id": p.id,
                    "name": p.name,
                    "version": p.version,
                    "summary": p.summary,
                    "state": state(rt, p).id(),
                    "installed_version": f.and_then(|f| f.version.clone()),
                    "installed_in": f.map(|f| f.root.display().to_string()),
                    "bundles": f.map(|f| f.bundles.clone()).unwrap_or_default(),
                    "download_bytes": p.download_bytes(&current_platform()),
                    "note": rt.notes.get(&p.id).map(|n| json!({"ok": n.ok, "text": n.text, "downloaded_bytes": n.downloaded})),
                })
            })
            .collect();
        let job = rt.job.as_ref().map(|j| {
            json!({"id": j.id, "action": j.action.verb(), "downloaded_bytes": j.bytes.load(Ordering::Relaxed), "total_bytes": j.total.load(Ordering::Relaxed)})
        });
        json!({
            "index": source(rt),
            "checking": rt.fetch.is_some(),
            "error": rt.error,
            "checked_secs_ago": rt.checked.map(|c| c.elapsed().as_secs()),
            "platform": current_platform(),
            "packages": packages,
            "job": job,
        })
    }

    fn start(app: &mut SoundApp, id: &str, action: Action) -> Result<(), String> {
        if app.packages_rt.job.is_some() {
            return Err("a package is already being changed; wait for it to finish".into());
        }
        let pkg = app.packages_rt.index.as_ref().and_then(|i| i.package(id)).cloned().ok_or_else(|| format!("no package {id} in the index"))?;
        let dir = package_dir(app).ok_or("this copy has no folder for packages")?;
        let found = app.packages_rt.found.get(id).cloned().flatten();
        let st = state(&app.packages_rt, &pkg);
        match action {
            Action::Remove if found.is_none() => return Err(format!("{} isn't installed", pkg.name)),
            Action::Install | Action::Update if st == State::Unavailable => {
                return Err(format!("{} has no download for this computer ({})", pkg.name, current_platform()));
            }
            Action::Install if st == State::Installed => return Err(format!("{} {} is already installed", pkg.name, pkg.version)),
            _ => {}
        }
        let (_, user_root) = roots_env(&app.packages_rt);
        // An installed copy is changed where it is; a new one goes to this account's plugin folder.
        let root = found.as_ref().map(|f| f.root.clone()).or(user_root).ok_or("couldn't find this account's plugin folder")?;
        let previous = app.packages_rt.record.as_ref().and_then(|r| r.get(id)).cloned();
        let work = Work {
            action,
            pkg,
            index_source: app.packages_rt.index_source.clone(),
            root,
            found,
            previous,
            dir,
            updater: crate::delta_update::updater_path(),
        };
        let (tx, rx) = std::sync::mpsc::channel();
        let bytes = Arc::new(AtomicU64::new(0));
        let total = Arc::new(AtomicU64::new(0));
        let (b, t) = (bytes.clone(), total.clone());
        std::thread::Builder::new()
            .name("package".into())
            .spawn(move || {
                let _ = tx.send(run(&work, &b, &t));
            })
            .map_err(|e| format!("couldn't start ({e})"))?;
        log::info!("packages: {} {id}", action.verb());
        app.packages_rt.notes.remove(id);
        app.packages_rt.job = Some(Job { id: id.to_string(), action, rx, bytes, total });
        Ok(())
    }

    /// Download, check, unpack and swap in one package (or remove it). Runs off the UI thread.
    fn run(w: &Work, bytes: &AtomicU64, total: &AtomicU64) -> Result<Done, String> {
        let stage = w.dir.join(format!("stage-{}-{}", w.pkg.id, std::process::id()));
        let _ = std::fs::remove_dir_all(&stage);
        std::fs::create_dir_all(&stage).map_err(|e| format!("couldn't create {} ({e})", stage.display()))?;
        let result = run_in(w, &stage, bytes, total);
        let _ = std::fs::remove_dir_all(&stage);
        result
    }

    fn run_in(w: &Work, stage: &Path, bytes: &AtomicU64, total: &AtomicU64) -> Result<Done, String> {
        if w.action == Action::Remove {
            let bundles = w.found.as_ref().map(|f| f.bundles.clone()).unwrap_or_default();
            let version = w.found.as_ref().and_then(|f| f.version.clone()).unwrap_or_default();
            let change = install::plan_remove(&w.root, &bundles, stage, &version)?;
            swap_in(&change.plan, w.updater.as_deref(), stage)?;
            return Ok(Done { entry: None, downloaded: 0 });
        }
        let platform = current_platform();
        let downloads = w.pkg.downloads_for(&platform);
        total.store(w.pkg.download_bytes(&platform), Ordering::Relaxed);
        let fetched = stage.join("download");
        let http = craft_update::Http::new(&format!("Solara/{VERSION}"))?;
        let stop = craft_update::Stop::default();
        let mut downloaded = 0u64;
        let mut unpacked = Vec::new();
        for (i, d) in downloads.iter().enumerate() {
            let from = d.resolve(&w.index_source);
            let dest = fetched.join(format!("{i}-{}.zip", d.format.extension()));
            if is_remote(&from) {
                let before = downloaded;
                http.download(&from, &dest, d.size, &d.sha256, &stop, &mut |t| {
                    if let craft_update::http::Transfer::Bytes { done, .. } = t {
                        bytes.store(before.saturating_add(done), Ordering::Relaxed);
                    }
                })?;
            } else {
                std::fs::create_dir_all(&fetched).map_err(|e| format!("couldn't create {} ({e})", fetched.display()))?;
                std::fs::copy(&from, &dest).map_err(|e| format!("couldn't read {from} ({e})"))?;
                if soundcraft_delta::sha256_file(&dest)? != d.sha256 {
                    return Err("the download doesn't match its published checksum, so it wasn't installed".into());
                }
            }
            downloaded = downloaded.saturating_add(d.size);
            bytes.store(downloaded, Ordering::Relaxed);
            unpacked.push(install::unpack(&dest, d, Os::current(), &stage.join("files"))?);
        }
        let previous = w.previous.as_ref().filter(|p| p.root == w.root);
        let mut change = install::plan_install(&w.pkg, &unpacked, &w.root, stage, previous)?;
        // The bundles a copy put there by something else has, under other names, go too.
        if let Some(f) = w.found.as_ref().filter(|f| !f.recorded) {
            let keep: Vec<String> = unpacked.iter().map(|u| u.bundle.clone()).collect();
            let extra: Vec<String> = f.bundles.iter().filter(|b| !keep.contains(b)).cloned().collect();
            change.plan.steps.extend(install::plan_remove(&w.root, &extra, stage, "")?.plan.steps);
        }
        change.plan.downloaded_bytes = downloaded;
        swap_in(&change.plan, w.updater.as_deref(), stage)?;
        let entry = InstalledPackage {
            id: w.pkg.id.clone(),
            name: w.pkg.name.clone(),
            version: w.pkg.version.clone(),
            root: w.root.clone(),
            bundles: change.bundles,
        };
        Ok(Done { entry: Some(entry), downloaded })
    }

    /// Swap the files in here, or through an administrator copy of the updater when the plugin
    /// folder needs one (a copy installed for everyone).
    fn swap_in(plan: &soundcraft_delta::Plan, updater: Option<&Path>, stage: &Path) -> Result<(), String> {
        if plan.steps.is_empty() {
            return Ok(());
        }
        let _ = std::fs::create_dir_all(&plan.install_dir);
        if soundcraft_delta::swap::writable(&plan.install_dir) {
            return install::apply(plan);
        }
        let updater =
            updater.ok_or_else(|| format!("{} needs an administrator, and the updater (solara-updater) isn't here", plan.install_dir.display()))?;
        let plan_path = stage.join(soundcraft_delta::PLAN_NAME);
        plan.write(&plan_path)?;
        elevated(updater, &plan_path, stage)?;
        install::tidy(plan);
        Ok(())
    }

    #[cfg(windows)]
    fn elevated(updater: &Path, plan_path: &Path, stage: &Path) -> Result<(), String> {
        let q = |p: &Path| p.display().to_string().replace('\'', "''");
        let script = format!(
            "$p = Start-Process -FilePath '{}' -ArgumentList '--swap-only --plan \"{}\"' -Verb RunAs -Wait -PassThru; exit $p.ExitCode",
            q(updater),
            q(plan_path)
        );
        let error_file = stage.join("swap-error.txt");
        let _ = std::fs::remove_file(&error_file);
        let status = std::process::Command::new("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-WindowStyle", "Hidden", "-Command", &script])
            .status()
            .map_err(|e| format!("couldn't ask Windows for permission ({e})"))?;
        if status.success() {
            return Ok(());
        }
        match std::fs::read_to_string(&error_file) {
            Ok(e) if !e.is_empty() => Err(e),
            _ => Err("Windows didn't give permission to change the plugin folder".into()),
        }
    }

    #[cfg(not(windows))]
    fn elevated(_updater: &Path, plan_path: &Path, _stage: &Path) -> Result<(), String> {
        Err(format!("this account can't change the plugin folder ({})", plan_path.display()))
    }

    /// Poll the index check and the running job.
    pub(crate) fn tick(app: &mut SoundApp, ctx: &egui::Context) {
        // Opened some other way than its menu (settings, `ui.set`): check the index once.
        if app.ui.show_packages && app.packages_rt.index.is_none() && app.packages_rt.fetch.is_none() && app.packages_rt.error.is_none() {
            refresh(app);
        }
        if let Some(rx) = &app.packages_rt.fetch {
            let got = match rx.try_recv() {
                Ok(r) => Some(r),
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => Some(Err("the index check stopped unexpectedly".into())),
            };
            if let Some(r) = got {
                app.packages_rt.fetch = None;
                match r {
                    Ok((index, src)) => {
                        log::info!("packages: {} packages in {src}", index.packages.len());
                        app.packages_rt.index = Some(index);
                        app.packages_rt.index_source = src;
                        app.packages_rt.error = None;
                        app.packages_rt.checked = Some(std::time::Instant::now());
                        rescan_found(app);
                    }
                    Err(e) => {
                        log::warn!("packages: {e}");
                        app.packages_rt.error = Some(e);
                    }
                }
            } else {
                ctx.request_repaint_after(std::time::Duration::from_millis(200));
            }
        }
        let Some(job) = &app.packages_rt.job else { return };
        let done = match job.rx.try_recv() {
            Ok(r) => r,
            Err(TryRecvError::Empty) => {
                ctx.request_repaint_after(std::time::Duration::from_millis(200));
                return;
            }
            Err(TryRecvError::Disconnected) => Err("the change stopped unexpectedly".into()),
        };
        let (id, action) = (job.id.clone(), job.action);
        app.packages_rt.job = None;
        finish(app, &id, action, done);
    }

    fn finish(app: &mut SoundApp, id: &str, action: Action, done: Result<Done, String>) {
        let pkg = app.packages_rt.index.as_ref().and_then(|i| i.package(id)).cloned();
        let name = pkg.as_ref().map_or(id.to_string(), |p| p.name.clone());
        let before = app.packages_rt.found.get(id).cloned().flatten().and_then(|f| f.version);
        let note = match done {
            Ok(d) => {
                let mut record = app.packages_rt.record.clone().unwrap_or_default();
                match d.entry {
                    Some(e) => record.set(e),
                    None => record.remove(id),
                }
                let saved = package_dir(app).map_or(Ok(()), |dir| record.write(&dir.join(RECORD_NAME)));
                if let Err(e) = saved {
                    log::warn!("packages: {e}");
                }
                // Solara's own plugin lists pick the change up now.
                soundcraft_vst3_host::rescan();
                soundcraft_clap_host::rescan();
                let version = pkg.as_ref().map(|p| p.version.clone()).unwrap_or_default();
                let text = match action {
                    Action::Remove => format!("{name} was removed."),
                    Action::Install => format!("{name} {version} was installed ({} downloaded).", size(d.downloaded)),
                    Action::Update => match &before {
                        Some(v) => format!("{name} was updated from {v} to {version} (only {name}: {} downloaded).", size(d.downloaded)),
                        None => format!("{name} was updated to {version} (only {name}: {} downloaded).", size(d.downloaded)),
                    },
                };
                log::info!("packages: {text}");
                Note { ok: true, text, downloaded: d.downloaded }
            }
            Err(e) => {
                let kept = match (&before, action) {
                    (Some(v), _) => format!(" {name} {v} is unchanged."),
                    (None, Action::Remove) | (None, Action::Update) => format!(" {name} is unchanged."),
                    (None, Action::Install) => String::new(),
                };
                let text = format!("{name} wasn't {}: {e}.{kept}", past(action));
                log::warn!("packages: {text}");
                Note { ok: false, text, downloaded: 0 }
            }
        };
        app.packages_rt.notes.insert(id.to_string(), note);
        rescan_found(app);
    }

    fn past(a: Action) -> &'static str {
        match a {
            Action::Install => "installed",
            Action::Update => "updated",
            Action::Remove => "removed",
        }
    }

    fn size(bytes: u64) -> String {
        if bytes >= 1 << 20 { format!("{:.1} MB", bytes as f64 / f64::from(1u32 << 20)) } else { format!("{:.0} KB", (bytes as f64 / 1024.0).ceil()) }
    }

    pub(crate) fn show(app: &mut SoundApp, ctx: &egui::Context) {
        if !app.ui.show_packages {
            return;
        }
        let mut open = true;
        let mut clicked: Option<(String, Action)> = None;
        let mut refresh_clicked = false;
        let t = crate::theme::Tokens::current();
        egui::Window::new("SolPlugs Packages")
            .open(&mut open)
            .default_size(vec2(640.0, 420.0))
            .default_pos(ctx.content_rect().center() - vec2(320.0, 210.0))
            .resizable(true)
            .collapsible(false)
            .show(ctx, |ui| {
                let rt = &app.packages_rt;
                ui.horizontal(|ui| {
                    let name = rt.index.as_ref().map(|i| i.name.clone()).filter(|n| !n.is_empty()).unwrap_or_else(|| "SolPlugs".into());
                    ui.label(RichText::new(format!("{name} repository")).strong());
                    if rt.fetch.is_some() {
                        ui.spinner();
                        ui.label(RichText::new("Checking for updates…").color(t.text_dim));
                    } else if let Some(c) = rt.checked {
                        let mins = c.elapsed().as_secs() / 60;
                        let when = if mins == 0 { "just now".to_string() } else { format!("{mins} min ago") };
                        ui.label(RichText::new(format!("Checked for updates {when}")).color(t.text_dim));
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        refresh_clicked =
                            ui.add_enabled(rt.fetch.is_none(), egui::Button::new("Refresh")).on_hover_text("Check the package index again").clicked();
                    });
                });
                if let Some(e) = &rt.error {
                    ui.colored_label(t.meter_red, format!("The package index couldn't be read: {e}"));
                }
                ui.separator();
                let Some(index) = &rt.index else {
                    if rt.error.is_none() {
                        ui.label("Reading the package index…");
                    }
                    return;
                };
                egui::ScrollArea::vertical().auto_shrink([false, true]).max_height(ui.available_height() - 28.0).show(ui, |ui| {
                    for p in &index.packages {
                        let st = state(rt, p);
                        let f = rt.found.get(&p.id).and_then(Option::as_ref);
                        let busy = rt.job.as_ref().filter(|j| j.id == p.id);
                        ui.horizontal(|ui| {
                            ui.vertical(|ui| {
                                ui.horizontal(|ui| {
                                    ui.label(RichText::new(&p.name).strong());
                                    ui.label(RichText::new(format!("v{}", p.version)).color(t.text_dim));
                                    if !p.vendor.is_empty() {
                                        ui.label(RichText::new(format!("· {}", p.vendor)).color(t.text_dim));
                                    }
                                });
                                if !p.summary.is_empty() {
                                    ui.label(RichText::new(&p.summary).small());
                                }
                                let formats: Vec<&str> = p.downloads_for(&current_platform()).iter().map(|d| d.format.label()).collect();
                                let line = match (st, f.and_then(|f| f.version.as_deref())) {
                                    (State::Unavailable, _) => "Not available for this computer yet".to_string(),
                                    (State::NotInstalled, _) => format!("{} · {}", formats.join(" + "), size(p.download_bytes(&current_platform()))),
                                    (State::Installed, Some(v)) => format!("Installed: {v} ({})", formats.join(" + ")),
                                    (State::UpdateAvailable, Some(v)) => {
                                        format!("Installed: {v} · update to {} ({})", p.version, size(p.download_bytes(&current_platform())))
                                    }
                                    _ => "Installed: another version".to_string(),
                                };
                                ui.label(RichText::new(line).small().color(t.text_dim));
                                if !p.homepage.is_empty() {
                                    ui.hyperlink_to(RichText::new("Releases").small(), &p.homepage);
                                }
                                if let Some(n) = rt.notes.get(&p.id) {
                                    ui.label(RichText::new(&n.text).small().color(if n.ok { t.text } else { t.meter_red }));
                                }
                            });
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if let Some(j) = busy {
                                    let (d, total) = (j.bytes.load(Ordering::Relaxed), j.total.load(Ordering::Relaxed));
                                    ui.spinner();
                                    if j.action != Action::Remove && total > 0 {
                                        ui.label(format!("{} of {}", size(d), size(total)));
                                    }
                                    return;
                                }
                                let idle = rt.job.is_none();
                                if f.is_some() && ui.add_enabled(idle, egui::Button::new("Remove")).clicked() {
                                    clicked = Some((p.id.clone(), Action::Remove));
                                }
                                match st {
                                    State::NotInstalled => {
                                        if ui.add_enabled(idle, egui::Button::new("Install")).clicked() {
                                            clicked = Some((p.id.clone(), Action::Install));
                                        }
                                    }
                                    State::UpdateAvailable | State::OtherVersion => {
                                        if ui.add_enabled(idle, egui::Button::new("Update")).clicked() {
                                            clicked = Some((p.id.clone(), Action::Update));
                                        }
                                    }
                                    State::Installed | State::Unavailable => {}
                                }
                            });
                        });
                        ui.separator();
                    }
                });
                let (_, user_root) = roots_env(rt);
                if let Some(r) = user_root {
                    ui.label(
                        RichText::new(format!("New plugins go to {} (VST3 and CLAP), where Solara, Reaper and other DAWs find them.", r.display()))
                            .small()
                            .color(t.text_dim),
                    );
                }
            });
        app.ui.show_packages = open;
        if refresh_clicked {
            refresh(app);
        }
        if let Some((id, action)) = clicked
            && let Err(e) = start(app, &id, action)
        {
            app.packages_rt.notes.insert(id, Note { ok: false, text: e, downloaded: 0 });
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use crate::services::Services;
        use soundcraft_delta::sha256_bytes;
        use std::io::Write;

        struct Tmp(PathBuf);
        impl Drop for Tmp {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }

        /// One package version: a VST3 zip next to the index, described for the index.
        fn version_of(dir: &Path, id: &str, name: &str, version: &str, binary: &[u8]) -> Value {
            let bundle = format!("{name}.vst3");
            let inner = format!("{bundle}/Contents/x86_64-win/{name}.vst3");
            let file = format!("{id}-{version}.zip");
            let mut w = zip::ZipWriter::new(std::fs::File::create(dir.join(&file)).unwrap());
            w.start_file(inner.as_str(), zip::write::SimpleFileOptions::default()).unwrap();
            w.write_all(binary).unwrap();
            w.finish().unwrap();
            let bytes = std::fs::read(dir.join(&file)).unwrap();
            json!({
                "id": id, "name": name, "version": version, "summary": "A test plugin",
                "downloads": [{
                    "platform": current_platform(), "format": "vst3", "url": file, "size": bytes.len(),
                    "sha256": sha256_bytes(&bytes), "bundle": bundle,
                    "files": [{"path": inner, "size": binary.len(), "sha256": sha256_bytes(binary)}],
                }],
            })
        }

        fn write_index(dir: &Path, name: &str, packages: Vec<Value>) -> String {
            let path = dir.join(name);
            std::fs::write(
                &path,
                serde_json::to_vec(&json!({"format": "solplugs-index", "format_version": 1, "name": "Test", "packages": packages})).unwrap(),
            )
            .unwrap();
            path.display().to_string()
        }

        fn settle(app: &mut SoundApp, ctx: &egui::Context) {
            for _ in 0..500 {
                tick(app, ctx);
                if app.packages_rt.fetch.is_none() && app.packages_rt.job.is_none() {
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            panic!("the package job never finished");
        }

        fn pkg_state(app: &SoundApp, id: &str) -> Value {
            let s = status(app);
            s["packages"].as_array().unwrap().iter().find(|p| p["id"] == id).cloned().unwrap()
        }

        #[test]
        fn browse_install_update_and_remove_one_package_at_a_time() {
            let dir = std::env::temp_dir().join(format!("solara-packages-ui-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            let _tmp = Tmp(dir.clone());
            let root = dir.join("plugins");
            let ctx = egui::Context::default();
            let mut app = SoundApp::new(soundcraft_engine::demo::demo_engine(), None, Services::default());
            app.package_dir = Some(dir.join("state"));
            app.packages_rt.root_override = Some(root.clone());

            let a1 = version_of(&dir, "alpha", "Alpha", "1.0.0", b"alpha one");
            let b1 = version_of(&dir, "beta", "Beta", "2.0.0", b"beta one");
            let index = write_index(&dir, "index1.json", vec![a1.clone(), b1.clone()]);
            command(&mut app, "app.packages.refresh", &json!({"index": index})).unwrap();
            settle(&mut app, &ctx);
            assert_eq!(pkg_state(&app, "alpha")["state"], "not_installed");
            assert_eq!(pkg_state(&app, "alpha")["version"], "1.0.0");

            for id in ["alpha", "beta"] {
                command(&mut app, "app.packages.install", &json!({"id": id})).unwrap();
                settle(&mut app, &ctx);
                assert_eq!(pkg_state(&app, id)["state"], "installed", "{:?}", pkg_state(&app, id));
            }
            let vst3 = root.join(roots::format_dir(Os::current(), soundcraft_packages::Format::Vst3));
            let alpha_bin = vst3.join("Alpha.vst3/Contents/x86_64-win/Alpha.vst3");
            let beta_bin = vst3.join("Beta.vst3/Contents/x86_64-win/Beta.vst3");
            assert_eq!(std::fs::read(&alpha_bin).unwrap(), b"alpha one");
            assert!(command(&mut app, "app.packages.install", &json!({"id": "alpha"})).is_err(), "already installed");

            // Alpha 1.0.1 is published: only Alpha downloads; Beta stays as it is.
            let beta_before = std::fs::metadata(&beta_bin).unwrap().modified().unwrap();
            let a2 = version_of(&dir, "alpha", "Alpha", "1.0.1", b"alpha two, a little longer");
            let index = write_index(&dir, "index2.json", vec![a2.clone(), b1.clone()]);
            command(&mut app, "app.packages.refresh", &json!({"index": index})).unwrap();
            settle(&mut app, &ctx);
            assert_eq!(pkg_state(&app, "alpha")["state"], "update_available");
            assert_eq!(pkg_state(&app, "alpha")["installed_version"], "1.0.0");
            assert_eq!(pkg_state(&app, "beta")["state"], "installed");
            command(&mut app, "app.packages.update", &json!({"id": "alpha"})).unwrap();
            settle(&mut app, &ctx);
            assert_eq!(pkg_state(&app, "alpha")["note"]["downloaded_bytes"], a2["downloads"][0]["size"], "only Alpha's zip is fetched");
            assert_eq!(std::fs::read(&alpha_bin).unwrap(), b"alpha two, a little longer");
            assert_eq!(pkg_state(&app, "alpha")["state"], "installed");
            let note = pkg_state(&app, "alpha")["note"]["text"].as_str().unwrap().to_string();
            assert!(note.contains("updated from 1.0.0 to 1.0.1"), "{note}");
            assert_eq!(std::fs::metadata(&beta_bin).unwrap().modified().unwrap(), beta_before);

            // Alpha 1.0.2's zip doesn't match its published hash: 1.0.1 stays, and the note says so.
            let mut a3 = version_of(&dir, "alpha", "Alpha", "1.0.2", b"alpha three");
            a3["downloads"][0]["sha256"] = json!(sha256_bytes(b"tampered"));
            let index = write_index(&dir, "index3.json", vec![a3, b1.clone()]);
            command(&mut app, "app.packages.refresh", &json!({"index": index})).unwrap();
            settle(&mut app, &ctx);
            command(&mut app, "app.packages.update", &json!({"id": "alpha"})).unwrap();
            settle(&mut app, &ctx);
            let note = pkg_state(&app, "alpha")["note"].clone();
            assert_eq!(note["ok"], false);
            assert!(note["text"].as_str().unwrap().contains("checksum") && note["text"].as_str().unwrap().contains("1.0.1 is unchanged"), "{note}");
            assert_eq!(std::fs::read(&alpha_bin).unwrap(), b"alpha two, a little longer");
            assert_eq!(pkg_state(&app, "alpha")["installed_version"], "1.0.1");

            command(&mut app, "app.packages.remove", &json!({"id": "alpha"})).unwrap();
            settle(&mut app, &ctx);
            assert!(!vst3.join("Alpha.vst3").exists());
            assert!(beta_bin.exists());
            assert_eq!(pkg_state(&app, "alpha")["state"], "not_installed");
            let record = Record::read(&dir.join("state").join(RECORD_NAME)).unwrap();
            assert_eq!(record.packages.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(), vec!["beta"]);
        }

        #[test]
        fn opening_the_window_reads_the_index_and_draws_it() {
            let dir = std::env::temp_dir().join(format!("solara-packages-win-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            let _tmp = Tmp(dir.clone());
            let index = write_index(&dir, "index.json", vec![version_of(&dir, "alpha", "Alpha", "1.0.0", b"a")]);
            let mut app = SoundApp::new(soundcraft_engine::demo::demo_engine(), None, Services::default());
            app.packages_rt.source = index;
            app.packages_rt.root_override = Some(dir.join("plugins"));
            crate::menus::run_ui_command(&mut app, "window.packages", &json!({"value": true})).unwrap().unwrap();
            assert!(app.ui.show_packages);
            let ctx = egui::Context::default();
            settle(&mut app, &ctx);
            assert_eq!(pkg_state(&app, "alpha")["state"], "not_installed");
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| show(&mut app, ui.ctx()));
            out.textures_delta.clear();
        }
    }
}
