//! Shared utilities for x86 and i686 backends.
//!
//! Both the x86-64 and i686 backends use the same x86 ISA family and share
//! register naming conventions, condition code mappings, and inline assembly
//! template parsing logic. This module provides shared free functions to
//! avoid duplicating these across both backends.

use crate::backend::inline_asm::{AsmOperand, AsmOperandKind};
use crate::common::types::IrType;
use crate::ir::reexports::BlockId;
use std::borrow::Cow;
use std::fmt::Write;

/// Packed element shifts saturate by the full count, not its low byte.
/// Encoding a large/negative C int modulo 256 would incorrectly make 256 an
/// identity. 255 is an equivalent saturated count for every 16/32/64-bit
/// logical/arithmetic packed shift. Byte-lane shifts have a separate contract.
pub(super) fn packed_shift_immediate(count: i64) -> i64 {
    if (0..=255).contains(&count) {
        count
    } else {
        255
    }
}

pub(super) fn is_packed_element_shift(mnemonic: &str) -> bool {
    matches!(
        mnemonic,
        "psllw" | "pslld" | "psllq" | "psrlw" | "psrld" | "psrlq" | "psraw" | "psrad"
    )
}

/// Map GCC inline asm condition code suffix to x86 SETcc/Jcc suffix.
///
/// GCC's `=@cc<cond>` output constraint maps directly to x86 condition codes.
/// This mapping is identical for both x86-64 and i686.
pub(crate) fn gcc_cc_to_x86(cond: &str) -> &'static str {
    match crate::common::asm_constraints::cond_code(cond) {
        Some(entry) => entry.setcc,
        // Unreachable for any program this compiler accepts: `asm_validate`
        // diagnoses every `=@cc` suffix outside the shared table with GCC's own
        // `unknown 'asm' flag output` message, and an `@cc` operand is the only
        // way a condition code reaches this function. The historical `e`
        // fallback is kept deliberately: a backend must never panic on a
        // malformed constraint, and a diagnostic has already been issued where
        // the program was parsed. (Before the shared table, `pe`/`po` were
        // silently mapped here even though GCC and Clang both reject them,
        // which made this compiler encode a constraint no other compiler
        // accepts.)
        None => "e",
    }
}

/// Convert any x86 register name to its 32-bit variant.
///
/// Accepts both 64-bit (x86-64) and 32/16/8-bit (i686) register names.
/// For `r8`-`r15` (x86-64 only), appends `d` suffix (e.g., `r8` -> `r8d`).
pub(crate) fn reg_to_32<'a>(reg: &'a str) -> Cow<'a, str> {
    match reg {
        "rax" | "eax" | "ax" | "al" | "ah" => Cow::Borrowed("eax"),
        "rbx" | "ebx" | "bx" | "bl" | "bh" => Cow::Borrowed("ebx"),
        "rcx" | "ecx" | "cx" | "cl" | "ch" => Cow::Borrowed("ecx"),
        "rdx" | "edx" | "dx" | "dl" | "dh" => Cow::Borrowed("edx"),
        "rsi" | "esi" | "si" => Cow::Borrowed("esi"),
        "rdi" | "edi" | "di" => Cow::Borrowed("edi"),
        "rbp" | "ebp" | "bp" => Cow::Borrowed("ebp"),
        "rsp" | "esp" | "sp" => Cow::Borrowed("esp"),
        _ if reg.starts_with('r') => Cow::Owned(format!("{}d", reg)), // r8 -> r8d
        _ => Cow::Borrowed(reg),
    }
}

/// Convert any x86 register name to its 16-bit variant.
///
/// Accepts both 64-bit and 32/16/8-bit register names.
/// For `r8`-`r15` (x86-64 only), appends `w` suffix (e.g., `r8` -> `r8w`).
pub(crate) fn reg_to_16<'a>(reg: &'a str) -> Cow<'a, str> {
    match reg {
        "rax" | "eax" | "ax" | "al" | "ah" => Cow::Borrowed("ax"),
        "rbx" | "ebx" | "bx" | "bl" | "bh" => Cow::Borrowed("bx"),
        "rcx" | "ecx" | "cx" | "cl" | "ch" => Cow::Borrowed("cx"),
        "rdx" | "edx" | "dx" | "dl" | "dh" => Cow::Borrowed("dx"),
        "rsi" | "esi" | "si" => Cow::Borrowed("si"),
        "rdi" | "edi" | "di" => Cow::Borrowed("di"),
        "rbp" | "ebp" | "bp" => Cow::Borrowed("bp"),
        "rsp" | "esp" | "sp" => Cow::Borrowed("sp"),
        _ if reg.starts_with('r') => Cow::Owned(format!("{}w", reg)), // r8 -> r8w
        _ => Cow::Borrowed(reg),
    }
}

/// Convert any x86 register name to its 8-bit low variant.
///
/// Accepts both 64-bit and 32/16/8-bit register names.
/// For `r8`-`r15` (x86-64 only), appends `b` suffix (e.g., `r8` -> `r8b`).
///
/// Note: `rsi`/`rdi` map to `sil`/`dil` (which exist only in 64-bit mode).
/// The 32-bit forms `esi`/`edi` are NOT mapped to `sil`/`dil` because those
/// 8-bit registers don't exist on i686. When i686 passes `"esi"`, it falls
/// through to the default arm and returns the register unchanged (correct
/// behavior: i686 `esi`/`edi` lack 8-bit low forms).
pub(crate) fn reg_to_8l<'a>(reg: &'a str) -> Cow<'a, str> {
    match reg {
        "rax" | "eax" | "ax" | "al" => Cow::Borrowed("al"),
        "rbx" | "ebx" | "bx" | "bl" => Cow::Borrowed("bl"),
        "rcx" | "ecx" | "cx" | "cl" => Cow::Borrowed("cl"),
        "rdx" | "edx" | "dx" | "dl" => Cow::Borrowed("dl"),
        "rsi" => Cow::Borrowed("sil"),
        "rdi" => Cow::Borrowed("dil"),
        _ if reg.starts_with('r') => Cow::Owned(format!("{}b", reg)), // r8 -> r8b
        _ => Cow::Borrowed(reg),
    }
}

/// Convert any x86 register name to its 8-bit high variant.
///
/// Only the four legacy registers (a/b/c/d) have high-byte forms.
/// Falls back to the low-byte variant for all other registers.
pub(crate) fn reg_to_8h<'a>(reg: &'a str) -> Cow<'a, str> {
    match reg {
        "rax" | "eax" | "ax" | "ah" => Cow::Borrowed("ah"),
        "rbx" | "ebx" | "bx" | "bh" => Cow::Borrowed("bh"),
        "rcx" | "ecx" | "cx" | "ch" => Cow::Borrowed("ch"),
        "rdx" | "edx" | "dx" | "dh" => Cow::Borrowed("dh"),
        _ => reg_to_8l(reg), // fallback to low byte
    }
}

/// Substitute `%0`, `%1`, `%[name]`, `%k0`, `%b1`, `%w2`, `%q3`, `%h4`,
/// `%c0`, `%P0`, `%a0`, `%n0`, `%l[name]` etc. in an x86 asm template line.
///
/// This is the template parsing logic shared between x86-64 and i686. The
/// actual operand emission is delegated to the `emit_operand` callback, which
/// handles architecture-specific differences (e.g., RIP-relative addressing on
/// x86-64 vs. absolute addressing on i686, and different default register widths).
pub(crate) fn substitute_x86_asm_operands(
    line: &str,
    op_regs: &[String],
    op_names: &[Option<String>],
    op_is_memory: &[bool],
    op_mem_addrs: &[String],
    op_types: &[IrType],
    gcc_to_internal: &[usize],
    goto_labels: &[(String, BlockId)],
    op_imm_values: &[Option<i64>],
    op_imm_symbols: &[Option<String>],
    emit_operand: fn(
        result: &mut String,
        idx: usize,
        modifier: Option<char>,
        op_regs: &[String],
        op_is_memory: &[bool],
        op_mem_addrs: &[String],
        op_types: &[IrType],
        op_imm_values: &[Option<i64>],
        op_imm_symbols: &[Option<String>],
    ),
) -> String {
    // Dialect alternatives are NOT resolved here. `inline_asm::
    // expand_dialect_alternatives` owns that, once, on the whole template
    // before it is split into lines -- and it is the only place that knows
    // whether the statement is extended asm and whether the target even has
    // assembler dialects. A second resolver here used to run afterwards on
    // every line and unconditionally strip braces, which silently undid both
    // decisions: basic asm was expanded (GCC emits it verbatim) and `%{lit%}`
    // lost its escaped braces. One rule, one implementation.
    let mut result = String::new();
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '%' && i + 1 < chars.len() {
            i += 1;
            // %% -> literal %
            if chars[i] == '%' {
                result.push('%');
                i += 1;
                continue;
            }
            // Check for x86 size/format modifiers:
            //   k (32), w (16), b (8-low), h (8-high), q (64), l (32-alt),
            //   c (raw constant), P (raw symbol), a (address), d (duplicate),
            //   x / t / g (vector register as xmm / ymm / zmm)
            // But 'l' followed by '[' may be a goto label reference %l[name]
            let mut modifier = None;
            if chars[i] == 'l'
                && i + 1 < chars.len()
                && chars[i + 1] == '['
                && !goto_labels.is_empty()
            {
                // This could be %l[name] goto label reference
                // Parse the name and check if it's a goto label first
                let saved_i = i;
                i += 1; // skip 'l'
                i += 1; // skip '['
                let name_start = i;
                while i < chars.len() && chars[i] != ']' {
                    i += 1;
                }
                let name: String = chars[name_start..i].iter().collect();
                if i < chars.len() {
                    i += 1;
                } // skip ']'

                if let Some((_, block_id)) = goto_labels.iter().find(|(n, _)| n == &name) {
                    // It's a goto label — emit the assembly label
                    result.push_str(&block_id.to_string());
                    continue;
                }
                // Not a goto label — backtrack and treat as %l (32-bit modifier) + [name]
                i = saved_i;
                modifier = Some('l');
                i += 1; // skip 'l', will parse [name] below
            } else if chars[i] == 'l'
                && i + 1 < chars.len()
                && chars[i + 1].is_ascii_digit()
                && !goto_labels.is_empty()
            {
                // %l<N> could be a goto label positional reference
                let saved_i = i;
                i += 1; // skip 'l'
                let mut num = 0usize;
                while i < chars.len() && chars[i].is_ascii_digit() {
                    num = num * 10 + (chars[i] as usize - '0' as usize);
                    i += 1;
                }
                // GCC numbers goto labels after all operands.
                // %l<N> where N >= num_operands refers to label (N - num_operands).
                let label_idx = num.wrapping_sub(gcc_to_internal.len());
                if label_idx < goto_labels.len() {
                    result.push_str(&goto_labels[label_idx].1.to_string());
                    continue;
                }
                // Not a valid label index — backtrack
                i = saved_i;
                modifier = Some('l');
                i += 1;
            } else if chars[i] == 'P' {
                // %P: raw symbol/value modifier (uppercase — always a modifier, never a register)
                if i + 1 < chars.len() && (chars[i + 1].is_ascii_digit() || chars[i + 1] == '[') {
                    modifier = Some('P');
                    i += 1;
                }
            } else if matches!(
                chars[i],
                'k' | 'w' | 'b' | 'h' | 'q' | 'l' | 'c' | 'a' | 'n' | 'd' | 'x' | 't' | 'g'
            ) && i + 1 < chars.len()
                && (chars[i + 1].is_ascii_digit() || chars[i + 1] == '[')
            {
                modifier = Some(chars[i]);
                i += 1;
            }

            if chars[i] == '[' {
                // Named operand: %[name] or %k[name]
                i += 1;
                let name_start = i;
                while i < chars.len() && chars[i] != ']' {
                    i += 1;
                }
                let name: String = chars[name_start..i].iter().collect();
                if i < chars.len() {
                    i += 1;
                } // skip ]

                let mut found = false;
                for (idx, op_name) in op_names.iter().enumerate() {
                    if let Some(n) = op_name {
                        if n == &name {
                            emit_operand(
                                &mut result,
                                idx,
                                modifier,
                                op_regs,
                                op_is_memory,
                                op_mem_addrs,
                                op_types,
                                op_imm_values,
                                op_imm_symbols,
                            );
                            found = true;
                            break;
                        }
                    }
                }
                if !found {
                    result.push('%');
                    if let Some(m) = modifier {
                        result.push(m);
                    }
                    result.push('[');
                    result.push_str(&name);
                    result.push(']');
                }
            } else if chars[i].is_ascii_digit() {
                // Positional operand: %0, %1, %k2, etc.
                // GCC operand numbers skip synthetic "+" inputs, so map through gcc_to_internal
                let mut num = 0usize;
                while i < chars.len() && chars[i].is_ascii_digit() {
                    num = num * 10 + (chars[i] as usize - '0' as usize);
                    i += 1;
                }
                // A plain `%N` whose index lands in the label slots is a
                // `goto` label, printed as its address.  gcc measures `%4`
                // with four operand slots and prints `$.L3` for it.
                if num >= gcc_to_internal.len() {
                    let label_idx = num - gcc_to_internal.len();
                    if label_idx < goto_labels.len() {
                        result.push('$');
                        result.push_str(&goto_labels[label_idx].1.to_string());
                        continue;
                    }
                }
                let internal_idx = if num < gcc_to_internal.len() {
                    gcc_to_internal[num]
                } else {
                    num // fallback: direct mapping
                };
                if internal_idx < op_regs.len() {
                    emit_operand(
                        &mut result,
                        internal_idx,
                        modifier,
                        op_regs,
                        op_is_memory,
                        op_mem_addrs,
                        op_types,
                        op_imm_values,
                        op_imm_symbols,
                    );
                } else {
                    result.push('%');
                    if let Some(m) = modifier {
                        result.push(m);
                    }
                    result.push_str(&num.to_string());
                }
            } else {
                // Not a recognized pattern, emit as-is (e.g. %rax, %eax, etc.)
                result.push('%');
                if let Some(m) = modifier {
                    result.push(m);
                }
                result.push(chars[i]);
                i += 1;
            }
        } else {
            result.push(chars[i]);
            i += 1;
        }
    }
    result
}

/// Fail closed when a memory operand reaches template substitution without an
/// address string.
///
/// `AsmOperand::mem_addr` is empty in two very different situations: "not
/// resolved yet" (the normal state before phase 1f, and the permanent state for
/// targets whose memory operands are printed from registers, e.g. arm/riscv)
/// and "resolution failed". The x86 family prints the string verbatim, so the
/// second case used to substitute nothing at all and hand the assembler a
/// truncated instruction: Linux 6.18 mm/zsmalloc.c (`folio_trylock` ->
/// `test_and_set_bit_lock` -> `GEN_BINARY_RMWcc`) produced
/// `lock  btsq  $0,` and the assembler answered "expecting lockable
/// instruction after `lock'". Both x86 backends call this immediately after
/// phase 1f, where an empty address is always a bug.
pub(crate) fn assert_memory_operands_resolved(operands: &[AsmOperand], arch: &str) {
    for (i, op) in operands.iter().enumerate() {
        assert!(
            !matches!(op.kind, AsmOperandKind::Memory) || !op.mem_addr.is_empty(),
            "{arch} inline asm: memory operand {i} (name {:?}, constraint {:?}) still has \
             no address after operand resolution; substituting it would print an empty \
             operand and emit a truncated instruction",
            op.name,
            op.constraint,
        );
    }
}

/// Emit a single operand with modifier — shared logic for `%n`, `%c`/`%P`,
/// memory operands, symbol immediates, and normal immediates.
///
/// Returns `true` if the operand was fully handled by the shared code.
/// Returns `false` if the caller needs to handle the final register
/// formatting case (which differs between x86-64 and i686 due to different
/// default register widths and addressing modes for `%a` + symbol).
pub(crate) fn emit_operand_common(
    result: &mut String,
    idx: usize,
    modifier: Option<char>,
    op_regs: &[String],
    op_is_memory: &[bool],
    op_mem_addrs: &[String],
    op_imm_values: &[Option<i64>],
    op_imm_symbols: &[Option<String>],
) -> bool {
    // GCC's 'd' modifier (i386 `print_operand` case 'd', "duplicate operand
    // for AVX instruction") prints a REGISTER operand twice, separated by
    // ", ", when the TU has AVX — that is how glibc's math-inline-asm.h turns
    // the two-operand template `%vdivss %1, %d0` into the three-operand VEX
    // form `vdivss %xmm1, %xmm0, %xmm0`.  Without AVX the operand is printed
    // once (`divss %xmm1, %xmm0`); the ISA decision is made by the caller,
    // which rewrites `%dN` to `%N` before the template reaches this parser
    // (see `X86Codegen::substitute_template_line`), so a `'d'` seen here
    // always duplicates.  Memory and immediate operands print once.
    // Measured against gcc 16.2 -mavx: `# %d0` with "+r"(long) →
    // `%rax, %ax` (GCC's second copy is a HImode quirk of the RTL printer;
    // no instruction consumes it — lccc prints the natural width twice),
    // "+x"(float) → `%xmm0, %xmm0`, "m" → `(%rdi)`; without -mavx →
    // `%rax`.  The old implementation printed the register once in every
    // mode, producing `vdivss %xmm1, %xmm0` — a spelling GAS rejects
    // ("number of operands mismatch") that only the builtin assembler's
    // leniency kept alive.
    if modifier == Some('d') {
        if op_is_memory[idx] {
            result.push_str(&op_mem_addrs[idx]);
            return true;
        }
        let has_symbol = op_imm_symbols.get(idx).and_then(|s| s.as_ref());
        let has_imm = op_imm_values.get(idx).and_then(|v| v.as_ref());
        if let Some(sym) = has_symbol {
            let _ = write!(result, "${}", sym);
            return true;
        }
        if let Some(imm) = has_imm {
            let _ = write!(result, "${}", imm);
            return true;
        }
        // Register: caller formats it at the operand's natural width and
        // then duplicates (see `emit_operand_with_modifier`).
        return false;
    }
    let is_raw = matches!(modifier, Some('c') | Some('P'));
    let is_neg = modifier == Some('n');
    let has_symbol = op_imm_symbols.get(idx).and_then(|s| s.as_ref());
    let has_imm = op_imm_values.get(idx).and_then(|v| v.as_ref());

    if is_neg {
        // %n: emit the negated constant value (no $ prefix)
        if let Some(&imm) = has_imm {
            result.push_str(&imm.wrapping_neg().to_string());
        } else {
            // Fallback: emit as-is if not an immediate (shouldn't happen with correct usage)
            result.push_str(&op_regs[idx]);
        }
        return true;
    }

    if is_raw {
        // %c / %P: emit raw value without $ or % prefix
        if let Some(sym) = has_symbol {
            result.push_str(sym);
        } else if let Some(imm) = has_imm {
            result.push_str(&imm.to_string());
        } else if op_is_memory[idx] {
            result.push_str(&op_mem_addrs[idx]);
        } else {
            // Register: emit without % prefix
            result.push_str(&op_regs[idx]);
        }
        return true;
    }

    if op_is_memory[idx] {
        // Memory operand — emit the pre-computed mem_addr string
        result.push_str(&op_mem_addrs[idx]);
        return true;
    }

    if modifier != Some('a') {
        // Not %a modifier — handle symbol/immediate cases
        if let Some(sym) = has_symbol {
            let _ = write!(result, "${}", sym);
            return true;
        }
        if let Some(imm) = has_imm {
            let _ = write!(result, "${}", imm);
            return true;
        }
    }

    // Not handled: %a modifier, or register operand needing arch-specific formatting
    false
}

/// The 231-family scalar FMA mnemonic for the sign algebra
/// `dest = (np ? -1 : 1) * (lhs * rhs) + (na ? -1 : 1) * acc`.
///
/// The x86 naming composes exactly: `n` negates the PRODUCT, `sub`
/// subtracts the ADDEND, so
/// `(false,false) vfmadd · (false,true) vfmsub · (true,false) vfnmadd ·
/// (true,true) vfnmsub`, each with the 231 form number and an `sd`/`ss`
/// suffix by lane width. Both the x86-64 and i686 backends lower the
/// four signed FMA3 families through this one table (the i686 arm used
/// to inline its own copy — three sites drifted in step only by luck).
pub(crate) fn fma231_mnemonic(
    negate_product: bool,
    negate_addend: bool,
    is_f64: bool,
) -> &'static str {
    match (negate_product, negate_addend, is_f64) {
        (false, false, true) => "vfmadd231sd",
        (false, false, false) => "vfmadd231ss",
        (false, true, true) => "vfmsub231sd",
        (false, true, false) => "vfmsub231ss",
        (true, false, true) => "vfnmadd231sd",
        (true, false, false) => "vfnmadd231ss",
        (true, true, true) => "vfnmsub231sd",
        (true, true, false) => "vfnmsub231ss",
    }
}

/// x86/i686 prefetch-ISA permissions resolved by the driver
/// (`Driver::resolved_prefetch_sse` / `Driver::resolved_prfchw`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct X86PrefetchIsa {
    /// `prefetchnta/t0/t1/t2` (`0F 18 /0-3`) are legal.
    pub(crate) sse: bool,
    /// `prefetchw` (`0F 0D /1`) is legal.
    pub(crate) prfchw: bool,
}

/// The x86/i686 instruction for an `IntrinsicOp::Prefetch` payload, or
/// `None` when the target has no legal prefetch form for it (the hint is
/// then dropped — always semantics-preserving).
///
/// GCC 16.2 parity, measured for all eight `(rw, locality)` pairs:
/// write intent becomes `prefetchw` exactly when PRFCHW is enabled (for
/// every locality); otherwise read and write hints share the locality
/// table (0 → `nta`, 1 → `t2`, 2 → `t1`, 3 → `t0`).
pub(crate) fn prefetch_mnemonic(payload: u8, isa: X86PrefetchIsa) -> Option<&'static str> {
    let hint = crate::ir::reexports::PrefetchHint::decode(payload);
    if hint.write && isa.prfchw {
        return Some("prefetchw");
    }
    if !isa.sse {
        return None;
    }
    Some(match hint.locality {
        0 => "prefetchnta",
        1 => "prefetcht2",
        2 => "prefetcht1",
        _ => "prefetcht0",
    })
}

#[cfg(test)]
mod prefetch_mnemonic_tests {
    use super::{X86PrefetchIsa, prefetch_mnemonic};
    use crate::ir::reexports::PrefetchHint;

    const LOCALITY: [&str; 4] = ["prefetchnta", "prefetcht2", "prefetcht1", "prefetcht0"];

    fn p(write: bool, loc: u8) -> u8 {
        PrefetchHint::new(write, loc).encode()
    }

    /// GCC 16.2, x86-64 baseline and `-m32 -msse`: read and write share the
    /// locality table (defect 15: write used to collapse onto `prefetcht0`).
    #[test]
    fn without_prfchw_write_hints_keep_their_locality() {
        let isa = X86PrefetchIsa {
            sse: true,
            prfchw: false,
        };
        for loc in 0..4u8 {
            assert_eq!(
                prefetch_mnemonic(p(false, loc), isa),
                Some(LOCALITY[loc as usize])
            );
            assert_eq!(
                prefetch_mnemonic(p(true, loc), isa),
                Some(LOCALITY[loc as usize])
            );
        }
    }

    /// GCC 16.2 `-mprfchw` / `-march=raptorlake`: every write hint is
    /// `prefetchw`, read hints are unchanged.
    #[test]
    fn with_prfchw_write_hints_are_prefetchw() {
        for sse in [true, false] {
            let isa = X86PrefetchIsa { sse, prfchw: true };
            for loc in 0..4u8 {
                assert_eq!(prefetch_mnemonic(p(true, loc), isa), Some("prefetchw"));
            }
        }
    }

    /// Defect 14: GCC emits nothing for `-m32` without SSE/PRFCHW.
    #[test]
    fn without_any_prefetch_isa_nothing_is_emitted() {
        let isa = X86PrefetchIsa::default();
        for payload in 0u8..=255 {
            assert_eq!(prefetch_mnemonic(payload, isa), None);
        }
    }

    #[test]
    fn every_payload_has_a_defined_mnemonic() {
        let isa = X86PrefetchIsa {
            sse: true,
            prfchw: false,
        };
        for payload in 0u8..=255 {
            assert_eq!(
                prefetch_mnemonic(payload, isa),
                prefetch_mnemonic(payload & 7, isa)
            );
        }
    }
}

#[cfg(test)]
mod packed_shift_count_tests {
    use super::packed_shift_immediate;

    #[test]
    fn immediate_encoding_preserves_saturation_instead_of_wrapping() {
        for count in [
            -512,
            -256,
            -1,
            256,
            257,
            512,
            i32::MAX as i64,
            i32::MIN as i64,
        ] {
            assert_eq!(packed_shift_immediate(count), 255);
        }
        for count in 0..=255 {
            assert_eq!(packed_shift_immediate(count), count);
        }
    }
}

#[cfg(test)]
mod substitution_layering_tests {
    //! Dialect resolution belongs to `inline_asm::expand_dialect_alternatives`,
    //! which alone knows whether the statement is extended asm and whether the
    //! target has assembler dialects. A second resolver used to live in this
    //! file and ran after it on every line, unconditionally stripping braces:
    //! that undid both decisions, so basic asm was expanded (GCC emits it
    //! verbatim) and `%{lit%}` lost its escaped braces. These tests fail if a
    //! brace-touching pass is ever reintroduced at this layer.
    use super::substitute_x86_asm_operands;
    use crate::common::types::IrType;

    fn noop_operand(
        _result: &mut String,
        _idx: usize,
        _modifier: Option<char>,
        _op_regs: &[String],
        _op_is_memory: &[bool],
        _op_mem_addrs: &[String],
        _op_types: &[IrType],
        _op_imm_values: &[Option<i64>],
        _op_imm_symbols: &[Option<String>],
    ) {
    }

    fn substitute(line: &str) -> String {
        substitute_x86_asm_operands(
            line,
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
            noop_operand,
        )
    }

    #[test]
    fn braces_reach_the_substitutor_output_untouched() {
        for line in [
            "mov{l} %eax, %ebx",
            "no{p|q}",
            "nop # {lit}",
            "ld1 {v0.4s}, [x0]",
            "nop {|}",
            "nop {}",
        ] {
            assert_eq!(substitute(line), line, "{line:?}");
        }
    }

    #[test]
    fn percent_substitution_still_happens_here() {
        // `%%` is this layer's job, and must survive exactly once.
        assert_eq!(substitute("mov %%eax, %%ebx"), "mov %eax, %ebx");
        assert_eq!(substitute("nop # {a|b} %%ecx"), "nop # {a|b} %ecx");
    }

    #[test]
    fn multibyte_text_survives_substitution() {
        let line = "# caf\u{e9} \u{20ac} \u{1f980} {v0.4s} %%eax";
        assert_eq!(
            substitute(line),
            "# caf\u{e9} \u{20ac} \u{1f980} {v0.4s} %eax"
        );
    }
}

/// Operand roles for a packed/vector FMA3 instruction, per the Intel SDM.
///
/// FMA3's digit triple is not a flavour of one instruction: each names a
/// DIFFERENT assignment of the destination register to `vfmadd132ps`,
/// `vfmadd213ps` and `vfmadd231ps`. The pseudocode is
///
/// ```text
/// 132:  dst = dst  * src3 + src2     destination is multiplicand 1
/// 213:  dst = src2 * dst  + src3     destination is multiplicand 2
/// 231:  dst = src2 * src3 + dst      destination is the ADDEND
/// ```
///
/// so an emitter that places `args = [a, b, c]` for `a*b + c` in one fixed
/// assignment gets exactly one of the three right. Measured consequences with
/// `a=3, b=5, c=7` (all three owe 22.0) are in `fma_role_digits`'s callers.
///
/// Lives here rather than in either backend because BOTH backends emit these
/// and the placement is a property of the instruction, not of the target: the
/// 32-bit backend emitted `vfmadd213ps %ymm1, %ymm2, %ymm0` -- the 132
/// placement -- for all six ops, mis-computing 213 and 231 on any AVX2-capable
/// i686 target while x86-64 had been fixed.
///
/// `#[derive(PartialEq, Eq)]` so tests can compare roles directly instead of
/// matching on names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FmaRoleOrder {
    /// `rm = b, vvvv = c, dst = a` -- computes `a*b + c` when the destination
    /// holds `a`. This is the 132 placement.
    RmBVvvvCDstA,
    /// `rm = c, vvvv = a, dst = b` -- computes `a*b + c` when the destination
    /// holds `b` and the ADDEND sits in the first source slot.
    RmCVvvvADstB,
    /// `rm = b, vvvv = a, dst = c` -- computes `a*b + c` when the destination
    /// holds the addend, which is what 231 is for.
    RmBVvvvADstC,
}

/// The operation digits of a `vfmadd{digits}{ps,pd}[v]{width}` mnemonic.
///
/// Only the digits between the `fmadd` prefix and the first non-digit are the
/// operation field. Collecting every digit in the string would splice the
/// width suffix into it: `vfmadd213ps128` would read as `"213128"`, miss both
/// arms and take the 132 default -- silently reintroducing the very defect the
/// per-digit placement exists to fix. The table keys are
/// `fmadd{digits}ps128` and `vfmadd{digits}ps256`/`...512` (see
/// `expr_builtins_lcccsimd`), so the operation field always starts right after
/// `fmadd`.
pub(crate) fn fma_role_digits(mnemonic: &str) -> &str {
    // The stem is part of the contract, not an implementation detail: the
    // digits only start after the WHOLE mnemonic stem.  Matching just `fmadd`
    // worked while `fmadd` was the only family; `vfmsub132ps` then had no
    // match at all and `vfmaddsub213ps` left `sub213ps`, whose first character
    // is not a digit, so both decoded to `""` and fell through to the 132
    // default.  For `sub`-family 213/231 spellings that default is the WRONG
    // placement, i.e. exactly the silent mis-computation the per-digit
    // placement exists to prevent.  Longest stem first: `fmaddsub` must win
    // over `fmadd`.
    let mut rest = mnemonic.strip_prefix('v').unwrap_or(mnemonic);
    for stem in [
        "fnmaddsub",
        "fnmsubadd",
        "fmaddsub",
        "fmsubadd",
        "fnmadd",
        "fnmsub",
        "fmadd",
        "fmsub",
    ] {
        if let Some(tail) = rest.strip_prefix(stem) {
            rest = tail;
            break;
        }
    }
    let end = rest
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(rest.len());
    &rest[..end]
}

/// Map a `vfmadd{digits}{ps,pd}[v]{width}` mnemonic onto its role order.
///
/// Anything unrecognised gets the 132 placement: it is the one every public
/// `_mm_fmadd_*` macro uses, so a new or misspelled mnemonic keeps the
/// conservative mapping rather than picking up an arbitrary permutation.
pub(crate) fn fma_role_order(mnemonic: &str) -> FmaRoleOrder {
    match fma_role_digits(mnemonic) {
        "213" => FmaRoleOrder::RmCVvvvADstB,
        "231" => FmaRoleOrder::RmBVvvvADstC,
        _ => FmaRoleOrder::RmBVvvvCDstA,
    }
}

#[cfg(test)]
mod fma_role_tests {
    use super::{FmaRoleOrder, fma_role_digits, fma_role_order};

    /// Every mnemonic shape either backend can hand `fma_role_order`, with the
    /// role it must decode to.
    ///
    /// The width-suffixed rows are the regression that mattered: reading every
    /// digit in the mnemonic turned `vfmadd213ps128` into `"213128"`, which
    /// missed both arms and fell through to the 132 placement -- silently
    /// mis-computing 213 exactly the way the un-fixed emitter did. Both suffix
    /// conventions are covered because the table really does use both
    /// (128-bit keys carry no `v`; 256/512 keep it).
    #[test]
    fn every_mnemonic_shape_decodes_to_its_own_placement() {
        for (mnemonic, want) in [
            // Bare operation field: what the emit sites pass.
            ("vfmadd132ps", FmaRoleOrder::RmBVvvvCDstA),
            ("vfmadd213ps", FmaRoleOrder::RmCVvvvADstB),
            ("vfmadd231ps", FmaRoleOrder::RmBVvvvADstC),
            // 128-bit table keys.
            ("fmadd132ps128", FmaRoleOrder::RmBVvvvCDstA),
            ("fmadd213ps128", FmaRoleOrder::RmCVvvvADstB),
            ("fmadd231pd128", FmaRoleOrder::RmBVvvvADstC),
            ("fmadd213pd128", FmaRoleOrder::RmCVvvvADstB),
            // 256/512-bit table keys.
            ("vfmadd132ps256", FmaRoleOrder::RmBVvvvCDstA),
            ("vfmadd213ps256", FmaRoleOrder::RmCVvvvADstB),
            ("vfmadd231pd256", FmaRoleOrder::RmBVvvvADstC),
            ("vfmadd213pd512", FmaRoleOrder::RmCVvvvADstB),
            ("vfmadd231ps512", FmaRoleOrder::RmBVvvvADstC),
            // Every other stem the headers emit, in all three widths: the
            // digits start after the WHOLE stem, so the `fmsub`/`fnmadd`/
            // `fmaddsub` forms must decode their own placement instead of
            // falling back to 132 (which is only correct for the 132 spelling).
            ("vfmsub132ps", FmaRoleOrder::RmBVvvvCDstA),
            ("vfnmadd213ps", FmaRoleOrder::RmCVvvvADstB),
            ("vfnmsub231pd", FmaRoleOrder::RmBVvvvADstC),
            ("vfmaddsub213ps", FmaRoleOrder::RmCVvvvADstB),
            ("vfmsubadd231pd", FmaRoleOrder::RmBVvvvADstC),
            ("fmaddsub132ps128", FmaRoleOrder::RmBVvvvCDstA),
            ("fnmadd231ps128", FmaRoleOrder::RmBVvvvADstC),
            ("fmsub132sd128", FmaRoleOrder::RmBVvvvCDstA),
            ("vfnmadd213pd256", FmaRoleOrder::RmCVvvvADstB),
            ("vfmaddsub231ps512", FmaRoleOrder::RmBVvvvADstC),
            // The `...v{width}` shape the 256-bit dispatch used to present.
            ("vfmadd213pdv256", FmaRoleOrder::RmCVvvvADstB),
            ("vfmadd231psv512", FmaRoleOrder::RmBVvvvADstC),
            // Fallbacks: unknown digits, no digits, and a non-FMA name must
            // all take the conservative 132 placement rather than inventing a
            // permutation.
            ("vfmadd999ps", FmaRoleOrder::RmBVvvvCDstA),
            ("vfmaddps", FmaRoleOrder::RmBVvvvCDstA),
            ("vfmaddsoftps", FmaRoleOrder::RmBVvvvCDstA),
            ("", FmaRoleOrder::RmBVvvvCDstA),
        ] {
            assert_eq!(fma_role_order(mnemonic), want, "`{mnemonic}`");
        }
    }

    /// Only the operation field is extracted: a width suffix must never join
    /// it.
    #[test]
    fn only_the_operation_digits_are_extracted() {
        assert_eq!(fma_role_digits("vfmadd132ps"), "132");
        assert_eq!(fma_role_digits("vfmadd213ps128"), "213");
        assert_eq!(fma_role_digits("vfmadd231pd512"), "231");
        assert_eq!(fma_role_digits("vfmadd213pdv256"), "213");
        // Every stem the headers emit, not just `fmadd`: a `sub`-family name
        // used to yield an empty field (and therefore the 132 default) for ALL
        // of its spellings.
        assert_eq!(fma_role_digits("vfmsub132ps"), "132");
        assert_eq!(fma_role_digits("vfnmadd231pd256"), "231");
        assert_eq!(fma_role_digits("vfmaddsub213ps"), "213");
        assert_eq!(fma_role_digits("vfmsubadd132ps512"), "132");
        assert_eq!(fma_role_digits("fmsub132ss128"), "132");
        // A name with no stem and no digits still yields an empty field rather
        // than a panic, which keeps the 132 default.
        assert_eq!(fma_role_digits(""), "");
        assert_eq!(fma_role_digits("vxorps256"), "");
    }

    /// The table is only worth anything if the three placements really compute
    /// `a*b + c`. Evaluate each role assignment against the SDM's pseudocode
    /// for its digit triple, for several inputs, instead of comparing names to
    /// names.
    #[test]
    fn every_placement_computes_a_times_b_plus_c() {
        /// SDM pseudocode, with `dst`/`src2`(=vvvv)/`src3`(=rm) as named slots.
        fn sdm(digits: &str, dst: f64, src2: f64, src3: f64) -> f64 {
            match digits {
                "132" => dst * src3 + src2,
                "213" => src2 * dst + src3,
                "231" => src2 * src3 + dst,
                other => panic!("no FMA3 pseudocode for {other}"),
            }
        }
        for (a, b, c) in [
            (3.0, 5.0, 7.0),
            (1.0, 0.0, 0.0),
            (-2.0, 4.5, -0.5),
            (1e300, 1e-300, 2.0),
        ] {
            for (digits, role) in [
                ("132", FmaRoleOrder::RmBVvvvCDstA),
                ("213", FmaRoleOrder::RmCVvvvADstB),
                ("231", FmaRoleOrder::RmBVvvvADstC),
            ] {
                // Slot contents: register 0 holds `a`, 1 `b`, 2 `c`.
                let regs = [a, b, c];
                let slot = |name: &str| match name {
                    "a" => regs[0],
                    "b" => regs[1],
                    _ => regs[2],
                };
                let (rm, vvvv, dst) = match role {
                    FmaRoleOrder::RmBVvvvCDstA => ("b", "c", "a"),
                    FmaRoleOrder::RmCVvvvADstB => ("c", "a", "b"),
                    FmaRoleOrder::RmBVvvvADstC => ("b", "a", "c"),
                };
                let got = sdm(digits, slot(dst), slot(vvvv), slot(rm));
                let want = a * b + c;
                assert_eq!(got, want, "{digits} with a={a} b={b} c={c}");
            }
        }
    }
}
