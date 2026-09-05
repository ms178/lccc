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

/// One exit site: `[start, ret_idx]` is the epilogue run plus its `ret`.
struct Exit {
    start: usize,
    ret_idx: usize,
    key: String,
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
            start = j;
        }
        if parts.len() < 2 {
            continue;
        }
        parts.reverse();
        let key = parts.join(";");
        exits.push(Exit {
            start,
            ret_idx: k,
            key,
        });
    }
    if exits.len() < 2 {
        return false;
    }

    // Group by identical epilogue text, keeping source order.
    let mut changed = false;
    let mut handled: Vec<bool> = vec![false; exits.len()];
    for a in 0..exits.len() {
        if handled[a] {
            continue;
        }
        let mut group: Vec<usize> = vec![a];
        for b in (a + 1)..exits.len() {
            if !handled[b] && exits[b].key == exits[a].key {
                group.push(b);
            }
        }
        if group.len() < 2 {
            continue;
        }
        for &g in &group {
            handled[g] = true;
        }
        *label_seq += 1;
        let label = format!(".Lepi{}", *label_seq);

        // Label the first member in the group (the shared copy).
        let head = &exits[group[0]];
        let head_start = head.start;
        store.replace(
            head_start,
            format!("{}:\n{}", label, store.get(head_start).trim_end()),
        );
        infos[head_start] = classify_line(store.get(head_start));

        // Replace every other member's whole run with one `jmp`.
        for &g in &group[1..] {
            let e = &exits[g];
            let (s, r) = (e.start, e.ret_idx);
            store.replace(s, format!("    jmp {}", label));
            infos[s] = classify_line(store.get(s));
            for k in (s + 1)..=r {
                mark_nop(&mut infos[k]);
            }
            changed = true;
        }
    }
    changed
}
