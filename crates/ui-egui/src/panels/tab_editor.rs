//! Tab Editor (Window › Tab Editor): the selected tab clip as notation, tab and piano roll, one
//! above the other on one time axis. The three are drawn from the same notes, so an edit in any
//! editor (MIDI Editor, Event List, here) shows in all of them. The buttons choose what the track
//! shows (tab, notation or both, as in the edit window lane); right-click a fret number to play
//! the note on another string.

use crate::SoundApp;
use crate::tab_view::{Drawn, bass_clef, draw_notation, draw_tab, layout, string_y};
use crate::theme::{bold, regular};
use egui::{Align2, Color32, Rect, Sense, Stroke, pos2, vec2};
use serde_json::json;
use soundcraft_model::{ClipContent, TabView, TrackKind};

const PX_PER_TICK: f32 = 0.09;
const LEFT: f32 = 46.0;

/// The tab clip the editor shows: the selected clip on a tab track, else the first clip of a
/// selected tab track.
fn target(app: &SoundApp) -> Option<soundcraft_model::ClipId> {
    let s = app.engine.session();
    let on_tab = |id| s.find_clip(id).and_then(|(t, _)| s.track(t)).is_some_and(|t| t.kind == TrackKind::Tab);
    crate::midi_editor::target_clip(app).filter(|c| on_tab(*c)).or_else(|| {
        s.tracks
            .iter()
            .filter(|t| t.kind == TrackKind::Tab && s.edit.selected_tracks.contains(&t.id))
            .flat_map(|t| t.clips().iter())
            .map(|c| c.id)
            .next()
    })
}

pub fn show(app: &mut SoundApp, ctx: &egui::Context) {
    let mut open = app.ui.show_tab_editor;
    if !open {
        return;
    }
    let mut action: Option<(&'static str, serde_json::Value)> = None;
    egui::Window::new("Tab Editor").open(&mut open).default_size(vec2(960.0, 420.0)).show(ctx, |ui| {
        let Some(cid) = target(app) else {
            ui.label("Select a clip on a tab track (Track › New › Tab Track, or open a TabGarden file).");
            return;
        };
        let s = app.engine.session();
        let Some((tid, clip)) = s.find_clip(cid).map(|(t, c)| (t, c.clone())) else { return };
        let Some(track) = s.track(tid) else { return };
        let Some(setup) = track.tab_setup() else { return };
        let ClipContent::Midi { sequence } = &clip.content else { return };
        let tuning = setup.tuning.sanitized();
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(format!("{} — {}", track.name, clip.name)).font(bold(13.0)));
            craft_ui::ComboBox::from_id_salt("tab_tuning").selected_text(format!("Tuning {}", tuning.label())).show_ui(ui, |ui| {
                for (id, label) in [("guitar", "Guitar (E A D G B E)"), ("drop_d", "Drop D"), ("seven", "7-string"), ("bass", "Bass (E A D G)")] {
                    if ui.button(label).clicked() {
                        action = Some(("track.tab_tuning", json!({"track": tid.0, "preset": id})));
                    }
                }
            });
            let mut capo = i64::from(tuning.capo);
            ui.label(egui::RichText::new("Capo").font(regular(11.5)));
            if ui.add(egui::DragValue::new(&mut capo).range(0..=i64::from(soundcraft_midi::tab::MAX_FRET))).changed() {
                action = Some(("track.tab_tuning", json!({"track": tid.0, "capo": capo})));
            }
            ui.separator();
            for v in TabView::ALL {
                if ui.selectable_label(setup.view == v, v.label()).clicked() && setup.view != v {
                    action = Some(("track.tab_view", json!({"track": tid.0, "view": v.id()})));
                }
            }
        });
        let len_ticks = (s.tempo.samples_to_ticks(clip.end(), s.sample_rate) - s.tempo.samples_to_ticks(clip.start, s.sample_rate)).max(1);
        let width = (len_ticks as f32 * PX_PER_TICK + LEFT + 40.0).max(ui.available_width());
        let base = s.tempo.samples_to_ticks(clip.start, s.sample_rate);
        egui::ScrollArea::horizontal().show(ui, |ui| {
            let notation_h = if setup.view.shows_notation() { 110.0 } else { 0.0 };
            let tab_h = if setup.view.shows_tab() { 18.0 * tuning.strings.len() as f32 + 8.0 } else { 0.0 };
            let roll_h = 120.0;
            let (r, resp) = ui.allocate_exact_size(vec2(width, notation_h + tab_h + roll_h + 24.0), Sense::click());
            let p = ui.painter_at(r);
            let paper = Color32::from_rgb(246, 243, 236);
            let ink = Color32::from_rgb(20, 20, 24);
            p.rect_filled(r, 4.0, paper);
            let x_of = |t: i64| r.min.x + LEFT + t as f32 * PX_PER_TICK;
            let drawn = layout(sequence, &tuning, x_of, r.min.x, r.max.x);
            // Bar lines and numbers over all three views.
            let mut bar = s.tempo.bar_beat_at_tick(base).bar.max(1);
            let mut guard = 0;
            loop {
                let t = s.tempo.bar_start_tick(bar) - base;
                if t > len_ticks || guard > 4000 {
                    break;
                }
                if t >= 0 {
                    let x = x_of(t) - 3.0;
                    p.line_segment([pos2(x, r.min.y + 14.0), pos2(x, r.max.y - 4.0)], Stroke::new(1.0, ink.gamma_multiply(0.35)));
                    p.text(pos2(x + 2.0, r.min.y + 2.0), Align2::LEFT_TOP, bar.to_string(), regular(9.0), Color32::from_rgb(120, 110, 100));
                }
                bar += 1;
                guard += 1;
            }
            let mut y = r.min.y + 18.0;
            if notation_h > 0.0 {
                let nr = Rect::from_min_size(pos2(r.min.x + 4.0, y), vec2(width - 8.0, notation_h));
                draw_notation(&p, nr, bass_clef(&tuning), &drawn, ink, None);
                y += notation_h;
            }
            let tab_rect = Rect::from_min_size(pos2(r.min.x + 4.0, y), vec2(width - 8.0, tab_h.max(1.0)));
            if tab_h > 0.0 {
                draw_tab(&p, tab_rect, &tuning, &drawn, ink, paper, None);
                y += tab_h;
            }
            let roll = Rect::from_min_size(pos2(r.min.x + 4.0, y + 4.0), vec2(width - 8.0, roll_h));
            draw_roll(&p, roll, &drawn, track.color);
            // Right-click a fret number: play the note on another string.
            if tab_h > 0.0
                && let Some(pos) = resp.interact_pointer_pos()
                && tab_rect.contains(pos)
            {
                let count = tuning.strings.len();
                let hit = drawn.iter().filter(|d| d.string > 0).min_by(|a, b| {
                    let da = (a.x0 - pos.x).abs() + (string_y(tab_rect, count, a.string) - pos.y).abs();
                    let db = (b.x0 - pos.x).abs() + (string_y(tab_rect, count, b.string) - pos.y).abs();
                    da.total_cmp(&db)
                });
                if let Some(h) = hit {
                    app.ui.tab_editor_note = Some((cid, h.note));
                }
            }
            let picked = app.ui.tab_editor_note.filter(|(c, _)| *c == cid).and_then(|(_, n)| drawn.iter().find(|d| d.note == n).copied());
            resp.context_menu(|ui| {
                let Some(d) = picked else {
                    ui.label("Right-click a fret number");
                    return;
                };
                ui.label(format!("Play {} on", soundcraft_midi::note_name(d.pitch)));
                for st in 1..=u8::try_from(tuning.strings.len()).unwrap_or(0) {
                    let Some(fret) = tuning.fret(st, d.pitch) else { continue };
                    let label = format!("String {st} (fret {fret}){}", if st == d.string { "  ✓" } else { "" });
                    if ui.button(label).clicked() {
                        action = Some(("event.tab_string", json!({"clip": cid.0, "note": d.note, "string": st})));
                        ui.close();
                    }
                }
            });
        });
        ui.label(egui::RichText::new("Notation, tab and piano roll show the same notes. Edit notes in the MIDI Editor; right-click a fret number to move it to another string.").small());
    });
    app.ui.show_tab_editor = open;
    if let Some((id, params)) = action
        && let Err(e) = app.run(id, params)
    {
        app.ui.status = e;
    }
}

/// A piano-roll strip: each note a bar at its pitch, the same x as the tab and notation.
fn draw_roll(p: &egui::Painter, r: Rect, drawn: &[Drawn], color: [u8; 3]) {
    p.rect_filled(r, 2.0, Color32::from_rgb(34, 36, 40));
    let (lo, hi) = drawn.iter().fold((127u8, 0u8), |(lo, hi), d| (lo.min(d.pitch), hi.max(d.pitch)));
    let (lo, hi) = if lo > hi { (40, 76) } else { (lo.saturating_sub(2), hi.saturating_add(2)) };
    let span = f32::from(hi.saturating_sub(lo).max(1));
    let row = (r.height() / span).clamp(1.5, 8.0);
    for c in (lo..=hi).filter(|p| p % 12 == 0) {
        let y = r.max.y - f32::from(c - lo) / span * r.height();
        p.line_segment([pos2(r.min.x, y), pos2(r.max.x, y)], Stroke::new(1.0, Color32::from_rgb(60, 62, 68)));
        p.text(pos2(r.min.x + 2.0, y), Align2::LEFT_BOTTOM, soundcraft_midi::note_name(c), regular(8.5), Color32::from_rgb(150, 150, 160));
    }
    let fill = Color32::from_rgb(color[0], color[1], color[2]);
    for d in drawn {
        let y = r.max.y - f32::from(d.pitch.saturating_sub(lo)) / span * r.height();
        p.rect_filled(Rect::from_min_max(pos2(d.x0, y - row * 0.5), pos2(d.x1, y + row * 0.5)), 1.0, fill);
    }
}
