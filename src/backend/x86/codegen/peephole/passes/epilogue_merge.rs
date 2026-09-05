//! Epilogue tail merging (cross-jumping on function exits).
//!
//! # The waste
//!
//! Every `return` site in a function that uses callee-saved registers gets
//! its own full epilogue. In `glibc_memcmp_common_alignment` — six returns,
//! six callee-saved registers — that is six byte-identical copies of
//!
//! ```text
//!     addq $120, %rsp
//!     popq %rbp
//!     popq %r15
//!     popq %r14
//!     popq %r13
//!     popq %r12
//!     popq %rbx
//!     ret
//! ```
//!
//! 48 instructions, 40 of them redundant. GCC cross-jumps: one epilogue,
//! `jmp` from the rest. Measured over `tests/benchmark/{programs,
//! kernel_corpus}`: **15 functions, 119 redundant instructions**, the worst
//! being 35 in `glibc_memcmp_common_alignment` and 14 in `sqlite_varint`'s
//! `main`.
//!
//! # The transform
//!
//! Group exit sites by their *exact* epilogue instruction sequence (the
//! maximal run of `popq %reg` / `addq $imm, %rsp` / `leave` immediately
//! preceding a `ret`). Keep the first member of each group, label it, and
//! replace every other member with a single `jmp` to that label.
//!
//! Saving per replaced site is `len(epilogue)` instructions, so a group is
//! only rewritten when the epilogue is at least two instructions long — a
//! bare `ret`, or a `ret` preceded by one pop, is already as short as the
//! `jmp` that would replace it.
//!
//! # Soundness
//!
//! * Only `popq %reg`, `addq $imm, %rsp` and `leave` are collected. None of
//!   them touches `%rax`/`%xmm0`, so the return value — materialised *before*
//!   the run — is untouched by the merge. The `jmp` replaces the run in its
//!   entirety, so the shared copy performs exactly the same restores in
//!   exactly the same order.
//! * A run containing a label is rejected: something may branch into its
//!   middle, and only whole runs may be replaced.
//! * Grouping is per function (`.cfi_startproc` / `.cfi_endproc` delimit),
//!   so a jump can never cross a function boundary.
//! * lccc emits no CFI inside epilogues (only a single
//!   `.cfi_def_cfa_offset` after the prologue), so no unwind directive is
//!   invalidated by the move. The pass re-checks this per function and bails
//!   out for any function that has CFI inside the body, which keeps it
//!   correct if the emitter ever becomes more precise.
//! * The pass runs on the final line store, after every other peephole, so
//!   nothing downstream re-reads the removed instructions.

use super::super::types::*;
use crate::common::fx_hash::FxHashMap;

/// One exit site: `[start, ret_idx]` is the epilogue run plus its `ret`.
struct Exit {
    start: usize,
    ret_idx: usize,
    /// Epilogue instruction texts in program order, excluding the `ret`.
    parts: Vec<String>,
    /// Line index of each entry in `parts`.
    line_of: Vec<usize>,
}

/// Whether `t` is an epilogue-only instruction: it restores frame state and
/// provably does not touch the return-value registers.
fn is_epilogue_insn(t: &str) -> bool {
    if t == "leave" {
        return true;
    }
    if let Some(rest) = t.strip_prefix("popq %") {
        // `popq %rax` would clobber the return value; every other GPR pop is
        // a callee-saved restore. Reject rax/xmm forms explicitly.
        return !rest.is_empty() && rest != "rax";
    }
    // `addq $N, %rsp` / `subq $N, %rsp`
    if let Some(rest) = t.strip_prefix("addq $") {
        return rest.ends_with(", %rsp");
    }
    false
}

pub(super) fn merge_epilogue_tails(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    if std::env::var_os("CCC_NO_EPILOGUE_MERGE").is_some() {
        return false;
    }
    let len = store.len();
    if len == 0 {
        return false;
    }

    let mut changed = false;
    let mut fn_start = 0usize;
    let mut i = 0usize;
    // Unique label suffix per function so two functions cannot collide.
    let mut label_seq: u32 = 0;

    while i < len {
        let t = infos[i].trimmed(store.get(i));
        if t == ".cfi_startproc" {
            fn_start = i;
        }
        if t != ".cfi_endproc" {
            i += 1;
            continue;
        }
        let fn_end = i;
        changed |= merge_one_function(store, infos, fn_start, fn_end, &mut label_seq);
        i += 1;
    }
    changed
}

fn merge_one_function(
    store: &mut LineStore,
    infos: &mut [LineInfo],
    fn_start: usize,
    fn_end: usize,
    label_seq: &mut u32,
) -> bool {
    // Bail on any CFI inside the body: moving code would invalidate it.
    for k in (fn_start + 1)..fn_end {
        let t = infos[k].trimmed(store.get(k));
        if t.starts_with(".cfi_") && !t.starts_with(".cfi_def_cfa") && !t.starts_with(".cfi_offset")
        {
            return false;
        }
    }

    // Collect exit sites.
    let mut exits: Vec<Exit> = Vec::new();
    for k in (fn_start + 1)..fn_end {
        if infos[k].is_nop() {
            continue;
        }
        if infos[k].trimmed(store.get(k)) != "ret" {
            continue;
        }
        // Walk backwards over the epilogue run, skipping nops.
        let mut start = k;
        let mut parts: Vec<String> = Vec::new();
        let mut lines: Vec<usize> = Vec::new();
        let mut j = k;
        while j > fn_start + 1 {
            j -= 1;
            if infos[j].is_nop() || infos[j].kind == LineKind::Empty {
                continue;
            }
            let tj = infos[j].trimmed(store.get(j));
            // A label inside the run: something may branch into the middle.
            if tj.ends_with(':') || tj.starts_with('.') {
                break;
            }
            if !is_epilogue_insn(tj) {
                break;
            }
            parts.push(tj.to_string());
            lines.push(j);
            start = j;
        }
        if parts.len() < 2 {
            continue;
        }
        parts.reverse();
        lines.reverse();
        exits.push(Exit {
            start,
            ret_idx: k,
            parts,
            line_of: lines,
        });
    }
    if exits.len() < 2 {
        return false;
    }

    // Group by longest common SUFFIX, not exact text.
    //
    // Exact matching only merges exits that restore the same registers in the
    // same way. Real functions also produce exits that differ only in how
    // much they restore — a path that never touched `%r15` pops one fewer
    // register — and the shorter epilogue is then a strict SUFFIX of the
    // longer one. Jumping into the middle of the longer copy merges those
    // too, which exact matching cannot.
    //
    // Longest first, so every shorter exit finds the deepest host available.
    let mut order: Vec<usize> = (0..exits.len()).collect();
    order.sort_by_key(|&i| std::cmp::Reverse(exits[i].parts.len()));

    let mut changed = false;
    let mut consumed: Vec<bool> = vec![false; exits.len()];
    // Label already planted inside a host, keyed by (host index, suffix len).
    let mut host_labels: FxHashMap<(usize, usize), String> = FxHashMap::default();
    // Per host, the line index at which each suffix length begins.
    for hi in 0..order.len() {
        let h = order[hi];
        if consumed[h] {
            continue;
        }
        for &g in order.iter().skip(hi + 1) {
            if consumed[g] || g == h {
                continue;
            }
            let (hp, gp) = (&exits[h].parts, &exits[g].parts);
            if gp.len() > hp.len() || gp.len() < 2 {
                continue;
            }
            // `g`'s epilogue must be a suffix of `h`'s.
            if hp[hp.len() - gp.len()..] != gp[..] {
                continue;
            }
            let key = (h, gp.len());
            let label = if let Some(l) = host_labels.get(&key) {
                l.clone()
            } else {
                *label_seq += 1;
                let l = format!(".Lepi{}", *label_seq);
                // Plant the label at the instruction where the shared suffix
                // begins inside the host.
                let at = exits[h].line_of[hp.len() - gp.len()];
                store.replace(at, format!("{}:\n{}", l, store.get(at).trim_end()));
                infos[at] = classify_line(store.get(at));
                host_labels.insert(key, l.clone());
                l
            };
            let (s0, r0) = (exits[g].start, exits[g].ret_idx);
            store.replace(s0, format!("    jmp {}", label));
            infos[s0] = classify_line(store.get(s0));
            for k in (s0 + 1)..=r0 {
                mark_nop(&mut infos[k]);
            }
            consumed[g] = true;
            changed = true;
        }
    }
    changed
}

