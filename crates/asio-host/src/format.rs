//! The safe half of the driver host: the sample formats drivers ask for, writing the mix into
//! them, and the buffer sizes a driver offers. Plain Rust, built and tested on every platform.

/// Byte order of a driver's samples.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Order {
    Little,
    Big,
}

/// How a driver stores one sample. The integer formats hold `bits` significant bits in a
/// `bytes`-wide container (for example 24 bits in 4 bytes, right-aligned).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SampleFormat {
    Int { bytes: usize, bits: u32, order: Order },
    F32(Order),
    F64(Order),
}

impl SampleFormat {
    /// The format a driver reports by its numeric code (the codes every driver uses), or `None`
    /// for one Solara cannot write (DSD, or an unknown code).
    pub fn from_code(code: i32) -> Option<SampleFormat> {
        let order = if code >= 16 { Order::Little } else { Order::Big };
        let int = |bytes, bits| Some(SampleFormat::Int { bytes, bits, order });
        match code % 16 {
            _ if !(0..32).contains(&code) => None,
            0 => int(2, 16),
            1 => int(3, 24),
            2 => int(4, 32),
            3 => Some(SampleFormat::F32(order)),
            4 => Some(SampleFormat::F64(order)),
            8 => int(4, 16),
            9 => int(4, 18),
            10 => int(4, 20),
            11 => int(4, 24),
            _ => None,
        }
    }

    /// Bytes one sample takes.
    pub fn bytes(self) -> usize {
        match self {
            SampleFormat::Int { bytes, .. } => bytes,
            SampleFormat::F32(_) => 4,
            SampleFormat::F64(_) => 8,
        }
    }

    /// A short name for logs and the settings window ("32-bit int", "24-bit int", "32-bit float").
    pub fn label(self) -> String {
        match self {
            SampleFormat::Int { bits, .. } => format!("{bits}-bit int"),
            SampleFormat::F32(_) => "32-bit float".to_string(),
            SampleFormat::F64(_) => "64-bit float".to_string(),
        }
    }
}

/// Write one channel of an interleaved block (`channels` samples per frame, taking channel `ch`)
/// into a driver buffer in `format`. Samples are clipped to -1..1 and NaN becomes silence; frames
/// the buffer has no room for are dropped, and room left over is zeroed.
pub fn write_channel(src: &[f32], channels: usize, ch: usize, format: SampleFormat, dst: &mut [u8]) {
    let size = format.bytes();
    let mut frames = src.chunks(channels.max(1)).map(|f| f.get(ch).copied().unwrap_or(0.0));
    for slot in dst.chunks_exact_mut(size) {
        match frames.next() {
            Some(x) => encode(if x.is_finite() { x.clamp(-1.0, 1.0) } else { 0.0 }, format, slot),
            None => slot.fill(0),
        }
    }
}

fn put(bytes: &[u8], order: Order, slot: &mut [u8]) {
    match order {
        Order::Little => {
            for (d, s) in slot.iter_mut().zip(bytes) {
                *d = *s;
            }
        }
        Order::Big => {
            for (d, s) in slot.iter_mut().zip(bytes.iter().rev()) {
                *d = *s;
            }
        }
    }
}

fn encode(x: f32, format: SampleFormat, slot: &mut [u8]) {
    match format {
        SampleFormat::F32(order) => put(&x.to_le_bytes(), order, slot),
        SampleFormat::F64(order) => put(&f64::from(x).to_le_bytes(), order, slot),
        SampleFormat::Int { bytes, bits, order } => {
            let max = ((1i64 << (bits.clamp(8, 32) - 1)) - 1) as f64;
            let v = (f64::from(x) * max).round() as i64;
            // The low `bytes` bytes of the value, little-endian first, then reordered as asked.
            let le = v.to_le_bytes();
            let used = le.get(..bytes.min(8)).unwrap_or(&[]);
            put(used, order, slot);
        }
    }
}

/// The buffer sizes a driver offers: `min` to `max` in `step`s (`step` -1 means powers of two
/// from `min`; 0 means `min` alone, or `min` and `max` when they differ). Out-of-range or
/// nonsensical values give `preferred` alone. A long list (a driver that allows every size)
/// keeps only the sizes people pick: powers of two, the smallest, the largest and the preferred.
pub fn buffer_sizes(min: i32, max: i32, preferred: i32, step: i32) -> Vec<u32> {
    const LIMIT: u32 = 1 << 16;
    let fallback = || u32::try_from(preferred).ok().filter(|p| (1..=LIMIT).contains(p)).into_iter().collect::<Vec<_>>();
    let (Ok(lo), Ok(hi)) = (u32::try_from(min), u32::try_from(max)) else { return fallback() };
    if lo == 0 || hi < lo || hi > LIMIT {
        return fallback();
    }
    let mut out: Vec<u32> = match step {
        -1 => std::iter::successors(Some(lo), |n| n.checked_mul(2)).take_while(|n| *n <= hi).collect(),
        0 => {
            if lo == hi {
                vec![lo]
            } else {
                vec![lo, hi]
            }
        }
        s if s > 0 => {
            let s = s.unsigned_abs();
            std::iter::successors(Some(lo), |n| n.checked_add(s)).take_while(|n| *n <= hi).collect()
        }
        _ => return fallback(),
    };
    if out.len() > 32 {
        let keep = |n: &u32| n.is_power_of_two() || *n == lo || *n == hi || i64::from(*n) == i64::from(preferred);
        out.retain(keep);
    }
    if let Ok(p) = u32::try_from(preferred)
        && (lo..=hi).contains(&p)
        && !out.contains(&p)
    {
        out.push(p);
        out.sort_unstable();
    }
    out
}

/// The size to open: `want` when the driver offers it, else the driver's preferred size, else
/// the smallest it offers.
pub fn choose_buffer(want: Option<u32>, offered: &[u32], preferred: i32) -> Option<u32> {
    want.filter(|w| offered.contains(w))
        .or_else(|| u32::try_from(preferred).ok().filter(|p| offered.contains(p)))
        .or_else(|| offered.first().copied())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_follow_the_driver_codes() {
        assert_eq!(SampleFormat::from_code(18), Some(SampleFormat::Int { bytes: 4, bits: 32, order: Order::Little }));
        assert_eq!(SampleFormat::from_code(17), Some(SampleFormat::Int { bytes: 3, bits: 24, order: Order::Little }));
        assert_eq!(SampleFormat::from_code(19), Some(SampleFormat::F32(Order::Little)));
        assert_eq!(SampleFormat::from_code(27), Some(SampleFormat::Int { bytes: 4, bits: 24, order: Order::Little }));
        assert_eq!(SampleFormat::from_code(0), Some(SampleFormat::Int { bytes: 2, bits: 16, order: Order::Big }));
        for dsd in [32, 33, 40, -1, 5, 21, 1000] {
            assert_eq!(SampleFormat::from_code(dsd), None, "{dsd}");
        }
    }

    #[test]
    fn samples_are_written_per_channel_in_the_drivers_format() {
        let block = [1.0f32, -1.0, 0.5, 0.0, f32::NAN, 2.0];
        let mut out = [0xAAu8; 12];
        write_channel(&block, 2, 0, SampleFormat::Int { bytes: 4, bits: 32, order: Order::Little }, &mut out);
        assert_eq!(i32::from_le_bytes([out[0], out[1], out[2], out[3]]), i32::MAX);
        assert_eq!(i32::from_le_bytes([out[4], out[5], out[6], out[7]]), (0.5 * f64::from(i32::MAX)).round() as i32);
        assert_eq!(&out[8..12], &[0, 0, 0, 0], "NaN is silence");
        let mut right = [0u8; 9];
        write_channel(&block, 2, 1, SampleFormat::Int { bytes: 3, bits: 24, order: Order::Little }, &mut right);
        let s24 = |b: &[u8]| (i32::from_le_bytes([0, b[0], b[1], b[2]])) >> 8;
        assert_eq!(s24(&right[0..3]), -8_388_607);
        assert_eq!(s24(&right[3..6]), 0);
        assert_eq!(s24(&right[6..9]), 8_388_607, "over full scale clips");
        let mut f = [0u8; 8];
        write_channel(&[0.25, 0.0, -0.25, 0.0], 2, 0, SampleFormat::F32(Order::Big), &mut f);
        assert_eq!(f32::from_be_bytes([f[0], f[1], f[2], f[3]]), 0.25);
        assert_eq!(f32::from_be_bytes([f[4], f[5], f[6], f[7]]), -0.25);
        // A buffer longer than the block is zeroed past it; a shorter one takes what fits.
        let mut long = [0xFFu8; 16];
        write_channel(&[0.0, 0.0], 1, 0, SampleFormat::F32(Order::Little), &mut long);
        assert!(long.iter().all(|b| *b == 0));
        let mut short = [0u8; 2];
        write_channel(&[1.0, 1.0, 1.0], 1, 0, SampleFormat::Int { bytes: 2, bits: 16, order: Order::Little }, &mut short);
        assert_eq!(i16::from_le_bytes(short), i16::MAX);
        // A missing channel is silence, not a panic.
        let mut none = [0xFFu8; 4];
        write_channel(&[1.0, 1.0], 2, 5, SampleFormat::F32(Order::Little), &mut none);
        assert_eq!(f32::from_le_bytes(none), 0.0);
    }

    #[test]
    fn buffer_sizes_follow_the_driver() {
        assert_eq!(buffer_sizes(32, 1024, 128, -1), [32, 64, 128, 256, 512, 1024]);
        assert_eq!(buffer_sizes(64, 64, 64, 0), [64]);
        assert_eq!(buffer_sizes(48, 480, 96, 48), [48, 96, 144, 192, 240, 288, 336, 384, 432, 480]);
        let every = buffer_sizes(16, 4096, 100, 1);
        assert!(every.contains(&128) && every.contains(&16) && every.contains(&4096) && every.contains(&100), "{every:?}");
        assert!(every.len() < 32, "{every:?}");
        assert_eq!(buffer_sizes(-5, 100, 64, 1), [64], "nonsense keeps the preferred size");
        assert_eq!(buffer_sizes(0, 0, 0, 0), Vec::<u32>::new());
        assert_eq!(buffer_sizes(64, 32, 128, 1), [128]);
        assert_eq!(buffer_sizes(64, 256, 100, -1), [64, 100, 128, 256], "the preferred size is always offered");
    }

    #[test]
    fn the_chosen_buffer_is_one_the_driver_offers() {
        let offered = [64, 128, 256, 512];
        assert_eq!(choose_buffer(Some(128), &offered, 256), Some(128));
        assert_eq!(choose_buffer(Some(100), &offered, 256), Some(256));
        assert_eq!(choose_buffer(None, &offered, 999), Some(64));
        assert_eq!(choose_buffer(Some(128), &[], -1), None);
    }
}
