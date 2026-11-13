//! The Session View of the Ableton-style layout: scenes (rows), each track's clip slots (one per
//! scene), what a clip does when it ends (follow actions), and which clips are playing. Also the
//! session's Groove Pool and a track's Instrument Rack macros.
//!
//! The playing clips are live state, not part of the saved document: [`Session::launch`] is never
//! saved, compared for "modified" or restored by Undo.

use crate::{Clip, Groove, MAX_NAME_CHARS, Session, Track, TrackId, truncate_chars};
use soundcraft_time::{Range, Samples};

/// Most scenes a session keeps (files are hostile input).
pub const MAX_SCENES: usize = 256;
/// Most grooves in the Groove Pool.
pub const MAX_GROOVES: usize = 256;
/// Macro knobs on an Instrument Rack.
pub const MACROS: usize = 8;
/// Most parameters one macro moves.
pub const MAX_MACRO_TARGETS: usize = 16;
/// Most clips launched at once (one current and some waiting per track).
pub const MAX_LAUNCHES: usize = 4096;
/// Most plays a follow action waits for.
pub const MAX_FOLLOW_PLAYS: u32 = 64;
/// Most passes of one clip reported for one window: a block never holds more than a few.
const MAX_PASSES: usize = 64;
/// Most follow actions taken looking for what plays in one window. The UI settles the launches as
/// the song plays, so a window only ever needs a few; the cap bounds a hostile chain.
const MAX_FOLLOWS: usize = 256;

/// A row of the Session View: the clips of every track in one slot, launched together.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Scene {
    pub id: u64,
    pub name: String,
}

/// What a clip does when its plays are over.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FollowAction {
    /// Play the same clip again (and decide again after it).
    #[default]
    Again,
    /// Play the next clip down the track, back to the top after the last.
    Next,
    /// Play any clip of the track, picked at random (the same pick on every play and bounce).
    Random,
    /// Stop the track.
    Stop,
}

impl FollowAction {
    pub const ALL: [FollowAction; 4] = [FollowAction::Again, FollowAction::Next, FollowAction::Random, FollowAction::Stop];

    pub fn id(self) -> &'static str {
        match self {
            FollowAction::Again => "again",
            FollowAction::Next => "next",
            FollowAction::Random => "random",
            FollowAction::Stop => "stop",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            FollowAction::Again => "Play Again",
            FollowAction::Next => "Next",
            FollowAction::Random => "Random",
            FollowAction::Stop => "Stop",
        }
    }

    pub fn parse(s: &str) -> Option<FollowAction> {
        let s = s.trim();
        FollowAction::ALL.into_iter().find(|a| a.id().eq_ignore_ascii_case(s) || a.label().eq_ignore_ascii_case(s))
    }
}

/// A clip's follow action: after `plays` plays of the clip, do `action`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Follow {
    pub action: FollowAction,
    #[serde(default = "one")]
    pub plays: u32,
}

fn one() -> u32 {
    1
}

impl Follow {
    /// Plays before the action, 1 to [`MAX_FOLLOW_PLAYS`].
    pub fn plays(&self) -> i64 {
        i64::from(self.plays.clamp(1, MAX_FOLLOW_PLAYS))
    }
}

/// A groove saved in the session's Groove Pool. Clips that use it keep a copy in
/// [`Clip::groove`] (what plays) and its id in [`Clip::groove_pool`], so changing the groove here
/// changes every clip that uses it.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct GroovePreset {
    pub id: u64,
    pub name: String,
    pub groove: Groove,
}

/// One parameter a macro moves: the macro's 0 sets `min`, its 1 sets `max`.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MacroTarget {
    pub param: String,
    pub min: f32,
    pub max: f32,
}

/// A macro knob of an Instrument Rack.
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct Macro {
    #[serde(default)]
    pub name: String,
    /// 0 to 1.
    #[serde(default)]
    pub value: f32,
    #[serde(default)]
    pub targets: Vec<MacroTarget>,
}

/// An Instrument Rack around a track's instrument: eight macros, each moving parameters of the
/// instrument inside.
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct Rack {
    #[serde(default)]
    pub macros: Vec<Macro>,
}

impl Rack {
    /// Eight unnamed macros at zero.
    pub fn new() -> Rack {
        Rack { macros: (0..MACROS).map(|i| Macro { name: format!("Macro {}", i + 1), value: 0.0, targets: Vec::new() }).collect() }
    }

    /// Exactly eight macros, values within 0..=1, bounded targets.
    pub fn sanitize(&mut self) {
        self.macros.truncate(MACROS);
        while self.macros.len() < MACROS {
            let n = self.macros.len() + 1;
            self.macros.push(Macro { name: format!("Macro {n}"), ..Macro::default() });
        }
        for m in &mut self.macros {
            truncate_chars(&mut m.name, MAX_NAME_CHARS);
            m.value = if m.value.is_finite() { m.value.clamp(0.0, 1.0) } else { 0.0 };
            m.targets.truncate(MAX_MACRO_TARGETS);
            m.targets.retain(|t| t.min.is_finite() && t.max.is_finite());
            m.targets.iter_mut().for_each(|t| truncate_chars(&mut t.param, MAX_NAME_CHARS));
        }
    }
}

/// A clip launched from the Session View: the clip in `slot` of `track` plays from `at`. A later
/// launch of the same track takes over at its own `at`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Launch {
    pub track: TrackId,
    pub slot: usize,
    pub at: Samples,
}

/// One pass of a launched clip: `clip` plays from its start at `start`, and sounds only within
/// `window` (the part of the asked range this pass covers). `run` is where the clip's current run
/// of plays began, which its follow action counts from.
#[derive(Debug, Clone, Copy)]
pub struct Pass<'a> {
    pub clip: &'a Clip,
    pub slot: usize,
    pub start: Samples,
    pub window: Range,
    pub run: Samples,
}

impl Track {
    /// The clip in Session View slot `i`, if any.
    pub fn slot(&self, i: usize) -> Option<&Clip> {
        self.slots.get(i).and_then(Option::as_ref)
    }

    /// The first filled slot after `slot`, going round; `slot` itself if it is the only one.
    fn next_filled(&self, slot: usize) -> Option<usize> {
        let n = self.slots.len();
        (1..=n).map(|k| (slot.saturating_add(k)) % n.max(1)).find(|i| self.slot(*i).is_some())
    }

    /// A filled slot picked from `seed`.
    fn random_filled(&self, seed: u64) -> Option<usize> {
        let filled = self.slots.iter().filter(|c| c.is_some()).count();
        if filled == 0 {
            return None;
        }
        let k = usize::try_from(mix64(seed) % filled as u64).unwrap_or(0);
        self.slots.iter().enumerate().filter(|(_, c)| c.is_some()).nth(k).map(|(i, _)| i)
    }
}

/// SplitMix64's finaliser: a well-spread hash of one word.
fn mix64(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

/// Follow the chain of clips launched on `t` in `slot` at `start`, reporting each pass that meets
/// `win`. Returns false once the chain stops or runs out of clips.
fn chain<'a>(t: &'a Track, mut slot: usize, mut start: Samples, win: Range, f: &mut impl FnMut(Pass<'a>)) {
    for _ in 0..MAX_FOLLOWS {
        let Some(clip) = t.slot(slot) else { return };
        let len = clip.length.max(1);
        let end = match clip.follow {
            None => Samples::MAX,
            Some(fo) => start.saturating_add(len.saturating_mul(fo.plays())),
        };
        if end > win.start {
            let from = win.start.max(start);
            let mut k = from.saturating_sub(start) / len;
            for _ in 0..MAX_PASSES {
                let ps = start.saturating_add(k.saturating_mul(len));
                if ps >= win.end || ps >= end {
                    break;
                }
                let window = Range { start: win.start.max(ps), end: win.end.min(end).min(ps.saturating_add(len)) };
                if window.start < window.end {
                    f(Pass { clip, slot, start: ps, window, run: start });
                }
                k = k.saturating_add(1);
            }
        }
        if end >= win.end {
            return;
        }
        let Some(fo) = clip.follow else { return };
        let next = match fo.action {
            FollowAction::Again => Some(slot),
            FollowAction::Next => t.next_filled(slot),
            FollowAction::Random => t.random_filled(t.id.0.rotate_left(32) ^ end as u64),
            FollowAction::Stop => None,
        };
        let Some(next) = next else { return };
        slot = next;
        start = end;
    }
}

impl Session {
    /// The passes of launched clips that sound on track `t` within `range`, in order of launch.
    /// A track with nothing launched reports nothing. Never allocates (it runs on the audio thread).
    pub fn launch_passes<'a>(&'a self, t: &'a Track, range: Range, mut f: impl FnMut(Pass<'a>)) {
        for (i, l) in self.launch.iter().enumerate() {
            if l.track != t.id {
                continue;
            }
            // A launch at the same moment later in the list replaces this one.
            if self.launch.iter().skip(i + 1).any(|m| m.track == t.id && m.at == l.at) {
                continue;
            }
            let until = self.launch.iter().filter(|m| m.track == t.id && m.at > l.at).map(|m| m.at).min().unwrap_or(Samples::MAX);
            let win = Range { start: l.at.max(range.start), end: until.min(range.end) };
            if win.start < win.end {
                chain(t, l.slot, l.at, win, &mut f);
            }
        }
    }

    /// Whether anything is launched (playing or waiting) on track `t`: its Arrangement clips are
    /// silent then.
    pub fn launched(&self, t: TrackId) -> bool {
        self.launch.iter().any(|l| l.track == t)
    }

    /// The slot playing on track `t` at `pos`, and where its current run of plays began.
    pub fn playing_slot(&self, t: &Track, pos: Samples) -> Option<(usize, Samples)> {
        let mut out = None;
        self.launch_passes(t, Range { start: pos, end: pos.saturating_add(1) }, |p| out = Some((p.slot, p.run)));
        out
    }

    /// Launches waiting to start after `pos` on track `t`.
    pub fn queued_slots(&self, t: TrackId, pos: Samples) -> impl Iterator<Item = usize> + '_ {
        self.launch.iter().filter(move |l| l.track == t && l.at > pos).map(|l| l.slot)
    }

    /// Fold what has played up to `pos` into the launches: each track keeps the clip playing at
    /// `pos` (launched where its current run began) and the launches still to come; a track that
    /// has stopped keeps only those to come. `None` when nothing would change.
    pub fn settled_launches(&self, pos: Samples) -> Option<Vec<Launch>> {
        let mut out: Vec<Launch> = Vec::with_capacity(self.launch.len());
        let mut tracks: Vec<TrackId> = Vec::new();
        for l in &self.launch {
            if !tracks.contains(&l.track) {
                tracks.push(l.track);
            }
        }
        for id in tracks {
            let Some(t) = self.track(id) else { continue };
            if let Some((slot, run)) = self.playing_slot(t, pos) {
                out.push(Launch { track: id, slot, at: run });
            }
            for l in self.launch.iter().filter(|l| l.track == id && l.at > pos) {
                match out.iter_mut().find(|m| m.track == id && m.at == l.at) {
                    Some(m) => *m = *l,
                    None => out.push(*l),
                }
            }
        }
        out.truncate(MAX_LAUNCHES);
        (out != self.launch).then_some(out)
    }

    /// [`Session::settled_launches`], applied. Returns whether anything changed.
    pub fn settle_launches(&mut self, pos: Samples) -> bool {
        match self.settled_launches(pos) {
            Some(v) => {
                self.launch = v;
                true
            }
            None => false,
        }
    }

    /// Repair the Session View after loading untrusted data: bounded scenes, every track with one
    /// slot per scene, sane slot clips, grooves and racks.
    pub(crate) fn sanitize_launcher(&mut self) {
        self.scenes.truncate(MAX_SCENES);
        self.scenes.iter_mut().for_each(|s| truncate_chars(&mut s.name, MAX_NAME_CHARS));
        let n = self.scenes.len();
        for t in &mut self.tracks {
            t.slots.resize(n, None);
            for c in t.slots.iter_mut().flatten() {
                c.sanitize();
                c.start = 0;
            }
            if let Some(r) = &mut t.rack {
                r.sanitize();
            }
        }
        self.grooves.truncate(MAX_GROOVES);
        for g in &mut self.grooves {
            truncate_chars(&mut g.name, MAX_NAME_CHARS);
            g.groove.sanitize();
        }
        self.launch.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ChannelFormat, ClipId, TrackKind};
    use soundcraft_midi::Sequence;

    fn session(lens: &[Samples]) -> (Session, TrackId) {
        let mut s = Session::default();
        let t = s.add_track(TrackKind::Midi, ChannelFormat::Mono, None);
        for (i, len) in lens.iter().enumerate() {
            s.scenes.push(Scene { id: 100 + i as u64, name: format!("{}", i + 1) });
            let c = Clip::midi(ClipId(200 + i as u64), "c", 0, *len, Sequence::default());
            if let Some(tr) = s.track_mut(t) {
                tr.slots.push(Some(c));
            }
        }
        (s, t)
    }

    fn passes(s: &Session, t: TrackId, a: Samples, b: Samples) -> Vec<(usize, Samples, Samples, Samples)> {
        let tr = s.track(t).cloned().unwrap();
        let mut v = Vec::new();
        s.launch_passes(&tr, Range { start: a, end: b }, |p| v.push((p.slot, p.start, p.window.start, p.window.end)));
        v
    }

    #[test]
    fn a_launched_clip_loops_until_the_next_launch_takes_over() {
        let (mut s, t) = session(&[100, 50]);
        s.launch.push(Launch { track: t, slot: 0, at: 10 });
        s.launch.push(Launch { track: t, slot: 1, at: 260 });
        assert_eq!(
            passes(&s, t, 0, 400),
            vec![(0, 10, 10, 110), (0, 110, 110, 210), (0, 210, 210, 260), (1, 260, 260, 310), (1, 310, 310, 360), (1, 360, 360, 400)]
        );
        assert!(passes(&s, t, 0, 10).is_empty(), "nothing before the launch");
    }

    #[test]
    fn follow_actions_play_next_again_random_and_stop() {
        let (mut s, t) = session(&[100, 50, 70]);
        let set = |s: &mut Session, i: usize, action: FollowAction, plays: u32| {
            if let Some(c) = s.track_mut(t).and_then(|tr| tr.slots.get_mut(i)).and_then(Option::as_mut) {
                c.follow = Some(Follow { action, plays });
            }
        };
        set(&mut s, 0, FollowAction::Next, 2);
        set(&mut s, 1, FollowAction::Stop, 1);
        s.launch.push(Launch { track: t, slot: 0, at: 0 });
        assert_eq!(passes(&s, t, 0, 1000), vec![(0, 0, 0, 100), (0, 100, 100, 200), (1, 200, 200, 250)]);
        set(&mut s, 1, FollowAction::Again, 1);
        let p = passes(&s, t, 0, 400);
        assert_eq!(p.last(), Some(&(1, 350, 350, 400)));
        set(&mut s, 0, FollowAction::Random, 1);
        set(&mut s, 1, FollowAction::Random, 1);
        set(&mut s, 2, FollowAction::Random, 1);
        let a = passes(&s, t, 0, 5000);
        assert_eq!(a, passes(&s, t, 0, 5000), "random picks the same on every play");
        let used: std::collections::BTreeSet<usize> = a.iter().map(|p| p.0).collect();
        assert!(used.len() > 1, "random moves between clips: {a:?}");
    }

    #[test]
    fn settling_keeps_the_current_run_and_drops_what_stopped() {
        let (mut s, t) = session(&[100, 50]);
        if let Some(c) = s.track_mut(t).and_then(|tr| tr.slots.get_mut(0)).and_then(Option::as_mut) {
            c.follow = Some(Follow { action: FollowAction::Next, plays: 1 });
        }
        if let Some(c) = s.track_mut(t).and_then(|tr| tr.slots.get_mut(1)).and_then(Option::as_mut) {
            c.follow = Some(Follow { action: FollowAction::Stop, plays: 2 });
        }
        s.launch.push(Launch { track: t, slot: 0, at: 0 });
        assert!(s.settle_launches(120));
        assert_eq!(s.launch, vec![Launch { track: t, slot: 1, at: 100 }]);
        assert_eq!(passes(&s, t, 120, 300), vec![(1, 100, 120, 150), (1, 150, 150, 200)]);
        assert!(!s.settle_launches(130), "nothing new");
        assert!(s.settle_launches(250));
        assert!(s.launch.is_empty(), "stopped");
    }

    #[test]
    fn a_hostile_session_view_is_repaired() {
        let (mut s, t) = session(&[100]);
        if let Some(tr) = s.track_mut(t) {
            tr.slots.resize(9, None);
            tr.rack = Some(Rack { macros: vec![Macro { value: f32::NAN, ..Macro::default() }] });
        }
        s.grooves.push(GroovePreset { id: 9, name: "x".into(), groove: Groove { swing: 7.0, humanize: f32::NAN } });
        s.launch.push(Launch { track: t, slot: 3, at: 0 });
        s.sanitize_launcher();
        let tr = s.track(t).unwrap();
        assert_eq!(tr.slots.len(), 1);
        assert_eq!(tr.rack.as_ref().map(|r| r.macros.len()), Some(MACROS));
        assert_eq!(tr.rack.as_ref().map(|r| r.macros[0].value), Some(0.0));
        assert_eq!(s.grooves[0].groove, Groove { swing: 1.0, humanize: 0.0 });
        assert!(s.launch.is_empty());
    }
}
