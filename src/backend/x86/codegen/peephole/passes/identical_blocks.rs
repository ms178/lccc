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
        if infos[j].is_nop() {
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
                            let dl = infos[end].trimmed(store.get(end));
                            if dl.starts_with(".loc") || dl.starts_with(".file") {
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
    // Determine the block-terminating instruction per block.
    let mut block_terminator: Vec<LineKind> = vec![LineKind::Empty; blocks.len()];
    for (bidx, &(start, end, _, _)) in blocks.iter().enumerate() {
        for k in (start + 1..end).rev() {
            if infos[k].is_nop() {
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
            block_hashes.entry((hasher, func_id)).or_default().push(idx);
        }
    }

    // Build label → block index map, for predecessor-cleanliness checks.
    let mut label_to_bidx: FxHashMap<String, usize> = FxHashMap::default();
    for (idx, b) in blocks.iter().enumerate() {
        label_to_bidx.insert(b.2.clone(), idx);
    }

    // SOUNDNESS: collect labels referenced from JUMP TABLES
    // (`.long .LBBxxx - .Ljt_n` entries in .rodata). Redirecting/eliminating a
    // block whose label appears in a jump table leaves the jump-table entry
    // pointing at a removed (NOP) block — `jmp *%rdx` then lands on garbage and
    // crashes. The pass cannot rewrite those `.long` entries, so any block that
    // is a jump-table target must be excluded from merging entirely.
    let mut jump_table_targets: FxHashSet<String> = FxHashSet::default();
    for i in 0..len {
        if infos[i].is_nop() {
            continue;
        }
        let line = infos[i].trimmed(store.get(i));
        if line.starts_with(".long ") {
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
