//! zlib (RFC 1950) / DEFLATE (RFC 1951) decoder for compressed ELF input
//! sections.
//!
//! GAS compresses DWARF sections by default when binutils is configured with
//! `--enable-compressed-debug-sections` (the GAS 2.47 oracle toolchain is):
//! `SHF_COMPRESSED` + `ELFCOMPRESS_ZLIB`, or the legacy `.zdebug_*` form. GNU
//! ld, lld and mold all decompress such input; a linker that does not sees a
//! section whose header size is the *compressed* size while its relocations
//! address the uncompressed bytes.
//!
//! The decoder is the canonical-code design of zlib's reference `puff`:
//! Huffman codes are decoded from per-length counts, which needs no tables
//! beyond the 288 + 32 symbol arrays and is fast enough for debug sections.
//! Every malformed stream is an error, never a panic, and the output is bounded
//! by the caller-supplied expected size (an input-controlled value).

const MAX_BITS: usize = 15;

/// Decoder errors are static strings so `Result<u16, Error>` stays in
/// registers on the per-symbol path; the public entry points add context.
type Error = &'static str;

struct Bits<'a> {
    data: &'a [u8],
    pos: usize,
    buf: u64,
    count: u32,
}

impl<'a> Bits<'a> {
    fn new(data: &'a [u8]) -> Self {
        Bits {
            data,
            pos: 0,
            buf: 0,
            count: 0,
        }
    }

    /// Top the buffer up to at least 56 bits where input remains.
    ///
    /// The fast path ORs in a whole unaligned 8-byte word but only accounts
    /// for the bytes that fit; the bits above `count` are then a copy of the
    /// next input bytes, so re-ORing them on the next refill is idempotent
    /// (the classic "lookahead" refill). Every consumer masks to `count`.
    #[inline]
    fn refill(&mut self) {
        if let Some(&word) = self.data.get(self.pos..).and_then(<[u8]>::first_chunk::<8>) {
            self.buf |= u64::from_le_bytes(word) << self.count;
            let take = (63 - self.count) / 8;
            self.pos += take as usize;
            self.count += take * 8;
        } else {
            while self.count <= 56 {
                let Some(&byte) = self.data.get(self.pos) else {
                    break;
                };
                self.buf |= u64::from(byte) << self.count;
                self.pos += 1;
                self.count += 8;
            }
        }
    }

    #[inline]
    fn need(&mut self, n: u32) -> Result<(), Error> {
        if self.count >= n {
            return Ok(());
        }
        self.refill();
        if self.count < n {
            return Err("compressed stream is truncated");
        }
        Ok(())
    }

    #[inline]
    fn bits(&mut self, n: u32) -> Result<u32, Error> {
        if n == 0 {
            return Ok(0);
        }
        self.need(n)?;
        let value = (self.buf & ((1u64 << n) - 1)) as u32;
        self.buf >>= n;
        self.count -= n;
        Ok(value)
    }

    /// Drop the bits of the current partial byte (stored-block alignment).
    fn align(&mut self) {
        let drop = self.count % 8;
        self.buf >>= drop;
        self.count -= drop;
    }

    /// The next `n` bytes after alignment, consuming any buffered whole bytes.
    fn bytes(&mut self, n: usize) -> Result<&'a [u8], Error> {
        debug_assert_eq!(self.count % 8, 0);
        // Return buffered bytes to the stream: they were read ahead.
        self.pos -= (self.count / 8) as usize;
        self.buf = 0;
        self.count = 0;
        let end = self
            .pos
            .checked_add(n)
            .filter(|&end| end <= self.data.len())
            .ok_or("compressed stream is truncated")?;
        let out = &self.data[self.pos..end];
        self.pos = end;
        Ok(out)
    }
}

/// Codes up to this length decode with one table lookup; longer ones (rare:
/// DEFLATE assigns them to the least frequent symbols) take the canonical
/// walk. 9 bits covers every fixed-Huffman literal and keeps the table at
/// 1 KiB, cheap to rebuild for each dynamic block.
const FAST_BITS: u32 = 9;

/// A canonical Huffman code: `count[len]` codes of each length and the
/// symbols in code order, plus a direct lookup table for short codes.
struct Huffman {
    count: [u16; MAX_BITS + 1],
    symbol: Vec<u16>,
    /// Indexed by the next `FAST_BITS` stream bits: `symbol | len << 9`, or
    /// 0 when the code is longer than `FAST_BITS`.
    fast: [u16; 1 << FAST_BITS],
}

impl Huffman {
    fn new(lengths: &[u8]) -> Result<Self, Error> {
        let mut count = [0u16; MAX_BITS + 1];
        for &len in lengths {
            count[len as usize] += 1;
        }
        // Over-subscribed codes are invalid; incomplete ones are allowed (a
        // single distance code is legal) and fail only if an unused code is hit.
        let mut left: i32 = 1;
        for &c in &count[1..] {
            left = (left << 1) - i32::from(c);
            if left < 0 {
                return Err("over-subscribed Huffman code");
            }
        }
        let mut offs = [0u16; MAX_BITS + 1];
        for len in 1..MAX_BITS {
            offs[len + 1] = offs[len] + count[len];
        }
        let mut symbol = vec![0u16; lengths.len()];
        for (sym, &len) in lengths.iter().enumerate() {
            if len != 0 {
                symbol[offs[len as usize] as usize] = sym as u16;
                offs[len as usize] += 1;
            }
        }
        count[0] = 0;

        // Canonical codes in symbol order; DEFLATE packs codes MSB-first into
        // an LSB-first stream, so the table index is the bit-reversed code.
        let mut fast = [0u16; 1 << FAST_BITS];
        let mut next_code = [0u32; MAX_BITS + 2];
        let mut code = 0u32;
        for len in 1..=MAX_BITS {
            code = (code + u32::from(count[len - 1])) << 1;
            next_code[len] = code;
        }
        for (sym, &len) in lengths.iter().enumerate() {
            let len = u32::from(len);
            if len == 0 || len > FAST_BITS {
                continue;
            }
            let code = next_code[len as usize];
            next_code[len as usize] += 1;
            let reversed = code.reverse_bits() >> (32 - len);
            let entry = sym as u16 | (len as u16) << 9;
            let mut index = reversed as usize;
            while index < fast.len() {
                fast[index] = entry;
                index += 1 << len;
            }
        }
        Ok(Huffman {
            count,
            symbol,
            fast,
        })
    }

    #[inline(always)]
    fn decode(&self, bits: &mut Bits<'_>) -> Result<u16, Error> {
        if bits.count < MAX_BITS as u32 {
            bits.refill();
        }
        let entry = self.fast[(bits.buf & ((1 << FAST_BITS) - 1)) as usize];
        if entry != 0 {
            let len = u32::from(entry >> 9);
            if len > bits.count {
                return Err("compressed stream is truncated");
            }
            bits.buf >>= len;
            bits.count -= len;
            return Ok(entry & 0x1ff);
        }
        self.decode_slow(bits)
    }

    #[cold]
    #[inline(never)]
    fn decode_slow(&self, bits: &mut Bits<'_>) -> Result<u16, Error> {
        let (mut code, mut first, mut index) = (0i32, 0i32, 0i32);
        for len in 1..=MAX_BITS {
            code |= bits.bits(1)? as i32;
            let count = i32::from(self.count[len]);
            if code - count < first {
                return Ok(self.symbol[(index + (code - first)) as usize]);
            }
            index += count;
            first = (first + count) << 1;
            code <<= 1;
        }
        Err("invalid Huffman code")
    }
}

const LENGTH_BASE: [u16; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131,
    163, 195, 227, 258,
];
const LENGTH_EXTRA: [u8; 29] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
];
const DIST_BASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
    2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
];
const DIST_EXTRA: [u8; 30] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13,
    13,
];

fn codes(
    bits: &mut Bits<'_>,
    out: &mut Vec<u8>,
    limit: usize,
    lit: &Huffman,
    dist: &Huffman,
) -> Result<(), Error> {
    loop {
        let sym = lit.decode(bits)?;
        match sym {
            0..=255 => {
                if out.len() == limit {
                    return Err("decompressed data exceeds the declared size");
                }
                out.push(sym as u8);
            }
            256 => return Ok(()),
            257..=285 => {
                let i = usize::from(sym - 257);
                let len =
                    usize::from(LENGTH_BASE[i]) + bits.bits(u32::from(LENGTH_EXTRA[i]))? as usize;
                let d = usize::from(dist.decode(bits)?);
                if d >= 30 {
                    return Err("invalid distance symbol");
                }
                let back =
                    usize::from(DIST_BASE[d]) + bits.bits(u32::from(DIST_EXTRA[d]))? as usize;
                if back > out.len() {
                    return Err("distance reaches before the start of the output");
                }
                if out.len() + len > limit {
                    return Err("decompressed data exceeds the declared size");
                }
                copy_match(out, back, len);
            }
            _ => return Err("invalid literal/length symbol"),
        }
    }
}

/// Append `len` bytes copied from `back` bytes before the end of `out`
/// (`1 <= back <= out.len()`). When the source overlaps the destination
/// (`back < len`) the result is a run of period `back`; copying in chunks no
/// longer than the bytes already written past `start` reads only final bytes,
/// so each chunk is a plain memcpy and the chunk size doubles.
pub(super) fn copy_match(out: &mut Vec<u8>, back: usize, len: usize) {
    let start = out.len() - back;
    let mut left = len;
    while left > 0 {
        let chunk = left.min(out.len() - start);
        out.extend_from_within(start..start + chunk);
        left -= chunk;
    }
}

fn fixed_tables() -> Result<(Huffman, Huffman), Error> {
    let mut lengths = [0u8; 288];
    lengths[..144].fill(8);
    lengths[144..256].fill(9);
    lengths[256..280].fill(7);
    lengths[280..].fill(8);
    Ok((Huffman::new(&lengths)?, Huffman::new(&[5u8; 30])?))
}

fn dynamic_tables(bits: &mut Bits<'_>) -> Result<(Huffman, Huffman), Error> {
    const ORDER: [usize; 19] = [
        16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
    ];
    let nlen = bits.bits(5)? as usize + 257;
    let ndist = bits.bits(5)? as usize + 1;
    let ncode = bits.bits(4)? as usize + 4;
    if nlen > 286 || ndist > 30 {
        return Err("bad dynamic block code counts");
    }
    let mut code_lengths = [0u8; 19];
    for &slot in &ORDER[..ncode] {
        code_lengths[slot] = bits.bits(3)? as u8;
    }
    let lencode = Huffman::new(&code_lengths)?;
    let mut lengths = vec![0u8; nlen + ndist];
    let mut i = 0;
    while i < nlen + ndist {
        let sym = lencode.decode(bits)?;
        let (value, repeat) = match sym {
            0..=15 => (sym as u8, 1),
            16 => {
                let Some(&prev) = i.checked_sub(1).and_then(|p| lengths.get(p)) else {
                    return Err("repeat with no previous length");
                };
                (prev, 3 + bits.bits(2)? as usize)
            }
            17 => (0, 3 + bits.bits(3)? as usize),
            18 => (0, 11 + bits.bits(7)? as usize),
            _ => return Err("invalid code-length symbol"),
        };
        if i + repeat > nlen + ndist {
            return Err("code lengths overrun the table");
        }
        lengths[i..i + repeat].fill(value);
        i += repeat;
    }
    if lengths[256] == 0 {
        return Err("dynamic block has no end-of-block code");
    }
    Ok((
        Huffman::new(&lengths[..nlen])?,
        Huffman::new(&lengths[nlen..])?,
    ))
}

/// Decode a raw DEFLATE stream of exactly `expected` bytes.
fn inflate_raw(bits: &mut Bits<'_>, expected: usize) -> Result<Vec<u8>, Error> {
    // The declared size is input-controlled: reserve no more than a sane
    // multiple of the compressed size up front (DEFLATE expands at most
    // 1032:1), and let the vector grow if the stream really is that large.
    let mut out = Vec::with_capacity(expected.min(bits.data.len().saturating_mul(1032)));
    loop {
        let last = bits.bits(1)? == 1;
        match bits.bits(2)? {
            0 => {
                bits.align();
                let header = bits.bytes(4)?;
                let len = u16::from_le_bytes([header[0], header[1]]);
                let nlen = u16::from_le_bytes([header[2], header[3]]);
                if len != !nlen {
                    return Err("stored block length check failed");
                }
                if out.len() + usize::from(len) > expected {
                    return Err("decompressed data exceeds the declared size");
                }
                out.extend_from_slice(bits.bytes(usize::from(len))?);
            }
            1 => {
                let (lit, dist) = fixed_tables()?;
                codes(bits, &mut out, expected, &lit, &dist)?;
            }
            2 => {
                let (lit, dist) = dynamic_tables(bits)?;
                codes(bits, &mut out, expected, &lit, &dist)?;
            }
            _ => return Err("invalid DEFLATE block type"),
        }
        if last {
            break;
        }
    }
    if out.len() != expected {
        return Err("decompressed size differs from the declared size");
    }
    Ok(out)
}

fn adler32(data: &[u8]) -> u32 {
    const MOD: u32 = 65521;
    let (mut a, mut b) = (1u32, 0u32);
    // 5552 is the largest run whose sums cannot overflow u32 (zlib's NMAX);
    // 5552 = 347 * 16. Within a 16-byte group, the per-byte recurrence
    // b += a_i telescopes to b += 16 a + sum((16 - i) * byte_i), a += sum(byte_i):
    // identical values at every group boundary, and vectorisable.
    for chunk in data.chunks(5552) {
        let mut groups = chunk.chunks_exact(16);
        for group in &mut groups {
            let (mut sum, mut weighted) = (0u32, 0u32);
            for (i, &byte) in group.iter().enumerate() {
                sum += u32::from(byte);
                weighted += (16 - i as u32) * u32::from(byte);
            }
            b += 16 * a + weighted;
            a += sum;
        }
        for &byte in groups.remainder() {
            a += u32::from(byte);
            b += a;
        }
        a %= MOD;
        b %= MOD;
    }
    (b << 16) | a
}

/// Decode a zlib stream (RFC 1950) that must produce exactly `expected`
/// bytes, verifying the header and the Adler-32 trailer.
pub fn zlib_decompress(data: &[u8], expected: usize) -> Result<Vec<u8>, String> {
    zlib_decompress_inner(data, expected).map_err(|e| format!("zlib: {e}"))
}

fn zlib_decompress_inner(data: &[u8], expected: usize) -> Result<Vec<u8>, Error> {
    let [cmf, flg, ..] = *data else {
        return Err("stream is truncated");
    };
    if cmf & 0x0f != 8 || cmf >> 4 > 7 {
        return Err("stream does not use DEFLATE with a <= 32 KiB window");
    }
    if (u16::from(cmf) << 8 | u16::from(flg)) % 31 != 0 {
        return Err("header check failed");
    }
    if flg & 0x20 != 0 {
        return Err("stream requires a preset dictionary");
    }
    let mut bits = Bits::new(&data[2..]);
    let out = inflate_raw(&mut bits, expected)?;
    bits.align();
    let trailer = bits.bytes(4)?;
    let want = u32::from_be_bytes([trailer[0], trailer[1], trailer[2], trailer[3]]);
    if adler32(&out) != want {
        return Err("Adler-32 checksum mismatch");
    }
    Ok(out)
}

/// The uncompressed form of a compressed input section.
#[derive(Debug)]
pub struct DecompressedSection {
    pub data: Vec<u8>,
    /// Alignment of the uncompressed data (`ch_addralign`); the section
    /// header's own `sh_addralign` describes the compression header.
    pub align: u64,
    /// The `.debug_*` name of a legacy `.zdebug_*` section.
    pub rename: Option<String>,
}

/// Decompress an input section if it is compressed, in either ELF form:
///
/// * gABI `SHF_COMPRESSED`: an `Elf64_Chdr` (24 bytes: type, reserved, size,
///   addralign) or `Elf32_Chdr` (12 bytes: type, size, addralign), then the
///   stream;
/// * legacy GNU `.zdebug_*` (`-gz=zlib-gnu`): `"ZLIB"`, the big-endian u64
///   uncompressed size, then a zlib stream.
///
/// Returns `Ok(None)` for an uncompressed section. The relocations of a
/// compressed section address its *uncompressed* bytes, so callers must apply
/// this before any relocation bounds check.
pub fn decompress_input_section(
    name: &str,
    flags: u64,
    raw: &[u8],
    class64: bool,
) -> Result<Option<DecompressedSection>, String> {
    use crate::backend::elf::{
        ELFCOMPRESS_ZLIB, ELFCOMPRESS_ZSTD, SHF_ALLOC, SHF_COMPRESSED, read_u32, read_u64,
    };

    if flags & SHF_COMPRESSED != 0 {
        if flags & SHF_ALLOC != 0 {
            return Err("SHF_COMPRESSED is not allowed on an SHF_ALLOC section".to_string());
        }
        let header = if class64 { 24 } else { 12 };
        let Some(chdr) = raw.get(..header) else {
            return Err("compressed section is smaller than its Chdr".to_string());
        };
        let (ch_type, size, align) = if class64 {
            (read_u32(chdr, 0), read_u64(chdr, 8), read_u64(chdr, 16))
        } else {
            let word = |off| u64::from(read_u32(chdr, off));
            (read_u32(chdr, 0), word(4), word(8))
        };
        if align > 1 && !align.is_power_of_two() {
            return Err(format!(
                "compressed section has invalid ch_addralign 0x{align:x}"
            ));
        }
        let size = usize::try_from(size)
            .map_err(|_| format!("ch_size 0x{size:x} does not fit in memory"))?;
        let payload = &raw[header..];
        let data = match ch_type {
            ELFCOMPRESS_ZLIB => zlib_decompress(payload, size)?,
            ELFCOMPRESS_ZSTD => super::zstd::decompress(payload, size)?,
            other => return Err(format!("unknown compression type (ch_type {other})")),
        };
        return Ok(Some(DecompressedSection {
            data,
            align,
            rename: None,
        }));
    }
    if let Some(suffix) = name.strip_prefix(".zdebug")
        && let Some(rest) = raw.strip_prefix(b"ZLIB")
    {
        let Some(&size) = rest.first_chunk::<8>() else {
            return Err("legacy .zdebug header is truncated".to_string());
        };
        let size = u64::from_be_bytes(size);
        let size = usize::try_from(size)
            .map_err(|_| format!("uncompressed size 0x{size:x} does not fit in memory"))?;
        return Ok(Some(DecompressedSection {
            data: zlib_decompress(&rest[8..], size)?,
            align: 1,
            rename: Some(format!(".debug{suffix}")),
        }));
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stored_fixed_and_dynamic_blocks_round_trip() {
        // Produced by CPython's zlib: level 0 (stored block) and level 9
        // (fixed-Huffman block).
        let stored = [
            0x78, 0x01, 0x01, 0x13, 0x00, 0xec, 0xff, 0x68, 0x65, 0x6c, 0x6c, 0x6f, 0x2c, 0x20,
            0x73, 0x74, 0x6f, 0x72, 0x65, 0x64, 0x20, 0x77, 0x6f, 0x72, 0x6c, 0x64, 0x47, 0x57,
            0x07, 0x3a,
        ];
        assert_eq!(
            zlib_decompress(&stored, 19).unwrap(),
            b"hello, stored world"
        );
        let fixed = [
            0x78, 0xda, 0x4b, 0x4c, 0x4a, 0x4e, 0x44, 0x42, 0x00, 0x2d, 0xf5, 0x05, 0xbf,
        ];
        assert_eq!(zlib_decompress(&fixed, 15).unwrap(), b"abcabcabcabcabc");

        // A dynamic-Huffman block: 150 skewed pseudo-random letters.
        let alpha = b"eeeeeeettttaaaooiinnsshrdlu";
        let mut x: u32 = 1;
        let want: Vec<u8> = (0..150)
            .map(|_| {
                x = x.wrapping_mul(1_103_515_245).wrapping_add(12345) & 0x7fff_ffff;
                alpha[(x >> 16) as usize % alpha.len()]
            })
            .collect();
        let dynamic = [
            0x78, 0xda, 0x15, 0x8d, 0x41, 0x0a, 0xc0, 0x40, 0x0c, 0x02, 0xdf, 0x2a, 0xec, 0xc0,
            0x06, 0x4a, 0x02, 0x89, 0xfd, 0x7f, 0xd3, 0x9b, 0xca, 0xa8, 0xe1, 0x4a, 0xc4, 0xc8,
            0xb7, 0x95, 0x35, 0x14, 0x67, 0xcc, 0x81, 0x69, 0x53, 0x13, 0x21, 0x1c, 0x2b, 0xca,
            0x02, 0x34, 0x03, 0x37, 0x8e, 0xd2, 0xc6, 0x12, 0x5a, 0xba, 0xef, 0x0e, 0x94, 0xcf,
            0xe3, 0xab, 0xf8, 0xa1, 0xc8, 0x0d, 0x16, 0x24, 0xee, 0x9b, 0x38, 0x73, 0xf7, 0xd5,
            0xe5, 0xb4, 0xc6, 0x8a, 0xb8, 0x0f, 0xeb, 0x16, 0xb3, 0x8b, 0xfc, 0x2b, 0xcd, 0x9e,
            0x6d, 0x73, 0x3f, 0xfc, 0xd2, 0x1f, 0x6a, 0x1e, 0x3e, 0x68,
        ];
        assert_eq!((dynamic[2] >> 1) & 3, 2, "vector is a dynamic block");
        assert_eq!(zlib_decompress(&dynamic, 150).unwrap(), want);
    }

    const FIXED: [u8; 13] = [
        0x78, 0xda, 0x4b, 0x4c, 0x4a, 0x4e, 0x44, 0x42, 0x00, 0x2d, 0xf5, 0x05, 0xbf,
    ];

    fn chdr64(ch_type: u32, size: u64, align: u64) -> Vec<u8> {
        let mut v = ch_type.to_le_bytes().to_vec();
        v.extend_from_slice(&[0; 4]);
        v.extend_from_slice(&size.to_le_bytes());
        v.extend_from_slice(&align.to_le_bytes());
        v
    }

    #[test]
    fn elf_compressed_sections_in_every_form() {
        use crate::backend::elf::{SHF_ALLOC, SHF_COMPRESSED};
        // ELFCLASS64: Elf64_Chdr, zlib.
        let sec = [chdr64(1, 15, 8), FIXED.to_vec()].concat();
        let d = decompress_input_section(".debug_info", SHF_COMPRESSED, &sec, true)
            .unwrap()
            .unwrap();
        assert_eq!(
            (d.data.as_slice(), d.align, d.rename),
            (&b"abcabcabcabcabc"[..], 8, None)
        );
        // ELFCLASS32: Elf32_Chdr is type, size, addralign (12 bytes).
        let mut sec32 = 1u32.to_le_bytes().to_vec();
        sec32.extend_from_slice(&15u32.to_le_bytes());
        sec32.extend_from_slice(&4u32.to_le_bytes());
        sec32.extend_from_slice(&FIXED);
        let d = decompress_input_section(".debug_line", SHF_COMPRESSED, &sec32, false)
            .unwrap()
            .unwrap();
        assert_eq!((d.data.len(), d.align), (15, 4));
        // Legacy .zdebug: "ZLIB", big-endian size, stream; renamed.
        let mut z = b"ZLIB".to_vec();
        z.extend_from_slice(&15u64.to_be_bytes());
        z.extend_from_slice(&FIXED);
        let d = decompress_input_section(".zdebug_str", 0, &z, true)
            .unwrap()
            .unwrap();
        assert_eq!(d.rename.as_deref(), Some(".debug_str"));
        // Uncompressed input is left alone, including a .zdebug section
        // without the magic.
        assert!(
            decompress_input_section(".debug_info", 0, &FIXED, true)
                .unwrap()
                .is_none()
        );
        assert!(
            decompress_input_section(".zdebug_info", 0, b"raw", true)
                .unwrap()
                .is_none()
        );
        // Rejections: SHF_ALLOC, unknown ch_type, bad alignment, short Chdr,
        // a size the stream does not produce.
        let flags = SHF_COMPRESSED | SHF_ALLOC;
        assert!(decompress_input_section(".text", flags, &sec, true).is_err());
        let unknown = [chdr64(7, 15, 1), FIXED.to_vec()].concat();
        let e = decompress_input_section(".debug_info", SHF_COMPRESSED, &unknown, true);
        assert!(e.unwrap_err().contains("ch_type 7"));
        let misaligned = [chdr64(1, 15, 3), FIXED.to_vec()].concat();
        assert!(
            decompress_input_section(".debug_info", SHF_COMPRESSED, &misaligned, true).is_err()
        );
        assert!(decompress_input_section(".debug_info", SHF_COMPRESSED, &sec[..20], true).is_err());
        let wrong = [chdr64(1, 16, 1), FIXED.to_vec()].concat();
        assert!(decompress_input_section(".debug_info", SHF_COMPRESSED, &wrong, true).is_err());
    }

    #[test]
    fn malformed_streams_are_errors() {
        let fixed = [
            0x78, 0xda, 0x4b, 0x4c, 0x4a, 0x4e, 0x44, 0x42, 0x00, 0x2d, 0xf5, 0x05, 0xbf,
        ];
        // Wrong declared size, either way.
        assert!(zlib_decompress(&fixed, 14).is_err());
        assert!(zlib_decompress(&fixed, 16).is_err());
        // Corrupt checksum, truncation, bad header, empty input.
        let mut bad = fixed;
        bad[12] ^= 1;
        assert!(zlib_decompress(&bad, 15).unwrap_err().contains("Adler-32"));
        assert!(zlib_decompress(&fixed[..8], 15).is_err());
        assert!(zlib_decompress(&[0x78, 0x00], 0).is_err());
        assert!(zlib_decompress(&[], 0).is_err());
        // A huge declared size does not allocate it up front.
        assert!(zlib_decompress(&fixed, usize::MAX / 2).is_err());
    }
}
