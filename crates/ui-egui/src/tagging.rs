//! Track tags in the UI: the chips on track headers and mixer strips, the Track Tags window for
//! the selected tracks, and the Smart Tag Scan window that suggests tags for every track.

use crate::SoundApp;
use crate::theme::{Tokens, regular};
use egui::{Align2, Color32, CornerRadius, Rect, Sense, Stroke, StrokeKind, pos2, vec2};
use serde_json::{Value, json};
use soundcraft_engine::tags::{KNOWN, Suggestion};
use soundcraft_model::TrackId;

/// The tagging windows' state.
#[derive(Default)]
pub struct TaggingUi {
    /// The Track Tags window: the tracks it edits and the tags as typed.
    pub editor: Option<(Vec<TrackId>, String)>,
    /// The Smart Tag Scan window, when open.
    pub scan: Option<ScanWindow>,
}

/// One row of the scan: a track, its suggestion and the tags as the user fixed them.
pub struct ScanRow {
    pub track: TrackId,
    pub name: String,
    pub suggested: Vec<String>,
    pub text: String,
    pub reasons: Vec<String>,
    pub apply: bool,
}

#[derive(Default)]
pub struct ScanWindow {
    pub rows: Vec<ScanRow>,
    /// Group the tracks by their tags when applying.
    pub group: bool,
    /// What the last Apply did.
    pub note: String,
    /// Start a scan when the window next shows.
    pub start: bool,
    #[cfg(not(target_arch = "wasm32"))]
    running: Option<std::sync::mpsc::Receiver<Vec<Suggestion>>>,
}

/// Tags typed in a field: comma-separated.
pub fn parse(text: &str) -> Vec<String> {
    soundcraft_model::clean_tags(text.split(','))
}

/// Tags as one line for a field.
pub fn join(tags: &[String]) -> String {
    tags.join(", ")
}

/// Paint `tags` as chips from the left of `r`, clipped to it, with text `size` points high.
pub fn chips(ui: &egui::Ui, r: Rect, tags: &[String], size: f32) {
    let t = Tokens::current();
    let painter = ui.painter().with_clip_rect(r.intersect(ui.clip_rect()));
    let mut x = r.min.x;
    for tag in tags {
        if x >= r.max.x - 4.0 {
            break;
        }
        let galley = painter.layout_no_wrap(tag.clone(), regular(size), t.text);
        let w = galley.size().x + 6.0;
        let chip = Rect::from_min_size(pos2(x, r.min.y), vec2(w, r.height()));
        painter.rect(chip, CornerRadius::same(3), t.accent.gamma_multiply(0.35), Stroke::new(1.0, t.accent.gamma_multiply(0.7)), StrokeKind::Inside);
        painter.galley(pos2(chip.min.x + 3.0, chip.center().y - galley.size().y / 2.0), galley, t.text);
        x += w + 3.0;
    }
}

/// The tag row of a track header, `r` wide: chips, or nothing when untagged. A click opens the
/// Track Tags window for the track.
pub fn header_row(app: &mut SoundApp, ui: &egui::Ui, id: TrackId, r: Rect, tags: &[String]) {
    if tags.is_empty() {
        return;
    }
    chips(ui, r, tags, 9.0);
    let resp = ui.interact(r, ui.scope_id().with(("tags", id.0)), Sense::click()).on_hover_text(format!("Tags: {}", join(tags)));
    if resp.clicked() {
        open_editor(app, vec![id]);
    }
}

/// Open the Track Tags window for `tracks`.
pub fn open_editor(app: &mut SoundApp, tracks: Vec<TrackId>) {
    let s = app.engine.session();
    let first = tracks.first().and_then(|t| s.track(*t)).map(|t| join(&t.tags)).unwrap_or_default();
    app.tagging.editor = Some((tracks, first));
}

/// Open the Smart Tag Scan window; it scans as it opens.
pub fn open_scan(app: &mut SoundApp) {
    let group = app.engine.session().tagging.group || app.tagging.scan.as_ref().is_none_or(|w| w.group);
    app.tagging.scan = Some(ScanWindow { group, start: true, ..ScanWindow::default() });
}

fn rows(found: Vec<Suggestion>) -> Vec<ScanRow> {
    found
        .into_iter()
        .map(|g| ScanRow {
            track: g.track,
            name: g.name,
            text: join(&g.tags),
            apply: !g.tags.is_empty() || !g.current.is_empty(),
            suggested: g.tags,
            reasons: g.reasons,
        })
        .collect()
}

fn start_scan(app: &SoundApp, w: &mut ScanWindow, ctx: &egui::Context) {
    let session = app.engine.session_arc();
    #[cfg(not(target_arch = "wasm32"))]
    {
        let (tx, rx) = std::sync::mpsc::channel();
        let ctx = ctx.clone();
        let spawned = std::thread::Builder::new().name("tag-scan".into()).spawn(move || {
            let _ = tx.send(soundcraft_engine::tags::scan(&session, &[], true));
            ctx.request_repaint();
        });
        if spawned.is_ok() {
            w.running = Some(rx);
            w.rows.clear();
            return;
        }
        w.rows = rows(soundcraft_engine::tags::scan(&app.engine.session_arc(), &[], false));
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = ctx;
        w.rows = rows(soundcraft_engine::tags::scan(&session, &[], true));
    }
}

/// The confirm command for the rows the user ticked.
pub fn apply_action(w: &ScanWindow) -> (String, Value) {
    let items: Vec<Value> =
        w.rows.iter().filter(|r| r.apply).map(|r| json!({"track": r.track.0, "tags": parse(&r.text), "suggested": r.suggested})).collect();
    ("track.tag_confirm".into(), json!({"items": items, "group": w.group}))
}

pub fn show(app: &mut SoundApp, ctx: &egui::Context) {
    editor_window(app, ctx);
    scan_window(app, ctx);
}

fn editor_window(app: &mut SoundApp, ctx: &egui::Context) {
    let Some((tracks, mut text)) = app.tagging.editor.take() else { return };
    let mut open = true;
    let mut action = None;
    let names: Vec<String> = tracks.iter().filter_map(|t| app.engine.session().track(*t)).map(|t| t.name.clone()).collect();
    egui::Window::new("Track Tags").open(&mut open).collapsible(false).resizable(false).anchor(Align2::CENTER_CENTER, vec2(0.0, -80.0)).show(
        ctx,
        |ui| {
            ui.label(if names.len() == 1 { names.join("") } else { format!("{} tracks", names.len()) });
            let field = crate::widgets::named_text_field(
                ui.add(egui::TextEdit::singleline(&mut text).desired_width(260.0).hint_text("Vocals, Dubs")),
                "Tags",
            );
            ui.horizontal_wrapped(|ui| {
                ui.set_max_width(280.0);
                for k in KNOWN {
                    let mut on = parse(&text).iter().any(|t| t.eq_ignore_ascii_case(k));
                    if ui.toggle_value(&mut on, k).clicked() {
                        let mut tags = parse(&text);
                        tags.retain(|t| !t.eq_ignore_ascii_case(k));
                        if on {
                            tags.push(k.to_string());
                        }
                        text = join(&tags);
                    }
                }
            });
            let enter = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            ui.horizontal(|ui| {
                if ui.button("OK").clicked() || enter {
                    action = Some(json!({"tracks": tracks.iter().map(|t| t.0).collect::<Vec<_>>(), "tags": parse(&text)}));
                }
                if ui.button("Cancel").clicked() {
                    action = Some(Value::Null);
                }
            });
        },
    );
    match action {
        Some(Value::Null) => {}
        Some(p) => {
            let _ = app.run("track.tags", p);
        }
        None if open => app.tagging.editor = Some((tracks, text)),
        None => {}
    }
}

fn scan_window(app: &mut SoundApp, ctx: &egui::Context) {
    let Some(mut w) = app.tagging.scan.take() else { return };
    if std::mem::take(&mut w.start) {
        start_scan(app, &mut w, ctx);
    }
    #[cfg(not(target_arch = "wasm32"))]
    let busy = match w.running.as_ref().map(std::sync::mpsc::Receiver::try_recv) {
        Some(Ok(found)) => {
            w.rows = rows(found);
            w.running = None;
            false
        }
        Some(Err(std::sync::mpsc::TryRecvError::Empty)) => true,
        Some(Err(std::sync::mpsc::TryRecvError::Disconnected)) => {
            w.running = None;
            w.note = "The scan stopped before it finished.".into();
            false
        }
        None => false,
    };
    #[cfg(target_arch = "wasm32")]
    let busy = false;
    let mut open = true;
    let (mut apply, mut rescan) = (false, false);
    let t = Tokens::current();
    egui::Window::new("Smart Tag Scan").open(&mut open).collapsible(false).resizable(true).default_size(vec2(620.0, 420.0)).show(ctx, |ui| {
        if busy {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Reading track names and listening to the audio…");
            });
            return;
        }
        ui.label(egui::RichText::new("Fix any tag that is wrong, then Apply. The next scan learns from your fixes.").color(t.text_dim));
        egui::ScrollArea::vertical().max_height(ui.available_height() - 64.0).show(ui, |ui| {
            egui::Grid::new("tag-scan").num_columns(3).striped(true).spacing(vec2(10.0, 4.0)).show(ui, |ui| {
                for r in &mut w.rows {
                    crate::widgets::checkbox(ui, &mut r.apply, "");
                    ui.label(&r.name);
                    let changed = parse(&r.text) != r.suggested;
                    let edit = ui.add(egui::TextEdit::singleline(&mut r.text).desired_width(260.0).hint_text("no tags").text_color(if changed {
                        t.accent
                    } else {
                        t.text
                    }));
                    let edit = crate::widgets::named_text_field(edit, &format!("Tags for {}", r.name));
                    if !r.reasons.is_empty() {
                        edit.on_hover_text(r.reasons.join("\n"));
                    }
                    if changed {
                        r.apply = true;
                    }
                    ui.end_row();
                }
            });
        });
        if w.rows.is_empty() {
            ui.label("No audio, instrument, MIDI or tab tracks to tag.");
        }
        ui.separator();
        crate::widgets::checkbox(ui, &mut w.group, "Group tracks by tags (drums in a folder, vocals in folders, DI and mics on a bus)");
        ui.horizontal(|ui| {
            apply = ui.add_enabled(w.rows.iter().any(|r| r.apply), egui::Button::new("Apply")).clicked();
            rescan = ui.button("Scan Again").clicked();
            if !w.note.is_empty() {
                ui.label(egui::RichText::new(&w.note).color(t.text_dim));
            }
        });
    });
    if apply {
        let (id, p) = apply_action(&w);
        w.note = match app.run(&id, p) {
            Ok(r) => format!(
                "Tagged {} tracks{}{}.",
                r["confirmed"],
                if r["learned"].as_u64().unwrap_or(0) > 0 { format!(", learned {} fixes", r["learned"]) } else { String::new() },
                if r["moved"].as_u64().unwrap_or(0) > 0 { format!(", moved {} into groups", r["moved"]) } else { String::new() }
            ),
            Err(e) => e.to_string(),
        };
        for r in &mut w.rows {
            if r.apply {
                r.suggested = parse(&r.text);
            }
        }
    }
    if rescan {
        w.note.clear();
        start_scan(app, &mut w, ctx);
    }
    if open {
        app.tagging.scan = Some(w);
    }
}

/// The colour a mixer strip's tag line is drawn in.
pub fn tag_text_color() -> Color32 {
    Tokens::current().accent
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Services;

    /// Show the tagging windows until the scan has rows (it runs on a worker thread).
    fn scanned(app: &mut SoundApp, ctx: &egui::Context) {
        for _ in 0..500 {
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| show(app, ui.ctx()));
            out.textures_delta.clear();
            if app.tagging.scan.as_ref().is_some_and(|w| !w.rows.is_empty()) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("the scan never finished");
    }

    #[test]
    fn the_scan_window_applies_a_fix_and_the_next_scan_learns_it() {
        let mut app = SoundApp::new(soundcraft_engine::Engine::default(), None, Services::default());
        for name in ["Hamish", "Kick"] {
            app.run("track.new", json!({"count": 1, "format": "mono", "name": name})).unwrap();
        }
        let r = crate::menus::run_ui_command(&mut app, "window.tag_scan", &json!({}));
        assert!(matches!(r, Some(Ok(_))));
        let ctx = egui::Context::default();
        scanned(&mut app, &ctx);
        let w = app.tagging.scan.as_mut().unwrap();
        let row = |w: &mut ScanWindow, name: &str| w.rows.iter().position(|r| r.name == name).unwrap();
        let (h, k) = (row(w, "Hamish"), row(w, "Kick"));
        assert!(w.rows[h].suggested.is_empty() && !w.rows[h].apply);
        assert_eq!(w.rows[k].suggested, ["Drums"]);
        // The user types the singer's tags.
        w.rows[h].text = "vocals, Mains".into();
        w.rows[h].apply = true;
        let (id, p) = apply_action(w);
        assert_eq!(p["group"], true);
        app.run(&id, p).unwrap();
        let s = app.engine.session();
        assert_eq!(s.track_by_name("Hamish").unwrap().tags, ["Vocals", "Mains"]);
        assert_eq!(s.track_by_name("Kick").unwrap().tags, ["Drums"]);
        assert!(s.track_by_name("Kick").unwrap().folder.is_some(), "grouped into the drum folder");
        // Scan again: the fix is now the suggestion.
        open_scan(&mut app);
        scanned(&mut app, &ctx);
        let w = app.tagging.scan.as_mut().unwrap();
        let h = row(w, "Hamish");
        assert_eq!(w.rows[h].suggested, ["Vocals", "Mains"]);
    }

    #[test]
    fn the_track_tags_window_needs_a_selection() {
        let mut app = SoundApp::new(soundcraft_engine::Engine::default(), None, Services::default());
        assert!(matches!(crate::menus::run_ui_command(&mut app, "window.track_tags", &json!({})), Some(Err(_))));
        app.run("track.new", json!({"count": 1, "format": "mono", "name": "Vox"})).unwrap();
        app.run("track.tags", json!({"track": "Vox", "tags": "Vocals"})).unwrap();
        app.engine.execute("edit.select_all_tracks", &json!({})).ok();
        let id = app.engine.session().track_by_name("Vox").unwrap().id;
        open_editor(&mut app, vec![id]);
        assert_eq!(app.tagging.editor.as_ref().map(|e| e.1.as_str()), Some("Vocals"));
        assert_eq!(parse(" a, b ,, A"), ["a", "b"]);
    }
}
