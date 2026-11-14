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

/// The key a learned fix is kept under: the name's words, numbers included ("Vocal Rec 1" and
/// "Vocal Rec 2" are different parts).
pub fn name_key(name: &str) -> String {
    words(name).join(" ")
}

fn digits(w: &str) -> bool {
    w.chars().all(|c| c.is_ascii_digit())
}

/// The learned fix for a track named `name`: the exact name first, then the name without its
/// trailing numbers one at a time ("Vocal Rec 1 3", a copy of "Vocal Rec 1", takes its fix), and
/// last a name that differs only in trailing numbers, when every such name was tagged alike
/// ("Vox Dub 3" takes the fix of "Vox Dub 2").
pub fn learned_fix<'a>(tagging: &'a Tagging, name: &str) -> Option<(String, &'a Vec<String>)> {
    let mut ws = words(name);
    if ws.is_empty() {
        return None;
    }
    loop {
        let key = ws.join(" ");
        if let Some(tags) = tagging.names.get(&key) {
            return Some((key, tags));
        }
        match ws.last() {
            Some(w) if digits(w) && ws.len() > 1 => {
                ws.pop();
            }
            _ => break,
        }
    }
    let stem = ws.join(" ");
    let strip = |k: &str| {
        let mut v: Vec<&str> = k.split(' ').collect();
        while v.len() > 1 && v.last().is_some_and(|w| digits(w)) {
            v.pop();
        }
        v.join(" ")
    };
    let mut alike = tagging.names.iter().filter(|(k, _)| strip(k) == stem);
    let (key, tags) = alike.next()?;
    let same = |a: &Vec<String>, b: &Vec<String>| a.len() == b.len() && a.iter().all(|t| has(b, t));
    alike.all(|(_, t)| same(t, tags)).then(|| (key.clone(), tags))
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
    // The middle of the longest clips: the middle of the whole track can fall in a gap.
    let mut clips: Vec<(i64, i64)> = t
        .clips()
        .iter()
        .filter(|c| !c.muted && matches!(c.content, ClipContent::Audio { .. }) && c.end() > c.start)
        .map(|c| (c.start, c.end()))
        .collect();
    clips.sort_by_key(|(a, b)| std::cmp::Reverse(b.saturating_sub(*a)));
    let sr = s.sample_rate.as_f64();
    let span = s.sample_rate.samples(LISTEN_SECONDS);
    clips.into_iter().take(LISTEN_TRIES).find_map(|(start, end)| {
        let mid = start.saturating_add(end.saturating_sub(start) / 2);
        let from = mid.saturating_sub(span / 2).max(start);
        let range = Range::new(from, from.saturating_add(span).min(end));
        analyse(&soundcraft_mix::render_clips(s, t.id, range), sr as f32)
    })
}

/// How many clips the scan tries before it gives up on hearing a track.
const LISTEN_TRIES: usize = 4;

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
    let silence = 10f32.powf(-60.0 / 20.0);
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
    if let Some(l) = &listening {
        hear(&mut tags, &mut reasons, l);
    } else if listen_to_audio && t.kind == TrackKind::Audio {
        reasons.push("not enough sound on the track to listen to".into());
    }
    // A learned fix for this name wins.
    if let Some((key, fixed)) = learned_fix(learned, &t.name) {
        tags = fixed.clone();
        reasons.push(format!("you tagged a track named \"{key}\" this way before"));
    }
    tags = order(clean_tags(tags.iter().map(String::as_str)));
    Suggestion { track: t.id, name: t.name.clone(), current: t.tags.clone(), tags, reasons, listening }
}

// The audio only fills in what the name leaves open: tested on real sessions, a direct guitar
// can look like drums and a frozen amp track like a direct one, so the rules are strict.
fn hear(tags: &mut Vec<String>, reasons: &mut Vec<String>, l: &Listening) {
    let named_kind = [DRUMS, VOCALS, GUITAR, BASS, KEYS, DI, MIC].iter().any(|k| has(tags, k));
    // Drums: sharp, unpitched hits with cymbal or stick brightness.
    if !named_kind && l.onsets_per_second >= 1.0 && l.pitched < 0.1 && l.crest_db >= 18.5 && l.highs >= 0.008 {
        add(tags, reasons, DRUMS, format!("the audio is unpitched hits ({:.1} a second) with bright attacks", l.onsets_per_second));
    }
    let drums = has(tags, DRUMS);
    let voice = has(tags, VOCALS);
    // A pickup straight into the desk: wide peaks and almost nothing above 5 kHz.
    if !drums && !voice && !has(tags, MIC) && !has(tags, DI) && l.crest_db >= 19.0 && l.highs < 0.0026 && l.onsets_per_second >= 1.0 {
        add(tags, reasons, DI, format!("the audio sounds like a direct signal ({:.0} dB peaks over average, almost no highs)", l.crest_db));
    }
    // The instrument behind a DI or mic: a bass has almost no highs at all.
    if !drums && !voice && !has(tags, GUITAR) && !has(tags, BASS) && !has(tags, KEYS) && (has(tags, DI) || has(tags, MIC)) {
        if l.pitched >= 0.3 && l.pitch_hz > 0.0 && l.pitch_hz < 110.0 && l.highs < 0.0015 {
            add(tags, reasons, BASS, format!("the pitch sits low (about {:.0} Hz) with no highs", l.pitch_hz));
        } else {
            add(tags, reasons, GUITAR, "a direct or miked track that does not sound like a bass".into());
        }
    }
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
            let learned = learned_fix(&s.tagging, &g.name).is_some();
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
    learn_all(tagging, &[(name.to_owned(), suggested.to_vec(), confirmed.to_vec())]);
}

/// [`learn`] for a batch of confirmed tracks `(name, suggested, confirmed)`. A word keeps only the
/// tags that every confirmed track with that word in its name has: "Xtra" on "Xtra Gtr Amp"
/// (Mic) and "Xtra Gtr DI" (DI) learns neither.
pub fn learn_all(tagging: &mut Tagging, items: &[(String, Vec<String>, Vec<String>)]) {
    for (name, suggested, confirmed) in items {
        learn_one(tagging, name, suggested, confirmed);
    }
    for (name, _, confirmed) in items {
        for w in words(name) {
            if let Some(entry) = tagging.words.get_mut(&w) {
                entry.retain(|t| has(confirmed, t));
            }
        }
    }
    tagging.words.retain(|_, v| !v.is_empty());
}

fn learn_one(tagging: &mut Tagging, name: &str, suggested: &[String], confirmed: &[String]) {
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
        if w.len() < 2 || digits(&w) || !word_tags(&w).is_empty() {
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
        assert_eq!(name_key("Vox Dub L 2"), "vox dub l 2");
    }

    fn v(tags: &[&str]) -> Vec<String> {
        tags.iter().map(|t| (*t).to_owned()).collect()
    }

    #[test]
    fn numbered_takes_keep_their_own_fix() {
        let mut t = Tagging::default();
        learn(&mut t, "VOCAL AHHS REC 1", &v(&[VOCALS]), &v(&[VOCALS, MAINS]));
        learn(&mut t, "VOCAL AHHS REC 2", &v(&[VOCALS]), &v(&[VOCALS, DUBS]));
        assert_eq!(learned_fix(&t, "VOCAL AHHS REC 1").map(|f| f.1.clone()), Some(v(&[VOCALS, MAINS])));
        assert_eq!(learned_fix(&t, "VOCAL AHHS REC 2").map(|f| f.1.clone()), Some(v(&[VOCALS, DUBS])));
        // A numbered copy of a take falls back to the take.
        assert_eq!(learned_fix(&t, "VOCAL AHHS REC 1 11").map(|f| f.1.clone()), Some(v(&[VOCALS, MAINS])));
        assert_eq!(learned_fix(&t, "VOCAL AHHS REC 2 3").map(|f| f.1.clone()), Some(v(&[VOCALS, DUBS])));
        // A take number never seen: the two takes disagree, so nothing is guessed.
        assert_eq!(learned_fix(&t, "VOCAL AHHS REC 3"), None);
        // When every numbered name was tagged alike, a new number takes it.
        learn(&mut t, "Hamish Gtr 2", &v(&[GUITAR]), &v(&[GUITAR, MIC]));
        assert_eq!(learned_fix(&t, "Hamish Gtr 5").map(|f| f.1.clone()), Some(v(&[GUITAR, MIC])));
    }

    #[test]
    fn a_word_shared_by_different_tracks_learns_only_what_they_share() {
        let mut t = Tagging::default();
        learn_all(
            &mut t,
            &[
                ("XTRA Guitar Crunchy".into(), v(&[GUITAR]), v(&[GUITAR, MIC])),
                ("XTRA Guitar Direct In".into(), v(&[GUITAR, DI]), v(&[GUITAR, DI])),
                ("LEFT - CRUNCHY".into(), v(&[LEFT]), v(&[LEFT, GUITAR, MIC])),
            ],
        );
        assert!(!t.words.contains_key("xtra"), "{t:?}");
        assert_eq!(t.words.get("crunchy"), Some(&v(&[MIC, GUITAR])));
    }

    fn heard(named: &[&str], l: Listening) -> Vec<String> {
        let (mut tags, mut reasons) = (v(named), Vec::new());
        hear(&mut tags, &mut reasons, &l);
        tags
    }

    #[test]
    fn the_audio_rules_hold_on_numbers_from_real_tracks() {
        let l = |crest_db, onsets_per_second, pitched, pitch_hz, highs| Listening {
            active_seconds: 20.0,
            rms_db: -20.0,
            crest_db,
            onsets_per_second,
            pitched,
            pitch_hz,
            highs,
        };
        // A real snare and kick.
        assert_eq!(heard(&[], l(22.7, 7.27, 0.028, 220.0, 0.02)), v(&[DRUMS]));
        assert_eq!(heard(&[], l(19.0, 1.76, 0.0, 0.0, 0.011)), v(&[DRUMS]));
        // A direct guitar: wide peaks and lots of attacks, but no highs. Not drums.
        assert_eq!(heard(&[], l(21.5, 7.52, 0.12, 73.4, 0.0023)), v(&[DI, GUITAR]));
        // A clean amp tone with few pitched frames: neither drums nor DI.
        assert_eq!(heard(&[], l(17.3, 5.67, 0.018, 148.0, 0.013)), v(&[]));
        // Frozen amp tracks look direct in crest but keep their highs.
        assert_eq!(heard(&[], l(22.6, 3.39, 0.97, 744.0, 0.0064)), v(&[]));
        assert_eq!(heard(&[], l(16.2, 2.95, 0.77, 73.9, 0.003)), v(&[]));
        // A drop-tuned guitar DI is a guitar; a bass DI has no highs at all.
        assert_eq!(heard(&[DI], l(16.2, 2.95, 0.77, 73.9, 0.003)), v(&[DI, GUITAR]));
        assert_eq!(heard(&[DI], l(14.8, 3.3, 0.37, 73.7, 0.000_55)), v(&[DI, BASS]));
        // The name wins: a named DI is never called drums.
        assert_eq!(heard(&[LEFT, DI], l(21.5, 7.52, 0.12, 73.4, 0.0023)), v(&[LEFT, DI, GUITAR]));
        // Held vocals.
        assert_eq!(heard(&[VOCALS], l(6.9, 0.31, 1.0, 370.0, 0.0009)), v(&[VOCALS]));
    }

    #[test]
    fn the_scan_hears_clips_with_a_long_gap_between_them() {
        use soundcraft_audio_io::AudioBuffer;
        use soundcraft_model::{ChannelFormat, Clip};
        let mut s = Session::default();
        let t = s.add_track(TrackKind::Audio, ChannelFormat::Mono, Some("Gap"));
        let sr = 48_000usize;
        let tone: Vec<f32> = (0..sr * 3).map(|i| (i as f32 * 220.0 * std::f32::consts::TAU / sr as f32).sin() * 0.3).collect();
        // Two 3-second clips four minutes apart: the middle of the track is silence.
        for (k, at) in [0i64, 240 * sr as i64].into_iter().enumerate() {
            let src = crate::io::add_source(
                &mut s,
                &format!("t{k}"),
                AudioBuffer { sample_rate: 48_000, channels: vec![tone.clone()] },
                None,
                soundcraft_audio_io::FileFormat::Wav,
            );
            let id = s.new_clip_id();
            crate::edit::place_clip(&mut s, t, Clip::audio(id, "t", src, 0, at, tone.len() as i64));
        }
        let track = s.track(t).unwrap().clone();
        let l = listen(&s, &track).expect("the clips have sound");
        assert!(l.active_seconds > 2.0 && (l.pitch_hz - 220.0).abs() < 5.0, "{l:?}");
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
