//! Property-based tests for `encode_ldar_stlr` and `encode_ldxr_stxr`,
//! focused on three input-validation contracts:
//!
//!   1. **Offset validation** — these instruction families carry NO
//!      immediate-offset field; a non-zero offset is unrepresentable and
//!      MUST be rejected.
//!   2. **Register-class checks** — the data register `<Rt>` (and, for
//!      STXR, the status register `<Ws>`) MUST be a general-purpose (W/X)
//!      register. FP/SIMD registers (B/H/S/D/V/Q) are architecturally
//!      UNALLOCATED for these encodings and MUST be rejected.
//!   3. **Size-field validation** — the `size` field is 2 bits
//!      (`[31:30]`); only `0b00..=0b11` are allocated. An out-of-range
//!      `forced_size` (>= 4) MUST be rejected rather than silently wrapped
//!      into the size field (which aliases a different instruction).
//!
//! ## Targets
//!
//! * **LDAR/STLR** (Load-Acquire / Store-Release Register, ARM ARM
//!   §C6.2.101 / §C6.2.275, plus LDARB/STLRB/LDARH/STLRH byte/halfword
//!   forms driven by `forced_size`):
//!   ```text
//!   LDAR/STLR: size 001000 1 L 0 11111 1 11111 Rn Rt   (no offset/V field)
//!   ```
//! * **LDXR/STXR** (Load/Store Exclusive Register, ARM ARM §C6.2.93 /
//!   §C6.2.138, plus LDXRB/STXRB/LDXRH/STXRH driven by `forced_size`):
//!   ```text
//!   LDXR: size 001000 1 0 1 11111 0 11111 Rn Rt         (no offset/V field)
//!   STXR: size 001000 0 0 1 Rs    0 11111 Rn Rt         (no offset/V field)
//!   ```
//!
//! ## Oracle
//!
//! Negative / error Contract (pbt-oracles §4e): out-of-range or
//! wrong-class inputs MUST be rejected with `Err`. Stronger oracles
//! (round-trip, state-machine) do not apply to a one-shot encoder with no
//! invertible inverse in this module; a Reference oracle (golden word
//! cross-checked against the ARM ARM bit layout) is used for the
//! happy-path field-placement properties that pin down the *correct*
//! behaviour the negative contracts are defending.
//!
//! ## Bug witnesses
//!
//! Properties that document confirmed defects are marked
//! `#[ignore = "documented bug: ..."]` so the default `cargo test` run
//! stays green. Run them explicitly to reproduce:
//!
//! ```text
//! cargo test --test pbt_arm_load_store load_store_ldar_ldxr_class_pbt -- --ignored
//! ```
//!
//! The offset-drop and size-truncation defects are already reported for
//! both functions under the transplanted campaign notes; the **register-class**
//! (FP/SIMD acceptance) finding is filed separately per function.

use lccc::pbt_internals::EncodeResult;
use lccc::pbt_internals::Operand;
use lccc::pbt_internals::encode_ldar_stlr;
use lccc::pbt_internals::encode_ldxr_stxr;
use proptest::prelude::*;

// ── helpers ───────────────────────────────────────────────────────────

fn gp_reg(width: char, num: u32) -> Operand {
    // Register 31 is spelled xzr/wzr (the canonical spelling; "x31" is not).
    Operand::Reg(if num == 31 {
        format!("{}zr", width)
    } else {
        format!("{}{}", width, num)
    })
}

/// An FP/SIMD register: B/H/S/D/V/Q prefix + 0..=31.
fn fp_reg(prefix: char, num: u32) -> Operand {
    Operand::Reg(format!("{}{}", prefix, num))
}

fn mem_op(base_num: u32, offset: i64) -> Operand {
    Operand::Mem {
        base: format!("x{}", base_num),
        offset,
    }
}

fn word(r: Result<EncodeResult, String>) -> u32 {
    match r {
        Ok(EncodeResult::Word(w)) => w,
        other => panic!("expected Word, got {:?}", other),
    }
}

/// Generates one FP/SIMD register prefix.
fn fp_prefix_strategy() -> impl Strategy<Value = char> {
    prop_oneof![
        Just('d'),
        Just('s'),
        Just('q'),
        Just('v'),
        Just('h'),
        Just('b'),
    ]
}

/// Generates `None` (auto-size from register width) or `Some(0..=3)`.
fn forced_size_strategy() -> impl Strategy<Value = Option<u32>> {
    prop_oneof![Just(None), (0u32..=3u32).prop_map(Some)]
}

// ── LDAR/STLR operand lists ──
fn ldar_stlr_ops(rt_w: char, rt: u32, base: u32, off: i64) -> Vec<Operand> {
    vec![gp_reg(rt_w, rt), mem_op(base, off)]
}
fn ldar_stlr_ops_reg(rt: Operand, base: u32, off: i64) -> Vec<Operand> {
    vec![rt, mem_op(base, off)]
}

// ── LDXR/STXR operand lists ──
fn ldxr_ops(rt_w: char, rt: u32, base: u32, off: i64) -> Vec<Operand> {
    vec![gp_reg(rt_w, rt), mem_op(base, off)]
}
fn ldxr_ops_reg(rt: Operand, base: u32, off: i64) -> Vec<Operand> {
    vec![rt, mem_op(base, off)]
}
fn stxr_ops(ws: u32, rt_w: char, rt: u32, base: u32, off: i64) -> Vec<Operand> {
    vec![gp_reg('w', ws), gp_reg(rt_w, rt), mem_op(base, off)]
}
fn stxr_ops_reg(ws: Operand, rt: Operand, base: u32, off: i64) -> Vec<Operand> {
    vec![ws, rt, mem_op(base, off)]
}

proptest! {
    // ══════════════════════════════════════════════════════════════════════
    //  encode_ldar_stlr
    // ══════════════════════════════════════════════════════════════════════

    // ── LDAR/STLR happy path: field placement round-trips (reference). ──
    // For valid GP registers, offset 0, and any legal forced_size, the
    // emitted word places Rt=[4:0], Rn=[9:5], size=[31:30], and keeps the
    // two reserved fields [20:16] and [14:10] pinned to 11111. PASSES.
    #[test]
    fn prop_ldar_stlr_fields_round_trip(
        is_load in any::<bool>(),
        width_is64 in any::<bool>(),
        rt_num in 0u32..=31u32,
        base_num in 0u32..=31u32,
        forced in forced_size_strategy(),
    ) {
        let rt_w = if width_is64 { 'x' } else { 'w' };
        let w = word(encode_ldar_stlr(&ldar_stlr_ops(rt_w, rt_num, base_num, 0), is_load, forced));
        let expected_size = forced.unwrap_or(if width_is64 { 0b11 } else { 0b10 });

        prop_assert_eq!( w        & 0x1F, rt_num,        "Rt field [4:0]");
        prop_assert_eq!((w >> 5)  & 0x1F, base_num,      "Rn field [9:5]");
        prop_assert_eq!((w >> 10) & 0x1F, 0b11111,       "Rt2 [14:10] must be 11111");
        prop_assert_eq!((w >> 16) & 0x1F, 0b11111,       "Rs [20:16] must be 11111");
        prop_assert_eq!((w >> 30) & 0x3,  expected_size, "size field [31:30]");
        // L bit [22] = 1 for load, 0 for store.
        prop_assert_eq!((w >> 22) & 0x1, if is_load { 1u32 } else { 0u32 }, "L bit [22]");
    }

    // ── LDAR/STLR size field: auto-detect and override (differential). ──
    // size[31:30] follows forced_size when given, else auto-derives from Rt
    // width (X→11, W→10). PASSES.
    #[test]
    fn prop_ldar_stlr_size_field_auto_and_override(
        is_load in any::<bool>(),
        s in 0u32..=3u32,
        rt_num in 0u32..=31u32,
    ) {
        for (width, auto_want) in [('x', 0b11u32), ('w', 0b10u32)] {
            let ops = ldar_stlr_ops(width, rt_num, 1, 0);
            let forced = word(encode_ldar_stlr(&ops, is_load, Some(s)));
            prop_assert_eq!((forced >> 30) & 0b11, s, "forced_size overrides width");
            let auto = word(encode_ldar_stlr(&ops, is_load, None));
            prop_assert_eq!((auto >> 30) & 0b11, auto_want, "auto-detect from width");
        }
    }

    // ── LDAR/STLR SIZE-FIELD: out-of-range forced_size must be Err. ──
    // size is a 2-bit field; only 0..=3 are allocated. forced_size >= 4 is
    // unallocated and MUST be rejected rather than silently wrapped.
    //
    // This is the negative contract for the size-field aspect. The defect is
    // already reported in
    // the transplanted campaign notes.
    #[test]
        fn prop_ldar_stlr_forced_size_out_of_range_rejected(
        is_load in any::<bool>(),
        width_is64 in any::<bool>(),
        s in 4u32..=255u32,
    ) {
        let rt_w = if width_is64 { 'x' } else { 'w' };
        let res = encode_ldar_stlr(&ldar_stlr_ops(rt_w, 0, 1, 0), is_load, Some(s));
        prop_assert!(res.is_err(),
            "ldar/stlr forced_size={} is outside the 2-bit size range (0..=3) and must \
             return Err, but got {:?} (size<<30 silently wrapped)", s, res);
    }

    // ── LDAR/STLR OFFSET: non-zero offset must be Err. ──
    // The LDAR/STLR encoding carries no offset field at all, so a non-zero
    // immediate offset is unrepresentable and MUST be rejected rather than
    // silently encoded as `[Rn]`.
    //
    // The defect is already reported in
    // the transplanted campaign notes.
    #[test]
        fn prop_ldar_stlr_nonzero_offset_rejected(
        is_load in any::<bool>(),
        rt_num in 0u32..=31u32,
        base_num in 0u32..=31u32,
        off in (-32768i64..32767i64).prop_filter("non-zero", |o| *o != 0),
    ) {
        let res = encode_ldar_stlr(&ldar_stlr_ops('x', rt_num, base_num, off), is_load, None);
        prop_assert!(res.is_err(),
            "ldar/stlr offset {} is unrepresentable (no offset field) and must be \
             rejected; got {:?}", off, res);
    }

    // ── LDAR/STLR REGISTER-CLASS: FP/SIMD Rt must be Err. ──
    // LDAR/STLR only operate on general-purpose registers. An FP/SIMD
    // register (B/H/S/D/V/Q) in the Rt position is architecturally
    // UNALLOCATED and MUST be rejected.
    //
    // ACTIVATED (the defect is fixed): `get_reg` accepts any register
    // `parse_reg_num` recognises (including all FP/SIMD prefixes) and the
    // encoder never checks `is_fp_reg`, so the FP register is silently
    // accepted and aliases to a W-register encoding. This failure IS the bug
    // being reported in
    // the transplanted campaign notes.
    #[test]
        fn prop_ldar_stlr_fp_simd_register_rejected(
        is_load in any::<bool>(),
        prefix in fp_prefix_strategy(),
        rt_num in 0u32..=31u32,
        base_num in 0u32..=31u32,
    ) {
        let rt = fp_reg(prefix, rt_num);
        let res = encode_ldar_stlr(&ldar_stlr_ops_reg(rt, base_num, 0), is_load, None);
        prop_assert!(res.is_err(),
            "ldar/stlr with FP/SIMD register {}{} in the Rt position is UNALLOCATED and \
             must be rejected; got {:?}", prefix, rt_num, res);
    }

    // ── LDAR/STLR REGISTER-CLASS (mechanism): FP/SIMD aliases W form. ──
    // REGRESSION (bug fixed): the exclusives have no FP/SIMD form, so every
    // FP/SIMD spelling must be rejected rather than silently aliasing the
    // W-register encoding (GAS 2.47 and llvm-mc 23.1.2 both reject).
    #[test]
    fn prop_ldar_stlr_rejects_fp_simd_registers(
        is_load in any::<bool>(),
        prefix in fp_prefix_strategy(),
        rt_num in 0u32..=31u32,
    ) {
        let res = encode_ldar_stlr(
            &ldar_stlr_ops_reg(fp_reg(prefix, rt_num), 1, 0), is_load, None);
        prop_assert!(res.is_err(),
            "FP/SIMD register must be rejected (no FP form), got {:?}", res);
    }

    // ══════════════════════════════════════════════════════════════════════
    //  encode_ldxr_stxr
    // ══════════════════════════════════════════════════════════════════════

    // ── LDXR/STXR happy path: field placement round-trips (reference). ──
    // For valid GP registers, offset 0, and any legal forced_size: Rt=[4:0],
    // Rn=[9:5]; for STXR the status Rs=[20:16] and for LDXR Rs=[20:16]=11111;
    // the unused Rt2=[14:10] is pinned to 11111; size=[31:30]. PASSES.
    #[test]
    fn prop_ldxr_stxr_fields_round_trip(
        is_load in any::<bool>(),
        width_is64 in any::<bool>(),
        rt_num in 0u32..=31u32,
        base_num in 0u32..=31u32,
        ws_num in 0u32..=31u32,
        forced in forced_size_strategy(),
    ) {
        let rt_w = if width_is64 { 'x' } else { 'w' };
        let w = if is_load {
            word(encode_ldxr_stxr(&ldxr_ops(rt_w, rt_num, base_num, 0), true, forced))
        } else {
            word(encode_ldxr_stxr(&stxr_ops(ws_num, rt_w, rt_num, base_num, 0), false, forced))
        };
        let expected_size = forced.unwrap_or(if width_is64 { 0b11 } else { 0b10 });

        prop_assert_eq!( w        & 0x1F, rt_num,        "Rt field [4:0]");
        prop_assert_eq!((w >> 5)  & 0x1F, base_num,      "Rn field [9:5]");
        prop_assert_eq!((w >> 10) & 0x1F, 0b11111,       "Rt2 [14:10] must be 11111");
        prop_assert_eq!((w >> 30) & 0x3,  expected_size, "size field [31:30]");

        let rs_field = (w >> 16) & 0x1F;
        if is_load {
            prop_assert_eq!(rs_field, 0b11111, "Rs [20:16] must be 11111 on load");
        } else {
            prop_assert_eq!(rs_field, ws_num, "Rs [20:16] must equal status reg on store");
        }
    }

    // ── LDXR/STXR size field: auto-detect and override (differential). ──
    // PASSES.
    #[test]
    fn prop_ldxr_stxr_size_field_auto_and_override(
        _is_load in any::<bool>(),
        s in 0u32..=3u32,
        rt_num in 0u32..=31u32,
    ) {
        for (width, auto_want) in [('x', 0b11u32), ('w', 0b10u32)] {
            let load_ops = ldxr_ops(width, rt_num, 1, 0);
            let forced = word(encode_ldxr_stxr(&load_ops, true, Some(s)));
            prop_assert_eq!((forced >> 30) & 0b11, s);
            let auto = word(encode_ldxr_stxr(&load_ops, true, None));
            prop_assert_eq!((auto >> 30) & 0b11, auto_want);
        }
    }

    // ── LDXR/STXR SIZE-FIELD: out-of-range forced_size must be Err. ──
    // size is a 2-bit field; forced_size >= 4 is unallocated and MUST be
    // rejected rather than silently wrapped (which aliases a different op).
    //
    // The defect is already reported in
    // the transplanted campaign notes.
    #[test]
        fn prop_ldxr_stxr_forced_size_out_of_range_rejected(
        is_load in any::<bool>(),
        s in 4u32..=255u32,
    ) {
        let ops = ldxr_ops('x', 0, 1, 0);
        let res = encode_ldxr_stxr(&ops, is_load, Some(s));
        prop_assert!(res.is_err(),
            "ldxr/stxr forced_size={} is outside the 2-bit size range (0..=3) and must \
             return Err, but got {:?} (size<<30 silently wrapped)", s, res);
    }

    // ── LDXR/STXR OFFSET: non-zero offset must be Err. ──
    // The LDXR/STXR encoding carries no offset field at all, so a non-zero
    // immediate offset is unrepresentable and MUST be rejected.
    //
    // The defect is already reported in
    // the transplanted campaign notes.
    #[test]
        fn prop_ldxr_stxr_nonzero_offset_rejected(
        is_load in any::<bool>(),
        rt_num in 0u32..=31u32,
        base_num in 0u32..=31u32,
        ws_num in 0u32..=31u32,
        off in (-32768i64..32767i64).prop_filter("non-zero", |o| *o != 0),
    ) {
        let res = if is_load {
            encode_ldxr_stxr(&ldxr_ops('x', rt_num, base_num, off), true, None)
        } else {
            encode_ldxr_stxr(&stxr_ops(ws_num, 'x', rt_num, base_num, off), false, None)
        };
        prop_assert!(res.is_err(),
            "ldxr/stxr offset {} is unrepresentable (no offset field) and must be \
             rejected; got {:?}", off, res);
    }

    // ── LDXR/STXR REGISTER-CLASS: FP/SIMD Rt must be Err (load form). ──
    // LDXR `<Rt>, [<Xn|SP>]` requires a GP register. An FP/SIMD register in
    // the Rt position is UNALLOCATED and MUST be rejected.
    //
    // EXPECTED TO FAIL today. Reported in
    // the transplanted campaign notes.
    #[test]
        fn prop_ldxr_fp_simd_register_rejected(
        prefix in fp_prefix_strategy(),
        rt_num in 0u32..=31u32,
        base_num in 0u32..=31u32,
    ) {
        let rt = fp_reg(prefix, rt_num);
        let res = encode_ldxr_stxr(&ldxr_ops_reg(rt, base_num, 0), true, None);
        prop_assert!(res.is_err(),
            "ldxr with FP/SIMD register {}{} in the Rt position is UNALLOCATED and \
             must be rejected; got {:?}", prefix, rt_num, res);
    }

    // ── STXR REGISTER-CLASS: FP/SIMD status and/or Rt must be Err. ──
    // STXR `<Ws>, <Rt>, [<Xn|SP>]` requires GP registers in BOTH the status
    // and data positions. An FP/SIMD register in either slot is UNALLOCATED
    // and MUST be rejected.
    //
    // EXPECTED TO FAIL today. Reported in
    // the transplanted campaign notes.
    #[test]
        fn prop_stxr_fp_simd_register_rejected(
        ws_fp in any::<bool>(),
        rt_fp in any::<bool>(),
        ws_prefix in fp_prefix_strategy(),
        rt_prefix in fp_prefix_strategy(),
        ws_num in 0u32..=31u32,
        rt_num in 0u32..=31u32,
        base_num in 0u32..=31u32,
    ) {
        prop_assume!(ws_fp || rt_fp, "at least one FP/SIMD operand");
        let ws = if ws_fp { fp_reg(ws_prefix, ws_num) } else { gp_reg('w', ws_num) };
        let rt = if rt_fp { fp_reg(rt_prefix, rt_num) } else { gp_reg('x', rt_num) };
        let res = encode_ldxr_stxr(&stxr_ops_reg(ws, rt, base_num, 0), false, None);
        prop_assert!(res.is_err(),
            "stxr with FP/SIMD register in the status and/or data position is UNALLOCATED \
             and must be rejected; got {:?}", res);
    }

    // REGRESSION (bug fixed): LDXR takes no FP/SIMD register.
    #[test]
    fn prop_ldxr_rejects_fp_simd_registers(
        prefix in fp_prefix_strategy(),
        rt_num in 0u32..=31u32,
    ) {
        let res = encode_ldxr_stxr(
            &ldxr_ops_reg(fp_reg(prefix, rt_num), 1, 0), true, None);
        prop_assert!(res.is_err(),
            "FP/SIMD register must be rejected (no FP form), got {:?}", res);
    }
}
