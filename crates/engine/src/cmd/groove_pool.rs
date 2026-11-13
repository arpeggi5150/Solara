//! The Groove Pool (Ableton-style layout): grooves saved in the session and dropped on MIDI clips.
//! It is the same groove the FL-style Channel Rack sets ([`soundcraft_model::Groove`] in
//! `Clip::groove`): a clip that uses a pool groove keeps a copy there and the pool id beside it, so
//! changing the pool groove changes every clip that uses it.
use super::{CommandSpec, always, bad, clip_ids_param, str_param};

use crate::{Engine, Result, cmd};
use serde_json::{Value, json};
use soundcraft_model::{ClipContent, ClipId, Groove, GroovePreset, MAX_GROOVES, MAX_NAME_CHARS, Session};

/// Grooves offered to start the pool with: name, swing, humanize.
pub const STARTERS: [(&str, f32, f32); 6] =
    [("Swing 25", 0.25, 0.0), ("Swing 50", 0.5, 0.0), ("Swing 75", 0.75, 0.0), ("Loose", 0.1, 0.4), ("Lazy", 0.4, 0.2), ("Shuffle", 1.0, 0.1)];

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "groove.add",
            "Add Groove",
            [],
            None,
            "{name?, swing?: 0..1, humanize?: 0..1, starter?: name, from_clip?: id}: save a groove to the Groove Pool",
            always,
            add
        ),
        cmd!("groove.edit", "Edit Groove", [], None, "{groove: id, name?, swing?, humanize?}: every clip that uses the groove follows", always, edit),
        cmd!(
            "groove.apply",
            "Apply Groove",
            [],
            None,
            "{clips?, groove: id | null}: MIDI clips take the pool groove (null takes it off)",
            always,
            apply
        ),
        cmd!("groove.delete", "Delete Groove", [], None, "{groove: id}: clips that used it keep its feel as their own", always, delete),
    ]
}

/// A 0..=1 amount from `key`, if given.
fn amount(p: &Value, key: &str) -> Option<f32> {
    p.get(key).and_then(Value::as_f64).filter(|v| v.is_finite()).map(|v| v.clamp(0.0, 1.0) as f32)
}

fn groove_id(s: &Session, cmd: &str, p: &Value) -> Result<u64> {
    let id = p.get("groove").and_then(Value::as_u64).ok_or_else(|| bad(cmd, "`groove` id required"))?;
    if !s.grooves.iter().any(|g| g.id == id) {
        return Err(bad(cmd, format!("no groove {id} in the Groove Pool")));
    }
    Ok(id)
}

fn add(e: &mut Engine, p: &Value) -> Result<Value> {
    let id = "groove.add";
    let s = e.session();
    if s.grooves.len() >= MAX_GROOVES {
        return Err(bad(id, format!("the Groove Pool holds at most {MAX_GROOVES} grooves")));
    }
    let (mut name, mut groove) = (String::from("Groove"), Groove::default());
    if let Some(st) = str_param(p, "starter") {
        let (n, sw, hu) = STARTERS.iter().find(|(n, _, _)| n.eq_ignore_ascii_case(st)).ok_or_else(|| bad(id, format!("no starter groove `{st}`")))?;
        (name, groove) = (n.to_string(), Groove { swing: *sw, humanize: *hu });
    }
    if let Some(c) = p.get("from_clip").and_then(Value::as_u64) {
        let (_, clip) = s.find_clip(ClipId(c)).ok_or_else(|| bad(id, format!("no clip {c}")))?;
        groove = clip.groove.unwrap_or_default();
        name = format!("{} Groove", clip.name);
    }
    if let Some(v) = amount(p, "swing") {
        groove.swing = v;
    }
    if let Some(v) = amount(p, "humanize") {
        groove.humanize = v;
    }
    if let Some(n) = str_param(p, "name") {
        name = n.to_string();
    }
    let name: String = name.chars().take(MAX_NAME_CHARS).collect();
    let s = e.session_mut();
    let gid = s.alloc();
    s.grooves.push(GroovePreset { id: gid, name, groove });
    Ok(json!({"groove": gid}))
}

/// Every clip (all playlists and slots) that uses pool groove `id` takes `g`.
fn follow(s: &mut Session, id: u64, g: Option<Groove>) -> usize {
    let mut n = 0;
    for t in &mut s.tracks {
        let slots = t.slots.iter_mut().flatten();
        for c in t.playlists.iter_mut().flat_map(|p| p.clips.iter_mut()).chain(slots) {
            if c.groove_pool == Some(id) {
                c.groove = g;
                if g.is_none() {
                    c.groove_pool = None;
                }
                n += 1;
            }
        }
    }
    n
}

fn edit(e: &mut Engine, p: &Value) -> Result<Value> {
    let gid = groove_id(e.session(), "groove.edit", p)?;
    let s = e.session_mut();
    let Some(g) = s.grooves.iter_mut().find(|g| g.id == gid) else { return Err(bad("groove.edit", "no such groove")) };
    if let Some(v) = amount(p, "swing") {
        g.groove.swing = v;
    }
    if let Some(v) = amount(p, "humanize") {
        g.groove.humanize = v;
    }
    if let Some(n) = str_param(p, "name") {
        g.name = n.chars().take(MAX_NAME_CHARS).collect();
    }
    let groove = g.groove;
    let n = follow(s, gid, Some(groove));
    Ok(json!({"groove": gid, "clips": n}))
}

fn apply(e: &mut Engine, p: &Value) -> Result<Value> {
    let id = "groove.apply";
    let pool = match p.get("groove") {
        Some(Value::Null) => None,
        _ => Some(groove_id(e.session(), id, p)?),
    };
    let s = e.session();
    let ids: Vec<ClipId> =
        clip_ids_param(e, p).into_iter().filter(|c| s.find_clip(*c).is_some_and(|(_, c)| matches!(c.content, ClipContent::Midi { .. }))).collect();
    if ids.is_empty() {
        return Err(bad(id, "grooves play on MIDI clips: select a MIDI clip"));
    }
    let groove = pool.and_then(|g| s.grooves.iter().find(|x| x.id == g)).map(|g| g.groove);
    let s = e.session_mut();
    for c in &ids {
        if let Some(c) = s.find_clip_mut(*c) {
            c.groove = groove;
            c.groove_pool = pool;
        }
    }
    Ok(json!({"clips": ids.len()}))
}

fn delete(e: &mut Engine, p: &Value) -> Result<Value> {
    let gid = groove_id(e.session(), "groove.delete", p)?;
    let s = e.session_mut();
    s.grooves.retain(|g| g.id != gid);
    for t in &mut s.tracks {
        let slots = t.slots.iter_mut().flatten();
        for c in t.playlists.iter_mut().flat_map(|p| p.clips.iter_mut()).chain(slots) {
            if c.groove_pool == Some(gid) {
                c.groove_pool = None;
            }
        }
    }
    Ok(json!({"grooves": s.grooves.len()}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use soundcraft_midi::Sequence;
    use soundcraft_model::{ChannelFormat, Clip, TrackKind};

    #[test]
    fn a_pool_groove_changes_every_clip_that_uses_it() {
        let mut e = Engine::default();
        let s = e.session_mut();
        let t = s.add_track(TrackKind::Midi, ChannelFormat::Mono, None);
        let a = s.new_clip_id();
        let b = s.new_clip_id();
        crate::edit::place_clip(s, t, Clip::midi(a, "a", 0, 48_000, Sequence::default()));
        crate::edit::place_clip(s, t, Clip::midi(b, "b", 48_000, 48_000, Sequence::default()));
        let g = e.execute("groove.add", &json!({"starter": "Swing 50"})).unwrap()["groove"].as_u64().unwrap();
        e.execute("groove.apply", &json!({"clips": [a.0, b.0], "groove": g})).unwrap();
        let swing = |e: &Engine, c: ClipId| e.session().find_clip(c).unwrap().1.groove.map(|g| g.swing);
        assert_eq!(swing(&e, a), Some(0.5));
        e.execute("groove.edit", &json!({"groove": g, "swing": 0.75})).unwrap();
        assert_eq!((swing(&e, a), swing(&e, b)), (Some(0.75), Some(0.75)), "shared: both follow");
        e.execute("groove.delete", &json!({"groove": g})).unwrap();
        assert_eq!(swing(&e, a), Some(0.75), "the clip keeps the feel");
        assert_eq!(e.session().find_clip(a).unwrap().1.groove_pool, None);
        assert!(e.execute("groove.apply", &json!({"clips": [a.0], "groove": g})).is_err());
        assert!(e.execute("groove.add", &json!({"starter": "Nope"})).is_err());
    }
}
