//! MOVBE fusion: `load` + `bswap` on the same value → one `movbe`.
//!
//! ## The shape
//!
//! ```text
//!     movq  (%rdi), %rax               movbeq (%rdi), %rax
//!     bswap %rax                 ->
//! ```
//!
//! Both instructions are emitted by `byte_load`'s big-endian rewrite (and by
//! `__builtin_bswap{16,32,64}` around a load in hand-written code), and the
//! oracle says the fused form is the one to have: for
//! `get_unaligned_be64`-shaped code at `-march=x86-64-v3`, GCC 16.2 and ICX
//! both emit `movbeq (%rdi), %rax` (2 instructions including `ret`), while
//! LCCC emitted `movq` + `bswapq` (3 + `ret`) and Clang 23.1 emitted a
//! twelve-instruction `vpmovzxbq`/`vpsllvq`/`vpor` sequence.  MOVBE is part of
//! the x86-64-v3 psABI level alongside AVX2/BMI/FMA, so the baseline this
//! project's code generation targets has it.
//!
//! ## Soundness
//!
//! No liveness proof is needed, and here is why — the two writes are to the
//! *same register* and there is nothing in between:
//!
//! * The pair must be *adjacent*: the `bswap` is the next real instruction
//!   (`next_insn` steps over only nops and inert directives).  So no
//!   instruction, and no label, can sit between the load and the swap — which
//!   also means no other instruction can observe the un-swapped datum, no
//!   control-flow edge can enter between them, and an edge entering at the
//!   load executes the same computation in both forms.
//! * The load's destination register is exactly the one `bswap` swaps.
//!   `bswap` is register-to-register only, so a swap of a differently-spelled
//!   register would have to travel through a copy, and the loaded value would
//!   be live across that copy — such a pair is refused (the widths and the
//!   register families must match exactly).
//! * Therefore, after the pair, the register holds the byte-reversed datum in
//!   both forms; before the pair, neither form has written it; and no reader
//!   exists in between.  A reader *after* the swap is the reason the fold is
//!   worth doing, so requiring the un-swapped value to be dead would be
//!   backwards — an earlier revision of this pass demanded exactly that and
//!   refused every shape it was written for (`be64(x)` returning the loaded
//!   value is a reader by definition).  The unit test
//!   `fuses_a_big_endian_load` is the regression cover.
//! * Widths: `movbew`/`movbel`/`movbeq` load 2/4/8 bytes and swap them, which
//!   is what `bswapw`/`bswapl`/`bswapq` on a value loaded with
//!   `movw`/`movl`/`movq` computes.  No width changes.
//! * Memory semantics are preserved: one load of the same width from the same
//!   addressing mode, at the same place in program order.  A `%gs:`/`%fs:`
//!   qualified operand keeps its segment prefix on the fused instruction.  If
//!   the address is invalid the fault now happens on the `movbe` instead of
//!   the `mov` — same address, same access size, so the same trap.
//! * The load must not be `pinned` and the swap must not be `pinned`
//!   ([`all_editable`]): pinned lines carry semantics this pass cannot see
//!   (parameter pre-stores, inline-asm regions).
//! * MOVBE must be legal for the TU ([`X86Isa::movbe`], from `-mmovbe` /
//!   `-mno-movbe` / the `-march=` baseline).  `-mno-movbe`, `-march=x86-64`
//!   and every non-x86-64 target keep the pair.
//! * `-Os`/`-Oz` (`optimize_for_size`) does not veto the fusion: `movbeq
//!   (%rdi), %rax` and `movq (%rdi), %rax; bswapq %rax` are both 6 bytes of
//!   encoding, and the fused form is one instruction shorter.  (GCC 16.2
//!   emits `movbe` at `-Os` too, for the same reason.)  The pass therefore has
//!   no size gate, only the permission gate and its skip name.

use super::super::types::*;
use super::liveness::FileLiveness;
use super::load_op_fuse::{is_plain_memory_operand, next_insn};
use super::relay_and_lea::provably_dead_lv;
use super::relay_and_lea::{plain_gp_operand, split_two_operands};

/// Parse `mov{w,l,q} MEM, %reg` into `(width, reg)`.  Only plain GP
/// destinations spelled exactly as their bank (the shape the backend emits),
/// and only plain memory operands: no register source, no immediate, no
/// symbol-without-parenthesis form.
fn parse_byte_load(t: &str) -> Option<(u8, RegId, &str)> {
    let (width, rest) = if let Some(r) = t.strip_prefix("movw ") {
        (2u8, r)
    } else if let Some(r) = t.strip_prefix("movl ") {
        (4, r)
    } else if let Some(r) = t.strip_prefix("movq ") {
        (8, r)
    } else {
        return None;
    };
    let (src, dst) = split_two_operands(rest)?;
    if !is_plain_memory_operand(src) {
        return None;
    }
    let fam = plain_gp_operand(dst)?;
    let bank = if width == 8 { 0 } else { 1 };
    if dst != REG_NAMES[bank][fam as usize] {
        return None;
    }
    Some((width, fam, src))
}

/// Parse `bswap{w,l,q} %reg` into `(width, reg)`.
fn parse_bswap(t: &str) -> Option<(u8, RegId)> {
    let (width, rest) = if let Some(r) = t.strip_prefix("bswapw ") {
        (2u8, r)
    } else if let Some(r) = t.strip_prefix("bswapl ") {
        (4, r)
    } else if let Some(r) = t.strip_prefix("bswapq ") {
        (8, r)
    } else {
        return None;
    };
    let reg = rest.trim();
    let fam = plain_gp_operand(reg)?;
    let bank = if width == 8 { 0 } else { 1 };
    if reg != REG_NAMES[bank][fam as usize] {
        return None;
    }
    Some((width, fam))
}

/// Parse `mov{w,l,q} %S, %D` into `(width, src_fam, dst_fam)` for a plain
/// register copy.  Only the exact bank spellings the backend emits.
fn parse_reg_copy(t: &str) -> Option<(u8, RegId, RegId)> {
    let (width, rest) = if let Some(r) = t.strip_prefix("movw ") {
        (2u8, r)
    } else if let Some(r) = t.strip_prefix("movl ") {
        (4, r)
    } else if let Some(r) = t.strip_prefix("movq ") {
        (8, r)
    } else {
        return None;
    };
    let (src, dst) = split_two_operands(rest)?;
    let s_fam = plain_gp_operand(src)?;
    let d_fam = plain_gp_operand(dst)?;
    let bank = if width == 8 { 0 } else { 1 };
    if src != REG_NAMES[bank][s_fam as usize] || dst != REG_NAMES[bank][d_fam as usize] {
        return None;
    }
    Some((width, s_fam, d_fam))
}

/// Rewrite `mov{w,l,q} MEM, %T; bswap{w,l,q} %T` into `movbe{w,l,q} MEM, %T`
/// for every such pair whose un-swapped value is dead.  Returns whether
/// anything changed; one call reaches a fixpoint for independent pairs (the
/// rewrite only ever shortens a window).
///
/// The MOVBE permission is a parameter rather than a read of the process-global
/// ISA mirror: the caller owns the TU's options (and unit tests can exercise
/// both arms without racing each other through `set_current`, which a global
/// read would — the first revision of these tests failed exactly that way).
pub(super) fn fuse_load_bswap(store: &mut LineStore, infos: &mut [LineInfo], movbe: bool) -> bool {
    // On a target without MOVBE the whole scan is skipped, so `-mno-movbe`
    // output is byte-identical.
    if !movbe {
        return false;
    }
    let len = infos.len();
    let mut changed = false;
    let mut i = 0;
    while i < len {
        if infos[i].is_nop() {
            i += 1;
            continue;
        }
        let line = infos[i].trimmed(store.get(i));
        let Some((l_width, l_fam, mem)) = parse_byte_load(line) else {
            i += 1;
            continue;
        };
        let j = next_insn(store, infos, i + 1, len);
        if j >= len {
            i += 1;
            continue;
        }
        let Some((b_width, b_fam)) = parse_bswap(infos[j].trimmed(store.get(j))) else {
            i += 1;
            continue;
        };
        if l_width != b_width || l_fam != b_fam {
            i += 1;
            continue;
        }
        // Both lines must be editable: `mark_nop` silently keeps a pinned line
        // (parameter pre-stores, inline-asm regions), which would leave `%T`
        // written by the load and never re-swapped.
        if !all_editable(infos, &[i, j]) {
            i += 1;
            continue;
        }
        let suffix = match l_width {
            2 => "w",
            4 => "l",
            _ => "q",
        };
        let bank = if l_width == 8 { 0 } else { 1 };
        // The allocator's copy of the loaded value (`movq %rax, %rdi`) is now
        // a copy of the *swapped* value, so it can be coalesced into the fused
        // instruction's destination -- `movbeq MEM, %T; movq %T, %D` becomes
        // `movbeq MEM, %D`, which is the oracle's exact form (`movbeq (%rdi),
        // %rax`).  The proof is the neighbouring retarget transform's:
        // liveness says `%T` is dead after the copy, with the syntactic
        // whole-function fallbacks (`provably_dead_lv`).  Done here because the
        // pair only becomes visible at this point in the pipeline; the phase-1
        // retarget saw `bswapq %T`, which is not one of its producers.
        let mut new_dest = REG_NAMES[bank][l_fam as usize];
        let k = next_insn(store, infos, j + 1, len);
        if k < len
            && all_editable(infos, &[i, k])
            && let Some((c_width, c_src, c_dst)) = parse_reg_copy(infos[k].trimmed(store.get(k)))
            && c_src == l_fam
            && c_dst != l_fam
            && c_width == l_width
        {
            let lv = FileLiveness::new(store, infos);
            if provably_dead_lv(&lv, store, infos, k, l_fam, &[i, k]) {
                new_dest = REG_NAMES[bank][c_dst as usize];
                mark_nop(&mut infos[k]);
            }
        }
        let new_line = format!("    movbe{suffix} {mem}, {new_dest}");
        replace_line(store, &mut infos[i], i, new_line);
        mark_nop(&mut infos[j]);
        changed = true;
        i = j + 1;
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_with_permission(asm: &str, movbe: bool) -> String {
        let mut store = LineStore::new(asm.to_string());
        let mut infos: Vec<LineInfo> = (0..store.len())
            .map(|i| classify_line(store.get(i)))
            .collect();
        fuse_load_bswap(&mut store, &mut infos, movbe);
        store.build_result(|i| infos[i].is_nop())
    }

    fn run(asm: &str) -> String {
        run_with_permission(asm, true)
    }

    #[test]
    fn fuses_a_big_endian_load() {
        let out = run("f:\n    movq (%rdi), %rax\n    bswapq %rax\n    ret\n");
        assert!(out.contains("movbeq (%rdi), %rax"), "{out}");
        assert!(!out.contains("bswap"), "{out}");
        assert_eq!(out.matches("movbe").count(), 1);
    }

    #[test]
    fn coalesces_the_allocators_copy_into_the_destination() {
        // `movbeq MEM, %T; movq %T, %D` -> `movbeq MEM, %D`: the copy exists
        // because the allocator homed the value after the swap, and the
        // oracle's own shape (`movbeq (%rdi), %rax`) has no such copy.
        // A realistic function frame: the deadness proof (liveness, then the
        // syntactic fallbacks) needs the symbol boundary to know where the
        // function ends.
        let out = run(concat!(
            ".globl f\n.type f, @function\nf:\n.cfi_startproc\n",
            "    movq (%rdi), %rdi\n    bswapq %rdi\n    movq %rdi, %rax\n    ret\n",
            ".cfi_endproc\n.size f, .-f\n"
        ));
        assert!(out.contains("movbeq (%rdi), %rax"), "{out}");
        assert!(!out.contains("bswap"), "{out}");
        assert_eq!(out.matches("movq").count(), 0, "no copy survives: {out}");
    }

    #[test]
    fn keeps_the_copy_when_the_intermediate_is_still_live() {
        // `%rdi` is read again after the copy, so folding the copy into the
        // movbe would destroy its value.
        let asm = concat!(
            ".globl f\n.type f, @function\nf:\n.cfi_startproc\n",
            "    movq (%rdi), %rdi\n    bswapq %rdi\n    movq %rdi, %rax\n",
            "    addq %rdi, %rcx\n    ret\n.cfi_endproc\n.size f, .-f\n"
        );
        let out = run(asm);
        assert!(out.contains("movbeq (%rdi), %rdi"), "{out}");
        assert!(out.contains("movq %rdi, %rax"), "{out}");
    }

    #[test]
    fn keeps_the_copy_when_it_is_a_width_mismatch() {
        // A 32-bit copy of a 64-bit movbe destination would drop the high half.
        let asm = "f:\n    movq (%rdi), %rdi\n    bswapq %rdi\n    movl %edi, %eax\n    ret\n";
        let out = run(asm);
        assert!(!out.contains("movbeq (%rdi), %rax"), "{out}");
    }

    #[test]
    fn fuses_each_width() {
        let out = run(concat!(
            "f:\n    movw (%rdi), %eax\n    bswapw %eax\n",
            "g:\n    movl (%rdi), %eax\n    bswapl %eax\n",
            "h:\n    movq 8(%rdi,%rsi,2), %r9\n    bswapq %r9\n"
        ));
        assert!(out.contains("movbew (%rdi), %eax"), "{out}");
        assert!(out.contains("movbel (%rdi), %eax"), "{out}");
        assert!(out.contains("movbeq 8(%rdi,%rsi,2), %r9"), "{out}");
    }

    #[test]
    fn keeps_the_pair_when_movbe_is_not_legal() {
        let asm = "f:\n    movq (%rdi), %rax\n    bswapq %rax\n    ret\n";
        let out = run_with_permission(asm, false);
        assert_eq!(out, asm, "-march=x86-64 keeps mov+bswap");
    }

    #[test]
    fn keeps_the_pair_when_a_reader_stands_between() {
        // `%eax` is read between the load and the swap: the un-swapped datum is
        // observable, so the pair is not a MOVBE.  (Adjacency alone refuses it,
        // which is the point: there is no liveness judgement to get wrong.)
        let asm = "f:\n    movl (%rdi), %eax\n    addl %eax, %ecx\n    bswapl %eax\n    ret\n";
        let out = run(asm);
        assert!(out.contains("movl (%rdi), %eax"), "{out}");
        assert!(!out.contains("movbel"), "{out}");
    }

    #[test]
    fn fuses_even_though_the_result_is_live_and_returned() {
        // The result of the pair is the function's return value: it is live by
        // construction, and that is exactly the shape the oracle folds.  An
        // earlier revision required the un-swapped value to be dead and refused
        // every one of these.
        let asm = "f:\n    movl (%rdi), %eax\n    bswapl %eax\n    addl %eax, %ecx\n    ret\n";
        let out = run(asm);
        assert!(out.contains("movbel (%rdi), %eax"), "{out}");
        assert!(!out.contains("bswap"), "{out}");
    }

    #[test]
    fn keeps_the_pair_when_a_label_stands_between() {
        // A label can be a jump target; deleting the swap would then change
        // what an edge into it computes.
        let asm = "f:\n    movq (%rdi), %rax\n.Lmid:\n    bswapq %rax\n    ret\n";
        let out = run(asm);
        assert!(!out.contains("movbe"), "{out}");
    }

    #[test]
    fn keeps_a_non_adjacent_pair() {
        let asm = "f:\n    movq (%rdi), %rax\n    movq %rax, %rcx\n    bswapq %rax\n    ret\n";
        let out = run(asm);
        assert!(!out.contains("movbe"), "{out}");
    }

    #[test]
    fn keeps_a_width_mismatch() {
        // `bswapl` on a byte-reversed 16-bit load is not a MOVBE of either
        // width; declining is the only sound answer.
        let asm = "f:\n    movw (%rdi), %eax\n    bswapl %eax\n    ret\n";
        let out = run(asm);
        assert!(!out.contains("movbe"), "{out}");
    }

    #[test]
    fn keeps_a_pinned_load() {
        let asm = "f:\n#APP\n    movq (%rdi), %rax\n#NO_APP\n    bswapq %rax\n    ret\n";
        let out = run(asm);
        assert!(!out.contains("movbe"), "{out}");
    }

    #[test]
    fn keeps_a_segment_prefix_on_the_fused_instruction() {
        // `%gs:` must survive the fusion: MOVBE takes the same segment
        // override, and dropping it would read a different address.  (The
        // segment prefix is spliced through verbatim, and the assembler
        // encodes it -- checked against `as` in the movbe gate.)
        let out = run("f:\n    movq %gs:16(%rax), %rcx\n    bswapq %rcx\n    ret\n");
        assert!(out.contains("movbeq %gs:16(%rax), %rcx"), "{out}");
    }

    #[test]
    fn leaves_an_absolute_segment_operand_alone() {
        // `%gs:16` (no parenthesised base) is not accepted by the shared
        // plain-memory-operand test, which exists to keep windows narrow; the
        // pair is therefore kept rather than risk a rewrite whose encoding is
        // not the same shape.
        let asm = "f:\n    movq %gs:16, %rax\n    bswapq %rax\n    ret\n";
        let out = run(asm);
        assert_eq!(out, asm);
    }
}
