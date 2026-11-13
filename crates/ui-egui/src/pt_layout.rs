//! The Pro Tools-style layout preset: the Edit window (toolbar with the edit modes, tools, counters
//! and transport on top, tracks and clips below) with the Mix window docked under it, so both are on
//! screen at once. Classic keeps them as two windows; this is the same Edit and Mix in one
//! arrangement, with the same engine, session and commands. The keys are the shared editing keys
//! plus [`PT_KEYS`]. Learnt from how such a DAW behaves, not from its code or artwork.

use crate::SoundApp;
use crate::new_layout::{LayoutPreset, MIXER_COLLAPSED_H, MIXER_FIT_H};
use crate::theme::{Tokens, bold, mono};
use egui::{Align2, Rect, Sense, Stroke, Ui, pos2, vec2};
use serde_json::json;

/// The switcher's name for this layout.
pub const LABEL: &str = "Pro Tools-style";

/// The tracks keep at least this much height above the open Mix.
pub const TRACKS_MIN_H: f32 = 300.0;
/// The open Mix is never shorter than its header and a little of the strips.
pub const MIX_MIN_H: f32 = MIXER_COLLAPSED_H + 120.0;

/// The layout's own keys before the shared ones: Cmd+= shows or hides the Mix under the Edit window
/// (with Mix in its own window, it brings that window forward as before).
pub const PT_KEYS: &[(&str, &str)] = &[("Cmd+=", "window.toggle_mix_edit")];

/// Draw the Pro Tools-style layout.
pub fn show(app: &mut SoundApp, ui: &mut Ui) {
    crate::edit_window::show_with(app, ui, |app, ui| {
        if app.ui.mix_separate {
            return;
        }
        let t = Tokens::current();
        // Below the status bar and above the dock tabs: what the toolbar, the status bar and the
        // dock tabs leave, less the room the tracks keep.
        let room = ui.available_rect_before_wrap().height() - 22.0;
        // Open, the Mix is tall enough for a whole strip, fader included, when the window allows
        // it, and the tracks keep at least `TRACKS_MIN_H`; shorter, the strips scroll and start
        // at the bottom, so the faders and names stay in view.
        let h = if app.ui.pt_mix_open { MIXER_FIT_H.min(room - TRACKS_MIN_H).max(MIX_MIN_H) } else { MIXER_COLLAPSED_H };
        egui::Panel::bottom("pt_mix").exact_size(h).frame(egui::Frame::NONE.fill(t.panel_bg)).show(ui, |ui| mix(app, ui));
    });
}

/// The Mix along the bottom: a header row that shows or hides it, the layout switcher, and the strips.
fn mix(app: &mut SoundApp, ui: &mut Ui) {
    let t = Tokens::current();
    let r = ui.max_rect();
    ui.painter().line_segment([pos2(r.min.x, r.min.y + 0.5), pos2(r.max.x, r.min.y + 0.5)], Stroke::new(1.0, t.border));
    let header = Rect::from_min_size(r.min, vec2(r.width(), MIXER_COLLAPSED_H));
    let open = app.ui.pt_mix_open;
    let toggle = Rect::from_min_size(header.min, vec2(120.0, header.height()));
    let resp = ui.interact(toggle, ui.scope_id().with("pt_mix_header"), Sense::click());
    let c = pos2(header.min.x + 15.0, header.center().y);
    let points = if open {
        vec![pos2(c.x - 4.0, c.y - 2.5), pos2(c.x + 4.0, c.y - 2.5), pos2(c.x, c.y + 3.0)]
    } else {
        vec![pos2(c.x - 2.5, c.y - 4.0), pos2(c.x + 3.0, c.y), pos2(c.x - 2.5, c.y + 4.0)]
    };
    ui.painter().add(egui::Shape::convex_polygon(points, t.text_dim, Stroke::NONE));
    ui.painter().text(pos2(header.min.x + 26.0, header.center().y), Align2::LEFT_CENTER, "MIX", bold(11.0), t.text_dim);
    let mac = ui.ctx().os().is_mac();
    let key = crate::keymap::chord_for(app, "window.toggle_mix_edit").map(|c| crate::keymap::display(&c, mac));
    if let Some(k) = &key {
        ui.painter().text(pos2(header.min.x + 60.0, header.center().y), Align2::LEFT_CENTER, k, mono(11.0), t.text_dim);
    }
    let tip = key.map_or_else(|| "Show or hide the Mix".to_string(), |k| format!("Show or hide the Mix ({k})"));
    if resp.on_hover_text(tip).clicked() {
        let _ = app.run("ui.mixer", json!({}));
    }
    // The way to the other layouts, which Classic finds in the Window menu.
    let row = Rect::from_min_max(pos2(toggle.max.x, header.min.y + 2.0), pos2(header.max.x - 8.0, header.max.y - 2.0));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(row).layout(egui::Layout::right_to_left(egui::Align::Center)));
    child.spacing_mut().item_spacing.x = 4.0;
    child.spacing_mut().button_padding = vec2(6.0, 1.0);
    for preset in LayoutPreset::ALL.into_iter().rev() {
        let on = app.ui.layout == preset;
        let text = egui::RichText::new(preset.label()).font(bold(10.5)).color(if on { t.mode_on_text } else { t.text });
        let button = egui::Button::new(text).fill(if on { t.mode_on } else { t.button }).selected(on);
        let tip = if preset.available() { format!("{} layout", preset.label()) } else { format!("{} layout: later", preset.label()) };
        if child.add_enabled(preset.available(), button).on_hover_text(tip.clone()).on_disabled_hover_text(tip).clicked() {
            let _ = app.run("ui.layout", json!({"preset": preset.id()}));
        }
    }
    if open {
        let body = Rect::from_min_max(pos2(r.min.x, header.max.y), r.max);
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(body));
        child.set_clip_rect(body.intersect(ui.clip_rect()));
        child.painter().rect_filled(body, 0.0, t.window_bg);
        egui::ScrollArea::vertical()
            .id_salt("pt_mix_scroll")
            .auto_shrink([false, false])
            .stick_to_bottom(true)
            .show(&mut child, |ui| crate::mix_window::content(app, ui));
    }
}

/// Whether the Pro Tools-style layout is showing.
pub fn active(app: &SoundApp) -> bool {
    app.ui.layout == LayoutPreset::ProToolsStyle
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
    fn the_pro_tools_layout_is_selectable_and_shows_edit_with_mix_open() {
        let mut app = app();
        app.ui.window = crate::MainWindow::Mix;
        assert!(LayoutPreset::ProToolsStyle.available());
        assert_eq!(app.run("ui.layout", json!({"preset": "proToolsStyle"})), Ok(json!({"preset": "proToolsStyle"})));
        assert!(active(&app));
        assert_eq!(app.ui.window, crate::MainWindow::Edit, "the Edit window, with Mix under it");
        assert!(app.ui.pt_mix_open);
        assert_eq!(LayoutPreset::parse(LABEL), Some(LayoutPreset::ProToolsStyle));
        // Cmd+= (Toggle Mix/Edit) folds the Mix away and back, and stays in the Edit window.
        app.run("window.toggle_mix_edit", json!({})).unwrap();
        assert!(!app.ui.pt_mix_open);
        assert_eq!(app.ui.window, crate::MainWindow::Edit);
        app.run("ui.mixer", json!({})).unwrap();
        assert!(app.ui.pt_mix_open);
        // NEW's mixer is its own.
        assert!(!app.ui.new_mixer_open);
    }

    #[test]
    fn switching_through_every_layout_and_back_keeps_the_session() {
        let mut app = app();
        app.engine.execute("session.new", &json!({"template": "drums"})).unwrap();
        let before = app.engine.session().clone();
        for preset in
            ["proToolsStyle", "new", "proToolsStyle", "classic", "proToolsStyle", "reaper", "proToolsStyle", "flStyle", "proToolsStyle", "new"]
        {
            app.run("ui.layout", json!({"preset": preset})).unwrap();
            assert_eq!(*app.engine.session(), before, "after switching to {preset}");
        }
        assert_eq!(app.ui.layout, LayoutPreset::New);
    }

    #[test]
    fn older_settings_open_the_pro_tools_mix() {
        let old: crate::UiState = serde_json::from_value(json!({"layout": "proToolsStyle"})).unwrap();
        assert_eq!(old.layout, LayoutPreset::ProToolsStyle);
        assert!(old.pt_mix_open);
    }
}
