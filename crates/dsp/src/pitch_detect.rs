//! Monophonic pitch tracking (YIN, de Cheveigné & Kawahara 2002) and note segmentation, for
//! audio-to-MIDI conversion.

/// A detected note: start and end in samples, MIDI pitch, and a velocity estimate.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DetectedNote {
    pub start: usize,
    pub end: usize,
    pub pitch: u8,
    pub velocity: u8,
}

/// Fundamental frequency of one frame, or None when unvoiced. `threshold` ≈ 0.1–0.2.
pub fn yin(frame: &[f32], sample_rate: f32, min_hz: f32, max_hz: f32, threshold: f32) -> Option<f32> {
    if !(sample_rate.is_finite() && sample_rate > 0.0) || frame.len() < 64 {
        return None;
    }
    let max_tau = ((sample_rate / min_hz.max(20.0)) as usize).min(frame.len() / 2);
    let min_tau = ((sample_rate / max_hz.max(min_hz + 1.0)) as usize).max(2);
    if max_tau <= min_tau.saturating_add(2) {
        return None;
    }
    let w = frame.len() - max_tau;
    let mut d = vec![0.0f32; max_tau + 1];
    for (tau, dv) in d.iter_mut().enumerate().skip(1) {
        let mut sum = 0.0f32;
        for j in 0..w {
            let (Some(a), Some(b)) = (frame.get(j), frame.get(j + tau)) else { break };
            let diff = a - b;
            sum += diff * diff;
        }
        *dv = sum;
    }
    // Cumulative mean normalised difference.
    let mut cmnd = vec![1.0f32; max_tau + 1];
    let mut running = 0.0f32;
    for tau in 1..=max_tau {
        let v = d.get(tau).copied().unwrap_or(0.0);
        running += v;
        if let Some(c) = cmnd.get_mut(tau) {
            *c = if running > 0.0 { v * tau as f32 / running } else { 1.0 };
        }
    }
    let mut tau = min_tau;
    while tau < max_tau {
        let c = cmnd.get(tau).copied().unwrap_or(1.0);
        if c < threshold {
            // Walk to the local minimum.
            while tau + 1 < max_tau && cmnd.get(tau + 1).copied().unwrap_or(1.0) < cmnd.get(tau).copied().unwrap_or(1.0) {
                tau += 1;
            }
            // Parabolic interpolation.
            let (a, b, c) =
                (cmnd.get(tau - 1).copied().unwrap_or(1.0), cmnd.get(tau).copied().unwrap_or(1.0), cmnd.get(tau + 1).copied().unwrap_or(1.0));
            let denom = a - 2.0 * b + c;
            let shift = if denom.abs() > 1e-9 { 0.5 * (a - c) / denom } else { 0.0 };
            let t = tau as f32 + shift.clamp(-1.0, 1.0);
            return (t > 0.0).then_some(sample_rate / t);
        }
        tau += 1;
    }
    None
}

/// Convert mono audio to notes: frames of `hop` samples, pitch per frame, then runs of the same
/// semitone at least `min_frames` long become notes.
pub fn audio_to_notes(audio: &[f32], sample_rate: f32) -> Vec<DetectedNote> {
    let sr = if sample_rate.is_finite() && sample_rate > 0.0 { sample_rate } else { 48_000.0 };
    let win = ((sr * 0.046) as usize).clamp(256, 8192);
    let hop = win / 4;
    let min_frames = 3;
    let mut frames: Vec<(usize, Option<u8>, f32)> = Vec::new();
    let mut pos = 0usize;
    while pos + win <= audio.len() && frames.len() < 2_000_000 {
        let Some(frame) = audio.get(pos..pos + win) else { break };
        let rms = (frame.iter().map(|x| x * x).sum::<f32>() / win as f32).sqrt();
        let pitch = if rms > 0.01 {
            yin(frame, sr, 50.0, 1600.0, 0.15).map(|f| (69.0 + 12.0 * (f / 440.0).log2()).round().clamp(0.0, 127.0) as u8)
        } else {
            None
        };
        frames.push((pos, pitch, rms));
        pos += hop;
    }
    let mut notes = Vec::new();
    let mut i = 0;
    while i < frames.len() {
        let Some((start, Some(p), _)) = frames.get(i).copied() else {
            i += 1;
            continue;
        };
        let mut j = i;
        let mut peak = 0.0f32;
        while let Some((_, Some(q), rms)) = frames.get(j).copied() {
            if q != p {
                break;
            }
            peak = peak.max(rms);
            j += 1;
        }
        if j - i >= min_frames {
            let end = frames.get(j).map_or(audio.len(), |f| f.0);
            let velocity = (40.0 + 87.0 * (peak / 0.5).min(1.0)) as u8;
            notes.push(DetectedNote { start, end, pitch: p, velocity: velocity.clamp(1, 127) });
        }
        i = j.max(i + 1);
    }
    notes
}

/// Runs of the same per-frame value at least `min_frames` long become notes (`None` is a rest).
fn runs(frames: &[(usize, [Option<u8>; MAX_VOICES], f32)], len: usize, min_frames: usize) -> Vec<DetectedNote> {
    let mut notes = Vec::new();
    for pitch in 0u8..=127 {
        let on = |i: usize| frames.get(i).is_some_and(|f| f.1.contains(&Some(pitch)));
        let mut i = 0;
        while i < frames.len() {
            if !on(i) {
                i += 1;
                continue;
            }
            let mut j = i;
            let mut peak = 0.0f32;
            while on(j) {
                peak = peak.max(frames.get(j).map_or(0.0, |f| f.2));
                j += 1;
            }
            if j - i >= min_frames {
                let start = frames.get(i).map_or(0, |f| f.0);
                let end = frames.get(j).map_or(len, |f| f.0);
                let velocity = (40.0 + 87.0 * (peak / 0.5).min(1.0)) as u8;
                notes.push(DetectedNote { start, end, pitch, velocity: velocity.clamp(1, 127) });
            }
            i = j.max(i + 1);
        }
    }
    notes.sort_by_key(|n| (n.start, n.pitch));
    notes
}

/// Most notes heard at once by [`audio_to_chords`].
pub const MAX_VOICES: usize = 6;

/// Convert mono audio of chords to notes (Audio to MIDI › Harmony): each frame's spectrum is
/// scored per semitone from its first harmonics, the strongest semitones that are not overtones
/// of a stronger lower one are heard, and runs of a semitone become notes.
pub fn audio_to_chords(audio: &[f32], sample_rate: f32) -> Vec<DetectedNote> {
    let sr = if sample_rate.is_finite() && sample_rate > 0.0 { sample_rate } else { 48_000.0 };
    // 8192 points (about 170 ms at 48 kHz) tell semitones apart down to the bass.
    let n = 8192usize;
    let hop = n / 2;
    let mut frames: Vec<(usize, [Option<u8>; MAX_VOICES], f32)> = Vec::new();
    let mut pos = 0usize;
    while pos + n <= audio.len() && frames.len() < 500_000 {
        let Some(frame) = audio.get(pos..pos + n) else { break };
        let rms = (frame.iter().map(|x| x * x).sum::<f32>() / n as f32).sqrt();
        let mut heard = [None; MAX_VOICES];
        if rms > 0.01 {
            let mags: Vec<f32> = crate::spectrum::magnitude_db(frame, n).iter().map(|db| crate::db_to_gain(*db)).collect();
            let at = |hz: f32| mags.get((hz * n as f32 / sr).round() as usize).copied().unwrap_or(0.0);
            let loudest = mags.iter().copied().fold(0.0f32, f32::max);
            // Salience of MIDI notes 36..=96 from four harmonics; a note needs its own fundamental.
            let mut score = [0.0f32; 128];
            for p in 36u8..=96 {
                let f0 = 440.0 * 2f32.powf((f32::from(p) - 69.0) / 12.0);
                if at(f0) < loudest * 0.2 {
                    continue;
                }
                let s: f32 = (1..=4).map(|h| at(f0 * h as f32) / h as f32).sum();
                if let Some(x) = score.get_mut(usize::from(p)) {
                    *x = s;
                }
            }
            let best = score.iter().copied().fold(0.0f32, f32::max);
            let mut order: Vec<u8> = (36u8..=96).filter(|p| score.get(usize::from(*p)).is_some_and(|s| *s > best * 0.35)).collect();
            order.sort_by(|a, b| score.get(usize::from(*b)).unwrap_or(&0.0).total_cmp(score.get(usize::from(*a)).unwrap_or(&0.0)));
            let mut k = 0;
            for p in order {
                // An octave, twelfth or two octaves above a note already heard is its overtone; so
                // is a semitone next to one (spectral leakage).
                let overtone = heard.iter().flatten().any(|q: &u8| {
                    let d = i16::from(p) - i16::from(*q);
                    matches!(d, 12 | 19 | 24 | 28 | 31 | -1 | 1)
                });
                if overtone {
                    continue;
                }
                if let Some(slot) = heard.get_mut(k) {
                    *slot = Some(p);
                    k += 1;
                } else {
                    break;
                }
            }
        }
        frames.push((pos, heard, rms));
        pos += hop;
    }
    runs(&frames, audio.len(), 2)
}

/// A drum hit's General MIDI note from the share of its energy in the lows and highs: kick (36)
/// for mostly lows, closed hat (42) for mostly highs, snare (38) otherwise.
pub fn drum_note(hit: &[f32], sample_rate: f32) -> u8 {
    let sr = if sample_rate.is_finite() && sample_rate > 0.0 { sample_rate } else { 48_000.0 };
    let n = 2048usize.min(hit.len().next_power_of_two()).max(256);
    let mags: Vec<f32> = crate::spectrum::magnitude_db(hit, n).iter().map(|db| crate::db_to_gain(*db).powi(2)).collect();
    let band = |lo: f32, hi: f32| -> f32 {
        mags.iter()
            .enumerate()
            .filter(|(k, _)| {
                let hz = *k as f32 * sr / n as f32;
                hz >= lo && hz < hi
            })
            .map(|(_, m)| *m)
            .sum()
    };
    let (low, mid, high) = (band(20.0, 180.0), band(180.0, 4000.0), band(4000.0, 20_000.0));
    let total = (low + mid + high).max(1e-12);
    if low / total > 0.5 {
        36
    } else if high / total > 0.5 {
        42
    } else {
        38
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(f: f32, secs: f32, sr: f32) -> Vec<f32> {
        (0..(secs * sr) as usize).map(|i| (i as f32 * f * std::f32::consts::TAU / sr).sin() * 0.5).collect()
    }

    #[test]
    fn yin_finds_a440() {
        let t = tone(440.0, 0.1, 48_000.0);
        let f = yin(&t[..2048], 48_000.0, 50.0, 1600.0, 0.15).unwrap();
        assert!((f - 440.0).abs() < 2.0, "{f}");
    }

    #[test]
    fn melody_to_notes() {
        let sr = 48_000.0;
        let mut a = tone(261.63, 0.4, sr);
        a.extend(vec![0.0; 4800]);
        a.extend(tone(392.0, 0.4, sr));
        let n = audio_to_notes(&a, sr);
        let pitches: Vec<u8> = n.iter().map(|x| x.pitch).collect();
        assert_eq!(pitches, vec![60, 67], "{n:?}");
    }

    #[test]
    fn chords_to_notes() {
        let sr = 48_000.0;
        let chord = |ps: &[f32], secs: f32| -> Vec<f32> {
            let mut v = vec![0.0f32; (secs * sr) as usize];
            for p in ps {
                let f = 440.0 * 2f32.powf((p - 69.0) / 12.0);
                for (i, x) in v.iter_mut().enumerate() {
                    *x += (i as f32 * f * std::f32::consts::TAU / sr).sin() * 0.2;
                }
            }
            v
        };
        let mut a = chord(&[60.0, 64.0, 67.0], 0.8);
        a.extend(chord(&[57.0, 60.0, 64.0], 0.8));
        let n = audio_to_chords(&a, sr);
        let at = |t: usize| -> Vec<u8> {
            let mut v: Vec<u8> = n.iter().filter(|x| x.start <= t && x.end > t).map(|x| x.pitch).collect();
            v.sort_unstable();
            v
        };
        assert_eq!(at(19_200), vec![60, 64, 67], "{n:?}");
        assert_eq!(at(57_600), vec![57, 60, 64], "{n:?}");
        assert!(audio_to_chords(&[], f32::NAN).is_empty());
    }

    #[test]
    fn drum_hits_are_told_apart() {
        let sr = 48_000.0;
        let kick: Vec<f32> = (0..4800).map(|i| (i as f32 * 60.0 * std::f32::consts::TAU / sr).sin() * (-(i as f32) / 1500.0).exp()).collect();
        let mut seed = 1u32;
        let mut noise = || {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (seed >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0
        };
        let raw: Vec<f32> = (0..4800).map(|_| noise()).collect();
        // Differencing twice tilts white noise to the highs: a hat.
        let hat: Vec<f32> = raw.windows(3).map(|w| (w[2] - 2.0 * w[1] + w[0]) * 0.25).collect();
        let snare: Vec<f32> = (0..4800).map(|i| (i as f32 * 200.0 * std::f32::consts::TAU / sr).sin() * 0.6 + raw[i] * 0.1).collect();
        assert_eq!(drum_note(&kick, sr), 36);
        assert_eq!(drum_note(&hat, sr), 42);
        assert_eq!(drum_note(&snare, sr), 38);
        assert_eq!(drum_note(&[], f32::NAN), 38);
    }

    #[test]
    fn silence_and_garbage() {
        assert!(audio_to_notes(&vec![0.0; 48_000], 48_000.0).is_empty());
        assert!(audio_to_notes(&[], f32::NAN).is_empty());
        assert!(yin(&[0.0; 10], 48_000.0, 50.0, 1600.0, 0.15).is_none());
    }

    #[test]
    fn huge_finite_rate_is_unvoiced() {
        // #241: the shortest lag saturates to usize::MAX; that is "no pitch", not an overflow.
        assert!(yin(&[0.0; 64], 1.0e30, 50.0, 1600.0, 0.15).is_none());
        assert!(yin(&[0.5; 8192], f32::MAX, 50.0, 1600.0, 0.15).is_none());
    }

    #[test]
    fn audio_to_notes_survives_huge_finite_rate() {
        assert!(audio_to_notes(&[0.5; 8192], 1.0e30).is_empty());
    }
}
