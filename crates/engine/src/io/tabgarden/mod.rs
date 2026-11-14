//! TabGarden files (`.tabgarden`): open one as a whole session, and write the session's MIDI
//! parts as one, as stems TabGarden opens and plays.
//!
//! A `.tabgarden` file is a zip of stored (uncompressed) entries:
//! - `tabgarden.json`: `format: "tabgarden"`, `formatVersion: 1`, title, artist, the instrument
//!   chosen for each track (`instruments: {"<track index>": {id: "gm:25", …}}`) and the names of
//!   the other entries (`files: {gp, mid, original}`);
//! - `score.gp`: the tab itself, a Guitar Pro 7 score ([`gpif`]), the copy TabGarden reads;
//! - `score.mid`: the score as a Standard MIDI File (optional; written here as the stems);
//! - `original.mid`: the MIDI file a song was imported from (optional, not read).
//!
//! Fretted parts open as tab tracks (their notes, tuning, capo and the string each note was
//! written on), drum kits as Drum Synth tracks and other parts as instrument tracks, with the
//! score's tempo and meters and its repeats played out. Every entry is treated as hostile: sizes,
//! counts and lengths are capped, and a malformed file is an error.

pub(crate) mod gpif;
pub(crate) mod xml;

use crate::{Engine, EngineError, Result};
use gpif::{GRACE_TICKS, GpBeat, GpKind, GpMasterBar, GpNote, GpScore, GpTempo, GpTrack, Grace};
use serde_json::{Value, json};
use soundcraft_midi::tab::{Tuning, place, tech as tab_tech};
use soundcraft_midi::{CtrlEvent, CtrlKind, Note, Sequence};
use soundcraft_model::{ChannelFormat, Clip, ClipContent, Insert, Session, TabSetup, TrackId, TrackKind};
use std::io::{Read as _, Write as _};

/// The file extension.
pub const TABGARDEN_EXTENSION: &str = "tabgarden";
/// The newest `formatVersion` read.
const FORMAT_VERSION: u64 = 1;
/// Largest `.tabgarden` (or `.gp`) file read.
const MAX_FILE: u64 = 256 * 1024 * 1024;
/// Largest single entry read, unpacked.
const MAX_ENTRY: u64 = 128 * 1024 * 1024;
/// Most entries in either zip.
const MAX_ENTRIES: usize = 256;
/// Most notes one track brings in.
const MAX_NOTES: usize = 500_000;
/// Ticks per sixteenth: the grid exported tab is written on.
const UNIT: i64 = 240;

fn io(what: impl std::fmt::Display) -> EngineError {
    EngineError::Io(format!("TabGarden file: {what}"))
}

/// The entries of a zip by name, each capped; directories are skipped.
fn unzip(bytes: &[u8], what: &str) -> Result<Vec<(String, Vec<u8>)>> {
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_FILE {
        return Err(io(format!("{what} is too large")));
    }
    let zerr = |err: zip::result::ZipError| io(format!("{what}: {err}"));
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(zerr)?;
    if z.len() > MAX_ENTRIES {
        return Err(io(format!("{what}: too many entries")));
    }
    let mut out = Vec::new();
    let mut total = 0u64;
    for i in 0..z.len() {
        let f = z.by_index(i).map_err(zerr)?;
        let name = f.name().map_err(zerr)?.replace('\\', "/");
        // A folder entry holds no bytes.
        if name.ends_with('/') {
            continue;
        }
        let size = f.size();
        if size > MAX_ENTRY {
            return Err(io(format!("{what}: {name} is too large")));
        }
        total = total.saturating_add(size);
        if total > MAX_FILE {
            return Err(io(format!("{what}: too large unpacked")));
        }
        let mut data = Vec::new();
        // The declared size is hostile too: never read past it (the CRC check then fails).
        f.take(size).read_to_end(&mut data).map_err(|err| io(format!("{what}: {name}: {err}")))?;
        out.push((name, data));
    }
    Ok(out)
}

fn entry<'a>(entries: &'a [(String, Vec<u8>)], name: &str) -> Option<&'a [u8]> {
    entries.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, d)| d.as_slice())
}

/// Read a Guitar Pro 7+ score (`.gp`).
pub(crate) fn read_gp(bytes: &[u8]) -> Result<GpScore> {
    let entries = unzip(bytes, "score.gp")?;
    let gpif = entry(&entries, "Content/score.gpif").ok_or_else(|| io("score.gp holds no Content/score.gpif"))?;
    let text = std::str::from_utf8(gpif.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(gpif)).map_err(|_| io("score.gpif is not UTF-8 text"))?;
    gpif::parse_gpif(text).map_err(io)
}

/// What a `.tabgarden` file holds.
pub(crate) struct TabgardenFile {
    pub(crate) manifest: Value,
    pub(crate) score: GpScore,
}

/// Unpack and check a `.tabgarden` file.
pub(crate) fn read_tabgarden(bytes: &[u8]) -> Result<TabgardenFile> {
    let entries = unzip(bytes, "the file")?;
    let manifest_bytes = entry(&entries, "tabgarden.json").ok_or_else(|| io("no tabgarden.json inside (not a TabGarden file)"))?;
    let manifest: Value = serde_json::from_slice(manifest_bytes).map_err(|err| io(format!("tabgarden.json: {err}")))?;
    if manifest.get("format").and_then(Value::as_str) != Some("tabgarden") {
        return Err(io("tabgarden.json does not say format \"tabgarden\""));
    }
    let version = manifest.get("formatVersion").and_then(Value::as_u64).unwrap_or(0);
    if version > FORMAT_VERSION {
        return Err(io(format!("format version {version} is newer than this Solara reads ({FORMAT_VERSION}); update Solara")));
    }
    let gp_name = manifest.pointer("/files/gp").and_then(Value::as_str).unwrap_or("score.gp");
    let gp = entry(&entries, gp_name).ok_or_else(|| io(format!("{gp_name} is missing")))?;
    let score = read_gp(gp)?;
    Ok(TabgardenFile { manifest, score })
}

/// General MIDI program a TabGarden instrument id plays (`gm:N`, a Sound Engine Suite preset, or
/// one of its own instruments), when it names one.
fn instrument_program(id: &str) -> Option<u8> {
    if let Some(n) = id.strip_prefix("gm:") {
        return n.trim().parse::<u8>().ok().filter(|p| *p <= 127);
    }
    let ses = id.strip_prefix("ses:");
    let program = match ses.unwrap_or(id) {
        "clean-guitar" => 25,
        "electric-clean" => 27,
        "electric-drive" | "heavy-rhythm" | "crunch" | "doom" => 30,
        "full-lead" | "banjo-lead" => 29,
        "bass" => 33,
        "bass-pick" => 34,
        "bass-fretless" => 35,
        "piano" => 0,
        "synth" => 81,
        "accordion" => 21,
        "guitar" => 24,
        "violin" => 40,
        "harp" => 46,
        "flute" => 73,
        "trumpet" => 56,
        "sax" => 65,
        "voice" => 52,
        _ => return None,
    };
    Some(program)
}

/// An instrument plugin for a General MIDI program: the Subtractive Synth set to pluck for
/// guitars and basses, the Drum Synth for kits.
fn instrument_for(program: u8, drums: bool) -> Insert {
    if drums {
        return Insert::new("drum_synth");
    }
    let mut ins = Insert::new("subtractive_synth");
    let set: &[(&str, f32)] = match program {
        24..=31 => &[
            ("osc1_wave", 0.0),
            ("osc2_wave", 1.0),
            ("osc2_semitones", 12.0),
            ("osc_mix", 30.0),
            ("cutoff", 1800.0),
            ("resonance", 15.0),
            ("filter_env_amount", 55.0),
            ("amp_attack", 1.0),
            ("amp_decay", 1200.0),
            ("amp_sustain", 0.0),
            ("amp_release", 250.0),
            ("filter_decay", 350.0),
            ("filter_sustain", 10.0),
        ],
        32..=39 => &[
            ("osc1_wave", 0.0),
            ("osc2_wave", 2.0),
            ("osc2_semitones", -12.0),
            ("osc_mix", 40.0),
            ("cutoff", 700.0),
            ("resonance", 10.0),
            ("filter_env_amount", 40.0),
            ("amp_attack", 1.0),
            ("amp_decay", 900.0),
            ("amp_sustain", 20.0),
            ("amp_release", 150.0),
            ("filter_decay", 250.0),
            ("filter_sustain", 15.0),
        ],
        _ => &[],
    };
    for (k, v) in set {
        ins.params.insert((*k).to_string(), *v);
    }
    ins.preset = match program {
        24..=31 => "Plucked Guitar".into(),
        32..=39 => "Plucked Bass".into(),
        _ => String::new(),
    };
    ins
}

/// Build a session from a score: one track per score track, with one clip holding all its notes.
pub(crate) fn session_from_score(score: &GpScore, manifest: Option<&Value>, fallback_name: &str) -> Result<Session> {
    let name = if score.title.trim().is_empty() { fallback_name.to_string() } else { score.title.trim().to_string() };
    let mut s = Session::new(name, soundcraft_time::SampleRate::new(48_000).map_err(|err| io(err.to_string()))?);
    let order = gpif::play_order(&score.bars);
    // Where each played bar starts, in ticks.
    let mut starts = Vec::with_capacity(order.len());
    let mut tick = 0i64;
    let mut meter = (0u32, 0u32);
    for &bi in &order {
        let Some(b) = score.bars.get(bi) else { continue };
        if (b.numerator, b.denominator) != meter {
            s.tempo.set_meter(tick, b.numerator.clamp(1, 64), b.denominator.clamp(1, 64)).map_err(|err| io(err.to_string()))?;
            meter = (b.numerator, b.denominator);
        }
        starts.push((bi, tick));
        tick = tick.saturating_add(b.ticks());
    }
    let song_end = tick.max(1);
    let mut tempos: Vec<(i64, f64)> = Vec::new();
    for t in &score.tempos {
        for (bi, at) in starts.iter().filter(|(bi, _)| *bi == t.bar) {
            let len = score.bars.get(*bi).map_or(3840, GpMasterBar::ticks);
            tempos.push((at.saturating_add((t.position * len as f64) as i64), t.bpm));
        }
    }
    tempos.sort_by_key(|t| t.0);
    if let Some(first) = tempos.first().copied() {
        s.tempo.set_tempo(0, first.1).map_err(|err| io(err.to_string()))?;
    }
    for (at, bpm) in tempos.iter().skip(1) {
        if (s.tempo.tempo_at_tick(*at) - bpm).abs() > 1e-6 {
            s.tempo.set_tempo(*at, *bpm).map_err(|err| io(err.to_string()))?;
        }
    }
    let picks = manifest.and_then(|m| m.get("instruments"));
    for (ti, t) in score.tracks.iter().enumerate() {
        let pick = picks.and_then(|p| p.get(ti.to_string())).and_then(|p| p.get("id")).and_then(Value::as_str);
        let program = pick.and_then(instrument_program).unwrap_or(t.program);
        let drums = matches!(t.kind, GpKind::Drums) || pick.is_some_and(|p| p.starts_with("gmkit:"));
        let kind = match t.kind {
            GpKind::Stringed { .. } if !drums => TrackKind::Tab,
            _ => TrackKind::Instrument,
        };
        let track_name = if t.name.trim().is_empty() { format!("Track {}", ti + 1) } else { t.name.trim().to_string() };
        let id = s.add_track(kind, ChannelFormat::Stereo, Some(&track_name));
        let seq = track_sequence(t, &order, &starts, drums)?;
        if let Some(tr) = s.track_mut(id) {
            tr.color = t.color;
            tr.instrument = Some(instrument_for(program, drums));
            if let GpKind::Stringed { tuning, capo } = &t.kind
                && kind == TrackKind::Tab
            {
                let strings: Vec<u8> = tuning.iter().rev().copied().collect();
                tr.tab = Some(TabSetup { tuning: Tuning { strings, capo: *capo }.sanitized(), program, ..TabSetup::default() });
            }
        }
        let len = s.tempo.tick_to_samples(song_end, s.sample_rate).max(1);
        let cid = s.new_clip_id();
        crate::edit::place_clip(&mut s, id, Clip::midi(cid, track_name, 0, len, seq));
    }
    let mut setup = crate::score::ScoreSetup::from_session(&s);
    setup.title = s.name.clone();
    setup.composer = score.artist.clone();
    setup.store(&mut s);
    Ok(s)
}

/// One track's notes, in the played-out bar order; ties join notes, strings become hints.
fn track_sequence(t: &GpTrack, order: &[usize], starts: &[(usize, i64)], drums: bool) -> Result<Sequence> {
    let strings = match &t.kind {
        GpKind::Stringed { tuning, .. } => tuning.len(),
        _ => 0,
    };
    let channel = if drums { 9 } else { t.channel.min(15) };
    let mut seq = Sequence::default();
    // Where each note ends as written, before techniques shorten or lengthen it: ties join there.
    let mut written_end: Vec<i64> = Vec::new();
    for (&bi, &(_, bar_tick)) in order.iter().zip(starts) {
        let Some(voices) = t.bars.get(bi) else { continue };
        for voice in voices {
            let mut at = bar_tick;
            // Time an on-beat grace note takes from the beat after it.
            let mut delay = 0i64;
            for beat in voice {
                let (start, length) = match beat.grace {
                    Grace::Before => (at.saturating_sub(GRACE_TICKS).max(0), GRACE_TICKS),
                    Grace::On => (at, GRACE_TICKS),
                    Grace::None => (at.saturating_add(delay), beat.ticks.saturating_sub(delay).max(1)),
                };
                for n in &beat.notes {
                    // A tied note lengthens the same pitch that ends where this beat starts.
                    // A tie with nothing to continue is not picked either.
                    if n.tie_in && beat.grace == Grace::None {
                        if let Some((prev, end)) =
                            seq.notes.iter_mut().zip(written_end.iter_mut()).rev().find(|(p, e)| p.pitch == n.pitch && **e == start)
                        {
                            prev.length = prev.length.saturating_add(length);
                            *end = end.saturating_add(length);
                        }
                        continue;
                    }
                    let mut tech = n.tech;
                    if beat.grace != Grace::None {
                        tech |= tab_tech::GRACE;
                    }
                    let string = n
                        .string
                        .filter(|_| strings > 0)
                        .and_then(|(s, _)| u8::try_from(strings.saturating_sub(usize::from(s))).ok())
                        .filter(|s| *s >= 1);
                    let velocity = played_velocity(beat.velocity, tech);
                    // Tremolo picking: what sounds of the beat is split into even strokes, as many
                    // as strokes of the tremolo's value it takes to fill it.
                    let sounding = played_length(length, tech);
                    let strokes =
                        if beat.tremolo > 0 && beat.tremolo < length { (sounding + beat.tremolo - 1) / beat.tremolo } else { 1 }.clamp(1, 64);
                    if strokes > 1 {
                        tech |= tab_tech::TREMOLO;
                    }
                    let each = (sounding / strokes).max(1);
                    for k in 0..strokes {
                        if seq.notes.len() >= MAX_NOTES {
                            return Err(io(format!("track {}: more than {MAX_NOTES} notes", t.name)));
                        }
                        let s0 = start.saturating_add(each.saturating_mul(k));
                        seq.notes.push(Note { pitch: n.pitch, velocity, release_velocity: 64, channel, start: s0, length: each });
                        written_end.push(if k + 1 == strokes { start.saturating_add(length) } else { s0.saturating_add(each) });
                        if string.is_some() || tech != 0 {
                            seq.tab.push(soundcraft_midi::tab::TabPos { start: s0, pitch: n.pitch, channel, string: string.unwrap_or(0), tech });
                        }
                    }
                }
                match beat.grace {
                    Grace::Before => {}
                    Grace::On => delay = GRACE_TICKS.min(beat_after(voice, beat)),
                    Grace::None => {
                        at = at.saturating_add(beat.ticks);
                        delay = 0;
                    }
                }
            }
        }
    }
    seq.sort();
    legato_slides(&mut seq);
    Ok(seq)
}

/// The length of the beat after `beat` in `voice` (for an on-beat grace note), else a whole note.
fn beat_after(voice: &[GpBeat], beat: &GpBeat) -> i64 {
    let i = voice.iter().position(|b| std::ptr::eq(b, beat)).unwrap_or(voice.len());
    voice.get(i + 1..).and_then(|rest| rest.iter().find(|b| b.grace == Grace::None)).map_or(UNIT * 16, |b| b.ticks / 2)
}

/// Velocity one dynamic step is worth.
const DYNAMIC_STEP: u8 = 16;

/// How hard a note with `tech` is played at a beat's `velocity`: a hammer-on or pull-off softer,
/// an accent harder.
fn played_velocity(velocity: u8, tech: u16) -> u8 {
    let mut v = velocity;
    if tech & tab_tech::HOPO_DEST != 0 {
        v = v.saturating_sub(DYNAMIC_STEP);
    }
    if tech & tab_tech::ACCENT != 0 {
        v = v.saturating_add(DYNAMIC_STEP);
    }
    v.clamp(1, 127)
}

/// How long a note with `tech` sounds when written `ticks` long: dead notes and palm mutes are cut
/// short, staccato halves, let ring doubles.
fn played_length(ticks: i64, tech: u16) -> i64 {
    let mut l = ticks;
    if tech & tab_tech::DEAD != 0 {
        l = l.min(DEAD_TICKS);
    } else if tech & tab_tech::PALM_MUTE != 0 {
        l = l.min(PALM_MUTE_TICKS);
    }
    if tech & tab_tech::STACCATO != 0 {
        l /= 2;
    }
    if tech & tab_tech::LET_RING != 0 {
        l = l.saturating_mul(2);
    }
    l.max(1)
}

/// How long a dead note sounds (a tenth of a quarter note).
const DEAD_TICKS: i64 = 96;
/// How long a palm-muted note sounds, at most.
const PALM_MUTE_TICKS: i64 = 254;

/// A legato slide is not picked again: the note it slides to (the next on the same string, starting
/// where it ends) is reached by portamento, which glides the sounding voice there. The slide note
/// is held a tick longer so the voice is still sounding when the glide starts.
fn legato_slides(seq: &mut Sequence) {
    let mut glides: Vec<(i64, i64, u8)> = Vec::new();
    for h in seq.tab.iter().filter(|h| h.tech & tab_tech::SLIDE_LEGATO != 0 && h.string > 0) {
        let Some(from) = seq.notes.iter().position(|n| n.start == h.start && n.pitch == h.pitch && n.channel == h.channel) else { continue };
        let Some(end) = seq.notes.get(from).map(Note::end) else { continue };
        let to = seq.tab.iter().find(|d| d.string == h.string && d.channel == h.channel && d.start == end && d.tech & tab_tech::GRACE == 0);
        let Some(to) = to.and_then(|d| seq.notes.iter().find(|n| n.start == d.start && n.pitch == d.pitch && n.channel == d.channel)) else {
            continue;
        };
        glides.push((to.start, to.end(), to.channel));
        if let Some(n) = seq.notes.get_mut(from) {
            n.length = n.length.saturating_add(1);
        }
    }
    for (start, end, channel) in glides {
        let on = start.saturating_sub(1).max(0);
        let off = end.saturating_sub(1).max(on.saturating_add(1));
        seq.ctrls.push(CtrlEvent { tick: on, channel, kind: CtrlKind::Cc(PORTAMENTO), value: 127 });
        seq.ctrls.push(CtrlEvent { tick: off, channel, kind: CtrlKind::Cc(PORTAMENTO), value: 0 });
    }
    seq.sort();
}

/// The portamento controller (on at 64 and over).
const PORTAMENTO: u8 = 65;

/// Open a `.tabgarden` file as a new session (replacing the open one).
pub fn open_tabgarden_bytes(e: &mut Engine, name: &str, bytes: &[u8]) -> Result<Value> {
    let file = read_tabgarden(bytes)?;
    let stem = std::path::Path::new(name).file_stem().and_then(std::ffi::OsStr::to_str).unwrap_or("TabGarden");
    let s = session_from_score(&file.score, Some(&file.manifest), stem)?;
    let tracks: Vec<Value> = s
        .tracks
        .iter()
        .map(|t| {
            let notes: usize = t.clips().iter().map(|c| if let ClipContent::Midi { sequence } = &c.content { sequence.notes.len() } else { 0 }).sum();
            json!({"name": t.name, "kind": t.kind.id(), "notes": notes, "tuning": t.tab.as_ref().map(|x| x.tuning.label())})
        })
        .collect();
    let summary = json!({"name": s.name, "tracks": tracks, "bars": gpif::play_order(&file.score.bars).len(), "tempo": s.tempo.tempo_at_tick(0)});
    e.replace_session(s);
    e.path = None;
    Ok(summary)
}

// ─── Export ───────────────────────────────────────────────────────────────────────────────────

/// A track's notes on the timeline (ticks from the session start), with their string hints.
fn timeline_sequence(s: &Session, t: &soundcraft_model::Track) -> Sequence {
    let mut seq = Sequence::default();
    for c in t.clips() {
        let ClipContent::Midi { sequence } = &c.content else { continue };
        if c.muted {
            continue;
        }
        let base = s.tempo.samples_to_ticks(c.start, s.sample_rate);
        let end = s.tempo.samples_to_ticks(c.end(), s.sample_rate);
        for n in &sequence.notes {
            let st = base.saturating_add(n.start);
            if st < base || st >= end || n.length <= 0 || seq.notes.len() >= MAX_NOTES {
                continue;
            }
            let mut m = *n;
            m.start = st;
            m.length = n.length.min(end - st).max(1);
            seq.notes.push(m);
        }
        for h in &sequence.tab {
            seq.tab.push(soundcraft_midi::tab::TabPos { start: base.saturating_add(h.start), ..*h });
        }
    }
    seq.sort();
    seq
}

/// One note of a written chord: pitch, string and fret, velocity, techniques.
type Chorded = (u8, Option<(u8, u8)>, u8, u16);

/// Where a note ends as written: a dead, palm-muted or staccato note sounds short but is written
/// up to the next note (at most a half note); a let-ring note sounds twice as long as written.
fn written_end(n: &Note, tech: u16, starts: &[i64]) -> i64 {
    if tech & (tab_tech::DEAD | tab_tech::PALM_MUTE | tab_tech::STACCATO) != 0 {
        // `starts` is sorted (the sequence is).
        let next = starts.get(starts.partition_point(|s| *s <= n.start)).copied().unwrap_or(n.end());
        return next.min(n.start.saturating_add(UNIT * 8)).max(n.end());
    }
    if tech & tab_tech::LET_RING != 0 {
        return n.start.saturating_add(n.length / 2);
    }
    n.end()
}

/// The beat's dynamic for a note played at `velocity`: the step a technique added or took away put
/// back.
fn written_velocity(velocity: u8, tech: u16) -> u8 {
    let mut v = velocity;
    if tech & tab_tech::HOPO_DEST != 0 {
        v = v.saturating_add(DYNAMIC_STEP);
    }
    if tech & tab_tech::ACCENT != 0 {
        v = v.saturating_sub(DYNAMIC_STEP);
    }
    v.clamp(1, 127)
}

fn quantize(tick: i64) -> i64 {
    (tick.max(0) + UNIT / 2) / UNIT * UNIT
}

/// What a track is written as.
fn export_kind(t: &soundcraft_model::Track, seq: &Sequence) -> GpKind {
    let drum_plugin = t.instrument.as_ref().is_some_and(|i| i.plugin.starts_with("drum_"));
    if drum_plugin || (!seq.notes.is_empty() && seq.notes.iter().all(|n| n.channel == 9)) {
        return GpKind::Drums;
    }
    match t.tab_setup() {
        Some(tab) => {
            let tuning = tab.tuning.sanitized();
            GpKind::Stringed { tuning: tuning.strings.iter().rev().copied().collect(), capo: tuning.capo }
        }
        None => GpKind::Other,
    }
}

/// One track's beats for every bar of `grid` ((start tick, length) per bar): one voice of chords
/// and rests on the sixteenth grid, notes crossing a bar line tied over.
fn export_bars(seq: &Sequence, kind: &GpKind, tuning: Option<&Tuning>, grid: &[(i64, i64)]) -> Vec<Vec<Vec<GpBeat>>> {
    let placed = tuning.map(|t| place(seq, t));
    // Chords: onset → (end, [(pitch, string, velocity, techniques)]).
    let mut onsets: std::collections::BTreeMap<i64, (i64, Vec<Chorded>)> = std::collections::BTreeMap::new();
    let starts: Vec<i64> = seq.notes.iter().map(|n| n.start).collect();
    for (i, n) in seq.notes.iter().enumerate() {
        let tech = soundcraft_midi::tab::hint(seq, n).map_or(0, |h| h.tech);
        // Grace notes take no time of their own in a bar; they are not written.
        if tech & tab_tech::GRACE != 0 {
            continue;
        }
        let a = quantize(n.start);
        let b = quantize(written_end(n, tech, &starts)).max(a + UNIT);
        let string = match (kind, &placed, tuning) {
            (GpKind::Stringed { .. }, Some(p), Some(t)) => p.get(i).filter(|p| p.string >= 1).and_then(|p| {
                // The file counts strings from the lowest, from 0.
                let count = u8::try_from(t.strings.len()).ok()?;
                Some((count.checked_sub(p.string)?, p.fret))
            }),
            _ => None,
        };
        let entry = onsets.entry(a).or_insert((b, Vec::new()));
        entry.0 = entry.0.max(b);
        if entry.1.len() < 16 && !entry.1.iter().any(|(p, ..)| *p == n.pitch) {
            entry.1.push((n.pitch, string, written_velocity(n.velocity, tech), tech));
        }
    }
    let keys: Vec<i64> = onsets.keys().copied().collect();
    let spans: Vec<(i64, i64, Vec<Chorded>)> =
        onsets.into_iter().enumerate().map(|(i, (a, (b, ns)))| (a, keys.get(i + 1).map_or(b, |next| b.min(*next)), ns)).collect();
    let mut bars = Vec::with_capacity(grid.len());
    for &(bs, bl) in grid {
        let be = bs + bl;
        let mut beats = Vec::new();
        let mut cursor = bs;
        for (a, b, ns) in spans.iter().filter(|(a, b, _)| *b > bs && *a < be) {
            let from = (*a).max(cursor);
            let to = (*b).min(be);
            if to <= from {
                continue;
            }
            if from > cursor {
                beats.push(GpBeat::new(from - cursor, 95, Vec::new()));
            }
            let velocity = ns.iter().map(|n| n.2).max().unwrap_or(95);
            let notes = ns
                .iter()
                .map(|(pitch, string, _, tech)| GpNote { pitch: *pitch, string: *string, tie_in: *a < bs, tie_out: *b > be, tech: *tech })
                .collect();
            beats.push(GpBeat::new(to - from, velocity, notes));
            cursor = to;
        }
        if cursor < be {
            beats.push(GpBeat::new(be - cursor, 95, Vec::new()));
        }
        bars.push(vec![beats]);
    }
    bars
}

/// The session's MIDI tracks (`only` limits them) as a Guitar Pro score.
pub(crate) fn score_from_session(s: &Session, only: Option<&[TrackId]>) -> Result<(GpScore, Vec<TrackId>)> {
    let chosen: Vec<&soundcraft_model::Track> = s.tracks.iter().filter(|t| t.kind.is_midi() && only.is_none_or(|ids| ids.contains(&t.id))).collect();
    if chosen.is_empty() {
        return Err(EngineError::BadParams("file.export_tabgarden".into(), "no MIDI, instrument or tab tracks to export".into()));
    }
    let seqs: Vec<Sequence> = chosen.iter().map(|t| timeline_sequence(s, t)).collect();
    let last = seqs.iter().flat_map(|q| q.notes.iter().map(|n| quantize(n.end()))).max().unwrap_or(0).max(1);
    // Bars on the session's meter map, sixteenth-aligned.
    let mut grid = Vec::new();
    let mut masters = Vec::new();
    let mut bar = 1i64;
    while grid.len() < 20_000 {
        let a = s.tempo.bar_start_tick(bar);
        let b = s.tempo.bar_start_tick(bar + 1);
        if b <= a {
            break;
        }
        let m = s.tempo.meter_at_tick(a);
        let (qa, qb) = (quantize(a), quantize(b));
        bar += 1;
        if qb <= qa {
            continue;
        }
        // A meter whose bar is not a whole number of sixteenths is written as n/16.
        let (num, den) = if qb - qa == m.ticks_per_bar() {
            (m.numerator, m.denominator)
        } else {
            (u32::try_from((qb - qa) / UNIT).unwrap_or(16).clamp(1, 64), 16)
        };
        grid.push((qa, qb - qa));
        masters.push(GpMasterBar { numerator: num, denominator: den, repeat_start: false, repeat_end: false, repeat_count: 0, endings: 0 });
        if qb >= last {
            break;
        }
    }
    let mut tempos = Vec::new();
    for t in s.tempo.tempos() {
        let Some(bi) = grid.iter().rposition(|(a, _)| *a <= t.tick) else { continue };
        let Some(&(a, len)) = grid.get(bi) else { continue };
        let position = ((t.tick - a) as f64 / len.max(1) as f64).clamp(0.0, 1.0);
        tempos.push(GpTempo { bar: bi, position, bpm: t.bpm });
    }
    let mut tracks = Vec::new();
    let mut next_channel = 0u8;
    for (t, seq) in chosen.iter().zip(&seqs) {
        let kind = export_kind(t, seq);
        let tuning = t.tab_setup().map(|x| x.tuning.sanitized());
        let channel = if matches!(kind, GpKind::Drums) {
            9
        } else {
            let c = next_channel;
            next_channel = next_channel.saturating_add(2);
            if next_channel == 8 || next_channel == 9 {
                next_channel = 10;
            }
            c.min(15)
        };
        let program = t
            .tab_setup()
            .map(|x| x.program)
            .or_else(|| seq_program(s, t))
            .unwrap_or(if matches!(kind, GpKind::Stringed { .. }) { 25 } else { 0 })
            .min(127);
        let bars = export_bars(seq, &kind, tuning.as_ref().filter(|_| matches!(kind, GpKind::Stringed { .. })), &grid);
        tracks.push(GpTrack { name: t.name.clone(), color: t.color, kind, program, channel, bars });
    }
    let setup = crate::score::ScoreSetup::from_session(s);
    Ok((GpScore { title: setup.title, artist: setup.composer, tempos, bars: masters, tracks }, chosen.iter().map(|t| t.id).collect()))
}

/// The first program change in a track's clips.
fn seq_program(_s: &Session, t: &soundcraft_model::Track) -> Option<u8> {
    t.clips().iter().find_map(|c| match &c.content {
        ClipContent::Midi { sequence } => sequence
            .ctrls
            .iter()
            .find(|ev| ev.kind == soundcraft_midi::CtrlKind::Program)
            .and_then(|ev| u8::try_from(ev.value).ok())
            .filter(|p| *p <= 127),
        _ => None,
    })
}

/// A Guitar Pro 7 file holding `gpif`.
pub(crate) fn write_gp(gpif: &str) -> Result<Vec<u8>> {
    let zerr = |err: zip::result::ZipError| io(format!("score.gp: {err}"));
    let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    w.start_file("VERSION", opts).map_err(zerr)?;
    w.write_all(b"7.0").map_err(|err| io(err.to_string()))?;
    w.add_directory("Content/", opts).map_err(zerr)?;
    w.start_file("Content/score.gpif", opts).map_err(zerr)?;
    w.write_all(gpif.as_bytes()).map_err(|err| io(err.to_string()))?;
    Ok(w.finish().map_err(zerr)?.into_inner())
}

/// The session's MIDI tracks (`only` limits them) as a `.tabgarden` file: the tab as `score.gp`,
/// every track's MIDI as `score.mid`.
pub fn export_tabgarden(e: &Engine, only: Option<&[TrackId]>) -> Result<Vec<u8>> {
    let s = e.session();
    let (score, ids) = score_from_session(s, only)?;
    let gp = write_gp(&gpif::write_gpif(&score))?;
    let mid = crate::io::export_midi(e, Some(&ids))?;
    let mut instruments = serde_json::Map::new();
    for (i, t) in score.tracks.iter().enumerate() {
        let id = if matches!(t.kind, GpKind::Drums) { "gmkit:0".to_string() } else { format!("gm:{}", t.program) };
        instruments.insert(i.to_string(), json!({"id": id, "kamiRule": false}));
    }
    let saved = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).ok().and_then(|d| i64::try_from(d.as_secs()).ok()).unwrap_or(0);
    let saved_at = chrono::DateTime::from_timestamp(saved, 0).map(|d| d.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)).unwrap_or_default();
    let manifest = json!({
        "format": "tabgarden",
        "formatVersion": FORMAT_VERSION,
        "app": "Solara",
        "appVersion": env!("CARGO_PKG_VERSION"),
        "savedAt": saved_at,
        "title": score.title,
        "artist": score.artist,
        "source": {"name": format!("{}.mid", s.name), "kind": "midi"},
        "instruments": instruments,
        "category": null,
        "royaltyFree": false,
        "files": {"gp": "score.gp", "mid": "score.mid", "original": null},
    });
    let text = serde_json::to_string_pretty(&manifest).map_err(|err| io(err.to_string()))?;
    // TabGarden reads stored entries only.
    let zerr = |err: zip::result::ZipError| io(err.to_string());
    let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for (name, data) in [("tabgarden.json", text.as_bytes()), ("score.gp", gp.as_slice()), ("score.mid", mid.as_slice())] {
        w.start_file(name, opts).map_err(zerr)?;
        w.write_all(data).map_err(|err| io(err.to_string()))?;
    }
    Ok(w.finish().map_err(zerr)?.into_inner())
}

#[cfg(test)]
mod tests;
