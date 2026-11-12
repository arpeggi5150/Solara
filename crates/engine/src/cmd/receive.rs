//! Receives: an audio, instrument or MIDI track mixes in a bus of its own, so other tracks can
//! send to it the way they send to an aux (the Reaper-style layout's Routing Matrix). Aux and
//! folder tracks already receive through their input bus.
use super::{CommandSpec, bad, bool_or, has_selection, tracks_required};
use crate::{Engine, Result, cmd};
use serde_json::{Value, json};
use soundcraft_model::{BusId, ChannelFormat, Route, Session, TrackKind};

pub fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        "mix.receive",
        "Track Receive",
        [],
        None,
        "{track?, value?: true} (true gives the track a receive bus named \"<track> Receive\"; false takes it away)",
        has_selection,
        receive
    )]
}

/// Most tries for a free bus name.
const NAME_TRIES: usize = 1000;

fn receive(e: &mut Engine, p: &Value) -> Result<Value> {
    let id = "mix.receive";
    let t = tracks_required(e, id, p)?.first().copied().ok_or_else(|| bad(id, "no track"))?;
    let on = bool_or(p, "value", true);
    let s = e.session_mut();
    let track = s.track(t).ok_or_else(|| bad(id, "no track"))?;
    if !matches!(track.kind, TrackKind::Audio | TrackKind::Instrument | TrackKind::Midi) {
        return Err(bad(id, "only audio, instrument and MIDI tracks take a receive; aux and folder tracks receive through their input"));
    }
    let (name, current) = (track.name.clone(), track.mixer.receive);
    if !on {
        if let Some(bus) = current {
            if let Some(tr) = s.track_mut(t) {
                tr.mixer.receive = None;
            }
            // The bus was this track's own: once nothing uses it, it goes too.
            if !in_use(s, bus) {
                s.busses.retain(|b| b.id != bus);
            }
        }
        return Ok(json!({"receive": false}));
    }
    if let Some(bus) = current.and_then(|b| s.bus(b)) {
        return Ok(json!({"receive": true, "bus": bus.name}));
    }
    let base = format!("{name} Receive");
    let bus_name = (1..=NAME_TRIES)
        .map(|n| if n == 1 { base.clone() } else { format!("{base} {n}") })
        .find(|n| s.bus_by_name(n).is_none() && s.track_by_name(n).is_none())
        .ok_or_else(|| bad(id, "no free bus name"))?;
    let bus = s.add_bus(&bus_name, ChannelFormat::Stereo);
    if let Some(tr) = s.track_mut(t) {
        tr.mixer.receive = Some(bus);
    }
    Ok(json!({"receive": true, "bus": bus_name}))
}

/// Whether any track reads, feeds or receives `bus`.
fn in_use(s: &Session, bus: BusId) -> bool {
    let r = Route::Bus(bus);
    s.tracks
        .iter()
        .any(|t| t.mixer.input == r || t.mixer.output == r || t.mixer.receive == Some(bus) || t.mixer.sends.iter().flatten().any(|x| x.target == r))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn engine() -> (Engine, u64, u64) {
        let mut e = Engine::default();
        e.execute("track.new", &json!({"kind": "audio", "name": "Gtr", "format": "stereo"})).unwrap();
        e.execute("track.new", &json!({"kind": "instrument", "name": "Keys"})).unwrap();
        let (gtr, keys) = (e.session().track_by_name("Gtr").unwrap().id.0, e.session().track_by_name("Keys").unwrap().id.0);
        (e, gtr, keys)
    }

    #[test]
    fn a_receive_makes_a_bus_other_tracks_can_send_to() {
        let (mut e, gtr, keys) = engine();
        let r = e.execute("mix.receive", &json!({"track": keys})).unwrap();
        assert_eq!(r, json!({"receive": true, "bus": "Keys Receive"}));
        // Asking again keeps the same bus.
        assert_eq!(e.execute("mix.receive", &json!({"track": keys})).unwrap(), r);
        e.execute("mix.send", &json!({"track": gtr, "bus": "Keys Receive", "level_db": 0.0})).unwrap();
        let s = e.session();
        let bus = s.bus_by_name("Keys Receive").unwrap().id;
        assert_eq!(s.track_by_name("Keys").unwrap().mixer.receive, Some(bus));
    }

    #[test]
    fn taking_a_receive_away_drops_its_bus_once_unused() {
        let (mut e, gtr, keys) = engine();
        e.execute("mix.receive", &json!({"track": keys})).unwrap();
        e.execute("mix.send", &json!({"track": gtr, "slot": 0, "bus": "Keys Receive"})).unwrap();
        e.execute("mix.send_remove", &json!({"track": gtr, "slot": 0})).unwrap();
        e.execute("mix.receive", &json!({"track": keys, "value": false})).unwrap();
        assert!(e.session().bus_by_name("Keys Receive").is_none());
        assert_eq!(e.session().track_by_name("Keys").unwrap().mixer.receive, None);
    }

    #[test]
    fn a_receive_bus_still_sent_to_stays() {
        let (mut e, gtr, keys) = engine();
        e.execute("mix.receive", &json!({"track": keys})).unwrap();
        e.execute("mix.send", &json!({"track": gtr, "slot": 0, "bus": "Keys Receive"})).unwrap();
        e.execute("mix.receive", &json!({"track": keys, "value": false})).unwrap();
        assert!(e.session().bus_by_name("Keys Receive").is_some(), "a send still uses it");
        assert_eq!(e.session().track_by_name("Keys").unwrap().mixer.receive, None);
    }

    #[test]
    fn aux_tracks_refuse_a_receive() {
        let (mut e, _, _) = engine();
        e.execute("track.new", &json!({"kind": "aux", "name": "Verb"})).unwrap();
        let verb = e.session().track_by_name("Verb").unwrap().id.0;
        assert!(e.execute("mix.receive", &json!({"track": verb})).is_err());
    }
}
