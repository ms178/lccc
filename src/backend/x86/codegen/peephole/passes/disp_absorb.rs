//! Constant pointer adjustment absorbed into the consumer's displacement.
//!
//! # The shape (RH-06)
//!
//! Instruction selection materialises a constant `GEP` as pointer
//! arithmetic, and the memory access that consumes it keeps a zero
//! displacement:
//!
//! ```text
//!     addq $256, %rdi
//!     vfmadd132ps (%rdi), %ymm1, %ymm0
//! ```
//!
//! x86 carries the constant in the addressing mode for free — GCC 16.2 and
//! Clang 23.1.5 both emit one instruction:
//!
//! ```text
//!     vfmadd132ps 256(%rdi), %ymm1, %ymm0
//! ```
//!
//! One instruction, one µop, one flags write and one register rename
//! lighter, and it removes an address-generation dependency from the load's
//! critical path.  The same fold applies to any scalar or vector consumer —
//! loads, stores, RMW ALU ops — and to `subq` with the sign flipped.
//!
//! # When it is legal
//!
//! 1. **Exact adjacency**: the `add{q}/sub{q} $imm, %rX` is followed by the
//!    consumer with only blank and debug-attribution lines between, both
//!    lines pinned-free, comment-free and volatile-free.  Adjacency is not
//!    cosmetic: every line the pair could straddle is one more thing that
//!    may read `%rX` or the flags.
//! 2. **`%rX` is a plain GP base**: never `%rsp`/`%rbp` (stack adjustments
//!    carry CFI meaning), and the consumer must reference the family
//!    exactly once, as the bare base of its sole memory operand —
//!    `D(%rX)` or `D(%rX, %i, s)`.  A second mention (register operand,
//!    RMW destination, index) would still read the un-adjusted value.
//! 3. **The displacement stays encodable**: `D + imm` folds only when the
//!    result fits in the 32-bit displacement field.
//! 4. **The adjustment's flags are not observed**: deleting `addq` deletes
//!    its flags write, so [`flags_dead_after`] — the single oracle every
//!    flag-discarding transform consults — must prove them dead.
//! 5. **`%rX` dies at the consumer**: the un-adjusted value must never be
//!    read again.  Within the block [`dead_in_block_after`] proves it; a
//!    `ret` directly after the consumer proves it for caller-saved
//!    families (the returning function cannot read its own caller-saved
//!    registers past `ret`, and a callee-saved family that still carried a
//!    live computed value at `ret` would already be a miscompile).
//!
//! `CCC_PEEPHOLE_SKIP=disp_absorb` disables the pass for bisection.

use super::super::types::*;
use super::flag_peepholes::flags_dead_after;
use super::helpers::implicit_read_reg_family;
use super::relay_and_lea::{
    dead_in_block_after, is_relayable_family, line_refs_family, plain_gp_operand,
    split_two_operands,
};

/// Fold every eligible `add/sub $imm, %rX` + memory consumer pair.
pub(super) fn run(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    let mut changed = false;
    let mut i = 0;
    while i < infos.len() {
        if infos[i].is_nop() {
            i += 1;
            continue;
        }
        if let Some((use_i, new_text)) = absorb_pair(store, infos, i) {
            replace_line(store, &mut infos[use_i], use_i, new_text);
            mark_nop(&mut infos[i]);
            changed = true;
            i = use_i + 1;
            continue;
        }
        i += 1;
    }
    changed
}

/// Detect the pair anchored at the `add`/`sub` on line `adj_i`; returns the
/// consumer's line index and its rewritten text.
fn absorb_pair(store: &LineStore, infos: &[LineInfo], adj_i: usize) -> Option<(usize, String)> {
    if infos[adj_i].pinned || infos[adj_i].volatile_access {
        return None;
    }
    let raw = store.get(adj_i);
    if raw.contains('#') {
        return None;
    }
    let adj = infos[adj_i].trimmed(raw);
    let (imm, reg) = imm_adjust(adj)?;
    let fam = plain_gp_operand(reg)?;
    // Relayable = plain renameable GP: excludes %rsp/%rbp and APX/eGPR
    // families the operand oracle does not model.
    if !is_relayable_family(fam) {
        return None;
    }

    // Next real instruction: blank/debug lines may interleave, nothing else.
    let mut use_i = adj_i + 1;
    while use_i < infos.len() && infos[use_i].is_steppable() {
        use_i += 1;
    }
    if use_i >= infos.len() || infos[use_i].pinned || infos[use_i].volatile_access {
        return None;
    }
    // Only a plain in-block instruction may consume: a call/jump target, a
    // label, inline asm or a directive all have semantics beyond "one memory
    // access here".
    if !matches!(infos[use_i].kind, LineKind::Other { .. } | LineKind::Cmp) {
        return None;
    }
    let raw_use = store.get(use_i);
    if raw_use.contains('#') || raw_use.contains('*') {
        return None;
    }
    let use_line = infos[use_i].trimmed(raw_use);
    // The implicit-operand screen: `mul`, `div`, string ops and friends read
    // registers the textual scan below cannot see.
    if implicit_read_reg_family(use_line).is_some() {
        return None;
    }
    let (before, disp, mem_and_tail, tail) = split_base_mem_operand(use_line, reg)?;
    // The family must appear exactly once: excise the memory operand and
    // require the remainder to be family-free (a register operand, an RMW
    // destination or an index would read the un-adjusted value).
    let remainder = format!("{before} {tail}");
    if line_refs_family(&remainder, fam) {
        return None;
    }
    let total = disp.checked_add(imm)?;
    i32::try_from(total).ok()?;

    // The adjustment's flags write disappears with it.
    if !flags_dead_after(store, infos, adj_i + 1) {
        return None;
    }

    // The un-adjusted `%rX` must die at the consumer.
    if !family_dead_after(store, infos, use_i + 1, fam) {
        return None;
    }
    if !all_editable(infos, &[adj_i, use_i]) {
        return None;
    }

    let disp_txt = if total == 0 {
        String::new()
    } else {
        total.to_string()
    };
    Some((use_i, format!("    {before}{disp_txt}{mem_and_tail}")))
}

/// `add{q} $imm, %reg` / `sub{q} $imm, %reg` → `(signed imm, reg)`.
///
/// Only the 64-bit width: a 32-bit `addl` truncates the register, which is
/// never how a pointer adjustment is spelled, and folding it would widen the
/// arithmetic.
fn imm_adjust(t: &str) -> Option<(i64, &str)> {
    let (mnem, rest) = t.split_once(char::is_whitespace)?;
    let negate = match mnem {
        "addq" => false,
        "subq" => true,
        _ => return None,
    };
    let (src, dst) = split_two_operands(rest)?;
    let imm: i64 = src.strip_prefix('$')?.parse().ok()?;
    let imm = if negate { imm.checked_neg()? } else { imm };
    Some((imm, dst.trim()))
}

/// Split `use_line` around its single `D(%reg…)` memory operand whose BASE
/// is `reg`: returns `(text before the displacement, D, the memory operand
/// with everything after it, the text after the operand alone)`.  Declines
/// when the line has no such operand or more than one `(`.
fn split_base_mem_operand<'a>(
    line: &'a str,
    reg: &str,
) -> Option<(&'a str, i64, &'a str, &'a str)> {
    let open = line.find('(')?;
    if line[open + 1..].contains('(') {
        return None; // two memory-ish operands: not the simple shape
    }
    let close = open + line[open..].find(')')?;
    let inner = &line[open + 1..close];
    let base = inner.split(',').next()?.trim();
    if base != reg {
        return None; // the register is the index, not the base
    }
    // Displacement: the numeric run directly before '('.
    let head = &line[..open];
    let disp_start = head
        .rfind(|c: char| !c.is_ascii_digit() && c != '-')
        .map_or(0, |p| p + 1);
    let disp_txt = &head[disp_start..];
    let disp: i64 = if disp_txt.is_empty() {
        0
    } else {
        disp_txt.parse().ok()?
    };
    Some((&line[..disp_start], disp, &line[open..], &line[close + 1..]))
}

/// `%fam` is dead once control passes `from`: either the in-block scan
/// proves a full overwrite before any read, or the next real line is `ret`
/// and the family is caller-saved (the returning function cannot observe
/// its own caller-saved registers past `ret`).
fn family_dead_after(store: &LineStore, infos: &[LineInfo], from: usize, fam: RegId) -> bool {
    let mut n = from;
    while n < store.len() && (infos[n].is_steppable() || infos[n].is_nop()) {
        n += 1;
    }
    if n < store.len() && infos[n].kind == LineKind::Ret && callersaved(fam) {
        return true;
    }
    dead_in_block_after(store, infos, from, fam)
}

/// SysV caller-saved GP families, in REG_NAMES order: rax(0), rcx(1),
/// rdx(2), rsi(6), rdi(7), r8–r11(8–11).
fn callersaved(fam: RegId) -> bool {
    matches!(fam, 0 | 1 | 2 | 6 | 7 | 8 | 9 | 10 | 11)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_pass(asm: &str) -> String {
        let mut store = LineStore::new(asm.to_string());
        let mut infos: Vec<LineInfo> = (0..store.len())
            .map(|i| classify_line(store.get(i)))
            .collect();
        run(&mut store, &mut infos);
        (0..store.len())
            .filter(|&i| !infos[i].is_nop())
            .map(|i| store.get(i).trim().to_string())
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn frame(body: &str) -> String {
        format!("f:\n.cfi_startproc\n{body}.cfi_endproc\n")
    }

    #[test]
    fn the_motivating_fma_shape_folds() {
        let out = run_pass(&frame(concat!(
            "    addq $256, %rdi\n",
            "    vfmadd132ps (%rdi), %ymm1, %ymm0\n",
            "    ret\n",
        )));
        assert!(out.contains("vfmadd132ps 256(%rdi), %ymm1, %ymm0"), "{out}");
        assert!(!out.contains("addq"), "{out}");
    }

    #[test]
    fn an_existing_displacement_accumulates() {
        let out = run_pass(&frame(concat!(
            "    addq $32, %rcx\n",
            "    movq 8(%rcx), %rax\n",
            "    ret\n",
        )));
        assert!(out.contains("movq 40(%rcx), %rax"), "{out}");
        assert!(!out.contains("addq"), "{out}");
    }

    #[test]
    fn subq_folds_with_the_sign_flipped() {
        let out = run_pass(&frame(concat!(
            "    subq $16, %rsi\n",
            "    movl (%rsi), %eax\n",
            "    ret\n",
        )));
        assert!(out.contains("movl -16(%rsi), %eax"), "{out}");
        assert!(!out.contains("subq"), "{out}");
    }

    #[test]
    fn a_store_consumer_folds() {
        let out = run_pass(&frame(concat!(
            "    addq $24, %rdx\n",
            "    movq %rax, (%rdx)\n",
            "    ret\n",
        )));
        assert!(out.contains("movq %rax, 24(%rdx)"), "{out}");
    }

    #[test]
    fn a_sib_base_folds() {
        let out = run_pass(&frame(concat!(
            "    addq $64, %rdi\n",
            "    movl (%rdi, %rax, 4), %ecx\n",
            "    ret\n",
        )));
        assert!(out.contains("movl 64(%rdi, %rax, 4), %ecx"), "{out}");
    }

    #[test]
    fn a_zero_total_displacement_is_spelled_bare() {
        let out = run_pass(&frame(concat!(
            "    addq $8, %rdi\n",
            "    movq -8(%rdi), %rax\n",
            "    ret\n",
        )));
        assert!(out.contains("movq (%rdi), %rax"), "{out}");
    }

    #[test]
    fn a_live_register_after_the_consumer_declines() {
        let out = run_pass(&frame(concat!(
            "    addq $8, %rdi\n",
            "    movq (%rdi), %rax\n",
            "    movq %rdi, %rdx\n",
            "    ret\n",
        )));
        assert!(out.contains("addq $8, %rdi"), "{out}");
    }

    #[test]
    fn a_second_mention_in_the_consumer_declines() {
        // RMW destination: `addq (%rdi), %rdi` reads the un-adjusted value.
        let out = run_pass(&frame(concat!(
            "    addq $8, %rdi\n",
            "    addq (%rdi), %rdi\n",
            "    movq %rdi, %rax\n",
            "    ret\n",
        )));
        assert!(out.contains("addq $8, %rdi"), "{out}");
    }

    #[test]
    fn the_register_as_index_declines() {
        let out = run_pass(&frame(concat!(
            "    addq $8, %rdi\n",
            "    movl (%rax, %rdi, 4), %ecx\n",
            "    ret\n",
        )));
        assert!(out.contains("addq $8, %rdi"), "{out}");
    }

    #[test]
    fn observed_flags_decline() {
        // The consumer itself reads the addq's flags.
        let out = run_pass(&frame(concat!(
            "    addq $8, %rdi\n",
            "    cmovcq (%rdi), %rax\n",
            "    ret\n",
        )));
        assert!(out.contains("addq $8, %rdi"), "{out}");
    }

    #[test]
    fn a_later_flag_reader_declines() {
        let out = run_pass(&frame(concat!(
            "    addq $8, %rdi\n",
            "    movq (%rdi), %rdi\n",
            "    jc .L1\n",
            ".L1:\n",
            "    ret\n",
        )));
        assert!(out.contains("addq $8, %rdi"), "{out}");
    }

    #[test]
    fn rsp_and_rbp_never_fold() {
        let out = run_pass(&frame(concat!(
            "    addq $16, %rsp\n",
            "    movq (%rsp), %rax\n",
            "    ret\n",
        )));
        assert!(out.contains("addq $16, %rsp"), "{out}");
    }

    #[test]
    fn a_callee_saved_base_before_ret_declines() {
        // %rbx would need an in-block overwrite proof; `ret` alone is not one.
        let out = run_pass(&frame(concat!(
            "    addq $8, %rbx\n",
            "    movq (%rbx), %rax\n",
            "    ret\n",
        )));
        assert!(out.contains("addq $8, %rbx"), "{out}");
    }

    #[test]
    fn an_intervening_instruction_declines() {
        let out = run_pass(&frame(concat!(
            "    addq $8, %rdi\n",
            "    movq %rsi, %rcx\n",
            "    movq (%rdi), %rax\n",
            "    ret\n",
        )));
        assert!(out.contains("addq $8, %rdi"), "{out}");
    }

    #[test]
    fn an_addl_width_mismatch_declines() {
        let out = run_pass(&frame(concat!(
            "    addl $8, %edi\n",
            "    movq (%rdi), %rax\n",
            "    ret\n",
        )));
        assert!(out.contains("addl $8, %edi"), "{out}");
    }

    #[test]
    fn a_call_consumer_declines() {
        let out = run_pass(&frame(concat!(
            "    addq $8, %rdi\n",
            "    call g@PLT\n",
            "    ret\n",
        )));
        assert!(out.contains("addq $8, %rdi"), "{out}");
    }
}
