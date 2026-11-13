//! The Ableton-style layout preset: the Browser on the left, the Session View (clips in a grid, one
//! column per track, one row per scene) beside it, the Arrangement in the centre, and the Device
//! View (the selected clip and the selected track's instrument, Drum Rack or Instrument Rack) along
//! the bottom. Same engine, session and commands as the other layouts; only the panels, the words
//! and the keys change (the keys are [`ABL_KEYS`]). Learnt from how such a DAW behaves, not from its
//! code or artwork.

use crate::SoundApp;
use crate::new_layout::{LayoutPreset, TOP_BAR_H};
use crate::theme::{Tokens, bold, mono, regular};
use crate::widgets::icon_button;
use egui::{Align2, Color32, Key, Modifiers, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use serde_json::{Value, json};
use soundcraft_model::{ClipContent, ClipId, FollowAction, Session, TrackId};
use soundcraft_time::{Samples, TimeFormat};

/// The switcher's name for this layout.
pub const LABEL: &str = "Ableton-style";

/// Default sizes of the panels.
pub const BROWSER_W: f32 = 190.0;
pub const SESSION_W: f32 = 640.0;
pub const DEVICE_H: f32 = 250.0;
/// A Session View column and row.
pub const CELL_W: f32 = 92.0;
/// The narrowest a Session View column gets before the grid scrolls sideways.
pub const MIN_CELL_W: f32 = 56.0;
pub const CELL_H: f32 = 24.0;
/// The scene column on the right of the grid.
pub const SCENE_W: f32 = 110.0;
/// A panel's title row.
pub const TITLE_H: f32 = 24.0;

/// The layout's own keys before the shared ones: chord, command. Chords use the registry's
/// spelling (`Cmd` is Ctrl off macOS).
pub const ABL_KEYS: &[(&str, &str)] = &[
    ("Tab", "ui.session_view"),
    ("Cmd+Alt+B", "ui.browser"),
    ("Cmd+Alt+L", "ui.device_view"),
    ("Enter", "ui.launch_scene"),
    ("Cmd+I", "scene.new"),
    ("Cmd+E", "edit.separate"),
    ("Cmd+J", "edit.consolidate"),
    ("Cmd+D", "edit.duplicate"),
    ("Cmd+U", "event.quantize"),
    ("Cmd+T", "track.new"),
    ("F9", "transport.record"),
    ("Cmd+L", "options.loop_playback"),
    ("Delete", "edit.clear"),
];

/// What the layout remembers between frames (not saved).
#[derive(Debug, Clone, Default)]
pub struct AblState {
    /// The selected scene (row), for Enter.
    pub scene: usize,
    /// The scene whose name is being edited, and the text so far.
    pub rename: Option<(usize, String)>,
    /// The selected Drum Rack pad (0..16).
    pub pad: usize,
    /// The transport was playing last frame (launched clips stop with it).
    pub was_playing: bool,
}

/// Something dragged in the layout: a slot's clip, a Browser audio file or a pool groove.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Drag {
    Slot(TrackId, usize),
    Source(soundcraft_model::SourceId),
    Groove(u64),
}

fn drag_id() -> egui::Id {
    egui::Id::unique("solara-abl-drag")
}

/// Whether the Ableton-style layout is showing.
pub fn active(app: &SoundApp) -> bool {
    app.ui.layout == LayoutPreset::AbletonStyle
}

/// The layout's UI commands. None for other ids, and for `ui.browser` outside this layout.
pub fn command(app: &mut SoundApp, id: &str, p: &Value) -> Option<Result<Value, String>> {
    let toggle = |v: &mut bool| {
        *v = p.get("value").and_then(Value::as_bool).unwrap_or(!*v);
        Ok(json!({"value": *v}))
    };
    Some(match id {
        "ui.session_view" => toggle(&mut app.ui.abl_session_open),
        "ui.device_view" => toggle(&mut app.ui.abl_device_open),
        "ui.browser" if active(app) => toggle(&mut app.ui.abl_browser_open),
        "ui.launch_scene" => {
            let scene = p.get("scene").and_then(Value::as_u64).map_or(app.abl.scene, |v| usize::try_from(v).unwrap_or(usize::MAX));
            app.run("scene.launch", json!({"scene": scene}))
        }
        _ => return None,
    })
}

/// The layout's keys that depend on what is selected, before [`ABL_KEYS`]: Delete on a selected
/// Session View clip deletes that clip. Returns whether the key was taken.
pub fn keys(app: &mut SoundApp, key: Key, m: Modifiers) -> bool {
    if key == Key::Delete
        && !m.any()
        && let Some((t, slot)) = selected_slot(app.engine.session())
    {
        if let Err(e) = app.run("slot.clear", json!({"track": t.0, "scene": slot})) {
            app.ui.status = e;
        }
        return true;
    }
    false
}

/// Every frame, whatever the layout: launched clips follow the song as it plays (follow actions are
/// folded in) and stop when the transport stops.
pub fn tick(app: &mut SoundApp) {
    let playing = app.is_playing();
    if !app.engine.session().launch.is_empty() {
        if playing {
            let at = app.engine.transport.position;
            let _ = app.engine.execute("launcher.settle", &json!({"at": at}));
        } else if app.abl.was_playing {
            let _ = app.engine.execute("launcher.stop_all", &json!({}));
        }
    }
    app.abl.was_playing = playing;
}

/// The selected Session View clip: its track and scene.
pub fn selected_slot(s: &Session) -> Option<(TrackId, usize)> {
    s.edit.selected_clips.iter().find_map(|c| s.find_slot_clip(*c))
}

/// Draw the Ableton-style layout.
pub fn show(app: &mut SoundApp, ui: &mut Ui) {
    let t = Tokens::current();
    crate::edit_window::drive_drag(app, ui.ctx());
    egui::Panel::top("abl_top_bar").exact_size(TOP_BAR_H).frame(egui::Frame::NONE.fill(t.toolbar_bg)).show(ui, |ui| top_bar(app, ui));
    if app.ui.abl_device_open {
        egui::Panel::bottom("abl_device_view")
            .resizable(true)
            .default_size(DEVICE_H)
            .size_range(140.0..=600.0)
            .frame(egui::Frame::NONE.fill(t.panel_bg))
            .show(ui, |ui| device_view(app, ui));
    }
    if app.ui.abl_browser_open {
        egui::Panel::left("abl_browser")
            .resizable(true)
            .default_size(BROWSER_W)
            .size_range(150.0..=360.0)
            .frame(egui::Frame::NONE.fill(t.panel_bg))
            .show(ui, |ui| browser(app, ui));
    }
    if app.ui.abl_session_open {
        egui::Panel::left("abl_session_view")
            .resizable(true)
            .default_size(SESSION_W)
            .size_range(260.0..=1400.0)
            .frame(egui::Frame::NONE.fill(t.window_bg))
            .show(ui, |ui| session_view(app, ui));
    }
    egui::CentralPanel::default().frame(egui::Frame::NONE.fill(t.window_bg)).show(ui, |ui| {
        let full = ui.max_rect();
        let body = title(ui, full, "ARRANGEMENT");
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(body));
        child.set_clip_rect(body.intersect(ui.clip_rect()));
        crate::edit_window::main_area(app, &mut child);
    });
    // A drag let go anywhere else is over.
    if ui.input(|i| i.pointer.any_released()) {
        ui.ctx().data_mut(|d| d.remove::<Drag>(drag_id()));
    }
}

/// A panel's title row with its top border, and the rest of the panel below it.
fn title(ui: &Ui, r: Rect, text: &str) -> Rect {
    let t = Tokens::current();
    ui.painter().line_segment([pos2(r.min.x, r.min.y + 0.5), pos2(r.max.x, r.min.y + 0.5)], Stroke::new(1.0, t.border));
    let row = Rect::from_min_size(r.min, vec2(r.width(), TITLE_H));
    ui.painter().text(pos2(row.min.x + 10.0, row.center().y), Align2::LEFT_CENTER, text, bold(11.0), t.text_dim);
    Rect::from_min_max(pos2(r.min.x, row.max.y), r.max)
}

/// The chord a command answers to here, for tooltips.
fn tip(app: &SoundApp, ui: &Ui, label: &str, id: &str) -> String {
    let mac = ui.ctx().os().is_mac();
    match crate::keymap::chord_for(app, id) {
        Some(c) => format!("{label} ({})", crate::keymap::display(&c, mac)),
        None => label.to_string(),
    }
}

/// The top bar: tempo, play controls, position, the layout switcher and the panel buttons.
fn top_bar(app: &mut SoundApp, ui: &mut Ui) {
    let t = Tokens::current();
    let r = ui.max_rect();
    ui.painter().line_segment([pos2(r.min.x, r.max.y - 0.5), pos2(r.max.x, r.max.y - 0.5)], Stroke::new(1.0, t.border));
    let mut row = ui.new_child(egui::UiBuilder::new().max_rect(r.shrink2(vec2(10.0, 6.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
    let ui = &mut row;
    ui.spacing_mut().item_spacing.x = 6.0;
    let (bars, bpm, looped, playing, recording) = {
        let s = app.engine.session();
        let at = app.position();
        let tick = s.tempo.samples_to_ticks(at, s.sample_rate);
        (
            s.format_position(at, TimeFormat::BarsBeats),
            s.tempo.tempo_at_tick(tick),
            s.edit.loop_playback,
            app.is_playing(),
            app.engine.transport.recording,
        )
    };
    crate::toolbar::tempo_field(app, ui, bpm);
    ui.add_space(8.0);
    let size = vec2(30.0, 28.0);
    let buttons = [
        ("play", playing, "Play (Space)".to_string(), "transport.play"),
        ("stop", false, "Stop (Space)".to_string(), "transport.stop"),
        ("record", recording, tip(app, ui, "Arrangement Record", "transport.record"), "transport.record"),
        ("loop", looped, tip(app, ui, "Loop", "options.loop_playback"), "options.loop_playback"),
    ];
    for (icon, on, hover, id) in buttons {
        if icon_button(ui, size, icon, on, &hover).clicked() {
            let _ = app.run(id, json!({}));
        }
    }
    ui.add_space(6.0);
    ui.label(egui::RichText::new(bars).font(mono(13.0)).color(t.counter_text));
    ui.add_space(8.0);
    crate::new_layout::switcher(app, ui);
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let panels = [
            ("Device View", "ui.device_view", app.ui.abl_device_open),
            ("Session View", "ui.session_view", app.ui.abl_session_open),
            ("Browser", "ui.browser", app.ui.abl_browser_open),
        ];
        for (label, id, on) in panels {
            let hover = tip(app, ui, &format!("Show or hide the {label}"), id);
            if ui.add(egui::Button::selectable(on, egui::RichText::new(label).font(regular(12.0)))).on_hover_text(hover).clicked() {
                let _ = app.run(id, json!({}));
            }
        }
    });
}

// ---- Browser -------------------------------------------------------------------------------

fn heading(ui: &mut Ui, text: &str) {
    let t = Tokens::current();
    ui.add_space(6.0);
    ui.label(egui::RichText::new(text).font(bold(10.5)).color(t.text_dim));
}

/// One Browser row; returns its response when the search shows it.
fn row(ui: &mut Ui, needle: &str, name: &str, hint: &str) -> Option<egui::Response> {
    if !needle.is_empty() && !name.to_lowercase().contains(needle) {
        return None;
    }
    let t = Tokens::current();
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 20.0), Sense::click_and_drag());
    if resp.hovered() || resp.dragged() {
        ui.painter().rect_filled(rect, 3.0, t.button_hi);
    }
    ui.painter().text(pos2(rect.min.x + 6.0, rect.center().y), Align2::LEFT_CENTER, name, regular(12.0), t.text);
    Some(resp.on_hover_text(hint))
}

/// The Browser: instruments and drums (click for a new track), the session's audio files (drag
/// one into an audio track's slot) and the Groove Pool (drag a groove onto a MIDI clip).
fn browser(app: &mut SoundApp, ui: &mut Ui) {
    let t = Tokens::current();
    let r = ui.max_rect();
    ui.expand_to_include_rect(r);
    let body = title(ui, r, "BROWSER");
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(body.shrink2(vec2(8.0, 4.0))));
    let ui = &mut child;
    ui.add(egui::TextEdit::singleline(&mut app.browser_search).hint_text("Search by name").desired_width(f32::INFINITY));
    let needle = app.browser_search.trim().to_lowercase();
    let plugins = soundcraft_dsp::plugins();
    let sources: Vec<(soundcraft_model::SourceId, String)> = app.engine.session().sources.iter().map(|f| (f.id, f.name.clone())).collect();
    let grooves: Vec<soundcraft_model::GroovePreset> = app.engine.session().grooves.clone();
    egui::ScrollArea::vertical().id_salt("abl_browser_scroll").auto_shrink([false, false]).show(ui, |ui| {
        heading(ui, "INSTRUMENTS");
        for info in plugins.iter().filter(|p| p.category == soundcraft_dsp::Category::Instrument) {
            if row(ui, &needle, info.name, "Click for a new track with this instrument").is_some_and(|r| r.clicked())
                && let Err(e) = app.run("track.new", json!({"kind": "instrument", "instrument": info.id, "name": info.name}))
            {
                app.ui.status = e;
            }
        }
        heading(ui, "AUDIO FILES");
        if sources.is_empty() {
            ui.label(egui::RichText::new("Import audio to see it here.").font(regular(11.0)).color(t.text_dim));
        }
        for (id, name) in &sources {
            if let Some(resp) = row(ui, &needle, name, "Drag into an audio track's slot in the Session View")
                && resp.drag_started()
            {
                ui.ctx().data_mut(|d| d.insert_temp(drag_id(), Drag::Source(*id)));
            }
        }
        heading(ui, "GROOVE POOL");
        for g in &grooves {
            let label = format!("{}  {:.0}% / {:.0}%", g.name, g.groove.swing * 100.0, g.groove.humanize * 100.0);
            if let Some(resp) = row(ui, &needle, &label, "Drag onto a MIDI clip: swing / humanize. Right-click to edit.") {
                if resp.drag_started() {
                    ui.ctx().data_mut(|d| d.insert_temp(drag_id(), Drag::Groove(g.id)));
                }
                resp.context_menu(|ui| {
                    let (mut swing, mut human) = (g.groove.swing * 100.0, g.groove.humanize * 100.0);
                    let a = ui.add(egui::Slider::new(&mut swing, 0.0..=100.0).text("Swing %"));
                    let b = ui.add(egui::Slider::new(&mut human, 0.0..=100.0).text("Humanize %"));
                    if a.changed() || b.changed() {
                        let p = json!({"groove": g.id, "swing": swing / 100.0, "humanize": human / 100.0});
                        let _ = app.engine.execute_merged("groove.edit", &p, &format!("groove.edit.{}", g.id));
                    }
                    if ui.button("Delete groove").clicked() {
                        let _ = app.run("groove.delete", json!({"groove": g.id}));
                        ui.close();
                    }
                });
            }
        }
        ui.menu_button(egui::RichText::new("+ Add groove").font(regular(12.0)), |ui| {
            for (name, _, _) in soundcraft_engine::cmd::groove_pool::STARTERS {
                if ui.button(name).clicked() {
                    let _ = app.run("groove.add", json!({"starter": name}));
                    ui.close();
                }
            }
        });
    });
}

// ---- Session View ----------------------------------------------------------------------------

fn track_color(c: [u8; 3]) -> Color32 {
    Color32::from_rgb(c[0], c[1], c[2])
}

/// Draw a play triangle in `r`.
fn triangle(ui: &Ui, r: Rect, color: Color32) {
    let c = r.center();
    let pts = vec![pos2(c.x - 3.5, c.y - 5.0), pos2(c.x + 5.0, c.y), pos2(c.x - 3.5, c.y + 5.0)];
    ui.painter().add(egui::Shape::convex_polygon(pts, color, Stroke::NONE));
}

/// Draw a stop square in `r`.
fn square(ui: &Ui, r: Rect, color: Color32) {
    ui.painter().rect_filled(Rect::from_center_size(r.center(), vec2(8.0, 8.0)), 1.0, color);
}

/// One column of the grid: id, name, colour, whether it holds MIDI, and its slots (name, colour,
/// clip id, length, whether it is audio).
struct Column {
    id: TrackId,
    name: String,
    color: Color32,
    midi: bool,
    slots: Vec<Option<(String, Color32, ClipId, Samples, bool)>>,
}

/// The Session View: tracks across, scenes down, the scene column on the right.
fn session_view(app: &mut SoundApp, ui: &mut Ui) {
    let t = Tokens::current();
    let r = ui.max_rect();
    ui.expand_to_include_rect(r);
    let body = title(ui, r, "SESSION");
    let s = app.engine.session();
    let pos = app.position();
    let playing = app.is_playing();
    let cols: Vec<Column> = s
        .tracks
        .iter()
        .filter(|tr| tr.kind.has_slots() && !tr.hidden)
        .map(|tr| Column {
            id: tr.id,
            name: tr.name.clone(),
            color: track_color(tr.color),
            midi: tr.kind.is_midi(),
            slots: (0..s.scenes.len())
                .map(|i| tr.slot(i).map(|c| (c.name.clone(), track_color(c.color.unwrap_or(tr.color)), c.id, c.length.max(1), c.is_audio())))
                .collect(),
        })
        .collect();
    let scenes: Vec<String> = s.scenes.iter().map(|x| x.name.clone()).collect();
    let now: Vec<Option<(usize, Samples)>> =
        cols.iter().map(|c| if playing { s.track(c.id).and_then(|tr| s.playing_slot(tr, pos)) } else { None }).collect();
    let queued: Vec<Vec<usize>> = cols.iter().map(|c| s.queued_slots(c.id, pos).collect()).collect();
    let selected = selected_slot(s);
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(body));
    child.set_clip_rect(body.intersect(ui.clip_rect()));
    // Columns narrow to keep the scene column in view, down to MIN_CELL_W; past that the grid scrolls.
    let seen = body.intersect(ui.clip_rect()).width().min(body.width());
    let cw = ((seen - SCENE_W - 24.0) / cols.len().max(1) as f32).clamp(MIN_CELL_W, CELL_W);
    let width = cols.len() as f32 * cw + SCENE_W + 8.0;
    let height = (scenes.len() as f32 + 3.0) * CELL_H + 8.0;
    egui::ScrollArea::both().id_salt("abl_session_scroll").auto_shrink([false, false]).show(&mut child, |ui| {
        let (area, _) = ui.allocate_exact_size(vec2(width.max(ui.available_width()), height), Sense::hover());
        let o = area.min + vec2(4.0, 0.0);
        let cell = |col: usize, row: usize| Rect::from_min_size(o + vec2(col as f32 * cw, row as f32 * CELL_H), vec2(cw - 2.0, CELL_H - 2.0));
        let scene_x = o.x + cols.len() as f32 * cw;
        let scene_cell = |row: usize| Rect::from_min_size(pos2(scene_x, o.y + row as f32 * CELL_H), vec2(SCENE_W - 4.0, CELL_H - 2.0));
        // Track titles.
        for (ci, c) in cols.iter().enumerate() {
            let r = cell(ci, 0);
            ui.painter().rect_filled(r, 2.0, c.color.gamma_multiply(0.85));
            ui.painter().with_clip_rect(r.shrink(1.0)).text(
                pos2(r.min.x + 6.0, r.center().y),
                Align2::LEFT_CENTER,
                &c.name,
                bold(11.0),
                Color32::BLACK,
            );
        }
        ui.painter().text(pos2(scene_x + 6.0, cell(0, 0).center().y), Align2::LEFT_CENTER, "Scenes", bold(11.0), t.text_dim);
        let hint = |a: &str| egui::RichText::new(a).font(regular(11.0));
        let drag: Option<Drag> = ui.ctx().data(|d| d.get_temp(drag_id()));
        let released = ui.input(|i| i.pointer.any_released());
        let copy = ui.input(|i| i.modifiers.command || i.modifiers.alt);
        // Slots.
        for (ci, c) in cols.iter().enumerate() {
            for (si, slot) in c.slots.iter().enumerate() {
                let r = cell(ci, si + 1);
                let id = ui.scope_id().with(("abl_slot", c.id.0, si));
                let resp = ui.interact(r, id, Sense::click_and_drag());
                let is_now = now.get(ci).copied().flatten().filter(|(n, _)| *n == si);
                let waiting = queued.get(ci).is_some_and(|q| q.contains(&si));
                match slot {
                    Some((name, color, clip, len, _audio)) => {
                        let on = selected == Some((c.id, si));
                        ui.painter().rect_filled(r, 2.0, color.gamma_multiply(if is_now.is_some() { 1.0 } else { 0.7 }));
                        if on {
                            ui.painter().rect_stroke(r, 2.0, Stroke::new(1.5, Color32::WHITE), StrokeKind::Inside);
                        }
                        let play = Rect::from_min_size(r.min, vec2(18.0, r.height()));
                        let blink = (ui.input(|i| i.time) * 4.0) as i64 % 2 == 0;
                        let tri = if is_now.is_some() {
                            t.meter_green
                        } else if waiting && blink {
                            t.meter_yellow
                        } else {
                            Color32::from_black_alpha(160)
                        };
                        triangle(ui, play, tri);
                        ui.painter().with_clip_rect(r.shrink(1.0)).text(
                            pos2(r.min.x + 20.0, r.center().y),
                            Align2::LEFT_CENTER,
                            name,
                            regular(11.0),
                            Color32::BLACK,
                        );
                        if let Some((_, run)) = is_now {
                            let frac = (pos.saturating_sub(run).rem_euclid(*len)) as f32 / *len as f32;
                            let bar = Rect::from_min_size(pos2(r.min.x, r.max.y - 3.0), vec2(r.width() * frac.clamp(0.0, 1.0), 3.0));
                            ui.painter().rect_filled(bar, 0.0, Color32::from_black_alpha(140));
                            ui.ctx().request_repaint();
                        }
                        if waiting {
                            ui.ctx().request_repaint();
                        }
                        let over_play = resp.interact_pointer_pos().is_some_and(|p| play.contains(p));
                        if resp.clicked() {
                            if over_play {
                                if let Err(e) = app.run("slot.launch", json!({"track": c.id.0, "scene": si})) {
                                    app.ui.status = e;
                                }
                            } else {
                                let _ = app.run("edit.select", json!({"clips": [clip.0], "tracks": [c.id.0]}));
                            }
                            app.abl.scene = si;
                        }
                        if resp.drag_started() {
                            ui.ctx().data_mut(|d| d.insert_temp(drag_id(), Drag::Slot(c.id, si)));
                        }
                        let resp =
                            resp.on_hover_text("Click the triangle to play the clip, the name to select it. Drag to move it, Ctrl+drag to copy.");
                        resp.context_menu(|ui| {
                            ui.label(hint("Follow action"));
                            for a in FollowAction::ALL {
                                if ui.button(a.label()).clicked() {
                                    let _ = app.run("clip.follow", json!({"clips": [clip.0], "action": a.id()}));
                                    ui.close();
                                }
                            }
                            if ui.button("No follow action").clicked() {
                                let _ = app.run("clip.follow", json!({"clips": [clip.0], "action": "none"}));
                                ui.close();
                            }
                            ui.separator();
                            if ui.button("Delete clip").clicked() {
                                let _ = app.run("slot.clear", json!({"track": c.id.0, "scene": si}));
                                ui.close();
                            }
                        });
                    }
                    None => {
                        ui.painter().rect_filled(r, 2.0, t.slot_empty);
                        let stop = Rect::from_center_size(r.center(), vec2(14.0, 14.0));
                        square(ui, stop, t.text_dim.gamma_multiply(0.6));
                        if resp.clicked() {
                            let _ = app.run("slot.stop", json!({"track": c.id.0}));
                            app.abl.scene = si;
                        }
                        if resp.double_clicked() && c.midi {
                            match app.run("slot.set", json!({"track": c.id.0, "scene": si, "bars": 1})) {
                                Ok(v) => {
                                    let _ = app.run("edit.select", json!({"clips": [v["clip"]], "tracks": [c.id.0]}));
                                }
                                Err(e) => app.ui.status = e,
                            }
                        }
                        let tip_text = if c.midi {
                            "Click: stop the track. Double-click: a new one-bar MIDI clip. Right-click: put the selected Arrangement clip here."
                        } else {
                            "Click: stop the track. Drop an audio file from the Browser here. Right-click: put the selected Arrangement clip here."
                        };
                        let resp = resp.on_hover_text(tip_text);
                        resp.context_menu(|ui| {
                            let arr = app
                                .engine
                                .session()
                                .edit
                                .selected_clips
                                .iter()
                                .copied()
                                .find(|id| app.engine.session().find_slot_clip(*id).is_none());
                            if ui.add_enabled(arr.is_some(), egui::Button::new("Put selected Arrangement clip here")).clicked() {
                                if let Some(a) = arr
                                    && let Err(e) = app.run("slot.set", json!({"track": c.id.0, "scene": si, "clip": a.0}))
                                {
                                    app.ui.status = e;
                                }
                                ui.close();
                            }
                        });
                    }
                }
                // Drops onto this slot.
                let hovered = ui.ctx().pointer_latest_pos().is_some_and(|p| r.contains(p));
                if let Some(d) = drag
                    && hovered
                {
                    ui.painter().rect_stroke(r, 2.0, Stroke::new(1.5, t.accent), StrokeKind::Inside);
                    if released {
                        let res = match d {
                            Drag::Slot(ft, fs) => {
                                app.run("slot.move", json!({"track": ft.0, "scene": fs, "to_track": c.id.0, "to_scene": si, "copy": copy}))
                            }
                            Drag::Source(f) => app.run("slot.set", json!({"track": c.id.0, "scene": si, "source": f.0})),
                            Drag::Groove(g) => match slot {
                                Some((_, _, clip, _, _)) => app.run("groove.apply", json!({"clips": [clip.0], "groove": g})),
                                None => Err("Drop a groove on a MIDI clip".to_string()),
                            },
                        };
                        if let Err(e) = res {
                            app.ui.status = e;
                        }
                    }
                }
            }
            // The track's stop button under its slots.
            let r = cell(ci, scenes.len() + 1);
            let resp = ui.interact(r, ui.scope_id().with(("abl_stop", c.id.0)), Sense::click());
            ui.painter().rect_filled(r, 2.0, t.button);
            square(ui, r, if now.get(ci).copied().flatten().is_some() { t.text } else { t.text_dim });
            if resp.on_hover_text("Stop the track's clip").clicked() {
                let _ = app.run("slot.stop", json!({"track": c.id.0}));
            }
        }
        // Scenes.
        for (si, name) in scenes.iter().enumerate() {
            scene_button(app, ui, scene_cell(si + 1), si, name, scenes.len());
        }
        // Stop All Clips and a new scene.
        let r = scene_cell(scenes.len() + 1);
        let resp = ui.interact(r, ui.scope_id().with("abl_stop_all"), Sense::click());
        ui.painter().rect_filled(r, 2.0, t.button);
        square(ui, Rect::from_min_size(r.min, vec2(18.0, r.height())), t.text);
        ui.painter().text(pos2(r.min.x + 20.0, r.center().y), Align2::LEFT_CENTER, "Stop All Clips", regular(11.0), t.text);
        if resp.clicked() {
            let _ = app.run("launcher.stop_all", json!({}));
        }
        let r = scene_cell(scenes.len() + 2);
        let resp = ui.interact(r, ui.scope_id().with("abl_new_scene"), Sense::click());
        ui.painter().rect_stroke(r, 2.0, Stroke::new(1.0, t.border), StrokeKind::Inside);
        ui.painter().text(r.center(), Align2::CENTER_CENTER, "+ Scene", regular(11.0), t.text_dim);
        if resp.on_hover_text(tip(app, ui, "Insert a scene at the bottom", "scene.new")).clicked() {
            let _ = app.run("scene.new", json!({}));
        }
        if cols.is_empty() {
            ui.painter().text(
                pos2(o.x + 6.0, cell(0, 1).center().y),
                Align2::LEFT_CENTER,
                "Add an audio, MIDI or instrument track to fill its slots.",
                regular(11.0),
                t.text_dim,
            );
        }
    });
}

// ---- Device View -------------------------------------------------------------------------------

/// The clip the Device View shows: the selected Session View clip, else a selected Arrangement clip.
/// One scene's button: click the triangle to launch the row; double-click to rename; right-click for the rest.
fn scene_button(app: &mut SoundApp, ui: &mut Ui, r: Rect, si: usize, name: &str, count: usize) {
    let t = Tokens::current();
    let on = app.abl.scene == si;
    ui.painter().rect_filled(r, 2.0, if on { t.button_hi } else { t.button });
    let play = Rect::from_min_size(r.min, vec2(18.0, r.height()));
    triangle(ui, play, t.text);
    let resp = ui.interact(r, ui.scope_id().with(("abl_scene", si)), Sense::click());
    let editing = app.abl.rename.as_ref().is_some_and(|(i, _)| *i == si);
    if editing {
        let text_rect = Rect::from_min_max(pos2(r.min.x + 20.0, r.min.y), r.max);
        let mut done = None;
        if let Some((_, text)) = &mut app.abl.rename {
            let edit = ui.put(text_rect, egui::TextEdit::singleline(text).font(regular(11.0)));
            edit.request_focus();
            if edit.lost_focus() {
                done = Some(text.clone());
            }
        }
        if let Some(text) = done {
            app.abl.rename = None;
            let _ = app.run("scene.rename", json!({"scene": si, "name": text}));
        }
    } else {
        ui.painter().text(pos2(r.min.x + 20.0, r.center().y), Align2::LEFT_CENTER, name, regular(11.0), t.text);
    }
    let over_play = resp.interact_pointer_pos().is_some_and(|p| play.contains(p));
    if resp.clicked() {
        app.abl.scene = si;
        if over_play && let Err(e) = app.run("scene.launch", json!({"scene": si})) {
            app.ui.status = e;
        }
    }
    if resp.double_clicked() && !over_play {
        app.abl.rename = Some((si, name.to_string()));
    }
    let resp = resp.on_hover_text(tip(app, ui, "Click the triangle to play the scene (every track's clip in the row)", "ui.launch_scene"));
    resp.context_menu(|ui| {
        if ui.button("Rename").clicked() {
            app.abl.rename = Some((si, name.to_string()));
            ui.close();
        }
        if ui.button("Insert scene below").clicked() {
            let _ = app.run("scene.new", json!({"at": si + 1}));
            ui.close();
        }
        if ui.add_enabled(si > 0, egui::Button::new("Move up")).clicked() {
            let _ = app.run("scene.move", json!({"scene": si, "to": si - 1}));
            ui.close();
        }
        if ui.add_enabled(si + 1 < count, egui::Button::new("Move down")).clicked() {
            let _ = app.run("scene.move", json!({"scene": si, "to": si + 1}));
            ui.close();
        }
        if ui.button("Delete scene").clicked() {
            let _ = app.run("scene.delete", json!({"scene": si}));
            ui.close();
        }
    });
}

fn device_clip(s: &Session) -> Option<(TrackId, soundcraft_model::Clip)> {
    s.edit.selected_clips.iter().find_map(|c| s.find_clip(*c).map(|(t, c)| (t, c.clone())))
}

/// The track whose devices show: the clip's, else the first selected track.
fn device_track(s: &Session) -> Option<TrackId> {
    device_clip(s).map(|(t, _)| t).or_else(|| s.edit.selected_tracks.first().copied()).filter(|t| s.track(*t).is_some())
}

/// The Device View: the clip box on the left, the notes of a MIDI clip, and the track's devices.
fn device_view(app: &mut SoundApp, ui: &mut Ui) {
    let t = Tokens::current();
    let r = ui.max_rect();
    ui.expand_to_include_rect(r);
    let body = title(ui, r, "CLIP / DEVICE");
    let s = app.engine.session();
    let clip = device_clip(s);
    let track = device_track(s);
    let clip_w = 220.0;
    let device_w = 560.0f32.min(body.width() * 0.55);
    let clip_box = Rect::from_min_size(body.min, vec2(clip_w, body.height()));
    let device_box = Rect::from_min_max(pos2(body.max.x - device_w, body.min.y), body.max);
    let notes_box = Rect::from_min_max(pos2(clip_box.max.x, body.min.y), pos2(device_box.min.x, body.max.y));
    ui.painter().line_segment([pos2(clip_box.max.x, body.min.y), pos2(clip_box.max.x, body.max.y)], Stroke::new(1.0, t.border));
    ui.painter().line_segment([pos2(device_box.min.x, body.min.y), pos2(device_box.min.x, body.max.y)], Stroke::new(1.0, t.border));
    let mut c = ui.new_child(egui::UiBuilder::new().max_rect(clip_box.shrink(8.0)));
    clip_panel(app, &mut c, clip.clone());
    let midi = clip.as_ref().is_some_and(|(_, c)| matches!(c.content, ClipContent::Midi { .. }));
    if midi && notes_box.width() > 200.0 {
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(notes_box));
        child.set_clip_rect(notes_box.intersect(ui.clip_rect()));
        child.ctx().data_mut(|data| data.insert_temp(egui::Id::unique("soundcraft-midi-body"), notes_box));
        crate::docking::minimum_body(&mut child, vec2(200.0, 100.0), |ui| crate::midi_editor::show(app, ui));
    } else {
        let text = if clip.is_some() { "Audio clip: its notes come from Convert to MIDI." } else { "Select a clip to see it here." };
        ui.painter().text(notes_box.center(), Align2::CENTER_CENTER, text, regular(11.0), t.text_dim);
    }
    let mut d = ui.new_child(egui::UiBuilder::new().max_rect(device_box.shrink(8.0)));
    devices(app, &mut d, track);
}

/// The selected clip's settings: name, follow action, warp, groove, Convert to MIDI.
fn clip_panel(app: &mut SoundApp, ui: &mut Ui, clip: Option<(TrackId, soundcraft_model::Clip)>) {
    let t = Tokens::current();
    let Some((_track, c)) = clip else {
        ui.label(egui::RichText::new("No clip selected").font(regular(11.0)).color(t.text_dim));
        return;
    };
    ui.spacing_mut().item_spacing.y = 4.0;
    ui.label(egui::RichText::new(&c.name).font(bold(12.0)).color(t.text));
    let id = c.id.0;
    // Follow action.
    let label = c.follow.map_or("None", |f| f.action.label());
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Follow").font(regular(11.0)).color(t.text_dim));
        craft_ui::ComboBox::from_id_salt("abl_follow").width(100.0).selected_text(label).show_ui(ui, |ui| {
            if ui.selectable_label(c.follow.is_none(), "None").clicked() {
                let _ = app.run("clip.follow", json!({"clips": [id], "action": "none"}));
            }
            for a in FollowAction::ALL {
                if ui.selectable_label(c.follow.is_some_and(|f| f.action == a), a.label()).clicked() {
                    let plays = c.follow.map_or(1, |f| f.plays);
                    let _ = app.run("clip.follow", json!({"clips": [id], "action": a.id(), "plays": plays}));
                }
            }
        });
    });
    if let Some(f) = c.follow {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("after plays").font(regular(11.0)).color(t.text_dim));
            let mut plays = f.plays;
            if ui.add(egui::DragValue::new(&mut plays).range(1..=soundcraft_model::MAX_FOLLOW_PLAYS)).changed() {
                let _ = app.run("clip.follow", json!({"clips": [id], "action": f.action.id(), "plays": plays}));
            }
        });
    }
    if c.is_audio() {
        let mut warp = c.warp;
        if ui.checkbox(&mut warp, egui::RichText::new("Warp: follow the tempo").font(regular(11.0))).changed() {
            let _ = app.run("clip.warp", json!({"clips": [id], "value": warp}));
        }
        ui.label(egui::RichText::new("Convert to MIDI").font(regular(11.0)).color(t.text_dim));
        ui.horizontal(|ui| {
            for (label, mode) in [("Melody", "melody"), ("Harmony", "harmony"), ("Drums", "drums")] {
                if ui.button(egui::RichText::new(label).font(regular(11.0))).clicked() {
                    match app.run("clip.audio_to_midi", json!({"clip": id, "mode": mode})) {
                        Ok(v) => app.ui.status = format!("Converted to {} MIDI notes on a new track", v["notes"]),
                        Err(e) => app.ui.status = e,
                    }
                }
            }
        });
    } else {
        let grooves: Vec<(u64, String)> = app.engine.session().grooves.iter().map(|g| (g.id, g.name.clone())).collect();
        let current = c.groove_pool.and_then(|g| grooves.iter().find(|x| x.0 == g)).map_or("None", |x| x.1.as_str()).to_string();
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Groove").font(regular(11.0)).color(t.text_dim));
            craft_ui::ComboBox::from_id_salt("abl_groove").width(110.0).selected_text(current).show_ui(ui, |ui| {
                if ui.selectable_label(c.groove_pool.is_none(), "None").clicked() {
                    let _ = app.run("groove.apply", json!({"clips": [id], "groove": null}));
                }
                for (g, name) in &grooves {
                    if ui.selectable_label(c.groove_pool == Some(*g), name).clicked() {
                        let _ = app.run("groove.apply", json!({"clips": [id], "groove": g}));
                    }
                }
            });
        });
        if grooves.is_empty() {
            ui.label(egui::RichText::new("Add grooves in the Browser's Groove Pool.").font(regular(10.5)).color(t.text_dim));
        }
    }
}

/// A macro knob for a 0..1 value; returns the new value while it is turned.
fn knob(ui: &mut Ui, value: f32, name: &str) -> (egui::Response, Option<f32>) {
    let mut v = value * 2.0 - 1.0;
    let resp = crate::widgets::pan_knob(ui, 34.0, &mut v, name);
    let out = (resp.dragged() || resp.double_clicked()).then_some(((v + 1.0) / 2.0).clamp(0.0, 1.0));
    (resp, out)
}

/// A macro's right-click menu: rename it, and map or unmap the instrument's parameters.
fn macro_menu(app: &mut SoundApp, ui: &mut Ui, track: u64, i: usize, m: &soundcraft_model::Macro, info: Option<&soundcraft_dsp::PluginInfo>) {
    let mut name = m.name.clone();
    ui.horizontal(|ui| {
        ui.label("Name");
        if ui.text_edit_singleline(&mut name).changed() {
            let _ = app.run("rack.rename", json!({"track": track, "macro": i, "name": name}));
        }
    });
    ui.menu_button("Map parameter", |ui| {
        egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
            for pi in info.map_or(&[][..], |x| x.params) {
                let mapped = m.targets.iter().any(|x| x.param == pi.id);
                if ui.add_enabled(!mapped, egui::Button::new(pi.name)).clicked() {
                    if let Err(e) = app.run("rack.map", json!({"track": track, "macro": i, "param": pi.id})) {
                        app.ui.status = e;
                    }
                    ui.close();
                }
            }
        });
    });
    ui.add_enabled_ui(!m.targets.is_empty(), |ui| {
        ui.menu_button("Unmap parameter", |ui| {
            for x in &m.targets {
                let label = info.and_then(|i| i.params.iter().find(|p| p.id == x.param)).map_or(x.param.as_str(), |p| p.name);
                if ui.button(label).clicked() {
                    let _ = app.run("rack.unmap", json!({"track": track, "macro": i, "param": x.param}));
                    ui.close();
                }
            }
        });
    });
}

/// The track's devices: the instrument, as a Drum Rack's pads or an Instrument Rack's macros.
fn devices(app: &mut SoundApp, ui: &mut Ui, track: Option<TrackId>) {
    let t = Tokens::current();
    let s = app.engine.session();
    let Some(tr) = track.and_then(|id| s.track(id)).cloned() else {
        ui.label(egui::RichText::new("Select a track to see its devices").font(regular(11.0)).color(t.text_dim));
        return;
    };
    let Some(ins) = tr.instrument.clone() else {
        ui.label(egui::RichText::new(format!("{}: no instrument", tr.name)).font(regular(11.0)).color(t.text_dim));
        return;
    };
    let info = soundcraft_dsp::plugin_info(&ins.plugin);
    let name = info.map_or(ins.plugin.as_str(), |i| i.name);
    ui.label(egui::RichText::new(format!("{} — {name}", tr.name)).font(bold(12.0)).color(t.text));
    if ins.plugin == "drum_rack" {
        drum_rack(app, ui, tr.id, &ins);
        return;
    }
    match &tr.rack {
        Some(rack) => {
            ui.label(egui::RichText::new("INSTRUMENT RACK").font(bold(10.5)).color(t.text_dim));
            ui.horizontal_wrapped(|ui| {
                for (i, m) in rack.macros.iter().enumerate() {
                    ui.vertical(|ui| {
                        ui.set_width(56.0);
                        let (resp, changed) = knob(ui, m.value, &m.name);
                        let tid = tr.id.0;
                        resp.on_hover_text(format!("{}: right-click to rename or map parameters", m.name)).context_menu(|ui| {
                            macro_menu(app, ui, tid, i, m, info);
                        });
                        if let Some(v) = changed {
                            let key = format!("rack.macro.{}.{i}", tr.id.0);
                            let _ = app.engine.execute_merged("rack.macro", &json!({"track": tr.id.0, "macro": i, "value": v}), &key);
                        }
                        let short: String = m.name.chars().take(10).collect();
                        ui.label(egui::RichText::new(short).font(regular(9.5)).color(t.text_dim));
                    });
                }
            });
            if ui.small_button("Remove Instrument Rack").on_hover_text("The instrument stays as it is").clicked() {
                let _ = app.run("rack.remove", json!({"track": tr.id.0}));
            }
        }
        None => {
            if ui.button("Group into Instrument Rack (8 macros)").clicked()
                && let Err(e) = app.run("rack.create", json!({"track": tr.id.0}))
            {
                app.ui.status = e;
            }
        }
    }
}

/// A Drum Rack: sixteen pads in a 4×4 grid (the lowest note bottom left), and the picked pad's
/// sound, tune and level.
fn drum_rack(app: &mut SoundApp, ui: &mut Ui, track: TrackId, ins: &soundcraft_model::Insert) {
    let t = Tokens::current();
    let param = |id: &str, d: f32| ins.params.get(id).copied().unwrap_or(d);
    let sound = |pad: usize| {
        let d = soundcraft_dsp::plugin_info("drum_rack")
            .and_then(|i| i.params.iter().find(|p| p.id == format!("p{}_sound", pad + 1)))
            .map_or(0.0, |p| p.default);
        let i = param(&format!("p{}_sound", pad + 1), d).round().max(0.0) as usize;
        soundcraft_dsp::plugins::drums::PAD_SOUNDS.get(i).copied().unwrap_or("Off")
    };
    ui.horizontal(|ui| {
        let (grid, _) = ui.allocate_exact_size(vec2(4.0 * 66.0, 4.0 * 34.0), Sense::hover());
        for pad in 0..16usize {
            let (col, row) = (pad % 4, 3 - pad / 4);
            let r = Rect::from_min_size(grid.min + vec2(col as f32 * 66.0, row as f32 * 34.0), vec2(64.0, 32.0));
            let resp = ui.interact(r, ui.scope_id().with(("abl_pad", pad)), Sense::click());
            let on = app.abl.pad == pad;
            ui.painter().rect_filled(r, 3.0, if on { t.accent_dark } else { t.button });
            ui.painter().text(pos2(r.min.x + 4.0, r.min.y + 9.0), Align2::LEFT_CENTER, sound(pad), regular(9.5), t.text);
            let note = 36 + pad;
            ui.painter().text(pos2(r.min.x + 4.0, r.max.y - 8.0), Align2::LEFT_CENTER, format!("{note}"), mono(9.0), t.text_dim);
            if resp.on_hover_text("Pick the pad to set its sound, tune and level").clicked() {
                app.abl.pad = pad;
            }
        }
        ui.vertical(|ui| {
            let n = app.abl.pad + 1;
            ui.label(egui::RichText::new(format!("PAD {n} (note {})", 35 + n)).font(bold(10.5)).color(t.text_dim));
            let current = sound(app.abl.pad);
            craft_ui::ComboBox::from_id_salt("abl_pad_sound").width(110.0).selected_text(current).show_ui(ui, |ui| {
                for (i, name) in soundcraft_dsp::plugins::drums::PAD_SOUNDS.iter().enumerate() {
                    if ui.selectable_label(*name == current, *name).clicked() {
                        let _ = app.run("mix.instrument_param", json!({"track": track.0, "param": format!("p{n}_sound"), "value": i}));
                    }
                }
            });
            for (key, label, lo, hi) in [("tune", "Tune (st)", -24.0f32, 24.0f32), ("level", "Level (dB)", -48.0, 6.0)] {
                let id = format!("p{n}_{key}");
                let mut v = param(&id, 0.0);
                if ui.add(egui::Slider::new(&mut v, lo..=hi).text(label)).changed() {
                    let p = json!({"track": track.0, "param": id, "value": v});
                    let _ = app.engine.execute_merged("mix.instrument_param", &p, &format!("abl.pad.{}.{id}", track.0));
                }
            }
        });
    });
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
    fn the_ableton_layout_is_selectable_with_its_panels() {
        let mut app = app();
        assert!(LayoutPreset::AbletonStyle.available());
        assert_eq!(app.run("ui.layout", json!({"preset": "abletonStyle"})), Ok(json!({"preset": "abletonStyle"})));
        assert!(active(&app));
        assert_eq!(LayoutPreset::parse(LABEL), Some(LayoutPreset::AbletonStyle));
        assert!(app.ui.abl_browser_open && app.ui.abl_session_open && app.ui.abl_device_open);
        app.run("ui.browser", json!({})).unwrap();
        assert!(!app.ui.abl_browser_open, "the Ableton Browser, not FL's");
        assert!(app.ui.fl_browser_open);
        app.run("ui.session_view", json!({})).unwrap();
        assert!(!app.ui.abl_session_open);
    }

    #[test]
    fn switching_through_every_layout_and_back_keeps_the_session() {
        let mut app = app();
        app.engine.execute("session.new", &json!({"template": "drums"})).unwrap();
        app.engine.execute("scene.new", &json!({})).unwrap();
        let before = app.engine.session().clone();
        for preset in [
            "abletonStyle",
            "new",
            "abletonStyle",
            "classic",
            "abletonStyle",
            "reaper",
            "abletonStyle",
            "flStyle",
            "abletonStyle",
            "proToolsStyle",
            "abletonStyle",
            "new",
        ] {
            app.run("ui.layout", json!({"preset": preset})).unwrap();
            assert_eq!(*app.engine.session(), before, "after switching to {preset}");
        }
    }

    #[test]
    fn enter_launches_the_selected_scene_and_launched_clips_stop_with_the_transport() {
        let mut app = app();
        app.ui.layout = LayoutPreset::AbletonStyle;
        let t = app.engine.session_mut().add_track(soundcraft_model::TrackKind::Midi, soundcraft_model::ChannelFormat::Mono, None);
        app.run("scene.new", json!({})).unwrap();
        app.run("scene.new", json!({})).unwrap();
        app.run("slot.set", json!({"track": t.0, "scene": 1})).unwrap();
        app.abl.scene = 1;
        app.run("ui.launch_scene", json!({})).unwrap();
        assert_eq!(app.engine.session().launch.len(), 1);
        app.abl.was_playing = true;
        app.engine.transport.playing = false;
        tick(&mut app);
        assert!(app.engine.session().launch.is_empty(), "stopping the transport stops the clips");
    }

    #[test]
    fn delete_deletes_the_selected_slot_clip() {
        let mut app = app();
        app.ui.layout = LayoutPreset::AbletonStyle;
        let t = app.engine.session_mut().add_track(soundcraft_model::TrackKind::Midi, soundcraft_model::ChannelFormat::Mono, None);
        app.run("scene.new", json!({})).unwrap();
        let c = app.run("slot.set", json!({"track": t.0, "scene": 0})).unwrap()["clip"].as_u64().unwrap();
        app.run("edit.select", json!({"clips": [c]})).unwrap();
        assert_eq!(selected_slot(app.engine.session()), Some((t, 0)));
        assert!(keys(&mut app, Key::Delete, Modifiers::NONE));
        assert!(app.engine.session().track(t).unwrap().slot(0).is_none());
    }
}
