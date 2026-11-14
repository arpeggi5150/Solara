//! Track tags: categories on tracks, the smart scan that suggests them, and grouping that
//! follows them (Drums into a folder, vocals into main, dub and left/right folders inside a
//! Vocals folder, DI and mic sets of one instrument into one bus).
use super::track::{delete, folder_bus, move_into_folder, move_to_folder, unique_folder_bus_name};
use super::{CommandSpec, bad, bool_or, has_tracks, track_param, tracks_param, tracks_required};
use crate::tags::{self, BASS, DI, DRUMS, DUBS, GUITAR, KEYS, LEFT, MAINS, MIC, RIGHT, VOCALS};
use crate::{Engine, Result, cmd};
use serde_json::{Value, json};
use soundcraft_model::{ChannelFormat, Route, Session, TrackId, TrackKind, clean_tags};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "track.tags",
            "Set Track Tags",
            [],
            None,
            "{tracks?, tags: [\"Vocals\", \"Dubs\"] | \"Vocals, Dubs\"} (replaces the tags; [] clears them)",
            has_tracks,
            set_tags
        ),
        cmd!("track.tag_add", "Add Track Tag", [], None, "{tracks?, tag: \"Vocals\" | tags: [...]}", has_tracks, add_tags),
        cmd!("track.tag_remove", "Remove Track Tag", [], None, "{tracks?, tag: \"Vocals\" | tags: [...]}", has_tracks, remove_tags),
        cmd!(query "track.tag_scan", "Scan Tracks for Tags", [], None,
            "{tracks?, audio?: true} -> {suggestions: [{track, name, current, tags, reasons, listening}]} (every audio, instrument, MIDI and tab track when `tracks` is left out; nothing changes until track.tag_confirm)",
            has_tracks, scan),
        cmd!(
            "track.tag_confirm",
            "Confirm Track Tags",
            [],
            None,
            "{items: [{track, tags, suggested?}], group?: bool} (sets each track's tags; where they differ from `suggested`, the next scan learns the fix; `group` turns grouping by tags on or off first)",
            has_tracks,
            confirm
        ),
        cmd!(
            "track.tag_group",
            "Group Tracks by Tags",
            ["Track"],
            None,
            "{value?: true} (true groups tagged tracks now and keeps them grouped as tags change; false stops following tags and leaves the groups as they are)",
            has_tracks,
            group
        ),
    ]
}

/// Tags from `tags` (an array or a comma-separated string) or `tag`.
fn tags_param(p: &Value) -> Option<Vec<String>> {
    let raw: Vec<String> = match p.get("tags").or_else(|| p.get("tag"))? {
        Value::Array(a) => a.iter().take(256).filter_map(Value::as_str).map(str::to_string).collect(),
        Value::String(s) => s.split(',').take(256).map(str::to_string).collect(),
        _ => return None,
    };
    Some(clean_tags(raw.iter().map(String::as_str)))
}

/// Canonical spelling for a known tag typed in another case ("vocals" is Vocals).
fn spelled(tags: Vec<String>) -> Vec<String> {
    tags.into_iter().map(|t| tags::KNOWN.iter().find(|k| k.eq_ignore_ascii_case(&t)).map_or(t, |k| (*k).to_string())).collect()
}

fn edit_tags(e: &mut Engine, id: &str, p: &Value, f: impl Fn(&mut Vec<String>, &[String])) -> Result<Value> {
    let tracks = tracks_required(e, id, p)?;
    let given = spelled(tags_param(p).ok_or_else(|| bad(id, "`tags` or `tag` required"))?);
    let s = e.session_mut();
    for t in &tracks {
        if let Some(tr) = s.track_mut(*t) {
            f(&mut tr.tags, &given);
            tr.tags = clean_tags(tr.tags.iter().map(String::as_str));
        }
    }
    let moved = follow_tags(e)?;
    let s = e.session();
    let now: Vec<Value> = tracks.iter().filter_map(|t| s.track(*t)).map(|t| json!({"track": t.id, "name": t.name, "tags": t.tags})).collect();
    Ok(json!({"tracks": now, "moved": moved}))
}

fn set_tags(e: &mut Engine, p: &Value) -> Result<Value> {
    edit_tags(e, "track.tags", p, |tags, given| *tags = given.to_vec())
}

fn add_tags(e: &mut Engine, p: &Value) -> Result<Value> {
    edit_tags(e, "track.tag_add", p, |tags, given| tags.extend(given.iter().cloned()))
}

fn remove_tags(e: &mut Engine, p: &Value) -> Result<Value> {
    edit_tags(e, "track.tag_remove", p, |tags, given| tags.retain(|t| !given.iter().any(|g| g.eq_ignore_ascii_case(t))))
}

fn scan(e: &mut Engine, p: &Value) -> Result<Value> {
    let tracks = if p.get("tracks").is_some() || p.get("track").is_some() { tracks_param(e, "track.tag_scan", p)? } else { Vec::new() };
    let found = tags::scan(e.session(), &tracks, bool_or(p, "audio", true));
    Ok(json!({"suggestions": found}))
}

fn confirm(e: &mut Engine, p: &Value) -> Result<Value> {
    let id = "track.tag_confirm";
    let items = p.get("items").and_then(Value::as_array).ok_or_else(|| bad(id, "`items` required"))?;
    let mut plan = Vec::new();
    for item in items.iter().take(4096) {
        let t = track_param(e, id, item, "track")?.ok_or_else(|| bad(id, "every item needs a `track`"))?;
        let confirmed = spelled(tags_param(item).ok_or_else(|| bad(id, "every item needs `tags`"))?);
        let suggested = item.get("suggested").map(|_| spelled(tags_param(&json!({"tags": item.get("suggested")})).unwrap_or_default()));
        plan.push((t, confirmed, suggested));
    }
    let s = e.session_mut();
    if let Some(on) = p.get("group").and_then(Value::as_bool) {
        s.tagging.group = on;
    }
    let mut lessons = Vec::new();
    for (t, confirmed, suggested) in plan {
        let Some(name) = s.track(t).map(|t| t.name.clone()) else { continue };
        if let Some(suggested) = suggested {
            lessons.push((name, suggested, confirmed.clone()));
        }
        if let Some(tr) = s.track_mut(t) {
            tr.tags = confirmed;
        }
    }
    let before = s.tagging.clone();
    tags::learn_all(&mut s.tagging, &lessons);
    let learned = if before == s.tagging {
        0
    } else {
        lessons.iter().filter(|(_, a, b)| a.len() != b.len() || a.iter().any(|t| !b.iter().any(|u| u.eq_ignore_ascii_case(t)))).count()
    };
    let moved = follow_tags(e)?;
    Ok(json!({"confirmed": items.len().min(4096), "learned": learned, "moved": moved}))
}

fn group(e: &mut Engine, p: &Value) -> Result<Value> {
    let on = bool_or(p, "value", true);
    e.session_mut().tagging.group = on;
    let moved = follow_tags(e)?;
    let s = e.session();
    let groups: Vec<Value> = s
        .tracks
        .iter()
        .filter_map(|t| t.tag_group.as_ref().map(|g| (t, g)))
        .map(|(t, g)| {
            let members: Vec<&str> = s.tracks.iter().filter(|m| member_of(s, m.id).as_deref() == Some(g.as_str())).map(|m| m.name.as_str()).collect();
            json!({"group": g, "track": t.name, "members": members})
        })
        .collect();
    Ok(json!({"value": on, "moved": moved, "groups": groups}))
}

/// Where tags put a track.
#[derive(Debug, Clone, PartialEq)]
struct Place {
    /// Kept on the folder or aux track that holds the group ([`soundcraft_model::Track::tag_group`]).
    key: String,
    /// The name the group's track gets.
    name: String,
    /// A bus with an aux track (true) or a folder (false).
    bus: bool,
    /// For a folder inside another: that folder's key and name.
    parent: Option<(String, String)>,
}

fn has(tags: &[String], tag: &str) -> bool {
    tags.iter().any(|t| t.eq_ignore_ascii_case(tag))
}

/// The group a track with `tags` belongs in, if any. Drums come first (a kick mic is in the drum
/// folder, not a DI/mic bus), then vocals, then DI and mic sets.
fn place(tags: &[String]) -> Option<Place> {
    let folder = |key: &str, name: &str, parent: Option<(String, String)>| Some(Place { key: key.into(), name: name.into(), bus: false, parent });
    if has(tags, DRUMS) {
        return folder("folder:Drums", "Drums", None);
    }
    if has(tags, VOCALS) {
        let vocals = Some(("folder:Vocals".to_string(), "Vocals".to_string()));
        return if has(tags, LEFT) || has(tags, RIGHT) {
            folder("folder:Vocals/LR", "Vocal L-R", vocals)
        } else if has(tags, DUBS) {
            folder("folder:Vocals/Dubs", "Vocal Dubs", vocals)
        } else if has(tags, MAINS) {
            folder("folder:Vocals/Mains", "Vocal Mains", vocals)
        } else {
            folder("folder:Vocals", "Vocals", None)
        };
    }
    if has(tags, DI) || has(tags, MIC) {
        // One bus per instrument: the guitar's DI and mics together, the bass's on their own.
        let instrument = [GUITAR, BASS, KEYS].into_iter().find(|i| has(tags, i));
        let other = tags.iter().find(|t| ![DI, MIC, DUBS, MAINS, LEFT, RIGHT].iter().any(|k| k.eq_ignore_ascii_case(t)));
        let what = instrument.map(str::to_string).or_else(|| other.cloned());
        // A set is one performance taken through a DI and mics: a left and a right part are two.
        let side = if has(tags, LEFT) {
            " L"
        } else if has(tags, RIGHT) {
            " R"
        } else {
            ""
        };
        let name = what.map_or_else(|| format!("DI-Mic{side}"), |w| format!("{w}{side} DI-Mic"));
        return Some(Place { key: format!("bus:{name}"), name, bus: true, parent: None });
    }
    None
}

/// The track that holds group `key`.
fn holder(s: &Session, key: &str) -> Option<TrackId> {
    s.tracks.iter().find(|t| t.tag_group.as_deref() == Some(key)).map(|t| t.id)
}

/// The tag group `track` is in now: its folder's, or the bus its output feeds.
fn member_of(s: &Session, track: TrackId) -> Option<String> {
    let t = s.track(track)?;
    if let Some(g) = t.folder.and_then(|f| s.track(f)).and_then(|f| f.tag_group.clone()) {
        return Some(g);
    }
    let Route::Bus(bus) = t.mixer.output else { return None };
    s.tracks.iter().find(|a| a.id != track && a.kind == TrackKind::Aux && a.mixer.input == Route::Bus(bus)).and_then(|a| a.tag_group.clone())
}

/// When grouping follows tags, put every tagged track in its group, take untagged ones out, and
/// remove groups left empty. Returns how many tracks moved.
fn follow_tags(e: &mut Engine) -> Result<usize> {
    if !e.session().tagging.group {
        return Ok(0);
    }
    let s = e.session();
    let plan: Vec<(TrackId, Option<String>, Option<Place>)> =
        s.tracks.iter().filter(|t| tags::taggable(t)).map(|t| (t.id, member_of(s, t.id), place(&t.tags))).collect();
    let mut moved = 0;
    for (t, now, want) in plan {
        if now.as_deref() == want.as_ref().map(|p| p.key.as_str()) {
            continue;
        }
        if now.is_some() {
            leave(e, t)?;
        }
        if let Some(want) = want {
            join(e, t, &want)?;
        }
        moved += 1;
    }
    sweep(e)?;
    Ok(moved)
}

/// Take `t` out of its tag group.
fn leave(e: &mut Engine, t: TrackId) -> Result<()> {
    let s = e.session_mut();
    let Some(track) = s.track(t) else { return Ok(()) };
    let in_tag_folder = track.folder.and_then(|f| s.track(f)).is_some_and(|f| f.tag_group.is_some());
    if in_tag_folder {
        // Out to the folder above, or the top, and its output back to where that sums.
        let folder = track.folder;
        let parent = folder.and_then(|f| s.track(f)).and_then(|f| f.folder);
        let old = folder.and_then(|f| folder_bus(s, f));
        let back = parent.and_then(|p| folder_bus(s, p)).unwrap_or(Route::Main);
        if let Some(tr) = s.track_mut(t) {
            tr.folder = parent;
            if old.as_ref() == Some(&tr.mixer.output) {
                tr.mixer.output = back;
            }
        }
        if let Some(i) = s.track_index(t) {
            // Below the folder's block.
            let tr = s.tracks.remove(i);
            let at = folder.and_then(|f| s.track_index(f)).map_or(s.tracks.len(), |f| {
                let mut at = f + 1;
                while s.tracks.get(at).is_some_and(|c| c.folder == folder || c.folder.is_some_and(|cf| inside(s, cf, folder))) {
                    at += 1;
                }
                at
            });
            s.tracks.insert(at.min(s.tracks.len()), tr);
        }
    } else {
        let back = track.folder.and_then(|f| folder_bus(s, f)).unwrap_or(Route::Main);
        if let Some(tr) = s.track_mut(t) {
            tr.mixer.output = back;
        }
    }
    Ok(())
}

/// Whether folder `f` sits inside `outer`.
fn inside(s: &Session, f: TrackId, outer: Option<TrackId>) -> bool {
    let Some(outer) = outer else { return false };
    let mut cur = Some(f);
    for _ in 0..64 {
        match cur {
            Some(c) if c == outer => return true,
            Some(c) => cur = s.track(c).and_then(|t| t.folder),
            None => return false,
        }
    }
    false
}

/// Put `t` in group `want`, making the group's folder or bus when it does not exist yet.
fn join(e: &mut Engine, t: TrackId, want: &Place) -> Result<()> {
    if want.bus {
        let aux = match holder(e.session(), &want.key) {
            Some(a) => a,
            None => new_bus_group(e, t, want),
        };
        let s = e.session_mut();
        if let Some(bus) = s.track(aux).map(|a| a.mixer.input.clone())
            && let Some(tr) = s.track_mut(t)
        {
            tr.mixer.output = bus;
        }
        return Ok(());
    }
    if let Some(folder) = holder(e.session(), &want.key) {
        move_into_folder(e, &json!({"tracks": [t.0], "folder": folder.0}))?;
        return Ok(());
    }
    let made = move_to_folder(e, &json!({"tracks": [t.0], "name": want.name}))?;
    let folder = made.get("folder").and_then(Value::as_u64).map(TrackId).ok_or_else(|| bad("track.tag_group", "no folder made"))?;
    if let Some(f) = e.session_mut().track_mut(folder) {
        f.tag_group = Some(want.key.clone());
    }
    if let Some((key, name)) = &want.parent {
        match holder(e.session(), key) {
            Some(parent) => {
                move_into_folder(e, &json!({"tracks": [folder.0], "folder": parent.0}))?;
            }
            None => {
                let made = move_to_folder(e, &json!({"tracks": [folder.0], "name": name}))?;
                let parent = made.get("folder").and_then(Value::as_u64).map(TrackId);
                if let Some(p) = parent.and_then(|p| e.session_mut().track_mut(p)) {
                    p.tag_group = Some(key.clone());
                }
            }
        }
    }
    Ok(())
}

/// An aux track and its bus for a bus group, placed just below `first`.
fn new_bus_group(e: &mut Engine, first: TrackId, want: &Place) -> TrackId {
    let s = e.session_mut();
    let bus_name = unique_folder_bus_name(s, &want.name, None);
    let bus = s.add_bus(&bus_name, ChannelFormat::Stereo);
    let aux = s.add_track(TrackKind::Aux, ChannelFormat::Stereo, Some(&want.name));
    let parent = s.track(first).and_then(|t| t.folder);
    let out = parent.and_then(|p| folder_bus(s, p)).unwrap_or(Route::Main);
    if let Some(a) = s.track_mut(aux) {
        a.mixer.input = Route::Bus(bus);
        a.mixer.output = out;
        a.tag_group = Some(want.key.clone());
        a.folder = parent;
    }
    if let (Some(i), Some(at)) = (s.track_index(aux), s.track_index(first)) {
        let a = s.tracks.remove(i);
        let at = if i < at { at } else { at + 1 };
        s.tracks.insert(at.min(s.tracks.len()), a);
    }
    aux
}

/// Remove tag groups that hold nothing any more, innermost first.
fn sweep(e: &mut Engine) -> Result<()> {
    for _ in 0..8 {
        let s = e.session();
        let empty: Vec<TrackId> = s
            .tracks
            .iter()
            .filter(|t| t.tag_group.is_some())
            .filter(|g| {
                if g.kind == TrackKind::Aux {
                    !s.tracks.iter().any(|m| m.id != g.id && m.mixer.output == g.mixer.input && g.mixer.input != Route::None)
                } else {
                    !s.tracks.iter().any(|m| m.folder == Some(g.id))
                }
            })
            .map(|g| g.id)
            .collect();
        if empty.is_empty() {
            return Ok(());
        }
        let busses: Vec<Route> =
            empty.iter().filter_map(|g| s.track(*g)).filter(|g| g.kind == TrackKind::Aux).map(|g| g.mixer.input.clone()).collect();
        delete(e, &json!({"tracks": empty.iter().map(|t| t.0).collect::<Vec<_>>()}))?;
        // An aux group's bus goes with it once nothing else uses it.
        let s = e.session_mut();
        for route in busses {
            let Route::Bus(bus) = route else { continue };
            let used = s.tracks.iter().any(|t| {
                t.mixer.input == route
                    || t.mixer.output == route
                    || t.mixer.receive == Some(bus)
                    || t.mixer.sends.iter().flatten().any(|x| x.target == route)
            });
            if !used {
                s.busses.retain(|b| b.id != bus);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Engine;

    fn engine(names: &[&str]) -> Engine {
        let mut e = Engine::new(Session::default());
        for n in names {
            let r = e.execute("track.new", &json!({"count": 1, "format": "mono", "name": n})).unwrap();
            let _ = r;
        }
        e
    }

    fn tags_of(e: &Engine, name: &str) -> Vec<String> {
        e.session().track_by_name(name).unwrap().tags.clone()
    }

    fn folder_of(e: &Engine, name: &str) -> Option<String> {
        let s = e.session();
        s.track_by_name(name).unwrap().folder.and_then(|f| s.track(f)).map(|f| f.name.clone())
    }

    fn bus_of(e: &Engine, name: &str) -> Option<String> {
        let s = e.session();
        match s.track_by_name(name).unwrap().mixer.output {
            Route::Bus(b) => s.tracks.iter().find(|a| a.kind == TrackKind::Aux && a.mixer.input == Route::Bus(b)).map(|a| a.name.clone()),
            _ => None,
        }
    }

    fn suggestion(e: &mut Engine, name: &str) -> Vec<String> {
        let r = e.execute("track.tag_scan", &json!({"audio": false})).unwrap();
        let all = r["suggestions"].as_array().unwrap().clone();
        let one = all.iter().find(|g| g["name"] == name).unwrap();
        one["tags"].as_array().unwrap().iter().map(|t| t.as_str().unwrap().to_string()).collect()
    }

    #[test]
    fn tags_are_set_added_removed_and_saved() {
        let mut e = engine(&["Vox"]);
        e.execute("track.tags", &json!({"track": "Vox", "tags": "vocals, Mains , ,vocals"})).unwrap();
        assert_eq!(tags_of(&e, "Vox"), ["Vocals", "Mains"]);
        e.execute("track.tag_add", &json!({"track": "Vox", "tag": "Hamish"})).unwrap();
        e.execute("track.tag_remove", &json!({"track": "Vox", "tag": "mains"})).unwrap();
        assert_eq!(tags_of(&e, "Vox"), ["Vocals", "Hamish"]);
        let text = serde_json::to_string(e.session()).unwrap();
        let back: Session = serde_json::from_str(&text).unwrap();
        assert_eq!(back.track_by_name("Vox").unwrap().tags, ["Vocals", "Hamish"]);
        e.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(tags_of(&e, "Vox"), ["Vocals", "Mains", "Hamish"]);
        assert!(e.execute("track.tags", &json!({"track": "Vox"})).is_err());
    }

    #[test]
    fn the_scan_reads_names_and_learns_from_a_fix() {
        let mut e = engine(&["Vox Dub L", "Vox Dub R", "Lead Vox", "Kick In", "Gtr DI", "Gtr SM57", "Bass D.I.", "Hamish 2", "Lead Gtr"]);
        assert_eq!(suggestion(&mut e, "Vox Dub L"), ["Vocals", "Dubs", "L"]);
        assert_eq!(suggestion(&mut e, "Lead Vox"), ["Vocals", "Mains"]);
        assert_eq!(suggestion(&mut e, "Kick In"), ["Drums"]);
        assert_eq!(suggestion(&mut e, "Gtr DI"), ["Guitar", "DI"]);
        assert_eq!(suggestion(&mut e, "Gtr SM57"), ["Guitar", "Mic"]);
        assert_eq!(suggestion(&mut e, "Bass D.I."), ["Bass", "DI"]);
        assert_eq!(suggestion(&mut e, "Lead Gtr"), ["Guitar"]);
        assert!(suggestion(&mut e, "Hamish 2").is_empty());
        // The singer's name means a main vocal; "Lead Gtr" is really a DI.
        e.execute(
            "track.tag_confirm",
            &json!({"items": [
                {"track": "Hamish 2", "tags": ["Vocals", "Mains"], "suggested": []},
                {"track": "Lead Gtr", "tags": ["Guitar", "DI"], "suggested": ["Guitar"]},
                {"track": "Kick In", "tags": ["Drums"], "suggested": ["Drums"]},
            ]}),
        )
        .unwrap();
        assert_eq!(tags_of(&e, "Hamish 2"), ["Vocals", "Mains"]);
        assert_eq!(suggestion(&mut e, "Hamish 2"), ["Vocals", "Mains"]);
        assert_eq!(suggestion(&mut e, "Lead Gtr"), ["Guitar", "DI"]);
        // The learned word carries to a new track: "Hamish Dub" is a vocal dub.
        e.execute("track.new", &json!({"count": 1, "format": "mono", "name": "Hamish Dub"})).unwrap();
        assert_eq!(suggestion(&mut e, "Hamish Dub"), ["Vocals", "Dubs"]);
        // Taking a suggestion as it was teaches nothing.
        assert!(!e.session().tagging.names.contains_key("kick in"));
    }

    #[test]
    fn tags_drive_grouping_and_a_retag_moves_the_track() {
        let mut e = engine(&[
            "Lead Vox",
            "Vox Dub",
            "Vox Dbl L",
            "Vox Dbl R",
            "Kick",
            "Snare",
            "Gtr DI",
            "Gtr Mic",
            "Bass DI",
            "Bass Amp",
            "Keys",
            "Left Gtr DI",
            "Left Gtr Amp",
        ]);
        let found = e.execute("track.tag_scan", &json!({"audio": false})).unwrap();
        let items: Vec<Value> = found["suggestions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|g| json!({"track": g["track"], "tags": g["tags"], "suggested": g["tags"]}))
            .collect();
        e.execute("track.tag_confirm", &json!({"items": items})).unwrap();
        e.execute("track.tag_group", &json!({})).unwrap();
        assert_eq!(folder_of(&e, "Kick").as_deref(), Some("Drums"));
        assert_eq!(folder_of(&e, "Snare").as_deref(), Some("Drums"));
        assert_eq!(folder_of(&e, "Lead Vox").as_deref(), Some("Vocal Mains"));
        assert_eq!(folder_of(&e, "Vox Dub").as_deref(), Some("Vocal Dubs"));
        assert_eq!(folder_of(&e, "Vox Dbl L").as_deref(), Some("Vocal L-R"));
        assert_eq!(folder_of(&e, "Vox Dbl R").as_deref(), Some("Vocal L-R"));
        assert_eq!(folder_of(&e, "Vocal Dubs").as_deref(), Some("Vocals"));
        assert_eq!(bus_of(&e, "Gtr DI").as_deref(), Some("Guitar DI-Mic"));
        assert_eq!(bus_of(&e, "Gtr Mic").as_deref(), Some("Guitar DI-Mic"));
        assert_eq!(bus_of(&e, "Bass Amp").as_deref(), Some("Bass DI-Mic"));
        assert_eq!(bus_of(&e, "Left Gtr DI").as_deref(), Some("Guitar L DI-Mic"));
        assert_eq!(bus_of(&e, "Left Gtr Amp").as_deref(), Some("Guitar L DI-Mic"));
        assert_eq!(folder_of(&e, "Keys"), None);
        assert_eq!(e.session().track_by_name("Keys").unwrap().mixer.output, Route::Main);
        // Folder members follow their folder: the summing bus of the folder, not the main out.
        let s = e.session();
        let drums = s.track_by_name("Drums").unwrap();
        assert_eq!(s.track_by_name("Kick").unwrap().mixer.output, drums.mixer.input);
        // Grouping again changes nothing.
        let before = e.session().clone();
        let r = e.execute("track.tag_group", &json!({})).unwrap();
        assert_eq!(r["moved"], 0);
        assert_eq!(e.session().tracks, before.tracks);

        // Retag the dub as a main: it moves, and the emptied dub folder goes.
        e.execute("track.tags", &json!({"track": "Vox Dub", "tags": ["Vocals", "Mains"]})).unwrap();
        assert_eq!(folder_of(&e, "Vox Dub").as_deref(), Some("Vocal Mains"));
        assert!(e.session().track_by_name("Vocal Dubs").is_none());
        // A guitar mic retagged as a bass mic changes bus; untagging takes a track out.
        e.execute("track.tags", &json!({"track": "Gtr Mic", "tags": "Bass, Mic"})).unwrap();
        assert_eq!(bus_of(&e, "Gtr Mic").as_deref(), Some("Bass DI-Mic"));
        e.execute("track.tags", &json!({"track": "Kick", "tags": []})).unwrap();
        assert_eq!(folder_of(&e, "Kick"), None);
        assert_eq!(e.session().track_by_name("Kick").unwrap().mixer.output, Route::Main);
        e.execute("track.tags", &json!({"tracks": ["Gtr DI", "Gtr Mic", "Bass DI", "Bass Amp", "Left Gtr DI", "Left Gtr Amp"], "tags": []})).unwrap();
        let s = e.session();
        assert!(s.tracks.iter().all(|t| t.kind != TrackKind::Aux), "the emptied DI-Mic busses go");
        assert!(!s.busses.iter().any(|b| b.name.contains("DI-Mic")));
        // Stopping leaves the groups as they are.
        e.execute("track.tag_group", &json!({"value": false})).unwrap();
        e.execute("track.tags", &json!({"track": "Snare", "tags": []})).unwrap();
        assert_eq!(folder_of(&e, "Snare").as_deref(), Some("Drums"));
    }

    #[test]
    fn hostile_tag_params_are_errors_not_crashes() {
        let mut e = engine(&["Vox"]);
        assert!(e.execute("track.tags", &json!({"track": "Vox", "tags": 5})).is_err());
        assert!(e.execute("track.tag_confirm", &json!({"items": [{"track": 999_999, "tags": []}]})).is_err());
        assert!(e.execute("track.tag_confirm", &json!({"items": "x"})).is_err());
        let huge: Vec<String> = (0..10_000).map(|i| format!("{i}{}", "x".repeat(100))).collect();
        e.execute("track.tags", &json!({"track": "Vox", "tags": huge})).unwrap();
        assert_eq!(tags_of(&e, "Vox").len(), soundcraft_model::MAX_TAGS);
        e.execute("track.tag_confirm", &json!({"items": [{"track": "Vox", "tags": ["a"], "suggested": 7}]})).unwrap();
    }
}
