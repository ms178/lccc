//! Property-based tests for `encode_neon_ldnr` (the LD2R/LD3R/LD4R — and
//! LD1R — "load single structure and replicate to all lanes" encoder).
//!
//! Field layout per ARM ARM "Advanced SIMD load/store single structure"
//! (verified against `llvm-mc-18 -triple=aarch64 -show-encoding`):
//!
//! ```text
//!  31 30 29 28:24 23  22  21  20:16  15:13 12  11:10  9:5  4:0
//!   0  Q  0  01101 post L   R   Rm    opcode S  size   Rn   Rt
//! ```
//!
//! The structure count is selected by the **R bit (bit 21)**, NOT by S:
//!   LD1R: R=0, opcode=110, S=0
//!   LD2R: R=1, opcode=110, S=0
//!   LD3R: R=0, opcode=111, S=0
//!   LD4R: R=1, opcode=111, S=0
//!
//! Golden encodings (llvm-mc-18):
//!   ld2r {v0.16b, v1.16b},          [x1]      -> 0x4D60C020
//!   ld3r {v2.4s, v3.4s, v4.4s},     [x5]      -> 0x4D40E8A2
//!   ld4r {v6.8h..v9.8h},            [x10]     -> 0x4D60E546
//!   ld3r {v14.2d..v16.2d},          [x17]     -> 0x4D40EE2E
//!   ld4r {v18.8b..v21.8b}, [x22], #4          -> 0x0DFFE2D2

use lccc::pbt_internals::EncodeResult;
use lccc::pbt_internals::Operand;
use lccc::pbt_internals::encode_neon_ldnr;
use proptest::prelude::*;

// --- helpers --------------------------------------------------------------

/// Valid register-replicate arrangements.
const ARRANGEMENTS: &[&str] = &["8b", "16b", "4h", "8h", "2s", "4s", "1d", "2d"];

/// Independent Q/size map (does NOT reuse the impl's `neon_arr_to_q_size`).
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

/// Independent reference encoder built field-by-field from the ARM ARM.
/// Uses R (bit 21) to select the structure count; S (bit 12) is always 0.
fn ref_encode_ldnr(rt: u32, rn: u32, arr: &str, num_structs: u32, post: bool) -> u32 {
    let (q, size) = ref_q_size(arr);
    let rm: u32 = if post { 0b11111 } else { 0b00000 };
    let bit23: u32 = if post { 1 } else { 0 };
    let (opcode, r) = match num_structs {
        1 => (0b110u32, 0u32),
        2 => (0b110, 1),
        3 => (0b111, 0),
        4 => (0b111, 1),
        _ => unreachable!("ref num_structs out of range: {num_structs}"),
    };
    (((q << 30)
        | (0b001101u32 << 24)
        | (bit23 << 23)
        | (1u32 << 22)
        | (r << 21)) // bit 20 unused
        | (rm << 16)
        | (opcode << 13)) // S is always 0 for the LDnR replicate group
        | (size << 10)
        | (rn << 5)
        | rt
}

/// Build `{ v{rt}.arr, v{rt+1}.arr, ... }` with `n` consecutive registers.
fn list_of(n: u32, start_rt: u32, arr: &str) -> Operand {
    let regs = (0..n)
        .map(|i| Operand::RegArrangement {
            reg: format!("v{}", start_rt.wrapping_add(i)),
            arrangement: arr.to_string(),
        })
        .collect();
    Operand::RegList(regs)
}

/// Build the operand vec for `ld{n}r { ... }, [xrn]` / `[xrn], #off`.
fn ldnr_ops(rt: u32, arr: &str, n: u32, rn: u32, post: bool, offset: i64) -> Vec<Operand> {
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
    vec![list_of(n, rt, arr), mem]
}

fn word_of(res: Result<EncodeResult, String>) -> u32 {
    match res {
        Ok(EncodeResult::Word(w)) => w,
        other => panic!("expected EncodeResult::Word, got {other:?}"),
    }
}

// --- properties -----------------------------------------------------------

proptest! {
    // === Oracle: differential / reference ================================
    // For every valid (Rt, Rn, arrangement, structure count, post-index) the
    // encoder's output must equal an independent field-by-field reconstruction
    // from the ARM ARM. NOTE: this currently FAILS for LD2R and LD4R — see
    // the bug report. LD1R (num_structs=1) and LD3R (num_structs=3) pass.
    #[test]
    #[ignore = "documented bug: LD2R/LD4R use swapped R/S bits"]
    fn prop_matches_arm_reference_encoding(
        rt in 0u32..=31u32,
        rn in 1u32..=31u32, // x0/reserved excluded to keep rn meaningful
        arr_idx in 0usize..ARRANGEMENTS.len(),
        num_structs in 1u32..=4u32,
        post in any::<bool>(),
    ) {
        // keep the register list inside the architectural range
        let arr = ARRANGEMENTS[arr_idx];
        let start_rt = rt.min(31u32.saturating_sub(num_structs - 1));
        let offset = match arr {
            "8b" | "16b" => num_structs as i64,
            "4h" | "8h" => num_structs as i64 * 2,
            "2s" | "4s" => num_structs as i64 * 4,
            _ => num_structs as i64 * 8,
        };
        let ops = ldnr_ops(start_rt, arr, num_structs, rn, post, offset);
        let got = word_of(encode_neon_ldnr(&ops, num_structs));
        let want = ref_encode_ldnr(start_rt, rn, arr, num_structs, post);
        prop_assert_eq!(got, want,
            "\nLD{}R {{..}}.{} [x{}]{}: got {:#010X} want {:#010X} \
             (bit21/R={}, bit12/S={})",
            num_structs, arr, rn, if post { ",#post" } else { "" },
            got, want, (got >> 21) & 1, (got >> 12) & 1);
    }

    // === Oracle: invariant — fixed top bits & L bit ======================
    // Independent of the (buggy) structure-count field, the fixed opcode
    // skeleton is correct: bit31=0, bits[29:24]=001101, L(bit22)=1, bit29=0.
    #[test]
    fn prop_fixed_top_bits_invariant(
        rt in 0u32..=31u32,
        rn in 1u32..=31u32,
        arr_idx in 0usize..ARRANGEMENTS.len(),
        num_structs in 2u32..=4u32,
        post in any::<bool>(),
    ) {
        let arr = ARRANGEMENTS[arr_idx];
        let start_rt = rt.min(31u32.saturating_sub(num_structs - 1));
        let ops = ldnr_ops(start_rt, arr, num_structs, rn, post, num_structs as i64);
        let w = word_of(encode_neon_ldnr(&ops, num_structs));
        prop_assert_eq!((w >> 31) & 1, 0u32, "bit31 must be 0");
        prop_assert_eq!((w >> 24) & 0b111111, 0b001101u32, "bits[29:24]=001101");
        prop_assert_eq!((w >> 29) & 1, 0u32, "bit29 must be 0");
        prop_assert_eq!((w >> 22) & 1, 1u32, "L bit (bit22) must be 1");
    }

    // === Oracle: invariant — Rt/Rn field placement =======================
    // The destination and base register numbers are always placed in their
    // canonical 5-bit fields, regardless of the structure-count encoding bug.
    #[test]
    fn prop_register_fields_place_correctly(
        rt in 0u32..=31u32,
        rn in 1u32..=31u32,
        arr_idx in 0usize..ARRANGEMENTS.len(),
        num_structs in 2u32..=4u32,
        post in any::<bool>(),
    ) {
        let arr = ARRANGEMENTS[arr_idx];
        let start_rt = rt.min(31u32.saturating_sub(num_structs - 1));
        let ops = ldnr_ops(start_rt, arr, num_structs, rn, post, num_structs as i64);
        let w = word_of(encode_neon_ldnr(&ops, num_structs));
        prop_assert_eq!(w & 0x1F, start_rt, "Rt in bits[4:0]");
        prop_assert_eq!((w >> 5) & 0x1F, rn, "Rn in bits[9:5]");
    }

    // === Oracle: determinism =============================================
    #[test]
    fn prop_is_deterministic(
        rt in 0u32..=31u32,
        rn in 1u32..=31u32,
        arr_idx in 0usize..ARRANGEMENTS.len(),
        num_structs in 2u32..=4u32,
        post in any::<bool>(),
    ) {
        let arr = ARRANGEMENTS[arr_idx];
        let start_rt = rt.min(31u32.saturating_sub(num_structs - 1));
        let ops = ldnr_ops(start_rt, arr, num_structs, rn, post, num_structs as i64);
        let a = word_of(encode_neon_ldnr(&ops, num_structs));
        let b = word_of(encode_neon_ldnr(&ops, num_structs));
        prop_assert_eq!(a, b);
    }

    // === Oracle: negative contract =======================================
    // Invalid inputs must be rejected with Err, never silently accepted.
    #[test]
    fn prop_negative_contract(arr_idx in 0usize..ARRANGEMENTS.len()) {
        let arr = ARRANGEMENTS[arr_idx];
        let one = list_of(1, 0, arr);
        let two = list_of(2, 0, arr);
        let mem = Operand::Mem { base: "x1".to_string(), offset: 0 };

        // too few operands
        prop_assert!(encode_neon_ldnr(std::slice::from_ref(&one), 2).is_err(),
            "fewer than 2 operands must error");

        // first operand not a RegList
        let not_list = Operand::RegArrangement { reg: "v0".into(), arrangement: arr.into() };
        prop_assert!(encode_neon_ldnr(&[not_list, mem.clone()], 2).is_err(),
            "first operand must be a RegList");

        // RegList element that is not a RegArrangement
        let bad_elem = Operand::RegList(vec![Operand::Reg("v0".into())]);
        prop_assert!(encode_neon_ldnr(&[bad_elem, mem.clone()], 2).is_err(),
            "RegList element must be a RegArrangement");

        // wrong register count vs num_structs
        prop_assert!(encode_neon_ldnr(&[two.clone(), mem.clone()], 3).is_err(),
            "num_regs != num_structs must error");

        // non-memory second operand
        prop_assert!(encode_neon_ldnr(&[two.clone(), Operand::Reg("x1".into())], 2).is_err(),
            "second operand must be a memory operand");

        // unsupported num_structs (0, 5) with a matching list length so it
        // reaches the opcode table, not the earlier count check
        prop_assert!(encode_neon_ldnr(&[one.clone(), mem.clone()], 0).is_err(),
            "num_structs=0 must error");
    }

    // === Oracle: negative contract — unsupported arrangement =============
    #[test]
    fn prop_unsupported_arrangement_rejected(num_structs in 2u32..=4u32) {
        let _bad = Operand::RegList(vec![Operand::RegArrangement {
            reg: "v0".into(),
            arrangement: "3b".into(), // not a valid replicate arrangement
        }]);
        let mem = Operand::Mem { base: "x1".to_string(), offset: 0 };
        // pad the list so its length matches num_structs (for n>1)
        let mut regs = vec![Operand::RegArrangement {
            reg: "v0".into(),
            arrangement: "3b".into(),
        }];
        for i in 1..num_structs {
            regs.push(Operand::RegArrangement {
                reg: format!("v{i}"),
                arrangement: "3b".into(),
            });
        }
        let ops = vec![Operand::RegList(regs), mem];
        prop_assert!(encode_neon_ldnr(&ops, num_structs).is_err(),
            "unsupported arrangement '3b' must be rejected, not silently encoded");
    }
}

// --- golden regression anchors (llvm-mc-18) --------------------------------
// These pin the exact misencoding: LD2R and LD4R differ from the real
// assembler output. LD3R matches.

#[test]
#[ignore = "documented bug: LD2R uses swapped R/S bits"]
fn golden_ld2r_matches_llvm_mc() {
    let ops = ldnr_ops(0, "16b", 2, 1, false, 0); // ld2r {v0.16b,v1.16b}, [x1]
    let got = word_of(encode_neon_ldnr(&ops, 2));
    // Correct (llvm-mc-18): 0x4D60C020  (R bit 21 = 1)
    assert_eq!(
        got, 0x4D60C020,
        "LD2R misencoded: got {:#010X}, want 0x4D60C020 (impl sets S/bit12 instead of R/bit21)",
        got
    );
}

#[test]
fn golden_ld3r_matches_llvm_mc() {
    let ops = ldnr_ops(2, "4s", 3, 5, false, 0); // ld3r {v2.4s,v3.4s,v4.4s}, [x5]
    let got = word_of(encode_neon_ldnr(&ops, 3));
    assert_eq!(
        got, 0x4D40E8A2,
        "LD3R encoding mismatch: got {:#010X}, want 0x4D40E8A2",
        got
    );
}

#[test]
#[ignore = "documented bug: LD4R uses swapped R/S bits"]
fn golden_ld4r_matches_llvm_mc() {
    let ops = ldnr_ops(6, "8h", 4, 10, false, 0); // ld4r {v6.8h..v9.8h}, [x10]
    let got = word_of(encode_neon_ldnr(&ops, 4));
    // Correct (llvm-mc-18): 0x4D60E546  (R bit 21 = 1)
    assert_eq!(
        got, 0x4D60E546,
        "LD4R misencoded: got {:#010X}, want 0x4D60E546 (impl sets S/bit12 instead of R/bit21)",
        got
    );
}

#[test]
#[ignore = "documented bug: LD4R post-index uses swapped R/S bits"]
fn golden_ld4r_post_index_matches_llvm_mc() {
    // ld4r {v18.8b..v21.8b}, [x22], #4  -> 0x0DFFE2D2
    let ops = ldnr_ops(18, "8b", 4, 22, true, 4);
    let got = word_of(encode_neon_ldnr(&ops, 4));
    assert_eq!(
        got, 0x0DFFE2D2,
        "LD4R post-index misencoded: got {:#010X}, want 0x0DFFE2D2",
        got
    );
}
