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

/// True for the alignment directives (`.p2align`, `.align`, `.balign`, any
/// argument forms). These are LAYOUT annotations, not code: they neither
/// define registers nor flags nor split semantic basic blocks. The block
/// model must treat them as transparent — a `.p2align` between a guard's
/// `jae` and the loop-head label historically severed the fall-through
/// edge (the adjacency test `X.end == B.start` failed with the directive
/// line in between), leaving loop-preheader blocks with an EMPTY recorded
/// predecessor set. Two such "orphaned" blocks then satisfied the
/// identical-predecessors merge condition (empty == empty) and one was
/// deleted — while its guard still fell through into it at runtime. The
/// surviving loop then ran with uninitialized pointer/counter/accumulator
/// registers (reproducer: tests/regression/vectorize_int_map_lanes.c at
/// -O2 -march=x86-64-v3 with hot-loop alignment; SIGSEGV in `vmovdqu
/// %ymm0,(%rdi)` inside `fill`, 20/20 runs).
/// Pseudo predecessor label for a function's entry region (`func_id`).
/// A NUL byte can never appear in a real assembler label, so the pseudo
/// label cannot collide with any block label.
fn entry_pseudo_label(func_id: u32) -> String {
    format!("\u{0}fn{func_id}")
}

fn is_alignment_directive(trimmed: &str) -> bool {
    trimmed.starts_with(".p2align")
        || trimmed.starts_with(".align")
        || trimmed.starts_with(".balign")
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
        if infos[j].kind == LineKind::Directive
            && is_alignment_directive(infos[j].trimmed(store.get(j)))
        {
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
/// Alignment directives are transparent layout annotations (see
/// `is_alignment_directive`): padding inside a block defines nothing.
fn block_is_clean(store: &LineStore, infos: &[LineInfo], start: usize, end: usize) -> bool {
    for j in (start + 1)..end {
        if infos[j].is_nop() {
            continue;
        }
        if infos[j].kind == LineKind::Directive
            && is_alignment_directive(infos[j].trimmed(store.get(j)))
        {
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
        if infos[j].kind == LineKind::Directive
            && is_alignment_directive(infos[j].trimmed(store.get(j)))
        {
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
    // Per-line function id and the line where each function's region starts.
    // The lines before a function's first .LBB block (its entry region) are
    // not modeled as a block, so edges from there — explicit jumps AND the
    // positional fall-through — were invisible to the predecessor model.
    // Blocks entered only from the entry region then carried an EMPTY
    // predecessor set, and "empty == empty" let the merge delete one while
    // its entry still reached it at runtime. Both edge kinds are now
    // attributed to a per-function pseudo label (NUL-prefixed: a real label
    // can never contain NUL, so it cannot collide).
    let mut func_id_of_line: Vec<u32> = vec![0; len];
    let mut func_boundary: FxHashMap<u32, usize> = FxHashMap::default();
    let mut i = 0;
    while i < len {
        if infos[i].kind == LineKind::Directive {
            let line = infos[i].trimmed(store.get(i));
            if line == ".cfi_startproc" {
                current_func_id += 1;
                func_boundary.insert(current_func_id, i);
            }
        }
        if infos[i].kind == LineKind::Label {
            let label = infos[i].trimmed(store.get(i));
            if let Some(label_name) = label.strip_suffix(':') {
                if !label_name.starts_with(".L") {
                    current_func_id += 1;
                    func_boundary.insert(current_func_id, i);
                }
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
                            // Alignment directives are equally inert for
                            // control flow: they are padding annotations for
                            // the FOLLOWING label. Stopping the block at them
                            // severed the fall-through edge (X.end landed on
                            // the directive, not on the successor's label), so
                            // blocks entered only by fall-through acquired an
                            // empty predecessor set and merged unsoundly.
                            let dl = infos[end].trimmed(store.get(end));
                            if dl.starts_with(".loc")
                                || dl.starts_with(".file")
                                || is_alignment_directive(dl)
                            {
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
                        func_id_of_line[k] = current_func_id;
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
        func_id_of_line[i] = current_func_id;
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
    // Determine the block-terminating instruction per block.
    let mut block_terminator: Vec<LineKind> = vec![LineKind::Empty; blocks.len()];
    for (bidx, &(start, end, _, _)) in blocks.iter().enumerate() {
        for k in (start + 1..end).rev() {
            if infos[k].is_nop() {
                continue;
            }
            // Alignment directives are now inside the block range; the
            // terminator is the last real instruction, never the padding.
            if infos[k].kind == LineKind::Directive
                && is_alignment_directive(infos[k].trimmed(store.get(k)))
            {
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
                    } else {
                        // Jump from a function's entry region: attribute it
                        // to that function's pseudo entry label. The edge is
                        // an explicit (rewritable) jump.
                        let pseudo = entry_pseudo_label(func_id_of_line[i]);
                        preds.entry(target.clone()).or_default().insert(pseudo);
                    }
                }
            }
        }
    }
    // Fall-through edges: block X (end == B.start, terminator not an
    // unconditional jmp/ret) falls through into block B.
    // A block entered this way can NEVER be selected as the merge
    // duplicate: its entry edge is positional (the predecessor simply
    // continues into the next line) and no branch rewrite can redirect
    // it. Predecessor SETS record label names only, not edge kinds, so
    // "identical preds" does not by itself prove the deleted block's
    // entries are all rewritable jumps (P can jump to the canonical and
    // fall through into the duplicate, making both pred sets {P}).
    let mut fallthrough_entries: FxHashSet<String> = FxHashSet::default();
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
                        fallthrough_entries.insert(b_label.clone());
                    }
                }
            }
        }
    }
    // Fall-through from a function's ENTRY REGION into its first block: the
    // region before the first .LBB label is positional predecessor code that
    // no block models. If its last instruction is not an unconditional
    // transfer, control flows into the first block — record the pseudo
    // predecessor AND a fall-through entry (the edge cannot be rewritten).
    {
        let mut seen_func: FxHashSet<u32> = FxHashSet::default();
        for &(start, _, ref bl, fid) in &blocks {
            if !seen_func.insert(fid) {
                continue;
            }
            // Only when no real block immediately precedes it (a same-function
            // predecessor is handled by the loop above).
            let preceded_by_block = blocks
                .iter()
                .any(|&(ps, pe, _, _)| pe == start);
            if preceded_by_block {
                continue;
            }
            let region_start = func_boundary.get(&fid).copied().unwrap_or(0);
            let mut last_insn: Option<LineKind> = None;
            for k in (region_start..start).rev() {
                if infos[k].is_nop() || infos[k].kind == LineKind::Label {
                    continue;
                }
                if infos[k].kind == LineKind::Directive {
                    // .cfi_*/.loc/.file/alignment lines carry no code.
                    continue;
                }
                last_insn = Some(infos[k].kind);
                break;
            }
            let flows_in = !matches!(
                last_insn,
                Some(LineKind::Jmp | LineKind::JmpIndirect | LineKind::Ret)
            );
            if flows_in {
                let pseudo = entry_pseudo_label(fid);
                preds.entry(bl.clone()).or_default().insert(pseudo);
                fallthrough_entries.insert(bl.clone());
            }
        }
    }

    // Phase 2: Hash each block's content.
    let mut block_hashes: FxHashMap<(u64, u32), Vec<usize>> = FxHashMap::default();
    for (idx, &(start, end, _, func_id)) in blocks.iter().enumerate() {
        let mut hasher = 0u64;
        let mut instr_count = 0u32;
        for j in (start + 1)..end {
            if infos[j].is_nop() {
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
            .filter(|&j| !infos[j].is_nop())
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
            // SOUNDNESS (defense in depth): never merge a block whose recorded
            // predecessor set is EMPTY. An empty set means either genuinely
            // dead code (merging is pointless) or — as happened with
            // alignment directives severing fall-through edges — an entry the
            // edge model could not see. The identical-predecessors guarantee
            // only holds when the model sees every edge, so empty is treated
            // as "unknown", never as "unreachable".
            if canonical_preds.is_empty() {
                continue;
            }
            // SOUNDNESS: the DELETED block must not have a fall-through
            // entry: that edge is positional and survives the merge
            // unredirected, so the predecessor would fall into the removed
            // text. (The canonical block keeps its position, so a
            // fall-through entry on the canonical side is fine.)
            if fallthrough_entries.contains(other_label.as_str()) {
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
                .filter(|&j| !infos[j].is_nop())
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
                    if !infos[j].is_nop() {
                        // Alignment directives inside the deleted range pad
                        // the FOLLOWING block's label (the block scan only
                        // traverses them because they are transparent), and
                        // that block survives the merge. Dropping the padding
                        // would silently unalign a loop header the layout
                        // pass deliberately aligned, so it is kept verbatim.
                        if infos[j].kind == LineKind::Directive
                            && is_alignment_directive(infos[j].trimmed(store.get(j)))
                        {
                            continue;
                        }
                        mark_nop(&mut infos[j]);
                        changed = true;
                    }
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

    /// Alignment directives must not sever fall-through edges in the block
    /// model. A `.p2align` between a guard's conditional branch and the
    /// loop-preheader label used to stop the block scan, so the preheader's
    /// recorded predecessor set came out EMPTY; two such preheaders then
    /// satisfied "identical predecessors" (empty == empty) and one was
    /// deleted — while its guard still fell through into it at runtime,
    /// leaving the surviving loop with uninitialized pointer/counter
    /// registers. Reproduces tests/regression/vectorize_int_map_lanes.c
    /// at -O2 -march=x86-64-v3 with hot-loop alignment (SIGSEGV in
    /// `vmovdqu %ymm0,(%rdi)`, 20/20 runs before the fix).
    #[test]
    fn alignment_directive_does_not_sever_fallthrough_entry() {
        let out = run(concat!(
            "f:\n",
            ".cfi_startproc\n",
            "    xorl %ecx, %ecx\n",
            "    cmpl $25, %r15d\n",
            "    jae .LBB3\n",
            ".p2align 4,,10\n",
            ".p2align 3\n",
            ".LBB1:\n",
            "    movq %rbx, %rdi\n",
            "    movl $0xa5a5a5a5, %eax\n",
            "    vmovd %eax, %xmm0\n",
            "    vpbroadcastd %xmm0, %ymm0\n",
            "    movl $32, %ecx\n",
            ".LBB2:\n",
            "    vmovdqu %ymm0, (%rdi)\n",
            "    vmovdqu %ymm0, 32(%rdi)\n",
            "    addq $64, %rdi\n",
            "    decl %ecx\n",
            "    jne .LBB2\n",
            ".LBB3:\n",
            "    ret\n",
            ".cfi_endproc\n",
            "g:\n",
            ".cfi_startproc\n",
            "    xorl %ecx, %ecx\n",
            "    cmpl $25, %r15d\n",
            "    jae .LBB7\n",
            ".p2align 4,,10\n",
            ".p2align 3\n",
            ".LBB5:\n",
            "    movq %rbx, %rdi\n",
            "    movl $0xa5a5a5a5, %eax\n",
            "    vmovd %eax, %xmm0\n",
            "    vpbroadcastd %xmm0, %ymm0\n",
            "    movl $32, %ecx\n",
            ".LBB6:\n",
            "    vmovdqu %ymm0, (%rdi)\n",
            "    vmovdqu %ymm0, 32(%rdi)\n",
            "    addq $64, %rdi\n",
            "    decl %ecx\n",
            "    jne .LBB6\n",
            ".LBB7:\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        // Both fall-through-entered preheader blocks must survive: their
        // entry edges are positional and cannot be redirected to a merge.
        assert!(
            out.matches("movq %rbx, %rdi").count() == 2,
            "a fall-through-entered preheader was merged away:\n{out}"
        );
        assert!(
            out.matches("vpbroadcastd %xmm0, %ymm0").count() == 2,
            "a fall-through-entered broadcast was merged away:\n{out}"
        );
        // The alignment directives must survive too (they pad the surviving
        // loop headers).
        assert!(
            out.matches(".p2align 4,,10").count() == 2,
            "loop-head alignment padding was dropped:\n{out}"
        );
    }

    /// A block entered only by explicit JUMPS from the same predecessor as
    /// the canonical candidate remains mergeable — the redirect rewrites
    /// every entry edge. This is the positive control for the fix above:
    /// the pass must keep merging sound cases, not stop entirely.
    #[test]
    fn jump_entered_identical_blocks_still_merge() {
        let out = run(concat!(
            "f:\n",
            ".cfi_startproc\n",
            "    testl %edi, %edi\n",
            "    jne .LBB1\n",
            "    jmp .LBB2\n",
            ".LBB1:\n",
            "    movl $1, %eax\n",
            "    addl $2, %eax\n",
            "    addl $3, %eax\n",
            "    addl $4, %eax\n",
            "    ret\n",
            ".LBB2:\n",
            "    movl $1, %eax\n",
            "    addl $2, %eax\n",
            "    addl $3, %eax\n",
            "    addl $4, %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert_eq!(
            out.matches("movl $1, %eax").count(),
            1,
            "jump-entered identical blocks did not merge:\n{out}"
        );
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
}
