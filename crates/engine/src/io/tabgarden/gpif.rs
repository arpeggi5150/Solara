//! Guitar Pro 7 and later scores (`.gp`): a zip holding `Content/score.gpif`, an XML document of
//! tracks, master bars, bars, voices, beats, notes and rhythms that refer to each other by id.
//!
//! Written from the structure of files observed, not from any other program's code. Read: tracks
//! with their tuning, capo, program and channel; meters, repeats and alternate endings; tempo
//! changes; every voice's beats with rhythm (dots and tuplets), dynamics and notes with string,
//! fret, pitch and ties; and the playing techniques TabGarden plays (dead and palm-muted notes,
//! hammer-ons and pull-offs, slides, let ring, accents, staccato, grace notes and tremolo
//! picking). Bends and harmonics are not read.
//! Written: the same, from Solara's notes, which TabGarden opens.

use super::xml::{self, Element};
use soundcraft_midi::tab::tech;
use std::collections::HashMap;
use std::fmt::Write as _;

/// Ticks per whole note at 960 per quarter.
const WHOLE: i64 = 3840;
/// Most tracks read.
const MAX_TRACKS: usize = 128;
/// Most master bars read (after repeats are played out).
const MAX_BARS: usize = 20_000;
/// Most ids one list may name.
const MAX_LIST: usize = 4096;
/// How many bars' worth of beats one voice of a bar may hold.
const MAX_OVERRUN: i64 = 4;
/// Most times a repeat is played.
const MAX_REPEAT: u32 = 16;

/// What kind of part a track is.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum GpKind {
    /// A fretted instrument: open string pitches, lowest string first, and the capo fret.
    Stringed { tuning: Vec<u8>, capo: u8 },
    /// A drum kit: note pitches are General MIDI drum keys.
    Drums,
    /// Anything else (keys, winds…): pitches only.
    Other,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct GpNote {
    pub(crate) pitch: u8,
    /// String, 0 = the lowest (as the file counts), and fret.
    pub(crate) string: Option<(u8, u8)>,
    /// Continues the same note from the previous beat.
    pub(crate) tie_in: bool,
    /// Carries on into the next beat.
    pub(crate) tie_out: bool,
    /// Playing techniques: `soundcraft_midi::tab::tech` bits.
    pub(crate) tech: u16,
}

/// Where a grace beat is played.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Grace {
    /// A normal beat.
    #[default]
    None,
    /// Just before the beat it ornaments, taking time from the beat before.
    Before,
    /// On the beat, delaying the beat it ornaments.
    On,
}

/// How long a grace note sounds (a 32nd note), whatever its written value.
pub(crate) const GRACE_TICKS: i64 = 120;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct GpBeat {
    pub(crate) ticks: i64,
    pub(crate) velocity: u8,
    /// Empty for a rest.
    pub(crate) notes: Vec<GpNote>,
    /// Tremolo picking: the length of each stroke in ticks, 0 for none.
    pub(crate) tremolo: i64,
    /// A grace beat takes no time of its own in the bar.
    pub(crate) grace: Grace,
}

impl GpBeat {
    pub(crate) fn new(ticks: i64, velocity: u8, notes: Vec<GpNote>) -> Self {
        GpBeat { ticks, velocity, notes, tremolo: 0, grace: Grace::None }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct GpTrack {
    pub(crate) name: String,
    pub(crate) color: [u8; 3],
    pub(crate) kind: GpKind,
    pub(crate) program: u8,
    /// MIDI channel 0..=15 (9 for drums).
    pub(crate) channel: u8,
    /// Per master bar, per voice, the beats.
    pub(crate) bars: Vec<Vec<Vec<GpBeat>>>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct GpMasterBar {
    pub(crate) numerator: u32,
    pub(crate) denominator: u32,
    pub(crate) repeat_start: bool,
    pub(crate) repeat_end: bool,
    /// Times the repeated section plays in all (2 = once more).
    pub(crate) repeat_count: u32,
    /// Bit n set: this bar is played on pass n + 1 (0 = every pass).
    pub(crate) endings: u32,
}

impl GpMasterBar {
    pub(crate) fn ticks(&self) -> i64 {
        i64::from(self.numerator.clamp(1, 64)) * WHOLE / i64::from(self.denominator.clamp(1, 64))
    }
}

/// A tempo change at `position` (0..1) through master bar `bar`, in quarter notes per minute.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct GpTempo {
    pub(crate) bar: usize,
    pub(crate) position: f64,
    pub(crate) bpm: f64,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct GpScore {
    pub(crate) title: String,
    pub(crate) artist: String,
    pub(crate) tempos: Vec<GpTempo>,
    pub(crate) bars: Vec<GpMasterBar>,
    pub(crate) tracks: Vec<GpTrack>,
}

fn ids(text: &str) -> Vec<i64> {
    text.split_ascii_whitespace().take(MAX_LIST).filter_map(|t| t.parse::<i64>().ok()).collect()
}

fn by_id<'a>(root: &'a Element, list: &str, item: &'a str) -> HashMap<i64, &'a Element> {
    root.child(list).map(|l| l.children(item).filter_map(|e| Some((e.attr("id")?.trim().parse::<i64>().ok()?, e))).collect()).unwrap_or_default()
}

fn property<'a>(props: Option<&'a Element>, name: &str) -> Option<&'a Element> {
    props?.children("Property").find(|p| p.attr("name") == Some(name))
}

fn num<T: std::str::FromStr>(e: Option<&Element>, child: &str) -> Option<T> {
    e?.text_at(&[child])?.parse().ok()
}

/// Rhythm ticks: note value, augmentation dots and a tuplet.
fn rhythm_ticks(r: &Element) -> i64 {
    let base = match r.text_at(&["NoteValue"]).unwrap_or("Quarter") {
        "DoubleWhole" => WHOLE * 2,
        "Whole" => WHOLE,
        "Half" => WHOLE / 2,
        "Quarter" => WHOLE / 4,
        "Eighth" => WHOLE / 8,
        "16th" => WHOLE / 16,
        "32nd" => WHOLE / 32,
        "64th" => WHOLE / 64,
        "128th" => WHOLE / 128,
        "256th" => WHOLE / 256,
        _ => WHOLE / 4,
    };
    let dots = r.child("AugmentationDot").and_then(|d| d.attr("count")).and_then(|c| c.trim().parse::<u32>().ok()).unwrap_or(0).min(3);
    let mut t = base;
    let mut add = base;
    for _ in 0..dots {
        add /= 2;
        t += add;
    }
    if let Some(tu) = r.child("PrimaryTuplet") {
        let n = tu.attr("num").and_then(|v| v.trim().parse::<i64>().ok()).unwrap_or(1).clamp(1, 64);
        let d = tu.attr("den").and_then(|v| v.trim().parse::<i64>().ok()).unwrap_or(1).clamp(1, 64);
        t = t * d / n;
    }
    t.max(1)
}

fn velocity(dynamic: &str) -> u8 {
    match dynamic.trim() {
        "PPP" => 15,
        "PP" => 31,
        "P" => 47,
        "MP" => 63,
        "MF" => 79,
        "FF" => 111,
        "FFF" => 127,
        _ => 95,
    }
}

/// MIDI pitch of a written pitch (`Step`, `Accidental`, `Octave`, middle C = C4).
fn spelled(p: &Element) -> Option<u8> {
    let step = match p.text_at(&["Step"])? {
        "C" => 0,
        "D" => 2,
        "E" => 4,
        "F" => 5,
        "G" => 7,
        "A" => 9,
        "B" => 11,
        _ => return None,
    };
    let alter = match p.text_at(&["Accidental"]).unwrap_or("") {
        "#" => 1,
        "##" | "x" => 2,
        "b" => -1,
        "bb" => -2,
        _ => 0,
    };
    let oct: i32 = p.text_at(&["Octave"])?.parse().ok()?;
    u8::try_from((oct.clamp(-2, 10) + 1) * 12 + step + alter).ok().filter(|v| *v <= 127)
}

fn parse_track(t: &Element) -> GpTrack {
    let name = t.text_at(&["Name"]).unwrap_or("").chars().take(soundcraft_model::MAX_NAME_CHARS).collect::<String>();
    let color = t
        .text_at(&["Color"])
        .map(|c| {
            let v: Vec<u8> = c.split_ascii_whitespace().take(3).filter_map(|x| x.parse::<u8>().ok()).collect();
            [v.first().copied().unwrap_or(128), v.get(1).copied().unwrap_or(128), v.get(2).copied().unwrap_or(128)]
        })
        .unwrap_or([128, 128, 128]);
    let set_type = t.text_at(&["InstrumentSet", "Type"]).unwrap_or("");
    // Guitar Pro 7 names the channel under MidiConnection, Guitar Pro 6 under GeneralMidi.
    let channel =
        num::<u8>(t.child("MidiConnection"), "PrimaryChannel").or_else(|| num::<u8>(t.child("GeneralMidi"), "PrimaryChannel")).unwrap_or(0).min(15);
    let program = t
        .path(&["Sounds", "Sound", "MIDI", "Program"])
        .and_then(|p| p.text.trim().parse::<u8>().ok())
        .or_else(|| num::<u8>(t.child("GeneralMidi"), "Program"))
        .unwrap_or(25)
        .min(127);
    let staff_props = t.path(&["Staves", "Staff", "Properties"]).or_else(|| t.child("Properties"));
    let tuning: Vec<u8> = property(staff_props, "Tuning")
        .and_then(|p| p.text_at(&["Pitches"]))
        .map(|s| {
            s.split_ascii_whitespace().take(soundcraft_midi::tab::MAX_STRINGS).filter_map(|x| x.parse::<u8>().ok()).map(|p| p.min(127)).collect()
        })
        .unwrap_or_default();
    let capo = property(staff_props, "CapoFret").and_then(|p| p.text_at(&["Fret"])).and_then(|f| f.parse::<u8>().ok()).unwrap_or(0);
    let gp6_kit = t.child("GeneralMidi").and_then(|g| g.attr("table")).is_some_and(|v| v.trim() == "Percussion");
    let drums = set_type == "drumKit" || channel == 9 || gp6_kit;
    let kind = if drums {
        GpKind::Drums
    } else if !tuning.is_empty() && tuning.iter().any(|p| *p > 0) {
        GpKind::Stringed { tuning, capo: capo.min(soundcraft_midi::tab::MAX_FRET) }
    } else {
        GpKind::Other
    };
    GpTrack { name, color, kind, program, channel: if drums { 9 } else { channel }, bars: Vec::new() }
}

/// Drum articulations of a track: index → the GM key it plays.
fn articulations(t: &Element) -> Vec<Option<u8>> {
    t.path(&["InstrumentSet", "Elements"])
        .map(|els| {
            els.children("Element")
                .flat_map(|e| e.child("Articulations").into_iter().flat_map(|a| a.children("Articulation")))
                .map(|a| a.text_at(&["OutputMidiNumber"]).and_then(|n| n.parse::<u8>().ok()).filter(|n| *n <= 127))
                .take(MAX_LIST)
                .collect()
        })
        .unwrap_or_default()
}

/// Parse a `score.gpif` document.
pub(crate) fn parse_gpif(text: &str) -> Result<GpScore, String> {
    let root = xml::parse(text)?;
    if root.name != "GPIF" {
        return Err("not a Guitar Pro score (no GPIF root)".into());
    }
    let mut score = GpScore {
        title: root.text_at(&["Score", "Title"]).unwrap_or("").chars().take(200).collect(),
        artist: root.text_at(&["Score", "Artist"]).unwrap_or("").chars().take(200).collect(),
        ..GpScore::default()
    };
    let track_els: Vec<&Element> = root.child("Tracks").map(|t| t.children("Track").take(MAX_TRACKS).collect()).unwrap_or_default();
    if track_els.is_empty() {
        return Err("the score has no tracks".into());
    }
    let mut tracks: Vec<GpTrack> = track_els.iter().map(|t| parse_track(t)).collect();
    let arts: Vec<Vec<Option<u8>>> = track_els.iter().map(|t| articulations(t)).collect();
    let (bars, voices, beats, notes, rhythms) = (
        by_id(&root, "Bars", "Bar"),
        by_id(&root, "Voices", "Voice"),
        by_id(&root, "Beats", "Beat"),
        by_id(&root, "Notes", "Note"),
        by_id(&root, "Rhythms", "Rhythm"),
    );
    let rhythm_of: HashMap<i64, i64> = rhythms.iter().map(|(k, r)| (*k, rhythm_ticks(r))).collect();
    let masters: Vec<&Element> = root.child("MasterBars").map(|m| m.children("MasterBar").take(MAX_BARS).collect()).unwrap_or_default();
    if masters.is_empty() {
        return Err("the score has no bars".into());
    }
    for m in &masters {
        let (numerator, denominator) = m
            .text_at(&["Time"])
            .and_then(|t| t.split_once('/'))
            .and_then(|(a, b)| Some((a.trim().parse::<u32>().ok()?, b.trim().parse::<u32>().ok()?)))
            .filter(|(a, b)| (1..=64).contains(a) && [1, 2, 4, 8, 16, 32, 64].contains(b))
            .unwrap_or((4, 4));
        let rep = m.child("Repeat");
        let flag = |k: &str| rep.and_then(|r| r.attr(k)).is_some_and(|v| v.trim() == "true");
        let repeat_count = rep.and_then(|r| r.attr("count")).and_then(|v| v.trim().parse::<u32>().ok()).unwrap_or(2).clamp(1, MAX_REPEAT);
        let endings = m.text_at(&["AlternateEndings"]).map_or(0, |s| {
            s.split_ascii_whitespace()
                .filter_map(|x| x.parse::<u32>().ok())
                .filter(|n| (1..=32).contains(n))
                .fold(0u32, |acc, n| acc | (1 << (n - 1)))
        });
        score.bars.push(GpMasterBar { numerator, denominator, repeat_start: flag("start"), repeat_end: flag("end"), repeat_count, endings });
        let bar_ids = m.text_at(&["Bars"]).map(ids).unwrap_or_default();
        let bar_len = score.bars.last().map_or(WHOLE, GpMasterBar::ticks);
        for (ti, track) in tracks.iter_mut().enumerate() {
            let mut voices_out = Vec::new();
            let bar = bar_ids.get(ti).and_then(|id| bars.get(id));
            for vid in bar.and_then(|b| b.text_at(&["Voices"])).map(ids).unwrap_or_default() {
                let Some(v) = voices.get(&vid) else { continue };
                let mut out = Vec::new();
                let mut used = 0i64;
                for bid in v.text_at(&["Beats"]).map(ids).unwrap_or_default() {
                    let Some(b) = beats.get(&bid) else { continue };
                    let grace = match b.text_at(&["GraceNotes"]).map(str::trim) {
                        None => Grace::None,
                        Some("OnBeat") => Grace::On,
                        Some(_) => Grace::Before,
                    };
                    let ticks = b
                        .child("Rhythm")
                        .and_then(|r| r.attr("ref"))
                        .and_then(|r| r.trim().parse::<i64>().ok())
                        .and_then(|r| rhythm_of.get(&r))
                        .copied()
                        .unwrap_or(WHOLE / 4);
                    // A voice that runs past its bar still plays (as TabGarden plays it), up to a
                    // few bars' worth so a hostile file cannot run on for ever.
                    let ticks = if grace == Grace::None { ticks.min(bar_len.saturating_mul(MAX_OVERRUN).saturating_sub(used)) } else { GRACE_TICKS };
                    if ticks <= 0 {
                        break;
                    }
                    if grace == Grace::None {
                        used += ticks;
                    }
                    let vel = velocity(b.text_at(&["Dynamic"]).unwrap_or(""));
                    let mut ns = Vec::new();
                    for nid in b.text_at(&["Notes"]).map(ids).unwrap_or_default().into_iter().take(16) {
                        let Some(n) = notes.get(&nid) else { continue };
                        if let Some(note) = parse_note(n, &track.kind, arts.get(ti).map_or(&[][..], Vec::as_slice)) {
                            ns.push(note);
                        }
                    }
                    // "1/2", "1/4", "1/8": strokes of an eighth, a 16th, a 32nd.
                    let tremolo = b.text_at(&["Tremolo"]).and_then(|t| {
                        let (n, d) = t.trim().split_once('/')?;
                        let (n, d) = (n.trim().parse::<i64>().ok()?, d.trim().parse::<i64>().ok()?);
                        (n > 0 && d > 0 && d <= 64).then(|| (WHOLE / 4).saturating_mul(n) / d).filter(|t| *t >= 30)
                    });
                    out.push(GpBeat { ticks, velocity: vel, notes: ns, tremolo: tremolo.unwrap_or(0), grace });
                }
                voices_out.push(out);
            }
            track.bars.push(voices_out);
        }
    }
    // Tempo: the master track's tempo automations ("bpm unit", unit 2 = quarter notes).
    if let Some(autos) = root.path(&["MasterTrack", "Automations"]) {
        for a in autos.children("Automation").take(MAX_LIST) {
            if a.text_at(&["Type"]) != Some("Tempo") {
                continue;
            }
            let mut v = a.text_at(&["Value"]).unwrap_or("").split_ascii_whitespace();
            let Some(bpm) = v.next().and_then(|x| x.parse::<f64>().ok()).filter(|b| b.is_finite() && *b > 0.0) else { continue };
            let factor = match v.next().and_then(|u| u.parse::<u32>().ok()).unwrap_or(2) {
                1 => 0.5,
                3 => 1.5,
                4 => 2.0,
                5 => 3.0,
                _ => 1.0,
            };
            let bar = a.text_at(&["Bar"]).and_then(|b| b.parse::<usize>().ok()).unwrap_or(0).min(score.bars.len().saturating_sub(1));
            let position = a.text_at(&["Position"]).and_then(|p| p.parse::<f64>().ok()).filter(|p| p.is_finite()).unwrap_or(0.0).clamp(0.0, 1.0);
            score.tempos.push(GpTempo { bar, position, bpm: (bpm * factor).clamp(5.0, 999.0) });
        }
    }
    score.tracks = tracks;
    Ok(score)
}

fn parse_note(n: &Element, kind: &GpKind, arts: &[Option<u8>]) -> Option<GpNote> {
    let props = n.child("Properties");
    let midi = property(props, "Midi").and_then(|p| p.text_at(&["Number"])).and_then(|x| x.parse::<u8>().ok()).filter(|p| *p <= 127);
    let string = property(props, "String").and_then(|p| p.text_at(&["String"])).and_then(|x| x.parse::<u8>().ok());
    let fret = property(props, "Fret").and_then(|p| p.text_at(&["Fret"])).and_then(|x| x.parse::<u8>().ok());
    let tie = n.child("Tie");
    let tie_in = tie.and_then(|t| t.attr("destination")).is_some_and(|v| v.trim() == "true");
    let tie_out = tie.and_then(|t| t.attr("origin")).is_some_and(|v| v.trim() == "true");
    let pitch = match kind {
        GpKind::Drums => {
            let art = n.text_at(&["InstrumentArticulation"]).and_then(|a| a.parse::<usize>().ok()).and_then(|i| arts.get(i).copied().flatten());
            // Guitar Pro 6 names a drum by the kit piece and how it is struck.
            let piece = || {
                let element = property(props, "Element").and_then(|p| p.text_at(&["Element"])).and_then(|x| x.trim().parse::<usize>().ok())?;
                let variation =
                    property(props, "Variation").and_then(|p| p.text_at(&["Variation"])).and_then(|x| x.trim().parse::<usize>().ok()).unwrap_or(0);
                drum_piece_key(element, variation)
            };
            art.or(midi).or_else(piece)
        }
        GpKind::Stringed { tuning, .. } => midi.or_else(|| {
            let open = *tuning.get(usize::from(string?))?;
            u8::try_from(u16::from(open) + u16::from(fret?)).ok().filter(|p| *p <= 127)
        }),
        GpKind::Other => None,
    }
    .or(midi)
    .or_else(|| property(props, "ConcertPitch").and_then(|p| p.child("Pitch")).and_then(spelled))?;
    let string = match kind {
        GpKind::Stringed { tuning, .. } => string.filter(|s| usize::from(*s) < tuning.len()).zip(fret.map(|f| f.min(soundcraft_midi::tab::MAX_FRET))),
        _ => None,
    };
    Some(GpNote { pitch, string, tie_in, tie_out, tech: techniques(n, props) })
}

/// The General MIDI drum key of a Guitar Pro 6 kit piece (`Element`) struck a given way
/// (`Variation`), as TabGarden plays it; an unknown way falls back to the piece's first.
///
/// Pieces 0, 1, 6, 7, 12, 13, 15 and 16 were checked against the MIDI TabGarden writes for real
/// Guitar Pro 6 files (its rim shot plays 38 and its ride bell 59). The others follow the kit's
/// order and have not been checked yet.
fn drum_piece_key(element: usize, variation: usize) -> Option<u8> {
    // Per piece, its keys by variation.
    const KIT: [&[u8]; 17] = [
        &[35],         // kick
        &[38, 38, 37], // snare: hit, rim shot, side stick
        &[42, 46, 92], // hi-hat: closed, open, half
        &[44],         // pedal hi-hat
        &[41],         // very low tom
        &[43],         // low floor tom
        &[45],         // low tom
        &[47],         // mid tom
        &[48],         // high tom
        &[50],         // very high tom
        &[56],         // cowbell
        &[54],         // tambourine
        &[57],         // medium crash
        &[49],         // high crash
        &[55],         // splash
        &[51, 59, 59], // ride: middle, edge, bell
        &[52],         // china
    ];
    let keys = KIT.get(element)?;
    keys.get(variation).or_else(|| keys.first()).copied()
}

/// A note's playing techniques, as `soundcraft_midi::tab::tech` bits.
fn techniques(n: &Element, props: Option<&Element>) -> u16 {
    let on = |name: &str| property(props, name).is_some_and(|p| p.child("Enable").is_some());
    let mut t = 0;
    for (name, bit) in [("Muted", tech::DEAD), ("PalmMuted", tech::PALM_MUTE), ("HopoOrigin", tech::HOPO), ("HopoDestination", tech::HOPO_DEST)] {
        if on(name) {
            t |= bit;
        }
    }
    let slide = property(props, "Slide").and_then(|p| p.text_at(&["Flags"])).and_then(|f| f.trim().parse::<u32>().ok()).unwrap_or(0);
    for (flag, bit) in [
        (1, tech::SLIDE_SHIFT),
        (2, tech::SLIDE_LEGATO),
        (4, tech::SLIDE_OUT_DOWN),
        (8, tech::SLIDE_OUT_UP),
        (16, tech::SLIDE_IN_BELOW),
        (32, tech::SLIDE_IN_ABOVE),
    ] {
        if slide & flag != 0 {
            t |= bit;
        }
    }
    if n.child("LetRing").is_some() {
        t |= tech::LET_RING;
    }
    if n.child("Vibrato").is_some() {
        t |= tech::VIBRATO;
    }
    // Accent flags: 1 staccato, 4 accent, 8 heavy accent.
    let accent = n.text_at(&["Accent"]).and_then(|a| a.trim().parse::<u32>().ok()).unwrap_or(0);
    if accent & 1 != 0 {
        t |= tech::STACCATO;
    }
    if accent & 12 != 0 {
        t |= tech::ACCENT;
    }
    t
}

/// The master bars in the order they play: repeats and alternate endings played out.
pub(crate) fn play_order(bars: &[GpMasterBar]) -> Vec<usize> {
    let mut order = Vec::new();
    let (mut i, mut start, mut pass, mut after_ending) = (0usize, 0usize, 1u32, false);
    // Count steps, so a hostile set of repeats cannot loop for ever.
    let mut steps = 0usize;
    while let Some(b) = bars.get(i) {
        steps += 1;
        if order.len() >= MAX_BARS || steps > MAX_BARS * 4 {
            break;
        }
        // The bar after the last alternate ending: the repeated section is over.
        if b.endings == 0 && after_ending {
            pass = 1;
            start = i;
            after_ending = false;
        }
        if b.repeat_start && pass == 1 {
            start = i;
        }
        let plays = b.endings == 0 || pass > 32 || b.endings & (1u32 << (pass - 1)) != 0;
        if plays {
            order.push(i);
            after_ending |= b.endings != 0;
        }
        if b.repeat_end && plays {
            if pass < b.repeat_count {
                pass += 1;
                i = start;
                after_ending = false;
                continue;
            }
            pass = 1;
            start = i + 1;
            after_ending = false;
        }
        i += 1;
    }
    order
}

// ─── Writing ──────────────────────────────────────────────────────────────────────────────────

/// Note value and dots for a beat of `ticks`, when one notehead can show it.
fn note_value(ticks: i64) -> Option<(&'static str, u32)> {
    const VALUES: [(&str, i64); 8] = [
        ("Whole", WHOLE),
        ("Half", WHOLE / 2),
        ("Quarter", WHOLE / 4),
        ("Eighth", WHOLE / 8),
        ("16th", WHOLE / 16),
        ("32nd", WHOLE / 32),
        ("64th", WHOLE / 64),
        ("128th", WHOLE / 128),
    ];
    for (name, base) in VALUES {
        if ticks == base {
            return Some((name, 0));
        }
        if ticks == base + base / 2 {
            return Some((name, 1));
        }
    }
    None
}

/// Split `ticks` into lengths one notehead each can show (longest first).
pub(crate) fn split_ticks(mut ticks: i64) -> Vec<i64> {
    let mut out = Vec::new();
    let mut unit = WHOLE + WHOLE / 2;
    while ticks > 0 && unit >= WHOLE / 128 {
        if ticks >= unit && note_value(unit).is_some() {
            out.push(unit);
            ticks -= unit;
            continue;
        }
        // Next smaller shape: dotted then plain, halving.
        unit = if note_value(unit).is_some_and(|(_, d)| d == 1) { unit * 2 / 3 } else { unit * 3 / 4 };
    }
    out
}

/// The drum keys a track plays, lowest first: its articulation list.
fn drum_keys(t: &GpTrack) -> Vec<u8> {
    let mut keys: Vec<u8> = t.bars.iter().flatten().flatten().flat_map(|b| b.notes.iter().map(|n| n.pitch)).collect();
    keys.sort_unstable();
    keys.dedup();
    if keys.is_empty() {
        keys.push(36);
    }
    keys
}

fn drum_articulation_xml(keys: &[u8]) -> String {
    let mut x = String::new();
    for k in keys {
        let name = soundcraft_midi::gm_drum_name(*k).unwrap_or("Drum");
        let _ = write!(
            x,
            "<Articulation><Name>{}</Name><StaffLine>0</StaffLine><Noteheads>noteheadBlack noteheadHalf noteheadWhole</Noteheads><TechniquePlacement>outside</TechniquePlacement><TechniqueSymbol></TechniqueSymbol><InputMidiNumbers>{k}</InputMidiNumbers><OutputRSESound></OutputRSESound><OutputMidiNumber>{k}</OutputMidiNumber></Articulation>",
            xml::escape(name)
        );
    }
    x
}

/// Write `score` as a `score.gpif` document.
pub(crate) fn write_gpif(score: &GpScore) -> String {
    let mut x = String::with_capacity(64 * 1024);
    x.push_str("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<GPIF><GPVersion>8.1.3</GPVersion><GPRevision required=\"12024\" recommended=\"13000\">13007</GPRevision><Encoding><EncodingDescription>GP8</EncodingDescription></Encoding>");
    let _ = write!(
        x,
        "<Score><Title>{}</Title><SubTitle>{}</SubTitle><Artist>{}</Artist><Album>{}</Album><Words>{}</Words><Music>{}</Music><WordsAndMusic>{}</WordsAndMusic><Copyright>{}</Copyright><Tabber>{}</Tabber><Instructions>{}</Instructions><Notices>{}</Notices><FirstPageHeader>{}</FirstPageHeader><FirstPageFooter>{}</FirstPageFooter><PageHeader>{}</PageHeader><PageFooter>{}</PageFooter><ScoreSystemsDefaultLayout>3</ScoreSystemsDefaultLayout><ScoreSystemsLayout></ScoreSystemsLayout><ScoreZoomPolicy>Value</ScoreZoomPolicy><ScoreZoom>1</ScoreZoom><MultiVoice>1&gt;</MultiVoice></Score>",
        xml::cdata(&score.title),
        xml::cdata(""),
        xml::cdata(&score.artist),
        xml::cdata(""),
        xml::cdata(""),
        xml::cdata(""),
        xml::cdata(""),
        xml::cdata(""),
        xml::cdata(""),
        xml::cdata(""),
        xml::cdata(""),
        xml::cdata(""),
        xml::cdata(""),
        xml::cdata(""),
        xml::cdata(""),
    );
    // Master track: the track list and the tempo changes.
    let track_list: Vec<String> = (0..score.tracks.len()).map(|i| i.to_string()).collect();
    let _ = write!(x, "<MasterTrack><Tracks>{}</Tracks><Automations>", track_list.join(" "));
    let tempos = if score.tempos.is_empty() { vec![GpTempo { bar: 0, position: 0.0, bpm: 120.0 }] } else { score.tempos.clone() };
    for t in &tempos {
        let _ = write!(
            x,
            "<Automation><Type>Tempo</Type><Linear>false</Linear><Bar>{}</Bar><Position>{}</Position><Visible>true</Visible><Value>{} 2</Value></Automation>",
            t.bar,
            t.position,
            (t.bpm * 1000.0).round() / 1000.0
        );
    }
    x.push_str("</Automations></MasterTrack><AudioTracks/><Tracks>");
    for (ti, t) in score.tracks.iter().enumerate() {
        let (set_name, set_type, lines, style) = match &t.kind {
            GpKind::Stringed { tuning, .. } if tuning.len() <= 4 && tuning.iter().max().is_some_and(|p| *p < 50) => {
                ("Electric Bass", "electricBass", 5, "StringedPick")
            }
            GpKind::Stringed { .. } => ("Electric Guitar", "electricGuitar", 5, "StringedPick"),
            GpKind::Drums => ("Drums", "drumKit", 5, "Default"),
            GpKind::Other => ("Piano", "acousticPiano", 5, "Default"),
        };
        let _ = write!(
            x,
            "<Track id=\"{ti}\"><Name>{}</Name><ShortName>{}</ShortName><Color>{} {} {}</Color><SystemsDefautLayout>3</SystemsDefautLayout><SystemsLayout></SystemsLayout><AutoBrush/><PalmMute>0</PalmMute><PlayingStyle>{style}</PlayingStyle><UseOneChannelPerString/><IconId>{}</IconId><InstrumentSet><Name>{set_name}</Name><Type>{set_type}</Type><LineCount>{lines}</LineCount><Elements>",
            xml::cdata(&t.name),
            xml::cdata(&t.name.chars().take(8).collect::<String>()),
            t.color[0],
            t.color[1],
            t.color[2],
            if matches!(t.kind, GpKind::Drums) { 7 } else { 5 },
        );
        if matches!(t.kind, GpKind::Drums) {
            let keys = drum_keys(t);
            let _ = write!(
                x,
                "<Element><Name>Drums</Name><Type>drums</Type><SoundbankName></SoundbankName><Articulations>{}</Articulations></Element>",
                drum_articulation_xml(&keys)
            );
        } else {
            x.push_str("<Element><Name>Pitched</Name><Type>pitched</Type><SoundbankName></SoundbankName><Articulations><Articulation><Name></Name><StaffLine>0</StaffLine><Noteheads>noteheadBlack noteheadHalf noteheadWhole</Noteheads><TechniquePlacement>outside</TechniquePlacement><TechniqueSymbol></TechniqueSymbol><InputMidiNumbers></InputMidiNumbers><OutputRSESound></OutputRSESound><OutputMidiNumber>0</OutputMidiNumber></Articulation></Articulations></Element>");
        }
        let channel = t.channel.min(15);
        let secondary = if channel == 9 { 9 } else { (channel + 1).min(15) };
        let _ = write!(
            x,
            "</Elements></InstrumentSet><Transpose><Chromatic>0</Chromatic><Octave>0</Octave></Transpose><ForcedSound>-1</ForcedSound><MidiConnection><Port>1</Port><PrimaryChannel>{channel}</PrimaryChannel><SecondaryChannel>{secondary}</SecondaryChannel><ForeOneChannelPerString>false</ForeOneChannelPerString></MidiConnection><PlaybackState>Default</PlaybackState><AudioEngineState>MIDI</AudioEngineState><Lyrics dispatched=\"true\"/><Staves><Staff><Properties>"
        );
        let (pitches, capo) = match &t.kind {
            GpKind::Stringed { tuning, capo } => (tuning.iter().map(u8::to_string).collect::<Vec<_>>().join(" "), *capo),
            _ => ("0 0 0 0 0 0".to_string(), 0),
        };
        let _ = write!(
            x,
            "<Property name=\"CapoFret\"><Fret>{capo}</Fret></Property><Property name=\"FretCount\"><Fret>24</Fret></Property><Property name=\"Tuning\"><Pitches>{pitches}</Pitches><Label>{}</Label><LabelVisible>true</LabelVisible><Flat/><Instrument>{}</Instrument></Property><Property name=\"PartialCapoFret\"><Fret>0</Fret></Property><Property name=\"PartialCapoStringFlags\"><Bitset>000000</Bitset></Property><Property name=\"DiagramCollection\"><Items/></Property><Property name=\"DiagramWorkingSet\"><Items/></Property></Properties></Staff></Staves>",
            xml::cdata(""),
            if matches!(t.kind, GpKind::Stringed { .. }) { "Guitar" } else { "Undefined" },
        );
        let program = t.program.min(127);
        let sound_name = format!("Track_{ti}_Initial");
        let path = if matches!(t.kind, GpKind::Drums) { "Midi/0".to_string() } else { format!("Midi/{program}") };
        let _ = write!(
            x,
            "<Sounds><Sound><Name>{}</Name><Label>{}</Label><Path>{}</Path><Role>{}</Role><MIDI><LSB>0</LSB><MSB>0</MSB><Program>{program}</Program></MIDI></Sound></Sounds><Automations><Automation><Type>Sound</Type><Linear>false</Linear><Bar>0</Bar><Position>0</Position><Visible>true</Visible><Value>{}</Value></Automation></Automations></Track>",
            xml::cdata(&sound_name),
            xml::cdata(&sound_name),
            xml::cdata(&path),
            xml::cdata("Factory"),
            xml::cdata(&format!("{path};{sound_name};Factory")),
        );
    }
    x.push_str("</Tracks>");
    write_bars(score, &mut x);
    x.push_str("</GPIF>");
    x
}

fn write_bars(score: &GpScore, x: &mut String) {
    let tracks = score.tracks.len();
    let (mut bars, mut voices, mut beats, mut notes) = (String::new(), String::new(), String::new(), String::new());
    let mut rhythms: Vec<(&'static str, u32)> = Vec::new();
    let (mut bar_id, mut voice_id, mut beat_id, mut note_id) = (0usize, 0usize, 0usize, 0usize);
    x.push_str("<MasterBars>");
    for (mi, mb) in score.bars.iter().enumerate() {
        let mut ids = Vec::with_capacity(tracks);
        for t in &score.tracks {
            let keys = if matches!(t.kind, GpKind::Drums) { drum_keys(t) } else { Vec::new() };
            ids.push(bar_id.to_string());
            let mut vids = Vec::new();
            let empty = Vec::new();
            let bar_voices = t.bars.get(mi).unwrap_or(&empty);
            let mut written: Vec<&[GpBeat]> = bar_voices.iter().filter(|v| !v.is_empty()).map(Vec::as_slice).take(4).collect();
            let rest = [GpBeat::new(mb.ticks(), 95, Vec::new())];
            if written.is_empty() {
                written.push(&rest);
            }
            for v in written {
                let mut bids = Vec::new();
                for b in v {
                    for (k, piece) in split_ticks(b.ticks).into_iter().enumerate() {
                        let Some(value) = note_value(piece) else { continue };
                        let rid = rhythms.iter().position(|r| *r == value).unwrap_or_else(|| {
                            rhythms.push(value);
                            rhythms.len() - 1
                        });
                        let pieces = split_ticks(b.ticks).len();
                        let mut nids = Vec::new();
                        for n in &b.notes {
                            nids.push(note_id.to_string());
                            let art = keys.iter().position(|key| *key == n.pitch).unwrap_or(0);
                            write_note(&mut notes, note_id, n, &t.kind, art, (k > 0 || n.tie_in, k + 1 < pieces || n.tie_out));
                            note_id += 1;
                        }
                        let dynamic = match b.velocity {
                            0..=22 => "PPP",
                            23..=38 => "PP",
                            39..=54 => "P",
                            55..=70 => "MP",
                            71..=86 => "MF",
                            87..=102 => "F",
                            103..=118 => "FF",
                            _ => "FFF",
                        };
                        let _ = write!(beats, "<Beat id=\"{beat_id}\"><Dynamic>{dynamic}</Dynamic><Rhythm ref=\"{rid}\"/>");
                        if !nids.is_empty() {
                            let _ = write!(beats, "<Notes>{}</Notes>", nids.join(" "));
                        }
                        beats.push_str("<Properties/><XProperties/></Beat>");
                        bids.push(beat_id.to_string());
                        beat_id += 1;
                    }
                }
                let _ = write!(voices, "<Voice id=\"{voice_id}\"><Beats>{}</Beats></Voice>", bids.join(" "));
                vids.push(voice_id.to_string());
                voice_id += 1;
            }
            while vids.len() < 4 {
                vids.push("-1".into());
            }
            let clef = match t.kind {
                GpKind::Drums => "Neutral",
                GpKind::Stringed { ref tuning, .. } if tuning.len() <= 4 && tuning.iter().max().is_some_and(|p| *p < 50) => "F4",
                _ => "G2",
            };
            let _ = write!(bars, "<Bar id=\"{bar_id}\"><Voices>{}</Voices><Clef>{clef}</Clef></Bar>", vids.join(" "));
            bar_id += 1;
        }
        let _ = write!(
            x,
            "<MasterBar><Key><AccidentalCount>0</AccidentalCount><Mode>Major</Mode><Sharps>Sharps</Sharps></Key><Time>{}/{}</Time>",
            mb.numerator, mb.denominator
        );
        if mb.repeat_start || mb.repeat_end {
            let _ = write!(
                x,
                "<Repeat start=\"{}\" end=\"{}\" count=\"{}\"/>",
                mb.repeat_start,
                mb.repeat_end,
                if mb.repeat_end { mb.repeat_count } else { 0 }
            );
        }
        let _ = write!(x, "<Bars>{}</Bars></MasterBar>", ids.join(" "));
    }
    x.push_str("</MasterBars>");
    let _ = write!(x, "<Bars>{bars}</Bars><Voices>{voices}</Voices><Beats>{beats}</Beats><Notes>{notes}</Notes><Rhythms>");
    for (i, (value, dots)) in rhythms.iter().enumerate() {
        let _ = write!(x, "<Rhythm id=\"{i}\"><NoteValue>{value}</NoteValue>");
        if *dots > 0 {
            let _ = write!(x, "<AugmentationDot count=\"{dots}\"/>");
        }
        x.push_str("</Rhythm>");
    }
    x.push_str("</Rhythms>");
}

fn write_note(x: &mut String, id: usize, n: &GpNote, kind: &GpKind, articulation: usize, (tie_in, tie_out): (bool, bool)) {
    let _ = write!(x, "<Note id=\"{id}\">");
    if tie_in || tie_out {
        let _ = write!(x, "<Tie origin=\"{tie_out}\" destination=\"{tie_in}\"/>");
    }
    const STEPS: [(&str, &str); 12] =
        [("C", ""), ("C", "#"), ("D", ""), ("D", "#"), ("E", ""), ("F", ""), ("F", "#"), ("G", ""), ("G", "#"), ("A", ""), ("A", "#"), ("B", "")];
    let (step, acc) = STEPS.get(usize::from(n.pitch % 12)).copied().unwrap_or(("C", ""));
    let octave = i32::from(n.pitch / 12) - 1;
    let _ = write!(
        x,
        "<Properties><Property name=\"ConcertPitch\"><Pitch><Step>{step}</Step><Accidental>{acc}</Accidental><Octave>{octave}</Octave></Pitch></Property>"
    );
    match (kind, n.string) {
        (GpKind::Stringed { .. }, Some((s, f))) => {
            let _ = write!(x, "<Property name=\"String\"><String>{s}</String></Property><Property name=\"Fret\"><Fret>{f}</Fret></Property>");
        }
        (GpKind::Drums, _) => {
            let _ = write!(x, "<Property name=\"String\"><String>0</String></Property><Property name=\"Fret\"><Fret>{}</Fret></Property>", n.pitch);
        }
        _ => {}
    }
    let _ = write!(x, "<Property name=\"Midi\"><Number>{}</Number></Property>", n.pitch);
    // Techniques go on the note as picked, not on the tied pieces after it.
    let t = if tie_in { 0 } else { n.tech };
    for (bit, name) in [(tech::DEAD, "Muted"), (tech::PALM_MUTE, "PalmMuted"), (tech::HOPO, "HopoOrigin"), (tech::HOPO_DEST, "HopoDestination")] {
        if t & bit != 0 {
            let _ = write!(x, "<Property name=\"{name}\"><Enable/></Property>");
        }
    }
    let slide = [
        (tech::SLIDE_SHIFT, 1),
        (tech::SLIDE_LEGATO, 2),
        (tech::SLIDE_OUT_DOWN, 4),
        (tech::SLIDE_OUT_UP, 8),
        (tech::SLIDE_IN_BELOW, 16),
        (tech::SLIDE_IN_ABOVE, 32),
    ]
    .iter()
    .filter(|(bit, _)| t & bit != 0)
    .fold(0u32, |acc, (_, flag)| acc | flag);
    if slide != 0 {
        let _ = write!(x, "<Property name=\"Slide\"><Flags>{slide}</Flags></Property>");
    }
    let _ = write!(x, "</Properties>");
    if t & tech::LET_RING != 0 {
        let _ = write!(x, "<LetRing/>");
    }
    if t & tech::VIBRATO != 0 {
        let _ = write!(x, "<Vibrato>Slight</Vibrato>");
    }
    let accent = if t & tech::STACCATO != 0 { 1 } else { 0 } | if t & tech::ACCENT != 0 { 4 } else { 0 };
    if accent != 0 {
        let _ = write!(x, "<Accent>{accent}</Accent>");
    }
    let _ = write!(x, "<InstrumentArticulation>{articulation}</InstrumentArticulation></Note>");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rhythms_dots_and_tuplets() {
        let r = |s: &str| rhythm_ticks(&xml::parse(s).unwrap());
        assert_eq!(r("<Rhythm><NoteValue>Quarter</NoteValue></Rhythm>"), 960);
        assert_eq!(r("<Rhythm><NoteValue>Eighth</NoteValue><AugmentationDot count=\"1\"/></Rhythm>"), 720);
        assert_eq!(r("<Rhythm><NoteValue>Eighth</NoteValue><PrimaryTuplet num=\"3\" den=\"2\"/></Rhythm>"), 320);
        assert_eq!(r("<Rhythm><NoteValue>Half</NoteValue><AugmentationDot count=\"2\"/></Rhythm>"), 3360);
        assert_eq!(r("<Rhythm><NoteValue>Bogus</NoteValue><PrimaryTuplet num=\"0\" den=\"999\"/></Rhythm>"), 960 * 64);
    }

    #[test]
    fn lengths_split_into_noteheads() {
        assert_eq!(split_ticks(3840), [3840]);
        assert_eq!(split_ticks(2880), [2880]);
        assert_eq!(split_ticks(1200), [960, 240]);
        assert_eq!(split_ticks(5760), [5760]);
        assert!(split_ticks(7).is_empty() || split_ticks(7).iter().sum::<i64>() <= 7);
        assert_eq!(split_ticks(0), Vec::<i64>::new());
    }

    #[test]
    fn repeats_and_endings_play_out() {
        let b = |s, e, c, endings| GpMasterBar { numerator: 4, denominator: 4, repeat_start: s, repeat_end: e, repeat_count: c, endings };
        // |: 0 1 :| 2
        assert_eq!(play_order(&[b(true, false, 2, 0), b(false, true, 2, 0), b(false, false, 2, 0)]), [0, 1, 0, 1, 2]);
        // |: 0 [1. 1 :| [2. 2 ] 3
        assert_eq!(play_order(&[b(true, false, 2, 0), b(false, true, 2, 1), b(false, false, 2, 2), b(false, false, 2, 0)]), [0, 1, 0, 2, 3]);
        // A repeat with no start goes back to the beginning.
        assert_eq!(play_order(&[b(false, false, 2, 0), b(false, true, 3, 0)]), [0, 1, 0, 1, 0, 1]);
        // Hostile: every bar a repeat end.
        let hostile: Vec<_> = (0..50).map(|_| b(true, true, 16, 0)).collect();
        assert!(play_order(&hostile).len() <= MAX_BARS);
    }
}
