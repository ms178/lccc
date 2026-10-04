//! Property-based tests for `encode_neon_ld_st_multi`.
//!
//! `encode_neon_ld_st_multi(operands, is_load, num_structs)` encodes the
//! AArch64 "load/store multiple structures" family (LD1/ST1/LD2/ST2/LD3/ST3/
//! LD4/ST4), including the no-offset and post-index (immediate/register)
//! addressing forms.
//!
//! ## Encoding (ARMv8-A ARM, "Load/store multiple structures")
//! ```text
//!   31  30    29-24     23    22   21    20-16    15-12   11-10   9-5   4-0
//!    0   Q   0011 00   post    L    0      Rm      opcode   size    Rn    Rt
//! ```
//! - `post` (bit 23) = 0 for the offset form, 1 for post-index.
//! - `Rm` (bits 20-16) = `00000` (no post-index), `11111` (immediate
//!   post-index), or the index register (register post-index).
//! - `opcode` (bits 15-12) depends on structure count & (for LD1) reg count.
//!
//! opcode table:
//! ```text
//!   LD1: 1 reg=0111, 2 reg=1010, 3 reg=0110, 4 reg=0010
//!   LD2: 1000   LD3: 0100   LD4: 0000
//! ```
//!
//! ## Oracles
//! 1. **Golden table** — 18 absolute encodings captured from `llvm-mc-18`
//!    (`--arch=aarch64 --show-encoding`). These are the primary independent
//!    oracle and were NOT derived from the SUT.
//! 2. **Reference encoder** — an independently-assembled word (field-by-field
//!    OR) used for differential properties over the generated input space.
//!
//! ## Findings (bug witnesses — all `#[ignore]`d so `cargo test` stays green)
//! 1. `rejects_wrong_immediate_post_index` — the encoder does NOT validate
//!    that a post-index immediate equals the transfer size; any value (e.g.
//!    `#5` for a 16-byte transfer) is silently accepted. `llvm-mc` rejects it
//!    with "invalid operand for instruction".
//! 2. `rejects_register_count_mismatch` — LD2/ST2/LD3/ST3/LD4/ST4 ignore the
//!    length of the register list (`ld2 {v0.8b}` or `ld2 {v0..v2}` encode as
//!    LD2 regardless). The ARM ARM requires the list length to equal the
//!    structure count; `llvm-mc` rejects mismatches.

use lccc::pbt_internals::EncodeResult;
use lccc::pbt_internals::Operand;
use lccc::pbt_internals::encode_neon_ld_st_multi;
use proptest::prelude::*;

// --- helpers --------------------------------------------------------------

/// Build `Operand::RegArrangement { reg: "v{n}", arrangement }`.
fn va(n: u32, arr: &str) -> Operand {
    Operand::RegArrangement {
        reg: format!("v{n}"),
        arrangement: arr.to_string(),
    }
}

/// A consecutive register list `{Vrt.T, V(rt+1).T, ...}` of `nr` entries.
fn reg_list(rt: u32, nr: u32, arr: &str) -> Operand {
    Operand::RegList((0..nr).map(|i| va(rt + i, arr)).collect())
}

fn mem(rn: u32) -> Operand {
    Operand::Mem {
        base: format!("x{rn}"),
        offset: 0,
    }
}

fn mem_post(rn: u32, imm: i64) -> Operand {
    Operand::MemPostIndex {
        base: format!("x{rn}"),
        offset: imm,
    }
}

fn ops_no_offset(rt: u32, nr: u32, arr: &str, rn: u32) -> Vec<Operand> {
    vec![reg_list(rt, nr, arr), mem(rn)]
}

fn ops_imm_post(rt: u32, nr: u32, arr: &str, rn: u32, imm: i64) -> Vec<Operand> {
    vec![reg_list(rt, nr, arr), mem_post(rn, imm)]
}

fn ops_reg_post(rt: u32, nr: u32, arr: &str, rn: u32, rm: u32) -> Vec<Operand> {
    vec![
        reg_list(rt, nr, arr),
        mem(rn),
        Operand::Reg(format!("x{rm}")),
    ]
}

fn reg_num_strategy() -> impl Strategy<Value = u32> {
    0u32..=31u32
}

fn arr_strategy() -> impl Strategy<Value = &'static str> {
    prop_oneof![
        Just("8b"),
        Just("16b"),
        Just("4h"),
        Just("8h"),
        Just("2s"),
        Just("4s"),
        Just("1d"),
        Just("2d"),
    ]
}

/// A valid (num_structs, num_regs, rt, rn, arrangement, is_load) combination.
/// LD1 admits 1-4 registers; LD2/3/4 require the list length to equal the
/// structure count. `rt` is bounded so the consecutive list stays in range.
fn valid_case() -> impl Strategy<Value = (u32, u32, u32, u32, &'static str, bool)> {
    prop_oneof![
        // LD1: 1-4 registers in the list.
        (1u32..=4).prop_flat_map(|nr| (
            Just(1u32),
            Just(nr),
            0u32..=(32 - nr),
            reg_num_strategy(),
            arr_strategy(),
            any::<bool>(),
        )),
        // LD2/LD3/LD4: list length == structure count.
        (2u32..=4).prop_flat_map(|ns| (
            Just(ns),
            Just(ns),
            0u32..=(32 - ns),
            reg_num_strategy(),
            arr_strategy(),
            any::<bool>(),
        )),
    ]
}

/// Spec opcode field for (structure count, register count). Returns `None`
/// for architecturally-invalid combinations.
fn spec_opcode(num_structs: u32, num_regs: u32) -> Option<u32> {
    match num_structs {
        1 => match num_regs {
            1 => Some(0b0111),
            2 => Some(0b1010),
            3 => Some(0b0110),
            4 => Some(0b0010),
            _ => None,
        },
        2 => Some(0b1000),
        3 => Some(0b0100),
        4 => Some(0b0000),
        _ => None,
    }
}

/// Spec (Q, size) for an arrangement string.
fn spec_q_size(arr: &str) -> Option<(u32, u32)> {
    match arr {
        "8b" => Some((0, 0b00)),
        "16b" => Some((1, 0b00)),
        "4h" => Some((0, 0b01)),
        "8h" => Some((1, 0b01)),
        "2s" => Some((0, 0b10)),
        "4s" => Some((1, 0b10)),
        "1d" => Some((0, 0b11)),
        "2d" => Some((1, 0b11)),
        _ => None,
    }
}

/// Independent reference encoder, assembled field-by-field. Structurally
/// distinct from the SUT's single nested-OR expression.
fn ref_encode(
    (rt, rn, q, size, opcode): (u32, u32, u32, u32, u32),
    is_load: bool,
    post: bool,
    rm: u32,
) -> u32 {
    let mut w = 0u32;
    // bit 31 = 0
    w |= (q & 1) << 30;
    w |= 0b001100u32 << 24; // bits 29-24
    w |= (post as u32) << 23;
    w |= (is_load as u32) << 22;
    // bit 21 = 0
    if post {
        w |= (rm & 0x1F) << 16; // Rm only meaningful for post-index
    }
    w |= (opcode & 0xF) << 12;
    w |= (size & 0x3) << 10;
    w |= (rn & 0x1F) << 5;
    w |= rt & 0x1F;
    w
}

fn word_of(res: Result<EncodeResult, String>) -> u32 {
    match res {
        Ok(EncodeResult::Word(w)) => w,
        other => panic!("expected EncodeResult::Word, got {other:?}"),
    }
}

type GoldenRow = (u32, u32, &'static str, u32, u32, bool, u32, i64, u32);

// --- golden table (llvm-mc-18 absolute oracle) ----------------------------
// mode: 0 = no offset, 1 = immediate post-index (aux = imm),
//       2 = register post-index (aux = rm).
// (rt, nr, arr, rn, num_structs, is_load, mode, aux, expected_word)
const GOLDEN: &[GoldenRow] = &[
    (0, 1, "16b", 1, 1, true, 0, 0, 0x4C407020), // ld1  {v0.16b},           [x1]
    (0, 1, "16b", 1, 1, false, 0, 0, 0x4C007020), // st1  {v0.16b},           [x1]
    (0, 1, "8b", 1, 1, true, 0, 0, 0x0C407020),  // ld1  {v0.8b},            [x1]
    (0, 1, "4s", 1, 1, true, 0, 0, 0x4C407820),  // ld1  {v0.4s},            [x1]
    (0, 1, "2d", 1, 1, true, 0, 0, 0x4C407C20),  // ld1  {v0.2d},            [x1]
    (0, 2, "4h", 1, 1, true, 0, 0, 0x0C40A420),  // ld1  {v0.4h, v1.4h},     [x1]
    (0, 4, "16b", 1, 1, true, 0, 0, 0x4C402020), // ld1  {v0..v3.16b},       [x1]
    (0, 2, "4s", 1, 2, true, 0, 0, 0x4C408820),  // ld2  {v0.4s, v1.4s},     [x1]
    (0, 3, "8b", 1, 3, true, 0, 0, 0x0C404020),  // ld3  {v0..v2.8b},        [x1]
    (0, 4, "8b", 1, 4, true, 0, 0, 0x0C400020),  // ld4  {v0..v3.8b},        [x1]
    (0, 4, "16b", 1, 4, false, 0, 0, 0x4C000020), // st4  {v0..v3.16b},       [x1]
    (31, 1, "16b", 30, 1, true, 0, 0, 0x4C4073DF), // ld1  {v31.16b},          [x30]
    (0, 1, "16b", 1, 1, true, 1, 16, 0x4CDF7020), // ld1  {v0.16b}, [x1], #16
    (0, 1, "4s", 1, 1, true, 1, 16, 0x4CDF7820), // ld1  {v0.4s},  [x1], #16
    (0, 1, "16b", 1, 1, true, 2, 2, 0x4CC27020), // ld1  {v0.16b}, [x1], x2
    (0, 2, "4s", 1, 2, true, 2, 3, 0x4CC38820),  // ld2  {v0.4s,v1.4s}, [x1], x3
    (0, 2, "16b", 5, 1, true, 0, 0, 0x4C40A0A0), // ld1  {v0.16b, v1.16b},   [x5]
    (0, 3, "16b", 5, 1, true, 0, 0, 0x4C4060A0), // ld1  {v0..v2.16b},       [x5]
];

#[test]
fn golden_table_matches_llvm_mc() {
    for &(rt, nr, arr, rn, ns, is_load, mode, aux, expected) in GOLDEN {
        let ops = match mode {
            0 => ops_no_offset(rt, nr, arr, rn),
            1 => ops_imm_post(rt, nr, arr, rn, aux),
            2 => ops_reg_post(rt, nr, arr, rn, aux as u32),
            _ => unreachable!(),
        };
        let got = word_of(encode_neon_ld_st_multi(&ops, is_load, ns));
        assert_eq!(
            got, expected,
            "golden case (rt={rt},nr={nr},{arr},rn={rn},ns={ns},load={is_load},mode={mode}): \
             got 0x{got:08X}, want 0x{expected:08X}",
        );

        // Cross-check the reference encoder against the same goldens.
        let (q, size) = spec_q_size(arr).unwrap();
        let opcode = spec_opcode(ns, nr).unwrap();
        let post = mode != 0;
        let rm = if mode == 0 {
            0
        } else if mode == 1 {
            0b11111
        } else {
            aux as u32
        };
        assert_eq!(
            ref_encode((rt, rn, q, size, opcode), is_load, post, rm),
            expected,
            "reference encoder drift on golden case",
        );
    }
}

// --- properties -----------------------------------------------------------

proptest! {
    // === Oracle: differential vs. independent reference (no offset) =======
    #[test]
    fn matches_reference_encoder_no_offset(
        (ns, nr, rt, rn, arr, is_load) in valid_case(),
    ) {
        let ops = ops_no_offset(rt, nr, arr, rn);
        let got = word_of(encode_neon_ld_st_multi(&ops, is_load, ns));
        let (q, size) = spec_q_size(arr).unwrap();
        let opcode = spec_opcode(ns, nr).unwrap();
        let want = ref_encode((rt, rn, q, size, opcode), is_load, false, 0);
        prop_assert_eq!(got, want);
    }

    // === Field placement: Rt/Rn round-trip ===============================
    #[test]
    fn register_fields_round_trip(
        (ns, nr, rt, rn, arr, is_load) in valid_case(),
    ) {
        let ops = ops_no_offset(rt, nr, arr, rn);
        let w = word_of(encode_neon_ld_st_multi(&ops, is_load, ns));
        prop_assert_eq!(w & 0x1F, rt & 0x1F, "Rt field (bits 4-0)");
        prop_assert_eq!((w >> 5) & 0x1F, rn & 0x1F, "Rn field (bits 9-5)");
    }

    // === Fixed-bits & field invariant ====================================
    #[test]
    fn fields_and_fixed_bits_invariant(
        (ns, nr, rt, rn, arr, is_load) in valid_case(),
    ) {
        let ops = ops_no_offset(rt, nr, arr, rn);
        let w = word_of(encode_neon_ld_st_multi(&ops, is_load, ns));
        let (q, size) = spec_q_size(arr).unwrap();
        let opcode = spec_opcode(ns, nr).unwrap();

        prop_assert_eq!(w >> 31, 0, "bit 31 must be 0");
        prop_assert_eq!((w >> 24) & 0x3F, 0b001100, "bits 29-24 must be 001100");
        prop_assert_eq!((w >> 23) & 1, 0, "bit 23 must be 0 (no post-index)");
        prop_assert_eq!((w >> 22) & 1, is_load as u32, "L bit must reflect is_load");
        prop_assert_eq!((w >> 21) & 1, 0, "bit 21 must be 0");
        prop_assert_eq!((w >> 16) & 0x1F, 0, "Rm must be 0 (no post-index)");
        prop_assert_eq!((w >> 30) & 1, q, "Q bit must match arrangement");
        prop_assert_eq!((w >> 12) & 0xF, opcode, "opcode field must match spec table");
        prop_assert_eq!((w >> 10) & 0x3, size, "size field must match arrangement");
    }

    // === Load/store differ ONLY in the L bit =============================
    #[test]
    fn load_store_differ_only_in_l_bit(
        (ns, nr, rt, rn, arr, _is_load) in valid_case(),
    ) {
        let ops = ops_no_offset(rt, nr, arr, rn);
        let ld = word_of(encode_neon_ld_st_multi(&ops, true, ns));
        let st = word_of(encode_neon_ld_st_multi(&ops, false, ns));
        prop_assert_eq!(ld ^ st, 1u32 << 22, "load vs store must differ only at bit 22 (L)");
    }

    // === Immediate post-index: bit 23 set, Rm = 11111 ===================
    #[test]
    fn immediate_post_index_sets_bit23_and_rm_31(
        (ns, nr, rt, rn, arr, is_load) in valid_case(),
    ) {
        // The chosen immediate value is irrelevant to the ENCODING (the SUT
        // does not consume it); correctness of the value is tested separately
        // by the (ignored) `rejects_wrong_immediate_post_index` witness.
        let ops = ops_imm_post(rt, nr, arr, rn, 0);
        let w = word_of(encode_neon_ld_st_multi(&ops, is_load, ns));
        let (q, size) = spec_q_size(arr).unwrap();
        let opcode = spec_opcode(ns, nr).unwrap();
        let want = ref_encode((rt, rn, q, size, opcode), is_load, true, 0b11111);
        prop_assert_eq!(w, want);
        prop_assert_eq!((w >> 23) & 1, 1, "post-index bit 23 must be set");
        prop_assert_eq!((w >> 16) & 0x1F, 0b11111, "Rm must be 11111 (immediate post-index)");
    }

    // === Register post-index: Rm encodes the index register ==============
    #[test]
    fn register_post_index_encodes_rm(
        (ns, nr, rt, rn, arr, is_load) in valid_case(),
        rm in reg_num_strategy(),
    ) {
        let ops = ops_reg_post(rt, nr, arr, rn, rm);
        let w = word_of(encode_neon_ld_st_multi(&ops, is_load, ns));
        let (q, size) = spec_q_size(arr).unwrap();
        let opcode = spec_opcode(ns, nr).unwrap();
        let want = ref_encode((rt, rn, q, size, opcode), is_load, true, rm);
        prop_assert_eq!(w, want);
        prop_assert_eq!((w >> 23) & 1, 1, "post-index bit 23 must be set");
        prop_assert_eq!((w >> 16) & 0x1F, rm, "Rm field must encode the index register");
    }

    // === Error contract: too few operands ================================
    #[test]
    fn rejects_too_few_operands(n in 0usize..2) {
        let ops: Vec<Operand> = (0..n).map(|_| Operand::RegList(vec![])).collect();
        let res = encode_neon_ld_st_multi(&ops, true, 1);
        prop_assert!(res.is_err(), "expected Err for {} operands, got {:?}", n, res);
    }

    // === BUG WITNESS 1: wrong immediate post-index MUST be rejected ======
    // A spec-conformant assembler rejects a post-index immediate that does not
    // equal the total transfer size. `llvm-mc` errors: "invalid operand for
    // instruction". The SUT silently accepts ANY value and encodes Rm=11111.
    #[test]
    #[ignore = "bug witness: post-index immediate is not validated against transfer size"]
    fn rejects_wrong_immediate_post_index(
        (ns, nr, rt, rn, arr, is_load) in valid_case(),
        imm in 1i64..=7, // always < smallest 8-byte transfer => never valid
    ) {
        let ops = ops_imm_post(rt, nr, arr, rn, imm);
        let res = encode_neon_ld_st_multi(&ops, is_load, ns);
        prop_assert!(
            res.is_err(),
            "post-index immediate #{imm} (ns={ns},nr={nr},{arr}) is not a valid transfer \
             size; expected Err, got {:?}",
            res,
        );
    }

    // === BUG WITNESS 2: LD2/3/4 register-count mismatch MUST be rejected =
    // The ARM ARM requires the register list length to equal the structure
    // count for LD2/ST2/LD3/ST3/LD4/ST4. `llvm-mc` rejects mismatches. The SUT
    // ignores `num_regs` for num_structs >= 2 and encodes unconditionally.
    #[test]
    #[ignore = "bug witness: LD2/3/4 do not validate register-list length"]
    fn rejects_register_count_mismatch(
        ns in 2u32..=4,
        nr in 1u32..=4,
    ) {
        prop_assume!(nr != ns);
        let ops = ops_no_offset(0, nr, "8b", 1);
        let res = encode_neon_ld_st_multi(&ops, true, ns);
        prop_assert!(
            res.is_err(),
            "ld{ns} requires exactly {ns} registers but got a list of {nr}; \
             expected Err, got {:?}",
            res,
        );
    }
}
