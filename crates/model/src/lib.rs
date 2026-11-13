//! The Solara session document.
//!
//! A [`Session`] is pure data: tracks with playlists of clips, the mixer (inserts, sends, routing,
//! automation), tempo/meter, memory locations and groups. Audio sample data lives in
//! [`SourcePool`] behind `Arc`s, so cloning a session for undo is cheap.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod auto_clip;
mod automation;
pub mod b64;
mod clip;
mod ids;
mod launcher;
mod markers;
mod mixer;
mod session;
mod source;
mod track;

pub use auto_clip::{AutoClip, MAX_AUTO_CLIPS};
pub use automation::{AutoParam, AutomationLane, AutomationMode, AutomationPoint};
pub use clip::{Clip, ClipContent, Fade, FadeShape, Groove};
pub use ids::{BusId, ClipId, GroupId, MarkerId, SourceId, TrackId};
pub use launcher::{
    Follow, FollowAction, GroovePreset, Launch, MACROS, MAX_FOLLOW_PLAYS, MAX_GROOVES, MAX_LAUNCHES, MAX_MACRO_TARGETS, MAX_SCENES, Macro,
    MacroTarget, Pass, Rack, Scene,
};
pub use markers::{Group, MarkerKind, MemoryLocation};
pub use mixer::{
    Bus, ChannelFormat, INSERT_SLOTS, Insert, Mixer, OutputPath, Route, SEND_SLOTS, SendSlot, Speaker, SurroundPan, fader_db_to_pos, fader_pos_to_db,
};
pub use session::{
    AutoName, BitDepthSetting, EditMode, EditState, FEET_FRAMES_START, MAX_COMMENT_CHARS, MAX_NAME_CHARS, SESSION_EXTENSION, Session, SessionError,
    Tool, TrackView, ZoomState, ZoomToggle, truncate_chars,
};
pub use source::{Source, SourceAudio, SourcePool, VideoSource};
pub use track::{Playlist, TRACK_COLORS, Track, TrackHeight, TrackKind};

pub use soundcraft_time as time;
