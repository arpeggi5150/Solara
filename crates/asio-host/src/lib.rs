//! Solara's low-latency driver host: plays the mix through the audio interface's own ASIO driver
//! on 64-bit Windows, for buffers down to the size the driver allows (128 samples is 2.7 ms at
//! 48 kHz), where the shared Windows audio path runs in 10 ms blocks.
//!
//! It uses the `azo` crate (MIT), which talks to the drivers' COM objects directly: no Steinberg
//! SDK code, licence or bindings are involved. Drivers are found in the registry where they
//! register themselves, and are listed by the names they register.
//!
//! Each open stream owns one thread that creates the driver (in its own single-threaded COM
//! apartment), starts it, and on drop stops it and releases it. The driver calls back on its own
//! audio thread; the callback renders an interleaved `f32` block through the caller's closure
//! and writes it into the driver's buffers in the driver's sample format. Driver callbacks carry
//! no user data, so one stream at a time can be open; opening a second while one runs is an
//! error.
//!
//! This is an isolated `unsafe` crate (like `soundcraft-clap-host`): `unsafe` is denied
//! crate-wide and allowed only in the private `ffi` module. The public API is safe, returns
//! `Result`/`Option` and does not panic. A driver that itself crashes takes the process with it;
//! in-process drivers cannot be sandboxed. Elsewhere than 64-bit Windows the crate is a stub
//! that lists no drivers.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod format;

#[cfg(all(windows, target_arch = "x86_64"))]
mod driver;
#[cfg(not(all(windows, target_arch = "x86_64")))]
#[path = "stub.rs"]
mod driver;
#[cfg(all(windows, target_arch = "x86_64"))]
#[allow(unsafe_code)]
mod ffi;

pub use format::{Order, SampleFormat, buffer_sizes, choose_buffer, write_channel};

/// The name the settings list this audio system under.
pub const SYSTEM: &str = "Low-latency driver";

/// Sample rates probed on each driver (the rates interfaces run at).
pub const RATES: [u32; 6] = [44_100, 48_000, 88_200, 96_000, 176_400, 192_000];

/// Most output channels a stream opens.
pub const MAX_CHANNELS: u16 = 64;

/// What a driver offers, read when the settings window lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DriverCaps {
    /// The name the driver registered.
    pub name: String,
    pub outputs: u16,
    pub inputs: u16,
    /// The rate the driver runs at now.
    pub rate: u32,
    /// Rates from [`RATES`] the driver accepts.
    pub rates: Vec<u32>,
    /// Buffer sizes the driver offers, smallest first.
    pub buffers: Vec<u32>,
    /// The size the driver prefers (as set in its control panel).
    pub preferred_buffer: u32,
    /// The sample format of its first output, `None` when Solara cannot write it.
    pub format: Option<SampleFormat>,
}

/// Something the driver told the host while running.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriverEvent {
    /// The driver needs to be reopened (its settings changed, or the device went away).
    Reset,
    /// The driver missed a deadline or lost sync (an audible dropout).
    Overload,
}

/// What to open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    /// The driver's registered name.
    pub driver: String,
    /// The sample rate wanted, in order of preference (the first the driver accepts is used,
    /// else the rate it runs at).
    pub rates: Vec<u32>,
    /// The buffer size wanted (else the driver's preferred size).
    pub buffer: Option<u32>,
    /// Output channels wanted (at most what the driver has, at least 1).
    pub channels: u16,
}

/// What was opened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Opened {
    pub driver: String,
    pub rate: u32,
    pub buffer: u32,
    pub channels: u16,
    pub format: SampleFormat,
}

/// Fills one interleaved block: `(samples, channels)`. Runs on the driver's audio thread, so it
/// must not block or allocate.
pub type Render = Box<dyn FnMut(&mut [f32], usize) + Send>;

/// Hears [`DriverEvent`]s, on whichever thread the driver reports them.
pub type Notify = Box<dyn Fn(DriverEvent) + Send + Sync>;

/// The drivers registered on this machine, by name (none elsewhere than 64-bit Windows).
pub fn drivers() -> Vec<String> {
    driver::drivers()
}

/// What the named driver offers. The driver is loaded for a moment to ask, unless it is the one
/// playing now, whose answers were kept when it opened.
pub fn probe(name: &str) -> Result<DriverCaps, String> {
    driver::probe(name)
}

/// Open and start the named driver, rendering through `render`. The stream plays until dropped.
pub fn open(request: &Request, render: Render, notify: Notify) -> Result<(Stream, Opened), String> {
    driver::open(request, render, notify).map(|(s, o)| (Stream { _driver: s }, o))
}

/// A running driver stream; dropping it stops and releases the driver.
pub struct Stream {
    _driver: driver::Stream,
}

impl std::fmt::Debug for Stream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Stream")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn listing_and_probing_never_fail_hard() {
        let names = drivers();
        assert!(names.iter().all(|n| !n.is_empty()));
        assert!(probe("No Such Driver 1234").is_err());
        let req = Request { driver: "No Such Driver 1234".into(), rates: vec![48_000], buffer: Some(128), channels: 2 };
        assert!(open(&req, Box::new(|_, _| {}), Box::new(|_| {})).is_err());
    }
}
