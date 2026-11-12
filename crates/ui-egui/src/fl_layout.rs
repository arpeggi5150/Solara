//! The FL-style layout preset: the Browser and the Channel Rack on the left, the Playlist (pattern
//! clips and automation clips) in the centre, the Piano Roll and the Mixer along the bottom. Same
//! engine, session and commands as the other layouts; only the panels, the words and the keys
//! change (the keys live in `keymap`). Learnt from how such a DAW behaves, not from its code or
//! artwork.

use crate::SoundApp;
use crate::edit_window::{rect, sample_at, x_of};
use crate::new_layout::{LayoutPreset, MIXER_COLLAPSED_H, TOP_BAR_H};
use crate::theme::{Tokens, bold, mono, regular};
use crate::widgets::icon_button;
use egui::{Align2, Color32, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use serde_json::{Value, json};
use soundcraft_engine::cmd::piano_roll::CHORDS;
use soundcraft_model::{AutoClip, AutoParam, ClipContent, ClipId, Session, TrackId};
use soundcraft_time::{Samples, TimeFormat};

/// Default widths and heights of the side and bottom panels.
pub const BROWSER_W: f32 = 200.0;
pub const RACK_W: f32 = 470.0;
pub const ROLL_H: f32 = 260.0;
/// Height of a panel's title row.
pub const TITLE_H: f32 = 24.0;
/// Height of an automation clip's band at the bottom of its track row.
pub const AUTO_H: f32 = 18.0;

/// What the FL-style layout remembers between frames (not saved).
#[derive(Debug, Clone, Default)]
pub struct FlState {
    /// The chord the piano roll's pencil stamps, if any.
    pub stamp: Option<&'static str>,
    /// An automation clip being dragged in the Playlist.
    pub auto_drag: Option<AutoDrag>,
    /// The pointer was over the piano roll last frame (its Delete deletes notes).
    pub roll_hovered: bool,
}

/// An automation clip drag: which clip, where in it the pointer took hold, and whether it copies.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AutoDrag {
    pub id: u64,
    pub grab: Samples,
    pub copy: bool,
}

/// A Browser item on its way to the Playlist or the Channel Rack.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Item {
    Pattern(ClipId),
    Sound(u8),
    Instrument(&'static str),
}

fn drag_id() -> egui::Id {
    egui::Id::unique("solara-fl-drag")
}

/// Whether the FL-style layout is showing.
pub fn active(app: &SoundApp) -> bool {
    app.ui.layout == LayoutPreset::FlStyle
}

/// The FL-style layout's UI commands: its panels and the piano roll tools. None for other ids.
pub fn command(app: &mut SoundApp, id: &str, p: &Value) -> Option<Result<Value, String>> {
    let toggle = |v: &mut bool| {
        *v = p.get("value").and_then(Value::as_bool).unwrap_or(!*v);
        Ok(json!({"value": *v}))
    };
    Some(match id {
        "ui.channel_rack" => toggle(&mut app.ui.fl_rack_open),
        "ui.piano_roll" => toggle(&mut app.ui.fl_roll_open),
        "ui.browser" => toggle(&mut app.ui.fl_browser_open),
        "ui.roll_slide" => roll_tool(app, "midi.slide", json!({})),
        "ui.roll_strum" => roll_tool(app, "midi.strum", json!({})),
        "ui.roll_arpeggiate" => roll_tool(app, "midi.arpeggiate", json!({})),
        _ => return None,
    })
}

/// Run a piano roll tool on the notes selected in the roll.
fn roll_tool(app: &mut SoundApp, cmd: &str, mut p: Value) -> Result<Value, String> {
    let clip = crate::midi_editor::target_clip(app).ok_or("Open a pattern or MIDI clip in the piano roll first")?;
    if app.midi.selected.is_empty() {
        return Err("Select notes in the piano roll first".into());
    }
    p["clip"] = json!(clip.0);
    p["indices"] = json!(app.midi.selected);
    let r = app.run(cmd, p)?;
    // Strum and arpeggiate rewrite the notes: keep the new ones selected.
    if let Some(ix) = r.get("indices").and_then(|v| serde_json::from_value::<Vec<usize>>(v.clone()).ok()) {
        app.midi.selected = ix;
    }
    Ok(r)
}

/// Draw the FL-style layout.
pub fn show(app: &mut SoundApp, ui: &mut Ui) {
    let t = Tokens::current();
    crate::edit_window::drive_drag(app, ui.ctx());
    app.fl.roll_hovered = false;
    egui::Panel::top("fl_top_bar").exact_size(TOP_BAR_H).frame(egui::Frame::NONE.fill(t.toolbar_bg)).show(ui, |ui| top_bar(app, ui));
    let room = ui.available_rect_before_wrap().height();
    let mixer_h = if app.ui.fl_mixer_open { crate::reaper_layout::open_mixer_height(room) } else { MIXER_COLLAPSED_H };
    egui::Panel::bottom("fl_mixer").exact_size(mixer_h).frame(egui::Frame::NONE.fill(t.panel_bg)).show(ui, |ui| mixer(app, ui));
    if app.ui.fl_roll_open && !app.ui.fl_mixer_open {
        egui::Panel::bottom("fl_piano_roll")
            .resizable(true)
            .default_size(ROLL_H)
            .size_range(140.0..=640.0)
            .frame(egui::Frame::NONE.fill(t.panel_bg))
            .show(ui, |ui| piano_roll(app, ui));
    }
    if app.ui.fl_browser_open {
        egui::Panel::left("fl_browser")
            .resizable(true)
            .default_size(BROWSER_W)
            .size_range(150.0..=360.0)
            .frame(egui::Frame::NONE.fill(t.panel_bg))
            .show(ui, |ui| browser(app, ui));
    }
    if app.ui.fl_rack_open {
        egui::Panel::left("fl_channel_rack")
            .resizable(true)
            .default_size(RACK_W)
            .size_range(320.0..=900.0)
            .frame(egui::Frame::NONE.fill(t.panel_bg))
            .show(ui, |ui| channel_rack(app, ui));
    }
    egui::CentralPanel::default().frame(egui::Frame::NONE.fill(t.window_bg)).show(ui, |ui| {
        let full = ui.max_rect();
        let mut playlist = ui.new_child(egui::UiBuilder::new().max_rect(full));
        playlist.set_clip_rect(full.intersect(ui.clip_rect()));
        crate::edit_window::main_area(app, &mut playlist);
        auto_clips(app, ui, full);
        playlist_drop(app, ui, full);
    });
    // A Browser drag let go anywhere else is over.
    if ui.input(|i| i.pointer.any_released()) {
        ui.ctx().data_mut(|d| d.remove::<Item>(drag_id()));
    }
}

/// The top bar: play controls, position and tempo, the layout switcher, and the panel buttons.
fn top_bar(app: &mut SoundApp, ui: &mut Ui) {
    let t = Tokens::current();
    let r = ui.max_rect();
    ui.painter().line_segment([pos2(r.min.x, r.max.y - 0.5), pos2(r.max.x, r.max.y - 0.5)], Stroke::new(1.0, t.border));
    let mut row = ui.new_child(egui::UiBuilder::new().max_rect(r.shrink2(vec2(10.0, 6.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
    let ui = &mut row;
    ui.spacing_mut().item_spacing.x = 6.0;
    let (bars, time, bpm, looped, playing, recording) = {
        let s = app.engine.session();
        let at = app.position();
        let tick = s.tempo.samples_to_ticks(at, s.sample_rate);
        (
            s.format_position(at, TimeFormat::BarsBeats),
            s.format_position(at, TimeFormat::MinSecs),
            s.tempo.tempo_at_tick(tick),
            s.edit.loop_playback,
            app.is_playing(),
            app.engine.transport.recording,
        )
    };
    let size = vec2(30.0, 28.0);
    let mac = ui.ctx().os().is_mac();
    let tip = |app: &SoundApp, label: &str, id: &str| match crate::keymap::chord_for(app, id) {
        Some(c) => format!("{label} ({})", crate::keymap::display(&c, mac)),
        None => label.to_string(),
    };
    let buttons = [
        ("play", playing, "Play (Space)".to_string(), "transport.play"),
        ("stop", false, "Stop (Space)".to_string(), "transport.stop"),
        ("record", recording, tip(app, "Record", "transport.record"), "transport.record"),
        ("loop", looped, tip(app, "Repeat", "options.loop_playback"), "options.loop_playback"),
    ];
    for (icon, on, tip, id) in buttons {
        if icon_button(ui, size, icon, on, &tip).clicked() {
            let _ = app.run(id, json!({}));
        }
    }
    ui.add_space(8.0);
    readout(ui, "Song position", |ui| {
        ui.label(egui::RichText::new(format!("{bars} / {time}")).font(mono(13.0)).color(t.counter_text));
    });
    ui.add_space(6.0);
    readout(ui, "Tempo", |ui| crate::toolbar::tempo_field(app, ui, bpm));
    ui.add_space(8.0);
    crate::new_layout::switcher(app, ui);
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let panels = [
            ("Mixer", "ui.mixer", app.ui.fl_mixer_open),
            ("Piano Roll", "ui.piano_roll", app.ui.fl_roll_open),
            ("Channel Rack", "ui.channel_rack", app.ui.fl_rack_open),
            ("Browser", "ui.browser", app.ui.fl_browser_open),
        ];
        for (label, id, on) in panels {
            let hover = tip(app, &format!("Show or hide the {label}"), id);
            if ui.add(egui::Button::selectable(on, egui::RichText::new(label).font(regular(12.0)))).on_hover_text(hover).clicked() {
                let _ = app.run(id, json!({}));
            }
        }
        ui.add_space(6.0);
        let hover = "A volume automation clip on the selected track, over the selection (one bar if none). Drag it to move, Ctrl+drag to copy.";
        if ui.add(egui::Button::new(egui::RichText::new("+ Automation clip").font(regular(12.0)))).on_hover_text(hover).clicked()
            && let Err(e) = new_auto_clip(app)
        {
            app.ui.status = e;
        }
    });
}

/// A volume automation clip on the selected (else first) track.
fn new_auto_clip(app: &mut SoundApp) -> Result<Value, String> {
    let s = app.engine.session();
    let track = s.edit.selected_tracks.first().copied().filter(|id| s.track(*id).is_some()).or_else(|| s.tracks.first().map(|t| t.id));
    let track = track.ok_or("Add a track first")?;
    let sel = s.edit.selection;
    app.run("auto_clip.new", json!({"track": track.0, "param": "volume", "start": sel.start, "end": sel.end}))
}

/// A label above a value, in the top bar.
fn readout(ui: &mut Ui, label: &str, value: impl FnOnce(&mut Ui)) {
    let t = Tokens::current();
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        ui.label(egui::RichText::new(label).font(regular(9.5)).color(t.counter_label));
        value(ui);
    });
}

/// A panel's title row with its top border, and the rest of the panel below it.
fn title(ui: &Ui, r: Rect, text: &str) -> Rect {
    let t = Tokens::current();
    ui.painter().line_segment([pos2(r.min.x, r.min.y + 0.5), pos2(r.max.x, r.min.y + 0.5)], Stroke::new(1.0, t.border));
    let row = Rect::from_min_size(r.min, vec2(r.width(), TITLE_H));
    ui.painter().text(pos2(row.min.x + 10.0, row.center().y), Align2::LEFT_CENTER, text, bold(11.0), t.text_dim);
    Rect::from_min_max(pos2(r.min.x, row.max.y), r.max)
}

/// The Browser: the session's patterns (drag one into the Playlist), sounds and instruments.
fn browser(app: &mut SoundApp, ui: &mut Ui) {
    let t = Tokens::current();
    let r = ui.max_rect();
    ui.expand_to_include_rect(r);
    let body = title(ui, r, "BROWSER");
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(body.shrink2(vec2(8.0, 4.0))));
    let ui = &mut child;
    ui.add(egui::TextEdit::singleline(&mut app.browser_search).hint_text("Search by name").desired_width(f32::INFINITY));
    let needle = app.browser_search.trim().to_lowercase();
    let current = crate::midi_editor::target_clip(app);
    let patterns = soundcraft_engine::cmd::playlist::list(app.engine.session());
    let plugins = soundcraft_dsp::plugins();
    egui::ScrollArea::vertical().id_salt("fl_browser_scroll").auto_shrink([false, false]).show(ui, |ui| {
        heading(ui, "PATTERNS");
        if patterns.is_empty() {
            ui.label(egui::RichText::new("Step a sound in the Channel Rack to make one.").font(regular(11.0)).color(t.text_dim));
        }
        for p in &patterns {
            let label = if p.places > 1 { format!("{}  ×{}", p.name, p.places) } else { p.name.clone() };
            let resp = row(ui, &needle, &label, Item::Pattern(p.clip), current == Some(p.clip));
            if let Some(resp) = resp {
                if resp.clicked() {
                    let _ = app.run("edit.select", json!({"clips": [p.clip.0], "tracks": [p.track.0]}));
                }
                // The selected place of a pattern placed more than once can become a pattern of its own.
                let selected = current.filter(|c| p.places > 1 && same_pattern(app.engine.session(), *c, p.clip));
                resp.context_menu(|ui| {
                    let unique = ui.add_enabled(selected.is_some(), egui::Button::new("Make the selected place unique"));
                    if unique.clicked() {
                        if let Some(c) = selected
                            && let Err(e) = app.run("playlist.make_unique", json!({"clip": c.0}))
                        {
                            app.ui.status = e;
                        }
                        ui.close();
                    }
                });
            }
        }
        heading(ui, "SOUNDS");
        for (pitch, name) in soundcraft_engine::cmd::pattern::SOUNDS {
            row(ui, &needle, name, Item::Sound(pitch), false);
        }
        heading(ui, "INSTRUMENTS");
        for info in plugins.iter().filter(|p| p.category == soundcraft_dsp::Category::Instrument) {
            row(ui, &needle, info.name, Item::Instrument(info.id), false);
        }
    });
}

/// Whether clips `a` and `b` are places of one pattern.
fn same_pattern(s: &Session, a: ClipId, b: ClipId) -> bool {
    let link = |c: ClipId| s.find_clip(c).and_then(|(_, x)| x.pattern_link);
    a == b || link(a).is_some_and(|l| link(b) == Some(l))
}

fn heading(ui: &mut Ui, text: &str) {
    let t = Tokens::current();
    ui.add_space(6.0);
    ui.label(egui::RichText::new(text).font(bold(10.5)).color(t.text_dim));
}

/// One Browser row: click to pick, drag to place.
fn row(ui: &mut Ui, needle: &str, name: &str, item: Item, on: bool) -> Option<egui::Response> {
    if !needle.is_empty() && !name.to_lowercase().contains(needle) {
        return None;
    }
    let t = Tokens::current();
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 20.0), Sense::click_and_drag());
    if on {
        ui.painter().rect_filled(rect, 3.0, t.accent_dark);
    } else if resp.hovered() || resp.dragged() {
        ui.painter().rect_filled(rect, 3.0, t.button_hi);
    }
    ui.painter().text(pos2(rect.min.x + 6.0, rect.center().y), Align2::LEFT_CENTER, name, regular(12.0), if on { Color32::WHITE } else { t.text });
    if resp.drag_started() {
        ui.ctx().data_mut(|d| d.insert_temp(drag_id(), item));
    }
    let hint = match item {
        Item::Pattern(_) => "Click to edit it in the Channel Rack and Piano Roll; drag it into the Playlist to play it there",
        Item::Sound(_) => "Drag onto the Channel Rack or a track to step it in a pattern",
        Item::Instrument(_) => "Drag into the Playlist for a new instrument track",
    };
    Some(resp.on_hover_text(hint))
}

/// The track the Channel Rack shows: the picked pattern's, else the first selected MIDI track, else
/// the first Drum Synth track, else the first MIDI track.
pub fn rack_track(app: &SoundApp) -> Option<TrackId> {
    let s = app.engine.session();
    let midi = |id: &TrackId| s.track(*id).is_some_and(|t| t.kind.is_midi());
    if let Some(t) = crate::midi_editor::target_clip(app).and_then(|c| s.find_clip(c)).map(|(t, _)| t) {
        return Some(t);
    }
    s.edit.selected_tracks.iter().copied().find(midi).or_else(|| {
        let drums = s.tracks.iter().find(|t| t.instrument.as_ref().is_some_and(|i| i.plugin == crate::new_layout::DRUM_SYNTH));
        drums.or_else(|| s.tracks.iter().find(|t| t.kind.is_midi())).map(|t| t.id)
    })
}

/// The Channel Rack: the pattern's swing and humanize over the step grid of its sounds.
fn channel_rack(app: &mut SoundApp, ui: &mut Ui) {
    let t = Tokens::current();
    let r = ui.max_rect();
    ui.expand_to_include_rect(r);
    let body = title(ui, r, "CHANNEL RACK");
    let track = rack_track(app);
    let s = app.engine.session_arc();
    let pattern = track.and_then(|t| crate::step_seq::focused_pattern(&s, t));
    let groove = pattern.and_then(|c| s.find_clip(c)).and_then(|(_, c)| c.groove).unwrap_or_default();
    // Swing and humanize, right of the title.
    let knobs = Rect::from_min_max(pos2(r.min.x + 130.0, r.min.y + 2.0), pos2(r.max.x - 8.0, r.min.y + TITLE_H - 2.0));
    let mut row = ui.new_child(egui::UiBuilder::new().max_rect(knobs).layout(egui::Layout::left_to_right(egui::Align::Center)));
    row.spacing_mut().item_spacing.x = 6.0;
    row.spacing_mut().slider_width = 70.0;
    row.add_enabled_ui(pattern.is_some(), |ui| {
        for (key, label, now) in [("swing", "Swing", groove.swing), ("humanize", "Humanize", groove.humanize)] {
            ui.label(egui::RichText::new(label).font(regular(11.0)).color(t.text_dim));
            let mut v = (now * 100.0).round();
            let resp = ui.add(egui::Slider::new(&mut v, 0.0..=100.0).show_value(true).suffix("%").step_by(1.0)).on_hover_text(match key {
                "swing" => "Delays every second sixteenth: more swing, more shuffle",
                _ => "Moves each hit's timing and loudness a little, the same way every play",
            });
            if resp.changed()
                && let Some(c) = pattern
                && let Err(e) = app.engine.execute_merged("rack.groove", &json!({"clip": c.0, key: v}), &format!("groove:{}:{key}", c.0))
            {
                app.ui.status = e.to_string();
            }
            if resp.drag_stopped() {
                app.engine.end_merge();
            }
        }
    });
    let mut grid = ui.new_child(egui::UiBuilder::new().max_rect(body));
    grid.set_clip_rect(body.intersect(ui.clip_rect()));
    crate::step_seq::show(app, &mut grid, track);
    // A sound dropped here joins the rack's pattern.
    if let Some(Item::Sound(pitch)) = ui.ctx().data(|d| d.get_temp::<Item>(drag_id()))
        && ui.input(|i| i.pointer.any_released())
        && ui.input(|i| i.pointer.interact_pos()).is_some_and(|p| r.contains(p))
    {
        let mut p = json!({"pitch": pitch});
        if let Some(tr) = track {
            p["track"] = json!(tr.0);
        }
        if let Err(e) = app.run("pattern.add_sound", p) {
            app.ui.status = e;
        }
        ui.ctx().data_mut(|d| d.remove::<Item>(drag_id()));
    }
}

/// The Piano Roll with the FL-style tools: stamp, slide, strum, arpeggiate and ghost notes.
fn piano_roll(app: &mut SoundApp, ui: &mut Ui) {
    let r = ui.max_rect();
    ui.expand_to_include_rect(r);
    let body = title(ui, r, "PIANO ROLL");
    let tools = Rect::from_min_max(pos2(r.min.x + 110.0, r.min.y + 2.0), pos2(r.max.x - 8.0, r.min.y + TITLE_H - 2.0));
    let mut row = ui.new_child(egui::UiBuilder::new().max_rect(tools).layout(egui::Layout::left_to_right(egui::Align::Center)));
    roll_tools(app, &mut row);
    app.fl.roll_hovered = ui.rect_contains_pointer(body);
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(body));
    child.set_clip_rect(body.intersect(ui.clip_rect()));
    child.ctx().data_mut(|data| data.insert_temp(egui::Id::unique("soundcraft-midi-body"), body));
    crate::docking::minimum_body(&mut child, vec2(420.0, 120.0), |ui| crate::midi_editor::show(app, ui));
}

fn roll_tools(app: &mut SoundApp, ui: &mut Ui) {
    let t = Tokens::current();
    ui.spacing_mut().item_spacing.x = 4.0;
    let mac = ui.ctx().os().is_mac();
    let key =
        |app: &SoundApp, id: &str| crate::keymap::chord_for(app, id).map(|c| format!(" ({})", crate::keymap::display(&c, mac))).unwrap_or_default();
    ui.label(egui::RichText::new("Stamp").font(regular(11.0)).color(t.text_dim));
    let name = app.fl.stamp.and_then(|id| CHORDS.iter().find(|c| c.0 == id)).map_or("Off", |c| c.1);
    craft_ui::ComboBox::from_id_salt("fl_stamp")
        .width(110.0)
        .selected_text(name)
        .show_ui(ui, |ui| {
            if ui.selectable_label(app.fl.stamp.is_none(), "Off").clicked() {
                app.fl.stamp = None;
            }
            for (id, label, _) in CHORDS {
                if ui.selectable_label(app.fl.stamp == Some(*id), *label).clicked() {
                    app.fl.stamp = Some(*id);
                }
            }
        })
        .response
        .on_hover_text("With a chord picked, a click in the roll writes the whole chord");
    ui.separator();
    let tools = [
        ("Slide", "ui.roll_slide", "The selected notes glide from the note before them"),
        ("Strum", "ui.roll_strum", "Spread each selected chord like a strummed guitar"),
        ("Arpeggiate", "ui.roll_arpeggiate", "Play each selected chord one note at a time"),
    ];
    for (label, id, hover) in tools {
        let hover = format!("{hover}{}", key(app, id));
        if ui.add(egui::Button::new(egui::RichText::new(label).font(regular(11.5)))).on_hover_text(hover).clicked()
            && let Some(Err(e)) = command(app, id, &json!({}))
        {
            app.ui.status = e;
        }
    }
    ui.separator();
    ui.checkbox(&mut app.ui.fl_ghost_notes, egui::RichText::new("Ghost notes").font(regular(11.5)))
        .on_hover_text("Show, faded, the notes other tracks play at the same time");
}

/// The Mixer along the bottom: a header row that shows or hides it, and the strips.
fn mixer(app: &mut SoundApp, ui: &mut Ui) {
    let t = Tokens::current();
    let r = ui.max_rect();
    ui.painter().line_segment([pos2(r.min.x, r.min.y + 0.5), pos2(r.max.x, r.min.y + 0.5)], Stroke::new(1.0, t.border));
    let header = Rect::from_min_size(r.min, vec2(r.width(), MIXER_COLLAPSED_H));
    let resp = ui.interact(header, ui.scope_id().with("fl_mixer_header"), Sense::click());
    let open = app.ui.fl_mixer_open;
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
            .id_salt("fl_mixer_scroll")
            .auto_shrink([false, false])
            .show(&mut child, |ui| crate::mix_window::content(app, ui));
    }
}

/// Where an automation clip's band sits in its track row, or None when the track has no row.
fn band(app: &SoundApp, s: &Session, c: &AutoClip, start: Samples, track: TrackId) -> Option<Rect> {
    let tl = rect(app.edit_layout.timeline);
    let (_, row) = app.edit_layout.rows.iter().find(|(id, _)| *id == track.0)?;
    let row = rect(*row);
    let y1 = row.max.y - 2.0;
    let x0 = x_of(s, tl, start);
    let x1 = x_of(s, tl, start.saturating_add(c.length));
    Some(Rect::from_min_max(pos2(x0, (y1 - AUTO_H).max(row.min.y)), pos2(x1.max(x0 + 2.0), y1)))
}

/// A value of `param` as a height fraction in its band (volume on the fader's scale).
fn level(param: &AutoParam, v: f32) -> f32 {
    let (lo, hi, _) = param.range();
    let f = match param {
        AutoParam::Volume | AutoParam::SendLevel(_) => soundcraft_model::fader_db_to_pos(v),
        AutoParam::Plugin { .. } => 0.5,
        _ => (v - lo) / (hi - lo).max(f32::EPSILON),
    };
    if f.is_finite() { f.clamp(0.0, 1.0) } else { 0.5 }
}

fn draw_auto_clip(ui: &Ui, clip_to: Rect, b: Rect, c: &AutoClip, ghost: bool) {
    let t = Tokens::current();
    let painter = ui.painter_at(clip_to);
    let alpha = if ghost { 70 } else { 150 };
    let [r, g, bl, _] = t.auto_read.to_array();
    painter.rect(
        b,
        3.0,
        Color32::from_rgba_unmultiplied(r, g, bl, alpha / 3),
        Stroke::new(1.0, Color32::from_rgba_unmultiplied(r, g, bl, alpha)),
        StrokeKind::Inside,
    );
    let inner = b.shrink2(vec2(1.0, 3.0));
    let pts: Vec<egui::Pos2> = c
        .points
        .iter()
        .map(|p| {
            let x = inner.min.x + inner.width() * (p.at as f32 / c.length.max(1) as f32);
            pos2(x, inner.max.y - inner.height() * level(&c.param, p.value))
        })
        .collect();
    painter.add(egui::Shape::line(pts, Stroke::new(1.5, t.auto_read)));
    painter.text(pos2(b.min.x + 4.0, b.min.y + 1.0), Align2::LEFT_TOP, &c.name, regular(9.5), t.text);
}

/// The automation clips in the Playlist: a band at the foot of each clip's track row. Drag to
/// move, Ctrl+drag to copy; right-click to delete.
fn auto_clips(app: &mut SoundApp, ui: &mut Ui, area: Rect) {
    let s = app.engine.session_arc();
    if s.auto_clips.is_empty() {
        return;
    }
    let tl = rect(app.edit_layout.timeline);
    let view = tl.intersect(area);
    let pointer = ui.input(|i| i.pointer.interact_pos());
    let copy_mod = ui.input(|i| i.modifiers.command || i.modifiers.ctrl);
    for c in &s.auto_clips {
        let Some(b) = band(app, &s, c, c.start, c.track) else { continue };
        if !b.intersects(view) {
            continue;
        }
        draw_auto_clip(ui, view, b, c, false);
        let resp = ui.interact(b.intersect(view), ui.scope_id().with(("fl_auto", c.id)), Sense::click_and_drag());
        let resp = resp.on_hover_text(format!("{} · drag to move, Ctrl+drag to copy, right-click to delete", c.name));
        if resp.drag_started()
            && let Some(p) = pointer
        {
            app.fl.auto_drag = Some(AutoDrag { id: c.id, grab: sample_at(&s, tl, p.x).saturating_sub(c.start), copy: copy_mod });
        }
        let dragging = app.fl.auto_drag.filter(|d| d.id == c.id);
        if let (Some(d), Some(p)) = (dragging, pointer) {
            let at = sample_at(&s, tl, p.x).saturating_sub(d.grab).max(0);
            let track = row_at(app, p).unwrap_or(c.track);
            if let Some(g) = band(app, &s, c, at, track) {
                draw_auto_clip(ui, view, g, c, true);
            }
            if resp.drag_stopped() {
                app.fl.auto_drag = None;
                let cmd = if d.copy { "auto_clip.copy" } else { "auto_clip.move" };
                if let Err(e) = app.run(cmd, json!({"id": c.id, "at": at, "track": track.0})) {
                    app.ui.status = e;
                }
            }
        }
        resp.context_menu(|ui| {
            if ui.button("Delete automation clip").clicked() {
                if let Err(e) = app.run("auto_clip.delete", json!({"id": c.id})) {
                    app.ui.status = e;
                }
                ui.close();
            }
        });
    }
    if app.fl.auto_drag.is_some() && !ui.input(|i| i.pointer.any_down()) {
        app.fl.auto_drag = None;
    }
}

/// The track whose Playlist row is under `p`.
fn row_at(app: &SoundApp, p: egui::Pos2) -> Option<TrackId> {
    app.edit_layout.rows.iter().find(|(_, r)| rect(*r).contains(p)).map(|(id, _)| TrackId(*id))
}

/// Finish a Browser drag let go over the Playlist: a pattern is placed there (and plays there),
/// a sound joins the pattern under the pointer, an instrument makes a new instrument track.
fn playlist_drop(app: &mut SoundApp, ui: &mut Ui, area: Rect) {
    let Some(item) = ui.ctx().data(|d| d.get_temp::<Item>(drag_id())) else { return };
    let (released, pos) = ui.input(|i| (i.pointer.any_released(), i.pointer.interact_pos()));
    let Some(p) = pos.filter(|p| area.contains(*p)) else { return };
    ui.ctx().set_cursor_icon(egui::CursorIcon::Copy);
    if !released {
        return;
    }
    ui.ctx().data_mut(|d| d.remove::<Item>(drag_id()));
    let s = app.engine.session();
    let at = sample_at(s, rect(app.edit_layout.timeline), p.x).max(0);
    let track = row_at(app, p);
    let result = match item {
        Item::Pattern(clip) => {
            let mut params = json!({"clip": clip.0, "at": at});
            if let Some(t) = track.filter(|t| s.track(*t).is_some_and(|x| x.kind.is_midi())) {
                params["track"] = json!(t.0);
            }
            app.run("playlist.place", params)
        }
        Item::Sound(pitch) => crate::new_layout::drop_sound(app, pitch, track, at),
        Item::Instrument(plugin) => {
            let name = soundcraft_dsp::plugins().iter().find(|i| i.id == plugin).map_or(plugin, |i| i.name);
            app.run("track.new", json!({"kind": "instrument", "instrument": plugin, "name": name}))
        }
    };
    if let Err(e) = result {
        app.ui.status = e;
    }
}

/// The notes other MIDI clips play while `clip` plays, in `clip`'s ticks (ghost notes).
pub fn ghost_notes(s: &Session, clip: ClipId) -> Vec<soundcraft_midi::Note> {
    let Some((track, c)) = s.find_clip(clip) else { return Vec::new() };
    let sr = s.sample_rate;
    let base = s.tempo.samples_to_ticks(c.start, sr);
    let mut out = Vec::new();
    for t in s.tracks.iter().filter(|t| t.id != track && t.kind.is_midi()) {
        for o in t.clips().iter().filter(|o| o.range().overlaps(&c.range())) {
            let ClipContent::Midi { sequence } = &o.content else { continue };
            let shift = s.tempo.samples_to_ticks(o.start, sr).saturating_sub(base);
            out.extend(sequence.notes.iter().map(|n| soundcraft_midi::Note { start: n.start.saturating_add(shift), ..*n }).take(4096));
        }
    }
    out
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
    fn the_fl_layout_is_selectable_and_keeps_the_session() {
        let mut app = app();
        app.engine.execute("session.new", &json!({"template": "drums"})).unwrap();
        let before = app.engine.session().clone();
        assert!(LayoutPreset::FlStyle.available());
        for preset in ["flStyle", "new", "flStyle", "reaper", "flStyle", "classic", "new"] {
            app.run("ui.layout", json!({"preset": preset})).unwrap();
            assert_eq!(*app.engine.session(), before, "after switching to {preset}");
        }
        app.run("ui.layout", json!({"preset": "flStyle"})).unwrap();
        assert!(active(&app));
        // F9's mixer is FL's own.
        let open = app.ui.fl_mixer_open;
        app.run("ui.mixer", json!({})).unwrap();
        assert_eq!(app.ui.fl_mixer_open, !open);
        assert!(!app.ui.new_mixer_open);
        assert!(app.run("ui.channel_rack", json!({"value": false})).is_ok());
        assert!(!app.ui.fl_rack_open);
    }

    #[test]
    fn the_rack_follows_the_picked_pattern() {
        let mut app = app();
        app.engine.execute("session.new", &json!({"template": "drums"})).unwrap();
        let drums = app.engine.session().tracks[0].id;
        assert_eq!(rack_track(&app), Some(drums));
        app.run("pattern.step", json!({"track": drums.0, "pitch": 36, "step": 0})).unwrap();
        let c = crate::midi_editor::target_clip(&app).unwrap();
        assert_eq!(app.engine.session().find_clip(c).map(|(t, _)| t), Some(drums));
    }

    #[test]
    fn roll_tools_need_selected_notes() {
        let mut app = app();
        assert!(roll_tool(&mut app, "midi.strum", json!({})).is_err());
        let t = app.engine.session_mut().add_track(soundcraft_model::TrackKind::Instrument, soundcraft_model::ChannelFormat::Stereo, Some("Keys"));
        let clip = app.run("midi.new_clip", json!({"track": t.0})).unwrap()["clip"].as_u64().unwrap();
        app.run("edit.select", json!({"clips": [clip]})).unwrap();
        assert!(roll_tool(&mut app, "midi.strum", json!({})).is_err(), "nothing selected yet");
        let r = app.run("midi.stamp", json!({"clip": clip, "chord": "major", "pitch": 60, "start_ticks": 0, "length_ticks": 960})).unwrap();
        app.midi.selected = serde_json::from_value(r["indices"].clone()).unwrap();
        let r = command(&mut app, "ui.roll_strum", &json!({})).unwrap().unwrap();
        assert_eq!(r["indices"].as_array().map(Vec::len), Some(3));
        assert_eq!(app.midi.selected.len(), 3);
    }

    #[test]
    fn ghost_notes_come_from_other_tracks_in_this_clips_time() {
        let mut app = app();
        let s = app.engine.session_mut();
        let a = s.add_track(soundcraft_model::TrackKind::Instrument, soundcraft_model::ChannelFormat::Stereo, Some("Bass"));
        let b = s.add_track(soundcraft_model::TrackKind::Instrument, soundcraft_model::ChannelFormat::Stereo, Some("Keys"));
        let ca = app.run("midi.new_clip", json!({"track": a.0, "start": 0, "end": 96_000})).unwrap()["clip"].as_u64().unwrap();
        let cb = app.run("midi.new_clip", json!({"track": b.0, "start": 0, "end": 96_000})).unwrap()["clip"].as_u64().unwrap();
        app.run("midi.note_add", json!({"clip": cb, "pitch": 64, "start_ticks": 480})).unwrap();
        app.run("midi.note_add", json!({"clip": ca, "pitch": 40, "start_ticks": 0})).unwrap();
        let g = ghost_notes(app.engine.session(), ClipId(ca));
        assert_eq!(g.len(), 1);
        assert_eq!((g[0].pitch, g[0].start), (64, 480));
    }
}
