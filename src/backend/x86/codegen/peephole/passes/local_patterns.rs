//! Local peephole pattern matching passes.
//!
//! Merges 7 simple local passes into a single linear scan (`combined_local_pass`)
//! to avoid redundant iteration over the lines array. Also includes
//! `fuse_movq_ext_truncation` which fuses movq + extension/truncation patterns.
//!
//! Merged passes in `combined_local_pass`:
//!   1. eliminate_redundant_movq_self: movq %reg, %reg (same src/dst)
//!   2. eliminate_reverse_move: movq %A,%B + movq %B,%A -> remove second
//!   3. eliminate_redundant_jumps: jmp to the immediately following label
//!   4. eliminate_cond_branch_inversion: jCC+jmp+label -> j!CC (inverted)
//!   5. eliminate_adjacent_store_load: store/load at same %rbp offset
//!   6. eliminate_redundant_zero_extend: redundant zero/sign extensions
//!   7. eliminate_redundant_xorl_zero: xorl %eax,%eax when %rax already zero

use super::super::types::*;
use super::flag_peepholes::flags_dead_after;
use super::helpers::{
    extract_jump_target, get_dest_reg, has_implicit_reg_usage, implicit_read_reg_family,
    is_callee_saved_reg, is_read_modify_write, is_valid_gp_reg, replace_reg_family,
    writes_family, writes_family_full,
};

/// Return which stack base register (`(%rsp)` vs `(%rbp)`) a line uses, if any.
/// Used to ensure an adjacent store and load refer to the SAME slot (same base),
/// so a numeric offset equality is meaningful.
fn line_stack_base(trimmed: &str) -> Option<&'static str> {
    if trimmed.contains("(%rsp)") {
        Some("rsp")
    } else if trimmed.contains("(%rbp)") {
        Some("rbp")
    } else {
        None
    }
}

/// Format a stack offset string for text matching/generation.
/// Checks context to decide between (%rbp) and (%rsp).
/// Convert a 64-bit register name to its 32-bit equivalent.
/// High registers (%r8-%r15) get 'd' suffix, classic regs use 'e' prefix form.
fn reg_64_to_32(r64: &str) -> String {
    match r64 {
        "%rax" => "%eax".into(),
        "%rcx" => "%ecx".into(),
        "%rdx" => "%edx".into(),
        "%rbx" => "%ebx".into(),
        "%rsp" => "%esp".into(),
        "%rbp" => "%ebp".into(),
        "%rsi" => "%esi".into(),
        "%rdi" => "%edi".into(),
        _ if r64.starts_with("%r") => format!("{}d", r64),
        _ => String::new(),
    }
}

/// 64-bit register name WITHOUT a leading `%` to its 32-bit sub-register.
///
/// Only `%r8`-`%r15` take the `d` suffix; the eight classic registers change
/// their `r` prefix to `e`. Appending `d` unconditionally produces names like
/// `rdxd`, which no assembler accepts — this rejected valid programs at the
/// integrated-assembler stage (observed building expat's xmlparse.c at -O2).
fn bare_reg_64_to_32(r64: &str) -> Option<String> {
    Some(match r64 {
        "rax" => "eax".into(),
        "rcx" => "ecx".into(),
        "rdx" => "edx".into(),
        "rbx" => "ebx".into(),
        "rsp" => "esp".into(),
        "rbp" => "ebp".into(),
        "rsi" => "esi".into(),
        "rdi" => "edi".into(),
        _ if r64.starts_with('r')
            && !r64[1..].is_empty()
            && r64[1..].chars().all(|c| c.is_ascii_digit()) =>
        {
            format!("{}d", r64)
        }
        // Already a 32-bit name: leave it alone.
        _ if r64.starts_with('e') || r64.ends_with('d') => r64.to_string(),
        _ => return None,
    })
}

fn stack_offset_str(offset: i32, context: &str) -> String {
    if context.contains("(%rsp)") {
        format!("{}(%rsp)", offset)
    } else {
        format!("{}(%rbp)", offset)
    }
}

pub(super) fn combined_local_pass(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    let mut changed = false;
    let len = store.len();

    // Track whether %rax is known to be zero for redundant xorl elimination.
    // This is set to true after `xorl %eax, %eax` and stays true across
    // StoreRbp instructions (which don't modify register values), but is
    // invalidated by anything that writes %rax, or by control flow barriers.
    let mut rax_is_zero = false;

    let mut i = 0;
    while i < len {
        if infos[i].is_nop() {
            i += 1;
            continue;
        }

        // --- Pattern: redundant xorl %eax, %eax elimination ---
        // When %rax is already known to be zero (from a previous xorl %eax, %eax),
        // and only StoreRbp instructions intervene (which read but don't modify
        // registers), the repeated xorl is redundant.
        //
        // Common pattern from codegen zeroing multiple local variables:
        //   xorl %eax, %eax          # sets rax = 0
        //   movq %rax, -N(%rbp)      # stores 0, rax still 0
        //   xorl %eax, %eax          # REDUNDANT
        //   movq %rax, -M(%rbp)      # stores 0, rax still 0
        if rax_is_zero {
            if let LineKind::Other { dest_reg: 0 } = infos[i].kind {
                let trimmed = infos[i].trimmed(store.get(i));
                if trimmed == "xorl %eax, %eax" {
                    mark_nop(&mut infos[i]);
                    changed = true;
                    i += 1;
                    continue;
                }
            }
        }

        // --- Pattern: dead xorl %eax before overwriting load ---
        // When xorl %eax, %eax is immediately followed by an instruction that
        // completely overwrites %rax (a load from stack), the xorl is dead.
        //
        //   xorl %eax, %eax          # DEAD — overwritten by next instruction
        //   movq -N(%rbp), %rax      # (or movslq, movzbq, etc.)
        if let LineKind::Other { dest_reg: 0 } = infos[i].kind {
            let trimmed = infos[i].trimmed(store.get(i));
            if trimmed == "xorl %eax, %eax" {
                // Check if the next non-nop instruction overwrites %rax.
                let mut j = i + 1;
                while j < len && infos[j].is_nop() {
                    j += 1;
                }
                if j < len {
                    let overwrites_rax = match infos[j].kind {
                        LineKind::LoadRbp { reg: 0, .. } => true, // movq/movslq/etc → %rax
                        // Do NOT treat `call` as overwriting %rax here — for variadic
                        // functions (printf, etc.), %al is an INPUT specifying the number
                        // of SSE register arguments. Removing xorl %eax before call
                        // leaves %al with garbage, causing crashes.
                        // LineKind::Call => true,
                        LineKind::Other { dest_reg: 0 } => {
                            // Check if it's a load that writes %rax (not a read-modify-write)
                            let nj = infos[j].trimmed(store.get(j));
                            let is_mov_into_rax = nj.starts_with("movq ") || nj.starts_with("movl ")
                                || nj.starts_with("movslq ") || nj.starts_with("movzbq ")
                                || nj.starts_with("movzwq ") || nj.starts_with("movsbq ")
                                || nj.starts_with("movswq ") || nj.starts_with("leaq ")
                                || nj.starts_with("movabsq ")
                                // `movl %eax, %eax` — the codegen zero-extension idiom.
                                // Handled below via the source-register check.
                                || nj.starts_with("xorl %eax"); // duplicate xorl
                            if is_mov_into_rax {
                                // SOUNDNESS: the instruction only "overwrites"
                                // %rax with a FRESH value if its source does NOT read
                                // %rax. A self-move such as `movl %eax, %eax` (or
                                // `movq %rax, %rax`, `leaq (%rax), %rax`) READS the
                                // current %rax, so removing the preceding `xorl` would
                                // leave garbage in %rax. Extract the source (first
                                // operand) and refuse to treat rax-reading sources as
                                // an overwrite.
                                //
                                // The source operand spans up to the LAST comma:
                                // SIB-addressing forms like
                                // `leaq (%rax, %rax, 2), %rax` contain commas inside
                                // the parens, so a first-comma split would cut the
                                // source at "(%rax" and lose the register reference.
                                //
                                // register_family_fast only matches bare "%reg"
                                // operands (it returns REG_NONE for anything not
                                // starting with '%'), so memory forms that READ %rax
                                // — `movq (%rax), %rax` loads through it,
                                // `-16(%rbp,%rax,8)` uses it as index — must be
                                // detected textually. gcc.c-torture 931004-11 (both
                                // miscompilation modes) pins this: `xorl %eax,%eax`
                                // materializing a zero index was removed before
                                // `leaq (%rax, %rax, 2), %rax`, leaving the entry
                                // value of %rax scaled into the element address.
                                // "movq %r12, %rax" -> src = "%r12"; "movl %eax, %eax" -> src = "%eax".
                                let src = nj
                                    .rsplit_once(',')
                                    .map(|(s, _)| s)
                                    .unwrap_or(nj)
                                    .splitn(2, char::is_whitespace)
                                    .nth(1)
                                    .unwrap_or("")
                                    .trim();
                                let src_fam = register_family_fast(src);
                                // Source must not reference the %rax family —
                                // register form (family 0) or memory/index form
                                // (textual check; REG_NONE covers all parenthesized
                                // forms and immediates, and immediates never name
                                // a register).
                                let src_touches_rax = src_fam == 0
                                    || src.contains("%rax")
                                    || src.contains("%eax")
                                    || src.contains("%ax")
                                    || src.contains("%al")
                                    || src.contains("%ah");
                                // Overwrite only when the source cannot read %rax.
                                !src_touches_rax
                            } else {
                                false
                            }
                        }
                        _ => false,
                    };
                    if overwrites_rax {
                        mark_nop(&mut infos[i]);
                        changed = true;
                        i += 1;
                        continue;
                    }
                }
            }
        }

        // Update rax_is_zero tracking based on current instruction.
        match infos[i].kind {
            LineKind::StoreRbp { .. } => {
                // Stores to stack don't modify registers, rax_is_zero unchanged.
            }
            LineKind::Other { dest_reg: 0 } => {
                // Something writes to %rax. Check if it's xorl %eax, %eax.
                let trimmed = infos[i].trimmed(store.get(i));
                rax_is_zero = trimmed == "xorl %eax, %eax";
            }
            LineKind::Other { dest_reg: REG_NONE } => {
                // REG_NONE is not a known non-rax destination; it means line
                // classification could not determine the destination. This is
                // intentional for volatile stack accesses quarantined by
                // pin_volatile_stack_slots and also occurs for unrecognized
                // memory-source forms. Such a line MAY write rax/eax, so no
                // register-value fact survives it. Keeping rax_is_zero here
                // removed the required zero before a pushq in sqlite3MultiValues
                // and corrupted an outgoing stack argument.
                rax_is_zero = false;
            }
            LineKind::Other { dest_reg: _ } => {
                // Writes to a known non-rax register, rax_is_zero unchanged.
                // But check if it also reads/clobbers rax implicitly.
                // Most Other instructions only write their dest_reg.
                // Conservative: only keep rax_is_zero if the instruction
                // doesn't reference rax at all (via reg_refs).
                if infos[i].reg_refs & 1 != 0 {
                    // References rax - could be a read or write, invalidate
                    // But actually a read of rax is fine for rax_is_zero.
                    // Only a write to rax matters. Since dest_reg != 0,
                    // rax is not the destination, so it's a read - OK.
                    // Exception: instructions like div/idiv/mul/cqto that
                    // implicitly clobber rax through dest_reg rdx.
                    let trimmed = infos[i].trimmed(store.get(i));
                    if trimmed.starts_with("div")
                        || trimmed.starts_with("idiv")
                        || trimmed.starts_with("mul")
                        || trimmed.starts_with("imul")
                        || trimmed == "cqto"
                        || trimmed == "cqo"
                        || trimmed == "cdq"
                        || trimmed.starts_with("xchg")
                        || trimmed.starts_with("cmpxchg")
                    {
                        rax_is_zero = false;
                    }
                    // Otherwise rax is only read, not written - keep tracking.
                }
            }
            LineKind::LoadRbp { reg: 0, .. } => {
                // Load to rax - rax is no longer zero
                rax_is_zero = false;
            }
            LineKind::LoadRbp { .. } => {
                // Load to non-rax register, rax_is_zero unchanged.
            }
            LineKind::Label
            | LineKind::Jmp
            | LineKind::JmpIndirect
            | LineKind::CondJmp
            | LineKind::Ret
            | LineKind::Call => {
                // Control flow or label - invalidate tracking
                rax_is_zero = false;
            }
            LineKind::Pop { reg: 0 } | LineKind::SetCC { reg: 0 } => {
                rax_is_zero = false;
            }
            LineKind::Pop { .. }
            | LineKind::SetCC { .. }
            | LineKind::Push { .. }
            | LineKind::Cmp
            | LineKind::Directive => {
                // Don't affect rax
            }
            _ => {
                // Conservative: invalidate
                rax_is_zero = false;
            }
        }

        // --- Pattern: self-move elimination (movq %reg, %reg) ---
        // Pre-classified as SelfMove during classify_line, avoiding string parsing.
        if infos[i].kind == LineKind::SelfMove {
            mark_nop(&mut infos[i]);
            changed = true;
            i += 1;
            continue;
        }

        // --- Pattern: reverse-move elimination ---
        // Detects `movq %regA, %regB` followed by `movq %regB, %regA` and
        // eliminates the second mov (since %regA still holds the original value).
        //
        // Safety: We only skip NOPs and StoreRbp between the two instructions.
        // IMPORTANT: Don't eliminate reverse-moves where regA is caller-saved
        // and the reverse-move is needed to reload a param after a call.
        // The `pinned` check in mark_nop handles some cases, but we also skip
        // this pattern entirely when a Call appears between the two moves.
        // StoreRbp reads registers but never modifies any GP register value.
        // Any other instruction type causes the search to stop via `break`.
        if let LineKind::Other { dest_reg: dest_a } = infos[i].kind {
            if is_valid_gp_reg(dest_a) {
                let line_i = infos[i].trimmed(store.get(i));
                // Parse "movq %srcReg, %dstReg" pattern
                if let Some(rest) = line_i.strip_prefix("movq ") {
                    if let Some((src_str, dst_str)) = rest.split_once(',') {
                        let src = src_str.trim();
                        let dst = dst_str.trim();
                        let src_fam = register_family_fast(src);
                        let dst_fam = register_family_fast(dst);
                        // Both must be GP registers, different families, both register operands
                        if is_valid_gp_reg(src_fam)
                            && is_valid_gp_reg(dst_fam)
                            && src_fam != dst_fam
                            && src.starts_with('%')
                            && dst.starts_with('%')
                        {
                            // Find the next non-NOP, non-StoreRbp instruction.
                            // Limit search to 8 lines to avoid pathological scanning.
                            let mut j = i + 1;
                            let search_limit = (i + 8).min(len);
                            while j < search_limit {
                                if infos[j].is_nop() {
                                    j += 1;
                                    continue;
                                }
                                if matches!(infos[j].kind, LineKind::StoreRbp { .. }) {
                                    j += 1;
                                    continue;
                                }
                                break;
                            }
                            if j < search_limit {
                                // Check if line j is the reverse: movq %dstReg, %srcReg
                                if let LineKind::Other { dest_reg: dest_b } = infos[j].kind {
                                    if dest_b == src_fam {
                                        let line_j = infos[j].trimmed(store.get(j));
                                        if let Some(rest_j) = line_j.strip_prefix("movq ") {
                                            if let Some((src_j, dst_j)) = rest_j.split_once(',') {
                                                let src_j = src_j.trim();
                                                let dst_j = dst_j.trim();
                                                let src_j_fam = register_family_fast(src_j);
                                                let dst_j_fam = register_family_fast(dst_j);
                                                if src_j_fam == dst_fam && dst_j_fam == src_fam {
                                                    mark_nop(&mut infos[j]);
                                                    changed = true;
                                                    i += 1;
                                                    continue;
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // --- Pattern: redundant jump to next label ---
        if infos[i].kind == LineKind::Jmp {
            let jmp_line = infos[i].trimmed(store.get(i));
            if let Some(target) = jmp_line.strip_prefix("jmp ") {
                let target = target.trim();
                // Find the next non-NOP, non-empty line
                let mut found_redundant = false;
                for j in (i + 1)..len {
                    if infos[j].is_nop() || infos[j].kind == LineKind::Empty {
                        continue;
                    }
                    if infos[j].kind == LineKind::Label {
                        let next = infos[j].trimmed(store.get(j));
                        if let Some(label) = next.strip_suffix(':') {
                            if label == target {
                                mark_nop(&mut infos[i]);
                                changed = true;
                                found_redundant = true;
                            }
                        }
                    }
                    break;
                }
                if found_redundant {
                    i += 1;
                    continue;
                }
            }
        }

        // --- Pattern: conditional branch inversion for fall-through ---
        // Detects:
        //   jCC .Ltrue        (conditional jump)
        //   jmp .Lfalse       (unconditional jump)
        //   .Ltrue:           (label matching the conditional target)
        //
        // Transforms to:
        //   j!CC .Lfalse      (inverted condition, jump to false target)
        //   .Ltrue:           (fall through naturally)
        if infos[i].kind == LineKind::CondJmp {
            let cond_line = infos[i].trimmed(store.get(i));
            // Parse: "jCC target" -> extract CC and target
            if let Some(space_pos) = cond_line.find(' ') {
                let cc = &cond_line[1..space_pos]; // e.g., "l", "ge", "ne"
                let cond_target = cond_line[space_pos + 1..].trim();
                // Find the next non-NOP line (should be jmp)
                let mut j = i + 1;
                while j < len && infos[j].is_nop() {
                    j += 1;
                }
                if j < len && infos[j].kind == LineKind::Jmp {
                    let jmp_line = infos[j].trimmed(store.get(j));
                    if let Some(jmp_target) = jmp_line.strip_prefix("jmp ") {
                        let jmp_target = jmp_target.trim();
                        // Find the next non-NOP/non-empty line after jmp (should be a label)
                        let mut k = j + 1;
                        while k < len && (infos[k].is_nop() || infos[k].kind == LineKind::Empty) {
                            k += 1;
                        }
                        if k < len && infos[k].kind == LineKind::Label {
                            let label_line = infos[k].trimmed(store.get(k));
                            if let Some(label_name) = label_line.strip_suffix(':') {
                                if label_name == cond_target {
                                    let inv_cc = invert_cc(cc);
                                    if inv_cc != cc {
                                        let new_line = format!("    j{} {}", inv_cc, jmp_target);
                                        replace_line(store, &mut infos[i], i, new_line);
                                        mark_nop(&mut infos[j]); // Remove the jmp
                                        changed = true;
                                        i += 1;
                                        continue;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // --- Pattern: adjacent store/load at same %rbp offset ---
        if let LineKind::StoreRbp {
            reg: sr,
            offset: so,
            size: ss,
        } = infos[i].kind
        {
            if i + 1 < len && !infos[i + 1].is_nop() {
                if let LineKind::LoadRbp {
                    reg: lr,
                    offset: lo,
                    size: ls,
                } = infos[i + 1].kind
                {
                    // Same base register (both %rsp or both %rbp) so the offsets refer
                    // to the same slot. With FPO the codegen uses (%rsp); otherwise
                    // (%rbp). Mixing bases at the same numeric offset would be wrong.
                    let store_base = line_stack_base(infos[i].trimmed(store.get(i)));
                    let load_base = line_stack_base(infos[i + 1].trimmed(store.get(i + 1)));
                    if so == lo
                        && ss == ls
                        && store_base.is_some()
                        && store_base == load_base
                        && sr != REG_NONE
                    {
                        if sr == lr {
                            // Same register: load is redundant
                            mark_nop(&mut infos[i + 1]);
                            changed = true;
                            i += 1;
                            continue;
                        } else if lr != REG_NONE
                            && is_valid_gp_reg(sr)
                            && is_valid_gp_reg(lr)
                            && matches!(ss, MoveSize::Q | MoveSize::L)
                            && lr != 4
                            && lr != 5
                            && sr != 4
                            && sr != 5
                        {
                            // Different registers at the SAME offset, ADJACENT (i+1):
                            //   movq %sr, OFF(%rsp)   ; store
                            //   movq OFF(%rsp), %lr   ; load, immediately after
                            // The slot just holds sr's value, so the load is exactly
                            // `movq %sr, %lr`. Provably safe: no instruction intervenes
                            // (i+1 is checked directly), no pushfq/popfq can have shifted
                            // %rsp, and no label/branch merges here. Restricted to Q/L
                            // (a plain move; SLQ/zero-extend differ).
                            let store_reg_str = reg_id_to_name(sr, ls);
                            let load_reg_str = reg_id_to_name(lr, ls);
                            let new_text = format!(
                                "    {} {}, {}",
                                ls.mnemonic(),
                                store_reg_str,
                                load_reg_str
                            );
                            replace_line(store, &mut infos[i + 1], i + 1, new_text);
                            changed = true;
                            i += 1;
                            continue;
                        }
                    }
                }
            }
        }

        // --- Pattern: store/load forwarding across the stored register's
        //     own modification (post-inc/dec shape) ---
        //   movq %S, OFF(%rsp)      ; spill of OLD value
        //   <one line writing %S, not reading OFF, not touching %D or flags-consumers>
        //   movq OFF(%rsp), %D      ; reload of OLD value
        // The reload wants the PRE-modification value, so plain forwarding is
        // wrong — instead hoist the copy above the modifier:
        //   movq %S, %D
        //   <modifier>
        // (store stays; never-read elimination removes it when dead).
        // Conditions: exact width match (Q/L), S != D, the middle line's
        // destination is %S itself (LineKind-visible write), middle line does
        // not reference %D and has no memory operand at OFF, and %D is not a
        // source of the middle line.
        if let LineKind::StoreRbp {
            reg: sr,
            offset: so,
            size: ss,
        } = infos[i].kind
        {
            if matches!(ss, MoveSize::Q | MoveSize::L) && is_valid_gp_reg(sr) && sr != 4 && sr != 5
            {
                let m = next_non_nop(infos, i + 1, len);
                if m < len && !infos[m].is_barrier() {
                    let l = next_non_nop(infos, m + 1, len);
                    if l < len {
                        if let LineKind::LoadRbp {
                            reg: lr,
                            offset: lo,
                            size: ls,
                        } = infos[l].kind
                        {
                            let store_base = line_stack_base(infos[i].trimmed(store.get(i)));
                            let load_base = line_stack_base(infos[l].trimmed(store.get(l)));
                            if so == lo
                                && ss == ls
                                && store_base.is_some()
                                && store_base == load_base
                                && is_valid_gp_reg(lr)
                                && lr != 4
                                && lr != 5
                                && lr != sr
                            {
                                // Middle line: FULLY redefines %S (explicitly
                                // at 32/64-bit width, or implicitly — `cqto`
                                // overwrites %rdx without naming it),
                                // doesn't read %D, no memory access, and is a
                                // plain register-dest instruction.  The
                                // full-width bar matters: a partial write
                                // (`movb $7, %sl`) leaves the upper bits of
                                // the copied value live, so deleting the
                                // copy would corrupt the later full read.
                                let mid_t = infos[m].trimmed(store.get(m));
                                let mid_writes_s = writes_family_full(&infos[m], mid_t, sr)
                                    && matches!(infos[m].kind, LineKind::Other { .. });
                                let mid_refs_d = infos[m].reg_refs & (1u16 << lr) != 0;
                                // leaq is address ARITHMETIC: `leaq 1(%r11), %r11`
                                // reads no memory (has_indirect_mem is a
                                // classification of the OPERAND TEXT, which
                                // flags any non-frame paren base — for leaq
                                // that is an arithmetic source, not a load).
                                // Any other paren operand is a real access.
                                let mid_is_lea =
                                    mid_t.starts_with("leaq ") || mid_t.starts_with("leal ");
                                let mid_has_mem = infos[m].rbp_offset != RBP_OFFSET_NONE
                                    || (!mid_is_lea
                                        && (infos[m].has_indirect_mem || mid_t.contains("(%")));
                                if mid_writes_s && !mid_refs_d && !mid_has_mem {
                                    let copy = format!(
                                        "    {} {}, {}",
                                        ls.mnemonic(),
                                        reg_id_to_name(sr, ls),
                                        reg_id_to_name(lr, ls)
                                    );
                                    // Rewrite the LOAD line into the hoisted copy and
                                    // swap it before the modifier by rewriting in place:
                                    // line i stays (store), line m becomes the copy,
                                    // line l becomes the modifier's text.
                                    let mid_text_owned = store.get(m).to_string();
                                    replace_line(store, &mut infos[m], m, copy);
                                    replace_line(store, &mut infos[l], l, mid_text_owned);
                                    changed = true;
                                    i += 1;
                                    continue;
                                }
                            }
                        }
                    }
                }
            }
        }

        // --- Pattern: redundant zero/sign extension (including cltq) ---
        // Uses pre-classified ExtKind to avoid repeated starts_with/ends_with
        // string comparisons on every iteration.
        let mut ext_idx = i + 1;
        while ext_idx < len && ext_idx < i + 10 {
            if infos[ext_idx].is_nop() {
                ext_idx += 1;
                continue;
            }
            if matches!(infos[ext_idx].kind, LineKind::StoreRbp { .. }) {
                ext_idx += 1;
                continue;
            }
            break;
        }

        if ext_idx < len && !infos[ext_idx].is_nop() {
            let next_ext = infos[ext_idx].ext_kind;
            let prev_ext = infos[i].ext_kind;

            let is_redundant_ext = match next_ext {
                ExtKind::MovzbqAlRax => matches!(
                    prev_ext,
                    ExtKind::ProducerMovzbqToRax | ExtKind::MovzbqAlRax
                ),
                ExtKind::MovzwqAxRax => matches!(
                    prev_ext,
                    ExtKind::ProducerMovzwqToRax | ExtKind::MovzwqAxRax
                ),
                ExtKind::MovsbqAlRax => matches!(
                    prev_ext,
                    ExtKind::ProducerMovsbqToRax | ExtKind::MovsbqAlRax
                ),
                ExtKind::MovslqEaxRax => matches!(
                    prev_ext,
                    ExtKind::ProducerMovslqToRax | ExtKind::MovslqEaxRax
                ),
                ExtKind::Cltq => matches!(
                    prev_ext,
                    ExtKind::ProducerMovslqToRax
                        | ExtKind::ProducerMovqConstRax
                        | ExtKind::MovslqEaxRax
                        | ExtKind::Cltq
                ),
                ExtKind::MovlEaxEax => matches!(
                    prev_ext,
                    ExtKind::ProducerArith32
                        | ExtKind::ProducerMovlToEax
                        | ExtKind::ProducerMovzbToEax
                        | ExtKind::ProducerMovzbqToRax
                        | ExtKind::ProducerMovzwToEax
                        | ExtKind::ProducerMovzwqToRax
                        | ExtKind::ProducerDiv32
                        | ExtKind::MovlEaxEax
                ),
                _ => false,
            };

            if is_redundant_ext {
                mark_nop(&mut infos[ext_idx]);
                changed = true;
                i += 1;
                continue;
            }

            // --- Extended scan: cltq past non-rax-clobbering instructions ---
            if next_ext == ExtKind::Cltq && !is_redundant_ext {
                let i_writes_rax = match infos[i].kind {
                    LineKind::Other { dest_reg } => dest_reg == 0,
                    LineKind::LoadRbp { reg, .. } => reg == 0,
                    LineKind::StoreRbp { .. } => false,
                    LineKind::Nop | LineKind::Empty => false,
                    _ => true, // conservative: barriers, calls, etc. may write rax
                };

                if !i_writes_rax && i > 0 {
                    let mut found_producer = false;
                    let scan_limit = i.saturating_sub(6);
                    let mut k = i - 1;
                    while k >= scan_limit {
                        if infos[k].is_nop() {
                            if k == 0 {
                                break;
                            }
                            k -= 1;
                            continue;
                        }
                        if matches!(infos[k].kind, LineKind::StoreRbp { .. }) {
                            if k == 0 {
                                break;
                            }
                            k -= 1;
                            continue;
                        }
                        // Stop at barriers (labels, calls, jumps, ret)
                        if infos[k].is_barrier() {
                            break;
                        }
                        // Check if this instruction is a sign-extension producer for rax
                        let k_ext = infos[k].ext_kind;
                        if matches!(
                            k_ext,
                            ExtKind::ProducerMovslqToRax
                                | ExtKind::ProducerMovqConstRax
                                | ExtKind::MovslqEaxRax
                                | ExtKind::Cltq
                        ) {
                            found_producer = true;
                            break;
                        }
                        // Check if this instruction writes to %rax (family 0)
                        let writes_rax = match infos[k].kind {
                            LineKind::Other { dest_reg } => dest_reg == 0,
                            LineKind::LoadRbp { reg, .. } => reg == 0,
                            _ => true, // conservative: treat unknown as writing rax
                        };
                        if writes_rax {
                            break;
                        }
                        if k == 0 {
                            break;
                        }
                        k -= 1;
                    }
                    if found_producer {
                        mark_nop(&mut infos[ext_idx]);
                        changed = true;
                        i += 1;
                        continue;
                    }
                }
            }
        }

        i += 1;
    }
    changed
}

// ── Redundant sign-extension elimination for callee-saved registers ──────────
//
// The codegen emits `movslq %REGd, %REG` after every signed i32 ALU op on
// callee-saved registers. Two patterns are eliminated:
//
// 1. Self sign-extension followed by movslq to rax:
//    movslq %r11d, %r11   ← redundant (the next movslq re-extends)
//    movslq %r11d, %rax
//
// 2. cltq followed by store: the sign-extension is unnecessary since the
//    stored value will be re-extended on the next load.

pub(super) fn eliminate_dead_sign_extensions(
    store: &mut LineStore,
    infos: &mut [LineInfo],
) -> bool {
    let len = store.len();
    let mut changed = false;
    let mut i = 0;

    while i < len {
        if infos[i].is_nop() {
            i += 1;
            continue;
        }

        let t = infos[i].trimmed(store.get(i));

        // Pattern 1: movslq %REGd, %REG followed by movslq %REGd, %rax
        // The first movslq is dead because the second re-extends from %REGd.
        if t.starts_with("movslq %") && t.contains("d, %") {
            // Parse: "movslq %XXXd, %XXX" (self sign-extension)
            let parts: Vec<&str> = t.split(", ").collect();
            if parts.len() == 2 {
                let src = parts[0].trim_start_matches("movslq ");
                let dst = parts[1];
                // Check if it's a self sign-extension: src without 'd' suffix == dst
                let src_base = src.trim_end_matches('d');
                if src_base == dst {
                    // Look ahead for another movslq from the same 32-bit register
                    let mut j = i + 1;
                    let mut found_re_extend = false;
                    while j < len && j < i + 4 {
                        if infos[j].is_nop() {
                            j += 1;
                            continue;
                        }
                        let t2 = infos[j].trimmed(store.get(j));
                        // Next instruction uses %REGd as source (re-extends)
                        if t2.starts_with("movslq ") && t2.contains(src) {
                            found_re_extend = true;
                            break;
                        }
                        // If the 64-bit form is used before re-extension, we can't remove
                        if t2.contains(dst) {
                            break;
                        }
                        j += 1;
                    }
                    if found_re_extend {
                        mark_nop(&mut infos[i]);
                        changed = true;
                        i += 1;
                        continue;
                    }
                }
            }
        }

        // Pattern 2: cltq is dead when the next %rax consumer is a 32-bit op
        // redefining %rax from %eax alone, OR a store of %rax whose slot is
        // only ever re-read narrow/sign-extending (smart recovery: a narrow
        // store is trivially safe; a full-width store keeps the cltq dead
        // iff every later load of the slot re-derives or ignores the upper
        // half — only a full-width movq reload observes the sign-extended
        // bits). Barriers, slot rewrites, slot-address escapes and indirect
        // memory end the scan conservatively.
        if t == "cltq" {
            let mut j = i + 1;
            let mut can_eliminate = false;
            while j < len && j < i + 8 {
                if infos[j].is_nop() {
                    j += 1;
                    continue;
                }
                if infos[j].is_barrier() {
                    break;
                }
                match infos[j].kind {
                    LineKind::StoreRbp {
                        reg: 0,
                        offset: n,
                        size,
                    } => {
                        // Narrow store persists only the low bits: the cltq
                        // upper half is discarded by the store itself — BUT
                        // only if no later full-width %rax consumer observes
                        // the extended bits before %rax is rewritten.
                        // `cltq; movl %eax,M; movq %rax,N; cmpq N(%rbp),%rax`
                        // (the add_overflow truncate/extend round-trip,
                        // gcc.c-torture execute/pr122943.c) previously
                        // eliminated the cltq on the narrow store alone,
                        // leaving the stale wide value in the full-width
                        // store's slot.
                        if !matches!(size, MoveSize::Q) {
                            let mut k2 = j + 1;
                            let mut narrow_only = true;
                            let mut scans2 = 0usize;
                            while k2 < len && scans2 < 64 {
                                scans2 += 1;
                                if infos[k2].is_nop() {
                                    k2 += 1;
                                    continue;
                                }
                                if infos[k2].is_barrier() {
                                    // Conservative: the extension may escape.
                                    narrow_only = false;
                                    break;
                                }
                                match infos[k2].kind {
                                    // Full-width store of %rax observes the
                                    // extended upper half.
                                    LineKind::StoreRbp {
                                        reg: 0,
                                        size: MoveSize::Q,
                                        ..
                                    } => {
                                        narrow_only = false;
                                        break;
                                    }
                                    // %eax-only store: upper half still stale.
                                    LineKind::StoreRbp { reg: 0, .. } => {
                                        k2 += 1;
                                        continue;
                                    }
                                    // Anything that REWRITES %rax makes the
                                    // stale extension irrelevant.
                                    LineKind::LoadRbp { reg: 0, .. }
                                    | LineKind::Other { dest_reg: 0 } => break,
                                    // Any other %rax reference (e.g. a 64-bit
                                    // `cmpq M(%rbp), %rax` or `movq %rax, %rX`)
                                    // may observe the upper half.
                                    _ => {
                                        if infos[k2].reg_refs & 1 != 0 {
                                            narrow_only = false;
                                            break;
                                        }
                                        k2 += 1;
                                        continue;
                                    }
                                }
                            }
                            if narrow_only {
                                can_eliminate = true;
                            }
                            break;
                        }
                        let mut k = j + 1;
                        let mut safe = true;
                        let mut saw_load = false;
                        let mut scans = 0usize;
                        while k < len && scans < 64 {
                            scans += 1;
                            if infos[k].is_nop() {
                                k += 1;
                                continue;
                            }
                            if infos[k].is_barrier() {
                                safe = false;
                                break;
                            }
                            match infos[k].kind {
                                LineKind::LoadRbp { offset: lo, .. } if lo == n => {
                                    saw_load = true;
                                    let tk = infos[k].trimmed(store.get(k)).trim_start();
                                    if tk.starts_with("movq ") {
                                        safe = false; // full-width read of upper half
                                        break;
                                    }
                                }
                                LineKind::StoreRbp { offset: so, .. } if so == n => {
                                    safe = false; // slot rewritten
                                    break;
                                }
                                _ => {
                                    if infos[k].has_indirect_mem {
                                        safe = false;
                                        break;
                                    }
                                    // Any other slot reference (xmm load, lea
                                    // escape, ...) is not proven narrow.
                                    let tk = infos[k].trimmed(store.get(k));
                                    if let Some((_, off)) = super::direct_stack_slot_in_line(tk) {
                                        if off == n {
                                            safe = false;
                                            break;
                                        }
                                    }
                                }
                            }
                            k += 1;
                        }
                        if scans >= 64 {
                            safe = false;
                        }
                        if safe && saw_load {
                            can_eliminate = true;
                        }
                        break;
                    }
                    LineKind::Other { dest_reg: 0 } => {
                        // Something writes to %rax — check if it's a 32-bit op
                        // that only uses %eax (the cltq upper bits don't matter)
                        let tj = infos[j].trimmed(store.get(j));
                        if is_32bit_eax_consumer(tj) {
                            can_eliminate = true;
                        }
                        break;
                    }
                    _ => {
                        // Non-store, non-%rax-write — check if it reads rax
                        if infos[j].reg_refs & 1 != 0 {
                            break; // reads rax, can't eliminate
                        }
                        j += 1;
                        continue;
                    }
                }
            }
            if can_eliminate {
                mark_nop(&mut infos[i]);
                changed = true;
                i += 1;
                continue;
            }
        }

        // Pattern 3: movslq %REGd, %REG (self sign-extension) where the register
        // is overwritten before being read in 64-bit context. Also handles the
        // common accumulator pattern:
        //   movslq %REGd, %REG       → NOP (or kept if REG has other 64-bit readers)
        //   movq %REG, %rax          → movl %REGd, %eax (32-bit copy)
        //   addl/imull/.., %eax      → unchanged (only uses 32 bits)
        if t.starts_with("movslq %") {
            let parts: Vec<&str> = t.split(", ").collect();
            if parts.len() == 2 {
                let src = parts[0].trim_start_matches("movslq ");
                let dst = parts[1];
                // Use register family comparison to detect self sign-extension.
                // This handles both classic regs (%ebx→%rbx) and extended (%r8d→%r8).
                let src_family = super::super::types::register_family_fast(src);
                let dst_family = super::super::types::register_family_fast(dst);
                if src_family == dst_family && src_family != super::super::types::REG_NONE {
                    let reg_family = dst_family;
                    if reg_family != 0 {
                        // rax is handled by cltq pattern
                        let reg_bit = 1u16 << reg_family;

                        // Sub-pattern 3a: next non-NOP is `movq %REG, %rax` followed
                        // by a 32-bit consumer of %eax. Replace both movslq+movq with
                        // a single `movl %REGd, %eax`.
                        let mut j = i + 1;
                        while j < len && infos[j].is_nop() {
                            j += 1;
                        }
                        if j < len && !infos[j].is_barrier() {
                            let reg64 = dst; // e.g., "%rbx"
                            let expected_movq = format!("movq {}, %rax", reg64);
                            let tj = infos[j].trimmed(store.get(j));
                            if tj == expected_movq {
                                // Found movq %REG, %rax. Check the next instruction
                                // is a 32-bit consumer of %eax.
                                let mut k = j + 1;
                                while k < len && infos[k].is_nop() {
                                    k += 1;
                                }
                                // A store of %rax persists all 64 bits, so it is
                                // NOT a 32-bit consumer: replacing the sign-extending
                                // movslq+movq with a zero-extending movl would corrupt
                                // the stored upper half.
                                let next_is_32bit = if k < len && !infos[k].is_barrier() {
                                    match infos[k].kind {
                                        LineKind::Other { dest_reg: 0 } => {
                                            let tk = infos[k].trimmed(store.get(k));
                                            is_32bit_eax_consumer(tk)
                                        }
                                        _ => false,
                                    }
                                } else {
                                    false
                                };

                                if next_is_32bit {
                                    // Verify %REG is overwritten before next 64-bit read
                                    let mut m = j + 1;
                                    let mut reg_safe = true;
                                    while m < len && m < j + 12 {
                                        if infos[m].is_nop() {
                                            m += 1;
                                            continue;
                                        }
                                        if infos[m].is_barrier() {
                                            break;
                                        }
                                        if infos[m].reg_refs & reg_bit == 0 {
                                            m += 1;
                                            continue;
                                        }
                                        // Found reference to %REG
                                        match infos[m].kind {
                                            LineKind::LoadRbp { reg, .. } if reg == reg_family => {
                                                break; // overwritten → safe
                                            }
                                            LineKind::Other { dest_reg }
                                                if dest_reg == reg_family =>
                                            {
                                                break; // overwritten → safe
                                            }
                                            _ => {
                                                reg_safe = false;
                                                break;
                                            }
                                        }
                                    }

                                    if reg_safe {
                                        // Replace: NOP the movslq, replace movq with movl
                                        mark_nop(&mut infos[i]);
                                        let new_movl = format!("    movl {}, %eax", src);
                                        replace_line(store, &mut infos[j], j, new_movl);
                                        changed = true;
                                        i += 1;
                                        continue;
                                    }
                                }
                            }
                        }

                        // Sub-pattern 3b: %REG is overwritten before any 64-bit read.
                        // Also safe if %REG is only read in 32-bit form (%REGd).
                        let reg32_suffix = src; // e.g. "%ebx"
                        let reg64_name = dst; // e.g. "%rbx"
                        let mut j2 = i + 1;
                        let mut can_eliminate = false;
                        while j2 < len && j2 < i + 12 {
                            if infos[j2].is_nop() {
                                j2 += 1;
                                continue;
                            }
                            if infos[j2].is_barrier() {
                                break;
                            }
                            if infos[j2].reg_refs & reg_bit == 0 {
                                j2 += 1;
                                continue;
                            }
                            // This instruction references our register family.
                            match infos[j2].kind {
                                LineKind::LoadRbp { reg, .. } if reg == reg_family => {
                                    // Overwritten by load from stack → safe
                                    can_eliminate = true;
                                    break;
                                }
                                LineKind::Other { dest_reg } if dest_reg == reg_family => {
                                    // Written to by another instruction → safe, UNLESS the
                                    // instruction also READS the 64-bit form (e.g.
                                    // `movzbl (%rcx,%rdi),%edi` uses %rdi as a SIB index
                                    // while writing %edi). NOP-ing the preceding movslq
                                    // then misaddresses the load for negative indices.
                                    let line_text = infos[j2].trimmed(store.get(j2));
                                    if line_text.contains(reg64_name) {
                                        break;
                                    }
                                    can_eliminate = true;
                                    break;
                                }
                                _ => {
                                    // Check if it only reads the 32-bit form.
                                    // If the line contains %REGd but NOT %REG (64-bit),
                                    // the sign-extension upper bits don't matter.
                                    let line_text = infos[j2].trimmed(store.get(j2));
                                    if line_text.contains(reg32_suffix)
                                        && !line_text.contains(reg64_name)
                                    {
                                        // 32-bit-only read → safe, continue scanning
                                        j2 += 1;
                                        continue;
                                    }
                                    // 64-bit read → NOT safe
                                    break;
                                }
                            }
                        }
                        if can_eliminate {
                            mark_nop(&mut infos[i]);
                            changed = true;
                            i += 1;
                            continue;
                        }
                    }
                }
            }
        }

        i += 1;
    }
    changed
}

// ── LEA-to-memory SIB folding ───────────────────────────────────────────────
//
// Fold the already-emitted form:
//   leaq (%base,%index,scale), %tmp
//   movX (%tmp), %dst
// into:
//   movX (%base,%index,scale), %dst
//
// This is deliberately a late textual peephole. It handles GEPs whose IR
// lifetime proof is unavailable by the time the mature accumulator backend
// emits them, while the whole-function dead-register check keeps the rewrite
// sound across CFG edges.

pub(super) fn fold_lea_into_memory_op(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    let len = store.len();
    let mut changed = false;
    let mut i = 0;
    while i < len {
        if infos[i].is_nop() {
            i += 1;
            continue;
        }
        let lea = infos[i].trimmed(store.get(i));
        if !lea.starts_with("leaq ") {
            i += 1;
            continue;
        }
        let Some((addr_text, dst_text)) = lea[5..].rsplit_once(',') else {
            i += 1;
            continue;
        };
        let addr_text = addr_text.trim();
        let dst_text = dst_text.trim();
        if !dst_text.starts_with('%') {
            i += 1;
            continue;
        }
        let dst_family = register_family_fast(dst_text);
        if dst_family == REG_NONE || dst_family > REG_GP_MAX {
            i += 1;
            continue;
        }
        let Some(open) = addr_text.find('(') else {
            i += 1;
            continue;
        };
        let Some(close) = addr_text.rfind(')') else {
            i += 1;
            continue;
        };
        if close <= open || close + 1 != addr_text.len() {
            i += 1;
            continue;
        }
        let displacement = addr_text[..open].trim();
        let fields: Vec<&str> = addr_text[open + 1..close]
            .split(',')
            .map(str::trim)
            .collect();
        // Single-base form: `leaq disp(%base), %dst` with no index/scale.
        // This is the dominant shape emitted for pointer induction in hot
        // loops (gzip longest_match): `leaq 2(%r9),%rdx; movzbl (%rdx),%edi`.
        // The SIB-only matcher below never fired on it, so each byte compare
        // paid a dead LEA + a register round-trip. Fold it to
        // `movzbl disp(%base),%edi` when the temporary is provably dead.
        if fields.len() == 1 {
            let base = fields[0];
            if !base.starts_with('%') {
                i += 1;
                continue;
            }
            let base_family = register_family_fast(base);
            if base_family == REG_NONE || base_family > REG_GP_MAX || dst_family == base_family {
                i += 1;
                continue;
            }
            let mut j = i + 1;
            while j < len && infos[j].is_nop() {
                j += 1;
            }
            if j >= len || infos[j].is_barrier() {
                i += 1;
                continue;
            }
            let next = infos[j].trimmed(store.get(j));
            let addr_pat = format!("({})", dst_text);
            // Match a BARE `(%dst)` memory operand. A displacement form like
            // `8(%rdx)` or `0x2(%rdx)` must not match — the replacement would
            // corrupt `8(` into `8disp(`.
            let bare_ok = next.match_indices(&addr_pat).any(|(pos, _)| {
                pos == 0 || {
                    let c = next.as_bytes()[pos - 1] as char;
                    c == ' ' || c == ',' || c == '\t'
                }
            });
            if !bare_ok {
                i += 1;
                continue;
            }
            // The address was already computed by the LEA; dropping it is valid
            // only if the destination register is dead after the memory op.
            if fam_read_after(store, infos, j + 1, dst_family) {
                i += 1;
                continue;
            }
            let sib = format!("{}({})", displacement, base);
            let replacement = next.replacen(&addr_pat, &sib, 1);
            if replacement == next {
                i += 1;
                continue;
            }
            // SOUNDNESS: the LEA dest must be FULLY absorbed by the address
            // replacement. If the memory op still references the register
            // OUTSIDE the replaced operand — e.g. as the store SOURCE in
            // `leaq 1296(%rsi),%rdx; movq %rdx,(%rdx)` (glibc __tls_init_tp:
            // pd->robust_head.list = &pd->robust_head, where GVN legally
            // CSEs both GEPs into one value) — dropping the LEA leaves that
            // use reading an unwritten register (LK-24 startup SIGSEGV).
            if line_refs_gp_family(&replacement, dst_family) {
                i += 1;
                continue;
            }
            mark_nop(&mut infos[i]);
            replace_line(store, &mut infos[j], j, format!("    {}", replacement));
            changed = true;
            i = j + 1;
            continue;
        }
        if fields.len() > 3 || fields.iter().any(|field| !field.starts_with('%')) {
            i += 1;
            continue;
        }
        let base = fields[0];
        let index = fields[1];
        let base_family = register_family_fast(base);
        let index_family = register_family_fast(index);
        if base_family == REG_NONE
            || index_family == REG_NONE
            || base_family > REG_GP_MAX
            || index_family > REG_GP_MAX
            || dst_family == base_family
            || dst_family == index_family
        {
            i += 1;
            continue;
        }
        if fields.len() == 3 && !matches!(fields[2], "1" | "2" | "4" | "8") {
            i += 1;
            continue;
        }

        let mut j = i + 1;
        while j < len && infos[j].is_nop() {
            j += 1;
        }
        if j >= len || infos[j].is_barrier() {
            i += 1;
            continue;
        }
        let next = infos[j].trimmed(store.get(j));
        let addr_pat = format!("({})", dst_text);
        let sib = if fields.len() == 3 {
            format!("{}({},{},{})", displacement, base, index, fields[2])
        } else {
            format!("{}({},{})", displacement, base, index)
        };
        if !next.contains(&addr_pat) {
            // A common store shape uses one extra address-register copy:
            //   leaq (...), %rdi
            //   movq %rdi, %rcx
            //   movb $0, (%rcx)
            // Fold that form as well, provided the copied temporary is dead.
            if let Some((src, tmp)) = next
                .strip_prefix("movq ")
                .and_then(|rest| rest.split_once(','))
            {
                let src = src.trim();
                let tmp = tmp.trim();
                let tmp_family = register_family_fast(tmp);
                if src == dst_text
                    && tmp.starts_with('%')
                    && tmp_family <= REG_GP_MAX
                    && tmp_family != base_family
                    && tmp_family != index_family
                {
                    let mut k = j + 1;
                    while k < len && infos[k].is_nop() {
                        k += 1;
                    }
                    if k < len && !infos[k].is_barrier() {
                        let mem_next = infos[k].trimmed(store.get(k));
                        let tmp_pat = format!("({})", tmp);
                        if mem_next.contains(&tmp_pat)
                            && !fam_read_after(store, infos, k + 1, tmp_family)
                            // SOUNDNESS (sqlite3): removing the leaq leaves the
                            // LEA destination register undefined. The
                            // temporary-copy check alone is not enough: the
                            // leaq dest (e.g. %r13 = GEP result) is commonly
                            // read by SUBSEQUENT stores through
                            // displacement-form operands like `movw $0, 2(%r13)`
                            // (field stores of a promoted struct). The fold
                            // must also prove the leaq dest is dead after the
                            // folded memory operation. Without this, sqlite3's
                            // opcode/schema-init store sequence wrote struct
                            // fields through an undefined base register.
                            && !fam_read_after(store, infos, k + 1, dst_family)
                        {
                            let replacement = mem_next.replacen(&tmp_pat, &sib, 1);
                            if replacement != mem_next
                                && !line_refs_gp_family(&replacement, tmp_family)
                                && !line_refs_gp_family(&replacement, dst_family)
                            {
                                mark_nop(&mut infos[i]);
                                mark_nop(&mut infos[j]);
                                replace_line(
                                    store,
                                    &mut infos[k],
                                    k,
                                    format!("    {}", replacement),
                                );
                                changed = true;
                                i = k + 1;
                                continue;
                            }
                        }
                    }
                }
            }
            i += 1;
            continue;
        }
        // The address fields were already evaluated by LEA. Removing it is
        // valid only if the temporary is dead after the memory operation.
        if fam_read_after(store, infos, j + 1, dst_family) {
            i += 1;
            continue;
        }
        let replacement = next.replacen(&addr_pat, &sib, 1);
        if replacement == next {
            i += 1;
            continue;
        }
        // Same full-absorption rule as the single-base arm: a leftover
        // reference to the LEA dest outside the replaced address operand
        // means the LEA is still live and must not be dropped.
        if line_refs_gp_family(&replacement, dst_family) {
            i += 1;
            continue;
        }
        mark_nop(&mut infos[i]);
        replace_line(store, &mut infos[j], j, format!("    {}", replacement));
        changed = true;
        i = j + 1;
    }
    changed
}

/// Does `line` reference any name of GP register family `fam`
/// (%rax/%eax/%ax/%al tiers)? Conservative true for out-of-range families.
fn line_refs_gp_family(line: &str, fam: u8) -> bool {
    use super::super::types::REG_NAMES;
    if fam as usize >= REG_NAMES[0].len() {
        return true;
    }
    for tier in REG_NAMES.iter() {
        let name = tier[fam as usize];
        if name.is_empty() {
            continue;
        }
        // REG_NAMES entries already carry the '%' prefix (see the
        // fold_lea_all_uses_in_block comment about the "%%rdx" bug class);
        // match them verbatim, with a boundary check so %r1 does not match
        // inside %r10.
        let pat = name;
        let mut start = 0;
        while let Some(pos) = line[start..].find(pat) {
            let abs = start + pos;
            let end = abs + pat.len();
            let boundary = line
                .as_bytes()
                .get(end)
                .map_or(true, |&c| !(c as char).is_ascii_alphanumeric());
            if boundary {
                return true;
            }
            start = end;
        }
    }
    false
}

// ── SIB indexed addressing folding ──────────────────────────────────────────
//
// The accumulator-based codegen computes `base + index` manually:
//   movq %REG_IDX, %rax       ; copy index to accumulator
//   addq %REG_BASE, %rax      ; compute address
//   [movq %rax, %REG_TMP]     ; optional: copy to another register
//   movX SRC, (%rax|%REG_TMP) ; store through computed address
//   (or movX (%rax|%REG_TMP), DST for loads)
//
// This pass folds these into x86 SIB indexed addressing:
//   movX SRC, (%REG_BASE,%REG_IDX)
//
// Requirements: REG_IDX and REG_BASE must be callee-saved or otherwise
// guaranteed not clobbered between definition and use.

/// Whole-function scan: does any line after `start` READ register family `fam`?
///
/// Path-insensitive and deliberately conservative: a line that references the
/// family counts as a read unless it is a provable pure write (mov*-family
/// store into the register, or the xor-self zeroing idiom). Implicit reads
/// (cltq/cdq/cqo, integer div/mul, shld/shrd) are detected via
/// [`implicit_read_reg_family`]. The scan stops at the next function's
/// `.cfi_startproc`.
///
/// This is used by [`fold_base_index_addressing`] to prove that a register
/// whose defining instruction is about to be removed is dead after the folded
/// memory operation. The previous window-until-barrier scans missed reads in
/// LATER basic blocks (e.g. phi copy-backs after a branch), which left a
/// never-defined register live and miscompiled switch/loop code (regression:
/// phi_gep_fold.c — zlib-ng zng_deflateSetParams).
fn fam_read_after(store: &LineStore, infos: &[LineInfo], start: usize, fam: u8) -> bool {
    if fam > 15 {
        return true; // unknown family: be conservative
    }
    let mask = 1u16 << fam;
    for n in start..store.len() {
        if infos[n].is_nop() {
            continue;
        }
        let td = infos[n].trimmed(store.get(n));
        if td.starts_with(".cfi_startproc") {
            break; // next function: its registers are independent
        }
        if implicit_read_reg_family(td) == Some(fam) {
            return true;
        }
        if infos[n].reg_refs & mask == 0 {
            continue;
        }
        let dest = get_dest_reg(&infos[n]);
        if dest == fam {
            // Pure write: mov-family store to the register (no memory operand
            // through it) or xor-self zeroing. Anything else with dest == fam
            // (addq %r8,%rax, ...) also READS the register.
            let name64 = REG_NAMES[0][fam as usize];
            let name32 = REG_NAMES[1][fam as usize];
            // SOUNDNESS: a mov to the family is a pure write only if the
            // SOURCE part does not reference the family — including through
            // displacement-form memory operands like `movq 8(%r13), %r13`
            // (or `movl 4(%r13), %r13d`), which the old `(%r13)`-substring
            // checks missed (`8(%r13)` does not contain `(r13)`). Such a
            // line READS the family, so it must block the fold.
            let src_part = td[..td.rfind(',').unwrap_or(td.len())].to_string();
            let fam_in_src = src_part.contains(&format!("%{}", name64))
                || src_part.contains(&format!("%{}", name32));
            let mov_store =
                (td.starts_with("mov") || td.starts_with("movabs") || td.starts_with("lea"))
                    && !fam_in_src;
            let explicit_lea_write = td.starts_with("lea")
                && td.ends_with(&format!(", %{}", name64))
                && !td[..td.rfind(',').unwrap_or(0)].contains(name64);
            let xor_self = (td.starts_with("xorl ") || td.starts_with("xorq "))
                && td.contains(&format!("{}, {}", name32, name32));
            if mov_store || explicit_lea_write || xor_self {
                continue;
            }
        }
        return true;
    }
    false
}

/// Fold `leaq (%base,%index[,scale]), %tmp` into EVERY memory operand that
/// uses `(%tmp)` within the same basic block, then delete the LEA.
///
/// `fold_lea_into_memory_op` only rewrites the IMMEDIATELY following
/// instruction and only when the temporary dies at that single use. Vectorized
/// inner loops break both assumptions at once. matmul's kernel is
///
/// ```text
///     leaq (%r8,%r10), %r11
///     leaq (%rsi,%r10), %r13          <- next insn is another LEA
///     vmovupd (%r13), %ymm0           <- %r13 used here ...
///     vfmadd231pd (%r11), %ymm1, %ymm0
///     vmovupd %ymm0, (%r13)           <- ... and again here
/// ```
///
/// so neither LEA folds and the loop pays two address computations plus two
/// registers per iteration. x86 addresses the whole thing for free in the SIB
/// byte, which is what ICX emits (`vfmadd213pd ymm1, ymm0, [rax+8*r8-64]`).
///
/// Soundness requirements, all enforced below:
///   * scan stops at the first barrier — folding across a branch would move an
///     address computation onto a path that never executed it;
///   * every intervening instruction must leave BOTH base and index untouched,
///     otherwise the folded operand would read different registers than the
///     LEA did (this is the check the older pass gets for free by only ever
///     looking at the very next line);
///   * `%tmp` must be dead after the last rewritten use, and must not be read
///     in any form the rewrite does not cover (a bare `(%tmp)` is the only
///     shape matched; `8(%tmp)` and plain register reads block the fold);
///   * `%tmp` must differ from base and index, or deleting the LEA would
///     change the value the folded operand reads.
pub(super) fn fold_lea_all_uses_in_block(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    let len = store.len();
    let mut changed = false;
    let mut i = 0;

    while i < len {
        if infos[i].is_nop() {
            i += 1;
            continue;
        }
        let lea = infos[i].trimmed(store.get(i));
        if !lea.starts_with("leaq ") {
            i += 1;
            continue;
        }

        let Some((addr_text, dst_text)) = lea[5..].rsplit_once(',') else {
            i += 1;
            continue;
        };
        let (addr_text, dst_text) = (addr_text.trim(), dst_text.trim());
        if !dst_text.starts_with('%') {
            i += 1;
            continue;
        }
        let dst_family = register_family_fast(dst_text);
        if dst_family == REG_NONE || dst_family > REG_GP_MAX {
            i += 1;
            continue;
        }

        let (Some(open), Some(close)) = (addr_text.find('('), addr_text.rfind(')')) else {
            i += 1;
            continue;
        };
        if close <= open || close + 1 != addr_text.len() {
            i += 1;
            continue;
        }
        let displacement = addr_text[..open].trim();
        let fields: Vec<&str> = addr_text[open + 1..close]
            .split(',')
            .map(str::trim)
            .collect();
        // Only the two- and three-field SIB forms; the single-base form is
        // already handled (and folding it here would duplicate that work).
        if fields.len() < 2 || fields.len() > 3 {
            i += 1;
            continue;
        }
        if fields.iter().any(|f| !f.starts_with('%')) {
            i += 1;
            continue;
        }
        if fields.len() == 3 && !matches!(fields[2], "1" | "2" | "4" | "8") {
            i += 1;
            continue;
        }

        let base_family = register_family_fast(fields[0]);
        let index_family = register_family_fast(fields[1]);
        if base_family == REG_NONE
            || index_family == REG_NONE
            || base_family > REG_GP_MAX
            || index_family > REG_GP_MAX
            || dst_family == base_family
            || dst_family == index_family
        {
            i += 1;
            continue;
        }

        let sib = if fields.len() == 3 {
            format!(
                "{}({},{},{})",
                displacement, fields[0], fields[1], fields[2]
            )
        } else {
            format!("{}({},{})", displacement, fields[0], fields[1])
        };
        let addr_pat = format!("({})", dst_text);
        let dst64 = REG_NAMES[0][dst_family as usize];
        let dst32 = REG_NAMES[1][dst_family as usize];
        let base_mask = 1u16 << base_family;
        let index_mask = 1u16 << index_family;
        let dst_mask = 1u16 << dst_family;

        // Collect every rewritable use up to the end of the block.
        let mut uses: Vec<usize> = Vec::new();
        let mut ok = true;
        let mut n = i + 1;
        while n < len {
            if infos[n].is_nop() {
                n += 1;
                continue;
            }
            if infos[n].is_barrier() {
                break;
            }
            let t = infos[n].trimmed(store.get(n));

            // Base or index redefined (explicitly or implicitly — `cqto`
            // overwrites %rdx without naming it) => the LEA's value is no
            // longer reproducible from them; stop (uses collected so far are
            // still valid only if we fold nothing after this point).
            if writes_family(&infos[n], t, base_family)
                || writes_family(&infos[n], t, index_family)
            {
                break;
            }
            if infos[n].reg_refs & dst_mask != 0 {
                // A bare `(%tmp)` operand is rewritable. Anything else that
                // mentions the register (displacement form, plain read, or a
                // write) is not, so the LEA has to stay.
                let bare = t.match_indices(&addr_pat).any(|(pos, _)| {
                    pos == 0 || matches!(t.as_bytes()[pos - 1] as char, ' ' | ',' | '\t')
                });
                // REG_NAMES entries already include the '%' prefix; the old
                // format!("%{}", ..) built "%%rdx", which never matched, so a
                // redefinition of the destination register was INVISIBLE to
                // this scan. The first LEA then folded uses belonging to a
                // second LEA into the same register (vectorize_float_matmul:
                // C[0][0] += ... executed against A's address).
                let mentions = t.contains(dst64) || t.contains(dst32);
                if bare {
                    // Reject a line that ALSO uses %tmp in a non-bare position.
                    let stripped = t.replace(&addr_pat, "");
                    if stripped.contains(dst64) || stripped.contains(dst32) {
                        ok = false;
                        break;
                    }
                    uses.push(n);
                } else if mentions {
                    // A pure redefinition of %tmp ends its live range cleanly:
                    // everything after belongs to a different value.
                    // `writes_family` also sees architectural implicit writes
                    // (`cqto` redefining an %rdx temporary).
                    if writes_family(&infos[n], t, dst_family)
                        && !t[..t.rfind(',').unwrap_or(t.len())].contains(dst64)
                    {
                        break;
                    }
                    ok = false;
                    break;
                }
            }
            let _ = (base_mask, index_mask);
            n += 1;
        }

        // Need at least two rewritable uses to beat the existing single-use
        // pass; with one use that pass already fires (and is better tested).
        if !ok || uses.len() < 2 {
            i += 1;
            continue;
        }
        // %tmp must be dead after the block region we rewrote.
        if fam_read_after(store, infos, n, dst_family) {
            i += 1;
            continue;
        }

        for &u in &uses {
            let t = infos[u].trimmed(store.get(u));
            let replacement = t.replacen(&addr_pat, &sib, 1);
            if replacement == t {
                ok = false;
                break;
            }
            replace_line(store, &mut infos[u], u, format!("    {}", replacement));
        }
        if ok {
            mark_nop(&mut infos[i]);
            changed = true;
        }
        i += 1;
    }
    changed
}

pub(super) fn fold_base_index_addressing(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    let len = store.len();
    let mut changed = false;
    let mut i = 0;

    while i < len {
        if infos[i].is_nop() {
            i += 1;
            continue;
        }

        // Look for: movq %REG, %rax (copy a GP register to accumulator)
        let ti = infos[i].trimmed(store.get(i));
        if !ti.starts_with("movq %") || !ti.ends_with(", %rax") {
            i += 1;
            continue;
        }

        // Extract the source register (the index)
        let idx_reg = &ti[5..ti.len() - 6]; // strip "movq " prefix and ", %rax" suffix
        if !idx_reg.starts_with('%') || idx_reg == "%rax" || idx_reg == "%rcx" {
            i += 1;
            continue;
        }
        // Must be a valid GP register
        let idx_family = register_family_fast(idx_reg);
        if idx_family == REG_NONE || idx_family > REG_GP_MAX {
            i += 1;
            continue;
        }

        // Next non-NOP: must be addq %REG_BASE, %rax
        let mut j = i + 1;
        while j < len && infos[j].is_nop() {
            j += 1;
        }
        if j >= len || infos[j].is_barrier() {
            i += 1;
            continue;
        }

        let tj = infos[j].trimmed(store.get(j));
        if !tj.starts_with("addq %") || !tj.ends_with(", %rax") {
            i += 1;
            continue;
        }

        let base_reg = &tj[5..tj.len() - 6]; // strip "addq " and ", %rax"
        if !base_reg.starts_with('%') || base_reg == "%rax" {
            i += 1;
            continue;
        }
        let base_family = register_family_fast(base_reg);
        if base_family == REG_NONE || base_family > REG_GP_MAX {
            i += 1;
            continue;
        }

        // Next non-NOP: either a memory op using (%rax), or movq %rax, %REG_TMP
        let mut k = j + 1;
        while k < len && infos[k].is_nop() {
            k += 1;
        }
        if k >= len || infos[k].is_barrier() {
            i += 1;
            continue;
        }

        let tk = infos[k].trimmed(store.get(k));

        // Case 1: Direct use — the mem op uses (%rax)
        if let Some(folded) = try_fold_mem_op_with_sib(tk, "(%rax)", base_reg, idx_reg) {
            // Safety: verify rax is dead after k. The NOP'd instructions
            // leave rax without the computed address. If anything reads rax
            // after k expecting the address, the fold is unsafe. The scan is
            // whole-function (not window-until-barrier): a read in a LATER
            // basic block is just as unsafe as one in the same block.
            let rax_dead = !fam_read_after(store, infos, k + 1, 0);
            if rax_dead {
                mark_nop(&mut infos[i]); // remove movq %REG, %rax
                mark_nop(&mut infos[j]); // remove addq %REG_BASE, %rax
                replace_line(store, &mut infos[k], k, folded);
                changed = true;
                i = k + 1;
                continue;
            }
        }

        // Case 2: Intermediate copy — movq %rax, %REG_TMP, then mem op on (%REG_TMP)
        // tmp can equal base_reg (common: base loaded into %rcx, then %rcx reused
        // for the computed address). After we eliminate the addq, %base_reg still
        // holds the original base value, so SIB (%base,%idx) is correct.
        if tk.starts_with("movq %rax, %") {
            let tmp_reg = &tk[11..]; // after "movq %rax, " (includes the %)
            let tmp_family = register_family_fast(tmp_reg);
            if tmp_family != REG_NONE && tmp_family <= REG_GP_MAX && tmp_family != idx_family
            // idx must differ from tmp
            {
                let mut m = k + 1;
                while m < len && infos[m].is_nop() {
                    m += 1;
                }
                if m < len && !infos[m].is_barrier() {
                    let tm = infos[m].trimmed(store.get(m));
                    let addr_pat = format!("(%{})", &tmp_reg[1..]); // e.g. "(%rcx)"
                    if let Some(folded) = try_fold_mem_op_with_sib(tm, &addr_pat, base_reg, idx_reg)
                    {
                        // Safety: verify %TMP is dead after m. If anything
                        // reads %TMP after the folded mem op, the NOP'd movq
                        // at k means %TMP holds a stale value. Whole-function
                        // scan (not window-until-barrier): cross-block reads
                        // (e.g. phi copy-backs) are just as unsafe.
                        let tmp_dead = !fam_read_after(store, infos, m + 1, tmp_family);
                        // SOUNDNESS: the fold removes the `movq %idx, %rax`
                        // AND `addq %base, %rax`, leaving %rax undefined. The
                        // tmp-copy deadness alone is not sufficient: a later
                        // instruction may read %rax expecting the computed
                        // address (the accumulator is reused constantly).
                        let rax_dead = !fam_read_after(store, infos, m + 1, 0);

                        if tmp_dead && rax_dead {
                            mark_nop(&mut infos[i]); // remove movq %REG, %rax
                            mark_nop(&mut infos[j]); // remove addq
                            mark_nop(&mut infos[k]); // remove movq %rax, %TMP
                            replace_line(store, &mut infos[m], m, folded);
                            changed = true;
                            i = m + 1;
                            continue;
                        }
                    }
                }
            }
        }

        i += 1;
    }
    changed
}

/// Try to replace a memory operand `(ADDR_PAT)` in an instruction with SIB `(%BASE,%IDX)`.
/// Returns the new instruction text if the pattern matches.
fn try_fold_mem_op_with_sib(
    instr: &str,
    addr_pat: &str,
    base_reg: &str,
    idx_reg: &str,
) -> Option<String> {
    // The instruction must contain the address pattern exactly once
    if !instr.contains(addr_pat) {
        return None;
    }
    // Don't fold into instructions that also reference rax/rcx in a way that conflicts
    // (the movq/addq we're removing clobber rax)
    // Build the SIB replacement
    let sib = format!("(%{}, %{})", &base_reg[1..], &idx_reg[1..]);
    let new_instr = format!("    {}", instr.replace(addr_pat, &sib));
    Some(new_instr)
}

// ── Accumulator ALU + store folding ─────────────────────────────────────────
//
// Folds the pattern:
//   movl %REGd, %eax            (copy 32-bit value to accumulator)
//   addl OFFSET(%rsp), %eax     (32-bit ALU op with memory source)
//   cltq                        (sign-extend for 64-bit store)
//   movq %rax, OFFSET2(%rsp)    (64-bit store)
//
// Into:
//   addl OFFSET(%rsp), %REGd    (ALU directly in register)
//   movl %REGd, OFFSET2(%rsp)   (32-bit store, no sign-extend needed)
//
// Saves 2 instructions per occurrence. Common in arith_loop where 32-bit
// variables are stored to 64-bit stack slots through the accumulator.

pub(super) fn fold_accumulator_alu_store(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    let len = store.len();
    let mut changed = false;
    let mut i = 0;

    while i < len {
        if infos[i].is_nop() {
            i += 1;
            continue;
        }

        // Step 1: Look for `movl %REGd, %eax`
        let (src_reg32, src_family) = {
            let ti = infos[i].trimmed(store.get(i));
            if !ti.starts_with("movl %") || !ti.ends_with(", %eax") {
                i += 1;
                continue;
            }
            let sr = &ti[5..ti.len() - 6];
            if !sr.starts_with('%') || sr == "%eax" {
                i += 1;
                continue;
            }
            let sf = register_family_fast(sr);
            if sf == REG_NONE || sf == 0 {
                i += 1;
                continue;
            }
            (sr.to_string(), sf)
        };

        // Step 2: Next non-NOP must be `addl/subl/andl/orl/xorl STACK, %eax`
        let mut j = i + 1;
        while j < len && infos[j].is_nop() {
            j += 1;
        }
        if j >= len || infos[j].is_barrier() {
            i += 1;
            continue;
        }

        let (alu_op_s, mem_src_s) = {
            let tj = infos[j].trimmed(store.get(j));
            let ao = if let Some(pos) = tj.find(' ') {
                &tj[..pos]
            } else {
                i += 1;
                continue;
            };
            if !matches!(ao, "addl" | "subl" | "andl" | "orl" | "xorl") {
                i += 1;
                continue;
            }
            if !tj.ends_with(", %eax") {
                i += 1;
                continue;
            }
            let ms = tj[ao.len() + 1..tj.len() - 6].trim().to_string();
            if !ms.ends_with("(%rsp)") && !ms.ends_with("(%rbp)") {
                i += 1;
                continue;
            }
            (ao.to_string(), ms)
        };

        // Step 3: The instruction after the ALU must be a 32-bit STORE of
        // %eax directly (no `cltq` sign-extension in between).
        //
        // SOUNDNESS: the original pattern was `movl %REGd,%eax;
        // OP mem,%eax; cltq; movq %rax,slot`, and the pass rewrote it to
        // `OP mem,%REGd; movl %REGd,slot`. That was UNSOUND: `cltq` sign-extends
        // the 32-bit ALU result to 64 bits, so the original `movq %rax,slot`
        // stores 8 bytes (upper 4 bytes = 0xFFFFFFFF when the result is
        // negative). The rewrite stored only 4 bytes (`movl %REGd`), corrupting
        // the adjacent stack slot and dropping the sign extension. We therefore
        // only fold when the ALU result is stored as 32 bits with NO sign
        // extension (`movl %eax,slot`), so `OP mem,%REGd; movl %REGd,slot` is
        // exactly equivalent.
        let mut m = j + 1;
        while m < len && infos[m].is_nop() {
            m += 1;
        }
        if m >= len || infos[m].is_barrier() {
            i += 1;
            continue;
        }

        let store_dst_s = {
            let is_32bit_store_rax = match infos[m].kind {
                LineKind::StoreRbp {
                    reg: 0,
                    size: MoveSize::L,
                    ..
                } => true,
                _ => {
                    let tm = infos[m].trimmed(store.get(m));
                    tm.starts_with("movl %eax, ")
                        && (tm.ends_with("(%rsp)") || tm.ends_with("(%rbp)"))
                }
            };
            if !is_32bit_store_rax {
                i += 1;
                continue;
            }
            let tm = infos[m].trimmed(store.get(m));
            tm[11..].to_string() // after "movl %eax, "
        };

        // Step 4: Verify %SRC_REG is dead from the point we modify it (j)
        // until it is overwritten. The transform does `OP mem, %REGd`, which
        // DESTROYS %REGd; if %REGd is read before being overwritten (or is live
        // on another edge at a barrier), the transform is unsound.
        //
        // ms178 (session 32): the old scan had TWO soundness holes that
        // miscompiled HUF_readDTableX2_wksp -O2 (kernel zstd):
        //   1. It was limited to a 16-instruction window (`n < j + 16`). A
        //      register whose next use lay beyond the window (e.g. a value
        //      homed in %r13 whose live range continues into a loop whose
        //      header is >16 instructions away) was treated as dead, the fold
        //      fired, and the loop read the clobbered register — nbBits became
        //      scaleLog (12-11=1) and HUF_fillDTableX2ForWeight overflowed the
        //      DTable with length=1<<11.
        //   2. In the fallback arm, an instruction that merely REFERENCES the
        //      source register (`tn.contains(src_reg32)`) was treated as
        //      benign and skipped. A reference is a READ: the fold destroys
        //      the value, so any read before the next overwrite is fatal.
        //   The scan must therefore run to the next barrier (labels/branches
        //   delimit the region where liveness can be proven block-locally) and
        //   treat ANY reference that is not a recognized overwrite as a use.
        let src_bit = 1u16 << src_family;
        let mut n = j + 1;
        let mut src_safe = true;
        while n < len {
            if infos[n].is_nop() {
                n += 1;
                continue;
            }
            if infos[n].is_barrier() {
                // A branch/label means %SRC may be LIVE on another path; the
                // transform overwrites %SRC with the ALU result, so we cannot
                // prove safety here. This was the soundness bug (previously it
                // broke and left src_safe=true, corrupting %SRC on other edges).
                src_safe = false;
                break;
            }
            if infos[n].reg_refs & src_bit == 0 {
                n += 1;
                continue;
            }
            match infos[n].kind {
                LineKind::LoadRbp { reg, .. } if reg == src_family => break,
                LineKind::Other { dest_reg } if dest_reg == src_family => break,
                _ => {
                    // Any other reference to the source family is a READ of
                    // the value the fold is about to destroy (or an
                    // unrecognized write form): not provably safe.
                    src_safe = false;
                    break;
                }
            }
        }
        if !src_safe {
            i += 1;
            continue;
        }

        // SOUNDNESS (v9 / BUG-003): after the fold, %eax no longer holds the
        // ALU result — `OP mem, %REGd` writes %REG, and the store is rewritten
        // to `movl %REGd, slot`. Later uses that still read %eax expecting the
        // add result (inlined adler32_copy_tail: `sum2 += new_adler` after
        // `adler += byte`) would observe the *pre-add* copy, typically the
        // just-loaded byte. Require %eax to be dead or overwritten after the
        // store before folding.
        if !rax_elidable_after(store, infos, m + 1, len) {
            i += 1;
            continue;
        }

        // Transform! Replace 3 instructions with 2 (no cltq, 32-bit store).
        mark_nop(&mut infos[i]);
        let new_alu = format!("    {} {}, {}", alu_op_s, mem_src_s, src_reg32);
        replace_line(store, &mut infos[j], j, new_alu);
        let new_store = format!("    movl {}, {}", src_reg32, store_dst_s);
        replace_line(store, &mut infos[m], m, new_store);

        changed = true;
        i = m + 1;
    }
    changed
}

/// Check if an instruction is a 32-bit operation that consumes %eax,
/// meaning it only uses the lower 32 bits and then zero-extends the result.
/// Examples: addl, subl, imull, andl, orl, xorl, movl, etc.
fn is_32bit_eax_consumer(trimmed: &str) -> bool {
    // 32-bit ALU ops on %eax — these read %eax (lower 32 bits only)
    // and write back to %eax (zero-extending to 64 bits).
    if trimmed.ends_with(", %eax") || trimmed.ends_with("l %eax") {
        let op = if let Some(pos) = trimmed.find(' ') {
            &trimmed[..pos]
        } else {
            trimmed
        };
        return matches!(
            op,
            "addl"
                | "subl"
                | "imull"
                | "andl"
                | "orl"
                | "xorl"
                | "shll"
                | "shrl"
                | "sarl"
                | "movl"
                | "leal"
        );
    }
    false
}

// ── FP XMM↔GPR round-trip elimination ────────────────────────────────────────
//
// float_ops.rs emits FP binops as:
//   movq -N(%rbp), %rax    ; load lhs into GPR
//   movq %rax, %xmm0       ; shuttle to XMM ← wasteful
//   movq -M(%rbp), %rcx    ; load rhs into GPR
//   movq %rcx, %xmm1       ; shuttle to XMM ← wasteful
//   mulsd %xmm1, %xmm0     ; actual operation
//   movq %xmm0, %rax       ; result back to GPR ← wasteful
//   movq %rax, -P(%rbp)    ; store
//
// This pass eliminates the GPR intermediaries:
//   LoadRbp{rax,Q}  + "movq %rax, %xmm0"  → "movsd -N(%rbp), %xmm0"
//   LoadRbp{rcx,Q}  + "movq %rcx, %xmm1"  → "movsd -M(%rbp), %xmm1"
//   "movq %xmm0,%rax" + StoreRbp{rax,Q}   → "movsd %xmm0, -P(%rbp)"
//
// This reduces 7 instructions to 4 (then fold_fp_memory_operands reduces to 3).

pub(super) fn eliminate_fp_xmm_roundtrips(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    let mut changed = false;
    let len = store.len();
    let mut i = 0;

    while i < len {
        if infos[i].is_nop() {
            i += 1;
            continue;
        }

        // Pattern X: XMM -> GPR -> XMM relay. The mature emitter uses the
        // integer accumulator as a compatibility bridge for values that are
        // already in XMM registers. Replace the two bit-moves with one scalar
        // FP move when the bridge register is dead afterwards.
        let line_i = infos[i].trimmed(store.get(i));
        if line_i.starts_with("movq %xmm") {
            let Some((src, dst_gpr)) = line_i[5..].rsplit_once(',') else {
                i += 1;
                continue;
            };
            let src = src.trim();
            let dst_gpr = dst_gpr.trim();
            let gpr_family = register_family_fast(dst_gpr);
            if (dst_gpr == "%rax" || dst_gpr == "%rcx")
                && gpr_family <= REG_GP_MAX
                && src.starts_with('%')
            {
                let mut j = i + 1;
                while j < len && infos[j].is_nop() {
                    j += 1;
                }
                if j < len {
                    let relay = infos[j].trimmed(store.get(j));
                    let expected = format!("movq {}, %xmm", dst_gpr);
                    if relay.starts_with(&expected) {
                        let dst_xmm = relay
                            .rsplit_once(',')
                            .map(|(_, dst)| dst.trim())
                            .unwrap_or("");
                        if !dst_xmm.is_empty() && !fam_read_after(store, infos, j + 1, gpr_family) {
                            let replacement = format!("    movsd {}, {}", src, dst_xmm);
                            replace_line(store, &mut infos[i], i, replacement);
                            mark_nop(&mut infos[j]);
                            changed = true;
                            i = j + 1;
                            continue;
                        }
                    }
                }
            }
        }

        // Pattern A: LoadRbp{rax(0) or rcx(1), Q} then "movq %gpr, %xmmN"
        if let LineKind::LoadRbp {
            reg: load_reg,
            offset,
            size: MoveSize::Q,
        } = infos[i].kind
        {
            if load_reg <= 1 {
                let mut j = i + 1;
                while j < len && j < i + 4 && infos[j].is_nop() {
                    j += 1;
                }
                if j < len && !infos[j].is_nop() {
                    let line_j = infos[j].trimmed(store.get(j));
                    // Generalize to any %xmmN destination (the emitter rotates
                    // through xmm2..xmm13); the load_reg selects the bridge GPR.
                    let bridge = if load_reg == 0 {
                        "movq %rax, "
                    } else {
                        "movq %rcx, "
                    };
                    if line_j.starts_with(bridge) {
                        let xmm_str = &line_j[bridge.len()..];
                        // SOUNDNESS: this rewrites the LOAD itself, so the
                        // bridge GPR loses its only definition. It may only be
                        // dropped when nothing reads it afterwards. Pattern B
                        // has always checked this (rax_elidable_after); Pattern
                        // A never did, and generalising it from %xmm0/%xmm1 to
                        // all sixteen XMM registers widened a latent
                        // miscompile: for
                        //     movq -24(%rbp), %rax
                        //     movq %rax, %xmm7
                        //     addq $7, %rax        <- still reads %rax
                        // the pass emitted `movsd -24(%rbp), %xmm7` and left
                        // `addq $7, %rax` reading a register nothing defines.
                        if xmm_str.starts_with("%xmm") && !xmm_str.contains(' ') {
                            let mut k = j + 1;
                            while k < len && infos[k].is_nop() {
                                k += 1;
                            }
                            let bridge_dead = if load_reg == 0 {
                                rax_elidable_after(store, infos, k, len)
                            } else {
                                !fam_read_after(store, infos, k, 1 /* rcx */)
                            };
                            if bridge_dead {
                                let load_text = infos[i].trimmed(store.get(i));
                                let base = if load_text.contains("(%rsp)") {
                                    "rsp"
                                } else {
                                    "rbp"
                                };
                                let new_text =
                                    format!("    movsd {}(%{}), {}", offset, base, xmm_str);
                                replace_line(store, &mut infos[i], i, new_text);
                                mark_nop(&mut infos[j]);
                                changed = true;
                                i += 1;
                                continue;
                            }
                        }
                    }
                }
            }
        }

        // Pattern B: "movq %xmmN, %rax" then StoreRbp{rax, Q} → one movsd.
        // The original pass only matched %xmm0; the accumulator-based FP
        // emitter rotates through %xmm2..%xmm13, so every other spill still
        // paid the GPR round-trip. Match any %xmmN (a valid 64-bit bit-copy
        // source) whose %rax bridge register is dead afterwards.
        if let LineKind::Other { dest_reg: 0 } = infos[i].kind {
            let line_i = infos[i].trimmed(store.get(i));
            if line_i.starts_with("movq %xmm") && line_i.ends_with(", %rax") {
                let src_xmm = &line_i[5..line_i.len() - 6]; // strip "movq " and ", %rax"
                if src_xmm.starts_with("%xmm") {
                    let mut j = i + 1;
                    while j < len && j < i + 4 && infos[j].is_nop() {
                        j += 1;
                    }
                    if j < len {
                        if let LineKind::StoreRbp {
                            reg: 0,
                            offset,
                            size: MoveSize::Q,
                        } = infos[j].kind
                        {
                            let mut k = j + 1;
                            while k < len && infos[k].is_nop() {
                                k += 1;
                            }
                            if rax_elidable_after(store, infos, k, len) {
                                let store_text = infos[j].trimmed(store.get(j));
                                let base = if store_text.contains("(%rsp)") {
                                    "rsp"
                                } else {
                                    "rbp"
                                };
                                let new_text =
                                    format!("    movsd {}, {}(%{})", src_xmm, offset, base);
                                mark_nop(&mut infos[i]);
                                replace_line(store, &mut infos[j], j, new_text);
                                changed = true;
                                i = j + 1;
                                continue;
                            }
                        }
                    }
                }
            }
        }

        // Pattern E: StoreRbp{rax, O} immediately followed by "movq %rax, %xmmN"
        // The stack store is dead (value used from %rax directly). NOP it so
        // Pattern D can fire on the adjacent movq-from-ptr + movq-to-xmm.
        if let LineKind::StoreRbp {
            reg: 0,
            offset,
            size: MoveSize::Q,
        } = infos[i].kind
        {
            let mut j = i + 1;
            while j < len && j < i + 4 && infos[j].is_nop() {
                j += 1;
            }
            if j < len {
                let line_j = infos[j].trimmed(store.get(j));
                if line_j == "movq %rax, %xmm0" || line_j == "movq %rax, %xmm1" {
                    // Verify stack slot O is not read between j+1 and block end.
                    if rbp_offset_dead_after(store, infos, j + 1, len, offset as i64) {
                        mark_nop(&mut infos[i]);
                        changed = true;
                        // Don't advance i past j; let Pattern D fire next iteration.
                        i += 1;
                        continue;
                    }
                }
            }
        }

        // Pattern D: "movq (%<ptr>), %rax" immediately followed by
        // "movq %rax, %xmmN" → "movsd (%<ptr>), %xmmN".
        // Fires after Pattern E removes the intervening dead StoreRbp.
        if let LineKind::Other { dest_reg: 0 } = infos[i].kind {
            let line_i = infos[i].trimmed(store.get(i));
            if line_i.starts_with("movq (%") && line_i.ends_with("), %rax") {
                let mut j = i + 1;
                while j < len && j < i + 4 && infos[j].is_nop() {
                    j += 1;
                }
                if j < len {
                    let line_j = infos[j].trimmed(store.get(j));
                    let xmm = if line_j == "movq %rax, %xmm0" {
                        Some("%xmm0")
                    } else if line_j == "movq %rax, %xmm1" {
                        Some("%xmm1")
                    } else {
                        None
                    };
                    if let Some(xmm_str) = xmm {
                        // Extract pointer register from "movq (%<ptr>), %rax"
                        let ptr_reg = &line_i[7..line_i.len() - 7]; // strip "movq (%" and "), %rax"
                        let mut k = j + 1;
                        while k < len && infos[k].is_nop() {
                            k += 1;
                        }
                        if rax_elidable_after(store, infos, k, len) {
                            let new_text = format!("    movsd (%{}), {}", ptr_reg, xmm_str);
                            replace_line(store, &mut infos[i], i, new_text);
                            mark_nop(&mut infos[j]);
                            changed = true;
                            i = j + 1;
                            continue;
                        }
                    }
                }
            }
        }

        // Pattern F: "movq %xmm0, %rax" + "movq %rax, %<gprA>" +
        //            "movq %<gprB>, %rcx" + "movq %<gprA>, (%rcx)"
        //          → "movsd %xmm0, (%<gprB>)" + NOP the rest.
        // This folds the 4-instruction store-through-pointer chain.
        if let LineKind::Other { dest_reg: 0 } = infos[i].kind {
            let line_i = infos[i].trimmed(store.get(i));
            if line_i == "movq %xmm0, %rax" {
                // Find J: "movq %rax, %<gprA>"
                let mut j = i + 1;
                while j < len && j < i + 4 && infos[j].is_nop() {
                    j += 1;
                }
                if j < len {
                    let line_j = infos[j].trimmed(store.get(j));
                    if line_j.starts_with("movq %rax, %") && !line_j.ends_with("%xmm0") {
                        let gpr_a = &line_j[12..]; // "movq %rax, %" is 12 chars
                                                   // Find K: "movq %<gprB>, %rcx"
                        let mut k = j + 1;
                        while k < len && k < j + 4 && infos[k].is_nop() {
                            k += 1;
                        }
                        if k < len {
                            let line_k = infos[k].trimmed(store.get(k));
                            if line_k.starts_with("movq %") && line_k.ends_with(", %rcx") {
                                let gpr_b = &line_k[6..line_k.len() - 6]; // strip "movq %" and ", %rcx"
                                                                          // Find L: "movq %<gprA>, (%rcx)"
                                let mut l = k + 1;
                                while l < len && l < k + 4 && infos[l].is_nop() {
                                    l += 1;
                                }
                                if l < len {
                                    let expected_l = format!("movq %{}, (%rcx)", gpr_a);
                                    let line_l = infos[l].trimmed(store.get(l));
                                    if line_l == expected_l {
                                        // Check rax not live after J, gprA not live after L.
                                        let mut after_j = j + 1;
                                        while after_j < len && infos[after_j].is_nop() {
                                            after_j += 1;
                                        }
                                        if rax_elidable_after(store, infos, after_j, len) {
                                            let new_text = format!("    movsd %xmm0, (%{})", gpr_b);
                                            replace_line(store, &mut infos[i], i, new_text);
                                            mark_nop(&mut infos[j]);
                                            mark_nop(&mut infos[k]);
                                            mark_nop(&mut infos[l]);
                                            changed = true;
                                            i = l + 1;
                                            continue;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        i += 1;
    }
    changed
}

// ── Pointer-deref stack elimination (Pattern H) ─────────────────────────────
//
// Matches the common codegen idiom:
//   movq (%<ptr>), %rax          [I] load through pointer into GPR
//   movq %rax, -O(%rbp)          [J] spill to stack slot
//   ... (gap, no write to O(%rbp) or %<ptr>) ...
//   movsd/mulsd/addsd -O(%rbp)   [K] FP use of the spilled value
//
// Folds to:  NOP I, NOP J,  replace O(%rbp) in K with (%<ptr>).
// This eliminates the GPR round-trip and stack spill.
//
pub(super) fn fold_ptr_deref_through_stack(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    let len = store.len();
    let mut changed = false;
    let mut i = 0;

    while i < len {
        if infos[i].is_nop() {
            i += 1;
            continue;
        }

        // Match I: "movq (%<ptr>), %rax"
        if let LineKind::Other { dest_reg: 0 } = infos[i].kind {
            let line_i = infos[i].trimmed(store.get(i));
            if line_i.starts_with("movq (%") && line_i.ends_with("), %rax") {
                let ptr_reg = &line_i[7..line_i.len() - 7]; // between "movq (%" and "), %rax"

                // Match J: immediately adjacent StoreRbp{0, O, Q}
                let mut j = i + 1;
                while j < len && j < i + 4 && infos[j].is_nop() {
                    j += 1;
                }
                if j >= len {
                    i += 1;
                    continue;
                }
                if let LineKind::StoreRbp {
                    reg: 0,
                    offset,
                    size: MoveSize::Q,
                } = infos[j].kind
                {
                    let store_text = infos[j].trimmed(store.get(j));
                    let base = if store_text.contains("(%rsp)") {
                        "rsp"
                    } else {
                        "rbp"
                    };
                    let offset_str = format!("{}(%{})", offset, base);
                    let ptr_mem = format!("(%{})", ptr_reg);

                    // Scan forward from j+1 for first FP use of O(%rbp).
                    let mut k = j + 1;
                    let mut ptr_modified = false;
                    let mut rax_overwritten = false;
                    let mut count = 0;
                    let mut found = false;

                    while k < len && count < 20 {
                        if infos[k].is_nop() {
                            k += 1;
                            continue;
                        }
                        let t = infos[k].trimmed(store.get(k));

                        // Stop at control flow.
                        if t.starts_with('j') || t.starts_with("call") || t.starts_with("ret") {
                            break;
                        }

                        // Check if ptr register is modified (appears as destination).
                        let ptr_with_pct = format!("%{}", ptr_reg);
                        if t.ends_with(&format!(", {}", ptr_with_pct)) {
                            ptr_modified = true;
                        }

                        // Check if %rax is read before overwritten (safety for NOP'ing I).
                        if !rax_overwritten {
                            match infos[k].kind {
                                LineKind::LoadRbp { reg: 0, .. }
                                | LineKind::Other { dest_reg: 0 } => {
                                    rax_overwritten = true;
                                }
                                _ => {
                                    if t.contains("%rax") || t.contains("%eax") || t.contains("%al")
                                    {
                                        // %rax is read before being overwritten → unsafe.
                                        break;
                                    }
                                }
                            }
                        }

                        // Check for StoreRbp writing the same offset → our store is overwritten.
                        if let LineKind::StoreRbp { offset: o, .. } = infos[k].kind {
                            if o == offset {
                                break;
                            }
                        }

                        // Found an FP instruction using O(%rbp) as source operand?
                        if t.contains(&offset_str) && !ptr_modified {
                            // Verify it's a source (not dest). StoreRbp is already caught above.
                            // For movsd/mulsd/addsd/subsd/divsd: the memory operand is the source
                            // if it comes before the comma.
                            if (t.starts_with("movsd ")
                                || t.starts_with("mulsd ")
                                || t.starts_with("addsd ")
                                || t.starts_with("subsd ")
                                || t.starts_with("divsd "))
                                && t.contains(&offset_str)
                            {
                                // Check O(%rbp) not read again after K.
                                if rbp_offset_dead_after(
                                    store,
                                    infos,
                                    k + 1,
                                    len,
                                    i64::from(offset),
                                ) {
                                    let new_text =
                                        format!("    {}", t.replace(&offset_str, &ptr_mem));
                                    mark_nop(&mut infos[i]);
                                    mark_nop(&mut infos[j]);
                                    replace_line(store, &mut infos[k], k, new_text);
                                    changed = true;
                                    found = true;
                                }
                                break;
                            }
                            break; // Unknown instruction using the offset → bail.
                        }

                        k += 1;
                        count += 1;
                    }

                    if found {
                        i = k + 1;
                        continue;
                    }
                }
            }
        }

        i += 1;
    }
    changed
}

// ── FP spill elimination around load (Pattern I) ─────────────────────────────
//
// Matches:
//   movsd %xmm0, O(%rbp)       [I] spill product to stack
//   ... (gap: address calc, no xmm usage) ...
//   movsd (%ptr), %xmm0        [K] load C (overwrites xmm0)
//   addsd O(%rbp), %xmm0       [L] C += spilled product
//
// Rewrites to:
//   (NOP)                       [I] eliminated
//   ... (gap) ...
//   movsd (%ptr), %xmm1        [K] load C into xmm1
//   addsd %xmm1, %xmm0         [L] product + C (register-only)
//
// This avoids the stack spill+reload by keeping the product in xmm0 and
// routing the C load through xmm1 instead.
//
pub(super) fn eliminate_fp_spill_around_load(
    store: &mut LineStore,
    infos: &mut [LineInfo],
) -> bool {
    let len = store.len();
    let mut changed = false;
    let mut i = 0;

    while i < len {
        if infos[i].is_nop() {
            i += 1;
            continue;
        }
        let line_i = infos[i].trimmed(store.get(i));

        // Match I: "movsd %xmm0, <offset>(%rbp)"
        if line_i.starts_with("movsd %xmm0, ")
            && (line_i.ends_with("(%rbp)") || line_i.ends_with("(%rsp)"))
        {
            let mem_operand = &line_i[13..]; // after "movsd %xmm0, " (13 chars)
                                             // Extract numeric offset from e.g. "-48(%rbp)"
            let paren_pos = mem_operand.find('(');
            if let Some(pp) = paren_pos {
                let offset_num: Result<i64, _> = mem_operand[..pp].parse();
                if let Ok(offset) = offset_num {
                    // Scan forward for K (load into xmm0) then L (addsd from same slot).
                    let mut k_pos = 0usize;
                    let mut l_pos = 0usize;
                    let mut xmm1_clear = true;
                    let mut j = i + 1;
                    let mut count = 0;

                    while j < len && count < 16 {
                        if infos[j].is_nop() {
                            j += 1;
                            continue;
                        }
                        let t = infos[j].trimmed(store.get(j));
                        if t.starts_with('j') || t.starts_with("call") || t.starts_with("ret") {
                            break;
                        }
                        if t.contains("%xmm1") {
                            xmm1_clear = false;
                            break;
                        }

                        // Find K: "movsd <something>, %xmm0" (load overwriting xmm0)
                        if k_pos == 0 && t.starts_with("movsd ") && t.ends_with(", %xmm0") {
                            k_pos = j;
                        }

                        // Find L: "addsd <offset>(%rbp), %xmm0" (reload from spill slot)
                        if k_pos > 0 {
                            let expected_l = format!("addsd {}, %xmm0", mem_operand);
                            if t == expected_l {
                                l_pos = j;
                                break;
                            }
                        }

                        j += 1;
                        count += 1;
                    }

                    if k_pos > 0 && l_pos > 0 && xmm1_clear {
                        // Verify the spill slot is dead after L.
                        if rbp_offset_dead_after(store, infos, l_pos + 1, len, offset) {
                            // Also verify xmm1 is dead after L.
                            let mut after_l = l_pos + 1;
                            while after_l < len && infos[after_l].is_nop() {
                                after_l += 1;
                            }
                            let xmm1_dead_after = if after_l >= len {
                                true
                            } else {
                                let t = infos[after_l].trimmed(store.get(after_l));
                                !t.contains("%xmm1")
                            };

                            if xmm1_dead_after {
                                // NOP I (the spill store).
                                mark_nop(&mut infos[i]);
                                // Change K: "movsd ..., %xmm0" → "movsd ..., %xmm1"
                                let line_k = infos[k_pos].trimmed(store.get(k_pos));
                                let new_k = format!("    {}", line_k.replace(", %xmm0", ", %xmm1"));
                                replace_line(store, &mut infos[k_pos], k_pos, new_k);
                                // Change L: "addsd O(%rbp), %xmm0" → "addsd %xmm1, %xmm0"
                                let new_l = "    addsd %xmm1, %xmm0".to_string();
                                replace_line(store, &mut infos[l_pos], l_pos, new_l);
                                changed = true;
                                i = l_pos + 1;
                                continue;
                            }
                        }
                    }
                }
            }
        }

        i += 1;
    }
    changed
}

// ── Loop-invariant FP stack promotion ────────────────────────────────────────
//
// Promotes a loop-invariant `movsd -O(%rbp), %xmm0` to a register:
//   Preheader: movq %rax, -O(%rbp) → movq %rax, %xmm2
//   Loop body: movsd -O(%rbp), %xmm0 → movapd %xmm2, %xmm0
//
// Conditions:
//   - The inner loop has a back-edge jmp to a label above
//   - -O(%rbp) is not written inside the loop body
//   - xmm2 is not used inside the loop body
//   - The preheader (block before loop header) stores to -O(%rbp)
//
pub(super) fn promote_loop_invariant_fp_load(
    store: &mut LineStore,
    infos: &mut [LineInfo],
) -> bool {
    let len = store.len();
    let mut changed = false;

    // Find loop back-edges: jmp to a label that appears before the jmp.
    let mut i = 0;
    while i < len {
        if infos[i].is_nop() {
            i += 1;
            continue;
        }
        if infos[i].kind != LineKind::Jmp {
            i += 1;
            continue;
        }
        let jmp_text = infos[i].trimmed(store.get(i));
        if !jmp_text.starts_with("jmp ") {
            i += 1;
            continue;
        }
        let target = &jmp_text[4..];
        let target_label = format!("{}:", target);

        // Find the target label (must be before the jmp = back-edge).
        let mut header_pos = None;
        for lbl in 0..i {
            if infos[lbl].kind == LineKind::Label {
                if infos[lbl].trimmed(store.get(lbl)) == target_label {
                    header_pos = Some(lbl);
                    break;
                }
            }
        }
        let header = match header_pos {
            Some(h) => h,
            None => {
                i += 1;
                continue;
            }
        };

        // Loop body is [header..=i]. Find movsd -O(%rbp), %xmm0 in body.
        let mut body_start = header;
        // Skip past header's condition check to the body label.
        for pos in header + 1..i {
            if infos[pos].kind == LineKind::Label {
                body_start = pos;
                break;
            }
        }

        // Scan body for "movsd -O(%rbp), %xmm0" candidates.
        for pos in body_start + 1..i {
            if infos[pos].is_nop() {
                continue;
            }
            let t = infos[pos].trimmed(store.get(pos));
            if !t.starts_with("movsd ") || !t.ends_with(", %xmm0") {
                continue;
            }
            // Check it's a stack load: "movsd -N(%rbp), %xmm0" or "movsd N(%rsp), %xmm0"
            let src = &t[6..t.len() - 7]; // between "movsd " and ", %xmm0"
            if !src.ends_with("(%rbp)") && !src.ends_with("(%rsp)") {
                continue;
            }
            let offset_str = src.to_string();

            // Check -O(%rbp) is NOT written in the loop body [body_start..=i].
            // Extract numeric offset from the source string (e.g., "-24(%rbp)" → -24)
            let numeric_offset_end = offset_str.find('(').unwrap_or(offset_str.len());
            let numeric_offset: i32 = offset_str[..numeric_offset_end].parse().unwrap_or(0);
            let mut written_in_body = false;
            for chk in body_start + 1..i {
                if infos[chk].is_nop() {
                    continue;
                }
                if let LineKind::StoreRbp { offset: o, .. } = infos[chk].kind {
                    if o == numeric_offset {
                        written_in_body = true;
                        break;
                    }
                }
                // Also check text for movsd stores to this offset.
                let ct = infos[chk].trimmed(store.get(chk));
                if ct.ends_with(&offset_str) && ct.starts_with("movsd ") {
                    written_in_body = true;
                    break;
                }
            }
            if written_in_body {
                continue;
            }

            // Check xmm2 is not used anywhere in [header..=i].
            let mut xmm2_used = false;
            for chk in header..=i {
                if infos[chk].is_nop() {
                    continue;
                }
                let ct = infos[chk].trimmed(store.get(chk));
                if ct.contains("%xmm2") {
                    xmm2_used = true;
                    break;
                }
            }
            if xmm2_used {
                continue;
            }

            // Find preheader store: "movq %rax, -O(%rbp)" before the header.
            // Scan backward, crossing labels but stopping at function boundaries (.size).
            let mut preheader_store = None;
            let mut ph = if header > 0 { header - 1 } else { 0 };
            let mut ph_count = 0;
            while ph_count < 60 {
                if infos[ph].is_nop() {
                    if ph == 0 {
                        break;
                    }
                    ph -= 1;
                    continue;
                }
                // Stop at function boundaries, not labels
                if infos[ph].kind == LineKind::Directive {
                    let dt = infos[ph].trimmed(store.get(ph));
                    if dt.starts_with(".size ")
                        || dt.starts_with(".globl ")
                        || dt.starts_with(".type ")
                    {
                        break;
                    }
                }
                // Labels are OK to cross — we're looking for ANY store to this offset
                // that dominates the loop header.
                if let LineKind::StoreRbp {
                    reg: 0, offset: o, ..
                } = infos[ph].kind
                {
                    if o == numeric_offset {
                        preheader_store = Some(ph);
                        break;
                    }
                }
                if ph == 0 {
                    break;
                }
                ph -= 1;
                ph_count += 1;
            }

            if let Some(ph_pos) = preheader_store {
                // Replace preheader store: "movq %rax, -O(%rbp)" → "movq %rax, %xmm2"
                let new_ph = "    movq %rax, %xmm2".to_string();
                replace_line(store, &mut infos[ph_pos], ph_pos, new_ph);
                // Replace body load: "movsd -O(%rbp), %xmm0" → "movapd %xmm2, %xmm0"
                let new_body = "    movapd %xmm2, %xmm0".to_string();
                replace_line(store, &mut infos[pos], pos, new_body);
                changed = true;
                break; // Only promote one per loop for now.
            }
        }

        i += 1;
    }
    changed
}

// ── Copy + operation fusion ──────────────────────────────────────────────────
//
// Fuses a register copy with the following operation that reads and writes
// the destination, when the copy is the sole producer:
//
//   movq %A, %B  +  leaq disp(%B), %B   →  leaq disp(%A), %B    (copy+lea)
//   movq %A, %B  +  addq $imm, %B        →  leaq imm(%A), %B     (copy+add)
//   movq %A, %B  +  shlq $N, %B          →  leaq (,%A,2^N), %B   (copy+shl, N=1..3)
//
// The addq→leaq rewrite drops flags. We verify the next non-NOP instruction
// sets its own flags (doesn't consume ours) before applying.
//
pub(super) fn fuse_copy_and_operation(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    let len = store.len();
    let mut changed = false;
    let mut i = 0;

    while i < len {
        if infos[i].is_nop() {
            i += 1;
            continue;
        }

        // Match: "movq %<src>, %<dst>" where src != dst, both are GP regs.
        if let LineKind::Other { dest_reg } = infos[i].kind {
            if dest_reg > 15 {
                i += 1;
                continue;
            }
            let line_i = infos[i].trimmed(store.get(i));
            if !line_i.starts_with("movq %") {
                i += 1;
                continue;
            }

            // Parse "movq %<src>, %<dst>"
            if let Some(comma) = line_i.find(", %") {
                let src_reg = &line_i[6..comma]; // after "movq %"
                let dst_reg_str = &line_i[comma + 3..]; // after ", %"
                                                        // A segment override source (e.g. `movq %fs:0, %rax` from TLS
                                                        // initial-exec) must NOT be folded into a following lea:
                                                        // `lea disp(%fs:0), %rax` would compute disp WITHOUT adding the
                                                        // segment base (LEA ignores segment overrides by design), so
                                                        // the TLS address would be wrong. `movq %fs:0, %rax` + `leaq
                                                        // tpoff(%rax), %rax` must stay two instructions.
                if src_reg == dst_reg_str || src_reg.contains('(') || src_reg.contains(':') {
                    i += 1;
                    continue;
                }

                let mut j = i + 1;
                while j < len && j < i + 4 && infos[j].is_nop() {
                    j += 1;
                }
                if j >= len {
                    i += 1;
                    continue;
                }
                let line_j = infos[j].trimmed(store.get(j));

                // Sub-pattern: leaq disp(%<dst>), %<dst> → leaq disp(%<src>), %<dst>
                let lea_prefix = format!("leaq ");
                let lea_base = format!("(%{}), %{}", dst_reg_str, dst_reg_str);
                if line_j.starts_with(&lea_prefix)
                    && line_j.ends_with(&format!("), %{}", dst_reg_str))
                    && line_j.contains(&format!("(%{})", dst_reg_str))
                {
                    let new_text = format!(
                        "    {}",
                        line_j.replace(&format!("(%{})", dst_reg_str), &format!("(%{})", src_reg),)
                    );
                    mark_nop(&mut infos[i]);
                    replace_line(store, &mut infos[j], j, new_text);
                    changed = true;
                    i = j + 1;
                    continue;
                }

                // Sub-pattern: addq $imm, %<dst> → leaq imm(%<src>), %<dst>
                // Only if the add's flags are dead.  The one-instruction
                // peek this used to be let an intervening flag-NEUTRAL line
                // (`mov`, `andn`, `lea`, ...) hide a later `jcc` that still
                // reads the add's flags — the central `flags_dead_after`
                // scan walks to the next real reader or writer.
                let add_suffix = format!(", %{}", dst_reg_str);
                if line_j.starts_with("addq $") && line_j.ends_with(&add_suffix) {
                    let imm_str = &line_j[6..line_j.len() - add_suffix.len()]; // between "addq $" and ", %dst"
                    if flags_dead_after(store, infos, j + 1) {
                        let new_text =
                            format!("    leaq {}(%{}), %{}", imm_str, src_reg, dst_reg_str);
                        mark_nop(&mut infos[i]);
                        replace_line(store, &mut infos[j], j, new_text);
                        changed = true;
                        i = j + 1;
                        continue;
                    }
                }
            }
        }

        i += 1;
    }
    changed
}

/// Returns true if %rax is live (potentially read) at instruction index `at`.
/// Conservative: treats "movq %<non-rax>, %rax" as a pure write (not live).
/// returns true iff eliding the value currently in %rax is SOUND from
/// instruction `at` onward: nothing reads %rax before it is written, and no
/// control-flow boundary (jump/call/ret) is crossed before a write. Reads of
/// the 32-bit sub-register (%eax) also count (a later `movl %eax, ...` would
/// observe the low 32 bits the codegen may still rely on via the accumulator
/// register cache). The old check looked only ONE instruction ahead, so an
/// intervening instruction that does not touch %rax (e.g. `movabsq $imm, %rcx`)
/// hid a LATER %rax read (e.g. `cmpq %rcx, %rax`) — the fuse then elided a
/// %rax the accumulator cache still promised (v5 miscompile class, exposed by
/// _mm_cvtsi128_si64 chains).
fn rax_elidable_after(store: &LineStore, infos: &[LineInfo], mut at: usize, len: usize) -> bool {
    while at < len {
        if infos[at].is_nop() {
            at += 1;
            continue;
        }
        let t = infos[at].trimmed(store.get(at));
        // Control-flow / calls: conservative — a branch may skip any write, so
        // the value could be observed on another path. Never fuse across them.
        if t.contains("call") || t.starts_with("j") || t.starts_with("ret") || t.contains("jmp") {
            return false;
        }
        // A definitive write of %rax/%eax makes earlier values dead.
        let is_write = (t.ends_with(", %rax") || t.ends_with(", %eax"))
            && !t.starts_with("cmp")
            && !t.starts_with("test")
            && !t.starts_with("and")
            && !t.starts_with("or")
            && !t.starts_with("xor")
            && !t.starts_with("sub")
            && !t.starts_with("add")
            && !t.starts_with("imul")
            && !t.starts_with("bt")
            && !t.starts_with("shl")
            && !t.starts_with("shr")
            && !t.starts_with("clc")
            && !t.starts_with("stc")
            && !t.starts_with("cmov");
        // `xorl %eax, %eax` / `xorq %rax, %rax` are writes too.
        let xor_self = t.starts_with("xor") && (t.contains("%eax") || t.contains("%rax"));
        if is_write || xor_self {
            return true;
        }
        // Any other mention of %rax/%eax is a READ.
        if t.contains("%rax") || t.contains("%eax") {
            return false;
        }
        at += 1;
    }
    true // reached block end with no read before a write
}

/// Returns true if the rbp offset `offset` is dead (not read before being
/// written or before a control-flow boundary) starting at instruction `start`.
/// Scans up to 32 instructions forward; stops at any jump/call.
fn rbp_offset_dead_after(
    store: &LineStore,
    infos: &[LineInfo],
    start: usize,
    len: usize,
    offset: i64,
) -> bool {
    // The slot must not be read anywhere AFTER this store in the WHOLE
    // function — not just in the current block. A block boundary (jmp/jcc/
    // ret) does NOT prove the slot dead: a later block may load the same
    // slot (e.g. a register-allocated F64 whose home slot is only spilled
    // once and read in two different blocks). Killing the store then leaves
    // the later block reading stale data (miscompile). Scanning the whole
    // remaining text is conservative and sound: a later StoreRbp to the same
    // offset re-validates the slot, any other mention is treated as a read.
    let offset_str_rbp = format!("{}(%rbp)", offset);
    let offset_str_rsp = format!("{}(%rsp)", offset);
    let mut i = start;
    let mut count = 0;
    while i < len && count < 2048 {
        if infos[i].is_nop() {
            i += 1;
            continue;
        }
        let t = infos[i].trimmed(store.get(i));
        // A later write to the same offset (StoreRbp) re-validates the slot.
        if let LineKind::StoreRbp { offset: o, .. } = infos[i].kind {
            if i64::from(o) == offset {
                return true;
            }
        }
        // Any other mention of the offset (a load, an address operand) means
        // the slot must stay valid — do NOT eliminate the store.
        if t.contains(&offset_str_rbp) || t.contains(&offset_str_rsp) {
            return false;
        }
        i += 1;
        count += 1;
    }
    // No read found anywhere in the remaining function text → dead.
    true
}

// ── rcx address-register copy elimination (Pattern G) ────────────────────────
//
// LCCC's codegen always copies the pointer into %rcx before a memory op:
//   movq %<ptr>, %rcx
//   movq (%rcx), %rax   OR   movsd (%rcx), %xmmN
//
// When %rcx is dead after the dereference we can fold to:
//   movq (%<ptr>), %rax   OR   movsd (%<ptr>), %xmmN
//
pub(super) fn eliminate_rcx_address_copy(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    let len = store.len();
    let mut changed = false;
    let mut i = 0;

    while i < len {
        if infos[i].is_nop() {
            i += 1;
            continue;
        }

        // Match: "movq %<gpr>, %rcx" (any GPR → rcx, rcx = family 1)
        if let LineKind::Other { dest_reg: 1 } = infos[i].kind {
            let line_i = infos[i].trimmed(store.get(i));
            if line_i.starts_with("movq %") && line_i.ends_with(", %rcx") {
                let src_reg = &line_i[6..line_i.len() - 6]; // between "movq %" and ", %rcx"
                                                            // src_reg must not be "rcx" itself (no-op move) and must be a plain GPR
                if src_reg != "rcx" && !src_reg.contains('(') && !src_reg.contains('$') {
                    let mut j = i + 1;
                    while j < len && j < i + 4 && infos[j].is_nop() {
                        j += 1;
                    }
                    if j < len {
                        let line_j = infos[j].trimmed(store.get(j));

                        // Sub-pattern G1: movq (%rcx), %rax → movq (%<src>), %rax
                        if line_j == "movq (%rcx), %rax" {
                            let mut k = j + 1;
                            while k < len && infos[k].is_nop() {
                                k += 1;
                            }
                            if !rcx_is_live_at(store, infos, k, len) {
                                let new = format!("    movq (%{}), %rax", src_reg);
                                mark_nop(&mut infos[i]);
                                replace_line(store, &mut infos[j], j, new);
                                changed = true;
                                i = j + 1;
                                continue;
                            }
                        }

                        // Sub-pattern G2: movsd (%rcx), %xmmN → movsd (%<src>), %xmmN
                        if line_j.starts_with("movsd (%rcx), %xmm") {
                            let xmm_dest = &line_j[14..]; // after "movsd (%rcx), " → "%xmmN"
                            let mut k = j + 1;
                            while k < len && infos[k].is_nop() {
                                k += 1;
                            }
                            if !rcx_is_live_at(store, infos, k, len) {
                                let new = format!("    movsd (%{}), {}", src_reg, xmm_dest);
                                mark_nop(&mut infos[i]);
                                replace_line(store, &mut infos[j], j, new);
                                changed = true;
                                i = j + 1;
                                continue;
                            }
                        }

                        // Sub-pattern G3: a pointer copy feeding a store:
                        //   movq %<ptr>, %rcx
                        //   movq %<value>, (%rcx)
                        // becomes:
                        //   movq %<value>, (%<ptr>)
                        //
                        // This is the common scalar-store form emitted by
                        // accumulator codegen. The address copy is removable
                        // only when the store's destination is exactly
                        // (%rcx), the value does not read %rcx, and no later
                        // instruction reads %rcx. A displacement operand is
                        // deliberately excluded: replacing only the base
                        // register there would need a different parser and
                        // could change the effective address.
                        if line_j.starts_with("mov") {
                            let Some((source, destination)) = line_j.rsplit_once(',') else {
                                i += 1;
                                continue;
                            };
                            if destination.trim() == "(%rcx)" && !source.contains("%rcx") {
                                let mut k = j + 1;
                                while k < len && infos[k].is_nop() {
                                    k += 1;
                                }
                                if !rcx_is_live_at(store, infos, k, len) {
                                    let replacement =
                                        line_j.replacen("(%rcx)", &format!("(%{})", src_reg), 1);
                                    if replacement != line_j && !replacement.contains("%rcx") {
                                        mark_nop(&mut infos[i]);
                                        replace_line(
                                            store,
                                            &mut infos[j],
                                            j,
                                            format!("    {}", replacement),
                                        );
                                        changed = true;
                                        i = j + 1;
                                        continue;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        i += 1;
    }
    changed
}

fn rcx_is_live_at(store: &LineStore, infos: &[LineInfo], at: usize, len: usize) -> bool {
    if at >= len {
        return false;
    }
    match infos[at].kind {
        // LoadRbp loads into a GP reg — doesn't use %rcx as address
        LineKind::LoadRbp { .. } => false,
        LineKind::Other { dest_reg: 1 } => {
            // rcx is the destination. "movq <src>, %rcx" is a pure write if src ≠ %rcx.
            let t = infos[at].trimmed(store.get(at));
            if t.starts_with("movq ") && t.ends_with(", %rcx") {
                let src = &t[5..t.len() - 6];
                src.contains("%rcx")
            } else {
                t.contains("%rcx")
            }
        }
        _ => infos[at].trimmed(store.get(at)).contains("%rcx"),
    }
}

// ── Movq + extension/truncation fusion ───────────────────────────────────────
//
// Fuses `movq %REG, %rax` followed by a cast instruction into a single
// instruction. The two-instruction pattern arises from the accumulator-based
// codegen model: emit_load_operand loads a 64-bit value into %rax, then
// emit_cast_instrs emits an extension/truncation on %rax/%eax/%ax/%al.
//
// Fused patterns (all require REG != rax, no intervening non-NOP instructions):
//   movq %REG, %rax + movl %eax, %eax   -> movl %REGd, %eax    (truncate to u32)
//   movq %REG, %rax + movslq %eax, %rax -> movslq %REGd, %rax  (sign-extend i32->i64)
//   movq %REG, %rax + cltq              -> movslq %REGd, %rax   (sign-extend i32->i64)
//   movq %REG, %rax + movzbq %al, %rax  -> movzbl %REGb, %eax  (zero-extend u8->i64)
//   movq %REG, %rax + movzwq %ax, %rax  -> movzwl %REGw, %eax  (zero-extend u16->i64)
//   movq %REG, %rax + movsbq %al, %rax  -> movsbq %REGb, %rax  (sign-extend i8->i64)

pub(super) fn fuse_movq_ext_truncation(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    let mut changed = false;
    let len = store.len();

    let mut i = 0;
    while i + 1 < len {
        // Look for ProducerMovqRegToRax
        if infos[i].ext_kind != ExtKind::ProducerMovqRegToRax {
            i += 1;
            continue;
        }

        // Find next non-NOP instruction (skip only NOPs, not stores)
        let mut j = i + 1;
        while j < len && infos[j].is_nop() {
            j += 1;
        }
        if j >= len {
            i += 1;
            continue;
        }

        // Check if next instruction is a fusable extension/truncation on %rax
        let next_ext = infos[j].ext_kind;
        let fusable = matches!(
            next_ext,
            ExtKind::MovlEaxEax
                | ExtKind::MovslqEaxRax
                | ExtKind::Cltq
                | ExtKind::MovzbqAlRax
                | ExtKind::MovzwqAxRax
                | ExtKind::MovsbqAlRax
        );
        if !fusable {
            i += 1;
            continue;
        }

        // Extract source register family from the movq instruction
        let movq_line = infos[i].trimmed(store.get(i));
        let src_family = if let Some(rest) = movq_line.strip_prefix("movq ") {
            if let Some((src, _dst)) = rest.split_once(',') {
                let src = src.trim();
                let fam = register_family_fast(src);
                // This fusion indexes the 16-entry GPR name table. XMM/MMX
                // register families are recognized by the parser but have no
                // 8/16/32-bit GPR aliases and must take the unfused path.
                if fam != REG_NONE && fam != 0 && fam <= REG_GP_MAX {
                    fam
                } else {
                    REG_NONE
                }
            } else {
                REG_NONE
            }
        } else {
            REG_NONE
        };

        if src_family == REG_NONE {
            i += 1;
            continue;
        }

        // Build the fused instruction based on the extension type
        let new_text = match next_ext {
            ExtKind::MovlEaxEax => {
                let src_32 = REG_NAMES[1][src_family as usize];
                format!("    movl {}, %eax", src_32)
            }
            ExtKind::MovslqEaxRax | ExtKind::Cltq => {
                let src_32 = REG_NAMES[1][src_family as usize];
                format!("    movslq {}, %rax", src_32)
            }
            ExtKind::MovzbqAlRax => {
                let src_8 = REG_NAMES[3][src_family as usize];
                format!("    movzbl {}, %eax", src_8)
            }
            ExtKind::MovzwqAxRax => {
                let src_16 = REG_NAMES[2][src_family as usize];
                format!("    movzwl {}, %eax", src_16)
            }
            ExtKind::MovsbqAlRax => {
                let src_8 = REG_NAMES[3][src_family as usize];
                format!("    movsbq {}, %rax", src_8)
            }
            _ => unreachable!("mov+ext fusion matched unexpected ExtKind"),
        };

        replace_line(store, &mut infos[i], i, new_text);
        mark_nop(&mut infos[j]);
        changed = true;
        i = j + 1;
        continue;
    }
    changed
}

// ── Sign-extend + move fusion ────────────────────────────────────────────────
//
// Fuses a sign-extend to %rax followed by a move from %rax to another register,
// when %rax is not needed afterward (or only used for a 32-bit compare that can
// be redirected to the original source register).
//
// Pattern A (rax dead):
//   movslq %Xd, %rax       →  movslq %Xd, %Y
//   movq %rax, %Y
//
// Pattern B (rax used only in cmpl):
//   movslq %Xd, %rax       →  movslq %Xd, %Y
//   movq %rax, %Y              cmpl $imm, %Xd
//   cmpl $imm, %eax
//
// Also handles cltq (= movslq %eax, %rax) as the sign-extend source.

pub(super) fn fuse_signext_and_move(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    let len = store.len();
    let mut changed = false;
    let mut i = 0;

    while i < len {
        if infos[i].is_nop() {
            i += 1;
            continue;
        }

        // Match: movslq %Xd, %rax (or cltq)
        let ti = infos[i].trimmed(store.get(i));
        let src_family = if ti.starts_with("movslq %") && ti.ends_with(", %rax") {
            let src_32 = &ti[7..ti.len() - 6]; // between "movslq " and ", %rax"
            if !src_32.starts_with('%') {
                i += 1;
                continue;
            }
            let fam = register_family_fast(src_32);
            if fam == REG_NONE || fam == 0 {
                i += 1;
                continue;
            }
            fam
        } else if ti == "cltq" {
            0u8 // cltq sign-extends %eax → %rax, src is rax family itself
        } else {
            i += 1;
            continue;
        };

        // For cltq, we can't retarget (src == dest == rax family), skip
        if src_family == 0 {
            i += 1;
            continue;
        }

        // Next non-NOP: must be movq %rax, %Y
        let j = next_non_nop(infos, i + 1, len);
        if j >= len || infos[j].is_barrier() {
            i += 1;
            continue;
        }

        let tj = infos[j].trimmed(store.get(j));
        if !tj.starts_with("movq %rax, %") {
            i += 1;
            continue;
        }
        let dest_reg = &tj[11..]; // after "movq %rax, " (includes %)
        let dest_family = register_family_fast(dest_reg);
        if dest_family == REG_NONE || dest_family == 0 || dest_family == src_family {
            i += 1;
            continue;
        }

        // Check if %rax is dead after j, or only used in a redirectable cmpl.
        // Scan forward from j+1 until the next basic block boundary (barrier).
        // No line limit — large functions like SQLite's 10K-line VDBE interpreter
        // can have long stretches of non-barrier instructions.
        let src_32_name = REG_NAMES[1][src_family as usize];
        let mut rax_dead = false; // conservative default: assume alive
        let mut cmpl_lines: Vec<usize> = Vec::new();
        let mut n = j + 1;
        while n < len {
            if infos[n].is_nop() {
                n += 1;
                continue;
            }
            if infos[n].is_barrier() {
                // At ANY barrier, conservatively assume rax is alive.
                // Even jmp/call could have successors that read rax
                // (e.g., fall-through after conditional, or rax used
                // by a different predecessor to the jmp target).
                // Only explicit overwrite (LoadRbp/Other with dest=0)
                // proves rax is dead.
                break;
            }
            // Check if this instruction references rax
            if infos[n].reg_refs & 1 != 0 {
                // rax is referenced. Check if it's a PURE write (rax overwritten → dead).
                // Instructions that both read and write rax (like movl %eax, %eax
                // for zero-extension) are NOT pure writes — they depend on the
                // current rax value.
                match infos[n].kind {
                    LineKind::LoadRbp { reg: 0, .. } => {
                        rax_dead = true;
                        break;
                    }
                    LineKind::Other { dest_reg: 0 } => {
                        // Verify it's a pure write, not a read-modify-write.
                        let tn = infos[n].trimmed(store.get(n));
                        if !is_read_modify_write(tn) {
                            // Also check: source operand must not reference any
                            // rax-family register (eax, ax, al, rax).
                            let is_rax_in_src = if let Some(comma) = tn.rfind(',') {
                                let src = &tn[..comma];
                                src.contains("%rax")
                                    || src.contains("%eax")
                                    || src.contains("%ax")
                                    || src.contains("%al")
                            } else {
                                true // single-operand → assumes reads rax
                            };
                            if !is_rax_in_src {
                                rax_dead = true;
                                break;
                            }
                        }
                        // Falls through: rax is read, not dead
                    }
                    _ => {}
                }
                // Check if it's a cmpl $imm, %eax
                let tn = infos[n].trimmed(store.get(n));
                if tn.starts_with("cmpl $") && tn.ends_with(", %eax") {
                    // Check src_family is not modified between i and n.
                    // Must check ALL instruction kinds that can write to a register,
                    // including calls (clobber caller-saved) and pop.
                    let src_bit = 1u16 << src_family;
                    let mut src_modified = false;
                    for chk in (i + 1)..n {
                        if infos[chk].is_nop() {
                            continue;
                        }
                        // Calls clobber all caller-saved registers. If src is
                        // caller-saved (rdi=6, rsi=7, r8-r11=8-11), it's modified.
                        if infos[chk].kind == LineKind::Call {
                            if src_family >= 6 {
                                // caller-saved families
                                src_modified = true;
                                break;
                            }
                        }
                        // Any barrier could modify the register
                        if infos[chk].is_barrier() && infos[chk].kind == LineKind::Call {
                            // already handled above
                        }
                        if infos[chk].reg_refs & src_bit == 0 {
                            continue;
                        }
                        match infos[chk].kind {
                            LineKind::Other { dest_reg: d } if d == src_family => {
                                src_modified = true;
                                break;
                            }
                            LineKind::LoadRbp { reg: r, .. } if r == src_family => {
                                src_modified = true;
                                break;
                            }
                            LineKind::Pop { reg: r } if r == src_family => {
                                src_modified = true;
                                break;
                            }
                            LineKind::StoreRbp { reg: r, .. } if r == src_family => {} // store reads, doesn't modify
                            _ => {} // read-only reference is fine
                        }
                    }
                    if !src_modified {
                        cmpl_lines.push(n);
                        n += 1;
                        continue;
                    }
                }
                // rax is used in a non-redirectable way → not dead
                rax_dead = false;
                break;
            }
            n += 1;
        }

        // The transform retargets the movslq away from %rax, DELETING the only
        // definition of %rax. That is legal only when %rax is provably dead
        // afterwards. Redirecting the cmpl operands we happened to find is not
        // sufficient on its own: the forward scan stops at the first barrier,
        // so a `cmpl $imm, %eax` in a SUCCESSOR block is never examined and
        // would be left reading a register nothing defines any more.
        //
        // (Reproducer: a switch lowered to a compare chain. The first
        // `cmpl $1, %eax` was redirected, the `je` ended the scan with %rax
        // still conservatively live, and the next block's `cmpl $2, %eax` read
        // a dead register -- `switch(x){case 1: ... case 2: ...}` returned the
        // default value for x == 2. This previously stayed hidden because the
        // parameter spill reloaded %rax; eliminating that dead spill exposed
        // it.)
        if !rax_dead {
            i += 1;
            continue;
        }

        // Apply the transformation
        let dest_reg_stripped = &dest_reg[1..]; // without leading %
        let new_signext = format!("    movslq {}, %{}", src_32_name, dest_reg_stripped);
        replace_line(store, &mut infos[i], i, new_signext);
        mark_nop(&mut infos[j]); // remove movq %rax, %Y

        // Redirect EVERY cmpl that read %eax, not just the first one: they all
        // lose their operand's definition when the movslq is retargeted.
        for cmp_idx in cmpl_lines {
            let tc = infos[cmp_idx].trimmed(store.get(cmp_idx));
            let new_cmp = format!(
                "    {}",
                tc.replace(", %eax", &format!(", {}", src_32_name))
            );
            replace_line(store, &mut infos[cmp_idx], cmp_idx, new_cmp);
        }

        changed = true;
        i = j + 1;
    }
    changed
}

// ── Phi-copy register coalescing ─────────────────────────────────────────────
//
// Eliminates temporary register copies used for SSA phi resolution:
//
//   movq %SRC, %TMP        →  <ops directly on SRC>
//   ... ops on %TMP ...
//   movq %TMP, %SRC
//
// Conditions:
//   - SRC is not read or written between the copy-out and copy-back
//   - TMP is dead after the copy-back (next barrier or TMP overwritten)
//   - No implicit register hazards (div, mul, etc.)
//   - Window limited to 6 instructions to keep analysis local

pub(super) fn coalesce_phi_register_copies(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    let len = store.len();
    let mut changed = false;
    let mut i = 0;

    while i < len {
        if infos[i].is_nop() {
            i += 1;
            continue;
        }

        // Match: movq %SRC, %TMP where both are GP regs, SRC != TMP
        let ti = infos[i].trimmed(store.get(i));
        if !ti.starts_with("movq %") {
            i += 1;
            continue;
        }
        if let Some(comma) = ti.find(", %") {
            let src_reg = &ti[5..comma]; // includes leading %
            let tmp_reg = &ti[comma + 2..]; // includes leading %
            if src_reg == tmp_reg || src_reg.contains('(') {
                i += 1;
                continue;
            }

            let src_family = register_family_fast(src_reg);
            let tmp_family = register_family_fast(tmp_reg);
            if src_family == REG_NONE || src_family > REG_GP_MAX {
                i += 1;
                continue;
            }
            if tmp_family == REG_NONE || tmp_family > REG_GP_MAX {
                i += 1;
                continue;
            }
            // Don't coalesce rax(0) or rcx(1) as SRC — they're accumulator regs
            // with heavy implicit use
            if src_family <= 1 || tmp_family <= 1 {
                i += 1;
                continue;
            }
            // Don't coalesce when SRC is a caller-saved register and TMP is callee-saved.
            // Caller-saved registers (rax=0, rcx=1, rdx=2, rsi=6, rdi=7, r8-r11=8-11)
            // get clobbered by function calls. Coalescing away the copy to a callee-saved
            // register loses the value across calls. This is critical for parameter
            // pre-stores (movq %rdi, %r12) that save params before calls.
            let src_is_caller_saved = matches!(src_family, 0 | 1 | 2 | 6 | 7 | 8 | 9 | 10 | 11);
            let tmp_is_callee_saved = matches!(tmp_family, 3 | 5 | 12 | 13 | 14 | 15);
            if src_is_caller_saved && tmp_is_callee_saved {
                i += 1;
                continue;
            }

            let src_bit = 1u16 << src_family;
            let tmp_bit = 1u16 << tmp_family;

            // Scan forward for the copy-back: movq %TMP, %SRC
            // Also track chains: if TMP is sign-extended to TMP2 (movslq %TMPd, %TMP2),
            // accept movq %TMP2, %SRC as the copy-back too.
            // Within a window of 8 non-NOP instructions.
            let copy_back_pat = format!("movq {}, {}", tmp_reg, src_reg);
            let mut j = i + 1;
            let mut instr_count = 0;
            let mut src_referenced = false;
            let mut has_implicit_hazard = false;
            let mut copy_back_pos = None;
            // SOUNDNESS: TMP must NOT be WRITTEN between the copy-out and the
            // copy-back (or, for a chain, before the defining movslq). If TMP is
            // modified (e.g. xorq/addq into it), then the copy-back moves the
            // MODIFIED value to SRC, and coalescing away the copy-back would lose
            // that modification. This was a miscompile: `movq %rbp,%r12;
            // xorq %r15,%r12; movslq %r12d,%r15; movq %r15,%rbp` was coalesced
            // into `xorq %rbp,%rbp` (dropping the xor).
            let mut tmp_written = false;
            // Chain tracking: if TMP gets sign-extended to a different register
            let mut chain_family: RegId = REG_NONE;
            let mut chain_bit: u16 = 0;
            let mut chain_pos: Option<usize> = None;

            while j < len && instr_count < 8 {
                if infos[j].is_nop() {
                    j += 1;
                    continue;
                }
                if infos[j].is_barrier() {
                    break;
                }

                let tj = infos[j].trimmed(store.get(j));

                // SOUNDNESS: if TMP is WRITTEN between the copy-out and the
                // copy-back, coalescing is unsound (the copy-back would carry a
                // modified value). Detect writes to TMP.
                match infos[j].kind {
                    LineKind::Other { dest_reg } if dest_reg == tmp_family => {
                        tmp_written = true;
                    }
                    LineKind::LoadRbp { reg, .. } if reg == tmp_family => {
                        tmp_written = true;
                    }
                    LineKind::Pop { reg } if reg == tmp_family => {
                        tmp_written = true;
                    }
                    _ => {}
                }
                if tmp_written {
                    break;
                }

                // Check for direct copy-back: movq %TMP, %SRC
                if tj == copy_back_pat {
                    copy_back_pos = Some(j);
                    break;
                }

                // Check for chain copy-back: movq %TMP2, %SRC (where TMP2 came from movslq %TMPd, %TMP2)
                if chain_family != REG_NONE {
                    let chain_wb =
                        format!("movq {}, {}", REG_NAMES[0][chain_family as usize], src_reg);
                    if tj == chain_wb {
                        copy_back_pos = Some(j);
                        break;
                    }
                }

                // Check SRC is not referenced between copy-out and copy-back
                if infos[j].reg_refs & src_bit != 0 {
                    src_referenced = true;
                    break;
                }

                // Track sign-extend chain: movslq %TMPd, %OTHER
                if chain_family == REG_NONE && tj.starts_with("movslq ") {
                    let tmp_32 = REG_NAMES[1][tmp_family as usize];
                    let chain_prefix = format!("movslq {}, %", tmp_32);
                    if tj.starts_with(&chain_prefix) {
                        let other_reg = &tj[chain_prefix.len() - 1..];
                        let other_fam = register_family_fast(other_reg);
                        if other_fam != REG_NONE
                            && other_fam != tmp_family
                            && other_fam != src_family
                            && other_fam > 1
                        {
                            // SOUNDNESS: the chain register (%OTHER) is DEFINED
                            // by this movslq. It may not be READ before this point:
                            // before the movslq, %OTHER holds an unrelated live value,
                            // and rewriting it to %SRC would corrupt that use. Record
                            // the defining position and verify no earlier read exists.
                            chain_family = other_fam;
                            chain_bit = 1u16 << chain_family;
                            chain_pos = Some(j);
                        }
                    }
                }

                // Check for implicit register hazards
                if has_implicit_reg_usage(tj) {
                    has_implicit_hazard = true;
                    break;
                }

                instr_count += 1;
                j += 1;
            }

            if src_referenced || has_implicit_hazard || tmp_written || copy_back_pos.is_none() {
                i += 1;
                continue;
            }

            // SOUNDNESS: if a chain register (%OTHER from movslq %TMP,%OTHER)
            // is READ before its defining movslq, it holds an unrelated live value
            // there and must NOT be rewritten to %SRC. Abort the coalesce.
            if let Some(cp) = chain_pos {
                let mut bad = false;
                for chk in (i + 1)..cp {
                    if infos[chk].is_nop() {
                        continue;
                    }
                    if infos[chk].reg_refs & chain_bit != 0 {
                        bad = true;
                        break;
                    }
                }
                if bad {
                    i += 1;
                    continue;
                }
            }

            let cb = copy_back_pos.unwrap();

            // Verify TMP (and chain TMP2 if present) are dead after the copy-back.
            let check_bits = tmp_bit | chain_bit;
            let mut tmp_dead = false;
            let mut n = cb + 1;
            let mut chk_count = 0;
            while n < len && chk_count < 8 {
                if infos[n].is_nop() {
                    n += 1;
                    continue;
                }
                if infos[n].is_barrier() {
                    // A branch/label/call is a control-flow barrier: the temp may
                    // be LIVE on another edge (e.g. a loop-carried value via a
                    // back-edge). We cannot prove it is dead here, so conservatively
                    // abort the coalesce (leave tmp_dead=false).
                    break;
                }
                if infos[n].reg_refs & check_bits != 0 {
                    // Check each referenced temp
                    let mut is_write = false;
                    match infos[n].kind {
                        LineKind::Other { dest_reg }
                            if dest_reg == tmp_family || dest_reg == chain_family =>
                        {
                            is_write = true;
                        }
                        LineKind::LoadRbp { reg, .. }
                            if reg == tmp_family || reg == chain_family =>
                        {
                            is_write = true;
                        }
                        _ => {}
                    }
                    if !is_write {
                        break; // TMP or chain reg read after copy-back → can't coalesce
                    }
                }
                chk_count += 1;
                n += 1;
            }
            if chk_count >= 8 {
                tmp_dead = true;
            }

            if !tmp_dead {
                i += 1;
                continue;
            }

            // The copy-back line may be PINNED (e.g. a "param pre-store" heuristic
            // pins `movq %arg_reg, %callee_saved` copies seen before the first call).
            // If the copy-back is pinned, mark_nop won't remove it, leaving a dangling
            // read of a dead temp after the copy-out is removed and the intermediate
            // ops are rewritten to SRC. That is a miscompile — abort the coalesce.
            if infos[cb].pinned {
                i += 1;
                continue;
            }

            // Apply: remove copy-out, replace TMP with SRC in intermediate ops, remove copy-back
            // For chain coalescing, also replace TMP2 with SRC
            mark_nop(&mut infos[i]);
            for mid in (i + 1)..cb {
                if infos[mid].is_nop() {
                    continue;
                }
                let needs_replace = infos[mid].reg_refs & (tmp_bit | chain_bit) != 0;
                if needs_replace {
                    let mut new_text = store.get(mid).to_string();
                    if infos[mid].reg_refs & tmp_bit != 0 {
                        new_text = replace_reg_family(&new_text, tmp_family, src_family);
                    }
                    if chain_family != REG_NONE && infos[mid].reg_refs & chain_bit != 0 {
                        new_text = replace_reg_family(&new_text, chain_family, src_family);
                    }
                    if new_text != store.get(mid) {
                        replace_line(store, &mut infos[mid], mid, new_text);
                    }
                }
            }
            mark_nop(&mut infos[cb]);

            changed = true;
            i = cb + 1;
            continue;
        }
        i += 1;
    }
    changed
}

// ── Loop-invariant GPR load hoisting ─────────────────────────────────────────
//
// Hoists stack loads that are invariant across a loop body to just before the
// loop header. Generalizes promote_loop_invariant_fp_load to GPR loads.
//
// Pattern:
//   .LBB_header:
//     ...
//   .LBB_body:
//     movq OFFSET(%rsp), %REG   ← invariant load (OFFSET not written in loop)
//     ...use %REG...
//     jmp .LBB_header            ← back-edge
//
// Transformed to:
//   movq OFFSET(%rsp), %REG     ← hoisted before header
//   .LBB_header:
//     ...
//   .LBB_body:
//     ...use %REG...             ← load removed
//     jmp .LBB_header

pub(super) fn hoist_loop_invariant_gpr_load(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    let len = store.len();
    let mut changed = false;

    let mut i = 0;
    while i < len {
        if infos[i].is_nop() {
            i += 1;
            continue;
        }
        // Match both unconditional (jmp) and conditional (jle, jl, jne, etc.) back-edges
        let jmp_text = infos[i].trimmed(store.get(i)).to_string();
        let target = if jmp_text.starts_with("jmp ") {
            jmp_text[4..].to_string()
        } else if jmp_text.starts_with("jl ") {
            jmp_text[3..].to_string()
        } else if jmp_text.starts_with("jle ") {
            jmp_text[4..].to_string()
        } else if jmp_text.starts_with("jne ") {
            jmp_text[4..].to_string()
        } else if jmp_text.starts_with("jge ") {
            jmp_text[4..].to_string()
        } else if jmp_text.starts_with("jg ") {
            jmp_text[3..].to_string()
        } else if jmp_text.starts_with("jb ") {
            jmp_text[3..].to_string()
        } else if jmp_text.starts_with("ja ") {
            jmp_text[3..].to_string()
        } else {
            i += 1;
            continue;
        };
        if !target.starts_with(".L") {
            i += 1;
            continue;
        }
        let target_label = format!("{}:", target);

        // Find the target label (must be before the branch = back-edge)
        let mut header_pos = None;
        for lbl in 0..i {
            if infos[lbl].kind == LineKind::Label {
                if infos[lbl].trimmed(store.get(lbl)) == target_label {
                    header_pos = Some(lbl);
                    break;
                }
            }
        }
        let header = match header_pos {
            Some(h) => h,
            None => {
                i += 1;
                continue;
            }
        };

        // Validate this is a real loop: the range [header..=i] must not contain
        // a ret instruction (which would indicate the range spans the epilogue
        // and is not a natural loop body).
        let mut has_ret = false;
        let mut has_call = false;
        for chk in header..=i {
            if infos[chk].is_nop() {
                continue;
            }
            match infos[chk].kind {
                LineKind::Ret => {
                    has_ret = true;
                    break;
                }
                LineKind::Call => {
                    has_call = true;
                }
                _ => {}
            }
        }
        if has_ret {
            i += 1;
            continue;
        }

        // Loop body is [header..=i]. Find the body start (first label after header).
        let mut body_start = header;
        for pos in header + 1..i {
            if infos[pos].kind == LineKind::Label {
                body_start = pos;
                break;
            }
        }

        // Scan body for movq OFFSET(%rsp), %REG (or %rbp) candidates.
        // Only hoist one load per loop per pass (to avoid interactions).
        // Skip loops with function calls — caller-saved regs could be clobbered.
        if has_call {
            i += 1;
            continue;
        }
        let mut hoisted_one = false;
        for pos in body_start + 1..i {
            if hoisted_one {
                break;
            }
            if infos[pos].is_nop() {
                continue;
            }

            // Match: movq OFFSET(%rsp), %REG or movq OFFSET(%rbp), %REG
            let t = infos[pos].trimmed(store.get(pos));
            if !t.starts_with("movq ") {
                continue;
            }
            // Parse: "movq SRC, %DST"
            let after_movq = &t[5..];
            let comma = match after_movq.find(", %") {
                Some(c) => c,
                None => continue,
            };
            let src_part = &after_movq[..comma];
            let dst_part = &after_movq[comma + 2..]; // includes %

            // Source must be a stack slot
            if !src_part.ends_with("(%rsp)") && !src_part.ends_with("(%rbp)") {
                continue;
            }
            // Destination must be a GP register
            let dst_family = register_family_fast(dst_part);
            if dst_family == REG_NONE || dst_family > REG_GP_MAX {
                continue;
            }
            // Don't hoist into rax/rcx (accumulator) or rsp/rbp
            // Skip rax (primary accumulator), rsp, rbp (frame registers)
            if dst_family == 0 || dst_family == 4 || dst_family == 5 {
                continue;
            }

            // Parse the numeric offset
            let offset_end = src_part.find('(').unwrap_or(src_part.len());
            let numeric_offset: i32 = src_part[..offset_end].parse().unwrap_or(i32::MIN);
            if numeric_offset == i32::MIN {
                continue;
            }

            // Check: the stack slot is NOT written anywhere in [header..=i]
            let mut slot_written = false;
            for chk in header..=i {
                if infos[chk].is_nop() {
                    continue;
                }
                if let LineKind::StoreRbp { offset: o, .. } = infos[chk].kind {
                    if o == numeric_offset {
                        slot_written = true;
                        break;
                    }
                }
                // Also check Other instructions that store to this offset
                let ct = infos[chk].trimmed(store.get(chk));
                if ct.ends_with(src_part)
                    && (ct.starts_with("movq ")
                        || ct.starts_with("movl ")
                        || ct.starts_with("movb ")
                        || ct.starts_with("movw "))
                {
                    // This could be a store TO this slot
                    if let Some(c) = ct.find(", ") {
                        if ct[c + 2..] == *src_part {
                            slot_written = true;
                            break;
                        }
                    }
                }
            }
            if slot_written {
                continue;
            }

            // Check: the destination register is NOT written by any other instruction
            // in [header..=i] besides this load. Also check it's not used as a
            // destination in any other instruction.
            let dst_bit = 1u16 << dst_family;
            let mut reg_written_elsewhere = false;
            for chk in header..=i {
                if chk == pos {
                    continue;
                } // skip the load itself
                if infos[chk].is_nop() {
                    continue;
                }
                match infos[chk].kind {
                    LineKind::Other { dest_reg } if dest_reg == dst_family => {
                        reg_written_elsewhere = true;
                        break;
                    }
                    LineKind::LoadRbp { reg, .. } if reg == dst_family => {
                        reg_written_elsewhere = true;
                        break;
                    }
                    LineKind::Call => {
                        // Calls clobber caller-saved regs. If dst is caller-saved, bail.
                        if !is_callee_saved_reg(dst_family) {
                            reg_written_elsewhere = true;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            if reg_written_elsewhere {
                continue;
            }

            // All checks passed. Hoist the load into the PREHEADER.
            //
            // SOUND FIX: the previous implementation placed the hoisted load
            // either in a NOP slot before the header or prepended to the header
            // label line. Both are wrong: the header label sits AFTER the entry
            // `jmp .LBB1`, so a load prepended to the header label (or placed in
            // a NOP between the entry jump and the label) is in a DEAD gap that is
            // skipped on entry — the destination register is never loaded and the
            // body reads an uninitialized register.
            //
            // Correct approach: the preheader's last instruction is an unconditional
            // forward `jmp <header>`. Replace THAT jmp with the hoisted load, so the
            // load executes in the preheader and then FALLS THROUGH into the header.
            // This guarantees the load runs on every path that enters the loop.
            //
            // We only hoist when there is a unique unconditional forward entry jump
            // to the header (the natural single-entry preheader). If the entry is a
            // fall-through (no jmp) or has multiple entry edges, we cannot place a
            // dominating load safely and we skip the candidate.
            let load_text = store.get(pos).to_string();
            let header_label = format!("{}:", target);
            // Scan backward from just before the header, skipping NOPs, for the
            // unconditional `jmp <header>` entry jump.
            let mut entry_jmp: Option<usize> = None;
            let mut p = header;
            while p > 0 {
                p -= 1;
                if infos[p].is_nop() {
                    continue;
                }
                let t = infos[p].trimmed(store.get(p));
                if t == format!("jmp {}", target) || t == format!("jmp {}", target_label) {
                    // Must be an unconditional forward jump to the header.
                    entry_jmp = Some(p);
                    break;
                }
                // Stop at a directive boundary or another branch/label (not the
                // immediately-preceding preheader block).
                if infos[p].kind == LineKind::Jmp
                    || infos[p].kind == LineKind::CondJmp
                    || infos[p].kind == LineKind::Label
                    || infos[p].kind == LineKind::Call
                    || infos[p].kind == LineKind::Ret
                {
                    break;
                }
                if p == 0 || p + 8 < header {
                    break;
                }
            }
            // If we didn't find the entry jump right before the header (fall-through
            // or multiple-entry), do NOT hoist (can't place a dominating load safely).
            let Some(entry) = entry_jmp else {
                i += 1;
                continue;
            };
            // SAFETY: replacing the preheader's `jmp <header>` with the hoisted
            // load relies on that load executing on EVERY entry into the loop via
            // fall-through from the preheader. That is only sound when the header
            // has NO other FORWARD entry edge: a conditional branch (e.g.
            // `je <header>`) from a different predecessor enters the loop
            // directly, bypassing the preheader, and would observe an
            // uninitialized destination register. (gzip's deflate: the `rsync`
            // guard's `je` into the hash-table loop header — `head` was reloaded
            // into %rcx only on the fall-through edge, so `head[ins_h]=strstart`
            // wrote through `&rsync`, corrupted globals and SIGSEGV'd.) Back-edges
            // (position > header) are fine: the load is invariant and the
            // destination is not written inside the loop (verified above).
            let mut forward_entries = 0u32;
            for idx in 0..header {
                if infos[idx].is_nop() {
                    continue;
                }
                if matches!(infos[idx].kind, LineKind::Jmp | LineKind::CondJmp) {
                    let t = infos[idx].trimmed(store.get(idx));
                    if let Some(tg) = extract_jump_target(t) {
                        if tg == target.as_str() {
                            forward_entries += 1;
                        }
                    }
                }
            }
            // Exactly one forward reference (the entry jmp itself) is allowed.
            if forward_entries != 1 {
                i += 1;
                continue;
            }
            // Replace the entry jmp with the load; execution falls through into the
            // header. The load is invariant (slot not written in loop, no calls) and
            // the dest register is not written in the loop (checked above).
            replace_line(
                store,
                &mut infos[entry],
                entry,
                load_text.trim_end().to_string(),
            );
            mark_nop(&mut infos[pos]); // remove original in-loop load
            changed = true;
            hoisted_one = true;
        }

        i += 1;
    }
    changed
}

// ── Loop-invariant broadcast hoisting ────────────────────────────────────────
//
// Hoists loop-invariant movsd+vbroadcastsd pairs out of the inner loop:
//
//   .loop:
//     movsd (%REG), %xmm1          →  (NOPed, hoisted before loop)
//     vbroadcastsd %xmm1, %ymm1    →  (NOPed, hoisted before loop)
//     vmovupd ...                      vmovupd ...
//     vfmadd231pd ...                  vfmadd231pd ...
//
// The hoist is safe when %REG is not modified within the loop body.

pub(super) fn hoist_loop_invariant_fp_broadcast(
    store: &mut LineStore,
    infos: &mut [LineInfo],
) -> bool {
    let len = store.len();
    let mut changed = false;
    let mut i = 0;

    while i < len {
        if infos[i].is_nop() {
            i += 1;
            continue;
        }
        // Find back-edge: jl/jle/jne/jb/ja/jmp to a label before this instruction
        let jmp_text = infos[i].trimmed(store.get(i));
        let target = if jmp_text.starts_with("jl ") {
            &jmp_text[3..]
        } else if jmp_text.starts_with("jle ") {
            &jmp_text[4..]
        } else if jmp_text.starts_with("jne ") {
            &jmp_text[4..]
        } else if jmp_text.starts_with("jmp ") {
            &jmp_text[4..]
        } else {
            i += 1;
            continue;
        };
        if !target.starts_with(".L") {
            i += 1;
            continue;
        }

        let target_label = format!("{}:", target);
        let mut header_pos = None;
        for lbl in 0..i {
            if infos[lbl].kind == LineKind::Label
                && infos[lbl].trimmed(store.get(lbl)) == target_label
            {
                header_pos = Some(lbl);
                break;
            }
        }
        let header = match header_pos {
            Some(h) => h,
            None => {
                i += 1;
                continue;
            }
        };

        // Validate: no ret or call in loop
        let mut has_ret = false;
        let mut has_call = false;
        for chk in header..=i {
            if infos[chk].is_nop() {
                continue;
            }
            match infos[chk].kind {
                LineKind::Ret => {
                    has_ret = true;
                    break;
                }
                LineKind::Call => {
                    has_call = true;
                }
                _ => {}
            }
        }
        if has_ret || has_call {
            i += 1;
            continue;
        }

        // Scan loop body for: movsd (%REG), %xmm1 followed by vbroadcastsd %xmm1, %ymm1
        let mut hoisted_one = false;
        for pos in header + 1..i {
            if hoisted_one {
                break;
            }
            if infos[pos].is_nop() {
                continue;
            }
            let t1 = infos[pos].trimmed(store.get(pos));
            if !t1.starts_with("movsd (%") || !t1.ends_with("), %xmm1") {
                continue;
            }

            // Extract the source register
            let reg_start = 7; // after "movsd (%"
            let reg_end = t1.find("), %xmm1").unwrap_or(0);
            if reg_end <= reg_start {
                continue;
            }
            let src_reg = &t1[reg_start..reg_end];

            // Next non-NOP must be vbroadcastsd
            let mut pos2 = pos + 1;
            while pos2 < i && infos[pos2].is_nop() {
                pos2 += 1;
            }
            if pos2 >= i {
                continue;
            }
            let t2 = infos[pos2].trimmed(store.get(pos2));
            if t2 != "vbroadcastsd %xmm1, %ymm1" {
                continue;
            }

            // Check that src_reg is NOT modified within the loop
            let write_pattern = format!(", %{}", src_reg);
            let mut reg_modified = false;
            for chk in header..=i {
                if chk == pos || chk == pos2 {
                    continue;
                }
                if infos[chk].is_nop() {
                    continue;
                }
                let ct = infos[chk].trimmed(store.get(chk));
                if ct.contains(&write_pattern) || ct.ends_with(&format!("%{}", src_reg)) {
                    // Check if it's a destination (after last comma)
                    if let Some(last_comma) = ct.rfind(", ") {
                        let dest_part = &ct[last_comma + 2..];
                        if dest_part.contains(src_reg) {
                            reg_modified = true;
                            break;
                        }
                    }
                }
            }
            if reg_modified {
                continue;
            }

            // Find a NOP slot JUST before the header (within 10 lines) to place
            // both hoisted instructions as a combined two-line string.
            let mut slot = None;
            for p in (0..header).rev() {
                if infos[p].is_nop() {
                    slot = Some(p);
                    break;
                }
                // Only search in the immediate preheader
                if p < header.saturating_sub(10) {
                    break;
                }
                if infos[p].kind == LineKind::Label {
                    break;
                }
            }

            if let Some(s) = slot {
                let movsd_text = store.get(pos).trim_end().to_string();
                let bcast_text = store.get(pos2).trim_end().to_string();
                // Combine both instructions into one slot
                let combined = format!("{}\n{}", movsd_text, bcast_text);
                replace_line(store, &mut infos[s], s, combined);
                mark_nop(&mut infos[pos]);
                mark_nop(&mut infos[pos2]);
                changed = true;
                hoisted_one = true;
            }
        }

        i += 1;
    }
    changed
}

// ── Add + sign-extend fusion ─────────────────────────────────────────────────
//
// Fuses addl + movslq when the intermediate 32-bit register is only used
// as a temporary:
//
//   addl %SRC, %TMP            →  addl %SRC, %DSTd
//   movslq %TMPd, %DST        →  (NOP)
//
// The 32-bit addl into %DSTd automatically zero-extends to 64-bit on x86-64.
// This is safe when the value is non-negative (array indices, loop counters).

// ── Increment chain collapse ─────────────────────────────────────────────────
//
// Collapses a common SSA phi-resolution pattern for loop counter increments:
//
//   leaq 1(%SRC), %TMP1       →  addl $1, %SRCd
//   movslq %TMP1d, %TMP2      →  movslq %SRCd, %SRC
//   movq %TMP2, %SRC          →  (removed)
//
// Saves 1 instruction per loop iteration. Common in loop counter increments.

pub(super) fn collapse_increment_chain(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    let len = store.len();
    let mut changed = false;
    let mut i = 0;

    while i < len {
        if infos[i].is_nop() {
            i += 1;
            continue;
        }

        // Match: leaq DISP(%SRC), %TMP1
        let ti = infos[i].trimmed(store.get(i));
        if !ti.starts_with("leaq ") {
            i += 1;
            continue;
        }

        let after_leaq = &ti[5..];
        let paren_open = match after_leaq.find('(') {
            Some(p) => p,
            None => {
                i += 1;
                continue;
            }
        };
        let disp_str = &after_leaq[..paren_open];
        let paren_close = match after_leaq.find(')') {
            Some(p) => p,
            None => {
                i += 1;
                continue;
            }
        };
        let src_reg = &after_leaq[paren_open + 1..paren_close];
        let after_paren = &after_leaq[paren_close + 1..];
        if !after_paren.starts_with(", %") {
            i += 1;
            continue;
        }
        let tmp1_reg_name = &after_paren[2..];

        let src_family = register_family_fast(src_reg);
        let tmp1_family = register_family_fast(tmp1_reg_name);
        if src_family == REG_NONE || src_family > REG_GP_MAX {
            i += 1;
            continue;
        }
        if tmp1_family == REG_NONE || tmp1_family > REG_GP_MAX {
            i += 1;
            continue;
        }
        if src_family == tmp1_family || src_family <= 1 {
            i += 1;
            continue;
        }

        let disp: i32 = match disp_str.parse() {
            Ok(d) => d,
            Err(_) => {
                i += 1;
                continue;
            }
        };

        // Next non-NOP: movslq %TMP1d, %TMP2
        let j = next_non_nop(infos, i + 1, len);
        if j >= len || infos[j].is_barrier() {
            i += 1;
            continue;
        }

        let tj = infos[j].trimmed(store.get(j));
        let tmp1_32 = REG_NAMES[1][tmp1_family as usize];
        let expected_prefix = format!("movslq {}, %", tmp1_32);
        if !tj.starts_with(&expected_prefix) {
            i += 1;
            continue;
        }
        let tmp2_reg_str = &tj[expected_prefix.len() - 1..];
        let tmp2_family = register_family_fast(tmp2_reg_str);
        if tmp2_family == REG_NONE || tmp2_family > REG_GP_MAX {
            i += 1;
            continue;
        }

        // Search for movq %TMP2, %SRC within a window of 4 non-NOP instructions
        let src_64 = REG_NAMES[0][src_family as usize];
        let expected_wb = format!("movq {}, {}", tmp2_reg_str, src_64);
        let mut k = j + 1;
        let mut wb_found = false;
        let mut wb_count = 0;
        while k < len && wb_count < 4 {
            if infos[k].is_nop() {
                k += 1;
                continue;
            }
            if infos[k].is_barrier() {
                break;
            }
            let tk = infos[k].trimmed(store.get(k));
            if tk == expected_wb {
                wb_found = true;
                break;
            }
            wb_count += 1;
            k += 1;
        }
        if !wb_found {
            i += 1;
            continue;
        }

        // Check SRC not referenced between i+1 and k
        let src_bit = 1u16 << src_family;
        let mut src_ref = false;
        for mid in (i + 1)..k {
            if infos[mid].is_nop() {
                continue;
            }
            if infos[mid].reg_refs & src_bit != 0 {
                src_ref = true;
                break;
            }
        }
        if src_ref {
            i += 1;
            continue;
        }

        // Check TMPs dead after k
        let tmp1_bit = 1u16 << tmp1_family;
        let tmp2_bit = 1u16 << tmp2_family;
        let mut tmps_dead = false;
        let mut n = k + 1;
        let mut chk = 0;
        while n < len && chk < 8 {
            if infos[n].is_nop() {
                n += 1;
                continue;
            }
            if infos[n].is_barrier() {
                // Control-flow barrier: temps may be live on another edge.
                // Cannot prove dead → abort (leave tmps_dead=false).
                break;
            }
            if infos[n].reg_refs & tmp1_bit != 0 {
                match infos[n].kind {
                    LineKind::Other { dest_reg } if dest_reg == tmp1_family => {}
                    LineKind::LoadRbp { reg, .. } if reg == tmp1_family => {}
                    _ => break,
                }
            }
            if tmp2_family != tmp1_family && infos[n].reg_refs & tmp2_bit != 0 {
                match infos[n].kind {
                    LineKind::Other { dest_reg } if dest_reg == tmp2_family => {}
                    LineKind::LoadRbp { reg, .. } if reg == tmp2_family => {}
                    _ => break,
                }
            }
            chk += 1;
            n += 1;
        }
        if chk >= 8 {
            tmps_dead = true;
        }
        if !tmps_dead {
            i += 1;
            continue;
        }

        // Flags safety check
        let post = next_non_nop(infos, k + 1, len);
        if post < len && !infos[post].is_barrier() {
            let tp = infos[post].trimmed(store.get(post));
            if tp.starts_with("ja")
                || tp.starts_with("jb")
                || tp.starts_with("je")
                || tp.starts_with("jn")
                || tp.starts_with("jg")
                || tp.starts_with("jl")
                || tp.starts_with("js")
                || tp.starts_with("jo")
                || tp.starts_with("set")
                || tp.starts_with("cmov")
                || tp.starts_with("adc")
                || tp.starts_with("sbb")
            {
                i += 1;
                continue;
            }
        }

        // Apply
        let src_32 = REG_NAMES[1][src_family as usize];
        let new_add = format!("    addl ${}, {}", disp, src_32);
        let new_ext = format!("    movslq {}, {}", src_32, src_64);
        replace_line(store, &mut infos[i], i, new_add);
        replace_line(store, &mut infos[j], j, new_ext);
        mark_nop(&mut infos[k]);

        changed = true;
        i = k + 1;
    }
    changed
}

// ── Cascaded shift folding ───────────────────────────────────────────────────
//
// Folds a cascaded shift pattern where a value is shifted, copied, then shifted again:
//
//   movq %SRC, %TMP            →  movq %SRC, %DST
//   shlq $A, %TMP              →  shlq $(A+B), %DST
//   movq %TMP, %DST
//   shlq $B, %DST
//
// Also matches when %TMP == %rax (accumulator pattern):
//   shlq $A, %rax
//   movq %rax, %DST
//   shlq $B, %DST
//
// Saves 2 instructions per occurrence. Common in array address computation
// where stride = element_size * vector_width (e.g., 8 * 4 = 32 for AVX2 doubles).

pub(super) fn fold_cascaded_shifts(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    let len = store.len();
    let mut changed = false;
    let mut i = 0;

    while i < len {
        if infos[i].is_nop() {
            i += 1;
            continue;
        }

        let ti = infos[i].trimmed(store.get(i));

        // Pattern 1: movq %SRC, %TMP; shlq $A, %TMP; movq %TMP, %DST; shlq $B, %DST
        if ti.starts_with("movq %") && ti.contains(", %") && !ti.contains("(%") {
            let comma = match ti.find(", %") {
                Some(c) => c,
                None => {
                    i += 1;
                    continue;
                }
            };
            let src_name = &ti[5..comma];
            let tmp_name = &ti[comma + 2..];
            if src_name == tmp_name || !src_name.starts_with('%') {
                i += 1;
                continue;
            }
            let src_fam = register_family_fast(src_name);
            let tmp_fam = register_family_fast(tmp_name);
            if src_fam == REG_NONE || tmp_fam == REG_NONE {
                i += 1;
                continue;
            }

            // Next: shlq $A, %TMP
            let j = next_non_nop(infos, i + 1, len);
            if j >= len || infos[j].is_barrier() {
                i += 1;
                continue;
            }
            let tj = infos[j].trimmed(store.get(j));
            let shl_suffix = format!(", {}", tmp_name);
            if !tj.starts_with("shlq $") || !tj.ends_with(&shl_suffix) {
                i += 1;
                continue;
            }
            let shift_a: u32 = match tj[6..tj.len() - shl_suffix.len()].parse() {
                Ok(s) => s,
                Err(_) => {
                    i += 1;
                    continue;
                }
            };

            // Next: movq %TMP, %DST
            let k = next_non_nop(infos, j + 1, len);
            if k >= len || infos[k].is_barrier() {
                i += 1;
                continue;
            }
            let tk = infos[k].trimmed(store.get(k));
            let mov_prefix = format!("movq {}, %", tmp_name);
            if !tk.starts_with(&mov_prefix) {
                i += 1;
                continue;
            }
            let dst_name = &tk[mov_prefix.len() - 1..]; // includes %
            let dst_fam = register_family_fast(dst_name);
            if dst_fam == REG_NONE || dst_fam == tmp_fam {
                i += 1;
                continue;
            }

            // Next: shlq $B, %DST
            let m = next_non_nop(infos, k + 1, len);
            if m >= len || infos[m].is_barrier() {
                i += 1;
                continue;
            }
            let tm = infos[m].trimmed(store.get(m));
            let shl_dst = format!(", {}", dst_name);
            if !tm.starts_with("shlq $") || !tm.ends_with(&shl_dst) {
                i += 1;
                continue;
            }
            let shift_b: u32 = match tm[6..tm.len() - shl_dst.len()].parse() {
                Ok(s) => s,
                Err(_) => {
                    i += 1;
                    continue;
                }
            };

            let total_shift = shift_a + shift_b;
            if total_shift > 63 {
                i += 1;
                continue;
            }

            // Check TMP is dead after this sequence
            let tmp_bit = 1u16 << tmp_fam;
            let mut tmp_dead = false;
            let mut n = m + 1;
            let mut chk = 0;
            while n < len && chk < 8 {
                if infos[n].is_nop() {
                    n += 1;
                    continue;
                }
                if infos[n].is_barrier() {
                    // Control-flow barrier: temp may be live on another edge.
                    // Cannot prove dead → abort (leave tmp_dead=false).
                    break;
                }
                if infos[n].reg_refs & tmp_bit != 0 {
                    match infos[n].kind {
                        LineKind::Other { dest_reg } if dest_reg == tmp_fam => {
                            tmp_dead = true;
                            break;
                        }
                        LineKind::LoadRbp { reg, .. } if reg == tmp_fam => {
                            tmp_dead = true;
                            break;
                        }
                        _ => break, // TMP read → not dead
                    }
                }
                chk += 1;
                n += 1;
            }
            if chk >= 8 {
                tmp_dead = true;
            }
            if !tmp_dead {
                i += 1;
                continue;
            }

            // Apply: movq %SRC, %DST + shlq $total, %DST
            let new_mov = format!("    movq {}, {}", src_name, dst_name);
            let new_shl = format!("    shlq ${}, {}", total_shift, dst_name);
            replace_line(store, &mut infos[i], i, new_mov);
            mark_nop(&mut infos[j]);
            replace_line(store, &mut infos[k], k, new_shl);
            mark_nop(&mut infos[m]);

            changed = true;
            i = m + 1;
            continue;
        }

        i += 1;
    }
    changed
}

// ── Loop rotation ────────────────────────────────────────────────────────────
//
// Moves the loop condition from the header to the latch, eliminating one
// unconditional branch per iteration:
//
//   .header:                    →  .header:            (preheader)
//       <setup>                 →      <setup>
//       <compare>               →      <compare>
//       jCC .exit               →      jCC .exit
//   .body:                      →  .body:              (rotated loop)
//       <body>                  →      <body>
//       jmp .header             →      <setup_copy>
//                               →      <compare_copy>
//                               →      j!CC .body
//
// Only applied when the header setup is 0-3 instructions (simple cases).
// The jmp is replaced with a multi-line string (setup + cmp + inverted branch).

pub(super) fn rotate_loops(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    let len = store.len();
    let mut changed = false;

    let mut i = 0;
    while i < len {
        if infos[i].is_nop() {
            i += 1;
            continue;
        }
        if infos[i].kind != LineKind::Jmp {
            i += 1;
            continue;
        }
        let jmp_text = infos[i].trimmed(store.get(i));
        if !jmp_text.starts_with("jmp ") {
            i += 1;
            continue;
        }
        let target = &jmp_text[4..];
        let target_label = format!("{}:", target);

        // Find the target label (must be before the jmp = back-edge)
        let mut header_pos = None;
        for lbl in 0..i {
            if infos[lbl].kind == LineKind::Label {
                if infos[lbl].trimmed(store.get(lbl)) == target_label {
                    header_pos = Some(lbl);
                    break;
                }
            }
        }
        let header = match header_pos {
            Some(h) => h,
            None => {
                i += 1;
                continue;
            }
        };

        // Validate: no ret/call in the loop body
        let mut has_ret = false;
        let mut has_call = false;
        for chk in header..=i {
            if infos[chk].is_nop() {
                continue;
            }
            match infos[chk].kind {
                LineKind::Ret => {
                    has_ret = true;
                    break;
                }
                LineKind::Call => {
                    has_call = true;
                }
                _ => {}
            }
        }
        if has_ret || has_call {
            i += 1;
            continue;
        }

        // Collect non-NOP instructions between header label and the first conditional jump.
        let mut header_instrs: Vec<usize> = Vec::new();
        let mut cond_jmp_pos = None;
        let mut body_label_pos = None;
        let mut pos = header + 1;
        while pos < i {
            if infos[pos].is_nop() {
                pos += 1;
                continue;
            }
            if infos[pos].kind == LineKind::CondJmp {
                cond_jmp_pos = Some(pos);
                // Find body label right after conditional jump
                let mut bl = pos + 1;
                while bl < i {
                    if infos[bl].is_nop() {
                        bl += 1;
                        continue;
                    }
                    if infos[bl].kind == LineKind::Label {
                        body_label_pos = Some(bl);
                    }
                    break;
                }
                break;
            }
            if infos[pos].kind == LineKind::Label {
                break;
            } // complex header
            if infos[pos].is_barrier() {
                break;
            }
            header_instrs.push(pos);
            pos += 1;
        }

        let cjmp = match cond_jmp_pos {
            Some(c) => c,
            None => {
                i += 1;
                continue;
            }
        };

        // Only handle simple headers (0-3 setup instructions before the cond jmp)
        if header_instrs.len() > 3 {
            i += 1;
            continue;
        }

        // The instruction(s) before the cond jmp must include a compare/test.
        // Find the compare in the header setup.
        let has_cmp = header_instrs.iter().any(|&idx| {
            let t = infos[idx].trimmed(store.get(idx));
            t.starts_with("cmpl ")
                || t.starts_with("cmpq ")
                || t.starts_with("cmpb ")
                || t.starts_with("cmpw ")
                || t.starts_with("testl ")
                || t.starts_with("testq ")
                || matches!(infos[idx].kind, LineKind::Cmp)
        });
        if !has_cmp {
            i += 1;
            continue;
        }

        // Get the conditional jump and invert it
        let cjmp_text = infos[cjmp].trimmed(store.get(cjmp));
        let (cond, exit_label) = match cjmp_text.find(' ') {
            Some(space) => (&cjmp_text[..space], &cjmp_text[space + 1..]),
            None => {
                i += 1;
                continue;
            }
        };

        let inv_cond = match cond {
            "je" => "jne",
            "jne" => "je",
            "jl" => "jge",
            "jge" => "jl",
            "jle" => "jg",
            "jg" => "jle",
            "jb" => "jae",
            "jae" => "jb",
            "jbe" => "ja",
            "ja" => "jbe",
            "js" => "jns",
            "jns" => "js",
            _ => {
                i += 1;
                continue;
            }
        };

        // Find the body label for the rotated backedge
        let body_label = match body_label_pos {
            Some(bl) => {
                let bt = infos[bl].trimmed(store.get(bl));
                bt.trim_end_matches(':').to_string()
            }
            None => {
                i += 1;
                continue;
            }
        };

        // Optimize the latch: detect redundant sign-extend pattern.
        // If the header setup is `movslq %Xd, %Y; cmpl $imm, %Yd` and the body
        // (just before the jmp) has `movslq %Xd, %X`, we can:
        // - NOP the body's `movslq %Xd, %X` (redundant self sign-extend)
        // - Retarget the latch's movslq to `movslq %Xd, %Y` (for next iter's index)
        // - Use `cmpl $imm, %Xd` directly instead of `cmpl $imm, %Yd`
        let mut nop_body_signext = None;
        let mut optimized_latch = false;

        if header_instrs.len() >= 2 {
            // Check: first header instr is `movslq %Xd, %Y`
            let first_setup = infos[header_instrs[0]].trimmed(store.get(header_instrs[0]));
            if first_setup.starts_with("movslq %") {
                if let Some(comma) = first_setup.find(", %") {
                    let src_32 = &first_setup[7..comma]; // e.g., "%r12d"
                    let dst_64 = &first_setup[comma + 2..]; // e.g., "%r14"
                                                            // src_32 should be a 32-bit register like "%r12d"
                                                            // Derive the 64-bit version by removing the 'd' suffix
                    let src_64 = if src_32.ends_with('d') {
                        let base = &src_32[..src_32.len() - 1];
                        if base.starts_with("%r") && base.len() >= 3 {
                            Some(base.to_string())
                        } else if base == "%eax" {
                            Some("%rax".to_string())
                        } else if base == "%ecx" {
                            Some("%rcx".to_string())
                        } else if base == "%edx" {
                            Some("%rdx".to_string())
                        } else if base == "%ebx" {
                            Some("%rbx".to_string())
                        } else if base == "%esi" {
                            Some("%rsi".to_string())
                        } else if base == "%edi" {
                            Some("%rdi".to_string())
                        } else {
                            None
                        }
                    } else {
                        None
                    };

                    if let Some(ref src_64_name) = src_64 {
                        // Look for `movslq %Xd, %X` in the body just before the jmp
                        let self_signext = format!("movslq {}, {}", src_32, src_64_name);
                        // Search backwards from the jmp for this pattern
                        let mut search = if i > 0 { i - 1 } else { 0 };
                        let mut search_count = 0;
                        while search > header && search_count < 4 {
                            if infos[search].is_nop() {
                                if search == 0 {
                                    break;
                                }
                                search -= 1;
                                continue;
                            }
                            let st = infos[search].trimmed(store.get(search));
                            if st == self_signext {
                                nop_body_signext = Some(search);
                                break;
                            }
                            search_count += 1;
                            if search == 0 {
                                break;
                            }
                            search -= 1;
                        }

                        // Check: the compare uses %Yd (the destination of the movslq)
                        if nop_body_signext.is_some() {
                            let last_setup = header_instrs.last().unwrap();
                            let cmp_text = infos[*last_setup].trimmed(store.get(*last_setup));
                            let dst_32 = reg_64_to_32(dst_64);
                            if !dst_32.is_empty() && cmp_text.contains(&dst_32) {
                                // Can optimize: replace cmpl's operand from %Yd to %Xd
                                optimized_latch = true;
                            }
                        }
                    }
                }
            }
        }

        // Build the replacement: duplicate setup + compare + inverted branch to body
        let mut replacement_lines = Vec::new();
        if optimized_latch {
            // Optimized: skip the self sign-extend in the body, keep movslq to index reg,
            // use source register directly in compare
            if let Some(nop_idx) = nop_body_signext {
                mark_nop(&mut infos[nop_idx]); // remove redundant movslq %Xd, %X
            }
            // Emit: movslq %Xd, %Y (for next iter's index)
            let first_setup = store.get(header_instrs[0]).to_string();
            replacement_lines.push(first_setup.trim_end().to_string());
            // Emit remaining setup except the compare, then emit optimized compare
            for &setup_idx in &header_instrs[1..header_instrs.len() - 1] {
                let text = store.get(setup_idx).to_string();
                replacement_lines.push(text.trim_end().to_string());
            }
            // Emit compare with source register instead of destination register
            let first_text = infos[header_instrs[0]].trimmed(store.get(header_instrs[0]));
            if let Some(comma) = first_text.find(", %") {
                let src_32 = &first_text[7..comma]; // e.g., "%r12d"
                let dst_64 = &first_text[comma + 2..]; // e.g., "%r14"
                                                       // `starts_with("%r")` is TRUE for %rdx/%rsi/... too, so the
                                                       // numbered-register test must be explicit or `%rdx` becomes
                                                       // the nonexistent `%rdxd`.
                let dst_32 = reg_64_to_32(dst_64);
                let last_setup_idx = *header_instrs.last().unwrap();
                let cmp_text = store.get(last_setup_idx).to_string();
                let optimized_cmp = cmp_text.trim_end().replace(&dst_32, src_32);
                replacement_lines.push(optimized_cmp);
            }
        } else {
            // Standard: duplicate all setup instructions verbatim
            for &setup_idx in &header_instrs {
                let text = store.get(setup_idx).to_string();
                replacement_lines.push(text.trim_end().to_string());
            }
        }
        replacement_lines.push(format!("    {} {}", inv_cond, body_label));
        // When the loop exits (inverted condition not taken), fall-through goes
        // to the next instruction. But the original loop exit target might be a
        // trampoline block elsewhere (created by phi elimination for critical
        // edges). We must emit a jmp to the original exit target so the fall-
        // through reaches the correct destination with phi initialization copies.
        replacement_lines.push(format!("    jmp {}", exit_label));

        let replacement = replacement_lines.join("\n");

        // Replace the jmp with the duplicated latch + exit jump
        store.replace(i, replacement);
        infos[i] = classify_line(store.get(i));

        changed = true;
        i += 1;
    }
    changed
}

/// Eliminate redundant `leaq src, %rax` when %rax already holds `src` from
/// a previous leaq in the same basic block.
///
/// Pattern: `leaq X, %rax` ... `leaq X, %rax` where %rax wasn't clobbered.
/// The second leaq is eliminated (marked NOP).
///
/// This handles the common accumulator pattern where alloca addresses are
/// recomputed multiple times for successive Load/Store operations.
pub(super) fn eliminate_redundant_leaq(store: &LineStore, infos: &mut [LineInfo]) -> bool {
    let len = store.len();
    let mut changed = false;
    // Track: the last `leaq SRC, %rax` and the register families its source
    // READS. A later `leaq SRC, %rax` is redundant ONLY if %rax still holds
    // the value SRC evaluated to at the time of the first leaq, which requires:
    //   (a) %rax was not written in between,
    //   (b) none of the registers referenced by SRC were written in between
    //       (their values are baked into the address), and
    //   (c) SRC does not reference %rax itself — a leaq whose source reads
    //       %rax modifies the base that its own source depends on, so a later
    //       identical `leaq SRC, %rax` computes a DIFFERENT address
    //       (e.g. `leaq 24(%rax), %rax` twice = base+24 then base+48).
    //       Such leaqs are therefore never cached (and never eliminated).
    // Control flow (labels, jmp, ANY conditional jump, call, ret) invalidates
    // the cache: a jump can enter the region from anywhere, so the first leaq
    // no longer dominates the second.
    let mut rax_leaq_src: Option<(String, Vec<RegId>)> = None;

    // Register families written by `line` (REG_NONE entries ignored).
    // Returns an empty slice when the destination is memory or unknown.
    fn written_families(line: &str) -> Vec<RegId> {
        // %rsp writes are as real as any data-register write: every cached
        // `leaq X(%rsp), %rax` bakes the CURRENT %rsp into the address.  A
        // push/pop/leave/enter makes that address stale even when the textual
        // displacement is identical.  Missing `push` let this peephole delete
        // a second `leaq 80(%rsp), %rax` after four pushes had moved %rsp;
        // sret calls passing a by-value struct then reused the argument address
        // as the return buffer (gcc.c-torture/execute/20040709-{1,2,3}.c).
        const RSP: RegId = 4;
        const RBP: RegId = 5;
        if line.starts_with("push") {
            // push writes memory and decrements %rsp. It has no destination
            // register operand, so the generic comma fallback would return
            // nothing — handle it before that.
            return vec![RSP];
        }
        if line.starts_with("pop") {
            // popq %r12 / pop %r12 — writes the operand register AND %rsp.
            let mut fams = Vec::with_capacity(2);
            if let Some(sp) = line.find('%') {
                let fam = register_family_fast(line[sp..].trim());
                if fam != REG_NONE {
                    fams.push(fam);
                }
            }
            fams.push(RSP);
            return fams;
        }
        match line {
            "cltq" => return vec![0],
            "cqto" | "cqo" => return vec![0, 2],
            "cdq" => return vec![2],
            "leave" => return vec![RSP, RBP],
            "enter" => return vec![RSP, RBP],
            _ => {}
        }
        // Generic: `... , %reg` — the trailing register is the destination.
        if let Some(comma) = line.rfind(',') {
            let dst = line[comma + 1..].trim();
            if let Some(pct) = dst.find('%') {
                let fam = register_family_fast(&dst[pct..]);
                if fam != REG_NONE {
                    return vec![fam];
                }
            }
        }
        Vec::new()
    }

    for i in 0..len {
        if infos[i].is_nop() {
            continue;
        }
        let line = infos[i].trimmed(store.get(i));

        // Block/control-flow boundary resets tracking: labels, unconditional
        // jumps, ANY conditional jump (back edges, jumps into the middle),
        // calls, returns.
        if line.ends_with(':')
            || line.starts_with(".LBB")
            || line == "ret"
            || line.starts_with("jmp ")
            || line.starts_with("call ")
            || (line.starts_with('j') && !line.starts_with("jmp "))
        {
            rax_leaq_src = None;
            continue;
        }

        // Check if this is `leaq X, %rax`
        if line.starts_with("leaq ") && line.ends_with(", %rax") {
            let src = &line[5..line.len() - 6]; // between "leaq " and ", %rax"
                                                // Collect the register families referenced by the source.
            let mut src_fams: Vec<RegId> = Vec::new();
            let mut rest = src;
            while let Some(pct) = rest.find('%') {
                let tok = &rest[pct + 1..];
                let end = tok
                    .find(|c: char| !c.is_ascii_alphanumeric())
                    .unwrap_or(tok.len());
                let reg = format!("%{}", &tok[..end]);
                let fam = register_family_fast(&reg);
                if fam != REG_NONE && !src_fams.contains(&fam) {
                    src_fams.push(fam);
                }
                rest = &rest[pct + 1..];
            }
            // SOUNDNESS (c): never treat leaqs whose source reads %rax as
            // redundant, and never cache them.
            let src_refs_rax = src_fams.contains(&0);
            if let Some((ref prev_src, _)) = rax_leaq_src {
                if !src_refs_rax && src == prev_src.as_str() {
                    // Cache is valid: no cached src register and no %rax was
                    // written since the cached leaq (any such write cleared it).
                    super::super::types::mark_nop(&mut infos[i]);
                    changed = true;
                    continue;
                }
            }
            if !src_refs_rax {
                rax_leaq_src = Some((src.to_string(), src_fams));
            } else {
                rax_leaq_src = None;
            }
            continue;
        }

        // Check if %rax or any cached source register is written (clobbered).
        // Any write to %rax invalidates; any write to a register whose value
        // is baked into the cached leaq's address invalidates as well.
        let written = written_families(line);
        let mut invalidate = false;
        for fam in &written {
            if *fam == 0 {
                invalidate = true;
                break;
            }
            if let Some((_, ref src_fams)) = rax_leaq_src {
                if src_fams.contains(fam) {
                    invalidate = true;
                    break;
                }
            }
        }
        if invalidate {
            rax_leaq_src = None;
        }
    }
    changed
}

/// Compute exact backward GPR liveness on the current textual CFG.
/// This is intentionally local to a peephole invocation, so transformations
/// never consult stale liveness. Direct branches are resolved through labels;
/// unknown indirect control flow is conservatively all-live.
fn compute_gpr_live_out(store: &LineStore, infos: &[LineInfo]) -> Vec<u16> {
    use crate::common::fx_hash::FxHashMap;
    let n = store.len();
    let mut labels = FxHashMap::default();
    for i in 0..n {
        if infos[i].kind == LineKind::Label {
            labels.insert(
                infos[i]
                    .trimmed(store.get(i))
                    .trim_end_matches(':')
                    .to_string(),
                i,
            );
        }
    }
    let mut uses = vec![0u16; n];
    let mut defs = vec![0u16; n];
    let caller_saved: u16 = [0u8, 1, 2, 4, 5, 8, 9, 10, 11]
        .iter()
        .fold(0, |m, r| m | (1u16 << r));
    let call_args: u16 = [0u8, 1, 2, 4, 5, 8, 9]
        .iter()
        .fold(0, |m, r| m | (1u16 << r));
    for i in 0..n {
        let refs = infos[i].reg_refs;
        match infos[i].kind {
            LineKind::LoadRbp { reg, .. } if is_valid_gp_reg(reg) => defs[i] = 1u16 << reg,
            LineKind::StoreRbp { reg, .. } if is_valid_gp_reg(reg) => uses[i] = 1u16 << reg,
            LineKind::Push { reg } if is_valid_gp_reg(reg) => uses[i] = 1u16 << reg,
            LineKind::Pop { reg } if is_valid_gp_reg(reg) => defs[i] = 1u16 << reg,
            LineKind::SetCC { reg } if is_valid_gp_reg(reg) => {
                uses[i] = 1u16 << reg;
                defs[i] = 1u16 << reg;
            }
            LineKind::Call => {
                uses[i] = call_args | refs;
                defs[i] = caller_saved;
            }
            LineKind::Ret => uses[i] = (1u16 << 0) | (1u16 << 2),
            LineKind::Cmp | LineKind::JmpIndirect => uses[i] = refs,
            LineKind::Other { dest_reg } if is_valid_gp_reg(dest_reg) => {
                let bit = 1u16 << dest_reg;
                defs[i] = bit;
                let t = infos[i].trimmed(store.get(i));
                uses[i] = if is_read_modify_write(t) {
                    refs
                } else {
                    refs & !bit
                };
                if has_implicit_reg_usage(t) {
                    uses[i] |= caller_saved;
                    defs[i] |= caller_saved;
                }
            }
            LineKind::Other { .. } => uses[i] = refs,
            _ => uses[i] = refs,
        }
    }
    let mut live_in = vec![0u16; n];
    let mut live_out = vec![0u16; n];
    for _ in 0..(n.max(1) * 2) {
        let mut changed = false;
        for i in (0..n).rev() {
            let t = infos[i].trimmed(store.get(i));
            let mut out = 0u16;
            match infos[i].kind {
                LineKind::Ret => {}
                LineKind::Jmp => {
                    if let Some(target) = t.split_whitespace().last().and_then(|x| labels.get(x)) {
                        out |= live_in[*target];
                    } else {
                        out = u16::MAX;
                    }
                }
                LineKind::JmpIndirect => out = u16::MAX,
                LineKind::CondJmp => {
                    if i + 1 < n {
                        out |= live_in[i + 1];
                    }
                    if let Some(target) = t.split_whitespace().last().and_then(|x| labels.get(x)) {
                        out |= live_in[*target];
                    } else {
                        out = u16::MAX;
                    }
                }
                _ => {
                    if i + 1 < n {
                        out = live_in[i + 1];
                    }
                }
            }
            let inn = uses[i] | (out & !defs[i]);
            if out != live_out[i] || inn != live_in[i] {
                live_out[i] = out;
                live_in[i] = inn;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    live_out
}

/// Fold a copy/32-bit-update/copy-back round trip:
///   movq SRC, TMP; <update> TMPd; movl TMPd, SRCd
/// into:
///   <update> SRCd
/// when TMP is provably dead afterwards.  The update is restricted to ordinary
/// explicit-operand integer ALU operations; one-operand IMUL and partial-width
/// forms are rejected. 32-bit writes zero-extend on x86-64, so both sequences
/// have identical SRC and flag semantics.
/// Fold a copy/32-bit-update/copy-back round trip:
///   movq SRC, TMP; <update> TMPd; movl TMPd, SRCd
/// into:
///   <update> SRCd
/// when TMP is provably dead afterwards.  The update is restricted to ordinary
/// explicit-operand integer ALU operations; one-operand IMUL and partial-width
/// forms are rejected. 32-bit writes zero-extend on x86-64, so both sequences
/// have identical SRC and flag semantics.
pub(super) fn fold_copy_shift_copyback(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    let len = store.len();
    let live_out = compute_gpr_live_out(store, infos);
    let mut changed = false;
    let mut i = 0;
    while i < len {
        if infos[i].is_nop() || infos[i].pinned {
            i += 1;
            continue;
        }
        let a = infos[i].trimmed(store.get(i));
        if !a.starts_with("movq %") || a.contains("(%") {
            i += 1;
            continue;
        }
        let Some((a_src, a_dst)) = a[5..].split_once(',') else {
            i += 1;
            continue;
        };
        let src = register_family_fast(a_src.trim());
        let tmp = register_family_fast(a_dst.trim());
        if !is_valid_gp_reg(src) || !is_valid_gp_reg(tmp) || src == tmp {
            i += 1;
            continue;
        }

        let j = next_non_nop(infos, i + 1, len);
        if j >= len || infos[j].is_barrier() {
            i += 1;
            continue;
        }
        let b = infos[j].trimmed(store.get(j));
        let mnemonic = b.split_ascii_whitespace().next().unwrap_or("");
        let safe_update = matches!(
            mnemonic,
            "incl"
                | "decl"
                | "negl"
                | "notl"
                | "addl"
                | "subl"
                | "andl"
                | "orl"
                | "xorl"
                | "shll"
                | "shrl"
                | "sarl"
                | "sall"
        ) || (mnemonic == "imull" && b.contains(','));
        if !safe_update || !matches!(infos[j].kind, LineKind::Other { dest_reg } if dest_reg == tmp)
        {
            i += 1;
            continue;
        }
        let tmp32 = reg_id_to_name(tmp, MoveSize::L);
        if !b.ends_with(tmp32) {
            i += 1;
            continue;
        }

        let k = next_non_nop(infos, j + 1, len);
        if k >= len || infos[k].is_barrier() {
            i += 1;
            continue;
        }
        let c = infos[k].trimmed(store.get(k));
        let expected = format!("movl {}, {}", tmp32, reg_id_to_name(src, MoveSize::L));
        if c != expected {
            i += 1;
            continue;
        }

        if live_out[k] & (1u16 << tmp) != 0 {
            i += 1;
            continue;
        }

        let rewritten = replace_reg_family(b, tmp, src);
        mark_nop(&mut infos[i]);
        replace_line(store, &mut infos[j], j, format!("    {}", rewritten));
        mark_nop(&mut infos[k]);
        changed = true;
        i = k + 1;
    }
    changed
}

/// Fold a zero-extended 32-bit XOR move diamond:
///   shll $N, B32
///   movl A32, T32
///   movq B64, A64
///   xorq T64, A64
/// into `shll ...; xorl B32, A32` when T is dead and the XOR flags are
/// overwritten before use. SHLL proves B's upper half is zero, so the 64-bit
/// and 32-bit XOR results are identical.
pub(super) fn fold_zero_extended_xor_moves(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    let len = store.len();
    let live_out = compute_gpr_live_out(store, infos);
    let mut changed = false;
    let mut i = 0;
    while i < len {
        if infos[i].is_nop() {
            i += 1;
            continue;
        }
        let sh = infos[i].trimmed(store.get(i));
        if !sh.starts_with("shll $") {
            i += 1;
            continue;
        }
        let Some((_, shdst)) = sh.rsplit_once(',') else {
            i += 1;
            continue;
        };
        let breg = register_family_fast(shdst.trim());
        if !is_valid_gp_reg(breg) {
            i += 1;
            continue;
        }

        let j = next_non_nop(infos, i + 1, len);
        if j >= len || infos[j].is_barrier() {
            i += 1;
            continue;
        }
        let m1 = infos[j].trimmed(store.get(j));
        if !m1.starts_with("movl %") || m1.contains("(%") {
            i += 1;
            continue;
        }
        let Some((asrc, tdst)) = m1[5..].split_once(',') else {
            i += 1;
            continue;
        };
        let areg = register_family_fast(asrc.trim());
        let treg = register_family_fast(tdst.trim());
        if !is_valid_gp_reg(areg)
            || !is_valid_gp_reg(treg)
            || areg == treg
            || areg == breg
            || treg == breg
        {
            i += 1;
            continue;
        }

        let k = next_non_nop(infos, j + 1, len);
        if k >= len || infos[k].is_barrier() {
            i += 1;
            continue;
        }
        let m2 = infos[k].trimmed(store.get(k));
        let expected_m2 = format!(
            "movq {}, {}",
            reg_id_to_name(breg, MoveSize::Q),
            reg_id_to_name(areg, MoveSize::Q)
        );
        if m2 != expected_m2 {
            i += 1;
            continue;
        }

        let m = next_non_nop(infos, k + 1, len);
        if m >= len || infos[m].is_barrier() {
            i += 1;
            continue;
        }
        let xr = infos[m].trimmed(store.get(m));
        let expected_xr = format!(
            "xorq {}, {}",
            reg_id_to_name(treg, MoveSize::Q),
            reg_id_to_name(areg, MoveSize::Q)
        );
        if xr != expected_xr {
            i += 1;
            continue;
        }

        if live_out[m] & (1u16 << treg) != 0 {
            i += 1;
            continue;
        }

        // The original XOR flags may differ in SF width; require them to be
        // killed before any consumer.
        let mut flags_dead = false;
        let mut q = m + 1;
        let limit = (q + 24).min(len);
        while q < limit {
            if infos[q].is_nop() {
                q += 1;
                continue;
            }
            let t = infos[q].trimmed(store.get(q));
            if matches!(infos[q].kind, LineKind::CondJmp | LineKind::SetCC { .. })
                || t.starts_with("cmov")
                || t.starts_with("adc")
                || t.starts_with("sbb")
                || t.starts_with("rcl")
                || t.starts_with("rcr")
                || t.starts_with("pushf")
            {
                break;
            }
            if matches!(infos[q].kind, LineKind::Call | LineKind::Ret) {
                flags_dead = true;
                break;
            }
            if matches!(
                infos[q].kind,
                LineKind::Label | LineKind::Jmp | LineKind::JmpIndirect
            ) {
                break;
            }
            let op = t.split_whitespace().next().unwrap_or("");
            if [
                "add", "sub", "and", "or", "xor", "shl", "shr", "sar", "rol", "ror", "imul", "cmp",
                "test", "inc", "dec", "neg", "ucomis", "comis",
            ]
            .iter()
            .any(|p| op.starts_with(p))
            {
                flags_dead = true;
                break;
            }
            q += 1;
        }
        if q >= limit {
            flags_dead = true;
        }
        if !flags_dead {
            i += 1;
            continue;
        }

        let folded = format!(
            "    xorl {}, {}",
            reg_id_to_name(breg, MoveSize::L),
            reg_id_to_name(areg, MoveSize::L)
        );
        replace_line(store, &mut infos[j], j, folded);
        mark_nop(&mut infos[k]);
        mark_nop(&mut infos[m]);
        changed = true;
        i = m + 1;
    }
    changed
}

/// Recognize canonical rotate synthesis and emit the native rotate instruction:
///   movq S,A; shlq $L,A; movq S,B; shrq $R,B;
///   movq A,S; orq B,S        (L+R=64)
/// -> rolq $L,S
/// The mirror image becomes RORQ. Temporary liveness and EFLAGS liveness are
/// proven on the textual CFG before rewriting.
pub(super) fn fold_rotate_idiom(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    let len = store.len();
    let live_out = compute_gpr_live_out(store, infos);
    let mut changed = false;
    let mut i = 0;
    while i < len {
        if infos[i].is_nop() {
            i += 1;
            continue;
        }
        let a = infos[i].trimmed(store.get(i));
        let Some(rest) = a.strip_prefix("movq ") else {
            i += 1;
            continue;
        };
        let Some((ss, aa)) = rest.split_once(',') else {
            i += 1;
            continue;
        };
        let sreg = register_family_fast(ss.trim());
        let areg = register_family_fast(aa.trim());
        if !is_valid_gp_reg(sreg) || !is_valid_gp_reg(areg) || sreg == areg {
            i += 1;
            continue;
        }
        let j = next_non_nop(infos, i + 1, len);
        if j >= len {
            i += 1;
            continue;
        }
        let sh1 = infos[j].trimmed(store.get(j));
        let (left, imm1) = if sh1.starts_with("shlq $") {
            (true, &sh1[6..sh1.find(',').unwrap_or(6)])
        } else if sh1.starts_with("shrq $") {
            (false, &sh1[6..sh1.find(',').unwrap_or(6)])
        } else {
            i += 1;
            continue;
        };
        if !sh1.ends_with(&format!(", {}", reg_id_to_name(areg, MoveSize::Q))) {
            i += 1;
            continue;
        }
        let n1 = if let Some(hex) = imm1.strip_prefix("0x") {
            u32::from_str_radix(hex, 16)
        } else {
            imm1.parse::<u32>()
        };
        let Ok(n1) = n1 else {
            i += 1;
            continue;
        };
        let k = next_non_nop(infos, j + 1, len);
        if k >= len {
            i += 1;
            continue;
        }
        let mv2 = infos[k].trimmed(store.get(k));
        let Some(r2) = mv2.strip_prefix("movq ") else {
            i += 1;
            continue;
        };
        let Some((s2, b2)) = r2.split_once(',') else {
            i += 1;
            continue;
        };
        let breg = register_family_fast(b2.trim());
        if register_family_fast(s2.trim()) != sreg
            || !is_valid_gp_reg(breg)
            || breg == sreg
            || breg == areg
        {
            i += 1;
            continue;
        }
        let m = next_non_nop(infos, k + 1, len);
        if m >= len {
            i += 1;
            continue;
        }
        let sh2 = infos[m].trimmed(store.get(m));
        let expected = if left { "shrq $" } else { "shlq $" };
        if !sh2.starts_with(expected)
            || !sh2.ends_with(&format!(", {}", reg_id_to_name(breg, MoveSize::Q)))
        {
            i += 1;
            continue;
        }
        let comma = sh2.find(',').unwrap_or(6);
        let raw = &sh2[6..comma];
        let n2 = if let Some(hex) = raw.strip_prefix("0x") {
            u32::from_str_radix(hex, 16)
        } else {
            raw.parse::<u32>()
        };
        let Ok(n2) = n2 else {
            i += 1;
            continue;
        };
        if n1 == 0 || n2 == 0 || n1 + n2 != 64 {
            i += 1;
            continue;
        }
        let q = next_non_nop(infos, m + 1, len);
        if q >= len {
            i += 1;
            continue;
        }
        if infos[q].trimmed(store.get(q))
            != format!(
                "movq {}, {}",
                reg_id_to_name(areg, MoveSize::Q),
                reg_id_to_name(sreg, MoveSize::Q)
            )
        {
            i += 1;
            continue;
        }
        let r = next_non_nop(infos, q + 1, len);
        if r >= len {
            i += 1;
            continue;
        }
        if infos[r].trimmed(store.get(r))
            != format!(
                "orq {}, {}",
                reg_id_to_name(breg, MoveSize::Q),
                reg_id_to_name(sreg, MoveSize::Q)
            )
        {
            i += 1;
            continue;
        }
        if live_out[r] & ((1u16 << areg) | (1u16 << breg)) != 0 {
            i += 1;
            continue;
        }
        // OR and rotate flags differ; require a kill before any consumer.
        let mut fdead = false;
        let mut x = r + 1;
        let lim = (x + 24).min(len);
        while x < lim {
            if infos[x].is_nop() {
                x += 1;
                continue;
            }
            let t = infos[x].trimmed(store.get(x));
            if matches!(infos[x].kind, LineKind::CondJmp | LineKind::SetCC { .. })
                || t.starts_with("cmov")
                || t.starts_with("adc")
                || t.starts_with("sbb")
            {
                break;
            }
            if matches!(infos[x].kind, LineKind::Call | LineKind::Ret) {
                fdead = true;
                break;
            }
            if matches!(
                infos[x].kind,
                LineKind::Label | LineKind::Jmp | LineKind::JmpIndirect
            ) {
                break;
            }
            let op = t.split_whitespace().next().unwrap_or("");
            if [
                "add", "sub", "and", "or", "xor", "shl", "shr", "sar", "imul", "cmp", "test",
                "inc", "dec", "neg", "ucomis", "comis",
            ]
            .iter()
            .any(|z| op.starts_with(z))
            {
                fdead = true;
                break;
            }
            x += 1;
        }
        if x >= lim {
            fdead = true;
        }
        if !fdead {
            i += 1;
            continue;
        }
        let rot = if left { "rolq" } else { "rorq" };
        replace_line(
            store,
            &mut infos[i],
            i,
            format!("    {} ${}, {}", rot, n1, reg_id_to_name(sreg, MoveSize::Q)),
        );
        for z in [j, k, m, q, r] {
            mark_nop(&mut infos[z]);
        }
        changed = true;
        i = r + 1;
    }
    changed
}

/// Remove vector register self-moves (`vmovdqu %ymm0, %ymm0`, `movaps %xmm1,
/// %xmm1`, ...). Moving a register to itself is a no-op that costs a decoded
/// instruction and a dependency without changing any observable state.
pub(super) fn eliminate_vector_self_moves(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    let len = store.len();
    let mut changed = false;
    for i in 0..len {
        if infos[i].is_nop() || infos[i].pinned {
            continue;
        }
        let t = infos[i].trimmed(store.get(i));
        let Some(rest) = t
            .strip_prefix("vmovdqu ")
            .or_else(|| t.strip_prefix("vmovdqa "))
            .or_else(|| t.strip_prefix("vmovupd "))
            .or_else(|| t.strip_prefix("vmovaps "))
            .or_else(|| t.strip_prefix("vmovd "))
            .or_else(|| t.strip_prefix("vmovq "))
            .or_else(|| t.strip_prefix("movdqu "))
            .or_else(|| t.strip_prefix("movdqa "))
            .or_else(|| t.strip_prefix("movups "))
            .or_else(|| t.strip_prefix("movaps "))
        else {
            continue;
        };
        if let Some((src, dst)) = rest.split_once(',') {
            if src.trim() == dst.trim() {
                mark_nop(&mut infos[i]);
                changed = true;
            }
        }
    }
    changed
}

#[cfg(test)]
mod lea_all_uses_tests {
    #[test]
    fn second_lea_redef_same_dst_not_cross_folded() {
        // Two LEAs into the SAME destination register within one block.
        // The first LEA's scan must stop at the second LEA (redefinition);
        // the second LEA's two uses must fold to ITS base/index, never the
        // first's. Regression: vectorize_float_matmul (C[0][0] wrote to A).
        let asm = "\
.LBB12:
    movq %rsi, %r11
    shlq $2, %r11
    leaq (%r15, %r11), %rdx
    movss (%rdx), %xmm3
    movss %xmm2, %xmm4
    mulss %xmm3, %xmm4
    movq %rsi, %r10
    shlq $2, %r10
    leaq (%r8, %r10), %rdx
    movss (%rdx), %xmm5
    addss %xmm4, %xmm5
    movss %xmm5, (%rdx)
    addl $1, %esi
    movslq %esi, %rsi
jmp .LBB11
";
        let out = super::super::peephole_optimize(asm.to_string());
        // The load feeding xmm5 and the store of xmm5 must NEVER use the
        // first LEA's (%r15,%r11) address.
        for line in out.lines() {
            if line.contains("xmm5") && line.contains("(%r15,%r11)") {
                panic!(
                    "second LEA's uses folded to the FIRST LEA's address:\n{}",
                    out
                );
            }
        }
    }
}

#[cfg(test)]
mod lea_scan_debug {
    use super::*;
    use crate::backend::x86::codegen::peephole::types::*;
    #[test]
    fn debug_second_lea_classification() {
        let line = "    leaq (%r8, %r10), %rdx";
        let info = classify_line(line);
        assert_eq!(get_dest_reg(&info), 2);
        assert!(info.reg_refs & (1 << 2) != 0);
    }
    #[test]
    fn debug_direct_pass_call() {
        let asm = "\
    leaq (%r15, %r11), %rdx
    movss (%rdx), %xmm3
    movq %rsi, %r10
    leaq (%r8, %r10), %rdx
    movss (%rdx), %xmm5
    movss %xmm5, (%rdx)
";
        let mut store = LineStore::new(asm.to_string());
        let mut infos: Vec<LineInfo> = (0..store.len())
            .map(|i| classify_line(store.get(i)))
            .collect();
        let changed = fold_lea_all_uses_in_block(&mut store, &mut infos);
        let out: Vec<String> = (0..store.len()).map(|i| store.get(i).to_string()).collect();
        eprintln!("changed={} out=\n{}", changed, out.join("\n"));
        for line in &out {
            assert!(
                !(line.contains("xmm5") && line.contains("(%r15,%r11)")),
                "cross-fold! {}",
                line
            );
        }
    }
}

/// Fuse `movsd %A, %D` + `vOP %S, %D, %D` into `vOP %S, %A, %D`.
///
/// The scalar-FP emitters stage the first operand into the destination with
/// a `movsd` and then apply the (already 3-operand VEX) operation — a pure
/// 2-operand-ISA habit. VEX scalar ops can read the source directly, so the
/// copy is one wasted uop AND one extra dependency-chain link per FP op
/// (nbody's inner pair-interaction showed 7–9 such copies per iteration:
/// every `dx*dx`, `dx*mag`, `dt/x` square/mul/div site).
///
/// Rewrite validity (no commutativity needed — operand ROLES are preserved):
///   original:  D := A (mov); D := D op S   (AT&T `vOP %S, %D, %D`)
///   rewritten: D := A op S                 (AT&T `vOP %S, %A, %D`)
/// Rejected when: a label/branch/call sits between (control could enter
/// between the mov and its consumer); any intervening instruction touches A
/// or D; the mov is a self-move; or S references D (S may be a register or
/// a memory operand, but it must not read the register the mov just wrote).
pub(super) fn fuse_mov_scalar_fp_into_vex_op(
    store: &mut LineStore,
    infos: &mut [LineInfo],
) -> bool {
    let len = store.len();
    let mut changed = false;
    let mut i = 0;

    while i + 1 < len {
        if infos[i].is_nop() {
            i += 1;
            continue;
        }
        // Match: `movsd %xmmA, %xmmD` / `movss %xmmA, %xmmD` (A != D).
        let line_i = infos[i].trimmed(store.get(i));
        let (width, rest) = if let Some(r) = line_i.strip_prefix("movsd %") {
            ("sd", r)
        } else if let Some(r) = line_i.strip_prefix("movss %") {
            ("ss", r)
        } else {
            i += 1;
            continue;
        };
        let Some((src_a, dst_d)) = rest.split_once(", %") else {
            i += 1;
            continue;
        };
        let dst_d = dst_d.trim();
        let src_a = src_a.trim();
        if !src_a.starts_with("xmm") || !dst_d.starts_with("xmm") || src_a == dst_d {
            i += 1;
            continue;
        }
        // Only %xmmN register moves (no memory sources here: those are the
        // load paths, not the copy-staging paths).
        let reg_num = |r: &str| -> Option<u8> {
            let n = r.strip_prefix("xmm").and_then(|d| d.parse::<u8>().ok());
            n.filter(|&n| n <= 15)
        };
        let (Some(a_num), Some(d_num)) = (reg_num(src_a), reg_num(dst_d)) else {
            i += 1;
            continue;
        };

        // Find the next active (non-Nop, non-Empty) line: only Nop/Empty
        // may sit between the mov and its consumer op — anything else (a
        // Label admits control flow between the pair; a Directive or
        // instruction can observe or clobber the staged register) breaks
        // the pair.
        let mut j = i + 1;
        while j < len && (infos[j].is_nop() || matches!(infos[j].kind, LineKind::Empty)) {
            j += 1;
        }
        if j >= len {
            i += 1;
            continue;
        }

        // Match: `vOP<width> %S, %xmmD, %xmmD`.
        let line_j = infos[j].trimmed(store.get(j));
        let (vop, body) = match binary_vex_scalar_mnemonic(line_j, width) {
            Some((m, rest)) => (m, rest.trim_start()),
            None => {
                i += 1;
                continue;
            }
        };
        let Some((s_operand, tail)) = body.split_once(',') else {
            i += 1;
            continue;
        };
        let tail = tail.trim_start();
        let Some((mid, last)) = tail.split_once(',') else {
            i += 1;
            continue;
        };
        let mid = mid.trim();
        let last = last.trim();
        // The two trailing operands must both be exactly the mov's dst.
        if mid != dst_d_full(dst_d) || last != dst_d_full(dst_d) {
            i += 1;
            continue;
        }
        let s_operand = s_operand.trim();
        // S must not read D (the register whose mov we are deleting).
        let d_full = dst_d_full(dst_d);
        if s_operand.contains(d_full.as_str()) {
            i += 1;
            continue;
        }
        // Memory operands may not write (they are loads here by VEX
        // encoding rules; a store-form would not match this shape).

        // Rewrite: delete the mov, retarget the vop's second source to A.
        let replacement = format!("    {} {}, %{}, %{}", vop, s_operand, src_a, dst_d);
        crate::backend::x86::codegen::peephole::types::mark_nop(&mut infos[i]);
        crate::backend::x86::codegen::peephole::types::replace_line(
            store,
            &mut infos[j],
            j,
            replacement,
        );
        changed = true;
        // Continue scanning from the rewritten op (its own result may feed
        // another fusible pair).
        i = j;
    }
    changed
}

/// Fold `movsd MEM, %D` + `vCOMM %S, %D, %D` into `vCOMM MEM, %S, %D`.
///
/// The load staged a memory value into %D which is then both an input and
/// the destination of a COMMUTATIVE scalar VEX op. AT&T VEX allows the
/// memory operand in the first (src2-of-Intel) position, so for commutative
/// ops the load can be folded into the op outright — but only when %D is
/// dead after the op: the mov must keep feeding any later reader. Deadness
/// proof: block-local textual uniqueness (no later active line in the same
/// basic block mentions %D), the same proof style relay_and_lea uses; a
/// label/branch/call ends the scan conservatively. Non-commutative ops
/// (vsub/vdiv) are excluded: their operand roles cannot be swapped.
pub(super) fn fold_scalar_fp_memory_into_vex_op(
    store: &mut LineStore,
    infos: &mut [LineInfo],
) -> bool {
    let len = store.len();
    let mut changed = false;
    let mut i = 0;

    while i + 1 < len {
        if infos[i].is_nop() {
            i += 1;
            continue;
        }
        // Match: `movsd <MEM>, %xmmD` / `movss <MEM>, %xmmD`.
        let line_i = infos[i].trimmed(store.get(i));
        let width = if line_i.starts_with("movsd ") {
            "sd"
        } else if line_i.starts_with("movss ") {
            "ss"
        } else {
            i += 1;
            continue;
        };
        let Some((mem, dst_d)) = line_i[6..].split_once(", %") else {
            i += 1;
            continue;
        };
        let dst_d = dst_d.trim();
        let mem = mem.trim();
        if !dst_d.starts_with("xmm")
            || !dst_d[3..].chars().all(|c| c.is_ascii_digit())
            || mem.starts_with('%')
            || !mem.contains('(')
            || mem.contains("%xmm")
        {
            // dst must be a plain %xmmN; the source must be a memory
            // operand (contains a paren) that is not an XMM-form (e.g.
            // `movsd (%rax), %xmm` is fine; reject register sources).
            i += 1;
            continue;
        }
        let d_full = format!("%{}", dst_d);

        // Next active line.
        let mut j = i + 1;
        while j < len && (infos[j].is_nop() || matches!(infos[j].kind, LineKind::Empty)) {
            j += 1;
        }
        if j >= len {
            i += 1;
            continue;
        }
        let line_j = infos[j].trimmed(store.get(j));
        let Some((vop, body)) = commutative_vex_scalar(line_j, width) else {
            i += 1;
            continue;
        };
        let body = body.trim_start();
        let Some((s_operand, tail)) = body.split_once(',') else {
            i += 1;
            continue;
        };
        let tail = tail.trim_start();
        let Some((mid, last)) = tail.split_once(',') else {
            i += 1;
            continue;
        };
        let mid = mid.trim();
        let last = last.trim();
        let s_operand = s_operand.trim();
        // Memory case shape: `vOP %D, %X, %X` — the loaded register is the
        // FIRST source; the other value X occupies both the middle source
        // and the destination slot. X must be a register distinct from D.
        if s_operand != d_full || mid != last || !mid.starts_with("%xmm") || mid == d_full {
            i += 1;
            continue;
        }

        // Deadness of %D after j: no later ACTIVE line in this basic block
        // mentions it. Labels/branches/calls end the scan conservatively
        // (a call could return to a path that reads D... no — calls end the
        // BLOCK context; treat the block as ended).
        let mut dead = true;
        let mut k = j + 1;
        let mut scanned = 0;
        while k < len && scanned < 64 {
            if infos[k].is_nop() || matches!(infos[k].kind, LineKind::Empty | LineKind::Directive) {
                k += 1;
                continue;
            }
            if matches!(
                infos[k].kind,
                LineKind::Label
                    | LineKind::Jmp
                    | LineKind::JmpIndirect
                    | LineKind::CondJmp
                    | LineKind::Call
                    | LineKind::Ret
            ) {
                break;
            }
            if infos[k].trimmed(store.get(k)).contains(&d_full) {
                dead = false;
                break;
            }
            scanned += 1;
            k += 1;
        }
        if !dead {
            i += 1;
            continue;
        }

        // Rewrite: fold MEM into the op's first source slot (AT&T first
        // operand = Intel src2, the memory-legal position); the duplicated
        // X operand pair stays verbatim. Delete the staging load.
        let replacement = format!("    {} {}, {}, {}", vop, mem, mid, last);
        crate::backend::x86::codegen::peephole::types::mark_nop(&mut infos[i]);
        crate::backend::x86::codegen::peephole::types::replace_line(
            store,
            &mut infos[j],
            j,
            replacement,
        );
        changed = true;
        i = j;
    }
    changed
}

/// Commutative scalar VEX mnemonics (operand roles may be swapped, which
/// the memory fold relies on). Returns (mnemonic, rest-of-line).
fn commutative_vex_scalar<'a>(line: &'a str, width: &str) -> Option<(&'static str, &'a str)> {
    let table: [(&str, &str); 4] = [
        ("vmul", "sd"),
        ("vadd", "sd"),
        ("vmul", "ss"),
        ("vadd", "ss"),
    ];
    for (prefix, w) in table {
        if w == width {
            // build "vmulsd" etc.
            let full: &'static str = match (prefix, w) {
                ("vmul", "sd") => "vmulsd",
                ("vadd", "sd") => "vaddsd",
                ("vmul", "ss") => "vmulss",
                _ => "vaddss",
            };
            if line.starts_with(full) && line[full.len()..].starts_with(' ') {
                return Some((full, &line[full.len()..]));
            }
        }
    }
    None
}

fn dst_d_full(dst: &str) -> String {
    format!("%{}", dst)
}

/// The role-preserving binary scalar VEX mnemonics for `width` (sd or ss).
/// Returns the mnemonic plus the rest of the line.
fn binary_vex_scalar_mnemonic<'a>(line: &'a str, width: &str) -> Option<(&'static str, &'a str)> {
    let _ = width;
    const MNEMONICS: [&str; 12] = [
        "vmulsd", "vaddsd", "vsubsd", "vdivsd", "vminsd", "vmaxsd", "vmulss", "vaddss", "vsubss",
        "vdivss", "vminss", "vmaxss",
    ];
    for m in MNEMONICS {
        if line.starts_with(m) && line[m.len()..].starts_with(' ') {
            let _ = width;
            return Some((m, &line[m.len()..]));
        }
    }
    None
}
