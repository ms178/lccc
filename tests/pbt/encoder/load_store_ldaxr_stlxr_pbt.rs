//! Property-based tests for `encode_ldaxr_stlxr`.
//!
//! `encode_ldaxr_stlxr` encodes the A64 exclusive load/store instructions
//! with acquire/release ordering:
//!
//!   * **LDAXR** `<Xt>, [<Xn|SP>]`  — Load-Acquire Exclusive Register
//!     (ARM ARM §C6.2.108), byte/halfword variants LDAXRB/LDAXRH.
//!   * **STLXR** `<Ws>, <Xt>, [<Xn|SP>]` — Store-Release Exclusive Register
//!     (ARM ARM §C6.2.290), byte/halfword variants STLXRB/LSTLXRH.
//!
//! ```text
//!   LDAXR: size 0010000 L=1 0 11111  o0=1  11111  Rn  Rt
//!          [31:30] [29:23] 22 21 [20:16] 15 [14:10] [9:5] [4:0]
//!
//!   STLXR: size 0010000 L=0 0  Rs    o0=1  11111  Rn  Rt
//!          [31:30] [29:23] 22 21 [20:16] 15 [14:10] [9:5] [4:0]
//! ```
//!
//! * `size`  = [31:30]: auto from register width (11 = 64-bit Xt,
//!   10 = 32-bit Wt), or `forced_size` (00 = byte, 01 = halfword).
//! * `L`     = bit 22: 1 for load (LDAXR), 0 for store (STLXR).
//! * `Rs`    = [20:16]: status register for the store form; 11111 for load.
//! * `o0`    = bit 15: 1 for the acquire/release variant — this single bit
//!   is what distinguishes LDAXR/STLXR from the plain LDXR/STXR encodings.
//!
//! This module is **complementary** to the inline
//! `prop_encode_ldxr_stxr_offset_tests` inside `load_store.rs` (which covers
//! the sibling `encode_ldxr_stxr`). It covers field placement, the load/store
//! discriminator, the acquire/release differential against `encode_ldxr_stxr`,
//! and the offset-validation finding.
//!
//! ## Finding — silent immediate-offset drop
//! The exclusive load/store group has **no** immediate-offset addressing form.
//! The ARM ARM permits only `LDAXR <Xt>, [<Xn|SP>]` and
//! `STLXR <Ws>, <Xt>, [<Xn|SP>]` — the effective address is exactly the base
//! register, and the encoding carries no offset field at all. The encoder
//! matches `Operand::Mem { base, .. }` and never inspects `offset`, so a
//! non-zero offset such as `ldaxr x0, [x1, #8]` is silently encoded as
//! `ldaxr x0, [x1]`. This is the same defect family already reported for the
//! sibling `encode_ldxr_stxr` / `encode_ldxp_stxp`.
//!
//! * Witness: `prop_nonzero_offset_rejected` (#[ignore]).
//! * Mechanism documented by: `prop_offset_silently_dropped` (#[ignore]).

use lccc::pbt_internals::EncodeResult;
use lccc::pbt_internals::Operand;
use lccc::pbt_internals::encode_ldaxr_stlxr;
use lccc::pbt_internals::encode_ldxr_stxr; // reference for the acquire/release differential
use proptest::prelude::*;

fn gp_reg(width: char, num: u32) -> Operand {
    // Register 31 is spelled xzr/wzr (the canonical spelling; "x31" is not).
    Operand::Reg(if num == 31 {
        format!("{}zr", width)
    } else {
        format!("{}{}", width, num)
    })
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
/// LDAXR operand list: Rt, [Rn, #off].
fn load_ops(rt_w: char, rt: u32, base: u32, off: i64) -> Vec<Operand> {
    vec![gp_reg(rt_w, rt), mem_op(base, off)]
}
/// STLXR operand list: Ws, Rt, [Rn, #off].
fn store_ops(ws: u32, rt_w: char, rt: u32, base: u32, off: i64) -> Vec<Operand> {
    vec![gp_reg('w', ws), gp_reg(rt_w, rt), mem_op(base, off)]
}
/// Generates `None` (auto-size from register width) or `Some(0..=3)` (forced
/// byte/halfword/etc. size).
fn forced_size_strategy() -> impl Strategy<Value = Option<u32>> {
    prop_oneof![Just(None), (0u32..=3u32).prop_map(Some),]
}

proptest! {
    // ── Property 1: field placement round-trips (reference / spec). ──
    // For all widths, base/status registers, and forced sizes, the emitted
    // word must place size=[31:30], the status reg Rs=[20:16] for the store
    // form (and 11111 for load), the unused Rt2=[14:10]=11111, Rn=[9:5] and
    // Rt=[4:0]. PASSES today.
    #[test]
    fn prop_fields_round_trip(
        is_load in any::<bool>(),
        width_is64 in any::<bool>(),
        rt_num in 0u32..=31u32,
        base_num in 0u32..=31u32,
        ws_num in 0u32..=31u32,
        forced in forced_size_strategy(),
    ) {
        let rt_w = if width_is64 { 'x' } else { 'w' };
        let w = if is_load {
            word(encode_ldaxr_stlxr(&load_ops(rt_w, rt_num, base_num, 0), true, forced))
        } else {
            word(encode_ldaxr_stlxr(&store_ops(ws_num, rt_w, rt_num, base_num, 0), false, forced))
        };
        let expected_size = forced.unwrap_or(if width_is64 { 0b11 } else { 0b10 });

        prop_assert_eq!( w        & 0x1F, rt_num,   "Rt field [4:0]");
        prop_assert_eq!((w >> 5)  & 0x1F, base_num, "Rn field [9:5]");
        prop_assert_eq!((w >> 10) & 0x1F, 0b11111,  "Rt2 [14:10] must be 11111");
        prop_assert_eq!((w >> 30) & 0x3,  expected_size, "size field [31:30]");

        let rs_field = (w >> 16) & 0x1F;
        if is_load {
            prop_assert_eq!(rs_field, 0b11111, "Rs [20:16] must be 11111 on load");
        } else {
            prop_assert_eq!(rs_field, ws_num, "Rs [20:16] must equal status reg on store");
        }
    }

    // ── Property 2: load/store discriminator via the L bit (spec). ──
    // Bit 22 (L) must be 1 for LDAXR and 0 for STLXR, regardless of width or
    // forced size. PASSES today.
    #[test]
    fn prop_load_store_discriminator(
        width_is64 in any::<bool>(),
        forced in forced_size_strategy(),
    ) {
        let rt_w = if width_is64 { 'x' } else { 'w' };
        let ld = word(encode_ldaxr_stlxr(&load_ops(rt_w, 0, 1, 0), true, forced));
        let st = word(encode_ldaxr_stlxr(&store_ops(0, rt_w, 0, 1, 0), false, forced));
        prop_assert_eq!((ld >> 22) & 0x1, 1u32, "LDAXR must set the L bit (22)");
        prop_assert_eq!((st >> 22) & 0x1, 0u32, "STLXR must clear the L bit (22)");
    }

    // ── Property 3: acquire/release differential vs LDXR/STXR (spec). ──
    // LDAXR/STLXR differ from LDXR/STXR by EXACTLY one bit: o0 = bit 15,
    // which selects acquire/release ordering. So, for identical operands,
    //   LDAXR word == LDXR word | (1 << 15)
    //   STLXR word == STXR word | (1 << 15)
    // and they must not differ in any other bit. PASSES today.
    #[test]
    fn prop_acquire_release_differential(
        width_is64 in any::<bool>(),
        rt_num in 0u32..=31u32,
        base_num in 0u32..=31u32,
        ws_num in 0u32..=31u32,
        forced in forced_size_strategy(),
    ) {
        let rt_w = if width_is64 { 'x' } else { 'w' };
        let ldaxr = word(encode_ldaxr_stlxr(&load_ops(rt_w, rt_num, base_num, 0), true, forced));
        let ldxr  = word(encode_ldxr_stxr(&load_ops(rt_w, rt_num, base_num, 0), true, forced));
        prop_assert_eq!(ldaxr, ldxr | (1u32 << 15),
            "LDAXR must equal LDXR with only bit 15 (o0) set");

        let stlxr = word(encode_ldaxr_stlxr(&store_ops(ws_num, rt_w, rt_num, base_num, 0), false, forced));
        let stxr  = word(encode_ldxr_stxr(&store_ops(ws_num, rt_w, rt_num, base_num, 0), false, forced));
        prop_assert_eq!(stlxr, stxr | (1u32 << 15),
            "STLXR must equal STXR with only bit 15 (o0) set");
    }

    // ── Property 4: NEGATIVE CONTRACT — non-zero offset must be Err. ──
    // The exclusive load/store group has no immediate-offset form: the
    // encoding carries no offset field at all. Hence a non-zero immediate
    // offset is UNREPRESENTABLE and MUST be rejected rather than silently
    // encoded as `[Rn]`.
    //
    // ACTIVATED: the encoder now rejects non-zero offsets (the old code
    // matched `Operand::Mem { base, .. }` and dropped the offset).
    #[test]
    fn prop_nonzero_offset_rejected(
        is_load in any::<bool>(),
        rt_num in 0u32..=31u32,
        base_num in 0u32..=31u32,
        ws_num in 0u32..=31u32,
        off in (-32768i64..32767i64).prop_filter("non-zero", |o| *o != 0),
    ) {
        let res = if is_load {
            encode_ldaxr_stlxr(&load_ops('x', rt_num, base_num, off), true, None)
        } else {
            encode_ldaxr_stlxr(&store_ops(ws_num, 'x', rt_num, base_num, off), false, None)
        };
        prop_assert!(res.is_err(),
            "non-zero offset {} on LDAXR/STLXR is unrepresentable (no offset field) \
             and must be rejected; got {:?}", off, res);
    }

    // ── Property 5: the offset is silently discarded (BUG MECHANISM). ──
    // The ARM ARM gives LDAXR/STLXR no offset field, yet the encoder matches
    // `Operand::Mem { base, .. }` and never inspects `offset`. Hence any two
}
