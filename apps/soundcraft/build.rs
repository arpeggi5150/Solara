//! Windows only: embed the app icon and version info (VERSIONINFO) into the app EXE (shipped as
//! `solara.exe`), so it shows in Explorer, the taskbar, the Start menu and Alt-Tab.
//!
//! On every other target this does nothing. A missing resource compiler is a warning, so a
//! cross-compile from macOS or Linux still links, unless `SOUNDCRAFT_REQUIRE_WINRES=1` turns it
//! into an error (for release builds).

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=../../assets/app-icon/solara.ico");
    println!("cargo:rerun-if-env-changed=SOUNDCRAFT_REQUIRE_WINRES");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut res = winresource::WindowsResource::new();
    res.set_icon("../../assets/app-icon/solara.ico")
        .set("ProductName", "Solara")
        .set("FileDescription", "Solara digital audio workstation")
        .set("CompanyName", "Zee Creations LLC")
        .set("LegalCopyright", "Solara (c) Zee Creations LLC. Source code MIT OR Apache-2.0, see NOTICE.")
        .set("OriginalFilename", "solara.exe")
        .set("InternalName", "solara");
    if let Err(e) = res.compile() {
        if std::env::var_os("SOUNDCRAFT_REQUIRE_WINRES").is_some() {
            eprintln!("embedding Windows resources failed: {e}");
            std::process::exit(1);
        }
        println!("cargo:warning=soundcraft.exe built without icon/version resources: {e}");
    }
}
