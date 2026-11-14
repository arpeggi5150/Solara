# Control protocol

Solara can be driven completely by other programs: scripts, test harnesses and AI agents.
Every menu item, button, mouse gesture and keystroke maps to a **command** (or to synthetic
input), and every command is reachable from:

| Surface | How |
|---|---|
| Desktop app control channel | `solara --control <port>` (`0` = pick a free port; printed as `SOLARA_CONTROL_PORT=…`) |
| CLI against a running app | `solara-cli app --port <port> <method or command id> [JSON]` (see [cli.md](cli.md)) |
| Headless CLI | `solara-cli run --demo --cmd 'mix.volume={"track":"Kick","db":-6}' --bounce out.wav` |
| MCP (headless) | `solara-cli mcp [--demo]` |
| MCP (bridged to the app) | `solara-cli mcp --connect <port>` |

## Isolated automation profiles

Set `SOUNDCRAFT_CONFIG_DIR` to a fresh directory to isolate UI preferences, `Autosave`, `Presets`, and `logs`.
The nonempty value is used as the complete settings directory, even in portable mode (`portable.txt` beside the executable, which otherwise uses `SolaraData` next to it). Unset retains the normal platform paths;
an explicit empty value disables the settings directory instead of falling back to a personal profile. Save synthetic sessions to explicit scratch paths. Do not use `SOUNDCRAFT_NO_PREFS`
when testing preference persistence across restarts.

Run UI acceptance with `--no-audio`. This disables the output player; it does not disable recording
input opened by arm/monitor commands. An automated UI smoke run must never arm a track, enable input
monitoring, or start recording. Control input injects egui events; it does not establish operating-system
input delivery or screen-reader acceptance. Native screenshots use the presented viewport frame.

## Wire format

Newline-delimited JSON over TCP on `127.0.0.1`. One request per line, one reply per line:

```json
{"id": 1, "method": "engine.execute", "params": {"command": "track.new", "params": {"count": 2, "format": "Stereo"}}}
{"id": 1, "ok": true, "result": {"tracks": [12, 13]}}
```

Audio devices: the `ui.audio_device` command (sent through `engine.execute`, like other `ui.*`
commands) takes `{list?, system?, output?, input?, sample_rate?, buffer_size?, reset_health?}`. It
lists audio systems and devices (`list`), sets the device choice, and returns the open device with
its `buffer_size` (null = the device default) and `health` since it opened or since `reset_health`:
`blocks`, `late` (filled slower than real time), `gaps` (a wait between blocks long enough for the
device to run dry), `xruns`, `worst_render_us`, `worst_gap_us`, `period_us` (the block the device
actually asks for, as measured) and `clean`. On Windows WASAPI runs in shared mode, where
the device asks for the system's period (usually 10 ms) whatever buffer is requested. On Windows
x64 the system `"Low-latency driver"` lists the installed ASIO drivers (by the names they
register) with the rates and the exact buffer sizes each offers; choosing one plays the output
through that driver at the chosen buffer size. Input stays on the default system. A driver that
will not open falls back to the default system, with a warning in the log.

Errors: `{"id": 1, "ok": false, "error": "bad parameters for `mix.volume`: no track named `Nope`"}`.

## Methods

| Method | Params | What it does |
|---|---|---|
| `engine.execute` (alias `command`) | `{command, params}` | Run a command programmatically. **Never opens a dialog**; empty params use defaults or the current selection. |
| `engine.commands` | `{filter?}` | Every command: id, label, menu path, shortcut, params doc, enabled + reason. Includes UI-layer commands (`window.*`). |
| `engine.parity` | `{}` | Parity report against the incumbent's menu catalog. |
| `session.inspect` | `{detail?: "summary"|"full"}` | The document: tracks, clips (with times), inserts (+params when full), sends, routing, automation, markers, groups, busses, sources (`loaded` true when decoded audio is in the pool), selection, zoom (`samples_per_px`, `scroll`), transport, undo label. |
| `ui.inspect` | `{}` | UI state (`window`, panels, dialogs), window size, playing/position, audio device, and `edit_layout` (timeline rect and every track row's rect, for clicking). |
| `ui.set` | any `UiState` fields | e.g. `{"window": "Mix", "narrow_mix": true}`. |
| `ui.menu.list` | `{}` | Every catalog menu path and the command that implements it (or null). |
| `ui.menu.invoke` | `{path}` or `{id}` | Like clicking the menu item: may open its dialog (e.g. `"Track > New..."`). |
| `ui.click` | `{x, y, button?, count?, cmd?, shift?, alt?, ctrl?}` | Synthetic click (window points). The reply is sent only after a UI pass has applied the press/release. |
| `ui.drag` | `{x, y, to_x, to_y, steps?}` | Synthetic drag with the primary button. |
| `ui.scroll` | `{x, y, dx?, dy?, unit?: "point"\|"line", steps?, cmd?, shift?, alt?, ctrl?}` | A two-finger trackpad swipe over `(x, y)`: `dx`/`dy` points spread over `steps` frames (default 8). Positive values move the content right/down (natural scrolling). `unit: "line"` sends mouse-wheel notches instead. With `cmd` it zooms; with `shift` it scrolls sideways. Point scrolling is trackpad-like: egui 0.37 scroll areas may rubber-band at an edge and spring back after the gesture, so a script that needs an exact final offset should use `unit: "line"` or drive the target with a command. |
| `ui.zoom` | `{x, y, factor, steps?}` | A trackpad pinch around `(x, y)`: `factor` > 1 spreads the fingers (zooms in). |
| `ui.move` | `{x, y}` | Move the pointer. |
| `ui.key` | `{key, cmd?, shift?, alt?, ctrl?}` | Key press (`Space`, `Enter`, `A`, `F7`, `=`…). |
| `ui.text` | `{text}` | Type text into the focused field. At most 64 KiB of UTF-8 text per request. |
| `ui.screenshot` | `{path?}` | PNG of the window (base64 in `png_base64` when no path). Exactly one reply per request. A request is bound to the window geometry, focus state and open session it was admitted for. A geometry or DPI change is retried within its 8 s deadline, then fails `screenshot stale`; a change of focused window or of the open document cancels it (`screenshot canceled: focus changed` / `... the session changed`) because those pixels must not be reported as a capture of the new state. It can also fail `screenshot queue full` (8 pending, 4 in flight), `screenshot too large` (64 MiB per image, 192 MiB aggregate **encoding peak**), `screenshot path longer than 4096 bytes`, or `screenshot unavailable: the window has no capturable size yet` (a first request before any window geometry is known, e.g. minimised before the first paint). Each request reserves its own worst-case encode peak — the captured RGBA image plus the raw copy, PNG buffer and base64 text built during encoding — so several small captures cannot retain or encode full-size images past the aggregate. The callback never writes a file: encoding and writing happen after collection, and a result arriving after the deadline returns `screenshot timed out` without creating the file. A native hidden/minimised window is asked for an explicit paint; browser hidden tabs cannot paint on request and time out. |
| `app.quit` | `{}` | Quit. |

Scripted input (`ui.click`, `ui.drag`, `ui.scroll`, `ui.zoom`, `ui.key`, `ui.text`, `ui.move`) queues at most 4096 waiting events; a request that would exceed that bound fails with `synthetic input queue full` instead of growing the queue. A gesture's reply is delivered only after a UI pass has applied all of its events. Requests that follow it on the same connection (inspect, engine commands, screenshots) are therefore ordered after the gesture; requests already queued behind an undelivered gesture wait as well. If no UI pass can deliver the gesture within 15 s, it is canceled with `synthetic input was not delivered` and the request chain continues. Scripted input is automation, not a clipboard action: copy/selection commands it triggers are not published to the OS clipboard or PRIMARY selection.

## Positions and references

- Tracks: id (number) or name (`"Kick"`).
- Clips: id (from `session.inspect`).
- Positions: samples (`96000`), seconds (`{"seconds": 2.5}`), or a time string in any timebase,
  optionally prefixed: `"bars_beats:5|1|000"`, `"min_secs:0:12.500"`, `"timecode:00:00:10:00"`,
  `"samples:48000"`.
- Ranges: `start` + `end` (or `start` + `length`); omitted = the current edit selection.

## Examples

```sh
P=$(solara --demo --control 0 | sed -n 's/SOLARA_CONTROL_PORT=//p')   # or read stderr
solara-cli app --port $P mix.solo '{"track":"Bass"}'
solara-cli app --port $P transport.play
solara-cli app --port $P ui.screenshot '{"path":"/tmp/shot.png"}'
solara-cli app --port $P ui.menu.invoke '{"path":"Track > New..."}'
```

Offscreen (no window): `cargo run -p soundcraft-ui-egui --example ui_shot -- out.png script.jsonl`,
where each script line is a request (`{"method": …, "params": …}`), `{"shot": "path.png"}` or
`{"steps": n}`. Use `--system-theme light|dark|none` to supply the initial OS appearance, and
script events such as `{"system_theme": "dark"}` to change it on subsequent frames.

Appearance defaults to Dark. `ui.theme {"mode":"system"}` follows the appearance supplied by
the native or web integration at launch and on later frames, while the saved choice stays System.
When the integration cannot report an appearance, as on Linux, System reads the desktop's
settings portal (`org.freedesktop.appearance` `color-scheme`) and follows its changes; without
a portal answer it uses Dark. Manual Light and Dark stay fixed when the OS appearance changes.
An offscreen render without `--system-theme` supplies no OS appearance and therefore resolves
System to Dark, or on Linux to the desktop's portal setting when one is available.

Layouts: the app opens in the NEW layout.
`ui.layout {"preset":"new"|"flStyle"|"classic"|"reaper"|"proToolsStyle"|"abletonStyle"}` switches between NEW,
the FL-style layout (Browser and Channel Rack on the left, Playlist in the centre, Piano Roll and
Mixer along the bottom), Classic (the Edit and Mix windows), the Reaper-style layout (transport on
top, track list and arrange view in the centre, mixer along the bottom) and the Pro Tools-style
layout (the Edit window with the Mix docked under it) and the Ableton-style layout (Browser and
Session View on the left, Arrangement in the centre, Device View along the bottom) without
touching the session; with no `preset` it returns the current one. The Ableton-style layout's
panels are `ui.browser`, `ui.session_view` and `ui.device_view` (`{"value"?: bool}`), and
`ui.launch_scene` launches the scene selected in its Session View.
`ui.mixer {"value"?: bool}` expands or collapses the bottom mixer of NEW (M), the FL-style layout
(F9), the Reaper-style layout (Ctrl+M) or the Pro Tools-style layout (Ctrl+=, where
`window.toggle_mix_edit` shows or hides the docked Mix instead of swapping windows); each keeps its
own state. The Pro Tools-style layout always shows an audio clip's gain line, and the Smart,
Grabber or Trim tool drags it up and down (`clip.gain`). The FL-style layout's panels are `ui.channel_rack`,
`ui.piano_roll` and `ui.browser` (`{"value"?: bool}`), and `ui.roll_slide`, `ui.roll_strum` and
`ui.roll_arpeggiate` apply to the notes selected in its piano roll. In NEW,
`ui.editor {"view": "auto"|"steps"|"piano"}` picks what the bottom Editor shows (B and P).

Each layout brings its own key set: NEW's single-letter keys, Classic's shortcuts, the
Pro Tools-style keys (Classic's shortcuts, with Ctrl+= for the docked Mix), the Ableton-style
keys (Tab Session View, Ctrl+Alt+B Browser, Ctrl+Alt+L Device View, Enter launch scene, Ctrl+I
insert scene, Ctrl+E split, Ctrl+J consolidate, Ctrl+D duplicate, Ctrl+U quantize, Ctrl+T new
track, F9 record, Ctrl+L loop, Delete clear), the FL-style keys (F6 Channel Rack, F7 Piano Roll, Alt+F8 Browser, F9 Mixer, R record, Ctrl+B
duplicate, Delete clear, Alt+S strum, Alt+A arpeggiate) and the
Reaper-style keys (Ctrl+R record, R repeat, S split, Ctrl+T new track, Ctrl+M mixer, Alt+R
Routing Matrix, ? Action List, Home/End, Delete, = and - zoom). The shared shortcuts (Space,
Ctrl+S, Ctrl+Z, ...) work in every layout. `ui.keymap {}` lists the keys assigned by hand in
the active layout's set; `ui.keymap {"command", "key": "Cmd+Shift+R"}` assigns one (taking it
from any other command in that set) and `{"command", "key": null}` goes back to the default.
`window.action_list` opens the Action List (every command: search, run, set a key) and
`window.routing_matrix` the Routing Matrix (every track's sends on one grid). What can receive
follows the layout: in the Reaper-style layout every track can, through
`mix.receive {"track", "value"?: bool}`, which gives an audio, instrument or MIDI track a
receive bus the mix engine adds in ahead of its inserts; in the other layouts sends go to aux,
folder and bus tracks only.

Session View, Ableton-style: scenes are rows of clip slots, one slot per audio, instrument and
MIDI track. `scene.new {name?, at?}`, `scene.rename {scene, name}`, `scene.move {scene, to}` and
`scene.delete {scene}` edit the rows; `slot.set {track, scene, clip? | bars? | source?}` fills a
slot with a copy of a clip, a new empty MIDI clip of `bars` bars or an audio file,
`slot.clear {track, scene}` empties it and `slot.move {track, scene, to_track?, to_scene, copy?}`
moves or copies it. `slot.launch {track, scene, quantize?: "bar"|"beat"|"none"}` plays a slot
from the next bar, `scene.launch {scene, quantize?}` plays the whole row (tracks with an empty
slot stop), `slot.stop {track}` stops one track and `launcher.stop_all {}` every track; a track
with a launched clip leaves its Arrangement clips silent until it stops. Launching is not an
edit: it is not undone and does not dirty the session. `clip.follow {clips?, action:
"again"|"next"|"random"|"stop"|"none", plays?}` sets what a slot clip does after it has played
`plays` times. `clip.warp {clips?, value?}` makes audio follow the tempo: a tempo change
re-stretches warped Arrangement audio from its original (pitch kept) so it stays on its bars.
`launcher.settle {at?}` reports what is playing on each track at a position.

Groove Pool: `groove.add {name?, swing?, humanize?, starter?, from_clip?}` saves a groove (swing
and humanize 0..1; starters Swing 25, Swing 50, Swing 75, Loose, Lazy and Shuffle),
`groove.edit {groove, name?, swing?, humanize?}` changes it for every clip that uses it,
`groove.apply {clips?, groove | null}` gives MIDI clips a pool groove and `groove.delete {groove}`
removes it (its clips keep their feel). Instrument Rack: `rack.create {track}` puts eight macros
around an instrument track's instrument, mapped to its first eight continuous parameters;
`rack.macro {track, macro, value: 0..1}` turns one, `rack.map {track, macro, param, min?, max?}`
and `rack.unmap {track, macro, param}` change what it moves, `rack.rename {track, macro, name}`
names it and `rack.remove {track}` takes the rack off (the instrument stays as it is). The Drum
Rack (`drum_rack`) is an instrument with sixteen pads from MIDI note 36, each with a sound, tune
and level (`pN_sound`, `pN_tune`, `pN_level`). `clip.audio_to_midi {clip, mode:
"melody"|"harmony"|"drums"}` writes the notes it hears in an audio clip to a new instrument track
beside it (a Drum Rack for drums), at the same place or in the same scene slot.

Patterns, FL-style: `playlist.place {"clip", "at", "track"?}` places the pattern a MIDI clip
holds; the places stay one pattern, so a step or note edited in one place reaches every place
that held the same notes (`playlist.make_unique {"clip"}` lets one go; `playlist.patterns` lists
each pattern once). `rack.groove {"clip", "swing"?: 0..100, "humanize"?: 0..100}` sets a pattern's
swing (every second sixteenth later) and humanize (repeatable timing and velocity variation),
applied as it plays; the notes stay put. Piano roll tools: `midi.stamp {"clip", "chord", "pitch",
"start_ticks", "length_ticks"?}`, `midi.slide {"clip", "indices", "value"?}` (portamento, CC65,
around the note: it glides from the note before), `midi.strum {"clip", "indices", "ticks"?,
"down"?}` and `midi.arpeggiate {"clip", "indices", "rate_ticks"?, "direction"?, "gate"?}`.
Automation clips: `auto_clip.new {"track", "param"?, "start"?, "end"?}` holds a stretch of a lane
as a clip; `auto_clip.move` and `auto_clip.copy {"id", "at", "track"?}` write its shape into the
lane where it lands, so it drives the parameter there; `auto_clip.delete {"id"}` and
`auto_clip.list {"track"?}`.

New projects come from `session.new {"template": "drums"}` (one
Drums track, as a fresh launch and File > New start) or `{"template": "empty"}` (no tracks); both start with loop
playback on.

Patterns are one-bar MIDI clips edited in NEW's step sequencer (16 sixteenth-note steps per
bar). `pattern.step {"clip"|"track", "pitch", "step", "velocity"?}` sets (velocity 1..127),
clears (0) or toggles (no velocity) a step; given a track instead of a clip, it uses the track's
first MIDI clip, making one on bar 1 if there is none. A step in a looped pattern changes in each
of its loops. `pattern.new {"track", "at"?}` makes an empty pattern, `pattern.add_sound {"pitch",
"track"?, "at"?}` is what dropping a Browser sound does (without a track it makes a Drums track),
and `pattern.copy {"clip", "at", "track"?}` drops an independent copy (Alt-drag in NEW). Each
selects its pattern, so Play loops it while loop playback is on. In NEW, dragging a MIDI clip's
end past its end runs `clip.loop` with that length.

`view.scroll_into_view {"track":"Bass"}` (a track id or name; without it, the first selected
track) brings the track to the top of the Edit window and the left of the Mix window, the same as
Scroll Into View in the track name menu. A window not on screen catches up when it is next shown.

## Panel docking

Tracks and Groups, Clips, MIDI Editor, Memory Locations, and Undo History use movable panels. Commands target the active Edit or Mix view; floating panels stay inside the current viewport. Edit and Mix retain separate arrangements; Clips and MIDI Editor belong to Edit. Transport, plug-in, and specialist utility windows retain their existing hosts. The document/session view is protected.

The Edit window starts with Tracks and Groups in a 168-point column in a dock at the left edge, Clips in a 230-point column in a dock at the right edge, and the MIDI Editor under the session view between them. Both docks run the window's full height. Mix starts with Tracks and Groups docked at its left edge. Memory Locations and Undo History float. A dock column stacks tab groups top to bottom: a panel dropped on a group's middle joins it as a tab, on its upper or lower third stacks a new group above or below it, and at a column's edge makes a new column. A panel dropped at an empty window edge makes a dock there. Layouts, active tabs, panel visibility, and floating rectangles are saved with the app's existing UI preferences. These operations do not edit document contents.

Panel IDs: `tracks`, `clips`, `midi`, `memory`, `history`. The protected center is `canvas`.

`ui.dock` accepts `operation` and `panel`. Operations are `open`, `close`, `activate`, `float` (with finite positive `rect: [x, y, width, height]`), or `move` (with `target` and `zone: tab|center|left|right|top|bottom|above|below`; PascalCase names such as `Left` also parse). `open` and `activate` show a hidden panel at its saved location and bring a visible panel to the front. A panel with no saved location, or whose saved anchor cannot be restored, opens at its default place: Tracks and Groups and Clips join their dock's first group, or make that dock again at its default width when it is gone; the MIDI Editor opens under the session view; Memory Locations and Undo History float at their default rectangles. Center and `tab` group with the target; side zones split beside it; `above` and `below` stack the panel in a new group above or below a floating target's group. `ui.dock.reset` restores the default arrangement. Invalid moves return an error without changing the layout.

Invoke these UI commands through `engine.execute`, for example `{"command":"ui.dock","params":{"operation":"float","panel":"tracks","rect":[100,100,320,440]}}`.

Advanced `ui.dock` operations use the same validated transaction as pointer gestures:
`moveFloating` updates `rect`, `resizeSplit` accepts a boolean child `path` and
`size` (`{"Ratio":0.5}`, `{"FixedFirst":280}`, or `{"FixedSecond":280}`),
`setStackOpen` takes `open`, and `resizeStack` takes a point `height` or null.
A center `move` can include `before` (panel ID or null to append) for tab order.
The equivalent `{"action": ...}` form accepts a serialized shared docking action;
this is also the form emitted by the renderer. Protected-canvas and app-specific panel
rules apply equally to both forms.

The saved UI preferences and window configurations carry `docking_version`. Settings with no
version, or an older one, hold the split-frame layouts of earlier releases: they load with the
default Edit and Mix layouts and keep every other setting.
