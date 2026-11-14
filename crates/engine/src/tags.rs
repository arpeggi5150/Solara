//! The smart tag scan: tags suggested for a track from the words of its name, from the audio on
//! it, and from what the user fixed on earlier scans ([`soundcraft_model::Tagging`]).

use soundcraft_model::{ClipContent, Session, Tagging, Track, TrackId, TrackKind, clean_tags};
use soundcraft_time::Range;

pub const VOCALS: &str = "Vocals";
pub const MAINS: &str = "Mains";
pub const DUBS: &str = "Dubs";
pub const LEFT: &str = "L";
pub const RIGHT: &str = "R";
pub const DRUMS: &str = "Drums";
pub const DI: &str = "DI";
pub const MIC: &str = "Mic";
pub const GUITAR: &str = "Guitar";
pub const BASS: &str = "Bass";
pub const KEYS: &str = "Keys";

/// The tags the scan knows, in the order it lists them on a track.
pub const KNOWN: [&str; 11] = [VOCALS, MAINS, DUBS, LEFT, RIGHT, DRUMS, GUITAR, BASS, KEYS, DI, MIC];

/// The tags a word of a track name stands for.
fn word_tags(word: &str) -> &'static [&'static str] {
    match word {
        "vox" | "vocal" | "vocals" | "voc" | "vocs" | "vx" | "vo" | "voice" | "bgv" | "bgvs" | "bv" | "bvs" | "harmony" | "harmonies" | "harm"
        | "choir" | "adlib" | "adlibs" | "singer" | "sing" | "lv" => &[VOCALS],
        "main" | "mains" => &[MAINS],
        "dub" | "dubs" | "dbl" | "dbls" | "double" | "doubles" | "doubled" => &[DUBS],
        "l" | "left" | "lt" => &[LEFT],
        "r" | "right" | "rt" => &[RIGHT],
        "drum" | "drums" | "kit" | "kick" | "kik" | "kck" | "bd" | "snare" | "snr" | "sn" | "sd" | "tom" | "toms" | "ft" | "floor" | "hat"
        | "hats" | "hh" | "hihat" | "hihats" | "oh" | "ohs" | "overhead" | "overheads" | "room" | "rooms" | "ride" | "crash" | "cymbal"
        | "cymbals" | "perc" | "percussion" | "clap" | "claps" | "shaker" | "tamb" | "trash" | "808" => &[DRUMS],
        "di" | "direct" => &[DI],
        "mic" | "mics" | "amp" | "cab" | "sm57" | "57" | "md421" | "421" | "r121" | "121" | "ribbon" | "condenser" | "u87" | "c414" | "414" => &[MIC],
        "gtr" | "gtrs" | "guitar" | "guitars" | "gtar" | "egtr" | "agtr" | "acoustic" | "rhythm" | "solo" => &[GUITAR],
        "bass" | "bs" | "sub" => &[BASS],
        "keys" | "key" | "piano" | "pno" | "synth" | "synths" | "organ" | "rhodes" | "wurli" | "pad" | "pads" | "keyboard" | "keyboards" => &[KEYS],
        _ => &[],
    }
}

/// The words of a track name, lower case: split at anything but a letter or digit, between a
/// lower- and an upper-case letter ("LeadVox") and between letters and digits ("Vox2"). At most
/// 32 words.
pub fn words(name: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    let chars: Vec<char> = name.chars().take(256).collect();
    for (i, &c) in chars.iter().enumerate() {
        let prev = i.checked_sub(1).and_then(|p| chars.get(p)).copied().filter(|p| p.is_alphanumeric());
        let next = chars.get(i + 1).copied();
        // camelCase splits before an upper-case letter that starts a lower-case word ("LeadVox"),
        // not inside odd capitals ("hI-hAT").
        let boundary = match prev {
            _ if !c.is_alphanumeric() => true,
            Some(p) => (p.is_lowercase() && c.is_uppercase() && next.is_some_and(char::is_lowercase)) || (p.is_alphabetic() != c.is_alphabetic()),
            None => false,
        };
        if boundary && !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
        }
        if c.is_alphanumeric() {
            cur.extend(c.to_lowercase());
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    // Model numbers are one word: "SM57", "R121".
    let mut joined: Vec<String> = Vec::new();
    for w in out {
        match joined.last_mut() {
            Some(last) if w.chars().all(|c| c.is_ascii_digit()) && !word_tags(&format!("{last}{w}")).is_empty() => last.push_str(&w),
            _ => joined.push(w),
        }
    }
    joined.truncate(32);
    joined
}

/// The key a learned fix is kept under: the name's words without numbers ("Vox Dub L 2" and
/// "Vox Dub L" are the same track in two sessions).
pub fn name_key(name: &str) -> String {
    words(name).into_iter().filter(|w| !w.chars().all(|c| c.is_ascii_digit())).collect::<Vec<_>>().join(" ")
}

/// What the audio on a track sounds like, from up to [`LISTEN_SECONDS`] around its middle.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
pub struct Listening {
    /// Seconds of the audio that are not silence.
    pub active_seconds: f32,
    /// Average level of the parts that are not silence, dBFS.
    pub rms_db: f32,
    /// Peak over average, dB: high for undistorted, unsquashed signals.
    pub crest_db: f32,
    /// Attacks per second of sound.
    pub onsets_per_second: f32,
    /// Share of frames with a clear pitch.
    pub pitched: f32,
    /// Middle pitch of the pitched frames, Hz (0 for none).
    pub pitch_hz: f32,
    /// Share of the energy above 5 kHz.
    pub highs: f32,
}

/// How much audio the scan listens to on each track.
pub const LISTEN_SECONDS: f64 = 30.0;

/// Listen to the audio on `t` (audio tracks only). `None` when there is under a second of sound.
pub fn listen(s: &Session, t: &Track) -> Option<Listening> {
    if t.kind != TrackKind::Audio {
        return None;
    }
    let clips = t.clips().iter().filter(|c| matches!(c.content, ClipContent::Audio { .. }));
    let (start, end) = clips.fold((i64::MAX, i64::MIN), |(a, b), c| (a.min(c.start), b.max(c.end())));
    if start >= end {
        return None;
    }
    let sr = s.sample_rate.as_f64();
    let span = s.sample_rate.samples(LISTEN_SECONDS);
    let mid = start.saturating_add((end - start) / 2);
    let from = mid.saturating_sub(span / 2).max(start);
    let range = Range::new(from, from.saturating_add(span).min(end));
    let audio = soundcraft_mix::render_clips(s, t.id, range);
    analyse(&audio, sr as f32)
}

/// [`listen`] on rendered channels.
pub fn analyse(channels: &[Vec<f32>], sample_rate: f32) -> Option<Listening> {
    let len = channels.iter().map(Vec::len).max()?;
    if len == 0 || !(8000.0..=768_000.0).contains(&sample_rate) {
        return None;
    }
    let n = channels.len().max(1) as f32;
    let mono: Vec<f32> = (0..len)
        .map(|i| channels.iter().map(|c| c.get(i).copied().unwrap_or(0.0)).sum::<f32>() / n)
        .map(|x| if x.is_finite() { x } else { 0.0 })
        .collect();
    let frame = (sample_rate * 0.05) as usize;
    let silence = 10f32.powf(-50.0 / 20.0);
    let mut active = Vec::new();
    for (i, f) in mono.chunks(frame.max(1)).enumerate() {
        let rms = (f.iter().map(|x| x * x).sum::<f32>() / f.len().max(1) as f32).sqrt();
        if rms > silence {
            active.push(i);
        }
    }
    let active_seconds = active.len() as f32 * 0.05;
    if active_seconds < 1.0 {
        return None;
    }
    let (mut energy, mut peak, mut count) = (0f64, 0f32, 0usize);
    for &i in &active {
        for x in mono.iter().skip(i * frame).take(frame) {
            energy += f64::from(x * x);
            peak = peak.max(x.abs());
            count += 1;
        }
    }
    let rms = (energy / count.max(1) as f64).sqrt() as f32;
    let db = |v: f32| 20.0 * v.max(1e-9).log10();
    let onsets = soundcraft_dsp::offline::detect_transients(std::slice::from_ref(&mono), sample_rate, 0.5).len();
    // Pitch on a 2048-sample window every quarter second of sound.
    let window = 2048usize;
    let mut pitches = Vec::new();
    let mut tried = 0usize;
    for &i in active.iter().step_by(5) {
        let at = i * frame;
        let Some(w) = mono.get(at..at.saturating_add(window)) else { continue };
        tried += 1;
        if let Some(f) = soundcraft_dsp::pitch_detect::yin(w, sample_rate, 40.0, 1500.0, 0.15) {
            pitches.push(f);
        }
    }
    let pitched = pitches.len() as f32 / tried.max(1) as f32;
    pitches.sort_by(f32::total_cmp);
    let pitch_hz = pitches.get(pitches.len() / 2).copied().unwrap_or(0.0);
    let mut hp = soundcraft_dsp::biquad::Biquad::design(soundcraft_dsp::biquad::FilterType::HighPass, 5000.0, 0.707, 0.0, sample_rate);
    let highs: f64 = mono.iter().map(|x| f64::from(hp.process_sample(*x)).powi(2)).sum();
    let total: f64 = mono.iter().map(|x| f64::from(*x).powi(2)).sum();
    Some(Listening {
        active_seconds,
        rms_db: db(rms),
        crest_db: db(peak) - db(rms),
        onsets_per_second: onsets as f32 / active_seconds,
        pitched,
        pitch_hz,
        highs: if total > 0.0 { (highs / total) as f32 } else { 0.0 },
    })
}

/// A suggestion for one track.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Suggestion {
    pub track: TrackId,
    pub name: String,
    /// The tags the track has now.
    pub current: Vec<String>,
    /// The tags the scan suggests.
    pub tags: Vec<String>,
    /// Why, one line per tag or fix.
    pub reasons: Vec<String>,
    /// The audio, when the scan listened to it.
    pub listening: Option<Listening>,
}

/// Tracks the scan and grouping work on: the ones that hold a performance.
pub fn taggable(t: &Track) -> bool {
    matches!(t.kind, TrackKind::Audio | TrackKind::Instrument | TrackKind::Midi | TrackKind::Tab)
}

fn add(tags: &mut Vec<String>, reasons: &mut Vec<String>, tag: &str, why: String) {
    if !tags.iter().any(|t| t.eq_ignore_ascii_case(tag)) {
        tags.push(tag.to_string());
        reasons.push(why);
    }
}

fn has(tags: &[String], tag: &str) -> bool {
    tags.iter().any(|t| t.eq_ignore_ascii_case(tag))
}

/// What the scan suggests for one track, before it looks at the other tracks.
fn suggest_one(s: &Session, t: &Track, learned: &Tagging, listen_to_audio: bool) -> Suggestion {
    let (mut tags, mut reasons) = (Vec::new(), Vec::new());
    let ws = words(&t.name);
    let mut i = 0;
    while let Some(w) = ws.get(i) {
        let next = ws.get(i + 1).map(String::as_str);
        // Two-word names: "bass drum" is a kick, "d i" is DI.
        if w == "bass" && matches!(next, Some("drum" | "drums")) {
            add(&mut tags, &mut reasons, DRUMS, "\"bass drum\" in the name".into());
            i += 2;
            continue;
        }
        if w == "d" && next == Some("i") {
            add(&mut tags, &mut reasons, DI, "\"D.I.\" in the name".into());
            i += 2;
            continue;
        }
        for tag in word_tags(w) {
            add(&mut tags, &mut reasons, tag, format!("\"{w}\" in the name"));
        }
        if let Some(extra) = learned.words.get(w) {
            for tag in extra {
                add(&mut tags, &mut reasons, tag, format!("\"{w}\": you tagged it {tag} before"));
            }
        }
        i += 1;
    }
    // "Lead" is the main vocal, but a lead guitar is just a guitar.
    if ws.iter().any(|w| w == "lead" || w == "ld") && has(&tags, VOCALS) {
        add(&mut tags, &mut reasons, MAINS, "\"lead\" vocal".into());
    }
    if t.kind == TrackKind::Tab {
        let strings = t.tab_setup().map_or(0, |s| s.tuning.strings.len());
        let tag = if strings <= 5 && strings > 0 { BASS } else { GUITAR };
        add(&mut tags, &mut reasons, tag, format!("a tab track with {strings} strings"));
    }
    let listening = if listen_to_audio { listen(s, t) } else { None };
    if let Some(l) = listening {
        let named_drums = has(&tags, DRUMS);
        let named_voice = has(&tags, VOCALS);
        if !named_drums && !named_voice && l.onsets_per_second >= 2.0 && l.pitched < 0.35 && l.crest_db >= 14.0 {
            add(&mut tags, &mut reasons, DRUMS, format!("the audio is mostly unpitched hits ({:.1} a second)", l.onsets_per_second));
        }
        let drums = has(&tags, DRUMS);
        // An undistorted pickup signal: clear pitch, wide peaks, little above 5 kHz.
        if !drums && !named_voice && !has(&tags, MIC) && !has(&tags, DI) && l.pitched >= 0.4 && l.crest_db >= 15.0 && l.highs < 0.02 {
            add(&mut tags, &mut reasons, DI, format!("the audio sounds like a direct signal ({:.0} dB peaks over average, few highs)", l.crest_db));
        }
        if !drums && !named_voice && !has(&tags, GUITAR) && !has(&tags, BASS) && (has(&tags, DI) || has(&tags, MIC)) && l.pitched >= 0.4 {
            if l.pitch_hz > 0.0 && l.pitch_hz < 100.0 {
                add(&mut tags, &mut reasons, BASS, format!("the pitch sits low (about {:.0} Hz)", l.pitch_hz));
            } else if l.pitch_hz >= 100.0 {
                add(&mut tags, &mut reasons, GUITAR, format!("the pitch sits in guitar range (about {:.0} Hz)", l.pitch_hz));
            }
        }
    }
    // A learned fix for this name wins.
    let key = name_key(&t.name);
    if let Some(fixed) = learned.names.get(&key).filter(|_| !key.is_empty()) {
        tags = fixed.clone();
        reasons.push(format!("you tagged a track named \"{key}\" this way before"));
    }
    tags = order(clean_tags(tags.iter().map(String::as_str)));
    Suggestion { track: t.id, name: t.name.clone(), current: t.tags.clone(), tags, reasons, listening }
}

/// Known tags first, in [`KNOWN`] order, then the user's own.
fn order(mut tags: Vec<String>) -> Vec<String> {
    tags.sort_by_key(|t| KNOWN.iter().position(|k| k.eq_ignore_ascii_case(t)).unwrap_or(KNOWN.len()));
    tags
}

/// Suggestions for `tracks` (every taggable track when empty). `listen_to_audio` false skips the
/// audio (names and learned fixes only).
pub fn scan(s: &Session, tracks: &[TrackId], listen_to_audio: bool) -> Vec<Suggestion> {
    let mut out: Vec<Suggestion> = s
        .tracks
        .iter()
        .filter(|t| taggable(t) && (tracks.is_empty() || tracks.contains(&t.id)))
        .map(|t| suggest_one(s, t, &s.tagging, listen_to_audio))
        .collect();
    // With dubs or doubles around, a plain vocal is the main one.
    let dubbed = out.iter().any(|g| has(&g.tags, VOCALS) && (has(&g.tags, DUBS) || has(&g.tags, LEFT) || has(&g.tags, RIGHT)));
    if dubbed {
        for g in &mut out {
            let learned = s.tagging.names.contains_key(&name_key(&g.name));
            if !learned && has(&g.tags, VOCALS) && !has(&g.tags, DUBS) && !has(&g.tags, LEFT) && !has(&g.tags, RIGHT) && !has(&g.tags, MAINS) {
                g.tags.push(MAINS.into());
                g.tags = order(std::mem::take(&mut g.tags));
                g.reasons.push("the one vocal without dub or side among dubbed vocals".into());
            }
        }
    }
    out
}

/// Learn from the user confirming `confirmed` where the scan suggested `suggested` for a track
/// named `name`: the name keeps these tags, and a word the scan did not know takes what the user
/// added. Nothing is learned when the user took the suggestion as it was.
pub fn learn(tagging: &mut Tagging, name: &str, suggested: &[String], confirmed: &[String]) {
    let same = suggested.len() == confirmed.len() && suggested.iter().all(|t| has(confirmed, t));
    if same {
        return;
    }
    let key = name_key(name);
    if key.is_empty() {
        return;
    }
    let full = tagging.names.len() >= soundcraft_model::MAX_LEARNED;
    if !full || tagging.names.contains_key(&key) {
        tagging.names.insert(key.clone(), confirmed.to_vec());
    }
    // A word stands for what a track is (a singer's name is a vocal), not its part in a set: mains,
    // dubs and sides are learned for the whole name only.
    let part = |t: &String| [MAINS, DUBS, LEFT, RIGHT].iter().any(|p| p.eq_ignore_ascii_case(t));
    let added: Vec<&String> = confirmed.iter().filter(|t| !has(suggested, t) && !part(t)).collect();
    let removed: Vec<&String> = suggested.iter().filter(|t| !has(confirmed, t)).collect();
    for w in words(name) {
        if w.len() < 2 || w.chars().all(|c| c.is_ascii_digit()) || !word_tags(&w).is_empty() {
            continue;
        }
        let entry_full = tagging.words.len() >= soundcraft_model::MAX_LEARNED && !tagging.words.contains_key(&w);
        if entry_full {
            continue;
        }
        let entry = tagging.words.entry(w).or_default();
        entry.retain(|t| !removed.iter().any(|r| r.eq_ignore_ascii_case(t)));
        for a in &added {
            if !has(entry, a) {
                entry.push((*a).clone());
            }
        }
        *entry = clean_tags(entry.iter().map(String::as_str));
    }
    tagging.words.retain(|_, v| !v.is_empty());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_split_into_words() {
        assert_eq!(words("Vox Dub L"), ["vox", "dub", "l"]);
        assert_eq!(words("LeadVox_02"), ["lead", "vox", "02"]);
        assert_eq!(words("Gtr SM57 (amp)"), ["gtr", "sm57", "amp"]);
        assert_eq!(words("OH-R.wav"), ["oh", "r", "wav"]);
        assert_eq!(words("DRUMS hI-hAT"), ["drums", "hi", "hat"]);
        assert_eq!(words("RIGHT- AARBEAR"), ["right", "aarbear"]);
        assert_eq!(name_key("Vox Dub L 2"), "vox dub l");
    }

    #[test]
    fn hostile_audio_is_not_analysed() {
        assert_eq!(analyse(&[], 48_000.0), None);
        assert_eq!(analyse(&[vec![f32::NAN; 48_000 * 2]], 48_000.0), None);
        assert_eq!(analyse(&[vec![0.5; 100]], f32::NAN), None);
    }

    #[test]
    fn a_plucked_tone_sounds_direct_and_noise_bursts_sound_like_drums() {
        let sr = 48_000.0f32;
        // A decaying 110 Hz pluck every half second.
        let pluck: Vec<f32> = (0..(sr as usize * 4))
            .map(|i| {
                let t = (i % (sr as usize / 2)) as f32 / sr;
                (2.0 * std::f32::consts::PI * 110.0 * i as f32 / sr).sin() * 0.8 * (-t * 3.0).exp()
            })
            .collect();
        let l = analyse(&[pluck], sr).unwrap();
        assert!(l.pitched > 0.8 && (l.pitch_hz - 110.0).abs() < 3.0, "{l:?}");
        assert!(l.highs < 0.01, "{l:?}");
        // Short noise hits, four a second.
        let mut seed = 1u32;
        let hits: Vec<f32> = (0..(sr as usize * 4))
            .map(|i| {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let noise = (seed >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0;
                let t = (i % (sr as usize / 4)) as f32 / sr;
                noise * (-t * 40.0).exp()
            })
            .collect();
        let l = analyse(&[hits], sr).unwrap();
        assert!(l.onsets_per_second >= 2.0 && l.pitched < 0.35 && l.crest_db >= 14.0, "{l:?}");
    }
}
