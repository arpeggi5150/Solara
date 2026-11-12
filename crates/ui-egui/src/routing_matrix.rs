//! The Routing Matrix (View › Routing Matrix): every track's sends on one grid. A row is a track
//! that sends; a column is where it can send. One click adds a send at 0 dB, another removes it;
//! a right click picks pre- or post-fader. Each row also sets the track's hardware input (the
//! input channel it records from) and its output.
//!
//! What can receive follows the active layout. In the Reaper-style layout every track can: an
//! audio or instrument track takes a receive bus (`mix.receive`) that the mix engine adds in
//! ahead of its FX. In the other layouts sends go to aux, folder and bus tracks only, and the
//! audio and instrument columns are greyed out with a note.

use crate::SoundApp;
use crate::new_layout::LayoutPreset;
use crate::theme::{Tokens, bold, regular};
use egui::{Align2, Color32, Rect, Sense, Stroke, StrokeKind, pos2, vec2};
use serde_json::{Value, json};
use soundcraft_model::{BusId, Route, Session, Track, TrackId, TrackKind};

const NAME_W: f32 = 150.0;
const ROUTE_W: f32 = 84.0;
const CELL_W: f32 = 64.0;
const ROW_H: f32 = 22.0;
const HEADER_H: f32 = 40.0;

/// Where a row can send.
#[derive(Debug, Clone, PartialEq)]
pub enum Column {
    /// The main output: the row's output is Main.
    Main,
    /// A track. `bus` is the bus it reads (an aux's input, another track's receive), once it has
    /// one; `open` is whether it takes sends in the active layout.
    Track { id: TrackId, name: String, kind: TrackKind, bus: Option<BusId>, open: bool },
    /// A bus no track reads, which a track's output or sends can still feed.
    Bus { id: BusId, name: String },
}

impl Column {
    pub fn name(&self) -> &str {
        match self {
            Column::Main => "Main",
            Column::Track { name, .. } | Column::Bus { name, .. } => name,
        }
    }

    fn bus(&self) -> Option<BusId> {
        match self {
            Column::Main => None,
            Column::Track { bus, .. } => *bus,
            Column::Bus { id, .. } => Some(*id),
        }
    }
}

/// Tracks with a row (and, for the kinds that can receive, a column).
fn routable(t: &Track) -> bool {
    matches!(t.kind, TrackKind::Audio | TrackKind::Instrument | TrackKind::Midi | TrackKind::Aux | TrackKind::Folder)
}

/// Whether every track takes sends: the Reaper-style layout's routing.
pub fn every_track_receives(layout: LayoutPreset) -> bool {
    layout == LayoutPreset::Reaper
}

/// The matrix's columns for this session and layout: Main, then every routable track, then the
/// buses no track reads.
pub fn columns(s: &Session, layout: LayoutPreset) -> Vec<Column> {
    let all = every_track_receives(layout);
    let mut read = Vec::new();
    let mut cols = vec![Column::Main];
    for t in s.tracks.iter().filter(|t| routable(t)) {
        let aux = matches!(t.kind, TrackKind::Aux | TrackKind::Folder);
        let bus = if aux {
            match &t.mixer.input {
                Route::Bus(b) => Some(*b),
                _ => None,
            }
        } else {
            t.mixer.receive
        };
        read.extend(bus);
        cols.push(Column::Track { id: t.id, name: t.name.clone(), kind: t.kind, bus, open: aux || all });
    }
    for b in s.busses.iter().filter(|b| !read.contains(&b.id)) {
        cols.push(Column::Bus { id: b.id, name: b.name.clone() });
    }
    cols
}

/// The send slot of `t` that feeds `bus`, and whether it is pre-fader.
pub fn send_to(t: &Track, bus: BusId) -> Option<(usize, bool)> {
    t.mixer.sends.iter().enumerate().find_map(|(i, s)| s.as_ref().filter(|s| s.target == Route::Bus(bus)).map(|s| (i, s.pre_fader)))
}

/// What a cell shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cell {
    /// The track itself, or a column that takes no sends in this layout.
    Closed,
    Off,
    PostFader,
    PreFader,
}

pub fn cell(row: &Track, col: &Column) -> Cell {
    match col {
        Column::Main => {
            if row.mixer.output == Route::Main {
                Cell::PostFader
            } else {
                Cell::Off
            }
        }
        Column::Track { id, open, .. } if *id == row.id || !*open => Cell::Closed,
        _ => match col.bus().and_then(|b| send_to(row, b)) {
            Some((_, true)) => Cell::PreFader,
            Some((_, false)) => Cell::PostFader,
            None => Cell::Off,
        },
    }
}

/// The bus a column receives on, made when it has none yet: an aux gets an input bus, an audio
/// or instrument track a receive.
fn receiving_bus(app: &mut SoundApp, col: &Column) -> Result<String, String> {
    let s = app.engine.session();
    if let Some(b) = col.bus().and_then(|b| s.bus(b)) {
        return Ok(b.name.clone());
    }
    let Column::Track { id, name, kind, .. } = col else { return Err("nothing receives there".into()) };
    if matches!(kind, TrackKind::Aux | TrackKind::Folder) {
        let base = format!("{name} In");
        let bus = (1..1000)
            .map(|n| if n == 1 { base.clone() } else { format!("{base} {n}") })
            .find(|n| s.bus_by_name(n).is_none() && s.track_by_name(n).is_none())
            .ok_or("no free bus name")?;
        app.run("mix.new_bus", json!({"name": bus}))?;
        app.run("track.input", json!({"tracks": [id.0], "input": bus}))?;
        Ok(bus)
    } else {
        let r = app.run("mix.receive", json!({"track": id.0}))?;
        r.get("bus").and_then(Value::as_str).map(str::to_string).ok_or_else(|| "no receive bus".into())
    }
}

/// Click on a cell: add the send (at 0 dB, post-fader) or take it away. On Main, route the
/// track's output to Main or nowhere.
pub fn toggle(app: &mut SoundApp, row: TrackId, col: &Column) -> Result<Value, String> {
    let s = app.engine.session();
    let track = s.track(row).ok_or("no such track")?;
    match cell(track, col) {
        Cell::Closed => Err(match col {
            Column::Track { id, .. } if *id == row => "a track does not send to itself".into(),
            _ => "in this layout, sends go to aux, folder and bus tracks; audio and instrument tracks take sends in the Reaper layout".into(),
        }),
        Cell::Off if *col == Column::Main => app.run("track.output", json!({"tracks": [row.0], "output": "main"})),
        Cell::PostFader | Cell::PreFader if *col == Column::Main => app.run("track.output", json!({"tracks": [row.0], "output": "none"})),
        Cell::Off => {
            let bus = receiving_bus(app, col)?;
            app.run("mix.send", json!({"track": row.0, "bus": bus, "level_db": 0.0}))
        }
        Cell::PostFader | Cell::PreFader => {
            let bus = col.bus().ok_or("no bus")?;
            let slot = app.engine.session().track(row).and_then(|t| send_to(t, bus)).map(|(slot, _)| slot).ok_or("no send")?;
            let r = app.run("mix.send_remove", json!({"track": row.0, "slot": slot}))?;
            // A receive with nothing left sending to it goes back to a plain track.
            if let Column::Track { id, kind: TrackKind::Audio | TrackKind::Instrument | TrackKind::Midi, .. } = col
                && !app.engine.session().tracks.iter().any(|t| send_to(t, bus).is_some() || t.mixer.output == Route::Bus(bus))
            {
                app.run("mix.receive", json!({"track": id.0, "value": false}))?;
            }
            Ok(r)
        }
    }
}

/// Make a send pre- or post-fader.
fn set_pre(app: &mut SoundApp, row: TrackId, col: &Column, pre: bool) {
    let slot = col.bus().and_then(|b| app.engine.session().track(row).and_then(|t| send_to(t, b))).map(|(slot, _)| slot);
    if let Some(slot) = slot
        && let Err(e) = app.run("mix.send_level", json!({"track": row.0, "slot": slot, "pre_fader": pre}))
    {
        app.ui.status = e;
    }
}

pub fn show(app: &mut SoundApp, ctx: &egui::Context) {
    if !app.ui.show_routing_matrix {
        return;
    }
    let mut open = true;
    // Wide enough for every column when the screen allows it; more scroll sideways.
    let n = {
        let s = app.engine.session();
        columns(s, app.ui.layout).len()
    };
    let want = NAME_W + 2.0 * ROUTE_W + n as f32 * CELL_W + 40.0;
    let screen = ctx.content_rect().width() - 40.0;
    egui::Window::new("Routing Matrix")
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .default_size(vec2(want.min(screen).max(420.0), 460.0))
        .min_size(vec2(420.0, 220.0))
        .show(ctx, |ui| grid(app, ui));
    if !open {
        app.ui.show_routing_matrix = false;
    }
}

fn grid(app: &mut SoundApp, ui: &mut egui::Ui) {
    let t = Tokens::current();
    let layout = app.ui.layout;
    let all = every_track_receives(layout);
    let note = if all {
        "Every track can receive. Click a cell to add a send at 0 dB, click again to remove it, right-click for pre- or post-fader."
    } else {
        "Sends go to aux, folder and bus tracks in this layout; audio and instrument columns are greyed out (they take sends in the Reaper layout). Click a cell to add or remove a send, right-click for pre- or post-fader."
    };
    ui.label(egui::RichText::new(note).font(regular(11.5)).color(t.text_dim));
    legend(ui);
    ui.separator();
    let (rows, cols) = {
        let s = app.engine.session();
        (s.tracks.iter().filter(|t| routable(t)).map(|t| t.id).collect::<Vec<_>>(), columns(s, layout))
    };
    if rows.is_empty() {
        ui.label("No tracks yet.");
        return;
    }
    let width = NAME_W + 2.0 * ROUTE_W + cols.len() as f32 * CELL_W;
    let height = HEADER_H + rows.len() as f32 * ROW_H;
    let mut clicked: Option<(TrackId, Column)> = None;
    let mut pre: Option<(TrackId, Column, bool)> = None;
    let mut route_buttons: Vec<(egui::Response, TrackId, bool)> = Vec::new();
    egui::ScrollArea::both().id_salt("routing_matrix_scroll").auto_shrink([false, false]).show(ui, |ui| {
        let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
        let p = ui.painter().clone();
        let x_cells = rect.min.x + NAME_W + 2.0 * ROUTE_W;
        // Header: the routing columns, then one per receiver.
        for (i, label) in ["Track", "Input", "Output"].into_iter().enumerate() {
            let x = if i == 0 { rect.min.x } else { rect.min.x + NAME_W + (i - 1) as f32 * ROUTE_W };
            p.text(pos2(x + 4.0, rect.min.y + HEADER_H - 6.0), Align2::LEFT_BOTTOM, label, bold(11.0), t.text_dim);
        }
        for (c, col) in cols.iter().enumerate() {
            let r = Rect::from_min_size(pos2(x_cells + c as f32 * CELL_W, rect.min.y), vec2(CELL_W, HEADER_H));
            let dim = matches!(col, Column::Track { open: false, .. });
            let color = if dim { t.text_dim.gamma_multiply(0.6) } else { t.text };
            p.with_clip_rect(r.shrink(2.0)).text(r.center_bottom() - vec2(0.0, 6.0), Align2::CENTER_BOTTOM, col.name(), regular(11.0), color);
            let kind = match col {
                Column::Main => "output",
                Column::Track { kind: TrackKind::Aux, .. } => "aux",
                Column::Track { kind: TrackKind::Folder, .. } => "folder",
                Column::Track { .. } => "track",
                Column::Bus { .. } => "bus",
            };
            p.text(r.center_top() + vec2(0.0, 6.0), Align2::CENTER_TOP, kind, regular(9.5), t.text_dim);
            let tip = if dim { format!("{}: takes sends in the Reaper layout", col.name()) } else { col.name().to_string() };
            ui.interact(r, ui.scope_id().with(("rm_head", c)), Sense::hover()).on_hover_text(tip);
        }
        p.line_segment([pos2(rect.min.x, rect.min.y + HEADER_H), pos2(rect.max.x, rect.min.y + HEADER_H)], Stroke::new(1.0, t.border));
        for (r_i, &row) in rows.iter().enumerate() {
            let y = rect.min.y + HEADER_H + r_i as f32 * ROW_H;
            let row_rect = Rect::from_min_size(pos2(rect.min.x, y), vec2(width, ROW_H));
            if r_i % 2 == 1 {
                p.rect_filled(row_rect, 0.0, t.panel_bg2);
            }
            let s = app.engine.session();
            let Some(track) = s.track(row) else { continue };
            let name_r = Rect::from_min_size(row_rect.min, vec2(NAME_W, ROW_H));
            p.with_clip_rect(name_r.shrink(2.0)).text(name_r.left_center() + vec2(6.0, 0.0), Align2::LEFT_CENTER, &track.name, regular(12.0), t.text);
            let input = crate::mix_window::input_name(app, track);
            let output = crate::mix_window::route_name(app, &track.mixer.output);
            for (k, (text, is_input)) in [(input, true), (output, false)].into_iter().enumerate() {
                let r = Rect::from_min_size(pos2(rect.min.x + NAME_W + k as f32 * ROUTE_W, y), vec2(ROUTE_W, ROW_H)).shrink2(vec2(3.0, 2.0));
                let resp = ui.interact(r, ui.scope_id().with(("rm_route", row.0, is_input)), Sense::click());
                p.rect(r, 3.0, if resp.hovered() { t.button_hi } else { t.button }, Stroke::new(1.0, t.border), StrokeKind::Inside);
                p.with_clip_rect(r.shrink(2.0)).text(r.left_center() + vec2(5.0, 0.0), Align2::LEFT_CENTER, text, regular(11.0), t.text);
                let tip = if is_input { "Hardware input (the channel it records from) or bus" } else { "Output: main, none or a bus" };
                route_buttons.push((resp.on_hover_text(tip), row, is_input));
            }
            for (c, col) in cols.iter().enumerate() {
                let r = Rect::from_min_size(pos2(x_cells + c as f32 * CELL_W, y), vec2(CELL_W, ROW_H)).shrink2(vec2(4.0, 3.0));
                let state = cell(track, col);
                let resp =
                    ui.interact(r, ui.scope_id().with(("rm_cell", row.0, c)), if state == Cell::Closed { Sense::hover() } else { Sense::click() });
                draw_cell(&p, r, state, resp.hovered());
                let what = match state {
                    Cell::Closed if matches!(col, Column::Track { id, .. } if *id == row) => String::new(),
                    Cell::Closed => format!("{} takes sends in the Reaper layout", col.name()),
                    Cell::Off if *col == Column::Main => format!("Route {} to Main", track.name),
                    Cell::Off => format!("Send {} to {}", track.name, col.name()),
                    _ if *col == Column::Main => format!("{} goes to Main (click to route it nowhere)", track.name),
                    Cell::PreFader => format!("{} → {}: pre-fader (click to remove)", track.name, col.name()),
                    Cell::PostFader => format!("{} → {}: post-fader (click to remove)", track.name, col.name()),
                };
                let resp = if what.is_empty() { resp } else { resp.on_hover_text(what) };
                if resp.clicked() {
                    clicked = Some((row, col.clone()));
                }
                if matches!(state, Cell::PreFader | Cell::PostFader) && *col != Column::Main {
                    resp.context_menu(|ui| {
                        if ui.radio(state == Cell::PostFader, "Post-fader").clicked() {
                            pre = Some((row, col.clone(), false));
                            ui.close();
                        }
                        if ui.radio(state == Cell::PreFader, "Pre-fader").clicked() {
                            pre = Some((row, col.clone(), true));
                            ui.close();
                        }
                        ui.separator();
                        if ui.button("Remove send").clicked() {
                            clicked = Some((row, col.clone()));
                            ui.close();
                        }
                    });
                }
            }
        }
        // Grid lines between receivers.
        for c in 0..=cols.len() {
            let x = x_cells + c as f32 * CELL_W;
            p.line_segment([pos2(x, rect.min.y), pos2(x, rect.max.y)], Stroke::new(1.0, t.border.gamma_multiply(0.5)));
        }
        // The input and output menus open under their buttons (the mixer's own menus).
        for (resp, row, is_input) in &route_buttons {
            crate::mix_window::route_popup(app, resp, *row, *is_input);
        }
    });
    if let Some((row, col)) = clicked
        && let Err(e) = toggle(app, row, &col)
    {
        app.ui.status = e;
    }
    if let Some((row, col, on)) = pre {
        set_pre(app, row, &col, on);
    }
}

fn draw_cell(p: &egui::Painter, r: Rect, state: Cell, hovered: bool) {
    let t = Tokens::current();
    match state {
        Cell::Closed => {
            p.rect_filled(r, 2.0, t.panel_bg2.gamma_multiply(0.6));
        }
        Cell::Off => {
            p.rect(r, 2.0, if hovered { t.button_hi } else { Color32::TRANSPARENT }, Stroke::new(1.0, t.border), StrokeKind::Inside);
        }
        Cell::PostFader => {
            p.rect_filled(r, 2.0, t.accent);
        }
        Cell::PreFader => {
            p.rect_filled(r, 2.0, t.accent);
            p.text(r.center(), Align2::CENTER_CENTER, "PRE", bold(9.5), Color32::WHITE);
        }
    }
}

fn legend(ui: &mut egui::Ui) {
    let t = Tokens::current();
    ui.horizontal(|ui| {
        for (state, label) in
            [(Cell::PostFader, "post-fader send"), (Cell::PreFader, "pre-fader send"), (Cell::Off, "no send"), (Cell::Closed, "can't send")]
        {
            let (r, _) = ui.allocate_exact_size(vec2(30.0, 14.0), Sense::hover());
            draw_cell(ui.painter(), r, state, false);
            ui.label(egui::RichText::new(label).font(regular(11.0)).color(t.text_dim));
            ui.add_space(8.0);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Services;
    use soundcraft_engine::Engine;

    fn app(layout: LayoutPreset) -> (SoundApp, TrackId, TrackId, TrackId) {
        let mut app = SoundApp::new(Engine::default(), None, Services::default());
        app.ui.layout = layout;
        app.run("track.new", json!({"kind": "audio", "name": "Gtr", "format": "stereo"})).unwrap();
        app.run("track.new", json!({"kind": "instrument", "name": "Keys"})).unwrap();
        app.run("track.new", json!({"kind": "aux", "name": "Verb", "format": "stereo"})).unwrap();
        let s = app.engine.session();
        let id = |n: &str| s.track_by_name(n).unwrap().id;
        let (g, k, v) = (id("Gtr"), id("Keys"), id("Verb"));
        (app, g, k, v)
    }

    fn col(app: &SoundApp, id: TrackId) -> Column {
        columns(app.engine.session(), app.ui.layout).into_iter().find(|c| matches!(c, Column::Track { id: x, .. } if *x == id)).unwrap()
    }

    fn click(app: &mut SoundApp, row: TrackId, column: TrackId) -> Result<Value, String> {
        let c = col(app, column);
        toggle(app, row, &c)
    }

    #[test]
    fn the_grid_lists_every_track_and_main() {
        let (app, g, k, v) = app(LayoutPreset::Reaper);
        let cols = columns(app.engine.session(), app.ui.layout);
        assert_eq!(cols.first(), Some(&Column::Main));
        for id in [g, k, v] {
            assert!(matches!(col(&app, id), Column::Track { open: true, .. }));
        }
    }

    #[test]
    fn classic_greys_out_audio_and_instrument_columns() {
        let (mut app, g, k, v) = app(LayoutPreset::Classic);
        assert!(matches!(col(&app, g), Column::Track { open: false, .. }));
        assert!(matches!(col(&app, k), Column::Track { open: false, .. }));
        assert!(matches!(col(&app, v), Column::Track { open: true, .. }));
        assert!(click(&mut app, g, k).is_err());
        assert!(app.engine.session().track(g).unwrap().mixer.sends.iter().all(Option::is_none));
    }

    #[test]
    fn a_click_adds_a_send_to_an_aux_and_another_removes_it() {
        let (mut app, g, _, v) = app(LayoutPreset::Classic);
        click(&mut app, g, v).unwrap();
        let s = app.engine.session();
        let Route::Bus(bus) = s.track(v).unwrap().mixer.input.clone() else { panic!("the aux got an input bus") };
        let sl = s.track(g).unwrap().mixer.sends.iter().flatten().next().unwrap();
        assert_eq!(sl.target, Route::Bus(bus));
        assert_eq!(sl.level_db, 0.0, "added at unity, so it is heard");
        assert_eq!(cell(s.track(g).unwrap(), &col(&app, v)), Cell::PostFader);
        // Right-click › Pre-fader.
        {
            let c = col(&app, v);
            set_pre(&mut app, g, &c, true)
        };
        let s = app.engine.session();
        assert_eq!(cell(s.track(g).unwrap(), &col(&app, v)), Cell::PreFader);
        click(&mut app, g, v).unwrap();
        let s = app.engine.session();
        assert!(s.track(g).unwrap().mixer.sends.iter().all(Option::is_none));
        assert_eq!(cell(s.track(g).unwrap(), &col(&app, v)), Cell::Off);
    }

    #[test]
    fn in_the_reaper_layout_an_instrument_track_receives_and_lets_go() {
        let (mut app, g, k, _) = app(LayoutPreset::Reaper);
        click(&mut app, g, k).unwrap();
        let s = app.engine.session();
        let bus = s.track(k).unwrap().mixer.receive.expect("Keys got a receive");
        assert_eq!(send_to(s.track(g).unwrap(), bus).map(|(_, pre)| pre), Some(false));
        // Itself: closed.
        assert!(click(&mut app, k, k).is_err());
        click(&mut app, g, k).unwrap();
        let s = app.engine.session();
        assert_eq!(s.track(k).unwrap().mixer.receive, None, "nothing sends to it any more");
        assert!(s.bus(bus).is_none(), "its receive bus is gone");
    }

    #[test]
    fn the_main_column_routes_the_output() {
        let (mut app, g, _, _) = app(LayoutPreset::Reaper);
        assert_eq!(app.engine.session().track(g).unwrap().mixer.output, Route::Main);
        toggle(&mut app, g, &Column::Main).unwrap();
        assert_eq!(app.engine.session().track(g).unwrap().mixer.output, Route::None);
        toggle(&mut app, g, &Column::Main).unwrap();
        assert_eq!(app.engine.session().track(g).unwrap().mixer.output, Route::Main);
    }
}
