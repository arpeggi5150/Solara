//! Patterns: one-bar MIDI clips edited as a step grid (the NEW layout's step sequencer). A
//! pattern is an ordinary MIDI clip ([`ClipContent::Midi`]), so it plays, loops, copies and saves
//! like any other clip; a step is a sixteenth-note hit at a grid position.
use super::{CommandSpec, always, bad, clip_id, has_tracks, position_param, track_param};

use crate::{Engine, Result, cmd};
use serde_json::{Value, json};
use soundcraft_midi::Note;
use soundcraft_model::{ClipContent, ClipId, Session, TrackId};
use soundcraft_time::{GridValue, NoteValue, Range, Samples, TICKS_PER_QUARTER};

/// Ticks per step: a sixteenth note.
pub const STEP_TICKS: i64 = TICKS_PER_QUARTER / 4;
/// Steps in a new pattern (one bar of 4/4).
pub const DEFAULT_STEPS: i64 = 16;
/// At most this many steps per pattern, so a long clip cannot ask for an unbounded grid.
pub const MAX_STEPS: i64 = 256;
/// Velocity of a step set without one.
pub const DEFAULT_VELOCITY: u8 = 100;

/// The built-in drum sounds, as (MIDI pitch, name), on the Drum Synth's note map.
pub const SOUNDS: [(u8, &str); 10] = [
    (36, "Kick"),
    (38, "Snare"),
    (39, "Clap"),
    (42, "Closed Hat"),
    (46, "Open Hat"),
    (45, "Low Tom"),
    (47, "Mid Tom"),
    (50, "High Tom"),
    (49, "Crash"),
    (51, "Ride"),
];

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "pattern.new",
            "New Pattern",
            [],
            None,
            "{track, at?: position (default: the bar at the insertion point), sound?: pitch to put on step 1} — a one-bar MIDI clip, selected so Play loops it",
            has_tracks,
            new_pattern
        ),
        cmd!(
            "pattern.step",
            "Set Step",
            [],
            None,
            "{clip | track, pitch: 0..127, step: 0.., velocity?: 0..127} — without velocity the step toggles; 0 clears it. With a track instead of a clip, its first MIDI clip, made on bar 1 if it has none. The pattern is selected so Play loops it",
            always,
            step
        ),
        cmd!(
            "pattern.add_sound",
            "Add Sound to Track",
            [],
            None,
            "{pitch, track?, at?: position} — the pattern under `at` (or the track's first) gets the sound on step 1, made if needed; without a track a new Drums track is made",
            always,
            add_sound
        ),
        cmd!(
            "pattern.copy",
            "Copy Pattern",
            [],
            None,
            "{clip, at: position, track?: id|name (default: the clip's own)} — an independent copy of the clip",
            always,
            copy
        ),
    ]
}

/// The name of a built-in sound, if `pitch` is one.
pub fn sound_name(pitch: u8) -> Option<&'static str> {
    SOUNDS.iter().find(|(p, _)| *p == pitch).map(|(_, n)| *n)
}

/// The clip's length in steps, at least 1 and at most [`MAX_STEPS`].
pub fn steps_in(s: &Session, start: Samples, length: Samples) -> i64 {
    let a = s.tempo.samples_to_ticks(start, s.sample_rate);
    let b = s.tempo.samples_to_ticks(start.saturating_add(length), s.sample_rate);
    (b.saturating_sub(a) / STEP_TICKS).clamp(1, MAX_STEPS)
}

/// The bar that holds `at`.
fn bar_at(s: &Session, at: Samples) -> Range {
    let bar = GridValue::Note { value: NoteValue::Bar, dotted: false, triplet: false };
    let start = bar.line_at_or_before(at.max(0), s.sample_rate, &s.tempo, s.frame_rate);
    Range::new(start, bar.next_line(start, s.sample_rate, &s.tempo, s.frame_rate))
}

/// Select the pattern's range on its track, so Play loops it while Loop Playback is on.
fn select(s: &mut Session, track: TrackId, clip: ClipId) {
    if let Some((_, c)) = s.find_clip(clip) {
        let r = c.range();
        s.edit.selection = r;
        s.edit.selected_tracks = vec![track];
        s.edit.selected_clips = vec![clip];
    }
}

/// A MIDI or instrument track, or an error naming the command.
fn midi_track(e: &Engine, cmd_id: &str, p: &Value) -> Result<TrackId> {
    let t = track_param(e, cmd_id, p, "track")?.ok_or_else(|| bad(cmd_id, "`track` required"))?;
    if !e.session().track(t).is_some_and(|x| x.kind.is_midi()) {
        return Err(bad(cmd_id, "patterns go on MIDI or instrument tracks"));
    }
    Ok(t)
}

fn make_pattern(e: &mut Engine, cmd_id: &str, track: TrackId, at: Samples) -> Result<ClipId> {
    let r = bar_at(e.session(), at);
    if r.is_empty() {
        return Err(bad(cmd_id, "no bar at that position"));
    }
    let s = e.session_mut();
    let id = s.new_clip_id();
    let n = s.tracks.iter().flat_map(|t| t.clips()).filter(|c| c.name.starts_with("Pattern ")).count().saturating_add(1);
    crate::edit::place_clip(
        s,
        track,
        soundcraft_model::Clip::midi(id, format!("Pattern {n}"), r.start, r.len(), soundcraft_midi::Sequence::default()),
    );
    Ok(id)
}

fn new_pattern(e: &mut Engine, p: &Value) -> Result<Value> {
    let track = midi_track(e, "pattern.new", p)?;
    let at = position_param(e, "pattern.new", p, "at")?.unwrap_or(e.session().edit.selection.start);
    let id = make_pattern(e, "pattern.new", track, at)?;
    if let Some(pitch) = p.get("sound").and_then(Value::as_i64) {
        set_step(e, "pattern.new", id, pitch, 0, Some(i64::from(DEFAULT_VELOCITY)))?;
    }
    select(e.session_mut(), track, id);
    Ok(json!({"clip": id}))
}

/// Set, clear (`velocity` 0) or toggle (`velocity` None) one step. Returns the step's velocity
/// after the change, 0 when it is clear.
fn set_step(e: &mut Engine, cmd_id: &str, clip: ClipId, pitch: i64, step: i64, velocity: Option<i64>) -> Result<u8> {
    let pitch = u8::try_from(pitch).ok().filter(|p| *p <= 127).ok_or_else(|| bad(cmd_id, "`pitch` must be 0..127"))?;
    let (start, length) = e.session().find_clip(clip).map(|(_, c)| (c.start, c.length)).ok_or_else(|| bad(cmd_id, "no such clip"))?;
    let steps = steps_in(e.session(), start, length);
    if !(0..steps).contains(&step) {
        return Err(bad(cmd_id, format!("`step` must be 0..{}", steps - 1)));
    }
    let velocity = velocity.map(|v| u8::try_from(v.clamp(0, 127)).unwrap_or(0));
    let tick = step.saturating_mul(STEP_TICKS);
    let found =
        |sequence: &soundcraft_midi::Sequence| sequence.notes.iter().position(|n| n.pitch == pitch && n.start >= tick && n.start < tick + STEP_TICKS);
    let target = match (e.session().find_clip(clip).map(|(_, c)| &c.content), velocity) {
        (Some(ClipContent::Midi { .. }), Some(v)) => v,
        (Some(ClipContent::Midi { sequence }), None) => {
            if found(sequence).is_some() {
                0
            } else {
                DEFAULT_VELOCITY
            }
        }
        _ => return Err(bad(cmd_id, "not a MIDI clip")),
    };
    // A looped pattern's loops are copies of it: the step changes in each loop it reaches.
    let s = e.session();
    let mut targets = vec![clip];
    if let Some((track, lid)) = s.find_clip(clip).and_then(|(t, c)| c.loop_id.map(|l| (t, l))) {
        let rate = s.sample_rate;
        targets.extend(s.track(track).into_iter().flat_map(|t| t.clips()).filter(|c| c.id != clip && c.loop_id == Some(lid)).filter_map(|c| {
            let ticks = s.tempo.samples_to_ticks(c.end(), rate).saturating_sub(s.tempo.samples_to_ticks(c.start, rate));
            (tick < ticks).then_some(c.id)
        }));
    }
    for id in targets {
        if let Some(ClipContent::Midi { sequence }) = e.session_mut().find_clip_mut(id).map(|c| &mut c.content) {
            if let Some(i) = found(sequence) {
                sequence.notes.remove(i);
            }
            if target > 0 {
                sequence.notes.push(Note { pitch, velocity: target, release_velocity: 64, channel: 9, start: tick, length: STEP_TICKS });
            }
            sequence.sort();
        }
    }
    Ok(target)
}

fn step(e: &mut Engine, p: &Value) -> Result<Value> {
    let pitch = p.get("pitch").and_then(Value::as_i64).ok_or_else(|| bad("pattern.step", "`pitch` required"))?;
    let step = p.get("step").and_then(Value::as_i64).ok_or_else(|| bad("pattern.step", "`step` required"))?;
    let velocity = p.get("velocity").and_then(Value::as_i64);
    let clip = match p.get("clip") {
        Some(_) => clip_id(p, "pattern.step")?,
        // No clip: the track's first MIDI clip, or a new pattern on the track's first bar.
        None => {
            let track = midi_track(e, "pattern.step", p)?;
            let first = e.session().track(track).and_then(|t| t.clips().iter().find(|c| matches!(c.content, ClipContent::Midi { .. })).map(|c| c.id));
            match first {
                Some(id) => id,
                None => make_pattern(e, "pattern.step", track, 0)?,
            }
        }
    };
    let v = set_step(e, "pattern.step", clip, pitch, step, velocity)?;
    // The pattern being stepped is the one Play loops.
    if let Some(track) = e.session().find_clip(clip).map(|(t, _)| t) {
        select(e.session_mut(), track, clip);
    }
    Ok(json!({"clip": clip, "pitch": pitch, "step": step, "velocity": v}))
}

fn add_sound(e: &mut Engine, p: &Value) -> Result<Value> {
    let pitch = p.get("pitch").and_then(Value::as_i64).ok_or_else(|| bad("pattern.add_sound", "`pitch` required"))?;
    if !(0..=127).contains(&pitch) {
        return Err(bad("pattern.add_sound", "`pitch` must be 0..127"));
    }
    let at = position_param(e, "pattern.add_sound", p, "at")?;
    let track = match track_param(e, "pattern.add_sound", p, "track")? {
        Some(_) => midi_track(e, "pattern.add_sound", p)?,
        None => {
            let r = e.execute("track.new", &json!({"kind": "instrument", "name": "Drums", "instrument": "drum_synth"}))?;
            let id = r.get("tracks").and_then(|t| t.get(0)).and_then(Value::as_u64).or_else(|| r.get("track").and_then(Value::as_u64));
            id.map(TrackId).or_else(|| e.session().tracks.last().map(|t| t.id)).ok_or_else(|| bad("pattern.add_sound", "no track"))?
        }
    };
    let s = e.session();
    let clips: Vec<(ClipId, Samples, Samples)> = s
        .track(track)
        .map(|t| t.clips().iter().filter(|c| matches!(c.content, ClipContent::Midi { .. })).map(|c| (c.id, c.start, c.end())).collect())
        .unwrap_or_default();
    let under = at.and_then(|a| clips.iter().find(|(_, s0, e0)| (*s0..*e0).contains(&a))).or(if at.is_none() { clips.first() } else { None });
    let clip = match under {
        Some((id, _, _)) => *id,
        None => make_pattern(e, "pattern.add_sound", track, at.unwrap_or(0))?,
    };
    // The sound sounds at once: step 1, unless the row already has steps.
    let has_row = matches!(e.session().find_clip(clip).map(|(_, c)| &c.content), Some(ClipContent::Midi { sequence }) if sequence.notes.iter().any(|n| i64::from(n.pitch) == pitch));
    if !has_row {
        set_step(e, "pattern.add_sound", clip, pitch, 0, Some(i64::from(DEFAULT_VELOCITY)))?;
    }
    select(e.session_mut(), track, clip);
    Ok(json!({"clip": clip, "track": track}))
}

fn copy(e: &mut Engine, p: &Value) -> Result<Value> {
    let id = clip_id(p, "pattern.copy")?;
    let at = position_param(e, "pattern.copy", p, "at")?.ok_or_else(|| bad("pattern.copy", "`at` required"))?.max(0);
    let (from, clip) = e.session().find_clip(id).map(|(t, c)| (t, c.clone())).ok_or_else(|| bad("pattern.copy", "no such clip"))?;
    let track = track_param(e, "pattern.copy", p, "track")?.unwrap_or(from);
    let kind_ok = e.session().track(track).is_some_and(|t| {
        t.kind == e.session().track(from).map_or(t.kind, |f| f.kind) || (t.kind.is_midi() && matches!(clip.content, ClipContent::Midi { .. }))
    });
    if !kind_ok {
        return Err(bad("pattern.copy", "that track cannot hold this clip"));
    }
    let s = e.session_mut();
    let mut c = clip;
    c.start = 0;
    c.loop_id = None;
    let length = c.length;
    let made = crate::edit::paste(s, track, at, &[c], length, false).map_err(|m| bad("pattern.copy", m))?;
    let new = made.first().copied().ok_or_else(|| bad("pattern.copy", "nothing was placed"))?;
    select(s, track, new);
    Ok(json!({"clip": new, "track": track}))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drums() -> (Engine, TrackId) {
        let mut e = Engine::default();
        e.execute("session.new", &json!({"template": "drums"})).unwrap();
        let t = e.session().tracks[0].id;
        (e, t)
    }

    fn notes(e: &Engine, clip: ClipId) -> Vec<Note> {
        match e.session().find_clip(clip).map(|(_, c)| &c.content) {
            Some(ClipContent::Midi { sequence }) => sequence.notes.clone(),
            _ => Vec::new(),
        }
    }

    #[test]
    fn a_new_pattern_is_one_bar_of_sixteen_steps_selected_for_looping() {
        let (mut e, t) = drums();
        let clip = ClipId(e.execute("pattern.new", &json!({"track": t.0})).unwrap()["clip"].as_u64().unwrap());
        let (_, c) = e.session().find_clip(clip).unwrap();
        assert_eq!(steps_in(e.session(), c.start, c.length), DEFAULT_STEPS);
        assert_eq!(c.name, "Pattern 1");
        assert_eq!(e.session().edit.selection, c.range(), "Play loops the pattern");
        assert_eq!(e.session().edit.selected_tracks, vec![t]);
    }

    #[test]
    fn a_step_toggles_sets_velocity_and_clears() {
        let (mut e, t) = drums();
        let clip = ClipId(e.execute("pattern.new", &json!({"track": t.0})).unwrap()["clip"].as_u64().unwrap());
        assert_eq!(e.execute("pattern.step", &json!({"clip": clip.0, "pitch": 36, "step": 4})).unwrap()["velocity"], 100);
        let n = notes(&e, clip);
        assert_eq!((n.len(), n[0].start, n[0].length, n[0].pitch), (1, 4 * STEP_TICKS, STEP_TICKS, 36));
        e.execute("pattern.step", &json!({"clip": clip.0, "pitch": 36, "step": 4, "velocity": 40})).unwrap();
        assert_eq!(notes(&e, clip).iter().map(|n| n.velocity).collect::<Vec<_>>(), [40], "setting a velocity replaces the hit");
        assert_eq!(e.execute("pattern.step", &json!({"clip": clip.0, "pitch": 36, "step": 4})).unwrap()["velocity"], 0);
        assert!(notes(&e, clip).is_empty());
        // Undo brings the hit back: steps are ordinary undoable edits.
        e.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(notes(&e, clip).len(), 1);
    }

    #[test]
    fn stepping_an_empty_track_makes_its_pattern_in_one_undo_and_selects_it_for_looping() {
        let (mut e, t) = drums();
        assert!(e.execute("pattern.step", &json!({"track": t.0, "pitch": 36, "step": 99})).is_err());
        assert!(e.session().tracks[0].clips().is_empty(), "a refused step leaves no pattern behind");
        let r = e.execute("pattern.step", &json!({"track": t.0, "pitch": 36, "step": 0})).unwrap();
        let clip = ClipId(r["clip"].as_u64().unwrap());
        assert_eq!(steps_in(e.session(), 0, e.session().tracks[0].clips()[0].length), DEFAULT_STEPS);
        assert_eq!(e.session().edit.selected_clips, [clip]);
        assert_eq!(e.session().edit.selection, e.session().tracks[0].clips()[0].range());
        // The next step lands in the same pattern.
        e.execute("pattern.step", &json!({"track": t.0, "pitch": 42, "step": 2})).unwrap();
        assert_eq!((e.session().tracks[0].clips().len(), notes(&e, clip).len()), (1, 2));
        e.execute("edit.undo", &json!({})).unwrap();
        e.execute("edit.undo", &json!({})).unwrap();
        assert!(e.session().tracks[0].clips().is_empty(), "the first step and its pattern are one undo");
    }

    #[test]
    fn a_step_in_a_looped_pattern_sounds_in_every_loop() {
        let (mut e, t) = drums();
        let clip = ClipId(e.execute("pattern.new", &json!({"track": t.0})).unwrap()["clip"].as_u64().unwrap());
        let bar = e.session().find_clip(clip).unwrap().1.length;
        // Two and a half loops: the cut-short last one only gets steps that fit in it.
        e.execute("clip.loop", &json!({"clips": [clip.0], "length": bar * 5 / 2})).unwrap();
        e.execute("pattern.step", &json!({"clip": clip.0, "pitch": 36, "step": 2})).unwrap();
        e.execute("pattern.step", &json!({"clip": clip.0, "pitch": 42, "step": 12})).unwrap();
        let clips: Vec<ClipId> = e.session().tracks[0].clips().iter().map(|c| c.id).collect();
        let hits: Vec<usize> = clips.iter().map(|c| notes(&e, *c).len()).collect();
        assert_eq!(hits, [2, 2, 1]);
    }

    #[test]
    fn hostile_steps_and_pitches_are_refused() {
        let (mut e, t) = drums();
        let clip = ClipId(e.execute("pattern.new", &json!({"track": t.0})).unwrap()["clip"].as_u64().unwrap());
        for p in [
            json!({"clip": clip.0, "pitch": 36, "step": 16}),
            json!({"clip": clip.0, "pitch": 36, "step": -1}),
            json!({"clip": clip.0, "pitch": 300, "step": 0}),
            json!({"clip": clip.0, "pitch": 36, "step": i64::MAX}),
        ] {
            assert!(e.execute("pattern.step", &p).is_err(), "{p}");
        }
        assert!(notes(&e, clip).is_empty());
    }

    #[test]
    fn a_sound_dropped_on_a_track_makes_a_pattern_and_sounds_on_step_one() {
        let (mut e, t) = drums();
        let r = e.execute("pattern.add_sound", &json!({"pitch": 38, "track": t.0})).unwrap();
        let clip = ClipId(r["clip"].as_u64().unwrap());
        assert_eq!(notes(&e, clip).iter().map(|n| (n.pitch, n.start)).collect::<Vec<_>>(), [(38, 0)]);
        // A second sound joins the same pattern.
        let again = e.execute("pattern.add_sound", &json!({"pitch": 42, "track": t.0})).unwrap();
        assert_eq!(again["clip"].as_u64(), Some(clip.0));
        assert_eq!(notes(&e, clip).len(), 2);
    }

    #[test]
    fn a_sound_dropped_on_empty_space_makes_a_drums_track() {
        let mut e = Engine::default();
        e.execute("session.new", &json!({"template": "empty"})).unwrap();
        e.execute("pattern.add_sound", &json!({"pitch": 36})).unwrap();
        let tracks = &e.session().tracks;
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0].name, "Drums");
        assert_eq!(tracks[0].clips().len(), 1);
    }

    #[test]
    fn a_sound_cannot_go_on_an_audio_track() {
        let mut e = Engine::default();
        e.execute("track.new", &json!({"name": "Vox"})).unwrap();
        assert!(e.execute("pattern.add_sound", &json!({"pitch": 36, "track": "Vox"})).is_err());
    }

    #[test]
    fn a_copied_pattern_is_independent() {
        let (mut e, t) = drums();
        let clip = ClipId(e.execute("pattern.add_sound", &json!({"pitch": 36, "track": t.0})).unwrap()["clip"].as_u64().unwrap());
        let (start, len) = e.session().find_clip(clip).map(|(_, c)| (c.start, c.length)).unwrap();
        let copy = ClipId(e.execute("pattern.copy", &json!({"clip": clip.0, "at": start + len})).unwrap()["clip"].as_u64().unwrap());
        assert_ne!(copy, clip);
        e.execute("pattern.step", &json!({"clip": copy.0, "pitch": 38, "step": 8})).unwrap();
        assert_eq!(notes(&e, clip).len(), 1, "editing the copy leaves the original");
        assert_eq!(notes(&e, copy).len(), 2);
    }
}
