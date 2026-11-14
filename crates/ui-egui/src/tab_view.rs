//! Drawing a tab track's notes: as tab (a line per string, the fret on it) and as notation (one
//! staff, written an octave above the sound as guitar and bass parts are). Both draw the same
//! notes at the same x, so the edit window lane, the Tab Editor and the piano roll stay in step.

use crate::theme::{bold, regular};
use egui::{Align2, Color32, Painter, Pos2, Rect, Shape, Stroke, pos2, vec2};
use soundcraft_midi::Sequence;
use soundcraft_midi::tab::{Tuning, place, tech};
use soundcraft_time::TICKS_PER_QUARTER;

/// One note as drawn: its span in x, pitch, and where it is played.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Drawn {
    /// Index in the sequence's notes.
    pub note: usize,
    pub x0: f32,
    pub x1: f32,
    pub pitch: u8,
    pub ticks: i64,
    /// String (1 = highest); 0 when the instrument cannot play the pitch.
    pub string: u8,
    pub fret: u8,
    /// Playing techniques (`soundcraft_midi::tab::tech` bits).
    pub tech: u16,
}

/// Lay `seq` out: each note's x from `x_of(tick)` (ticks in the sequence's own time), only those
/// that reach `[min_x, max_x]`.
pub fn layout(seq: &Sequence, tuning: &Tuning, x_of: impl Fn(i64) -> f32, min_x: f32, max_x: f32) -> Vec<Drawn> {
    let placed = place(seq, tuning);
    seq.notes
        .iter()
        .zip(placed)
        .filter_map(|(n, p)| {
            let x0 = x_of(n.start);
            let x1 = x_of(n.end()).max(x0 + 2.0);
            (x1 >= min_x && x0 <= max_x).then_some(Drawn {
                note: p.note,
                x0,
                x1,
                pitch: n.pitch,
                ticks: n.length,
                string: p.string,
                fret: p.fret,
                tech: p.tech,
            })
        })
        .collect()
}

/// The y of string `string` (1 = top line) in a staff of `count` strings filling `r`.
pub fn string_y(r: Rect, count: usize, string: u8) -> f32 {
    let gap = r.height() / count.max(1) as f32;
    r.min.y + gap * (f32::from(string.max(1)) - 0.5)
}

/// Draw a tab staff in `r`: the string lines, "TAB" at the left, and every note's fret on its
/// string, on a patch of `paper` so the line does not run through the number. `hot` marks a note.
pub fn draw_tab(p: &Painter, r: Rect, tuning: &Tuning, notes: &[Drawn], ink: Color32, paper: Color32, hot: Option<usize>) {
    let count = tuning.strings.len().max(1);
    let line = Stroke::new(1.0, ink.gamma_multiply(0.55));
    for s in 1..=count {
        let y = string_y(r, count, u8::try_from(s).unwrap_or(1));
        p.line_segment([pos2(r.min.x, y), pos2(r.max.x, y)], line);
    }
    let gap = r.height() / count as f32;
    let size = (gap * 0.95).clamp(6.0, 13.0);
    if r.height() > 40.0 {
        let x = r.min.x + 10.0;
        for (i, ch) in ["T", "A", "B"].iter().enumerate() {
            let y = r.center().y + (i as f32 - 1.0) * size;
            p.text(pos2(x, y), Align2::CENTER_CENTER, *ch, bold(size * 0.9), ink.gamma_multiply(0.7));
        }
    }
    for d in notes {
        if d.string == 0 {
            // Out of the instrument's range: shown under the staff.
            p.text(pos2(d.x0, r.max.y - 2.0), Align2::LEFT_BOTTOM, "?", bold(size), Color32::from_rgb(220, 80, 60));
            continue;
        }
        let y = string_y(r, count, d.string);
        let grace = d.tech & tech::GRACE != 0;
        let fs = if grace { size * 0.72 } else { size };
        let label = if d.tech & tech::DEAD != 0 { "x".to_string() } else { d.fret.to_string() };
        let w = fs * 0.62 * label.len() as f32 + 2.0;
        let cell = Rect::from_center_size(pos2(d.x0 + w * 0.5, y), vec2(w, fs));
        p.rect_filled(cell, 2.0, if hot == Some(d.note) { Color32::from_rgb(255, 200, 80) } else { paper });
        p.text(cell.center(), Align2::CENTER_CENTER, label, bold(fs), ink);
        draw_techniques(p, r, notes, d, cell, ink, size);
    }
}

/// The marks for a tab note's techniques around its fret `cell`: P.M. at the top of the staff, h or p
/// over a hammer-on or pull-off, a slash for a slide, ~ for vibrato, > for an accent.
fn draw_techniques(p: &Painter, r: Rect, notes: &[Drawn], d: &Drawn, cell: Rect, ink: Color32, size: f32) {
    let small = regular((size * 0.75).max(6.0));
    let faint = ink.gamma_multiply(0.75);
    if d.tech & tech::PALM_MUTE != 0 {
        p.text(pos2(cell.left(), r.min.y), Align2::LEFT_TOP, "P.M.", small.clone(), faint);
    }
    if d.tech & tech::ACCENT != 0 {
        p.text(pos2(cell.center().x, r.min.y), Align2::CENTER_TOP, ">", small.clone(), faint);
    }
    if d.tech & tech::HOPO_DEST != 0 {
        // Higher than the note before on the string: hammer-on; lower: pull-off.
        let before = notes.iter().filter(|o| o.string == d.string && o.x0 < d.x0).max_by(|a, b| a.x0.total_cmp(&b.x0));
        let label = if before.is_some_and(|b| b.pitch > d.pitch) { "p" } else { "h" };
        p.text(pos2(cell.center().x - cell.width() * 0.5 - 2.0, cell.top()), Align2::RIGHT_BOTTOM, label, small.clone(), faint);
    }
    let stroke = Stroke::new(1.1, faint);
    let (cy, h) = (cell.center().y, cell.height() * 0.35);
    if d.tech & (tech::SLIDE_LEGATO | tech::SLIDE_SHIFT) != 0 {
        let next = notes.iter().filter(|o| o.string == d.string && o.x0 > d.x0).min_by(|a, b| a.x0.total_cmp(&b.x0));
        let (x0, x1) = (cell.right() + 1.0, next.map_or(cell.right() + 10.0, |n| n.x0 - 1.0).max(cell.right() + 4.0));
        let up = next.is_none_or(|n| n.pitch >= d.pitch);
        let (ya, yb) = if up { (cy + h, cy - h) } else { (cy - h, cy + h) };
        p.line_segment([pos2(x0, ya), pos2(x1, yb)], stroke);
    }
    if d.tech & (tech::SLIDE_IN_BELOW | tech::SLIDE_IN_ABOVE) != 0 {
        let (ya, yb) = if d.tech & tech::SLIDE_IN_BELOW != 0 { (cy + h, cy - h) } else { (cy - h, cy + h) };
        p.line_segment([pos2(cell.left() - 9.0, ya), pos2(cell.left() - 1.0, yb)], stroke);
    }
    if d.tech & (tech::SLIDE_OUT_DOWN | tech::SLIDE_OUT_UP) != 0 {
        let (ya, yb) = if d.tech & tech::SLIDE_OUT_UP != 0 { (cy + h, cy - h) } else { (cy - h, cy + h) };
        p.line_segment([pos2(cell.right() + 1.0, ya), pos2(cell.right() + 9.0, yb)], stroke);
    }
    if d.tech & tech::VIBRATO != 0 {
        p.text(pos2(cell.right() + 1.0, cell.top()), Align2::LEFT_BOTTOM, "~", small, faint);
    }
}

/// Diatonic step of a MIDI pitch (C4 = 60 → 0) and whether it needs a sharp.
fn diatonic(pitch: i32) -> (i32, bool) {
    let oct = pitch.div_euclid(12) - 5;
    let (step, sharp) = match pitch.rem_euclid(12) {
        0 => (0, false),
        1 => (0, true),
        2 => (1, false),
        3 => (1, true),
        4 => (2, false),
        5 => (3, false),
        6 => (3, true),
        7 => (4, false),
        8 => (4, true),
        9 => (5, false),
        10 => (5, true),
        _ => (6, false),
    };
    (oct * 7 + step, sharp)
}

/// Whether a tuning reads in bass clef (its lowest string below C2).
pub fn bass_clef(tuning: &Tuning) -> bool {
    tuning.strings.iter().min().is_some_and(|p| *p < 36)
}

/// Draw one five-line staff in `r` with the notes as noteheads (written an octave above the sound,
/// treble clef for guitars, bass clef for basses), stems and flags by length, ledger lines and
/// sharps. `hot` marks a note.
pub fn draw_notation(p: &Painter, r: Rect, bass: bool, notes: &[Drawn], ink: Color32, hot: Option<usize>) {
    // Middle line: B4 (treble, step 6) or D3 (bass, step -6).
    let middle = if bass { -6 } else { 6 };
    let (top, bottom) = (middle + 4, middle - 4);
    // Written an octave above the sound.
    let step_of = |d: &Drawn| diatonic(i32::from(d.pitch) + 12);
    // Fit the staff and every note's ledger lines into `r`, centred on their span.
    let (lo, hi) = notes.iter().map(|d| step_of(d).0).fold((bottom - 2, top + 2), |(lo, hi), st| (lo.min(st - 1), hi.max(st + 1)));
    let step_h = (r.height() / ((hi - lo) as f32 + 2.0)).clamp(1.5, 6.0);
    let mid = (hi + lo) as f32 * 0.5;
    let y_of = |step: i32| r.center().y - (step as f32 - mid) * step_h;
    let stroke = Stroke::new(1.0, ink.gamma_multiply(0.8));
    for k in [-4, -2, 0, 2, 4] {
        let y = y_of(middle + k);
        p.line_segment([pos2(r.min.x, y), pos2(r.max.x, y)], stroke);
    }
    draw_clef(p, pos2(r.min.x + 12.0, y_of(middle)), step_h, bass, ink);
    let head = vec2(step_h * 2.6, step_h * 1.9);
    // Notes that start together are one chord: one stem and one set of flags.
    let mut order: Vec<&Drawn> = notes.iter().collect();
    order.sort_by(|a, b| a.x0.total_cmp(&b.x0));
    let mut i = 0;
    while let Some(first) = order.get(i) {
        let mut j = i + 1;
        while order.get(j).is_some_and(|d| (d.x0 - first.x0).abs() < 0.5) {
            j += 1;
        }
        let chord = order.get(i..j).unwrap_or_default();
        i = j;
        let x = first.x0 + head.x * 0.5 + 1.0;
        let mut steps = Vec::with_capacity(chord.len());
        let mut chord_color = ink;
        for d in chord {
            let (step, sharp) = step_of(d);
            steps.push(step);
            let y = y_of(step);
            let color = if hot == Some(d.note) { Color32::from_rgb(230, 120, 0) } else { ink };
            if color != ink {
                chord_color = color;
            }
            let mut l = top + 2;
            while l <= step {
                p.line_segment([pos2(x - head.x, y_of(l)), pos2(x + head.x, y_of(l))], stroke);
                l += 2;
            }
            let mut l = bottom - 2;
            while l >= step {
                p.line_segment([pos2(x - head.x, y_of(l)), pos2(x + head.x, y_of(l))], stroke);
                l -= 2;
            }
            let q = d.ticks as f32 / TICKS_PER_QUARTER as f32;
            let half = if d.tech & tech::GRACE != 0 { head * 0.33 } else { head * 0.5 };
            if d.tech & tech::DEAD != 0 {
                // A dead note: an x for a head.
                let s = Stroke::new(1.4, color);
                p.line_segment([pos2(x - half.x, y - half.y), pos2(x + half.x, y + half.y)], s);
                p.line_segment([pos2(x - half.x, y + half.y), pos2(x + half.x, y - half.y)], s);
            } else if q >= 1.75 {
                p.add(Shape::ellipse_stroke(pos2(x, y), half, Stroke::new(1.4, color)));
            } else {
                p.add(Shape::ellipse_filled(pos2(x, y), half, color));
            }
            if sharp {
                let cx = x - head.x - 3.0;
                let h = step_h * 1.6;
                for dx in [-1.6, 1.2] {
                    p.line_segment([pos2(cx + dx - 0.6, y - h), pos2(cx + dx + 0.6, y + h)], Stroke::new(1.0, color));
                }
                for dy in [-0.6, 0.6] {
                    p.line_segment([pos2(cx - 3.2, y + dy * h), pos2(cx + 3.2, y + dy * h - 0.8)], Stroke::new(1.0, color));
                }
            }
        }
        let (Some(low), Some(high)) = (steps.iter().min().copied(), steps.iter().max().copied()) else { continue };
        let q = first.ticks as f32 / TICKS_PER_QUARTER as f32;
        if q < 3.5 {
            let up = (low + high) < middle * 2;
            let len = step_h * 7.0;
            // The stem runs from the far notehead through the chord and on past the near one.
            let (sx, sy0, sy1) = if up { (x + head.x * 0.45, y_of(low), y_of(high) - len) } else { (x - head.x * 0.45, y_of(high), y_of(low) + len) };
            p.line_segment([pos2(sx, sy0), pos2(sx, sy1)], Stroke::new(1.1, chord_color));
            let flags = if q <= 0.3 {
                2
            } else if q <= 0.6 {
                1
            } else {
                0
            };
            for f in 0..flags {
                let fy = sy1 + if up { f as f32 * step_h * 1.5 } else { -(f as f32) * step_h * 1.5 };
                p.line_segment(
                    [pos2(sx, fy), pos2(sx + step_h * 2.0, fy + if up { step_h * 2.5 } else { -step_h * 2.5 })],
                    Stroke::new(1.4, chord_color),
                );
            }
        }
    }
}

/// A clef drawn in code: a treble clef as a spiral round the G line with a stem, a bass clef as a
/// curl from the F line and two dots. `at` is the middle line's left end.
fn draw_clef(p: &Painter, at: Pos2, step_h: f32, bass: bool, ink: Color32) {
    let stroke = Stroke::new(1.6, ink);
    if bass {
        // F line: one line above the middle.
        let f = pos2(at.x - 2.0, at.y - step_h * 2.0);
        let curl: Vec<Pos2> = (0..=16)
            .map(|i| {
                let a = -2.8 + i as f32 * 0.2;
                let rad = step_h * (1.2 + i as f32 * 0.09);
                pos2(f.x + a.cos() * rad + step_h, f.y + a.sin() * rad + step_h * 0.8)
            })
            .collect();
        p.circle_filled(pos2(f.x - step_h * 0.4, f.y), step_h * 0.6, ink);
        p.add(Shape::line(curl, stroke));
        p.circle_filled(pos2(f.x + step_h * 3.2, f.y - step_h * 0.9), step_h * 0.35, ink);
        p.circle_filled(pos2(f.x + step_h * 3.2, f.y + step_h * 0.9), step_h * 0.35, ink);
    } else {
        // G line: one line below the middle.
        let g = pos2(at.x, at.y + step_h * 2.0);
        let spiral: Vec<Pos2> = (0..=28)
            .map(|i| {
                let a = i as f32 * 0.38;
                let rad = step_h * (0.6 + i as f32 * 0.07);
                pos2(g.x + a.cos() * rad, g.y - a.sin() * rad)
            })
            .collect();
        p.add(Shape::line(spiral, stroke));
        p.line_segment([pos2(g.x + step_h * 0.4, g.y + step_h * 4.0), pos2(g.x + step_h * 0.4, g.y - step_h * 8.5)], stroke);
        p.circle_filled(pos2(g.x - step_h * 0.4, g.y + step_h * 4.0), step_h * 0.5, ink);
        // The "8" under it: the part sounds an octave lower than written.
        p.text(pos2(g.x, g.y + step_h * 6.0), Align2::CENTER_CENTER, "8", regular(step_h * 2.0), ink);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soundcraft_midi::Note;

    #[test]
    fn notes_lay_out_in_the_same_place_for_every_view() {
        let seq = Sequence {
            notes: vec![
                Note { pitch: 40, velocity: 90, release_velocity: 64, channel: 0, start: 0, length: 960 },
                Note { pitch: 64, velocity: 90, release_velocity: 64, channel: 0, start: 960, length: 480 },
                Note { pitch: 10, velocity: 90, release_velocity: 64, channel: 0, start: 1920, length: 480 },
            ],
            ..Sequence::default()
        };
        let d = layout(&seq, &Tuning::guitar(), |t| t as f32 / 10.0, 0.0, 1000.0);
        assert_eq!(d.len(), 3);
        assert_eq!((d[0].x0, d[0].string, d[0].fret), (0.0, 6, 0));
        assert_eq!((d[1].x0, d[1].string, d[1].fret), (96.0, 1, 0));
        assert_eq!(d[2].string, 0, "too low for a guitar");
        // Only notes that reach the visible span.
        assert_eq!(layout(&seq, &Tuning::guitar(), |t| t as f32 / 10.0, 150.0, 1000.0).len(), 1);
        assert!(!bass_clef(&Tuning::guitar()) && bass_clef(&Tuning::bass()));
        let r = Rect::from_min_size(pos2(0.0, 0.0), vec2(100.0, 60.0));
        assert!(string_y(r, 6, 1) < string_y(r, 6, 6));
    }
}
