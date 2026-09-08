//! Identical block merging pass.
//!
//! Detects basic blocks with identical instruction sequences and the same
//! jump target, then merges them by redirecting all branches to duplicates
//! to a single canonical copy. The duplicate blocks are eliminated.
//!
//! This primarily targets phi elimination trampoline blocks in large switch
//! statements (e.g., sqlite3VdbeExec), where many case blocks produce
//! identical phi copy sequences.
//!
//! SOUNDNESS: two blocks with identical TEXT are NOT automatically
//! interchangeable — they may be reached with different live register/stack
//! values from different predecessors. Merging them would give one predecessor
//! the other path's inputs. We therefore require that two blocks share the
//! EXACT SAME SET of predecessor blocks before merging. Identical predecessors
//! guarantee identical live-in state, so the merge is semantics-preserving.
//!
//! KNOWN LIMITATION (pred visibility): the predecessor scan only attributes
//! edges whose source lies inside a tracked `.LBB` block. Code in function
//! entry regions (before the first `.LBB` label) and after non-`.LBB` local
//! labels (`.Lmset_loop_*` memset islands, `.Lvc*` pools, …) is untracked, so
//! jumps from there contribute NO predecessor. Two blocks whose in-edges are
//! all invisible therefore compare equal on EMPTY predecessor sets and can
//! still merge. That is the pass's working mode for entry-adjacent dispatch
//! (see test non_jump_table_blocks_still_merge) and is only sound because such
//! blocks' register state at the jump sites usually coincides; a general fix
//! needs entry-region/island tracking in the block scan. Alignment directives
//! between blocks no longer erase fall-through edges (see is_inert_directive),
//! which was the one case where the erasure was a pure analysis bug rather
//! than an inherent limitation.

use super::super::types::*;
use crate::common::fx_hash::{FxHashMap, FxHashSet};

// ── Soundness helpers ────────────────────────────────────────────────────
//
// Identical TEXT + identical predecessor LABELS is NOT sufficient to prove two
// blocks are interchangeable, because live-in *state* can still differ:
//   1. EFLAGS: a block reached via `je B` (fall-through is B2) carries the
//      flags set by the predecessor's last compare. Two blocks with the same
//      predecessor labels may be entered with different incoming flags if the
//      flag-producing edges differ. Merging such blocks and reading the wrong
//      flags is a miscompile. We therefore require that neither block is
//      *flag-dependent at entry* (i.e. it establishes its own flags before any
//      flag-consuming instruction, so incoming EFLAGS are irrelevant).
//   2. Registers: identical predecessor labels only guarantee identical live-in
//      register VALUES if each predecessor is a straight-line basic block. If a
//      "predecessor" block contains internal non-.LBB labels (so it is not a
//      single basic block), its register state on the edge to one target can
//      differ from its state on the edge to the other. We therefore require the
//      merged blocks and all their predecessors to be "clean" (single-entry,
//      no internal labels), so the register values at entry are path-independent.

/// Directives that are inert for control-flow and register-state analysis:
/// they emit no semantically relevant instructions into the text stream
/// (`.p2align`/`.balign`/`.align` emit NOP padding; `.loc`/`.file` emit
/// debug metadata). A block boundary scan must treat them as transparent,
/// exactly like NOP lines — breaking a block at such a directive silently
/// ERASES the fall-through edge between the two blocks around it, and the
/// predecessor analysis then believes the following block has no
/// predecessors at all.
///
/// Observed miscompile (vectorize_int_map_lanes, -O3 -march=x86-64-v3): the
/// loop-header alignment pass emits `.p2align 4` between a loop guard's
/// conditional branch and the preheader label; identical_blocks lost the
/// fall-through edge, saw 25 byte-identical memset preheaders with EMPTY
/// predecessor sets, merged them, and every merged-away preheader NOPed its
/// `rdi`/`rcx`/`ymm0` setup straight into the store loop → SIGSEGV.
///
/// `.cfi_*` is deliberately NOT in this set: unwind directives are
/// function-scoped metadata that must survive block merging verbatim, and
/// no lccc codegen path emits them between `.LBB` blocks. Keeping them as
/// block terminators preserves the pre-existing (and tested) behavior.
fn is_inert_directive(trimmed: &str) -> bool {
    trimmed.starts_with(".loc")
        || trimmed.starts_with(".file")
        || trimmed.starts_with(".p2align")
        || trimmed.starts_with(".balign")
        || trimmed.starts_with(".align")
}

/// A line inside a block range that carries no block semantics: a NOP or an
/// inert directive. Every content consumer below (boundary scan, terminator
/// scan, hash, text identity, cleanliness) skips exactly these, so a block's
/// identity never depends on its padding.
fn line_is_transparent(store: &LineStore, infos: &[LineInfo], j: usize) -> bool {
    if infos[j].is_nop() {
        return true;
    }
    infos[j].kind == LineKind::Directive && is_inert_directive(infos[j].trimmed(store.get(j)))
}

/// Does this instruction set the EFLAGS (condition flags)?
fn sets_flags(store: &LineStore, infos: &[LineInfo], i: usize) -> bool {
    match infos[i].kind {
        LineKind::Cmp => true, // cmp/test/ucomis*
        LineKind::Push { .. }
        | LineKind::Pop { .. }
        | LineKind::Label
        | LineKind::Jmp
        | LineKind::CondJmp
        | LineKind::JmpIndirect
        | LineKind::Call
        | LineKind::Ret
        | LineKind::Directive
        | LineKind::SelfMove
        | LineKind::SetCC { .. } => false,
        _ => {
            let t = infos[i].trimmed(store.get(i));
            let mnem = t.split_whitespace().next().unwrap_or("");
            matches!(
                mnem,
                "addl"
                    | "addq"
                    | "addw"
                    | "addb"
                    | "subl"
                    | "subq"
                    | "subw"
                    | "subb"
                    | "andl"
                    | "andq"
                    | "andw"
                    | "andb"
                    | "orl"
                    | "orq"
                    | "orw"
                    | "orb"
                    | "xorl"
                    | "xorq"
                    | "xorw"
                    | "xorb"
                    | "imull"
                    | "imulq"
                    | "imulw"
                    | "mull"
                    | "mulq"
                    | "mulw"
                    | "incl"
                    | "incq"
                    | "incw"
                    | "decl"
                    | "decq"
                    | "decw"
                    | "negl"
                    | "negq"
                    | "negw"
                    | "notl"
                    | "notq"
                    | "notw"
                    | "shll"
                    | "shlq"
                    | "shlw"
                    | "shrl"
                    | "shrq"
                    | "shrw"
                    | "sarl"
                    | "sarq"
                    | "sarw"
                    | "roll"
                    | "rolq"
                    | "rolw"
                    | "rorl"
                    | "rorq"
                    | "rorw"
                    | "testl"
                    | "testq"
                    | "testw"
                    | "testb"
                    | "cmpl"
                    | "cmpq"
                    | "cmpw"
                    | "cmpb"
                    | "btl"
                    | "btq"
                    | "btw"
                    | "btb"
            )
        }
    }
}

/// Does this instruction consume (read) the EFLAGS?
fn reads_flags(store: &LineStore, infos: &[LineInfo], i: usize) -> bool {
    match infos[i].kind {
        LineKind::CondJmp | LineKind::SetCC { .. } => true,
        _ => {
            let t = infos[i].trimmed(store.get(i));
            let mnem = t.split_whitespace().next().unwrap_or("");
            mnem.starts_with("cmov")
                || mnem.starts_with("set")
                || mnem == "adcl"
                || mnem == "adcq"
                || mnem == "adcw"
                || mnem == "sbbl"
                || mnem == "sbbq"
                || mnem == "sbbw"
        }
    }
}

/// True if the block reads EFLAGS *before* establishing them itself, i.e. it
/// depends on the incoming (predecessor-set) flags.
fn block_is_flag_dependent(
    store: &LineStore,
    infos: &[LineInfo],
    start: usize,
    end: usize,
) -> bool {
    let mut flags_set = false;
    for j in (start + 1)..end {
        if infos[j].is_nop() {
            continue;
        }
        if sets_flags(store, infos, j) {
            flags_set = true;
        } else if reads_flags(store, infos, j) && !flags_set {
            return true;
        }
    }
    false
}

/// True if the block is a clean, single-entry straight-line basic block: it has
/// no internal label (other than its own entry label) and no directives that
/// would split it, so its register state at exit is path-independent.
fn block_is_clean(store: &LineStore, infos: &[LineInfo], start: usize, end: usize) -> bool {
    for j in (start + 1)..end {
        // Transparent lines (NOPs, inert directives) carry no block
        // semantics: they cannot branch or define a label, so they do not
        // affect the single-entry straight-line property.
        if line_is_transparent(store, infos, j) {
            continue;
        }
        match infos[j].kind {
            LineKind::Label | LineKind::Directive => return false,
            _ => {}
        }
    }
    true
}

/// Whether the block contains any flag-consuming instruction at all (used to
/// decide if EFLAGS matter for the merge).
fn block_has_flag_use(store: &LineStore, infos: &[LineInfo], start: usize, end: usize) -> bool {
    for j in (start + 1)..end {
        if infos[j].is_nop() {
            continue;
        }
        if reads_flags(store, infos, j) || sets_flags(store, infos, j) {
            return true;
        }
    }
    false
}

/// Merge identical basic blocks.
/// Returns true if any blocks were merged.
pub(super) fn merge_identical_blocks(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    let len = store.len();
    if len < 10 {
        return false;
    }

    // Phase 1: Find all labels and their block boundaries.
    let mut blocks: Vec<(usize, usize, String, u32)> = Vec::new(); // (start_line, end_line, label_name, func_id)
    let mut block_of_line: Vec<isize> = vec![-1; len]; // line -> block index (or -1)
    let mut current_func_id: u32 = 0;
    let mut i = 0;
    while i < len {
        if infos[i].kind == LineKind::Directive {
            let line = infos[i].trimmed(store.get(i));
            if line == ".cfi_startproc" {
                current_func_id += 1;
            }
        }
        if infos[i].kind == LineKind::Label {
            let label = infos[i].trimmed(store.get(i));
            if let Some(label_name) = label.strip_suffix(':') {
                if label_name.starts_with(".LBB") {
                    let start = i;
                    let mut end = i + 1;
                    while end < len {
                        if infos[end].is_nop() {
                            end += 1;
                            continue;
                        }
                        if infos[end].kind == LineKind::Label {
                            break;
                        }
                        if infos[end].kind == LineKind::Directive {
                            // .loc/.file debug directives are INERT text markers
                            // (parsed but carry no semantics): they must not
                            // fragment a basic block, or the predecessor /
                            // fall-through analysis below misfires and can
                            // merge (then delete) epilogue blocks it shouldn't
                            // (gzip -g -O2: treat_file lost its epilogue+ret
                            // and fell through into create_outfile -> SIGSEGV).
                            // The same is true for alignment directives (see
                            // is_inert_directive): breaking the block there
                            // erases the fall-through edge to the next block.
                            let dl = infos[end].trimmed(store.get(end));
                            if is_inert_directive(dl) {
                                end += 1;
                                continue;
                            }
                            break;
                        }
                        end += 1;
                    }
                    let bidx = blocks.len() as isize;
                    for k in start..end {
                        block_of_line[k] = bidx;
                    }
                    blocks.push((start, end, label_name.to_string(), current_func_id));
                    i = end;
                    continue;
                } else if !label_name.starts_with(".L") {
                    // Global (non-local) label: a new function entry point.
                    // Function identity must NOT rely on .cfi_startproc —
                    // the kernel builds with -fno-asynchronous-unwind-tables
                    // and emits no CFI directives at all, which previously
                    // left every block in the whole object sharing
                    // func_id = 0 and let identical return blocks merge
                    // ACROSS FUNCTIONS: update_srbds_msr's early-return
                    // block was redirected into mds_apply_mitigation in
                    // .init.text (modpost section mismatch + a wrong
                    // cross-section jump at runtime).
                    current_func_id += 1;
                }
            }
        }
        i += 1;
    }

    if blocks.len() < 2 {
        return false;
    }

    // Phase 1.5: Compute predecessor set for each block.
    // preds[label] = sorted set of predecessor block labels.
    // A block X is a predecessor of block B if:
    //   - X ends with an (un)conditional jump targeting B, OR
    //   - X falls through into B (X.end == B.start and X's last non-NOP
    //     instruction is not an unconditional jmp / ret).
    let mut preds: FxHashMap<String, FxHashSet<String>> = FxHashMap::default();
    // Determine the block-terminating instruction per block. Inert
    // directives are skipped: a `ret` followed by `.cfi_endproc` (or an
    // aligned block's trailing `.p2align`) must still classify as a Ret /
    // real terminator, not as a fall-through-capable Directive line.
    let mut block_terminator: Vec<LineKind> = vec![LineKind::Empty; blocks.len()];
    for (bidx, &(start, end, _, _)) in blocks.iter().enumerate() {
        for k in (start + 1..end).rev() {
            if line_is_transparent(store, infos, k) {
                continue;
            }
            block_terminator[bidx] = infos[k].kind;
            break;
        }
    }
    // Scan all jumps/cond-jumps; add source block to target's preds.
    for i in 0..len {
        if infos[i].is_nop() {
            continue;
        }
        if matches!(infos[i].kind, LineKind::Jmp | LineKind::CondJmp) {
            let trimmed = infos[i].trimmed(store.get(i));
            if let Some(space_pos) = trimmed.find(' ') {
                let target = trimmed[space_pos + 1..]
                    .trim()
                    .trim_end_matches(':')
                    .to_string();
                if let Some(&src_bidx) = block_of_line.get(i) {
                    if src_bidx >= 0 {
                        let src_label = &blocks[src_bidx as usize].2;
                        preds
                            .entry(target.clone())
                            .or_default()
                            .insert(src_label.clone());
                    }
                }
            }
        }
    }
    // Fall-through edges: block X (end == B.start, terminator not an
    // unconditional jmp/ret) falls through into block B.
    for bi in 0..blocks.len() {
        let b_start = blocks[bi].0;
        let b_label = blocks[bi].2.clone();
        // Only one block can end at b_start (the immediately preceding one), but
        // check all blocks for safety.
        for xi in 0..blocks.len() {
            if xi == bi {
                continue;
            }
            if blocks[xi].1 == b_start {
                // X immediately precedes B.
                match block_terminator[xi] {
                    LineKind::Jmp | LineKind::JmpIndirect | LineKind::Ret => {
                        // X jumps away/returns — does NOT fall through to B.
                        // (A Jmp to B is already handled above as a branch edge.)
                    }
                    _ => {
                        // X falls through into B (CondJmp or plain fall-through).
                        let x_label = blocks[xi].2.clone();
                        preds.entry(b_label.clone()).or_default().insert(x_label);
                    }
                }
            }
        }
    }

    // Phase 2: Hash each block's content.
    let mut block_hashes: FxHashMap<(u64, u32), Vec<usize>> = FxHashMap::default();
    for (idx, &(start, end, _, func_id)) in blocks.iter().enumerate() {
        let mut hasher = 0u64;
        let mut instr_count = 0u32;
        for j in (start + 1)..end {
            if line_is_transparent(store, infos, j) {
                continue;
            }
            let line = store.get(j);
            for byte in line.bytes() {
                hasher ^= byte as u64;
                hasher = hasher.wrapping_mul(0x100000001b3);
            }
            instr_count += 1;
        }
        if instr_count >= 4 {
            hasher ^= instr_count as u64;
        }
        // The push must NOT be nested inside the `instr_count >= 4` refinement:
        // blocks with fewer than 4 instructions would never be registered and
        // could never merge (e.g. 2-instruction `call; ret` trampolines).
        block_hashes.entry((hasher, func_id)).or_default().push(idx);
    }

    // Build label → block index map, for predecessor-cleanliness checks.
    let mut label_to_bidx: FxHashMap<String, usize> = FxHashMap::default();
    for (idx, b) in blocks.iter().enumerate() {
        label_to_bidx.insert(b.2.clone(), idx);
    }

    // SOUNDNESS: collect labels referenced from JUMP TABLES
    // (`.long .LBBxxx - .Ljt_n` entries for i686 REL tables and
    // `.quad .LBBxxx` entries for x86-64 absolute tables, both in .rodata).
    // Redirecting/eliminating a block whose label appears in a jump table
    // leaves the jump-table entry pointing at a removed (NOP) block — the
    // indirect jump then lands on garbage (or, for `.quad` entries, the
    // relocation is emitted against a label that no longer exists, and the
    // ELF writer silently resolves it to symbol index 0, which the linker
    // reports as an undefined `<section 0 ''>`). The pass cannot rewrite
    // those entries, so any block that is a jump-table target must be
    // excluded from merging entirely. (Linux 6.18 syscall_64.o: 104
    // `__x64_sys_ni_syscall` switch cases are byte-identical; merging them
    // left 98 `.quad` entries dangling and broke the vmlinux link.)
    let mut jump_table_targets: FxHashSet<String> = FxHashSet::default();
    for i in 0..len {
        if infos[i].is_nop() {
            continue;
        }
        let line = infos[i].trimmed(store.get(i));
        if line.starts_with(".long ") || line.starts_with(".quad ") {
            // Extract every `.LBB...` label mentioned in the entry.
            // The token is ".LBB" followed by the name characters; start scanning
            // the name AFTER the ".LBB" prefix so `j` always advances past pos.
            let mut rest = &line[..];
            while let Some(pos) = rest.find(".LBB") {
                let name_start = pos + ".LBB".len();
                let mut j = name_start;
                while j < rest.len()
                    && (rest.as_bytes()[j].is_ascii_alphanumeric() || rest.as_bytes()[j] == b'_')
                {
                    j += 1;
                }
                let tok = rest[pos..j].to_string();
                jump_table_targets.insert(tok);
                rest = &rest[j..];
            }
        }
    }

    // Compute the fall-through successor label for each block.
    // A block FALLS THROUGH to the immediately-following block unless its
    // terminator is an unconditional jump / indirect jump / return. The
    // fall-through successor is NOT part of the block's text, so two identical
    // blocks that fall through to DIFFERENT successors must not be merged
    // (redirecting branches would change the fall-through destination).
    // blocks are in source order, so the fall-through successor of block `bi`
    // is the next block whose start equals block `bi`'s end (if it doesn't end
    // in an unconditional jump/ret).
    let mut fallthrough: Vec<Option<String>> = vec![None; blocks.len()];
    for bi in 0..blocks.len() {
        let term = block_terminator[bi];
        let has_explicit_exit =
            matches!(term, LineKind::Jmp | LineKind::JmpIndirect | LineKind::Ret);
        if !has_explicit_exit {
            // Find the block that starts exactly where this block ends.
            let end_pos = blocks[bi].1;
            for x in 0..blocks.len() {
                if x != bi && blocks[x].0 == end_pos {
                    fallthrough[bi] = Some(blocks[x].2.clone());
                    break;
                }
            }
        }
    }

    // Precompute per-block soundness flags ONCE (not per candidate pair).
    // flag_dep[i]: block reads incoming EFLAGS before establishing its own.
    // clean[i]:    block is a single-entry straight-line basic block.
    // preds_clean[i]: all predecessor blocks are clean basic blocks.
    let nblocks = blocks.len();
    let mut flag_dep = vec![false; nblocks];
    let mut clean = vec![false; nblocks];
    let mut preds_clean = vec![false; nblocks];
    for bi in 0..nblocks {
        let (bs, be, ref bl, _) = blocks[bi];
        flag_dep[bi] = block_is_flag_dependent(store, infos, bs, be);
        clean[bi] = block_is_clean(store, infos, bs, be);
        let mut all_clean = true;
        if let Some(pset) = preds.get(bl) {
            for pl in pset {
                if let Some(&pidx) = label_to_bidx.get(pl) {
                    let (ps, pe, _, _) = blocks[pidx];
                    if !block_is_clean(store, infos, ps, pe) {
                        all_clean = false;
                        break;
                    }
                }
            }
        }
        preds_clean[bi] = all_clean;
    }

    // Phase 3: Merge only truly-identical blocks with IDENTICAL predecessor sets.
    let mut changed = false;
    let mut redirects: FxHashMap<String, String> = FxHashMap::default(); // old_label → canonical_label

    for ((_hash, _func_id), group) in &block_hashes {
        if group.len() < 2 {
            continue;
        }
        let canonical_idx = group[0];
        let (can_start, can_end, _, _) = &blocks[canonical_idx];
        let canonical_label = blocks[canonical_idx].2.clone();
        let canonical_preds = preds.get(&canonical_label).cloned().unwrap_or_default();

        let canonical_instrs: Vec<String> = ((*can_start + 1)..*can_end)
            .filter(|&j| !line_is_transparent(store, infos, j))
            .map(|j| store.get(j).to_string())
            .collect();

        // SOUNDNESS: skip if the canonical block is flag-dependent, not
        // clean, has a non-clean predecessor, or is a jump-table target.
        if flag_dep[canonical_idx]
            || !clean[canonical_idx]
            || !preds_clean[canonical_idx]
            || jump_table_targets.contains(&canonical_label)
        {
            continue;
        }

        for &other_idx in &group[1..] {
            let (other_start, other_end, ref other_label, _) = blocks[other_idx];
            let other_preds = preds.get(other_label).cloned().unwrap_or_default();
            // SOUNDNESS: require identical predecessor sets.
            if canonical_preds != other_preds {
                continue;
            }
            // SOUNDNESS: never merge/eliminate a jump-table target, a
            // flag-dependent block, a non-clean block, or one with a non-clean
            // predecessor.
            if jump_table_targets.contains(other_label) {
                continue;
            }
            if flag_dep[other_idx] || !clean[other_idx] || !preds_clean[other_idx] {
                continue;
            }

            let other_instrs: Vec<String> = ((other_start + 1)..other_end)
                .filter(|&j| !line_is_transparent(store, infos, j))
                .map(|j| store.get(j).to_string())
                .collect();

            if canonical_instrs == other_instrs {
                // SOUNDNESS: the fall-through successor is not part of the
                // block text; both blocks must fall through to the SAME next
                // block (or neither falls through), else redirecting changes
                // control flow after the merged block.
                if fallthrough[canonical_idx] != fallthrough[other_idx] {
                    continue;
                }

                redirects.insert(other_label.clone(), canonical_label.clone());
                for j in other_start..other_end {
                    if line_is_transparent(store, infos, j) {
                        // Inert directives survive the merge: a trailing
                        // `.p2align` pads the LIVE successor's label, and
                        // deleting it would silently defeat that block's
                        // alignment.
                        continue;
                    }
                    mark_nop(&mut infos[j]);
                    changed = true;
                }
            }
        }
    }

    if redirects.is_empty() {
        return false;
    }

    // Phase 4: Rewrite all branch targets that reference redirected labels.
    for i in 0..len {
        if infos[i].is_nop() {
            continue;
        }
        match infos[i].kind {
            LineKind::Jmp | LineKind::CondJmp => {
                let line = store.get(i).to_string();
                let trimmed = infos[i].trimmed(&line);
                if let Some(space_pos) = trimmed.find(' ') {
                    let target = trimmed[space_pos + 1..].trim();
                    if let Some(canonical) = redirects.get(target) {
                        let prefix = &trimmed[..space_pos + 1];
                        let new_line = format!("    {}{}", prefix, canonical);
                        replace_line(store, &mut infos[i], i, new_line);
                        changed = true;
                    }
                }
            }
            _ => {}
        }
    }

    changed
}

#[cfg(test)]
mod tests {
    use super::super::super::peephole_optimize;

    fn run(asm: &str) -> String {
        peephole_optimize(asm.to_string())
    }

    /// x86-64 absolute jump tables (`.quad .LBBn` in `.Ljt_0`) must protect
    /// their target blocks from identical-block merging, exactly like the
    /// i686 `.long .LBBn - .Ljt_n` form. Merging a `.quad` target leaves the
    /// jump-table relocation pointing at a removed label; the ELF writer then
    /// emits symbol index 0 and the link dies with `<section 0 ''>`.
    ///
    /// Mirrors Linux 6.18 `arch/x86/entry/syscall_64.c`: 104 `case nr:
    /// return __x64_sys_ni_syscall(regs);` blocks are byte-identical; merging
    /// them (with only `.long` tables protected) left 98 dangling `.quad`
    /// entries and broke the vmlinux link.
    #[test]
    fn quad_jump_table_targets_are_not_merged() {
        let out = run(concat!(
            "x64_sys_call:\n",
            ".cfi_startproc\n",
            "    cmpq $3, %rax\n",
            "    jae .LBB2\n",
            "    jmpq *.Ljt_0(,%rax,8)\n",
            ".LBB0:\n",
            "    call __x64_sys_ni_syscall\n",
            "    ret\n",
            ".LBB1:\n",
            "    call __x64_sys_ni_syscall\n",
            "    ret\n",
            ".LBB2:\n",
            "    ret\n",
            ".cfi_endproc\n",
            ".section .rodata\n",
            ".align 8\n",
            ".Ljt_0:\n",
            "    .quad .LBB0\n",
            "    .quad .LBB1\n",
        ));
        // Both jump-table targets must survive with their labels defined.
        assert!(
            out.contains(".LBB0:\n"),
            "case 0 block was merged away:\n{out}"
        );
        assert!(
            out.contains(".LBB1:\n"),
            "case 1 block was merged away:\n{out}"
        );
        assert!(out.contains(".quad .LBB0"), "{out}");
        assert!(out.contains(".quad .LBB1"), "{out}");
    }

    /// `jc`/`jnc` are GAS aliases for jb/jae and MUST classify as
    /// conditional jumps: the kernel's bit-test idiom branches with them,
    /// and a predecessor edge that goes missing lets identical_blocks merge
    /// a block that still has a live branch. kernel/events/core.c
    /// is_sb_event lost its return-false block exactly this way.
    #[test]
    fn jc_jnc_branch_targets_are_live_predecessors() {
        let out = run(concat!(
            "f:\n",
            "    jmp .LBB0\n",
            ".LBB1:\n",
            "    xorl %eax, %eax\n",
            "    ret\n",
            ".LBB0:\n",
            "    btq $8, %r9\n",
            "    jc .LBB2\n",
            ".LBB4:\n",
            "    btq $30, %r9\n",
            "    jnc .LBB3\n",
            ".LBB2:\n",
            "    movl $1, %eax\n",
            "    ret\n",
            ".LBB3:\n",
            "    xorl %eax, %eax\n",
            "    ret\n",
        ));
        // .LBB3 is the jnc target: it must survive even though .LBB1 has
        // identical text (their predecessor sets differ once jnc counts).
        assert!(
            out.contains(".LBB3:\n"),
            "jnc target block was merged away:\n{out}"
        );
        assert!(out.contains("jnc .LBB3"), "{out}");
        // And the genuinely unreferenced twin may still be canonicalised.
        assert!(out.matches("xorl %eax, %eax").count() >= 1, "{out}");
    }

    /// Two functions with byte-identical epilogue blocks must NEVER merge
    /// across the function boundary. The kernel builds with
    /// -fno-asynchronous-unwind-tables and emits no .cfi_startproc at all;
    /// when func_id grouping relied on CFI directives every block in the
    /// object shared func_id 0, and update_srbds_msr's early-return block
    /// was merged into mds_apply_mitigation (a cross-section branch from
    /// .text into .init.text: modpost mismatch + wrong jump at runtime).
    /// Function identity must come from global (non-.L) labels instead.
    #[test]
    fn identical_blocks_never_merge_across_functions_without_cfi() {
        let out = run(concat!(
            "update_srbds_msr:\n",
            "    cmpq $0, %rdi\n",
            "    je .LBB1\n",
            ".LBB0:\n",
            "    movq %rbp, %rsp\n",
            "    popq %rbp\n",
            "    ret\n",
            ".LBB1:\n",
            "    movq %rbp, %rsp\n",
            "    popq %rbp\n",
            "    ret\n",
            "mds_apply_mitigation:\n",
            "    cmpq $0, %rdi\n",
            "    je .LBB4\n",
            ".LBB3:\n",
            "    movq %rbp, %rsp\n",
            "    popq %rbp\n",
            "    ret\n",
            ".LBB4:\n",
            "    movq %rbp, %rsp\n",
            "    popq %rbp\n",
            "    ret\n",
        ));
        // Within each function the pair may merge (1 survivor per function),
        // but the functions' blocks must never merge into a single one.
        assert_eq!(out.matches("popq %rbp").count(), 2, "{out}");
        // And each function's own epilogue must stay reachable from its own
        // function: two disjoint "movq %rbp, %rsp" survivors, one per function.
        assert_eq!(out.matches("movq %rbp, %rsp").count(), 2, "{out}");
        // No branch in update_srbds_msr may target a block defined after the
        // mds_apply_mitigation label. (Cheap proxy: the label itself survives.)
        assert!(out.contains("mds_apply_mitigation:\n"), "{out}");
    }

    /// Control: blocks that are NOT jump-table targets still merge.
    #[test]
    fn non_jump_table_blocks_still_merge() {
        // Both .LBB2 and .LBB3 are reached from the same dispatcher block,
        // have identical text, and neither is a jump-table target — the pass
        // must still merge them (the .quad protection must not disable the
        // pass wholesale).
        let out = run(concat!(
            "f:\n",
            ".cfi_startproc\n",
            "    cmpq $1, %rdi\n",
            "    je .LBB2\n",
            "    jmp .LBB3\n",
            ".LBB2:\n",
            "    call ext_a\n",
            "    ret\n",
            ".LBB3:\n",
            "    call ext_a\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        // One canonical block survives; the duplicate is redirected.
        let defs = out.matches("call ext_a").count();
        assert_eq!(defs, 1, "expected 1 surviving call block:\n{out}");
    }

    /// Alignment directives between a fall-through predecessor and a block
    /// must not erase the fall-through edge from the predecessor analysis.
    /// This is the exact shape of the loop-header alignment miscompile
    /// (vectorize_int_map_lanes at -O3 -march=x86-64-v3), reduced to ONE
    /// function so func_id separation cannot mask it: `.p2align 4` between
    /// each guard block's branch and its preheader label made the
    /// byte-identical memset preheaders all show EMPTY predecessor sets;
    /// they merged, and the merged-away copies NOPed their rdi/rcx/ymm0
    /// setup straight into the store loop → SIGSEGV. Boundary transparency
    /// (is_inert_directive) records the fall-through edge through the
    /// `.p2align`, so the preheaders' predecessor sets differ ({.LBB1} vs
    /// {.LBB5}) and no merge may happen.
    #[test]
    fn identical_blocks_p2align_between_blocks_preserves_fallthrough() {
        let out = run(concat!(
            "main:\n",
            ".cfi_startproc\n",
            "    xorl %r15d, %r15d\n",
            ".LBB1:\n",
            "    cmpq $2, %rdi\n",
            "    jae .LBB5\n",
            "    .p2align 4\n",
            ".LBB2:\n",
            "    movq %rbx, %rdi\n",
            "    movl $0xa5a5a5a5, %eax\n",
            "    vmovd %eax, %xmm0\n",
            "    vpbroadcastd %xmm0, %ymm0\n",
            "    movq $32, %rcx\n",
            ".Lmset_loop_0:\n",
            "    vmovdqu %ymm0, (%rdi)\n",
            "    addq $64, %rdi\n",
            "    decq %rcx\n",
            "    jne .Lmset_loop_0\n",
            "    ret\n",
            ".LBB5:\n",
            "    cmpq $3, %rsi\n",
            "    jae .LBB9\n",
            "    .p2align 4\n",
            ".LBB6:\n",
            "    movq %rbx, %rdi\n",
            "    movl $0xa5a5a5a5, %eax\n",
            "    vmovd %eax, %xmm0\n",
            "    vpbroadcastd %xmm0, %ymm0\n",
            "    movq $32, %rcx\n",
            ".Lmset_loop_1:\n",
            "    vmovdqu %ymm0, (%rdi)\n",
            "    addq $64, %rdi\n",
            "    decq %rcx\n",
            "    jne .Lmset_loop_1\n",
            "    ret\n",
            ".LBB9:\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        // Both preheader blocks must survive with their setup intact.
        assert_eq!(
            out.matches("vpbroadcastd %xmm0, %ymm0").count(),
            2,
            "a memset preheader was merged away through the .p2align gap:\n{out}"
        );
        assert_eq!(out.matches("movl $0xa5a5a5a5, %eax").count(), 2, "{out}");
        // The trip-count setup may be narrowed (movq→movl) by other passes;
        // only its survival matters here.
        assert!(
            out.matches("movl $32, %ecx").count() + out.matches("movq $32, %rcx").count() == 2,
            "{out}"
        );
    }
}
