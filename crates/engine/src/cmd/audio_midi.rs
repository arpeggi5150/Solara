//! Edit › Copy Special › Audio as MIDI: pitch-track the selected audio into MIDI notes on the
//! clipboard (paste them onto a MIDI or instrument track). And Audio to MIDI (Ableton-style
//! layout): an audio clip becomes a MIDI clip on a new instrument track, as a melody, as harmony
//! (chords) or as drums.
use super::{CommandSpec, bad, has_range, range_param, tracks_required};

use crate::{Clipboard, Engine, Result, cmd};
use serde_json::{Value, json};
use soundcraft_midi::{Note, Sequence};
use soundcraft_model::{Clip, ClipId};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "clip.audio_to_midi",
            "Convert to MIDI",
            [],
            None,
            "{clip, mode: melody|harmony|drums}: a new instrument track (a synth, or a Drum Rack for drums) with the clip's notes, where the clip is (same place, or same scene)",
            super::always,
            audio_to_midi
        ),
        cmd!(noundo "edit.copy_audio_as_midi", "Audio as MIDI", ["Edit", "Copy Special"], None, "{track?, start?, end?} — monophonic pitch tracking of the selected audio", has_range, audio_as_midi),
    ]
}

fn audio_as_midi(e: &mut Engine, p: &Value) -> Result<Value> {
    let id = "edit.copy_audio_as_midi";
    let t = tracks_required(e, id, p)?.first().copied().ok_or_else(|| bad(id, "no track"))?;
    let r = range_param(e, id, p)?;
    if r.is_empty() {
        return Err(bad(id, "select some audio first"));
    }
    let s = e.session();
    if s.track(t).is_none_or(|x| x.kind != soundcraft_model::TrackKind::Audio) {
        return Err(bad(id, "select audio on an audio track"));
    }
    let r = crate::io::bounded_to_track(s, t, r);
    let audio = soundcraft_mix::render_clips(s, t, r);
    let n = audio.first().map_or(0, Vec::len);
    let k = audio.len().max(1) as f32;
    let mono: Vec<f32> = (0..n).map(|i| audio.iter().map(|c| c.get(i).copied().unwrap_or(0.0)).sum::<f32>() / k).collect();
    let detected = soundcraft_dsp::pitch_detect::audio_to_notes(&mono, s.sample_rate.as_f64() as f32);
    let base = s.tempo.samples_to_ticks(r.start, s.sample_rate);
    let mut seq = Sequence::default();
    for d in &detected {
        let a = s.tempo.samples_to_ticks(r.start + d.start as i64, s.sample_rate) - base;
        let b = s.tempo.samples_to_ticks(r.start + d.end as i64, s.sample_rate) - base;
        seq.notes.push(Note { pitch: d.pitch, velocity: d.velocity, release_velocity: 64, channel: 0, start: a.max(0), length: (b - a).max(1) });
    }
    seq.sort();
    let count = seq.notes.len();
    let clip = Clip::midi(ClipId(0), "Audio as MIDI", 0, r.len(), seq);
    e.clipboard = Clipboard { tracks: vec![vec![clip]], automation: vec![Vec::new()], ..Clipboard::of_range(e.session(), r) };
    Ok(json!({"notes": count, "length": r.len()}))
}

/// The most audio Audio to MIDI reads from one clip (ten minutes at 48 kHz).
const MAX_CONVERT_SAMPLES: usize = 48_000 * 600;

fn audio_to_midi(e: &mut Engine, p: &Value) -> Result<Value> {
    let id = "clip.audio_to_midi";
    let cid = super::clip_id(p, id)?;
    let mode = super::str_param(p, "mode").unwrap_or("melody");
    if !matches!(mode, "melody" | "harmony" | "drums") {
        return Err(bad(id, format!("unknown mode `{mode}` (melody, harmony or drums)")));
    }
    let s = e.session();
    let (track, clip) = s.find_clip(cid).map(|(t, c)| (t, c.clone())).ok_or_else(|| bad(id, format!("no clip {}", cid.0)))?;
    let soundcraft_model::ClipContent::Audio { source, offset } = clip.content else { return Err(bad(id, "only audio clips convert to MIDI")) };
    let audio = s.pool.get(source).ok_or_else(|| bad(id, "the clip's audio file is not loaded"))?;
    // The clip's audio as mono, as it sits in the file (the clip's gain does not change the notes).
    let start = usize::try_from(offset.max(0)).unwrap_or(0);
    let len = usize::try_from(clip.length).unwrap_or(0).min(MAX_CONVERT_SAMPLES);
    let k = audio.buffer.channels.len().max(1) as f32;
    let mono: Vec<f32> =
        (start..start.saturating_add(len)).map(|i| audio.buffer.channels.iter().map(|c| c.get(i).copied().unwrap_or(0.0)).sum::<f32>() / k).collect();
    let sr = s.sample_rate.as_f64() as f32;
    let detected: Vec<soundcraft_dsp::pitch_detect::DetectedNote> = match mode {
        "harmony" => soundcraft_dsp::pitch_detect::audio_to_chords(&mono, sr),
        "drums" => {
            let hits = soundcraft_dsp::offline::detect_transients(std::slice::from_ref(&mono), sr, 0.5);
            let sixteenth = s.tempo.tick_to_samples(TICKS_PER_SIXTEENTH, s.sample_rate).max(1) as usize;
            hits.iter()
                .map(|&at| {
                    let hit = mono.get(at..(at + 2048).min(mono.len())).unwrap_or(&[]);
                    let peak = hit.iter().fold(0.0f32, |m, x| m.max(x.abs()));
                    let velocity = (40.0 + 87.0 * peak.min(1.0)) as u8;
                    soundcraft_dsp::pitch_detect::DetectedNote {
                        start: at,
                        end: at + sixteenth,
                        pitch: soundcraft_dsp::pitch_detect::drum_note(hit, sr),
                        velocity: velocity.clamp(1, 127),
                    }
                })
                .collect()
        }
        _ => soundcraft_dsp::pitch_detect::audio_to_notes(&mono, sr),
    };
    // Notes in ticks from the clip's start (a slot clip starts at 0).
    let base = s.tempo.samples_to_ticks(clip.start, s.sample_rate);
    let at = |x: usize| s.tempo.samples_to_ticks(clip.start.saturating_add(i64::try_from(x).unwrap_or(i64::MAX)), s.sample_rate) - base;
    let mut seq = Sequence::default();
    for d in &detected {
        let (a, b) = (at(d.start), at(d.end));
        seq.notes.push(Note { pitch: d.pitch, velocity: d.velocity, release_velocity: 64, channel: 0, start: a.max(0), length: (b - a).max(1) });
    }
    seq.sort();
    let count = seq.notes.len();
    let slot = s.find_slot_clip(cid).map(|(_, i)| i);
    let name = format!("{} MIDI", s.track(track).map_or("Audio", |t| t.name.as_str()));
    let s = e.session_mut();
    let t = s.add_track(soundcraft_model::TrackKind::Instrument, soundcraft_model::ChannelFormat::Stereo, Some(&name));
    // Next to the audio's track.
    if let (Some(from), Some(to)) = (s.track_index(t), s.track_index(track)) {
        let tr = s.tracks.remove(from);
        s.tracks.insert((to + 1).min(s.tracks.len()), tr);
    }
    let plugin = if mode == "drums" { "drum_rack" } else { "subtractive_synth" };
    let n = s.scenes.len();
    if let Some(tr) = s.track_mut(t) {
        tr.instrument = Some(soundcraft_model::Insert::new(plugin));
        tr.slots.resize(n, None);
    }
    let new_id = s.new_clip_id();
    let mut mc = Clip::midi(new_id, format!("{} ({mode})", clip.name), clip.start, clip.length, seq);
    mc.follow = clip.follow;
    match slot {
        Some(i) => {
            if let Some(x) = s.track_mut(t).and_then(|tr| tr.slots.get_mut(i)) {
                *x = Some(mc);
            }
        }
        None => crate::edit::place_clip(s, t, mc),
    }
    Ok(json!({"track": t, "clip": new_id, "notes": count}))
}

/// Ticks in a sixteenth note: a drum hit's note length.
const TICKS_PER_SIXTEENTH: i64 = soundcraft_time::TICKS_PER_QUARTER / 4;

#[cfg(test)]
mod tests {
    use super::*;
    use soundcraft_audio_io::AudioBuffer;
    use soundcraft_model::{ChannelFormat, TrackKind};

    #[test]
    fn audio_clips_convert_to_melody_harmony_and_drums() {
        let mut e = Engine::default();
        let sr = 48_000.0f32;
        let tone =
            |f: f32, secs: f32| -> Vec<f32> { (0..(secs * sr) as usize).map(|i| (i as f32 * f * std::f32::consts::TAU / sr).sin() * 0.3).collect() };
        let mut mel = tone(220.0, 0.4);
        mel.extend(tone(330.0, 0.4));
        let chord: Vec<f32> = tone(261.63, 0.8).iter().zip(tone(329.63, 0.8)).zip(tone(392.0, 0.8)).map(|((a, b), c)| a + b + c).collect();
        let mut drums = vec![0.0f32; 48_000];
        for k in 0..4 {
            for i in 0..4800 {
                let x = (i as f32 * 55.0 * std::f32::consts::TAU / sr).sin() * (-(i as f32) / 1200.0).exp();
                if let Some(d) = drums.get_mut(k * 12_000 + i) {
                    *d = x;
                }
            }
        }
        let s = e.session_mut();
        let t = s.add_track(TrackKind::Audio, ChannelFormat::Mono, Some("Gtr"));
        let mut place = |name: &str, a: Vec<f32>, at: i64| {
            let n = a.len() as i64;
            let src =
                crate::io::add_source(s, name, AudioBuffer { sample_rate: 48_000, channels: vec![a] }, None, soundcraft_audio_io::FileFormat::Wav);
            let id = s.new_clip_id();
            crate::edit::place_clip(s, t, Clip::audio(id, name, src, 0, at, n));
            id
        };
        let (m, h, d) = (place("mel", mel, 0), place("chord", chord, 100_000), place("beat", drums, 200_000));
        let pitches = |e: &Engine, r: &Value| -> Vec<u8> {
            let c = e.session().find_clip(ClipId(r["clip"].as_u64().unwrap())).unwrap().1.clone();
            match c.content {
                soundcraft_model::ClipContent::Midi { sequence } => sequence.notes.iter().map(|n| n.pitch).collect(),
                _ => Vec::new(),
            }
        };
        let r = e.execute("clip.audio_to_midi", &json!({"clip": m.0, "mode": "melody"})).unwrap();
        assert_eq!(pitches(&e, &r), vec![57, 64]);
        let track = soundcraft_model::TrackId(r["track"].as_u64().unwrap());
        assert_eq!(e.session().track(track).unwrap().clips()[0].start, 0, "where the audio was");
        assert_eq!(e.session().tracks.iter().position(|x| x.id == track), e.session().tracks.iter().position(|x| x.id == t).map(|i| i + 1));
        let r = e.execute("clip.audio_to_midi", &json!({"clip": h.0, "mode": "harmony"})).unwrap();
        assert_eq!(pitches(&e, &r), vec![60, 64, 67]);
        let r = e.execute("clip.audio_to_midi", &json!({"clip": d.0, "mode": "drums"})).unwrap();
        assert_eq!(pitches(&e, &r), vec![36, 36, 36, 36]);
        let t2 = soundcraft_model::TrackId(r["track"].as_u64().unwrap());
        assert_eq!(e.session().track(t2).unwrap().instrument.as_ref().map(|i| i.plugin.as_str()), Some("drum_rack"));
        assert!(e.execute("clip.audio_to_midi", &json!({"clip": m.0, "mode": "kazoo"})).is_err());
        let midi_clip = r["clip"].as_u64().unwrap();
        assert!(e.execute("clip.audio_to_midi", &json!({"clip": midi_clip})).is_err(), "MIDI does not convert");
    }

    #[test]
    fn copies_a_melody_as_midi_and_pastes_it() {
        let mut e = Engine::default();
        let sr = 48_000.0f32;
        let tone = |f: f32| -> Vec<f32> { (0..(0.4 * sr) as usize).map(|i| (i as f32 * f * std::f32::consts::TAU / sr).sin() * 0.5).collect() };
        let mut a = tone(220.0);
        a.extend(tone(330.0));
        let frames = a.len() as i64;
        let s = e.session_mut();
        let src = crate::io::add_source(s, "mel", AudioBuffer { sample_rate: 48_000, channels: vec![a] }, None, soundcraft_audio_io::FileFormat::Wav);
        let t = s.add_track(TrackKind::Audio, ChannelFormat::Mono, None);
        let cid = s.new_clip_id();
        crate::edit::place_clip(s, t, Clip::audio(cid, "mel", src, 0, 0, frames));
        let r = e.execute("edit.copy_audio_as_midi", &json!({"track": t.0, "start": 0, "end": frames})).unwrap();
        assert_eq!(r["notes"], 2, "{r}");
        let m = e.session_mut().add_track(TrackKind::Midi, ChannelFormat::Mono, None);
        e.execute("edit.paste", &json!({"track": m.0, "at": 0})).unwrap();
        let clip = e.session().track(m).unwrap().clips()[0].clone();
        match clip.content {
            soundcraft_model::ClipContent::Midi { sequence } => assert_eq!(sequence.notes.iter().map(|n| n.pitch).collect::<Vec<_>>(), vec![57, 64]),
            _ => panic!("expected MIDI"),
        }
    }
}
