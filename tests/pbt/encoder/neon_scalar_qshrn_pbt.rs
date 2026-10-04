//! Property-based tests for `encode_neon_scalar_qshrn`.
//!
//! `encode_neon_scalar_qshrn(operands, u_bit, is_rounding)` emits the AArch64
//! **"Advanced SIMD scalar shift by immediate (narrow)"** encoding for the four
//! saturating-shift-right-narrow scalar instructions, selected by the two flag
//! parameters:
//!
//!  | `u_bit` | `is_rounding` | mnemonic   |
//!  |---------|---------------|------------|
//!  | 0       | false         | `SQSHRN`   |
//!  | 0       | true          | `SQRSHRN`  |
//!  | 1       | false         | `UQSHRN`   |
//!  | 1       | true          | `UQRSHRN`  |
//!
//! Documented bit layout (ARM DDI 0487, "Advanced SIMD scalar shift by
//! immediate"):
//! ```text
//!   31 30 29 28 27 26 25 24 23 | 22 21 20 19 | 18 17 16 | 15 14 13 12 11 | 10 | 9 8 7 6 5 | 4 3 2 1 0
//!    0  1  U  1  1  1  1  1  0 |<------- immh ------->|<-- immb -->|<----- opcode ---->|  1 |    Rn     |    Rd
//! ```
//! The destination scalar letter fixes the element size: `b`→8 (src `h`),
//! `h`→16 (src `s`), `s`→32 (src `d`). `immh:immb = 2*dest_bits - shift`, and
//! the valid shift window is `[1, dest_bits]`.
//!
//! ## Oracle
//! The reference word is re-assembled field-by-field from the layout above
//! (independent of the implementation) and additionally pinned to golden
//! values produced by the system assembler `llvm-mc-18`, e.g.
//! `sqshrn b0,h0,#1 = 0x5F0F9400`, `uqrshrn b0,h0,#1 = 0x7F0F9C00`.
//!
//! ## Focus areas
//!  * **shift-range validation** — `shift == 0` or `shift > dest_bits` must
//!    yield `Err` (passing); the `as u32` cast silently truncates huge/negative
//!    shifts (ignored witness).
//!  * **is_high** — *not applicable*: scalar SQSHRN is inherently single-
//!    element (no `.2`/high-half form). Bit 30 is the fixed scalar marker `1`,
//!    not a Q bit; we assert it is constant.
//!  * **u-bit** — bit 29 must mirror `u_bit` for `{0,1}` (passing); values
//!    outside the 1-bit field are not range-checked (ignored witness).
//!  * **rounding variants** — opcode bits `[15:11]` are `10010` (non-round)
//!    vs `10011` (round); the fixed `1` at bit 10 is constant (passing).
//!
//! ## Finding (documented by the `#[ignore]`d witnesses)
//! The implementation builds the high field with `0b011110 << 23`, which clears
//! **bit 28**. The ISA fixes bit 28 = `1` (confirmed by `llvm-mc-18`: every
//! scalar shift-by-immediate word has top byte `0x5F`/`0x7F`, never `0x4F`/
//! `0x6F`). Concretely `sqshrn b0,h0,#1` should be `0x5F0F9400` but the encoder
//! yields `0x4F0F9400` — exactly one bit (bit 28) wrong. The differential,
//! golden, and bit-28-invariant properties below all fail today and are kept
//! `#[ignore]` so `cargo test` stays green; run with `--ignored`.

use lccc::pbt_internals::EncodeResult;
use lccc::pbt_internals::Operand;
use lccc::pbt_internals::encode_neon_scalar_qshrn;
use proptest::prelude::*;
use std::io::Write;
use std::process::{Command, Stdio};

// --- helpers --------------------------------------------------------------

/// `(dest_prefix, dest_elem_bits, source_letter, max_valid_shift)`.
const DEST: &[(&str, u32, char, u32)] = &[
    ("b", 8, 'h', 8),   // Bn <- Hn  (narrow 16->8)
    ("h", 16, 's', 16), // Hn <- Sn  (narrow 32->16)
    ("s", 32, 'd', 32), // Sn <- Dn  (narrow 64->32)
];

fn reg(prefix: &str, n: u32) -> Operand {
    Operand::Reg(format!("{prefix}{n}"))
}

fn word_of(res: Result<EncodeResult, String>) -> Result<u32, String> {
    match res {
        Ok(EncodeResult::Word(w)) => Ok(w),
        Ok(other) => Err(format!("expected EncodeResult::Word, got {other:?}")),
        Err(e) => Err(e),
    }
}

/// Run the encoder for `(dest, dest_num) <- (src_letter, src_num), #shift`.
fn run(
    dest_prefix: &str,
    dest_num: u32,
    src_num: u32,
    shift: i64,
    u_bit: u32,
    is_rounding: bool,
) -> Result<u32, String> {
    let src_letter = DEST
        .iter()
        .find(|(p, _, _, _)| *p == dest_prefix)
        .map(|(_, _, c, _)| *c)
        .unwrap();
    let ops = vec![
        reg(dest_prefix, dest_num),
        reg(&src_letter.to_string(), src_num),
        Operand::Imm(shift),
    ];
    word_of(encode_neon_scalar_qshrn(&ops, u_bit, is_rounding))
}

/// Independent reference encoder built straight from the ISA layout above.
/// NOTE: uses `0b111110 << 23` (bit 28 = 1), the *correct* high field — this
/// deliberately diverges from the implementation's `0b011110 << 23`.
fn ref_encode(
    rd: u32,
    rn: u32,
    dest_elem_bits: u32,
    u_bit: u32,
    is_rounding: bool,
    shift: u32,
) -> u32 {
    let immhb = dest_elem_bits * 2 - shift;
    let opcode = if is_rounding { 0b100111u32 } else { 0b100101 };
    (0b01u32 << 30)
        | ((u_bit & 1) << 29)
        | (0b111110u32 << 23) // bits 28-23, bit 28 == 1 (ISA-fixed)
        | ((immhb >> 3) << 19)
        | ((immhb & 7) << 16)
        | (opcode << 10)
        | ((rn & 0x1F) << 5)
        | (rd & 0x1F)
}

/// Differential oracle: assemble one instruction with `llvm-mc-18` and return
/// its little-endian 32-bit word, or `None` if the tool is unavailable / the
/// operand combination is rejected.
fn llvm_mc(
    mnem: &str,
    dest: &str,
    dest_num: u32,
    src_letter: char,
    src_num: u32,
    shift: u32,
) -> Option<u32> {
    let text = format!("{mnem} {dest}{dest_num}, {src_letter}{src_num}, #{shift}\n");
    let mut child = Command::new("llvm-mc-18")
        .args(["--triple=aarch64", "--assemble", "--show-encoding"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;
    {
        let mut stdin = child.stdin.take()?;
        stdin.write_all(text.as_bytes()).ok()?;
    }
    let out = child.wait_with_output().ok()?;
    if !out.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    let line = stdout.lines().find(|l| l.contains("encoding:"))?;
    let bytes_str = line.split('[').nth(1)?.split(']').next()?;
    let bytes: Vec<u8> = bytes_str
        .split(',')
        .map(|s| s.trim().trim_start_matches("0x"))
        .filter_map(|s| u8::from_str_radix(s, 16).ok())
        .collect();
    if bytes.len() == 4 {
        Some(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    } else {
        None
    }
}

// --- golden table (absolute oracle, derived from llvm-mc-18) --------------

/// `(mnemonic, u_bit, is_rounding, dest_prefix, dest_num, src_num, shift, expected_word)`.
/// All words were emitted by `llvm-mc-18 --triple=aarch64`.
#[rustfmt::skip]
type GoldenRow = (&'static str, u32, bool, &'static str, u32, u32, u32, u32);

const GOLDEN: &[GoldenRow] = &[
    ("sqshrn", 0, false, "b", 0, 0, 1, 0x5F0F9400), // sqshrn  b0,h0,#1
    ("sqshrn", 0, false, "b", 0, 0, 8, 0x5F089400), // sqshrn  b0,h0,#8
    ("sqshrn", 0, false, "h", 0, 0, 16, 0x5F109400), // sqshrn  h0,s0,#16
    ("sqshrn", 0, false, "s", 0, 0, 32, 0x5F209400), // sqshrn  s0,d0,#32
    ("sqrshrn", 0, true, "b", 0, 0, 1, 0x5F0F9C00), // sqrshrn b0,h0,#1
    ("sqrshrn", 0, true, "s", 0, 0, 16, 0x5F309C00), // sqrshrn s0,d0,#16
    ("uqshrn", 1, false, "b", 0, 0, 1, 0x7F0F9400), // uqshrn  b0,h0,#1
    ("uqshrn", 1, false, "h", 0, 0, 8, 0x7F189400), // uqshrn  h0,s0,#8
    ("uqrshrn", 1, true, "b", 0, 0, 8, 0x7F089C00), // uqrshrn b0,h0,#8
    ("uqrshrn", 1, true, "s", 0, 0, 32, 0x7F209C00), // uqrshrn s0,d0,#32
];

// =====================================================================
// PASSING properties: verify sub-behaviours not affected by the bit-28 bug
// =====================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    // --- Rd / Rn fields are preserved -----------------------------------
    #[test]
    fn prop_rd_rn_preserved(
        dest_idx in 0usize..DEST.len(),
        rd in 0u32..32u32,
        rn in 0u32..32u32,
        shift in 1u32..=32u32,
        u_bit in 0u32..2u32,
        is_rounding in any::<bool>(),
    ) {
        let (dest, _, _, max_shift) = DEST[dest_idx];
        let shift = (shift % max_shift) + 1;
        let w = run(dest, rd, rn, shift as i64, u_bit, is_rounding)
            .expect("valid input must encode");
        prop_assert_eq!(w & 0x1F, rd, "Rd field (bits 4-0)");
        prop_assert_eq!((w >> 5) & 0x1F, rn, "Rn field (bits 9-5)");
    }

    // --- immh:immb (bits 22-16) == 2*dest_bits - shift ------------------
    #[test]
    fn prop_immh_immb_value(
        dest_idx in 0usize..DEST.len(),
        shift in 1u32..=32u32,
        u_bit in 0u32..2u32,
        is_rounding in any::<bool>(),
    ) {
        let (dest, ebits, _, max_shift) = DEST[dest_idx];
        let shift = (shift % max_shift) + 1;
        let w = run(dest, 0, 0, shift as i64, u_bit, is_rounding).unwrap();
        let immh_immb = (w >> 16) & 0x7F;
        prop_assert_eq!(immh_immb, ebits * 2 - shift, "immh:immb for {} shift {}", dest, shift);
    }

    // --- U bit (29), rounding opcode (15-11), fixed bit 10 --------------
    #[test]
    fn prop_u_bit_and_rounding_opcode(
        dest_idx in 0usize..DEST.len(),
        shift in 1u32..=32u32,
        u_bit in 0u32..2u32,
        is_rounding in any::<bool>(),
    ) {
        let (dest, _, _, max_shift) = DEST[dest_idx];
        let shift = (shift % max_shift) + 1;
        let w = run(dest, 0, 0, shift as i64, u_bit, is_rounding).unwrap();
        prop_assert_eq!((w >> 29) & 1, u_bit, "U bit");
        prop_assert_eq!((w >> 10) & 1, 1u32, "fixed bit 10");
        let opc = (w >> 11) & 0x1F;
        let want = if is_rounding { 0b10011u32 } else { 0b10010u32 };
        prop_assert_eq!(opc, want, "opcode bits 15-11 (rounding={})", is_rounding);
    }

    // --- Scalar marker + non-(bit-28) fixed bits ------------------------
    // Addresses the `is_high` focus: the scalar form has NO Q bit / high-half
    // variant; bit 30 is the constant scalar marker `1`. Bit 28 is asserted
    // separately (and fails today) so this property stays green.
    #[test]
    fn prop_scalar_marker_and_non28_fixed_bits(
        dest_idx in 0usize..DEST.len(),
        shift in 1u32..=32u32,
        u_bit in 0u32..2u32,
        is_rounding in any::<bool>(),
    ) {
        let (dest, _, _, max_shift) = DEST[dest_idx];
        let shift = (shift % max_shift) + 1;
        let w = run(dest, 0, 0, shift as i64, u_bit, is_rounding).unwrap();
        prop_assert_eq!((w >> 30) & 0x3, 0b01u32, "bits 31-30 == 01 (scalar, no Q bit)");
        prop_assert_eq!((w >> 24) & 0x0F, 0b1111u32, "bits 27-24 == 1111");
        prop_assert_eq!((w >> 23) & 1, 0u32, "bit 23 == 0 (start of immh region)");
    }

    // --- shift-range validation (realistic window) ----------------------
    // shift==0 and shift>dest_bits are rejected; the whole in-range window
    // is accepted.
    #[test]
    fn prop_rejects_out_of_range_shift(
        dest_idx in 0usize..DEST.len(),
        shift in proptest::sample::select(vec![0u32, 1, 2, 8, 9, 16, 17, 32, 33, 64]),
        u_bit in 0u32..2u32,
        is_rounding in any::<bool>(),
    ) {
        let (dest, ebits, src, _) = DEST[dest_idx];
        let ops = vec![
            reg(dest, 0),
            reg(&src.to_string(), 0),
            Operand::Imm(shift as i64),
        ];
        let res = encode_neon_scalar_qshrn(&ops, u_bit, is_rounding);
        let should_err = shift == 0 || shift > ebits;
        prop_assert_eq!(res.is_err(), should_err, "dest={} shift={}", dest, shift);
    }

    // --- arity / operand type / dest prefix / register range -----------
    #[test]
    fn prop_rejects_malformed_operands(
        n in 0u32..=2u32,                       // operand counts 0,1,2 (< 3)
        bad_reg in 32u32..200u32,               // beyond the 5-bit field
        bad_pfx_idx in 0usize..4usize,          // x/w/v/q (or d, which is unsupported)
    ) {
        // (a) insufficient arity
        let short: Vec<Operand> = (0..n).map(|_| reg("b", 0)).collect();
        prop_assert!(
            encode_neon_scalar_qshrn(&short, 0, false).is_err(),
            "{} operands must be rejected (need >= 3)",
            n,
        );
        // (b) non-Reg operands
        let ops_bad_kind = vec![Operand::Imm(1), Operand::Imm(2), Operand::Imm(3)];
        prop_assert!(
            encode_neon_scalar_qshrn(&ops_bad_kind, 0, false).is_err(),
            "non-Reg operands must be rejected",
        );
        // (c) out-of-range register number (parse_reg_num rejects >= 32)
        let ops_bad_reg = vec![reg("b", bad_reg), reg("h", 1), Operand::Imm(1)];
        prop_assert!(
            encode_neon_scalar_qshrn(&ops_bad_reg, 0, false).is_err(),
            "register b{} (>= 32) must be rejected",
            bad_reg,
        );
        // (d) unsupported destination prefix (d is a *source* width, not a
        //     valid narrowing destination; x/w/v are not NEON scalars)
        let bad_pfx = ["d", "x", "w", "v"][bad_pfx_idx];
        let ops_bad_pfx = vec![reg(bad_pfx, 0), reg("h", 1), Operand::Imm(1)];
        prop_assert!(
            encode_neon_scalar_qshrn(&ops_bad_pfx, 0, false).is_err(),
            "unsupported dest prefix '{}' must be rejected",
            bad_pfx,
        );
    }

    // ===================================================================
    // BUG WITNESSES — each is #[ignore]d so the default run stays green.
    // ===================================================================

    // --- WITNESS: differential oracle vs llvm-mc-18 (bit 28 bug) --------
    #[test]
    #[ignore = "bug: encoder clears bit 28; llvm-mc-18 requires bit 28 = 1"]
    fn prop_matches_llvm_mc(
        dest_idx in 0usize..DEST.len(),
        rd in 0u32..32u32,
        rn in 0u32..32u32,
        shift in 1u32..=32u32,
        u_bit in 0u32..2u32,
        is_rounding in any::<bool>(),
    ) {
        let (dest, _, src, max_shift) = DEST[dest_idx];
        let shift = (shift % max_shift) + 1;
        let mnem = match (u_bit, is_rounding) {
            (0, false) => "sqshrn",
            (0, true) => "sqrshrn",
            (1, false) => "uqshrn",
            (1, true) => "uqrshrn",
            _ => unreachable!(),
        };
        let got = run(dest, rd, rn, shift as i64, u_bit, is_rounding).unwrap();
        if let Some(refw) = llvm_mc(mnem, dest, rd, src, rn, shift) {
            prop_assert_eq!(
                got, refw,
                "{} {}{},{}{},#{}: got 0x{:08X} want 0x{:08X}",
                mnem, dest, rd, src, rn, shift, got, refw
            );
        }
    }

    // --- WITNESS: reference encoder (correct bit 28) --------------------
    #[test]
    #[ignore = "bug: encoder clears bit 28; reference uses 0b111110<<23"]
    fn prop_matches_reference_encoder(
        dest_idx in 0usize..DEST.len(),
        rd in 0u32..32u32,
        rn in 0u32..32u32,
        shift in 1u32..=32u32,
        u_bit in 0u32..2u32,
        is_rounding in any::<bool>(),
    ) {
        let (dest, ebits, _, max_shift) = DEST[dest_idx];
        let shift = (shift % max_shift) + 1;
        let got = run(dest, rd, rn, shift as i64, u_bit, is_rounding).unwrap();
        let want = ref_encode(rd, rn, ebits, u_bit, is_rounding, shift);
        prop_assert_eq!(got, want, "dest={} shift={}", dest, shift);
    }

    // --- WITNESS: bit 28 must be the ISA-fixed 1 ------------------------
    #[test]
    #[ignore = "bug: encoder emits bit 28 = 0; spec requires bit 28 = 1"]
    fn prop_bit_28_is_set(
        dest_idx in 0usize..DEST.len(),
        shift in 1u32..=32u32,
        u_bit in 0u32..2u32,
        is_rounding in any::<bool>(),
    ) {
        let (dest, _, _, max_shift) = DEST[dest_idx];
        let shift = (shift % max_shift) + 1;
        let w = run(dest, 0, 0, shift as i64, u_bit, is_rounding).unwrap();
        prop_assert_eq!((w >> 28) & 1, 1u32, "bit 28 must be 1; word=0x{:08x}", w);
    }

    // --- WITNESS: u_bit is a 1-bit field and must be range-checked ------
    #[test]
    #[ignore = "bug: u_bit is OR-shifted without range check (corrupts scalar marker)"]
    fn prop_rejects_out_of_range_u_bit(big_u in 2u32..16u32) {
        let ops = vec![reg("b", 0), reg("h", 1), Operand::Imm(1)];
        let res = encode_neon_scalar_qshrn(&ops, big_u, false);
        prop_assert!(
            res.is_err(),
            "u_bit={} is out of the 1-bit field (spec: bit 29); expected Err, got {:?}",
            big_u,
            res,
        );
    }

    // --- WITNESS: huge/negative shifts must not truncate into range -----
    #[test]
    #[ignore = "bug: `get_imm(..) as u32` silently truncates out-of-range shifts"]
    fn prop_rejects_truncating_shift(
        k in 1u32..8u32,            // small "in-range-looking" residue
        big in proptest::sample::select(vec![1u64 << 32, (1u64 << 32) + 5, u64::MAX, (1u64 << 40)]),
    ) {
        // (a) huge positive shift whose low 32 bits land in [1,8]
        let huge_pos = (big as i64) + k as i64; // residue in-range after `as u32`
        let ops_pos = vec![reg("b", 0), reg("h", 1), Operand::Imm(huge_pos)];
        let res_pos = encode_neon_scalar_qshrn(&ops_pos, 0, false);
        prop_assert!(
            res_pos.is_err(),
            "shift {} is far out of range; expected Err, got {:?}",
            huge_pos,
            res_pos,
        );
        // (b) negative shift whose `as u32` wrap lands in range
        let neg = -((1i64 << 32) - k as i64); // (-2^32 + k) as u32 == k  -> in range
        let ops_neg = vec![reg("b", 0), reg("h", 1), Operand::Imm(neg)];
        let res_neg = encode_neon_scalar_qshrn(&ops_neg, 0, false);
        prop_assert!(
            res_neg.is_err(),
            "shift {} is negative/out of range; expected Err, got {:?}",
            neg,
            res_neg,
        );
    }
}

// --- golden table: absolute oracle vs llvm-mc-18 (witnesses bit-28 bug) ---

#[test]
#[ignore = "bug: every word differs from llvm-mc-18 in bit 28 (0x4F.. vs 0x5F..)"]
fn golden_matches_llvm_mc() {
    for &(mnem, u_bit, is_rounding, dest, dnum, snum, shift, expected) in GOLDEN {
        let got = run(dest, dnum, snum, shift as i64, u_bit, is_rounding)
            .unwrap_or_else(|e| panic!("{mnem} {dest}{dnum}...#{shift}: encode failed: {e}"));
        assert_eq!(
            got, expected,
            "{mnem} {dest}{dnum}, <src{snum}>, #{shift}: got 0x{got:08X}, want 0x{expected:08X} \
             (differs in bit 28: impl=0x4F/0x6F top byte, llvm-mc=0x5F/0x7F)"
        );
    }
}
