//! Key sets per layout, and the keys people assign themselves (Action List, `ui.keymap`).
//!
//! Each layout brings its own key set: NEW its single-letter keys, Classic the command registry's
//! shortcuts, Reaper-style the keys a Reaper user expects. A key a person assigns belongs to the
//! active key set and wins over that set's defaults. Every set still falls through to the shared
//! registry shortcuts (Cmd+S, Cmd+Z, ...), so switching layout never loses them.

use crate::SoundApp;
use crate::new_layout::LayoutPreset;
use egui::{Key, Modifiers};
use serde_json::{Value, json};
use std::collections::BTreeMap;

/// Keys assigned by hand: key set id → command id → chord ("Cmd+Shift+R").
pub type Overrides = BTreeMap<String, BTreeMap<String, String>>;

/// Most keys one key set may hold by hand. Input-derived, so capped.
pub const MAX_OVERRIDES: usize = 512;

/// The key set a layout brings with it.
pub fn set_id(layout: LayoutPreset) -> &'static str {
    match layout {
        LayoutPreset::Reaper => "reaper",
        LayoutPreset::Classic => "classic",
        _ => "new",
    }
}

/// A key set's display name.
pub fn set_label(id: &str) -> &'static str {
    match id {
        "reaper" => "Reaper",
        "classic" => "Classic",
        _ => "NEW",
    }
}

/// The Reaper-style layout's own keys: chord, command, parameters. Chords use the registry's
/// spelling: `Cmd` is Ctrl off macOS. Space, Ctrl+Z, Ctrl+S and the rest come from the shared
/// shortcuts.
pub const REAPER_KEYS: &[(&str, &str)] = &[
    ("Cmd+R", "transport.record"),
    ("R", "options.loop_playback"),
    ("S", "edit.separate"),
    ("Cmd+T", "track.new"),
    ("Cmd+M", "ui.mixer"),
    ("Alt+R", "window.routing_matrix"),
    // `?` is Shift+/ on most keyboards; hosts report it either way.
    ("Questionmark", "window.action_list"),
    ("Shift+Questionmark", "window.action_list"),
    ("Shift+Slash", "window.action_list"),
    ("Home", "transport.rtz"),
    ("End", "transport.go_to_end"),
    ("Delete", "edit.clear"),
    ("Equals", "view.zoom_in"),
    ("Minus", "view.zoom_out"),
];

/// The default chord a key set gives a command, if any (first one listed).
pub fn default_chord(set: &str, id: &str) -> Option<&'static str> {
    match set {
        "reaper" => REAPER_KEYS.iter().find(|(_, c)| *c == id).map(|(k, _)| *k).or_else(|| crate::menus::shortcut_for(id)),
        "new" => crate::new_layout::NEW_KEYS.iter().find(|(_, c, _)| *c == id).map(|(k, _, _)| *k).or_else(|| crate::menus::shortcut_for(id)),
        _ => crate::menus::shortcut_for(id),
    }
}

/// The chord a command answers to in the active key set: the hand-assigned one, else the default.
pub fn chord_for(app: &SoundApp, id: &str) -> Option<String> {
    let set = set_id(app.ui.layout);
    app.ui.key_overrides.get(set).and_then(|m| m.get(id)).cloned().or_else(|| default_chord(set, id).map(str::to_string))
}

/// "Cmd+Shift+R" for a pressed key. Ctrl off macOS reports as `command`, which is the registry's Cmd.
pub fn chord(key: Key, m: Modifiers, mac: bool) -> String {
    let mut s = String::new();
    if m.command || (!mac && m.ctrl) {
        s.push_str("Cmd+");
    }
    if mac && m.ctrl {
        s.push_str("Ctrl+");
    }
    if m.alt {
        s.push_str("Alt+");
    }
    if m.shift {
        s.push_str("Shift+");
    }
    s.push_str(key.name());
    s
}

/// A chord as people read it on this platform: Ctrl rather than Cmd off macOS.
pub fn display(chord: &str, mac: bool) -> String {
    let label = crate::shortcuts::shortcut_label(chord, mac);
    let mut parts: Vec<&str> = label.split('+').collect();
    if let Some(last) = parts.last_mut() {
        *last = match *last {
            "Questionmark" => "?",
            "Slash" => "/",
            "Equals" => "=",
            "Minus" => "-",
            "Plus" => "+",
            other => other,
        };
    }
    parts.join("+")
}

/// Whether `chord` (registry spelling) is the pressed key.
fn pressed(chord: &str, key: Key, m: Modifiers, mac: bool) -> bool {
    crate::shortcuts::parse(chord).is_some_and(|(want, k)| k == key && crate::shortcuts::mods_match(want, m, mac))
}

/// Keys that only pick modifiers, which cannot be a shortcut on their own.
fn assignable(key: Key) -> bool {
    !matches!(key, Key::Escape)
}

/// What a press does in the active key set before the shared shortcuts: a hand-assigned key first,
/// then the layout's own keys. Returns whether the press was taken.
pub fn dispatch(app: &mut SoundApp, key: Key, m: Modifiers, mac: bool) -> bool {
    let set = set_id(app.ui.layout);
    let assigned = app.ui.key_overrides.get(set).and_then(|o| o.iter().find(|(_, c)| pressed(c, key, m, mac)).map(|(id, _)| id.clone()));
    if let Some(id) = assigned {
        crate::menus::invoke_menu(app, &id, "");
        return true;
    }
    match app.ui.layout {
        LayoutPreset::Reaper => match REAPER_KEYS.iter().find(|(c, _)| pressed(c, key, m, mac)) {
            Some((_, id)) => {
                crate::menus::invoke_menu(app, id, "");
                true
            }
            None => false,
        },
        LayoutPreset::New => crate::new_layout::keys(app, key, m),
        _ => false,
    }
}

/// The Action List is waiting for a key for `id`: the next press is that key. Escape cancels.
/// Returns whether the press was taken.
pub fn capture(app: &mut SoundApp, key: Key, m: Modifiers, mac: bool) -> bool {
    let Some(id) = app.keymap_capture.take() else { return false };
    if key == Key::Escape || !assignable(key) {
        app.escape_taken = key == Key::Escape;
        app.ui.status = "Key assignment canceled".into();
        return true;
    }
    let c = chord(key, m, mac);
    match assign(app, &id, Some(&c)) {
        Ok(_) => app.ui.status = format!("{} is now {}", label(&id), display(&c, mac)),
        Err(e) => app.ui.status = e,
    }
    true
}

/// A command's label for people, from the registry or the UI commands.
pub fn label(id: &str) -> String {
    soundcraft_engine::find_command(id)
        .map(|c| c.label.to_string())
        .or_else(|| crate::menus::UI_COMMANDS.iter().find(|c| c.0 == id).map(|c| c.1.to_string()))
        .unwrap_or_else(|| id.to_string())
}

/// Whether `id` is a command a key can run.
fn known(id: &str) -> bool {
    soundcraft_engine::find_command(id).is_some() || crate::menus::UI_COMMANDS.iter().any(|c| c.0 == id)
}

/// Give `id` the chord `key` in the active key set (taking it from any other command there), or
/// with `None` go back to the set's default.
pub fn assign(app: &mut SoundApp, id: &str, key: Option<&str>) -> Result<Value, String> {
    if !known(id) {
        return Err(format!("unknown command `{id}`"));
    }
    let set = set_id(app.ui.layout).to_string();
    let map = app.ui.key_overrides.entry(set.clone()).or_default();
    match key {
        None => {
            map.remove(id);
        }
        Some(k) => {
            let k = k.trim();
            if crate::shortcuts::parse(k).is_none() {
                return Err(format!("`{k}` is not a key (try Cmd+Shift+R, Alt+F or F5)"));
            }
            if !map.contains_key(id) && map.len() >= MAX_OVERRIDES {
                return Err(format!("the {} key set already holds {MAX_OVERRIDES} assigned keys", set_label(&set)));
            }
            let same = |c: &str| crate::shortcuts::parse(c) == crate::shortcuts::parse(k);
            map.retain(|other, c| other == id || !same(c));
            map.insert(id.to_string(), k.to_string());
        }
    }
    Ok(json!({"set": set, "command": id, "key": app.ui.key_overrides.get(&set).and_then(|m| m.get(id))}))
}

/// `ui.keymap`: `{}` lists the active set's assigned keys; `{command, key}` assigns one;
/// `{command, key: null}` goes back to the default.
pub fn command(app: &mut SoundApp, p: &Value) -> Result<Value, String> {
    let Some(id) = p.get("command").and_then(Value::as_str) else {
        let set = set_id(app.ui.layout);
        return Ok(json!({"set": set, "keys": app.ui.key_overrides.get(set).cloned().unwrap_or_default()}));
    };
    match p.get("key") {
        Some(Value::String(k)) => assign(app, id, Some(k)),
        Some(Value::Null) | None => assign(app, id, None),
        Some(_) => Err("`key` must be a string such as \"Cmd+Shift+R\", or null".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Services;
    use soundcraft_engine::Engine;

    fn app(layout: LayoutPreset) -> SoundApp {
        let mut app = SoundApp::new(Engine::default(), None, Services::default());
        app.ui.layout = layout;
        app
    }

    #[test]
    fn every_layout_key_names_a_real_command_and_a_real_key() {
        for (c, id) in REAPER_KEYS {
            assert!(known(id), "{id}");
            assert!(crate::shortcuts::parse(c).is_some(), "{c}");
        }
        for (c, id, params) in crate::new_layout::NEW_KEYS {
            assert!(known(id), "{id}");
            assert!(crate::shortcuts::parse(c).is_some(), "{c}");
            assert!(serde_json::from_str::<Value>(params).is_ok(), "{params}");
        }
    }

    #[test]
    fn a_pressed_chord_reads_back_as_the_same_key() {
        let m = Modifiers { alt: false, ctrl: true, shift: true, mac_cmd: false, command: true };
        let c = chord(Key::R, m, false);
        assert_eq!(c, "Cmd+Shift+R");
        assert!(pressed(&c, Key::R, m, false));
        assert_eq!(display(&c, false), "Ctrl+Shift+R");
        assert_eq!(display("Questionmark", false), "?");
    }

    #[test]
    fn reaper_keys_work_in_the_reaper_layout_and_new_keys_in_new() {
        let mut app = app(LayoutPreset::Reaper);
        assert!(dispatch(&mut app, Key::R, Modifiers::ALT, false));
        assert!(app.ui.show_routing_matrix, "Alt+R opens the Routing Matrix");
        assert!(dispatch(&mut app, Key::Questionmark, Modifiers::NONE, false));
        assert!(app.ui.show_action_list);
        let open = app.ui.reaper_mixer_open;
        assert!(dispatch(&mut app, Key::M, Modifiers::COMMAND, false));
        assert_ne!(app.ui.reaper_mixer_open, open, "Ctrl+M shows or hides the mixer");
        // B picks NEW's step sequencer only in NEW.
        assert!(!dispatch(&mut app, Key::B, Modifiers::NONE, false));
        app.ui.layout = LayoutPreset::New;
        assert!(!dispatch(&mut app, Key::R, Modifiers::ALT, false));
        assert!(dispatch(&mut app, Key::B, Modifiers::NONE, false));
        assert_eq!(chord_for(&app, "ui.mixer").as_deref(), Some("M"));
        app.ui.layout = LayoutPreset::Reaper;
        assert_eq!(chord_for(&app, "ui.mixer").as_deref(), Some("Cmd+M"));
    }

    #[test]
    fn an_assigned_key_moves_from_the_command_that_had_it() {
        let mut app = app(LayoutPreset::Reaper);
        assign(&mut app, "track.new", Some("F9")).unwrap();
        assign(&mut app, "window.action_list", Some("F9")).unwrap();
        let keys = app.ui.key_overrides.get("reaper").unwrap();
        assert_eq!(keys.get("window.action_list").map(String::as_str), Some("F9"));
        assert!(!keys.contains_key("track.new"));
        assert!(assign(&mut app, "no.such.command", Some("F9")).is_err());
        assert!(assign(&mut app, "track.new", Some("Cmd+Nope")).is_err());
        assert_eq!(
            app.run("ui.keymap", serde_json::json!({"command": "window.action_list", "key": null})).map(|v| v["key"].clone()),
            Ok(Value::Null)
        );
        assert_eq!(app.run("ui.keymap", serde_json::json!({})).map(|v| v["set"].clone()), Ok(serde_json::json!("reaper")));
    }

    #[test]
    fn hand_assigned_keys_are_capped() {
        let mut app = app(LayoutPreset::Reaper);
        let ids: Vec<&str> = soundcraft_engine::command_specs().iter().map(|c| c.id).collect();
        let keys: Vec<String> = (0..MAX_OVERRIDES).map(|i| format!("Cmd+Alt+Shift+F{}", i % 20 + 1)).collect();
        // Fill the set directly: there are fewer distinct chords than the cap.
        let map = app.ui.key_overrides.entry("reaper".into()).or_default();
        for (i, id) in ids.iter().enumerate().take(MAX_OVERRIDES) {
            map.insert((*id).to_string(), keys.get(i).cloned().unwrap_or_default());
        }
        if map.len() == MAX_OVERRIDES
            && let Some(extra) = ids.get(MAX_OVERRIDES)
        {
            assert!(assign(&mut app, extra, Some("F12")).is_err());
        }
    }
}
