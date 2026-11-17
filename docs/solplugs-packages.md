# SolPlugs packages

_Updated 2026-11-17._

Solara's plugin store: browse the SolPlugs package repository inside Solara, and install, update and
remove plugins without leaving it. Window > SolPlugs Packages… (also in the app menu of every mode).

## The package index

One JSON file, `index.json` at the root of [arpeggi5150/SolPlugs](https://github.com/arpeggi5150/SolPlugs),
read from `https://raw.githubusercontent.com/arpeggi5150/SolPlugs/main/index.json`
(`soundcraft_packages::DEFAULT_INDEX_URL`). It lists every package: id, name, version, a short
summary, vendor, a homepage (its releases page), and per platform and plugin format one zip with its
URL, size and SHA-256, the bundle at the top of the zip (`Song Sense.vst3`) and the size and SHA-256
of every file inside it. The format is documented in `crates/packages/src/index.rs`.

A package's zips stay wherever its releases are published: SolPlugs in SolPlugs-Releases, Song Sense
in Song-Sense-Releases. The index only points at them.

Write or update an entry from release zips with:

```text
cargo xtask package-entry --index index.json --id song-sense --name "Song Sense" --version 1.0.1 \
  --summary "…" --vendor "Zee Creations" --homepage https://github.com/arpeggi5150/Song-Sense-Releases \
  --zip vst3 Song-Sense-1.0.1-VST3.zip https://github.com/…/Song-Sense-1.0.1-VST3.zip \
  --zip clap Song-Sense-1.0.1-CLAP.zip https://github.com/…/Song-Sense-1.0.1-CLAP.zip
```

It hashes the zips and every file in them and checks the whole index as Solara does.

## In Solara

- The browser checks the index when it opens, and on Refresh. Each package shows its version, a
  short line, and Install, Update or Remove.
- **One package is one download.** Updating a plugin downloads only that plugin's zips.
- **Every download is hash-checked**: each zip against the index's SHA-256 as it downloads, then
  every file in it against its own SHA-256 as it is unpacked (a file the index doesn't list, or one
  outside the package's bundle, refuses the install). The files are then swapped in all or none with
  `soundcraft-delta`'s swap. If any check or step fails, the installed version stays as it was and
  the browser says why.
- New packages go to this account's plugin folder (`%LOCALAPPDATA%\Programs\Common\VST3` and
  `…\CLAP` on Windows), which needs no administrator and which Solara, Reaper and other DAWs scan. A
  package already on the computer is updated or removed where it is; one in Common Files (installed
  for everyone) is changed through `solara-updater --swap-only` as an administrator.
- A plugin a running program has loaded can still be replaced: the old file is moved aside and
  deleted the next time the browser looks.
- What Solara installed is recorded in `%APPDATA%\Solara\Packages\packages.json`. A package put there
  by something else (Solara's installer, Song Sense's own setup) is found by its bundle names; its
  version is known when every file matches the index.
- Testing: point the browser at another index with the `SOLARA_PACKAGE_INDEX` environment variable
  or `app.packages.refresh {"index": "C:\\test\\index.json"}`. Download URLs in a local index may be
  paths relative to it.

## The installer

The Windows MSI asks **Install SolPlugs?** after the scope page. Yes (the default, and what a silent
install does unless `INSTALL_SOLPLUGS=0`) installs every package in the index for that platform as
native VST3 and CLAP plugins: in Common Files for everyone, or in `%LOCALAPPDATA%\Programs\Common` for
Only for me. `package.ps1` fetches them from the index (`-SolPlugsIndex`, `-NoSolPlugs`), checking each
zip and every file against it; a mismatch stops the build, an index that can't be reached leaves
SolPlugs out with a warning.
