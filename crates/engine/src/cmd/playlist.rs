//! The FL-style Playlist and Channel Rack: patterns placed as clips that stay one pattern, and a
//! pattern's swing and humanize. A placed pattern is an ordinary MIDI clip that shares a
//! `pattern_link` with the other places of the same pattern; editing one place edits every place
//! that held the same notes ([`sync_links`]).
use super::{CommandSpec, always, bad, clip_id, f64_or, position_param, track_param};

use crate::{Engine, Result, cmd};
use serde_json::{Value, json};
use soundcraft_model::{ClipContent, ClipId, Groove, Session, TrackId};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(query "playlist.patterns", "List Patterns", [], None, "{} — each pattern once: name, a clip of it, its track and how often it is placed", always, patterns),
        cmd!(
            "playlist.place",
            "Place Pattern",
            [],
            None,
            "{clip, at, track?} — places the pattern `clip` belongs to; the places stay one pattern",
            always,
            place
        ),
        cmd!("playlist.make_unique", "Make Pattern Unique", [], None, "{clip} — the clip becomes a pattern of its own", always, make_unique),
        cmd!("rack.groove", "Swing and Humanize", [], None, "{clip, swing?: 0..100, humanize?: 0..100}", always, groove),
    ]
}

/// The notes and groove a linked clip shares with the other places of its pattern.
fn shared(c: &soundcraft_model::Clip) -> (&ClipContent, Option<Groove>) {
    (&c.content, c.groove)
}

/// After a command: the places of a pattern that held the same notes as an edited place before
/// the edit take the edit too. Places that already differed (a trimmed or split copy) keep theirs.
pub fn sync_links(e: &mut Engine, before: &Session) {
    let s = e.session();
    if !s.tracks.iter().flat_map(|t| t.clips()).any(|c| c.pattern_link.is_some()) {
        return;
    }
    let mut updates: Vec<(ClipId, ClipContent, Option<Groove>)> = Vec::new();
    for c in s.tracks.iter().flat_map(|t| t.clips()) {
        let Some(link) = c.pattern_link else { continue };
        let Some((_, old)) = before.find_clip(c.id) else { continue };
        if old.pattern_link != Some(link) || shared(old) == shared(c) {
            continue;
        }
        for sib in s.tracks.iter().flat_map(|t| t.clips()) {
            if sib.id != c.id && sib.pattern_link == Some(link) && shared(sib) == shared(old) && !updates.iter().any(|u| u.0 == sib.id) {
                updates.push((sib.id, c.content.clone(), c.groove));
            }
        }
    }
    if updates.is_empty() {
        return;
    }
    let s = e.session_mut();
    for (id, content, groove) in updates {
        if let Some(c) = s.find_clip_mut(id) {
            c.content = content;
            c.groove = groove;
        }
    }
}

fn midi_clip(e: &Engine, cmd: &str, p: &Value) -> Result<(TrackId, soundcraft_model::Clip)> {
    let id = clip_id(p, cmd)?;
    let (t, c) = e.session().find_clip(id).ok_or_else(|| bad(cmd, "no such clip"))?;
    if !matches!(c.content, ClipContent::Midi { .. }) {
        return Err(bad(cmd, "not a pattern (MIDI clip)"));
    }
    Ok((t, c.clone()))
}

fn patterns(e: &mut Engine, _: &Value) -> Result<Value> {
    Ok(
        json!({"patterns": list(e.session()).iter().map(|p| json!({"name": p.name, "clip": p.clip, "track": p.track, "places": p.places})).collect::<Vec<_>>()}),
    )
}

/// One pattern of the session.
#[derive(Debug, Clone, PartialEq)]
pub struct Pattern {
    pub name: String,
    /// Its first place on the timeline.
    pub clip: ClipId,
    pub track: TrackId,
    pub places: usize,
}

/// Every pattern once, in track order then time: linked places count as one pattern, any other
/// MIDI clip is a pattern of its own.
pub fn list(s: &Session) -> Vec<Pattern> {
    let mut out: Vec<(Option<u64>, Pattern)> = Vec::new();
    for t in &s.tracks {
        for c in t.clips().iter().filter(|c| matches!(c.content, ClipContent::Midi { .. })) {
            if let Some(link) = c.pattern_link
                && let Some((_, p)) = out.iter_mut().find(|(l, _)| *l == Some(link))
            {
                p.places += 1;
                continue;
            }
            out.push((c.pattern_link, Pattern { name: c.name.clone(), clip: c.id, track: t.id, places: 1 }));
        }
    }
    out.into_iter().map(|(_, p)| p).collect()
}

fn place(e: &mut Engine, p: &Value) -> Result<Value> {
    let (from, clip) = midi_clip(e, "playlist.place", p)?;
    let at = position_param(e, "playlist.place", p, "at")?.ok_or_else(|| bad("playlist.place", "`at` required"))?.max(0);
    let track = track_param(e, "playlist.place", p, "track")?.unwrap_or(from);
    if !e.session().track(track).is_some_and(|t| t.kind.is_midi()) {
        return Err(bad("playlist.place", "a pattern goes on a MIDI or instrument track"));
    }
    let s = e.session_mut();
    let link = match clip.pattern_link {
        Some(l) => l,
        None => {
            let l = s.alloc();
            if let Some(c) = s.find_clip_mut(clip.id) {
                c.pattern_link = Some(l);
            }
            l
        }
    };
    let mut c = clip;
    c.start = 0;
    c.loop_id = None;
    c.pattern_link = Some(link);
    let length = c.length;
    let made = crate::edit::paste(s, track, at, &[c], length, false).map_err(|m| bad("playlist.place", m))?;
    let new = made.first().copied().ok_or_else(|| bad("playlist.place", "nothing was placed"))?;
    s.edit.selected_tracks = vec![track];
    s.edit.selected_clips = vec![new];
    Ok(json!({"clip": new, "track": track}))
}

fn make_unique(e: &mut Engine, p: &Value) -> Result<Value> {
    let (_, clip) = midi_clip(e, "playlist.make_unique", p)?;
    if let Some(c) = e.session_mut().find_clip_mut(clip.id) {
        c.pattern_link = None;
    }
    Ok(json!({"clip": clip.id}))
}

/// Percent (0..100) from `key`, or the current amount.
fn amount(p: &Value, key: &str, now: f32) -> Result<f32> {
    let v = f64_or(p, key, f64::from(now) * 100.0);
    if !v.is_finite() {
        return Err(bad("rack.groove", format!("`{key}` must be a number")));
    }
    Ok((v.clamp(0.0, 100.0) / 100.0) as f32)
}

fn groove(e: &mut Engine, p: &Value) -> Result<Value> {
    let (track, clip) = midi_clip(e, "rack.groove", p)?;
    let now = clip.groove.unwrap_or_default();
    let g = Groove { swing: amount(p, "swing", now.swing)?, humanize: amount(p, "humanize", now.humanize)? };
    let value = (!g.is_straight()).then_some(g);
    // The loops of a looped pattern swing with it.
    let mut targets = vec![clip.id];
    if let Some(lid) = clip.loop_id {
        targets.extend(e.session().track(track).into_iter().flat_map(|t| t.clips()).filter(|c| c.loop_id == Some(lid)).map(|c| c.id));
    }
    let s = e.session_mut();
    for id in targets {
        if let Some(c) = s.find_clip_mut(id) {
            c.groove = value;
        }
    }
    Ok(json!({"clip": clip.id, "swing": f64::from(g.swing) * 100.0, "humanize": f64::from(g.humanize) * 100.0}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use soundcraft_midi::Note;

    fn drums() -> (Engine, TrackId, ClipId) {
        let mut e = Engine::default();
        e.execute("session.new", &json!({"template": "drums"})).unwrap();
        let t = e.session().tracks[0].id;
        let c = e.execute("pattern.new", &json!({"track": t.0, "at": 0})).unwrap()["clip"].as_u64().unwrap();
        (e, t, ClipId(c))
    }

    fn notes(e: &Engine, clip: ClipId) -> Vec<Note> {
        match e.session().find_clip(clip).map(|(_, c)| &c.content) {
            Some(ClipContent::Midi { sequence }) => sequence.notes.clone(),
            _ => Vec::new(),
        }
    }

    #[test]
    fn a_placed_pattern_plays_its_notes_and_stays_one_pattern() {
        let (mut e, t, a) = drums();
        e.execute("pattern.step", &json!({"clip": a.0, "pitch": 36, "step": 0})).unwrap();
        let bar = e.session().find_clip(a).unwrap().1.length;
        let b = ClipId(e.execute("playlist.place", &json!({"clip": a.0, "at": bar * 2})).unwrap()["clip"].as_u64().unwrap());
        assert_eq!(notes(&e, b), notes(&e, a));
        assert_eq!(list(e.session()).len(), 1, "two places, one pattern");
        assert_eq!(list(e.session())[0].places, 2);
        // A step set on either place reaches the other.
        e.execute("pattern.step", &json!({"clip": b.0, "pitch": 38, "step": 4})).unwrap();
        assert_eq!(notes(&e, a), notes(&e, b));
        assert_eq!(notes(&e, a).len(), 2);
        // Undo takes the step back from both in one go.
        e.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(notes(&e, a).len(), 1);
        assert_eq!(notes(&e, b).len(), 1);
        // Moving a place moves only it: the song changes, the pattern doesn't.
        e.execute("edit.move_clips", &json!({"clips": [b.0], "to": bar})).unwrap();
        assert_eq!(e.session().find_clip(b).map(|(_, c)| c.start), Some(bar));
        assert_eq!(e.session().find_clip(a).map(|(_, c)| c.start), Some(0));
        assert_eq!(notes(&e, a), notes(&e, b));
        assert_eq!(e.session().track(t).unwrap().clips().len(), 2);
    }

    #[test]
    fn a_unique_place_no_longer_follows() {
        let (mut e, _, a) = drums();
        let b = ClipId(e.execute("playlist.place", &json!({"clip": a.0, "at": 960_000})).unwrap()["clip"].as_u64().unwrap());
        e.execute("playlist.make_unique", &json!({"clip": b.0})).unwrap();
        e.execute("pattern.step", &json!({"clip": a.0, "pitch": 36, "step": 0})).unwrap();
        assert!(notes(&e, b).is_empty());
        assert_eq!(list(e.session()).len(), 2);
    }

    #[test]
    fn groove_is_stored_on_the_pattern_and_its_places() {
        let (mut e, _, a) = drums();
        let b = ClipId(e.execute("playlist.place", &json!({"clip": a.0, "at": 960_000})).unwrap()["clip"].as_u64().unwrap());
        let r = e.execute("rack.groove", &json!({"clip": a.0, "swing": 60})).unwrap();
        assert_eq!(r["swing"].as_f64().map(f64::round), Some(60.0));
        for c in [a, b] {
            let g = e.session().find_clip(c).unwrap().1.groove.unwrap();
            assert!((g.swing - 0.6).abs() < 1e-6 && g.humanize == 0.0);
        }
        e.execute("rack.groove", &json!({"clip": a.0, "swing": 0})).unwrap();
        assert_eq!(e.session().find_clip(a).unwrap().1.groove, None, "straight again stores nothing");
        assert!(e.execute("rack.groove", &json!({"clip": a.0, "swing": "x"})).is_ok(), "non-numbers keep the amount");
        assert!(e.execute("rack.groove", &json!({"clip": 999_999})).is_err());
    }

    #[test]
    fn a_pattern_only_goes_on_midi_tracks() {
        let (mut e, _, a) = drums();
        let audio = e.session_mut().add_track(soundcraft_model::TrackKind::Audio, soundcraft_model::ChannelFormat::Mono, Some("Vox"));
        assert!(e.execute("playlist.place", &json!({"clip": a.0, "at": 0, "track": audio.0})).is_err());
        assert!(e.execute("playlist.place", &json!({"clip": a.0})).is_err(), "`at` required");
    }
}
