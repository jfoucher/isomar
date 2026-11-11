//! A Blosc 1 decoder for what the Copernicus Marine Zarr stores hold: LZ4 blocks, optionally byte
//! shuffled and split into one stream per byte of an element, and frames stored uncompressed.
//!
//! A frame is a 16-byte header (version, flags, element size, uncompressed size, block size,
//! compressed size), then the offset of each block, then the blocks, each compressed on its own. That is
//! what makes byte ranges worth it: in a time-major chunk the forecast hours are a run of blocks, which can
//! be fetched and decoded without the years of analysis before them.

use std::ops::Range;

#[derive(Debug, Clone, PartialEq)]
pub struct BloscError(pub String);

impl std::fmt::Display for BloscError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "blosc: {}", self.0)
    }
}

impl std::error::Error for BloscError {}

fn err<T>(s: &str) -> Result<T, BloscError> {
    Err(BloscError(s.to_string()))
}

const DOSHUFFLE: u8 = 0x01;
const MEMCPYED: u8 = 0x02;
const DOBITSHUFFLE: u8 = 0x04;
const DONT_SPLIT: u8 = 0x10;
/// The compressor code in the flags' top three bits.
const LZ4: u8 = 1;

pub const HEADER_LEN: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header {
    pub flags: u8,
    pub typesize: usize,
    /// Uncompressed size.
    pub nbytes: usize,
    pub blocksize: usize,
    /// The whole frame's size.
    pub cbytes: usize,
}

fn u32_at(b: &[u8], at: usize) -> Result<usize, BloscError> {
    b.get(at..at + 4).map(|x| u32::from_le_bytes([x[0], x[1], x[2], x[3]]) as usize).ok_or_else(|| BloscError("truncated".into()))
}

impl Header {
    pub fn parse(b: &[u8]) -> Result<Header, BloscError> {
        if b.len() < HEADER_LEN {
            return err("shorter than its header");
        }
        if b[0] != 2 {
            return err(&format!("format version {}", b[0]));
        }
        let h = Header { flags: b[2], typesize: b[3].max(1) as usize, nbytes: u32_at(b, 4)?, blocksize: u32_at(b, 8)?, cbytes: u32_at(b, 12)? };
        if h.flags & DOBITSHUFFLE != 0 {
            return err("bit shuffle is not supported");
        }
        if !h.memcpyed() && h.flags >> 5 != LZ4 {
            return err(&format!("compressor {} is not LZ4", h.flags >> 5));
        }
        if h.blocksize == 0 && h.nbytes > 0 {
            return err("no block size");
        }
        Ok(h)
    }

    pub fn memcpyed(&self) -> bool {
        self.flags & MEMCPYED != 0
    }

    pub fn nblocks(&self) -> usize {
        if self.blocksize == 0 { 0 } else { self.nbytes.div_ceil(self.blocksize) }
    }

    /// The bytes of the header and the block offsets after it: what to fetch first.
    pub fn index_len(&self) -> usize {
        if self.memcpyed() { HEADER_LEN } else { HEADER_LEN + 4 * self.nblocks() }
    }

    /// The uncompressed bytes of block `k`.
    pub fn block_range(&self, k: usize) -> Range<usize> {
        let start = k * self.blocksize;
        start..(start + self.blocksize).min(self.nbytes)
    }

    /// The blocks that hold the uncompressed bytes `r`.
    pub fn blocks_for(&self, r: Range<usize>) -> Range<usize> {
        if self.blocksize == 0 || r.is_empty() {
            return 0..0;
        }
        (r.start / self.blocksize)..r.end.min(self.nbytes).div_ceil(self.blocksize)
    }
}

/// The offsets of the blocks in the frame, read from its first `index_len` bytes.
pub fn block_offsets(b: &[u8], h: &Header) -> Result<Vec<usize>, BloscError> {
    (0..h.nblocks()).map(|k| u32_at(b, HEADER_LEN + 4 * k)).collect()
}

/// Where block `k` lies in the frame: from its offset to the next one after it (or the frame's end).
/// Blocks compressed by several threads are written in the order they finished, so that next one is not
/// always block `k + 1`'s.
pub fn compressed_range(h: &Header, offsets: &[usize], k: usize) -> Range<usize> {
    let start = offsets[k];
    let end = offsets.iter().copied().filter(|o| *o > start).min().unwrap_or(h.cbytes);
    start..end
}

/// The bytes of the frame that hold all of `blocks`: one range to fetch.
pub fn span(h: &Header, offsets: &[usize], blocks: Range<usize>) -> Range<usize> {
    let rs: Vec<Range<usize>> = blocks.filter(|k| *k < offsets.len()).map(|k| compressed_range(h, offsets, k)).collect();
    let start = rs.iter().map(|r| r.start).min().unwrap_or(0);
    let end = rs.iter().map(|r| r.end).max().unwrap_or(0);
    start..end
}

/// Decodes a whole frame.
pub fn decode(b: &[u8]) -> Result<Vec<u8>, BloscError> {
    let h = Header::parse(b)?;
    if h.memcpyed() {
        return b.get(HEADER_LEN..HEADER_LEN + h.nbytes).map(<[u8]>::to_vec).ok_or_else(|| BloscError("truncated".into()));
    }
    let offsets = block_offsets(b, &h)?;
    decode_blocks(&h, &offsets, b, 0, 0..h.nblocks())
}

/// Decodes the blocks `blocks` of a frame whose header is `h` and block offsets `offsets`, from `data`:
/// the frame's bytes from offset `base` on (all of it with `base` 0, or a range fetched for those
/// blocks). Returns the uncompressed bytes of those blocks, one after the other.
pub fn decode_blocks(h: &Header, offsets: &[usize], data: &[u8], base: usize, blocks: Range<usize>) -> Result<Vec<u8>, BloscError> {
    let mut out = Vec::with_capacity(blocks.len() * h.blocksize);
    for k in blocks {
        if k >= offsets.len() {
            return err("no such block");
        }
        let r = compressed_range(h, offsets, k);
        let (Some(s), Some(e)) = (r.start.checked_sub(base), r.end.checked_sub(base)) else { return err("block before the bytes fetched") };
        let src = data.get(s..e).ok_or_else(|| BloscError("block past the bytes fetched".into()))?;
        let size = h.block_range(k).len();
        let leftover = size < h.blocksize;
        out.extend(decode_block(h, src, size, leftover)?);
    }
    Ok(out)
}

fn decode_block(h: &Header, mut src: &[u8], size: usize, leftover: bool) -> Result<Vec<u8>, BloscError> {
    let ts = h.typesize;
    let splits = if h.flags & DONT_SPLIT == 0 && !leftover && ts > 1 && size.is_multiple_of(ts) { ts } else { 1 };
    let part = size / splits;
    let mut raw = Vec::with_capacity(size);
    for _ in 0..splits {
        let n = u32_at(src, 0)?;
        src = &src[4..];
        let chunk = src.get(..n).ok_or_else(|| BloscError("stream past its block".into()))?;
        if n == part {
            raw.extend_from_slice(chunk);
        } else {
            lz4_block(chunk, part, &mut raw)?;
        }
        src = &src[n..];
    }
    if raw.len() != size {
        return err("block of the wrong size");
    }
    if h.flags & DOSHUFFLE != 0 && ts > 1 {
        Ok(unshuffle(&raw, ts))
    } else {
        Ok(raw)
    }
}

/// Undoes a byte shuffle: element `i`'s byte `j` was moved to `j * n + i`. Bytes past the last whole
/// element stay where they are.
pub fn unshuffle(b: &[u8], ts: usize) -> Vec<u8> {
    let n = b.len() / ts;
    let mut out = vec![0u8; b.len()];
    for j in 0..ts {
        let plane = &b[j * n..(j + 1) * n];
        for (i, v) in plane.iter().enumerate() {
            out[i * ts + j] = *v;
        }
    }
    out[n * ts..].copy_from_slice(&b[n * ts..]);
    out
}

/// Decompresses one LZ4 block (the raw block format, no frame) of `size` bytes onto `out`.
pub fn lz4_block(src: &[u8], size: usize, out: &mut Vec<u8>) -> Result<(), BloscError> {
    let start = out.len();
    let mut i = 0;
    let length = |i: &mut usize, mut n: usize| -> Result<usize, BloscError> {
        if n == 15 {
            loop {
                let b = *src.get(*i).ok_or_else(|| BloscError("lz4: truncated length".into()))?;
                *i += 1;
                n += b as usize;
                if b != 255 {
                    break;
                }
            }
        }
        Ok(n)
    };
    while i < src.len() {
        let token = src[i];
        i += 1;
        let lit = length(&mut i, (token >> 4) as usize)?;
        let lits = src.get(i..i + lit).ok_or_else(|| BloscError("lz4: literals past the end".into()))?;
        out.extend_from_slice(lits);
        i += lit;
        if i >= src.len() {
            break;
        }
        let off = src.get(i..i + 2).map(|x| u16::from_le_bytes([x[0], x[1]]) as usize).ok_or_else(|| BloscError("lz4: truncated offset".into()))?;
        i += 2;
        let len = length(&mut i, (token & 15) as usize)? + 4;
        if off == 0 || off > out.len() - start {
            return err("lz4: bad offset");
        }
        if out.len() - start + len > size {
            return err("lz4: past the block");
        }
        let from = out.len() - off;
        for k in 0..len {
            let v = out[from + k];
            out.push(v);
        }
    }
    if out.len() - start != size {
        return err("lz4: wrong size");
    }
    Ok(())
}

/// Little-endian `f32`s.
pub fn f32s(b: &[u8]) -> Vec<f32> {
    b.as_chunks::<4>().0.iter().map(|x| f32::from_le_bytes(*x)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILL: f32 = 9.969_21e36;

    /// What the fixtures hold (`scripts/gen-blosc-fixtures.py`).
    fn expected(n: usize) -> Vec<f32> {
        (0..n).map(|i| if i % 97 == 0 { FILL } else { (((i * 37) % 1000) as f64 / 100.0 - 5.0) as f32 }).collect()
    }

    fn fixture(name: &str) -> Vec<u8> {
        std::fs::read(format!("{}/src/currents/fixtures/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap()
    }

    #[test]
    fn every_kind_of_frame_decodes_to_what_was_compressed() {
        for (name, n) in [("shuffle.blosc", 520 * 128), ("noshuffle.blosc", 520 * 128), ("single.blosc", 100), ("memcpy.blosc", 300)] {
            let got = f32s(&decode(&fixture(name)).unwrap());
            assert_eq!(got.len(), n, "{name}");
            assert!(got.iter().zip(expected(n)).all(|(a, b)| a.to_bits() == b.to_bits()), "{name}");
        }
    }

    #[test]
    fn a_run_of_blocks_decodes_from_its_byte_range_alone() {
        let frame = fixture("shuffle.blosc");
        let h = Header::parse(&frame).unwrap();
        assert_eq!((h.nblocks(), h.blocksize, h.typesize), (5, 65536, 4), "four whole blocks and a short one");
        let offsets = block_offsets(&frame[..h.index_len()], &h).unwrap();
        // The elements 40 000 to 66 560: the last three blocks, and the short one among them.
        let wanted = 40_000 * 4..66_560 * 4;
        let blocks = h.blocks_for(wanted.clone());
        assert_eq!(blocks, 2..5);
        let bytes = span(&h, &offsets, blocks.clone());
        let fetched = &frame[bytes.clone()];
        assert!(fetched.len() < frame.len(), "a part of the frame: {} of {}", fetched.len(), frame.len());
        let raw = decode_blocks(&h, &offsets, fetched, bytes.start, blocks.clone()).unwrap();
        let first = h.block_range(blocks.start).start;
        let got = f32s(&raw[wanted.start - first..wanted.end - first]);
        assert!(got.iter().zip(&expected(66_560)[40_000..]).all(|(a, b)| a.to_bits() == b.to_bits()));
    }

    #[test]
    fn broken_frames_are_errors_not_panics() {
        let frame = fixture("shuffle.blosc");
        for cut in [0, 10, 16, 30, 200, frame.len() - 1] {
            assert!(decode(&frame[..cut]).is_err(), "cut at {cut}");
        }
        let mut other = frame.clone();
        other[2] = (other[2] & 0x1f) | (3 << 5);
        assert!(decode(&other).is_err(), "zlib is not LZ4");
        let mut garbage = frame;
        for b in garbage.iter_mut().skip(40).step_by(7) {
            *b ^= 0x5a;
        }
        let _ = decode(&garbage);
    }
}
