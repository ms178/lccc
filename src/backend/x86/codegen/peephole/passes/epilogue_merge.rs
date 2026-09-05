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
//! Cross-jumping keeps one copy and turns every duplicate into a jump.  A
//! longest-common-suffix match additionally catches an exit which restores a
//! strict suffix of another exit's saves: it can jump directly into the middle
//! of the longer epilogue.
//!
//! # Soundness boundary
//!
//! This is deliberately a narrow *terminal* text transform.
//!
//! * Only SysV AMD64 callee-save restores (`popq %rbx/%rbp/%r12..%r15`),
//!   `addq $imm, %rsp`, and `leave` form a tail.  In particular, a temporary
//!   `popq %rdi` or `popq %rsp` is not mistaken for an ABI restore.
//! * A run containing a label is rejected: a branch may enter its middle.
//! * Grouping is per `.cfi_startproc` / `.cfi_endproc` pair, so a jump can
//!   never cross a function boundary.
//! * CFI is allowed only in the unconditional entry prefix.  A CFI directive
//!   after a body label or branch is path-specific unwind state; merging that
//!   function would make an exit jump to a PC with another path's CFI state,
//!   so the pass declines it.
//! * Labels are allocated from a file-wide occupied-name set.  Inline assembly
//!   is allowed to define any local symbol, including the old `.Lepi1` name;
//!   a generated target is therefore never allowed to collide with it.
//! * The pass runs after all structured peepholes.  No later line pass sees a
//!   synthetic `label + instruction` replacement as one opaque line.

use super::super::types::*;
use crate::common::fx_hash::{FxHashMap, FxHashSet};

/// One exit site: `[start, ret_idx]` is the epilogue run plus its `ret`.
struct Exit {
    start: usize,
    ret_idx: usize,
    /// Epilogue instruction texts in program order, excluding the `ret`.
    parts: Vec<String>,
    /// Line index of each entry in `parts`.
    line_of: Vec<usize>,
}

/// Whether `t` is one of the ABI restore instructions that can safely appear
/// in an independently shared epilogue suffix.
fn is_epilogue_insn(t: &str) -> bool {
    if t == "leave" {
        return true;
    }
    if let Some(rest) = t.strip_prefix("popq %") {
        // A return site may restore only the SysV AMD64 callee-saved GPRs.
        // Exact text matching would make a caller-saved temporary pop appear
        // harmless too, but suffix sharing is an ABI epilogue optimization,
        // not a license to cross-jump arbitrary stack-manipulating text.
        return matches!(rest, "rbx" | "rbp" | "r12" | "r13" | "r14" | "r15");
    }
    // `addq $N, %rsp`.  A `subq` here would allocate, not restore, a frame.
    if let Some(rest) = t.strip_prefix("addq $") {
        return rest.ends_with(", %rsp");
    }
    false
}

/// `true` when a CFI operation occurs after control flow has split into the
/// body.  Entry-prologue CFI is shared by every path and is stable; body/exit
/// CFI describes a particular dynamic stack state and is not.
fn has_path_sensitive_cfi(
    store: &LineStore,
    infos: &[LineInfo],
    fn_start: usize,
    fn_end: usize,
) -> bool {
    let mut entered_body = false;
    for k in (fn_start + 1)..fn_end {
        let text = infos[k].trimmed(store.get(k));
        if text.starts_with(".cfi_") {
            if entered_body {
                return true;
            }
            continue;
        }
        if infos[k].is_nop() {
            continue;
        }
        if matches!(
            infos[k].kind,
            LineKind::Label
                | LineKind::Jmp
                | LineKind::JmpIndirect
                | LineKind::CondJmp
                | LineKind::Ret
                | LineKind::InlineAsm
        ) {
            entered_body = true;
        }
    }
    false
}

/// Collect every existing label definition.  `LineStore` normally has exactly
/// one physical line per entry, but the routine deliberately splits embedded
/// replacements too: it remains correct if a future terminal pass runs after
/// another one that planted a label into a replacement slot.
fn occupied_label_names(store: &LineStore) -> FxHashSet<String> {
    let mut labels = FxHashSet::default();
    for i in 0..store.len() {
        for line in store.get(i).lines() {
            let text = line.trim();
            if let Some(label) = text.strip_suffix(':') {
                if !label.is_empty() {
                    labels.insert(label.to_string());
                }
            }
        }
    }
    labels
}

/// Allocate a local symbol that cannot collide with assembler or inline-asm
/// labels already present in the translation unit.
fn fresh_label(label_seq: &mut u64, occupied: &mut FxHashSet<String>) -> String {
    loop {
        *label_seq = label_seq
            .checked_add(1)
            .expect("epilogue label sequence exhausted");
        let label = format!(".L__lccc_epilogue_{}", *label_seq);
        if occupied.insert(label.clone()) {
            return label;
        }
    }
}

pub(super) fn merge_epilogue_tails(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    if std::env::var_os("CCC_NO_EPILOGUE_MERGE").is_some() {
        return false;
    }
    if store.is_empty() {
        return false;
    }

    let mut changed = false;
    let mut fn_start = None;
    // A sequence is file-global because GAS local symbols are file-global too.
    let mut label_seq = 0u64;
    let mut occupied = occupied_label_names(store);

    for i in 0..store.len() {
        let text = infos[i].trimmed(store.get(i));
        if text == ".cfi_startproc" {
            fn_start = Some(i);
            continue;
        }
        if text != ".cfi_endproc" {
            continue;
        }
        // Do not infer a function from malformed/unpaired CFI.  A malformed
        // hand-written assembly fragment must stay untouched, not inherit a
        // previous function's range.
        let Some(start) = fn_start.take() else {
            continue;
        };
        changed |= merge_one_function(store, infos, start, i, &mut label_seq, &mut occupied);
    }
    changed
}

fn merge_one_function(
    store: &mut LineStore,
    infos: &mut [LineInfo],
    fn_start: usize,
    fn_end: usize,
    label_seq: &mut u64,
    occupied: &mut FxHashSet<String>,
) -> bool {
    // The normal emitter emits only entry-prefix CFI.  Fail closed if a
    // hand-written or future precise-unwind function updates CFI in its CFG.
    if has_path_sensitive_cfi(store, infos, fn_start, fn_end) {
        return false;
    }

    // Collect exit sites.
    let mut exits: Vec<Exit> = Vec::new();
    for k in (fn_start + 1)..fn_end {
        if infos[k].is_nop() || infos[k].trimmed(store.get(k)) != "ret" {
            continue;
        }
        // Walk backwards over the epilogue run, skipping nops.  Stop before
        // labels/directives so the candidate is necessarily a whole basic-tail
        // fragment with no externally visible entry point in its interior.
        let mut start = k;
        let mut parts = Vec::new();
        let mut lines = Vec::new();
        let mut j = k;
        while j > fn_start + 1 {
            j -= 1;
            if infos[j].is_nop() || infos[j].kind == LineKind::Empty {
                continue;
            }
            let text = infos[j].trimmed(store.get(j));
            if text.ends_with(':') || text.starts_with('.') || !is_epilogue_insn(text) {
                break;
            }
            parts.push(text.to_string());
            lines.push(j);
            start = j;
        }
        // A two-instruction tail replaced by a jump is break-even only if it
        // includes `ret`; `parts` excludes it, so require two restore ops.
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

    // Longest host first, then source order for reproducible assembly.  A
    // short exit can jump into the deepest matching suffix of a longer one.
    let mut order: Vec<usize> = (0..exits.len()).collect();
    order.sort_unstable_by(|&a, &b| {
        exits[b]
            .parts
            .len()
            .cmp(&exits[a].parts.len())
            .then_with(|| exits[a].start.cmp(&exits[b].start))
    });

    let mut changed = false;
    let mut consumed = vec![false; exits.len()];
    // A host may serve several different suffix lengths.  Each length needs a
    // label at a distinct instruction, but all guests of the same length reuse
    // it rather than planting duplicate names at the same program point.
    let mut host_labels: FxHashMap<(usize, usize), String> = FxHashMap::default();
    for hi in 0..order.len() {
        let host = order[hi];
        if consumed[host] {
            continue;
        }
        for &guest in order.iter().skip(hi + 1) {
            if consumed[guest] || guest == host {
                continue;
            }
            let (host_parts, guest_parts) = (&exits[host].parts, &exits[guest].parts);
            if guest_parts.len() < 2 || guest_parts.len() > host_parts.len() {
                continue;
            }
            if host_parts[host_parts.len() - guest_parts.len()..] != guest_parts[..] {
                continue;
            }

            let key = (host, guest_parts.len());
            let label = if let Some(label) = host_labels.get(&key) {
                label.clone()
            } else {
                let label = fresh_label(label_seq, occupied);
                let at = exits[host].line_of[host_parts.len() - guest_parts.len()];
                store.replace(at, format!("{}:\n{}", label, store.get(at).trim_end()));
                infos[at] = classify_line(store.get(at));
                host_labels.insert(key, label.clone());
                label
            };

            let (start, ret_idx) = (exits[guest].start, exits[guest].ret_idx);
            store.replace(start, format!("    jmp {}", label));
            infos[start] = classify_line(store.get(start));
            for k in (start + 1)..=ret_idx {
                mark_nop(&mut infos[k]);
            }
            consumed[guest] = true;
            changed = true;
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn merge(asm: &str) -> (bool, String) {
        let mut store = LineStore::new(asm.to_string());
        let mut infos: Vec<LineInfo> = (0..store.len())
            .map(|i| classify_line(store.get(i)))
            .collect();
        let changed = merge_epilogue_tails(&mut store, &mut infos);
        let result = store.build_result(|i| infos[i].is_nop());
        (changed, result)
    }

    #[test]
    fn suffix_merge_jumps_to_the_deepest_compatible_restore() {
        let asm = concat!(
            "f:\n",
            ".cfi_startproc\n",
            "    subq $16, %rsp\n",
            "    .cfi_def_cfa_offset 24\n",
            ".Llong:\n",
            "    addq $16, %rsp\n",
            "    popq %rbp\n",
            "    popq %rbx\n",
            "    ret\n",
            ".Lshort:\n",
            "    popq %rbp\n",
            "    popq %rbx\n",
            "    ret\n",
            ".cfi_endproc\n",
        );
        let (changed, result) = merge(asm);
        assert!(changed, "{result}");
        assert!(
            result.contains(".L__lccc_epilogue_1:\n    popq %rbp"),
            "{result}"
        );
        assert!(
            result.contains(".Lshort:\n    jmp .L__lccc_epilogue_1"),
            "{result}"
        );
        assert_eq!(result.matches("    ret\n").count(), 1, "{result}");
    }

    #[test]
    fn generated_label_skips_an_inline_assembly_collision() {
        let asm = concat!(
            "f:\n",
            ".cfi_startproc\n",
            ".L__lccc_epilogue_1:\n",
            "    nop\n",
            ".Lfirst:\n",
            "    addq $8, %rsp\n",
            "    popq %rbx\n",
            "    ret\n",
            ".Lsecond:\n",
            "    addq $8, %rsp\n",
            "    popq %rbx\n",
            "    ret\n",
            ".cfi_endproc\n",
        );
        let (changed, result) = merge(asm);
        assert!(changed, "{result}");
        assert_eq!(
            result.matches(".L__lccc_epilogue_1:").count(),
            1,
            "{result}"
        );
        assert!(result.contains(".L__lccc_epilogue_2:"), "{result}");
        assert!(result.contains("jmp .L__lccc_epilogue_2"), "{result}");
    }

    #[test]
    fn path_sensitive_cfi_disables_the_whole_function() {
        let asm = concat!(
            "f:\n",
            ".cfi_startproc\n",
            "    subq $16, %rsp\n",
            "    .cfi_def_cfa_offset 24\n",
            ".Lbody:\n",
            "    .cfi_def_cfa_offset 16\n",
            ".Lfirst:\n",
            "    addq $16, %rsp\n",
            "    popq %rbx\n",
            "    ret\n",
            ".Lsecond:\n",
            "    addq $16, %rsp\n",
            "    popq %rbx\n",
            "    ret\n",
            ".cfi_endproc\n",
        );
        let (changed, result) = merge(asm);
        assert!(!changed, "{result}");
        assert!(!result.contains(".L__lccc_epilogue_"), "{result}");
        assert_eq!(result.matches("    ret\n").count(), 2, "{result}");
    }

    #[test]
    fn caller_saved_pop_is_not_an_epilogue_tail() {
        let asm = concat!(
            "f:\n",
            ".cfi_startproc\n",
            ".Lfirst:\n",
            "    addq $8, %rsp\n",
            "    popq %rdi\n",
            "    ret\n",
            ".Lsecond:\n",
            "    addq $8, %rsp\n",
            "    popq %rdi\n",
            "    ret\n",
            ".cfi_endproc\n",
        );
        let (changed, result) = merge(asm);
        assert!(!changed, "{result}");
        assert_eq!(result.matches("    ret\n").count(), 2, "{result}");
    }
}
