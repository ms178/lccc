//! Shared helper functions used across peephole optimization passes.
//!
//! These are small utility functions for register manipulation, label parsing,
//! epilogue detection, and instruction analysis that multiple passes need.

use super::super::types::*;

// ── Register helpers ─────────────────────────────────────────────────────────

/// Check if a register ID refers to a valid general-purpose register (0..=15).
#[inline]
pub(super) fn is_valid_gp_reg(reg: RegId) -> bool {
    reg != REG_NONE && reg <= REG_GP_MAX
}

/// Check if a register family ID is callee-saved (rbx=3, r12=12, r13=13, r14=14, r15=15).
pub(super) fn is_callee_saved_reg(reg: RegId) -> bool {
    matches!(reg, 3 | 12 | 13 | 14 | 15)
}

/// True for instructions that change RSP at runtime (`push*`, `pop*`, or
/// `subq/addq $N, %rsp`).
///
/// The text-based peephole passes reason about `%rsp`-relative stack-slot
/// offsets as if they were absolute. A runtime RSP shift makes every offset
/// inside the shifted window refer to a DIFFERENT physical slot, so slot-based
/// reasoning (dead-store elimination, store forwarding, memory folding) must
/// stop conservatively at these lines. The codegen used to emit
/// `pushq/popfq` around i128 binop operand prep — those windows are gone, but
/// F128 conversions still use `subq/addq %rsp`, and prologue pushes occur at
/// function start (harmless: reasoning starts after the prologue).
#[inline]
pub(super) fn is_rsp_shift_line(t: &str) -> bool {
    let t = t.trim();
    if t.starts_with("push") || t.starts_with("pop") {
        return true;
    }
    if t.starts_with("subq $") || t.starts_with("addq $") {
        return t.ends_with("%rsp");
    }
    false
}

/// Check if a line of assembly text references a given register family.
/// Uses the pre-computed reg_refs bitmask for O(1) lookup.
#[inline]
pub(super) fn line_references_reg_fast(info: &LineInfo, reg: RegId) -> bool {
    info.reg_refs & (1u16 << reg) != 0
}

// ── Register rewriting ───────────────────────────────────────────────────────

/// Replace all register-family occurrences of `old_family` with `new_family` in `line`.
/// Handles all register sizes: 64-bit (%rax), 32-bit (%eax), 16-bit (%ax), 8-bit (%al).
/// For example, replacing family 0 (rax) with family 1 (rcx) will convert:
///   %rax -> %rcx, %eax -> %ecx, %ax -> %cx, %al -> %cl
pub(super) fn replace_reg_family(line: &str, old_id: RegId, new_id: RegId) -> String {
    // REG_NAMES only covers GP register families 0-15.
    // XMM/YMM registers (families 24-39) are not in REG_NAMES and must be
    // skipped to avoid out-of-bounds access that corrupts instruction text.
    if old_id as usize >= REG_NAMES[0].len() || new_id as usize >= REG_NAMES[0].len() {
        return line.to_string();
    }
    let mut result = line.to_string();
    // Replace in order from shortest to longest so that smaller sub-register
    // names (e.g., %ebx→%ebp) are replaced before their 64-bit counterparts
    // (%rbx→%rbp). This prevents 64-bit replacement from consuming a 32-bit
    // operand, leaving the wrong register size in the output.
    for size_idx in [3, 2, 1, 0] {
        let old_name = REG_NAMES[size_idx][old_id as usize];
        let new_name = REG_NAMES[size_idx][new_id as usize];
        if old_name == new_name {
            continue;
        }
        result = replace_reg_name_exact(&result, old_name, new_name);
    }
    result
}

/// Replace all complete occurrences of `old_reg` with `new_reg` in `line`.
/// A "complete" occurrence means `old_reg` is not a prefix of a longer register
/// name (e.g., replacing `%r8` must not match `%r8d` or `%r8b`).
/// After `old_reg`, the next character must be a delimiter: `,`, `)`, ` `, or end-of-string.
pub(super) fn replace_reg_name_exact(line: &str, old_reg: &str, new_reg: &str) -> String {
    let mut result = String::with_capacity(line.len());
    let bytes = line.as_bytes();
    let old_bytes = old_reg.as_bytes();
    let old_len = old_bytes.len();
    let mut pos = 0;

    while pos < bytes.len() {
        if pos + old_len <= bytes.len() && &bytes[pos..pos + old_len] == old_bytes {
            // Check that this is a complete register name
            let after = pos + old_len;
            let is_complete =
                after >= bytes.len() || matches!(bytes[after], b',' | b')' | b' ' | b'\t' | b'\n');
            if is_complete {
                result.push_str(new_reg);
                pos += old_len;
                continue;
            }
        }
        result.push(bytes[pos] as char);
        pos += 1;
    }
    result
}

/// Replace register family occurrences only in the source operand part
/// (before the last comma). The destination operand is left unchanged.
pub(super) fn replace_reg_family_in_source(line: &str, old_id: RegId, new_id: RegId) -> String {
    if let Some(comma_pos) = line.rfind(',') {
        let src_part = &line[..comma_pos];
        let dst_part = &line[comma_pos..];
        let new_src = replace_reg_family(src_part, old_id, new_id);
        format!("{}{}", new_src, dst_part)
    } else {
        replace_reg_family(line, old_id, new_id)
    }
}

// ── Instruction analysis ─────────────────────────────────────────────────────

/// Check if an instruction has implicit register usage that makes register
/// substitution unsafe (div/idiv/mul use rax/rdx, shifts use cl, etc.).
pub(super) fn has_implicit_reg_usage(trimmed: &str) -> bool {
    let nb = trimmed.as_bytes();
    if nb.len() < 3 {
        return false;
    }
    // Integer div/mul use implicit rax:rdx. Exclude SSE variants (divsd/mulsd/etc.)
    // which start with div[sp] or mul[sp] — these have no implicit register usage.
    (nb[0] == b'd' && nb[1] == b'i' && nb[2] == b'v'
        && !(nb.len() > 3 && (nb[3] == b's' || nb[3] == b'p'))) || // div, divl, divq (not divsd/divpd)
    (nb[0] == b'i' && nb[1] == b'd' && nb[2] == b'i') || // idiv
    (nb[0] == b'm' && nb[1] == b'u' && nb[2] == b'l'
        && !(nb.len() > 3 && (nb[3] == b's' || nb[3] == b'p'))) || // mul, mulq (not mulsd/mulpd)
    trimmed.starts_with("cltq") || // sign-extend eax -> rax (implicit read+write rax)
    trimmed.starts_with("cbw") ||  // sign-extend al -> ax (implicit read+write rax)
    trimmed.starts_with("cqto") || trimmed.starts_with("cdq") ||
    trimmed.starts_with("cqo") || trimmed.starts_with("cwd") ||
    trimmed.starts_with("rep ") || trimmed.starts_with("repne ") ||
    trimmed.starts_with("cpuid") || trimmed.starts_with("syscall") ||
    trimmed.starts_with("rdtsc") || trimmed.starts_with("rdmsr") ||
    trimmed.starts_with("wrmsr") ||
    trimmed.starts_with("xchg") || trimmed.starts_with("cmpxchg") ||
    trimmed.starts_with("lock ") ||
    // shld/shrd use %cl implicitly (i128 shifts); treat like other shifts.
    trimmed.starts_with("shld") || trimmed.starts_with("shrd") ||
    // x87 FPU status/control instructions that write to %ax or memory.
    // fnstsw writes to %ax (implicit destination), and the peephole's
    // copy propagation must not rewrite %ax to another register since
    // the x86 ISA only allows %ax or memory for fnstsw.
    trimmed.starts_with("fnstsw") || trimmed.starts_with("fnstcw") ||
    trimmed.starts_with("fstcw") || trimmed.starts_with("fnstenv") ||
    trimmed.starts_with("fldenv") || trimmed.starts_with("fldcw")
}

/// Return the register FAMILY an instruction implicitly READS, when the
/// implicit read can be pinned to one family. Unlike `has_implicit_reg_usage`
/// (a boolean veto), this lets peephole deadness analysis distinguish e.g.
/// `cltq` (implicitly reads %eax, family 0) from an unrelated instruction.
///
/// Only families that are not already visible via explicit `%reg` text matter;
/// callers must treat `None` as "no known implicit read of a single family"
/// (conservative callers may fall back to `has_implicit_reg_usage`).
pub(super) fn implicit_read_reg_family(trimmed: &str) -> Option<RegId> {
    let nb = trimmed.as_bytes();
    if nb.len() < 3 {
        return None;
    }
    // Sign/zero-extension idioms that read the accumulator family implicitly.
    //   cltq/cdqe: read %eax (family 0), write %rax
    //   cqto/cqo:  read %rax (family 0), write %rdx
    //   cdq:       read %eax (family 0), write %edx
    //   cbw/cwtl/cwd: read %ax (family 0), write wider accumulator
    if trimmed.starts_with("cltq")
        || trimmed.starts_with("cdqe")
        || trimmed.starts_with("cqto")
        || trimmed.starts_with("cqo")
        || trimmed.starts_with("cdq")
        || trimmed.starts_with("cwd")
        || trimmed.starts_with("cbw")
        || trimmed.starts_with("cwtl")
    {
        return Some(0);
    }
    // Integer div/idiv read %rax:%rdx (families 0 and 2); mul reads %rax.
    // Exclude SSE variants (divsd/mulsd/...) which have explicit operands.
    if (nb[0] == b'd'
        && nb[1] == b'i'
        && nb[2] == b'v'
        && !(nb.len() > 3 && (nb[3] == b's' || nb[3] == b'p')))
        || (nb[0] == b'i' && nb[1] == b'd' && nb[2] == b'i')
        || (nb[0] == b'm'
            && nb[1] == b'u'
            && nb[2] == b'l'
            && !(nb.len() > 3 && (nb[3] == b's' || nb[3] == b'p')))
    {
        // div/idiv also read %rdx; the caller checks the load's own family.
        return Some(0);
    }
    // Double-precision shifts read %cl implicitly (family 1).
    if trimmed.starts_with("shld") || trimmed.starts_with("shrd") {
        return Some(1);
    }
    None
}

/// Check if an instruction is a shift/rotate that implicitly uses %cl.
pub(super) fn is_shift_or_rotate(trimmed: &str) -> bool {
    let nb = trimmed.as_bytes();
    nb.len() >= 3
        && (
            (nb[0] == b's' && (nb[1] == b'h' || nb[1] == b'a')) || // shl, shr, sal, sar
        (nb[0] == b'r' && (nb[1] == b'o' || nb[1] == b'c'))
            // rol, ror, rcl, rcr
        )
}

/// Parse a movq %src, %dst instruction. Returns (src_family, dst_family) if valid.
pub(super) fn parse_reg_to_reg_movq(info: &LineInfo, trimmed: &str) -> Option<(RegId, RegId)> {
    if let LineKind::Other { dest_reg } = info.kind {
        if dest_reg == REG_NONE || dest_reg > REG_GP_MAX {
            return None;
        }
        if let Some(rest) = trimmed.strip_prefix("movq ") {
            if let Some((src_part, dst_part)) = rest.split_once(',') {
                let src = src_part.trim();
                let dst = dst_part.trim();
                if !src.starts_with("%r") || !dst.starts_with("%r") {
                    return None;
                }
                let sfam = register_family_fast(src);
                let dfam = register_family_fast(dst);
                if sfam == REG_NONE
                    || sfam > REG_GP_MAX
                    || sfam == 4
                    || sfam == 5
                    || dfam == REG_NONE
                    || dfam > REG_GP_MAX
                    || dfam == 4
                    || dfam == 5
                    || sfam == dfam
                {
                    return None;
                }
                return Some((sfam, dfam));
            }
        }
    }
    None
}

pub(super) fn parse_reg_to_reg_movl(info: &LineInfo, trimmed: &str) -> Option<(RegId, RegId)> {
    if let LineKind::Other { dest_reg } = info.kind {
        if dest_reg == REG_NONE || dest_reg > REG_GP_MAX {
            return None;
        }
        if let Some(rest) = trimmed.strip_prefix("movl ") {
            if let Some((src_part, dst_part)) = rest.split_once(',') {
                let src = src_part.trim();
                let dst = dst_part.trim();
                if !src.starts_with('%') || !dst.starts_with('%') {
                    return None;
                }
                let sfam = register_family_fast(src);
                let dfam = register_family_fast(dst);
                if sfam == REG_NONE
                    || sfam > REG_GP_MAX
                    || sfam == 4
                    || sfam == 5
                    || dfam == REG_NONE
                    || dfam > REG_GP_MAX
                    || dfam == 4
                    || dfam == 5
                    || sfam == dfam
                {
                    return None;
                }
                return Some((sfam, dfam));
            }
        }
    }
    None
}

/// Get the destination register of an instruction (the register it writes to).
/// Whether `trimmed` is a self-referencing zeroing idiom such as
/// `xorl %eax, %eax`, `xorq %r12, %r12` or `subl %ecx, %ecx`.
///
/// These instructions do not READ the named register: the CPU recognizes the
/// pattern as a dependency-breaking define of zero. Any analysis that treats
/// the source operand as a use will happily rewrite it and destroy both the
/// zero and the register it was supposed to clear.
pub(super) fn is_self_zeroing_idiom(trimmed: &str) -> bool {
    let mut it = trimmed.split_whitespace();
    let Some(op) = it.next() else { return false };
    if !matches!(
        op,
        "xor"
            | "xorb"
            | "xorw"
            | "xorl"
            | "xorq"
            | "sub"
            | "subb"
            | "subw"
            | "subl"
            | "subq"
            | "pxor"
            | "xorps"
            | "xorpd"
    ) {
        return false;
    }
    let rest: String = it.collect::<Vec<_>>().join(" ");
    let Some((a, b)) = rest.split_once(',') else {
        return false;
    };
    let (a, b) = (a.trim(), b.trim());
    !a.is_empty() && a == b && a.starts_with('%')
}

pub(super) fn get_dest_reg(info: &LineInfo) -> RegId {
    match info.kind {
        LineKind::Other { dest_reg } => dest_reg,
        LineKind::StoreRbp { .. } => REG_NONE, // stores don't write to a register
        LineKind::LoadRbp { reg, .. } => reg,
        LineKind::SetCC { reg } => reg,
        LineKind::Pop { reg } => reg,
        LineKind::Cmp => REG_NONE,
        LineKind::Push { .. } => REG_NONE,
        _ => REG_NONE,
    }
}

/// Does the line classified as `info` (text `trimmed`) write ANY part of GP
/// family `fam`?
///
/// `get_dest_reg` yields the single classified destination, which is the
/// explicit AT&T destination or the primary implicit one (`%rax` for
/// `cltq`/`cqto`/`div`/`mul`).  Instructions with a SECOND implicit output
/// (`cqto` → `%rdx`, `idivq` → `%rax:%rdx`, `cpuid`, `rep stos`, `syscall`,
/// ...) are invisible to that answer, and every copy-rewriting pass that
/// asked "is the copy source still intact at line j?" via
/// `get_dest_reg(j) == src` let `movq %rdx, %r8; ...; cqto; idivq; movzbl
/// %r8b` be rewritten to read the remainder in `%dl`.  Use this predicate for
/// every redefinition check.
///
/// Exactness contract (this predicate is safe to use on ACCEPTANCE paths —
/// "the register is fully redefined, the old value is unobservable" — where
/// an over-approximation would wrongly enable a transform):
///
/// * the architectural implicit write set ([`implicit_write_refs`]) is exact
///   and wins over the classified destination;
/// * the classified destination counts only if the line does not implicitly
///   READ the family: the classifier reports `%rax` as the primary
///   destination of `cqto`/`cdq`/`cqo`, but %rax is those instructions'
///   implicit INPUT — their only output is `%rdx`, which the oracle already
///   reported.  (An instruction that both reads and writes the family
///   implicitly — `idivq` on %rax, `cltq` — is decided by its write set, so
///   the read-guard only ever suppresses a classified answer for a family
///   the instruction genuinely only reads.)
#[inline]
pub(super) fn writes_family(info: &LineInfo, trimmed: &str, fam: RegId) -> bool {
    if fam > REG_GP_MAX {
        return false;
    }
    let bit = 1u16 << fam;
    // 1. Architectural implicit writes: exact.
    if implicit_write_refs(trimmed.as_bytes()) & bit != 0 {
        return true;
    }
    // 2. The classified destination (explicit AT&T destination or the primary
    //    implicit one), suppressed when the instruction implicitly reads the
    //    family without writing it (the `cqto`-family %rax misclassification).
    get_dest_reg(info) == fam && implicit_read_refs(trimmed.as_bytes()) & bit == 0
}

/// Does the line fully REDEFINE GP family `fam` at ≥32 bits (a 32-bit write
/// zero-extends to the whole 64-bit family)?
///
/// This is the ACCEPTANCE-grade counterpart of [`writes_family`]: a true
/// answer proves every consumer of the old value unreachable, so transforms
/// that delete a definition (narrow sign-extension removal, the three-line
/// copy-swap middle, ...) must ask THIS predicate.  [`writes_family`] answers
/// "any part written" — byte/word partials (`movb %al`, `lahf`, `xlat`,
/// `divb`, `setcc`) and conditional writes (`cmpxchg`) pass that bar but do
/// NOT retire the old 64-bit value.
#[inline]
pub(super) fn writes_family_full(info: &LineInfo, trimmed: &str, fam: RegId) -> bool {
    if fam > REG_GP_MAX {
        return false;
    }
    let bit = 1u16 << fam;
    // 1. Architectural implicit writes at full width: exact (see
    //    `implicit_full_write_refs` for the exclusions).
    if implicit_full_write_refs(trimmed.as_bytes()) & bit != 0 {
        return true;
    }
    // 2. The explicit destination: full only when the destination token
    //    names the family at 32 or 64 bits.  A memory destination redefines
    //    no register; `%ax`/`%al`/`%ah`-style tokens redefine only part.
    get_dest_reg(info) == fam && dest_operand_is_full_width(trimmed, fam)
}

/// The explicit destination token (text after the last comma) names `fam` at
/// 32 or 64 bits.  `REG_NAMES[0]` is the 64-bit row, `REG_NAMES[1]` the
/// 32-bit row; any other width (or a memory operand) is a partial/no write.
#[inline]
fn dest_operand_is_full_width(trimmed: &str, fam: RegId) -> bool {
    let dest = trimmed.rsplit(',').next().unwrap_or(trimmed).trim();
    dest == REG_NAMES[0][fam as usize] || dest == REG_NAMES[1][fam as usize]
}

// ── Label/jump parsing ───────────────────────────────────────────────────────

/// Parse ".LBB<number>:" label into its number, e.g. ".LBB123:" -> Some(123)
#[inline]
pub(super) fn parse_label_number(label_with_colon: &str) -> Option<u32> {
    let s = label_with_colon.strip_suffix(':')?;
    parse_dotl_number(s)
}

/// Parse ".LBB<number>" into its number, e.g. ".LBB123" -> Some(123)
#[inline]
pub(super) fn parse_dotl_number(s: &str) -> Option<u32> {
    let rest = s.strip_prefix(".LBB")?;
    let b = rest.as_bytes();
    if b.is_empty() || b[0] < b'0' || b[0] > b'9' {
        return None;
    }
    let mut v: u32 = 0;
    for &c in b {
        if c.is_ascii_digit() {
            v = v.wrapping_mul(10).wrapping_add((c - b'0') as u32);
        } else {
            return None;
        }
    }
    Some(v)
}

/// Extract the jump target label from a jump/branch instruction.
pub(super) fn extract_jump_target(s: &str) -> Option<&str> {
    // Handle: jmp .LBB1, je .LBB1, jne .LBB1, etc.
    if let Some(rest) = s.strip_prefix("jmp ") {
        return Some(rest.trim());
    }
    // Conditional jumps: j<cc> <target>
    if s.starts_with('j') {
        if let Some(space) = s.find(' ') {
            return Some(s[space + 1..].trim());
        }
    }
    None
}

// ── Epilogue detection ───────────────────────────────────────────────────────

/// Check if a LoadRbp at position `pos` is part of the function epilogue.
/// The epilogue pattern is: callee-save restores, then `movq %rbp, %rsp; popq %rbp; ret/jmp`.
/// We look forward from `pos` for a Ret or Jmp (for __x86_return_thunk) within a small window,
/// only allowing other LoadRbp or Pop or Other (for stack teardown) instructions between.
pub(super) fn is_near_epilogue(infos: &[LineInfo], pos: usize) -> bool {
    let limit = (pos + 20).min(infos.len());
    for j in (pos + 1)..limit {
        if infos[j].is_nop() {
            continue;
        }
        match infos[j].kind {
            // More callee-save restores or stack teardown moves are expected
            LineKind::LoadRbp { .. } | LineKind::Pop { .. } | LineKind::SelfMove => continue,
            // Stack pointer restoration (movq %rbp, %rsp) is classified as Other
            LineKind::Other { .. } => continue,
            // Found the return instruction - this is an epilogue
            LineKind::Ret | LineKind::Jmp | LineKind::JmpIndirect => return true,
            // Any other instruction type means we're not in the epilogue
            _ => return false,
        }
    }
    false
}

/// Check if an x86 instruction is a read-modify-write for its destination register.
/// For most two-operand instructions like `addq %src, %dst`, the destination is read
/// before being written.  For move instructions like `movq %src, %dst` or `leaq ...(%rip), %dst`,
/// the destination is only written.
///
/// Returns true if the instruction reads the destination register (read-modify-write).
pub(super) fn is_read_modify_write(trimmed: &str) -> bool {
    let b = trimmed.as_bytes();
    if b.len() < 3 {
        return true; // conservative
    }

    // Move instructions are write-only for the destination:
    // movq, movl, movw, movb, movabs, movzbl, movzbq, movzwl, movzwq,
    // movslq, movsbl, movsbq, movswl, movswq
    if b[0] == b'm' && b[1] == b'o' && b[2] == b'v' {
        return false;
    }

    // LEA is write-only for the destination, UNLESS the destination register
    // also appears in the address computation (source operand). For example:
    //   leaq 4(%rax), %rax   -- reads %rax (base) and writes %rax (dest)
    //   leaq (%rax,%rcx), %rax  -- reads %rax and writes %rax
    //   leaq 8(%rcx), %rax   -- does NOT read %rax, only writes it
    if b[0] == b'l' && b[1] == b'e' && b[2] == b'a' {
        // Check if dest register appears in the source (memory) operand.
        // Source operand is everything before the last comma.
        if let Some(comma) = trimmed.rfind(',') {
            let dest_part = trimmed[comma + 1..].trim();
            let src_part = &trimmed[..comma];
            // If the dest register name appears in the source part, it's a read
            if src_part.contains(dest_part) {
                return true;
            }
        }
        return false;
    }

    // Conditional moves (cmovXX) are read-modify-write because the old value
    // is kept when the condition is false.
    if b[0] == b'c' && b[1] == b'm' && b[2] == b'o' {
        return true; // cmov reads the original destination
    }

    // Check if it's a setCC instruction (write-only for byte reg)
    if b[0] == b's' && b[1] == b'e' && b[2] == b't' {
        return false;
    }

    // Dest-only two/three-operand BMI1/ABM: dest is written, not read.
    // Default-true would treat `popcntl %edi, %eax` as RMW of eax and keep
    // a dead staging `movq %rdi, %rax` that the producer never needed.
    if trimmed.starts_with("popcnt")
        || trimmed.starts_with("lzcnt")
        || trimmed.starts_with("tzcnt")
        || trimmed.starts_with("andn")
        || trimmed.starts_with("blsr")
        || trimmed.starts_with("blsi")
        || trimmed.starts_with("blsmsk")
        || trimmed.starts_with("bzhi")
        || trimmed.starts_with("bextr")
        || trimmed.starts_with("pext")
        || trimmed.starts_with("pdep")
        || trimmed.starts_with("shlx")
        || trimmed.starts_with("shrx")
        || trimmed.starts_with("sarx")
    {
        return false;
    }

    // Default: assume read-modify-write (conservative)
    true
}

// ── tests: the redefinition predicate ────────────────────────────────────────

#[cfg(test)]
mod writes_family_tests {
    use super::*;

    #[test]
    fn is_exact_for_the_widening_branch() {
        let info = classify_line("    cqto");
        // The classifier names %rax as cqto's primary destination, but %rax
        // is the implicit INPUT; the only family written is %rdx.
        assert!(writes_family(&info, "cqto", 2));
        assert!(!writes_family(&info, "cqto", 0));
    }

    #[test]
    fn covers_the_implicit_second_output() {
        let info = classify_line("    idivq %r11");
        assert!(writes_family(&info, "idivq %r11", 0));
        assert!(writes_family(&info, "idivq %r11", 2));
        assert!(!writes_family(&info, "idivq %r11", 3));
    }

    #[test]
    fn still_answers_explicit_destinations() {
        let info = classify_line("    movq %rcx, %rax");
        assert!(writes_family(&info, "movq %rcx, %rax", 0));
        assert!(!writes_family(&info, "movq %rcx, %rax", 1));
        // An RMW of another family is not a write of the source family; a
        // read of a family the instruction never writes must not be one
        // either.
        let info = classify_line("    addq %rdx, %rcx");
        assert!(!writes_family(&info, "addq %rdx, %rcx", 2));
        assert!(writes_family(&info, "addq %rdx, %rcx", 1));
    }

    #[test]
    fn rejects_non_gp_families() {
        let info = classify_line("    movsd %xmm0, %xmm1");
        assert!(!writes_family(&info, "movsd %xmm0, %xmm1", 25));
    }

    // ── the full-width acceptance oracle ─────────────────────────────────────

    #[test]
    fn full_width_requires_32_or_64_bit_writes() {
        // movl zero-extends: a full family redefinition.  Byte/word writes
        // leave the upper bits of the old value observable.
        let info = classify_line("    movl %ecx, %eax");
        assert!(writes_family_full(&info, "movl %ecx, %eax", 0));
        let info = classify_line("    movb $1, %al");
        assert!(!writes_family_full(&info, "movb $1, %al", 0));
        let info = classify_line("    movw %cx, %ax");
        assert!(!writes_family_full(&info, "movw %cx, %ax", 0));
        let info = classify_line("    sete %al");
        assert!(!writes_family_full(&info, "sete %al", 0));
        // A memory destination redefines no register.
        let info = classify_line("    movq %rax, (%rdi)");
        assert!(!writes_family_full(&info, "movq %rax, (%rdi)", 0));
        // 32-bit ALU forms zero-extend too.
        let info = classify_line("    addl %ecx, %eax");
        assert!(writes_family_full(&info, "addl %ecx, %eax", 0));
    }

    #[test]
    fn implicit_full_writes_exclude_partials_and_conditionals() {
        // Full-width implicit writes.
        let info = classify_line("    cqto");
        assert!(writes_family_full(&info, "cqto", 2));
        assert!(!writes_family_full(&info, "cqto", 0)); // %rax is the input
        let info = classify_line("    idivq %r11");
        assert!(writes_family_full(&info, "idivq %r11", 0));
        assert!(writes_family_full(&info, "idivq %r11", 2));
        let info = classify_line("    cltq");
        assert!(writes_family_full(&info, "cltq", 0));
        // Partial implicit writes must NOT retire the old value.
        let info = classify_line("    lahf");
        assert!(!writes_family_full(&info, "lahf", 0));
        let info = classify_line("    idivb %cl");
        assert!(!writes_family_full(&info, "idivb %cl", 0));
        let info = classify_line("    xlat");
        assert!(!writes_family_full(&info, "xlat", 0));
        let info = classify_line("    fnstsw");
        assert!(!writes_family_full(&info, "fnstsw", 0));
        // Conditional accumulator reloads are not full redefinitions.
        let info = classify_line("    cmpxchgq %rbx, (%rdi)");
        assert!(!writes_family_full(&info, "cmpxchgq %rbx, (%rdi)", 0));
        let info = classify_line("    xbegin .L1");
        assert!(!writes_family_full(&info, "xbegin .L1", 0));
        // The over-approximate `int` row proves nothing.
        let info = classify_line("    int $0x80");
        assert!(!writes_family_full(&info, "int $0x80", 0));
        assert!(!writes_family_full(&info, "int $0x80", 5));
    }
}
