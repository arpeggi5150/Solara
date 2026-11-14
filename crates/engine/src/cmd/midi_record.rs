//! Recording from a MIDI keyboard: the messages captured while the transport recorded become a
//! MIDI clip on each record-enabled MIDI, instrument or tab track (replacing what was there, or
//! layered on top with Options › MIDI Merge).
use super::{CommandSpec, always, bad, i64_or, track_param};

use crate::{Engine, Result, cmd};
use serde_json::{Value, json};
use soundcraft_midi::{CtrlEvent, CtrlKind, Note, Sequence};
use soundcraft_model::{Clip, TrackId};
use soundcraft_time::Samples;

/// Most messages one take may hold.
const MAX_EVENTS: usize = 1_000_000;

pub fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        "record.midi_take",
        "Record MIDI Take",
        [],
        None,
        "{start, end, events: [[sample, status, data1, data2], ...], track?} — a keyboard take as a MIDI clip from `start` to `end` on `track`, else on every record-enabled MIDI, instrument or tab track",
        always,
        midi_take
    )]
}

fn midi_take(e: &mut Engine, p: &Value) -> Result<Value> {
    let id = "record.midi_take";
    let start = i64_or(p, "start", 0).max(0);
    let end = i64_or(p, "end", start);
    if end <= start {
        return Err(bad(id, "`end` must be after `start`"));
    }
    let raw = p.get("events").and_then(Value::as_array).ok_or_else(|| bad(id, "`events` required"))?;
    if raw.len() > MAX_EVENTS {
        return Err(bad(id, format!("at most {MAX_EVENTS} events")));
    }
    let mut events: Vec<(Samples, [u8; 3])> = Vec::with_capacity(raw.len());
    for ev in raw {
        let a = ev.as_array().ok_or_else(|| bad(id, "each event is [sample, status, data1, data2]"))?;
        let at = a.first().and_then(Value::as_i64).ok_or_else(|| bad(id, "an event's sample must be a number"))?;
        let byte = |i: usize| a.get(i).and_then(Value::as_u64).and_then(|v| u8::try_from(v).ok()).unwrap_or(0);
        events.push((at.clamp(start, end), [byte(1), byte(2) & 0x7F, byte(3) & 0x7F]));
    }
    events.sort_by_key(|(at, _)| *at);
    let tracks: Vec<TrackId> = match track_param(e, id, p, "track")? {
        Some(t) => vec![t],
        None => e.session().tracks.iter().filter(|t| t.kind.is_midi() && t.mixer.record_arm).map(|t| t.id).collect(),
    };
    if tracks.is_empty() {
        return Err(bad(id, "no MIDI, instrument or tab track is record-enabled"));
    }
    if let Some(t) = tracks.iter().find(|t| !e.session().track(**t).is_some_and(|t| t.kind.is_midi())) {
        return Err(bad(id, format!("track {} is not a MIDI, instrument or tab track", t.0)));
    }
    let seq = take_sequence(e, start, end, &events);
    let notes = seq.notes.len();
    let merge = e.session().edit.midi_merge;
    let mut clips = Vec::new();
    for t in tracks {
        let s = e.session_mut();
        let cid = s.new_clip_id();
        let name = s.track(t).map_or_else(|| "MIDI".to_string(), |t| format!("{} MIDI", t.name));
        let clip = Clip::midi(cid, &name, start, end - start, seq.clone());
        if merge {
            // Layered on what is there: overlapping MIDI clips all play.
            if let Some(pl) = s.track_mut(t).and_then(|t| t.playlist_mut()) {
                pl.clips.push(clip);
                pl.sort();
            }
        } else {
            crate::edit::place_clip(s, t, clip);
        }
        clips.push(cid.0);
    }
    Ok(json!({"clips": clips, "notes": notes}))
}

/// The take's messages as a sequence in ticks from `start`: note-ons paired with their note-offs
/// (a note still held at `end` ends there), controllers, pressure and pitch bend as controller
/// events.
fn take_sequence(e: &Engine, start: Samples, end: Samples, events: &[(Samples, [u8; 3])]) -> Sequence {
    let s = e.session();
    let (sr, tempo) = (s.sample_rate, &s.tempo);
    let base = tempo.samples_to_ticks(start, sr);
    let tick = |at: Samples| tempo.samples_to_ticks(at, sr).saturating_sub(base).max(0);
    let mut seq = Sequence::default();
    // Held notes: (channel, pitch) → (start tick, velocity).
    let mut held: Vec<(u8, u8, i64, u8)> = Vec::new();
    let finish = |seq: &mut Sequence, (ch, pitch, from, vel): (u8, u8, i64, u8), to: i64, release: u8| {
        seq.notes.push(Note { pitch, velocity: vel.max(1), release_velocity: release, channel: ch, start: from, length: (to - from).max(1) });
    };
    for &(at, [status, d1, d2]) in events {
        let ch = status & 0x0F;
        let t = tick(at);
        match status & 0xF0 {
            0x90 if d2 > 0 => {
                // Struck again while held: the first ends here.
                if let Some(i) = held.iter().position(|h| h.0 == ch && h.1 == d1) {
                    let h = held.swap_remove(i);
                    finish(&mut seq, h, t, 64);
                }
                held.push((ch, d1, t, d2));
            }
            0x80 | 0x90 => {
                if let Some(i) = held.iter().position(|h| h.0 == ch && h.1 == d1) {
                    let h = held.swap_remove(i);
                    finish(&mut seq, h, t, if status & 0xF0 == 0x80 { d2 } else { 64 });
                }
            }
            0xB0 => seq.ctrls.push(CtrlEvent { tick: t, channel: ch, kind: CtrlKind::Cc(d1), value: i32::from(d2) }),
            0xA0 => seq.ctrls.push(CtrlEvent { tick: t, channel: ch, kind: CtrlKind::PolyPressure(d1), value: i32::from(d2) }),
            0xD0 => seq.ctrls.push(CtrlEvent { tick: t, channel: ch, kind: CtrlKind::ChannelPressure, value: i32::from(d1) }),
            0xE0 => seq.ctrls.push(CtrlEvent { tick: t, channel: ch, kind: CtrlKind::PitchBend, value: (i32::from(d2) << 7 | i32::from(d1)) - 8192 }),
            _ => {}
        }
    }
    let last = tick(end);
    for h in held {
        finish(&mut seq, h, last, 64);
    }
    seq.sort();
    seq
}

#[cfg(test)]
mod tests {
    use super::*;
    use soundcraft_model::ClipContent;
    use soundcraft_model::{Session, TrackKind};

    #[test]
    fn a_keyboard_take_becomes_a_clip_on_the_armed_track() {
        let mut e = Engine::new(Session::default());
        e.execute("track.new", &json!({"kind": "instrument", "name": "Keys"})).unwrap();
        let keys = e.session().tracks.iter().find(|t| t.kind == TrackKind::Instrument).map(|t| t.id).unwrap();
        assert!(e.execute("record.midi_take", &json!({"start": 0, "end": 48_000, "events": []})).is_err(), "nothing armed");
        e.session_mut().track_mut(keys).unwrap().mixer.record_arm = true;
        // At 120 bpm and 48 kHz a quarter note is 24 000 samples (960 ticks).
        let sr = e.session().sample_rate.hz();
        let q = i64::from(sr) / 2;
        let events = json!([[q, 0x90, 60, 100], [q * 2, 0x80, 60, 30], [q * 2, 0x90, 64, 90], [q * 2, 0xB0, 64, 127], [q * 3, 0x90, 67, 80]]);
        let r = e.execute("record.midi_take", &json!({"start": q, "end": q * 4, "events": events})).unwrap();
        assert_eq!(r["notes"], 3);
        let s = e.session();
        let t = s.track(keys).unwrap();
        let clip = t.clips().iter().find(|c| c.start == q).unwrap();
        assert_eq!(clip.length, q * 3);
        let ClipContent::Midi { sequence } = &clip.content else { panic!("not MIDI") };
        let notes: Vec<(i64, u8, i64, u8)> = sequence.notes.iter().map(|n| (n.start, n.pitch, n.length, n.velocity)).collect();
        assert_eq!(notes, [(0, 60, 960, 100), (960, 64, 1920, 90), (1920, 67, 960, 80)], "a note held at the end stops there");
        assert_eq!(sequence.notes[0].release_velocity, 30);
        assert_eq!(sequence.ctrls.len(), 1);
        // One undo step takes the take away.
        e.execute("edit.undo", &json!({})).unwrap();
        assert!(e.session().track(keys).unwrap().clips().is_empty());
        // Hostile input is refused, not a crash.
        assert!(e.execute("record.midi_take", &json!({"start": 10, "end": 5, "events": []})).is_err());
        assert!(e.execute("record.midi_take", &json!({"start": 0, "end": 5, "events": [["x"]]})).is_err());
        assert!(e.execute("record.midi_take", &json!({"start": 0, "end": 5, "events": [[1, 999, -4, 1e9]]})).is_ok());
    }
}
