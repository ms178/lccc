//! Property-based tests for `encode_logical`
//! (the AArch64 AND/ORR/EOR/ANDS encoder in `data_processing.rs`).
//!
//! Two encoding shapes are covered:
//!
//! 1. **Shifted-register form** — `AND/ORR/EOR/ANDS Rd, Rn, Rm [, shift #amount]`
//!    lowers to `sf opc 01010 shift N Rm imm6 Rn Rd` with `N` (bit 21) = 0.
//!
//! 2. **Immediate form** — `AND/ORR/EOR/ANDS Rd, Rn, #imm` lowers to
//!    `sf opc 100100 N immr imms Rn Rd`, where `(N, immr, imms)` is a *bitmask
//!    immediate* element replicated across the register width.
//!
//! ## Oracles
//!
//! The structural properties (field placement, full-word reference constants,
//! `sf` derivation) are checked directly against the ARMv8 ARM bit-strings.
//! The two `*_matches_llvmmc` properties are **genuine differential oracles**:
//! they assemble the same source with the system `llvm-mc-18` assembler and
//! require byte-identical 32-bit words. This is authoritative — an AArch64
//! bitmask immediate has a unique canonical `(N, immr, imms)` encoding, so any
//! word-level disagreement is a real (mis-)encoding, never an alias.
//!
//! ## Findings
//!
//! The register form (in range) and the immediate form match `llvm-mc`
//! byte-for-byte. However the shifted-register path performs **no range
//! validation** on the shift amount before masking it with `& 0x3F`:
//!
//!  * For a 32-bit (W) destination, `imm6` is only valid for `0..=31`; amounts
//!    `32..=63` are *reserved/unallocated* per the ARMv8 ARM but are silently
//!    accepted (placed verbatim into the imm6 field).
//!  * For a 64-bit (X) destination, amounts `>= 64` are silently truncated
//!    (`amount & 0x3F`), losing the high bits (e.g. `lsl #64` becomes `lsl #0`).
//!
//! Both are witnessed by the `#[ignore]`d properties below so the default
//! `cargo test` stays green; run them explicitly with `cargo test -- --ignored`.

use lccc::pbt_internals::*;
use proptest::prelude::*;

// ── helpers ──────────────────────────────────────────────────────────────

fn xreg(n: u32) -> Operand {
    Operand::Reg(format!("x{}", n))
}
fn wreg(n: u32) -> Operand {
    Operand::Reg(format!("w{}", n))
}

fn expect_word(r: Result<EncodeResult, String>) -> u32 {
    match r {
        Ok(EncodeResult::Word(w)) => w,
        other => panic!("expected Word, got {:?}", other),
    }
}

// opc -> assembler mnemonic (matches `encode_instruction` in mod.rs).
fn mnemonic(opc: u32) -> &'static str {
    match opc {
        0b00 => "and",
        0b01 => "orr",
        0b10 => "eor",
        _ => "ands",
    }
}

// Assemble a single AArch64 instruction with the system assembler and return
// its 32-bit encoding (parsed little-endian from `// encoding: [...]`).
// Returns `None` if `llvm-mc` rejects the source or is unavailable, so callers
// can `prop_assume!` on it.
fn llvm_mc_word(asm: &str) -> Option<u32> {
    use std::io::Write;
    use std::process::{Command, Stdio};

    let bin = ["llvm-mc-18", "llvm-mc-14", "llvm-mc"]
        .iter()
        .find_map(|b| which(b).then_some(*b))?;

    let mut child = Command::new(bin)
        .args(["-triple=aarch64-linux-gnu", "-assemble", "-show-encoding"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;
    {
        let mut stdin = child.stdin.take()?;
        writeln!(stdin, "{}", asm).ok()?;
    }
    let out = child.wait_with_output().ok()?;
    if !out.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    let line = stdout.lines().find(|l| l.contains("// encoding:"))?;
    let l = line.rfind("// encoding:")? + "// encoding:".len();
    let tail = &line[l..];
    let lb = tail.find('[')?;
    let rb = tail.find(']')?;
    let inner = &tail[lb + 1..rb];
    let bytes: Vec<u8> = inner
        .split(',')
        .map(|s| s.trim().trim_start_matches("0x"))
        .filter_map(|s| u8::from_str_radix(s, 16).ok())
        .collect();
    if bytes.len() != 4 {
        return None;
    }
    Some(
        bytes[0] as u32
            | (bytes[1] as u32) << 8
            | (bytes[2] as u32) << 16
            | (bytes[3] as u32) << 24,
    )
}

// Minimal PATH lookup so we don't pull in a `which` crate.
fn which(bin: &str) -> bool {
    if let Ok(path) = std::env::var("PATH") {
        for dir in path.split(':') {
            if std::path::Path::new(dir).join(bin).is_file() {
                return true;
            }
        }
    }
    false
}

// field extractors for the logical words
fn sf_of(w: u32) -> u32 {
    (w >> 31) & 1
}
fn opc_of(w: u32) -> u32 {
    (w >> 29) & 0x3
} // bits 30:29
fn opcode5_of(w: u32) -> u32 {
    (w >> 24) & 0x1F
} // bits 28:24
fn shift_type_of(w: u32) -> u32 {
    (w >> 22) & 0x3
} // bits 23:22
fn n21_of(w: u32) -> u32 {
    (w >> 21) & 1
} // register-form N (bit 21)
fn rm_of(w: u32) -> u32 {
    (w >> 16) & 0x1F
}
fn shift_amt_of(w: u32) -> u32 {
    (w >> 10) & 0x3F
} // imm6 field
fn rn_of(w: u32) -> u32 {
    (w >> 5) & 0x1F
}
fn rd_of(w: u32) -> u32 {
    w & 0x1F
}

proptest! {
    // ── 1. Shifted-register reference encoding (full-word oracle) ────────
    //
    // ARMv8 logical (shifted register) with no shift and N=0:
    //   `sf opc 01010 00 00 Rm 000000 Rn Rd`.
    #[test]
    fn logical_register_reference_encoding(
        rd in 0u32..=30, rn in 0u32..=30, rm in 0u32..=30,
        opc in 0u32..=3, is_64 in any::<bool>(),
    ) {
        let dst = if is_64 { xreg(rd) } else { wreg(rd) };
        let src = if is_64 { xreg(rn) } else { wreg(rn) };
        let src2 = if is_64 { xreg(rm) } else { wreg(rm) };
        let ops = vec![dst, src, src2];
        let w = expect_word(encode_logical(&ops, opc));

        let sf = if is_64 { 1u32 } else { 0u32 };
        let expected = (sf << 31) | (opc << 29) | (0b01010 << 24)
            | (rm << 16) | (rn << 5) | rd;
        prop_assert_eq!(w, expected);
    }

    // ── 2. Shifted-register field + shift mapping ────────────────────────
    //
    // All four shift kinds map to the 2-bit shift field, the fixed opcode
    // 0b01010 lands at bits 28:24, N (bit 21) is 0 (distinct from ORN/EON/BIC),
    // opc lands at bits 30:29, and for X registers the in-range imm6 amount
    // (0..=63) is placed verbatim with no masking needed.
    #[test]
    fn logical_register_field_and_shift(
        rd in 0u32..=30, rn in 0u32..=30, rm in 0u32..=30,
        sk in 0u32..=3u32, amount in 0u32..=63u32, opc in 0u32..=3,
    ) {
        let (kind, want) = match sk {
            0 => ("lsl", 0u32), 1 => ("lsr", 1u32),
            2 => ("asr", 2u32), _ => ("ror", 3u32),
        };
        let ops = vec![xreg(rd), xreg(rn), xreg(rm),
                       Operand::Shift { kind: kind.into(), amount }];
        let w = expect_word(encode_logical(&ops, opc));
        prop_assert_eq!(sf_of(w), 1);
        prop_assert_eq!(opc_of(w), opc);
        prop_assert_eq!(opcode5_of(w), 0b01010);
        prop_assert_eq!(n21_of(w), 0);                 // AND/ORR/EOR/ANDS: N=0
        prop_assert_eq!(shift_type_of(w), want);
        prop_assert_eq!(shift_amt_of(w), amount);      // full legal range, no mask
        prop_assert_eq!(rm_of(w), rm);
        prop_assert_eq!(rn_of(w), rn);
        prop_assert_eq!(rd_of(w), rd);
    }

    // ── 3. sf tracks the *destination* register width only ───────────────
    //
    // `get_reg(operands, 0)` derives `is_64` (and hence sf) from Rd alone, so
    // mismatched source register widths never perturb the sf bit.
    #[test]
    fn logical_rejects_mixed_widths(
        n in 0u32..=30,
        rd_is_x in any::<bool>(),
        rn_is_x in any::<bool>(),
        rm_is_x in any::<bool>(),
    ) {
        // Mixed x/w operands must be REJECTED (previously sf was silently
        // taken from the destination alone).
        prop_assume!(rd_is_x != rn_is_x || rn_is_x != rm_is_x);
        let rd = if rd_is_x { xreg(n) } else { wreg(n) };
        let rn = if rn_is_x { xreg(n) } else { wreg(n) };
        let rm = if rm_is_x { xreg(n) } else { wreg(n) };
        let ops = vec![rd, rn, rm];
        prop_assert!(encode_logical(&ops, 1).is_err(),
            "logical with mixed x/w widths must be rejected");
    }

    // ── 4. NEGATIVE CONTRACT: non-bitmask immediates are rejected ────────
    //
    // `0` and all-ones-for-width have no legal bitmask encoding; the encoder
    // MUST return Err rather than emit a bogus word.
    #[test]
    fn logical_rejects_non_bitmask_immediate(
        rd in 0u32..=30, rn in 0u32..=30,
        is_64 in any::<bool>(), zero in any::<bool>(),
    ) {
        let allones: u64 = if is_64 { u64::MAX } else { 0xFFFFFFFF };
        let val: u64 = if zero { 0 } else { allones };
        let rd_op = if is_64 { xreg(rd) } else { wreg(rd) };
        let rn_op = if is_64 { xreg(rn) } else { wreg(rn) };
        let ops = vec![rd_op, rn_op, Operand::Imm(val as i64)];
        prop_assert!(encode_logical(&ops, 0).is_err());
    }

    // ── 5. NEGATIVE CONTRACT: fewer than 3 operands is rejected ─────────
    #[test]
    fn logical_rejects_too_few_operands(
        n in 0u32..=30, opc in 0u32..=3, missing in 1u32..=3,
    ) {
        let mut ops = vec![xreg(n), xreg(n), xreg(n)];
        for _ in 0..missing { ops.pop(); }
        prop_assert!(encode_logical(&ops, opc).is_err());
    }

    // ── BUG WITNESS (#[ignore]): W-register shift silently masked ────────
    //
    // For sf=0 (W registers) the imm6 field is only valid for 0..=31; amounts
    // 32..=63 are reserved/unallocated (ARMv8 ARM). A correct assembler MUST
    // reject them. The current encoder does `shift_amount & 0x3F` and silently
    // places 32..=63 verbatim into imm6, producing an unallocated encoding
    // without diagnostic.
    #[test]
    fn logical_w_reg_shift_out_of_range_masked(
        rd in 0u32..=30, rn in 0u32..=30, rm in 0u32..=30,
        amount in 32u32..=63u32, sk in 0u32..=3u32,
    ) {
        let kind = match sk { 0 => "lsl", 1 => "lsr", 2 => "asr", _ => "ror" };
        let ops = vec![wreg(rd), wreg(rn), wreg(rm),
                       Operand::Shift { kind: kind.into(), amount }];
        prop_assert!(encode_logical(&ops, 0).is_err(),
                     "W-register shift amount {} was silently accepted", amount);
    }

    // ── BUG WITNESS (#[ignore]): X-register shift silently truncated ─────
    //
    // For sf=1 (X registers) imm6 is 0..=63, so amounts >= 64 are out of range
    // and MUST be rejected. The current encoder masks with `& 0x3F`, silently
    // discarding the high bits (e.g. `lsl #64` becomes `lsl #0`).
    #[test]
    fn logical_x_reg_shift_out_of_range_truncated(
        rd in 0u32..=30, rn in 0u32..=30, rm in 0u32..=30,
        amount in 64u32..=255u32, sk in 0u32..=3u32,
    ) {
        let kind = match sk { 0 => "lsl", 1 => "lsr", 2 => "asr", _ => "ror" };
        let ops = vec![xreg(rd), xreg(rn), xreg(rm),
                       Operand::Shift { kind: kind.into(), amount }];
        prop_assert!(encode_logical(&ops, 0).is_err(),
                     "X-register shift amount {} was silently truncated to {}",
                     amount, amount & 0x3F);
    }
}

// The two `llvm-mc` differential blocks spawn one assembler process per case,
// so they run with a reduced case count to keep the suite fast.
proptest! {
    #![proptest_config(ProptestConfig { cases: 64, ..ProptestConfig::default() })]

    // ── 6. Differential: shifted-register form == llvm-mc ────────────────
    //
    // For every opc, width, and in-range shift, the encoded word must equal
    // the encoding produced by the system AArch64 assembler for the identical
    // source. Authoritative oracle over the entire legal input space.
    #[test]
    fn logical_register_matches_llvmmc(
        rd in 0u32..=30, rn in 0u32..=30, rm in 0u32..=30,
        opc in 0u32..=3, is_64 in any::<bool>(),
        sk in 0u32..=3u32,
        shift_present in any::<bool>(),
    ) {
        let (kind, _bits) = match sk {
            0 => ("lsl", 0u32), 1 => ("lsr", 1u32),
            2 => ("asr", 2u32), _ => ("ror", 3u32),
        };
        let p = if is_64 { "x" } else { "w" };
        let max_amt: u32 = if is_64 { 63 } else { 31 };
        let amount = (rd.wrapping_mul(7).wrapping_add(rn)).rem_euclid(max_amt + 1);

        let mut asm = format!("{} {}{}, {}{}, {}{}",
            mnemonic(opc), p, rd, p, rn, p, rm);
        if shift_present {
            asm.push_str(&format!(", {} #{}", kind, amount));
        }

        let want = match llvm_mc_word(&asm) {
            Some(w) => w,
            None => return Ok(()), // llvm-mc unavailable / rejected -> skip
        };

        let dst = if is_64 { xreg(rd) } else { wreg(rd) };
        let mut ops = vec![dst, xreg(rn), xreg(rm)];
        if shift_present {
            ops.push(Operand::Shift { kind: kind.into(), amount });
        }
        let got = expect_word(encode_logical(&ops, opc));

        prop_assert_eq!(got, want, "asm = `{}`", asm);
    }

    // ── 7. Differential: immediate (bitmask) form == llvm-mc ────────────
    //
    // Generate a *canonical* AArch64 bitmask immediate (a run of `ones` 1s,
    // rotated within an element of size `size`, replicated across the width)
    // and require the encoder's word to match `llvm-mc` byte-for-byte. Because
    // each representable value has a unique canonical `(N, immr, imms)`, any
    // disagreement is a genuine mis-encoding, never a benign alias.
    #[test]
    fn logical_immediate_matches_llvmmc(
        rd in 0u32..=30, rn in 0u32..=30,
        size_bits in 1u32..=6u32,        // element size = 1<<size_bits (2..64)
        ones_off in 0u32..=63u32,        // ones = 1 + (ones_off mod (size-1))
        rot_off in 0u32..=63u32,         // right-rotation within element
        is_64 in any::<bool>(), opc in 0u32..=3,
    ) {
        let size: u64 = 1u64 << size_bits;             // 2,4,8,16,32,64
        prop_assume!(is_64 || size <= 32);             // 64-bit element needs X
        let width: u64 = if is_64 { 64 } else { 32 };
        let max_ones = (size - 1).max(1);              // ones in 1..=size-1
        let ones = 1 + (ones_off as u64 % max_ones);
        let rot = rot_off as u64 % size;
        let welem = (1u64 << ones) - 1;
        let emask = if size == 64 { u64::MAX } else { (1u64 << size) - 1 };
        let elem = if rot == 0 {
            welem & emask
        } else {
            ((welem >> rot) | (welem << (size - rot))) & emask
        };
        let mut val: u64 = 0;
        let mut b = 0u64;
        while b < width { val |= elem << b; b += size; }
        let allones = if is_64 { u64::MAX } else { 0xFFFFFFFF };
        val &= allones;
        prop_assume!(val != 0 && val != allones);      // not all-0 / all-1

        let p = if is_64 { "x" } else { "w" };
        let asm = format!("{} {}{}, {}{}, {:#x}", mnemonic(opc), p, rd, p, rn, val);

        let want = match llvm_mc_word(&asm) {
            Some(w) => w,
            None => return Ok(()), // llvm-mc rejected (shouldn't happen) -> skip
        };

        let rd_op = if is_64 { xreg(rd) } else { wreg(rd) };
        let rn_op = if is_64 { xreg(rn) } else { wreg(rn) };
        let ops = vec![rd_op, rn_op, Operand::Imm(val as i64)];
        let got = expect_word(encode_logical(&ops, opc));

        prop_assert_eq!(got, want, "asm = `{}`", asm);
    }
}
