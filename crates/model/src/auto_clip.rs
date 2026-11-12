//! Automation clips (FL-style): a stretch of one automation lane held as a clip that can be moved
//! and copied. The lane is what plays; a clip writes its shape into the lane where it sits, so the
//! mix reads automation the same way with or without clips.

use crate::clip::bound;
use crate::{AutoParam, AutomationLane, AutomationPoint, MAX_NAME_CHARS, TrackId, truncate_chars};
use soundcraft_time::{MAX_POSITION, Samples};

/// Most automation clips one session holds. Input-derived, so capped.
pub const MAX_AUTO_CLIPS: usize = 4096;
/// Most breakpoints one automation clip holds.
pub const MAX_AUTO_CLIP_POINTS: usize = 65_536;

/// One automation clip.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AutoClip {
    pub id: u64,
    pub name: String,
    pub track: TrackId,
    pub param: AutoParam,
    /// Timeline start (samples).
    pub start: Samples,
    /// Length (samples, > 0).
    pub length: Samples,
    /// The shape: breakpoints at offsets from `start`, sorted, all inside `0..length`.
    pub points: Vec<AutomationPoint>,
}

impl AutoClip {
    pub fn end(&self) -> Samples {
        self.start.saturating_add(self.length)
    }

    /// Take the lane's shape over `start..start + length` into the clip. Its value at both ends is
    /// always kept, so a clip over a ramp or an empty stretch still has a shape.
    pub fn capture(&mut self, lane: Option<&AutomationLane>, default: f32) {
        let (start, end) = (self.start, self.end());
        let at = |x: Samples| lane.map_or(default, |l| if l.points.is_empty() { default } else { l.value_at(x, default) });
        let mut pts = vec![AutomationPoint { at: 0, value: at(start) }];
        if let Some(l) = lane {
            pts.extend(
                l.points
                    .iter()
                    .filter(|p| p.at > start && p.at < end - 1)
                    .take(MAX_AUTO_CLIP_POINTS - 2)
                    .map(|p| AutomationPoint { at: p.at - start, value: p.value }),
            );
        }
        if self.length > 1 {
            pts.push(AutomationPoint { at: self.length - 1, value: at(end - 1) });
        }
        self.points = pts;
    }

    /// Write the clip's shape into `lane` where the clip sits now, replacing what was there.
    pub fn write(&self, lane: &mut AutomationLane) {
        lane.clear_range(self.start, self.end());
        for p in &self.points {
            lane.set_point(self.start.saturating_add(p.at), p.value);
        }
    }

    /// Positions bounded, points sorted and inside the clip, values finite and in range.
    pub fn sanitize(&mut self) {
        bound(&mut self.start);
        self.length = self.length.clamp(1, MAX_POSITION);
        truncate_chars(&mut self.name, MAX_NAME_CHARS);
        let (lo, hi, _) = self.param.range();
        let len = self.length;
        self.points.truncate(MAX_AUTO_CLIP_POINTS);
        self.points.retain(|p| p.value.is_finite() && (0..len).contains(&p.at));
        self.points.iter_mut().for_each(|p| p.value = p.value.clamp(lo, hi));
        self.points.sort_by_key(|p| p.at);
        self.points.dedup_by_key(|p| p.at);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clip(start: Samples, length: Samples) -> AutoClip {
        AutoClip { id: 1, name: "Volume".into(), track: TrackId(1), param: AutoParam::Volume, start, length, points: Vec::new() }
    }

    #[test]
    fn capture_keeps_the_ends_and_the_points_between() {
        let mut lane = AutomationLane::new(AutoParam::Volume);
        lane.set_point(0, 0.0);
        lane.set_point(150, -10.0);
        lane.set_point(1000, -20.0);
        let mut c = clip(100, 100);
        c.capture(Some(&lane), 0.0);
        let at: Vec<Samples> = c.points.iter().map(|p| p.at).collect();
        assert_eq!(at, [0, 50, 99]);
        assert_eq!(c.points[1].value, -10.0);
        // Empty lane: flat at the default.
        let mut flat = clip(0, 10);
        flat.capture(None, -3.0);
        assert!(flat.points.iter().all(|p| p.value == -3.0));
    }

    #[test]
    fn write_puts_the_shape_where_the_clip_sits() {
        let mut c = clip(0, 100);
        c.points = vec![AutomationPoint { at: 0, value: -6.0 }, AutomationPoint { at: 99, value: 0.0 }];
        c.start = 500;
        let mut lane = AutomationLane::new(AutoParam::Volume);
        lane.set_point(550, 12.0);
        c.write(&mut lane);
        assert_eq!(lane.points.len(), 2, "what the clip covers is replaced");
        assert_eq!(lane.value_at(500, 0.0), -6.0);
        assert_eq!(lane.value_at(599, 0.0), 0.0);
    }

    #[test]
    fn sanitize_drops_hostile_points() {
        let mut c = clip(-5, 0);
        c.points = vec![AutomationPoint { at: 0, value: f32::NAN }, AutomationPoint { at: 0, value: 99.0 }, AutomationPoint { at: 7, value: 0.0 }];
        c.sanitize();
        assert_eq!(c.start, 0);
        assert_eq!(c.length, 1);
        assert_eq!(c.points, [AutomationPoint { at: 0, value: 12.0 }]);
    }
}
