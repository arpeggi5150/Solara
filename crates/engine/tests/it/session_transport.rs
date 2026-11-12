use serde_json::json;
use soundcraft_engine::{Engine, TransportRequest, TransportStatus};
use soundcraft_model::Session;
use soundcraft_time::Range;

#[test]
fn replacing_session_resets_status_and_discards_old_transport_requests() {
    let mut engine = Engine::default();
    engine.transport = TransportStatus { playing: true, recording: true, position: 48_000 };
    engine.execute("transport.play", &json!({"from": 96_000})).unwrap();
    engine.execute("transport.record", &json!({})).unwrap();
    let mut session = Session::default();
    session.edit.selection = Range::point(12_000);
    engine.replace_session(session);
    assert_eq!(engine.transport, TransportStatus { position: 12_000, ..TransportStatus::default() });
    assert_eq!(engine.transport_requests, vec![TransportRequest::ResetSession]);
    assert!(!engine.is_dirty());
}

#[test]
fn failed_new_session_keeps_the_current_transport() {
    let mut engine = Engine::default();
    engine.transport = TransportStatus { playing: true, position: 48_000, ..TransportStatus::default() };
    engine.execute("transport.play", &json!({})).unwrap();
    let status = engine.transport;
    let requests = engine.transport_requests.clone();
    assert!(engine.execute("session.new", &json!({"sample_rate": 0})).is_err());
    assert_eq!(engine.transport, status);
    assert_eq!(engine.transport_requests, requests);
}

#[test]
fn the_drums_template_opens_clean_with_one_drums_track() {
    let mut engine = Engine::default();
    engine.execute("session.new", &json!({"template": "drums"})).unwrap();
    let tracks = &engine.session().tracks;
    assert_eq!(tracks.len(), 1);
    assert_eq!(tracks[0].name, "Drums");
    assert_eq!(tracks[0].kind, soundcraft_model::TrackKind::Instrument);
    assert!(!engine.is_dirty());
    assert!(engine.undo_label().is_none(), "the Drums track is not an undoable edit");
}

#[test]
fn the_empty_template_has_no_tracks_and_an_unknown_template_is_refused() {
    let mut engine = Engine::default();
    engine.execute("session.new", &json!({"template": "drums"})).unwrap();
    engine.execute("session.new", &json!({"template": "empty"})).unwrap();
    assert!(engine.session().tracks.is_empty());
    assert!(engine.execute("session.new", &json!({"template": "nope"})).is_err());
    assert!(engine.session().tracks.is_empty());
}

#[test]
fn new_projects_loop_until_loop_playback_is_turned_off() {
    let mut engine = Engine::default();
    for template in ["drums", "empty"] {
        engine.execute("session.new", &json!({"template": template})).unwrap();
        assert!(engine.session().edit.loop_playback, "{template}");
    }
    engine.execute("session.new", &json!({})).unwrap();
    assert!(!engine.session().edit.loop_playback, "a blank session keeps the old default");
}
