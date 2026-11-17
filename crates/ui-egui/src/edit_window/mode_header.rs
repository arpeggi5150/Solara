//! Track headers of the modes that are not Pro Tools-style: Reaper-style's track control panel,
//! FL-style's playlist track, Ableton-style's arrangement track (right of the clips) and NEW's own.
//! Every control runs the same commands as the Pro Tools-style header and the mixer.

use super::header::header_meter;
use super::tracks::{click_track, in_edit_window, track_context_menu};
use super::{Align2, Color32, Rect, Sense, SoundApp, Stroke, Tokens, Track, TrackId, TrackKind, Ui, bold, json, pos2, regular, rgb, shade, vec2};
use crate::modes::Header;

/// A flat header button: its letter or mark, lit in `on` when set.
fn key(ui: &Ui, r: Rect, track: &Track, label: &str, text: &str, lit: bool, on: Color32) -> egui::Response {
    let t = Tokens::current();
    let name = format!("{label} for {}", track.name);
    let resp = ui.interact(r, ui.scope_id().with((label, track.id.0)), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, ui.is_enabled(), lit, &name));
    let fill = if lit {
        on
    } else if resp.hovered() {
        t.button_hi
    } else {
        t.button
    };
    ui.painter().rect_filled(r, 2.0, fill);
    ui.painter().rect_stroke(r, 2.0, Stroke::new(1.0, t.button_border), egui::StrokeKind::Inside);
    let ink = if lit { t.text_dark } else { t.text };
    if text == "●" {
        ui.painter().circle_filled(r.center(), (r.height() * 0.24).max(2.5), if lit { Color32::WHITE } else { t.rec });
    } else {
        ui.painter().text(r.center(), Align2::CENTER_CENTER, text, bold(10.5), ink);
    }
    resp.on_hover_text(label)
}

/// The track name in a field: a click selects, a double-click renames, a right-click opens the menu.
fn name_field(app: &mut SoundApp, ui: &mut Ui, track: &Track, r: Rect, fill: Color32, ink: Color32) {
    let id = track.id;
    ui.painter().rect_filled(r, 2.0, fill);
    ui.painter().with_clip_rect(r.shrink(1.0)).text(pos2(r.min.x + 4.0, r.center().y), Align2::LEFT_CENTER, &track.name, regular(11.5), ink);
    let resp = ui.interact(r, ui.scope_id().with(("mode_name", id.0)), Sense::click());
    let resp = resp.on_hover_text(if track.tags.is_empty() {
        track.name.clone()
    } else {
        format!("{}\nTags: {}", track.name, crate::tagging::join(&track.tags))
    });
    if resp.double_clicked() {
        app.dialogs.open_rename_track(id, &track.name);
    } else if resp.clicked() {
        click_track(app, id, ui.input(|i| i.modifiers), in_edit_window);
    }
    resp.context_menu(|ui| track_context_menu(app, ui, id));
}

/// A volume knob: up and right turn it up, a double-click resets to 0 dB.
fn volume_knob(app: &mut SoundApp, ui: &mut Ui, track: &Track, r: Rect) {
    let t = Tokens::current();
    let id = track.id;
    let db = track.mixer.volume_db;
    let resp = ui.interact(r, ui.scope_id().with(("mode_vol", id.0)), Sense::click_and_drag());
    resp.widget_info(|| egui::WidgetInfo::slider(ui.is_enabled(), f64::from(db), format!("Volume for {}", track.name)));
    let mut next = None;
    if resp.dragged() {
        let d = resp.drag_delta();
        next = Some((db + (d.x - d.y) * 0.25).clamp(-60.0, 12.0));
    }
    if resp.double_clicked() {
        next = Some(0.0);
    }
    if let Some(v) = next
        && (v - db).abs() > f32::EPSILON
    {
        let _ = app.engine.execute_merged("mix.volume", &json!({"track": id.0, "db": v}), &format!("fader:{}", id.0));
    }
    if resp.drag_stopped() {
        app.engine.end_merge();
    }
    let k = ((db + 60.0) / 72.0).clamp(0.0, 1.0);
    dial(ui, r, k, t.counter_text);
    resp.on_hover_text(format!("Volume {:+.1} dB", db));
}

/// A pan knob: drag to pan, double-click to centre.
fn pan_knob(app: &mut SoundApp, ui: &mut Ui, track: &Track, r: Rect) {
    let t = Tokens::current();
    let id = track.id;
    let Some(&pan) = track.mixer.pan.first() else { return };
    let resp = ui.interact(r, ui.scope_id().with(("mode_pan", id.0)), Sense::click_and_drag());
    resp.widget_info(|| egui::WidgetInfo::slider(ui.is_enabled(), f64::from(pan), format!("Pan for {}", track.name)));
    let mut next = None;
    if resp.dragged() {
        let d = resp.drag_delta();
        next = Some((pan + (d.x - d.y) * 0.01).clamp(-1.0, 1.0));
    }
    if resp.double_clicked() {
        next = Some(0.0);
    }
    if let Some(v) = next
        && (v - pan).abs() > f32::EPSILON
    {
        let _ = app.engine.execute_merged("mix.pan", &json!({"track": id.0, "pan": v, "index": 0}), &format!("pan:{}:0", id.0));
    }
    if resp.drag_stopped() {
        app.engine.end_merge();
    }
    dial(ui, r, pan * 0.5 + 0.5, t.text);
    resp.on_hover_text(format!("Pan {}", crate::widgets::pan_text(pan)));
}

/// A small dial at `k` (0..1) of its sweep.
fn dial(ui: &Ui, r: Rect, k: f32, color: Color32) {
    let t = Tokens::current();
    let p = ui.painter();
    let c = r.center();
    let rad = r.width().min(r.height()) * 0.42;
    let start = std::f32::consts::PI * 0.75;
    let at = start + std::f32::consts::PI * 1.5 * k.clamp(0.0, 1.0);
    p.circle_filled(c, rad, t.knob_bg);
    p.circle_stroke(c, rad, Stroke::new(1.0, t.knob_ring));
    p.line_segment([c, c + vec2(at.cos(), at.sin()) * rad], Stroke::new(1.6, color));
}

/// The header background: a click selects the track, a right-click opens the mode's track menu.
fn background(app: &mut SoundApp, ui: &mut Ui, track: &Track, head: Rect, fill: Color32) {
    let id = track.id;
    ui.painter().rect_filled(head, 0.0, fill);
    let bg = ui.interact(head, ui.scope_id().with(("hdr", id.0)), Sense::click());
    if bg.clicked() {
        click_track(app, id, ui.input(|i| i.modifiers), in_edit_window);
    }
    bg.context_menu(|ui| track_context_menu(app, ui, id));
}

fn toggle(app: &mut SoundApp, cmd: &str, id: TrackId) {
    if let Err(e) = app.run(cmd, json!({"track": id.0})) {
        app.ui.status = e;
    }
}

/// Draw `track`'s header in `style`.
pub(super) fn draw(app: &mut SoundApp, ui: &mut Ui, track: &Track, head: Rect, selected: bool, head_bg: Color32, style: Header) {
    match style {
        Header::Reaper => reaper(app, ui, track, head, selected, head_bg),
        Header::Fl => fl(app, ui, track, head, selected, head_bg),
        Header::Ableton => ableton(app, ui, track, head, selected, head_bg),
        Header::New | Header::ProTools => new(app, ui, track, head, selected, head_bg),
    }
}

/// The track control panel: number, record arm, name, M and S on the first row; FX, routing, volume and
/// pan below; the meter at the right.
fn reaper(app: &mut SoundApp, ui: &mut Ui, track: &Track, head: Rect, selected: bool, head_bg: Color32) {
    let t = Tokens::current();
    let id = track.id;
    background(app, ui, track, head, head_bg);
    let m = &track.mixer;
    let n = app.engine.session().track_index(id).map_or(0, |i| i + 1);
    let at = |x: f32, y: f32, w: f32, h: f32| Rect::from_min_size(pos2(head.min.x + x, head.min.y + y), vec2(w, h));
    let num = at(3.0, 3.0, 22.0, (head.height() - 6.0).clamp(14.0, 40.0));
    ui.painter().rect_filled(num, 2.0, rgb(track.color));
    ui.painter().text(pos2(num.center().x, num.min.y + 9.0), Align2::CENTER_CENTER, n.to_string(), bold(10.5), shade(rgb(track.color), -0.75));
    let master = track.kind == TrackKind::Master;
    let mut x = 29.0;
    if track.kind.has_playlist() {
        if key(ui, at(x, 3.0, 18.0, 18.0), track, "Record arm", "●", m.record_arm, t.rec).clicked() {
            toggle(app, "mix.record_arm", id);
        }
        x += 21.0;
    }
    let right = head.width() - 12.0;
    let name_w = (right - x - 46.0).max(30.0);
    let field = if selected { t.name_field_sel } else { shade(t.panel_bg, -0.25) };
    let ink = if selected { t.text_dark } else { t.text };
    name_field(app, ui, track, at(x, 3.0, name_w, 18.0), field, ink);
    if !master {
        if key(ui, at(right - 42.0, 3.0, 20.0, 18.0), track, "Mute", "M", m.mute, t.mute).clicked() {
            toggle(app, "mix.mute", id);
        }
        if key(ui, at(right - 20.0, 3.0, 20.0, 18.0), track, "Solo", "S", m.solo, t.solo).clicked() {
            toggle(app, "mix.solo", id);
        }
    }
    if head.height() >= 44.0 {
        let y = 25.0;
        volume_knob(app, ui, track, at(29.0, y, 18.0, 18.0));
        pan_knob(app, ui, track, at(51.0, y, 18.0, 18.0));
        let fx_on = m.inserts.iter().any(Option::is_some);
        let fx = key(ui, at(right - 64.0, y, 28.0, 18.0), track, "Track FX", "FX", fx_on, t.mode_on);
        let free = m.inserts.iter().position(Option::is_none);
        if let Some(slot) = free {
            egui::Popup::menu(&fx).show(|ui| crate::mix_window::plugin_menu(app, ui, id, slot, false));
        }
        if key(ui, at(right - 32.0, y, 32.0, 18.0), track, "Track routing", "Route", false, t.mode_on).clicked() {
            let _ = app.run("window.routing_matrix", json!({"value": true}));
        }
        ui.painter().text(
            pos2(head.min.x + 74.0, head.min.y + y + 9.0),
            Align2::LEFT_CENTER,
            format!("{:+.1}", m.volume_db),
            regular(10.0),
            t.text_dim,
        );
    }
    let md = app.meters.get(&id).copied().unwrap_or_default();
    header_meter(
        ui,
        Rect::from_min_max(pos2(head.max.x - 9.0, head.min.y + 2.0), pos2(head.max.x - 2.0, head.max.y - 2.0)),
        &md,
        track.channels() >= 2,
    );
}

/// A playlist track: a colour bar, the name, and the light that mutes it (Ctrl-click solos).
fn fl(app: &mut SoundApp, ui: &mut Ui, track: &Track, head: Rect, selected: bool, head_bg: Color32) {
    let t = Tokens::current();
    let id = track.id;
    background(app, ui, track, head, head_bg);
    ui.painter().rect_filled(Rect::from_min_size(head.min, vec2(5.0, head.height())), 0.0, rgb(track.color));
    let ink = if selected { Color32::WHITE } else { t.text };
    let name = Rect::from_min_max(pos2(head.min.x + 9.0, head.min.y + 3.0), pos2(head.max.x - 26.0, head.min.y + 21.0));
    name_field(app, ui, track, name, if selected { shade(t.accent_dark, -0.1) } else { shade(head_bg, -0.12) }, ink);
    if track.kind != TrackKind::Master {
        let led = Rect::from_center_size(pos2(head.max.x - 13.0, head.min.y + 12.0), vec2(14.0, 14.0));
        let resp = ui.interact(led, ui.scope_id().with(("fl_led", id.0)), Sense::click());
        let on = !track.mixer.mute;
        resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::CheckBox, ui.is_enabled(), on, format!("Track on for {}", track.name)));
        ui.painter().circle_filled(led.center(), 5.5, if on { t.mode_on } else { shade(t.mode_on, -0.75) });
        ui.painter().circle_stroke(led.center(), 5.5, Stroke::new(1.0, t.button_border));
        if track.mixer.solo {
            ui.painter().circle_stroke(led.center(), 7.0, Stroke::new(1.2, t.solo));
        }
        let resp = resp.on_hover_text("Mute (Ctrl-click: solo)");
        if resp.clicked() {
            let cmd = if ui.input(|i| i.modifiers.command) { "mix.solo" } else { "mix.mute" };
            toggle(app, cmd, id);
        }
    }
    if head.height() >= 44.0 && track.kind.has_playlist() {
        let arm = Rect::from_min_size(pos2(head.min.x + 9.0, head.min.y + 25.0), vec2(18.0, 16.0));
        if key(ui, arm, track, "Record arm", "●", track.mixer.record_arm, t.rec).clicked() {
            toggle(app, "mix.record_arm", id);
        }
        let kind = match track.kind {
            TrackKind::Audio => "Audio",
            TrackKind::Tab => "Tab",
            _ => "Pattern",
        };
        ui.painter().text(pos2(arm.max.x + 6.0, arm.center().y), Align2::LEFT_CENTER, kind, regular(10.0), t.text_dim);
    }
}

/// The arrangement track, right of the clips: the name bar in the track's colour, then the track
/// activator (its number; off mutes), solo and arm, then volume and pan.
fn ableton(app: &mut SoundApp, ui: &mut Ui, track: &Track, head: Rect, selected: bool, head_bg: Color32) {
    let t = Tokens::current();
    let id = track.id;
    background(app, ui, track, head, head_bg);
    ui.painter().vline(head.min.x + 0.5, head.y_range(), Stroke::new(1.0, t.border));
    let bar = Rect::from_min_max(pos2(head.min.x + 1.0, head.min.y + 1.0), pos2(head.max.x - 10.0, head.min.y + 18.0));
    let color = rgb(track.color);
    let fill = if selected { shade(color, 0.25) } else { color };
    name_field(app, ui, track, bar, fill, Color32::from_rgb(16, 16, 16));
    let m = &track.mixer;
    let n = app.engine.session().track_index(id).map_or(0, |i| i + 1);
    let at = |x: f32, y: f32, w: f32, h: f32| Rect::from_min_size(pos2(head.min.x + x, head.min.y + y), vec2(w, h));
    if head.height() >= 38.0 && track.kind != TrackKind::Master {
        let y = 21.0;
        if key(ui, at(4.0, y, 22.0, 15.0), track, "Track activator", &n.to_string(), !m.mute, t.mode_on).clicked() {
            toggle(app, "mix.mute", id);
        }
        if key(ui, at(29.0, y, 18.0, 15.0), track, "Solo", "S", m.solo, Color32::from_rgb(90, 160, 230)).clicked() {
            toggle(app, "mix.solo", id);
        }
        if track.kind.has_playlist() && key(ui, at(50.0, y, 18.0, 15.0), track, "Arm", "●", m.record_arm, t.rec).clicked() {
            toggle(app, "mix.record_arm", id);
        }
        let vol = at(74.0, y, 56.0, 15.0);
        ui.painter().rect_filled(vol, 2.0, t.counter_bg);
        ui.painter().text(vol.center(), Align2::CENTER_CENTER, format!("{:.2}", m.volume_db), regular(10.0), t.counter_text);
        let resp = ui.interact(vol, ui.scope_id().with(("abl_vol", id.0)), Sense::click_and_drag());
        resp.widget_info(|| egui::WidgetInfo::slider(ui.is_enabled(), f64::from(m.volume_db), format!("Volume for {}", track.name)));
        if resp.dragged() {
            let v = (m.volume_db - resp.drag_delta().y * 0.2).clamp(-60.0, 12.0);
            let _ = app.engine.execute_merged("mix.volume", &json!({"track": id.0, "db": v}), &format!("fader:{}", id.0));
        }
        if resp.drag_stopped() {
            app.engine.end_merge();
        }
        if resp.double_clicked() {
            let _ = app.run("mix.volume", json!({"track": id.0, "db": 0.0}));
        }
        pan_knob(app, ui, track, at(134.0, y - 1.0, 18.0, 18.0));
    }
    let md = app.meters.get(&id).copied().unwrap_or_default();
    header_meter(
        ui,
        Rect::from_min_max(pos2(head.max.x - 8.0, head.min.y + 2.0), pos2(head.max.x - 2.0, head.max.y - 2.0)),
        &md,
        track.channels() >= 2,
    );
}

/// NEW: a colour dot and the name, then mute, solo and record, and the volume knob.
fn new(app: &mut SoundApp, ui: &mut Ui, track: &Track, head: Rect, selected: bool, head_bg: Color32) {
    let t = Tokens::current();
    let id = track.id;
    background(app, ui, track, head, head_bg);
    ui.painter().circle_filled(pos2(head.min.x + 11.0, head.min.y + 12.0), 5.0, rgb(track.color));
    let name = Rect::from_min_max(pos2(head.min.x + 20.0, head.min.y + 3.0), pos2(head.max.x - 12.0, head.min.y + 21.0));
    name_field(app, ui, track, name, if selected { t.accent_dark } else { head_bg }, if selected { Color32::WHITE } else { t.text });
    let m = &track.mixer;
    if head.height() >= 40.0 && track.kind != TrackKind::Master {
        let at = |x: f32| Rect::from_min_size(pos2(head.min.x + x, head.min.y + 24.0), vec2(20.0, 16.0));
        if key(ui, at(20.0), track, "Mute", "M", m.mute, t.mute).clicked() {
            toggle(app, "mix.mute", id);
        }
        if key(ui, at(43.0), track, "Solo", "S", m.solo, t.solo).clicked() {
            toggle(app, "mix.solo", id);
        }
        if track.kind.has_playlist() && key(ui, at(66.0), track, "Record", "●", m.record_arm, t.rec).clicked() {
            toggle(app, "mix.record_arm", id);
        }
        volume_knob(app, ui, track, Rect::from_min_size(pos2(head.min.x + 92.0, head.min.y + 23.0), vec2(18.0, 18.0)));
        ui.painter().text(
            pos2(head.min.x + 114.0, head.min.y + 32.0),
            Align2::LEFT_CENTER,
            format!("{:+.1} dB", m.volume_db),
            regular(10.0),
            t.text_dim,
        );
    }
    let md = app.meters.get(&id).copied().unwrap_or_default();
    header_meter(
        ui,
        Rect::from_min_max(pos2(head.max.x - 8.0, head.min.y + 2.0), pos2(head.max.x - 2.0, head.max.y - 2.0)),
        &md,
        track.channels() >= 2,
    );
}
