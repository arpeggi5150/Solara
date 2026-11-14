//! Guitar Pro 6 scores (`.gpx`): a small file system of 4 KiB sectors ("BCFS"), usually packed
//! with a bit-level LZ scheme ("BCFZ"), holding `score.gpif`, the same XML the `.gp` reader reads.
//!
//! Written from public descriptions of the container, not from any other program's code.

/// Sector size of the file system.
const SECTOR: usize = 0x1000;
/// Most bytes an unpacked file system may take.
const MAX_UNPACKED: usize = 64 << 20;

/// Whether `bytes` start like a Guitar Pro 6 file.
pub(crate) fn sniff(bytes: &[u8]) -> bool {
    bytes.starts_with(b"BCFZ") || bytes.starts_with(b"BCFS")
}

/// The text of `score.gpif` inside a `.gpx` file.
pub(crate) fn score_gpif(bytes: &[u8]) -> Result<String, String> {
    let fs = if let Some(packed) = bytes.strip_prefix(b"BCFZ") { unpack(packed)? } else { bytes.to_vec() };
    let files = files(fs.strip_prefix(b"BCFS").ok_or("not a Guitar Pro 6 file system")?)?;
    let gpif = files.iter().find(|(n, _)| n.eq_ignore_ascii_case("score.gpif")).map(|(_, d)| d).ok_or("the file holds no score.gpif")?;
    let gpif = gpif.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(gpif);
    String::from_utf8(gpif.to_vec()).map_err(|_| "score.gpif is not UTF-8 text".to_string())
}

/// Reads bits, the highest of each byte first.
struct Bits<'a> {
    data: &'a [u8],
    bit: usize,
}

impl Bits<'_> {
    fn one(&mut self) -> Option<u32> {
        let byte = *self.data.get(self.bit / 8)?;
        let b = (byte >> (7 - self.bit % 8)) & 1;
        self.bit += 1;
        Some(u32::from(b))
    }
    /// `n` bits, the first read the highest.
    fn high_first(&mut self, n: u32) -> Option<u32> {
        let mut v = 0;
        for i in (0..n).rev() {
            v |= self.one()? << i;
        }
        Some(v)
    }
    /// `n` bits, the first read the lowest.
    fn low_first(&mut self, n: u32) -> Option<u32> {
        let mut v = 0;
        for i in 0..n {
            v |= self.one()? << i;
        }
        Some(v)
    }
}

/// Unpack a BCFZ stream (after its tag): a 32-bit unpacked length, then chunks that are either
/// up to three literal bytes or a copy of earlier output.
fn unpack(packed: &[u8]) -> Result<Vec<u8>, String> {
    let len = packed.get(..4).and_then(|b| <[u8; 4]>::try_from(b).ok()).map(i32::from_le_bytes).ok_or("the file ends early")?;
    let len = usize::try_from(len).ok().filter(|l| *l <= MAX_UNPACKED).ok_or("the packed score is too large")?;
    let mut out: Vec<u8> = Vec::with_capacity(len.min(1 << 20));
    let mut bits = Bits { data: packed.get(4..).unwrap_or_default(), bit: 0 };
    while out.len() < len {
        let Some(flag) = bits.one() else { break };
        if flag == 1 {
            let Some(word) = bits.high_first(4) else { break };
            let (Some(offset), Some(size)) = (bits.low_first(word), bits.low_first(word)) else { break };
            let (offset, size) = (offset as usize, size as usize);
            if offset == 0 || offset > out.len() {
                return Err("the packed score refers back past its start".into());
            }
            let from = out.len() - offset;
            for i in 0..size.min(offset) {
                let b = out.get(from + i).copied().ok_or("the packed score is damaged")?;
                out.push(b);
            }
        } else {
            let Some(n) = bits.low_first(2) else { break };
            for _ in 0..n {
                let Some(b) = bits.high_first(8) else { break };
                out.push(u8::try_from(b).unwrap_or(0));
            }
        }
    }
    out.truncate(len);
    Ok(out)
}

fn i32_at(data: &[u8], at: usize) -> Option<i32> {
    data.get(at..at.checked_add(4)?).and_then(|b| <[u8; 4]>::try_from(b).ok()).map(i32::from_le_bytes)
}

/// The files of a BCFS file system (after its tag): sector 0 is a header; a sector starting with
/// 2 describes a file by its name, size and the sectors that hold it.
fn files(data: &[u8]) -> Result<Vec<(String, Vec<u8>)>, String> {
    let mut out = Vec::new();
    let mut at = SECTOR;
    while at + 3 < data.len() {
        if i32_at(data, at) == Some(2) {
            let name_bytes = data.get(at + 4..(at + 4 + 127).min(data.len())).unwrap_or_default();
            let name: String = name_bytes.iter().take_while(|b| **b != 0).map(|&b| char::from(b)).collect();
            let size = i32_at(data, at + 0x8C).and_then(|s| usize::try_from(s).ok()).unwrap_or(0).min(data.len());
            let mut body = Vec::with_capacity(size);
            let mut list = at + 0x94;
            // The sector list ends with 0, or where this sector ends.
            while list + 4 <= at + SECTOR && body.len() < size {
                let Some(block) = i32_at(data, list).and_then(|b| usize::try_from(b).ok()).filter(|b| *b != 0) else { break };
                let start = block.checked_mul(SECTOR).ok_or("a file names a sector past the end")?;
                let chunk = data.get(start..(start + SECTOR).min(data.len())).ok_or("a file names a sector past the end")?;
                body.extend_from_slice(chunk);
                list += 4;
            }
            body.truncate(size);
            out.push((name, body));
            if out.len() > 4096 {
                break;
            }
        }
        at += SECTOR;
    }
    Ok(out)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A BCFS image holding `files`, laid out as Guitar Pro 6 lays it out.
    pub(crate) fn file_system(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut data = vec![0u8; SECTOR]; // the header sector
        for (name, body) in files {
            let entry = data.len();
            data.resize(entry + SECTOR, 0);
            data[entry..entry + 4].copy_from_slice(&2i32.to_le_bytes());
            data[entry + 4..entry + 4 + name.len()].copy_from_slice(name.as_bytes());
            data[entry + 0x8C..entry + 0x90].copy_from_slice(&(body.len() as i32).to_le_bytes());
            for (k, chunk) in body.chunks(SECTOR).enumerate() {
                let block = data.len() / SECTOR;
                data[entry + 0x94 + 4 * k..entry + 0x98 + 4 * k].copy_from_slice(&(block as i32).to_le_bytes());
                let at = data.len();
                data.resize(at + SECTOR, 0);
                data[at..at + chunk.len()].copy_from_slice(chunk);
            }
        }
        let mut out = b"BCFS".to_vec();
        out.extend(data);
        out
    }

    struct BitWriter {
        out: Vec<u8>,
        bit: usize,
    }
    impl BitWriter {
        fn one(&mut self, b: u32) {
            if self.bit.is_multiple_of(8) {
                self.out.push(0);
            }
            if b != 0 {
                let last = self.out.len() - 1;
                self.out[last] |= 1 << (7 - self.bit % 8);
            }
            self.bit += 1;
        }
        fn high_first(&mut self, v: u32, n: u32) {
            for i in (0..n).rev() {
                self.one((v >> i) & 1);
            }
        }
        fn low_first(&mut self, v: u32, n: u32) {
            for i in 0..n {
                self.one((v >> i) & 1);
            }
        }
    }

    /// Pack `data` as BCFZ: runs repeated from just before as copies, the rest as literals.
    pub(crate) fn pack(data: &[u8]) -> Vec<u8> {
        let mut w = BitWriter { out: Vec::new(), bit: 0 };
        let mut i = 0;
        while i < data.len() {
            // A copy of the bytes `offset` back, when at least 4 repeat.
            let best = (1..=i.min(255))
                .map(|off| (off, (0..off.min(255)).take_while(|k| data.get(i + k) == data.get(i + k - off)).count()))
                .max_by_key(|x| x.1);
            if let Some((off, len)) = best.filter(|x| x.1 >= 4) {
                w.one(1);
                w.high_first(8, 4);
                w.low_first(off as u32, 8);
                w.low_first(len as u32, 8);
                i += len;
            } else {
                let n = (data.len() - i).min(3);
                w.one(0);
                w.low_first(n as u32, 2);
                for b in &data[i..i + n] {
                    w.high_first(u32::from(*b), 8);
                }
                i += n;
            }
        }
        let mut out = b"BCFZ".to_vec();
        out.extend((data.len() as i32).to_le_bytes());
        out.extend(w.out);
        out
    }

    #[test]
    fn packed_and_plain_file_systems_give_the_score_back() {
        let gpif = "<GPIF><Score><Title>Garden</Title></Score>".repeat(200) + "</GPIF>";
        let fs = file_system(&[("misc.xml", b"<x/>"), ("score.gpif", gpif.as_bytes())]);
        assert!(fs.len() > 3 * SECTOR, "the score spans more than one sector");
        assert_eq!(score_gpif(&fs).unwrap(), gpif);
        let packed = pack(&fs);
        assert!(packed.len() < fs.len() / 4, "repeats are packed as copies");
        assert!(sniff(&packed) && sniff(&fs));
        assert_eq!(score_gpif(&packed).unwrap(), gpif);
    }

    #[test]
    fn damaged_files_are_refused_not_a_crash() {
        let fs = file_system(&[("score.gpif", b"<GPIF/>")]);
        let packed = pack(&fs);
        for cut in 0..packed.len() {
            let _ = score_gpif(&packed[..cut]);
        }
        for cut in (0..fs.len()).step_by(97) {
            let _ = score_gpif(&fs[..cut]);
        }
        // A copy from before the start, and a sector past the end.
        let mut bad = b"BCFZ".to_vec();
        bad.extend(100i32.to_le_bytes());
        bad.extend([0b1100_0111, 0xFF, 0xFF]);
        assert!(score_gpif(&bad).is_err());
        let mut far = fs.clone();
        let entry = 4 + SECTOR + 0x94;
        far[entry..entry + 4].copy_from_slice(&1_000_000i32.to_le_bytes());
        assert!(score_gpif(&far).is_err());
        let mut huge = b"BCFZ".to_vec();
        huge.extend(i32::MAX.to_le_bytes());
        assert!(score_gpif(&huge).is_err(), "an unpacked size past the cap");
    }
}
