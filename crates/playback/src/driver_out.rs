//! Output through an audio interface's own low-latency driver (`soundcraft-asio-host`, 64-bit
//! Windows): the same mix callback, health counters and fault handling as the cpal streams, on
//! the driver's audio thread at the buffer size the driver allows.

use super::callback::{AudioState, Fault, guarded_block};
use super::{Arc, AudioConfig, Cmd, Ordering, Plugin, Receiver, Sender, Session, Shared, channel, mark_audio_thread};
use soundcraft_asio_host::{DriverEvent, Request};

/// A started driver stream and what the player keeps of it.
pub(crate) struct DriverOutput {
    pub(crate) stream: soundcraft_asio_host::Stream,
    pub(crate) tx: Sender<Cmd>,
    pub(crate) grx: Receiver<Vec<Box<dyn Plugin>>>,
    pub(crate) fault: Arc<Fault>,
    pub(crate) name: String,
    pub(crate) rate: u32,
    pub(crate) channels: u16,
    pub(crate) buffer: u32,
}

/// Open the driver `cfg` names at the chosen rate (else the session rate, else the driver's own;
/// the mix resamples) and the chosen buffer size (else the driver's preferred size).
pub(crate) fn open(session: &Arc<Session>, shared: &Arc<Shared>, want_rate: u32, want_ch: u16, cfg: &AudioConfig) -> Result<DriverOutput, String> {
    let name = match cfg.output.clone() {
        Some(n) => n,
        None => soundcraft_asio_host::drivers().into_iter().next().ok_or_else(|| "no low-latency driver is installed".to_string())?,
    };
    let caps = soundcraft_asio_host::probe(&name)?;
    let rate = cfg.sample_rate.into_iter().chain(Some(want_rate)).find(|r| caps.rates.contains(r)).unwrap_or(caps.rate);
    if rate == 0 {
        return Err(format!("`{name}` reports no sample rate"));
    }
    let request = Request { driver: name.clone(), rates: vec![rate], buffer: cfg.buffer_size, channels: want_ch.max(2) };
    let (tx, rx) = channel();
    let (gtx, grx) = channel();
    let state = AudioState::new(rx, Arc::clone(shared), Arc::clone(session), f64::from(rate), gtx);
    let fault = Arc::new(Fault::default());
    let render = renderer(state, &fault, shared, rate);
    let heard = Arc::clone(&fault);
    let notify = Box::new(move |e: DriverEvent| match e {
        // The driver must be reopened: the same "reconnect the audio device" trouble as a lost
        // cpal stream.
        DriverEvent::Reset => heard.gone.store(true, Ordering::Relaxed),
        DriverEvent::Overload => {
            heard.health.xruns.fetch_add(1, Ordering::Relaxed);
        }
    });
    let (stream, opened) = soundcraft_asio_host::open(&request, render, notify)?;
    if opened.rate != rate {
        return Err(format!("`{name}` runs at {} Hz, not the {rate} Hz it was opened for", opened.rate));
    }
    log::info!("audio device: {name} (low-latency driver) @ {rate} Hz, {} ch, {}, buffer {}", opened.channels, opened.format.label(), opened.buffer);
    if let Some(want) = cfg.buffer_size
        && want != opened.buffer
    {
        log::warn!("audio device {name} did not take the {want}-sample buffer; it runs at {}", opened.buffer);
    }
    Ok(DriverOutput { stream, tx, grx, fault, name, rate, channels: opened.channels, buffer: opened.buffer })
}

/// The driver's render callback: fill the block through the mix (a panic silences it, as in the
/// cpal streams) and count how cleanly it was fed.
fn renderer(mut state: AudioState, fault: &Arc<Fault>, shared: &Arc<Shared>, rate: u32) -> soundcraft_asio_host::Render {
    let (failed, guard) = (Arc::clone(fault), Arc::clone(shared));
    let rate = u64::from(rate.max(1));
    let (clock, mut last, mut epoch) = (std::time::Instant::now(), None::<u64>, 0u64);
    let now = move || u64::try_from(clock.elapsed().as_nanos()).unwrap_or(u64::MAX);
    Box::new(move |out: &mut [f32], channels: usize| {
        mark_audio_thread();
        let started = now();
        guarded_block(&failed, &guard, out, |d| state.fill(d, channels));
        let frames = u64::try_from(out.len() / channels.max(1)).unwrap_or(0);
        let period = frames.saturating_mul(1_000_000_000) / rate;
        let done = now();
        // A reset starts timing afresh, so the wait before it is not counted.
        let reset = failed.health_epoch();
        if reset != epoch {
            epoch = reset;
            last = None;
        }
        failed.health.note(period, last.map(|l| started.saturating_sub(l)), done.saturating_sub(started));
        last = Some(started);
    })
}
