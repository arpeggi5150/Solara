//! Automation clips (FL-style Playlist): a stretch of a track's automation lane held as a clip that
//! can be moved, copied and reused. The lane is what plays, so a clip writes its shape into the
//! lane wherever it goes, and the mix reads it like any other automation. Edits made to the lane
//! inside a clip are taken into the clip when it next moves or is copied.
use super::{CommandSpec, always, bad, position_param, range_param, str_param, track_param};

use crate::{Engine, Result, cmd};
use serde_json::{Value, json};
use soundcraft_model::{AutoClip, AutoParam, MAX_AUTO_CLIPS, Session, Track, TrackId};
use soundcraft_time::{GridValue, NoteValue, Samples};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(query "auto_clip.list", "List Automation Clips", [], None, "{track?}", always, list),
        cmd!(
            "auto_clip.new",
            "New Automation Clip",
            [],
            None,
            "{track, param?: volume|pan|mute|send_a_level|plugin:a:<param> (volume), start?, end? (the edit selection, or one bar)}",
            always,
            new
        ),
        cmd!("auto_clip.move", "Move Automation Clip", [], None, "{id, at, track?}", always, move_clip),
        cmd!("auto_clip.copy", "Copy Automation Clip", [], None, "{id, at, track?}", always, copy),
        cmd!("auto_clip.delete", "Delete Automation Clip", [], None, "{id} — also clears its stretch of the lane", always, delete),
    ]
}

/// The parameter's value without automation: the fader, the panner, or the range's default.
fn base_value(t: &Track, param: &AutoParam) -> f32 {
    match param {
        AutoParam::Volume => t.mixer.volume_db,
        AutoParam::Pan(i) => t.mixer.pan.get(usize::from(*i)).copied().unwrap_or(0.0),
        AutoParam::Mute => f32::from(u8::from(t.mixer.mute)),
        p => p.range().2,
    }
}

fn json_of(c: &AutoClip) -> Value {
    json!({"id": c.id, "name": c.name, "track": c.track, "param": c.param.label(), "start": c.start, "end": c.end(), "points": c.points.len()})
}

fn list(e: &mut Engine, p: &Value) -> Result<Value> {
    let track = track_param(e, "auto_clip.list", p, "track")?;
    let clips: Vec<Value> = e.session().auto_clips.iter().filter(|c| track.is_none_or(|t| c.track == t)).map(json_of).collect();
    Ok(json!({"clips": clips}))
}

fn find(s: &Session, cmd: &str, p: &Value) -> Result<AutoClip> {
    let id = p.get("id").and_then(Value::as_u64).ok_or_else(|| bad(cmd, "`id` required"))?;
    let c = s.auto_clips.iter().find(|c| c.id == id).cloned().ok_or_else(|| bad(cmd, "no such automation clip"))?;
    if s.track(c.track).is_none() {
        return Err(bad(cmd, "its track is gone"));
    }
    Ok(c)
}

/// The clip's shape as its lane holds it now, so lane edits inside it are kept.
fn recapture(s: &Session, c: &mut AutoClip) {
    if let Some(t) = s.track(c.track) {
        c.capture(t.lane(&c.param), base_value(t, &c.param));
    }
}

/// Write `c` into its track's lane, where it sits.
fn write(s: &mut Session, c: &AutoClip) {
    if let Some(t) = s.track_mut(c.track) {
        c.write(t.lane_mut(&c.param));
    }
}

fn new(e: &mut Engine, p: &Value) -> Result<Value> {
    let track = track_param(e, "auto_clip.new", p, "track")?.ok_or_else(|| bad("auto_clip.new", "`track` required"))?;
    let name = str_param(p, "param").unwrap_or("volume");
    let param = AutoParam::parse(name).ok_or_else(|| bad("auto_clip.new", format!("unknown parameter `{name}`")))?;
    if e.session().auto_clips.len() >= MAX_AUTO_CLIPS {
        return Err(bad("auto_clip.new", format!("a session holds at most {MAX_AUTO_CLIPS} automation clips")));
    }
    let mut r = range_param(e, "auto_clip.new", p)?;
    if r.is_empty() {
        let s = e.session();
        let bar = GridValue::Note { value: NoteValue::Bar, dotted: false, triplet: false };
        let start = bar.line_at_or_before(r.start, s.sample_rate, &s.tempo, s.frame_rate);
        r = soundcraft_time::Range::new(start, bar.next_line(start, s.sample_rate, &s.tempo, s.frame_rate));
    }
    if r.is_empty() {
        return Err(bad("auto_clip.new", "the clip needs a length"));
    }
    let s = e.session_mut();
    let label = s.track(track).map(|t| format!("{} {}", t.name, param.label())).unwrap_or_default();
    let mut c = AutoClip { id: s.alloc(), name: label, track, param, start: r.start, length: r.len(), points: Vec::new() };
    recapture(s, &mut c);
    c.sanitize();
    write(s, &c);
    let out = json_of(&c);
    s.auto_clips.push(c);
    Ok(out)
}

/// Where a moved or copied clip lands: `at`, on `track` (same parameter) or its own track.
fn destination(e: &Engine, cmd: &str, p: &Value, c: &AutoClip) -> Result<(Samples, TrackId)> {
    let at = position_param(e, cmd, p, "at")?.ok_or_else(|| bad(cmd, "`at` required"))?.max(0);
    let track = track_param(e, cmd, p, "track")?.unwrap_or(c.track);
    Ok((at, track))
}

fn move_clip(e: &mut Engine, p: &Value) -> Result<Value> {
    let mut c = find(e.session(), "auto_clip.move", p)?;
    let (at, track) = destination(e, "auto_clip.move", p, &c)?;
    let s = e.session_mut();
    recapture(s, &mut c);
    // The stretch it leaves goes back to the lane around it.
    if let Some(t) = s.track_mut(c.track) {
        t.lane_mut(&c.param).clear_range(c.start, c.end());
    }
    c.start = at;
    c.track = track;
    c.sanitize();
    write(s, &c);
    if let Some(slot) = s.auto_clips.iter_mut().find(|x| x.id == c.id) {
        *slot = c.clone();
    }
    Ok(json_of(&c))
}

fn copy(e: &mut Engine, p: &Value) -> Result<Value> {
    let mut c = find(e.session(), "auto_clip.copy", p)?;
    if e.session().auto_clips.len() >= MAX_AUTO_CLIPS {
        return Err(bad("auto_clip.copy", format!("a session holds at most {MAX_AUTO_CLIPS} automation clips")));
    }
    let (at, track) = destination(e, "auto_clip.copy", p, &c)?;
    let s = e.session_mut();
    recapture(s, &mut c);
    // The original keeps the shape it has now too.
    if let Some(slot) = s.auto_clips.iter_mut().find(|x| x.id == c.id) {
        slot.points.clone_from(&c.points);
    }
    c.id = s.alloc();
    c.start = at;
    c.track = track;
    c.sanitize();
    write(s, &c);
    let out = json_of(&c);
    s.auto_clips.push(c);
    Ok(out)
}

fn delete(e: &mut Engine, p: &Value) -> Result<Value> {
    let id = p.get("id").and_then(Value::as_u64).ok_or_else(|| bad("auto_clip.delete", "`id` required"))?;
    let s = e.session_mut();
    let i = s.auto_clips.iter().position(|c| c.id == id).ok_or_else(|| bad("auto_clip.delete", "no such automation clip"))?;
    let c = s.auto_clips.remove(i);
    if let Some(t) = s.track_mut(c.track) {
        t.lane_mut(&c.param).clear_range(c.start, c.end());
    }
    Ok(json!({"deleted": id}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use soundcraft_model::{ChannelFormat, TrackKind};

    fn session() -> (Engine, TrackId) {
        let mut e = Engine::default();
        let t = e.session_mut().add_track(TrackKind::Audio, ChannelFormat::Mono, Some("Pad"));
        (e, t)
    }

    fn volume_at(e: &Engine, t: TrackId, at: Samples) -> f32 {
        let tr = e.session().track(t).unwrap();
        tr.lane(&AutoParam::Volume).map_or(tr.mixer.volume_db, |l| l.value_at(at, tr.mixer.volume_db))
    }

    /// A clip over 1000..2000 that ramps from -20 dB to 0 dB.
    fn ramp(e: &mut Engine, t: TrackId) -> u64 {
        e.execute("auto_clip.new", &json!({"track": t.0, "param": "volume", "start": 1000, "end": 2000})).unwrap();
        let id = e.session().auto_clips[0].id;
        let tr = e.session_mut().track_mut(t).unwrap();
        let lane = tr.lane_mut(&AutoParam::Volume);
        lane.set_point(1000, -20.0);
        lane.set_point(1999, 0.0);
        id
    }

    #[test]
    fn a_copied_clip_drives_the_parameter_where_it_lands() {
        let (mut e, t) = session();
        let id = ramp(&mut e, t);
        let r = e.execute("auto_clip.copy", &json!({"id": id, "at": 5000})).unwrap();
        assert_ne!(r["id"].as_u64(), Some(id));
        assert_eq!(volume_at(&e, t, 5000), -20.0);
        assert_eq!(volume_at(&e, t, 5999), 0.0);
        assert!((volume_at(&e, t, 5500) + 10.0).abs() < 0.1, "the ramp came along");
        // The original still drives its own stretch.
        assert_eq!(volume_at(&e, t, 1000), -20.0);
        assert_eq!(e.session().auto_clips.len(), 2);
    }

    #[test]
    fn a_moved_clip_takes_its_shape_and_leaves_its_old_stretch() {
        let (mut e, t) = session();
        let id = ramp(&mut e, t);
        e.execute("auto_clip.move", &json!({"id": id, "at": 8000})).unwrap();
        let tr = e.session().track(t).unwrap();
        let lane = tr.lane(&AutoParam::Volume).unwrap();
        assert!(lane.points.iter().all(|p| p.at >= 8000), "{:?}", lane.points);
        assert_eq!(volume_at(&e, t, 8000), -20.0);
        assert_eq!(e.session().auto_clips[0].start, 8000);
        // Undo puts it back.
        e.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(e.session().auto_clips[0].start, 1000);
        assert_eq!(volume_at(&e, t, 1000), -20.0);
    }

    #[test]
    fn a_clip_copies_to_another_track() {
        let (mut e, t) = session();
        let other = e.session_mut().add_track(TrackKind::Audio, ChannelFormat::Mono, Some("Lead"));
        let id = ramp(&mut e, t);
        e.execute("auto_clip.copy", &json!({"id": id, "at": 0, "track": other.0})).unwrap();
        assert_eq!(volume_at(&e, other, 0), -20.0);
        assert_eq!(e.execute("auto_clip.list", &json!({"track": other.0})).unwrap()["clips"].as_array().map(Vec::len), Some(1));
    }

    #[test]
    fn hostile_calls_are_refused() {
        let (mut e, t) = session();
        assert!(e.execute("auto_clip.new", &json!({"track": t.0, "param": "nonsense", "start": 0, "end": 10})).is_err());
        assert!(e.execute("auto_clip.move", &json!({"id": 77, "at": 0})).is_err());
        let id = ramp(&mut e, t);
        assert!(e.execute("auto_clip.copy", &json!({"id": id})).is_err(), "`at` required");
        e.execute("auto_clip.delete", &json!({"id": id})).unwrap();
        assert!(e.session().auto_clips.is_empty());
        assert!(e.session().track(t).unwrap().lane(&AutoParam::Volume).is_none_or(|l| l.points.is_empty()));
    }

    #[test]
    fn a_new_clip_without_a_range_is_one_bar() {
        let (mut e, t) = session();
        let r = e.execute("auto_clip.new", &json!({"track": t.0})).unwrap();
        assert!(r["end"].as_i64().unwrap() > r["start"].as_i64().unwrap());
        assert_eq!(r["param"], "volume");
    }
}
