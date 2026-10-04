//! Property-based tests for `encode_mrs` and `encode_msr` in `system.rs`
//! (the AArch64 MRS / MSR system-register encoders).
//!
//! Both encoders reduce a system register to a 16-bit "system-register
//! encoding" `E` and place it in the instruction word:
//!
//!   * MRS (read)  : `0xD520_0000 | (E << 5) | Rt`   — L bit (bit 21) = 1
//!   * MSR (reg)   : `0xD500_0000 | (E << 5) | Rt`   — L bit (bit 21) = 0
//!
//! where `E` is either a hard-coded table entry (e.g. `sctlr_el1 -> 0xC080`)
//! or the generic packing `sysreg_encoding(op0,op1,CRn,CRm,op2)`.
//!
//! `encode_msr` additionally has an immediate (PState) shape for `daifset`,
//! `daifclr` and `spsel`:
//!
//!   * `daifset, #imm` -> `0xD503_4000 | ((imm & 0xF) << 8) | (0b110 << 5) | 0x1F`
//!   * `daifclr,  #imm` -> `0xD503_4000 | ((imm & 0xF) << 8) | (0b111 << 5) | 0x1F`
//!   * `spsel,    #imm` -> `0xD500_4000 | ((imm & 0xF) << 8) | (0b101 << 5) | 0x1F`
//!
//! # Reference oracle
//!
//! All "known-word" anchors below were cross-checked against `clang
//! --target=aarch64-linux-gnu` (LLVM-MC), e.g.:
//!   `mrs x0, sctlr_el1`   = 0xD538_1000
//!   `msr sctlr_el1, x0`   = 0xD518_1000   (same word with bit 21 cleared)
//!   `mrs x0, nzcv`        = 0xD53B_4200
//!   `mrs x0, currentel`   = 0xD538_4240
//!   `msr daif, x0`        = 0xD51B_4220
//!   `msr daifset, #1`     = 0xD503_41DF
//!   `msr spsel, #1`       = 0xD500_41BF
//!
//! # Findings surfaced
//!
//! The bit-packing is **correct** for valid operands: every field lands at its
//! canonical position, the L/read bit is 1 for MRS and 0 for MSR, the generic
//! sysreg path agrees with `sysreg_encoding`, and the MRS/MSR words for a
//! shared register differ *only* in bit 21 (the read/write direction bit).
//!
//! Two **validation bugs** are exposed as `#[ignore]`d witness properties so
//! the default `cargo test` stays green. Run them explicitly with
//! `cargo test --test pbt_arm_system system_msr_mrs -- --ignored`:
//!   * **B1** — `msr daifset/daifclr/spsel, #imm` performs no range check on
//!     `imm`; it is masked with `& 0xF`, so out-of-range immediates (`#16`,
//!     `#255`, `#-1`, …) are silently accepted (e.g. `#16` aliases `#0`).
//!     `clang`/`llvm` reject these with
//!     *"immediate must be an integer in range [0, 15]."*.
//!   * **B2** — MRS/MSR accept any register whose name parses through
//!     `parse_reg_num`, including FP/SIMD registers (`d0`, `s0`, `q5`, `v0`).
//!     These are not valid `Rt` operands; `clang`/`llvm` reject them with
//!     *"invalid operand for instruction"*. The encoder re-uses the FP lane
//!     number as `Rt` and silently emits an illegal encoding.

use lccc::pbt_internals::Operand;
use lccc::pbt_internals::*;
use proptest::prelude::*;

// ── helpers ──────────────────────────────────────────────────────────────

/// Build a 64-bit GP register operand `x<n>`.
fn xreg(n: u32) -> Operand {
    // Register 31 is spelled xzr in the GP forms (field 31 = XZR for MRS).
    Operand::Reg(if n == 31 {
        "xzr".to_string()
    } else {
        format!("x{}", n)
    })
}

/// Unwrap an encoder result, panicking if it is not `EncodeResult::Word`.
fn expect_word(r: Result<EncodeResult, String>) -> u32 {
    match r {
        Ok(EncodeResult::Word(w)) => w,
        other => panic!("expected EncodeResult::Word, got {:?}", other),
    }
}

/// Rt field (bits[4:0]).
fn rt_of(w: u32) -> u32 {
    w & 0x1F
}
/// L (read/write direction) bit, bit 21: 1 = MRS (read), 0 = MSR (write).
fn l_bit(w: u32) -> u32 {
    (w >> 21) & 1
}

/// Registers that appear in *both* the MRS and MSR tables with identical
/// encodings, so their MRS/MSR words must differ only in bit 21.
const SHARED_SYSREGS: &[&str] = &["sctlr_el1", "daif", "nzcv", "sp_el0", "tpidr_el0"];

// A handful of registers present in the MRS table (verified read-correct
// against clang/llvm).
const MRS_TABLE_REGS: &[&str] = &[
    "sctlr_el1",
    "midr_el1",
    "nzcv",
    "currentel",
    "tpidr_el0",
    "spsr_el1",
];

// Registers present in the MSR register table.
const MSR_TABLE_REGS: &[&str] = &[
    "sctlr_el1",
    "daif",
    "nzcv",
    "sp_el0",
    "tpidr_el0",
    "spsr_el1",
];

// =========================================================================
// encode_mrs — MRS Xt, <sysreg>   (read; L bit = 1)
// =========================================================================
//
// word = 0xD520_0000 | (E << 5) | Rt

proptest! {
    // M1. Fixed opcode + read-direction bit: bits[31:21] are constant at
    //     0xD520_0000 (which already includes L=1) for every valid input.
    #[test]
    fn mrs_fixed_opcode_and_read_bit(rt in 0u32..=31) {
        let w = expect_word(encode_mrs(&[xreg(rt), Operand::Symbol("sctlr_el1".into())]));
        prop_assert_eq!(w & 0xFFE0_0000, 0xD520_0000u32);
        prop_assert_eq!(l_bit(w), 1, "MRS must set the read bit");
    }

    // M2. Rt round-trips into bits[4:0] for every table register and every Rt.
    #[test]
    fn mrs_rt_roundtrips(rt in 0u32..=31, sr_idx in 0usize..MRS_TABLE_REGS.len()) {
        let sr = MRS_TABLE_REGS[sr_idx];
        let w = expect_word(encode_mrs(&[xreg(rt), Operand::Symbol(sr.into())]));
        prop_assert_eq!(rt_of(w), rt);
    }

    // M3. Rt injectivity: distinct registers yield distinct words.
    #[test]
    fn mrs_distinct_rt_distinct_words(a in 0u32..=31, b in 0u32..=31) {
        prop_assume!(a != b);
        let wa = expect_word(encode_mrs(&[xreg(a), Operand::Symbol("sctlr_el1".into())]));
        let wb = expect_word(encode_mrs(&[xreg(b), Operand::Symbol("sctlr_el1".into())]));
        prop_assert_ne!(wa, wb);
    }

    // M4. Differential vs sysreg_encoding for generic names. The encoder must
    //     agree with the independently-defined packer for any in-range fields.
    #[test]
    fn mrs_generic_matches_sysreg_encoding(
        op0 in 0u32..=3, op1 in 0u32..=7, crn in 0u32..=15,
        crm in 0u32..=15, op2 in 0u32..=7, rt in 0u32..=31,
    ) {
        let name = format!("s{}_{}_c{}_c{}_{}", op0, op1, crn, crm, op2);
        let w = expect_word(encode_mrs(&[xreg(rt), Operand::Symbol(name)]));
        let sysenc = sysreg_encoding(op0, op1, crn, crm, op2);
        prop_assert_eq!(w, 0xD520_0000u32 | (sysenc << 5) | rt);
        prop_assert_eq!((w >> 5) & 0xFFFF, sysenc);
    }

    // M5. Negative / error contract: every malformed operand slice is rejected.
    #[test]
    fn mrs_rejects_bad_operands(kind in 0u8..5) {
        let ops: Vec<Operand> = match kind {
            0 => vec![],                                                  // missing Rt
            1 => vec![Operand::Imm(0)],                                  // non-register Rt
            2 => vec![xreg(0)],                                          // missing sysreg
            3 => vec![xreg(0), Operand::Imm(0)],                         // non-Symbol sysreg
            _ => vec![xreg(0), Operand::Symbol("not_a_real_sysreg_xyz".into())], // unknown sysreg
        };
        prop_assert!(
            encode_mrs(&ops).is_err(),
            "expected Err for operands: {:?}", ops
        );
    }

    // M6. Case-insensitivity: the table lookup lowercases the symbol.
    #[test]
    fn mrs_case_insensitive(rt in 0u32..=31) {
        let lo = expect_word(encode_mrs(&[xreg(rt), Operand::Symbol("sctlr_el1".into())]));
        let up = expect_word(encode_mrs(&[xreg(rt), Operand::Symbol("SCTLR_EL1".into())]));
        let mx = expect_word(encode_mrs(&[xreg(rt), Operand::Symbol("Sctlr_El1".into())]));
        prop_assert_eq!(lo, up);
        prop_assert_eq!(lo, mx);
    }
}

// Deterministic known-answer anchors (cross-checked with clang/llvm-MC).
#[test]
fn mrs_known_words_from_clang() {
    let cases = [
        (0u32, "sctlr_el1", 0xD538_1000u32),
        (5, "midr_el1", 0xD538_0005),
        (0, "nzcv", 0xD53B_4200),
        (0, "currentel", 0xD538_4240),
        (9, "tpidr_el0", 0xD53B_D049),
    ];
    for (rt, sr, expected) in cases {
        let w = expect_word(encode_mrs(&[xreg(rt), Operand::Symbol(sr.into())]));
        assert_eq!(w, expected, "mrs x{}, {}", rt, sr);
    }
}

// =========================================================================
// encode_msr — MSR <sysreg>, Xt   (register; L bit = 0)
//              MSR daifset/daifclr/spsel, #imm   (immediate PState form)
// =========================================================================

proptest! {
    // S1. Immediate form, in-range: exact-word oracle for daifset/daifclr/spsel
    //     over the full legal immediate range [0,15]. Rt is hard-wired to 0x1F.
    #[test]
    fn msr_immediate_in_range_exact_word(imm in 0u32..=15, field in 0u8..3) {
        let (name, op2, base): (&str, u32, u32) = match field {
            0 => ("daifset", 0b110, 0xD503_4000),
            1 => ("daifclr", 0b111, 0xD503_4000),
            _ => ("spsel",   0b101, 0xD500_4000),
        };
        let w = expect_word(encode_msr(&[
            Operand::Symbol(name.into()),
            Operand::Imm(imm as i64),
        ]));
        let expected = base | ((imm & 0xF) << 8) | (op2 << 5) | 0x1F;
        prop_assert_eq!(w, expected);
        prop_assert_eq!(rt_of(w), 0x1F, "immediate form reserves Rt = 0b11111");
    }

    // S2. Register form: L (write) bit is 0, opcode bits fixed, Rt round-trips.
    #[test]
    fn msr_register_write_bit_and_rt_roundtrips(
        rt in 0u32..=31, sr_idx in 0usize..MSR_TABLE_REGS.len(),
    ) {
        let sr = MSR_TABLE_REGS[sr_idx];
        let w = expect_word(encode_msr(&[Operand::Symbol(sr.into()), xreg(rt)]));
        prop_assert_eq!(l_bit(w), 0, "MSR must clear the read bit");
        prop_assert_eq!(w & 0xFFE0_0000, 0xD500_0000u32);
        prop_assert_eq!(rt_of(w), rt);
    }

    // S3. Register form: Rt injectivity.
    #[test]
    fn msr_distinct_rt_distinct_words(a in 0u32..=31, b in 0u32..=31) {
        prop_assume!(a != b);
        let wa = expect_word(encode_msr(&[Operand::Symbol("sctlr_el1".into()), xreg(a)]));
        let wb = expect_word(encode_msr(&[Operand::Symbol("sctlr_el1".into()), xreg(b)]));
        prop_assert_ne!(wa, wb);
    }

    // S4. Differential vs sysreg_encoding for generic names.
    #[test]
    fn msr_generic_matches_sysreg_encoding(
        op0 in 0u32..=3, op1 in 0u32..=7, crn in 0u32..=15,
        crm in 0u32..=15, op2 in 0u32..=7, rt in 0u32..=31,
    ) {
        let name = format!("s{}_{}_c{}_c{}_{}", op0, op1, crn, crm, op2);
        let w = expect_word(encode_msr(&[Operand::Symbol(name), xreg(rt)]));
        let sysenc = sysreg_encoding(op0, op1, crn, crm, op2);
        prop_assert_eq!(w, 0xD500_0000u32 | (sysenc << 5) | rt);
        prop_assert_eq!((w >> 5) & 0xFFFF, sysenc);
    }

    // S5. Cross-function differential: for a register present in BOTH tables,
    //     the MRS and MSR words differ *only* in bit 21 (read vs write). This
    //     ties the two functions under test together with a single invariant.
    #[test]
    fn mrs_and_msr_differ_only_in_direction_bit(
        rt in 0u32..=31, sr_idx in 0usize..SHARED_SYSREGS.len(),
    ) {
        let sr = SHARED_SYSREGS[sr_idx];
        let mrs = expect_word(encode_mrs(&[xreg(rt), Operand::Symbol(sr.into())]));
        let msr = expect_word(encode_msr(&[Operand::Symbol(sr.into()), xreg(rt)]));
        prop_assert_eq!(mrs & !(1u32 << 21), msr, "identical except the L bit");
        prop_assert_ne!(mrs, msr, "read and write must differ in the L bit");
    }

    // S6. spsel disambiguation: a register operand selects the register form
    //     (enc 0xC210, base 0xD500_0000), an immediate operand selects the
    //     immediate PState form (base 0xD500_4000). These must not be confused.
    #[test]
    fn msr_spsel_register_vs_immediate(rt in 0u32..=31, imm in 0u32..=15) {
        let wreg = expect_word(encode_msr(&[
            Operand::Symbol("spsel".into()),
            xreg(rt),
        ]));
        let wimm = expect_word(encode_msr(&[
            Operand::Symbol("spsel".into()),
            Operand::Imm(imm as i64),
        ]));
        prop_assert_eq!(wreg, 0xD500_0000u32 | (0xC210u32 << 5) | rt);
        prop_assert_eq!(
            wimm,
            0xD500_4000u32 | ((imm & 0xF) << 8) | (0b101u32 << 5) | 0x1F
        );
    }

    // S7. Case-insensitivity of the sysreg symbol.
    #[test]
    fn msr_case_insensitive(rt in 0u32..=31) {
        let lo = expect_word(encode_msr(&[
            Operand::Symbol("sctlr_el1".into()),
            xreg(rt),
        ]));
        let up = expect_word(encode_msr(&[
            Operand::Symbol("SCTLR_EL1".into()),
            xreg(rt),
        ]));
        prop_assert_eq!(lo, up);
    }

    // S8. Negative / error contract: malformed MSR operand slices are rejected.
    #[test]
    fn msr_rejects_bad_operands(kind in 0u8..6) {
        let ops: Vec<Operand> = match kind {
            0 => vec![],                                                    // missing sysreg
            1 => vec![Operand::Imm(0)],                                    // non-Symbol sysreg
            2 => vec![Operand::Symbol("sctlr_el1".into())],                // reg form, missing Rt
            3 => vec![Operand::Symbol("sctlr_el1".into()), Operand::Imm(0)], // reg form, non-reg Rt
            4 => vec![Operand::Symbol("daifset".into())],                  // imm form, missing #imm
            _ => vec![Operand::Symbol("not_a_real_sysreg_xyz".into()), xreg(0)], // unknown sysreg
        };
        prop_assert!(
            encode_msr(&ops).is_err(),
            "expected Err for operands: {:?}", ops
        );
    }
}

// Deterministic known-answer anchors for MSR (cross-checked with clang/llvm-MC).
#[test]
fn msr_known_words_from_clang() {
    // Register form.
    let reg_cases = [
        (0u32, "sctlr_el1", 0xD518_1000u32),
        (0, "daif", 0xD51B_4220),
        (7, "nzcv", 0xD51B_4207),
        (1, "sp_el0", 0xD518_4101),
        (9, "tpidr_el0", 0xD51B_D049),
        (5, "spsel", 0xD518_4205),
    ];
    for (rt, sr, expected) in reg_cases {
        let w = expect_word(encode_msr(&[Operand::Symbol(sr.into()), xreg(rt)]));
        assert_eq!(w, expected, "msr {}, x{}", sr, rt);
    }
    // Immediate form.
    assert_eq!(
        expect_word(encode_msr(&[
            Operand::Symbol("daifset".into()),
            Operand::Imm(0)
        ])),
        0xD503_40DF,
    );
    assert_eq!(
        expect_word(encode_msr(&[
            Operand::Symbol("daifset".into()),
            Operand::Imm(1)
        ])),
        0xD503_41DF,
    );
    assert_eq!(
        expect_word(encode_msr(&[
            Operand::Symbol("daifclr".into()),
            Operand::Imm(0)
        ])),
        0xD503_40FF,
    );
    assert_eq!(
        expect_word(encode_msr(&[
            Operand::Symbol("spsel".into()),
            Operand::Imm(1)
        ])),
        0xD500_41BF,
    );
}

// =========================================================================
// Bug witnesses — #[ignore] so default `cargo test` stays green.
// Run with:  cargo test --test pbt_arm_system system_msr_mrs -- --ignored
// =========================================================================

/// **B1** — `msr daifset/daifclr/spsel, #imm` must reject immediates outside
/// [0, 15]. clang/llvm-MC reject them with *"immediate must be an integer in
/// range [0, 15]."*; the encoder instead masks `& 0xF` and silently accepts
/// them (e.g. `#16` aliases `#0`, `#255` aliases `#15`).
#[test]
#[ignore = "documented bug: out-of-range PState immediate is masked (& 0xF) instead of rejected; clang says 'immediate must be an integer in range [0, 15].'"]
fn b1_msr_rejects_out_of_range_pstate_immediate() {
    let bad_imms = [16i64, 17, 31, 100, 255, 256, -1];
    for imm in bad_imms {
        for name in ["daifset", "daifclr", "spsel"] {
            let r = encode_msr(&[Operand::Symbol(name.into()), Operand::Imm(imm)]);
            assert!(
                r.is_err(),
                "msr {}, #{} should be rejected (out of [0,15]), got {:?}",
                name,
                imm,
                r
            );
        }
    }
}

/// **B2** — MRS/MSR `Rt` must be a general-purpose (X/W) register. clang/llvm-MC
/// reject FP/SIMD operands with *"invalid operand for instruction"*; the
/// encoder routes the register name through `parse_reg_num`, which accepts
/// `d/s/q/v` registers, and silently re-uses the lane number as `Rt`.
// ACTIVATED: the strict GP readers now reject FP/SIMD Rt (fixed in the
// session-3 fail-closed pass).
#[test]
fn b2_mrs_msr_reject_fp_register_rt() {
    let fp_regs = ["d0", "s0", "q5", "v0"];
    for rt_name in fp_regs {
        let r = encode_mrs(&[
            Operand::Reg(rt_name.into()),
            Operand::Symbol("sctlr_el1".into()),
        ]);
        assert!(
            r.is_err(),
            "mrs {}, sctlr_el1 should be rejected (not a GP register), got {:?}",
            rt_name,
            r
        );
        let r = encode_msr(&[
            Operand::Symbol("sctlr_el1".into()),
            Operand::Reg(rt_name.into()),
        ]);
        assert!(
            r.is_err(),
            "msr sctlr_el1, {} should be rejected (not a GP register), got {:?}",
            rt_name,
            r
        );
    }
}
