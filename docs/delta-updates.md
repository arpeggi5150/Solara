# Delta updates

_Updated 2026-11-17._

Solara updates by downloading and installing only what changed between two versions, never the
whole installer. This page is the design, the file format and how to make and test a delta.

Update checks are still **off** (`updates_ui::UPDATE_CHECKS_ENABLED = false`), so nothing here runs
by itself: an update starts only through the control channel (`app.delta_update`, see below).
Turning checks on, and publishing deltas with releases, are separate steps.

## Mechanism

A delta is file-level with binary patches:

- **`delta.json`**, the hash manifest: the app, platform (`windows-x64`), from/to versions, and for
  each file that changed its path, its SHA-256 before and after, its new size and its payload
  (name, size, SHA-256). Unchanged files are not listed at all.
- **One payload per changed file**: a bsdiff patch from the old file to the new one, deflated; or,
  when that would not be smaller (a new file, or a rewritten one), the whole new file, deflated.
  A removed file has no payload.

bsdiff (the `bsdiff` crate, BSD-2-Clause, pure Rust) suits Solara because each release changes one
big executable a little: the patch carries only the changed bytes. Deflate is `flate2` with its
pure-Rust backend, already in the workspace.

The release publishes `delta.json` with its SHA-256 (`delta.json.sha256`), and the payloads beside it.

## The steps

1. **Release side** (`cargo xtask delta`, or `package.ps1 -DeltaFrom`): compare the old and new
   install folders and write the delta (`crates/delta/src/make.rs`).
2. **The app** (`crates/ui-egui/src/delta_update.rs`, on a background thread):
   - fetch `delta.json` and refuse it unless its SHA-256 is the published one;
   - refuse a delta for another app, platform or starting version, or when any file it changes is
     not exactly the file it starts from;
   - download each payload, refusing any whose size or SHA-256 differs;
   - build each new file in a staging folder (`%APPDATA%\Solara\Updates\<version>-<pid>`) and
     refuse it unless its SHA-256 is the published one;
   - copy `solara-updater.exe` into the staging folder, write `plan.json`, start the updater from
     there and quit fully. The app never writes into its own install folder.
3. **The standalone updater** (`apps/soundcraft-updater`, shipped as `solara-updater.exe`):
   - waits for Solara and anything else running from the install folder (by PID and by exe path)
     to exit. After 10 s it names the process and its PID and asks: **Yes** closes it, **No**
     cancels the update. It never waits forever: a process that survives three closes also ends the
     update;
   - checks every staged file again, then swaps them in all or none: each new file is copied
     beside its target and checked, each old file moves aside to `.solara-old`, each new one moves
     into place; if any move fails every file goes back. A copy installed for everyone (Program
     Files) is swapped by an administrator copy of the updater (Windows asks for permission);
   - writes how it went to `Updates\last-result.json` and starts Solara again with the session
     that was open: the new version, or the old one when the update was cancelled or refused.
     Solara shows the result at its next start until it is closed, and deletes the finished
     staging folders.

Any failed check refuses the update and leaves the installed version as it was, still working.

## Making a delta

```text
cargo xtask delta --old <old install folder> --new <new install folder> --out <folder> \
  --from-version 0.1.0 --to-version 0.1.1 [--platform windows-x64]
```

On Windows, `package.ps1 -DeltaFrom <old install folder> -DeltaFromVersion 0.1.0` builds as usual and
also writes `dist\release\solara-<v>-windows-<arch>-delta-from-<old>\`. The delta is made from the
MSI's staging folder (`solara.exe`, `solara-cli.exe`, `solara-updater.exe`, `NOTICE`), the files
both the MSI and the portable zip install.

## Updating by hand

With Solara started with `--control <port>` (or `SOLARA_CONTROL_PORT`):

```json
{"id":1,"method":"app.delta_update","params":{"manifest":"https://…/delta.json","sha256":"<from delta.json.sha256>"}}
{"id":2,"method":"app.delta_update.status","params":{}}
```

## Where releases live

Solara's own releases are the GitHub releases of `arpeggi5150/Solara`. There is no delta feed yet:
no release publishes `delta.json`, and the app has no feed URL built in. Command Center installs
Solara 0.1.0 from an AI-Vault release that packs `soundcraft.exe`, so a delta made from the MSI's
file names does not apply to that copy (the updater refuses it, since the files don't match).

## Known limits

- An MSI install updated by a delta still shows its old version in Apps & installed apps until the
  next full installer. A Windows Installer repair keeps the newer files (it only replaces missing or
  older versioned files).
- Deltas go from one version to the next; skipping versions needs a delta per pair or the full
  installer.
- The manifest is checked against the published SHA-256 over HTTPS; it is not separately signed.
