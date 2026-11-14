//! Guitar Pro 3, 4 and 5 scores (`.gp3`, `.gp4`, `.gp5`): a little-endian binary file of song
//! information, 64 MIDI channels, measure headers, tracks, then every measure's beats per track.
//!
//! Written from public descriptions of the format and checked against real files, not from any
//! other program's code. Read into the same score the `.gp` (GPIF) reader makes: tracks with
//! tuning, capo, program and channel; meters, repeats and alternate endings; the tempo and its
//! changes; both voices' beats with their durations (dots and tuplets) and dynamics; notes with
//! string, fret and ties; and the techniques TabGarden plays (dead and palm-muted notes,
//! hammer-ons and pull-offs, slides, let ring, accents, staccato, vibrato, grace notes and tremolo
//! picking). Chord diagrams, bends, harmonics, lyrics, text and the mixer settings are skipped.

use super::gpif::{GRACE_TICKS, GpBeat, GpKind, GpMasterBar, GpNote, GpScore, GpTempo, GpTrack, Grace};
use soundcraft_midi::tab::{MAX_FRET, tech};

const WHOLE: i64 = 3840;
const MAX_TRACKS: usize = 128;
const MAX_MEASURES: usize = 20_000;
const MAX_BEATS: usize = 1024;
const MAX_BEND_POINTS: usize = 64;
const MAX_OVERRUN: i64 = 4;
const MAX_REPEAT: u32 = 16;
/// The 64 MIDI channels of the file (4 ports of 16).
const CHANNELS: usize = 64;

/// Which format a file is, from its version string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Version {
    V3,
    V4,
    /// 5.00, before the 5.10 additions.
    V500,
    V510,
}

/// Whether `bytes` start like a Guitar Pro 3–5 file.
pub(crate) fn sniff(bytes: &[u8]) -> bool {
    bytes.get(1..21).is_some_and(|s| s == b"FICHIER GUITAR PRO v" || s.starts_with(b"FICHIER GUITAR PRO L"))
}

struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

type R<T> = Result<T, String>;

impl Reader<'_> {
    fn take(&mut self, n: usize) -> R<&[u8]> {
        let end = self.pos.checked_add(n).filter(|e| *e <= self.data.len()).ok_or_else(|| format!("the file ends early (at byte {})", self.pos))?;
        let s = self.data.get(self.pos..end).ok_or("the file ends early")?;
        self.pos = end;
        Ok(s)
    }
    fn skip(&mut self, n: usize) -> R<()> {
        self.take(n).map(|_| ())
    }
    fn u8(&mut self) -> R<u8> {
        Ok(self.take(1)?.first().copied().unwrap_or(0))
    }
    fn i8(&mut self) -> R<i8> {
        Ok(i8::from_le_bytes([self.u8()?]))
    }
    fn bool(&mut self) -> R<bool> {
        Ok(self.u8()? != 0)
    }
    fn i16(&mut self) -> R<i16> {
        let b = self.take(2)?;
        Ok(i16::from_le_bytes([b.first().copied().unwrap_or(0), b.get(1).copied().unwrap_or(0)]))
    }
    fn i32(&mut self) -> R<i32> {
        let b = self.take(4)?;
        let mut a = [0u8; 4];
        a.copy_from_slice(b);
        Ok(i32::from_le_bytes(a))
    }
    /// A count from the file, refused past `max`.
    fn count(&mut self, max: usize, what: &str) -> R<usize> {
        let n = self.i32()?;
        usize::try_from(n).ok().filter(|n| *n <= max).ok_or_else(|| format!("{what}: {n} is not a count this reader takes (at most {max})"))
    }
    fn text(bytes: &[u8]) -> String {
        // Guitar Pro 3–5 text is in the writer's Windows code page; Latin-1 keeps ASCII exact.
        bytes.iter().map(|&b| char::from(b)).filter(|c| !c.is_control()).collect::<String>().trim().to_string()
    }
    /// A length byte, then `size` bytes of which the first `length` are the text.
    fn byte_size_string(&mut self, size: usize) -> R<String> {
        let len = usize::from(self.u8()?);
        let b = self.take(size)?;
        Ok(Self::text(b.get(..len.min(size)).unwrap_or_default()))
    }
    /// A 32-bit size, then a length byte and the text.
    fn int_byte_size_string(&mut self) -> R<String> {
        let size = self.count(1 << 16, "text")?;
        if size == 0 {
            return Ok(String::new());
        }
        let len = usize::from(self.u8()?);
        let b = self.take(size - 1)?;
        Ok(Self::text(b.get(..len.min(size - 1)).unwrap_or_default()))
    }
    /// A 32-bit length, then the text.
    fn int_size_string(&mut self) -> R<String> {
        let len = self.count(1 << 20, "text")?;
        Ok(Self::text(self.take(len)?))
    }
}

#[derive(Clone, Copy, Default)]
struct Channel {
    program: u8,
}

/// A track as the file describes it before the beats are read.
struct TrackInfo {
    strings: usize,
    /// Open pitches, string 1 (the highest) first.
    tuning: Vec<u8>,
    capo: u8,
    drums: bool,
}

/// Read a Guitar Pro 3, 4 or 5 file.
pub(crate) fn parse(bytes: &[u8]) -> Result<GpScore, String> {
    let mut r = Reader { data: bytes, pos: 0 };
    let version_text = r.byte_size_string(30)?;
    let v = match version_text.strip_prefix("FICHIER GUITAR PRO ").and_then(|t| t.get(1..)) {
        Some(n) if n.starts_with("3.") => Version::V3,
        Some(n) if n.starts_with("4.") => Version::V4,
        Some("5.00") => Version::V500,
        Some(n) if n.starts_with("5.") => Version::V510,
        _ => return Err(format!("not a Guitar Pro 3, 4 or 5 file (version \"{version_text}\")")),
    };
    let (title, artist) = song_info(&mut r, v)?;
    let tempo = r.i32()?;
    let channels = settings(&mut r, v)?;
    let measures = r.count(MAX_MEASURES, "measures")?;
    let tracks = r.count(MAX_TRACKS, "tracks")?;
    if measures == 0 || tracks == 0 {
        return Err("the score has no measures or no tracks".into());
    }
    let mut score =
        GpScore { title, artist, tempos: vec![GpTempo { bar: 0, position: 0.0, bpm: f64::from(tempo.clamp(5, 999)) }], ..GpScore::default() };
    for i in 0..measures {
        let prev = score.bars.last().copied();
        score.bars.push(measure_header(&mut r, v, i, prev)?);
    }
    let mut infos = Vec::with_capacity(tracks);
    for i in 0..tracks {
        let (info, track) = track(&mut r, v, i, &channels)?;
        infos.push(info);
        score.tracks.push(track);
    }
    r.skip(match v {
        Version::V500 => 2,
        Version::V510 => 1,
        _ => 0,
    })?;
    for mi in 0..measures {
        let bar_len = score.bars.get(mi).map_or(WHOLE, GpMasterBar::ticks);
        for ti in 0..tracks {
            let info = infos.get(ti).ok_or("track out of range")?;
            let voices = measure(&mut r, v, info, mi, bar_len, &mut score.tempos)?;
            if let Some(t) = score.tracks.get_mut(ti) {
                t.bars.push(voices);
            }
        }
    }
    link_techniques(&mut score);
    score.tempos.sort_by(|a, b| a.bar.cmp(&b.bar).then(a.position.total_cmp(&b.position)));
    Ok(score)
}

/// The title and artist; the rest of the song information, the lyrics and page setup are skipped.
fn song_info(r: &mut Reader, v: Version) -> R<(String, String)> {
    let title = r.int_byte_size_string()?.chars().take(200).collect();
    let _subtitle = r.int_byte_size_string()?;
    let artist = r.int_byte_size_string()?.chars().take(200).collect();
    let fields = if v >= Version::V500 { 6 } else { 5 }; // album, words, (music,) copyright, tab, instructions
    for _ in 0..fields {
        r.int_byte_size_string()?;
    }
    let notice = r.count(1000, "notice lines")?;
    for _ in 0..notice {
        r.int_byte_size_string()?;
    }
    if v <= Version::V4 {
        let _triplet_feel = r.bool()?;
    }
    if v >= Version::V4 {
        // Lyrics: the track they follow, then five lines from a measure each.
        r.i32()?;
        for _ in 0..5 {
            r.i32()?;
            r.int_size_string()?;
        }
    }
    if v == Version::V510 {
        // The master effect: volume, an unknown word, and an 11-band equalizer.
        r.skip(4 + 4 + 11)?;
    }
    if v >= Version::V500 {
        // Page setup: size, margins, score size, header and footer flags and their ten texts.
        r.skip(7 * 4 + 2)?;
        for _ in 0..10 {
            r.int_byte_size_string()?;
        }
        let _tempo_name = r.int_byte_size_string()?;
    }
    Ok((title, artist))
}

/// What follows the tempo: key, the 64 MIDI channels (their programs kept), directions.
fn settings(r: &mut Reader, v: Version) -> R<[Channel; CHANNELS]> {
    if v == Version::V510 {
        let _hide_tempo = r.bool()?;
    }
    if v >= Version::V500 {
        r.i8()?; // key
        r.i32()?; // octave
    } else {
        r.i32()?; // key
        if v == Version::V4 {
            r.i8()?; // octave
        }
    }
    let mut channels = [Channel::default(); CHANNELS];
    for c in &mut channels {
        let program = r.i32()?;
        c.program = u8::try_from(program.clamp(0, 127)).unwrap_or(0);
        r.skip(8)?; // volume, balance, chorus, reverb, phaser, tremolo, two unused bytes
    }
    if v >= Version::V500 {
        r.skip(19 * 2)?; // directions (coda, segno, fine…)
        r.i32()?; // master reverb
    }
    Ok(channels)
}

/// One track's voices of measure `mi`; tempo changes on its beats go into `tempos`.
fn measure(r: &mut Reader, v: Version, info: &TrackInfo, mi: usize, bar_len: i64, tempos: &mut Vec<GpTempo>) -> R<Vec<Vec<GpBeat>>> {
    let voices = if v >= Version::V500 { 2 } else { 1 };
    let mut out = Vec::with_capacity(voices);
    for _ in 0..voices {
        let mut beats = Vec::new();
        let mut used = 0i64;
        let count = r.count(MAX_BEATS, "beats in a measure")?;
        for _ in 0..count {
            let b = beat(r, v, info)?;
            if let Some(bpm) = b.tempo {
                let position = if bar_len > 0 { (used as f64 / bar_len as f64).clamp(0.0, 1.0) } else { 0.0 };
                if !tempos.iter().any(|t| t.bar == mi && (t.position - position).abs() < 1e-9) {
                    tempos.push(GpTempo { bar: mi, position, bpm: f64::from(bpm.clamp(5, 999)) });
                }
            }
            // A voice that runs past its bar still plays, up to a few bars' worth.
            let ticks = b.ticks.min(bar_len.saturating_mul(MAX_OVERRUN).saturating_sub(used));
            if ticks <= 0 {
                continue;
            }
            used += ticks;
            if let Some(g) = b.grace {
                beats.push(g);
            }
            beats.push(GpBeat { ticks, ..b.beat });
        }
        out.push(beats);
    }
    // A line break flag; the last measure's may be left out.
    if v >= Version::V500 && r.pos < r.data.len() {
        r.u8()?;
    }
    Ok(out)
}

fn measure_header(r: &mut Reader, v: Version, index: usize, prev: Option<GpMasterBar>) -> R<GpMasterBar> {
    if v >= Version::V500 && index > 0 {
        r.skip(1)?;
    }
    let flags = r.u8()?;
    let (mut numerator, mut denominator) = prev.map_or((4, 4), |p| (p.numerator, p.denominator));
    if flags & 0x01 != 0 {
        numerator = u32::from(r.u8()?).clamp(1, 64);
    }
    if flags & 0x02 != 0 {
        let d = u32::from(r.u8()?);
        denominator = if [1, 2, 4, 8, 16, 32, 64].contains(&d) { d } else { 4 };
    }
    let repeat_start = flags & 0x04 != 0;
    let mut repeat_end = false;
    let mut repeat_count = 2;
    if flags & 0x08 != 0 {
        repeat_end = true;
        let n = u32::from(r.u8()?);
        // Guitar Pro 5 stores how many times the section plays; 3 and 4 how many times it repeats.
        repeat_count = if v >= Version::V500 { n } else { n.saturating_add(1) }.clamp(1, MAX_REPEAT);
    }
    let mut endings = 0u32;
    if v < Version::V500 && flags & 0x10 != 0 {
        let n = u32::from(r.u8()?);
        if (1..=32).contains(&n) {
            endings = 1 << (n - 1);
        }
    }
    if flags & 0x20 != 0 {
        r.int_byte_size_string()?; // marker name
        r.skip(4)?; // and colour
    }
    if flags & 0x40 != 0 {
        r.skip(2)?; // key signature
    }
    if v >= Version::V500 {
        if flags & 0x10 != 0 {
            endings = u32::from(r.u8()?);
        }
        if flags & 0x03 != 0 {
            r.skip(4)?; // beaming
        }
        if flags & 0x10 == 0 {
            r.skip(1)?;
        }
        r.u8()?; // triplet feel
    }
    Ok(GpMasterBar { numerator, denominator, repeat_start, repeat_end, repeat_count, endings })
}

fn track(r: &mut Reader, v: Version, index: usize, channels: &[Channel; CHANNELS]) -> R<(TrackInfo, GpTrack)> {
    if v >= Version::V500 && (index == 0 || v == Version::V500) {
        r.skip(1)?;
    }
    let flags = r.u8()?;
    let name = r.byte_size_string(40)?;
    let strings = r.count(7, "strings on a track")?.max(1);
    let mut tuning = Vec::with_capacity(7);
    for _ in 0..7 {
        let p = r.i32()?;
        tuning.push(u8::try_from(p.clamp(0, 127)).unwrap_or(0));
    }
    tuning.truncate(strings);
    let _port = r.i32()?;
    let channel_index = usize::try_from(r.i32()?.saturating_sub(1)).unwrap_or(0).min(CHANNELS - 1);
    let _effect_channel = r.i32()?;
    let _frets = r.i32()?;
    let capo = u8::try_from(r.i32()?.clamp(0, i32::from(MAX_FRET))).unwrap_or(0);
    let c = r.take(4)?;
    let color = [c.first().copied().unwrap_or(200), c.get(1).copied().unwrap_or(200), c.get(2).copied().unwrap_or(200)];
    if v >= Version::V500 {
        r.skip(2 + 1 + 1)?; // display flags, auto accentuation, MIDI bank
        r.skip(1 + 3 * 4 + 12)?; // humanize and RSE settings
        r.skip(3 * 4)?; // RSE instrument, its unknown word and sound bank
        if v == Version::V500 {
            r.skip(2 + 1)?; // effect number
        } else {
            r.skip(4)?; // effect number
            r.skip(4)?; // equalizer
            r.int_byte_size_string()?; // effect
            r.int_byte_size_string()?; // effect category
        }
    }
    let midi_channel = u8::try_from(channel_index % 16).unwrap_or(0);
    let drums = flags & 0x01 != 0 || midi_channel == 9;
    let program = channels.get(channel_index).map_or(0, |c| c.program);
    let kind = if drums { GpKind::Drums } else { GpKind::Stringed { tuning: tuning.iter().rev().copied().collect(), capo } };
    let info = TrackInfo { strings, tuning, capo, drums };
    Ok((info, GpTrack { name, color, kind, program, channel: if drums { 9 } else { midi_channel }, bars: Vec::new() }))
}

/// A beat as read: the beat, the grace notes played before it, and a tempo change on it.
struct ReadBeat {
    ticks: i64,
    beat: GpBeat,
    grace: Option<GpBeat>,
    tempo: Option<i32>,
}

/// While reading, marks a note that hammers on or pulls off to the next on its string (the tremolo
/// bit, which only playback sets); `link_techniques` turns it into the origin and destination bits.
const HAMMER: u16 = tech::TREMOLO;

fn duration_ticks(d: i8, dotted: bool, tuplet: i32) -> i64 {
    // -2 whole, -1 half, 0 quarter … 4 a 64th.
    let d = i32::from(d).clamp(-2, 6);
    let mut t = WHOLE / (1i64 << (d + 2).clamp(0, 8));
    if dotted {
        t = t * 3 / 2;
    }
    let (n, m) = match tuplet {
        3 => (3, 2),
        5..=7 => (tuplet, 4),
        9..=13 => (tuplet, 8),
        _ => (1, 1),
    };
    (t * i64::from(m) / i64::from(n)).max(1)
}

fn beat(r: &mut Reader, v: Version, info: &TrackInfo) -> R<ReadBeat> {
    let flags = r.u8()?;
    // A status byte marks an empty beat (0) or a rest (2).
    let status = if flags & 0x40 != 0 { Some(r.u8()?) } else { None };
    let rest = status.is_some();
    let duration = r.i8()?;
    let tuplet = if flags & 0x20 != 0 { r.i32()? } else { 1 };
    let ticks = duration_ticks(duration, flags & 0x01 != 0, tuplet);
    if flags & 0x02 != 0 {
        chord(r, v, info.strings)?;
    }
    if flags & 0x04 != 0 {
        r.int_byte_size_string()?; // text
    }
    let mut beat_tech = 0u16;
    let mut tremolo = 0i64;
    if flags & 0x08 != 0 {
        beat_tech = beat_effects(r, v)?;
    }
    let mut tempo = None;
    if flags & 0x10 != 0 {
        tempo = mix_table(r, v)?;
    }
    let string_flags = r.u8()?;
    let mut notes = Vec::new();
    let mut graces = Vec::new();
    let mut velocity = None;
    for k in 0..7u8 {
        if string_flags & (1 << (6 - k)) == 0 {
            continue;
        }
        let string_no = usize::from(k) + 1; // 1 = the highest
        let n = note(r, v)?;
        if usize::from(k) >= info.strings {
            continue;
        }
        if n.tremolo > 0 {
            tremolo = n.tremolo;
        }
        let open = info.tuning.get(string_no - 1).copied().unwrap_or(40);
        let pitch_of = |fret: u8| {
            if info.drums { fret.min(127) } else { u8::try_from(u16::from(open) + u16::from(info.capo) + u16::from(fret)).unwrap_or(127).min(127) }
        };
        // Strings as the GPIF reader counts them: 0 = the lowest.
        let low_first = u8::try_from(info.strings - string_no).unwrap_or(0);
        let string = (!info.drums).then_some((low_first, n.fret.min(MAX_FRET)));
        velocity = velocity.or(n.velocity);
        if let Some(g) = n.grace {
            let gt = if g.dead { tech::DEAD } else { 0 } | if g.hammer { HAMMER } else { 0 };
            graces.push((
                g.on_beat,
                g.velocity,
                GpNote {
                    pitch: pitch_of(g.fret),
                    string: (!info.drums).then_some((low_first, g.fret.min(MAX_FRET))),
                    tie_in: false,
                    tie_out: false,
                    tech: gt,
                },
            ));
        }
        notes.push(GpNote { pitch: pitch_of(n.fret), string, tie_in: n.tie, tie_out: false, tech: n.tech | beat_tech });
    }
    if v >= Version::V500 {
        let flags2 = r.i16()?;
        if flags2 & 0x0800 != 0 {
            r.u8()?;
        }
    }
    let velocity = velocity.unwrap_or(95);
    let grace = (!graces.is_empty()).then(|| {
        let on = graces.iter().any(|g| g.0);
        let vel = graces.first().map_or(velocity, |g| g.1);
        GpBeat {
            ticks: GRACE_TICKS,
            velocity: vel,
            notes: graces.into_iter().map(|g| g.2).collect(),
            tremolo: 0,
            grace: if on { Grace::On } else { Grace::Before },
        }
    });
    if rest {
        notes.clear();
    }
    // An empty beat (what an unused second voice holds) takes no time.
    let ticks = if status == Some(0) { 0 } else { ticks };
    Ok(ReadBeat { ticks, beat: GpBeat { ticks, velocity, notes, tremolo, grace: Grace::None }, grace, tempo })
}

fn chord(r: &mut Reader, v: Version, strings: usize) -> R<()> {
    let new_format = r.bool()?;
    if !new_format {
        r.int_byte_size_string()?;
        let first_fret = r.i32()?;
        if first_fret != 0 {
            r.skip(6 * 4)?;
        }
        return Ok(());
    }
    let _ = strings;
    match v {
        Version::V3 => r.skip(124),
        _ => r.skip(106),
    }
}

/// Beat-wide effects; returns the techniques they put on every note of the beat.
fn beat_effects(r: &mut Reader, v: Version) -> R<u16> {
    let mut t = 0u16;
    if v == Version::V3 {
        let flags = r.u8()?;
        if flags & 0x03 != 0 {
            t |= tech::VIBRATO;
        }
        if flags & 0x20 != 0 {
            let effect = r.u8()?;
            let _ = effect;
            r.i32()?; // tremolo bar value or tap/slap/pop
        }
        if flags & 0x40 != 0 {
            r.skip(2)?; // strokes
        }
        return Ok(t);
    }
    let f1 = r.u8()?;
    let f2 = r.u8()?;
    if f1 & 0x02 != 0 {
        t |= tech::VIBRATO;
    }
    if f1 & 0x20 != 0 {
        r.i8()?; // tap, slap, pop
    }
    if f2 & 0x04 != 0 {
        bend(r)?; // tremolo bar
    }
    if f1 & 0x40 != 0 {
        r.skip(2)?; // strokes up and down
    }
    if f2 & 0x02 != 0 {
        r.i8()?; // pick stroke
    }
    Ok(t)
}

fn bend(r: &mut Reader) -> R<()> {
    r.i8()?; // type
    r.i32()?; // value
    let points = r.count(MAX_BEND_POINTS, "bend points")?;
    r.skip(points * 9)
}

/// A mix table change; returns the tempo it sets, if it sets one.
fn mix_table(r: &mut Reader, v: Version) -> R<Option<i32>> {
    r.i8()?; // instrument
    if v >= Version::V500 {
        r.skip(4 * 4)?; // RSE instrument, unknown, sound bank, effect number
    }
    let mut changes = [0i8; 6];
    for c in &mut changes {
        *c = r.i8()?; // volume, balance, chorus, reverb, phaser, tremolo
    }
    if v >= Version::V500 {
        r.int_byte_size_string()?; // tempo name
    }
    let tempo = r.i32()?;
    for c in changes {
        if c >= 0 {
            r.u8()?; // how many beats the change takes
        }
    }
    if tempo >= 0 {
        r.u8()?;
        if v == Version::V510 {
            r.bool()?; // hide tempo
        }
    }
    if v >= Version::V4 {
        r.u8()?; // which changes apply to every track
    }
    if v >= Version::V500 {
        r.i8()?; // wah
    }
    if v == Version::V510 {
        r.int_byte_size_string()?; // RSE effect
        r.int_byte_size_string()?; // and its category
    }
    Ok((tempo > 0).then_some(tempo))
}

struct ReadGrace {
    fret: u8,
    velocity: u8,
    dead: bool,
    on_beat: bool,
    hammer: bool,
}

struct ReadNote {
    fret: u8,
    tie: bool,
    velocity: Option<u8>,
    tech: u16,
    tremolo: i64,
    grace: Option<ReadGrace>,
}

/// Velocity from a dynamic 1 (ppp) … 8 (fff).
fn dynamic_velocity(d: i8) -> u8 {
    let d = i16::from(d).clamp(1, 8);
    u8::try_from(16 * d - 1).unwrap_or(95)
}

fn note(r: &mut Reader, v: Version) -> R<ReadNote> {
    let flags = r.u8()?;
    let mut n = ReadNote { fret: 0, tie: false, velocity: None, tech: 0, tremolo: 0, grace: None };
    if flags & 0x02 != 0 || flags & 0x40 != 0 {
        n.tech |= tech::ACCENT;
    }
    if flags & 0x20 != 0 {
        match r.u8()? {
            2 => n.tie = true,
            3 => n.tech |= tech::DEAD,
            _ => {}
        }
    }
    if v < Version::V500 && flags & 0x01 != 0 {
        r.skip(2)?; // a duration of its own, and its tuplet
    }
    if flags & 0x10 != 0 {
        n.velocity = Some(dynamic_velocity(r.i8()?));
    }
    if flags & 0x20 != 0 {
        n.fret = u8::try_from(r.i8()?.max(0)).unwrap_or(0);
    }
    if flags & 0x80 != 0 {
        r.skip(2)?; // fingering
    }
    if v >= Version::V500 {
        if flags & 0x01 != 0 {
            r.skip(8)?; // duration percent
        }
        r.u8()?; // accidental spelling
    }
    if flags & 0x08 != 0 {
        note_effects(r, v, &mut n)?;
    }
    Ok(n)
}

fn note_effects(r: &mut Reader, v: Version, n: &mut ReadNote) -> R<()> {
    let (f1, f2) = if v == Version::V3 { (r.u8()?, 0u8) } else { (r.u8()?, r.u8()?) };
    if f1 & 0x01 != 0 {
        bend(r)?;
    }
    if f1 & 0x02 != 0 {
        n.tech |= HAMMER;
    }
    if f1 & 0x08 != 0 {
        n.tech |= tech::LET_RING;
    }
    if v == Version::V3 && f1 & 0x04 != 0 {
        n.tech |= tech::SLIDE_SHIFT;
    }
    if f1 & 0x10 != 0 {
        let fret = u8::try_from(r.i8()?.max(0)).unwrap_or(0);
        let velocity = dynamic_velocity(i8::try_from(r.u8()?).unwrap_or(6));
        let (transition, flags) = if v >= Version::V500 {
            let t = r.u8()?;
            r.u8()?; // duration
            (t, r.u8()?)
        } else {
            r.u8()?; // duration
            (r.u8()?, 0)
        };
        n.grace = Some(ReadGrace { fret, velocity, dead: flags & 0x01 != 0, on_beat: flags & 0x02 != 0, hammer: transition == 3 });
    }
    if v == Version::V3 {
        return Ok(());
    }
    if f2 & 0x01 != 0 {
        n.tech |= tech::STACCATO;
    }
    if f2 & 0x02 != 0 {
        n.tech |= tech::PALM_MUTE;
    }
    if f2 & 0x04 != 0 {
        n.tremolo = match r.u8()? {
            1 => 480,
            2 => 240,
            3 => 120,
            _ => 0,
        };
    }
    if f2 & 0x08 != 0 {
        if v >= Version::V500 {
            let s = r.u8()?;
            for (flag, bit) in [
                (0x01, tech::SLIDE_SHIFT),
                (0x02, tech::SLIDE_LEGATO),
                (0x04, tech::SLIDE_OUT_DOWN),
                (0x08, tech::SLIDE_OUT_UP),
                (0x10, tech::SLIDE_IN_BELOW),
                (0x20, tech::SLIDE_IN_ABOVE),
            ] {
                if s & flag != 0 {
                    n.tech |= bit;
                }
            }
        } else {
            n.tech |= match r.i8()? {
                -2 => tech::SLIDE_IN_ABOVE,
                -1 => tech::SLIDE_IN_BELOW,
                1 => tech::SLIDE_SHIFT,
                2 => tech::SLIDE_LEGATO,
                3 => tech::SLIDE_OUT_DOWN,
                4 => tech::SLIDE_OUT_UP,
                _ => 0,
            };
        }
    }
    if f2 & 0x10 != 0 {
        let kind = r.u8()?;
        if v >= Version::V500 {
            match kind {
                2 => r.skip(3)?, // artificial: the note, its accidental and octave
                3 => r.skip(1)?, // tapped: the fret
                _ => {}
            }
        }
    }
    if f2 & 0x20 != 0 {
        r.skip(2)?; // trill: fret and period
    }
    if f2 & 0x40 != 0 {
        n.tech |= tech::VIBRATO;
    }
    Ok(())
}

/// Hammer-ons and pull-offs lead to the next note on the same string; a tie continues the note
/// before it on that string, so it takes that note's pitch, and that note carries on.
fn link_techniques(score: &mut GpScore) {
    for t in &mut score.tracks {
        let voices = t.bars.iter().map(Vec::len).max().unwrap_or(0);
        for vi in 0..voices {
            // Every note of the voice in order, as (bar, beat, note).
            let mut at = Vec::new();
            for (bi, bar) in t.bars.iter().enumerate() {
                for (ki, beat) in bar.get(vi).map_or(&[][..], Vec::as_slice).iter().enumerate() {
                    at.extend((0..beat.notes.len()).map(|ni| (bi, ki, ni)));
                }
            }
            let mut last: [Option<(usize, usize, usize)>; 8] = [None; 8];
            for (bi, ki, ni) in at {
                let get = |t: &GpTrack, (b, k, n): (usize, usize, usize)| t.bars.get(b)?.get(vi)?.get(k)?.notes.get(n).cloned();
                let Some(note) = get(t, (bi, ki, ni)) else { continue };
                let Some((string, _)) = note.string else { continue };
                let Some(slot) = last.get_mut(usize::from(string)) else { continue };
                let prev = slot.replace((bi, ki, ni));
                let Some(p) = prev.and_then(|p| get(t, p).map(|n| (p, n))) else { continue };
                let ((pb, pk, pn), pnote) = p;
                let Some(cur) = t.bars.get_mut(bi).and_then(|x| x.get_mut(vi)).and_then(|x| x.get_mut(ki)).and_then(|x| x.notes.get_mut(ni)) else {
                    continue;
                };
                if note.tie_in {
                    cur.pitch = pnote.pitch;
                    cur.string = pnote.string;
                } else if pnote.tech & HAMMER != 0 {
                    cur.tech |= tech::HOPO_DEST;
                }
                if note.tie_in
                    && let Some(pm) = t.bars.get_mut(pb).and_then(|x| x.get_mut(vi)).and_then(|x| x.get_mut(pk)).and_then(|x| x.notes.get_mut(pn))
                {
                    pm.tie_out = true;
                }
            }
        }
        // The hammer marker becomes the origin bit Solara plays.
        for n in t.bars.iter_mut().flatten().flatten().flat_map(|b| b.notes.iter_mut()) {
            if n.tech & HAMMER != 0 {
                n.tech = (n.tech & !HAMMER) | tech::HOPO;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a Guitar Pro 5.10 file the way the format lays one out.
    #[derive(Default)]
    struct File(Vec<u8>);
    impl File {
        fn u8(&mut self, b: u8) -> &mut Self {
            self.0.push(b);
            self
        }
        fn i32(&mut self, v: i32) -> &mut Self {
            self.0.extend(v.to_le_bytes());
            self
        }
        fn zeros(&mut self, n: usize) -> &mut Self {
            self.0.extend(std::iter::repeat_n(0, n));
            self
        }
        fn ibs(&mut self, s: &str) -> &mut Self {
            self.i32(s.len() as i32 + 1).u8(s.len() as u8);
            self.0.extend(s.as_bytes());
            self
        }
        fn bss(&mut self, s: &str, size: usize) -> &mut Self {
            self.u8(s.len() as u8);
            self.0.extend(s.as_bytes());
            self.zeros(size - s.len())
        }
    }

    /// A note on `string` (1 = highest) at `fret`: flags, then what they announce.
    struct N {
        string: u8,
        fret: u8,
        kind: u8,
        effects: Option<(u8, u8, Vec<u8>)>,
    }

    fn n(string: u8, fret: u8) -> N {
        N { string, fret, kind: 1, effects: None }
    }

    fn beat(f: &mut File, duration: i8, notes: &[N], tempo: Option<i32>) {
        f.u8(if tempo.is_some() { 0x10 } else { 0 }).u8(duration as u8);
        if let Some(t) = tempo {
            f.u8(0xFF).zeros(16);
            for _ in 0..6 {
                f.u8(0xFF);
            }
            f.ibs("").i32(t).u8(0).u8(0).u8(0).u8(0xFF).ibs("").ibs("");
        }
        f.u8(notes.iter().fold(0, |m, x| m | 1 << (7 - x.string)));
        let mut sorted: Vec<&N> = notes.iter().collect();
        sorted.sort_by_key(|x| x.string);
        for x in sorted {
            f.u8(0x20 | if x.effects.is_some() { 0x08 } else { 0 }).u8(x.kind).u8(x.fret).u8(0);
            if let Some((f1, f2, extra)) = &x.effects {
                f.u8(*f1).u8(*f2);
                f.0.extend(extra);
            }
        }
        f.u8(0).u8(0);
    }

    /// Two 4/4 measures of one guitar track at 100 bpm (a tempo change to 150 in measure 2).
    fn song() -> Vec<u8> {
        let mut f = File::default();
        f.bss("FICHIER GUITAR PRO v5.10", 30);
        f.ibs("Garden Path").ibs("").ibs("Zee Test");
        for _ in 0..6 {
            f.ibs("");
        }
        f.i32(0); // notice lines
        f.i32(0);
        for _ in 0..5 {
            f.i32(0).i32(0);
        }
        f.zeros(19).zeros(30);
        for _ in 0..10 {
            f.ibs("");
        }
        f.ibs("Moderate").i32(100).u8(0).u8(0).i32(0);
        for c in 0..64 {
            f.i32(if c == 0 { 29 } else { 25 }).zeros(8);
        }
        f.zeros(38).i32(0).i32(2).i32(1);
        // Measure 1: 4/4 with its repeat closing (played twice in all); measure 2 plain.
        f.u8(0x01 | 0x02 | 0x04 | 0x08).u8(4).u8(4).u8(2).zeros(4).u8(0).u8(0);
        f.u8(0).u8(0).u8(0).u8(0);
        // The track.
        f.u8(0).u8(0).bss("Lead", 40).i32(6);
        for p in [64, 59, 55, 50, 45, 40, 0] {
            f.i32(p);
        }
        f.i32(1).i32(1).i32(2).i32(24).i32(2).u8(10).u8(20).u8(30).u8(0);
        f.zeros(4).zeros(1 + 12 + 12).zeros(16).zeros(4).ibs("").ibs("");
        f.u8(0);
        // Measure 1: a hammer-on 5→7 on string 3, a palm-muted dead note, a legato slide.
        f.i32(4);
        beat(&mut f, 0, &[N { effects: Some((0x02, 0, vec![])), ..n(3, 5) }], None);
        beat(&mut f, 0, &[n(3, 7)], None);
        beat(&mut f, 0, &[N { kind: 3, effects: Some((0, 0x02, vec![])), ..n(6, 0) }], None);
        beat(&mut f, 0, &[N { effects: Some((0, 0x08, vec![0x02])), ..n(1, 3) }], None);
        f.i32(1).u8(0x40).u8(0).u8(0).u8(0).u8(0).u8(0); // an empty second voice
        f.u8(0);
        // Measure 2: a grace note before a half note tied into the next, then a tempo change.
        f.i32(3);
        beat(&mut f, -1, &[N { effects: Some((0x10, 0, vec![3, 6, 0, 3, 0])), ..n(2, 3) }], None);
        beat(&mut f, 0, &[N { kind: 2, ..n(2, 0) }], Some(150));
        beat(&mut f, 0, &[n(2, 1)], None);
        f.i32(0);
        f.0
    }

    #[test]
    fn a_guitar_pro_5_song_reads_with_its_techniques() {
        let s = parse(&song()).unwrap();
        assert_eq!((s.title.as_str(), s.artist.as_str()), ("Garden Path", "Zee Test"));
        assert_eq!(s.bars.len(), 2);
        assert!(s.bars[0].repeat_start && s.bars[0].repeat_end && s.bars[0].repeat_count == 2);
        assert_eq!(s.tempos.iter().map(|t| (t.bar, t.position, t.bpm)).collect::<Vec<_>>(), [(0, 0.0, 100.0), (1, 0.5, 150.0)]);
        let t = &s.tracks[0];
        assert_eq!((t.name.as_str(), t.program, t.color, t.channel), ("Lead", 29, [10, 20, 30], 0));
        assert_eq!(t.kind, GpKind::Stringed { tuning: vec![40, 45, 50, 55, 59, 64], capo: 2 });
        let m1 = &t.bars[0][0];
        assert_eq!(m1.iter().map(|b| b.ticks).collect::<Vec<_>>(), [960; 4]);
        assert!(t.bars[0][1].is_empty(), "the empty voice takes no time");
        // Pitches count the capo; strings count from the lowest (0).
        let first = &m1[0].notes[0];
        assert_eq!((first.pitch, first.string, first.tech), (55 + 2 + 5, Some((3, 5)), tech::HOPO));
        assert_eq!(m1[1].notes[0].tech, tech::HOPO_DEST);
        assert_eq!(m1[2].notes[0].tech, tech::DEAD | tech::PALM_MUTE);
        assert_eq!(m1[3].notes[0].tech, tech::SLIDE_LEGATO);
        let m2 = &t.bars[1][0];
        assert_eq!(m2[0].grace, Grace::Before);
        assert_eq!((m2[0].notes[0].pitch, m2[0].velocity), (59 + 2 + 3, 95), "grace fret 3 at forte");
        assert_eq!(m2[1].ticks, 1920);
        assert!(m2[1].notes[0].tie_out && m2[2].notes[0].tie_in);
        assert_eq!(m2[2].notes[0].pitch, m2[1].notes[0].pitch, "a tie takes the pitch it continues");
    }

    #[test]
    fn a_guitar_pro_5_song_opens_as_a_session() {
        let mut e = crate::Engine::new(soundcraft_model::Session::default());
        let r = super::super::open_guitar_pro_bytes(&mut e, "garden.gp5", &song()).unwrap();
        assert_eq!(r["name"], "Garden Path");
        assert_eq!(r["bars"], 3, "the repeat plays measure 1 twice");
        assert_eq!(r["tracks"][0]["kind"], "tab");
        assert_eq!(e.session().tracks.len(), 1);
    }

    #[test]
    fn damaged_guitar_pro_5_files_are_refused_not_a_crash() {
        let good = song();
        for cut in 0..good.len() {
            assert!(parse(&good[..cut]).is_err() || cut + 8 > good.len(), "cut at {cut}");
        }
        // Every byte flipped in turn: an error or a score, never a panic.
        for i in 0..good.len() {
            let mut bad = good.clone();
            bad[i] ^= 0xFF;
            let _ = parse(&bad);
        }
        assert!(parse(b"FICHIER GUITAR PRO v9.99").is_err());
        assert!(!sniff(b"PK\x03\x04"));
    }
}
