//! Branching on a difference is branching on disequality.
//!
//! `compare256`-style scanners (zlib-ng, zstd's match finder) XOR two loaded
//! words, branch on the result, and consume the difference only on the
//! mismatch path:
//!
//! ```c
//! diff = sv ^ mv;
//! if (diff)
//!     return len + (uint32_t)(__builtin_ctzll(diff) >> 3);
//! ```
//!
//! Branching on `diff` keeps the XOR on the hot all-equal path: x86 lowers it
//! as `xor` + `jne`, a pair that never macro-fuses, and the XOR destroys one
//! of the loaded values. Branching on `sv != mv` instead costs one fusible
//! `cmp` + `jne`, and — once the branch no longer reads the difference — the
//! XOR's only remaining consumer is the mismatch arm, so `expr_sink` moves it
//! off the hot path entirely. GCC 16.2 and Clang 23.1.5 both emit the `cmp`
//! form for the source above; measured on zlib_ng_compare256 the `cmp` shape
//! recovers a third of the gap to GCC's kernel.
//!
//! Two shapes, both unconditional (the rewrite never adds work to a path
//! that did not already compute the difference, and where the difference
//! stays live the swap trades a non-fusing flag producer for a fusing one):
//!
//!  * `CondBranch(d)` where `d = a ^ b` or `d = a - b`: a fresh
//!    `Cmp Ne(a, b)` is appended to the branching block and branched on.
//!    `a` and `b` dominate the definition of `d`, which dominates this use,
//!    so both are available at the branch.
//!  * `Cmp Eq/Ne(d, 0)`: rewritten in place to `Cmp Eq/Ne(a, b)`. Wrapping
//!    `Sub` and `Xor` are zero exactly on equal inputs, so the predicate is
//!    unchanged on every input; a `Sub` with undefined signed overflow is
//!    only ever *more* defined after the rewrite.
//!
//! The difference's own definition is never touched: a dead difference is
//! DCE's to collect, a live one keeps its value and its location. (Moving
//! the XOR into the mismatch arm at the *source* level is not equivalent —
//! it changed which values stay in registers across the loop and regressed
//! badly; the branch condition is the only thing this pass changes.)
//!
//! `cvp` keeps the `CtzNonZero` promotion across this rewrite: a dominating
//! `a != b` edge proves `a ^ b` and `a - b` nonzero (its
//! `difference_proves_nonzero`), exactly as the pre-rewrite `CondBranch(d)`
//! edge proved `d` itself nonzero. Running this rewrite therefore never
//! costs the bare `bsf`/`tzcnt` lowering the old shape earned.
//!
//! Kill switch: `CCC_NO_DIFF_BRANCH` (resolved once, in
//! `bit_idioms_facts.rs`, next to the other `CCC_NO_*` gates).

use super::*;
use crate::common::source::Span;
use crate::ir::reexports::Terminator;

/// The `(lhs, rhs, ty)` of a value's unique defining `Xor`/`Sub` over an
/// integer ring of at most 64 bits. 128-bit rings are refused: a 128-bit
/// compare-and-branch expands to a multi-word sequence whose cost is not the
/// single fusible `cmp` this rewrite is priced on.
#[derive(Clone, Copy)]
struct DiffDef {
    op: IrBinOp,
    lhs: Operand,
    rhs: Operand,
    ty: IrType,
}

/// Rewrite zero-tests of two-operand differences into direct comparisons of
/// the operands. Returns the number of rewrites. Runs before
/// `recognize_function`'s def/use snapshot is taken: the in-place `Cmp`
/// rewrite keeps every dest and every semantic, but the appended `Cmp`
/// mints fresh ids and adds operand reads that a stale single-use proof
/// must never miss.
pub(super) fn rewrite_diff_zero_branches(func: &mut IrFunction) -> usize {
    let max = func.max_value_id() as usize + 1;
    let mut diffs: Vec<Option<DiffDef>> = vec![None; max];
    let mut defined = vec![false; max];
    let mut uses: Vec<u32> = vec![0; max];
    for block in &func.blocks {
        for inst in &block.instructions {
            inst.for_each_used_value(|v| {
                if let Some(cnt) = uses.get_mut(v as usize) {
                    *cnt = cnt.saturating_add(1);
                }
            });
            let Some(dest) = inst.dest() else { continue };
            let idx = dest.0 as usize;
            if idx >= max {
                continue;
            }
            if defined[idx] {
                // Malformed multi-def input: trust neither definition.
                diffs[idx] = None;
                continue;
            }
            defined[idx] = true;
            if let Instruction::BinOp {
                op: op @ (IrBinOp::Xor | IrBinOp::Sub),
                lhs,
                rhs,
                ty,
                ..
            } = inst
            {
                if ty.is_integer() && ty.size() <= 8 {
                    diffs[idx] = Some(DiffDef {
                        op: *op,
                        lhs: *lhs,
                        rhs: *rhs,
                        ty: *ty,
                    });
                }
            }
        }
        block.terminator.for_each_used_value(|v| {
            if let Some(cnt) = uses.get_mut(v as usize) {
                *cnt = cnt.saturating_add(1);
            }
        });
    }

    let mut changes = 0;

    // Shape 2: `Cmp Eq/Ne(d, 0)` -> `Cmp Eq/Ne(a, b)`, in place. The zero may
    // sit on either side; `Eq`/`Ne` are symmetric so no swap is needed.
    // For `Sub`, require single-use: when `d = a - b` stays live after the
    // compare (e.g. `--chain_length != 0`), comparing `a` vs `b` keeps `a` live
    // across `Sub` and emits `lea -1; cmp $1; jcc; mov` instead of `sub $1; jcc`.
    for block in &mut func.blocks {
        for inst in &mut block.instructions {
            let Instruction::Cmp {
                op, lhs, rhs, ty, ..
            } = inst
            else {
                continue;
            };
            if !matches!(op, IrCmpOp::Eq | IrCmpOp::Ne) {
                continue;
            }
            let value = match (&*lhs, &*rhs) {
                (Operand::Value(v), z) | (z, Operand::Value(v)) if const_u64(*z) == Some(0) => *v,
                _ => continue,
            };
            let Some(d) = diffs.get(value.0 as usize).copied().flatten() else {
                continue;
            };
            if d.op == IrBinOp::Sub && uses.get(value.0 as usize).copied().unwrap_or(0) > 1 {
                continue;
            }
            // The compare must read the difference at its own width: a
            // narrower compare of a wider difference tests only low bits.
            if !facts::same_width_scalar(d.ty, *ty) {
                continue;
            }
            *lhs = d.lhs;
            *rhs = d.rhs;
            changes += 1;
        }
    }

    // Shape 1: `CondBranch(d)` -> append `c = Cmp Ne(a, b)`, branch on `c`.
    // Skip multi-use `Sub(a, const)` (loop decrement `if (--n)`) for the same
    // live-range reason as Shape 2.
    let mut next = func.max_value_id().saturating_add(1);
    for block in &mut func.blocks {
        let d = {
            let Terminator::CondBranch {
                cond: Operand::Value(v),
                true_label,
                false_label,
            } = &block.terminator
            else {
                continue;
            };
            if true_label == false_label {
                // Degenerate branch: cfg_simplify's business, not a compare's.
                continue;
            }
            match diffs.get(v.0 as usize).copied().flatten() {
                Some(d)
                    if !(d.op == IrBinOp::Sub
                        && matches!(d.rhs, Operand::Const(_))
                        && uses.get(v.0 as usize).copied().unwrap_or(0) > 1) =>
                {
                    d
                }
                _ => continue,
            }
        };
        let fresh = Value(next);
        next = next.saturating_add(1);
        block.push_inst(
            Instruction::Cmp {
                dest: fresh,
                op: IrCmpOp::Ne,
                lhs: d.lhs,
                rhs: d.rhs,
                ty: d.ty,
            },
            Span::dummy(),
        );
        if let Terminator::CondBranch { cond, .. } = &mut block.terminator {
            *cond = Operand::Value(fresh);
        }
        changes += 1;
    }
    if next > func.next_value_id {
        func.next_value_id = next;
    }
    changes
}
