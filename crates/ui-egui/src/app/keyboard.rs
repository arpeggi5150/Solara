//! MIDI keyboards: the enabled input ports (Setup › MIDI › MIDI Input Devices, every port when
//! none is chosen) play the record-enabled and input-monitored instruments, and while recording
//! what they play becomes a MIDI take.

use super::{Samples, SoundApp, json};
use soundcraft_playback::midi_in::{self, MidiInputs, RecordedMidi};
use std::sync::mpsc::{Receiver, sync_channel};

/// How often the ports are checked for a keyboard plugged in or out (seconds).
const RESCAN: f64 = 5.0;
/// Most messages one take keeps.
const MAX_TAKE: usize = 1_000_000;

pub(crate) struct Keyboards {
    _inputs: MidiInputs,
    recorded: Receiver<RecordedMidi>,
    /// The player the ports feed, and the ports and choice they were opened for.
    player: usize,
    ports: Vec<String>,
    chosen: Vec<String>,
    checked: f64,
}

impl SoundApp {
    /// Keep the keyboards connected to the current player, and collect what they play while recording.
    pub(super) fn watch_keyboards(&mut self) {
        let Some(player) = &self.player else {
            self.keyboards = None;
            return;
        };
        let now = self.last_frame.unwrap_or_default();
        let chosen: Vec<String> = self.engine.session().edit.flags.iter().filter_map(|f| f.strip_prefix("midi.input.")).map(str::to_string).collect();
        let id = player.id();
        let rescan = self.keyboards.as_ref().is_none_or(|k| now - k.checked >= RESCAN || now < k.checked);
        let ports = rescan.then(midi_in::input_names);
        let stale = match &mut self.keyboards {
            None => true,
            Some(k) => {
                if ports.is_some() {
                    k.checked = now;
                }
                k.player != id || k.chosen != chosen || ports.as_ref().is_some_and(|p| *p != k.ports)
            }
        };
        if stale {
            let ports = ports.unwrap_or_else(midi_in::input_names);
            let (tx, rx) = sync_channel(midi_in::QUEUE);
            let wanted = |name: &str| chosen.is_empty() || chosen.iter().any(|c| c == name);
            let inputs = MidiInputs::open(wanted, &tx);
            for f in &inputs.failed {
                log::warn!("MIDI input: {f}");
            }
            if !inputs.names.is_empty() {
                log::info!("MIDI input: {}", inputs.names.join(", "));
            }
            let recorded = player.set_midi_input(rx);
            self.keyboards = Some(Keyboards { _inputs: inputs, recorded, player: id, ports, chosen, checked: now });
        }
        let recording = self.engine.transport.recording;
        if let Some(k) = &self.keyboards {
            for m in k.recorded.try_iter() {
                if recording && self.midi_take.len() < MAX_TAKE {
                    self.midi_take.push((m.at, m.msg));
                }
            }
        }
    }

    /// Whether a take is recording a keyboard: a record-enabled MIDI, instrument or tab track.
    pub(super) fn records_keyboard(&self) -> bool {
        self.engine.session().tracks.iter().any(|t| t.mixer.record_arm && t.kind.is_midi() && !t.inactive)
    }

    /// Turn what the keyboards played into MIDI clips from `start` to `end`.
    pub(super) fn place_midi_take(&mut self, start: Samples, end: Samples) {
        // The last messages heard before Stop are still on their way from the audio thread.
        if let Some(k) = &self.keyboards {
            self.midi_take.extend(k.recorded.try_iter().map(|m| (m.at, m.msg)).take(MAX_TAKE));
        }
        let events = std::mem::take(&mut self.midi_take);
        if !self.records_keyboard() || end <= start {
            return;
        }
        let events: Vec<_> = events.iter().map(|(at, m)| json!([at, m[0], m[1], m[2]])).collect();
        match self.engine.execute("record.midi_take", &json!({"start": start, "end": end, "events": events})) {
            Ok(r) => {
                let notes = r.get("notes").and_then(serde_json::Value::as_u64).unwrap_or(0);
                self.ui.status = format!("Recorded {notes} MIDI note(s)");
            }
            Err(e) => self.ui.status = format!("MIDI take not kept: {e}"),
        }
    }
}
