//! `.note.gnu.build-id` generation (`--build-id[=sha1|md5|uuid|0xHEX|tree|fast|none]`).
//!
//! The build-id is a stable fingerprint of the linked image used by
//! debuggers (gdb's debuginfod lookup), core-dump matching, distro debuginfo
//! extraction, and the kernel's module/vmlinux verification.
//!
//! Supported GNU ld / LLD / mold styles:
//! - `sha1` / `tree` / `fast` (default for bare `--build-id`): 20-byte SHA-1
//!   digest (`descsz = 20`, total note = 36 bytes).
//! - `md5`: 16-byte RFC 1321 MD5 digest (`descsz = 16`, total note = 32 bytes).
//! - `uuid`: 16-byte RFC 4122 version-4 UUID (`descsz = 16`, total note = 32 bytes).
//! - `0xHEX`: explicit byte sequence (accepting optional `-` or `:` separators,
//!   padded to 4-byte ELF note alignment).
//! - `none` / `0`: omit `.note.gnu.build-id`.

/// Total size of the default SHA-1 build-id note (`12 + 4 + 20 = 36`).
pub const BUILD_ID_NOTE_SIZE: u64 = 12 + 4 + 20;

/// Parsed `--build-id` style.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum BuildIdStyle {
    #[default]
    None,
    /// 20-byte SHA-1 (`--build-id`, `--build-id=sha1`, `--build-id=tree`, `--build-id=fast`).
    Sha1,
    /// 16-byte MD5 (`--build-id=md5`).
    Md5,
    /// 16-byte random RFC 4122 v4 UUID (`--build-id=uuid`).
    Uuid,
    /// User-supplied hex byte string (`--build-id=0x...`).
    Hex(Vec<u8>),
}

impl BuildIdStyle {
    /// Parse `(build_id_enabled, build_id_style)` from `LinkerArgs`.
    pub fn parse(enabled: bool, raw_style: Option<&str>) -> Result<Self, String> {
        if !enabled {
            return Ok(Self::None);
        }
        let Some(style) = raw_style else {
            return Ok(Self::Sha1);
        };
        match style {
            "none" | "0" => Ok(Self::None),
            "" | "sha1" | "tree" | "fast" => Ok(Self::Sha1),
            "md5" => Ok(Self::Md5),
            "uuid" => Ok(Self::Uuid),
            s if s.starts_with("0x") || s.starts_with("0X") => {
                let hex_part = &s[2..];
                let digits: Vec<u8> = hex_part
                    .bytes()
                    .filter(|&b| b != b'-' && b != b':')
                    .collect();
                if digits.is_empty() {
                    return Err(format!(
                        "invalid --build-id hex string '{s}': expected hex digits after 0x"
                    ));
                }
                let mut bytes = Vec::with_capacity(digits.len().div_ceil(2));
                let mut idx = 0;
                if digits.len() % 2 == 1 {
                    let hi = hex_nibble(digits[0])
                        .ok_or_else(|| format!("invalid hex digit in --build-id='{s}'"))?;
                    bytes.push(hi);
                    idx = 1;
                }
                while idx < digits.len() {
                    let hi = hex_nibble(digits[idx])
                        .ok_or_else(|| format!("invalid hex digit in --build-id='{s}'"))?;
                    let lo = hex_nibble(digits[idx + 1])
                        .ok_or_else(|| format!("invalid hex digit in --build-id='{s}'"))?;
                    bytes.push((hi << 4) | lo);
                    idx += 2;
                }
                Ok(Self::Hex(bytes))
            }
            other => Err(format!(
                "unrecognized --build-id style '{other}' (expected sha1, md5, uuid, 0xHEX, or none)"
            )),
        }
    }

    pub fn is_enabled(&self) -> bool {
        !matches!(self, Self::None)
    }

    /// Unpadded descriptor byte length (`n_descsz`).
    pub fn desc_len(&self) -> usize {
        match self {
            Self::None => 0,
            Self::Sha1 => 20,
            Self::Md5 | Self::Uuid => 16,
            Self::Hex(b) => b.len(),
        }
    }

    /// 4-byte-aligned total `.note.gnu.build-id` section size (`16 + align4(descsz)`).
    pub fn note_size(&self) -> u64 {
        match self {
            Self::None => 0,
            _ => {
                let desc_aligned = (self.desc_len() + 3) & !3;
                (16 + desc_aligned) as u64
            }
        }
    }
}

fn hex_nibble(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Write the SHA-1 note header + name with a ZEROED descriptor at `off`.
/// Call [`patch_build_id`] after the image is complete.
pub fn write_build_id_skeleton(out: &mut [u8], off: usize) {
    write_build_id_skeleton_styled(out, off, &BuildIdStyle::Sha1);
}

/// Write the note header + `"GNU\0"` + zeroed descriptor for `style` at `off`.
pub fn write_build_id_skeleton_styled(out: &mut [u8], off: usize, style: &BuildIdStyle) {
    let total = style.note_size() as usize;
    if total == 0 || off + total > out.len() {
        return;
    }
    let descsz = style.desc_len() as u32;
    out[off..off + 4].copy_from_slice(&4u32.to_le_bytes()); // namesz
    out[off + 4..off + 8].copy_from_slice(&descsz.to_le_bytes()); // descsz
    out[off + 8..off + 12].copy_from_slice(&3u32.to_le_bytes()); // NT_GNU_BUILD_ID
    out[off + 12..off + 16].copy_from_slice(b"GNU\0");
    out[off + 16..off + total].fill(0);
}

/// Hash the whole image (descriptor still zero) with SHA-1 and patch the digest in.
pub fn patch_build_id(out: &mut [u8], note_off: usize) {
    patch_build_id_styled(out, note_off, &BuildIdStyle::Sha1);
}

/// Compute and write the build-id descriptor for `style` at `note_off`.
pub fn patch_build_id_styled(out: &mut [u8], note_off: usize, style: &BuildIdStyle) {
    let total = style.note_size() as usize;
    if total == 0 || note_off + total > out.len() {
        return;
    }
    // Zero the descriptor region before hashing so content-derived digests are well-defined.
    out[note_off + 16..note_off + total].fill(0);
    match style {
        BuildIdStyle::None => {}
        BuildIdStyle::Sha1 => {
            let digest = sha1(out);
            out[note_off + 16..note_off + 36].copy_from_slice(&digest);
        }
        BuildIdStyle::Md5 => {
            let digest = md5(out);
            out[note_off + 16..note_off + 32].copy_from_slice(&digest);
        }
        BuildIdStyle::Uuid => {
            let id = generate_uuid_v4(out);
            out[note_off + 16..note_off + 32].copy_from_slice(&id);
        }
        BuildIdStyle::Hex(bytes) => {
            out[note_off + 16..note_off + 16 + bytes.len()].copy_from_slice(bytes);
        }
    }
}

fn generate_uuid_v4(fallback_image: &[u8]) -> [u8; 16] {
    let mut buf = [0u8; 16];
    let mut filled = false;
    if let Ok(mut f) = std::fs::File::open("/dev/urandom") {
        use std::io::Read;
        if f.read_exact(&mut buf).is_ok() {
            filled = true;
        }
    }
    if !filled {
        let h = sha1(fallback_image);
        buf.copy_from_slice(&h[..16]);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0);
        let pid = u64::from(std::process::id());
        for (i, b) in (nanos ^ pid.rotate_left(32))
            .to_le_bytes()
            .iter()
            .enumerate()
        {
            buf[i] ^= *b;
        }
    }
    // RFC 4122 section 4.4: version 4 (0b0100) and variant 1 (0b10xx).
    buf[6] = (buf[6] & 0x0f) | 0x40;
    buf[8] = (buf[8] & 0x3f) | 0x80;
    buf
}

/// Minimal, dependency-free SHA-1 (FIPS 180-1). Linker-grade use only:
/// build-id fingerprinting, not cryptographic authentication.
///
/// Streaming: hashes `data` in place (no padded copy).
pub fn sha1(data: &[u8]) -> [u8; 20] {
    let mut h: [u32; 5] = [
        0x6745_2301,
        0xEFCD_AB89,
        0x98BA_DCFE,
        0x1032_5476,
        0xC3D2_E1F0,
    ];
    let mut w = [0u32; 80];

    let mut chunks = data.chunks_exact(64);
    for chunk in &mut chunks {
        sha1_block(&mut h, &mut w, chunk);
    }

    let rem = chunks.remainder();
    let ml = (data.len() as u64).wrapping_mul(8);
    let mut tail = [0u8; 128];
    tail[..rem.len()].copy_from_slice(rem);
    tail[rem.len()] = 0x80;
    let tail_len = if rem.len() < 56 { 64 } else { 128 };
    tail[tail_len - 8..tail_len].copy_from_slice(&ml.to_be_bytes());
    for chunk in tail[..tail_len].chunks_exact(64) {
        sha1_block(&mut h, &mut w, chunk);
    }

    let mut out = [0u8; 20];
    for i in 0..5 {
        out[4 * i..4 * i + 4].copy_from_slice(&h[i].to_be_bytes());
    }
    out
}

#[inline]
fn sha1_block(h: &mut [u32; 5], w: &mut [u32; 80], chunk: &[u8]) {
    for i in 0..16 {
        w[i] = u32::from_be_bytes([
            chunk[4 * i],
            chunk[4 * i + 1],
            chunk[4 * i + 2],
            chunk[4 * i + 3],
        ]);
    }
    for i in 16..80 {
        w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
    }
    let (mut a, mut b, mut c, mut d, mut e) = (h[0], h[1], h[2], h[3], h[4]);
    for (i, &wi) in w.iter().enumerate() {
        let (f, k) = match i {
            0..=19 => ((b & c) | ((!b) & d), 0x5A82_7999u32),
            20..=39 => (b ^ c ^ d, 0x6ED9_EBA1),
            40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1B_BCDC),
            _ => (b ^ c ^ d, 0xCA62_C1D6),
        };
        let tmp = a
            .rotate_left(5)
            .wrapping_add(f)
            .wrapping_add(e)
            .wrapping_add(k)
            .wrapping_add(wi);
        e = d;
        d = c;
        c = b.rotate_left(30);
        b = a;
        a = tmp;
    }
    h[0] = h[0].wrapping_add(a);
    h[1] = h[1].wrapping_add(b);
    h[2] = h[2].wrapping_add(c);
    h[3] = h[3].wrapping_add(d);
    h[4] = h[4].wrapping_add(e);
}

/// Minimal, dependency-free streaming MD5 (RFC 1321) for `--build-id=md5`.
pub fn md5(data: &[u8]) -> [u8; 16] {
    let mut state: [u32; 4] = [0x6745_2301, 0xefcd_ab89, 0x98ba_dcfe, 0x1032_5476];
    let mut chunks = data.chunks_exact(64);
    for chunk in &mut chunks {
        md5_block(&mut state, chunk);
    }
    let rem = chunks.remainder();
    let bit_len = (data.len() as u64).wrapping_mul(8);
    let mut tail = [0u8; 128];
    tail[..rem.len()].copy_from_slice(rem);
    tail[rem.len()] = 0x80;
    let tail_len = if rem.len() < 56 { 64 } else { 128 };
    tail[tail_len - 8..tail_len].copy_from_slice(&bit_len.to_le_bytes());
    for chunk in tail[..tail_len].chunks_exact(64) {
        md5_block(&mut state, chunk);
    }
    let mut out = [0u8; 16];
    for (i, word) in state.iter().enumerate() {
        out[4 * i..4 * i + 4].copy_from_slice(&word.to_le_bytes());
    }
    out
}

#[inline]
fn md5_block(state: &mut [u32; 4], chunk: &[u8]) {
    const S: [u32; 64] = [
        7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5,
        9, 14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10,
        15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
    ];
    const K: [u32; 64] = [
        0xd76aa478, 0xe8c7b756, 0x242070db, 0xc1bdceee, 0xf57c0faf, 0x4787c62a, 0xa8304613,
        0xfd469501, 0x698098d8, 0x8b44f7af, 0xffff5bb1, 0x895cd7be, 0x6b901122, 0xfd987193,
        0xa679438e, 0x49b40821, 0xf61e2562, 0xc040b340, 0x265e5a51, 0xe9b6c7aa, 0xd62f105d,
        0x02441453, 0xd8a1e681, 0xe7d3fbc8, 0x21e1cde6, 0xc33707d6, 0xf4d50d87, 0x455a14ed,
        0xa9e3e905, 0xfcefa3f8, 0x676f02d9, 0x8d2a4c8a, 0xfffa3942, 0x8771f681, 0x6d9d6122,
        0xfde5380c, 0xa4beea44, 0x4bdecfa9, 0xf6bb4b60, 0xbebfbc70, 0x289b7ec6, 0xeaa127fa,
        0xd4ef3085, 0x04881d05, 0xd9d4d039, 0xe6db99e5, 0x1fa27cf8, 0xc4ac5665, 0xf4292244,
        0x432aff97, 0xab9423a7, 0xfc93a039, 0x655b59c3, 0x8f0ccc92, 0xffeff47d, 0x85845dd1,
        0x6fa87e4f, 0xfe2ce6e0, 0xa3014314, 0x4e0811a1, 0xf7537e82, 0xbd3af235, 0x2ad7d2bb,
        0xeb86d391,
    ];
    let mut m = [0u32; 16];
    for i in 0..16 {
        m[i] = u32::from_le_bytes([
            chunk[4 * i],
            chunk[4 * i + 1],
            chunk[4 * i + 2],
            chunk[4 * i + 3],
        ]);
    }
    let (mut a, mut b, mut c, mut d) = (state[0], state[1], state[2], state[3]);
    for i in 0..64 {
        let (f, g) = match i {
            0..=15 => ((b & c) | ((!b) & d), i),
            16..=31 => ((d & b) | ((!d) & c), (5 * i + 1) % 16),
            32..=47 => (b ^ c ^ d, (3 * i + 5) % 16),
            _ => (c ^ (b | (!d)), (7 * i) % 16),
        };
        let tmp = d;
        d = c;
        c = b;
        b = b.wrapping_add(
            a.wrapping_add(f)
                .wrapping_add(K[i])
                .wrapping_add(m[g])
                .rotate_left(S[i]),
        );
        a = tmp;
    }
    state[0] = state[0].wrapping_add(a);
    state[1] = state[1].wrapping_add(b);
    state[2] = state[2].wrapping_add(c);
    state[3] = state[3].wrapping_add(d);
}

/// A synthetic input object carrying one empty `.note.gnu.build-id` section
/// sized for SHA-1.
pub fn synthetic_note_object() -> super::Elf64Object {
    synthetic_note_object_styled(&BuildIdStyle::Sha1)
}

/// A synthetic input object carrying one empty `.note.gnu.build-id` section
/// sized for `style`.
pub fn synthetic_note_object_styled(style: &BuildIdStyle) -> super::Elf64Object {
    use super::{Elf64Object, Elf64Section, SectionData};
    let note_size = style.note_size();
    let sections = vec![
        Elf64Section {
            name_idx: 0,
            name: String::new(),
            sh_type: 0,
            flags: 0,
            addr: 0,
            offset: 0,
            size: 0,
            link: 0,
            info: 0,
            addralign: 0,
            entsize: 0,
        },
        Elf64Section {
            name_idx: 0,
            name: ".note.gnu.build-id".into(),
            sh_type: 7, // SHT_NOTE
            flags: 0x2, // SHF_ALLOC
            addr: 0,
            offset: 0,
            size: note_size,
            link: 0,
            info: 0,
            addralign: 4,
            entsize: 0,
        },
    ];
    let section_data = vec![
        SectionData::empty(),
        SectionData::owned(vec![0u8; note_size as usize]),
    ];
    Elf64Object {
        sections,
        symbols: Vec::new(),
        section_data,
        relocations: vec![Vec::new(); 2],
        source_name: "<build-id>".into(),
        e_flags: 0,
    }
}

/// Patch the SHA-1 build-id note of an already-laid-out output section, if present.
pub fn patch_output_build_id(out: &mut [u8], note_file_offset: Option<u64>) -> bool {
    patch_output_build_id_styled(out, note_file_offset, &BuildIdStyle::Sha1)
}

/// Patch the build-id note of an already-laid-out output section using `style`.
pub fn patch_output_build_id_styled(
    out: &mut [u8],
    note_file_offset: Option<u64>,
    style: &BuildIdStyle,
) -> bool {
    let Some(off) = note_file_offset else {
        return false;
    };
    let total = style.note_size() as usize;
    let off = off as usize;
    if total == 0 || off + total > out.len() {
        return false;
    }
    write_build_id_skeleton_styled(out, off, style);
    patch_build_id_styled(out, off, style);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(d: &[u8]) -> String {
        d.iter().map(|b| format!("{:02x}", b)).collect()
    }

    #[test]
    fn sha1_vectors() {
        assert_eq!(
            hex(&sha1(b"abc")),
            "a9993e364706816aba3e25717850c26c9cd0d89d"
        );
        assert_eq!(hex(&sha1(b"")), "da39a3ee5e6b4b0d3255bfef95601890afd80709");
        assert_eq!(
            hex(&sha1(
                b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"
            )),
            "84983e441c3bd26ebaae4aa1f95129e5e54670f1"
        );
        let long = vec![b'a'; 1000];
        assert_eq!(
            hex(&sha1(&long)),
            "291e9a6c66994949b57ba5e650361e98fc36b1ba"
        );
    }

    #[test]
    fn md5_rfc1321_vectors() {
        assert_eq!(hex(&md5(b"")), "d41d8cd98f00b204e9800998ecf8427e");
        assert_eq!(hex(&md5(b"abc")), "900150983cd24fb0d6963f7d28e17f72");
        assert_eq!(
            hex(&md5(
                b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789"
            )),
            "d174ab98d277d9f5a5611c2c9f419d9f"
        );
    }

    #[test]
    fn build_id_styles_parse_and_emit() {
        assert_eq!(BuildIdStyle::parse(true, None).unwrap(), BuildIdStyle::Sha1);
        assert_eq!(
            BuildIdStyle::parse(true, Some("md5")).unwrap(),
            BuildIdStyle::Md5
        );
        assert_eq!(
            BuildIdStyle::parse(true, Some("uuid")).unwrap(),
            BuildIdStyle::Uuid
        );
        let hex_style = BuildIdStyle::parse(true, Some("0xdead-beef:42")).unwrap();
        assert_eq!(
            hex_style,
            BuildIdStyle::Hex(vec![0xde, 0xad, 0xbe, 0xef, 0x42])
        );
        assert_eq!(hex_style.note_size(), 24); // 16 + align4(5) = 24
        assert!(BuildIdStyle::parse(true, Some("bogus")).is_err());

        let mut img = vec![0x11u8; 96];
        assert!(patch_output_build_id_styled(&mut img, Some(16), &hex_style));
        assert_eq!(u32::from_le_bytes(img[20..24].try_into().unwrap()), 5);
        assert_eq!(&img[32..37], &[0xde, 0xad, 0xbe, 0xef, 0x42]);
        assert_eq!(&img[37..40], &[0, 0, 0]);

        let uuid_style = BuildIdStyle::Uuid;
        assert!(patch_output_build_id_styled(
            &mut img,
            Some(16),
            &uuid_style
        ));
        assert_eq!(img[32 + 6] >> 4, 4); // UUID v4
        assert_eq!(img[32 + 8] >> 6, 0b10); // RFC 4122 variant
    }

    #[test]
    fn note_roundtrip() {
        let mut img = vec![0u8; 128];
        write_build_id_skeleton(&mut img, 40);
        assert_eq!(&img[52..56], b"GNU\0");
        patch_build_id(&mut img, 40);
        assert_ne!(&img[56..76], &[0u8; 20][..]);
        let d1 = img[56..76].to_vec();
        for b in &mut img[56..76] {
            *b = 0;
        }
        patch_build_id(&mut img, 40);
        assert_eq!(d1, img[56..76].to_vec());
    }
}
