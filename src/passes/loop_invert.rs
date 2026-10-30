//! Machine-level loop inversion (bottom-testing), run after phi elimination.
//!
//! lccc emits counted loops in top-test form: the header tests, the body runs,
//! and the latch jumps unconditionally back to the header.
//!
//! ```text
//! .LBB5:  cmpq $8192, %r12 ; jae .LBB9    # not taken
//! .LBB6:  movzbl (%rbx,%r12), %r8d
//!         testb %r8b, %r8b ; je .LBB8     # not taken
//! .LBB7:  leaq 1(%r12), %r12 ; jmp .LBB5  # TAKEN, every iteration
//! ```
//!
//! GCC and Clang emit the bottom-test form, where the single backward branch
//! *is* the loop test. Measured on `tests/bench/k_memchr.c` by hand-editing the
//! assembly and timing all four variants (best-of-13, this host):
//!
//! | variant | time |
//! |---|---|
//! | as emitted, 6 insn/iter | 17.459 ms |
//! | **bottom-tested, guard kept, 5 insn/iter** | **8.769 ms** |
//! | bottom-tested + load folded into the compare, 4 insn/iter | 8.771 ms |
//! | GCC 14.2 reference | ~8.80 ms |
//!
//! Nearly a **2x** difference, and note the third row: folding the load into
//! the compare on top of the rotation buys *nothing*. The cost was never the
//! instruction count, it was the unconditional `jmp` — the rotated loop has a
//! single taken branch per iteration instead of a taken conditional plus a
//! taken jump.
//!
//! # Why here and not in `loop_rotate`
//!
//! [`crate::passes::loop_rotate`] does this on SSA, where duplicating the
//! header means rewriting header phis; that is the part which produced a
//! string of miscompiles, and the pass is still opt-in and restricted to
//! single-block bodies (it declines `k_memchr` with
//! `not single-block body (body_len=3)`).
//!
//! This pass runs after `eliminate_phis`, so **there are no phis left**.
//! Inversion degenerates to copying a handful of side-effect-free
//! instructions and retargeting one edge — no phi surgery, nothing to get
//! wrong. Phi elimination has already placed the induction variable's copy at
//! the end of the latch, so the duplicated test naturally reads the *updated*
//! value, which is precisely what a bottom test must do.
//!
//! # When it is legal
//!
//! For a natural loop with header `H`, a single latch `T` (`T != H`) whose
//! terminator is `Branch(H)`:
//!
//! 1. `H` ends in a `CondBranch` with exactly one target inside the loop (the
//!    body entry `B`) and one outside (the exit `X`). A loop whose header does
//!    not decide the exit is not a top-test loop.
//! 2. Every instruction in `H` is **pure and duplicable** — no memory access,
//!    no calls, no side effects. Memory duplication was implemented, reasoned
//!    about and **rejected on measurement** (S24): a header load that feeds
//!    the exit test is precisely the shape of a byte-walk loop, and rotating
//!    such a loop costs 15.9% on this host against the top-tested form lccc
//!    already emits — while GCC 14.2 emits the rotated form and is 15.9%
//!    slower than lccc for it. See `docs/SESSION_FOLLOWUP_S21_LOOP_ROTATION.md`
//!    §5 for the three experiments that localize it to the loop shape.
//! 3. Every value defined in `H` is used **only inside `H`**. Otherwise the
//!    body, once reached from `T`, would read the value `H` computed on its
//!    single guard execution instead of this iteration's.
//! 4. `H` has at least one predecessor outside the loop, so the guard it
//!    becomes is actually reachable.
//! 5. `H` is small (`MAX_DUP_INSTS`), because its instructions are duplicated.
//!
//! The transform then appends a renamed copy of `H`'s instructions to `T` and
//! gives `T` a copy of `H`'s terminator. `H` keeps its own terminator and
//! becomes the loop guard, executed once. The back edge becomes `T -> B`.

use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::common::types::IrType;
use crate::ir::instruction::{Instruction, Terminator};
use crate::ir::reexports::{BlockId, IrConst, IrFunction, Operand, Value};
use crate::passes::CfgAnalysis;

/// Duplicating more than a handful of instructions trades I-cache for branches.
/// A counted loop's test is one or two instructions; anything much larger is
/// not the shape this pass is for.
const MAX_DUP_INSTS: usize = 8;

/// True when `inst` may be duplicated into the latch: pure, no memory traffic,
/// no control flow, no side effects.
fn is_duplicable(inst: &Instruction) -> bool {
    matches!(
        inst,
        Instruction::Cmp { .. }
            | Instruction::BinOp { .. }
            | Instruction::UnaryOp { .. }
            | Instruction::Copy { .. }
            | Instruction::Cast { .. }
            | Instruction::Select { .. }
            | Instruction::GetElementPtr { .. }
            | Instruction::GlobalAddr { .. }
    )
}

/// Whether loop inversion runs at all.
///
/// The kill switch is resolved **once**, by the driver, and then passed down
/// explicitly. It used to be read out of the process environment deep inside
/// the pass; because `std::env::set_var` is process-global, a test that
/// toggled it raced with every test running in parallel on the other threads
/// of the test binary, and those tests observed the wrong value (the symptom
/// was a ~1-in-10 flake in this module). Configuration a caller can supply
/// belongs in a parameter, not in ambient global state.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct LoopInvertConfig {
    pub enabled: bool,
    pub debug: bool,
    /// Run the VALUE-bounded affine exit-compare fold on the duplicated
    /// bottom test (`CCC_NO_AFFINE_EXIT_FOLD` owns every producer of that
    /// fold, so the driver resolves it and passes it down, like `enabled`).
    pub affine_fold: bool,
    /// Report the fold through `CCC_DEBUG_AFFINE_FOLD`, resolved the same way.
    pub affine_fold_debug: bool,
    /// Replace a dead, constant-trip +1 induction with a zero-tested countdown.
    /// The driver resolves `CCC_NO_LOOP_DOWNCOUNT` once, alongside `enabled`.
    pub downcount: bool,
}

impl LoopInvertConfig {
    /// The driver's constructor.  This module reads **no** environment variable.
    /// An ambient read reachable from the pass (as a per-function `from_env`
    /// would provide) lets any caller re-read the environment per function.
    /// Taking the two booleans as parameters makes the "resolved once by the
    /// driver" claim structural: the ratchet
    /// in `tests/regression/check_env_test_hygiene.sh` can then fall and stay
    /// down, instead of counting a read that happens to have no caller today.
    pub const fn new(
        enabled: bool,
        debug: bool,
        affine_fold: bool,
        affine_fold_debug: bool,
        downcount: bool,
    ) -> Self {
        Self {
            enabled,
            debug,
            affine_fold,
            affine_fold_debug,
            downcount,
        }
    }

    /// Configuration used by the unit tests: no environment access at all.
    #[cfg(test)]
    const fn testing() -> Self {
        Self {
            enabled: true,
            debug: false,
            affine_fold: true,
            affine_fold_debug: false,
            downcount: true,
        }
    }

    #[cfg(test)]
    const fn disabled() -> Self {
        Self {
            enabled: false,
            debug: false,
            affine_fold: true,
            affine_fold_debug: false,
            downcount: true,
        }
    }

    #[cfg(test)]
    const fn no_affine_fold() -> Self {
        Self {
            enabled: true,
            debug: false,
            affine_fold: false,
            affine_fold_debug: false,
            downcount: true,
        }
    }

    #[cfg(test)]
    const fn no_downcount() -> Self {
        Self {
            downcount: false,
            ..Self::testing()
        }
    }
}

/// The pass itself, parameterised by [`LoopInvertConfig`].
pub(crate) fn invert_loops_with(func: &mut IrFunction, cfg: LoopInvertConfig) -> usize {
    let debug = cfg.debug;
    if func.blocks.len() < 3 {
        return 0;
    }
    if !cfg.enabled {
        return 0;
    }

    // Phis must already be gone; this pass's whole safety argument rests on it.
    debug_assert!(
        !func
            .blocks
            .iter()
            .flat_map(|b| b.instructions.iter())
            .any(|i| matches!(i, Instruction::Phi { .. })),
        "loop_invert runs after eliminate_phis; a surviving phi means the \
         pipeline order changed and the duplication below is no longer safe"
    );

    let mut rotated = 0;
    // One rotation per call per loop: the CFG changes, so re-derive it.
    loop {
        let Some(plan) = find_one(func, debug) else {
            break;
        };
        apply(func, &plan);
        if cfg.affine_fold {
            fold_value_bound_after_invert(func, &plan, cfg.affine_fold_debug);
        }
        if cfg.downcount {
            downcount_after_invert(func, &plan, debug);
        }
        rotated += 1;
        if rotated > 64 {
            break; // pathological input guard
        }
    }
    rotated
}

struct Plan {
    header: usize,
    latch: usize,
    /// The loop body (header included) at `find_one` time; the value-bound
    /// fold's invariance scans need it after the CFG changed.
    body: Vec<usize>,
}

fn find_one(func: &IrFunction, debug: bool) -> Option<Plan> {
    let cfg = crate::ir::analysis::CfgAnalysis::build(func);
    // MERGE BY HEADER. `find_natural_loops` returns one loop per BACK EDGE,
    // so a loop with two latches arrives as two single-latch loops and the
    // single-latch guard below would wave both through, duplicating the test
    // once per latch. Merging first makes the guard mean what it says.
    let loops = crate::passes::loop_analysis::merge_loops_by_header(
        crate::passes::loop_analysis::find_natural_loops(
            cfg.num_blocks,
            &cfg.preds,
            &cfg.succs,
            &cfg.idom,
        ),
    );

    let label_to_idx: FxHashMap<u32, usize> = func
        .blocks
        .iter()
        .enumerate()
        .map(|(i, b)| (b.label.0, i))
        .collect();

    'next_loop: for lp in &loops {
        let h = lp.header;
        if h >= func.blocks.len() {
            continue;
        }

        // A single latch, branching unconditionally to the header.
        let mut latch = None;
        for &p32 in cfg.preds.row(h) {
            let p = p32 as usize;
            if lp.body.contains(&p) {
                if latch.is_some() {
                    continue 'next_loop; // more than one latch
                }
                latch = Some(p);
            }
        }
        let Some(t) = latch else { continue };
        if t == h {
            continue; // self-loop: already bottom-tested
        }
        if !matches!(&func.blocks[t].terminator, Terminator::Branch(l) if *l == func.blocks[h].label)
        {
            continue;
        }

        // Guard 1: the header decides the exit.
        let Terminator::CondBranch {
            true_label,
            false_label,
            ..
        } = &func.blocks[h].terminator
        else {
            continue;
        };
        let (Some(&tb), Some(&fb)) = (
            label_to_idx.get(&true_label.0),
            label_to_idx.get(&false_label.0),
        ) else {
            continue;
        };
        let t_in = lp.body.contains(&tb);
        let f_in = lp.body.contains(&fb);
        if t_in == f_in {
            continue; // both inside (not an exit test) or both outside
        }

        // Guard 5 + 2: small, and every instruction pure.
        let hb = &func.blocks[h];
        if hb.instructions.len() > MAX_DUP_INSTS {
            continue;
        }
        if !hb.instructions.iter().all(is_duplicable) {
            if debug {
                eprintln!("[INV] header {} has a non-duplicable instruction", h);
            }
            continue;
        }

        // Guard 3: nothing defined in the header escapes it.
        let defined: FxHashSet<u32> = hb
            .instructions
            .iter()
            .filter_map(|i| i.dest())
            .map(|d| d.0)
            .collect();
        if !defined.is_empty() {
            for (bi, b) in func.blocks.iter().enumerate() {
                if bi == h {
                    continue;
                }
                let mut escaped = None;
                for inst in &b.instructions {
                    inst.for_each_used_value(|v| {
                        if defined.contains(&v) && escaped.is_none() {
                            escaped = Some(v);
                        }
                    });
                }
                terminator_uses(&b.terminator, &mut |v| {
                    if defined.contains(&v) && escaped.is_none() {
                        escaped = Some(v);
                    }
                });
                if let Some(v) = escaped {
                    if debug {
                        let used_in_loop = lp.body.contains(&bi);
                        eprintln!(
                            "[INV] header {} defines v{}, used in block {} ({})",
                            h,
                            v,
                            bi,
                            if used_in_loop {
                                "in loop"
                            } else {
                                "outside loop"
                            }
                        );
                    }
                    continue 'next_loop;
                }
            }
        }

        // Guard 4: an entry from outside the loop, so the guard is reachable.
        if !cfg
            .preds
            .row(h)
            .iter()
            .any(|&p| !lp.body.contains(&(p as usize)))
        {
            continue;
        }

        if debug {
            eprintln!("[INV] rotating header={} latch={}", h, t);
        }
        return Some(Plan {
            header: h,
            latch: t,
            body: lp.body.iter().copied().collect(),
        });
    }
    None
}

fn apply(func: &mut IrFunction, plan: &Plan) {
    // Seed from the SOUND high-water mark: InlineAsm outputs are definitions
    // that `dest()` does not report, so a dest-only max() scan silently
    // re-issues IDs already taken by an inline-asm output and corrupts SSA
    // numbering (pr84524: the cloned latch test collided with `"+r"(v)` at
    // id 55, demoted the asm output to an indirect slot, and stored through
    // an uninitialized pointer). Write the new bound back so every later
    // pass seeds above it.
    let mut next_id = func.sound_next_value_id();

    let header_insts = func.blocks[plan.header].instructions.clone();
    let header_term = func.blocks[plan.header].terminator.clone();

    // Fresh names for everything the header defines, so the copy in the latch
    // is a separate live range from the guard's.
    let mut remap: FxHashMap<u32, u32> = FxHashMap::default();
    let mut cloned: Vec<Instruction> = Vec::with_capacity(header_insts.len());
    for inst in &header_insts {
        let mut c = inst.clone();
        // Rewrite USES first: an earlier header instruction may feed this one.
        c.for_each_operand_mut(|op| {
            if let Operand::Value(v) = op {
                if let Some(&n) = remap.get(&v.0) {
                    *v = Value(n);
                }
            }
        });
        // Bare-Value positions (Load/Store ptr, GEP base, ...) are not
        // Operands: without this walk a clone keeps naming the guard's
        // definitions instead of its own fresh ones (separate-live-range
        // violation). Same two-walk substitution contract as loop_memset.
        c.for_each_value_use_mut(|v| {
            if let Some(&n) = remap.get(&v.0) {
                *v = Value(n);
            }
        });
        if let Some(d) = c.dest() {
            let fresh = next_id;
            next_id += 1;
            remap.insert(d.0, fresh);
            set_dest(&mut c, Value(fresh));
        }
        cloned.push(c);
    }

    let mut term = header_term;
    if let Terminator::CondBranch { cond, .. } = &mut term {
        if let Operand::Value(v) = cond {
            if let Some(&n) = remap.get(&v.0) {
                *v = Value(n);
            }
        }
    }

    let latch = &mut func.blocks[plan.latch];
    // Keep source_spans parallel to instructions, or -g emits .loc against the
    // wrong instruction. The duplicated test inherits the latch's last span
    // when one is being tracked.
    if latch.source_spans.len() == latch.instructions.len() {
        let fill = latch.source_spans.last().copied();
        if let Some(sp) = fill {
            latch
                .source_spans
                .extend(std::iter::repeat_n(sp, cloned.len()));
        } else {
            latch.source_spans.clear();
        }
    }
    latch.instructions.extend(cloned);
    latch.terminator = term;
    func.next_value_id = func.next_value_id.max(next_id);
}

/// AFFOLD-INV: fold the freshly duplicated bottom test `x + C <strict> n`
/// into `x <strict> t`, `t = n - C` computed once in the guard block.
///
/// This is the machine-level sibling of `loop_rotate::fold_loop_value_bound`
/// (same proof, post-phi-elim shape).  Soundness in exact arithmetic — no
/// use of signed-overflow UB, so `-fwrapv` code is served faithfully:
///
/// * The GUARD is the retained header itself: inversion just duplicated its
///   `add x, C ; cmp , n` into the latch, so the body edge carries the fact
///   `x_entry + C <strict> n`.  That fact is exact only when the guard's own
///   add cannot have wrapped, which is why `x` must enter with a CONSTANT
///   seed: the loop's unique outside predecessor must end by `x = start`
///   with `start + C` representable.  Without that, a prior inner-loop exit
///   value of `x` re-entering under `-fwrapv` wraps the guard sum and the
///   folded latch diverges from the original.
/// * `t = n - C` cannot underflow on any path that executes the latch test:
///   the guard established `n >= start + C + 1 > type_min + C`.
/// * STRICT compare and STEP + 1 only, for the same reasons as the SSA
///   fold: `<=` with `n == type_max` turns a legally infinite `-fwrapv`
///   loop finite, and a wider step overshoots into a wrapping sum near
///   `type_max`.
/// * By induction (base from the guard, step from the previous latch test
///   being true) every latch-time `x + C` is `<= n <= type_max`, so the
///   original wrapping add was exact and both compares decide identically.
/// * `n` is a value the loop never writes; the increment is the loop's ONLY
///   write to `x` (`tmp = x + 1; x = tmp` in the latch, before the test),
///   so every circuit advances `x` exactly once.
///
/// The rewrite deletes the duplicated add (its single consumer was the
/// duplicated compare) — the hot latch drops from `lea; cmp; jcc` to
/// `cmp; jcc` against a register computed once.
fn fold_value_bound_after_invert(func: &mut IrFunction, plan: &Plan, debug: bool) -> bool {
    use crate::ir::reexports::{IrBinOp, IrCmpOp, IrConst};
    use crate::passes::loop_rotate::{integer_constant, integer_type_bounds};

    const MAX_C: i128 = i64::MAX as i128;
    let (h, t) = (plan.header, plan.latch);
    let label_pos = |l: BlockId| func.blocks.iter().position(|b| b.label == l);
    let in_loop = |l: BlockId| label_pos(l).is_some_and(|i| plan.body.contains(&i));

    // Normalize a compare to `(affine_value, bound_value, strict_op)`.
    let normalize = |lhs: &Operand,
                     op: IrCmpOp,
                     rhs: &Operand|
     -> Option<(Value, Value, IrCmpOp)> {
        match (lhs, op, rhs) {
            (Operand::Value(a), IrCmpOp::Slt | IrCmpOp::Ult, Operand::Value(n)) => {
                Some((*a, *n, op))
            }
            (Operand::Value(n), IrCmpOp::Sgt, Operand::Value(a)) => Some((*a, *n, IrCmpOp::Slt)),
            (Operand::Value(n), IrCmpOp::Ugt, Operand::Value(a)) => Some((*a, *n, IrCmpOp::Ult)),
            _ => None,
        }
    };

    // ── The guard (retained header): `t0 = x0 + C ; cc0 = cmp(t0, n)`,
    // continue on the TRUE edge.
    let Terminator::CondBranch {
        cond: Operand::Value(cc0),
        true_label,
        false_label,
    } = func.blocks[h].terminator
    else {
        return false;
    };
    if !in_loop(true_label) || in_loop(false_label) {
        return false;
    }
    let hb = &func.blocks[h];
    let Some(Instruction::Cmp {
        lhs, op, rhs, ty, ..
    }) = hb.instructions.iter().find(|i| i.dest() == Some(cc0))
    else {
        return false;
    };
    let ty = *ty;
    let Some((t0, nv, strict_op)) = normalize(lhs, *op, rhs) else {
        return false;
    };
    if ty.is_unsigned() != matches!(strict_op, IrCmpOp::Ult) {
        return false; // domain mismatch
    }
    let Some((lo, hi)) = integer_type_bounds(ty) else {
        return false;
    };
    // The affine add in the guard: `t0 = x0 + C`.
    let add_of =
        |block: &crate::ir::reexports::BasicBlock, dest: Value| -> Option<(usize, Value, i128)> {
            block
                .instructions
                .iter()
                .enumerate()
                .find_map(|(i, inst)| match inst {
                    Instruction::BinOp {
                        dest: d,
                        op: IrBinOp::Add,
                        lhs,
                        rhs,
                        ty: add_ty,
                    } if *d == dest && *add_ty == ty => match (lhs, rhs) {
                        (Operand::Value(v), Operand::Const(k))
                        | (Operand::Const(k), Operand::Value(v)) => {
                            Some((i, *v, integer_constant(*k, ty)?))
                        }
                        _ => None,
                    },
                    _ => None,
                })
        };
    let Some((_, x0, c)) = add_of(hb, t0) else {
        return false;
    };
    if !(1..=MAX_C).contains(&c) || x0 == nv {
        return false;
    }
    if hb
        .instructions
        .iter()
        .any(|i| i.dest() == Some(x0) || i.dest() == Some(nv))
    {
        return false; // the guard must only READ the IV and the bound
    }

    // ── Constant seed: unique outside predecessor whose LAST write to `x0`
    // is `x0 = start` (every loop entry re-executes it).
    let header_label = func.blocks[h].label;
    let mut preds = func.blocks.iter().enumerate().filter(|(bi, b)| {
        !plan.body.contains(bi)
            && match &b.terminator {
                Terminator::Branch(l) => *l == header_label,
                Terminator::CondBranch {
                    true_label,
                    false_label,
                    ..
                } => *true_label == header_label || *false_label == header_label,
                _ => false,
            }
    });
    let (Some((p, _)), None) = (preds.next(), preds.next()) else {
        return false;
    };
    let start = func.blocks[p]
        .instructions
        .iter()
        .rev()
        .find(|i| i.dest() == Some(x0))
        .and_then(|i| match i {
            Instruction::Copy {
                src: Operand::Const(k),
                ..
            } => integer_constant(*k, ty),
            _ => None,
        });
    let Some(start) = start else {
        return false;
    };
    let first = start + c;
    if !(lo..=hi).contains(&first) {
        return false;
    }

    // ── The loop writes `x0` exactly once — `tmp = x0 + 1 ; x0 = tmp` in the
    // latch, before the duplicated test — and never writes `n`.
    let body: Vec<usize> = plan.body.iter().copied().filter(|&bi| bi != h).collect();
    let mut x0_writes: Vec<(usize, usize)> = Vec::new();
    for &bi in &body {
        for (ii, inst) in func.blocks[bi].instructions.iter().enumerate() {
            if inst.dest() == Some(nv) {
                return false;
            }
            if inst.dest() == Some(x0) {
                x0_writes.push((bi, ii));
            }
        }
    }
    let [(wb, wi)] = x0_writes[..] else {
        return false;
    };
    if wb != t {
        return false;
    }
    let plus_one = |lhs: &Operand, rhs: &Operand| -> bool {
        match (lhs, rhs) {
            (Operand::Value(v), Operand::Const(k)) | (Operand::Const(k), Operand::Value(v)) => {
                *v == x0 && integer_constant(*k, ty) == Some(1)
            }
            _ => false,
        }
    };
    // Either phi-elim's `tmp = x0 + 1 ; x0 = tmp`, or the coalesced direct
    // `x0 = x0 + 1`.  Both read the pre-increment `x0` exactly once.
    let step_ok = match &func.blocks[t].instructions[wi] {
        Instruction::BinOp {
            op: IrBinOp::Add,
            lhs,
            rhs,
            ty: add_ty,
            ..
        } => *add_ty == ty && plus_one(lhs, rhs),
        Instruction::Copy {
            src: Operand::Value(tmp),
            ..
        } => {
            let tmp = *tmp;
            let feeds = func.blocks[t].instructions[..wi].iter().any(|inst| {
                matches!(inst, Instruction::BinOp {
                    dest,
                    op: IrBinOp::Add,
                    lhs,
                    rhs,
                    ty: add_ty,
                } if *dest == tmp && *add_ty == ty && plus_one(lhs, rhs))
            });
            let tmp_writes = body
                .iter()
                .flat_map(|&bi| func.blocks[bi].instructions.iter())
                .filter(|i| i.dest() == Some(tmp))
                .count();
            feeds && tmp_writes == 1
        }
        _ => false,
    };
    if !step_ok {
        return false;
    }

    // ── The duplicated bottom test in the latch, AFTER the increment.
    let Terminator::CondBranch {
        cond: Operand::Value(cc1),
        true_label: tl1,
        false_label: fl1,
    } = func.blocks[t].terminator
    else {
        return false;
    };
    if !in_loop(tl1) || in_loop(fl1) {
        return false;
    }
    let tb = &func.blocks[t];
    let Some((icmp, (clhs, cop, crhs))) =
        tb.instructions
            .iter()
            .enumerate()
            .find_map(|(i, inst)| match inst {
                Instruction::Cmp {
                    dest,
                    lhs,
                    op,
                    rhs,
                    ty: cmp_ty,
                } if *dest == cc1 && *cmp_ty == ty => Some((i, (*lhs, *op, *rhs))),
                _ => None,
            })
    else {
        return false;
    };
    let Some((t1, nv1, strict1)) = normalize(&clhs, cop, &crhs) else {
        return false;
    };
    if nv1 != nv || strict1 != strict_op {
        return false;
    }
    let Some((iadd, x1, c1)) = add_of(tb, t1) else {
        return false;
    };
    if x1 != x0 || c1 != c || !(wi < iadd && iadd < icmp) {
        return false;
    }
    // Single consumers (structural for a fresh clone; verified cheaply).
    let t1_uses: usize = func
        .blocks
        .iter()
        .flat_map(|b| b.instructions.iter())
        .map(|i| {
            let mut n = 0;
            i.for_each_used_value(|v| {
                if v == t1.0 {
                    n += 1;
                }
            });
            n
        })
        .sum();
    if t1_uses != 1 {
        return false;
    }

    // ── All obligations hold: hoist `t = n - C` into the guard and fold.
    let tsub = Value(func.sound_next_value_id());
    let sub = Instruction::BinOp {
        dest: tsub,
        op: IrBinOp::Sub,
        lhs: Operand::Value(nv),
        rhs: Operand::Const(IrConst::from_i64(c as i64, ty)),
        ty,
    };
    let guard = &mut func.blocks[h];
    if guard.source_spans.len() == guard.instructions.len() {
        if let Some(sp) = guard.source_spans.last().copied() {
            guard.source_spans.push(sp);
        }
    }
    guard.instructions.push(sub);
    let latch = &mut func.blocks[t];
    if latch.source_spans.len() == latch.instructions.len() {
        latch.source_spans.remove(iadd);
    }
    latch.instructions.remove(iadd);
    let icmp = icmp - 1; // the removed add preceded the compare
    if let Instruction::Cmp { lhs, op, rhs, .. } = &mut latch.instructions[icmp] {
        *lhs = Operand::Value(x0);
        *op = strict_op;
        *rhs = Operand::Value(tsub);
    }
    func.next_value_id = func.next_value_id.max(tsub.0 + 1);
    if debug {
        eprintln!(
            "[AFFOLD] {}: value-bound fold (inverted latch): v{} + {} <strict> v{} -> v{} <strict> (v{} - {})",
            func.name, x0.0, c, nv.0, x0.0, nv.0, c
        );
    }
    true
}

/// Prove and fold a dead, constant-trip up-counter after inversion.
///
/// For `iv = 0; do { body; next = iv + 1; iv = next; } while (iv < N)`
/// with unsigned `N > 0`, the recurrence has exactly N iterations and
/// `remaining = N - iv`.  If both SSA values are otherwise dead, replacing
/// the initial value by N, the increment by a decrement and the latch test by
/// `remaining != 0` preserves every observable event.  The entry test is
/// true on the first iteration and is removed with its compare.
///
/// This intentionally accepts only the fully materialised two-Copy shape
/// left by phi elimination.  All definitions, uses, constants, CFG edges and
/// instruction positions are checked before the first mutation.
fn downcount_after_invert(func: &mut IrFunction, plan: &Plan, debug: bool) -> bool {
    use crate::ir::reexports::{IrBinOp, IrCmpOp};
    use crate::passes::loop_rotate::{integer_constant, integer_type_bounds};

    let (h, t) = (plan.header, plan.latch);
    if h == t {
        return false;
    }
    let body = func.blocks[t].label;
    let (guard_cond, exit) = match func.blocks[h].terminator {
        Terminator::CondBranch {
            cond: Operand::Value(c),
            true_label,
            false_label,
        } if true_label == body => (c, false_label),
        _ => return false,
    };
    let latch_cond = match func.blocks[t].terminator {
        Terminator::CondBranch {
            cond: Operand::Value(c),
            true_label,
            false_label,
        } if true_label == body && false_label == exit => c,
        _ => return false,
    };
    if guard_cond == latch_cond || func.blocks[h].instructions.len() != 1 {
        return false;
    }

    // Inversion must have left exactly one explicit preheader edge.  In
    // particular, do not infer an entry value from a block with another edge.
    let cfg = CfgAnalysis::build(func);
    let preds = cfg.preds.row(h);
    if preds.len() != 1 {
        return false;
    }
    let p = preds[0] as usize;
    if p == h
        || p == t
        || !matches!(
            &func.blocks[p].terminator,
            Terminator::Branch(label) if *label == func.blocks[h].label
        )
    {
        return false;
    }

    fn cmp_shape(inst: &Instruction, dest: Value) -> Option<(Value, IrConst, IrType)> {
        match inst {
            Instruction::Cmp {
                dest: d,
                op: IrCmpOp::Ult,
                lhs: Operand::Value(iv),
                rhs: Operand::Const(limit),
                ty,
            } if *d == dest && matches!(ty, IrType::U32 | IrType::U64) => Some((*iv, *limit, *ty)),
            _ => None,
        }
    }
    // The header gate above (`instructions.len() != 1`) has already proven the
    // header holds exactly this compare, so the site is a constant and every
    // later rewrite can address it by index.
    const GUARD_CMP: usize = 0;
    let guard_cmp = GUARD_CMP;
    let Some((iv, limit_const, ty)) =
        cmp_shape(&func.blocks[h].instructions[guard_cmp], guard_cond)
    else {
        return false;
    };
    let Some(latch_cmp) = func.blocks[t]
        .instructions
        .iter()
        .position(|inst| matches!(inst, Instruction::Cmp { dest, .. } if *dest == latch_cond))
    else {
        return false;
    };
    let Some((latch_iv, latch_limit, latch_ty)) =
        cmp_shape(&func.blocks[t].instructions[latch_cmp], latch_cond)
    else {
        return false;
    };
    if latch_iv != iv || latch_ty != ty {
        return false;
    }
    let Some(limit) = integer_constant(limit_const, ty) else {
        return false;
    };
    if integer_constant(latch_limit, ty) != Some(limit)
        || limit <= 0
        || integer_type_bounds(ty).is_none_or(|(_, hi)| limit > hi)
    {
        return false;
    }

    let mut iv_defs = Vec::new();
    for (bi, block) in func.blocks.iter().enumerate() {
        for (ii, inst) in block.instructions.iter().enumerate() {
            if inst.dest() == Some(iv) {
                iv_defs.push((bi, ii));
            }
        }
    }
    if iv_defs.len() != 2 {
        return false;
    }
    let Some(&(_, init)) = iv_defs.iter().find(|(bi, _)| *bi == p) else {
        return false;
    };
    let Some(&(_, copy)) = iv_defs.iter().find(|(bi, _)| *bi == t) else {
        return false;
    };
    if !matches!(
        func.blocks[p].instructions[init],
        Instruction::Copy {
            src: Operand::Const(k),
            ..
        } if integer_constant(k, ty) == Some(0)
    ) {
        return false;
    }
    let next = match func.blocks[t].instructions[copy] {
        Instruction::Copy {
            src: Operand::Value(next),
            ..
        } => next,
        _ => return false,
    };
    if next == iv {
        return false;
    }

    let mut next_defs = Vec::new();
    for (bi, block) in func.blocks.iter().enumerate() {
        for (ii, inst) in block.instructions.iter().enumerate() {
            if inst.dest() == Some(next) {
                next_defs.push((bi, ii));
            }
        }
    }
    if next_defs.len() != 1 || next_defs[0].0 != t {
        return false;
    }
    let update = next_defs[0].1;
    if !matches!(
        func.blocks[t].instructions[update],
        Instruction::BinOp {
            op: IrBinOp::Add,
            lhs: Operand::Value(lhs),
            rhs: Operand::Const(step),
            ty: update_ty,
            ..
        } if lhs == iv && update_ty == ty && integer_constant(step, ty) == Some(1)
    ) || update >= copy
        || copy >= latch_cmp
    {
        return false;
    }

    // No semantic use may survive outside the three recurrence sites.  This
    // also rejects live-out values, asm outputs, hidden pointer uses and a
    // second condition consuming either compare result.
    for (bi, block) in func.blocks.iter().enumerate() {
        for (ii, inst) in block.instructions.iter().enumerate() {
            let site = (bi, ii);
            let mut valid = true;
            inst.for_each_used_value(|v| {
                if v == iv.0
                    && site != (h, guard_cmp)
                    && site != (t, update)
                    && site != (t, latch_cmp)
                {
                    valid = false;
                }
                if v == next.0 && site != (t, copy) {
                    valid = false;
                }
                if (v == guard_cond.0 && site != (h, guard_cmp))
                    || (v == latch_cond.0 && site != (t, latch_cmp))
                {
                    valid = false;
                }
            });
            if !valid {
                return false;
            }
        }
        let mut valid = true;
        terminator_uses(&block.terminator, &mut |v| {
            if v == iv.0
                || v == next.0
                || (v == guard_cond.0 && bi != h)
                || (v == latch_cond.0 && bi != t)
            {
                valid = false;
            }
        });
        if !valid {
            return false;
        }
    }

    // All obligations are proven; mutate only now.
    func.blocks[p].instructions[init] = Instruction::Copy {
        dest: iv,
        src: Operand::Const(limit_const),
    };
    let guard = &mut func.blocks[h];
    if guard.source_spans.len() == guard.instructions.len() {
        guard.source_spans.remove(guard_cmp);
    }
    guard.instructions.remove(guard_cmp);
    func.blocks[h].terminator = Terminator::Branch(body);
    if let Instruction::BinOp { op, .. } = &mut func.blocks[t].instructions[update] {
        *op = IrBinOp::Sub;
    }
    if let Instruction::Cmp { op, rhs, .. } = &mut func.blocks[t].instructions[latch_cmp] {
        *op = IrCmpOp::Ne;
        *rhs = Operand::Const(IrConst::from_i64(0, ty));
    }
    if debug {
        eprintln!(
            "[LOOP-DOWNCOUNT] {}: v{} 0..{} -> N..0 in .LBB{}",
            func.name, iv.0, limit, body.0
        );
    }
    true
}

fn set_dest(inst: &mut Instruction, new: Value) {
    macro_rules! d {
        ($($v:ident),+ $(,)?) => {
            match inst {
                $(Instruction::$v { dest, .. } => *dest = new,)+
                _ => {}
            }
        };
    }
    d!(
        Cmp,
        BinOp,
        UnaryOp,
        Copy,
        Cast,
        Select,
        GetElementPtr,
        GlobalAddr
    );
}

fn terminator_uses(term: &Terminator, f: &mut impl FnMut(u32)) {
    let mut on = |op: &Operand| {
        if let Operand::Value(v) = op {
            f(v.0);
        }
    };
    match term {
        Terminator::Return(Some(op)) => on(op),
        Terminator::CondBranch { cond, .. } => on(cond),
        Terminator::Switch { val, .. } => on(val),
        Terminator::IndirectBranch { target, .. } => on(target),
        _ => {}
    }
}

/// Unused import guard: `BlockId` keeps the signature readable above.
const _: Option<BlockId> = None;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::types::IrType;
    use crate::ir::instruction::BasicBlock;
    use crate::ir::reexports::IrConst;
    use crate::ir::reexports::{IrBinOp, IrCmpOp};

    fn blk(label: u32, instructions: Vec<Instruction>, terminator: Terminator) -> BasicBlock {
        BasicBlock {
            label: BlockId(label),
            instructions,
            terminator,
            source_spans: Vec::new(),
        }
    }
    fn br(t: u32) -> Terminator {
        Terminator::Branch(BlockId(t))
    }
    fn cond(c: u32, t: u32, f: u32) -> Terminator {
        Terminator::CondBranch {
            cond: Operand::Value(Value(c)),
            true_label: BlockId(t),
            false_label: BlockId(f),
        }
    }
    fn ret() -> Terminator {
        Terminator::Return(None)
    }
    fn cmp(dest: u32, lhs: u32) -> Instruction {
        Instruction::Cmp {
            dest: Value(dest),
            op: IrCmpOp::Slt,
            lhs: Operand::Value(Value(lhs)),
            rhs: Operand::Const(IrConst::I32(100)),
            ty: IrType::I32,
        }
    }
    fn add1(dest: u32, src: u32) -> Instruction {
        Instruction::BinOp {
            dest: Value(dest),
            op: IrBinOp::Add,
            lhs: Operand::Value(Value(src)),
            rhs: Operand::Const(IrConst::I32(1)),
            ty: IrType::I32,
        }
    }
    fn func_of(blocks: Vec<BasicBlock>) -> IrFunction {
        let mut f = IrFunction::new("t".into(), IrType::Void, Vec::new(), false);
        f.blocks = blocks;
        f
    }

    /// Post-phi counted-loop input for the dead-IV downcount proof:
    ///   0: iv = 0 -> 1
    ///   1: c = iv < limit; c ? 2 : 3
    ///   2: next = iv + 1; iv = next -> 1
    ///   3: return
    fn dead_counted_loop(limit: i64, ty: IrType) -> IrFunction {
        func_of(vec![
            blk(
                0,
                vec![Instruction::Copy {
                    dest: Value(20),
                    src: Operand::Const(IrConst::from_i64(0, ty)),
                }],
                br(1),
            ),
            blk(
                1,
                vec![Instruction::Cmp {
                    dest: Value(10),
                    op: IrCmpOp::Ult,
                    lhs: Operand::Value(Value(20)),
                    rhs: Operand::Const(IrConst::from_i64(limit, ty)),
                    ty,
                }],
                cond(10, 2, 3),
            ),
            blk(
                2,
                vec![
                    Instruction::BinOp {
                        dest: Value(26),
                        op: IrBinOp::Add,
                        lhs: Operand::Value(Value(20)),
                        rhs: Operand::Const(IrConst::from_i64(1, ty)),
                        ty,
                    },
                    Instruction::Copy {
                        dest: Value(20),
                        src: Operand::Value(Value(26)),
                    },
                ],
                br(1),
            ),
            blk(3, vec![], ret()),
        ])
    }

    fn assert_downcounted(f: &IrFunction, limit: i64, ty: IrType) {
        assert!(f.blocks[1].instructions.is_empty());
        assert!(matches!(
            f.blocks[1].terminator,
            Terminator::Branch(BlockId(2))
        ));
        assert!(matches!(
            f.blocks[0].instructions[0],
            Instruction::Copy {
                src: Operand::Const(k),
                ..
            } if k == IrConst::from_i64(limit, ty)
        ));
        assert!(matches!(
            f.blocks[2].instructions[0],
            Instruction::BinOp {
                op: IrBinOp::Sub,
                ..
            }
        ));
        assert!(f.blocks[2].instructions.iter().any(|inst| matches!(
            inst,
            Instruction::Cmp {
                op: IrCmpOp::Ne,
                rhs: Operand::Const(k),
                ..
            } if *k == IrConst::from_i64(0, ty)
        )));
    }

    #[test]
    fn dead_constant_trip_loop_downcounts_and_removes_guard() {
        let mut f = dead_counted_loop(8, IrType::U32);
        assert_eq!(invert_loops_with(&mut f, LoopInvertConfig::testing()), 1);
        assert_downcounted(&f, 8, IrType::U32);
    }

    #[test]
    fn downcount_handles_one_and_maximum_u32_trips() {
        for limit in [1, u32::MAX as i64] {
            let mut f = dead_counted_loop(limit, IrType::U32);
            assert_eq!(invert_loops_with(&mut f, LoopInvertConfig::testing()), 1);
            assert_downcounted(&f, limit, IrType::U32);
        }
    }

    #[test]
    fn downcount_handles_u64_max_trip_bit_pattern() {
        // U64 constants use the I64 bit pattern; the mathematical domain in
        // the matcher is i128, so -1 here means 2^64 - 1, not a signed limit.
        let mut f = dead_counted_loop(-1, IrType::U64);
        assert_eq!(invert_loops_with(&mut f, LoopInvertConfig::testing()), 1);
        assert_downcounted(&f, -1, IrType::U64);
    }

    #[test]
    fn downcount_is_idempotent() {
        let mut f = dead_counted_loop(8, IrType::U64);
        assert_eq!(invert_loops_with(&mut f, LoopInvertConfig::testing()), 1);
        assert_downcounted(&f, 8, IrType::U64);
        assert_eq!(invert_loops_with(&mut f, LoopInvertConfig::testing()), 0);
        assert_downcounted(&f, 8, IrType::U64);
    }

    #[test]
    fn downcount_refuses_zero_trip_loop() {
        let mut f = dead_counted_loop(0, IrType::U32);
        assert_eq!(invert_loops_with(&mut f, LoopInvertConfig::testing()), 1);
        assert!(matches!(
            f.blocks[1].terminator,
            Terminator::CondBranch { .. }
        ));
        assert!(matches!(
            f.blocks[2].instructions[0],
            Instruction::BinOp {
                op: IrBinOp::Add,
                ..
            }
        ));
    }

    #[test]
    fn downcount_refuses_live_out_iv_and_next() {
        for live in [20, 26] {
            let mut f = dead_counted_loop(8, IrType::U32);
            f.blocks[3].terminator = Terminator::Return(Some(Operand::Value(Value(live))));
            assert_eq!(invert_loops_with(&mut f, LoopInvertConfig::testing()), 1);
            assert!(matches!(
                f.blocks[1].terminator,
                Terminator::CondBranch { .. }
            ));
            assert!(matches!(
                f.blocks[2].instructions[0],
                Instruction::BinOp {
                    op: IrBinOp::Add,
                    ..
                }
            ));
        }
    }

    #[test]
    fn downcount_refuses_unproven_recurrences() {
        let mut signed = dead_counted_loop(8, IrType::I32);
        for inst in &mut signed.blocks[1].instructions {
            if let Instruction::Cmp { op, .. } = inst {
                *op = IrCmpOp::Slt;
            }
        }

        let mut step_two = dead_counted_loop(8, IrType::U32);
        for inst in &mut step_two.blocks[2].instructions {
            if let Instruction::BinOp { rhs, .. } = inst {
                *rhs = Operand::Const(IrConst::from_i64(2, IrType::U32));
            }
        }

        let mut variable_limit = dead_counted_loop(8, IrType::U32);
        variable_limit.blocks[0]
            .instructions
            .push(Instruction::Copy {
                dest: Value(30),
                src: Operand::Const(IrConst::from_i64(8, IrType::U32)),
            });
        for inst in &mut variable_limit.blocks[1].instructions {
            if let Instruction::Cmp { rhs, .. } = inst {
                *rhs = Operand::Value(Value(30));
            }
        }

        for (name, mut f) in [
            ("signed", signed),
            ("step two", step_two),
            ("variable limit", variable_limit),
        ] {
            assert_eq!(
                invert_loops_with(&mut f, LoopInvertConfig::testing()),
                1,
                "{name}"
            );
            assert!(
                matches!(f.blocks[1].terminator, Terminator::CondBranch { .. }),
                "{name}: entry guard was removed"
            );
            assert!(
                matches!(
                    f.blocks[2].instructions[0],
                    Instruction::BinOp {
                        op: IrBinOp::Add,
                        ..
                    }
                ),
                "{name}: recurrence was changed"
            );
        }
    }

    #[test]
    fn downcount_obeys_its_kill_switch() {
        let mut f = dead_counted_loop(8, IrType::U32);
        assert_eq!(
            invert_loops_with(&mut f, LoopInvertConfig::no_downcount()),
            1
        );
        assert!(matches!(
            f.blocks[1].terminator,
            Terminator::CondBranch { .. }
        ));
    }

    /// The canonical counted loop, exactly the `k_memchr` shape:
    ///   0 preheader -> 1
    ///   1 header : cmp ; i < n ? 2 : 4
    ///   2 body   : -> 3
    ///   3 latch  : i++ ; -> 1
    ///   4 exit
    fn counted_loop() -> IrFunction {
        func_of(vec![
            blk(0, vec![], br(1)),
            blk(1, vec![cmp(10, 20)], cond(10, 2, 4)),
            blk(2, vec![], br(3)),
            blk(3, vec![add1(20, 20)], br(1)),
            blk(4, vec![], ret()),
        ])
    }

    #[test]
    fn a_top_test_counted_loop_is_bottom_tested() {
        let mut f = counted_loop();
        assert_eq!(invert_loops_with(&mut f, LoopInvertConfig::testing()), 1);

        // The header keeps its test and becomes the guard, executed once.
        assert!(matches!(
            f.blocks[1].terminator,
            Terminator::CondBranch { .. }
        ));
        assert_eq!(f.blocks[1].instructions.len(), 1);

        // The latch now carries a COPY of the test and branches like the
        // header: the back edge goes to the body, not the header.
        let latch = &f.blocks[3];
        assert_eq!(latch.instructions.len(), 2, "increment + duplicated cmp");
        match &latch.terminator {
            Terminator::CondBranch {
                true_label,
                false_label,
                cond,
            } => {
                assert_eq!(true_label.0, 2, "back edge must target the body");
                assert_eq!(false_label.0, 4, "exit unchanged");
                // ...and it must test the LATCH's copy, not the guard's value.
                assert!(
                    matches!(cond, Operand::Value(v) if v.0 != 10),
                    "the duplicated test must define a fresh value"
                );
            }
            other => panic!("latch must end in a CondBranch, got {:?}", other),
        }
    }

    #[test]
    fn the_duplicated_test_reads_the_updated_induction_variable() {
        // Phi elimination puts `i = i + 1` at the end of the latch, so the
        // duplicated compare that follows it must read the NEW value -- that
        // is the whole point of a bottom test.
        let mut f = counted_loop();
        invert_loops_with(&mut f, LoopInvertConfig::testing());
        let latch = &f.blocks[3];
        let Instruction::Cmp { lhs, .. } = &latch.instructions[1] else {
            panic!("expected the duplicated Cmp second");
        };
        assert!(
            matches!(lhs, Operand::Value(v) if v.0 == 20),
            "must compare the incremented IV"
        );
    }

    #[test]
    fn value_ids_stay_unique_after_inversion() {
        let mut f = counted_loop();
        invert_loops_with(&mut f, LoopInvertConfig::testing());
        let mut seen = FxHashSet::default();
        for b in &f.blocks {
            for i in &b.instructions {
                if let Some(d) = i.dest() {
                    assert!(seen.insert(d.0), "duplicate definition of v{}", d.0);
                }
            }
        }
    }

    /// pr84524 end-to-end shape: an InlineAsm output lives at an ID above
    /// every `dest()` ID. The latch clone's fresh-ID allocator must seed from
    /// the sound high-water mark (InlineAsm outputs are definitions that
    /// `dest()` does not report) or it re-issues the asm output's ID, which
    /// demotes the asm output slot to indirect storage and miscompiles the
    /// asm-emit path (store through an uninitialized pointer).
    #[test]
    fn inversion_never_reuses_inline_asm_output_ids() {
        let asm_out = Value(55);
        let asm = Instruction::InlineAsm {
            template: String::new(),
            outputs: vec![("+r".to_string(), asm_out, None)],
            inputs: vec![("r".to_string(), Operand::Value(Value(99)), None)],
            clobbers: Vec::new(),
            operand_types: vec![IrType::U16, IrType::U16],
            goto_labels: Vec::new(),
            input_symbols: vec![None],
            seg_overrides: vec![
                crate::common::types::AddressSpace::Default,
                crate::common::types::AddressSpace::Default,
            ],
        };
        // The asm lives OUTSIDE the rotated loop (pr84524: the colliding asm
        // sat in a sibling inner loop while the outer loop was inverted);
        // the collision is purely an ID-allocation hazard across the function.
        let mut f = counted_loop();
        f.blocks[4].instructions.insert(0, asm);
        assert_eq!(invert_loops_with(&mut f, LoopInvertConfig::testing()), 1);

        let mut seen = FxHashSet::default();
        for b in &f.blocks {
            for i in &b.instructions {
                if let Some(d) = i.dest() {
                    assert!(seen.insert(d.0), "duplicate definition of v{}", d.0);
                }
                if let Instruction::InlineAsm { outputs, .. } = i {
                    for (_, v, _) in outputs {
                        assert!(
                            seen.insert(v.0),
                            "inline-asm output v{} re-defined elsewhere",
                            v.0
                        );
                    }
                }
            }
        }
        assert!(seen.contains(&55), "asm output id must survive");
        // The allocator high-water mark must be published for later passes.
        assert!(
            f.next_value_id > 55,
            "next_value_id must be bumped above the allocated range, got {}",
            f.next_value_id
        );
    }

    // ── negative controls ───────────────────────────────────────────────────

    #[test]
    fn a_header_containing_a_load_is_not_inverted() {
        // MEASURED DECISION, not a precaution. Duplicating a header load into
        // the latch is sound (same count, same order, same addresses), and it
        // was implemented; on a 4096-byte scan it then cost 15.9% against the
        // top-tested loop this pass leaves alone (49.3 ms vs 59.2 ms, 31
        // interleaved samples, checksums equal), and hand-reverting only the
        // loop shape in the emitted assembly recovered 100% of the loss while
        // leaving every other byte of the function identical.  The rotation is
        // therefore refused for memory headers; the reverted change and the
        // three experiments are recorded in
        // docs/SESSION_FOLLOWUP_S21_LOOP_ROTATION.md §5.
        let mut f = counted_loop();
        f.blocks[1].instructions.insert(
            0,
            Instruction::Load {
                dest: Value(30),
                ptr: Value(20),
                ty: IrType::I32,
                seg_override: Default::default(),
                volatile: false,
            },
        );
        assert_eq!(invert_loops_with(&mut f, LoopInvertConfig::testing()), 0);
    }

    #[test]
    fn a_header_containing_a_store_is_not_inverted() {
        // Same measurement: `for (i = 0; (a[i] = b[i]) != 0; i++)` is a memory
        // header, and the store's value is a header definition that would need
        // fresh ids and a home. No corpus kernel reaches this shape with a
        // rotation win, so it stays refused with the loads.
        let mut f = counted_loop();
        f.blocks[1].instructions.insert(
            0,
            Instruction::Store {
                val: Operand::Value(Value(20)),
                ptr: Value(21),
                ty: IrType::I32,
                seg_override: Default::default(),
                volatile: false,
            },
        );
        assert_eq!(invert_loops_with(&mut f, LoopInvertConfig::testing()), 0);
    }

    #[test]
    fn a_header_whose_value_escapes_into_the_body_is_not_inverted() {
        // The body would read the value the guard computed on its single
        // execution instead of this iteration's.
        let mut f = counted_loop();
        f.blocks[2].instructions.push(add1(31, 10));
        assert_eq!(invert_loops_with(&mut f, LoopInvertConfig::testing()), 0);
    }

    #[test]
    fn a_self_loop_is_not_inverted() {
        // Already bottom-tested; there is nothing to move.
        let mut f = func_of(vec![
            blk(0, vec![], br(1)),
            blk(1, vec![cmp(10, 20), add1(20, 20)], cond(10, 1, 2)),
            blk(2, vec![], ret()),
        ]);
        assert_eq!(invert_loops_with(&mut f, LoopInvertConfig::testing()), 0);
    }

    #[test]
    fn a_loop_with_two_latches_is_not_inverted() {
        //   1 header : ? 2 : 5 ; 2 : ? 3 : 4 ; 3 -> 1 ; 4 -> 1
        let mut f = func_of(vec![
            blk(0, vec![], br(1)),
            blk(1, vec![cmp(10, 20)], cond(10, 2, 5)),
            blk(2, vec![cmp(11, 20)], cond(11, 3, 4)),
            blk(3, vec![add1(20, 20)], br(1)),
            blk(4, vec![add1(21, 20)], br(1)),
            blk(5, vec![], ret()),
        ]);
        assert_eq!(invert_loops_with(&mut f, LoopInvertConfig::testing()), 0);
    }

    #[test]
    fn a_header_that_does_not_decide_the_exit_is_not_inverted() {
        // Both header successors are inside the loop: this is not a top-test
        // loop, so there is no test to sink.
        let mut f = func_of(vec![
            blk(0, vec![], br(1)),
            blk(1, vec![cmp(10, 20)], cond(10, 2, 3)),
            blk(2, vec![], br(4)),
            blk(3, vec![], br(4)),
            blk(4, vec![cmp(11, 20), add1(20, 20)], cond(11, 1, 5)),
            blk(5, vec![], ret()),
        ]);
        assert_eq!(invert_loops_with(&mut f, LoopInvertConfig::testing()), 0);
    }

    #[test]
    fn an_oversized_header_is_not_duplicated() {
        let mut f = counted_loop();
        for k in 0..(MAX_DUP_INSTS as u32 + 1) {
            f.blocks[1].instructions.insert(0, add1(40 + k, 20));
        }
        // Those values are used nowhere, so only the size guard can stop it.
        assert_eq!(invert_loops_with(&mut f, LoopInvertConfig::testing()), 0);
    }

    #[test]
    fn the_pass_can_be_disabled() {
        // Escape hatch for bisecting a codegen regression. Exercised through
        // the config parameter: setting the environment variable here would be
        // process-global and would race with the tests running in parallel.
        let mut f = counted_loop();
        let n = invert_loops_with(&mut f, LoopInvertConfig::disabled());
        assert_eq!(n, 0);
    }

    // ── AFFOLD-INV: the value-bound fold on the duplicated bottom test ──────
    //
    // The behavioural algebra (trip equivalence at the type extremes) is
    // pinned by `loop_rotate::affine_value_bound_tests` and by the runtime
    // gate `check_affine_value_bound.sh`; these tests pin the machine-level
    // STRUCTURE — where the subtraction lands, that the duplicated add dies,
    // which operands the rewritten compare reads — and every refusal clause.

    const IV: u32 = 20;
    const N: u32 = 30;
    const TMP: u32 = 21;

    /// `for (iv = seed; iv + 4 < n; iv++)` post-phi-elim:
    ///   0 preheader : n = param0 ; iv = seed       -> 1
    ///   1 header    : t0 = iv + 4 ; cc = t0 < n    ? 2 : 4
    ///   2 body      : -> 3
    ///   3 latch     : tmp = iv + 1 ; iv = tmp      -> 1
    ///   4 exit
    fn vb_loop(seed: Operand, step: i32, cmp_op: IrCmpOp, direct_inc: bool) -> IrFunction {
        let ty = IrType::I32;
        let affine = Instruction::BinOp {
            dest: Value(11),
            op: IrBinOp::Add,
            lhs: Operand::Value(Value(IV)),
            rhs: Operand::Const(IrConst::I32(4)),
            ty,
        };
        let test = Instruction::Cmp {
            dest: Value(10),
            op: cmp_op,
            lhs: Operand::Value(Value(11)),
            rhs: Operand::Value(Value(N)),
            ty,
        };
        let latch_insts = if direct_inc {
            vec![Instruction::BinOp {
                dest: Value(IV),
                op: IrBinOp::Add,
                lhs: Operand::Value(Value(IV)),
                rhs: Operand::Const(IrConst::I32(step)),
                ty,
            }]
        } else {
            vec![
                Instruction::BinOp {
                    dest: Value(TMP),
                    op: IrBinOp::Add,
                    lhs: Operand::Value(Value(IV)),
                    rhs: Operand::Const(IrConst::I32(step)),
                    ty,
                },
                Instruction::Copy {
                    dest: Value(IV),
                    src: Operand::Value(Value(TMP)),
                },
            ]
        };
        func_of(vec![
            blk(
                0,
                vec![
                    Instruction::ParamRef {
                        dest: Value(N),
                        param_idx: 0,
                        ty,
                    },
                    Instruction::Copy {
                        dest: Value(IV),
                        src: seed,
                    },
                ],
                br(1),
            ),
            blk(1, vec![affine, test], cond(10, 2, 4)),
            blk(2, vec![], br(3)),
            blk(3, latch_insts, br(1)),
            blk(4, vec![], ret()),
        ])
    }

    fn const_seed() -> Operand {
        Operand::Const(IrConst::I32(0))
    }

    /// The latch's compare, as `(lhs, op, rhs)`.
    fn latch_cmp(f: &IrFunction) -> (Operand, IrCmpOp, Operand) {
        f.blocks[3]
            .instructions
            .iter()
            .find_map(|i| match i {
                Instruction::Cmp { lhs, op, rhs, .. } => Some((*lhs, *op, *rhs)),
                _ => None,
            })
            .expect("inverted latch must carry a compare")
    }

    /// The guard's hoisted `n - 4`, if present.
    fn guard_sub(f: &IrFunction) -> Option<(u32, i64)> {
        f.blocks[1].instructions.iter().find_map(|i| match i {
            Instruction::BinOp {
                dest,
                op: IrBinOp::Sub,
                lhs: Operand::Value(n),
                rhs: Operand::Const(k),
                ..
            } if n.0 == N => Some((dest.0, k.to_i64().unwrap())),
            _ => None,
        })
    }

    #[test]
    fn vb_inv_folds_the_duplicated_test() {
        for direct in [false, true] {
            let mut f = vb_loop(const_seed(), 1, IrCmpOp::Slt, direct);
            assert_eq!(invert_loops_with(&mut f, LoopInvertConfig::testing()), 1);
            let (sub_dest, sub_k) = guard_sub(&f).expect("hoisted n - 4 in the guard");
            assert_eq!(sub_k, 4);
            // The duplicated add is gone; the compare reads the post-increment
            // IV against the pre-folded bound.
            let adds_of_4 = f.blocks[3]
                .instructions
                .iter()
                .filter(|i| {
                    matches!(
                        i,
                        Instruction::BinOp {
                            op: IrBinOp::Add,
                            rhs: Operand::Const(IrConst::I32(4)),
                            ..
                        }
                    )
                })
                .count();
            assert_eq!(adds_of_4, 0, "the duplicated affine add must die");
            assert_eq!(
                latch_cmp(&f),
                (
                    Operand::Value(Value(IV)),
                    IrCmpOp::Slt,
                    Operand::Value(Value(sub_dest))
                )
            );
            // The guard's own test is untouched (it still decides entry).
            assert!(f.blocks[1].instructions.iter().any(|i| matches!(
                i,
                Instruction::Cmp {
                    lhs: Operand::Value(v),
                    ..
                } if v.0 == 11
            )));
        }
    }

    #[test]
    fn vb_inv_obeys_the_affold_kill_switch() {
        // CCC_NO_AFFINE_EXIT_FOLD owns every producer: inversion must still
        // happen, the fold must not.
        let mut f = vb_loop(const_seed(), 1, IrCmpOp::Slt, false);
        assert_eq!(
            invert_loops_with(&mut f, LoopInvertConfig::no_affine_fold()),
            1
        );
        assert!(guard_sub(&f).is_none());
    }

    #[test]
    fn vb_inv_refuses_a_non_constant_seed() {
        // The -fwrapv re-entry trap: an IV that lives across outer
        // iterations re-enters with an unknown value, the guard's own add
        // may have wrapped, and the folded latch diverges.  Seed from a
        // parameter instead of a constant: invert, but never fold.
        let mut f = vb_loop(Operand::Value(Value(40)), 1, IrCmpOp::Slt, false);
        f.blocks[0].instructions.insert(
            0,
            Instruction::ParamRef {
                dest: Value(40),
                param_idx: 1,
                ty: IrType::I32,
            },
        );
        assert_eq!(invert_loops_with(&mut f, LoopInvertConfig::testing()), 1);
        assert!(guard_sub(&f).is_none());
    }

    #[test]
    fn vb_inv_refuses_step_two() {
        let mut f = vb_loop(const_seed(), 2, IrCmpOp::Slt, false);
        assert_eq!(invert_loops_with(&mut f, LoopInvertConfig::testing()), 1);
        assert!(guard_sub(&f).is_none());
    }

    #[test]
    fn vb_inv_refuses_non_strict_compare() {
        let mut f = vb_loop(const_seed(), 1, IrCmpOp::Sle, false);
        assert_eq!(invert_loops_with(&mut f, LoopInvertConfig::testing()), 1);
        assert!(guard_sub(&f).is_none());
    }

    #[test]
    fn vb_inv_refuses_unsigned_compare_on_signed_domain_mismatch() {
        // `ult` on a SIGNED i32 type: the domain check must refuse.
        let mut f = vb_loop(const_seed(), 1, IrCmpOp::Ult, false);
        assert_eq!(invert_loops_with(&mut f, LoopInvertConfig::testing()), 1);
        assert!(guard_sub(&f).is_none());
    }

    #[test]
    fn vb_inv_refuses_a_bound_written_in_the_loop() {
        let mut f = vb_loop(const_seed(), 1, IrCmpOp::Slt, false);
        f.blocks[2].instructions.push(Instruction::Copy {
            dest: Value(N),
            src: Operand::Const(IrConst::I32(7)),
        });
        assert_eq!(invert_loops_with(&mut f, LoopInvertConfig::testing()), 1);
        assert!(guard_sub(&f).is_none());
    }

    #[test]
    fn vb_inv_refuses_two_outside_entries() {
        // A second edge into the header: the constant seed no longer
        // dominates every entry, so the guard fact is unproven on one path.
        let mut f = vb_loop(const_seed(), 1, IrCmpOp::Slt, false);
        f.blocks.push(blk(
            5,
            vec![Instruction::Copy {
                dest: Value(IV),
                src: Operand::Const(IrConst::I32(1)),
            }],
            br(1),
        ));
        // Make it reachable so CfgAnalysis keeps it: entry now forks.
        f.blocks[0].terminator = Terminator::CondBranch {
            cond: Operand::Value(Value(N)),
            true_label: BlockId(1),
            false_label: BlockId(5),
        };
        assert_eq!(invert_loops_with(&mut f, LoopInvertConfig::testing()), 1);
        assert!(guard_sub(&f).is_none());
    }
}
