use super::*;

/// A small score in the shape Guitar Pro 7 files have: a guitar (standard tuning, capo 0) and a
/// drum kit, two 4/4 bars repeated once, a tempo of 96, a chord, a rest, a tie over the bar line
/// and a dotted half.
const GPIF: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<GPIF><GPVersion>8.1.3</GPVersion>
<Score><Title><![CDATA[Garden Test]]></Title><Artist><![CDATA[Zee]]></Artist></Score>
<MasterTrack><Tracks>0 1</Tracks><Automations><Automation><Type>Tempo</Type><Linear>false</Linear><Bar>0</Bar><Position>0</Position><Visible>true</Visible><Value>96 2</Value></Automation></Automations></MasterTrack>
<Tracks>
<Track id="0"><Name><![CDATA[Electric guitar]]></Name><Color>10 20 30</Color><InstrumentSet><Name>Electric Guitar</Name><Type>electricGuitar</Type></InstrumentSet>
<MidiConnection><Port>1</Port><PrimaryChannel>0</PrimaryChannel><SecondaryChannel>1</SecondaryChannel></MidiConnection>
<Staves><Staff><Properties><Property name="CapoFret"><Fret>0</Fret></Property><Property name="Tuning"><Pitches>40 45 50 55 59 64</Pitches></Property></Properties></Staff></Staves>
<Sounds><Sound><MIDI><LSB>0</LSB><MSB>0</MSB><Program>29</Program></MIDI></Sound></Sounds></Track>
<Track id="1"><Name><![CDATA[Drums]]></Name><Color>0 0 200</Color><InstrumentSet><Name>Drums</Name><Type>drumKit</Type><Elements><Element><Name>Drums</Name><Articulations>
<Articulation><Name>Kick</Name><OutputMidiNumber>36</OutputMidiNumber></Articulation>
<Articulation><Name>Hat</Name><OutputMidiNumber>42</OutputMidiNumber></Articulation>
</Articulations></Element></Elements></InstrumentSet>
<MidiConnection><Port>1</Port><PrimaryChannel>9</PrimaryChannel><SecondaryChannel>9</SecondaryChannel></MidiConnection>
<Sounds><Sound><MIDI><Program>0</Program></MIDI></Sound></Sounds></Track>
</Tracks>
<MasterBars>
<MasterBar><Time>4/4</Time><Repeat start="true" end="false" count="0"/><Bars>0 2</Bars></MasterBar>
<MasterBar><Time>4/4</Time><Repeat start="false" end="true" count="2"/><Bars>1 3</Bars></MasterBar>
</MasterBars>
<Bars>
<Bar id="0"><Voices>0 -1 -1 -1</Voices><Clef>G2</Clef></Bar>
<Bar id="1"><Voices>1 -1 -1 -1</Voices><Clef>G2</Clef></Bar>
<Bar id="2"><Voices>2 -1 -1 -1</Voices><Clef>Neutral</Clef></Bar>
<Bar id="3"><Voices>-1 -1 -1 -1</Voices><Clef>Neutral</Clef></Bar>
</Bars>
<Voices>
<Voice id="0"><Beats>0 1 2 3</Beats></Voice>
<Voice id="1"><Beats>4 5</Beats></Voice>
<Voice id="2"><Beats>6 7</Beats></Voice>
</Voices>
<Beats>
<Beat id="0"><Dynamic>F</Dynamic><Rhythm ref="1"/><Notes>0</Notes></Beat>
<Beat id="1"><Dynamic>MF</Dynamic><Rhythm ref="0"/><Notes>1 2 3</Notes></Beat>
<Beat id="2"><Dynamic>F</Dynamic><Rhythm ref="1"/></Beat>
<Beat id="3"><Dynamic>F</Dynamic><Rhythm ref="2"/><Notes>4</Notes></Beat>
<Beat id="4"><Dynamic>F</Dynamic><Rhythm ref="0"/><Notes>5</Notes></Beat>
<Beat id="5"><Dynamic>F</Dynamic><Rhythm ref="3"/></Beat>
<Beat id="6"><Dynamic>F</Dynamic><Rhythm ref="2"/><Notes>6 7</Notes></Beat>
<Beat id="7"><Dynamic>F</Dynamic><Rhythm ref="2"/><Notes>8</Notes></Beat>
</Beats>
<Notes>
<Note id="0"><Properties><Property name="String"><String>0</String></Property><Property name="Fret"><Fret>0</Fret></Property><Property name="Midi"><Number>40</Number></Property></Properties></Note>
<Note id="1"><Properties><Property name="String"><String>1</String></Property><Property name="Fret"><Fret>2</Fret></Property><Property name="Midi"><Number>47</Number></Property></Properties></Note>
<Note id="2"><Properties><Property name="String"><String>2</String></Property><Property name="Fret"><Fret>2</Fret></Property><Property name="Midi"><Number>52</Number></Property></Properties></Note>
<Note id="3"><Properties><Property name="String"><String>3</String></Property><Property name="Fret"><Fret>1</Fret></Property><Property name="Midi"><Number>56</Number></Property></Properties></Note>
<Note id="4"><Tie origin="true" destination="false"/><Properties><Property name="String"><String>4</String></Property><Property name="Fret"><Fret>5</Fret></Property><Property name="Midi"><Number>64</Number></Property></Properties></Note>
<Note id="5"><Tie origin="false" destination="true"/><Properties><Property name="String"><String>4</String></Property><Property name="Fret"><Fret>5</Fret></Property><Property name="Midi"><Number>64</Number></Property></Properties></Note>
<Note id="6"><Properties><Property name="Midi"><Number>36</Number></Property></Properties><InstrumentArticulation>0</InstrumentArticulation></Note>
<Note id="7"><Properties><Property name="Midi"><Number>99</Number></Property></Properties><InstrumentArticulation>1</InstrumentArticulation></Note>
<Note id="8"><Properties><Property name="Midi"><Number>36</Number></Property></Properties><InstrumentArticulation>0</InstrumentArticulation></Note>
</Notes>
<Rhythms>
<Rhythm id="0"><NoteValue>Quarter</NoteValue></Rhythm>
<Rhythm id="1"><NoteValue>Eighth</NoteValue></Rhythm>
<Rhythm id="2"><NoteValue>Half</NoteValue></Rhythm>
<Rhythm id="3"><NoteValue>Half</NoteValue><AugmentationDot count="1"/></Rhythm>
</Rhythms>
</GPIF>"#;

fn tabgarden_file(gpif: &str, manifest: &Value) -> Vec<u8> {
    let gp = write_gp(gpif).unwrap();
    let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    w.start_file("tabgarden.json", opts).unwrap();
    w.write_all(serde_json::to_string_pretty(manifest).unwrap().as_bytes()).unwrap();
    w.start_file("score.gp", opts).unwrap();
    w.write_all(&gp).unwrap();
    w.finish().unwrap().into_inner()
}

fn manifest() -> Value {
    json!({"format": "tabgarden", "formatVersion": 1, "app": "TabGarden", "title": "Garden Test", "instruments": {"0": {"id": "gm:30", "kamiRule": false}}, "files": {"gp": "score.gp", "mid": null, "original": null}})
}

fn notes_of(s: &Session, i: usize) -> Sequence {
    let t = &s.tracks[i];
    match &t.clips()[0].content {
        ClipContent::Midi { sequence } => sequence.clone(),
        _ => panic!("not MIDI"),
    }
}

#[test]
fn a_score_opens_with_every_track_its_tuning_strings_and_repeats() {
    let score = gpif::parse_gpif(GPIF).unwrap();
    assert_eq!(score.tracks.len(), 2);
    let s = session_from_score(&score, Some(&manifest()), "x").unwrap();
    assert_eq!(s.name, "Garden Test");
    assert_eq!(s.tracks.len(), 2);
    let (gtr, drums) = (&s.tracks[0], &s.tracks[1]);
    assert_eq!(gtr.kind, TrackKind::Tab);
    assert_eq!(gtr.name, "Electric guitar");
    assert_eq!(gtr.color, [10, 20, 30]);
    let tab = gtr.tab.clone().unwrap();
    assert_eq!(tab.tuning.strings, [64, 59, 55, 50, 45, 40], "string 1 is the high E");
    assert_eq!(tab.program, 30, "TabGarden's pick wins over the file's program");
    assert_eq!(drums.kind, TrackKind::Instrument);
    assert_eq!(drums.instrument.as_ref().unwrap().plugin, "drum_synth");
    assert!((s.tempo.tempo_at_tick(0) - 96.0).abs() < 1e-9);
    // Bars 1-2 repeated: 4 bars, so the clips run 4 × 3840 ticks.
    let seq = notes_of(&s, 0);
    let starts: Vec<(i64, u8, i64)> = seq.notes.iter().map(|n| (n.start, n.pitch, n.length)).collect();
    // Bar 1: E2 (8th), chord (q), rest (8th), E4 (half, tied into bar 2's quarter), then the repeat.
    assert!(starts.contains(&(0, 40, 480)));
    assert!(starts.contains(&(480, 47, 960)) && starts.contains(&(480, 52, 960)) && starts.contains(&(480, 56, 960)));
    assert!(starts.contains(&(1920, 64, 1920 + 960)), "the tie joins the half and the quarter: {starts:?}");
    assert!(starts.contains(&(7680, 40, 480)), "the repeat plays bar 1 again");
    assert_eq!(seq.notes.len(), 10);
    // The tab shows the strings the file wrote: the E4 on the B string at fret 5.
    let placed = place(&seq, &tab.tuning);
    let e4 = seq.notes.iter().position(|n| n.start == 1920 && n.pitch == 64).unwrap();
    assert_eq!((placed[e4].string, placed[e4].fret), (2, 5));
    let e2 = seq.notes.iter().position(|n| n.start == 0).unwrap();
    assert_eq!((placed[e2].string, placed[e2].fret), (6, 0));
    // Drums: the articulation's key, not the note's own number, on channel 10.
    let d = notes_of(&s, 1);
    assert!(d.notes.iter().all(|n| n.channel == 9));
    assert!(d.notes.iter().any(|n| n.pitch == 42) && !d.notes.iter().any(|n| n.pitch == 99));
}

#[test]
fn a_tabgarden_file_opens_as_the_session() {
    let mut e = Engine::new(Session::default());
    let bytes = tabgarden_file(GPIF, &manifest());
    let out = open_tabgarden_bytes(&mut e, "song.tabgarden", &bytes).unwrap();
    assert_eq!(out["tracks"].as_array().unwrap().len(), 2);
    assert_eq!(out["tracks"][0]["kind"], "tab");
    assert_eq!(out["tracks"][0]["tuning"], "E A D G B E");
    assert_eq!(e.session().tracks.len(), 2);
    assert!(!e.is_dirty());
}

#[test]
fn bad_files_are_errors_not_crashes() {
    let mut e = Engine::new(Session::default());
    for bytes in [Vec::new(), b"PK\x03\x04junk".to_vec(), vec![0u8; 100]] {
        assert!(open_tabgarden_bytes(&mut e, "x.tabgarden", &bytes).is_err());
    }
    // Wrong format, a newer version, and a missing score.
    let mut wrong = manifest();
    wrong["format"] = json!("other");
    assert!(open_tabgarden_bytes(&mut e, "x", &tabgarden_file(GPIF, &wrong)).is_err());
    let mut newer = manifest();
    newer["formatVersion"] = json!(2);
    let err = open_tabgarden_bytes(&mut e, "x", &tabgarden_file(GPIF, &newer)).unwrap_err().to_string();
    assert!(err.contains("newer"), "{err}");
    let mut missing = manifest();
    missing["files"]["gp"] = json!("nope.gp");
    assert!(open_tabgarden_bytes(&mut e, "x", &tabgarden_file(GPIF, &missing)).is_err());
    // Scores that are not scores.
    for gpif in ["<GPIF/>", "<NotGP/>", "<GPIF><Tracks><Track id=\"0\"/></Tracks></GPIF>", "<GPIF><Tracks>", "\u{0}"] {
        assert!(open_tabgarden_bytes(&mut e, "x", &tabgarden_file(gpif, &manifest())).is_err(), "{gpif:?}");
    }
    // Hostile numbers: dangling ids, giant meters, tuplets of zero, huge tempos — opens safely.
    let hostile = GPIF
        .replace("<Time>4/4</Time>", "<Time>999/3</Time>")
        .replace("<Beats>0 1 2 3</Beats>", "<Beats>0 1 2 3 77 -5 99999999999999999999</Beats>")
        .replace("96 2", "1e308 9")
        .replace("count=\"2\"", "count=\"4000000000\"");
    let s = session_from_score(&gpif::parse_gpif(&hostile).unwrap(), None, "x").unwrap();
    assert_eq!(s.tracks.len(), 2);
}

#[test]
fn the_kami_rule_plays_white_keys_and_survives_export() {
    let mut e = Engine::new(Session::default());
    let mut m = manifest();
    m["instruments"]["0"]["kamiRule"] = json!(true);
    open_tabgarden_bytes(&mut e, "song.tabgarden", &tabgarden_file(GPIF, &m)).unwrap();
    let t = e.session().tracks[0].id;
    assert_eq!(e.session().edit.value(&naturals_key(t), 0.0), 1.0, "played on the white keys");
    assert_eq!(e.session().edit.value(&naturals_key(e.session().tracks[1].id), 0.0), 0.0);
    let file = read_tabgarden(&export_tabgarden(&e, None).unwrap()).unwrap();
    assert_eq!(file.manifest["instruments"]["0"]["kamiRule"], true);
    assert_eq!(file.manifest["instruments"]["1"]["kamiRule"], false);
}

#[test]
fn exported_stems_open_again_with_the_same_notes_and_strings() {
    let mut e = Engine::new(Session::default());
    open_tabgarden_bytes(&mut e, "song.tabgarden", &tabgarden_file(GPIF, &manifest())).unwrap();
    let bytes = export_tabgarden(&e, None).unwrap();
    // Every entry is stored, as TabGarden requires; the manifest names them.
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(&bytes)).unwrap();
    let names: Vec<String> = (0..z.len()).map(|i| z.by_index(i).unwrap().name().unwrap().to_string()).collect();
    assert_eq!(names, ["tabgarden.json", "score.gp", "score.mid"]);
    for i in 0..z.len() {
        assert_eq!(z.by_index(i).unwrap().compression(), zip::CompressionMethod::Stored);
    }
    let file = read_tabgarden(&bytes).unwrap();
    assert_eq!(file.manifest["formatVersion"], 1);
    assert_eq!(file.manifest["instruments"]["0"]["id"], "gm:30");
    assert_eq!(file.manifest["instruments"]["1"]["id"], "gmkit:0");
    assert!(file.manifest["savedAt"].as_str().unwrap().ends_with('Z'));
    // score.mid holds both stems.
    let mid = entry(&unzip(&bytes, "t").unwrap(), "score.mid").unwrap().to_vec();
    assert_eq!(soundcraft_midi::read_smf(&mid).unwrap().tracks.iter().filter(|t| !t.sequence.notes.is_empty()).count(), 2);
    // Open the export: same tracks, notes, tuning and strings.
    let before = e.session().clone();
    let mut again = Engine::new(Session::default());
    open_tabgarden_bytes(&mut again, "stems.tabgarden", &bytes).unwrap();
    let after = again.session();
    assert_eq!(after.tracks.len(), 2);
    assert_eq!(after.tracks[0].kind, TrackKind::Tab);
    assert_eq!(after.tracks[0].tab.as_ref().unwrap().tuning, before.tracks[0].tab.as_ref().unwrap().tuning);
    for i in 0..2 {
        let (a, b) = (notes_of(&before, i), notes_of(after, i));
        let key = |q: &Sequence| q.notes.iter().map(|n| (n.start, n.pitch, n.length, n.channel)).collect::<Vec<_>>();
        assert_eq!(key(&a), key(&b), "track {i}");
        if i == 0 {
            let tuning = &after.tracks[0].tab.as_ref().unwrap().tuning;
            let pa: Vec<_> = place(&a, tuning).iter().map(|p| (p.string, p.fret)).collect();
            let pb: Vec<_> = place(&b, tuning).iter().map(|p| (p.string, p.fret)).collect();
            assert_eq!(pa, pb);
        }
    }
    assert!((after.tempo.tempo_at_tick(0) - 96.0).abs() < 1e-6);
}

#[test]
fn a_plain_midi_track_exports_as_a_tab_score() {
    let mut e = Engine::new(Session::default());
    let t = e.session_mut().add_track(TrackKind::Instrument, ChannelFormat::Stereo, Some("Keys"));
    let notes = vec![
        Note { pitch: 60, velocity: 100, release_velocity: 64, channel: 0, start: 10, length: 900 },
        Note { pitch: 72, velocity: 100, release_velocity: 64, channel: 0, start: 3000, length: 2000 },
    ];
    let s = e.session_mut();
    let cid = s.new_clip_id();
    let len = s.tempo.tick_to_samples(8000, s.sample_rate);
    crate::edit::place_clip(s, t, Clip::midi(cid, "Keys", 0, len, Sequence { notes, ..Sequence::default() }));
    let bytes = export_tabgarden(&e, None).unwrap();
    let file = read_tabgarden(&bytes).unwrap();
    assert_eq!(file.score.tracks.len(), 1);
    assert_eq!(file.score.tracks[0].kind, GpKind::Other);
    let mut again = Engine::new(Session::default());
    open_tabgarden_bytes(&mut again, "k.tabgarden", &bytes).unwrap();
    let seq = notes_of(again.session(), 0);
    // Quantized to sixteenths; the long note crosses the bar line and is tied back together.
    let got: Vec<(i64, u8, i64)> = seq.notes.iter().map(|n| (n.start, n.pitch, n.length)).collect();
    assert_eq!(got, [(0, 60, 960), (3120, 72, 1920)]);
    assert!(export_tabgarden(&Engine::new(Session::default()), None).is_err(), "nothing to export");
}

/// Real scores, when `SOLARA_GP_SAMPLES` names a folder of `.gp` / `.tabgarden` files: each opens,
/// exports and opens again with the same notes.
#[test]
fn sample_files_open_and_round_trip() {
    let Ok(dir) = std::env::var("SOLARA_GP_SAMPLES") else { return };
    let mut seen = 0;
    for f in std::fs::read_dir(dir).unwrap().flatten() {
        let path = f.path();
        let ext = path.extension().and_then(|x| x.to_str()).unwrap_or("").to_ascii_lowercase();
        let bytes = std::fs::read(&path).unwrap();
        let mut e = Engine::new(Session::default());
        let summary = match ext.as_str() {
            "gp" => {
                let score = read_gp(&bytes).unwrap();
                e.replace_session(session_from_score(&score, None, "x").unwrap());
                json!({"tracks": score.tracks.len()})
            }
            "tabgarden" => open_tabgarden_bytes(&mut e, "x", &bytes).unwrap(),
            _ => continue,
        };
        seen += 1;
        let out = export_tabgarden(&e, None).unwrap();
        let mut again = Engine::new(Session::default());
        open_tabgarden_bytes(&mut again, "x", &out).unwrap();
        assert_eq!(again.session().tracks.len(), e.session().tracks.len(), "{}", path.display());
        let count = |s: &Session| {
            s.tracks
                .iter()
                .map(|t| {
                    t.clips().iter().map(|c| if let ClipContent::Midi { sequence } = &c.content { sequence.notes.len() } else { 0 }).sum::<usize>()
                })
                .collect::<Vec<_>>()
        };
        eprintln!("{}: {summary} notes {:?} -> {:?}", path.display(), count(e.session()), count(again.session()));
    }
    assert!(seen > 0);
}

/// One guitar, two 4/4 bars of playing techniques: a palm mute, a dead note, a grace note before a
/// hammer-on, a legato slide, tremolo picking, a tie with nothing before it and an accent.
const TECHNIQUES: &str = r#"<GPIF><Score><Title><![CDATA[Techniques]]></Title></Score>
<MasterTrack><Tracks>0</Tracks></MasterTrack>
<Tracks><Track id="0"><Name><![CDATA[Gtr]]></Name><InstrumentSet><Type>electricGuitar</Type></InstrumentSet>
<MidiConnection><PrimaryChannel>0</PrimaryChannel></MidiConnection>
<Staves><Staff><Properties><Property name="Tuning"><Pitches>40 45 50 55 59 64</Pitches></Property></Properties></Staff></Staves></Track></Tracks>
<MasterBars>
<MasterBar><Time>4/4</Time><Bars>0</Bars></MasterBar>
<MasterBar><Time>4/4</Time><Bars>1</Bars></MasterBar>
</MasterBars>
<Bars><Bar id="0"><Voices>0 -1 -1 -1</Voices></Bar><Bar id="1"><Voices>1 -1 -1 -1</Voices></Bar></Bars>
<Voices><Voice id="0"><Beats>0 1 2 3 4 5 6</Beats></Voice><Voice id="1"><Beats>7 8 9</Beats></Voice></Voices>
<Beats>
<Beat id="0"><Dynamic>F</Dynamic><Rhythm ref="1"/><Notes>0</Notes></Beat>
<Beat id="1"><Dynamic>F</Dynamic><Rhythm ref="1"/><Notes>1</Notes></Beat>
<Beat id="2"><Dynamic>F</Dynamic><GraceNotes>BeforeBeat</GraceNotes><Rhythm ref="1"/><Notes>2</Notes></Beat>
<Beat id="3"><Dynamic>F</Dynamic><Rhythm ref="1"/><Notes>3</Notes></Beat>
<Beat id="4"><Dynamic>F</Dynamic><Rhythm ref="1"/><Notes>4</Notes></Beat>
<Beat id="5"><Dynamic>F</Dynamic><Rhythm ref="0"/><Notes>5</Notes></Beat>
<Beat id="6"><Dynamic>F</Dynamic><Rhythm ref="0"/><Notes>6</Notes></Beat>
<Beat id="7"><Dynamic>F</Dynamic><Tremolo>1/4</Tremolo><Rhythm ref="0"/><Notes>7</Notes></Beat>
<Beat id="8"><Dynamic>F</Dynamic><Rhythm ref="2"/><Notes>8</Notes></Beat>
<Beat id="9"><Dynamic>F</Dynamic><Rhythm ref="0"/><Notes>9</Notes></Beat>
</Beats>
<Notes>
<Note id="0"><Properties><Property name="String"><String>0</String></Property><Property name="Fret"><Fret>0</Fret></Property><Property name="Midi"><Number>40</Number></Property><Property name="PalmMuted"><Enable/></Property></Properties></Note>
<Note id="1"><Properties><Property name="String"><String>1</String></Property><Property name="Fret"><Fret>0</Fret></Property><Property name="Midi"><Number>45</Number></Property><Property name="Muted"><Enable/></Property></Properties></Note>
<Note id="2"><Properties><Property name="String"><String>2</String></Property><Property name="Fret"><Fret>0</Fret></Property><Property name="Midi"><Number>50</Number></Property></Properties></Note>
<Note id="3"><Properties><Property name="String"><String>2</String></Property><Property name="Fret"><Fret>2</Fret></Property><Property name="Midi"><Number>52</Number></Property><Property name="HopoOrigin"><Enable/></Property></Properties></Note>
<Note id="4"><Properties><Property name="String"><String>2</String></Property><Property name="Fret"><Fret>4</Fret></Property><Property name="Midi"><Number>54</Number></Property><Property name="HopoDestination"><Enable/></Property></Properties></Note>
<Note id="5"><Properties><Property name="String"><String>3</String></Property><Property name="Fret"><Fret>4</Fret></Property><Property name="Midi"><Number>59</Number></Property><Property name="Slide"><Flags>2</Flags></Property></Properties></Note>
<Note id="6"><Properties><Property name="String"><String>3</String></Property><Property name="Fret"><Fret>5</Fret></Property><Property name="Midi"><Number>60</Number></Property></Properties></Note>
<Note id="7"><Properties><Property name="String"><String>5</String></Property><Property name="Fret"><Fret>0</Fret></Property><Property name="Midi"><Number>64</Number></Property></Properties></Note>
<Note id="8"><Tie origin="false" destination="true"/><Properties><Property name="String"><String>4</String></Property><Property name="Fret"><Fret>1</Fret></Property><Property name="Midi"><Number>60</Number></Property></Properties></Note>
<Note id="9"><Properties><Property name="String"><String>5</String></Property><Property name="Fret"><Fret>3</Fret></Property><Property name="Midi"><Number>67</Number></Property></Properties><Accent>4</Accent></Note>
</Notes>
<Rhythms>
<Rhythm id="0"><NoteValue>Quarter</NoteValue></Rhythm>
<Rhythm id="1"><NoteValue>Eighth</NoteValue></Rhythm>
<Rhythm id="2"><NoteValue>Half</NoteValue></Rhythm>
</Rhythms></GPIF>"#;

#[test]
fn techniques_play_as_tabgarden_plays_them_and_survive_export() {
    use soundcraft_midi::tab::tech;
    let score = gpif::parse_gpif(TECHNIQUES).unwrap();
    let s = session_from_score(&score, None, "t").unwrap();
    let seq = notes_of(&s, 0);
    let notes: Vec<(i64, u8, i64, u8)> = seq.notes.iter().map(|n| (n.start, n.pitch, n.length, n.velocity)).collect();
    let tech_of = |start: i64, pitch: u8| seq.tab.iter().find(|t| t.start == start && t.pitch == pitch).map_or(0, |t| t.tech);
    // A palm mute and a dead note sound short; the hammer-on is played softer.
    assert!(notes.contains(&(0, 40, 254, 95)), "{notes:?}");
    assert!(notes.contains(&(480, 45, 96, 95)));
    assert_eq!(tech_of(0, 40), tech::PALM_MUTE);
    assert_eq!(tech_of(480, 45), tech::DEAD);
    // The grace note sounds a 32nd before its beat and takes no time from the bar.
    assert!(notes.contains(&(840, 50, 120, 95)));
    assert_eq!(tech_of(840, 50) & tech::GRACE, tech::GRACE);
    assert!(notes.contains(&(960, 52, 480, 95)) && notes.contains(&(1440, 54, 480, 79)));
    assert_eq!(tech_of(1440, 54), tech::HOPO_DEST);
    // The legato slide glides into the next note on its string instead of picking it.
    assert!(notes.contains(&(1920, 59, 961, 95)) && notes.contains(&(2880, 60, 960, 95)));
    assert!(seq.ctrls.iter().any(|c| c.tick == 2879 && c.kind == CtrlKind::Cc(PORTAMENTO) && c.value == 127));
    // Tremolo: a quarter picked as four 16ths. The tie with nothing before it is silent.
    assert_eq!(notes.iter().filter(|n| n.1 == 64).map(|n| (n.0, n.2)).collect::<Vec<_>>(), [(3840, 240), (4080, 240), (4320, 240), (4560, 240)]);
    assert!(!notes.iter().any(|n| n.1 == 60 && n.0 == 4800));
    // The accent is one dynamic step louder.
    assert!(notes.contains(&(6720, 67, 960, 111)));
    assert_eq!(seq.notes.len(), 12);

    // Written back out and read again: the techniques and the notes as written come back.
    let mut e = Engine::new(Session::default());
    e.replace_session(s);
    let bytes = export_tabgarden(&e, None).unwrap();
    open_tabgarden_bytes(&mut e, "again.tabgarden", &bytes).unwrap();
    let again = notes_of(e.session(), 0);
    let tech_again = |start: i64, pitch: u8| again.tab.iter().find(|t| t.start == start && t.pitch == pitch).map_or(0, |t| t.tech);
    assert_eq!(tech_again(0, 40), tech::PALM_MUTE);
    assert_eq!(tech_again(480, 45), tech::DEAD);
    assert_eq!(tech_again(960, 52), tech::HOPO);
    assert_eq!(tech_again(1440, 54), tech::HOPO_DEST);
    assert_eq!(tech_again(1920, 59), tech::SLIDE_LEGATO);
    assert_eq!(tech_again(6720, 67), tech::ACCENT);
    let back: Vec<(i64, u8, i64, u8)> = again.notes.iter().map(|n| (n.start, n.pitch, n.length, n.velocity)).collect();
    assert!(back.contains(&(0, 40, 254, 95)) && back.contains(&(1440, 54, 480, 79)) && back.contains(&(6720, 67, 960, 111)), "{back:?}");
}

/// A Guitar Pro 6 drum track: the channel sits under `GeneralMidi`, and each note names a kit
/// piece and how it is struck instead of a key.
const GP6_DRUMS: &str = r#"<GPIF><Score><Title><![CDATA[Kit]]></Title></Score>
<MasterTrack><Tracks>0</Tracks></MasterTrack>
<Tracks><Track id="0"><Name><![CDATA[Drumkit]]></Name><Instrument ref="drmkt" />
<GeneralMidi table="Percussion"><Program>0</Program><Port>0</Port><PrimaryChannel>9</PrimaryChannel><SecondaryChannel>9</SecondaryChannel></GeneralMidi></Track></Tracks>
<MasterBars><MasterBar><Time>4/4</Time><Bars>0</Bars></MasterBar></MasterBars>
<Bars><Bar id="0"><Voices>0 -1 -1 -1</Voices></Bar></Bars>
<Voices><Voice id="0"><Beats>0 1 2 3</Beats></Voice></Voices>
<Beats>
<Beat id="0"><Rhythm ref="0"/><Notes>0 1</Notes></Beat>
<Beat id="1"><Rhythm ref="0"/><Notes>2</Notes></Beat>
<Beat id="2"><Rhythm ref="0"/><Notes>3 4</Notes></Beat>
<Beat id="3"><Rhythm ref="0"/><Notes>5</Notes></Beat>
</Beats>
<Notes>
<Note id="0"><Properties><Property name="Element"><Element>0</Element></Property><Property name="Variation"><Variation>0</Variation></Property></Properties></Note>
<Note id="1"><Properties><Property name="Element"><Element>16</Element></Property><Property name="Variation"><Variation>0</Variation></Property></Properties></Note>
<Note id="2"><Properties><Property name="Element"><Element>1</Element></Property><Property name="Variation"><Variation>1</Variation></Property></Properties></Note>
<Note id="3"><Properties><Property name="Element"><Element>12</Element></Property><Property name="Variation"><Variation>0</Variation></Property></Properties></Note>
<Note id="4"><Properties><Property name="Element"><Element>15</Element></Property><Property name="Variation"><Variation>2</Variation></Property></Properties></Note>
<Note id="5"><Properties><Property name="Element"><Element>999</Element></Property><Property name="Variation"><Variation>99999999999999999999</Variation></Property></Properties></Note>
</Notes>
<Rhythms><Rhythm id="0"><NoteValue>Quarter</NoteValue></Rhythm></Rhythms></GPIF>"#;

#[test]
fn guitar_pro_6_kit_pieces_play_the_keys_tabgarden_plays() {
    let s = session_from_score(&gpif::parse_gpif(GP6_DRUMS).unwrap(), None, "k").unwrap();
    let seq = notes_of(&s, 0);
    let mut hits: Vec<(i64, u8)> = seq.notes.iter().map(|n| (n.start, n.pitch)).collect();
    hits.sort_unstable();
    // Kick and china, the rim shot, a medium crash and the ride bell; an unknown piece is left out.
    assert_eq!(hits, [(0, 35), (0, 52), (960, 38), (1920, 57), (1920, 59)]);
}
