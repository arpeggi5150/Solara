//! Tab tracks, TabGarden and Guitar Pro files: open a `.tabgarden` or Guitar Pro file as the session, export the MIDI
//! parts to one, and set a tab track's view, tuning and the string a note is played on.
use super::{CommandSpec, always, bad, bool_or, i64_or, str_param, track_param, tracks_param};

use crate::{Engine, Result, cmd};
use serde_json::{Value, json};
use soundcraft_midi::tab::{MAX_FRET, MAX_STRINGS, Tuning};
use soundcraft_model::{ClipContent, TabView, TrackId, TrackKind};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(noundo "file.open_tabgarden", "Open TabGarden File...", ["File"], None, "{path} — opens a .tabgarden file as a new session, every track in it", always, |e, p| {
            let path = str_param(p, "path").ok_or_else(|| bad("file.open_tabgarden", "`path` required"))?.to_string();
            let bytes = crate::io::read_file(&path)?;
            crate::io::open_tabgarden_bytes(e, &path, &bytes)
        }),
        cmd!(
            noundo "file.open_guitar_pro",
            "Open Guitar Pro File...",
            ["File"],
            None,
            "{path} — opens a Guitar Pro file (.gp, .gpx, .gp5, .gp4, .gp3) as a new session, every track in it",
            always,
            |e, p| {
                let path = str_param(p, "path").ok_or_else(|| bad("file.open_guitar_pro", "`path` required"))?.to_string();
                let bytes = crate::io::read_file(&path)?;
                crate::io::open_guitar_pro_bytes(e, &path, &bytes)
            }
        ),
        cmd!(noundo "file.export_tabgarden", "TabGarden File...", ["File", "Export"], None, "{path, tracks?, replace?: false} — MIDI, instrument and tab tracks as a .tabgarden file TabGarden opens", always, export),
        cmd!(
            "track.tab_view",
            "Tab View",
            [],
            None,
            "{track?, view: tab|notation|both} — what a tab track shows (the selected tab tracks without `track`)",
            always,
            tab_view
        ),
        cmd!(
            "track.tab_tuning",
            "Tab Tuning",
            [],
            None,
            "{track?, preset?: guitar|bass|drop_d|seven, strings?: [MIDI pitch, string 1 (highest) first], capo?: 0..36}",
            always,
            tab_tuning
        ),
        cmd!(
            "event.tab_string",
            "Move Note to String",
            [],
            None,
            "{clip, note: index, string: 1-based} — plays the note on that string in the tab",
            always,
            tab_string
        ),
    ]
}

fn export(e: &mut Engine, p: &Value) -> Result<Value> {
    let path = str_param(p, "path").ok_or_else(|| bad("file.export_tabgarden", "`path` required"))?.to_string();
    crate::file_access::check_write(std::path::Path::new(&path)).map_err(crate::EngineError::Io)?;
    if crate::io::replaces(e, "file.export_tabgarden", std::path::Path::new(&path)) && !bool_or(p, "replace", false) {
        return Err(bad("file.export_tabgarden", format!("{path} already exists: pass `replace: true` to replace it")));
    }
    let only = if p.get("tracks").is_some() || p.get("track").is_some() { Some(tracks_param(e, "file.export_tabgarden", p)?) } else { None };
    let bytes = crate::io::export_tabgarden(e, only.as_deref())?;
    crate::io::write_file(&path, &bytes)?;
    Ok(json!({"path": path, "bytes": bytes.len()}))
}

/// `track`, else the selected tab tracks; every one must be a tab track.
fn tab_tracks(e: &Engine, cmd: &str, p: &Value) -> Result<Vec<TrackId>> {
    let ids = match track_param(e, cmd, p, "track")? {
        Some(t) => vec![t],
        None => e.target_tracks(),
    };
    let s = e.session();
    let ids: Vec<TrackId> = ids.into_iter().filter(|id| s.track(*id).is_some_and(|t| t.kind == TrackKind::Tab)).collect();
    if ids.is_empty() {
        return Err(bad(cmd, "no tab track: pass `track` or select a tab track"));
    }
    Ok(ids)
}

fn tab_view(e: &mut Engine, p: &Value) -> Result<Value> {
    let view = str_param(p, "view").and_then(TabView::from_id).ok_or_else(|| bad("track.tab_view", "`view` must be tab, notation or both"))?;
    let ids = tab_tracks(e, "track.tab_view", p)?;
    let s = e.session_mut();
    for id in &ids {
        if let Some(t) = s.track_mut(*id) {
            let mut setup = t.tab_setup().unwrap_or_default();
            setup.view = view;
            t.tab = Some(setup);
        }
    }
    Ok(json!({"tracks": ids.iter().map(|t| t.0).collect::<Vec<_>>(), "view": view.id()}))
}

fn tab_tuning(e: &mut Engine, p: &Value) -> Result<Value> {
    let cmd = "track.tab_tuning";
    let preset = match str_param(p, "preset") {
        None => None,
        Some("guitar") => Some(Tuning::guitar().strings),
        Some("bass") => Some(Tuning::bass().strings),
        Some("drop_d") => Some(vec![64, 59, 55, 50, 45, 38]),
        Some("seven") => Some(vec![64, 59, 55, 50, 45, 40, 35]),
        Some(other) => return Err(bad(cmd, format!("unknown preset `{other}` (guitar, bass, drop_d or seven)"))),
    };
    let strings = match p.get("strings") {
        Some(v) => {
            let arr = v.as_array().ok_or_else(|| bad(cmd, "`strings` must be an array of MIDI pitches"))?;
            if arr.is_empty() || arr.len() > MAX_STRINGS {
                return Err(bad(cmd, format!("`strings` must name 1 to {MAX_STRINGS} strings")));
            }
            let mut out = Vec::with_capacity(arr.len());
            for x in arr {
                let n = x.as_u64().filter(|n| *n <= 127).ok_or_else(|| bad(cmd, "each string is a MIDI pitch 0..=127"))?;
                out.push(u8::try_from(n).unwrap_or(127));
            }
            Some(out)
        }
        None => preset,
    };
    let capo = p.get("capo").map(|_| u8::try_from(i64_or(p, "capo", 0).clamp(0, i64::from(MAX_FRET))).unwrap_or(0));
    if strings.is_none() && capo.is_none() {
        return Err(bad(cmd, "pass `preset`, `strings` or `capo`"));
    }
    let ids = tab_tracks(e, cmd, p)?;
    let s = e.session_mut();
    let mut label = String::new();
    for id in &ids {
        if let Some(t) = s.track_mut(*id) {
            let mut setup = t.tab_setup().unwrap_or_default();
            if let Some(st) = &strings {
                setup.tuning.strings = st.clone();
            }
            if let Some(c) = capo {
                setup.tuning.capo = c;
            }
            setup.tuning = setup.tuning.sanitized();
            label = setup.tuning.label();
            t.tab = Some(setup);
        }
    }
    Ok(json!({"tracks": ids.iter().map(|t| t.0).collect::<Vec<_>>(), "tuning": label}))
}

fn tab_string(e: &mut Engine, p: &Value) -> Result<Value> {
    let cmd = "event.tab_string";
    let clip = super::clip_id(p, cmd)?;
    let index = p.get("note").and_then(Value::as_u64).and_then(|n| usize::try_from(n).ok()).ok_or_else(|| bad(cmd, "`note` index required"))?;
    let string = p.get("string").and_then(Value::as_u64).and_then(|n| u8::try_from(n).ok()).ok_or_else(|| bad(cmd, "`string` (1-based) required"))?;
    let s = e.session();
    let (tid, _) = s.find_clip(clip).ok_or_else(|| bad(cmd, format!("no clip {}", clip.0)))?;
    let tuning = s.track(tid).and_then(|t| t.tab_setup()).map(|t| t.tuning).ok_or_else(|| bad(cmd, "the clip is not on a tab track"))?;
    let c = e.session_mut().find_clip_mut(clip).ok_or_else(|| bad(cmd, format!("no clip {}", clip.0)))?;
    let ClipContent::Midi { sequence } = &mut c.content else { return Err(bad(cmd, "not a MIDI clip")) };
    if index >= sequence.notes.len() {
        return Err(bad(cmd, format!("the clip has {} notes", sequence.notes.len())));
    }
    if !soundcraft_midi::tab::pin(sequence, &tuning, index, string) {
        return Err(bad(cmd, format!("that note cannot be played on string {string}")));
    }
    soundcraft_midi::tab::prune(sequence);
    let fret = sequence.notes.get(index).and_then(|n| tuning.sanitized().fret(string, n.pitch));
    Ok(json!({"string": string, "fret": fret}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use soundcraft_model::{ChannelFormat, Session};

    #[test]
    fn tab_tracks_take_a_view_a_tuning_and_a_string() {
        let mut e = Engine::new(Session::default());
        let out = e.execute("track.new", &json!({"kind": "tab", "name": "Gtr"})).unwrap();
        let tid = e.session().tracks.iter().find(|t| t.kind == TrackKind::Tab).map(|t| t.id).unwrap();
        assert!(out.is_object());
        let t = e.session().track(tid).unwrap();
        assert_eq!(t.tab.as_ref().unwrap().view, TabView::Both);
        assert!(t.instrument.is_some(), "a tab track plays through an instrument");
        e.execute("track.tab_view", &json!({"track": tid.0, "view": "tab"})).unwrap();
        assert_eq!(e.session().track(tid).unwrap().tab.as_ref().unwrap().view, TabView::Tab);
        e.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(e.session().track(tid).unwrap().tab.as_ref().unwrap().view, TabView::Both);
        assert!(e.execute("track.tab_view", &json!({"track": tid.0, "view": "sideways"})).is_err());
        let r = e.execute("track.tab_tuning", &json!({"track": tid.0, "preset": "drop_d", "capo": 2})).unwrap();
        assert_eq!(r["tuning"], "D A D G B E (capo 2)");
        assert!(e.execute("track.tab_tuning", &json!({"track": tid.0, "strings": []})).is_err());
        assert!(e.execute("track.tab_tuning", &json!({"track": tid.0, "strings": [300]})).is_err());
        // A plain track is not a tab track.
        let other = e.session_mut().add_track(TrackKind::Audio, ChannelFormat::Mono, None);
        assert!(e.execute("track.tab_view", &json!({"track": other.0, "view": "tab"})).is_err());
        // Pin a note to a string.
        e.execute("track.tab_tuning", &json!({"track": tid.0, "preset": "guitar", "capo": 0})).unwrap();
        let s = e.session_mut();
        let cid = s.new_clip_id();
        let seq = soundcraft_midi::Sequence {
            notes: vec![soundcraft_midi::Note { pitch: 57, velocity: 90, release_velocity: 64, channel: 0, start: 0, length: 960 }],
            ..Default::default()
        };
        crate::edit::place_clip(s, tid, soundcraft_model::Clip::midi(cid, "c", 0, 48_000, seq));
        let r = e.execute("event.tab_string", &json!({"clip": cid.0, "note": 0, "string": 4})).unwrap();
        assert_eq!(r["fret"], 7);
        assert!(e.execute("event.tab_string", &json!({"clip": cid.0, "note": 0, "string": 1})).is_err(), "A3 is below the high E");
        assert!(e.execute("event.tab_string", &json!({"clip": cid.0, "note": 9, "string": 4})).is_err());
    }
}
