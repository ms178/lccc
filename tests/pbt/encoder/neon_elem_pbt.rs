//! Property-based tests for `encode_neon_elem` — the AArch64 NEON
//! *Advanced SIMD vector by element* (non-long) multiply family: `MUL`,
//! `MLA`, `MLS`, `SQDMULH`, `SQRDMULH` (by element).
//!
//! ```text
//!   31 30 29 28-24 23-22 21 20 19-16 15-12 11 10 9-5 4-0
//!    0  Q  U  01111  size  L  M   Rm   opcode  H   0  Rn  Rd
//! ```
//! `size`/`Q` come from the *destination* arrangement; `U` and the 4-bit
//! `opcode` identify the mnemonic (dispatch in `encoder/mod.rs`):
//!   * mul     → (U=0, opc=1000)   * mla  → (U=0, opc=0000)
//!   * mls     → (U=0, opc=0100)   * sqdmulh   → (U=0, opc=1100)
//!   * sqrdmulh→ (U=0, opc=1101)
//!     (Whether those `opcode` *values* match the ARMv8-A ARM is a `mod.rs`
//!     dispatch concern and is explicitly out of scope here — these properties
//!     test `encode_neon_elem`'s field-placement contract for whatever
//!     `(u_bit, opcode)` it is handed.)
//!
//! Per the ARMv8-A ARM (Advanced SIMD vector by element, multiply group):
//!   * halfword (`.h`) form: `index` = H:L:M (3 bits, 0..=7) and **Rm is
//!     constrained to V0–V15** (the M bit is sourced from the index, so only
//!     the low 4 bits of Rm are encodable);
//!   * word (`.s`) form: `index` = H:L (2 bits, 0..=3), and M = Rm[4], so Rm
//!     may be V0–V31.
//!
//! ## Oracle
//! A `ref_encode_elem` reference encoder re-assembles the word directly from
//! the documented layout, independent of this crate's implementation; a
//! hand-derived golden table anchors absolute correctness of the fixed fields.
//!
//! ## Findings (FAILING — `#[ignore]`d so `cargo test` stays green)
//! * **Out-of-range lane index silently wraps** — `prop_out_of_range_index_must_error`.
//!   The sibling `encode_neon_elem_long` rejects indices past the element-size
//!   bound (`neon.rs:266`, `neon.rs:273`), establishing the crate contract.
//!   `encode_neon_elem` does *not*: it masks with `index >> n & 1`, so e.g.
//!   halfword index 8 aliases index 0 and word index 4 aliases index 0. Silent
//!   corruption, no `Err`.
//! * **Halfword Rm silently truncated (V16–V31 alias V0–V15)** —
//!   `prop_halfword_rm_above_v15_must_be_rejected`. For the `.h` form the ARM
//!   constrains Rm to V0–V15, but the encoder masks with `rm & 0xF` instead of
//!   validating, so `v{N+16}.h[0]` aliases `v{N}.h[0]`.
//! * **`elem_size` of the lane operand is ignored** — `elem_size_mismatch_accepted`.
//!   Softer: `v0.4h, v1.4h, v2.s[0]` is accepted even though the lane size `.s`
//!   contradicts the arrangement `.4h`; the index is interpreted as a halfword
//!   index regardless. The function never reads `Operand::RegLane::elem_size`.

use lccc::pbt_internals::EncodeResult;
use lccc::pbt_internals::Operand;
use lccc::pbt_internals::encode_neon_elem;
use proptest::prelude::*;

// --- helpers --------------------------------------------------------------

/// `Operand::RegArrangement { reg: "v{n}", arrangement }`.
fn va(n: u32, arr: &str) -> Operand {
    Operand::RegArrangement {
        reg: format!("v{n}"),
        arrangement: arr.to_string(),
    }
}

/// `Operand::RegLane { reg: "v{n}", elem_size, index }` — the by-element operand.
fn lane(n: u32, elem: &str, index: u32) -> Operand {
    Operand::RegLane {
        reg: format!("v{n}"),
        elem_size: elem.to_string(),
        index,
    }
}

fn word_of(res: Result<EncodeResult, String>) -> u32 {
    match res {
        Ok(EncodeResult::Word(w)) => w,
        other => panic!("expected EncodeResult::Word, got {other:?}"),
    }
}

/// `(u_bit, opcode)` pairs actually handed to `encode_neon_elem` by the dispatch.
fn u_opc_strategy() -> impl Strategy<Value = (u32, u32)> {
    prop_oneof![
        Just((0u32, 0b1000u32)), // mul
        Just((0u32, 0b0000u32)), // mla
        Just((0u32, 0b0100u32)), // mls
        Just((0u32, 0b1100u32)), // sqdmulh
        Just((0u32, 0b1101u32)), // sqrdmulh
    ]
}

/// `(arrangement, rm, index)` triples that are architecturally VALID for the
/// non-long by-element form: halfword (.4h/.8h) → Rm 0..=15, index 0..=7;
/// word (.2s/.4s) → Rm 0..=31, index 0..=3.
fn valid_case() -> impl Strategy<Value = (&'static str, u32, u32)> {
    prop_oneof![
        (Just("4h"), 0u32..=15, 0u32..=7),
        (Just("8h"), 0u32..=15, 0u32..=7),
        (Just("2s"), 0u32..=31, 0u32..=3),
        (Just("4s"), 0u32..=31, 0u32..=3),
    ]
}

/// Independent reference encoder assembled straight from the ARMv8-A layout.
/// For the word form it places Rm[3:0] in the Rm field and M=Rm[4] separately,
/// matching the architecture rather than the implementation's redundant
/// double-write of bit 20.
fn ref_encode_elem(
    rd: u32,
    rn: u32,
    rm: u32,
    index: u32,
    arr: &str,
    u_bit: u32,
    opcode: u32,
) -> u32 {
    let (q, size) = match arr {
        "4h" => (0u32, 0b01u32),
        "8h" => (1u32, 0b01u32),
        "2s" => (0u32, 0b10u32),
        "4s" => (1u32, 0b10u32),
        _ => panic!("bad arrangement in reference encoder: {arr}"),
    };
    let (h, l, m, rm_field) = if size == 0b01 {
        assert!(index <= 7 && rm <= 15, "ref: invalid halfword input");
        let h = (index >> 2) & 1;
        let l = (index >> 1) & 1;
        let m = index & 1; // M comes from the index for .h
        (h, l, m, rm & 0xF)
    } else {
        assert!(index <= 3, "ref: invalid word index");
        let h = (index >> 1) & 1;
        let l = index & 1;
        let m = (rm >> 4) & 1; // M = Rm[4] for .s
        (h, l, m, rm & 0xF)
    };

    let mut w = 0u32;
    w |= q << 30;
    w |= u_bit << 29;
    w |= 0b01111u32 << 24;
    w |= size << 22;
    w |= l << 21;
    w |= m << 20;
    w |= rm_field << 16;
    w |= opcode << 12;
    w |= h << 11;
    // bit 10 is architecturally 0
    w |= rn << 5;
    w |= rd;
    w
}

type GoldenRow = (u32, u32, u32, u32, &'static str, u32, u32, u32);

// --- golden table (absolute oracle) ---------------------------------------
//
// Each entry: (Rd, Rn, Rm, index, arrangement, u_bit, opcode, word),
// hand-derived field-by-field from the layout in the module header. The lane
// `elem_size` is implied by the arrangement's element class (".h" vs ".s").
// All eight values were verified by nibble decomposition.
const GOLDEN: &[GoldenRow] = &[
    // mul v0.4h, v1.4h, v2.h[0]      (U=0, opc=1000)
    (0, 1, 2, 0, "4h", 0, 0b1000, 0x0F428020),
    // mla v0.4h, v1.4h, v2.h[7]      (U=0, opc=0000, halfword max index)
    (0, 1, 2, 7, "4h", 0, 0b0000, 0x0F720820),
    // sqdmulh v0.4h, v1.4h, v2.h[0]  (U=0, opc=1100)
    (0, 1, 2, 0, "4h", 0, 0b1100, 0x0F42C020),
    // sqdmulh v0.4s, v1.4s, v2.s[0]  (U=0, opc=1100, word)
    (0, 1, 2, 0, "4s", 0, 0b1100, 0x4F82C020),
    // mul v0.4s, v1.4s, v16.s[0]     (U=0, opc=1000, M bit taken from Rm[4]=1)
    (0, 1, 16, 0, "4s", 0, 0b1000, 0x4F908020),
    // mls v0.4s, v1.4s, v2.s[3]      (U=0, opc=0100, word max index)
    (0, 1, 2, 3, "4s", 0, 0b0100, 0x4FA24820),
];

#[test]
fn elem_matches_golden_table() {
    for &(rd, rn, rm, index, arr, u_bit, opcode, expected) in GOLDEN {
        let elem = if arr.ends_with('h') { "h" } else { "s" };
        let ops = vec![va(rd, arr), va(rn, arr), lane(rm, elem, index)];
        let got = word_of(encode_neon_elem(&ops, u_bit, opcode));
        assert_eq!(
            got, expected,
            "arr={arr} idx={index} rm=v{rm} u={u_bit} opc=0b{opcode:b}: \
             got 0x{got:08X}, want 0x{expected:08X}",
        );
        // The reference encoder must agree with the hand-derived golden value too.
        assert_eq!(
            ref_encode_elem(rd, rn, rm, index, arr, u_bit, opcode),
            expected,
            "reference encoder drift",
        );
    }
}

// --- properties (PASS on current implementation) -------------------------

proptest! {
    // === Oracle: differential reference encoder ============================
    // For every valid (arrangement, Rm, index, U, opcode) and register pair,
    // the implementation must equal the independently-assembled word.
    #[test]
    fn matches_reference_encoder(
        rd in 0u32..=31,
        rn in 0u32..=31,
        case in valid_case(),
        u_opc in u_opc_strategy(),
    ) {
        let (arr, rm, index) = case;
        let (u_bit, opcode) = u_opc;
        let elem = if arr.ends_with('h') { "h" } else { "s" };
        let ops = vec![va(rd, arr), va(rn, arr), lane(rm, elem, index)];
        let got = word_of(encode_neon_elem(&ops, u_bit, opcode));
        let want = ref_encode_elem(rd, rn, rm, index, arr, u_bit, opcode);
        prop_assert_eq!(got, want);
    }

    // === Fixed-bits invariant =============================================
    // The architecturally-constant bits never change for any valid input:
    // bit31=0, bits28-24=01111, bit10=0.
    #[test]
    fn fixed_bits_are_constant(
        rd in 0u32..=31,
        rn in 0u32..=31,
        case in valid_case(),
        u_opc in u_opc_strategy(),
    ) {
        let (arr, rm, index) = case;
        let (u_bit, opcode) = u_opc;
        let elem = if arr.ends_with('h') { "h" } else { "s" };
        let ops = vec![va(rd, arr), va(rn, arr), lane(rm, elem, index)];
        let w = word_of(encode_neon_elem(&ops, u_bit, opcode));

        prop_assert_eq!((w >> 31) & 1, 0, "bit 31 must be 0");
        prop_assert_eq!((w >> 24) & 0x1F, 0b01111, "bits 28-24 must be 01111");
        prop_assert_eq!((w >> 10) & 1, 0, "bit 10 must be 0");
    }

    // === Field placement & index round-trip ===============================
    // Rd (bits 4-0) and Rn (bits 9-5) round-trip exactly; U (bit 29) and the
    // 4-bit opcode (bits 15-12) map correctly; and the lane index reconstructs
    // from the scattered H(bit11):L(bit21):M(bit20) fields. For the word form
    // the full 5-bit Rm must reconstruct as M:Rm[3:0].
    #[test]
    fn fields_and_index_round_trip(
        rd in 0u32..=31,
        rn in 0u32..=31,
        case in valid_case(),
        u_opc in u_opc_strategy(),
    ) {
        let (arr, rm, index) = case;
        let (u_bit, opcode) = u_opc;
        let elem = if arr.ends_with('h') { "h" } else { "s" };
        let ops = vec![va(rd, arr), va(rn, arr), lane(rm, elem, index)];
        let w = word_of(encode_neon_elem(&ops, u_bit, opcode));

        prop_assert_eq!(w & 0x1F, rd, "Rd field (bits 4-0)");
        prop_assert_eq!((w >> 5) & 0x1F, rn, "Rn field (bits 9-5)");
        prop_assert_eq!((w >> 29) & 1, u_bit, "U bit (bit 29)");
        prop_assert_eq!((w >> 12) & 0xF, opcode, "opcode field (bits 15-12)");

        let h = (w >> 11) & 1;
        let l = (w >> 21) & 1;
        let m = (w >> 20) & 1;
        if arr.ends_with('h') {
            // halfword: index = H:L:M (3 bits)
            prop_assert_eq!((h << 2) | (l << 1) | m, index, "halfword index H:L:M");
            prop_assert_eq!((w >> 16) & 0xF, rm & 0xF, "halfword Rm = low 4 bits");
        } else {
            // word: index = H:L (2 bits), M = Rm[4]
            prop_assert_eq!((h << 1) | l, index, "word index H:L");
            prop_assert_eq!(m, (rm >> 4) & 1, "word M bit must equal Rm[4]");
            prop_assert_eq!((m << 4) | ((w >> 16) & 0xF), rm, "word Rm = M:Rm[3:0]");
        }
    }
}

// --- confirmed findings: FAILING, `#[ignore]`d ---------------------------
//
// These assert the documented range contracts and FAIL on the current
// implementation. `#[ignore]` keeps `cargo test` green; run explicitly with
// `cargo test -- --ignored`.

proptest! {
    // === BUG: out-of-range lane index silently wraps ======================
    // Halfword indices must be 0..=7, word indices 0..=3; anything larger
    // cannot be encoded and must yield `Err` (the long sibling
    // `encode_neon_elem_long` enforces exactly this at `neon.rs:266`/`:273`).
    // `encode_neon_elem` masks instead, aliasing out-of-range indices to 0.
    #[test]
    #[ignore = "documented bug: out-of-range by-element index silently wraps (no Err)"]
    fn prop_out_of_range_index_must_error(
        idx_h in 8u32..=0xFFFF,
        idx_s in 4u32..=0xFFFF,
    ) {
        let h_ops = vec![va(0, "4h"), va(1, "4h"), lane(2, "h", idx_h)];
        prop_assert!(encode_neon_elem(&h_ops, 0, 0b1000).is_err(),
            "halfword index {idx_h} must be rejected");

        let s_ops = vec![va(0, "4s"), va(1, "4s"), lane(2, "s", idx_s)];
        prop_assert!(encode_neon_elem(&s_ops, 0, 0b1000).is_err(),
            "word index {idx_s} must be rejected");
    }

    // === BUG: halfword Rm V16–V31 alias V0–V15 ============================
    // For the halfword (`.h`) by-element form the ARMv8-A ARM constrains Rm to
    // V0–V15 (M is sourced from the index, leaving a 4-bit Rm field). The
    // encoder masks with `rm & 0xF` instead of validating, so `v{N+16}.h[0]`
    // aliases `v{N}.h[0]`. Asserts the range contract; FAILS on the current
    // implementation.
    #[test]
    #[ignore = "documented bug: halfword by-element Rm V16-V31 aliases V0-V15"]
    fn prop_halfword_rm_above_v15_must_be_rejected(rm in 16u32..=31) {
        let ops = vec![va(0, "4h"), va(1, "4h"), lane(rm, "h", 0)];
        let res = encode_neon_elem(&ops, 0, 0b1000);
        prop_assert!(res.is_err(),
            "halfword by-element Rm must be V0-V15 (ARM DDI 0487); \
             v{rm}.h[0] must be rejected, but got {:?}", res);
    }
}

/// Concrete demonstration of the index-aliasing bug: halfword index 8 encodes
/// identically to index 0, and word index 4 encodes identically to index 0.
/// Pins the buggy behaviour so the finding is unambiguous; it will fail (as
/// intended) once the encoder is fixed to reject out-of-range indices.
#[test]
#[ignore = "documented bug: out-of-range index silently wraps"]
fn out_of_range_index_silently_aliases_zero() {
    // halfword: index 8 → H:L:M = 0:0:0 = index 0
    let lo = word_of(encode_neon_elem(
        &[va(0, "4h"), va(1, "4h"), lane(2, "h", 0)],
        0,
        0b1000,
    ));
    let hi = word_of(encode_neon_elem(
        &[va(0, "4h"), va(1, "4h"), lane(2, "h", 8)],
        0,
        0b1000,
    ));
    assert_eq!(
        lo, hi,
        "halfword index 0 (0x{lo:08X}) and index 8 (0x{hi:08X}) encode identically \
         — index was silently truncated",
    );

    // word: index 4 → H:L = 0:0 = index 0
    let lo = word_of(encode_neon_elem(
        &[va(0, "4s"), va(1, "4s"), lane(2, "s", 0)],
        0,
        0b1000,
    ));
    let hi = word_of(encode_neon_elem(
        &[va(0, "4s"), va(1, "4s"), lane(2, "s", 4)],
        0,
        0b1000,
    ));
    assert_eq!(
        lo, hi,
        "word index 0 (0x{lo:08X}) and index 4 (0x{hi:08X}) encode identically \
         — index was silently truncated",
    );
}

/// Concrete demonstration of the Rm-aliasing bug: `v{N+16}.h[0]` encodes
/// identically to `v{N}.h[0]` (both collapse to Rm field = N). `#[ignore]`d
/// so default `cargo test` stays green.
#[test]
#[ignore = "documented bug: halfword by-element Rm V16-V31 aliases V0-V15"]
fn halfword_rm_silently_aliases_v16_to_v0() {
    for n in 0u32..16 {
        let lo = word_of(encode_neon_elem(
            &[va(0, "4h"), va(1, "4h"), lane(n, "h", 0)],
            0,
            0b1000,
        ));
        let hi = word_of(encode_neon_elem(
            &[va(0, "4h"), va(1, "4h"), lane(n + 16, "h", 0)],
            0,
            0b1000,
        ));
        assert_eq!(
            lo,
            hi,
            "v{n}.h[0] (0x{lo:08X}) and v{}.h[0] (0x{hi:08X}) encode identically \
             — Rm was silently truncated",
            n + 16,
        );
    }
}

/// Softer finding: `elem_size` of the lane operand is never consulted. A `.s`
/// lane paired with a `.4h` arrangement is accepted and the index is decoded
/// as a halfword index. `#[ignore]`d.
#[test]
#[ignore = "documented soft finding: lane elem_size is ignored (not validated against arrangement)"]
fn elem_size_mismatch_accepted() {
    // v0.4h, v1.4h, v2.s[0] — lane size '.s' contradicts arrangement '.4h',
    // yet this is accepted and produces a valid halfword-by-element word.
    let mismatch = vec![va(0, "4h"), va(1, "4h"), lane(2, "s", 0)];
    let res = encode_neon_elem(&mismatch, 0, 0b1000);
    assert!(
        res.is_ok(),
        "elem_size mismatch should be rejected but was accepted as {res:?}"
    );
}
