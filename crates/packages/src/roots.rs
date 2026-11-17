//! Where plugins live: a plugin root holds one folder per format (`VST3`, `CLAP` on Windows and
//! macOS; `.vst3`, `.clap` in the home folder elsewhere), the folders every DAW scans.

use std::path::{Path, PathBuf};

use crate::index::Format;

/// The environment values the plugin folders come from.
#[derive(Clone, Debug, Default)]
pub struct Env {
    pub home: Option<PathBuf>,
    /// `%COMMONPROGRAMFILES%` (Windows).
    pub common_program_files: Option<PathBuf>,
    /// `%LOCALAPPDATA%` (Windows).
    pub local_app_data: Option<PathBuf>,
}

impl Env {
    pub fn current() -> Self {
        Env {
            home: std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from),
            common_program_files: std::env::var_os("COMMONPROGRAMFILES").map(PathBuf::from),
            local_app_data: std::env::var_os("LOCALAPPDATA").map(PathBuf::from),
        }
    }
}

/// Which operating system's layout to use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Os {
    Windows,
    MacOs,
    Unix,
}

impl Os {
    pub fn current() -> Os {
        if cfg!(windows) {
            Os::Windows
        } else if cfg!(target_os = "macos") {
            Os::MacOs
        } else {
            Os::Unix
        }
    }
}

/// The format's folder inside a plugin root.
pub fn format_dir(os: Os, format: Format) -> &'static str {
    match (os, format) {
        (Os::Unix, Format::Vst3) => ".vst3",
        (Os::Unix, Format::Clap) => ".clap",
        (_, Format::Vst3) => "VST3",
        (_, Format::Clap) => "CLAP",
    }
}

/// Where Solara installs packages: this account's plugin root, which needs no administrator
/// (`%LOCALAPPDATA%\Programs\Common` on Windows, `~/Library/Audio/Plug-Ins` on macOS, the home
/// folder elsewhere).
pub fn user_root(os: Os, env: &Env) -> Option<PathBuf> {
    match os {
        Os::Windows => env.local_app_data.as_ref().map(|l| l.join("Programs").join("Common")),
        Os::MacOs => env.home.as_ref().map(|h| h.join("Library/Audio/Plug-Ins")),
        Os::Unix => env.home.clone(),
    }
}

/// The plugin root for everyone on the computer (Solara's installer puts SolPlugs there when it
/// installs for everyone).
pub fn system_root(os: Os, env: &Env) -> Option<PathBuf> {
    match os {
        Os::Windows => env.common_program_files.clone(),
        Os::MacOs => Some(PathBuf::from("/Library/Audio/Plug-Ins")),
        Os::Unix => None,
    }
}

/// Every root to look in for an installed package: this account's first.
pub fn all_roots(os: Os, env: &Env) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = user_root(os, env).into_iter().collect();
    if let Some(s) = system_root(os, env)
        && !v.contains(&s)
    {
        v.push(s);
    }
    v
}

/// The path of a bundle (relative to its root, `/`-separated): `VST3/Song Sense.vst3`.
pub fn bundle_rel(os: Os, format: Format, bundle: &str) -> String {
    format!("{}/{bundle}", format_dir(os, format))
}

/// The absolute path of a bundle under `root`.
pub fn bundle_path(root: &Path, os: Os, format: Format, bundle: &str) -> PathBuf {
    root.join(format_dir(os, format)).join(bundle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_roots() {
        let env = Env { home: None, common_program_files: Some("C:/CPF".into()), local_app_data: Some("C:/U/AppData/Local".into()) };
        assert_eq!(user_root(Os::Windows, &env), Some(PathBuf::from("C:/U/AppData/Local/Programs/Common")));
        assert_eq!(all_roots(Os::Windows, &env), vec![PathBuf::from("C:/U/AppData/Local/Programs/Common"), PathBuf::from("C:/CPF")]);
        assert_eq!(bundle_rel(Os::Windows, Format::Vst3, "A.vst3"), "VST3/A.vst3");
        assert_eq!(bundle_path(Path::new("C:/CPF"), Os::Windows, Format::Clap, "A.clap"), PathBuf::from("C:/CPF/CLAP/A.clap"));
    }

    #[test]
    fn unix_roots() {
        let env = Env { home: Some("/home/u".into()), ..Env::default() };
        assert_eq!(all_roots(Os::Unix, &env), vec![PathBuf::from("/home/u")]);
        assert_eq!(bundle_rel(Os::Unix, Format::Clap, "A.clap"), ".clap/A.clap");
        assert_eq!(all_roots(Os::MacOs, &env), vec![PathBuf::from("/home/u/Library/Audio/Plug-Ins"), PathBuf::from("/Library/Audio/Plug-Ins")]);
    }
}
