//! `.note.gnu.build-id` generation (`--build-id=sha1`).
//!
//! The build-id is a stable fingerprint of the linked image used by
//! debuggers (gdb's debuginfod lookup), core-dump matching, and the kernel's
//! module/vmlinux verification. GNU ld, mold, lld and wild all emit it; the
//! kernel passes `--build-id=sha1` for vmlinux and modules.
//!
//! We hash the complete output image with the note's descriptor field zeroed
//! (so the hash is well-defined), then patch the digest in. This makes the id
//! deterministic for identical inputs and unique across different images —
//! the two properties consumers rely on. (The exact byte value differs
//! between linkers; GNU ld hashes an internal normalized form. No consumer
//! compares ids across linkers.)
//!
//! Note layout (36 bytes, 4-byte aligned):
//!   u32 namesz = 4          ("GNU\0")
//!   u32 descsz = 20         (SHA-1)
//!   u32 type   = 3          (NT_GNU_BUILD_ID)
//!   "GNU\0"
//!   20-byte SHA-1 digest

/// Total size of the SHA-1 build-id note.
pub const BUILD_ID_NOTE_SIZE: u64 = 12 + 4 + 20;

/// Write the note header + name with a ZEROED descriptor at `off`.
/// Call [`patch_build_id`] after the image is complete.
pub fn write_build_id_skeleton(out: &mut [u8], off: usize) {
    out[off..off + 4].copy_from_slice(&4u32.to_le_bytes()); // namesz
    out[off + 4..off + 8].copy_from_slice(&20u32.to_le_bytes()); // descsz
    out[off + 8..off + 12].copy_from_slice(&3u32.to_le_bytes()); // NT_GNU_BUILD_ID
    out[off + 12..off + 16].copy_from_slice(b"GNU\0");
    for b in &mut out[off + 16..off + 36] {
        *b = 0;
    }
}

/// Hash the whole image (descriptor still zero) and patch the digest in.
pub fn patch_build_id(out: &mut [u8], note_off: usize) {
    let digest = sha1(out);
    out[note_off + 16..note_off + 36].copy_from_slice(&digest);
}

/// Minimal, dependency-free SHA-1 (FIPS 180-1). Linker-grade use only:
/// build-id fingerprinting, not security.
///
/// Streaming: hashes `data` in place (no padded copy). The previous
/// implementation cloned the full image for padding, adding an 11 MB
/// allocation + memcpy to every kernel-sized --build-id link.
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

    // Final block(s): remainder + 0x80 + zeros + bit-length (big-endian).
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha1_vectors() {
        // FIPS 180-1 test vectors
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
        // >1 block
        let long = vec![b'a'; 1000];
        assert_eq!(
            hex(&sha1(&long)),
            "291e9a6c66994949b57ba5e650361e98fc36b1ba"
        );
    }

    fn hex(d: &[u8]) -> String {
        d.iter().map(|b| format!("{:02x}", b)).collect()
    }

    #[test]
    fn note_roundtrip() {
        let mut img = vec![0u8; 128];
        write_build_id_skeleton(&mut img, 40);
        assert_eq!(&img[52..56], b"GNU\0");
        patch_build_id(&mut img, 40);
        // digest is non-zero and deterministic
        assert_ne!(&img[56..76], &[0u8; 20][..]);
        let d1 = img[56..76].to_vec();
        // same content (with desc zeroed) -> same digest
        for b in &mut img[56..76] {
            *b = 0;
        }
        patch_build_id(&mut img, 40);
        assert_eq!(d1, img[56..76].to_vec());
    }
}
