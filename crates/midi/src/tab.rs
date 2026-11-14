//! Tablature: where on a fretted instrument each note of a [`Sequence`] is played.
//!
//! The notes stay the one source of truth (pitch, time, length). A [`Tuning`] says which pitch
//! each open string sounds, and a sequence may carry [`TabPos`] hints that pin a note to a string
//! (as a tab file wrote it). A note without a hint, or whose hint no longer fits its pitch (after a
//! transpose, say), is placed by [`place`]: the lowest fret that keeps notes sounding together on
//! different strings.

use crate::{Note, Sequence};
use serde::{Deserialize, Serialize};

/// Most strings an instrument may have.
pub const MAX_STRINGS: usize = 10;
/// Highest fret written.
pub const MAX_FRET: u8 = 36;

/// The open pitch of each string, string 1 (the highest, nearest the floor) first, and the capo.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tuning {
    /// MIDI pitch of each open string, string 1 first. 1..=[`MAX_STRINGS`] strings.
    pub strings: Vec<u8>,
    /// Capo fret: every string sounds this many semitones higher, and frets are counted from it.
    #[serde(default)]
    pub capo: u8,
}

impl Default for Tuning {
    fn default() -> Self {
        Tuning::guitar()
    }
}

impl Tuning {
    /// Standard six-string guitar: E4 B3 G3 D3 A2 E2.
    pub fn guitar() -> Self {
        Tuning { strings: vec![64, 59, 55, 50, 45, 40], capo: 0 }
    }

    /// Standard four-string bass: G2 D2 A1 E1.
    pub fn bass() -> Self {
        Tuning { strings: vec![43, 38, 33, 28], capo: 0 }
    }

    /// The tuning with hostile values made safe: 1..=MAX_STRINGS strings of pitches 0..=127, a capo
    /// of at most [`MAX_FRET`]. An empty tuning becomes standard guitar.
    pub fn sanitized(&self) -> Tuning {
        let strings: Vec<u8> = self.strings.iter().take(MAX_STRINGS).map(|p| (*p).min(127)).collect();
        if strings.is_empty() {
            return Tuning { capo: self.capo.min(MAX_FRET), ..Tuning::guitar() };
        }
        Tuning { strings, capo: self.capo.min(MAX_FRET) }
    }

    /// Short name: "E A D G B E" (low to high, as players read a tuning).
    pub fn label(&self) -> String {
        let names: Vec<String> = self.strings.iter().rev().map(|p| pitch_class(*p).to_string()).collect();
        let mut s = names.join(" ");
        if self.capo > 0 {
            s.push_str(&format!(" (capo {})", self.capo));
        }
        s
    }

    /// The fret `pitch` falls on, on string `string` (1-based), counting from the capo, or `None`
    /// when it cannot be played there.
    pub fn fret(&self, string: u8, pitch: u8) -> Option<u8> {
        let open = *self.strings.get(usize::from(string).checked_sub(1)?)?;
        let open = u16::from(open) + u16::from(self.capo);
        let f = u16::from(pitch).checked_sub(open)?;
        u8::try_from(f).ok().filter(|f| *f <= MAX_FRET)
    }

    /// The pitch string `string` (1-based) sounds at fret `fret` (counted from the capo).
    pub fn pitch(&self, string: u8, fret: u8) -> Option<u8> {
        let open = *self.strings.get(usize::from(string).checked_sub(1)?)?;
        let p = u16::from(open) + u16::from(self.capo) + u16::from(fret);
        u8::try_from(p).ok().filter(|p| *p <= 127)
    }

    /// Lowest and highest pitch the instrument can play.
    pub fn range(&self) -> (u8, u8) {
        let lo = self.strings.iter().min().copied().unwrap_or(40).saturating_add(self.capo);
        let hi = self.strings.iter().max().copied().unwrap_or(64).saturating_add(self.capo).saturating_add(MAX_FRET).min(127);
        (lo, hi)
    }
}

fn pitch_class(p: u8) -> &'static str {
    ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"].get(usize::from(p % 12)).copied().unwrap_or("?")
}

/// A note pinned to a string: the note that starts at `start` with `pitch` (and `channel`) is
/// played on `string` (1-based).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TabPos {
    pub start: i64,
    pub pitch: u8,
    #[serde(default)]
    pub channel: u8,
    pub string: u8,
    /// How the note is played: [`tech`] bits.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub tech: u16,
}

fn is_zero(v: &u16) -> bool {
    *v == 0
}

/// Playing techniques a tab note can carry (bits of [`TabPos::tech`]). The notes themselves
/// already sound the technique (lengths, velocities, extra notes, glides); these bits say how to
/// draw it and what to write back to a score.
pub mod tech {
    /// Palm muted (P.M.).
    pub const PALM_MUTE: u16 = 1;
    /// A dead (muted) note, drawn as x.
    pub const DEAD: u16 = 1 << 1;
    /// Hammered on or pulled off to the next note on the string.
    pub const HOPO: u16 = 1 << 2;
    /// Reached by a hammer-on or pull-off, not picked.
    pub const HOPO_DEST: u16 = 1 << 3;
    /// Slides to the next note on the string without picking it again.
    pub const SLIDE_LEGATO: u16 = 1 << 4;
    /// Slides to the next note on the string, which is picked again.
    pub const SLIDE_SHIFT: u16 = 1 << 5;
    /// Slides in from below.
    pub const SLIDE_IN_BELOW: u16 = 1 << 6;
    /// Slides in from above.
    pub const SLIDE_IN_ABOVE: u16 = 1 << 7;
    /// Slides out downwards.
    pub const SLIDE_OUT_DOWN: u16 = 1 << 8;
    /// Slides out upwards.
    pub const SLIDE_OUT_UP: u16 = 1 << 9;
    /// Let ring.
    pub const LET_RING: u16 = 1 << 10;
    /// A grace note, played just before the beat.
    pub const GRACE: u16 = 1 << 11;
    /// Accented.
    pub const ACCENT: u16 = 1 << 12;
    /// Staccato.
    pub const STACCATO: u16 = 1 << 13;
    /// Vibrato.
    pub const VIBRATO: u16 = 1 << 14;
    /// One stroke of a tremolo-picked note.
    pub const TREMOLO: u16 = 1 << 15;
}

/// Where one note is played.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Placed {
    /// Index of the note in the sequence's `notes`.
    pub note: usize,
    /// String, 1-based; 0 when the note is out of the instrument's range.
    pub string: u8,
    pub fret: u8,
    /// The note's [`tech`] bits.
    pub tech: u16,
}

/// The hint for `n`, if any.
pub fn hint<'a>(seq: &'a Sequence, n: &Note) -> Option<&'a TabPos> {
    seq.tab.iter().find(|t| t.start == n.start && t.pitch == n.pitch && t.channel == n.channel)
}

/// Place every note of `seq` on `tuning`: a hint is kept when it still fits the note's pitch; other
/// notes take the lowest fret on a string not already sounding at that moment. Notes outside the
/// range get string 0. Returned in note order.
pub fn place(seq: &Sequence, tuning: &Tuning) -> Vec<Placed> {
    let tuning = tuning.sanitized();
    let mut out: Vec<Placed> = Vec::with_capacity(seq.notes.len());
    // Notes still sounding: (end tick, string).
    let mut busy: Vec<(i64, u8)> = Vec::new();
    let mut order: Vec<usize> = (0..seq.notes.len()).collect();
    order.sort_by_key(|i| seq.notes.get(*i).map(|n| (n.start, std::cmp::Reverse(n.pitch))));
    for i in order {
        let Some(n) = seq.notes.get(i) else { continue };
        busy.retain(|(end, _)| *end > n.start);
        let taken = |s: u8| busy.iter().any(|(_, b)| *b == s);
        let h = hint(seq, n);
        let tech = h.map_or(0, |t| t.tech);
        let pinned = h.map(|t| t.string).filter(|s| *s > 0).and_then(|s| tuning.fret(s, n.pitch).map(|f| (s, f)));
        let chosen = pinned.or_else(|| {
            let count = u8::try_from(tuning.strings.len()).unwrap_or(0);
            (1..=count).filter(|s| !taken(*s)).filter_map(|s| tuning.fret(s, n.pitch).map(|f| (s, f))).min_by_key(|(s, f)| (*f, *s))
        });
        let chosen = chosen.or_else(|| {
            // Every fitting string is busy: share the lowest-fret one rather than drop the note.
            let count = u8::try_from(tuning.strings.len()).unwrap_or(0);
            (1..=count).filter_map(|s| tuning.fret(s, n.pitch).map(|f| (s, f))).min_by_key(|(s, f)| (*f, *s))
        });
        match chosen {
            Some((string, fret)) => {
                busy.push((n.end(), string));
                out.push(Placed { note: i, string, fret, tech });
            }
            None => out.push(Placed { note: i, string: 0, fret: 0, tech }),
        }
    }
    out.sort_by_key(|p| p.note);
    out
}

/// Pin note `index` of `seq` to `string`, when its pitch can be played there. Returns whether it
/// was pinned.
pub fn pin(seq: &mut Sequence, tuning: &Tuning, index: usize, string: u8) -> bool {
    let Some(n) = seq.notes.get(index).copied() else { return false };
    if tuning.sanitized().fret(string, n.pitch).is_none() {
        return false;
    }
    let tech = hint(seq, &n).map_or(0, |t| t.tech);
    seq.tab.retain(|t| !(t.start == n.start && t.pitch == n.pitch && t.channel == n.channel));
    seq.tab.push(TabPos { start: n.start, pitch: n.pitch, channel: n.channel, string, tech });
    true
}

/// Drop hints that no longer name a note of the sequence.
pub fn prune(seq: &mut Sequence) {
    let notes = &seq.notes;
    seq.tab.retain(|t| notes.iter().any(|n| n.start == t.start && n.pitch == t.pitch && n.channel == t.channel));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(pitch: u8, start: i64, length: i64) -> Note {
        Note { pitch, velocity: 100, release_velocity: 64, channel: 0, start, length }
    }

    #[test]
    fn standard_tuning_maps_pitches_to_frets() {
        let g = Tuning::guitar();
        assert_eq!(g.fret(6, 40), Some(0));
        assert_eq!(g.fret(6, 45), Some(5));
        assert_eq!(g.fret(1, 64), Some(0));
        assert_eq!(g.fret(1, 63), None, "below the open string");
        assert_eq!(g.fret(7, 50), None, "no seventh string");
        assert_eq!(g.fret(0, 50), None);
        assert_eq!(g.pitch(5, 2), Some(47));
        assert_eq!(g.label(), "E A D G B E");
        let capo = Tuning { capo: 2, ..Tuning::guitar() };
        assert_eq!(capo.fret(6, 42), Some(0));
        assert_eq!(capo.label(), "E A D G B E (capo 2)");
    }

    #[test]
    fn chords_spread_across_strings_and_hints_win() {
        // An open E major chord: E2 B2 E3 G#3 B3 E4.
        let mut seq = Sequence { notes: [40, 47, 52, 56, 59, 64].iter().map(|p| note(*p, 0, 960)).collect(), ..Sequence::default() };
        let placed = place(&seq, &Tuning::guitar());
        let mut strings: Vec<u8> = placed.iter().map(|p| p.string).collect();
        strings.sort_unstable();
        assert_eq!(strings, [1, 2, 3, 4, 5, 6], "{placed:?}");
        // A lone A3 goes to the lowest fret (string 3, fret 2); pinned to string 4 it is fret 7.
        seq.notes = vec![note(57, 0, 960)];
        assert_eq!(place(&seq, &Tuning::guitar())[0], Placed { note: 0, string: 3, fret: 2, tech: 0 });
        assert!(pin(&mut seq, &Tuning::guitar(), 0, 4));
        assert_eq!(place(&seq, &Tuning::guitar())[0], Placed { note: 0, string: 4, fret: 7, tech: 0 });
        assert!(!pin(&mut seq, &Tuning::guitar(), 0, 2), "A3 is below the open B string");
        // After a transpose the hint no longer matches, so the note is placed afresh.
        seq.notes[0].pitch = 59;
        assert_eq!(place(&seq, &Tuning::guitar())[0].string, 2);
        prune(&mut seq);
        assert!(seq.tab.is_empty());
    }

    #[test]
    fn hostile_tunings_and_notes_are_safe() {
        let t = Tuning { strings: vec![200; 40], capo: 255 }.sanitized();
        assert_eq!(t.strings.len(), MAX_STRINGS);
        assert!(t.strings.iter().all(|p| *p <= 127));
        assert_eq!(t.fret(1, 127), None);
        assert_eq!(Tuning { strings: vec![], capo: 0 }.sanitized(), Tuning::guitar());
        let seq = Sequence { notes: vec![note(10, 0, 1), note(127, 0, 1)], ..Sequence::default() };
        let placed = place(&seq, &Tuning::guitar());
        assert_eq!(placed[0].string, 0, "too low for a guitar");
        assert_eq!(placed[1].string, 0, "too high for a guitar");
    }
}
