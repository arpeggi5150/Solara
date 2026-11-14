//! Drivers on 64-bit Windows: listing them, asking what they offer, and running one stream on a
//! thread of its own (the driver's COM apartment).

use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use azo::driver::{Driver, Metadata, SafeHandle};
use azo::dto::ChannelId;
use azo::windows_core::GUID;

use crate::ffi;
use crate::format::{SampleFormat, buffer_sizes, choose_buffer};
use crate::{DriverCaps, MAX_CHANNELS, Notify, Opened, RATES, Render, Request};

/// How long to wait for a driver to load, answer or stop before giving up on it.
const WAIT: Duration = Duration::from_secs(10);

/// The playing driver's answers, so the settings window does not load it a second time.
static PLAYING: Mutex<Option<DriverCaps>> = Mutex::new(None);

fn playing() -> std::sync::MutexGuard<'static, Option<DriverCaps>> {
    PLAYING.lock().unwrap_or_else(PoisonError::into_inner)
}

fn registered() -> Vec<(String, GUID)> {
    let all = Metadata::enumerate().unwrap_or_default();
    let mut out: Vec<(String, GUID)> = Vec::new();
    for m in all {
        let name = m.description.to_string_lossy().trim().to_string();
        if !name.is_empty() && !out.iter().any(|(n, _)| *n == name) {
            out.push((name, m.clsid));
        }
    }
    out
}

pub(crate) fn drivers() -> Vec<String> {
    registered().into_iter().map(|(n, _)| n).collect()
}

fn clsid(name: &str) -> Result<GUID, String> {
    registered().into_iter().find(|(n, _)| n == name).map(|(_, g)| g).ok_or_else(|| format!("no driver named `{name}`"))
}

/// Run `f` on a thread of its own (a driver lives in that thread's COM apartment) and wait for
/// its answer. A panic inside the driver wrapper is caught and reported as an error.
fn on_own_thread<T: Send + 'static>(what: &str, f: impl FnOnce() -> Result<T, String> + Send + 'static) -> Result<T, String> {
    let (tx, rx) = sync_channel(1);
    std::thread::Builder::new()
        .name("solara-driver-probe".into())
        .spawn(move || {
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).unwrap_or_else(|_| Err("the driver misbehaved".to_string()));
            let _ = tx.send(r);
        })
        .map_err(|e| format!("{what}: {e}"))?;
    rx.recv_timeout(WAIT).map_err(|_| format!("{what}: the driver did not answer"))?
}

/// Load and initialise the driver in the calling thread's apartment.
fn load(name: &str, guid: &GUID) -> Result<SafeHandle, String> {
    let d = SafeHandle::new(guid).map_err(|e| format!("cannot load `{name}`: {e}"))?;
    if !d.init(None) {
        return Err(format!("`{name}` did not start (is the interface connected, or in use by another program?)"));
    }
    Ok(d)
}

fn rate_of(r: f64) -> u32 {
    if r.is_finite() && r > 0.0 && r < 10_000_000.0 { r.round() as u32 } else { 0 }
}

fn read_caps(name: &str, d: &SafeHandle) -> Result<DriverCaps, String> {
    let counts = d.channel_counts().map_err(|e| format!("`{name}`: no channel count ({e})"))?;
    let sizes = d.buffer_size().map_err(|e| format!("`{name}`: no buffer sizes ({e})"))?;
    let step = match sizes.granularity {
        None => 0,
        Some(azo::dto::Granularity::Exponential) => -1,
        Some(azo::dto::Granularity::Linear { step }) => step.get(),
    };
    let format = d.channel_info(ChannelId { input: false, index: 0 }).ok().and_then(|i| SampleFormat::from_code(i.sample_type.0));
    Ok(DriverCaps {
        name: name.to_string(),
        outputs: u16::try_from(counts.out.max(0)).unwrap_or(u16::MAX),
        inputs: u16::try_from(counts.in_.max(0)).unwrap_or(u16::MAX),
        rate: d.get_sample_rate().map(rate_of).unwrap_or(0),
        rates: RATES.iter().copied().filter(|r| d.can_sample_rate(f64::from(*r)).is_ok()).collect(),
        buffers: buffer_sizes(sizes.min, sizes.max, sizes.preferred, step),
        preferred_buffer: u32::try_from(sizes.preferred).unwrap_or(0),
        format,
    })
}

pub(crate) fn probe(name: &str) -> Result<DriverCaps, String> {
    if let Some(caps) = playing().as_ref().filter(|c| c.name == name) {
        return Ok(caps.clone());
    }
    if ffi::busy() {
        return Err(format!("`{name}` was not loaded: another driver is playing"));
    }
    let guid = clsid(name)?;
    let owned = name.to_string();
    on_own_thread("probe", move || {
        let d = load(&owned, &guid)?;
        read_caps(&owned, &d)
    })
}

/// A stream: its thread holds the driver and waits for the word to stop.
pub(crate) struct Stream {
    stop: Option<SyncSender<()>>,
    done: Option<Receiver<()>>,
}

impl Drop for Stream {
    fn drop(&mut self) {
        // Hang up: the stream thread stops the driver, takes the slot out and releases it.
        drop(self.stop.take());
        if let Some(done) = self.done.take()
            && done.recv_timeout(WAIT).is_err()
        {
            log::warn!("the audio driver did not stop in time; leaving it");
        }
    }
}

/// Set up buffers and the slot for `request` on the loaded driver, and start it.
fn start(d: &SafeHandle, caps: &DriverCaps, request: &Request, render: Render, notify: Notify) -> Result<Opened, String> {
    let name = &caps.name;
    let rate = request.rates.iter().copied().find(|r| caps.rates.contains(r)).unwrap_or(caps.rate);
    if rate != caps.rate {
        d.set_sample_rate(f64::from(rate)).map_err(|e| format!("`{name}` would not run at {rate} Hz ({e})"))?;
    }
    let rate = d.get_sample_rate().map(rate_of).unwrap_or(rate);
    let channels = request.channels.clamp(1, MAX_CHANNELS).min(caps.outputs);
    if channels == 0 {
        return Err(format!("`{name}` has no outputs"));
    }
    let format = caps.format.ok_or_else(|| format!("`{name}` uses a sample format Solara cannot write"))?;
    // Every channel opened must use the first channel's format.
    for i in 1..i32::from(channels) {
        let f = d.channel_info(ChannelId { input: false, index: i }).ok().and_then(|c| SampleFormat::from_code(c.sample_type.0));
        if f != Some(format) {
            return Err(format!("`{name}` mixes sample formats across its outputs"));
        }
    }
    let wanted = choose_buffer(request.buffer, &caps.buffers, i32::try_from(caps.preferred_buffer).unwrap_or(-1))
        .ok_or_else(|| format!("`{name}` offers no buffer size"))?;
    let mut tries = vec![wanted];
    if caps.preferred_buffer != wanted && caps.preferred_buffer > 0 {
        tries.push(caps.preferred_buffer);
    }
    let mut made = None;
    for size in tries {
        let Ok(frames) = i32::try_from(size) else { continue };
        match ffi::create_buffers(d, usize::from(channels), frames) {
            Ok(h) => {
                made = Some((size, h));
                break;
            }
            Err(e) => log::warn!("`{name}` refused a {size}-sample buffer ({e})"),
        }
    }
    let (buffer, halves) = made.ok_or_else(|| format!("`{name}` would not make its buffers"))?;
    let frames = usize::try_from(buffer).unwrap_or(0);
    let ready = d.output_ready().is_ok().then(|| ffi::ready_handle(d));
    let slot = ffi::Slot { halves, frames, format, scratch: vec![0.0; frames.saturating_mul(usize::from(channels))], render, ready };
    if ffi::install(slot, notify).is_err() {
        let _ = d.dispose_buffers();
        return Err("another driver stream is already playing".to_string());
    }
    if let Err(e) = d.start() {
        drop(ffi::uninstall());
        let _ = d.dispose_buffers();
        return Err(format!("`{name}` would not start ({e})"));
    }
    Ok(Opened { driver: name.clone(), rate, buffer, channels, format })
}

pub(crate) fn open(request: &Request, render: Render, notify: Notify) -> Result<(Stream, Opened), String> {
    if ffi::busy() {
        return Err("another driver stream is already playing".to_string());
    }
    let guid = clsid(&request.driver)?;
    let name = request.driver.clone();
    let request = request.clone();
    let (opened_tx, opened_rx) = sync_channel::<Result<Opened, String>>(1);
    let (stop_tx, stop_rx) = sync_channel::<()>(1);
    let (done_tx, done_rx) = sync_channel::<()>(1);
    std::thread::Builder::new()
        .name("solara-driver".into())
        .spawn(move || {
            let run = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let d = load(&request.driver, &guid)?;
                let caps = read_caps(&request.driver, &d)?;
                let opened = start(&d, &caps, &request, render, notify)?;
                *playing() = Some(caps);
                Ok::<_, String>((d, opened))
            }));
            let (d, opened) = match run {
                Ok(Ok(x)) => x,
                Ok(Err(e)) => {
                    let _ = opened_tx.send(Err(e));
                    return;
                }
                Err(_) => {
                    drop(ffi::uninstall());
                    let _ = opened_tx.send(Err("the driver misbehaved while opening".to_string()));
                    return;
                }
            };
            let _ = opened_tx.send(Ok(opened));
            // Play until the stream is dropped (the sender hangs up).
            let _ = stop_rx.recv();
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let _ = d.stop();
                drop(ffi::uninstall());
                let _ = d.dispose_buffers();
            }));
            *playing() = None;
            drop(d);
            let _ = done_tx.send(());
        })
        .map_err(|e| format!("cannot start the driver thread: {e}"))?;
    // On a timeout the stop sender is dropped here, so a driver that opens late stops at once.
    let opened = opened_rx.recv_timeout(WAIT).map_err(|_| format!("`{name}` did not open in time"))??;
    Ok((Stream { stop: Some(stop_tx), done: Some(done_rx) }, opened))
}
