//! The Session View (Ableton-style layout): scenes, clip slots, launching and stopping clips and
//! scenes, follow actions, and warping (audio that follows the tempo).
//!
//! Scenes and slot contents are part of the document and undo like any edit. What is launched is
//! live state (`Session::launch`): it reaches playback and redraws, but is never saved, never marks
//! the session modified and is not undone.
use super::{CommandSpec, always, bad, bool_or, clip_ids_param, i64_or, str_param, track_param};

use crate::{Engine, Result, TransportRequest, cmd};
use serde_json::{Value, json};
use soundcraft_midi::Sequence;
use soundcraft_model::{Clip, ClipContent, ClipId, Follow, FollowAction, Launch, MAX_FOLLOW_PLAYS, MAX_SCENES, Scene, Session, TrackId};
use soundcraft_time::Samples;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "scene.new",
            "Insert Scene",
            [],
            None,
            "{name?, at?: index (default: after the last)}: a new empty row of clip slots",
            always,
            new_scene
        ),
        cmd!("scene.rename", "Rename Scene", [], None, "{scene: index, name}", always, rename_scene),
        cmd!(
            "scene.move",
            "Move Scene",
            [],
            None,
            "{scene: index, to: index}: the row and every track's clip in it move together",
            always,
            move_scene
        ),
        cmd!("scene.delete", "Delete Scene", [], None, "{scene: index}: the row and its clips", always, delete_scene),
        cmd!(noundo "scene.launch", "Launch Scene", [], None, "{scene: index, quantize?: bar|beat|none}: every track plays its clip in the row (tracks with an empty slot stop)", always, launch_scene),
        cmd!(noundo "launcher.stop_all", "Stop All Clips", [], None, "{}: every launched clip stops; the Arrangement plays again", always, |e, _| {
            if !e.session().launch.is_empty() {
                e.runtime_session_mut().launch.clear();
            }
            Ok(json!({}))
        }),
        cmd!(noundo "slot.launch", "Launch Clip", [], None, "{track, scene: index, quantize?: bar|beat|none}: play the clip in the slot from the next bar (from the playhead when stopped)", always, launch_slot),
        cmd!(noundo "slot.stop", "Stop Track Clips", [], None, "{track}: the track's launched clip stops", always, stop_track),
        cmd!(
            "slot.set",
            "Put Clip in Slot",
            [],
            None,
            "{track, scene: index, clip?: id (copied from the Arrangement or another slot), bars?: n (a new empty MIDI clip), source?: audio file id}",
            always,
            set_slot
        ),
        cmd!("slot.clear", "Delete Slot Clip", [], None, "{track, scene: index}", always, clear_slot),
        cmd!("slot.move", "Move Slot Clip", [], None, "{track, scene, to_track?, to_scene, copy?: false}", always, move_slot),
        cmd!(noundo "launcher.settle", "Settle Launched Clips", [], None, "{at?: position (default: the transport)}: fold the clips played so far into what is launched", always, settle),
        cmd!(
            "clip.follow",
            "Follow Action",
            [],
            None,
            "{clips?, action: again|next|random|stop|none, plays?: 1}: what a Session View clip does after it has played `plays` times",
            always,
            follow
        ),
        cmd!(
            "clip.warp",
            "Warp",
            [],
            None,
            "{clips?, value?: bool}: warped audio follows the tempo, stretched (pitch kept) to stay in time when the tempo changes",
            always,
            warp
        ),
    ]
}

/// A scene index from `key`, within the session's scenes.
fn scene_param(s: &Session, cmd: &str, p: &Value, key: &str) -> Result<usize> {
    let i = p.get(key).and_then(Value::as_u64).ok_or_else(|| bad(cmd, format!("`{key}` scene index required")))?;
    let i = usize::try_from(i).unwrap_or(usize::MAX);
    if i >= s.scenes.len() {
        return Err(bad(cmd, format!("no scene {i} (the session has {})", s.scenes.len())));
    }
    Ok(i)
}

/// The track named by `key`, which must hold clips (audio, MIDI or instrument).
fn clip_track(e: &Engine, cmd: &str, p: &Value, key: &str) -> Result<TrackId> {
    let t = track_param(e, cmd, p, key)?.ok_or_else(|| bad(cmd, format!("`{key}` required")))?;
    let ok = e.session().track(t).is_some_and(|x| x.kind.has_slots());
    if !ok {
        return Err(bad(cmd, "only audio, MIDI and instrument tracks have clip slots"));
    }
    Ok(t)
}

/// Every track has one slot per scene.
fn fit_slots(s: &mut Session) {
    let n = s.scenes.len();
    for t in &mut s.tracks {
        if t.slots.len() != n {
            t.slots.resize(n, None);
        }
    }
}

fn new_scene(e: &mut Engine, p: &Value) -> Result<Value> {
    let id = "scene.new";
    if e.session().scenes.len() >= MAX_SCENES {
        return Err(bad(id, format!("a session holds at most {MAX_SCENES} scenes")));
    }
    let len = e.session().scenes.len();
    let at = p.get("at").and_then(Value::as_u64).map_or(len, |v| usize::try_from(v).unwrap_or(usize::MAX).min(len));
    let s = e.session_mut();
    fit_slots(s);
    let name = str_param(p, "name").map_or_else(|| (s.scenes.len() + 1).to_string(), |n| n.chars().take(soundcraft_model::MAX_NAME_CHARS).collect());
    let sid = s.alloc();
    s.scenes.insert(at, Scene { id: sid, name });
    for t in &mut s.tracks {
        t.slots.insert(at.min(t.slots.len()), None);
    }
    for l in &mut s.launch {
        if l.slot >= at {
            l.slot = l.slot.saturating_add(1);
        }
    }
    Ok(json!({"scene": at, "id": sid}))
}

fn rename_scene(e: &mut Engine, p: &Value) -> Result<Value> {
    let i = scene_param(e.session(), "scene.rename", p, "scene")?;
    let name = str_param(p, "name").ok_or_else(|| bad("scene.rename", "`name` required"))?;
    let name: String = name.chars().take(soundcraft_model::MAX_NAME_CHARS).collect();
    if let Some(sc) = e.session_mut().scenes.get_mut(i) {
        sc.name = name;
    }
    Ok(json!({"scene": i}))
}

fn move_scene(e: &mut Engine, p: &Value) -> Result<Value> {
    let from = scene_param(e.session(), "scene.move", p, "scene")?;
    let to = scene_param(e.session(), "scene.move", p, "to")?;
    if from == to {
        return Ok(json!({"scene": to}));
    }
    let s = e.session_mut();
    fit_slots(s);
    let sc = s.scenes.remove(from);
    s.scenes.insert(to, sc);
    for t in &mut s.tracks {
        let c = t.slots.remove(from);
        t.slots.insert(to, c);
    }
    let moved = |i: usize| {
        if i == from {
            to
        } else if from < to && i > from && i <= to {
            i - 1
        } else if to < from && i >= to && i < from {
            i + 1
        } else {
            i
        }
    };
    for l in &mut s.launch {
        l.slot = moved(l.slot);
    }
    Ok(json!({"scene": to}))
}

fn delete_scene(e: &mut Engine, p: &Value) -> Result<Value> {
    let i = scene_param(e.session(), "scene.delete", p, "scene")?;
    let s = e.session_mut();
    fit_slots(s);
    s.scenes.remove(i);
    for t in &mut s.tracks {
        t.slots.remove(i);
    }
    s.launch.retain(|l| l.slot != i);
    for l in &mut s.launch {
        if l.slot > i {
            l.slot -= 1;
        }
    }
    Ok(json!({"scenes": s.scenes.len()}))
}

/// Where a launch starts: the next bar (or beat) after the transport while it plays, else the
/// transport position, and then the transport starts there.
fn launch_time(e: &mut Engine, p: &Value) -> Samples {
    let pos = e.transport.position.max(0);
    if !e.transport.playing {
        e.transport_requests.push(TransportRequest::PlayFrom(pos));
        return pos;
    }
    let s = e.session();
    let sr = s.sample_rate;
    let q = str_param(p, "quantize").unwrap_or("bar");
    let tick = s.tempo.samples_to_ticks(pos, sr);
    let bb = s.tempo.bar_beat_at_tick(tick);
    let next = match q {
        "none" => return pos,
        "beat" => {
            let m = s.tempo.meter_at_tick(tick);
            let start = s.tempo.tick_at_bar_beat(soundcraft_time::BarBeat { tick: 0, ..bb });
            if start >= tick { start } else { start.saturating_add(m.ticks_per_beat()) }
        }
        _ => {
            let start = s.tempo.bar_start_tick(bb.bar);
            if start >= tick { start } else { s.tempo.bar_start_tick(bb.bar.saturating_add(1)) }
        }
    };
    s.tempo.tick_to_samples(next, sr).max(pos)
}

/// Launch `slot` on track `t` at `at` (an empty slot stops the track there).
fn launch(s: &mut Session, t: TrackId, slot: usize, at: Samples) {
    // A launch still waiting is replaced by the new one.
    s.launch.retain(|l| !(l.track == t && l.at >= at));
    let filled = s.track(t).is_some_and(|tr| tr.slot(slot).is_some());
    if filled {
        s.launch.push(Launch { track: t, slot, at });
    } else if s.launch.iter().any(|l| l.track == t) {
        // Stop at `at`: settle what plays until then, and nothing after.
        s.launch.push(Launch { track: t, slot: usize::MAX, at });
    }
    s.launch.truncate(soundcraft_model::MAX_LAUNCHES);
}

fn launch_slot(e: &mut Engine, p: &Value) -> Result<Value> {
    let id = "slot.launch";
    let t = clip_track(e, id, p, "track")?;
    let slot = scene_param(e.session(), id, p, "scene")?;
    if e.session().track(t).and_then(|tr| tr.slot(slot)).is_none() {
        return Err(bad(id, "the slot is empty"));
    }
    let at = launch_time(e, p);
    launch(e.runtime_session_mut(), t, slot, at);
    Ok(json!({"track": t, "scene": slot, "at": at}))
}

fn launch_scene(e: &mut Engine, p: &Value) -> Result<Value> {
    let slot = scene_param(e.session(), "scene.launch", p, "scene")?;
    let tracks: Vec<TrackId> = e.session().tracks.iter().filter(|t| t.kind.has_slots()).map(|t| t.id).collect();
    let at = launch_time(e, p);
    let s = e.runtime_session_mut();
    let mut n = 0;
    for t in tracks {
        if s.track(t).is_some_and(|tr| tr.slot(slot).is_some()) {
            n += 1;
        }
        launch(s, t, slot, at);
    }
    Ok(json!({"scene": slot, "clips": n, "at": at}))
}

fn stop_track(e: &mut Engine, p: &Value) -> Result<Value> {
    let t = track_param(e, "slot.stop", p, "track")?.ok_or_else(|| bad("slot.stop", "`track` required"))?;
    if e.session().launched(t) {
        e.runtime_session_mut().launch.retain(|l| l.track != t);
    }
    Ok(json!({"track": t}))
}

fn settle(e: &mut Engine, p: &Value) -> Result<Value> {
    let at = super::position_param(e, "launcher.settle", p, "at")?.unwrap_or(e.transport.position);
    if e.session().launch.is_empty() {
        return Ok(json!({"changed": false}));
    }
    let mut s = e.session().clone();
    let changed = s.settle_launches(at);
    if changed {
        e.runtime_session_mut().launch = s.launch;
    }
    Ok(json!({"changed": changed}))
}

/// The most bars a new empty slot clip spans.
const MAX_NEW_BARS: i64 = 64;

fn set_slot(e: &mut Engine, p: &Value) -> Result<Value> {
    let id = "slot.set";
    let t = clip_track(e, id, p, "track")?;
    let slot = scene_param(e.session(), id, p, "scene")?;
    let s = e.session();
    let kind = s.track(t).map(|x| x.kind);
    let midi_track = kind.is_some_and(|k| k.is_midi());
    let mut clip = if let Some(cid) = p.get("clip").and_then(Value::as_u64) {
        let (_, c) = s.find_clip(ClipId(cid)).ok_or_else(|| bad(id, format!("no clip {cid}")))?;
        if c.is_video() || matches!(c.content, ClipContent::Midi { .. }) != midi_track {
            return Err(bad(id, if midi_track { "MIDI clips go on MIDI and instrument tracks" } else { "audio clips go on audio tracks" }));
        }
        c.clone()
    } else if let Some(src) = p.get("source").and_then(Value::as_u64) {
        if midi_track {
            return Err(bad(id, "audio files go on audio tracks"));
        }
        let f = s.source(soundcraft_model::SourceId(src)).ok_or_else(|| bad(id, format!("no audio file {src}")))?;
        let len = i64::try_from(f.frames).unwrap_or(0).max(1);
        Clip::audio(ClipId(0), f.name.clone(), f.id, 0, 0, len)
    } else {
        if !midi_track {
            return Err(bad(id, "a new empty clip goes on a MIDI or instrument track; pass `clip` or `source` for audio"));
        }
        let bars = i64_or(p, "bars", 1).clamp(1, MAX_NEW_BARS);
        let ticks = s.tempo.bar_start_tick(1i64.saturating_add(bars)).saturating_sub(s.tempo.bar_start_tick(1));
        let len = s.tempo.tick_to_samples(ticks, s.sample_rate).max(1);
        Clip::midi(ClipId(0), "Clip", 0, len, Sequence::default())
    };
    let s = e.session_mut();
    fit_slots(s);
    clip.id = s.new_clip_id();
    clip.start = 0;
    // A copy is a clip of its own: it is not another place of a pattern or a loop.
    clip.pattern_link = None;
    clip.loop_id = None;
    clip.group = None;
    let cid = clip.id;
    if let Some(x) = s.track_mut(t).and_then(|tr| tr.slots.get_mut(slot)) {
        *x = Some(clip);
    }
    Ok(json!({"clip": cid, "track": t, "scene": slot}))
}

fn clear_slot(e: &mut Engine, p: &Value) -> Result<Value> {
    let t = clip_track(e, "slot.clear", p, "track")?;
    let slot = scene_param(e.session(), "slot.clear", p, "scene")?;
    if e.session().track(t).and_then(|tr| tr.slot(slot)).is_none() {
        return Ok(json!({"cleared": false}));
    }
    let s = e.session_mut();
    if let Some(x) = s.track_mut(t).and_then(|tr| tr.slots.get_mut(slot)) {
        *x = None;
    }
    s.launch.retain(|l| !(l.track == t && l.slot == slot));
    Ok(json!({"cleared": true}))
}

fn move_slot(e: &mut Engine, p: &Value) -> Result<Value> {
    let id = "slot.move";
    let from_t = clip_track(e, id, p, "track")?;
    let from = scene_param(e.session(), id, p, "scene")?;
    let to_t = if p.get("to_track").is_some() { clip_track(e, id, p, "to_track")? } else { from_t };
    let to = scene_param(e.session(), id, p, "to_scene")?;
    let copy = bool_or(p, "copy", false);
    let s = e.session();
    let c = s.track(from_t).and_then(|tr| tr.slot(from)).cloned().ok_or_else(|| bad(id, "the slot is empty"))?;
    let midi = |t: TrackId| s.track(t).is_some_and(|x| x.kind.is_midi());
    if midi(from_t) != midi(to_t) {
        return Err(bad(id, "MIDI clips go on MIDI and instrument tracks, audio clips on audio tracks"));
    }
    if from_t == to_t && from == to {
        return Ok(json!({"clip": c.id}));
    }
    let s = e.session_mut();
    let mut c = c;
    if copy {
        c.id = s.new_clip_id();
    } else {
        if let Some(x) = s.track_mut(from_t).and_then(|tr| tr.slots.get_mut(from)) {
            *x = None;
        }
        s.launch.retain(|l| !(l.track == from_t && l.slot == from));
    }
    let cid = c.id;
    if let Some(x) = s.track_mut(to_t).and_then(|tr| tr.slots.get_mut(to)) {
        *x = Some(c);
    }
    Ok(json!({"clip": cid}))
}

fn follow(e: &mut Engine, p: &Value) -> Result<Value> {
    let id = "clip.follow";
    let ids = clip_ids_param(e, p);
    if ids.is_empty() {
        return Err(bad(id, "no clips: pass `clips` or select a clip"));
    }
    let action = str_param(p, "action").ok_or_else(|| bad(id, "`action` required (again, next, random, stop or none)"))?;
    let fo = if action.eq_ignore_ascii_case("none") {
        None
    } else {
        let a =
            FollowAction::parse(action).ok_or_else(|| bad(id, format!("unknown follow action `{action}` (again, next, random, stop or none)")))?;
        let plays = u32::try_from(i64_or(p, "plays", 1).clamp(1, i64::from(MAX_FOLLOW_PLAYS))).unwrap_or(1);
        Some(Follow { action: a, plays })
    };
    let s = e.session_mut();
    let mut n = 0;
    for c in &ids {
        if let Some(c) = s.find_clip_mut(*c) {
            c.follow = fo;
            n += 1;
        }
    }
    Ok(json!({"clips": n}))
}

fn warp(e: &mut Engine, p: &Value) -> Result<Value> {
    let ids = clip_ids_param(e, p);
    let s = e.session();
    let audio: Vec<ClipId> = ids.iter().copied().filter(|c| s.find_clip(*c).is_some_and(|(_, c)| c.is_audio())).collect();
    if audio.is_empty() {
        return Err(bad("clip.warp", "select an audio clip to warp"));
    }
    let v = p.get("value").and_then(Value::as_bool).unwrap_or_else(|| !audio.iter().all(|c| s.find_clip(*c).is_some_and(|(_, c)| c.warp)));
    let s = e.session_mut();
    for c in &audio {
        if let Some(c) = s.find_clip_mut(*c) {
            c.warp = v;
        }
    }
    Ok(json!({"value": v, "clips": audio.len()}))
}

/// After a tempo change: warped audio clips stretch (pitch kept) to cover the same bars and beats
/// as before, Arrangement clips keep their bar position, and Session View MIDI clips keep their
/// length in beats. Clips that cannot be stretched keep their audio and say why.
pub(crate) fn follow_tempo(e: &mut Engine, before: &Session) {
    let s = e.session();
    let sr = s.sample_rate;
    let (old, new) = (&before.tempo, &s.tempo);
    // (track, slot or None, clip id, new start, new length)
    let mut work: Vec<(TrackId, Option<usize>, ClipId, Samples, Samples)> = Vec::new();
    for t in &s.tracks {
        for c in t.clips().iter().filter(|c| c.warp && c.is_audio()) {
            let a = new.tick_to_samples(old.samples_to_ticks(c.start, sr), sr);
            let b = new.tick_to_samples(old.samples_to_ticks(c.end(), sr), sr);
            work.push((t.id, None, c.id, a, b.saturating_sub(a).max(1)));
        }
        for (i, c) in t.slots.iter().enumerate() {
            let Some(c) = c else { continue };
            if c.warp && c.is_audio() || matches!(c.content, ClipContent::Midi { .. }) {
                let len = new.tick_to_samples(old.samples_to_ticks(c.length, sr), sr).max(1);
                work.push((t.id, Some(i), c.id, 0, len));
            }
        }
    }
    let mut failed = Vec::new();
    for (t, slot, id, start, len) in work {
        let Some((_, cur)) = e.session().find_clip(id).map(|(t, c)| (t, c.clone())) else { continue };
        if cur.start == start && cur.length == len {
            continue;
        }
        let s = e.session_mut();
        let mut nc = cur.clone();
        nc.start = start;
        if cur.is_audio() {
            let orig = super::clip::warp_original(s, id);
            let mut base = cur.clone();
            if let Some((src, off, olen)) = orig {
                base.content = ClipContent::Audio { source: src, offset: off };
                base.length = olen.max(1);
                base.stretch = 1.0;
            }
            match super::more_util::render_stretched(s, "clip.warp", &base, len) {
                Ok(Some(src)) => {
                    if orig.is_none()
                        && let ClipContent::Audio { source, offset } = base.content
                    {
                        let key = |k: &str| format!("elastic.{}.{k}", id.0);
                        s.edit.values.insert(key("src"), source.0 as f64);
                        s.edit.values.insert(key("off"), offset as f64);
                        s.edit.values.insert(key("len"), base.length as f64);
                    }
                    nc.content = ClipContent::Audio { source: src, offset: 0 };
                    nc.stretch = 1.0;
                }
                Ok(None) => continue,
                Err(err) => {
                    failed.push(format!("{}: {err}", cur.name));
                    continue;
                }
            }
        }
        nc.length = len;
        nc.clamp_fades();
        let old_src = cur.source();
        match slot {
            Some(i) => {
                if let Some(x) = s.track_mut(t).and_then(|tr| tr.slots.get_mut(i)) {
                    *x = Some(nc);
                }
            }
            None => super::more_util::replace_clip(s, t, id, nc),
        }
        // A stretch rendered by an earlier tempo change that nothing plays any more goes.
        if let Some(old_src) = old_src
            && super::clip::warp_original(s, id).is_none_or(|(o, _, _)| o != old_src)
            && s.clips_using(old_src).next().is_none()
            && s.source(old_src).is_some_and(|f| f.name.ends_with("-TCE"))
        {
            s.sources.retain(|f| f.id != old_src);
            s.pool.remove(old_src);
        }
    }
    if !failed.is_empty() {
        e.messages.push(format!("Warp could not follow the tempo: {}", failed.join("; ")));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soundcraft_audio_io::{AudioBuffer, FileFormat};
    use soundcraft_model::{ChannelFormat, TrackKind};

    fn engine() -> (Engine, TrackId, TrackId) {
        let mut e = Engine::default();
        let s = e.session_mut();
        let a = s.add_track(TrackKind::Audio, ChannelFormat::Mono, Some("Loop"));
        let m = s.add_track(TrackKind::Midi, ChannelFormat::Mono, Some("Keys"));
        e.execute("scene.new", &json!({})).unwrap();
        e.execute("scene.new", &json!({"name": "Chorus"})).unwrap();
        (e, a, m)
    }

    #[test]
    fn scenes_are_rows_that_can_be_named_moved_and_deleted() {
        let (mut e, a, m) = engine();
        let c = e.execute("slot.set", &json!({"track": m.0, "scene": 1, "bars": 2})).unwrap()["clip"].as_u64().unwrap();
        assert_eq!(e.session().scenes.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["1", "Chorus"]);
        assert_eq!(e.session().track(a).unwrap().slots.len(), 2);
        e.execute("scene.rename", &json!({"scene": 0, "name": "Verse"})).unwrap();
        e.execute("scene.move", &json!({"scene": 1, "to": 0})).unwrap();
        assert_eq!(e.session().scenes[0].name, "Chorus");
        assert_eq!(e.session().track(m).unwrap().slot(0).map(|c| c.id.0), Some(c), "the clip moves with its scene");
        e.execute("scene.delete", &json!({"scene": 0})).unwrap();
        assert_eq!(e.session().scenes.len(), 1);
        assert!(e.session().track(m).unwrap().slot(0).is_none());
        assert!(e.undo());
        assert_eq!(e.session().scenes.len(), 2);
        assert!(e.execute("scene.rename", &json!({"scene": 9, "name": "x"})).is_err());
    }

    #[test]
    fn launching_a_clip_starts_the_transport_and_quantizes_to_the_bar_while_playing() {
        let (mut e, _a, m) = engine();
        e.execute("slot.set", &json!({"track": m.0, "scene": 0})).unwrap();
        e.execute("slot.set", &json!({"track": m.0, "scene": 1})).unwrap();
        let saved = e.session().clone();
        e.replace_session(saved.clone());
        e.transport_requests.clear();
        let r = e.execute("slot.launch", &json!({"track": m.0, "scene": 0})).unwrap();
        assert_eq!(r["at"], 0);
        assert!(e.transport_requests.contains(&TransportRequest::PlayFrom(0)));
        assert!(!e.is_dirty(), "launching is not an edit");
        assert_eq!(e.undo_label(), None, "launching is not undone");
        e.transport.playing = true;
        e.transport.position = 1000;
        let r = e.execute("slot.launch", &json!({"track": m.0, "scene": 1})).unwrap();
        let bar2 = e.session().tempo.tick_to_samples(e.session().tempo.bar_start_tick(2), e.session().sample_rate);
        assert_eq!(r["at"].as_i64(), Some(bar2), "the next bar");
        assert_eq!(e.session().launch.len(), 2);
        e.execute("launcher.stop_all", &json!({})).unwrap();
        assert!(e.session().launch.is_empty());
        assert_eq!(e.session().scenes, saved.scenes);
    }

    #[test]
    fn a_scene_launches_its_row_and_stops_tracks_with_empty_slots() {
        let (mut e, a, m) = engine();
        e.execute("slot.set", &json!({"track": m.0, "scene": 1})).unwrap();
        e.transport.playing = true;
        e.execute("slot.launch", &json!({"track": m.0, "scene": 1, "quantize": "none"})).unwrap();
        let r = e.execute("scene.launch", &json!({"scene": 0, "quantize": "none"})).unwrap();
        assert_eq!(r["clips"], 0);
        let s = e.session().clone();
        assert!(s.playing_slot(s.track(m).unwrap(), 10).is_none(), "the empty slot stopped the track");
        assert!(!s.launched(a));
    }

    #[test]
    fn follow_actions_and_slot_moves_are_edits() {
        let (mut e, _a, m) = engine();
        let c = e.execute("slot.set", &json!({"track": m.0, "scene": 0})).unwrap()["clip"].as_u64().unwrap();
        e.execute("clip.follow", &json!({"clips": [c], "action": "next", "plays": 2})).unwrap();
        assert_eq!(e.session().find_clip(ClipId(c)).unwrap().1.follow, Some(Follow { action: FollowAction::Next, plays: 2 }));
        assert!(e.execute("clip.follow", &json!({"clips": [c], "action": "sideways"})).is_err());
        e.execute("slot.move", &json!({"track": m.0, "scene": 0, "to_scene": 1})).unwrap();
        assert!(e.session().track(m).unwrap().slot(0).is_none());
        assert_eq!(e.session().track(m).unwrap().slot(1).unwrap().follow.map(|f| f.action), Some(FollowAction::Next));
        e.execute("slot.move", &json!({"track": m.0, "scene": 1, "to_scene": 0, "copy": true})).unwrap();
        let s = e.session();
        assert_ne!(s.track(m).unwrap().slot(0).unwrap().id, s.track(m).unwrap().slot(1).unwrap().id);
    }

    #[test]
    fn warped_audio_stays_in_time_when_the_tempo_changes() {
        let (mut e, a, m) = engine();
        let sr = 48_000.0f32;
        // One bar at 120 bpm (4/4): 2 s.
        let tone: Vec<f32> = (0..96_000).map(|i| (i as f32 * 220.0 * std::f32::consts::TAU / sr).sin() * 0.5).collect();
        let s = e.session_mut();
        let src = crate::io::add_source(s, "loop", AudioBuffer { sample_rate: 48_000, channels: vec![tone] }, None, FileFormat::Wav);
        let cid = s.new_clip_id();
        crate::edit::place_clip(s, a, Clip::audio(cid, "loop", src, 0, 96_000, 96_000));
        let unwarped = s.new_clip_id();
        crate::edit::place_clip(s, a, Clip::audio(unwarped, "plain", src, 0, 800_000, 96_000));
        e.execute("slot.set", &json!({"track": m.0, "scene": 0, "bars": 1})).unwrap();
        e.execute("clip.warp", &json!({"clips": [cid.0], "value": true})).unwrap();
        e.execute("event.tempo", &json!({"bpm": 60.0, "at": 0})).unwrap();
        let s = e.session();
        let c = s.find_clip(cid).unwrap().1;
        assert_eq!((c.start, c.length), (192_000, 192_000), "bar 2 is now at 4 s, and the clip covers the same bar");
        assert!(s.find_clip(cid).unwrap().1.source() != Some(src));
        let plain = s.find_clip(unwarped).unwrap().1;
        assert_eq!((plain.start, plain.length), (800_000, 96_000), "unwarped audio keeps its samples");
        assert_eq!(s.track(m).unwrap().slot(0).unwrap().length, 192_000, "a MIDI slot clip keeps its one bar");
        // A second change re-renders from the original and drops the stretch nothing plays.
        let sources = s.sources.len();
        e.execute("event.tempo", &json!({"bpm": 240.0, "at": 0})).unwrap();
        let s = e.session();
        let c = s.find_clip(cid).unwrap().1;
        assert_eq!((c.start, c.length), (48_000, 48_000));
        assert_eq!(s.sources.len(), sources, "the earlier stretch is gone");
        assert!(e.undo());
        assert_eq!(e.session().find_clip(cid).unwrap().1.length, 192_000, "one undo step with the tempo");
    }
}
