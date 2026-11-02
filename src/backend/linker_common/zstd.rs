//! Zstandard (RFC 8878) decoder for `ELFCOMPRESS_ZSTD` input sections.
//!
//! binutils (`--compress-debug-sections=zstd`, `objcopy`, `ld -r`) and clang
//! (`-gz=zstd`) emit these whenever they are built with libzstd, as the Debian,
//! Arch and Fedora toolchains are. GNU ld, lld and mold all accept them.
//!
//! Scope: everything a single-shot compressor can emit into an ELF section --
//! one or more frames (skippable frames are skipped), raw / RLE / compressed
//! blocks, Huffman-coded literals (one or four streams, including treeless
//! reuse), FSE-coded sequences in all four table modes, repeat offsets, and
//! the optional XXH64 content checksum, which is verified. Dictionaries are
//! rejected: a section has no way to name one.
//!
//! As in `inflate.rs`, input is untrusted: every malformed frame is an error,
//! never a panic, and the output is bounded by the declared section size.

type Res<T> = Result<T, String>;

fn err<T>(msg: &str) -> Res<T> {
    Err(format!("zstd: {msg}"))
}

// ── Bit readers ─────────────────────────────────────────────────────────────

/// Little-endian bit `offset .. offset + n` of `src` (n <= 64), zero beyond
/// the end of `src`.
#[inline(always)]
fn bits_le(src: &[u8], offset: usize, n: u32) -> u64 {
    if n == 0 {
        return 0;
    }
    // Fast path: one unaligned 8-byte load covers `n + offset % 8 <= 63`.
    if n <= 56
        && let Some(&word) = src.get(offset / 8..).and_then(<[u8]>::first_chunk::<8>)
    {
        let word = u64::from_le_bytes(word);
        return (word >> (offset % 8)) & ((1u64 << n) - 1);
    }
    let mut value = 0u64;
    let mut got = 0u32;
    let mut byte = offset / 8;
    let mut shift = (offset % 8) as u32;
    while got < n {
        let b = u64::from(src.get(byte).copied().unwrap_or(0)) >> shift;
        value |= b << got;
        got += 8 - shift;
        shift = 0;
        byte += 1;
    }
    if n == 64 {
        value
    } else {
        value & ((1u64 << n) - 1)
    }
}

/// A forward bit reader (FSE table descriptions).
struct Forward<'a> {
    src: &'a [u8],
    bit: usize,
}

impl Forward<'_> {
    fn read(&mut self, n: u32) -> Res<u32> {
        let value = bits_le(self.src, self.bit, n) as u32;
        self.bit += n as usize;
        if self.bit > self.src.len() * 8 {
            return err("table description is truncated");
        }
        Ok(value)
    }

    fn rewind(&mut self, n: u32) {
        self.bit -= n as usize;
    }

    fn bytes_used(&self) -> usize {
        self.bit.div_ceil(8)
    }
}

/// A backward bit reader: zstd entropy streams are written forwards and read
/// from the end, starting just below the highest set bit of the last byte.
/// Reading below bit 0 yields zeros; callers check for over-consumption.
struct Backward<'a> {
    src: &'a [u8],
    /// Bits not yet consumed; negative once the stream is over-read.
    offset: i64,
}

impl<'a> Backward<'a> {
    fn new(src: &'a [u8]) -> Res<Self> {
        let Some(&last) = src.last() else {
            return err("empty entropy stream");
        };
        if last == 0 {
            return err("entropy stream has no end marker");
        }
        let marker = 7 - last.leading_zeros() as i64;
        Ok(Backward {
            src,
            offset: (src.len() as i64 - 1) * 8 + marker,
        })
    }

    #[inline(always)]
    fn peek(&self, n: u32) -> u64 {
        let start = self.offset - i64::from(n);
        // Fast path first: a whole 8-byte window below the read position.
        if start >= 0
            && n <= 56
            && let Some(&word) = self
                .src
                .get(start as usize / 8..)
                .and_then(<[u8]>::first_chunk::<8>)
        {
            let word = u64::from_le_bytes(word);
            return (word >> (start % 8)) & ((1u64 << n) - 1);
        }
        if start >= 0 {
            bits_le(self.src, start as usize, n)
        } else if start > -i64::from(n) {
            let have = (i64::from(n) + start) as u32;
            bits_le(self.src, 0, have) << (-start)
        } else {
            0
        }
    }

    #[inline(always)]
    fn read(&mut self, n: u32) -> u64 {
        let value = self.peek(n);
        self.offset -= i64::from(n);
        value
    }

    #[inline]
    fn consume(&mut self, n: u32) {
        self.offset -= i64::from(n);
    }
}

// ── FSE ─────────────────────────────────────────────────────────────────────

#[derive(Clone, Default)]
struct Fse {
    log: u32,
    symbol: Vec<u8>,
    bits: Vec<u8>,
    base: Vec<u16>,
}

impl Fse {
    /// Build a decoding table from normalised counts (-1 = "less than one").
    fn from_counts(counts: &[i16], log: u32) -> Res<Self> {
        let size = 1usize << log;
        let mut symbol = vec![0u8; size];
        let mut next = vec![0u16; counts.len()];
        let mut high = size;
        for (s, &c) in counts.iter().enumerate() {
            if c == -1 {
                high -= 1;
                symbol[high] = s as u8;
                next[s] = 1;
            }
        }
        let step = (size >> 1) + (size >> 3) + 3;
        let mask = size - 1;
        let mut pos = 0usize;
        for (s, &c) in counts.iter().enumerate() {
            if c <= 0 {
                continue;
            }
            next[s] = c as u16;
            for _ in 0..c {
                symbol[pos] = s as u8;
                loop {
                    pos = (pos + step) & mask;
                    if pos < high {
                        break;
                    }
                }
            }
        }
        if pos != 0 {
            return err("FSE counts do not fill the table");
        }
        let mut bits = vec![0u8; size];
        let mut base = vec![0u16; size];
        for i in 0..size {
            let s = usize::from(symbol[i]);
            let state = next[s];
            next[s] += 1;
            let nb = log - (15 - state.leading_zeros());
            bits[i] = nb as u8;
            base[i] = ((u32::from(state) << nb) - size as u32) as u16;
        }
        Ok(Fse {
            log,
            symbol,
            bits,
            base,
        })
    }

    fn rle(sym: u8) -> Self {
        Fse {
            log: 0,
            symbol: vec![sym],
            bits: vec![0],
            base: vec![0],
        }
    }

    /// Read a table description (RFC 8878 §4.1.1); returns bytes consumed.
    fn read(src: &[u8], max_log: u32, max_symbol: usize) -> Res<(Self, usize)> {
        let mut r = Forward { src, bit: 0 };
        let log = r.read(4)? + 5;
        if log > max_log {
            return err("FSE accuracy log too large");
        }
        let mut remaining: i32 = 1 << log;
        let mut counts: Vec<i16> = Vec::with_capacity(max_symbol + 1);
        while remaining > 0 {
            if counts.len() > max_symbol {
                return err("FSE description has too many symbols");
            }
            let nbits = 32 - ((remaining + 1) as u32).leading_zeros();
            let mut value = r.read(nbits)?;
            let low_mask = (1u32 << (nbits - 1)) - 1;
            let threshold = (1u32 << nbits) - 1 - (remaining as u32 + 1);
            if value & low_mask < threshold {
                r.rewind(1);
                value &= low_mask;
            } else if value > low_mask {
                value -= threshold;
            }
            let count = value as i32 - 1;
            remaining -= count.abs();
            counts.push(count as i16);
            if count == 0 {
                loop {
                    let repeat = r.read(2)?;
                    counts.resize(counts.len() + repeat as usize, 0);
                    if repeat != 3 {
                        break;
                    }
                }
            }
        }
        if remaining != 0 || counts.len() > max_symbol + 1 {
            return err("corrupt FSE description");
        }
        Ok((Fse::from_counts(&counts, log)?, r.bytes_used()))
    }

    #[inline]
    fn init(&self, s: &mut Backward<'_>) -> usize {
        s.read(self.log) as usize
    }

    #[inline]
    fn peek(&self, state: usize) -> u8 {
        self.symbol[state]
    }

    #[inline(always)]
    fn update(&self, state: usize, s: &mut Backward<'_>) -> usize {
        usize::from(self.base[state]) + s.read(u32::from(self.bits[state])) as usize
    }
}

// ── Huffman literals ────────────────────────────────────────────────────────

#[derive(Clone)]
struct Huffman {
    max_bits: u32,
    symbol: Vec<u8>,
    bits: Vec<u8>,
}

impl Huffman {
    /// Read a tree description (§4.2.1); returns bytes consumed.
    fn read(src: &[u8]) -> Res<(Self, usize)> {
        let Some(&header) = src.first() else {
            return err("missing Huffman tree description");
        };
        let mut weights: Vec<u8> = Vec::with_capacity(256);
        let used;
        if header < 128 {
            let size = usize::from(header);
            let Some(stream) = src.get(1..1 + size) else {
                return err("Huffman weights are truncated");
            };
            let (fse, table_len) = Fse::read(stream, 6, 255)?;
            let Some(body) = stream.get(table_len..) else {
                return err("Huffman weights are truncated");
            };
            let mut s = Backward::new(body)?;
            let mut s1 = fse.init(&mut s);
            let mut s2 = fse.init(&mut s);
            loop {
                if weights.len() >= 255 {
                    return err("too many Huffman weights");
                }
                weights.push(fse.peek(s1));
                s1 = fse.update(s1, &mut s);
                if s.offset < 0 {
                    weights.push(fse.peek(s2));
                    break;
                }
                weights.push(fse.peek(s2));
                s2 = fse.update(s2, &mut s);
                if s.offset < 0 {
                    weights.push(fse.peek(s1));
                    break;
                }
            }
            used = 1 + size;
        } else {
            let count = usize::from(header) - 127;
            let size = count.div_ceil(2);
            let Some(packed) = src.get(1..1 + size) else {
                return err("Huffman weights are truncated");
            };
            for i in 0..count {
                let byte = packed[i / 2];
                weights.push(if i % 2 == 0 { byte >> 4 } else { byte & 15 });
            }
            used = 1 + size;
        }
        if weights.len() > 255 {
            return err("too many Huffman weights");
        }
        let mut sum = 0u32;
        for &w in &weights {
            if w > 11 {
                return err("Huffman weight too large");
            }
            if w > 0 {
                sum += 1 << (w - 1);
            }
        }
        if sum == 0 {
            return err("Huffman weights are all zero");
        }
        let max_bits = 32 - sum.leading_zeros();
        if max_bits > 11 {
            return err("Huffman code too long");
        }
        let left = (1u32 << max_bits) - sum;
        if !left.is_power_of_two() {
            return err("Huffman weights do not form a complete code");
        }
        weights.push((left.trailing_zeros() + 1) as u8);

        let lengths: Vec<u32> = weights
            .iter()
            .map(|&w| {
                if w > 0 {
                    max_bits + 1 - u32::from(w)
                } else {
                    0
                }
            })
            .collect();
        let mut rank_count = [0u32; 13];
        for &l in &lengths {
            rank_count[l as usize] += 1;
        }
        let size = 1usize << max_bits;
        let mut rank_start = [0usize; 13];
        let mut bits = vec![0u8; size];
        let mut start = 0usize;
        for len in (1..=max_bits).rev() {
            rank_start[len as usize] = start;
            let end = start + rank_count[len as usize] as usize * (1 << (max_bits - len));
            bits[start..end].fill(len as u8);
            start = end;
        }
        if start != size {
            return err("Huffman table is incomplete");
        }
        let mut symbol = vec![0u8; size];
        for (sym, &len) in lengths.iter().enumerate() {
            if len > 0 {
                let span = 1usize << (max_bits - len);
                let at = rank_start[len as usize];
                symbol[at..at + span].fill(sym as u8);
                rank_start[len as usize] += span;
            }
        }
        Ok((
            Huffman {
                max_bits,
                symbol,
                bits,
            },
            used,
        ))
    }

    fn decode_stream(&self, src: &[u8], out: &mut Vec<u8>, count: usize) -> Res<()> {
        let mut s = Backward::new(src)?;
        let mask = (1u64 << self.max_bits) - 1;
        let mut left = count;
        // Fast loop: while at least 64 bits remain, load the lowest 8-byte
        // window whose top lies at or above the read position -- it holds
        // 57..=64 unread bits, since the start byte rounds *up* -- and decode
        // every symbol whose `max_bits` peek lies inside it (>= 4 per load).
        // `offset <= 8 * len - 1`, so the window never passes the end.
        while left > 0 && s.offset >= 64 {
            let byte = ((s.offset - 57) / 8) as usize;
            // Never short (see above); a short window would only fall
            // back to the exact per-symbol loop below.
            let Some(&word) = src.get(byte..).and_then(<[u8]>::first_chunk::<8>) else {
                break;
            };
            let word = u64::from_le_bytes(word);
            let mut avail = (s.offset - byte as i64 * 8) as u32;
            while left > 0 && avail >= self.max_bits {
                let index = ((word >> (avail - self.max_bits)) & mask) as usize;
                out.push(self.symbol[index]);
                avail -= u32::from(self.bits[index]);
                left -= 1;
            }
            s.offset = byte as i64 * 8 + i64::from(avail);
        }
        for _ in 0..left {
            let index = s.peek(self.max_bits) as usize;
            out.push(self.symbol[index]);
            s.consume(u32::from(self.bits[index]));
        }
        if s.offset != 0 {
            return err("Huffman stream length mismatch");
        }
        Ok(())
    }
}

// ── Sequences ───────────────────────────────────────────────────────────────

const LL_BASE: [u32; 36] = [
    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 18, 20, 22, 24, 28, 32, 40, 48, 64,
    128, 256, 512, 1024, 2048, 4096, 8192, 16384, 32768, 65536,
];
const LL_BITS: [u8; 36] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 3, 3, 4, 6, 7, 8, 9, 10, 11,
    12, 13, 14, 15, 16,
];
const ML_BASE: [u32; 53] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27,
    28, 29, 30, 31, 32, 33, 34, 35, 37, 39, 41, 43, 47, 51, 59, 67, 83, 99, 131, 259, 515, 1027,
    2051, 4099, 8195, 16387, 32771, 65539,
];
const ML_BITS: [u8; 53] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    1, 1, 1, 1, 2, 2, 3, 3, 4, 4, 5, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16,
];
const LL_DEFAULT: [i16; 36] = [
    4, 3, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 1, 1, 1, 2, 2, 2, 2, 2, 2, 2, 2, 2, 3, 2, 1, 1, 1, 1, 1,
    -1, -1, -1, -1,
];
const ML_DEFAULT: [i16; 53] = [
    1, 4, 3, 2, 2, 2, 2, 2, 2, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, -1, -1, -1, -1, -1, -1, -1,
];
const OF_DEFAULT: [i16; 29] = [
    1, 1, 1, 1, 1, 1, 2, 2, 2, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, -1, -1, -1, -1, -1,
];

/// Per-frame state that compressed blocks may reuse ("repeat" modes).
struct FrameState {
    huffman: Option<Huffman>,
    ll: Option<Fse>,
    of: Option<Fse>,
    ml: Option<Fse>,
    rep: [usize; 3],
}

#[derive(Clone, Copy)]
enum Kind {
    Ll,
    Of,
    Ml,
}

impl Kind {
    fn limits(self) -> (u32, usize) {
        match self {
            Kind::Ll => (9, 35),
            Kind::Of => (8, 31),
            Kind::Ml => (9, 52),
        }
    }

    fn predefined(self) -> Res<Fse> {
        match self {
            Kind::Ll => Fse::from_counts(&LL_DEFAULT, 6),
            Kind::Of => Fse::from_counts(&OF_DEFAULT, 5),
            Kind::Ml => Fse::from_counts(&ML_DEFAULT, 6),
        }
    }
}

/// Resolve one table from its 2-bit mode; returns bytes consumed.
fn table(mode: u8, kind: Kind, src: &[u8], slot: &mut Option<Fse>) -> Res<usize> {
    let (max_log, max_symbol) = kind.limits();
    let (fse, used) = match mode {
        0 => (kind.predefined()?, 0),
        1 => {
            let Some(&sym) = src.first() else {
                return err("RLE table is truncated");
            };
            if usize::from(sym) > max_symbol {
                return err("RLE table symbol out of range");
            }
            (Fse::rle(sym), 1)
        }
        2 => Fse::read(src, max_log, max_symbol)?,
        _ => {
            if slot.is_none() {
                return err("repeat table mode with no previous table");
            }
            return Ok(0);
        }
    };
    if fse.symbol.iter().any(|&s| usize::from(s) > max_symbol) {
        return err("table symbol out of range");
    }
    *slot = Some(fse);
    Ok(used)
}

fn literals(block: &[u8], st: &mut FrameState) -> Res<(Vec<u8>, usize)> {
    let Some(&b0) = block.first() else {
        return err("missing literals section");
    };
    let kind = b0 & 3;
    let format = (b0 >> 2) & 3;
    let byte = |i: usize| -> Res<usize> {
        block
            .get(i)
            .map(|&b| usize::from(b))
            .ok_or_else(|| "zstd: literals header is truncated".to_string())
    };
    if kind < 2 {
        let (size, header) = match format {
            0 | 2 => (usize::from(b0 >> 3), 1),
            1 => ((usize::from(b0) >> 4) + (byte(1)? << 4), 2),
            _ => (
                (usize::from(b0) >> 4) + (byte(1)? << 4) + (byte(2)? << 12),
                3,
            ),
        };
        if size > 128 * 1024 {
            return err("literals exceed the maximum block size");
        }
        if kind == 0 {
            let Some(raw) = block.get(header..header + size) else {
                return err("raw literals are truncated");
            };
            return Ok((raw.to_vec(), header + size));
        }
        return Ok((vec![byte(header)? as u8; size], header + 1));
    }
    let (header, field) = match format {
        0 | 1 => (3, 10),
        2 => (4, 14),
        _ => (5, 18),
    };
    let mut combined = 0u64;
    for i in 0..header {
        combined |= (byte(i)? as u64) << (8 * i);
    }
    let mask = (1u64 << field) - 1;
    let regen = ((combined >> 4) & mask) as usize;
    let comp = ((combined >> (4 + field)) & mask) as usize;
    let streams = if format == 0 { 1 } else { 4 };
    if regen > 128 * 1024 {
        return err("literals exceed the maximum block size");
    }
    let Some(mut data) = block.get(header..header + comp) else {
        return err("compressed literals are truncated");
    };
    if kind == 2 {
        let (huffman, used) = Huffman::read(data)?;
        st.huffman = Some(huffman);
        data = &data[used..];
    }
    let Some(huffman) = st.huffman.as_ref() else {
        return err("treeless literals with no previous Huffman table");
    };
    let mut out = Vec::with_capacity(regen);
    if streams == 1 {
        huffman.decode_stream(data, &mut out, regen)?;
    } else {
        if data.len() < 6 {
            return err("literals jump table is truncated");
        }
        let jump = |i: usize| usize::from(u16::from_le_bytes([data[2 * i], data[2 * i + 1]]));
        let (l1, l2, l3) = (jump(0), jump(1), jump(2));
        let body = &data[6..];
        let Some(l4) = body.len().checked_sub(l1 + l2 + l3) else {
            return err("literals jump table exceeds the section");
        };
        let seg = regen.div_ceil(4);
        let Some(last) = regen.checked_sub(3 * seg) else {
            return err("too few literals for four streams");
        };
        let mut at = 0;
        for (len, count) in [(l1, seg), (l2, seg), (l3, seg), (l4, last)] {
            huffman.decode_stream(&body[at..at + len], &mut out, count)?;
            at += len;
        }
    }
    Ok((out, header + comp))
}

/// Execute a sequences section, appending to `out`. Matches may not reach
/// below `frame_start`: zstd frames are independent.
fn sequences(
    src: &[u8],
    lits: &[u8],
    out: &mut Vec<u8>,
    frame_start: usize,
    limit: usize,
    st: &mut FrameState,
) -> Res<()> {
    let byte = |i: usize| -> Res<usize> {
        src.get(i)
            .map(|&b| usize::from(b))
            .ok_or_else(|| "zstd: sequences header is truncated".to_string())
    };
    let b0 = byte(0)?;
    let (count, mut at) = match b0 {
        0 => (0, 1),
        1..=127 => (b0, 1),
        128..=254 => (((b0 - 128) << 8) + byte(1)?, 2),
        _ => (byte(1)? + (byte(2)? << 8) + 0x7f00, 3),
    };
    if count == 0 {
        if at != src.len() {
            return err("trailing bytes after an empty sequences section");
        }
        if out.len() + lits.len() > limit {
            return err("output exceeds the declared size");
        }
        out.extend_from_slice(lits);
        return Ok(());
    }
    let modes = byte(at)? as u8;
    at += 1;
    if modes & 3 != 0 {
        return err("reserved bits set in sequence compression modes");
    }
    at += table(modes >> 6, Kind::Ll, &src[at..], &mut st.ll)?;
    at += table((modes >> 4) & 3, Kind::Of, &src[at..], &mut st.of)?;
    at += table((modes >> 2) & 3, Kind::Ml, &src[at..], &mut st.ml)?;
    let (Some(ll), Some(of), Some(ml)) = (&st.ll, &st.of, &st.ml) else {
        return err("missing sequence table");
    };
    let Some(stream) = src.get(at..) else {
        return err("sequences bitstream is missing");
    };
    let mut s = Backward::new(stream)?;
    let mut ll_state = ll.init(&mut s);
    let mut of_state = of.init(&mut s);
    let mut ml_state = ml.init(&mut s);
    let mut lit_at = 0usize;
    for n in 0..count {
        let of_code = u32::from(of.peek(of_state));
        let ml_code = usize::from(ml.peek(ml_state));
        let ll_code = usize::from(ll.peek(ll_state));
        if of_code > 31 {
            return err("offset code out of range");
        }
        let of_value = (1u64 << of_code) + s.read(of_code);
        let match_len = ML_BASE[ml_code] as usize + s.read(u32::from(ML_BITS[ml_code])) as usize;
        let lit_len = LL_BASE[ll_code] as usize + s.read(u32::from(LL_BITS[ll_code])) as usize;
        if n + 1 != count {
            ll_state = ll.update(ll_state, &mut s);
            ml_state = ml.update(ml_state, &mut s);
            of_state = of.update(of_state, &mut s);
        }

        let rep = &mut st.rep;
        let offset = if of_value > 3 {
            let offset = (of_value - 3) as usize;
            *rep = [offset, rep[0], rep[1]];
            offset
        } else {
            let index = of_value as usize + usize::from(lit_len == 0);
            match index {
                1 => rep[0],
                2 => {
                    *rep = [rep[1], rep[0], rep[2]];
                    rep[0]
                }
                3 => {
                    *rep = [rep[2], rep[0], rep[1]];
                    rep[0]
                }
                _ => {
                    let offset = rep[0] - 1;
                    if offset == 0 {
                        return err("repeat offset underflow");
                    }
                    *rep = [offset, rep[0], rep[1]];
                    offset
                }
            }
        };

        let Some(chunk) = lits.get(lit_at..lit_at + lit_len) else {
            return err("sequence uses more literals than were decoded");
        };
        if out.len() + lit_len + match_len > limit {
            return err("output exceeds the declared size");
        }
        out.extend_from_slice(chunk);
        lit_at += lit_len;
        if offset == 0 || offset > out.len() - frame_start {
            return err("match offset reaches before the start of the output");
        }
        super::inflate::copy_match(out, offset, match_len);
    }
    if s.offset != 0 {
        return err("sequences bitstream length mismatch");
    }
    let rest = &lits[lit_at..];
    if out.len() + rest.len() > limit {
        return err("output exceeds the declared size");
    }
    out.extend_from_slice(rest);
    Ok(())
}

// ── Frames ──────────────────────────────────────────────────────────────────

fn xxh64(data: &[u8]) -> u64 {
    const P1: u64 = 0x9E37_79B1_85EB_CA87;
    const P2: u64 = 0xC2B2_AE3D_27D4_EB4F;
    const P3: u64 = 0x1656_67B1_9E37_79F9;
    const P4: u64 = 0x85EB_CA77_C2B2_AE63;
    const P5: u64 = 0x27D4_EB2F_1656_67C5;
    let round = |acc: u64, lane: u64| {
        acc.wrapping_add(lane.wrapping_mul(P2))
            .rotate_left(31)
            .wrapping_mul(P1)
    };
    let mut rest = data;
    let mut h = if data.len() >= 32 {
        let mut v = [P1.wrapping_add(P2), P2, 0, 0u64.wrapping_sub(P1)];
        while let Some((stripe, tail)) = rest.split_first_chunk::<32>() {
            let (words, _) = stripe.as_chunks::<8>();
            for (lane, &word) in v.iter_mut().zip(words) {
                *lane = round(*lane, u64::from_le_bytes(word));
            }
            rest = tail;
        }
        let mut h = v[0]
            .rotate_left(1)
            .wrapping_add(v[1].rotate_left(7))
            .wrapping_add(v[2].rotate_left(12))
            .wrapping_add(v[3].rotate_left(18));
        for lane in v {
            h = (h ^ round(0, lane)).wrapping_mul(P1).wrapping_add(P4);
        }
        h
    } else {
        P5
    };
    h = h.wrapping_add(data.len() as u64);
    while let Some((&word, tail)) = rest.split_first_chunk::<8>() {
        h ^= round(0, u64::from_le_bytes(word));
        h = h.rotate_left(27).wrapping_mul(P1).wrapping_add(P4);
        rest = tail;
    }
    if let Some((&word, tail)) = rest.split_first_chunk::<4>() {
        h ^= u64::from(u32::from_le_bytes(word)).wrapping_mul(P1);
        h = h.rotate_left(23).wrapping_mul(P2).wrapping_add(P3);
        rest = tail;
    }
    for &b in rest {
        h ^= u64::from(b).wrapping_mul(P5);
        h = h.rotate_left(11).wrapping_mul(P1);
    }
    h ^= h >> 33;
    h = h.wrapping_mul(P2);
    h ^= h >> 29;
    h = h.wrapping_mul(P3);
    h ^ (h >> 32)
}

/// Decode one frame starting at `src[0]`, appending to `out`; returns the
/// number of input bytes consumed.
fn frame(src: &[u8], out: &mut Vec<u8>, limit: usize) -> Res<usize> {
    let le = |at: usize, n: usize| -> Res<u64> {
        let Some(b) = src.get(at..at + n) else {
            return err("frame header is truncated");
        };
        Ok(b.iter().rev().fold(0u64, |v, &x| (v << 8) | u64::from(x)))
    };
    let magic = le(0, 4)? as u32;
    if magic & 0xffff_fff0 == 0x184d_2a50 {
        let len = le(4, 4)? as usize;
        return if src.len() - 8 >= len {
            Ok(8 + len)
        } else {
            err("skippable frame is truncated")
        };
    }
    if magic != 0xfd2f_b528 {
        return err("bad frame magic");
    }
    let fhd = le(4, 1)? as u8;
    if fhd & 0x08 != 0 {
        return err("reserved frame header bit set");
    }
    let single = fhd & 0x20 != 0;
    let checksum = fhd & 0x04 != 0;
    let mut at = 5 + usize::from(!single);
    let dict_len = [0, 1, 2, 4][usize::from(fhd & 3)];
    if le(at, dict_len)? != 0 {
        return err("frames that need a dictionary are not supported in ELF sections");
    }
    at += dict_len;
    let fcs_len = match fhd >> 6 {
        0 => usize::from(single),
        1 => 2,
        2 => 4,
        _ => 8,
    };
    let declared = match fcs_len {
        0 => None,
        2 => Some(le(at, 2)? + 256),
        n => Some(le(at, n)?),
    };
    at += fcs_len;

    let begin = out.len();
    let mut st = FrameState {
        huffman: None,
        ll: None,
        of: None,
        ml: None,
        rep: [1, 4, 8],
    };
    loop {
        let header = le(at, 3)? as u32;
        at += 3;
        let last = header & 1 != 0;
        let size = (header >> 3) as usize;
        if size > 128 * 1024 {
            return err("block exceeds the maximum block size");
        }
        match (header >> 1) & 3 {
            0 => {
                let Some(raw) = src.get(at..at + size) else {
                    return err("raw block is truncated");
                };
                if out.len() + size > limit {
                    return err("output exceeds the declared size");
                }
                out.extend_from_slice(raw);
                at += size;
            }
            1 => {
                let Some(&b) = src.get(at) else {
                    return err("RLE block is truncated");
                };
                if out.len() + size > limit {
                    return err("output exceeds the declared size");
                }
                out.resize(out.len() + size, b);
                at += 1;
            }
            2 => {
                let Some(block) = src.get(at..at + size) else {
                    return err("compressed block is truncated");
                };
                let (lits, used) = literals(block, &mut st)?;
                sequences(&block[used..], &lits, out, begin, limit, &mut st)?;
                at += size;
            }
            _ => return err("reserved block type"),
        }
        if last {
            break;
        }
    }
    if let Some(declared) = declared
        && declared != (out.len() - begin) as u64
    {
        return err("frame content size does not match the decoded size");
    }
    if checksum {
        let want = le(at, 4)? as u32;
        if xxh64(&out[begin..]) as u32 != want {
            return err("content checksum mismatch");
        }
        at += 4;
    }
    Ok(at)
}

/// Decode the zstd frames in `src`, which must produce exactly `expected`
/// bytes.
pub fn decompress(src: &[u8], expected: usize) -> Result<Vec<u8>, String> {
    // Bound the reservation: the declared size is input-controlled.
    let mut out = Vec::with_capacity(expected.min(src.len().saturating_mul(64)));
    let mut at = 0;
    while at < src.len() {
        at += frame(&src[at..], &mut out, expected)?;
    }
    if at == 0 {
        return err("empty stream");
    }
    if out.len() != expected {
        return Err(format!(
            "zstd: decompressed {} bytes, but the header declares {expected}",
            out.len()
        ));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 300 skewed pseudo-random letters followed by a repeat of their first
    /// 100: the frame (zstd 1.5 `-19 -C`) has a compressed block with
    /// four-stream Huffman literals, FSE sequences and a content checksum.
    fn skewed_input() -> Vec<u8> {
        let alpha = b"eeeeeeettttaaaooiinnsshrdlu";
        let mut x: u32 = 1;
        let head: Vec<u8> = (0..300)
            .map(|_| {
                x = x.wrapping_mul(1_103_515_245).wrapping_add(12345) & 0x7fff_ffff;
                alpha[(x >> 16) as usize % alpha.len()]
            })
            .collect();
        let mut all = head.clone();
        all.extend_from_slice(&head[..100]);
        all
    }

    const FRAME: [u8; 169] = [
        0x28, 0xb5, 0x2f, 0xfd, 0x04, 0x68, 0xe5, 0x04, 0x00, 0xd6, 0x92, 0x24, 0x0d, 0xd0, 0xa5,
        0x03, 0x72, 0x50, 0xb7, 0x51, 0x73, 0x78, 0x0b, 0x00, 0x50, 0x44, 0x20, 0x00, 0x1f, 0x00,
        0x1e, 0x00, 0xa0, 0x54, 0x4f, 0x44, 0x0b, 0xec, 0xad, 0xb7, 0x33, 0x40, 0xf9, 0xaa, 0xfd,
        0x2b, 0x15, 0xcd, 0x3e, 0xb2, 0xe8, 0x8a, 0x7a, 0x70, 0x0b, 0x4c, 0x17, 0xcd, 0x84, 0x74,
        0xf5, 0x1e, 0x2a, 0x03, 0x4f, 0x3c, 0xfa, 0xdb, 0xd3, 0x5d, 0xfc, 0x9c, 0xa9, 0x1d, 0xae,
        0x24, 0x0e, 0x42, 0xae, 0xb5, 0x4e, 0x49, 0x6c, 0xf5, 0x4c, 0x9f, 0x21, 0xfc, 0xab, 0xde,
        0x70, 0x3f, 0x27, 0x0d, 0x01, 0x77, 0x87, 0x70, 0x29, 0x13, 0xaa, 0xf8, 0xe9, 0xae, 0xd9,
        0xf3, 0x7b, 0xf1, 0x22, 0xdd, 0x78, 0x20, 0x7f, 0x5c, 0x21, 0x43, 0xca, 0xce, 0xd3, 0x96,
        0xf7, 0x9c, 0x4f, 0xdf, 0x04, 0xe4, 0xb0, 0x93, 0xe2, 0x88, 0xa9, 0xba, 0xe4, 0x15, 0x89,
        0xc1, 0x41, 0x83, 0x4f, 0x5d, 0x87, 0x28, 0x10, 0x84, 0x8a, 0x5b, 0x31, 0xab, 0x01, 0xb6,
        0x37, 0xcf, 0xda, 0x40, 0xae, 0xd9, 0x4c, 0x06, 0x01, 0x00, 0x2d, 0xe0, 0x25, 0x5d, 0x53,
        0x85, 0x92, 0xf0, 0xbb,
    ];

    #[test]
    fn a_compressed_frame_decodes_and_is_verified() {
        // Literals header: compressed (type 2), size format 1 = four streams.
        assert_eq!(FRAME[9] & 3, 2);
        assert_eq!((FRAME[9] >> 2) & 3, 1);
        assert_eq!(decompress(&FRAME, 400).unwrap(), skewed_input());
        // Two frames back to back decode to the concatenation.
        let two = [FRAME, FRAME].concat();
        assert_eq!(
            decompress(&two, 800).unwrap(),
            [skewed_input(), skewed_input()].concat()
        );
    }

    #[test]
    fn malformed_frames_are_errors() {
        assert!(decompress(&FRAME, 399).is_err());
        assert!(decompress(&FRAME, 401).is_err());
        assert!(decompress(&FRAME[..100], 400).is_err());
        assert!(decompress(&[], 0).is_err());
        let mut bad = FRAME;
        bad[168] ^= 0x10;
        assert!(decompress(&bad, 400).unwrap_err().contains("checksum"));
        // Every single-byte truncation and every bit flip in the frame body
        // must be rejected or decode to the same bytes, never panic.
        let want = skewed_input();
        for cut in 0..FRAME.len() {
            assert!(decompress(&FRAME[..cut], 400).is_err());
        }
        for at in 0..FRAME.len() {
            for bit in 0..8 {
                let mut flipped = FRAME;
                flipped[at] ^= 1 << bit;
                if let Ok(out) = decompress(&flipped, 400) {
                    assert_eq!(out, want, "flip at {at}:{bit} decoded to wrong data");
                }
            }
        }
    }

    #[test]
    fn xxh64_matches_the_reference_vectors() {
        assert_eq!(xxh64(b""), 0xEF46_DB37_51D8_E999);
        assert_eq!(xxh64(b"a"), 0xD24E_C4F1_A98C_6E5B);
        assert_eq!(xxh64(b"abc"), 0x44BC_2CF5_AD77_0999);
    }
}
