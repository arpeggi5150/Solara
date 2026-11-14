//! MIDI keyboards: each input port opened through `midir` sends its messages to the audio thread
//! through a bounded channel the callback never blocks on. The audio thread plays them on the
//! record-enabled and input-monitored instruments, and while recording hands each one back
//! stamped with the session sample it was heard at.

use soundcraft_time::Samples;
use std::sync::mpsc::SyncSender;

/// Messages waiting for the audio thread at most; a keyboard faster than that loses the rest.
pub const QUEUE: usize = 4096;
/// Recorded messages waiting for the UI at most.
pub const RECORDED: usize = 1 << 16;

/// A message played while recording, at the session sample it was heard at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecordedMidi {
    pub at: Samples,
    pub msg: [u8; 3],
}

/// A channel message from raw bytes; system messages (clock, sysex…) and running status are not.
pub fn channel_message(bytes: &[u8]) -> Option<[u8; 3]> {
    let status = *bytes.first()?;
    if !(0x80..0xF0).contains(&status) {
        return None;
    }
    let d1 = bytes.get(1).copied().unwrap_or(0) & 0x7F;
    let d2 = bytes.get(2).copied().unwrap_or(0) & 0x7F;
    Some([status, d1, d2])
}

/// The MIDI input ports the system offers, by name.
#[cfg(not(target_arch = "wasm32"))]
pub fn input_names() -> Vec<String> {
    let Ok(m) = midir::MidiInput::new("Solara") else { return Vec::new() };
    m.ports().iter().filter_map(|p| m.port_name(p).ok()).collect()
}

#[cfg(target_arch = "wasm32")]
pub fn input_names() -> Vec<String> {
    Vec::new()
}

/// Open MIDI input ports.
pub struct MidiInputs {
    #[cfg(not(target_arch = "wasm32"))]
    _conns: Vec<midir::MidiInputConnection<()>>,
    /// The ports opened.
    pub names: Vec<String>,
    /// Ports that would not open, with why.
    pub failed: Vec<String>,
}

impl MidiInputs {
    /// Open every input port `wanted` accepts; their messages go to `tx`.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn open(wanted: impl Fn(&str) -> bool, tx: &SyncSender<[u8; 3]>) -> MidiInputs {
        let mut out = MidiInputs { _conns: Vec::new(), names: Vec::new(), failed: Vec::new() };
        for name in input_names().into_iter().take(64) {
            if !wanted(&name) {
                continue;
            }
            // Connecting consumes the client, so each port gets its own.
            let Ok(m) = midir::MidiInput::new("Solara") else { continue };
            let Some(port) = m.ports().into_iter().find(|p| m.port_name(p).ok().as_deref() == Some(name.as_str())) else { continue };
            let tx = tx.clone();
            match m.connect(
                &port,
                "solara-in",
                move |_, bytes, _| {
                    if let Some(msg) = channel_message(bytes) {
                        // Full: the audio thread is behind; drop rather than wait.
                        let _ = tx.try_send(msg);
                    }
                },
                (),
            ) {
                Ok(c) => {
                    out._conns.push(c);
                    out.names.push(name);
                }
                Err(e) => out.failed.push(format!("{name}: {e}")),
            }
        }
        out
    }

    #[cfg(target_arch = "wasm32")]
    pub fn open(_wanted: impl Fn(&str) -> bool, _tx: &SyncSender<[u8; 3]>) -> MidiInputs {
        MidiInputs { names: Vec::new(), failed: Vec::new() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_channel_messages_pass() {
        assert_eq!(channel_message(&[0x90, 60, 100]), Some([0x90, 60, 100]));
        assert_eq!(channel_message(&[0xC0, 5]), Some([0xC0, 5, 0]));
        assert_eq!(channel_message(&[0x91, 200, 255]), Some([0x91, 72, 127]), "data bytes are 7-bit");
        assert_eq!(channel_message(&[0xF8]), None, "clock");
        assert_eq!(channel_message(&[0xF0, 1, 2, 0xF7]), None, "sysex");
        assert_eq!(channel_message(&[60, 100]), None, "running status");
        assert_eq!(channel_message(&[]), None);
    }
}
