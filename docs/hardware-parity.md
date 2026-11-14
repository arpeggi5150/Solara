# Hardware parity

> **Last reviewed:** 2026-10-10 · **Last updated:** 2026-11-14 · **Change:** minor (MIDI keyboard input and recording; previously minor: ASIO output via azo at 128 samples; trivial: bounces of a bus fed by an aux compensate its latency (#246); previously minor: audio device, rate and buffer choice; ASIO feature) · **Target:** Avid Pro Tools Ultimate 2026.4.1

A DAW is judged by how it drives audio interfaces, MIDI gear, control surfaces and sync. This is
Solara's weakest dimension. Ours is measured from `crates/playback` (cpal) and the absence of
any MIDI-device or control-surface code; Pro Tools' from its documentation and Setup menus.

**Hardware: ~12 % ready** (estimated). **Performance: ~35 %** (estimated; nothing benchmarked
against Pro Tools). Remaining ≈ 110–180 h for hardware, 40–70 h for performance.

Legend: ✅ works · 🟡 partial · ❌ missing.

## Audio interfaces

| Feature | Pro Tools (macOS / Windows) | Solara macOS | Windows | Linux | FreeBSD | Web |
|---|---|---|---|---|---|---|
| Driver API | Core Audio / ASIO | 🟡 Core Audio via cpal | 🟡 WASAPI shared via cpal; ASIO drivers for output on x64 through the MIT `azo` crate, no Steinberg SDK (verified at 128 samples on an Apogee Duet and a Focusrite USB interface) | 🟡 ALSA/Pulse via cpal; no JACK | 🟡 cpal (ALSA layer) | 🟡 Web Audio |
| Choose playback device | ✅ Playback Engine | 🟡 Setup › Playback Engine (not yet verified on macOS) | ✅ Setup › Playback Engine (WASAPI; ASIO drivers as “Low-latency driver”) | 🟡 (not yet verified) | 🟡 (not yet verified) | ❌ |
| Choose input device | ✅ | 🟡 Setup › Playback Engine (not yet verified on macOS) | 🟡 WASAPI (input does not use the ASIO driver yet) | 🟡 (not yet verified) | ❌ no recording | ❌ |
| Buffer size (32–2048) | ✅ | 🟡 device buffer when the driver allows it (not yet verified); mix in 512-sample blocks | 🟡 the sizes each ASIO driver offers (128 verified clean for 60 s on two interfaces: 0 late, 0 gaps, 0 xruns); WASAPI shared runs at the system period (10 ms); the mix renders blocks the device's size, at most 512 | 🟡 (not yet verified) | 🟡 | ❌ |
| Duplex stream on one clock | ✅ | ❌ separate input/output streams | ❌ | ❌ | ❌ | ❌ |
| Hardware I/O mapping (I/O Setup to physical channels) | ✅ | 🟡 multichannel output in order; no per-channel map | 🟡 | 🟡 | 🟡 | ❌ |
| Low-latency monitoring | ✅ | ❌ (`options.low_latency_monitoring` is a stored flag) | ❌ | ❌ | ❌ | ❌ |
| Latency reporting and record offset compensation | ✅ | ❌ | ❌ | ❌ | ❌ | ❌ |
| Sample rate follows the session; resample when it can't | ✅ (switches device) | 🟡 resamples (input pitch bug fixed, #28) | 🟡 | 🟡 | 🟡 | 🟡 |
| Avid HDX / HD Native DSP, Hybrid Engine DSP mode | ✅ | ❌ (out of reach: Avid hardware) | ❌ | ❌ | ❌ | ❌ |
| Disk allocation / record to disk | ✅ | ❌ recording into RAM (PR #84) | ❌ | ❌ | ❌ | ❌ |

## MIDI

| Feature | Pro Tools | Solara (all platforms) |
|---|---|---|
| MIDI input devices (keyboards, pads) | ✅ | ✅ through `midir` (CoreMIDI, WinMM, ALSA): the chosen ports (all when none is chosen) play record-enabled and input-monitored instruments and record MIDI takes; timing per audio block |
| MIDI output to external gear | ✅ | ❌ |
| MIDI Thru, input filter | ✅ | ❌ (stored flags only) |
| MIDI Beat Clock out, MTC generate/chase | ✅ | ❌ |
| MIDI learn / controller mapping | ✅ (via surfaces and plug-in maps) | ❌ |
| Virtual MIDI keyboard | ✅ (MIDI Keyboard window) | 🟡 window exists, no live audition path |

## Control surfaces

| Feature | Pro Tools | Solara |
|---|---|---|
| EUCON (S1/S3/S4/S6, Dock, Control app) | ✅ | ❌ Avid-proprietary SDK; owner decision |
| HUI | ✅ | ❌ |
| Mackie Control Universal | ✅ (via HUI emulation and personalities) | ❌ |
| Generic MIDI controller personalities | ✅ | ❌ |

## Sync and video hardware

| Feature | Pro Tools | Solara |
|---|---|---|
| LTC / MTC chase, 9-pin machine control | ✅ (Ultimate) | ❌ |
| Word clock / video reference awareness | ✅ via interface | ❌ |
| Video output to a second display or video card | ✅ | ❌ (video window only) |
| Satellite linking of systems | ✅ | ❌ |

## Performance

| Feature | Pro Tools | Solara | Evidence |
|---|---|---|---|
| Parallel mixing across cores | ✅ | ✅ | `soundcraft-mix` renders independent strips in parallel (rayon) |
| Plugin delay compensation | ✅ | ✅ | Bounces of a bus that an aux returns onto are compensated like the main mix (#246). |
| Disk streaming of long sources | ✅ | ❌ | Whole files decoded into the `SourcePool` |
| Track count (Ultimate: 2,048 voices) | ✅ | unmeasured | No benchmark exists; add `cargo xtask bench` |
| Real-time audio thread discipline | ✅ | ✅ | No blocking/allocation in steady state, audio-thread-aware logger |
| Freeze / commit to save CPU | ✅ | 🟡 | `track.commit` renders; `track.freeze` only sets a flag (no CPU saved) |
| GPU-accelerated UI | — | ✅ (wgpu/glow via eframe) | llvmpipe X11 issue #107 |

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-11-14 | minor | MIDI keyboard input: plays armed and monitored MIDI, instrument and tab tracks live and records takes while the project plays |
| 2026-11-13 | minor | ASIO drivers for output on Windows x64 through azo (no Steinberg SDK); cpal's `asio` feature removed; 128 samples verified on two interfaces |
| 2026-10-13 | trivial | bounces of a bus fed by an aux compensate its latency (#246) |
| 2026-10-12 | minor | Audio device settings: system, output, input, sample rate and buffer size; opt-in ASIO |
| 2026-10-10 | major | Created: audit of audio interface, MIDI, control-surface, sync and performance support against Pro Tools 2026.4.1 |
