//! Solara's standalone updater (shipped as `solara-updater.exe`).
//!
//! Solara downloads and checks a delta update, builds the new files in a staging folder, starts a
//! copy of this program from that folder and quits. This program then:
//!
//! 1. waits for Solara, and anything else running from the install folder, to exit; after 10 s it
//!    names each process still running (name and PID) and asks whether to close it or cancel;
//! 2. swaps the staged files in (checked against their SHA-256 again, all or none), as an
//!    administrator when the install folder needs one (a copy installed for everyone);
//! 3. writes how it went where Solara reads it at its next start, and starts Solara again: the new
//!    version, or the old one when the update was cancelled or refused.
//!
//! It never replaces a file of a program that is still running.
#![cfg_attr(windows, windows_subsystem = "windows")]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use clap::{Parser, ValueEnum};
use soundcraft_delta::wait::{Busy, Proc, Processes, Timing, wait_for_exit};
use soundcraft_delta::{Plan, UpdateResult, swap};

/// Apply a staged Solara update
#[derive(Parser)]
#[command(name = "solara-updater", version)]
struct Args {
    /// The plan Solara wrote (plan.json in the staging folder)
    #[arg(long)]
    plan: PathBuf,
    /// A process to wait for (Solara's own PID); repeatable
    #[arg(long = "wait-pid")]
    wait_pid: Vec<u32>,
    /// What to do about a process still running after 10 s
    #[arg(long, value_enum, default_value = "ask")]
    if_busy: IfBusy,
    /// Don't start Solara afterwards
    #[arg(long)]
    no_relaunch: bool,
    /// Only swap the files in (the administrator copy started for an install folder that needs one)
    #[arg(long, hide = true)]
    swap_only: bool,
}

#[derive(Clone, Copy, ValueEnum)]
enum IfBusy {
    Ask,
    Close,
    Cancel,
}

/// The file the administrator copy leaves its error in.
const SWAP_ERROR: &str = "swap-error.txt";

fn main() {
    let args = match Args::try_parse() {
        Ok(a) => a,
        Err(e) => e.exit(),
    };
    std::process::exit(run(&args));
}

fn run(args: &Args) -> i32 {
    let plan = match Plan::read(&args.plan) {
        Ok(p) => p,
        Err(e) => {
            ask_ok(&format!("Solara couldn't be updated: {e}"));
            return 2;
        }
    };
    let log = Log(plan.stage_dir.join("updater.log"));
    if args.swap_only {
        log.line("swapping as administrator");
        return match swap::apply(&plan) {
            Ok(()) => 0,
            Err(e) => {
                log.line(&e);
                let _ = std::fs::write(plan.stage_dir.join(SWAP_ERROR), &e);
                1
            }
        };
    }
    log.line(&format!("updating {} {} to {} in {}", plan.app, plan.from_version, plan.to_version, plan.install_dir.display()));
    let outcome = wait(args, &plan, &log).and_then(|()| install(&plan, &log));
    let result = UpdateResult {
        ok: outcome.is_ok(),
        from_version: plan.from_version.clone(),
        to_version: plan.to_version.clone(),
        message: match &outcome {
            Ok(()) => format!("Solara was updated to {}. The update downloaded {}.", plan.to_version, size(plan.downloaded_bytes)),
            Err(e) => format!("Solara wasn't updated: {e}. Solara {} is unchanged.", plan.from_version),
        },
        downloaded_bytes: plan.downloaded_bytes,
    };
    log.line(&result.message);
    if let Err(e) = result.write(&plan.result_file) {
        log.line(&format!("couldn't write the result: {e}"));
    }
    if !args.no_relaunch
        && let Some(r) = &plan.relaunch
    {
        relaunch(&plan.install_dir, &r.exe, &r.args, &log);
    }
    i32::from(outcome.is_err())
}

fn wait(args: &Args, plan: &Plan, log: &Log) -> Result<(), String> {
    let mut procs = SystemProcesses(sysinfo::System::new().map_err(|e| format!("couldn't list the running programs ({e})"))?);
    let mut ask = |p: &Proc| {
        log.line(&format!("still running after 10 s: {} (PID {})", p.name, p.pid));
        let answer = match args.if_busy {
            IfBusy::Close => Busy::Close,
            IfBusy::Cancel => Busy::Cancel,
            IfBusy::Ask => ask_close(p, &plan.from_version),
        };
        log.line(&format!("{} (PID {}): {answer:?}", p.name, p.pid));
        answer
    };
    wait_for_exit(&mut procs, &args.wait_pid, &plan.install_dir, std::process::id(), Timing::default(), &mut ask)
}

/// Swap the files in here, or in an administrator copy when the install folder needs one.
fn install(plan: &Plan, log: &Log) -> Result<(), String> {
    if swap::writable(&plan.install_dir) {
        return swap::apply(plan);
    }
    log.line("the install folder needs an administrator");
    elevated_swap(plan)
}

#[cfg(windows)]
fn elevated_swap(plan: &Plan) -> Result<(), String> {
    let me = std::env::current_exe().map_err(|e| format!("couldn't find the updater ({e})"))?;
    let plan_path = plan.stage_dir.join(soundcraft_delta::PLAN_NAME);
    let q = |p: &Path| p.display().to_string().replace('\'', "''");
    let script = format!(
        "$p = Start-Process -FilePath '{}' -ArgumentList '--swap-only --plan \"{}\"' -Verb RunAs -Wait -PassThru; exit $p.ExitCode",
        q(&me),
        q(&plan_path)
    );
    let _ = std::fs::remove_file(plan.stage_dir.join(SWAP_ERROR));
    let status = std::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-WindowStyle", "Hidden", "-Command", &script])
        .status()
        .map_err(|e| format!("couldn't ask Windows for permission ({e})"))?;
    if status.success() {
        return Ok(());
    }
    match std::fs::read_to_string(plan.stage_dir.join(SWAP_ERROR)) {
        Ok(e) if !e.is_empty() => Err(e),
        _ => Err("Windows didn't give permission to change the install folder".into()),
    }
}

#[cfg(not(windows))]
fn elevated_swap(plan: &Plan) -> Result<(), String> {
    Err(format!("this account can't change {}", plan.install_dir.display()))
}

fn relaunch(dir: &Path, exe: &str, args: &[String], log: &Log) {
    let Ok(rel) = soundcraft_delta::safe_relative(exe) else { return };
    match std::process::Command::new(dir.join(rel)).args(args).current_dir(dir).spawn() {
        Ok(child) => log.line(&format!("started {exe} (PID {})", child.id())),
        Err(e) => log.line(&format!("couldn't start {exe}: {e}")),
    }
}

/// `bytes` as KB or MB.
fn size(bytes: u64) -> String {
    if bytes >= 1 << 20 { format!("{:.1} MB", bytes as f64 / f64::from(1u32 << 20)) } else { format!("{:.0} KB", (bytes as f64 / 1024.0).ceil()) }
}

/// "Close this process?" — Yes closes it, No cancels the update.
fn ask_close(p: &Proc, from_version: &str) -> Busy {
    let text = format!(
        "Solara can't finish updating while {name} (PID {pid}) is still running.\n\n\
         Close {name} now? Anything unsaved in it will be lost.\n\n\
         Choose No to cancel the update and keep Solara {from_version}.",
        name = p.name,
        pid = p.pid
    );
    let answer = rfd::MessageDialog::new()
        .set_title("Solara Update")
        .set_description(text)
        .set_level(rfd::MessageLevel::Warning)
        .set_buttons(rfd::MessageButtons::YesNo)
        .show();
    if answer == rfd::MessageDialogResult::Yes { Busy::Close } else { Busy::Cancel }
}

fn ask_ok(text: &str) {
    let _ = rfd::MessageDialog::new().set_title("Solara Update").set_description(text).set_level(rfd::MessageLevel::Error).show();
}

/// The running processes, from the system.
struct SystemProcesses(sysinfo::System);

impl Processes for SystemProcesses {
    fn list(&mut self) -> Vec<Proc> {
        use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, UpdateKind};
        self.0.refresh_processes_specifics(ProcessesToUpdate::All, true, ProcessRefreshKind::nothing().with_exe(UpdateKind::OnlyIfNotSet));
        self.0
            .processes()
            .iter()
            .map(|(pid, p)| Proc { pid: pid.as_u32(), name: p.name().to_string_lossy().into_owned(), exe: p.exe().map(Path::to_path_buf) })
            .collect()
    }

    fn kill(&mut self, pid: u32) -> bool {
        self.0.process(sysinfo::Pid::from_u32(pid)).is_some_and(sysinfo::Process::kill)
    }

    fn sleep(&mut self, d: Duration) {
        std::thread::sleep(d);
    }
}

/// The updater's log, beside its plan.
struct Log(PathBuf);

impl Log {
    fn line(&self, text: &str) {
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&self.0) {
            let _ = writeln!(f, "{text}");
        }
    }
}
