//! The NEW layout preset: one screen, no window hunting. A top bar with the project, tempo, key,
//! meter, transport, layout switcher and Save/Undo/Redo; the Browser on the left; the timeline in
//! the centre; the Editor and the Mixer at the bottom (the Mixer collapsed until it is expanded);
//! and the Inspector on the right. It is a layout only: the same engine, session and commands as
//! the Classic layout, so switching between them mid-song loses nothing.

use crate::SoundApp;
use crate::theme::{Tokens, bold, mono, regular};
use crate::widgets::icon_button;
use egui::{Align2, CornerRadius, Key, Modifiers, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use soundcraft_model::{Route, TrackId, TrackKind};

/// The layout presets the switcher offers. Only NEW and Classic exist in 0.1.0; the others are
/// listed and refused until they are built.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LayoutPreset {
    #[default]
    New,
    FlStyle,
    Classic,
    Reaper,
    AbletonStyle,
}

impl LayoutPreset {
    /// In switcher order; keys 1 to 5 pick them.
    pub const ALL: [Self; 5] = [Self::New, Self::FlStyle, Self::Classic, Self::Reaper, Self::AbletonStyle];

    pub fn label(self) -> &'static str {
        match self {
            Self::New => "NEW",
            Self::FlStyle => "FL-style",
            Self::Classic => "Classic",
            Self::Reaper => "Reaper",
            Self::AbletonStyle => "Ableton-style",
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            Self::New => "new",
            Self::FlStyle => "flStyle",
            Self::Classic => "classic",
            Self::Reaper => "reaper",
            Self::AbletonStyle => "abletonStyle",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.id().eq_ignore_ascii_case(s.trim()) || p.label().eq_ignore_ascii_case(s.trim()))
    }

    /// Whether this preset is built yet.
    pub fn available(self) -> bool {
        matches!(self, Self::New | Self::Classic)
    }
}

/// What the bottom Editor shows. `Auto` follows the selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EditorView {
    #[default]
    Auto,
    StepSequencer,
    PianoRoll,
}

impl EditorView {
    fn id(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::StepSequencer => "steps",
            Self::PianoRoll => "piano",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        [Self::Auto, Self::StepSequencer, Self::PianoRoll].into_iter().find(|v| v.id() == s.trim())
    }
}

/// Top bar height.
pub const TOP_BAR_H: f32 = 48.0;
/// Browser width on the left.
pub const BROWSER_W: f32 = 220.0;
/// Inspector width on the right.
pub const INSPECTOR_W: f32 = 240.0;
/// Editor height at the bottom.
pub const EDITOR_H: f32 = 240.0;
/// Height of the collapsed Mixer: its header row only.
pub const MIXER_COLLAPSED_H: f32 = 26.0;
/// Height of the expanded Mixer.
pub const MIXER_EXPANDED_H: f32 = 330.0;

/// `ui.layout`, `ui.mixer` and `ui.editor`. None when `id` is none of them.
pub fn command(app: &mut SoundApp, id: &str, p: &Value) -> Option<Result<Value, String>> {
    Some(match id {
        "ui.layout" => layout_command(app, p),
        "ui.mixer" => {
            app.ui.new_mixer_open = p.get("value").and_then(Value::as_bool).unwrap_or(!app.ui.new_mixer_open);
            Ok(json!({"value": app.ui.new_mixer_open}))
        }
        "ui.editor" => match p.get("view").and_then(Value::as_str) {
            None => Ok(json!({"view": app.ui.new_editor.id()})),
            Some(v) => match EditorView::parse(v) {
                Some(view) => {
                    app.ui.new_editor = view;
                    Ok(json!({"view": view.id()}))
                }
                None => Err(format!("unknown editor view `{v}` (auto, steps or piano)")),
            },
        },
        _ => return None,
    })
}

fn layout_command(app: &mut SoundApp, p: &Value) -> Result<Value, String> {
    let Some(name) = p.get("preset").and_then(Value::as_str) else {
        return Ok(json!({"preset": app.ui.layout.id()}));
    };
    let preset = LayoutPreset::parse(name).ok_or_else(|| format!("unknown layout `{name}` (new, flStyle, classic, reaper or abletonStyle)"))?;
    if !preset.available() {
        return Err(format!("the {} layout is not built yet", preset.label()));
    }
    app.ui.layout = preset;
    if preset == LayoutPreset::Classic {
        // Classic shows its Edit window; Mix stays one shortcut away as before.
        app.ui.window = crate::MainWindow::Edit;
    }
    Ok(json!({"preset": preset.id()}))
}

/// The NEW layout's own keys, unmodified (Ctrl+D is the registry's Duplicate). Returns whether the
/// key was taken. Space, Ctrl+D and the rest fall through to the shared shortcuts.
pub fn keys(app: &mut SoundApp, key: Key, mods: Modifiers) -> bool {
    if mods.any() {
        return false;
    }
    let run = |app: &mut SoundApp, id: &str, p: Value| {
        if let Err(e) = app.run(id, p) {
            app.ui.status = e;
        }
    };
    match key {
        Key::R => run(app, "transport.record", json!({})),
        Key::L => run(app, "options.loop_playback", json!({})),
        Key::M => run(app, "ui.mixer", json!({})),
        Key::B => run(app, "ui.editor", json!({"view": "steps"})),
        Key::P => run(app, "ui.editor", json!({"view": "piano"})),
        Key::Num1 | Key::Num2 | Key::Num3 | Key::Num4 | Key::Num5 => {
            let index = [Key::Num1, Key::Num2, Key::Num3, Key::Num4, Key::Num5].iter().position(|k| *k == key).unwrap_or(0);
            let preset = LayoutPreset::ALL.get(index).copied().unwrap_or_default();
            run(app, "ui.layout", json!({"preset": preset.id()}));
        }
        _ => return false,
    }
    true
}

/// Draw the NEW layout.
pub fn show(app: &mut SoundApp, ui: &mut Ui) {
    let t = Tokens::current();
    crate::edit_window::drive_drag(app, ui.ctx());
    egui::Panel::top("new_top_bar").exact_size(TOP_BAR_H).frame(egui::Frame::NONE.fill(t.toolbar_bg)).show(ui, |ui| top_bar(app, ui));
    // Expanded, the Mixer takes up to half the window and the timeline keeps the rest.
    // Expanded, the Mixer takes up to 60% of the window, the Editor folds to its title row and
    // the timeline keeps the rest.
    let mixer_h = if app.ui.new_mixer_open { open_mixer_height(ui.available_rect_before_wrap().height()) } else { MIXER_COLLAPSED_H };
    egui::Panel::bottom("new_mixer").exact_size(mixer_h).frame(egui::Frame::NONE.fill(t.panel_bg)).show(ui, |ui| mixer(app, ui));
    if app.ui.new_mixer_open {
        egui::Panel::bottom("new_editor_folded")
            .exact_size(EDITOR_TITLE_H)
            .frame(egui::Frame::NONE.fill(t.panel_bg))
            .show(ui, |ui| editor_title(app, ui, ui.max_rect()));
    } else {
        egui::Panel::bottom("new_editor")
            .resizable(true)
            .default_size(EDITOR_H)
            .size_range(120.0..=640.0)
            .frame(egui::Frame::NONE.fill(t.panel_bg))
            .show(ui, |ui| editor(app, ui));
    }
    egui::Panel::left("new_browser")
        .resizable(true)
        .default_size(BROWSER_W)
        .size_range(160.0..=420.0)
        .frame(egui::Frame::NONE.fill(t.panel_bg))
        .show(ui, |ui| browser(app, ui));
    egui::Panel::right("new_inspector")
        .resizable(true)
        .default_size(INSPECTOR_W)
        .size_range(180.0..=420.0)
        .frame(egui::Frame::NONE.fill(t.panel_bg))
        .show(ui, |ui| inspector(app, ui));
    egui::CentralPanel::default().frame(egui::Frame::NONE.fill(t.window_bg)).show(ui, |ui| {
        let full = ui.max_rect();
        let strip = Rect::from_min_max(pos2(full.min.x, full.max.y - ADD_STRIP_H), full.max);
        let timeline_rect = Rect::from_min_max(full.min, pos2(full.max.x, strip.min.y));
        let mut timeline = ui.new_child(egui::UiBuilder::new().max_rect(timeline_rect));
        // A short timeline (the Mixer open in a small window) must not draw its rows over the
        // Editor and the Mixer below it.
        timeline.set_clip_rect(timeline_rect.intersect(ui.clip_rect()));
        crate::edit_window::main_area(app, &mut timeline);
        add_strip(app, ui, strip);
        browser_drop(app, ui);
    });
}

/// Height the open Mixer needs to show a whole channel strip, fader included, without scrolling:
/// its header, the strip ([`crate::mix_window::STRIP_MIN_H`]) and the strips' scroll bar.
pub const MIXER_FIT_H: f32 = MIXER_COLLAPSED_H + crate::mix_window::STRIP_MIN_H + 16.0;
/// The timeline keeps at least this much height when the Mixer is open.
pub const TIMELINE_MIN_H: f32 = 90.0;

/// The open Mixer's height when `h` is left below the top bar: 60% of it, and at least
/// enough for a whole strip, while the timeline keeps [`TIMELINE_MIN_H`] and the folded Editor
/// its title row.
pub fn open_mixer_height(h: f32) -> f32 {
    let room = (h - TIMELINE_MIN_H - EDITOR_TITLE_H - ADD_STRIP_H).max(MIXER_EXPANDED_H);
    (h * 0.6).max(MIXER_FIT_H).min(room)
}

/// Height of the one-click Add Bass / Chords / Lead row under the timeline.
pub const ADD_STRIP_H: f32 = 30.0;

/// The tracks the one-click row adds: name and instrument. Each plays the Subtractive Synth.
pub const QUICK_TRACKS: [&str; 3] = ["Bass", "Chords", "Lead"];

/// Add a quick track by name: an instrument track playing the Subtractive Synth.
pub fn add_quick_track(app: &mut SoundApp, name: &str) -> Result<Value, String> {
    app.run("track.new", json!({"kind": "instrument", "name": name, "instrument": "subtractive_synth"}))
}

/// One click adds Bass, Chords or Lead.
fn add_strip(app: &mut SoundApp, ui: &mut Ui, strip: Rect) {
    let t = Tokens::current();
    ui.painter().rect_filled(strip, 0.0, t.panel_bg2);
    let mut row =
        ui.new_child(egui::UiBuilder::new().max_rect(strip.shrink2(vec2(10.0, 3.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
    row.label(egui::RichText::new("Add").font(regular(11.5)).color(t.text_dim));
    for name in QUICK_TRACKS {
        if row
            .add(egui::Button::new(egui::RichText::new(format!("+ {name}")).font(regular(11.5))))
            .on_hover_text(format!("Add a {name} track"))
            .clicked()
            && let Err(e) = add_quick_track(app, name)
        {
            app.ui.status = e;
        }
    }
}

/// A small caps section title.
fn heading(ui: &mut Ui, text: &str) -> egui::Response {
    let t = Tokens::current();
    ui.add_space(6.0);
    ui.label(egui::RichText::new(text).font(bold(11.0)).color(t.text_dim))
}

fn top_bar(app: &mut SoundApp, ui: &mut Ui) {
    let t = Tokens::current();
    let r = ui.max_rect();
    ui.painter().line_segment([pos2(r.min.x, r.max.y - 0.5), pos2(r.max.x, r.max.y - 0.5)], Stroke::new(1.0, t.border));
    let mut row = ui.new_child(egui::UiBuilder::new().max_rect(r.shrink2(vec2(10.0, 6.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
    let ui = &mut row;
    ui.spacing_mut().item_spacing.x = 6.0;
    let (name, bpm, key, meter, looped, click, playing, recording) = {
        let s = app.engine.session();
        let at = app.position();
        let tick = s.tempo.samples_to_ticks(at, s.sample_rate);
        let m = s.tempo.meter_at_tick(tick);
        let key = s.key_signatures.iter().rev().find(|(k, _)| *k <= at).or(s.key_signatures.first()).map(|(_, k)| k.clone());
        (
            s.name.clone(),
            s.tempo.tempo_at_tick(tick),
            key,
            format!("{}/{}", m.numerator, m.denominator),
            s.edit.loop_playback,
            s.edit.click,
            app.is_playing(),
            app.engine.transport.recording,
        )
    };
    let mut project = name;
    soundcraft_model::truncate_chars(&mut project, 32);
    ui.label(egui::RichText::new(project).font(bold(13.0)).color(t.text));
    ui.add_space(8.0);
    readout(ui, "Tempo", |ui| crate::toolbar::tempo_field(app, ui, bpm));
    readout(ui, "Key", |ui| {
        ui.label(egui::RichText::new(key.unwrap_or_else(|| "-".into())).font(mono(12.0)).color(t.counter_text));
    });
    readout(ui, "Time", |ui| {
        ui.label(egui::RichText::new(meter).font(mono(12.0)).color(t.counter_text));
    });
    ui.add_space(8.0);
    let size = vec2(34.0, 28.0);
    if icon_button(ui, size, "play", playing, "Play (Space)").clicked() {
        let _ = app.run("transport.play", json!({}));
    }
    if icon_button(ui, size, "stop", false, "Stop (Space)").clicked() {
        let _ = app.run("transport.stop", json!({}));
    }
    if icon_button(ui, size, "record", recording, "Record (R)").clicked() {
        let _ = app.run("transport.record", json!({}));
    }
    if icon_button(ui, size, "loop", looped, "Loop (L)").clicked() {
        let _ = app.run("options.loop_playback", json!({}));
    }
    if icon_button(ui, size, "click", click, "Metronome").clicked() {
        let _ = app.run("options.click", json!({}));
    }
    ui.add_space(8.0);
    switcher(app, ui);
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        for (label, id, tip) in
            [("Redo", "edit.redo", "Redo (Ctrl+Shift+Z)"), ("Undo", "edit.undo", "Undo (Ctrl+Z)"), ("Save", "session.save", "Save (Ctrl+S)")]
        {
            if ui.add(egui::Button::new(egui::RichText::new(label).font(regular(12.0)))).on_hover_text(tip).clicked() {
                crate::menus::invoke_menu(app, id, "");
            }
        }
    });
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

/// The layout switcher: NEW and Classic switch; the others are named and greyed until built.
fn switcher(app: &mut SoundApp, ui: &mut Ui) {
    let t = Tokens::current();
    for (index, preset) in LayoutPreset::ALL.into_iter().enumerate() {
        let on = app.ui.layout == preset;
        let text = egui::RichText::new(preset.label()).font(bold(11.0)).color(if on { t.mode_on_text } else { t.text });
        let button = egui::Button::new(text).fill(if on { t.mode_on } else { t.button }).selected(on);
        let tip = if preset.available() { format!("{} layout ({})", preset.label(), index + 1) } else { format!("{} layout: later", preset.label()) };
        if ui.add_enabled(preset.available(), button).on_hover_text(tip).on_disabled_hover_text(format!("{} layout: later", preset.label())).clicked()
        {
            let _ = app.run("ui.layout", json!({"preset": preset.id()}));
        }
    }
}

/// What the Browser lists: the drum sounds, instruments and effects by name, filtered by the
/// search text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BrowserItem {
    /// A Drum Synth sound, by MIDI pitch.
    Sound(u8),
    Instrument(&'static str),
    Effect(&'static str),
}

/// The Browser item being dragged, kept in egui memory until the drag ends.
fn drag_id() -> egui::Id {
    egui::Id::unique("solara-browser-drag")
}

fn browser(app: &mut SoundApp, ui: &mut Ui) {
    let t = Tokens::current();
    let inner = ui.max_rect().shrink(8.0);
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(inner));
    let ui = &mut child;
    ui.label(egui::RichText::new("BROWSER").font(bold(12.0)).color(t.text));
    ui.add_space(4.0);
    ui.add(egui::TextEdit::singleline(&mut app.browser_search).hint_text("Search by name").desired_width(f32::INFINITY));
    let needle = app.browser_search.trim().to_lowercase();
    let plugins = soundcraft_dsp::plugins();
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        let _ = heading(ui, "SOUNDS");
        for (pitch, name) in soundcraft_engine::cmd::pattern::SOUNDS {
            browser_row(ui, &needle, name, BrowserItem::Sound(pitch));
        }
        let _ = heading(ui, "LOOPS");
        ui.label(egui::RichText::new("Drop audio files on the timeline.").font(regular(11.0)).color(t.text_dim));
        let _ = heading(ui, "INSTRUMENTS");
        for info in plugins.iter().filter(|p| p.category == soundcraft_dsp::Category::Instrument) {
            browser_row(ui, &needle, info.name, BrowserItem::Instrument(info.id));
        }
        let _ = heading(ui, "EFFECTS");
        for info in plugins.iter().filter(|p| p.category != soundcraft_dsp::Category::Instrument) {
            browser_row(ui, &needle, info.name, BrowserItem::Effect(info.id));
        }
    });
}

fn browser_row(ui: &mut Ui, needle: &str, name: &str, item: BrowserItem) {
    if !needle.is_empty() && !name.to_lowercase().contains(needle) {
        return;
    }
    let t = Tokens::current();
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 20.0), Sense::drag());
    if resp.hovered() || resp.dragged() {
        ui.painter().rect_filled(rect, 3.0, t.row_selected);
    }
    ui.painter().text(pos2(rect.min.x + 6.0, rect.center().y), Align2::LEFT_CENTER, name, regular(12.0), t.text);
    if resp.drag_started() {
        ui.ctx().data_mut(|d| d.insert_temp(drag_id(), item));
    }
    let hint = match item {
        BrowserItem::Sound(_) => "Drag onto a track to step it in a pattern",
        BrowserItem::Instrument(_) => "Drag onto the timeline for a new instrument track",
        BrowserItem::Effect(_) => "Drag onto a track to insert it",
    };
    resp.on_hover_text(hint);
}

/// Finish a Browser drag released over the timeline: a sound goes into the pattern under the
/// pointer (made if needed, on a new Drums track when no track is there); an effect goes into the
/// first free insert of the track under the pointer; an instrument makes a new instrument track.
fn browser_drop(app: &mut SoundApp, ui: &mut Ui) {
    let Some(item) = ui.ctx().data(|d| d.get_temp::<BrowserItem>(drag_id())) else { return };
    let (released, pos) = ui.input(|i| (i.pointer.any_released(), i.pointer.interact_pos()));
    if let Some(p) = pos
        && ui.max_rect().contains(p)
    {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Copy);
    }
    if !released {
        return;
    }
    ui.ctx().data_mut(|d| d.remove::<BrowserItem>(drag_id()));
    let Some(p) = pos.filter(|p| ui.max_rect().contains(*p)) else { return };
    let row = app.edit_layout.rows.iter().find(|(_, r)| crate::edit_window::rect(*r).contains(p)).map(|(id, _)| *id);
    let result = match item {
        BrowserItem::Sound(pitch) => {
            let s = app.engine.session();
            let at = crate::edit_window::sample_at(s, crate::edit_window::rect(app.edit_layout.timeline), p.x).max(0);
            drop_sound(app, pitch, row.map(TrackId), at)
        }
        BrowserItem::Instrument(plugin) => {
            let name = soundcraft_dsp::plugins().iter().find(|i| i.id == plugin).map_or(plugin, |i| i.name);
            app.run("track.new", json!({"kind": "instrument", "instrument": plugin, "name": name}))
        }
        BrowserItem::Effect(plugin) => {
            let Some(track) = row else {
                app.ui.status = "Drop an effect on a track".into();
                return;
            };
            app.run("mix.insert", json!({"track": track, "plugin": plugin}))
        }
    };
    if let Err(e) = result {
        app.ui.status = e;
    }
}

/// Drop a drum sound at `at`: on `track` it joins the pattern there (made if needed); with no
/// track it makes a Drums track. The Editor turns to the step sequencer on that pattern.
pub fn drop_sound(app: &mut SoundApp, pitch: u8, track: Option<TrackId>, at: i64) -> Result<Value, String> {
    let mut p = json!({"pitch": pitch, "at": at});
    if let Some(t) = track {
        p["track"] = json!(t.0);
    }
    let r = app.run("pattern.add_sound", p)?;
    app.ui.new_editor = EditorView::StepSequencer;
    Ok(r)
}

/// The track the Inspector and Editor follow: the first selected track, else the first track.
fn focus_track(app: &SoundApp) -> Option<TrackId> {
    let s = app.engine.session();
    s.edit.selected_tracks.first().copied().filter(|id| s.track(*id).is_some()).or_else(|| s.tracks.iter().find(|t| !t.hidden).map(|t| t.id))
}

fn inspector(app: &mut SoundApp, ui: &mut Ui) {
    let t = Tokens::current();
    let inner = ui.max_rect().shrink(8.0);
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(inner));
    let ui = &mut child;
    ui.label(egui::RichText::new("INSPECTOR").font(bold(12.0)).color(t.text));
    let Some(id) = focus_track(app) else {
        ui.add_space(6.0);
        ui.label(egui::RichText::new("No track selected").font(regular(11.5)).color(t.text_dim));
        return;
    };
    let Some(track) = app.engine.session().track(id).cloned() else { return };
    ui.add_space(4.0);
    ui.label(egui::RichText::new(&track.name).font(bold(14.0)).color(t.text));
    // Labels sit above their sliders, which fill the column with room for the value box.
    ui.spacing_mut().slider_width = (ui.available_width() - 72.0).max(60.0);
    let label = heading(ui, "VOLUME");
    let mut db = track.mixer.volume_db;
    let resp = ui.add(egui::Slider::new(&mut db, -60.0..=12.0).suffix(" dB").fixed_decimals(1)).labelled_by(label.id);
    if resp.changed() {
        let _ = app.engine.execute_merged("mix.volume", &json!({"tracks": [id], "db": db}), "inspector_volume");
    }
    if resp.drag_stopped() || resp.lost_focus() {
        app.engine.end_merge();
    }
    if !track.mixer.pan.is_empty() {
        // A stereo track pans each side: left and right.
        let sides = track.mixer.pan.len().min(2);
        for (index, mut pan) in track.mixer.pan.iter().copied().take(sides).enumerate() {
            let text = match (sides, index) {
                (1, _) => "PAN",
                (_, 0) => "PAN LEFT",
                _ => "PAN RIGHT",
            };
            let label = heading(ui, text);
            let resp = ui.add(egui::Slider::new(&mut pan, -1.0..=1.0).fixed_decimals(2)).labelled_by(label.id);
            if resp.changed() {
                let _ = app.engine.execute_merged("mix.pan", &json!({"tracks": [id], "pan": pan, "index": index}), "inspector_pan");
            }
            if resp.drag_stopped() || resp.lost_focus() {
                app.engine.end_merge();
            }
        }
    }
    let _ = heading(ui, "INSERTS");
    let plugins = soundcraft_dsp::plugins();
    let inserts: Vec<String> = track
        .mixer
        .inserts
        .iter()
        .flatten()
        .map(|i| plugins.iter().find(|p| p.id == i.plugin).map_or_else(|| i.plugin.clone(), |p| p.name.to_string()))
        .collect();
    slot_list(ui, &inserts, "Drag an effect from the Browser onto the track");
    let _ = heading(ui, "SENDS");
    let s = app.engine.session();
    let sends: Vec<String> = track
        .mixer
        .sends
        .iter()
        .flatten()
        .map(|send| {
            let to = match send.target {
                Route::Bus(b) => s.bus(b).map_or_else(|| "Bus".to_string(), |b| b.name.clone()),
                _ => "Main".to_string(),
            };
            format!("{to}  {:.1} dB", send.level_db)
        })
        .collect();
    slot_list(ui, &sends, "No sends");
}

fn slot_list(ui: &mut Ui, items: &[String], empty: &str) {
    let t = Tokens::current();
    if items.is_empty() {
        ui.add(egui::Label::new(egui::RichText::new(empty).font(regular(11.0)).color(t.text_dim)).wrap());
    }
    for item in items {
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 20.0), Sense::hover());
        ui.painter().rect(rect, CornerRadius::same(3), t.slot_bg, Stroke::new(1.0, t.border), StrokeKind::Inside);
        ui.painter().text(pos2(rect.min.x + 6.0, rect.center().y), Align2::LEFT_CENTER, item, regular(11.5), t.text);
    }
}

/// Height of the Editor's title row, all that shows while the Mixer is expanded.
pub const EDITOR_TITLE_H: f32 = 22.0;

/// What the Editor shows now. `Auto` follows the focused track: a Drum Synth track gets the step
/// sequencer, other MIDI and instrument tracks the piano roll. B and P pick one outright.
fn editor_view(app: &SoundApp) -> EditorView {
    let track = focus_track(app).and_then(|id| app.engine.session().track(id));
    match app.ui.new_editor {
        EditorView::Auto => match track {
            Some(t) if t.instrument.as_ref().is_some_and(|i| i.plugin == DRUM_SYNTH) => EditorView::StepSequencer,
            Some(t) if matches!(t.kind, TrackKind::Midi | TrackKind::Instrument) => EditorView::PianoRoll,
            _ => EditorView::Auto,
        },
        v => v,
    }
}

/// The Drum Synth's registry id: its tracks open in the step sequencer.
pub const DRUM_SYNTH: &str = "drum_synth";

/// The Editor's title row, with its top border.
fn editor_title(app: &SoundApp, ui: &Ui, r: Rect) {
    let t = Tokens::current();
    ui.painter().line_segment([pos2(r.min.x, r.min.y + 0.5), pos2(r.max.x, r.min.y + 0.5)], Stroke::new(1.0, t.border));
    let title = Rect::from_min_size(r.min, vec2(r.width(), EDITOR_TITLE_H));
    let label = match editor_view(app) {
        EditorView::StepSequencer => "EDITOR · STEP SEQUENCER",
        EditorView::PianoRoll => "EDITOR · PIANO ROLL",
        EditorView::Auto => "EDITOR",
    };
    ui.painter().text(pos2(title.min.x + 10.0, title.center().y), Align2::LEFT_CENTER, label, bold(11.0), t.text_dim);
}

fn editor(app: &mut SoundApp, ui: &mut Ui) {
    let t = Tokens::current();
    let r = ui.max_rect();
    // Claim the whole panel, so the resizable panel keeps its height whatever the view draws.
    ui.expand_to_include_rect(r);
    editor_title(app, ui, r);
    let view = editor_view(app);
    let title = Rect::from_min_size(r.min, vec2(r.width(), EDITOR_TITLE_H));
    let body = Rect::from_min_max(pos2(r.min.x, title.max.y), r.max);
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(body));
    match view {
        EditorView::PianoRoll => {
            child.ctx().data_mut(|data| data.insert_temp(egui::Id::unique("soundcraft-midi-body"), body));
            crate::docking::minimum_body(&mut child, vec2(420.0, 160.0), |ui| crate::midi_editor::show(app, ui));
        }
        EditorView::StepSequencer => crate::step_seq::show(app, &mut child, focus_track(app)),
        EditorView::Auto => {
            child.painter().text(body.center(), Align2::CENTER_CENTER, "Select a track or clip to edit it here.", regular(12.0), t.text_dim);
        }
    }
}

/// The Mixer: a header row that M or a click expands in place, above the Editor's bottom edge.
fn mixer(app: &mut SoundApp, ui: &mut Ui) {
    let t = Tokens::current();
    let r = ui.max_rect();
    ui.painter().line_segment([pos2(r.min.x, r.min.y + 0.5), pos2(r.max.x, r.min.y + 0.5)], Stroke::new(1.0, t.border));
    let header = Rect::from_min_size(r.min, vec2(r.width(), MIXER_COLLAPSED_H));
    let resp = ui.interact(header, ui.scope_id().with("new_mixer_header"), Sense::click());
    let c = pos2(header.min.x + 15.0, header.center().y);
    let points = if app.ui.new_mixer_open {
        vec![pos2(c.x - 4.0, c.y - 2.5), pos2(c.x + 4.0, c.y - 2.5), pos2(c.x, c.y + 3.0)]
    } else {
        vec![pos2(c.x - 2.5, c.y - 4.0), pos2(c.x + 3.0, c.y), pos2(c.x - 2.5, c.y + 4.0)]
    };
    ui.painter().add(egui::Shape::convex_polygon(points, t.text_dim, Stroke::NONE));
    ui.painter().text(pos2(header.min.x + 26.0, header.center().y), Align2::LEFT_CENTER, "MIXER", bold(11.0), t.text_dim);
    ui.painter().text(pos2(header.max.x - 10.0, header.center().y), Align2::RIGHT_CENTER, "M", mono(11.0), t.text_dim);
    if resp.on_hover_text("Expand or collapse the mixer (M)").clicked() {
        let _ = app.run("ui.mixer", json!({}));
    }
    if app.ui.new_mixer_open {
        let body = Rect::from_min_max(pos2(r.min.x, header.max.y), r.max);
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(body));
        child.painter().rect_filled(body, 0.0, t.window_bg);
        egui::ScrollArea::vertical()
            .id_salt("new_mixer_scroll")
            .auto_shrink([false, false])
            .show(&mut child, |ui| crate::mix_window::content(app, ui));
    }
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
    fn new_is_the_default_layout_and_the_mixer_starts_collapsed() {
        let ui = crate::UiState::default();
        assert_eq!(ui.layout, LayoutPreset::New);
        assert!(!ui.new_mixer_open);
        let old: crate::UiState = serde_json::from_value(json!({})).unwrap();
        assert_eq!(old.layout, LayoutPreset::New, "settings saved before layouts open in NEW");
    }

    #[test]
    fn the_layout_switches_between_new_and_classic_and_refuses_unbuilt_ones() {
        let mut app = app();
        assert_eq!(app.run("ui.layout", json!({"preset": "classic"})), Ok(json!({"preset": "classic"})));
        assert_eq!(app.ui.layout, LayoutPreset::Classic);
        assert!(app.run("ui.layout", json!({"preset": "reaper"})).is_err());
        assert_eq!(app.ui.layout, LayoutPreset::Classic);
        assert!(app.run("ui.layout", json!({"preset": "nope"})).is_err());
        assert_eq!(app.run("ui.layout", json!({"preset": "new"})), Ok(json!({"preset": "new"})));
        assert_eq!(app.run("ui.layout", json!({})), Ok(json!({"preset": "new"})));
    }

    #[test]
    fn switching_layouts_keeps_the_session() {
        let mut app = app();
        app.engine.execute("session.new", &json!({"template": "drums"})).unwrap();
        let before = app.engine.session().clone();
        app.run("ui.layout", json!({"preset": "classic"})).unwrap();
        app.run("ui.layout", json!({"preset": "new"})).unwrap();
        assert_eq!(*app.engine.session(), before);
    }

    #[test]
    fn keys_toggle_the_mixer_pick_the_editor_and_switch_layouts() {
        let mut app = app();
        assert!(keys(&mut app, Key::M, Modifiers::NONE));
        assert!(app.ui.new_mixer_open);
        assert!(keys(&mut app, Key::M, Modifiers::NONE));
        assert!(!app.ui.new_mixer_open);
        assert!(keys(&mut app, Key::B, Modifiers::NONE));
        assert_eq!(app.ui.new_editor, EditorView::StepSequencer);
        assert!(keys(&mut app, Key::P, Modifiers::NONE));
        assert_eq!(app.ui.new_editor, EditorView::PianoRoll);
        assert!(keys(&mut app, Key::Num3, Modifiers::NONE));
        assert_eq!(app.ui.layout, LayoutPreset::Classic);
        assert!(keys(&mut app, Key::Num1, Modifiers::NONE));
        assert_eq!(app.ui.layout, LayoutPreset::New);
        // An unbuilt layout's key leaves NEW in place and says why.
        assert!(keys(&mut app, Key::Num2, Modifiers::NONE));
        assert_eq!(app.ui.layout, LayoutPreset::New);
        assert!(app.ui.status.contains("not built yet"));
        // Modified keys belong to the shared shortcuts (Ctrl+D is Duplicate).
        assert!(!keys(&mut app, Key::D, Modifiers::COMMAND));
        assert!(!keys(&mut app, Key::M, Modifiers::SHIFT));
    }

    #[test]
    fn one_click_adds_bass_chords_and_lead() {
        let mut app = app();
        app.engine.execute("session.new", &json!({"template": "drums"})).unwrap();
        for name in QUICK_TRACKS {
            add_quick_track(&mut app, name).unwrap();
        }
        let names: Vec<&str> = app.engine.session().tracks.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["Drums", "Bass", "Chords", "Lead"]);
        assert!(app.engine.session().tracks.iter().all(|t| t.kind == TrackKind::Instrument));
    }

    /// Run one frame with the pointer up (and Alt held when asked), finishing the app's gesture.
    fn release(app: &mut SoundApp, alt: bool) {
        let ctx = egui::Context::default();
        let input =
            egui::RawInput { events: vec![egui::Event::ModifiersChanged(if alt { Modifiers::ALT } else { Modifiers::NONE })], ..Default::default() };
        let mut out = ctx.run_ui(input, |ui| crate::edit_window::drive_drag(app, ui.ctx()));
        out.textures_delta.clear();
    }

    fn drums_with_pattern() -> (SoundApp, TrackId, soundcraft_model::ClipId) {
        let mut app = app();
        app.engine.execute("session.new", &json!({"template": "drums"})).unwrap();
        let t = app.engine.session().tracks[0].id;
        let r = drop_sound(&mut app, 36, Some(t), 0).unwrap();
        (app, t, soundcraft_model::ClipId(r["clip"].as_u64().unwrap()))
    }

    #[test]
    fn the_gate_drop_a_sound_step_it_and_space_loops_the_pattern() {
        let (mut app, t, clip) = drums_with_pattern();
        assert_eq!(app.ui.new_editor, EditorView::StepSequencer);
        assert_eq!(editor_view(&app), EditorView::StepSequencer);
        app.run("pattern.step", json!({"clip": clip.0, "pitch": 42, "step": 2})).unwrap();
        let range = app.engine.session().find_clip(clip).unwrap().1.range();
        assert!(app.engine.session().edit.loop_playback, "the Drums template plays in a loop");
        assert_eq!(app.engine.session().edit.selection, range);
        // Space plays the selected pattern, looped.
        app.run("transport.toggle", json!({})).unwrap();
        app.handle_transport(0.25);
        assert!(app.is_playing());
        assert_eq!(app.sim.and_then(|(_, _, l)| l), Some(range));
        // Past the bar's end the playhead comes round to the pattern's start again.
        for _ in 0..12 {
            app.handle_transport(0.25);
            assert!(app.is_playing());
            assert!(range.contains(app.position()), "{} left the pattern {range:?}", app.position());
        }
        // M still folds the mixer in and out of the same window.
        assert!(keys(&mut app, Key::M, Modifiers::NONE));
        assert!(app.ui.new_mixer_open);
        assert_eq!(app.ui.layout, LayoutPreset::New);
        assert_eq!(crate::step_seq::focused_pattern(app.engine.session(), t), Some(clip));
    }

    #[test]
    fn a_sound_dropped_on_empty_space_makes_a_drums_track() {
        let mut app = app();
        app.engine.execute("session.new", &json!({"template": "empty"})).unwrap();
        drop_sound(&mut app, 38, None, 0).unwrap();
        let s = app.engine.session();
        assert_eq!(s.tracks.len(), 1);
        assert_eq!(s.tracks[0].name, "Drums");
        assert_eq!(editor_view(&app), EditorView::StepSequencer);
    }

    #[test]
    fn a_drums_track_opens_in_the_step_sequencer_and_a_synth_track_in_the_piano_roll() {
        let mut app = app();
        app.engine.execute("session.new", &json!({"template": "drums"})).unwrap();
        assert_eq!(editor_view(&app), EditorView::StepSequencer);
        let bass = add_quick_track(&mut app, "Bass").unwrap();
        let id = bass.get("tracks").and_then(|t| t.get(0)).and_then(Value::as_u64).map(TrackId).unwrap_or(app.engine.session().tracks[1].id);
        app.engine.execute("edit.select", &json!({"tracks": [id.0]})).unwrap();
        assert_eq!(editor_view(&app), EditorView::PianoRoll);
    }

    #[test]
    fn alt_dragging_a_pattern_drops_an_independent_copy() {
        let (mut app, t, clip) = drums_with_pattern();
        let bar = app.engine.session().find_clip(clip).unwrap().1.length;
        app.gesture = Some(crate::Gesture::MoveClips { clips: vec![clip], grab_at: 0, delta: bar, from_track: t, to_track: t });
        release(&mut app, true);
        let s = app.engine.session();
        let starts: Vec<i64> = s.tracks[0].clips().iter().map(|c| c.start).collect();
        assert_eq!(starts, [0, bar], "the original stays and a copy lands one bar later");
        assert_ne!(s.tracks[0].clips()[1].id, clip);
    }

    #[test]
    fn a_plain_drag_still_moves_a_pattern() {
        let (mut app, t, clip) = drums_with_pattern();
        let bar = app.engine.session().find_clip(clip).unwrap().1.length;
        app.gesture = Some(crate::Gesture::MoveClips { clips: vec![clip], grab_at: 0, delta: bar, from_track: t, to_track: t });
        release(&mut app, false);
        let starts: Vec<i64> = app.engine.session().tracks[0].clips().iter().map(|c| c.start).collect();
        assert_eq!(starts, [bar]);
    }

    #[test]
    fn dragging_a_patterns_edge_past_its_end_loops_it() {
        let (mut app, t, clip) = drums_with_pattern();
        let bar = app.engine.session().find_clip(clip).unwrap().1.length;
        app.gesture = Some(crate::Gesture::TrimEnd { clip, track: t, to: bar * 4 });
        release(&mut app, false);
        let s = app.engine.session();
        let clips = s.tracks[0].clips();
        assert_eq!(clips.len(), 4, "four bars of the pattern");
        assert!(clips.iter().all(|c| c.loop_id.is_some() && c.loop_id == clips[0].loop_id));
    }

    #[test]
    fn the_open_mixer_fits_a_whole_strip_when_the_window_has_room() {
        // R7-Win11's maximised window, and a shorter one where 60% alone cut the fader off.
        for h in [1000.0, 800.0, 760.0] {
            assert!(open_mixer_height(h) >= MIXER_FIT_H, "{h}");
            assert!(h - open_mixer_height(h) >= TIMELINE_MIN_H + EDITOR_TITLE_H + ADD_STRIP_H - 0.01, "{h}");
        }
        // A very short window keeps some timeline and lets the strip scroll.
        let short = open_mixer_height(600.0);
        assert!((MIXER_EXPANDED_H..MIXER_FIT_H).contains(&short));
    }

    #[test]
    fn l_turns_loop_playback_off_and_on() {
        let mut app = app();
        let before = app.engine.session().edit.loop_playback;
        assert!(keys(&mut app, Key::L, Modifiers::NONE));
        assert_eq!(app.engine.session().edit.loop_playback, !before);
    }
}
