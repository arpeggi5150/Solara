//! The Reaper-style layout preset: the transport bar on top, the track list (track control panel)
//! on the left of the arrange view in the centre, and the mixer (mixer control panel) along the
//! bottom. Same engine, session and commands as NEW and Classic; only the panels, the words and the
//! keys change (the keys live in `keymap`). Learnt from how such a DAW behaves, not from its code
//! or artwork.

use crate::SoundApp;
use crate::new_layout::{LayoutPreset, MIXER_COLLAPSED_H, MIXER_FIT_H, TOP_BAR_H};
use crate::theme::{Tokens, bold, mono, regular};
use crate::widgets::icon_button;
use egui::{Align2, Rect, Sense, Stroke, Ui, pos2, vec2};
use serde_json::json;
use soundcraft_time::TimeFormat;

/// The arrange view keeps at least this much height above the open mixer.
pub const ARRANGE_MIN_H: f32 = 120.0;

/// The open mixer's height when `room` is left below the transport bar.
pub fn open_mixer_height(room: f32) -> f32 {
    MIXER_FIT_H.min(room - ARRANGE_MIN_H).max(MIXER_COLLAPSED_H + 120.0)
}

/// Draw the Reaper-style layout.
pub fn show(app: &mut SoundApp, ui: &mut Ui) {
    let t = Tokens::current();
    crate::edit_window::drive_drag(app, ui.ctx());
    egui::Panel::top("reaper_transport").exact_size(TOP_BAR_H).frame(egui::Frame::NONE.fill(t.toolbar_bg)).show(ui, |ui| transport(app, ui));
    let room = ui.available_rect_before_wrap().height();
    // Open, the mixer fits a whole strip, fader included, when the window allows it, and the
    // arrange view keeps the rest (at least `ARRANGE_MIN_H`).
    let mixer_h = if app.ui.reaper_mixer_open { open_mixer_height(room) } else { MIXER_COLLAPSED_H };
    egui::Panel::bottom("reaper_mixer").exact_size(mixer_h).frame(egui::Frame::NONE.fill(t.panel_bg)).show(ui, |ui| mixer(app, ui));
    egui::CentralPanel::default().frame(egui::Frame::NONE.fill(t.window_bg)).show(ui, |ui| {
        let full = ui.max_rect();
        let mut arrange = ui.new_child(egui::UiBuilder::new().max_rect(full));
        arrange.set_clip_rect(full.intersect(ui.clip_rect()));
        crate::edit_window::main_area(app, &mut arrange);
    });
}

/// The transport bar: position, play controls, tempo and time signature, the layout switcher, and
/// the Actions and Routing buttons.
fn transport(app: &mut SoundApp, ui: &mut Ui) {
    let t = Tokens::current();
    let r = ui.max_rect();
    ui.painter().line_segment([pos2(r.min.x, r.max.y - 0.5), pos2(r.max.x, r.max.y - 0.5)], Stroke::new(1.0, t.border));
    let mut row = ui.new_child(egui::UiBuilder::new().max_rect(r.shrink2(vec2(10.0, 6.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
    let ui = &mut row;
    ui.spacing_mut().item_spacing.x = 6.0;
    let (bars, time, bpm, meter, looped, click, playing, recording) = {
        let s = app.engine.session();
        let at = app.position();
        let tick = s.tempo.samples_to_ticks(at, s.sample_rate);
        let m = s.tempo.meter_at_tick(tick);
        (
            s.format_position(at, TimeFormat::BarsBeats),
            s.format_position(at, TimeFormat::MinSecs),
            s.tempo.tempo_at_tick(tick),
            format!("{}/{}", m.numerator, m.denominator),
            s.edit.loop_playback,
            s.edit.click,
            app.is_playing(),
            app.engine.transport.recording,
        )
    };
    let size = vec2(30.0, 28.0);
    let keys = |id: &str| crate::keymap::chord_for(app, id).map(|c| crate::keymap::display(&c, ui.ctx().os().is_mac()));
    let tip = |label: &str, key: Option<String>| key.map_or_else(|| label.to_string(), |k| format!("{label} ({k})"));
    let tips = [
        tip("Go to start", keys("transport.rtz")),
        tip("Play", Some("Space".into())),
        tip("Stop", Some("Space".into())),
        tip("Record", keys("transport.record")),
        tip("Repeat", keys("options.loop_playback")),
        tip("Go to end", keys("transport.go_to_end")),
        tip("Metronome", None),
    ];
    let [rtz_tip, play_tip, stop_tip, rec_tip, loop_tip, end_tip, click_tip] = tips;
    if icon_button(ui, size, "rtz", false, &rtz_tip).clicked() {
        let _ = app.run("transport.rtz", json!({}));
    }
    if icon_button(ui, size, "play", playing, &play_tip).clicked() {
        let _ = app.run("transport.play", json!({}));
    }
    if icon_button(ui, size, "stop", false, &stop_tip).clicked() {
        let _ = app.run("transport.stop", json!({}));
    }
    if icon_button(ui, size, "record", recording, &rec_tip).clicked() {
        let _ = app.run("transport.record", json!({}));
    }
    if icon_button(ui, size, "loop", looped, &loop_tip).clicked() {
        let _ = app.run("options.loop_playback", json!({}));
    }
    if icon_button(ui, size, "end", false, &end_tip).clicked() {
        let _ = app.run("transport.go_to_end", json!({}));
    }
    if icon_button(ui, size, "click", click, &click_tip).clicked() {
        let _ = app.run("options.click", json!({}));
    }
    ui.add_space(8.0);
    readout(ui, "Position", |ui| {
        ui.label(egui::RichText::new(format!("{bars} / {time}")).font(mono(13.0)).color(t.counter_text));
    });
    ui.add_space(6.0);
    readout(ui, "BPM", |ui| crate::toolbar::tempo_field(app, ui, bpm));
    readout(ui, "Time sig", |ui| {
        ui.label(egui::RichText::new(meter).font(mono(12.0)).color(t.counter_text));
    });
    ui.add_space(8.0);
    crate::new_layout::switcher(app, ui);
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let mac = ui.ctx().os().is_mac();
        for (label, id) in [("Routing", "window.routing_matrix"), ("Actions", "window.action_list")] {
            let key = crate::keymap::chord_for(app, id).map(|c| crate::keymap::display(&c, mac));
            let tip = match (id, key) {
                ("window.routing_matrix", Some(k)) => format!("Routing Matrix ({k})"),
                ("window.routing_matrix", None) => "Routing Matrix".to_string(),
                (_, Some(k)) => format!("Action List ({k})"),
                (_, None) => "Action List".to_string(),
            };
            if ui.add(egui::Button::new(egui::RichText::new(label).font(regular(12.0)))).on_hover_text(tip).clicked() {
                let _ = app.run(id, json!({"value": true}));
            }
        }
    });
}

/// A label above a value, in the transport bar.
fn readout(ui: &mut Ui, label: &str, value: impl FnOnce(&mut Ui)) {
    let t = Tokens::current();
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        ui.label(egui::RichText::new(label).font(regular(9.5)).color(t.counter_label));
        value(ui);
    });
}

/// The mixer along the bottom: a header row that shows or hides it (Ctrl+M), and the strips.
fn mixer(app: &mut SoundApp, ui: &mut Ui) {
    let t = Tokens::current();
    let r = ui.max_rect();
    ui.painter().line_segment([pos2(r.min.x, r.min.y + 0.5), pos2(r.max.x, r.min.y + 0.5)], Stroke::new(1.0, t.border));
    let header = Rect::from_min_size(r.min, vec2(r.width(), MIXER_COLLAPSED_H));
    let resp = ui.interact(header, ui.scope_id().with("reaper_mixer_header"), Sense::click());
    let open = app.ui.reaper_mixer_open;
    let c = pos2(header.min.x + 15.0, header.center().y);
    let points = if open {
        vec![pos2(c.x - 4.0, c.y - 2.5), pos2(c.x + 4.0, c.y - 2.5), pos2(c.x, c.y + 3.0)]
    } else {
        vec![pos2(c.x - 2.5, c.y - 4.0), pos2(c.x + 3.0, c.y), pos2(c.x - 2.5, c.y + 4.0)]
    };
    ui.painter().add(egui::Shape::convex_polygon(points, t.text_dim, Stroke::NONE));
    ui.painter().text(pos2(header.min.x + 26.0, header.center().y), Align2::LEFT_CENTER, "MIXER", bold(11.0), t.text_dim);
    let mac = ui.ctx().os().is_mac();
    let key = crate::keymap::chord_for(app, "ui.mixer").map(|c| crate::keymap::display(&c, mac));
    if let Some(k) = &key {
        ui.painter().text(pos2(header.max.x - 10.0, header.center().y), Align2::RIGHT_CENTER, k, mono(11.0), t.text_dim);
    }
    let tip = key.map_or_else(|| "Show or hide the mixer".to_string(), |k| format!("Show or hide the mixer ({k})"));
    if resp.on_hover_text(tip).clicked() {
        let _ = app.run("ui.mixer", json!({}));
    }
    if open {
        let body = Rect::from_min_max(pos2(r.min.x, header.max.y), r.max);
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(body));
        child.set_clip_rect(body.intersect(ui.clip_rect()));
        child.painter().rect_filled(body, 0.0, t.window_bg);
        egui::ScrollArea::vertical()
            .id_salt("reaper_mixer_scroll")
            .auto_shrink([false, false])
            .show(&mut child, |ui| crate::mix_window::content(app, ui));
    }
}

/// Whether the Reaper-style layout is showing.
pub fn active(app: &SoundApp) -> bool {
    app.ui.layout == LayoutPreset::Reaper
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Services;
    use soundcraft_engine::Engine;

    fn app() -> SoundApp {
        let mut app = SoundApp::new(Engine::default(), None, Services::default());
        app.ui.layout = LayoutPreset::New;
        app
    }

    #[test]
    fn the_reaper_layout_is_selectable_and_its_mixer_starts_open() {
        let mut app = app();
        assert!(LayoutPreset::Reaper.available());
        assert_eq!(app.run("ui.layout", json!({"preset": "reaper"})), Ok(json!({"preset": "reaper"})));
        assert!(active(&app));
        assert!(app.ui.reaper_mixer_open);
        // Ctrl+M folds Reaper's mixer, not NEW's.
        assert_eq!(app.run("ui.mixer", json!({})), Ok(json!({"value": false})));
        assert!(!app.ui.reaper_mixer_open);
        assert!(!app.ui.new_mixer_open);
        app.run("ui.layout", json!({"preset": "new"})).unwrap();
        assert_eq!(app.run("ui.mixer", json!({})), Ok(json!({"value": true})));
        assert!(app.ui.new_mixer_open);
        assert!(!app.ui.reaper_mixer_open);
    }

    #[test]
    fn switching_new_classic_reaper_and_back_keeps_the_session() {
        let mut app = app();
        app.engine.execute("session.new", &json!({"template": "drums"})).unwrap();
        let before = app.engine.session().clone();
        for preset in ["reaper", "classic", "reaper", "new"] {
            app.run("ui.layout", json!({"preset": preset})).unwrap();
            assert_eq!(*app.engine.session(), before, "after switching to {preset}");
        }
        assert_eq!(app.ui.layout, LayoutPreset::New);
    }

    #[test]
    fn older_settings_open_the_reaper_mixer() {
        let old: crate::UiState = serde_json::from_value(json!({"layout": "reaper"})).unwrap();
        assert_eq!(old.layout, LayoutPreset::Reaper);
        assert!(old.reaper_mixer_open);
        assert!(old.key_overrides.is_empty());
    }
}
