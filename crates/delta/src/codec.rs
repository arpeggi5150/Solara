//! Payload bytes: deflate around a bsdiff patch or a whole file.

use std::io::{Read, Write};

use flate2::Compression;
use flate2::read::DeflateDecoder;
use flate2::write::DeflateEncoder;

pub(crate) fn deflate(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let mut enc = DeflateEncoder::new(Vec::new(), Compression::best());
    enc.write_all(bytes).map_err(|e| format!("couldn't compress ({e})"))?;
    enc.finish().map_err(|e| format!("couldn't compress ({e})"))
}

/// Inflate `bytes`, refusing more than `max` bytes of output.
pub(crate) fn inflate(bytes: &[u8], max: u64) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    DeflateDecoder::new(bytes)
        .take(max.saturating_add(1))
        .read_to_end(&mut out)
        .map_err(|e| format!("the payload is not valid deflate data ({e})"))?;
    if out.len() as u64 > max {
        return Err(format!("the payload expands past {max} bytes"));
    }
    Ok(out)
}

/// A deflated bsdiff patch turning `old` into `new`.
pub(crate) fn make_patch(old: &[u8], new: &[u8]) -> Result<Vec<u8>, String> {
    let mut raw = Vec::new();
    bsdiff::diff(old, new, &mut raw).map_err(|e| format!("couldn't diff ({e})"))?;
    deflate(&raw)
}

/// Apply a deflated bsdiff patch to `old`. The output may not exceed `max` bytes.
pub(crate) fn apply_patch(old: &[u8], payload: &[u8], max: u64) -> Result<Vec<u8>, String> {
    // A raw patch is the new file plus 24 bytes of control per block: twice the output and a
    // margin bounds any patch our own `make_patch` writes.
    let raw = inflate(payload, max.saturating_mul(2).saturating_add(1 << 20))?;
    let mut out = Vec::new();
    bsdiff::patch(old, &mut raw.as_slice(), &mut out).map_err(|e| format!("the patch doesn't apply ({e})"))?;
    if out.len() as u64 > max {
        return Err(format!("the patch builds a file larger than {max} bytes"));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_patch_round_trips_and_is_small_for_a_small_change() {
        let old: Vec<u8> = (0..200_000u32).flat_map(|i| (i.wrapping_mul(2_654_435_761)).to_le_bytes()).collect();
        let mut new = old.clone();
        new[1000..1010].copy_from_slice(b"0123456789");
        new.extend_from_slice(b"tail");
        let p = make_patch(&old, &new).unwrap();
        assert!(p.len() < 4_000, "patch is {} bytes", p.len());
        assert_eq!(apply_patch(&old, &p, new.len() as u64).unwrap(), new);
        assert!(apply_patch(&old, &p, 10).is_err());
        assert!(apply_patch(&old, b"garbage", 1 << 20).is_err());
    }

    #[test]
    fn inflate_is_bounded() {
        let z = deflate(&[0u8; 10_000]).unwrap();
        assert_eq!(inflate(&z, 10_000).unwrap().len(), 10_000);
        assert!(inflate(&z, 9_999).is_err());
    }
}
