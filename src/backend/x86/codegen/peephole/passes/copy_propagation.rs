//! Register copy propagation pass.
//!
//! Propagates register-to-register copies across entire basic blocks to
//! eliminate intermediate moves. The accumulator-based codegen routes many
//! operations through %rax as an intermediary, producing chains like:
//!
//!   movq %rax, %rcx             # copy rax -> rcx
//!   movq %rcx, %rdx             # copy rcx -> rdx (really rax -> rdx)
//!   addq %rcx, %r8              # uses rcx (really rax)
//!
//! After propagation, this becomes:
//!
//!   movq %rax, %rcx             # potentially dead
//!   movq %rax, %rdx             # uses rax directly
//!   addq %rax, %r8              # uses rax directly
//!
//! The dead movq instructions are cleaned up by subsequent passes.

use super::super::types::*;
use super::helpers::*;
use super::relay_and_lea::{plain_gp_operand, split_two_operands};

/// Dynamic memory operands are normally alias-analysis barriers, but a single
/// ordinary move/LEA has fully explicit register semantics.  It is safe to
/// rewrite an aliased address register even when that instruction overwrites
/// the alias source: x86 evaluates source addresses before writing the
/// destination.  Multi-instruction and implicit-register forms stay opaque.
fn allows_address_copy_propagation(trimmed: &str) -> bool {
    let mnemonic = trimmed.split_ascii_whitespace().next().unwrap_or("");
    (mnemonic.starts_with("mov") || mnemonic.starts_with("lea"))
        && !trimmed.contains(';')
        && !has_implicit_reg_usage(trimmed)
}

// ── narrow-identity substitution ─────────────────────────────────────────────
//
// A `movl` reg-reg copy establishes a LOW-32 identity: bits 0..=31 of the
// destination family equal bits 0..=31 of the source family (the upper half
// is zeroed on BOTH sides of the identity — it is not tracked).  The 64-bit
// arm of the pass below consumes `copy_src` only; two hot shapes were left
// paying for staging relays because their consumers read the 32-bit part:
//
//   movzbl (%r10,%r9), %eax      movl %ebp, %eax
//   movl   %eax, %r9d            movl %eax, (%r8,%r9,4)
//   cmpl   %r9d, %esi
//
// Both are value-identical operand substitutions under the low-32 identity.
// The exact contract each substitution honors:
//
//   N1  `copy_src32[F] == S` claims equality of bits 0..=31 ONLY.
//   N2  A consumer substitution rewrites every occurrence of family `F`
//       whose width is 32 bits or narrower and requires:
//         (a) NO occurrence of the 64-bit name of `F` (reads bits 32..=63,
//             outside the identity) and NO legacy high-byte name of `F`
//             (%ah/%ch/%dh/%bh) — those stay unrewritten, and a line mixing
//             rewritten and unrewritten occurrences of one family is
//             refused outright;
//         (b) the line does not WRITE family `F` at any width — the result
//             must land in the register the program expects (this also
//             covers self-RMW forms, where retargeting BOTH operands would
//             redirect the result);
//         (c) no implicit register usage (break, as in the 64-bit arm);
//         (d) no legacy high-byte name of ANY family next to an
//             introduced REX-requiring low byte (%spl/%bpl/%sil/%dil/
//             %r8b-%r15b) — that mix is unencodable (the 64-bit arm
//             holds the same line);
//       Memory address operands are intrinsically safe to refuse: base and
//       index registers are 64-bit names, so (a) rejects any line that
//       references `F` inside an address.
//   N3  A scalar store `movX %S, MEM` reads exactly width X of family `S`,
//       so `movq` needs the FULL identity (`copy_src`) while `movl/movw/
//       movb` need only `copy_src32`; only the source operand is rewritten.
//   N4  Both sites run under the pass's existing state discipline: every
//       barrier clears the tables, every write invalidates the family, and
//       the line is reclassified after each rewrite.

/// Whole-name occurrence test mirroring `replace_reg_name_exact`'s
/// boundary rule: `%r8` must not match inside `%r8d`.
fn contains_reg_name(line: &str, name: &str) -> bool {
    let bytes = line.as_bytes();
    let nb = name.as_bytes();
    let mut pos = 0;
    while pos + nb.len() <= bytes.len() {
        if &bytes[pos..pos + nb.len()] == nb {
            let after = pos + nb.len();
            if (after == bytes.len() || matches!(bytes[after], b',' | b')' | b' ' | b'\t' | b'\n'))
                && line.is_char_boundary(pos)
                && line.is_char_boundary(after)
            {
                return true;
            }
        }
        pos += 1;
    }
    false
}

/// N2(a): true when `line` never reads family `fam` outside bits 0..=31.
fn only_narrow_uses_of(line: &str, fam: RegId) -> bool {
    if contains_reg_name(line, REG_NAMES[0][fam as usize]) {
        return false;
    }
    if (fam as usize) < 4 && contains_reg_name(line, HIGH_BYTE_NAMES[fam as usize]) {
        return false;
    }
    true
}

/// True when introducing family `new_fam` into `line` would mix a
/// REX-requiring low-byte name (`%spl`/`%bpl`/`%sil`/`%dil`/`%r8b`-`%r15b`)
/// with a legacy high-byte name (`%ah`/`%ch`/`%dh`/`%bh`) — unencodable
/// (the assembler rejects it). Rewrites never touch high bytes
/// themselves, so any high byte present in the instruction (comments
/// stripped) stays. Guards BOTH copy arms.
fn mixes_rex_with_high_byte(line: &str, new_fam: RegId) -> bool {
    if (new_fam as usize) < 4 || (new_fam as usize) >= REG_NAMES[3].len() {
        return false; // low fams need no REX for their low byte
    }
    let code = line.split('#').next().unwrap_or(line);
    HIGH_BYTE_NAMES.iter().any(|hb| contains_reg_name(code, hb))
}

/// Try to replace uses of `dst_id` with `src_id` in instruction at index `j`.
/// Returns true if a replacement was made.
fn try_propagate_into(
    store: &mut LineStore,
    infos: &mut [LineInfo],
    j: usize,
    src_id: RegId,
    dst_id: RegId,
) -> bool {
    let trimmed = infos[j].trimmed(store.get(j));

    // Dynamic memory is opaque except for the narrow source-address case
    // above.  In particular, do not weaken barriers for inline asm or
    // instructions with architectural operands omitted from the text.
    if infos[j].has_indirect_mem && !allows_address_copy_propagation(trimmed) {
        return false;
    }

    // Defense in depth: a line classified as inline asm must never be
    // rewritten even if some future caller reaches here with a textually
    // plausible template line — user bytes are immutable.
    if infos[j].kind == LineKind::InlineAsm {
        return false;
    }

    // The instruction must reference the destination register
    if infos[j].reg_refs & (1u16 << dst_id) == 0 {
        return false;
    }

    // Skip instructions with implicit register usage
    if has_implicit_reg_usage(trimmed) {
        return false;
    }

    // SOUNDNESS: a self-referencing zeroing idiom (`xor %r,%r`, `sub %r,%r`)
    // does NOT read %r — it is a pure define whose result is always zero, and
    // the register appears twice only as an encoding artifact. Treating the
    // first operand as a use and rewriting it to the copy source turns
    // `xorl %eax,%eax` into `xorl %ecx,%eax`, which computes a garbage value
    // AND destroys the zeroing. In an argument-setup sequence that silently
    // wrecks the call: `f(a,b,c,d)` lost its %rdi/%rsi/%rdx/%rcx setup and
    // clobbered %rcx (reduced from expat's XmlInitUnknownEncodingNS, which
    // segfaulted when xmltok.c was built at -O2).
    if is_self_zeroing_idiom(trimmed) {
        return false;
    }

    // Skip shift/rotate when propagating into %rcx (they need %cl)
    if dst_id == 1 && is_shift_or_rotate(trimmed) {
        return false;
    }

    // ENCODING: introducing a REX low-byte name next to a legacy high
    // byte (%ah/%ch/%dh/%bh) is unencodable — refuse the rewrite.
    if mixes_rex_with_high_byte(trimmed, src_id) {
        return false;
    }

    let next_dest = get_dest_reg(&infos[j]);

    let src_name = REG_NAMES[0][src_id as usize];
    let dst_name = REG_NAMES[0][dst_id as usize];

    // Case 1: next instruction writes to src_id
    if next_dest == src_id {
        // Source register is being written by this instruction.
        // Only safe if dst appears ONLY in a memory base position like (%dst).
        let dst_paren = format!("({})", dst_name);
        if !trimmed.contains(dst_paren.as_str()) {
            return false;
        }
        let src_paren = format!("({})", src_name);
        let src_direct_count = trimmed.matches(src_name).count();
        let src_paren_count = trimmed.matches(src_paren.as_str()).count();
        let is_dest_only = if let Some((_before, after_comma)) = trimmed.rsplit_once(',') {
            after_comma.trim() == src_name
        } else {
            false
        };
        let src_as_source = src_direct_count - src_paren_count - if is_dest_only { 1 } else { 0 };
        if src_as_source > 0 {
            return false;
        }
        let new_text = format!("    {}", replace_reg_family(trimmed, dst_id, src_id));
        replace_line(store, &mut infos[j], j, new_text);
        return true;
    }

    // Case 2: dst is not the destination - replace all occurrences
    if next_dest != dst_id {
        let new_content = replace_reg_family(trimmed, dst_id, src_id);
        if new_content != trimmed {
            let new_text = format!("    {}", new_content);
            replace_line(store, &mut infos[j], j, new_text);
            return true;
        }
        return false;
    }

    // Case 3: dst is the destination AND a source (e.g., addq %rcx, %rcx)
    // For single-operand read-modify-write instructions (negq, notq, incq, decq),
    // the single operand is both source and destination. Don't replace.
    if !trimmed.contains(',') {
        return false;
    }
    // Only replace the source-position occurrences.
    let new_content = replace_reg_family_in_source(trimmed, dst_id, src_id);
    if new_content != trimmed {
        let new_text = format!("    {}", new_content);
        replace_line(store, &mut infos[j], j, new_text);
        return true;
    }
    false
}

pub(super) fn propagate_register_copies(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    let mut changed = false;
    let len = store.len();

    // copy_src[dst] = src means "dst currently holds the same value as src"
    let mut copy_src: [RegId; 16] = [REG_NONE; 16];
    // copy_src32 holds the same relation for the LOW 32 bits (from movl
    // reg-reg chains). Besides shortening movl chains to their ultimate
    // 32-bit source it feeds the narrow N2 consumers (the upper 32 bits
    // of a 32-bit chain are not tracked here, so a line reading the
    // 64-bit name is refused) and the N3 scalar-store source rewrite;
    // only the zero-upper pass may DELETE a movl (a movl self-copy
    // zeroes the upper 32 bits).
    let mut copy_src32: [RegId; 16] = [REG_NONE; 16];

    let mut i = 0;
    while i < len {
        // At basic block boundaries, clear all copies. Previously this kept
        // copy state across a CondJmp, but that is UNSOUND: the branch-taken
        // target is reached via a separate edge whose copy state may differ,
        // and when that path falls back into this basic block's successor the
        // propagated source is stale. Conservative and correct: clear at every
        // barrier (jump, conditional jump, call, label, ret, directive).
        if infos[i].is_barrier() {
            copy_src = [REG_NONE; 16];
            copy_src32 = [REG_NONE; 16];
            i += 1;
            continue;
        }

        if infos[i].is_nop() {
            i += 1;
            continue;
        }

        // User inline assembly is opaque: its template may redefine any
        // register and its bytes must reach the assembler untouched.  Both
        // propagation into the line (a template line can textually name a
        // tracked copy's source — the `src == dest` reader-retargeting arm
        // below would rewrite the user's own instruction) and copy state
        // ACROSS the region (the emitter substitutes template operands the
        // classifier cannot attribute) are forbidden.  Kill all identities.
        if infos[i].kind == LineKind::InlineAsm {
            copy_src = [REG_NONE; 16];
            copy_src32 = [REG_NONE; 16];
            i += 1;
            continue;
        }

        // Dynamic-memory lines remain barriers, except for one final reader
        // that overwrites the alias SOURCE itself.  Rewrite that address before
        // clearing state; do not propagate into arbitrary memory operations,
        // because doing so can interfere with stronger SIB/LEA folds.
        let trimmed = infos[i].trimmed(store.get(i));
        if infos[i].has_indirect_mem {
            // Owned copy: `trimmed` borrows the store, and the mutable
            // rewrite calls below must not see a live borrow.
            let line_text = trimmed.to_string();
            if allows_address_copy_propagation(&line_text) {
                let dest = get_dest_reg(&infos[i]);
                for reg in 0..16u8 {
                    let src = copy_src[reg as usize];
                    if src != REG_NONE && src == dest && infos[i].reg_refs & (1u16 << reg) != 0 {
                        if try_propagate_into(store, infos, i, src, reg) {
                            changed = true;
                        }
                        break;
                    }
                }
                // N3: a scalar store whose SOURCE register is a tracked
                // copy can read the copy's origin directly — the memory
                // destination is untouched and the store width selects the
                // identity level (`movq` needs the full 64-bit identity,
                // the narrow forms need only low-32).  All text is
                // materialised into owned strings before the rewrite so
                // the mutable store call borrows nothing stale.
                if !infos[i].pinned {
                    let parsed: Option<(usize, &str)> = line_text
                        .strip_prefix("movq ")
                        .map(|rest| (0usize, rest))
                        .or_else(|| line_text.strip_prefix("movl ").map(|rest| (1, rest)))
                        .or_else(|| line_text.strip_prefix("movw ").map(|rest| (2, rest)))
                        .or_else(|| line_text.strip_prefix("movb ").map(|rest| (3, rest)));
                    if let Some((width_row, rest)) = parsed {
                        if let Some((src_op, dst_op)) = split_two_operands(rest) {
                            if dst_op.contains('(') && src_op.starts_with('%') {
                                if let Some(fam) = plain_gp_operand(src_op) {
                                    let tracked = if width_row == 0 {
                                        copy_src[fam as usize]
                                    } else {
                                        copy_src32[fam as usize]
                                    };
                                    if tracked != REG_NONE && tracked != fam {
                                        let mnemonic = String::from(
                                            &line_text[..line_text.len() - rest.len()],
                                        );
                                        let new_src = replace_reg_name_exact(
                                            src_op,
                                            REG_NAMES[width_row][fam as usize],
                                            REG_NAMES[width_row][tracked as usize],
                                        );
                                        let dst_owned = dst_op.to_string();
                                        let full = store.get(i).to_string();
                                        let lead_len = full.len() - line_text.len();
                                        let new_line = format!(
                                            "{}{}{}, {}",
                                            &full[..lead_len],
                                            mnemonic,
                                            new_src,
                                            dst_owned
                                        );
                                        replace_line(store, &mut infos[i], i, new_line);
                                        changed = true;
                                    }
                                }
                            }
                        }
                    }
                }
            }
            copy_src = [REG_NONE; 16];
            copy_src32 = [REG_NONE; 16];
            i += 1;
            continue;
        }

        // Check if this is a reg-to-reg movq that defines a new copy
        if let Some((src_id, dst_id)) = parse_reg_to_reg_movq(&infos[i], trimmed) {
            // Resolve transitive copies
            let ultimate_src = if copy_src[src_id as usize] != REG_NONE {
                copy_src[src_id as usize]
            } else {
                src_id
            };

            if ultimate_src != src_id && ultimate_src != dst_id {
                let new_src_name = REG_NAMES[0][ultimate_src as usize];
                let dst_name = REG_NAMES[0][dst_id as usize];
                let new_text = format!("    movq {}, {}", new_src_name, dst_name);
                replace_line(store, &mut infos[i], i, new_text);
                changed = true;
            } else if ultimate_src == dst_id {
                mark_nop(&mut infos[i]);
                changed = true;
                i += 1;
                continue;
            }

            // Before recording: invalidate any copies that have dst as their source.
            for k in 0..16u8 {
                if copy_src[k as usize] == dst_id {
                    copy_src[k as usize] = REG_NONE;
                }
            }

            // Record the copy (a 64-bit identity implies the low-32 identity)
            copy_src[dst_id as usize] = ultimate_src;
            copy_src32[dst_id as usize] = ultimate_src;
            for k in 0..16u8 {
                if copy_src32[k as usize] == dst_id {
                    copy_src32[k as usize] = REG_NONE;
                }
            }

            i += 1;
            continue;
        }

        // 32-bit reg-reg copy: shorten to the ultimate low-32 source. A
        // `movl %B, %C` where B's low 32 bits provably equal A's rewrites to
        // `movl %A, %C` — value-identical (both zero the upper half). The
        // self-reduced `movl %R, %R` is left for the zero-upper pass to
        // delete when R's upper half is provably zero.
        if let Some((src_id, dst_id)) = parse_reg_to_reg_movl(&infos[i], trimmed) {
            let ultimate_src = if copy_src32[src_id as usize] != REG_NONE {
                copy_src32[src_id as usize]
            } else {
                src_id
            };
            if ultimate_src != src_id {
                let new_src_name = REG_NAMES[1][ultimate_src as usize];
                let dst_name = REG_NAMES[1][dst_id as usize];
                let new_text = format!("    movl {}, {}", new_src_name, dst_name);
                replace_line(store, &mut infos[i], i, new_text);
                changed = true;
            }
            // Before recording: invalidate any copies that have dst as their source.
            for k in 0..16u8 {
                if copy_src32[k as usize] == dst_id {
                    copy_src32[k as usize] = REG_NONE;
                }
            }
            copy_src32[dst_id as usize] = ultimate_src;
            // A movl establishes only the low 32 bits of dst, so the 64-bit
            // copy graph through/downstream-of dst is dead — its value is no
            // longer equal to any tracked register (a 32-bit write zeroes
            // bits 32..=63, and the resulting value is not the source's
            // full-width value). Failing to invalidate let a later movq
            // propagate a stale full-width identity (miscompiled
            // vla_struct_sizeof / small_slot_* / phi tests).
            copy_src[dst_id as usize] = REG_NONE;
            for k in 0..16u8 {
                if copy_src[k as usize] == dst_id {
                    copy_src[k as usize] = REG_NONE;
                }
            }
            i += 1;
            continue;
        }

        // Inline asm is opaque: `"=r"` outputs and clobbers may redefine any
        // register and the text is pinned (never rewritten).  Every recorded
        // copy is stale afterwards; the classified `dest_reg` is REG_NONE for
        // this kind, so the generic invalidation below would keep them alive.
        if infos[i].kind == LineKind::InlineAsm {
            copy_src = [REG_NONE; 16];
            copy_src32 = [REG_NONE; 16];
            i += 1;
            continue;
        }

        // Not a copy instruction. Try to propagate active copies into this instruction.
        let dest_reg = get_dest_reg(&infos[i]);

        let mut did_propagate = false;
        for reg in 0..16u8 {
            let src = copy_src[reg as usize];
            if src == REG_NONE {
                continue;
            }
            if infos[i].reg_refs & (1u16 << reg) == 0 {
                continue;
            }
            let cur_trimmed = infos[i].trimmed(store.get(i));
            if has_implicit_reg_usage(cur_trimmed) {
                break;
            }

            if try_propagate_into(store, infos, i, src, reg) {
                changed = true;
                did_propagate = true;
                break;
            }
        }

        // N2: low-32 identities into consumers that read only the 32-bit
        // (or narrower) part of the family.  `movl` zero-extends
        // identically on both sides of the identity, so those reads see
        // exactly the same value for the copy and its origin.  This kills
        // the staging relays the 64-bit arm cannot touch (`movl %ebp,
        // %eax` before a `cmpl`/store; `movl %eax, %r9d` between a
        // `movzbl` load and its compare).
        for reg in 0..16u8 {
            let src = copy_src32[reg as usize];
            if src == REG_NONE || src == reg {
                continue;
            }
            if infos[i].reg_refs & (1u16 << reg) == 0 {
                continue;
            }
            // N2(b): the identity covers READS only — a line writing the
            // family must keep its destination register.
            if get_dest_reg(&infos[i]) == reg {
                continue;
            }
            let cur_trimmed = infos[i].trimmed(store.get(i));
            // N2(c): mirrors the 64-bit arm above.
            if has_implicit_reg_usage(cur_trimmed) {
                break;
            }
            // Shift/rotate counts are %cl by architecture: the 64-bit arm
            // refuses to propagate into %rcx for exactly this reason, and
            // the narrow arm must hold the same line even though %cl is a
            // byte-width (narrow) occurrence.
            if reg == 1 && is_shift_or_rotate(cur_trimmed) {
                continue;
            }
            // N2(a): refuse 64-bit reads and mixed high-byte lines.
            if !only_narrow_uses_of(cur_trimmed, reg) {
                continue;
            }
            // N2(d): refuse a REX low-byte introduction next to a legacy
            // high byte (unencodable). Uses the shared UTF-8-safe
            // rewriter: a byte-wise `as char` copy recodes non-ASCII
            // comment bytes even when nothing is replaced.
            if mixes_rex_with_high_byte(cur_trimmed, src) {
                continue;
            }
            let mut new_text = cur_trimmed.to_string();
            for row in 1..=3usize {
                new_text = replace_reg_name_exact(
                    &new_text,
                    REG_NAMES[row][reg as usize],
                    REG_NAMES[row][src as usize],
                );
            }
            if new_text != cur_trimmed {
                let new_line = {
                    let full = store.get(i);
                    let lead_len = full.len() - cur_trimmed.len();
                    format!("{}{}", &full[..lead_len], new_text)
                };
                replace_line(store, &mut infos[i], i, new_line);
                changed = true;
                break;
            }
        }

        // If we propagated, don't increment i - re-process.
        // But we still need to do invalidation below.
        let _ = did_propagate;

        // Invalidate copies affected by this instruction's writes.
        if dest_reg != REG_NONE && dest_reg <= REG_GP_MAX {
            copy_src[dest_reg as usize] = REG_NONE;
            for k in 0..16u8 {
                if copy_src[k as usize] == dest_reg {
                    copy_src[k as usize] = REG_NONE;
                }
            }
            copy_src32[dest_reg as usize] = REG_NONE;
            for k in 0..16u8 {
                if copy_src32[k as usize] == dest_reg {
                    copy_src32[k as usize] = REG_NONE;
                }
            }
        }

        // Instructions with implicit register usage conservatively invalidate
        // all. The oracle pass below additionally retires exactly the
        // identities of implicit writers the boolean veto list does not name
        // (single-operand `imul`, string primitives, `loop`, ...): those
        // clobber architectural registers without tripping the blanket.
        {
            let cur_trimmed = infos[i].trimmed(store.get(i));
            if has_implicit_reg_usage(cur_trimmed) {
                copy_src = [REG_NONE; 16];
                copy_src32 = [REG_NONE; 16];
            }
            let implicit_writes = implicit_write_refs(cur_trimmed.as_bytes());
            for fam in 0..=REG_GP_MAX {
                if implicit_writes & (1u16 << fam) != 0 {
                    for k in 0..16u8 {
                        if k == fam || copy_src[k as usize] == fam {
                            copy_src[k as usize] = REG_NONE;
                        }
                        if k == fam || copy_src32[k as usize] == fam {
                            copy_src32[k as usize] = REG_NONE;
                        }
                    }
                }
            }
        }

        i += 1;
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::peephole_common::LineStore;

    fn build_pinned(asm: &str) -> (LineStore, Vec<LineInfo>) {
        let store = LineStore::new(asm.to_string());
        let mut infos: Vec<LineInfo> = (0..store.len())
            .map(|i| classify_line(store.get(i)))
            .collect();
        let mut in_asm = false;
        for i in 0..infos.len() {
            let t = infos[i].trimmed(store.get(i)).to_string();
            if t == "#APP" {
                in_asm = true;
                continue;
            }
            if t == "#NO_APP" {
                in_asm = false;
                continue;
            }
            if in_asm {
                infos[i] = LineInfo {
                    kind: LineKind::InlineAsm,
                    ext_kind: ExtKind::None,
                    trim_start: infos[i].trim_start,
                    has_indirect_mem: true,
                    rbp_offset: RBP_OFFSET_NONE,
                    reg_refs: u16::MAX,
                    pinned: true,
                };
            }
        }
        (store, infos)
    }

    /// Run `propagate_register_copies` to a local fixpoint (the pipeline
    /// drives it repeatedly; two iterations expose chain-then-consume).
    fn propagate(asm: &str) -> String {
        let (mut store, mut infos) = build_pinned(asm);
        for _ in 0..2 {
            propagate_register_copies(&mut store, &mut infos);
        }
        (0..store.len())
            .filter(|&i| !infos[i].is_nop())
            .map(|i| store.get(i).trim().to_string())
            .collect::<Vec<_>>()
            .join("\n")
    }

    // NOTE on the choice of copy sources in these tests: the reg-reg copy
    // parsers reject the frame families (%rsp/%rbp) by design, so every
    // fixture uses %ebx/%edx-class sources.

    #[test]
    fn narrow_consumer_is_retargeted_to_the_movl_origin() {
        // N2: `cmpl` reads only the low 32 bits of %eax, which the movl
        // identity pins to %ebx — the read is retargeted.
        let out = propagate(concat!(
            "f:\n",
            ".cfi_startproc\n",
            "    movl %ebx, %eax\n",
            "    cmpl %eax, %esi\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("cmpl %ebx, %esi"), "{out}");
    }

    #[test]
    fn byte_and_word_reads_are_retargeted() {
        let out = propagate(concat!(
            "f:\n",
            ".cfi_startproc\n",
            "    movl %ebx, %eax\n",
            "    testb %al, %al\n",
            "    movl %edx, %ecx\n",
            "    testw %cx, %cx\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("testb %bl, %bl"), "{out}");
        assert!(out.contains("testw %dx, %dx"), "{out}");
    }

    #[test]
    fn movl_store_source_is_retargeted() {
        // N3: the store reads exactly the low 32 bits the identity covers.
        let out = propagate(concat!(
            "f:\n",
            ".cfi_startproc\n",
            "    movl %ebx, %eax\n",
            "    movl %eax, (%r8,%r9,4)\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("movl %ebx, (%r8,%r9,4)"), "{out}");
    }

    #[test]
    fn movq_store_source_uses_the_full_identity() {
        let out = propagate(concat!(
            "f:\n",
            ".cfi_startproc\n",
            "    movq %rdx, %rax\n",
            "    movq %rax, (%r8)\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("movq %rdx, (%r8)"), "{out}");
    }

    #[test]
    fn movq_store_refuses_a_low32_only_identity() {
        // `movl` zeroes the upper half — the 64-bit store must keep %rax.
        let out = propagate(concat!(
            "f:\n",
            ".cfi_startproc\n",
            "    movl %ebx, %eax\n",
            "    movq %rax, (%r8)\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("movq %rax, (%r8)"), "{out}");
        assert!(!out.contains("movq %rbx"), "{out}");
    }

    #[test]
    fn sixty_four_bit_reader_keeps_the_copy() {
        // N2(a): `addq` reads bits 32..=63, outside the movl identity.
        let out = propagate(concat!(
            "f:\n",
            ".cfi_startproc\n",
            "    movl %ebx, %eax\n",
            "    addq %rax, %r9\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("addq %rax, %r9"), "{out}");
    }

    #[test]
    fn lock_cmpxchg_between_copy_and_consumer_keeps_the_copy() {
        // F7 (audit of PR #607): `cmpxchgq` IMPLICITLY REWRITES %rax when
        // the comparison fails (the accumulator is reloaded from memory),
        // and the `lock ` prefix must not hide that from the implicit-write
        // oracle.  A 32-bit consumer after the lock window cannot be
        // retargeted to the copy's source: it reads the CURRENT %eax,
        // which the failing cmpxchg just replaced.  The identity must be
        // retired at the implicit write, not carried across it.  This is
        // now reachable through NARROW consumers (the new low-32
        // propagation), far more common than the 64-bit readers above —
        // exactly the shape a future edit to `implicit_write_refs` would
        // silently break.
        let out = propagate(concat!(
            "f:\n",
            ".cfi_startproc\n",
            "    movl %ebx, %eax\n",
            "    lock cmpxchgq %rbx, (%rdi)\n",
            "    cmpl %eax, %esi\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("cmpl %eax, %esi"), "{out}");
        assert!(!out.contains("cmpl %ebx, %esi"), "{out}");
    }

    #[test]
    fn plain_cmpxchg_between_copy_and_consumer_keeps_the_copy() {
        // The un-prefixed spelling of the same contract (the implicit
        // RAX write is identical without `lock`).
        let out = propagate(concat!(
            "f:\n",
            ".cfi_startproc\n",
            "    movl %ebx, %eax\n",
            "    cmpxchgq %rbx, (%rdi)\n",
            "    cmpl %eax, %esi\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("cmpl %eax, %esi"), "{out}");
        assert!(!out.contains("cmpl %ebx, %esi"), "{out}");
    }

    #[test]
    fn consumer_before_the_implicit_write_still_retargets() {
        // Positive control: a 32-bit consumer BEFORE the lock window reads
        // the pre-window %eax, which the movl identity still describes, so
        // the retarget to %ebx is correct and must fire.
        let out = propagate(concat!(
            "f:\n",
            ".cfi_startproc\n",
            "    movl %ebx, %eax\n",
            "    cmpl %eax, %esi\n",
            "    lock cmpxchgq %rbx, (%rdi)\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("cmpl %ebx, %esi"), "{out}");
        assert!(!out.contains("cmpl %eax, %esi"), "{out}");
    }

    #[test]
    fn address_use_keeps_the_copy() {
        // N2(a)/(d): a base/index reference is a 64-bit read.
        let out = propagate(concat!(
            "f:\n",
            ".cfi_startproc\n",
            "    movl %ebx, %eax\n",
            "    movl (%rax), %esi\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("movl (%rax), %esi"), "{out}");
    }

    #[test]
    fn dest_write_keeps_the_copy() {
        // N2(b): the result must land in %eax.
        let out = propagate(concat!(
            "f:\n",
            ".cfi_startproc\n",
            "    movl %ebx, %eax\n",
            "    subl %edx, %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("subl %edx, %eax"), "{out}");
    }

    #[test]
    fn high_byte_line_keeps_the_copy() {
        // N2(a): %ah stays unrewritten; a mixed line is refused outright.
        let out = propagate(concat!(
            "f:\n",
            ".cfi_startproc\n",
            "    movl %ebx, %eax\n",
            "    addb %ah, %cl\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("addb %ah, %cl"), "{out}");
        assert!(!out.contains("%bh"), "{out}");
    }

    #[test]
    fn shift_count_in_cl_is_never_retargeted() {
        // The count of a variable shift is %cl by architecture; the
        // 64-bit arm refuses %rcx propagation there and the narrow arm
        // must not sneak the byte-width occurrence through.
        let out = propagate(concat!(
            "f:\n",
            ".cfi_startproc\n",
            "    movq %rdi, %rcx\n",
            "    shlq %cl, %r8\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("shlq %cl, %r8"), "{out}");
        assert!(!out.contains("%dil"), "{out}");
    }

    #[test]
    fn setcc_destination_is_never_retargeted() {
        // SetCC writes the family at 8-bit width — N2(b) refuses it.
        let out = propagate(concat!(
            "f:\n",
            ".cfi_startproc\n",
            "    movl %ebx, %eax\n",
            "    sete %al\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("sete %al"), "{out}");
    }

    #[test]
    fn store_relay_dies_end_to_end_when_the_dest_is_redefined() {
        // The fannkuch store shape through the full peephole pipeline:
        // the store source is retargeted, then the now-dead relay is
        // deleted (%eax is fully redefined before any read).
        let out = super::super::super::peephole_optimize(
            concat!(
                "f:\n",
                ".cfi_startproc\n",
                "    movl %ebx, %eax\n",
                "    movl %eax, (%r8,%r9,4)\n",
                "    movl %edx, %eax\n",
                "    ret\n",
                ".cfi_endproc\n",
            )
            .to_string(),
        );
        assert!(out.contains("movl %ebx, (%r8,%r9,4)"), "{out}");
        assert!(!out.contains("movl %ebx, %eax"), "{out}");
        assert!(out.contains("movl %edx, %eax"), "{out}");
    }

    #[test]
    fn imul_three_operand_overwrite_kills_the_relay_end_to_end() {
        // The magic-division staple: `imull $imm, %src, %r9d` writes %r9d
        // without reading it, so the staging relay in front of it dies.
        // Regression pin for the phantom-RMW bug that kept the glibc_strstr
        // `movl %eax, %r9d` alive one instruction per hot iteration.
        let out = super::super::super::peephole_optimize(
            concat!(
                "f:\n",
                ".cfi_startproc\n",
                "    movl %eax, %r9d\n",
                "    imull $5, %ebx, %r9d\n",
                "    movq %r9, %rbx\n",
                "    ret\n",
                ".cfi_endproc\n",
            )
            .to_string(),
        );
        assert!(!out.contains("movl %eax, %r9d"), "{out}");
        assert!(out.contains("imull $5, %ebx, %r9d"), "{out}");
        assert!(out.contains("movq %r9, %rbx"), "{out}");
    }

    #[test]
    fn imul_two_operand_still_reads_its_destination() {
        // The two-operand form genuinely reads the destination: the relay
        // must survive.
        let out = super::super::super::peephole_optimize(
            concat!(
                "f:\n",
                ".cfi_startproc\n",
                "    movl %eax, %r9d\n",
                "    imull %ebx, %r9d\n",
                "    movq %r9, %rbx\n",
                "    ret\n",
                ".cfi_endproc\n",
            )
            .to_string(),
        );
        assert!(out.contains("movl %eax, %r9d"), "{out}");
    }

    #[test]
    fn movl_store_source_is_retargeted_from_a_frame_family() {
        // fannkuch's swap-loop shape: the RA homes perm[i] in %ebp and the
        // store source is retargeted through the low-32 identity.
        let out = propagate(concat!(
            "f:\n",
            ".cfi_startproc\n",
            "    movl %ebp, %eax\n",
            "    movl %eax, (%r8,%r9,4)\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("movl %ebp, (%r8,%r9,4)"), "{out}");
    }

    #[test]
    fn frame_family_relay_dies_end_to_end_when_the_dest_is_redefined() {
        let out = super::super::super::peephole_optimize(
            concat!(
                "f:\n",
                ".cfi_startproc\n",
                "    movl %ebp, %eax\n",
                "    movl %eax, (%r8,%r9,4)\n",
                "    movl %edx, %eax\n",
                "    ret\n",
                ".cfi_endproc\n",
            )
            .to_string(),
        );
        assert!(out.contains("movl %ebp, (%r8,%r9,4)"), "{out}");
        assert!(!out.contains("movl %ebp, %eax"), "{out}");
        assert!(out.contains("movl %edx, %eax"), "{out}");
    }

    #[test]
    fn copy_state_does_not_cross_inline_asm() {
        // `movq %rax, %rcx` copies; the asm block redefines %rax (and %rcx)
        // opaquely; the consumer must keep reading %rcx, never be renamed to
        // %rax.
        let asm = concat!(
            "f:\n",
            ".cfi_startproc\n",
            "    movq %rax, %rcx\n",
            "#APP\n",
            "    movq %rdx, %rcx\n",
            "#NO_APP\n",
            "    addq $1, %rcx\n",
            "    ret\n",
            ".cfi_endproc\n",
        );
        let (mut store, mut infos) = build_pinned(asm);
        propagate_register_copies(&mut store, &mut infos);
        assert!(
            !(0..store.len()).any(|i| store.get(i).contains("addq $1, %rax")),
            "consumer must not be renamed across the asm region"
        );
    }

    #[test]
    fn inline_asm_lines_are_never_rewritten() {
        // A tracked copy whose source the template textually names must not
        // leak into the user's asm bytes.
        let asm = concat!(
            "f:\n",
            ".cfi_startproc\n",
            "    movq %rax, %rcx\n",
            "#APP\n",
            "    addq $1, %rax\n",
            "#NO_APP\n",
            "    ret\n",
            ".cfi_endproc\n",
        );
        let (mut store, mut infos) = build_pinned(asm);
        propagate_register_copies(&mut store, &mut infos);
        assert!(
            (0..store.len()).any(|i| store.get(i).trim() == "addq $1, %rax"),
            "user asm bytes are immutable"
        );
    }

    #[cfg(test)]
    mod redteam_repro_cp {
        //! Red-team repros for narrow copy propagation (Agent-B audit).
        use super::super::super::peephole_optimize;

        fn run(asm: &str) -> String {
            peephole_optimize(asm.to_string())
        }

        #[test]
        fn narrow_prop_refuses_high_byte_lines_needing_rex() {
            // Introducing %bpl (REX-requiring) next to %ah (REX-forbidden)
            // produces unencodable assembly. The narrow arm must refuse.
            let out = run(concat!(
                "f:\n",
                ".cfi_startproc\n",
                "    movl %ebp, %ecx\n",
                "    cmpb %cl, %ah\n",
                "    sete %al\n",
                "    ret\n",
                ".cfi_endproc\n",
            ));
            assert!(
                !out.contains("%bpl"),
                "must not introduce a REX byte next to %ah: {out}"
            );
        }

        #[test]
        fn narrow_prop_preserves_utf8_comment_bytes() {
            // Non-ASCII comment bytes must round-trip byte-identically.
            let out = run(concat!(
                "f:\n",
                ".cfi_startproc\n",
                "    movl %ebx, %eax\n",
                "    cmpl %eax, %esi # caf\u{e9} na\u{ef}ve \u{3b1}\u{3b2}\n",
                "    ret\n",
                ".cfi_endproc\n",
            ));
            assert!(
                out.contains("caf\u{e9} na\u{ef}ve \u{3b1}\u{3b2}"),
                "UTF-8 comment bytes must round-trip: {out}"
            );
        }
    }
}
