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

/// Pseudo predecessor label for a function's entry region (`func_id`).
/// A NUL byte can never appear in a real assembler label, so the pseudo
/// label cannot collide with any block label.
fn entry_pseudo_label(func_id: u32) -> String {
    format!("\u{0}fn{func_id}")
}

/// True only for single-statement, default-fill alignment directives.
/// Explicit fill bytes can execute instructions and are not transparent.
/// The admitted forms are layout annotations: they neither
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
fn is_alignment_directive(trimmed: &str) -> bool {
    (trimmed.starts_with(".p2align")
        || trimmed.starts_with(".align")
        || trimmed.starts_with(".balign"))
        && crate::backend::peephole_common::inert_directive(trimmed)
}

/// True for lines that carry no block identity: NOPs and
/// alignment-directive padding. Skipped by the block hash and the
/// text-identity comparison, so blocks that differ ONLY in padding hash
/// and compare equal and become mergeable (their padding is semantically
/// null; the surviving copy keeps its own, and the deleted copy's
/// directives stay verbatim to pad whatever follows the removed range).
/// Debug markers are DELIBERATELY hashed: identical code at different
/// source lines must not merge, or `-g` stepping attributes the wrong
/// lines. `.cfi_*` stays a block terminator (function-scoped unwind
/// metadata that must survive merging verbatim).
fn line_is_identity_transparent(store: &LineStore, infos: &[LineInfo], j: usize) -> bool {
    if infos[j].is_nop() {
        return true;
    }
    infos[j].kind == LineKind::Directive && is_alignment_directive(infos[j].trimmed(store.get(j)))
}

/// Collect every `.LBB...` label token mentioned in an arbitrary assembler
/// line. Used for BOTH jump-table entries and address-taken label references
/// (`leaq .LBB2(%rip), %rdi`): a single extractor, so the two soundness
/// checks can never drift apart in what they consider a label token.
fn collect_lbb_tokens(line: &str, out: &mut FxHashSet<String>) {
    let mut rest = line;
    while let Some(pos) = crate::backend::peephole_common::find_lbb(rest) {
        let name_start = pos + ".LBB".len();
        let mut j = name_start;
        while j < rest.len()
            && (rest.as_bytes()[j].is_ascii_alphanumeric() || rest.as_bytes()[j] == b'_')
        {
            j += 1;
        }
        out.insert(rest[pos..j].to_string());
        rest = &rest[j..];
    }
}

/// Does this instruction consume (read) the EFLAGS?
///
/// Shared with `return_home`: a rewrite that replaces a flag-writing root
/// must know whether anything in its window consumes what the root wrote.
/// The shared classifier treats unknown instructions as readers, so callers
/// fail closed instead of silently assuming an unmodeled instruction is safe.
pub(super) fn reads_flags(store: &LineStore, infos: &[LineInfo], i: usize) -> bool {
    if infos[i].is_steppable() || infos[i].kind == LineKind::Empty {
        return false;
    }
    super::flag_peepholes::flags_effect(infos[i].trimmed(store.get(i)))
        == super::flag_peepholes::FlagsEffect::Reads
}

/// True if a block can observe any incoming arithmetic flag before that bit is
/// definitely replaced. Partial writes (INC preserves CF, SAHF preserves OF,
/// BT preserves ZF) and zero-count/unknown writers cannot kill the bits they
/// leave live. Unknown/read-write instructions are treated as readers.
fn block_is_flag_dependent(
    store: &LineStore,
    infos: &[LineInfo],
    start: usize,
    end: usize,
) -> bool {
    let mut incoming = super::flag_peepholes::F_ALL;
    for j in (start + 1)..end {
        if infos[j].is_steppable() || infos[j].kind == LineKind::Empty {
            continue;
        }
        let text = infos[j].trimmed(store.get(j));
        match super::flag_peepholes::flags_effect(text) {
            super::flag_peepholes::FlagsEffect::Reads => {
                if incoming != 0 {
                    return true;
                }
            }
            super::flag_peepholes::FlagsEffect::Writes => {
                incoming &= !super::flag_peepholes::flags_written_mask(text);
            }
            super::flag_peepholes::FlagsEffect::Neutral => {}
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
        if infos[j].kind == LineKind::Directive {
            // Padding and debug markers define nothing: the exit register
            // state stays path-independent with them inside the block.
            let dl = infos[j].trimmed(store.get(j));
            if is_alignment_directive(dl) || is_debug_marker_directive(dl) {
                continue;
            }
        }
        match infos[j].kind {
            LineKind::Label | LineKind::Directive => return false,
            _ => {}
        }
    }
    true
}

/// Merge identical basic blocks.
/// Returns true if any blocks were merged.
/// `end line -> indices of the blocks that end there`, in block order.
///
/// A block's predecessor set is built from the blocks that END where it
/// STARTS (`X.end == B.start`), never from the blocks that START there: for a
/// well-formed block list the only block starting at `B.start` is `B` itself,
/// so a start-keyed lookup is a silent no-op -- it yields EMPTY predecessor
/// sets and an empty `fallthrough_entries` set, and the latter is the guard
/// that keeps a fall-through-entered block from being deleted.
///
/// This is a free function so the invariant can be asserted directly rather
/// than only through whatever merge decisions a hand-built shape happens to
/// reach.
fn blocks_ending_at(blocks: &[(usize, usize, String, u32)]) -> FxHashMap<usize, Vec<usize>> {
    let mut ending: FxHashMap<usize, Vec<usize>> = FxHashMap::default();
    for (xi, b) in blocks.iter().enumerate() {
        ending.entry(b.1).or_default().push(xi);
    }
    ending
}

/// Entry CFI CFA offset per block (`None` if unknown or obscured by CFI state
/// directives or any path-sensitive `.cfi_*` inside the function body).
fn compute_block_cfa_offsets(
    store: &LineStore,
    infos: &[LineInfo],
    blocks: &[(usize, usize, String, u32)],
) -> Vec<Option<i64>> {
    let mut line_to_bidx: FxHashMap<usize, usize> = FxHashMap::default();
    for (bidx, &(start, _, _, _)) in blocks.iter().enumerate() {
        line_to_bidx.insert(start, bidx);
    }
    let mut out = vec![None; blocks.len()];
    let mut cfa: Option<i64> = Some(0);
    let mut entered_body = false;
    let mut fn_blocks: Vec<usize> = Vec::new();
    for (i, info) in infos.iter().enumerate() {
        if info.is_nop() {
            continue;
        }
        if let Some(&bidx) = line_to_bidx.get(&i) {
            out[bidx] = cfa;
            fn_blocks.push(bidx);
        }
        let t = info.trimmed(store.get(i));
        if info.kind == LineKind::Label {
            let name = t.trim_end_matches(':');
            if !name.starts_with(".L") {
                cfa = Some(0);
                entered_body = false;
                fn_blocks.clear();
            } else {
                entered_body = true;
            }
            continue;
        }
        if info.kind == LineKind::Directive && super::helpers::directive_emits_data(t) {
            cfa = None;
            for &bidx in &fn_blocks {
                out[bidx] = None;
            }
            entered_body = true;
            continue;
        }
        // DIRECTIVE-LAW: selecting — only `.cfi_*` directives adjust CFA tracking;
        // data-emitting directives above invalidate CFA across the function.
        if info.kind == LineKind::Directive && t.starts_with(".cfi_") {
            if t == ".cfi_startproc" {
                cfa = Some(0);
                entered_body = false;
                fn_blocks.clear();
            } else if t == ".cfi_endproc" {
                cfa = None;
                entered_body = false;
                fn_blocks.clear();
            } else if entered_body {
                cfa = None;
                for &bidx in &fn_blocks {
                    out[bidx] = None;
                }
            } else if let Some(rest) = t.strip_prefix(".cfi_def_cfa_offset ") {
                cfa = rest.trim_ascii().parse::<i64>().ok();
            } else if let Some(rest) = t.strip_prefix(".cfi_adjust_cfa_offset ") {
                let delta = rest.trim_ascii().parse::<i64>().ok();
                cfa = match (cfa, delta) {
                    (Some(c), Some(d)) => c.checked_add(d),
                    _ => None,
                };
            } else if t.starts_with(".cfi_offset ")
                || t.starts_with(".cfi_rel_offset ")
                || t.starts_with(".cfi_register ")
                || t.starts_with(".cfi_restore ")
                || t.starts_with(".cfi_same_value ")
                || t.starts_with(".cfi_undefined ")
                || t.starts_with(".cfi_personality")
                || t.starts_with(".cfi_lsda")
            {
                // Unwind register location annotations leave CFA offset untouched.
            } else {
                cfa = None;
            }
            continue;
        }
        if matches!(
            info.kind,
            LineKind::Jmp
                | LineKind::JmpIndirect
                | LineKind::CondJmp
                | LineKind::Ret
                | LineKind::InlineAsm
        ) {
            entered_body = true;
        }
    }
    out
}

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
                        // DIRECTIVE-LAW: bails -- the class test opens a branch that
                        // reads the directive's text; the window below never advances
                        // past a directive it has not classified.
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
                            if is_debug_marker_directive(dl) || is_alignment_directive(dl) {
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
            // Debug markers are skipped the same way: a `.loc` trailing a
            // block otherwise poses as a `Directive` terminator and invents
            // a fall-through edge the block's real terminator (e.g. an
            // unconditional jump) does not have.
            if infos[k].kind == LineKind::Directive {
                let dl = infos[k].trimmed(store.get(k));
                if is_alignment_directive(dl) || is_debug_marker_directive(dl) {
                    continue;
                }
            }
            block_terminator[bidx] = infos[k].kind;
            break;
        }
    }
    let mut first_lbb_of_func: FxHashMap<u32, usize> = FxHashMap::default();
    for &(start, _, _, fid) in &blocks {
        first_lbb_of_func.entry(fid).or_insert(start);
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
                    .trim_ascii()
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
                        // Only attribute jumps BEFORE the first .LBB block to the
                        // function's entry region; post-directive orphan fragments
                        // receive distinct gap ids so they never alias entry preds.
                        let fid = func_id_of_line[i];
                        let pseudo = if first_lbb_of_func.get(&fid).is_none_or(|&first| i < first) {
                            entry_pseudo_label(fid)
                        } else {
                            format!("\0gap_{i}")
                        };
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
    // `start -> block index` index: the previous formulation rescanned every
    // block for every block (`O(B^2)`, and on a wide `switch` B is the number
    // of arm blocks).  Blocks are disjoint and in source order, so at most
    // ONE block can end at a given start; the index makes the predecessor
    // lookup a single hash hit with identical results.  `entry().or_insert`
    // keeps the FIRST claimant if a degenerate input ever produced two, which
    // is exactly what the old first-match `if blocks[xi].1 == b_start` did.
    // `end -> block indices that end there`.  A list, not a single index: the
    // old scan visited EVERY block ending at `b_start` (it did not break), and
    // degenerate input can have more than one; keeping all claimants makes the
    // predecessor sets identical to the old first-match-free scan.
    let blocks_ending_at = blocks_ending_at(&blocks);
    let mut block_starting_at: FxHashMap<usize, usize> = FxHashMap::default();
    for (xi, b) in blocks.iter().enumerate() {
        block_starting_at.entry(b.0).or_insert(xi);
    }
    for bi in 0..blocks.len() {
        let b_start = blocks[bi].0;
        let b_label = blocks[bi].2.clone();
        // Every block that ENDS exactly where B starts immediately precedes B
        // (`blocks_ending_at`, not `block_starting_at`: the latter would only
        // ever return B itself).
        if let Some(xs) = blocks_ending_at.get(&b_start) {
            for &xi in xs {
                if xi == bi {
                    continue;
                }
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
        // `O(1)` membership for "some block ends exactly here".  The old
        // `blocks.iter().any(|..| pe == start)` rescanned every block once per
        // function — quadratic in the block count on a wide `switch`.
        let block_ends: FxHashSet<usize> = blocks.iter().map(|b| b.1).collect();
        for &(start, _, ref bl, fid) in &blocks {
            if !seen_func.insert(fid) {
                continue;
            }
            // Only when no real block immediately precedes it (a same-function
            // predecessor is handled by the loop above).
            let preceded_by_block = block_ends.contains(&start);
            if preceded_by_block {
                continue;
            }
            let region_start = func_boundary.get(&fid).copied().unwrap_or(0);
            let mut last_insn: Option<LineKind> = None;
            for k in (region_start..start).rev() {
                if infos[k].is_nop() || infos[k].kind == LineKind::Label {
                    continue;
                }
                // The law, not the class: `.cfi_*`/.loc/.file/alignment lines carry
                // no code and are stepped over, and a data directive -- which *is*
                // code the text does not spell out -- ends the backward walk like
                // any other opaque line.  Testing `kind == Directive` here skipped
                // `.long .L1-.L2` and then read the instruction before it as the
                // last one, which is how `flows_in` was decided.
                if infos[k].is_steppable() {
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

    // Phase 2: Hash each block's content. Padding (NOPs, alignment
    // directives) is not identity: blocks that differ only in padding
    // share a hash and become merge candidates.
    let mut block_hashes: FxHashMap<(u64, u32), Vec<usize>> = FxHashMap::default();
    for (idx, &(start, end, _, func_id)) in blocks.iter().enumerate() {
        let mut hasher = 0u64;
        let mut instr_count = 0u32;
        for j in (start + 1)..end {
            if line_is_identity_transparent(store, infos, j) {
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
            collect_lbb_tokens(line, &mut jump_table_targets);
        }
    }

    // SOUNDNESS: a block label whose ADDRESS is taken is a DATA reference,
    // not a control-flow edge. `&&label` (GNU computed goto) emits
    // `leaq .LBBn(%rip), %reg`; jump tables emit `.quad .LBBn`; a
    // static initializer can emit `.long .LBBn - .LBBm`. The pass can
    // rewrite BRANCHES to a canonical block, but it cannot rewrite those
    // symbol operands. Deleting the duplicate label therefore leaves an
    // undefined local symbol -- the integrated assembler then fails with
    // "`.LBBn` is referenced ... but has no symbol-table entry", or an
    // external assembler silently binds it to absolute 0 and the computed
    // goto jumps to address 0. Reproducer: gcc.c-torture/compile/pr84136.c
    // and pr17913.c, whose `leaq .LBB2(%rip), %rdi` referenced a label the
    // identical-block merge had removed. Protect every such label by
    // excluding its block from merging entirely (canonical AND duplicate):
    // computed-goto code is rare, so the foregone merge is free in practice.
    let mut address_taken_labels: FxHashSet<String> = FxHashSet::default();
    for i in 0..len {
        if infos[i].is_nop() {
            continue;
        }
        // A label DEFINITION or a branch TARGET is control flow: the pass
        // already models those edges and may legitimately rewrite them.
        if matches!(
            infos[i].kind,
            LineKind::Label | LineKind::Jmp | LineKind::CondJmp | LineKind::JmpIndirect
        ) {
            continue;
        }
        let line = infos[i].trimmed(store.get(i));
        if line.contains(".LBB") {
            collect_lbb_tokens(line, &mut address_taken_labels);
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
            // Find the block that starts exactly where this block ends, in
            // O(1) through the same start index built above (the old inner
            // scan made this quadratic in the block count).
            let end_pos = blocks[bi].1;
            if let Some(&x) = block_starting_at.get(&end_pos)
                && x != bi
            {
                fallthrough[bi] = Some(blocks[x].2.clone());
            }
        }
    }

    // Precompute per-block soundness flags ONCE (not per candidate pair).
    // flag_dep[i]: block reads incoming EFLAGS before establishing its own.
    // clean[i]:    single-entry straight-line block with a modeled exit.
    // preds_clean[i]: all predecessor blocks are clean basic blocks.
    let nblocks = blocks.len();
    let mut flag_dep = vec![false; nblocks];
    let mut clean = vec![false; nblocks];
    let mut preds_clean = vec![false; nblocks];
    // `clean` is a property of ONE block, but it was recomputed for every
    // PREDECESSOR edge: with E predecessor edges the old code re-walked a
    // block's lines up to E times — the last quadratic path in this pass on a
    // wide `switch` (every arm shares the enclosing blocks as predecessors).
    // Memoise the per-block verdict; the walk itself is unchanged, so the
    // pass makes exactly the same decisions in O(lines + edges).
    let mut block_clean_memo: Vec<Option<bool>> = vec![None; nblocks];
    for bi in 0..nblocks {
        let (bs, be, ref bl, _) = blocks[bi];
        flag_dep[bi] = block_is_flag_dependent(store, infos, bs, be);
        // None is also the sentinel for an UNMODELED fall-through (e.g. a
        // scan stopped at an opaque directive). Two such sentinels do not
        // prove a common successor: merging the equal prefixes would route
        // both entries into the canonical prefix's different opaque tail.
        let exit_known = matches!(
            block_terminator[bi],
            LineKind::Jmp | LineKind::JmpIndirect | LineKind::Ret
        ) || fallthrough[bi].is_some();
        clean[bi] = exit_known && block_is_clean(store, infos, bs, be);
        block_clean_memo[bi] = Some(clean[bi]);
        let mut all_clean = true;
        if let Some(pset) = preds.get(bl) {
            for pl in pset {
                if let Some(&pidx) = label_to_bidx.get(pl) {
                    let is_clean = match block_clean_memo[pidx] {
                        Some(v) => v,
                        None => {
                            let (ps, pe, _, _) = blocks[pidx];
                            let v = block_is_clean(store, infos, ps, pe);
                            block_clean_memo[pidx] = Some(v);
                            v
                        }
                    };
                    if !is_clean {
                        all_clean = false;
                        break;
                    }
                } else if pl.starts_with("\0gap_") {
                    all_clean = false;
                    break;
                }
            }
        }
        preds_clean[bi] = all_clean;
    }

    // Phase 3: Merge identical blocks. Blocks with identical predecessor sets
    // merge freely; tail-terminated blocks (`ret` or unconditional `jmp`) with
    // distinct non-empty predecessor sets also merge when both are clean,
    // non-flag-dependent, and share identical entry CFI CFA depth.
    let block_cfa = compute_block_cfa_offsets(store, infos, &blocks);
    let mut changed = false;
    let mut redirects: FxHashMap<String, String> = FxHashMap::default(); // old_label → canonical_label

    for ((_hash, _func_id), group) in &block_hashes {
        if group.len() < 2 {
            continue;
        }
        // Prefer a clean, non-flag-dep, non-empty-pred canonical candidate.
        // Protected blocks (fall-through targets, jump-table targets,
        // address-taken labels) can serve as the surviving canonical target
        // for tail-merging, while never themselves being deleted.
        let Some(&canonical_idx) = group
            .iter()
            .find(|&&bi| {
                let lbl = &blocks[bi].2;
                !flag_dep[bi]
                    && clean[bi]
                    && preds_clean[bi]
                    && preds.get(lbl).is_some_and(|p| !p.is_empty())
                    && (fallthrough_entries.contains(lbl.as_str())
                        || jump_table_targets.contains(lbl)
                        || address_taken_labels.contains(lbl))
                    && matches!(block_terminator[bi], LineKind::Ret | LineKind::Jmp)
                    && fallthrough[bi].is_none()
            })
            .or_else(|| {
                group.iter().find(|&&bi| {
                    let lbl = &blocks[bi].2;
                    !flag_dep[bi]
                        && clean[bi]
                        && preds_clean[bi]
                        && preds.get(lbl).is_some_and(|p| !p.is_empty())
                        && !jump_table_targets.contains(lbl)
                        && !address_taken_labels.contains(lbl)
                })
            })
        else {
            continue;
        };
        let (can_start, can_end, _, _) = &blocks[canonical_idx];
        let canonical_label = blocks[canonical_idx].2.clone();
        let canonical_preds = preds.get(&canonical_label).cloned().unwrap_or_default();

        let canonical_instrs: Vec<String> = ((*can_start + 1)..*can_end)
            .filter(|&j| !line_is_identity_transparent(store, infos, j))
            .map(|j| store.get(j).to_string())
            .collect();

        if canonical_preds.is_empty()
            || flag_dep[canonical_idx]
            || !clean[canonical_idx]
            || !preds_clean[canonical_idx]
        {
            continue;
        }

        let canonical_is_tail = matches!(
            block_terminator[canonical_idx],
            LineKind::Ret | LineKind::Jmp
        ) && fallthrough[canonical_idx].is_none();

        for &other_idx in group {
            if other_idx == canonical_idx {
                continue;
            }
            let (other_start, other_end, ref other_label, _) = blocks[other_idx];
            let other_preds = preds.get(other_label).cloned().unwrap_or_default();
            // SOUNDNESS (defense in depth): never merge a block whose recorded
            // predecessor set is EMPTY (dead or unknown positional entry).
            if other_preds.is_empty() {
                continue;
            }
            let same_preds = canonical_preds == other_preds;
            if !same_preds {
                let other_is_tail =
                    matches!(block_terminator[other_idx], LineKind::Ret | LineKind::Jmp)
                        && fallthrough[other_idx].is_none();
                if !canonical_is_tail
                    || !other_is_tail
                    || block_cfa[canonical_idx].is_none()
                    || block_cfa[canonical_idx] != block_cfa[other_idx]
                {
                    continue;
                }
            } else if jump_table_targets.contains(&canonical_label)
                || address_taken_labels.contains(&canonical_label)
            {
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
            if jump_table_targets.contains(other_label)
                || address_taken_labels.contains(other_label)
            {
                continue;
            }
            if flag_dep[other_idx] || !clean[other_idx] || !preds_clean[other_idx] {
                continue;
            }

            let other_instrs: Vec<String> = ((other_start + 1)..other_end)
                .filter(|&j| !line_is_identity_transparent(store, infos, j))
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
                    let target = trimmed[space_pos + 1..].trim_ascii();
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

    #[test]
    fn flag_reader_oracle_covers_carry_and_flag_materialization() {
        for text in [
            "rcll $1, %edi",
            "rcrq $1, %rdi",
            "lahf",
            "pushfq",
            "cmc",
            "loope .L1",
            "loopnz .L2",
            "adcxq %r8, %r9",
            "adoxq %r8, %r9",
            "fcmovb %st(1), %st",
        ] {
            let store = super::LineStore::new(format!("{text}\n"));
            let info = super::classify_line(store.get(0));
            assert!(
                super::reads_flags(&store, &[info], 0),
                "{text} consumes EFLAGS"
            );
        }
    }

    fn flag_dependent(body: &str) -> bool {
        let asm = format!(".LBB0:\n{body}\n");
        let store = super::LineStore::new(asm);
        let infos: Vec<_> = (0..store.len())
            .map(|i| super::classify_line(store.get(i)))
            .collect();
        super::block_is_flag_dependent(&store, &infos, 0, store.len())
    }

    #[test]
    fn a_flag_neutral_not_does_not_kill_incoming_flags() {
        assert!(flag_dependent("    notl %eax\n    setb %cl"));
    }

    #[test]
    fn inc_and_zero_count_shift_do_not_kill_all_incoming_flags() {
        assert!(flag_dependent("    incq %rax\n    setb %cl"));
        assert!(flag_dependent("    shlq $0, %rax\n    setb %cl"));
    }

    #[test]
    fn a_full_flag_writer_kills_incoming_flag_dependence() {
        assert!(!flag_dependent("    addl $1, %eax\n    setb %cl"));
    }

    fn run(asm: &str) -> String {
        peephole_optimize(asm.to_string())
    }

    /// Call `merge_identical_blocks` ALONE and report how many copies of
    /// `needle` survive.
    fn survivors_after_isolated_merge(asm: &str, needle: &str) -> usize {
        let mut store = super::LineStore::new(asm.to_string());
        let mut infos: Vec<_> = (0..store.len())
            .map(|index| super::classify_line(store.get(index)))
            .collect();
        super::merge_identical_blocks(&mut store, &mut infos);
        (0..store.len())
            .filter(|&i| !infos[i].is_nop() && store.get(i).trim_ascii() == needle)
            .count()
    }

    fn blk(start: usize, end: usize, label: &str) -> (usize, usize, String, u32) {
        (start, end, label.to_string(), 0)
    }

    /// The predecessor lookup must be keyed on block ENDS. A start-keyed index
    /// returns only the block itself for every query, i.e. no predecessor at
    /// all, and it also empties `fallthrough_entries` -- which is the guard
    /// that stops the pass from deleting a block whose entry is positional.
    #[test]
    fn blocks_ending_at_indexes_ends_not_starts() {
        // A: [0,3)  B: [3,6)  C: [6,9) -- B and C start where the previous ends.
        let blocks = vec![blk(0, 3, ".LA"), blk(3, 6, ".LB"), blk(6, 9, ".LC")];
        let ending = super::blocks_ending_at(&blocks);
        assert_eq!(ending.get(&3).map(Vec::as_slice), Some(&[0usize][..]));
        assert_eq!(ending.get(&6).map(Vec::as_slice), Some(&[1][..]));
        assert_eq!(ending.get(&9).map(Vec::as_slice), Some(&[2][..]));
        // Nothing ends at a block's own start, which is why keying on starts
        // can never find a predecessor.
        for (bi, b) in blocks.iter().enumerate() {
            assert!(
                ending.get(&b.0).is_none_or(|xs| !xs.contains(&bi)),
                "a block must not be its own predecessor"
            );
        }
    }

    /// Degenerate input can have SEVERAL blocks ending at one line (a block
    /// with no instructions of its own). Every claimant must be kept: the old
    /// scan visited all of them, so keeping one would change predecessor sets.
    #[test]
    fn blocks_ending_at_keeps_every_claimant() {
        let blocks = vec![blk(0, 4, ".LA"), blk(4, 4, ".LM"), blk(4, 8, ".LB")];
        let ending = super::blocks_ending_at(&blocks);
        assert_eq!(ending.get(&4).map(Vec::as_slice), Some(&[0usize, 1][..]));
    }

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
    /// GNU computed goto (`&&label`) makes a block label a DATA operand:
    /// `leaq .LBB2(%rip), %rdi`. Two byte-identical `ret` blocks used to be
    /// merged, deleting `.LBB2:` while the `leaq` still named it. The
    /// integrated assembler then reported "`.LBB2` is referenced ... but has
    /// no symbol-table entry" (gcc.c-torture/compile/pr84136.c, pr17913.c).
    /// Both labels must survive with their blocks intact.
    #[test]
    fn computed_goto_label_addresses_are_not_merged() {
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    leaq .LBB2(%rip), %rdi\n",
            "    movq %rdi, a(%rip)\n",
            "    testq %rdi, %rdi\n",
            "    je .LBB2\n",
            ".LBB1:\n",
            "    ret\n",
            ".LBB2:\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains(".LBB1:\n"), "canonical label vanished:\n{out}");
        assert!(
            out.contains(".LBB2:\n"),
            "address-taken label was merged away:\n{out}"
        );
        assert!(
            out.contains("leaq .LBB2(%rip)"),
            "reference vanished:\n{out}"
        );
    }

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

    /// Blocks that differ ONLY in alignment-directive padding hash and
    /// compare equal and merge: padding is semantically null. The deleted
    /// copy's directives stay verbatim (they pad the surviving successor's
    /// label), so the padding count is unchanged by the merge.
    #[test]
    fn blocks_differing_only_in_padding_still_merge() {
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
            ".p2align 4,,10\n",
            ".p2align 3\n",
            ".LBB3:\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert_eq!(
            out.matches("movl $1, %eax").count(),
            1,
            "padding-only difference blocked the merge:\n{out}"
        );
        assert_eq!(
            out.matches(".p2align 4,,10").count(),
            1,
            "deleted copy's padding was not preserved verbatim:\n{out}"
        );
        assert!(out.contains(".LBB3:\n"), "successor label lost:\n{out}");
    }

    const TRAILING_LOC: &str = concat!(
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
        "    jmp .LBB3\n",
        "    .loc 1 10 0\n",
        ".LBB2:\n",
        "    movl $1, %eax\n",
        "    addl $2, %eax\n",
        "    addl $3, %eax\n",
        "    addl $4, %eax\n",
        "    jmp .LBB3\n",
        "    .loc 1 10 0\n",
        ".LBB3:\n",
        "    ret\n",
        ".cfi_endproc\n",
    );

    /// A trailing `.loc` must not pose as the block terminator: the block
    /// above still ends in `jmp`, so it has no fall-through edge. Treating
    /// the marker as a `Directive` terminator invents a fall-through edge
    /// into the layout successor, the two blocks' fall-through successors
    /// differ, and the merge is wrongly skipped. (The markers also must not
    /// count as dirt: exactly one copy survives, on the canonical block.)
    /// The pass itself, on visible markers: the pipeline hides rows.
    #[test]
    fn trailing_debug_markers_do_not_fake_fallthrough() {
        let mut store = super::LineStore::new(TRAILING_LOC.to_string());
        let mut infos: Vec<_> = (0..store.len())
            .map(|index| super::classify_line(store.get(index)))
            .collect();
        assert!(super::merge_identical_blocks(&mut store, &mut infos));
        let out = store.build_result(|index| infos[index].is_nop());
        assert_eq!(
            out.matches("movl $1, %eax").count(),
            1,
            "trailing .loc blocked the merge (faked fall-through edge?):\n{out}"
        );
        assert_eq!(
            out.matches(".loc 1 10 0").count(),
            1,
            "debug marker was not preserved exactly once:\n{out}"
        );
    }

    /// Through the pipeline the rows are hidden, so the result is the
    /// row-free result, rows kept. (Before rows were hidden this input merged
    /// only under `-g`: the visible `.loc` between `jmp .LBB3` and `.LBB3:`
    /// kept the jump-to-next-label elision from firing.)
    #[test]
    fn trailing_debug_markers_leave_the_pipeline_result_unchanged() {
        let plain: String = TRAILING_LOC
            .lines()
            .filter(|l| !l.trim_ascii_start().starts_with(".loc"))
            .map(|l| format!("{l}\n"))
            .collect();
        let out = run(TRAILING_LOC);
        let code: String = out
            .lines()
            .filter(|l| !l.trim_ascii_start().starts_with(".loc"))
            .map(|l| format!("{l}\n"))
            .collect();
        assert_eq!(code, run(&plain));
        assert!(out.contains(".loc 1 10 0"), "{out}");
    }

    #[test]
    fn s97_effectful_alignment_is_not_block_identity() {
        for text in [
            ".p2align 4,0xf8",
            ".p2align 4,0xf9",
            ".align 16,0xcc",
            ".balign 16,0x90",
            ".p2align 4; clc",
            ".p2align_future 4",
        ] {
            assert!(
                !super::is_alignment_directive(text),
                "opaque padding: {text}"
            );
        }
        for text in [
            ".p2align 4",
            ".p2align 4,,10",
            ".balign 16",
            ".align 16 # ; clc",
        ] {
            assert!(super::is_alignment_directive(text), "safe padding: {text}");
        }
    }

    #[test]
    fn s97_unknown_fallthrough_is_not_a_shared_exit() {
        let original = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/regression/fixtures/peephole-padding-effects/caller.s"
        ));
        for (first, second) in [
            (".p2align 4,0xf8", ".p2align 4,0xf9"),
            (".byte 0xf8", ".byte 0xf9"),
            (
                ".cfi_def_cfa_offset 8\n    clc",
                ".cfi_def_cfa_offset 8\n    stc",
            ),
        ] {
            let asm = original
                .replace(".p2align 4,0xf8", first)
                .replace(".p2align 4,0xf9", second);
            let mut store = super::LineStore::new(asm);
            let mut infos: Vec<_> = (0..store.len())
                .map(|index| super::classify_line(store.get(index)))
                .collect();
            assert!(
                !super::merge_identical_blocks(&mut store, &mut infos),
                "a truncated prefix has an unknown exit, not a shared exit: {first}"
            );
        }
    }

    #[test]
    #[cfg(all(target_arch = "x86_64", target_os = "linux"))]
    fn s97_custom_padding_real_pipeline_native() {
        use crate::common::temp_files::TempFile;
        use std::process::Command;
        let fixtures = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/regression/fixtures/peephole-padding-effects");
        let original = std::fs::read_to_string(fixtures.join("caller.s")).unwrap();
        let optimized = run(&original);
        let mut store = super::LineStore::new(original.clone());
        let mut infos: Vec<_> = (0..store.len())
            .map(|index| super::classify_line(store.get(index)))
            .collect();
        super::merge_identical_blocks(&mut store, &mut infos);
        let isolated = store.build_result(|index| infos[index].is_nop());
        let assembly = TempFile::try_new("padding", "witness", "s").unwrap();
        let executable = TempFile::try_new("padding", "witness", "exe").unwrap();
        for (stage, code) in [
            ("original", original),
            ("isolated", isolated),
            ("optimized", optimized),
        ] {
            std::fs::write(assembly.path(), &code).unwrap();
            let compiled = Command::new("gcc")
                .args(["-O2", "-Werror", "-no-pie", "-o"])
                .arg(executable.path())
                .arg(fixtures.join("driver.c"))
                .arg(assembly.path())
                .output()
                .unwrap();
            assert!(
                compiled.status.success() && compiled.stderr.is_empty(),
                "{stage}: {}",
                String::from_utf8_lossy(&compiled.stderr)
            );
            let result = Command::new(executable.path()).output().unwrap();
            assert!(
                result.status.success(),
                "{stage}: {:?}\n{code}",
                result.status.code()
            );
        }
    }

    /// Direct classification contract: debug markers vs padding vs code.
    #[test]
    fn debug_marker_and_padding_classification() {
        use super::{is_alignment_directive, is_debug_marker_directive};
        assert!(is_debug_marker_directive(".loc 1 10 0"));
        assert!(is_debug_marker_directive(".file 1 \"x.c\""));
        assert!(!is_debug_marker_directive(".p2align 4,,10"));
        assert!(!is_debug_marker_directive(".cfi_def_cfa_offset 8"));
        assert!(!is_debug_marker_directive("movl $1, %eax"));
        assert!(is_alignment_directive(".p2align 4,,10"));
        assert!(is_alignment_directive(".p2align 3"));
        assert!(is_alignment_directive(".align 16"));
        assert!(is_alignment_directive(".balign 8"));
        assert!(!is_alignment_directive(".loc 1 2 0"));
        assert!(!is_alignment_directive("movl $1, %eax"));
    }

    #[test]
    fn distinct_pred_tail_blocks_merge_when_cfa_matches_and_refuse_when_different() {
        let same_cfa = concat!(
            "f:\n",
            ".cfi_startproc\n",
            "    testl %edi, %edi\n",
            "    je .LBB2\n",
            ".LBB1:\n",
            "    testl %esi, %esi\n",
            "    jne .LBB3\n",
            "    jmp .LBB4\n",
            ".LBB2:\n",
            "    movl $2, %eax\n",
            "    ret\n",
            ".LBB3:\n",
            "    movl $2, %eax\n",
            "    ret\n",
            ".LBB4:\n",
            "    xorl %eax, %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
        );
        assert_eq!(survivors_after_isolated_merge(same_cfa, "movl $2, %eax"), 1);

        let diff_cfa = concat!(
            "g:\n",
            ".cfi_startproc\n",
            "    testl %edi, %edi\n",
            "    je .LBB2\n",
            ".LBB1:\n",
            "    pushq %rbx\n",
            "    .cfi_adjust_cfa_offset 8\n",
            "    popq %rbx\n",
            "    jmp .LBB3\n",
            ".LBB2:\n",
            "    movl $2, %eax\n",
            "    ret\n",
            "    .cfi_adjust_cfa_offset 8\n",
            ".LBB3:\n",
            "    movl $2, %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
        );
        assert_eq!(survivors_after_isolated_merge(diff_cfa, "movl $2, %eax"), 2);
    }
}
