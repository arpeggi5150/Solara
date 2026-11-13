//! The FFI boundary: the only module of this crate allowed to use `unsafe`.
//!
//! Driver callbacks are plain functions with no user data, so the stream's state lives in one
//! process-wide slot. The driver's audio thread takes it with `try_lock` (never waiting); the
//! stream thread installs it before the driver starts and takes it out after the driver stops and
//! before the buffers are released, so the raw buffer pointers are only written while valid.

use std::ffi::{c_long, c_void};
use std::sync::{Mutex, PoisonError, TryLockError};

use azo::driver::{Driver, SafeHandle, UnsafeHandle};
use azo::dto::ChannelId;
use azo::sys;

use crate::format::{SampleFormat, write_channel};
use crate::{DriverEvent, Notify, Render};

/// One output channel's two buffer halves, as the driver handed them out.
#[derive(Clone, Copy)]
pub(crate) struct Halves([*mut c_void; 2]);

/// Everything the audio callback needs.
pub(crate) struct Slot {
    pub(crate) halves: Vec<Halves>,
    pub(crate) frames: usize,
    pub(crate) format: SampleFormat,
    /// Interleaved render buffer, `frames × halves.len()` long (allocated before start).
    pub(crate) scratch: Vec<f32>,
    pub(crate) render: Render,
    /// The driver, to tell it a block is ready; `None` when it does not need telling.
    pub(crate) ready: Option<ReadyHandle>,
}

/// The driver's interface, used from the audio thread only to signal that output is ready.
pub(crate) struct ReadyHandle(UnsafeHandle);

// SAFETY: the buffer pointers are owned by the driver and stay valid from `create_buffers` until
// `dispose_buffers`; the slot is taken out of `SLOT` (by the stream thread, after `stop`) before
// `dispose_buffers` runs, so no other thread can reach them afterwards. The driver interface in
// `ReadyHandle` is only used for `output_ready`, which drivers expect to be called from their own
// audio thread (that is where hosts call it), and is released on the stream thread that created
// it, after `stop`.
#[allow(unsafe_code)]
unsafe impl Send for Slot {}

static SLOT: Mutex<Option<Slot>> = Mutex::new(None);
/// Who hears driver messages. Apart from the audio slot, so a message never waits on a block
/// being rendered (or is dropped because one is).
static NOTIFY: Mutex<Option<Notify>> = Mutex::new(None);

/// Put the stream's state where the callbacks find it. Fails if a stream is already installed.
pub(crate) fn install(slot: Slot, notify: Notify) -> Result<(), Slot> {
    let mut g = SLOT.lock().unwrap_or_else(PoisonError::into_inner);
    if g.is_some() {
        return Err(slot);
    }
    *g = Some(slot);
    *NOTIFY.lock().unwrap_or_else(PoisonError::into_inner) = Some(notify);
    Ok(())
}

/// Take the stream's state out (waiting for a callback in progress to finish).
pub(crate) fn uninstall() -> Option<Slot> {
    NOTIFY.lock().unwrap_or_else(PoisonError::into_inner).take();
    SLOT.lock().unwrap_or_else(PoisonError::into_inner).take()
}

/// Whether a stream is installed now.
pub(crate) fn busy() -> bool {
    SLOT.lock().unwrap_or_else(PoisonError::into_inner).is_some()
}

/// The handle `output_ready` is sent through.
pub(crate) fn ready_handle(driver: &SafeHandle) -> ReadyHandle {
    // SAFETY: the clone (one more COM reference) is kept in the slot, which the stream thread
    // takes out and drops after `stop` and before it drops `driver`, so it never outlives the
    // driver instance or its COM apartment.
    ReadyHandle(unsafe { driver.as_unsafe() }.clone())
}

/// Ask the driver for double buffers on its first `channels` outputs, `frames` long, with this
/// module's callbacks. Returns each channel's two halves.
pub(crate) fn create_buffers(driver: &SafeHandle, channels: usize, frames: c_long) -> Result<Vec<Halves>, azo::Error> {
    let ids = (0..channels).filter_map(|i| c_long::try_from(i).ok()).map(|index| ChannelId { input: false, index });
    // SAFETY: `CALLBACKS` is a static, so it outlives the buffers; the returned pointers are only
    // dereferenced by `process`, while the slot holding them is installed (see `Slot`).
    let halves = unsafe { driver.create_buffers(ids, frames, &raw const CALLBACKS) }?;
    Ok(halves.map(Halves).collect())
}

static CALLBACKS: sys::Callbacks = sys::Callbacks { buffer_switch, sample_rate_did_change, asio_message, buffer_switch_time_info };

/// Run `f` with the installed slot, unless a callback is already holding it or none is installed.
/// A panic is caught: unwinding into the driver is not allowed.
fn with_slot(f: impl FnOnce(&mut Slot)) {
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut guard = match SLOT.try_lock() {
            Ok(g) => g,
            Err(TryLockError::Poisoned(p)) => p.into_inner(),
            Err(TryLockError::WouldBlock) => return,
        };
        if let Some(slot) = guard.as_mut() {
            f(slot);
        }
    }));
}

/// Render one block and write it into buffer half `index`.
fn process(index: c_long) {
    let Ok(half) = usize::try_from(index) else { return };
    with_slot(|s| {
        let channels = s.halves.len();
        let n = s.frames.saturating_mul(channels);
        let Some(block) = s.scratch.get_mut(..n) else { return };
        (s.render)(block, channels);
        let bytes = s.frames.saturating_mul(s.format.bytes());
        for (ch, h) in s.halves.iter().enumerate() {
            let Some(&ptr) = h.0.get(half) else { continue };
            if ptr.is_null() {
                continue;
            }
            // SAFETY: the driver allocated `frames` samples of `format` for each half of each
            // channel in `create_buffers`; they stay valid while the slot is installed, and during
            // a buffer switch the host owns half `index` exclusively.
            let out = unsafe { std::slice::from_raw_parts_mut(ptr.cast::<u8>(), bytes) };
            write_channel(block, channels, ch, s.format, out);
        }
        if let Some(r) = &s.ready {
            let _ = r.0.output_ready();
        }
    });
}

fn notify(event: DriverEvent) {
    let _ = std::panic::catch_unwind(|| {
        if let Some(n) = NOTIFY.lock().unwrap_or_else(PoisonError::into_inner).as_ref() {
            n(event);
        }
    });
}

unsafe extern "system" fn buffer_switch(index: c_long, _direct_process: sys::Bool) {
    process(index);
}

unsafe extern "system" fn buffer_switch_time_info(params: *mut sys::Time, index: c_long, _direct_process: sys::Bool) -> *mut sys::Time {
    process(index);
    params
}

unsafe extern "system" fn sample_rate_did_change(_rate: sys::SampleRate) {
    // The stream was opened for one rate; reopening picks up the new one.
    notify(DriverEvent::Reset);
}

unsafe extern "system" fn asio_message(selector: sys::MessageSelector, value: c_long, _message: *const c_void, _opt: *const f64) -> c_long {
    let yes = sys::Bool::TRUE.0;
    match selector {
        sys::MessageSelector::SELECTOR_SUPPORTED => {
            let supported = [
                sys::MessageSelector::ENGINE_VERSION,
                sys::MessageSelector::RESET_REQUEST,
                sys::MessageSelector::RESYNC_REQUEST,
                sys::MessageSelector::LATENCIES_CHANGED,
                sys::MessageSelector::SUPPORTS_TIME_INFO,
                sys::MessageSelector::OVERLOAD,
            ];
            sys::Bool::from(supported.iter().any(|s| s.0 == value)).0
        }
        sys::MessageSelector::ENGINE_VERSION => 2,
        sys::MessageSelector::RESET_REQUEST => {
            notify(DriverEvent::Reset);
            yes
        }
        sys::MessageSelector::RESYNC_REQUEST | sys::MessageSelector::OVERLOAD => {
            notify(DriverEvent::Overload);
            yes
        }
        sys::MessageSelector::LATENCIES_CHANGED | sys::MessageSelector::SUPPORTS_TIME_INFO => yes,
        _ => sys::Bool::FALSE.0,
    }
}
