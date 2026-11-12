# Solara 0.1.0: Step 6 items

Items found during earlier steps that must be fixed in Step 6 (Windows EXE build and report). Step 6 is not done until each is fixed and checked on R7-Win11.

## 1. Bring Solara to the front on launch
- Found: Step 4 (PR #4) verification on R7-Win11, 2026-11-11.
- What happens: the Solara window opened behind other open windows. Windows did not bring it to the front.
- Fix: on launch, bring the Solara window to the front and give it focus.

## 2. Make the mixer strip fit when the mixer is open
- Found: Step 4 (PR #4) verification on R7-Win11, 2026-11-11.
- What happens: in the NEW layout with the mixer open (M), the bottom of the fader strip is cut off at the bottom of the window. It scrolls into view.
- Fix: make the whole strip, fader included, fit in the open mixer without scrolling.

Filed at Z's request on 2026-11-11. GitHub issues are turned off in arpeggi5150/Solara, so the list is kept here, in the repository.

## After 0.1.0 (not Step 6)
- 128-sample low-latency audio on Windows. Z chose on 2026-11-11 to ship 0.1.0 at the WASAPI shared-mode default (10 ms, 480 samples at 48 kHz), which played clean on the Apogee Duet and the Focusrite. Low latency comes later.
  - Route: ASIO via the `azo` crate (https://crates.io/crates/azo, MIT; its `azo-sys` is MIT too). `azo` talks to the drivers' COM objects directly and does not use the Steinberg ASIO SDK.
  - Do NOT use the Steinberg ASIO SDK, `asio-sys`, or cpal's `asio` feature (they need the SDK).
  - Test on both of Z's interfaces (Apogee Duet USB, Focusrite USB) on R7-Win11, at 128 samples.
