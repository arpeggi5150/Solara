//! The updater's wait for the app, and for anything else running from the install folder, to
//! exit. It never waits forever: after the timeout it names each process still running and asks
//! whether to close it or cancel the update.

use std::path::{Path, PathBuf};
use std::time::Duration;

/// A running process.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Proc {
    pub pid: u32,
    pub name: String,
    pub exe: Option<PathBuf>,
}

/// The system's process list (a fake one in tests).
pub trait Processes {
    fn list(&mut self) -> Vec<Proc>;
    /// Ask `pid` to end now; whether the request went through.
    fn kill(&mut self, pid: u32) -> bool;
    fn sleep(&mut self, d: Duration);
}

/// The answer to "this process is still running".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Busy {
    /// End the process and carry on.
    Close,
    /// Leave everything as it is.
    Cancel,
}

/// How long to wait before asking, and between checks.
#[derive(Clone, Copy, Debug)]
pub struct Timing {
    pub timeout: Duration,
    pub poll: Duration,
    /// After a process was told to close, how long to wait for it before asking again.
    pub grace: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Self { timeout: Duration::from_secs(10), poll: Duration::from_millis(200), grace: Duration::from_secs(5) }
    }
}

/// The processes holding the update up: any in `pids`, and any started from inside
/// `install_dir`, other than `self_pid`.
pub fn blockers(list: &[Proc], pids: &[u32], install_dir: &Path, self_pid: u32) -> Vec<Proc> {
    let dir = normalize(install_dir);
    let mut out: Vec<Proc> = list
        .iter()
        .filter(|p| p.pid != self_pid && (pids.contains(&p.pid) || p.exe.as_deref().is_some_and(|e| normalize(e).starts_with(&dir))))
        .cloned()
        .collect();
    out.sort_by_key(|p| p.pid);
    out.dedup_by_key(|p| p.pid);
    out
}

/// Wait until no blocker is left. After `timing.timeout`, ask `ask` about the first one still
/// running; [`Busy::Close`] ends it and waits `timing.grace` before asking again, [`Busy::Cancel`]
/// returns the error that names it. A process that survives three closes also ends the wait.
pub fn wait_for_exit(
    procs: &mut dyn Processes,
    pids: &[u32],
    install_dir: &Path,
    self_pid: u32,
    timing: Timing,
    ask: &mut dyn FnMut(&Proc) -> Busy,
) -> Result<(), String> {
    let polls = |d: Duration| (d.as_millis() / timing.poll.as_millis().max(1)).max(1);
    let mut left = polls(timing.timeout);
    let mut closes: Vec<(u32, u32)> = Vec::new();
    loop {
        let Some(first) = blockers(&procs.list(), pids, install_dir, self_pid).into_iter().next() else { return Ok(()) };
        if left > 0 {
            left -= 1;
            procs.sleep(timing.poll);
            continue;
        }
        let tries = closes.iter().find(|(p, _)| *p == first.pid).map_or(0, |(_, n)| *n);
        if tries >= 3 {
            return Err(format!("{} (PID {}) would not close, so the update was cancelled", first.name, first.pid));
        }
        match ask(&first) {
            Busy::Cancel => return Err(format!("{} (PID {}) is still running, so the update was cancelled", first.name, first.pid)),
            Busy::Close => {
                procs.kill(first.pid);
                match closes.iter_mut().find(|(p, _)| *p == first.pid) {
                    Some(c) => c.1 += 1,
                    None => closes.push((first.pid, 1)),
                }
                left = polls(timing.grace);
            }
        }
    }
}

/// A path for comparison: no `\\?\` prefix, `/` separators, and on Windows lower case.
fn normalize(p: &Path) -> String {
    let s = p.to_string_lossy();
    let s = s.strip_prefix(r"\\?\").unwrap_or(&s).replace('\\', "/");
    let s = if cfg!(windows) { s.to_lowercase() } else { s };
    format!("{}/", s.trim_end_matches('/'))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Processes that exit after a number of list calls (`None`: only when killed).
    struct Fake {
        procs: Vec<(Proc, Option<u32>)>,
        listed: u32,
        slept: Duration,
        killed: Vec<u32>,
        kill_works: bool,
    }

    impl Processes for Fake {
        fn list(&mut self) -> Vec<Proc> {
            self.listed += 1;
            let n = self.listed;
            self.procs.retain(|(_, exits)| exits.is_none_or(|at| n < at));
            self.procs.iter().map(|(p, _)| p.clone()).collect()
        }
        fn kill(&mut self, pid: u32) -> bool {
            self.killed.push(pid);
            if self.kill_works {
                self.procs.retain(|(p, _)| p.pid != pid);
            }
            true
        }
        fn sleep(&mut self, d: Duration) {
            self.slept += d;
        }
    }

    fn proc(pid: u32, name: &str, exe: &str) -> Proc {
        Proc { pid, name: name.into(), exe: Some(PathBuf::from(exe)) }
    }

    fn fake(procs: Vec<(Proc, Option<u32>)>) -> Fake {
        Fake { procs, listed: 0, slept: Duration::ZERO, killed: Vec::new(), kill_works: true }
    }

    const DIR: &str = "/opt/Solara";

    #[test]
    fn the_app_and_helpers_from_the_install_folder_hold_the_update_others_do_not() {
        let list = vec![
            proc(10, "solara.exe", "/opt/Solara/solara.exe"),
            proc(11, "solara-cli.exe", "/opt/Solara/solara-cli.exe"),
            proc(12, "solara.exe", "/opt/Solara-portable/solara.exe"),
            proc(13, "updater", "/opt/Solara/solara-updater.exe"),
            Proc { pid: 14, name: "other".into(), exe: None },
        ];
        let b: Vec<u32> = blockers(&list, &[14], Path::new(DIR), 13).iter().map(|p| p.pid).collect();
        assert_eq!(b, vec![10, 11, 14]);
    }

    #[test]
    fn an_app_that_exits_in_time_is_waited_for_without_asking() {
        let mut f = fake(vec![(proc(10, "solara.exe", "/opt/Solara/solara.exe"), Some(5))]);
        let mut asked = 0;
        wait_for_exit(&mut f, &[10], Path::new(DIR), 1, Timing::default(), &mut |_| {
            asked += 1;
            Busy::Cancel
        })
        .unwrap();
        assert_eq!(asked, 0);
        assert!(f.slept < Duration::from_secs(10));
    }

    #[test]
    fn after_ten_seconds_it_names_the_process_and_closes_it_when_told() {
        let mut f = fake(vec![(proc(10, "solara.exe", "/opt/Solara/solara.exe"), None)]);
        let mut asked = Vec::new();
        wait_for_exit(&mut f, &[10], Path::new(DIR), 1, Timing::default(), &mut |p| {
            asked.push((p.name.clone(), p.pid));
            Busy::Close
        })
        .unwrap();
        assert_eq!(asked, vec![("solara.exe".to_string(), 10)]);
        assert_eq!(f.killed, vec![10]);
        assert!(f.slept >= Duration::from_secs(10) && f.slept < Duration::from_secs(11), "{:?}", f.slept);
    }

    #[test]
    fn cancelling_names_the_process_and_stops() {
        let mut f = fake(vec![(proc(42, "solara-cli.exe", "/opt/Solara/solara-cli.exe"), None)]);
        let e = wait_for_exit(&mut f, &[], Path::new(DIR), 1, Timing::default(), &mut |_| Busy::Cancel).unwrap_err();
        assert_eq!(e, "solara-cli.exe (PID 42) is still running, so the update was cancelled");
        assert!(f.killed.is_empty());
    }

    #[test]
    fn a_process_that_will_not_close_never_makes_it_wait_forever() {
        let mut f = fake(vec![(proc(10, "solara.exe", "/opt/Solara/solara.exe"), None)]);
        f.kill_works = false;
        let e = wait_for_exit(&mut f, &[10], Path::new(DIR), 1, Timing::default(), &mut |_| Busy::Close).unwrap_err();
        assert!(e.contains("would not close"), "{e}");
        assert_eq!(f.killed.len(), 3);
    }
}
