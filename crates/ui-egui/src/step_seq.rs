//! The NEW layout's step sequencer: a grid of sounds by sixteenth-note steps over one pattern (a
//! MIDI clip). A click on an empty step sets it, louder the higher in the cell it lands; a click on
//! a lit step clears it, and dragging up or down on a step sets its velocity. Every change is a
//! `pattern.step` command, which also selects the pattern so Space loops it.

use crate::SoundApp;
use crate::theme::{Tokens, bold, regular};
use egui::{Align2, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use serde_json::json;
use soundcraft_engine::cmd::pattern::{DEFAULT_STEPS, SOUNDS, STEP_TICKS, sound_name, steps_in};
use soundcraft_model::{Clip, ClipContent, ClipId, Session, TrackId};

/// Width of the sound names on the left.
pub const NAME_W: f32 = 96.0;
/// Height of the row above the grid: the pattern's name and a hint.
pub const HEADER_H: f32 = 24.0;
/// Row height bounds; rows share the Editor's height between them.
pub const ROW_MIN_H: f32 = 18.0;
pub const ROW_MAX_H: f32 = 26.0;
/// Narrowest step cell before the grid scrolls sideways.
pub const CELL_MIN_W: f32 = 16.0;

/// The pattern the step sequencer edits on `track`: the selected MIDI clip there, else the
/// track's first MIDI clip. None when the track has no MIDI clip yet.
pub fn focused_pattern(s: &Session, track: TrackId) -> Option<ClipId> {
    let t = s.track(track)?;
    let midi = |c: &&Clip| matches!(c.content, ClipContent::Midi { .. });
    t.clips().iter().filter(midi).find(|c| s.edit.selected_clips.contains(&c.id)).or_else(|| t.clips().iter().find(midi)).map(|c| c.id)
}

/// The grid's rows: the built-in sounds, then any other pitch the pattern plays, high to low.
pub fn rows(clip: Option<&Clip>) -> Vec<(u8, String)> {
    let mut rows: Vec<(u8, String)> = SOUNDS.iter().map(|(p, n)| (*p, (*n).to_string())).collect();
    if let Some(ClipContent::Midi { sequence }) = clip.map(|c| &c.content) {
        let mut extra: Vec<u8> = sequence.notes.iter().map(|n| n.pitch).filter(|p| sound_name(*p).is_none()).collect();
        extra.sort_unstable_by(|a, b| b.cmp(a));
        extra.dedup();
        rows.extend(extra.into_iter().map(|p| (p, format!("Note {p}"))));
    }
    rows
}

/// Each row's velocities by step (0 = off).
pub fn grid(clip: Option<&Clip>, rows: &[(u8, String)], steps: usize) -> Vec<Vec<u8>> {
    let mut g = vec![vec![0u8; steps]; rows.len()];
    if let Some(ClipContent::Midi { sequence }) = clip.map(|c| &c.content) {
        for n in &sequence.notes {
            let Some(row) = rows.iter().position(|(p, _)| *p == n.pitch) else { continue };
            let Ok(step) = usize::try_from(n.start.max(0) / STEP_TICKS) else { continue };
            if let Some(cell) = g.get_mut(row).and_then(|r| r.get_mut(step)) {
                *cell = (*cell).max(n.velocity);
            }
        }
    }
    g
}

/// Velocity from where in a cell the pointer is: 1 at the bottom edge, 127 at the top.
pub fn velocity_at(cell: Rect, y: f32) -> i64 {
    let h = cell.height().max(1.0);
    let f = (cell.max.y - y) / h;
    let f = if f.is_finite() { f.clamp(0.0, 1.0) } else { 0.0 };
    (1.0 + f * 126.0).round().clamp(1.0, 127.0) as i64
}

/// Draw the step sequencer for `track` into `ui`.
pub fn show(app: &mut SoundApp, ui: &mut Ui, track: Option<TrackId>) {
    let t = Tokens::current();
    let r = ui.max_rect();
    let s = app.engine.session_arc();
    let Some(track) = track.filter(|id| s.track(*id).is_some_and(|x| x.kind.is_midi())) else {
        ui.painter().text(r.center(), Align2::CENTER_CENTER, "Steps go on a Drums or instrument track.", regular(12.0), t.text_dim);
        return;
    };
    let pattern = focused_pattern(&s, track);
    let clip = pattern.and_then(|id| s.find_clip(id)).map(|(_, c)| c);
    let steps = usize::try_from(clip.map_or(DEFAULT_STEPS, |c| steps_in(&s, c.start, c.length))).unwrap_or(16);
    let rows = rows(clip);
    let grid = grid(clip, &rows, steps);

    let header = Rect::from_min_size(pos2(r.min.x + 10.0, r.min.y), vec2(r.width() - 20.0, HEADER_H));
    let name = clip.map_or_else(|| "New pattern".to_string(), |c| c.name.clone());
    let name_end = ui.painter().text(pos2(header.min.x, header.center().y), Align2::LEFT_CENTER, &name, bold(11.5), t.text).max.x;
    // A fresh pattern goes right after the track's last clip, and the grid turns to it.
    let new_r = Rect::from_min_size(pos2(name_end + 10.0, header.min.y + 3.0), vec2(96.0, HEADER_H - 6.0));
    let new_button = egui::Button::new(egui::RichText::new("+ New pattern").font(regular(11.0)));
    if ui.put(new_r, new_button).on_hover_text("Add an empty one-bar pattern after the last one").clicked() {
        let after = s.track(track).and_then(|x| x.clips().iter().map(Clip::end).max()).unwrap_or(0);
        if let Err(e) = app.run("pattern.new", json!({"track": track.0, "at": after})) {
            app.ui.status = e;
        }
    }
    ui.painter().text(
        pos2(header.max.x, header.center().y),
        Align2::RIGHT_CENTER,
        format!("{steps} steps · click higher for louder · Space plays the loop"),
        regular(11.0),
        t.text_dim,
    );

    // The playing step, while the playhead is inside this pattern.
    let playing = clip.filter(|_| app.is_playing()).and_then(|c| {
        let at = app.position();
        (at >= c.start && at < c.end()).then(|| {
            let ticks = s.tempo.samples_to_ticks(at, s.sample_rate).saturating_sub(s.tempo.samples_to_ticks(c.start, s.sample_rate));
            usize::try_from(ticks / STEP_TICKS).unwrap_or(usize::MAX)
        })
    });

    let body = Rect::from_min_max(pos2(r.min.x + 10.0, header.max.y), pos2(r.max.x - 10.0, r.max.y - 6.0));
    let row_h = (body.height() / rows.len().max(1) as f32).clamp(ROW_MIN_H, ROW_MAX_H);
    let cell_w = ((body.width() - NAME_W) / steps.max(1) as f32).max(CELL_MIN_W);
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(body));
    egui::ScrollArea::both().id_salt("step_seq_scroll").auto_shrink([false, false]).show(&mut child, |ui| {
        let size = vec2(NAME_W + cell_w * steps as f32, row_h * rows.len() as f32);
        let (area, _) = ui.allocate_exact_size(size, Sense::hover());
        for (ri, (pitch, label)) in rows.iter().enumerate() {
            let y0 = area.min.y + row_h * ri as f32;
            let name_r = Rect::from_min_size(pos2(area.min.x, y0), vec2(NAME_W - 6.0, row_h));
            ui.painter().text(pos2(name_r.min.x, name_r.center().y), Align2::LEFT_CENTER, label, regular(11.5), t.text);
            for step in 0..steps {
                let cell = Rect::from_min_size(pos2(area.min.x + NAME_W + cell_w * step as f32, y0), vec2(cell_w, row_h)).shrink(1.5);
                let velocity = grid.get(ri).and_then(|row| row.get(step)).copied().unwrap_or(0);
                draw_cell(ui, cell, step, velocity, playing == Some(step));
                let id = ui.make_persistent_id(("step", *pitch, step));
                let resp = ui.interact(cell, id, Sense::click_and_drag());
                let text = format!("{label} step {}", step + 1);
                resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::CheckBox, true, velocity > 0, &text));
                let pointer = resp.interact_pointer_pos();
                if resp.clicked() {
                    let v = if velocity > 0 { 0 } else { pointer.map_or(100, |p| velocity_at(cell, p.y)) };
                    set_step(app, track, pattern, *pitch, step, v, None);
                } else if resp.dragged()
                    && let Some(p) = pointer
                {
                    set_step(app, track, pattern, *pitch, step, velocity_at(cell, p.y), Some(&format!("step_velocity:{pitch}:{step}")));
                    ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
                }
                if resp.drag_stopped() {
                    app.engine.end_merge();
                }
            }
        }
    });
    if playing.is_some() {
        ui.ctx().request_repaint();
    }
}

fn draw_cell(ui: &Ui, cell: Rect, step: usize, velocity: u8, now: bool) {
    let t = Tokens::current();
    // Every other beat (four steps) is shaded, so the bar reads at a glance.
    let base = if (step / 4).is_multiple_of(2) { t.slot_bg } else { t.panel_bg2 };
    ui.painter().rect(cell, 2.0, base, Stroke::new(1.0, t.border), StrokeKind::Inside);
    if velocity > 0 {
        let h = (cell.height() - 2.0) * f32::from(velocity) / 127.0;
        let fill = Rect::from_min_max(pos2(cell.min.x + 1.0, cell.max.y - 1.0 - h.max(2.0)), pos2(cell.max.x - 1.0, cell.max.y - 1.0));
        ui.painter().rect_filled(fill, 1.5, t.accent);
        ui.painter().rect_stroke(cell, 2.0, Stroke::new(1.0, t.accent), StrokeKind::Inside);
    }
    if now {
        ui.painter().rect_stroke(cell.expand(1.0), 2.0, Stroke::new(1.5, t.playhead), StrokeKind::Outside);
    }
}

/// Set one step through `pattern.step`: on the pattern when there is one, else on the track, which
/// makes the pattern. A drag merges into one undo step under `merge`.
fn set_step(app: &mut SoundApp, track: TrackId, pattern: Option<ClipId>, pitch: u8, step: usize, velocity: i64, merge: Option<&str>) {
    let mut p = json!({"pitch": pitch, "step": step, "velocity": velocity});
    match pattern {
        Some(c) => p["clip"] = json!(c.0),
        None => p["track"] = json!(track.0),
    }
    let r = match merge {
        Some(key) => app.engine.execute_merged("pattern.step", &p, key).map_err(|e| e.to_string()),
        None => app.run("pattern.step", p),
    };
    if let Err(e) = r {
        app.ui.status = e;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Services;
    use soundcraft_engine::Engine;

    fn drums() -> (SoundApp, TrackId) {
        let mut app = SoundApp::new(Engine::default(), None, Services::default());
        app.engine.execute("session.new", &json!({"template": "drums"})).unwrap();
        let t = app.engine.session().tracks[0].id;
        (app, t)
    }

    #[test]
    fn an_empty_drums_track_shows_sixteen_steps_of_the_built_in_sounds() {
        let (app, t) = drums();
        assert_eq!(focused_pattern(app.engine.session(), t), None);
        let rows = rows(None);
        assert_eq!(rows.first().map(|r| r.1.as_str()), Some("Kick"));
        let g = grid(None, &rows, 16);
        assert!(g.iter().all(|r| r.len() == 16 && r.iter().all(|v| *v == 0)));
    }

    #[test]
    fn velocity_follows_click_height() {
        let cell = Rect::from_min_size(pos2(0.0, 0.0), vec2(20.0, 20.0));
        assert_eq!(velocity_at(cell, 20.0), 1);
        assert_eq!(velocity_at(cell, 0.0), 127);
        assert_eq!(velocity_at(cell, 10.0), 64);
        assert_eq!(velocity_at(cell, -50.0), 127);
        assert_eq!(velocity_at(cell, f32::NAN), 1);
    }

    #[test]
    fn a_step_makes_the_pattern_and_shows_in_the_grid() {
        let (mut app, t) = drums();
        set_step(&mut app, t, None, 38, 4, 90, None);
        let s = app.engine.session();
        let clip = focused_pattern(s, t).unwrap();
        let c = s.find_clip(clip).unwrap().1;
        let rows = rows(Some(c));
        let g = grid(Some(c), &rows, 16);
        let snare = rows.iter().position(|r| r.0 == 38).unwrap();
        assert_eq!(g[snare][4], 90);
        assert_eq!(s.edit.selected_clips, [clip], "the stepped pattern is selected, so Space loops it");
    }

    #[test]
    fn other_pitches_get_their_own_rows() {
        let (mut app, t) = drums();
        set_step(&mut app, t, None, 60, 0, 100, None);
        let s = app.engine.session();
        let c = s.find_clip(focused_pattern(s, t).unwrap()).unwrap().1;
        assert_eq!(rows(Some(c)).last().map(|r| r.1.clone()), Some("Note 60".to_string()));
    }
}
