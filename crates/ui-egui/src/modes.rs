//! Modes: one engine and one set of commands, six ways to arrange them.
//!
//! A mode is a [`LayoutPreset`] plus the [`Mode`] profile below: the menu bar, the right-click menus
//! for tracks, clips and empty lanes, what a left click does in the timeline, the track header, the
//! rulers, the mixer strips, where the settings live, and the colours. Every entry names an existing
//! catalog path or command id, so a mode only rearranges and relabels what is already there; it never
//! adds a function of its own. Learnt from how each DAW behaves, not from its code, wording beyond
//! feature names, or artwork. The mode logos are original parody art drawn in code: never anyone's
//! real logo or brand mark.
//!
//! Switching mode changes UI state only, never the session (`ui.layout`).

use crate::SoundApp;
use crate::new_layout::LayoutPreset;
use crate::theme::{Skin, Tokens, bold, regular};
use egui::{Align2, Color32, Pos2, Rect, Sense, Shape, Stroke, Ui, pos2, vec2};
use serde_json::{Value, json};

/// A menu bar entry. Labels are the mode's own words; the function behind each one is shared.
#[derive(Debug, Clone, Copy)]
pub enum Item {
    /// A leaf of the catalog menu tree (`menus::tree`) under the mode's own label.
    Path(&'static str, &'static str),
    /// A command by id with JSON parameters. `{}` invokes it menu-style (it may open its dialog).
    Cmd(&'static str, &'static str, &'static str),
    /// A submenu of the mode's own entries.
    Sub(&'static str, &'static [Item]),
    /// A whole catalog submenu, such as `Track > Change Track Width`, under the mode's own label.
    Tree(&'static str, &'static str),
    Sep,
    /// The recent sessions.
    Recent,
    About,
    Quit,
    /// The six modes, as radio items.
    Modes,
    /// Every catalog menu, so no command is out of reach in any mode.
    AllMenus,
}

/// One menu of the bar.
#[derive(Debug, Clone, Copy)]
pub struct Root {
    pub label: &'static str,
    pub items: &'static [Item],
}

/// A right-click menu entry for a track, a clip, or an empty lane.
#[derive(Debug, Clone, Copy)]
pub enum Ctx {
    /// A command on the target: `{"tracks": [...]}` for tracks, the clip (or the edit selection) for clips.
    Run(&'static str, &'static str),
    /// A command with fixed parameters, not aimed at the target (Paste, a new track).
    Plain(&'static str, &'static str, &'static str),
    /// A catalog menu leaf, menu-style (it may open its dialog).
    Path(&'static str, &'static str),
    Sub(&'static str, &'static [Ctx]),
    Sep,
    Rename(&'static str),
    Delete(&'static str),
    Color(&'static str),
    Height(&'static str),
    Folders,
    Tags,
    /// The track's next free insert slot, with the plugin picker.
    InsertFx(&'static str),
    /// The Routing Matrix.
    Routing(&'static str),
    ScrollIntoView,
}

/// What a left click does in a track lane. The session's edit tool is never changed by a mode: modes
/// without tools read the click from where it lands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Click {
    /// The edit tool decides (Pro Tools-style, Classic).
    Tools,
    /// A click on an item selects it and a drag moves it; its edges trim, its top corners fade; a click
    /// on empty space moves the edit cursor and a drag selects time; on an envelope a click adds a point
    /// (Reaper-style, NEW).
    Items,
    /// As `Items`, but a click on an empty instrument lane paints a new clip there, and a right click on a
    /// clip deletes it; the clip's menu is on its title bar (FL-style).
    Paint,
    /// A clip's title bar moves it and its body selects time; its edges trim (Ableton-style).
    TitleBar,
}

/// The track header's controls.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Header {
    /// Name, record, input, solo, mute, playlist and view selectors, and the optional columns.
    ProTools,
    /// The track control panel: number, record arm, name, volume and pan knobs, M, S, FX, routing.
    Reaper,
    /// A playlist track: name, and a mute light (Ctrl-click solos).
    Fl,
    /// On the right of the arrangement: the name bar, track activator, solo, arm, volume and pan.
    Ableton,
    /// Name, mute, solo, record, and a volume slider.
    New,
}

/// The mixer strip's sections, by Mix Window View id, and their captions.
#[derive(Debug, Clone, Copy)]
pub struct MixStyle {
    /// The sections shown; `None` keeps the person's View › Mix Window Views choice.
    pub views: Option<&'static [&'static str]>,
    /// The master strip leads, at the left.
    pub master_first: bool,
    /// Section captions: Pro Tools' names → the mode's.
    pub words: &'static [(&'static str, &'static str)],
}

/// A mode's whole profile.
#[derive(Debug, Clone, Copy)]
pub struct Mode {
    pub preset: LayoutPreset,
    /// `None`: the catalog menu bar (Pro Tools-style names).
    pub menus: Option<&'static [Root]>,
    /// What a clip is called: "Clip", or "Item".
    pub clip_word: &'static str,
    pub click: Click,
    pub header: Header,
    /// Header width, without Pro Tools' optional columns.
    pub header_w: f32,
    /// The track headers sit right of the timeline.
    pub header_right: bool,
    /// The rulers shown; `None` keeps the session's View › Rulers choice.
    pub rulers: Option<&'static [&'static str]>,
    pub track_menu: &'static [Ctx],
    pub clip_menu: &'static [Ctx],
    /// Right-click on an empty part of a lane.
    pub lane_menu: &'static [Ctx],
    pub mix: MixStyle,
    pub skin: Skin,
    /// Where the settings live, as people find them: menu › item, and its key.
    pub settings: &'static str,
}

/// The profile of `preset`.
pub fn of(preset: LayoutPreset) -> &'static Mode {
    match preset {
        LayoutPreset::New => &NEW,
        LayoutPreset::FlStyle => &FL,
        LayoutPreset::Classic => &CLASSIC,
        LayoutPreset::Reaper => &REAPER,
        LayoutPreset::ProToolsStyle => &PRO_TOOLS,
        LayoutPreset::AbletonStyle => &ABLETON,
    }
}

/// The active mode's profile.
pub fn current(app: &SoundApp) -> &'static Mode {
    of(app.ui.layout)
}

// ---------------------------------------------------------------------------------------------
// Pro Tools-style and Classic: the catalog's own menus, tools, columns and rulers.

const PT_TRACK_MENU: &[Ctx] = &[
    Ctx::Rename("Rename..."),
    Ctx::Run("Duplicate...", "track.duplicate"),
    Ctx::Run("Make Inactive", "track.make_inactive"),
    Ctx::Run("Hide", "track.hide"),
    Ctx::Delete("Delete"),
    Ctx::Run("Split into Mono", "track.split_into_mono"),
    Ctx::Run("Commit...", "track.commit"),
    Ctx::Run("Freeze", "track.freeze"),
    Ctx::Run("Move to New Folder...", "track.move_to_new_folder"),
    Ctx::Run("Group...", "track.group"),
    Ctx::Folders,
    Ctx::Tags,
    Ctx::ScrollIntoView,
    Ctx::Sep,
    Ctx::Height("Track Height"),
    Ctx::Color("Track Color"),
];

const PT_CLIP_MENU: &[Ctx] = &[
    Ctx::Run("Mute/Unmute", "edit.mute_clips"),
    Ctx::Run("Edit Lock/Unlock", "clip.edit_lock"),
    Ctx::Run("Time Lock/Unlock", "clip.time_lock"),
    Ctx::Run("Quantize to Grid", "clip.quantize_to_grid"),
    Ctx::Run("Render Clip Gain", "clip.gain_render"),
    Ctx::Run("Bring to Front", "clip.bring_to_front"),
    Ctx::Run("Send to Back", "clip.send_to_back"),
    Ctx::Sep,
    Ctx::Rename("Rename..."),
    Ctx::Delete("Delete"),
];

const PT_LANE_MENU: &[Ctx] = &[Ctx::Plain("Paste", "edit.paste", "{}")];

const PT_MIX: MixStyle = MixStyle { views: None, master_first: false, words: &[] };

pub const PRO_TOOLS: Mode = Mode {
    preset: LayoutPreset::ProToolsStyle,
    menus: None,
    clip_word: "Clip",
    click: Click::Tools,
    header: Header::ProTools,
    header_w: crate::edit_window::HEADER_W,
    header_right: false,
    rulers: None,
    track_menu: PT_TRACK_MENU,
    clip_menu: PT_CLIP_MENU,
    lane_menu: PT_LANE_MENU,
    mix: PT_MIX,
    skin: Skin::ProTools,
    settings: "Setup › Preferences…",
};

pub const CLASSIC: Mode = Mode { preset: LayoutPreset::Classic, skin: Skin::Classic, ..PRO_TOOLS };

// ---------------------------------------------------------------------------------------------
// Reaper-style.

const REAPER_MENUS: &[Root] = &[
    Root {
        label: "File",
        items: &[
            Item::Path("New project", "File > New..."),
            Item::Path("Open project...", "File > Open > Session..."),
            Item::Recent,
            Item::Sep,
            Item::Path("Save project", "File > Save Session"),
            Item::Path("Save project as...", "File > Save Session As..."),
            Item::Path("Save new version of project...", "File > Save Session Copy In..."),
            Item::Path("Save project as template...", "File > Save As Template..."),
            Item::Path("Revert to saved", "File > Revert Session to Saved..."),
            Item::Sep,
            Item::Path("Project settings...", "Setup > Session"),
            Item::Sep,
            Item::Path("Render...", "File > Bounce Mix..."),
            Item::Sub(
                "Import",
                &[
                    Item::Path("Media file...", "File > Import > Audio..."),
                    Item::Path("MIDI file...", "File > Import > MIDI..."),
                    Item::Path("Video file...", "File > Import > Video..."),
                    Item::Path("Project data...", "File > Import > Session Data..."),
                ],
            ),
            Item::Sub(
                "Export",
                &[
                    Item::Path("Project MIDI...", "File > Export > MIDI..."),
                    Item::Path("Selected tracks as new project...", "File > Export > Selected Tracks as New Session..."),
                    Item::Path("Project info as text...", "File > Export > Session Info as Text..."),
                ],
            ),
            Item::Sep,
            Item::Path("Close project", "File > Close Session"),
            Item::Quit,
        ],
    },
    Root {
        label: "Edit",
        items: &[
            Item::Path("Undo", "Edit > Can't Undo"),
            Item::Path("Redo", "Edit > Can't Redo"),
            Item::Path("Undo history", "Window > Undo History"),
            Item::Sep,
            Item::Path("Cut", "Edit > Cut"),
            Item::Path("Copy", "Edit > Copy"),
            Item::Path("Paste", "Edit > Paste"),
            Item::Path("Remove", "Edit > Clear"),
            Item::Sep,
            Item::Path("Select all", "Edit > Select All"),
            Item::Path("Restore last selection", "Edit > Restore Last Selection"),
            Item::Sep,
            Item::Path("Split items at time selection", "Edit > Separate > At Selection"),
            Item::Path("Split items at grid", "Edit > Separate > On Grid"),
            Item::Path("Split items at transients", "Edit > Separate > At Transients"),
            Item::Path("Heal splits in items", "Edit > Heal Separation"),
            Item::Path("Glue items", "Edit > Consolidate"),
            Item::Path("Duplicate items", "Edit > Duplicate"),
            Item::Sep,
            Item::Path("Trim items to time selection", "Edit > Trim > To Selection"),
            Item::Path("Insert empty space at time selection", "Edit > Insert Silence"),
            Item::Path("Remove silence from items...", "Edit > Strip Silence"),
        ],
    },
    Root {
        label: "View",
        items: &[
            Item::Cmd("Mixer", "ui.mixer", "{}"),
            Item::Path("Track manager", "View > Other Displays > Track List"),
            Item::Path("Region/Marker manager", "Window > Memory Locations"),
            Item::Path("Project media", "Window > Clip List"),
            Item::Path("Routing matrix", "View > Routing Matrix"),
            Item::Path("Big clock", "Window > Big Counter"),
            Item::Path("Transport", "Window > Transport"),
            Item::Path("Virtual MIDI keyboard", "Window > MIDI Keyboard"),
            Item::Path("MIDI editor", "Window > MIDI Editor"),
            Item::Path("Performance meter", "Window > System Usage"),
            Item::Sep,
            Item::Cmd("Zoom in", "view.zoom_in", "{}"),
            Item::Cmd("Zoom out", "view.zoom_out", "{}"),
            Item::Cmd("Zoom to time selection", "view.zoom_to_selection", "{}"),
            Item::Cmd("Zoom out project", "view.zoom_fit", "{}"),
            Item::Sep,
            Item::Path("Track numbers", "View > Track Number"),
            Item::Tree("Item appearance", "View > Clip"),
            Item::Tree("Peaks display", "View > Waveforms"),
            Item::Sep,
            Item::AllMenus,
        ],
    },
    Root {
        label: "Insert",
        items: &[
            Item::Cmd("New track", "track.new", r#"{"count":1}"#),
            Item::Path("Multiple new tracks...", "Track > New..."),
            Item::Cmd("Virtual instrument on new track", "track.new", r#"{"kind":"instrument"}"#),
            Item::Cmd("New folder track", "track.new", r#"{"kind":"folder"}"#),
            Item::Sep,
            Item::Path("Media file...", "File > Import > Audio..."),
            Item::Path("New MIDI item", "Clip > New MIDI Clip"),
            Item::Path("Click source", "Track > Create Click Track"),
            Item::Sep,
            Item::Cmd("Marker", "markers.add", "{}"),
            Item::Path("Time signature/tempo change...", "Event > Time Operations > Change Meter..."),
            Item::Path("Empty space in project...", "Event > Time Operations > Insert Time..."),
        ],
    },
    Root {
        label: "Item",
        items: &[
            Item::Path("Rename item...", "Clip > Rename..."),
            Item::Sub(
                "Grouping",
                &[
                    Item::Path("Group items", "Clip > Group"),
                    Item::Path("Remove items from group", "Clip > Ungroup"),
                    Item::Path("Remove all groups", "Clip > Ungroup All"),
                    Item::Path("Regroup items", "Clip > Regroup"),
                ],
            ),
            Item::Sub(
                "Item settings",
                &[
                    Item::Path("Lock item", "Clip > Edit Lock/Unlock"),
                    Item::Path("Lock item to time", "Clip > Time Lock/Unlock"),
                    Item::Path("Mute item", "Edit > Mute"),
                    Item::Path("Loop item source...", "Clip > Loop..."),
                    Item::Path("Unloop item...", "Clip > Unloop..."),
                ],
            ),
            Item::Sep,
            Item::Path("Glue items", "Edit > Consolidate"),
            Item::Path("Split items at time selection", "Edit > Separate > At Selection"),
            Item::Path("Heal splits in items", "Edit > Heal Separation"),
            Item::Sep,
            Item::Sub(
                "Item processing",
                &[
                    Item::Path("Quantize item positions to grid", "Clip > Quantize to Grid"),
                    Item::Path("Apply item volume", "Clip > Clip Gain > Render"),
                    Item::Path("Bypass item volume", "Clip > Clip Gain > Bypass"),
                    Item::Path("Stretch to project tempo", "Clip > Conform to Tempo"),
                    Item::Path("Remove stretch markers", "Clip > Remove Warp"),
                    Item::Path("Remove pitch shift", "Clip > Remove Pitch Shift"),
                ],
            ),
            Item::Tree("Fades", "Edit > Fades"),
            Item::Sep,
            Item::Path("Bring item to front", "Clip > Bring to Front"),
            Item::Path("Send item to back", "Clip > Send to Back"),
            Item::Path("Capture item...", "Clip > Capture..."),
        ],
    },
    Root {
        label: "Track",
        items: &[
            Item::Cmd("Insert new track", "track.new", r#"{"count":1}"#),
            Item::Path("Insert multiple new tracks...", "Track > New..."),
            Item::Path("Duplicate tracks", "Track > Duplicate..."),
            Item::Path("Remove tracks", "Track > Delete"),
            Item::Sep,
            Item::Sub(
                "Render/freeze tracks",
                &[
                    Item::Path("Freeze tracks", "Track > Freeze"),
                    Item::Path("Render tracks to new stem...", "Track > Commit..."),
                    Item::Path("Render tracks in place...", "Track > Bounce..."),
                ],
            ),
            Item::Path("Track grouping...", "Track > Group..."),
            Item::Path("Move tracks to new folder...", "Track > Move to New Folder..."),
            Item::Path("Track routing...", "View > Routing Matrix"),
            Item::Path("Explode stereo to mono", "Track > Split into Mono"),
            Item::Path("Disable track", "Track > Make Inactive"),
            Item::Sep,
            Item::Path("Track tags...", "Track > Tags..."),
            Item::Path("Smart tag scan...", "Track > Smart Tag Scan..."),
            Item::Path("Group tracks by tags", "Track > Group by Tags"),
            Item::Sep,
            Item::Tree("Track channels", "Track > Change Track Width"),
            Item::Tree("Bypass FX", "Track > Bypass Inserts"),
            Item::Tree("Mute sends", "Track > Mute Sends"),
            Item::Path("Save track template...", "Track > Save Track Preset..."),
        ],
    },
    Root {
        label: "Options",
        items: &[
            Item::Sub(
                "Record mode",
                &[
                    Item::Path("Loop record", "Options > Loop Record"),
                    Item::Path("Auto-punch time selection", "Options > QuickPunch"),
                    Item::Path("Track punch", "Options > TrackPunch"),
                ],
            ),
            Item::Path("Metronome enabled", "Options > Click"),
            Item::Path("Metronome settings...", "Setup > Click/Countoff..."),
            Item::Path("Repeat", "Options > Loop Playback"),
            Item::Tree("Auto-scroll view", "Options > Edit Window Scrolling"),
            Item::Path("Move edit cursor on playback", "Options > Insertion Follows Playback"),
            Item::Path("Envelope points move with items", "Options > Automation Follows Edit"),
            Item::Tree("Solo mode", "Options > Solo Mode"),
            Item::Sep,
            Item::Path("Audio device...", "Setup > Playback Engine..."),
            Item::Path("MIDI devices...", "Setup > MIDI > MIDI Input Devices..."),
            Item::Path("Preferences...", "Setup > Preferences..."),
            Item::Sep,
            Item::Modes,
        ],
    },
    Root {
        label: "Actions",
        items: &[Item::Path("Show action list...", "Window > Action List..."), Item::Path("Keyboard shortcuts...", "Setup > Keyboard Shortcuts...")],
    },
    Root {
        label: "Help",
        items: &[Item::About, Item::Cmd("Project info", "window.session_info", "{}"), Item::Cmd("Audio health", "window.audio_health", "{}")],
    },
];

const REAPER_TRACK_MENU: &[Ctx] = &[
    Ctx::Plain("Insert new track", "track.new", r#"{"count":1}"#),
    Ctx::Plain("Insert virtual instrument on new track", "track.new", r#"{"kind":"instrument"}"#),
    Ctx::Path("Insert multiple new tracks...", "Track > New..."),
    Ctx::Sep,
    Ctx::Delete("Remove tracks"),
    Ctx::Run("Duplicate tracks", "track.duplicate"),
    Ctx::Sub("Render/freeze tracks", &[Ctx::Run("Freeze tracks", "track.freeze"), Ctx::Run("Render tracks to stereo stem", "track.commit")]),
    Ctx::Sep,
    Ctx::InsertFx("Track FX"),
    Ctx::Routing("Track routing..."),
    Ctx::Run("Track grouping...", "track.group"),
    Ctx::Run("Move to new folder", "track.move_to_new_folder"),
    Ctx::Folders,
    Ctx::Sep,
    Ctx::Rename("Rename track"),
    Ctx::Color("Track color"),
    Ctx::Height("Track height"),
    Ctx::Tags,
    Ctx::Run("Explode stereo to mono", "track.split_into_mono"),
    Ctx::Run("Disable track", "track.make_inactive"),
    Ctx::Run("Hide track", "track.hide"),
    Ctx::ScrollIntoView,
];

const REAPER_ITEM_MENU: &[Ctx] = &[
    Ctx::Rename("Rename item..."),
    Ctx::Sub(
        "Item settings",
        &[
            Ctx::Run("Mute", "edit.mute_clips"),
            Ctx::Run("Lock", "clip.edit_lock"),
            Ctx::Run("Lock to time", "clip.time_lock"),
            Ctx::Run("Bring to front", "clip.bring_to_front"),
            Ctx::Run("Send to back", "clip.send_to_back"),
        ],
    ),
    Ctx::Sub("Group", &[Ctx::Run("Group items", "clip.group"), Ctx::Run("Remove items from group", "clip.ungroup")]),
    Ctx::Sep,
    Ctx::Plain("Glue items", "edit.consolidate", "{}"),
    Ctx::Plain("Split items at time selection", "edit.separate", "{}"),
    Ctx::Plain("Heal splits in items", "edit.heal", "{}"),
    Ctx::Sep,
    Ctx::Sub(
        "Item processing",
        &[
            Ctx::Run("Quantize item positions to grid", "clip.quantize_to_grid"),
            Ctx::Run("Apply item volume", "clip.gain_render"),
            Ctx::Run("Remove stretch markers", "clip.remove_warp"),
        ],
    ),
    Ctx::Sep,
    Ctx::Plain("Copy items", "edit.copy", "{}"),
    Ctx::Plain("Cut items", "edit.cut", "{}"),
    Ctx::Plain("Paste", "edit.paste", "{}"),
    Ctx::Delete("Remove items"),
];

const REAPER_LANE_MENU: &[Ctx] = &[
    Ctx::Plain("Paste", "edit.paste", "{}"),
    Ctx::Path("Insert new MIDI item", "Clip > New MIDI Clip"),
    Ctx::Path("Insert media file...", "File > Import > Audio..."),
    Ctx::Sep,
    Ctx::Plain("Insert marker", "markers.add", "{}"),
    Ctx::Plain("Select all items", "edit.select_all", "{}"),
    Ctx::Plain("Insert new track", "track.new", r#"{"count":1}"#),
];

pub const REAPER: Mode = Mode {
    preset: LayoutPreset::Reaper,
    menus: Some(REAPER_MENUS),
    clip_word: "Item",
    click: Click::Items,
    header: Header::Reaper,
    header_w: 236.0,
    header_right: false,
    rulers: Some(&["markers", "bars_beats", "min_secs"]),
    track_menu: REAPER_TRACK_MENU,
    clip_menu: REAPER_ITEM_MENU,
    lane_menu: REAPER_LANE_MENU,
    mix: MixStyle {
        views: Some(&["inserts_ae", "sends_ae", "io"]),
        master_first: true,
        words: &[("INSERTS A-E", "FX"), ("INSERTS F-J", "FX 6-10"), ("SENDS A-E", "SENDS"), ("SENDS F-J", "SENDS 6-10"), ("I / O", "ROUTE")],
    },
    skin: Skin::Reaper,
    settings: "Options › Preferences… (Ctrl+P)",
};

// ---------------------------------------------------------------------------------------------
// FL-style.

const FL_MENUS: &[Root] = &[
    Root {
        label: "FILE",
        items: &[
            Item::Path("New", "File > New..."),
            Item::Path("Open...", "File > Open > Session..."),
            Item::Recent,
            Item::Path("Save", "File > Save Session"),
            Item::Path("Save as...", "File > Save Session As..."),
            Item::Path("Save new version", "File > Save Session Copy In..."),
            Item::Sep,
            Item::Sub("Import", &[Item::Path("Audio file...", "File > Import > Audio..."), Item::Path("MIDI file...", "File > Import > MIDI...")]),
            Item::Sub("Export", &[Item::Path("Wave file...", "File > Bounce Mix..."), Item::Path("MIDI file...", "File > Export > MIDI...")]),
            Item::Sep,
            Item::Path("Close project", "File > Close Session"),
            Item::Quit,
        ],
    },
    Root {
        label: "EDIT",
        items: &[
            Item::Path("Undo", "Edit > Can't Undo"),
            Item::Path("Redo", "Edit > Can't Redo"),
            Item::Path("Undo history", "Window > Undo History"),
            Item::Sep,
            Item::Path("Cut", "Edit > Cut"),
            Item::Path("Copy", "Edit > Copy"),
            Item::Path("Paste", "Edit > Paste"),
            Item::Path("Delete", "Edit > Clear"),
            Item::Sep,
            Item::Path("Select all", "Edit > Select All"),
            Item::Path("Clone", "Edit > Duplicate"),
            Item::Path("Chop", "Edit > Separate > At Selection"),
        ],
    },
    Root {
        label: "ADD",
        items: &[
            Item::Cmd("Channel (instrument)", "track.new", r#"{"kind":"instrument"}"#),
            Item::Cmd("Audio track", "track.new", r#"{"kind":"audio"}"#),
            Item::Cmd("Mixer insert", "track.new", r#"{"kind":"aux"}"#),
            Item::Path("More...", "Track > New..."),
            Item::Sep,
            Item::Path("Audio clip...", "File > Import > Audio..."),
            Item::Path("Pattern clip", "Clip > New MIDI Clip"),
            Item::Cmd("Marker", "markers.add", "{}"),
        ],
    },
    Root {
        label: "PATTERNS",
        items: &[
            Item::Path("Insert pattern clip", "Clip > New MIDI Clip"),
            Item::Path("Quantize...", "Event > MIDI Operations > Quantize..."),
            Item::Path("Transpose...", "Event > MIDI Operations > Transpose..."),
            Item::Path("Change velocity...", "Event > MIDI Operations > Change Velocity..."),
            Item::Path("Remove duplicate notes", "Event > Remove Duplicate Notes"),
            Item::Sep,
            Item::Cmd("Channel rack", "ui.channel_rack", "{}"),
            Item::Cmd("Piano roll", "ui.piano_roll", "{}"),
        ],
    },
    Root {
        label: "VIEW",
        items: &[
            Item::Cmd("Channel rack", "ui.channel_rack", "{}"),
            Item::Cmd("Piano roll", "ui.piano_roll", "{}"),
            Item::Cmd("Mixer", "ui.mixer", "{}"),
            Item::Cmd("Browser", "ui.browser", "{}"),
            Item::Path("Event editor", "Window > MIDI Event List"),
            Item::Path("Project picker", "Window > Clip List"),
            Item::Sep,
            Item::Cmd("Zoom in", "view.zoom_in", "{}"),
            Item::Cmd("Zoom out", "view.zoom_out", "{}"),
            Item::Sep,
            Item::AllMenus,
        ],
    },
    Root {
        label: "OPTIONS",
        items: &[
            Item::Path("MIDI settings", "Setup > MIDI > MIDI Input Devices..."),
            Item::Path("Audio settings", "Setup > Playback Engine..."),
            Item::Path("General settings", "Setup > Preferences..."),
            Item::Path("Project info", "Setup > Session"),
            Item::Sep,
            Item::Path("Metronome", "Options > Click"),
            Item::Path("Countdown before recording", "Setup > Click/Countoff..."),
            Item::Path("Loop recording", "Options > Loop Record"),
            Item::Path("Song loop", "Options > Loop Playback"),
            Item::Sep,
            Item::Modes,
        ],
    },
    Root {
        label: "TOOLS",
        items: &[
            Item::Path("Smart tag scan...", "Track > Smart Tag Scan..."),
            Item::Path("Track tags...", "Track > Tags..."),
            Item::Path("Routing", "View > Routing Matrix"),
            Item::Path("Strip silence...", "Edit > Strip Silence"),
            Item::Path("Tempo...", "Event > Tempo Operations > Tempo Operations Window"),
            Item::Sep,
            Item::Path("Action list", "Window > Action List..."),
            Item::Path("Keyboard shortcuts", "Setup > Keyboard Shortcuts..."),
        ],
    },
    Root {
        label: "HELP",
        items: &[Item::About, Item::Cmd("Project info", "window.session_info", "{}"), Item::Cmd("Audio health", "window.audio_health", "{}")],
    },
];

const FL_TRACK_MENU: &[Ctx] = &[
    Ctx::Rename("Rename, color and icon..."),
    Ctx::Color("Color"),
    Ctx::Height("Track size"),
    Ctx::Sep,
    Ctx::Plain("Insert one", "track.new", r#"{"count":1}"#),
    Ctx::Delete("Delete one"),
    Ctx::Run("Clone", "track.duplicate"),
    Ctx::Sep,
    Ctx::InsertFx("Mixer insert effects"),
    Ctx::Routing("Route to mixer..."),
    Ctx::Run("Group with...", "track.group"),
    Ctx::Folders,
    Ctx::Tags,
    Ctx::Sep,
    Ctx::Run("Freeze", "track.freeze"),
    Ctx::Run("Render to audio clip", "track.commit"),
    Ctx::ScrollIntoView,
];

const FL_CLIP_MENU: &[Ctx] = &[
    Ctx::Rename("Rename..."),
    Ctx::Run("Make unique", "clip.capture"),
    Ctx::Run("Mute", "edit.mute_clips"),
    Ctx::Sep,
    Ctx::Plain("Chop", "edit.separate", "{}"),
    Ctx::Plain("Clone", "edit.duplicate", "{}"),
    Ctx::Plain("Merge", "edit.consolidate", "{}"),
    Ctx::Run("Quantize start times", "clip.quantize_to_grid"),
    Ctx::Sep,
    Ctx::Run("Lock", "clip.edit_lock"),
    Ctx::Delete("Delete"),
];

const FL_LANE_MENU: &[Ctx] = &[
    Ctx::Plain("Paste", "edit.paste", "{}"),
    Ctx::Path("Insert pattern clip", "Clip > New MIDI Clip"),
    Ctx::Path("Insert audio clip...", "File > Import > Audio..."),
    Ctx::Plain("Add marker", "markers.add", "{}"),
];

pub const FL: Mode = Mode {
    preset: LayoutPreset::FlStyle,
    menus: Some(FL_MENUS),
    clip_word: "Clip",
    click: Click::Paint,
    header: Header::Fl,
    header_w: 128.0,
    header_right: false,
    rulers: Some(&["markers", "bars_beats"]),
    track_menu: FL_TRACK_MENU,
    clip_menu: FL_CLIP_MENU,
    lane_menu: FL_LANE_MENU,
    mix: MixStyle {
        views: Some(&["inserts_ae", "inserts_fj"]),
        master_first: true,
        words: &[("INSERTS A-E", "SLOTS 1-5"), ("INSERTS F-J", "SLOTS 6-10")],
    },
    skin: Skin::Fl,
    settings: "OPTIONS › General settings (F10)",
};

// ---------------------------------------------------------------------------------------------
// Ableton-style.

const ABLETON_MENUS: &[Root] = &[
    Root {
        label: "File",
        items: &[
            Item::Path("New Live Set", "File > New..."),
            Item::Path("Open Live Set...", "File > Open > Session..."),
            Item::Recent,
            Item::Sep,
            Item::Path("Save Live Set", "File > Save Session"),
            Item::Path("Save Live Set As...", "File > Save Session As..."),
            Item::Path("Save a Copy...", "File > Save Session Copy In..."),
            Item::Path("Save Live Set As Template...", "File > Save As Template..."),
            Item::Sep,
            Item::Path("Import Audio File...", "File > Import > Audio..."),
            Item::Path("Import MIDI File...", "File > Import > MIDI..."),
            Item::Sep,
            Item::Path("Export Audio/Video...", "File > Bounce Mix..."),
            Item::Path("Export MIDI Clip...", "File > Export > MIDI..."),
            Item::Sep,
            Item::Path("Close Live Set", "File > Close Session"),
            Item::Quit,
        ],
    },
    Root {
        label: "Edit",
        items: &[
            Item::Path("Undo", "Edit > Can't Undo"),
            Item::Path("Redo", "Edit > Can't Redo"),
            Item::Sep,
            Item::Path("Cut", "Edit > Cut"),
            Item::Path("Copy", "Edit > Copy"),
            Item::Path("Paste", "Edit > Paste"),
            Item::Path("Duplicate", "Edit > Duplicate"),
            Item::Path("Delete", "Edit > Clear"),
            Item::Sep,
            Item::Path("Split", "Edit > Separate > At Selection"),
            Item::Path("Consolidate", "Edit > Consolidate"),
            Item::Path("Select All", "Edit > Select All"),
            Item::Path("Rename", "Clip > Rename..."),
            Item::Sep,
            Item::Path("Insert Silence", "Edit > Insert Silence"),
            Item::Path("Quantize Settings...", "Event > MIDI Operations > Quantize..."),
            Item::Path("Record Quantization...", "Event > MIDI Operations > Input Quantize..."),
            Item::Path("Warp From Here (Straight)", "Clip > Conform to Tempo"),
        ],
    },
    Root {
        label: "Create",
        items: &[
            Item::Cmd("Insert Audio Track", "track.new", r#"{"kind":"audio"}"#),
            Item::Cmd("Insert MIDI Track", "track.new", r#"{"kind":"instrument"}"#),
            Item::Cmd("Insert Return Track", "track.new", r#"{"kind":"aux"}"#),
            Item::Sep,
            Item::Cmd("Insert Scene", "scene.new", "{}"),
            Item::Path("Insert Empty MIDI Clip", "Clip > New MIDI Clip"),
            Item::Sep,
            Item::Cmd("Add Locator", "markers.add", "{}"),
            Item::Path("Insert Time...", "Event > Time Operations > Insert Time..."),
            Item::Path("Delete Time...", "Event > Time Operations > Cut Time..."),
        ],
    },
    Root {
        label: "View",
        items: &[
            Item::Cmd("Toggle Arrangement/Session View", "ui.session_view", "{}"),
            Item::Cmd("Browser", "ui.browser", "{}"),
            Item::Cmd("Detail View", "ui.device_view", "{}"),
            Item::Cmd("Mixer", "ui.mixer", "{}"),
            Item::Path("Groove Pool", "Window > Clip Effects"),
            Item::Sep,
            Item::Cmd("Zoom In", "view.zoom_in", "{}"),
            Item::Cmd("Zoom Out", "view.zoom_out", "{}"),
            Item::Cmd("Zoom to Arrangement Time Selection", "view.zoom_to_selection", "{}"),
            Item::Sep,
            Item::AllMenus,
        ],
    },
    Root {
        label: "Options",
        items: &[
            Item::Path("Metronome", "Options > Click"),
            Item::Path("Count-In...", "Setup > Click/Countoff..."),
            Item::Path("Loop", "Options > Loop Playback"),
            Item::Path("Follow", "Options > Insertion Follows Playback"),
            Item::Path("Delay Compensation", "Options > Delay Compensation"),
            Item::Sep,
            Item::Path("Edit Key Map...", "Setup > Keyboard Shortcuts..."),
            Item::Path("Track Tags...", "Track > Tags..."),
            Item::Path("Smart Tag Scan...", "Track > Smart Tag Scan..."),
            Item::Sep,
            Item::Path("Preferences...", "Setup > Preferences..."),
            Item::Sep,
            Item::Modes,
        ],
    },
    Root {
        label: "Help",
        items: &[Item::About, Item::Cmd("Set Info", "window.session_info", "{}"), Item::Cmd("Audio Health", "window.audio_health", "{}")],
    },
];

const ABLETON_TRACK_MENU: &[Ctx] = &[
    Ctx::Plain("Insert Audio Track", "track.new", r#"{"kind":"audio"}"#),
    Ctx::Plain("Insert MIDI Track", "track.new", r#"{"kind":"instrument"}"#),
    Ctx::Plain("Insert Return Track", "track.new", r#"{"kind":"aux"}"#),
    Ctx::Sep,
    Ctx::Run("Group Tracks", "track.move_to_new_folder"),
    Ctx::Folders,
    Ctx::Sep,
    Ctx::Rename("Rename"),
    Ctx::Run("Duplicate", "track.duplicate"),
    Ctx::Delete("Delete"),
    Ctx::Sep,
    Ctx::Run("Freeze Track", "track.freeze"),
    Ctx::Run("Flatten", "track.commit"),
    Ctx::InsertFx("Add Audio Effect"),
    Ctx::Routing("Input/Output..."),
    Ctx::Tags,
    Ctx::Sep,
    Ctx::Color("Color"),
    Ctx::Height("Track Height"),
];

const ABLETON_CLIP_MENU: &[Ctx] = &[
    Ctx::Plain("Cut", "edit.cut", "{}"),
    Ctx::Plain("Copy", "edit.copy", "{}"),
    Ctx::Plain("Duplicate", "edit.duplicate", "{}"),
    Ctx::Delete("Delete"),
    Ctx::Sep,
    Ctx::Plain("Split", "edit.separate", "{}"),
    Ctx::Plain("Consolidate", "edit.consolidate", "{}"),
    Ctx::Run("Quantize", "clip.quantize_to_grid"),
    Ctx::Run("Deactivate Clip", "edit.mute_clips"),
    Ctx::Sep,
    Ctx::Rename("Rename"),
    Ctx::Run("Warp From Here", "clip.remove_warp"),
];

const ABLETON_LANE_MENU: &[Ctx] = &[
    Ctx::Plain("Paste", "edit.paste", "{}"),
    Ctx::Path("Insert Empty MIDI Clip", "Clip > New MIDI Clip"),
    Ctx::Plain("Add Locator", "markers.add", "{}"),
    Ctx::Path("Insert Time...", "Event > Time Operations > Insert Time..."),
    Ctx::Plain("Select All", "edit.select_all", "{}"),
];

pub const ABLETON: Mode = Mode {
    preset: LayoutPreset::AbletonStyle,
    menus: Some(ABLETON_MENUS),
    clip_word: "Clip",
    click: Click::TitleBar,
    header: Header::Ableton,
    header_w: 196.0,
    header_right: true,
    rulers: Some(&["bars_beats", "markers"]),
    track_menu: ABLETON_TRACK_MENU,
    clip_menu: ABLETON_CLIP_MENU,
    lane_menu: ABLETON_LANE_MENU,
    mix: MixStyle { views: Some(&["sends_ae", "io"]), master_first: false, words: &[("SENDS A-E", "SENDS"), ("I / O", "IN/OUT")] },
    skin: Skin::Ableton,
    settings: "Options › Preferences… (Ctrl+,)",
};

// ---------------------------------------------------------------------------------------------
// NEW: Solara's own.

const NEW_MENUS: &[Root] = &[
    Root {
        label: "Solara",
        items: &[
            Item::About,
            Item::Path("Settings...", "Setup > Preferences..."),
            Item::Cmd("Session Info", "window.session_info", "{}"),
            Item::Cmd("Session Audio Health", "window.audio_health", "{}"),
            Item::Sep,
            Item::Modes,
            Item::Sep,
            Item::Quit,
        ],
    },
    Root {
        label: "File",
        items: &[
            Item::Path("New Song", "File > New..."),
            Item::Path("Open...", "File > Open > Session..."),
            Item::Recent,
            Item::Path("Save", "File > Save Session"),
            Item::Path("Save As...", "File > Save Session As..."),
            Item::Sep,
            Item::Path("Import Audio...", "File > Import > Audio..."),
            Item::Path("Import MIDI...", "File > Import > MIDI..."),
            Item::Path("Export Mix...", "File > Bounce Mix..."),
            Item::Path("Export MIDI...", "File > Export > MIDI..."),
            Item::Sep,
            Item::Path("Close", "File > Close Session"),
        ],
    },
    Root {
        label: "Edit",
        items: &[
            Item::Path("Undo", "Edit > Can't Undo"),
            Item::Path("Redo", "Edit > Can't Redo"),
            Item::Sep,
            Item::Path("Cut", "Edit > Cut"),
            Item::Path("Copy", "Edit > Copy"),
            Item::Path("Paste", "Edit > Paste"),
            Item::Path("Delete", "Edit > Clear"),
            Item::Path("Duplicate", "Edit > Duplicate"),
            Item::Path("Split", "Edit > Separate > At Selection"),
            Item::Path("Join", "Edit > Consolidate"),
            Item::Sep,
            Item::Path("Select All", "Edit > Select All"),
        ],
    },
    Root {
        label: "Create",
        items: &[
            Item::Cmd("Bass", "track.new", r#"{"kind":"instrument","name":"Bass","instrument":"subtractive_synth"}"#),
            Item::Cmd("Chords", "track.new", r#"{"kind":"instrument","name":"Chords","instrument":"subtractive_synth"}"#),
            Item::Cmd("Lead", "track.new", r#"{"kind":"instrument","name":"Lead","instrument":"subtractive_synth"}"#),
            Item::Cmd("Audio Track", "track.new", r#"{"kind":"audio"}"#),
            Item::Path("Tracks...", "Track > New..."),
            Item::Sep,
            Item::Path("Empty MIDI Clip", "Clip > New MIDI Clip"),
            Item::Cmd("Marker", "markers.add", "{}"),
        ],
    },
    Root {
        label: "View",
        items: &[
            Item::Cmd("Mixer", "ui.mixer", "{}"),
            Item::Cmd("Step Sequencer", "ui.editor", r#"{"view":"steps"}"#),
            Item::Cmd("Piano Roll", "ui.editor", r#"{"view":"piano"}"#),
            Item::Sep,
            Item::Cmd("Zoom In", "view.zoom_in", "{}"),
            Item::Cmd("Zoom Out", "view.zoom_out", "{}"),
            Item::Sep,
            Item::AllMenus,
        ],
    },
    Root {
        label: "Help",
        items: &[Item::Path("Keyboard Shortcuts...", "Setup > Keyboard Shortcuts..."), Item::Path("Action List...", "Window > Action List...")],
    },
];

const NEW_TRACK_MENU: &[Ctx] = &[
    Ctx::Rename("Rename..."),
    Ctx::Color("Colour"),
    Ctx::Height("Height"),
    Ctx::Sep,
    Ctx::Run("Duplicate", "track.duplicate"),
    Ctx::Delete("Delete"),
    Ctx::Sep,
    Ctx::InsertFx("Add Effect"),
    Ctx::Run("Freeze", "track.freeze"),
    Ctx::Folders,
    Ctx::Tags,
];

const NEW_CLIP_MENU: &[Ctx] = &[
    Ctx::Rename("Rename..."),
    Ctx::Plain("Split", "edit.separate", "{}"),
    Ctx::Plain("Duplicate", "edit.duplicate", "{}"),
    Ctx::Plain("Join", "edit.consolidate", "{}"),
    Ctx::Run("Quantize", "clip.quantize_to_grid"),
    Ctx::Run("Mute", "edit.mute_clips"),
    Ctx::Sep,
    Ctx::Delete("Delete"),
];

const NEW_LANE_MENU: &[Ctx] =
    &[Ctx::Plain("Paste", "edit.paste", "{}"), Ctx::Path("Empty MIDI Clip", "Clip > New MIDI Clip"), Ctx::Plain("Marker", "markers.add", "{}")];

pub const NEW: Mode = Mode {
    preset: LayoutPreset::New,
    menus: Some(NEW_MENUS),
    clip_word: "Clip",
    click: Click::Items,
    header: Header::New,
    header_w: 176.0,
    header_right: false,
    rulers: Some(&["markers", "bars_beats"]),
    track_menu: NEW_TRACK_MENU,
    clip_menu: NEW_CLIP_MENU,
    lane_menu: NEW_LANE_MENU,
    mix: MixStyle {
        views: Some(&["instruments", "inserts_ae", "sends_ae"]),
        master_first: false,
        words: &[("INSERTS A-E", "EFFECTS"), ("SENDS A-E", "SENDS")],
    },
    skin: Skin::New,
    settings: "Solara › Settings…",
};

// ---------------------------------------------------------------------------------------------
// The menu bar.

/// The active mode's menus, drawn in the bar.
pub fn menu_roots(app: &mut SoundApp, ui: &mut Ui, roots: &[Root]) {
    let enabled = !app.ui.show_preferences;
    for root in roots {
        ui.menu_button(root.label, |ui| {
            ui.add_enabled_ui(enabled, |ui| {
                for item in root.items {
                    menu_item(app, ui, item);
                }
            });
        });
    }
}

fn menu_item(app: &mut SoundApp, ui: &mut Ui, item: &Item) {
    match *item {
        Item::Path(label, path) => path_item(app, ui, label, path),
        Item::Cmd(label, id, params) => cmd_item(app, ui, label, id, params),
        Item::Sub(label, items) => {
            ui.menu_button(label, |ui| {
                for i in items {
                    menu_item(app, ui, i);
                }
            });
        }
        Item::Tree(label, path) => {
            let tree = crate::menus::tree();
            if let Some(node) = find_node(&tree, path) {
                let extra = crate::menus::ui_aliases();
                ui.menu_button(label, |ui| {
                    for c in &node.children {
                        crate::menus::menu_node(app, ui, c, &extra);
                    }
                });
            }
        }
        Item::Sep => {
            ui.separator();
        }
        Item::Recent => crate::recent::menu(app, ui),
        Item::About => {
            if ui.button("About Solara").clicked() {
                app.ui.show_about = true;
                ui.close();
            }
            if crate::updates_ui::UPDATE_CHECKS_ENABLED && ui.button("Check for Updates…").clicked() {
                let _ = app.run("app.checkForUpdates", json!({}));
                ui.close();
            }
            if ui.button("SolPlugs Packages…").clicked() {
                let _ = crate::menus::run_ui_command(app, "window.packages", &json!({"value": true}));
                ui.close();
            }
        }
        Item::Quit => {
            if ui.button(if cfg!(target_os = "macos") { "Quit" } else { "Exit" }).clicked() {
                let _ = crate::dialogs::run_or_ask(app, "app.quit", json!({}));
                ui.close();
            }
        }
        Item::Modes => {
            ui.menu_button("Mode", |ui| {
                for preset in LayoutPreset::ALL {
                    if ui.radio(app.ui.layout == preset, preset.label()).clicked() {
                        let _ = app.run("ui.layout", json!({"preset": preset.id()}));
                        ui.close();
                    }
                }
            });
        }
        Item::AllMenus => {
            let tree = crate::menus::tree();
            let extra = crate::menus::ui_aliases();
            ui.menu_button("All Solara Menus", |ui| {
                for root in &tree {
                    crate::menus::menu_node(app, ui, root, &extra);
                }
            });
        }
    }
}

/// The node at `path` in the catalog menu tree.
pub fn find_node<'a>(nodes: &'a [crate::menus::MenuNode], path: &str) -> Option<&'a crate::menus::MenuNode> {
    nodes.iter().find_map(|n| {
        if n.path == path {
            Some(n)
        } else if path.starts_with(&format!("{} > ", n.path)) {
            find_node(&n.children, path)
        } else {
            None
        }
    })
}

/// The chord a command has in the active mode, as people read it.
fn chord(app: &SoundApp, ui: &Ui, id: &str) -> String {
    let mac = ui.ctx().os().is_mac();
    crate::keymap::chord_for(app, id).map_or_else(
        || crate::menus::shortcut_for(id).map(|s| crate::shortcuts::shortcut_label(s, mac)).unwrap_or_default(),
        |c| crate::keymap::display(&c, mac),
    )
}

fn path_item(app: &mut SoundApp, ui: &mut Ui, label: &str, path: &str) {
    let extra = crate::menus::ui_aliases();
    let id = crate::menus::command_for_path(path, &extra);
    let (enabled, checked) = crate::menus::item_state(app, path, id.as_deref());
    // Undo and Redo name their step, in the mode's own words.
    let label = match (id.as_deref(), crate::menus::live_label(app, id.as_deref())) {
        (Some("edit.undo" | "edit.redo"), Some(live)) if live.starts_with("Can't") => label.to_string(),
        (Some("edit.undo" | "edit.redo"), Some(live)) => live.split_once(' ').map_or(live.clone(), |(_, step)| format!("{label} {step}")),
        _ => label.to_string(),
    };
    let label = if checked { format!("✔ {label}") } else { label };
    let keys = id.as_deref().map(|i| chord(app, ui, i)).unwrap_or_default();
    let resp = ui.add_enabled(enabled, egui::Button::new(label).shortcut_text(keys));
    if resp.clicked()
        && let Some(id) = id
    {
        crate::menus::invoke_menu(app, &id, path);
        ui.close();
    }
}

fn cmd_item(app: &mut SoundApp, ui: &mut Ui, label: &str, id: &str, params: &str) {
    let enabled = !id.contains('.') || id.starts_with("ui.") || id.starts_with("window.") || command_enabled(app, id);
    let keys = chord(app, ui, id);
    if ui.add_enabled(enabled, egui::Button::new(label).shortcut_text(keys)).clicked() {
        run_cmd(app, id, params);
        ui.close();
    }
}

fn command_enabled(app: &SoundApp, id: &str) -> bool {
    soundcraft_engine::find_command(id).is_none_or(|c| (c.enabled)(&app.engine).is_ok() && !app.engine.blocked_while_recording(id))
}

/// Run a menu or context command: `{}` menu-style (its dialog), otherwise with the parameters given.
fn run_cmd(app: &mut SoundApp, id: &str, params: &str) {
    let p: Value = serde_json::from_str(params).unwrap_or_else(|_| json!({}));
    if p.as_object().is_some_and(serde_json::Map::is_empty) {
        crate::menus::invoke_menu(app, id, "");
    } else if let Err(e) = app.run(id, p) {
        app.ui.status = e;
    }
}

// ---------------------------------------------------------------------------------------------
// Right-click menus.

/// What a right-click menu acts on.
#[derive(Debug, Clone)]
pub enum Target {
    /// These tracks (the clicked one, or the selection it belongs to).
    Tracks(soundcraft_model::TrackId, Vec<u64>),
    /// A clip and how its commands aim at it: `{}` for the edit selection, `{"clips": [id]}` alone.
    Clip(soundcraft_model::ClipId, Value),
    /// An empty part of a lane.
    Lane,
}

/// Draw a right-click menu of the active mode.
pub fn context_menu(app: &mut SoundApp, ui: &mut Ui, entries: &[Ctx], target: &Target) {
    for e in entries {
        context_entry(app, ui, e, target);
    }
}

fn target_params(target: &Target) -> Value {
    match target {
        Target::Tracks(_, sel) => json!({"tracks": sel}),
        Target::Clip(_, p) => p.clone(),
        Target::Lane => json!({}),
    }
}

fn context_entry(app: &mut SoundApp, ui: &mut Ui, e: &Ctx, target: &Target) {
    use soundcraft_model::TrackId;
    match *e {
        Ctx::Run(label, id) => {
            if ui.add_enabled(command_enabled(app, id), egui::Button::new(label)).clicked() {
                if let Err(err) = app.run(id, target_params(target)) {
                    app.ui.status = err;
                }
                ui.close();
            }
        }
        Ctx::Plain(label, id, params) => {
            if ui.add_enabled(id.starts_with("ui.") || command_enabled(app, id), egui::Button::new(label)).clicked() {
                let p: Value = serde_json::from_str(params).unwrap_or_else(|_| json!({}));
                if let Err(err) = app.run(id, p) {
                    app.ui.status = err;
                }
                ui.close();
            }
        }
        Ctx::Path(label, path) => path_item(app, ui, label, path),
        Ctx::Sub(label, entries) => {
            ui.menu_button(label, |ui| {
                for e in entries {
                    context_entry(app, ui, e, target);
                }
            });
        }
        Ctx::Sep => {
            ui.separator();
        }
        Ctx::Rename(label) => {
            if ui.button(label).clicked() {
                match target {
                    Target::Tracks(id, _) => {
                        let name = app.engine.session().track(*id).map(|t| t.name.clone()).unwrap_or_default();
                        app.dialogs.open_rename_track(*id, &name);
                    }
                    Target::Clip(c, _) => {
                        let name = app.engine.session().find_clip(*c).map(|(_, x)| x.name.clone()).unwrap_or_default();
                        app.dialogs.open_rename_clip(*c, &name);
                    }
                    Target::Lane => {}
                }
                ui.close();
            }
        }
        Ctx::Delete(label) => {
            if ui.button(label).clicked() {
                match target {
                    Target::Tracks(_, sel) => {
                        if let Err(err) = app.run("track.delete", json!({"tracks": sel})) {
                            app.ui.status = err;
                        }
                    }
                    Target::Clip(c, _) => delete_clip(app, *c),
                    Target::Lane => {}
                }
                ui.close();
            }
        }
        Ctx::Color(label) => {
            ui.menu_button(label, |ui| {
                for (i, c) in soundcraft_model::TRACK_COLORS.iter().enumerate() {
                    let (r, resp) = ui.allocate_exact_size(vec2(120.0, 14.0), Sense::click());
                    ui.painter().rect_filled(r, 2.0, crate::theme::rgb(*c));
                    if resp.clicked() {
                        let res = match target {
                            Target::Tracks(_, sel) => app.run("track.color", json!({"tracks": sel, "color": i})),
                            Target::Clip(c, _) => app.run("clip.color", json!({"clips": [c.0], "color": i})),
                            Target::Lane => Ok(Value::Null),
                        };
                        if let Err(err) = res {
                            app.ui.status = err;
                        }
                        ui.close();
                    }
                }
            });
        }
        Ctx::Height(label) => {
            if let Target::Tracks(_, sel) = target {
                ui.menu_button(label, |ui| {
                    for h in soundcraft_model::TrackHeight::ALL {
                        if ui.button(h.label()).clicked() {
                            let own: Vec<TrackId> = sel.iter().map(|&t| TrackId(t)).collect();
                            let tracks = crate::edit_window::height_targets(app.engine.session(), ui.input(|i| i.modifiers), &own);
                            let _ = app.run("track.height", json!({"tracks": tracks, "height": h.label()}));
                            ui.close();
                        }
                    }
                });
            }
        }
        Ctx::Folders => {
            if let Target::Tracks(id, sel) = target {
                crate::edit_window::folder_entries(app, ui, *id, sel);
            }
        }
        Ctx::Tags => {
            if let Target::Tracks(id, sel) = target {
                crate::edit_window::tag_entries(app, ui, *id, sel);
            }
        }
        Ctx::InsertFx(label) => {
            if let Target::Tracks(id, _) = target {
                let free = app.engine.session().track(*id).and_then(|t| t.mixer.inserts.iter().position(Option::is_none));
                match free {
                    Some(slot) => {
                        ui.menu_button(label, |ui| crate::mix_window::plugin_menu(app, ui, *id, slot, false));
                    }
                    None => {
                        ui.add_enabled(false, egui::Button::new(label)).on_disabled_hover_text("Every insert slot is in use");
                    }
                }
            }
        }
        Ctx::Routing(label) => {
            if ui.button(label).clicked() {
                let _ = app.run("window.routing_matrix", json!({"value": true}));
                ui.close();
            }
        }
        Ctx::ScrollIntoView => {
            if let Target::Tracks(id, _) = target
                && ui.button("Scroll Into View").clicked()
            {
                let _ = app.run("view.scroll_into_view", json!({"track": id.0}));
                ui.close();
            }
        }
    }
}

/// Delete one clip, or the edit selection it belongs to.
pub fn delete_clip(app: &mut SoundApp, c: soundcraft_model::ClipId) {
    let s = app.engine.session();
    let in_selection = crate::edit_window::clip_in_selection(s, c);
    let selected: Vec<u64> = s.edit.selected_clips.iter().map(|id| id.0).collect();
    let track: Vec<u64> = s.find_clip(c).map(|(t, _)| t.0).into_iter().collect();
    let res = if !in_selection {
        app.run("edit.select", json!({"clips": [c.0]})).and_then(|_| app.run("edit.clear", json!({})))
    } else if selected.is_empty() {
        app.run("edit.clear", json!({}))
    } else {
        // Selected clips go by id (an empty range), so clips between them that the span covers stay. The clicked
        // clip's track satisfies Clear's track check even when the track selection was emptied meanwhile.
        app.run("edit.clear", json!({"clips": selected, "tracks": track, "start": 0}))
    };
    if let Err(err) = res {
        app.ui.status = err;
    }
}

// ---------------------------------------------------------------------------------------------
// The mode picker, top right, with each mode's parody logo.

/// The top-right mode button: the current mode's logo; a click opens the picker with all six.
pub fn toggle(app: &mut SoundApp, ui: &mut Ui) {
    let size = vec2(30.0, 20.0);
    let (r, resp) = ui.allocate_exact_size(size, Sense::click());
    let current = app.ui.layout;
    let resp = resp.on_hover_text(format!("{} mode: click to change mode", current.label()));
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::ComboBox, true, format!("Mode: {}", current.label())));
    if resp.hovered() {
        ui.painter().rect_filled(r, 4.0, Tokens::current().button_hi);
    }
    logo(ui.painter(), Rect::from_center_size(r.center(), vec2(18.0, 18.0)), current);
    egui::Popup::menu(&resp).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| picker(app, ui));
}

/// All six modes as logo tiles. A pick changes the whole mode at once; the session stays as it is.
pub fn picker(app: &mut SoundApp, ui: &mut Ui) {
    let t = Tokens::current();
    ui.label(egui::RichText::new("MODE").font(bold(10.5)).color(t.text_dim));
    // A fixed-size block of tiles, so the popup has its final size (and place) from its first frame.
    let (tile, gap) = (vec2(96.0, 74.0), 6.0);
    let (area, _) = ui.allocate_exact_size(vec2(3.0 * tile.x + 2.0 * gap, 2.0 * tile.y + gap), Sense::hover());
    for (i, preset) in LayoutPreset::ALL.into_iter().enumerate() {
        let on = app.ui.layout == preset;
        let (col, row) = ((i % 3) as f32, (i / 3) as f32);
        let r = Rect::from_min_size(area.min + vec2(col * (tile.x + gap), row * (tile.y + gap)), tile);
        let resp = ui.interact(r, ui.scope_id().with(("mode_tile", preset.id())), Sense::click());
        resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::RadioButton, true, on, preset.label()));
        let bg = if on {
            t.accent_dark
        } else if resp.hovered() {
            t.button_hi
        } else {
            t.button
        };
        ui.painter().rect_filled(r, 6.0, bg);
        logo(ui.painter(), Rect::from_center_size(pos2(r.center().x, r.min.y + 26.0), vec2(36.0, 36.0)), preset);
        ui.painter().text(pos2(r.center().x, r.max.y - 12.0), Align2::CENTER_CENTER, preset.label(), regular(11.5), t.text);
        let resp = resp.on_hover_text(format!("{} ({})", preset.label(), of(preset).settings));
        if resp.clicked() {
            if let Err(e) = app.run("ui.layout", json!({"preset": preset.id()})) {
                app.ui.status = e;
            }
            if !app.ask_mode {
                ui.close();
            }
        }
    }
}

/// On a fresh install, ask which mode to work in. Pro Tools-style is picked until another tile is; Start
/// (or Escape) keeps the pick, which is saved with the settings, so the question comes only once.
pub fn first_run(app: &mut SoundApp, ctx: &egui::Context) {
    if !app.ask_mode {
        return;
    }
    let t = Tokens::current();
    let resp = egui::Modal::new(egui::Id::unique("solara_first_mode")).show(ctx, |ui| {
        ui.label(egui::RichText::new("Welcome to Solara").font(bold(18.0)).color(t.text));
        ui.label(
            egui::RichText::new("How do you like to work? Pick the DAW you know best. You can change it any time with the logo at the top right.")
                .font(regular(12.0))
                .color(t.text_dim),
        );
        ui.add_space(6.0);
        picker(app, ui);
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(format!("{} mode", app.ui.layout.label())).font(regular(12.0)).color(t.text));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| ui.button(egui::RichText::new("Start").font(bold(13.0))).clicked())
                .inner
        })
        .inner
    });
    if resp.inner || resp.should_close() {
        app.ask_mode = false;
    }
}

/// Draw `preset`'s parody logo in `r`: original art, drawn in code, never a real DAW's mark.
pub fn logo(p: &egui::Painter, r: Rect, preset: LayoutPreset) {
    let s = r.width().min(r.height());
    let c = r.center();
    let at = |x: f32, y: f32| pos2(c.x + x * s, c.y + y * s);
    let tile = |fill: Color32| p.rect_filled(Rect::from_center_size(c, vec2(s, s)), s * 0.22, fill);
    match preset {
        // A rising sun: Solara's own.
        LayoutPreset::New => {
            tile(Color32::from_rgb(28, 30, 58));
            for k in 0..8 {
                let a = k as f32 * std::f32::consts::TAU / 8.0;
                let (sn, cs) = a.sin_cos();
                p.line_segment([at(cs * 0.24, sn * 0.24), at(cs * 0.38, sn * 0.38)], Stroke::new(s * 0.06, Color32::from_rgb(255, 196, 80)));
            }
            p.circle_filled(c, s * 0.17, Color32::from_rgb(255, 170, 60));
        }
        // Two cherries on one stem.
        LayoutPreset::FlStyle => {
            tile(Color32::from_rgb(44, 52, 58));
            let stem = Stroke::new(s * 0.05, Color32::from_rgb(110, 190, 80));
            p.add(Shape::line(vec![at(-0.16, 0.12), at(-0.04, -0.12), at(0.06, -0.3)], stem));
            p.add(Shape::line(vec![at(0.16, 0.12), at(0.12, -0.1), at(0.06, -0.3)], stem));
            p.add(Shape::convex_polygon(vec![at(0.06, -0.3), at(0.3, -0.36), at(0.2, -0.22)], Color32::from_rgb(120, 200, 90), Stroke::NONE));
            for x in [-0.16, 0.16] {
                p.circle_filled(at(x, 0.2), s * 0.15, Color32::from_rgb(214, 40, 58));
                p.circle_filled(at(x - 0.05, 0.15), s * 0.04, Color32::from_rgb(255, 150, 160));
            }
        }
        // A cassette with its two reels.
        LayoutPreset::Classic => {
            tile(Color32::from_rgb(54, 54, 58));
            let body = Rect::from_center_size(c, vec2(s * 0.76, s * 0.5));
            p.rect_filled(body, s * 0.06, Color32::from_rgb(200, 196, 186));
            p.rect_filled(Rect::from_center_size(at(0.0, -0.02), vec2(s * 0.56, s * 0.2)), s * 0.03, Color32::from_rgb(60, 60, 64));
            for x in [-0.15, 0.15] {
                p.circle_filled(at(x, -0.02), s * 0.07, Color32::from_rgb(230, 230, 226));
                p.circle_filled(at(x, -0.02), s * 0.025, Color32::from_rgb(60, 60, 64));
            }
        }
        // A scythe: a long handle and a curved blade.
        LayoutPreset::Reaper => {
            tile(Color32::from_rgb(40, 44, 48));
            p.line_segment([at(-0.26, 0.36), at(0.18, -0.34)], Stroke::new(s * 0.07, Color32::from_rgb(176, 128, 82)));
            let blade: Vec<Pos2> = (0..=10)
                .map(|k| {
                    let u = k as f32 / 10.0;
                    at(0.18 - u * 0.5, -0.34 + (u * std::f32::consts::PI).sin() * -0.06 + u * 0.12)
                })
                .collect();
            p.add(Shape::line(blade, Stroke::new(s * 0.08, Color32::from_rgb(214, 222, 230))));
        }
        // A toolbox with a waveform on its side.
        LayoutPreset::ProToolsStyle => {
            tile(Color32::from_rgb(30, 40, 58));
            let body = Rect::from_center_size(at(0.0, 0.08), vec2(s * 0.72, s * 0.44));
            p.rect_filled(body, s * 0.05, Color32::from_rgb(70, 130, 200));
            p.rect_stroke(
                Rect::from_center_size(at(0.0, -0.2), vec2(s * 0.3, s * 0.16)),
                s * 0.05,
                Stroke::new(s * 0.05, Color32::from_rgb(170, 190, 210)),
                egui::StrokeKind::Middle,
            );
            let wave: Vec<Pos2> =
                (0..=12).map(|k| at(-0.28 + k as f32 * 0.0467, 0.08 + if k % 2 == 0 { -0.1 } else { 0.1 } * ((k % 4) as f32 * 0.5 + 0.4))).collect();
            p.add(Shape::line(wave, Stroke::new(s * 0.04, Color32::WHITE)));
        }
        // A grid of clip slots, one of them playing.
        LayoutPreset::AbletonStyle => {
            tile(Color32::from_rgb(34, 34, 34));
            for row in 0..3 {
                for col in 0..3 {
                    let q = Rect::from_center_size(at(-0.24 + col as f32 * 0.24, -0.24 + row as f32 * 0.24), vec2(s * 0.19, s * 0.19));
                    let lit = (row, col) == (1, 1);
                    p.rect_filled(q, s * 0.03, if lit { Color32::from_rgb(255, 176, 40) } else { Color32::from_rgb(92, 92, 92) });
                    if lit {
                        let m = q.center();
                        let h = s * 0.055;
                        p.add(Shape::convex_polygon(
                            vec![pos2(m.x - h, m.y - h), pos2(m.x + h, m.y), pos2(m.x - h, m.y + h)],
                            Color32::BLACK,
                            Stroke::NONE,
                        ));
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Services;
    use soundcraft_engine::Engine;

    fn every_mode() -> impl Iterator<Item = &'static Mode> {
        LayoutPreset::ALL.into_iter().map(of)
    }

    fn items<'a>(list: &'a [Item], out: &mut Vec<&'a Item>) {
        for i in list {
            out.push(i);
            if let Item::Sub(_, sub) = i {
                items(sub, out);
            }
        }
    }

    fn ctxs<'a>(list: &'a [Ctx], out: &mut Vec<&'a Ctx>) {
        for c in list {
            out.push(c);
            if let Ctx::Sub(_, sub) = c {
                ctxs(sub, out);
            }
        }
    }

    fn known(id: &str) -> bool {
        soundcraft_engine::find_command(id).is_some()
            || crate::menus::UI_COMMANDS.iter().any(|c| c.0 == id)
            || matches!(
                id,
                "ui.mixer" | "ui.layout" | "ui.editor" | "ui.channel_rack" | "ui.piano_roll" | "ui.browser" | "ui.session_view" | "ui.device_view"
            )
    }

    #[test]
    fn every_menu_entry_is_an_existing_function() {
        let tree = crate::menus::tree();
        let extra = crate::menus::ui_aliases();
        for mode in every_mode() {
            let mut all = Vec::new();
            for root in mode.menus.unwrap_or_default() {
                items(root.items, &mut all);
            }
            for item in all {
                match *item {
                    Item::Path(label, path) => {
                        let node = find_node(&tree, path).unwrap_or_else(|| panic!("{:?} {label}: no menu {path}", mode.preset));
                        assert!(node.children.is_empty(), "{path} is a submenu");
                        assert!(crate::menus::command_for_path(path, &extra).is_some(), "{:?} {label}: {path} runs nothing", mode.preset);
                    }
                    Item::Tree(label, path) => {
                        assert!(find_node(&tree, path).is_some_and(|n| !n.children.is_empty()), "{:?} {label}: no submenu {path}", mode.preset)
                    }
                    Item::Cmd(label, id, params) => {
                        assert!(known(id), "{:?} {label}: unknown command {id}", mode.preset);
                        assert!(serde_json::from_str::<Value>(params).is_ok(), "{params}");
                    }
                    _ => {}
                }
            }
        }
    }

    #[test]
    fn every_context_entry_is_an_existing_function() {
        let tree = crate::menus::tree();
        for mode in every_mode() {
            let mut all = Vec::new();
            ctxs(mode.track_menu, &mut all);
            ctxs(mode.clip_menu, &mut all);
            ctxs(mode.lane_menu, &mut all);
            for c in all {
                match *c {
                    Ctx::Run(label, id) | Ctx::Plain(label, id, _) => assert!(known(id), "{:?} {label}: unknown command {id}", mode.preset),
                    Ctx::Path(label, path) => assert!(find_node(&tree, path).is_some(), "{:?} {label}: no menu {path}", mode.preset),
                    _ => {}
                }
            }
        }
    }

    #[test]
    fn modes_without_the_catalog_bar_keep_every_menu_and_the_settings_and_mode_entries() {
        for mode in every_mode() {
            let Some(roots) = mode.menus else { continue };
            let mut all = Vec::new();
            for root in roots {
                items(root.items, &mut all);
            }
            assert!(all.iter().any(|i| matches!(i, Item::AllMenus)), "{:?}", mode.preset);
            assert!(all.iter().any(|i| matches!(i, Item::Modes)), "{:?}", mode.preset);
            assert!(all.iter().any(|i| matches!(i, Item::Path(_, "Setup > Preferences..."))), "{:?} has no settings", mode.preset);
        }
    }

    #[test]
    fn each_mode_has_its_own_menu_bar_and_words() {
        let bar = |p: LayoutPreset| of(p).menus.map(|r| r.iter().map(|r| r.label).collect::<Vec<_>>());
        assert_eq!(bar(LayoutPreset::Reaper).unwrap_or_default(), ["File", "Edit", "View", "Insert", "Item", "Track", "Options", "Actions", "Help"]);
        assert_eq!(bar(LayoutPreset::FlStyle).unwrap_or_default(), ["FILE", "EDIT", "ADD", "PATTERNS", "VIEW", "OPTIONS", "TOOLS", "HELP"]);
        assert_eq!(bar(LayoutPreset::AbletonStyle).unwrap_or_default(), ["File", "Edit", "Create", "View", "Options", "Help"]);
        assert_eq!(bar(LayoutPreset::ProToolsStyle), None);
        assert_eq!(of(LayoutPreset::Reaper).clip_word, "Item");
        assert!(of(LayoutPreset::AbletonStyle).header_right);
    }

    #[test]
    fn the_menu_commands_with_parameters_run() {
        let _palette = crate::theme::lock_test_palette();
        for mode in every_mode() {
            let mut app = SoundApp::new(Engine::default(), None, Services::default());
            app.ui.layout = mode.preset;
            let mut all = Vec::new();
            for root in mode.menus.unwrap_or_default() {
                items(root.items, &mut all);
            }
            for item in all {
                if let Item::Cmd(label, id, params) = *item
                    && params != "{}"
                {
                    let p: Value = serde_json::from_str(params).unwrap_or_default();
                    assert!(app.run(id, p).is_ok(), "{:?} {label}", mode.preset);
                }
            }
        }
    }

    fn harness() -> egui_kittest::Harness<'static, SoundApp> {
        let app = SoundApp::new(soundcraft_engine::demo::demo_engine(), None, Services::default());
        let mut h = egui_kittest::Harness::builder().with_size(vec2(1600.0, 1000.0)).build_ui_state(
            |ui, app: &mut SoundApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            app,
        );
        h.run_steps(4);
        h
    }

    #[test]
    fn one_pick_in_the_top_right_picker_changes_the_whole_mode_and_keeps_the_session() {
        use egui_kittest::kittest::Queryable;
        let _palette = crate::theme::lock_test_palette();
        let mut h = harness();
        // A fresh start is Pro Tools-style: its catalog menus and its colours.
        assert_eq!(h.state().ui.layout, LayoutPreset::ProToolsStyle);
        assert!(h.query_by_label("AudioSuite").is_some());
        assert_eq!(crate::theme::skin(), Skin::ProTools);
        let before = h.state().engine.session().clone();
        for preset in [
            LayoutPreset::Reaper,
            LayoutPreset::FlStyle,
            LayoutPreset::AbletonStyle,
            LayoutPreset::Classic,
            LayoutPreset::New,
            LayoutPreset::ProToolsStyle,
        ] {
            h.query_by_label(&format!("Mode: {}", h.state().ui.layout.label())).expect("the mode toggle").click();
            h.run_steps(2);
            assert!(h.query_by_label("MODE").is_some(), "the picker opened");
            let tile = h.query_by_label(preset.label()).expect("a picker tile");
            let at = tile.rect().center();
            h.hover_at(at);
            h.step();
            h.drag_at(at);
            h.step();
            h.drop_at(at);
            h.run_steps(3);
            assert_eq!(h.state().ui.layout, preset);
            assert_eq!(crate::theme::skin(), of(preset).skin, "{preset:?} colours");
            if let Some(roots) = of(preset).menus {
                for root in roots {
                    assert!(h.query_all_by_label(root.label).next().is_some(), "{preset:?} menu {}", root.label);
                }
            }
            assert_eq!(*h.state().engine.session(), before, "{preset:?} left the session as it was");
        }
    }

    #[test]
    fn a_fresh_install_asks_for_a_mode_once_with_pro_tools_style_picked() {
        use egui_kittest::kittest::Queryable;
        let _palette = crate::theme::lock_test_palette();
        let mut h = harness();
        assert!(h.query_by_label("Welcome to Solara").is_none(), "only a fresh install asks");
        h.state_mut().ask_mode = true;
        h.run_steps(3);
        assert!(h.query_by_label("Welcome to Solara").is_some());
        assert_eq!(h.state().ui.layout, LayoutPreset::ProToolsStyle);
        let press = |h: &mut egui_kittest::Harness<'static, SoundApp>, label: &str| {
            let at = h.query_by_label(label).unwrap_or_else(|| panic!("{label}")).rect().center();
            h.hover_at(at);
            h.step();
            h.drag_at(at);
            h.step();
            h.drop_at(at);
            h.run_steps(3);
        };
        // A pick changes the mode at once and the question stays open until Start.
        press(&mut h, "Reaper-style");
        assert_eq!(h.state().ui.layout, LayoutPreset::Reaper);
        assert!(h.state().ask_mode);
        press(&mut h, "Start");
        assert!(!h.state().ask_mode);
        assert!(h.query_by_label("Welcome to Solara").is_none());
        assert_eq!(h.state().ui.layout, LayoutPreset::Reaper);
    }
}
