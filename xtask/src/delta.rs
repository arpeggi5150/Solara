//! `cargo xtask delta`: write the delta update between two install folders (release side).
//!
//! ```text
//! cargo xtask delta --old <folder> --new <folder> --out <folder> --from-version 0.1.0 --to-version 0.1.1 [--platform windows-x64]
//! ```
//!
//! The folders are what an install holds (the portable zip's folder, or the MSI's staging folder).
//! It prints the manifest's SHA-256, which the release publishes with `delta.json`, and the bytes
//! an update downloads next to the size of the new folder.

use std::path::PathBuf;

pub fn run(args: &[String]) -> Result<(), String> {
    let get = |name: &str| -> Option<String> { args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned() };
    let need = |name: &str| {
        get(name).ok_or_else(|| {
            format!("{name} is required\n\nusage: cargo xtask delta --old DIR --new DIR --out DIR --from-version X --to-version Y [--platform P]")
        })
    };
    let opts = soundcraft_delta::MakeOptions {
        app: "solara".into(),
        platform: get("--platform").unwrap_or_else(soundcraft_delta::current_platform),
        from_version: need("--from-version")?,
        to_version: need("--to-version")?,
    };
    let (old, new, out) = (PathBuf::from(need("--old")?), PathBuf::from(need("--new")?), PathBuf::from(need("--out")?));
    let r = soundcraft_delta::make(&old, &new, &out, &opts)?;
    println!("delta {} {} -> {} written to {}", opts.platform, opts.from_version, opts.to_version, out.display());
    println!("  files changed {}, unchanged {}", r.changed, r.unchanged);
    println!("  download: {} bytes (manifest {} + payloads {})", r.manifest_bytes + r.payload_bytes, r.manifest_bytes, r.payload_bytes);
    println!("  new folder: {} bytes uncompressed", r.new_folder_bytes);
    println!("  manifest sha256: {}", r.manifest_sha256);
    std::fs::write(out.join("delta.json.sha256"), format!("{}  delta.json\n", r.manifest_sha256)).map_err(|e| e.to_string())?;
    Ok(())
}
