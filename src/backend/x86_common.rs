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

/// Resolve GCC inline asm dialect alternatives in a template string.
///
/// GCC inline asm supports `{alt0|alt1}` syntax where `alt0` is the AT&T
/// dialect version and `alt1` is the Intel dialect version. Since we always
/// emit AT&T syntax, we select the first alternative and strip the braces.
///
/// Examples:
///   `"rep; bsr{q %1, %0| %0, %1}"` -> `"rep; bsrq %1, %0"`
///   `"bt{l} %[Bit],%[Base]"`       -> `"btl %[Bit],%[Base]"`
///   `"{lock }cmpxchg %1, %2"`      -> `"lock cmpxchg %1, %2"`
///
/// The `%{` escape (literal brace) is not used in practice, so we don't
/// handle it specially here.
fn resolve_dialect_alternatives(line: &str) -> Cow<'_, str> {
    if !line.contains('{') {
        return Cow::Borrowed(line);
    }
    let mut result = String::with_capacity(line.len());
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '{' {
            // Collect the first alternative (AT&T) until '|' or '}'
            i += 1; // skip '{'
            while i < chars.len() && chars[i] != '|' && chars[i] != '}' {
                result.push(chars[i]);
                i += 1;
            }
            // Skip the second alternative (Intel) and closing '}'
            if i < chars.len() && chars[i] == '|' {
                i += 1; // skip '|'
                while i < chars.len() && chars[i] != '}' {
                    i += 1;
                }
            }
            if i < chars.len() && chars[i] == '}' {
                i += 1; // skip '}'
            }
        } else {
            result.push(chars[i]);
            i += 1;
        }
    }
    Cow::Owned(result)
}

/// Map GCC inline asm condition code suffix to x86 SETcc/Jcc suffix.
///
/// GCC's `=@cc<cond>` output constraint maps directly to x86 condition codes.
/// This mapping is identical for both x86-64 and i686.
pub(crate) fn gcc_cc_to_x86(cond: &str) -> &'static str {
    match cond {
        "e" | "z" => "e",    // equal / zero
        "ne" | "nz" => "ne", // not equal / not zero
        "s" => "s",          // sign (negative)
        "ns" => "ns",        // not sign (non-negative)
        "o" => "o",          // overflow
        "no" => "no",        // no overflow
        "c" => "c",          // carry
        "nc" => "nc",        // no carry
        "a" | "nbe" => "a",  // above (unsigned >)
        "ae" | "nb" => "ae", // above or equal (unsigned >=)
        "b" | "nae" => "b",  // below (unsigned <)
        "be" | "na" => "be", // below or equal (unsigned <=)
        "g" | "nle" => "g",  // greater (signed >)
        "ge" | "nl" => "ge", // greater or equal (signed >=)
        "l" | "nge" => "l",  // less (signed <)
        "le" | "ng" => "le", // less or equal (signed <=)
        "p" | "pe" => "p",   // parity even
        "np" | "po" => "np", // parity odd / no parity
        // TODO: emit a warning/diagnostic for unrecognized condition code suffixes
        _ => "e", // fallback to equal
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
    // Pre-process GCC dialect alternatives: {att_syntax|intel_syntax}
    // We always target AT&T syntax, so select the first alternative.
    let line = resolve_dialect_alternatives(line);
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
                let label_idx = num.wrapping_sub(op_regs.len());
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
