# Attribution

Every asset bundled with or committed to Solara (based on SoundCraft) is listed here with its author, source and
license. `cargo xtask assets` fails CI when a tracked asset file has no row.

**Rule:** no Avid, Pro Tools, Adobe or other proprietary icons, images, sounds or presets — ever.
Assets must be original, public domain (CC0) or permissively licensed. See `AGENTS.md`.

## Icons

All UI icons (tools, transport, track buttons) are drawn procedurally in
`crates/ui-egui/src/icons.rs` by the SoundCraft contributors (MIT OR Apache-2.0). No UI icon image
files are used. The Solara app icon, wordmark and splash (`assets/app-icon/`, `assets/brand/`,
`assets/splash/`) belong to Zee Creations LLC and are not open source; they are listed under
Files below.

## Audio

The demo session's audio (drums, bass, pad) and its MIDI parts are synthesised in code in
`crates/engine/src/demo.rs` by the SoundCraft contributors (MIT OR Apache-2.0). No sample files
are bundled.

## Fonts

SoundCraft bundles no font files. It uses the default fonts shipped inside the `egui` crate
(Ubuntu-Light: Ubuntu Font Licence 1.0; Hack: MIT; Noto Emoji: OFL-1.1; emoji-icon-font: MIT)
and, at runtime, the operating system's own UI fonts when available (never redistributed).

## Bundled egui font icons

These dependency fonts are embedded by the pinned egui font definitions; no font file is added
here. The subset uses default_fonts; the parent face uses monochrome_emoji_fonts.

| Asset | Author | Source | Licence |
|---|---|---|---|
| `epaint_default_fonts/fonts/egui-icons.ttf` | John Slegers (emoji-icon-font) | https://github.com/storytold/egui/blob/6d518fc799681571963842f95b55a6b68431b15a/crates/epaint_default_fonts/fonts/egui-icons.txt; original https://github.com/jslegers/emoji-icon-font | MIT (complete copyright and permission text in NOTICE) |
| `epaint_default_fonts/fonts/emoji-icon-font.ttf` | John Slegers | https://github.com/storytold/egui/blob/6d518fc799681571963842f95b55a6b68431b15a/crates/epaint_default_fonts/fonts/emoji-icon-font-mit-license.txt; original https://github.com/jslegers/emoji-icon-font | MIT (complete copyright and permission text in NOTICE) |

## Files

| Path | Author | Source | License |
|---|---|---|---|
| `assets/app-icon/solara.svg` | Zee Creations LLC | Solara final assets (2026-11-10), vector master | Proprietary: © Zee Creations LLC, all rights reserved; not covered by the MIT / Apache-2.0 code licence (see NOTICE) |
| `assets/app-icon/solara-icon-1024x1024.png` | Zee Creations LLC | Solara final assets (2026-11-10) | Proprietary: © Zee Creations LLC, all rights reserved; not covered by the MIT / Apache-2.0 code licence (see NOTICE) |
| `assets/app-icon/solara.ico` | Zee Creations LLC | Solara final assets (2026-11-10), 6 sizes 16–256 px | Proprietary: © Zee Creations LLC, all rights reserved; not covered by the MIT / Apache-2.0 code licence (see NOTICE) |
| `assets/app-icon/solara.icns` | Zee Creations LLC | Solara final assets (2026-11-10) | Proprietary: © Zee Creations LLC, all rights reserved; not covered by the MIT / Apache-2.0 code licence (see NOTICE) |
| `assets/app-icon/hicolor/` | Zee Creations LLC | Solara final assets (2026-11-10): the 16–512 px icon PNGs and `solara.svg`, renamed to the app id | Proprietary: © Zee Creations LLC, all rights reserved; not covered by the MIT / Apache-2.0 code licence (see NOTICE) |
| `assets/brand/` | Zee Creations LLC | Solara final assets (2026-11-10): SOLARA wordmark (SVG, horizontal and stacked PNG) | Proprietary: © Zee Creations LLC, all rights reserved; not covered by the MIT / Apache-2.0 code licence (see NOTICE) |
| `assets/splash/` | Zee Creations LLC | Solara final assets (2026-11-10): start-up splash, 1280 × 720 | Proprietary: © Zee Creations LLC, all rights reserved; not covered by the MIT / Apache-2.0 code licence (see NOTICE) |
| `packaging/macos/dmg/` (all files) | @XusBadia | original (macOS DMG window background and layout, from the app icon; see `packaging/macos/dmg/README.md`) | MIT OR Apache-2.0 |
| `crates/audio-io/tests/fixtures/` | SoundCraft contributors | original test tones and silence generated with ffmpeg (sine) or by hand; commands in `crates/audio-io/tests/fixtures/README.md` | MIT OR Apache-2.0 |
