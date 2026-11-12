//! The Action List (Window › Action List…): every command in Solara, searchable. Pick one to run
//! it, or give it a key in the active layout's key set (see `keymap`).

use crate::SoundApp;
use crate::theme::{Tokens, mono, regular};
use egui::{Key, vec2};

/// Height of one row in the list.
const ROW_H: f32 = 20.0;

/// One command the list shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Action {
    pub id: String,
    pub label: String,
    /// Menu path ("Edit › Separate"), empty when the command is in no menu.
    pub path: String,
}

/// Every engine and UI command once, best match first. An empty search lists them by name.
pub fn search(query: &str) -> Vec<Action> {
    let q = query.trim();
    let mut seen = std::collections::HashSet::new();
    let mut hits: Vec<(i32, Action)> = crate::palette::entries()
        .into_iter()
        .filter(|e| seen.insert(e.id.clone()))
        .filter_map(|e| {
            crate::palette::score(&format!("{} {} {}", e.label, e.path, e.id), q).map(|s| (s, Action { id: e.id, label: e.label, path: e.path }))
        })
        .collect();
    hits.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.label.cmp(&b.1.label)).then_with(|| a.1.id.cmp(&b.1.id)));
    hits.into_iter().map(|(_, a)| a).collect()
}

/// Run an action the way its menu item would (dialogs included).
pub fn run(app: &mut SoundApp, action: &Action) {
    let path = action.path.replace(" › ", " > ");
    crate::menus::invoke_menu(app, &action.id, &path);
}

pub fn show(app: &mut SoundApp, ctx: &egui::Context) {
    if !app.ui.show_action_list {
        app.keymap_capture = None;
        return;
    }
    let t = Tokens::current();
    let mac = ctx.os() == egui::os::OperatingSystem::Mac;
    let mut open = true;
    let mut run_now: Option<Action> = None;
    let set = crate::keymap::set_id(app.ui.layout);
    egui::Window::new("Action List")
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .default_size(vec2(720.0, 480.0))
        .min_size(vec2(420.0, 240.0))
        .show(ctx, |ui| {
            let typing = ui
                .horizontal(|ui| {
                    let field =
                        egui::TextEdit::singleline(&mut app.action_search).hint_text("Search actions…").desired_width(ui.available_width() - 150.0);
                    let resp = ui.add(field);
                    if app.keymap_capture.is_some() {
                        resp.surrender_focus();
                    }
                    ui.label(egui::RichText::new(format!("Key set: {}", crate::keymap::set_label(set))).font(regular(11.5)).color(t.text_dim));
                    resp.has_focus() || resp.lost_focus()
                })
                .inner;
            let hits = search(&app.action_search);
            if app.action_selected.as_ref().is_none_or(|id| !hits.iter().any(|a| &a.id == id)) {
                app.action_selected = hits.first().map(|a| a.id.clone());
            }
            let (down, up, enter) = ctx.input(|i| (i.key_pressed(Key::ArrowDown), i.key_pressed(Key::ArrowUp), i.key_pressed(Key::Enter)));
            let at = app.action_selected.as_ref().and_then(|id| hits.iter().position(|a| &a.id == id)).unwrap_or(0);
            // In the search field, Up and Down move the selection and Enter runs it.
            if typing && app.keymap_capture.is_none() {
                if down {
                    app.action_selected = hits.get((at + 1).min(hits.len().saturating_sub(1))).map(|a| a.id.clone());
                }
                if up {
                    app.action_selected = hits.get(at.saturating_sub(1)).map(|a| a.id.clone());
                }
                if enter {
                    run_now = hits.get(at).cloned();
                }
            }
            egui::Panel::bottom("action_list_footer").exact_size(34.0).frame(egui::Frame::NONE).show(ui, |ui| {
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    let chosen = app.action_selected.as_ref().and_then(|id| hits.iter().find(|a| &a.id == id)).cloned();
                    let has = chosen.is_some();
                    if ui.add_enabled(has, egui::Button::new("Run")).on_hover_text("Run the selected action (Enter)").clicked() {
                        run_now = chosen.clone();
                    }
                    let capturing = app.keymap_capture.is_some();
                    let set_key = ui
                        .add_enabled(has && !capturing, egui::Button::new("Set key…"))
                        .on_hover_text(format!("Give the selected action a key in the {} key set", crate::keymap::set_label(set)));
                    if set_key.clicked()
                        && let Some(a) = &chosen
                    {
                        // The next key press is the new key; the button must not keep focus, or Space
                        // and Enter would press it again instead.
                        set_key.surrender_focus();
                        app.keymap_capture = Some(a.id.clone());
                        app.ui.status = format!("Press a key for {} (Esc cancels)", a.label);
                    }
                    let assigned = chosen.as_ref().is_some_and(|a| app.ui.key_overrides.get(set).is_some_and(|m| m.contains_key(&a.id)));
                    if ui
                        .add_enabled(assigned && !capturing, egui::Button::new("Default key"))
                        .on_hover_text("Go back to the key set's own key")
                        .clicked()
                        && let Some(a) = &chosen
                    {
                        match crate::keymap::assign(app, &a.id, None) {
                            Ok(_) => app.ui.status = format!("{} is back to its default key", a.label),
                            Err(e) => app.ui.status = e,
                        }
                    }
                    if let Some(id) = &app.keymap_capture {
                        ui.label(
                            egui::RichText::new(format!("Press a key for {} … Esc cancels", crate::keymap::label(id)))
                                .font(regular(12.0))
                                .color(t.accent),
                        );
                    }
                });
            });
            ui.separator();
            let list_h = ui.available_height().max(ROW_H * 3.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                let w = ui.available_width();
                header(ui, "Action", w * 0.42);
                header(ui, "Menu", w * 0.38);
                header(ui, "Key", w * 0.20);
            });
            egui::ScrollArea::vertical().id_salt("action_list_rows").max_height(list_h).auto_shrink([false, false]).show_rows(
                ui,
                ROW_H,
                hits.len(),
                |ui, range| {
                    for a in hits.get(range).unwrap_or_default() {
                        let selected = app.action_selected.as_deref() == Some(a.id.as_str());
                        let key = crate::keymap::chord_for(app, &a.id).map(|c| crate::keymap::display(&c, mac)).unwrap_or_default();
                        let w = ui.available_width();
                        let (rect, resp) = ui.allocate_exact_size(vec2(w, ROW_H), egui::Sense::click());
                        if selected {
                            ui.painter().rect_filled(rect, 2.0, t.accent_dark);
                        } else if resp.hovered() {
                            ui.painter().rect_filled(rect, 2.0, t.button);
                        }
                        let text = if selected { egui::Color32::WHITE } else { t.text };
                        let y = rect.center().y;
                        let cell = |x: f32, width: f32, s: &str, font: egui::FontId, color: egui::Color32| {
                            let r = egui::Rect::from_min_max(
                                egui::pos2(rect.min.x + x, rect.min.y),
                                egui::pos2(rect.min.x + x + width - 6.0, rect.max.y),
                            );
                            ui.painter().with_clip_rect(r).text(egui::pos2(r.min.x + 4.0, y), egui::Align2::LEFT_CENTER, s, font, color);
                        };
                        cell(0.0, w * 0.42, &a.label, regular(12.0), text);
                        cell(w * 0.42, w * 0.38, &a.path, regular(11.0), if selected { text } else { t.text_dim });
                        cell(w * 0.80, w * 0.20, &key, mono(11.0), text);
                        let resp = resp.on_hover_text(&a.id);
                        if resp.clicked() {
                            app.action_selected = Some(a.id.clone());
                        }
                        if resp.double_clicked() {
                            run_now = Some(a.clone());
                        }
                    }
                },
            );
        });
    if let Some(a) = run_now {
        run(app, &a);
    }
    if !open {
        app.ui.show_action_list = false;
        app.keymap_capture = None;
    }
}

fn header(ui: &mut egui::Ui, text: &str, w: f32) {
    let t = Tokens::current();
    let (r, _) = ui.allocate_exact_size(vec2(w, 16.0), egui::Sense::hover());
    ui.painter().text(egui::pos2(r.min.x + 4.0, r.center().y), egui::Align2::LEFT_CENTER, text, regular(11.0), t.text_dim);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Services;
    use crate::new_layout::LayoutPreset;
    use egui::Modifiers;
    use soundcraft_engine::Engine;

    fn app(layout: LayoutPreset) -> SoundApp {
        let mut app = SoundApp::new(Engine::default(), None, Services::default());
        app.ui.layout = layout;
        app
    }

    #[test]
    fn search_finds_commands_and_lists_each_once() {
        let hits = search("new track");
        assert!(hits.iter().any(|a| a.id == "track.new"), "{hits:?}");
        let all = search("");
        let mut ids: Vec<&str> = all.iter().map(|a| a.id.as_str()).collect();
        let n = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), n, "every command once");
        assert!(n > 100, "every command: {n}");
        assert!(all.iter().any(|a| a.id == "window.routing_matrix"));
        assert!(search("zzzzqqqq").is_empty());
    }

    #[test]
    fn a_key_set_in_the_list_runs_its_command_in_that_layout_only() {
        let mut app = app(LayoutPreset::Reaper);
        app.ui.show_action_list = true;
        app.keymap_capture = Some("window.routing_matrix".into());
        assert!(crate::keymap::capture(&mut app, Key::F7, Modifiers::NONE, false));
        assert_eq!(app.keymap_capture, None);
        assert_eq!(crate::keymap::chord_for(&app, "window.routing_matrix").as_deref(), Some("F7"));
        assert!(!app.ui.show_routing_matrix);
        assert!(crate::keymap::dispatch(&mut app, Key::F7, Modifiers::NONE, false));
        assert!(app.ui.show_routing_matrix);
        // NEW has its own key set: F7 is not assigned there.
        app.ui.layout = LayoutPreset::New;
        assert!(!crate::keymap::dispatch(&mut app, Key::F7, Modifiers::NONE, false));
    }

    #[test]
    fn escape_cancels_a_key_assignment() {
        let mut app = app(LayoutPreset::Reaper);
        app.keymap_capture = Some("track.new".into());
        assert!(crate::keymap::capture(&mut app, Key::Escape, Modifiers::NONE, false));
        assert_eq!(app.keymap_capture, None);
        assert!(app.ui.key_overrides.is_empty());
    }
}
