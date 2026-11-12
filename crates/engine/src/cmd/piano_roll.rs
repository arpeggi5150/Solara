//! FL-style piano roll tools: slide notes, chord stamps, strum and arpeggiate. They edit MIDI clips
//! like the other note commands; a slide is a pair of portamento (CC65) events around the note,
//! so it plays on any instrument that follows portamento and survives MIDI export.
use super::{CommandSpec, always, bad, clip_id, i64_or, str_param};

use crate::{Engine, Result, cmd};
use serde_json::{Value, json};
use soundcraft_midi::{CtrlEvent, CtrlKind, Note, Sequence};
use soundcraft_model::{ClipContent, ClipId};
use soundcraft_time::TICKS_PER_QUARTER;

/// The portamento controller.
pub const PORTAMENTO_CC: u8 = 65;
/// Most notes one stamp, strum or arpeggio may write.
const MAX_NEW_NOTES: usize = 4096;

/// Chord stamps: id, name, intervals above the root in semitones.
pub const CHORDS: &[(&str, &str, &[u8])] = &[
    ("major", "Major", &[0, 4, 7]),
    ("minor", "Minor", &[0, 3, 7]),
    ("7", "Dominant 7th", &[0, 4, 7, 10]),
    ("maj7", "Major 7th", &[0, 4, 7, 11]),
    ("min7", "Minor 7th", &[0, 3, 7, 10]),
    ("sus2", "Sus 2", &[0, 2, 7]),
    ("sus4", "Sus 4", &[0, 5, 7]),
    ("dim", "Diminished", &[0, 3, 6]),
    ("aug", "Augmented", &[0, 4, 8]),
    ("add9", "Add 9", &[0, 4, 7, 14]),
    ("power", "Power", &[0, 7, 12]),
];

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "midi.slide",
            "Slide Notes",
            [],
            None,
            "{clip, indices: [n], value?: bool (default: toggle)} — a slide glides from the note before it",
            always,
            slide
        ),
        cmd!(
            "midi.stamp",
            "Stamp Chord",
            [],
            None,
            "{clip, chord: major|minor|7|maj7|min7|sus2|sus4|dim|aug|add9|power, pitch: root, start_ticks, length_ticks?: 480, velocity?: 100}",
            always,
            stamp
        ),
        cmd!("midi.strum", "Strum", [], None, "{clip, indices: [n], ticks?: 30 between strings, down?: false}", always, strum),
        cmd!(
            "midi.arpeggiate",
            "Arpeggiate",
            [],
            None,
            "{clip, indices: [n], rate_ticks?: 240 (a sixteenth), direction?: up|down|updown, gate?: 0.1..1 (0.9)}",
            always,
            arpeggiate
        ),
    ]
}

fn seq_mut<'a>(e: &'a mut Engine, id: ClipId, cmd: &str) -> Result<&'a mut Sequence> {
    match e.session_mut().find_clip_mut(id).map(|c| &mut c.content) {
        Some(ClipContent::Midi { sequence }) => Ok(sequence),
        Some(_) => Err(bad(cmd, "not a MIDI clip")),
        None => Err(bad(cmd, "no such clip")),
    }
}

fn sequence(e: &Engine, id: ClipId, cmd: &str) -> Result<Sequence> {
    match e.session().find_clip(id).map(|(_, c)| &c.content) {
        Some(ClipContent::Midi { sequence }) => Ok(sequence.clone()),
        Some(_) => Err(bad(cmd, "not a MIDI clip")),
        None => Err(bad(cmd, "no such clip")),
    }
}

/// `indices` that name notes of `seq`, each once, ascending.
fn indices(p: &Value, seq: &Sequence) -> Vec<usize> {
    let mut v: Vec<usize> = p
        .get("indices")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_u64).filter_map(|x| usize::try_from(x).ok()).filter(|i| *i < seq.notes.len()).collect())
        .unwrap_or_default();
    v.sort_unstable();
    v.dedup();
    v
}

/// Where a slide's portamento goes on and off: just before the note starts and just before it ends.
fn slide_ticks(n: &Note) -> (i64, i64) {
    let on = n.start.saturating_sub(1).max(0);
    (on, n.start.saturating_add(n.length).saturating_sub(1).max(on.saturating_add(1)))
}

fn is_cc(c: &CtrlEvent, tick: i64, on: bool) -> bool {
    c.kind == CtrlKind::Cc(PORTAMENTO_CC) && c.tick == tick && (c.value >= 64) == on
}

/// Whether `n` is a slide note in `seq`.
pub fn is_slide(seq: &Sequence, n: &Note) -> bool {
    let (on, _) = slide_ticks(n);
    seq.ctrls.iter().any(|c| is_cc(c, on, true))
}

fn set_slide(seq: &mut Sequence, n: &Note, value: bool) {
    let (on, off) = slide_ticks(n);
    seq.ctrls.retain(|c| !is_cc(c, on, true) && !is_cc(c, off, false));
    if value {
        let ch = n.channel;
        seq.ctrls.push(CtrlEvent { tick: on, channel: ch, kind: CtrlKind::Cc(PORTAMENTO_CC), value: 127 });
        seq.ctrls.push(CtrlEvent { tick: off, channel: ch, kind: CtrlKind::Cc(PORTAMENTO_CC), value: 0 });
    }
}

fn slide(e: &mut Engine, p: &Value) -> Result<Value> {
    let id = clip_id(p, "midi.slide")?;
    let seq = sequence(e, id, "midi.slide")?;
    let idx = indices(p, &seq);
    if idx.is_empty() {
        return Err(bad("midi.slide", "select the notes to slide"));
    }
    let notes: Vec<Note> = idx.iter().filter_map(|i| seq.notes.get(*i).copied()).collect();
    // Toggle: all of them become slides unless all of them already are.
    let value = p.get("value").and_then(Value::as_bool).unwrap_or_else(|| !notes.iter().all(|n| is_slide(&seq, n)));
    let s = seq_mut(e, id, "midi.slide")?;
    for n in &notes {
        set_slide(s, n, value);
    }
    s.sort();
    Ok(json!({"slides": if value { notes.len() } else { 0 }, "value": value}))
}

/// The chord stamp named `id`.
pub fn chord(id: &str) -> Option<&'static [u8]> {
    CHORDS.iter().find(|(c, name, _)| c.eq_ignore_ascii_case(id) || name.eq_ignore_ascii_case(id)).map(|(_, _, iv)| *iv)
}

/// Where `notes` sit in `seq` after sorting, each matched once.
fn positions(seq: &Sequence, notes: &[Note]) -> Vec<usize> {
    let mut taken = vec![false; seq.notes.len()];
    let mut out = Vec::with_capacity(notes.len());
    for m in notes {
        if let Some(j) = seq.notes.iter().enumerate().position(|(j, n)| !taken.get(j).copied().unwrap_or(true) && n == m) {
            if let Some(t) = taken.get_mut(j) {
                *t = true;
            }
            out.push(j);
        }
    }
    out.sort_unstable();
    out
}

fn stamp(e: &mut Engine, p: &Value) -> Result<Value> {
    let id = clip_id(p, "midi.stamp")?;
    let name = str_param(p, "chord").unwrap_or("major");
    let iv = chord(name).ok_or_else(|| bad("midi.stamp", format!("unknown chord `{name}`")))?;
    let root = p.get("pitch").and_then(Value::as_i64).ok_or_else(|| bad("midi.stamp", "`pitch` (the root) required"))?;
    if !(0..=127).contains(&root) {
        return Err(bad("midi.stamp", "`pitch` must be 0..127"));
    }
    let start = i64_or(p, "start_ticks", 0).max(0);
    let length = i64_or(p, "length_ticks", TICKS_PER_QUARTER / 2).max(1);
    let velocity = u8::try_from(i64_or(p, "velocity", 100).clamp(1, 127)).unwrap_or(100);
    let channel = sequence(e, id, "midi.stamp")?.notes.first().map_or(0, |n| n.channel);
    let new: Vec<Note> = iv
        .iter()
        .filter_map(|i| u8::try_from(root + i64::from(*i)).ok().filter(|p| *p <= 127))
        .map(|pitch| Note { pitch, velocity, release_velocity: 64, channel, start, length })
        .collect();
    let s = seq_mut(e, id, "midi.stamp")?;
    s.notes.retain(|n| !(n.start == start && new.iter().any(|m| m.pitch == n.pitch)));
    s.notes.extend(new.iter().copied());
    s.sort();
    Ok(json!({"indices": positions(s, &new)}))
}

/// The chosen notes grouped into chords: notes that start together, low to high.
fn chords_of(seq: &Sequence, idx: &[usize]) -> Vec<Vec<Note>> {
    let mut groups: Vec<Vec<Note>> = Vec::new();
    for n in idx.iter().filter_map(|i| seq.notes.get(*i)) {
        match groups.iter_mut().find(|g| g.first().is_some_and(|f| f.start == n.start)) {
            Some(g) => g.push(*n),
            None => groups.push(vec![*n]),
        }
    }
    for g in &mut groups {
        g.sort_by_key(|n| n.pitch);
    }
    groups
}

/// Replace `old` with `new` in the clip and report where the new notes ended up.
fn replace(e: &mut Engine, cmd: &str, id: ClipId, old: &[usize], new: &[Note]) -> Result<Value> {
    if new.len() > MAX_NEW_NOTES {
        return Err(bad(cmd, format!("that would write {} notes; at most {MAX_NEW_NOTES}", new.len())));
    }
    let s = seq_mut(e, id, cmd)?;
    let mut i = 0;
    s.notes.retain(|_| {
        let keep = old.binary_search(&i).is_err();
        i += 1;
        keep
    });
    s.notes.extend(new.iter().copied());
    s.sort();
    Ok(json!({"indices": positions(s, new)}))
}

fn strum(e: &mut Engine, p: &Value) -> Result<Value> {
    let id = clip_id(p, "midi.strum")?;
    let seq = sequence(e, id, "midi.strum")?;
    let idx = indices(p, &seq);
    if idx.is_empty() {
        return Err(bad("midi.strum", "select the chords to strum"));
    }
    let gap = i64_or(p, "ticks", 30).clamp(1, TICKS_PER_QUARTER);
    let down = p.get("down").and_then(Value::as_bool).unwrap_or(false);
    let mut new = Vec::with_capacity(idx.len());
    for mut g in chords_of(&seq, &idx) {
        if down {
            g.reverse();
        }
        for (k, n) in g.iter().enumerate() {
            // Each string a little later; every note still ends where it did.
            let delay = gap.saturating_mul(i64::try_from(k).unwrap_or(0)).min(n.length.saturating_sub(1));
            new.push(Note { start: n.start.saturating_add(delay), length: n.length.saturating_sub(delay).max(1), ..*n });
        }
    }
    replace(e, "midi.strum", id, &idx, &new)
}

fn arpeggiate(e: &mut Engine, p: &Value) -> Result<Value> {
    let id = clip_id(p, "midi.arpeggiate")?;
    let seq = sequence(e, id, "midi.arpeggiate")?;
    let idx = indices(p, &seq);
    if idx.is_empty() {
        return Err(bad("midi.arpeggiate", "select the chords to arpeggiate"));
    }
    let rate = i64_or(p, "rate_ticks", TICKS_PER_QUARTER / 4).clamp(1, TICKS_PER_QUARTER * 4);
    let gate = p.get("gate").and_then(Value::as_f64).filter(|g| g.is_finite()).unwrap_or(0.9).clamp(0.1, 1.0);
    let len = ((rate as f64 * gate).round() as i64).max(1);
    let direction = str_param(p, "direction").unwrap_or("up");
    let mut new = Vec::new();
    for g in chords_of(&seq, &idx) {
        let order: Vec<Note> = match direction {
            "up" => g.clone(),
            "down" => g.iter().rev().copied().collect(),
            "updown" | "up_down" => {
                let mut v = g.clone();
                v.extend(g.iter().rev().skip(1).take(g.len().saturating_sub(2)).copied());
                v
            }
            other => return Err(bad("midi.arpeggiate", format!("unknown direction `{other}` (up, down or updown)"))),
        };
        let Some(first) = g.first() else { continue };
        let span = g.iter().map(|n| n.length).max().unwrap_or(rate).max(rate);
        let steps = usize::try_from(span / rate).unwrap_or(0).max(1);
        if new.len().saturating_add(steps) > MAX_NEW_NOTES {
            return Err(bad("midi.arpeggiate", format!("that would write more than {MAX_NEW_NOTES} notes")));
        }
        for (k, n) in order.iter().cycle().take(steps).enumerate() {
            let start = first.start.saturating_add(rate.saturating_mul(i64::try_from(k).unwrap_or(0)));
            new.push(Note { start, length: len, ..*n });
        }
    }
    replace(e, "midi.arpeggiate", id, &idx, &new)
}

#[cfg(test)]
mod tests {
    use super::*;
    use soundcraft_model::{ChannelFormat, TrackKind};

    fn keys() -> (Engine, u64) {
        let mut e = Engine::default();
        let t = e.session_mut().add_track(TrackKind::Instrument, ChannelFormat::Stereo, Some("Keys"));
        let clip = e.execute("midi.new_clip", &json!({"track": t.0})).unwrap()["clip"].as_u64().unwrap();
        (e, clip)
    }

    fn seq(e: &Engine, clip: u64) -> Sequence {
        sequence(e, ClipId(clip), "test").unwrap()
    }

    #[test]
    fn stamp_writes_the_chord_and_reports_it() {
        let (mut e, clip) = keys();
        let r = e.execute("midi.stamp", &json!({"clip": clip, "chord": "min7", "pitch": 57, "start_ticks": 960})).unwrap();
        let s = seq(&e, clip);
        let pitches: Vec<u8> = s.notes.iter().map(|n| n.pitch).collect();
        assert_eq!(pitches, [57, 60, 64, 67]);
        assert!(s.notes.iter().all(|n| n.start == 960));
        assert_eq!(r["indices"].as_array().map(Vec::len), Some(4));
        // Stamping again on the same spot does not double the notes.
        e.execute("midi.stamp", &json!({"clip": clip, "chord": "min7", "pitch": 57, "start_ticks": 960})).unwrap();
        assert_eq!(seq(&e, clip).notes.len(), 4);
        assert!(e.execute("midi.stamp", &json!({"clip": clip, "chord": "nope", "pitch": 60})).is_err());
        assert!(e.execute("midi.stamp", &json!({"clip": clip, "pitch": 300})).is_err());
        // Notes past the top of the range are left out, not wrapped.
        e.execute("midi.stamp", &json!({"clip": clip, "chord": "power", "pitch": 120, "start_ticks": 0})).unwrap();
        assert!(seq(&e, clip).notes.iter().all(|n| n.pitch <= 127));
    }

    #[test]
    fn strum_spreads_a_chord_and_keeps_its_end() {
        let (mut e, clip) = keys();
        e.execute("midi.stamp", &json!({"clip": clip, "chord": "major", "pitch": 60, "start_ticks": 0, "length_ticks": 960})).unwrap();
        e.execute("midi.strum", &json!({"clip": clip, "indices": [0, 1, 2], "ticks": 40})).unwrap();
        let s = seq(&e, clip);
        let starts: Vec<(u8, i64)> = s.notes.iter().map(|n| (n.pitch, n.start)).collect();
        assert_eq!(starts, [(60, 0), (64, 40), (67, 80)]);
        assert!(s.notes.iter().all(|n| n.start + n.length == 960));
        // A down strum starts from the top string.
        let (mut e, clip) = keys();
        e.execute("midi.stamp", &json!({"clip": clip, "chord": "major", "pitch": 60, "start_ticks": 0, "length_ticks": 960})).unwrap();
        e.execute("midi.strum", &json!({"clip": clip, "indices": [0, 1, 2], "ticks": 40, "down": true})).unwrap();
        assert_eq!(seq(&e, clip).notes.first().map(|n| n.pitch), Some(67));
    }

    #[test]
    fn arpeggiate_turns_a_chord_into_steps() {
        let (mut e, clip) = keys();
        e.execute("midi.stamp", &json!({"clip": clip, "chord": "major", "pitch": 60, "start_ticks": 0, "length_ticks": 960})).unwrap();
        e.execute("midi.arpeggiate", &json!({"clip": clip, "indices": [0, 1, 2], "rate_ticks": 240, "direction": "updown"})).unwrap();
        let s = seq(&e, clip);
        let p: Vec<(i64, u8)> = s.notes.iter().map(|n| (n.start, n.pitch)).collect();
        assert_eq!(p, [(0, 60), (240, 64), (480, 67), (720, 64)]);
        assert!(e.execute("midi.arpeggiate", &json!({"clip": clip, "indices": [0], "direction": "sideways"})).is_err());
        assert!(e.execute("midi.arpeggiate", &json!({"clip": clip, "indices": []})).is_err());
    }

    #[test]
    fn slide_toggles_portamento_around_the_note() {
        let (mut e, clip) = keys();
        e.execute("midi.note_add", &json!({"clip": clip, "pitch": 48, "start_ticks": 0, "length_ticks": 480})).unwrap();
        e.execute("midi.note_add", &json!({"clip": clip, "pitch": 55, "start_ticks": 480, "length_ticks": 480})).unwrap();
        let r = e.execute("midi.slide", &json!({"clip": clip, "indices": [1]})).unwrap();
        assert_eq!(r["value"], true);
        let s = seq(&e, clip);
        assert!(is_slide(&s, &s.notes[1]) && !is_slide(&s, &s.notes[0]));
        let cc: Vec<(i64, i32)> = s.ctrls.iter().map(|c| (c.tick, c.value)).collect();
        assert_eq!(cc, [(479, 127), (959, 0)]);
        // Again: off, and the events go.
        e.execute("midi.slide", &json!({"clip": clip, "indices": [1]})).unwrap();
        assert!(seq(&e, clip).ctrls.is_empty());
        assert!(e.execute("midi.slide", &json!({"clip": clip, "indices": [99]})).is_err());
    }
}
