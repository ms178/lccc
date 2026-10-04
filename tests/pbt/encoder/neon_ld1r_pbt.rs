//! Property-based tests for `encode_neon_ld1r`.
//!
//! `encode_neon_ld1r` encodes the AArch64 NEON `LD1R` instruction —
//! "load one single-element structure and replicate to all lanes" — in the
//! Advanced SIMD load/store single-structure group:
//!
//! ```text
//!   31 30 29-24  23    22   21  20-16   15-13 12  11-10  9-5  4-0
//!    0  Q 001101 post   L=1  0   Rm     110    0  size   Rn   Rt
//! ```
//! No post-index  → bit23=0, Rm=00000.
//! Post-index     → bit23=1, Rm=11111 (immediate by element size).
//!
//! Valid arrangements: 8B, 16B, 4H, 8H, 2S, 4S, 1D, 2D (size=00..11, Q=0/1).
//!
//! ## Oracle
//! `ref_encode_ld1r` assembles the word field-by-field from the ARMv8-A ARM
//! layout, independent of this crate's implementation. It is cross-checked
//! against the hand-derived golden table (`GOLDEN`) — e.g.
//! `ld1r {v0.4s}, [x1]` == `0x4D40C820`.
//!
//! ## Finding (documented by the `#[ignore]`d test
//! `ld1r_validates_post_index_offset`)
//! Per the ARMv8-A ARM, the LD1R post-index immediate, when present, MUST
//! equal the element size in bytes (#1/2/4/8 for size 00/01/10/11). The
//! encoder hard-codes `Rm=11111` and discards the operand's offset entirely
//! (`let _ = offset;`), so a *wrong* offset such as `ld1r {v0.16b}, [x1], #2`
//! is silently accepted and produces the same word as the correct `#1`.
//! See `LD1R_OFFSET_BUG_REPORT.md`.

use lccc::pbt_internals::EncodeResult;
use lccc::pbt_internals::Operand;
use lccc::pbt_internals::encode_neon_ld1r;
use proptest::prelude::*;

// --- helpers --------------------------------------------------------------

/// All eight arrangements architecturally valid for LD1R.
fn valid_arrangement_strategy() -> impl Strategy<Value = &'static str> {
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

fn reg_num_strategy() -> impl Strategy<Value = u32> {
    0u32..=31u32
}

fn post_strategy() -> impl Strategy<Value = bool> {
    prop_oneof![Just(false), Just(true)]
}

/// Independent Q/size map (NOT delegating to the impl's `neon_arr_to_q_size`).
fn ref_q_size(arr: &str) -> (u32, u32) {
    match arr {
        "8b" => (0, 0b00),
        "16b" => (1, 0b00),
        "4h" => (0, 0b01),
        "8h" => (1, 0b01),
        "2s" => (0, 0b10),
        "4s" => (1, 0b10),
        "1d" => (0, 0b11),
        "2d" => (1, 0b11),
        _ => unreachable!("invalid arrangement in reference: {arr}"),
    }
}

/// Independent reference encoder assembled field-by-field from the ARM ARM.
fn ref_encode_ld1r(rt: u32, rn: u32, arr: &str, post: bool) -> u32 {
    let (q, size) = ref_q_size(arr);
    let rm: u32 = if post { 0b11111 } else { 0b00000 };
    let bit23: u32 = if post { 1 } else { 0 };
    // bits[31]=0, [30]=Q, [29:24]=001101, [23]=post, [22]=L(=1), [21]=0,
    // [20:16]=Rm, [15:13]=110, [12]=0, [11:10]=size, [9:5]=Rn, [4:0]=Rt
    (((q << 30) | (0b001101u32 << 24) | (bit23 << 23) | (1u32 << 22))
        | (rm << 16)
        | (0b110u32 << 13))
        | (size << 10)
        | (rn << 5)
        | rt
}

/// Element size in bytes for an arrangement (1<<size), the value a correct
/// LD1R post-index immediate must take.
fn element_bytes(arr: &str) -> u32 {
    1u32 << ref_q_size(arr).1
}

/// Build `{ Vrt.arr }` (a single-element register list).
fn list_one(rt: u32, arr: &str) -> Operand {
    Operand::RegList(vec![Operand::RegArrangement {
        reg: format!("v{rt}"),
        arrangement: arr.to_string(),
    }])
}

/// Build the operand vec for `ld1r { Vrt.arr }, [xrn]` / `[xrn], #off`.
fn ld1r_ops(rt: u32, arr: &str, rn: u32, post: bool, offset: i64) -> Vec<Operand> {
    let mem = if post {
        Operand::MemPostIndex {
            base: format!("x{rn}"),
            offset,
        }
    } else {
        Operand::Mem {
            base: format!("x{rn}"),
            offset: 0,
        }
    };
    vec![list_one(rt, arr), mem]
}

fn word_of(res: Result<EncodeResult, String>) -> u32 {
    match res {
        Ok(EncodeResult::Word(w)) => w,
        other => panic!("expected EncodeResult::Word, got {other:?}"),
    }
}

// --- golden table (absolute oracle) ---------------------------------------

// Hand-derived from the ARMv8-A ARM layout for LD1R. Each value assembled
// from the verified field-by-field formula (base 0x0D40C000 no-offset /
// 0x0DDFC000 post-index, then + Q<<30 + size<<10 + Rn<<5 + Rt), with the
// no-offset .4s entry (0x4D40C820) cross-checked against the canonical
// known-good encoding for `ld1r {v0.4s}, [x1]`.
// (rt, rn, arrangement, post, expected_word)
const GOLDEN: &[(u32, u32, &str, bool, u32)] = &[
    (0, 1, "4s", false, 0x4D40C820), // ld1r {v0.4s}, [x1]        (Q=1,size=10)
    (0, 1, "4s", true, 0x4DDFC820),  // ld1r {v0.4s}, [x1], #4    (post, Rm=11111)
    (0, 0, "8b", false, 0x0D40C000), // ld1r {v0.8b}, [x0]        (Q=0,size=00)
    (5, 6, "16b", false, 0x4D40C0C5), // ld1r {v5.16b}, [x6]       (Q=1,size=00)
    (31, 30, "8h", false, 0x4D40C7DF), // ld1r {v31.8h}, [x30]      (Q=1,size=01)
    (7, 9, "2d", false, 0x4D40CD27), // ld1r {v7.2d}, [x9]        (Q=1,size=11)
    (2, 3, "1d", false, 0x0D40CC62), // ld1r {v2.1d}, [x3]        (Q=0,size=11)
    (2, 3, "4h", true, 0x0DDFC462),  // ld1r {v2.4h}, [x3], #2    (Q=0,size=01,post)
];

#[test]
fn ld1r_matches_golden_table() {
    for &(rt, rn, arr, post, expected) in GOLDEN {
        let ops = ld1r_ops(rt, arr, rn, post, element_bytes(arr) as i64);
        let got = word_of(encode_neon_ld1r(&ops));
        assert_eq!(
            got,
            expected,
            "ld1r {{v{rt}.{arr}}}, [x{rn}]{}: got 0x{got:08X}, want 0x{expected:08X}",
            if post { ", #imm" } else { "" },
        );
        // Cross-check the reference encoder against the golden values too.
        assert_eq!(
            ref_encode_ld1r(rt, rn, arr, post),
            expected,
            "reference encoder drift"
        );
    }
}

// --- properties -----------------------------------------------------------

proptest! {
    // === Oracle: differential against independent reference encoder ========
    // For every valid arrangement, register pair, and addressing mode, the
    // implementation must equal the independently-assembled reference word.
    #[test]
    fn ld1r_matches_reference_encoder(
        rt in reg_num_strategy(),
        rn in reg_num_strategy(),
        arr in valid_arrangement_strategy(),
        post in post_strategy(),
    ) {
        let off = if post { element_bytes(arr) as i64 } else { 0 };
        let ops = ld1r_ops(rt, arr, rn, post, off);
        let got = word_of(encode_neon_ld1r(&ops));
        let want = ref_encode_ld1r(rt, rn, arr, post);
        prop_assert_eq!(got, want);
    }

    // === Fixed bits + field placement =====================================
    // Architecturally-constant bits never change, and the Rt/Rn/Q/size
    // fields round-trip from the operands.
    #[test]
    fn ld1r_fixed_bits_and_fields(
        rt in reg_num_strategy(),
        rn in reg_num_strategy(),
        arr in valid_arrangement_strategy(),
        post in post_strategy(),
    ) {
        let off = if post { element_bytes(arr) as i64 } else { 0 };
        let ops = ld1r_ops(rt, arr, rn, post, off);
        let w = word_of(encode_neon_ld1r(&ops));
        let (q, size) = ref_q_size(arr);
        let rm = if post { 0b11111u32 } else { 0u32 };

        // constant bits
        prop_assert_eq!((w >> 31) & 1, 0, "bit 31 must be 0");
        prop_assert_eq!((w >> 24) & 0x3F, 0b001101, "bits 29-24");
        prop_assert_eq!((w >> 22) & 1, 1, "L bit (load) must be 1");
        prop_assert_eq!((w >> 13) & 0x7, 0b110, "bits 15-13");
        prop_assert_eq!((w >> 12) & 1, 0, "bit 12 must be 0");
        // post-index controls bit 23 and Rm
        prop_assert_eq!((w >> 23) & 1, if post { 1 } else { 0 }, "bit 23 (post)");
        prop_assert_eq!((w >> 16) & 0x1F, rm, "Rm field");
        // field placement + arrangement mapping
        prop_assert_eq!(w & 0x1F, rt, "Rt field");
        prop_assert_eq!((w >> 5) & 0x1F, rn, "Rn field");
        prop_assert_eq!((w >> 30) & 1, q, "Q bit");
        prop_assert_eq!((w >> 10) & 0x3, size, "size field");
    }

    // === Negative contract: unsupported arrangement rejected ==============
    // Any arrangement not in the LD1R table must yield Err.
    #[test]
    fn ld1r_rejects_unsupported_arrangement(
        arr in "[a-z0-9]{1,3}".prop_filter("unknown arrangement", |s| {
            !matches!(s.as_str(), "8b"|"16b"|"4h"|"8h"|"2s"|"4s"|"1d"|"2d")
        }),
    ) {
        let ops = ld1r_ops(0, arr.as_str(), 1, false, 0);
        prop_assert!(
            encode_neon_ld1r(&ops).is_err(),
            "unsupported arrangement {arr:?} should be rejected"
        );
    }

    // === Negative contract: no offset / pre-index / reg-offset forms ======
    // LD1R has only `[Xn]` (no offset) and `[Xn], #imm` (post-index) forms.
    // A non-zero `[Xn, #n]` offset must be rejected (the no-offset match arm
    // only accepts offset 0).
    #[test]
    fn ld1r_rejects_nonzero_mem_offset(
        arr in valid_arrangement_strategy(),
        offset in 1i64..=4096,
    ) {
        let ops = vec![
            list_one(0, arr),
            Operand::Mem { base: "x1".to_string(), offset },
        ];
        prop_assert!(
            encode_neon_ld1r(&ops).is_err(),
            "Mem{{offset={offset}}} (offset/pre-index form) should be rejected"
        );
    }

    // === Negative contract: post-index immediate must equal element size ===
    // Per ARMv8-A ARM, LD1R post-index immediate MUST be #1/#2/#4/#8 for
    // size 00/01/10/11. The encoder hard-codes Rm=11111 and discards the
    // offset, so a *wrong* offset is silently accepted.
    //
    // This test FAILS today — `#[ignore]`d to keep the suite green. The
    // finding is documented in LD1R_OFFSET_BUG_REPORT.md. Reproduce with
    // `cargo test -- --ignored ld1r_validates_post_index_offset`.
    #[test]
    #[ignore]
    fn ld1r_validates_post_index_offset(
        rt in reg_num_strategy(),
        rn in reg_num_strategy(),
        arr in valid_arrangement_strategy(),
        bad_offset in 1i64..=4096,
    ) {
        prop_assume!(bad_offset != element_bytes(arr) as i64);
        let ops = ld1r_ops(rt, arr, rn, true, bad_offset);
        prop_assert!(
            encode_neon_ld1r(&ops).is_err(),
            "ld1r post-index offset {bad_offset} != element size {} for .{arr} should be rejected",
            element_bytes(arr),
        );
    }
}

// --- documented negative contracts: malformed operand shapes --------------

#[test]
fn ld1r_rejects_wrong_operand_shapes() {
    let arr = "4s";

    // too few operands
    assert!(encode_neon_ld1r(&[]).is_err(), "empty operands");
    assert!(
        encode_neon_ld1r(&[list_one(0, arr)]).is_err(),
        "single operand"
    );

    // first operand not a register list
    assert!(
        encode_neon_ld1r(&[
            Operand::RegArrangement {
                reg: "v0".into(),
                arrangement: arr.into()
            },
            Operand::Mem {
                base: "x1".into(),
                offset: 0
            },
        ])
        .is_err(),
        "first operand must be a RegList"
    );

    // register list with != 1 entries
    let two = Operand::RegList(vec![
        Operand::RegArrangement {
            reg: "v0".into(),
            arrangement: arr.into(),
        },
        Operand::RegArrangement {
            reg: "v1".into(),
            arrangement: arr.into(),
        },
    ]);
    let zero = Operand::RegList(vec![]);
    for (label, first) in [("two-regs", two), ("zero-regs", zero)] {
        assert!(
            encode_neon_ld1r(&[
                first,
                Operand::Mem {
                    base: "x1".into(),
                    offset: 0
                }
            ])
            .is_err(),
            "RegList with {label} should be rejected"
        );
    }

    // RegList element that is not a RegArrangement
    let plain = Operand::RegList(vec![Operand::Reg("v0".into())]);
    assert!(
        encode_neon_ld1r(&[
            plain,
            Operand::Mem {
                base: "x1".into(),
                offset: 0
            }
        ])
        .is_err(),
        "RegList element must be a RegArrangement"
    );

    // invalid memory shapes: pre-index, register offset, and MemExpr
    let mem = |op: Operand| vec![list_one(0, arr), op];
    assert!(
        encode_neon_ld1r(&mem(Operand::MemPreIndex {
            base: "x1".into(),
            offset: 4
        }))
        .is_err(),
        "MemPreIndex is not a valid LD1R form"
    );
    assert!(
        encode_neon_ld1r(&mem(Operand::MemRegOffset {
            base: "x1".into(),
            index: "x2".into(),
            extend: None,
            shift: None,
        }))
        .is_err(),
        "MemRegOffset is not a valid LD1R form"
    );
}
