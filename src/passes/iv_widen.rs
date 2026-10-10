//! Induction-variable widening with **provenance-tracked derived closure**.
//!
//! Detects narrow (`I8`/`U8`/`I16`/`U16`/`I32`/`U32`) basic induction
//! variables whose 64-bit-context use is addressing (a GEP offset — directly,
//! or through a widening `Cast`, a const-scale `Shl`/`Mul`, or a const-offset
//! `Add`/`Sub` chain), and widens them to pointer-width (`I64`/`U64` on LP64).
//!
//! The transform eliminates the per-iteration `movslq`/`movsbl` the x86-64
//! backend must re-emit after every narrow `addl`/`addb` (a narrow op clobbers
//! the upper bits, so the next 64-bit addressing use would read garbage
//! without a re-extension on the loop-carried path). After widening, the
//! latch op is `addq`/`subq` and the IV is consumed directly by the GEP with
//! no `Cast` and no re-extension.
//!
//! ```text
//!   BEFORE (per-iteration movslq):          AFTER (no movslq):
//!   .LBB5:                                    .LBB5:
//!     movslq %ebp, %rbx            ;dead        movb $0, (%r14, %rbp)
//!     movb   $0, (%r14, %rbp)                    addq %r15, %rbp
//!     addl   %r15d, %ebp                         cmpl $10000000, %ebp
//!     movslq %ebp, %rbp            ;re-ext      jle .LBB5
//!     cmpl   $10000000, %ebp
//!     jle    .LBB5
//! ```
//!
//! # Soundness theorems
//!
//! Every widening decision is backed by one of the following provable
//! identities, each with an explicit evidence kind recorded in the plan:
//!
//! 1. **Addressing anchors UB.** For the transform to be legal, the IV must
//!    have at least one GEP-offset use. On every *defined* execution the
//!    index stays in-bounds, so the wide recurrence agrees with the narrow
//!    recurrence on all iterations the C program defines.
//!
//! 2. **Signed-overflow-is-UB (signed `Add`/`Sub`/`Mul`).** For a signed
//!    member, `sext(x op c) == sext(x) op sext(c)` holds whenever the narrow
//!    `x op c` does not wrap, and a wrap is itself undefined behaviour in the
//!    source. Hence the identity holds on all defined iterations with *no
//!    range analysis*.
//!
//! 3. **Bitwise identity (`And`/`Or`/`Xor` with constants).**
//!    `ext(x op c) == ext(x) op ext(c)` for `op ∈ {And, Or, Xor}` and any
//!    constant `c`, for both `sext` and `zext` — unconditional, bit-level.
//!    (The prior closure attempt tried to prove `Or` via interval bounds and
//!    got it wrong; the correct proof is bitwise.)
//!
//! 4. **`sext` preserves both comparison orders.** For 32-bit `a, b`:
//!    `a <s b ⟺ sext(a) <s sext(b)` and `a <u b ⟺ sext(a) <u sext(b)`.
//!    A sext-widened IV can therefore be compared at I64 with *any*
//!    predicate, provided the other operand is also sext-widened.
//!    (`zext` preserves only the unsigned order, so a zext-widened IV may
//!    only use unsigned predicates and equality.)
//!
//! 5. **Counted-loop bound (unsigned).** Unsigned wrap is *defined*, so an
//!    unsigned IV may be widened only when the loop provably terminates
//!    inside the type's range: an exit test `i < n` with a unit step (body
//!    values `≤ n-1 ≤ 2^w-2`, latch result `≤ 2^w-1` — no wrap anywhere), or
//!    `i > bound` with step `-1`. The bound may be a RUNTIME loop-invariant
//!    value — the no-wrap argument is by construction (any seed `< n`,
//!    unit steps, exit at `≥ n` — every executed value is `< 2^w`), not by
//!    constant folding. This is the same discipline SCEV-based compilers
//!    apply.
//!
//! 5′. **Trajectory of the counted bound (exposed latches).** Theorem 5's
//!    no-wrap construction assumes every body pass is *gated* by the exit
//!    test — true for a phi-guarded header (`preheader → header(cmp) → body
//!    → latch → header`) and FALSE for rotated/bottom-tested loops, whose
//!    first body pass runs at the untested seed. At a ring-top seed
//!    (`0xFFFFFFFF`) the narrow latch wraps to 0 where the wide latch does
//!    not, and every consumer of that divergent value sees the wide one
//!    (the IVW-RANGE-1 miscompile: a rotated do-while read `buf[2^32]`
//!    instead of `buf[0]`). Theorem 5′ therefore classifies the proof by the
//!    cmp's trajectory position — `latch_exposed = cmp_is_latch ∨
//!    ¬cmp_is_header ∨ ¬header_clean` — behind three structural gates (body
//!    entry closure, no inner cycle through the latch, and obligation P:
//!    branch-target LABELS resolved to block INDICES before any body-membership
//!    test, the two numbering domains diverging after every CFG
//!    simplification):
//!
//!    * **phi-guarded** proofs keep the STRICT post-entry range (`[0, n-1]`
//!      incrementing): the seed itself never reaches the body untested, so
//!      no value can wrap and the whole closure widens as under theorem 5.
//!    * **latch-exposed** proofs use the SEED-INCLUSIVE range: a const seed
//!      (resolved through the single `Copy{src: Const}` level mem2reg leaves)
//!      folds into `[0, max(seed, n-1+mag)]` and widens normally; a RUNTIME
//!      seed admits only the full domain `[0, max]`, and the worklist
//!      INTERCEPT takes over: of the divergent (non-value-exact) members,
//!      only bit-exact `& const` reads are admitted (theorem 3 holds for
//!      every value, divergent or not), guard cmps and out-of-loop escapes
//!      are repaired by truncation (the narrow bits of a diverged wide value
//!      ARE the narrow value), and Cast/GEP/Intrinsic offset slots DECLINE —
//!      a wide address computed from a value the narrow program wrapped is
//!      a different address. Arithmetic folds decline by themselves: the
//!      seed-inclusive `[0, max]` range overflows the narrow type at the
//!      first `+mag`.
//!    * Signed predicates (`Slt`/`Sle`/`Sgt`/`Sge`) on an UNSIGNED phi
//!      decline the whole proof: a signed test does not confine an unsigned
//!      value (`0xFFFFFFFF <s 64` is true), so no unsigned range follows
//!      from it. Both branch polarities are recognised: an exit-on-true arm
//!      normalises to the negated continue-condition before the bound is
//!      read.

//!
//! # Provenance
//!
//! The derived **closure** is built forward from the seed phi. Every admitted
//! member records *which* already-admitted value it derives from
//! (`MemberKind`), so membership is a proof tree, not a map lookup — the
//! failure mode of the previous closure attempt (an `And(v81, 31)` member
//! whose operand `v81` was never admitted) is structurally impossible here:
//! an instruction can only join the closure after its operand has joined, and
//! `verify_plan` re-checks every recorded operand/opcode/type against the
//! live IR before any mutation.
//!
//! # Supported shapes (beyond the plain `Cast → GEP` chain)
//!
//! - `I32`/`U32` IVs, signed and unsigned, incrementing and decrementing
//!   (`Add`/`Sub` latches, both operand orders, and reversed `Sub(step, phi)`
//!   rejected).
//! - Const-offset addressing: `a[i+1]`, `a[i-1]`, `a[i*3]`, `a[i&7]`,
//!   `a[(i&7)+1]`, `s[i-1] = s[i] + p` (the closure shapes that previously
//!   kept `movslq` on the carried path).
//! - Narrow constant-count shifts `a[i<<1]` (`Shl` both signednesses, `AShr`
//!   signed, `LShr` unsigned): the shift count is a shift AMOUNT and is never
//!   widened; only the member and the op's type are retyped.
//! - Signedness-changing widening casts (`U32→I64` / `I32→U64`), which the
//!   C frontend emits for unsigned index chains (`unsigned i; a[i>>1]`):
//!   the cast value equals the widened member on every executed iteration,
//!   so it is retained as a same-size reinterpretation of the wide value and
//!   the chain below it is enqueued and classified normally (`CrossCast`) —
//!   that is where the I64 scaling and the GEP live.
//! - Same-width casts (C's `int < unsigned` promotion) are retained as
//!   truncations rather than widened, keeping the mixed-signedness cmp exact.
//! - Rotated self-loops (header == latch) are widenable when a preheader
//!   exists for the hoists.
//! - Duplicated latch steps (`a[(i + 1) & m]` + `while (i++ < n)` — the
//!   frontend emits two value-identical `Add(phi, c)`s because iv_widen runs
//!   ahead of the merging CSE): `canonicalise_latch_twin` redirects the back
//!   edge to the earlier twin and deletes the dead original, routing the
//!   shape through the intercept's bit-exact And arm instead of a fold that
//!   overflows at the ring top.
//! - Loop-exit comparisons that cannot be safely widened (incompatible
//!   predicate, a loop-variant other operand, or an other operand that is not
//!   available in the preheader, e.g. a bound reloaded after the loop) keep a
//!   narrow cmp fed by one truncation, instead of aborting the whole widening.
//!
//! Out-of-scope (the pass declines the IV):
//! - `i8`/`i16` IVs: the C int-promotion latch is `trunc(add(phi, 1):I32)`,
//!   not a narrow `add`, and widening it would change the DEFINED 8/16-bit
//!   wrap of the truncation unless a no-wrap bound is proven (see
//!   engineering/FOLLOWUP-2026-09-02-iv-widen-agentb-audit.md, §5). Candidacy
//!   is therefore restricted to `I32`/`U32`; extending it requires a
//!   narrow-width no-wrap bound proof first.
//! - loop-variant latch steps (an invariant step is cast once in the
//!   preheader; a variant step cannot be hoisted);
//! - narrow shifts with a non-constant or out-of-range count, `AShr` on
//!   unsigned IVs, and `LShr` on signed IVs (the mismatched cases are not
//!   extension-transparent);
//! - `Sub(step, phi)` latches (not an induction recurrence);
//! - IVs used in `Select` *conditions* or other narrow in-loop arithmetic;
//! - exit-merge phis (a phi operand is evaluated on the edge, so its
//!   truncation would run once per iteration);
//! - unsigned IVs without a provable counted-loop bound.
//!
//! The pass runs AFTER IVSR + univsr (the residual scalar IVs are the
//! widening candidates) and BEFORE loop_rotate (so the rotated form sees the
//! widened phi and emits `addq` directly).

use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::common::types::IrType;
use crate::ir::analysis::{CfgAnalysis, FlatAdj};
use crate::ir::constants::IrConst;
use crate::ir::instruction::{BasicBlock, Instruction, Operand, Terminator, Value};
use crate::ir::reexports::{
    BlockId, IrBinOp, IrCmpOp, IrFunction, replace_operand_value, replace_terminator_value,
};
use crate::passes::loop_analysis::{
    DominanceChecker, NaturalLoop, find_natural_loops, merge_loops_by_header,
};

/// Entry point used by the dirty-tracking pipeline.
pub(crate) fn run_function(func: &mut IrFunction) -> usize {
    widen_ivs_in_function(func)
}

/// Per-function driver: returns the number of IVs widened.
fn widen_ivs_in_function(func: &mut IrFunction) -> usize {
    if func.blocks.len() < 3 {
        return 0;
    }
    // LP64 only: on ILP32 widening to I64 is the wrong direction.
    if crate::common::types::target_ptr_size() != 8 {
        return 0;
    }
    let debug = std::env::var("CCC_IV_WIDEN_DEBUG").is_ok();
    if std::env::var("CCC_NO_IV_WIDEN").is_ok() {
        return 0;
    }

    let mut total = 0usize;
    // Iterate to fixpoint: widening one IV can expose another (nested loops
    // where the outer IV initializes the inner). Small cap for pathology.
    for _ in 0..8 {
        let cfg = CfgAnalysis::build(func);
        if cfg.num_blocks < 3 {
            break;
        }
        let raw = find_natural_loops(cfg.num_blocks, &cfg.preds, &cfg.succs, &cfg.idom);
        if raw.is_empty() {
            break;
        }
        let loops = merge_loops_by_header(raw);
        let dom = DominanceChecker::new(cfg.num_blocks, &cfg.idom);
        // Innermost-first: smallest body. Widening an inner IV does not
        // disturb outer-loop analysis.
        let mut sorted: Vec<&NaturalLoop> = loops.iter().collect();
        sorted.sort_by_key(|lp| lp.body.len());
        let mut did = false;
        for lp in sorted {
            if try_widen_loop(func, lp, &cfg, &dom, debug) {
                total += 1;
                did = true;
                break; // IR changed — rebuild before the next candidate.
            }
        }
        if !did {
            break;
        }
    }
    if debug && total > 0 {
        eprintln!("[IV-WIDEN] {}: widened {} IVs", func.name, total);
    }
    total
}

// ---------------------------------------------------------------------------
// Extension kind and constant widening
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExtKind {
    Se,
    Ze,
}

fn is_unsigned_ty(ty: IrType) -> bool {
    matches!(ty, IrType::U8 | IrType::U16 | IrType::U32 | IrType::U64)
}

fn ext_kind_for(ty: IrType) -> ExtKind {
    if is_unsigned_ty(ty) {
        ExtKind::Ze
    } else {
        ExtKind::Se
    }
}

fn wide_ty_for(ty: IrType) -> IrType {
    if is_unsigned_ty(ty) {
        IrType::U64
    } else {
        IrType::I64
    }
}

fn narrow_bit_width(ty: IrType) -> u32 {
    match ty {
        IrType::I8 | IrType::U8 => 8,
        IrType::I16 | IrType::U16 => 16,
        IrType::I32 | IrType::U32 => 32,
        _ => 64,
    }
}

fn type_min_i128(ty: IrType) -> i128 {
    let w = narrow_bit_width(ty);
    if is_unsigned_ty(ty) {
        0
    } else {
        -(1i128 << (w - 1))
    }
}

fn type_max_i128(ty: IrType) -> i128 {
    let w = narrow_bit_width(ty);
    if is_unsigned_ty(ty) {
        (1i128 << w) - 1
    } else {
        (1i128 << (w - 1)) - 1
    }
}

/// Widen an integer constant from `from_ty` to 64 bits with the extension
/// kind implied by `from_ty`. Critical for unsigned: a `U32` constant whose
/// bit 31 is set must become `0xFFFFFFFF` (4294967295), **not** `-1` — the
/// previous revision used `to_i64()` which sign-extends the raw bit pattern
/// and silently flipped such constants.
/// Widen an integer constant from `from_ty` to 64 bits with the extension
/// kind implied by `from_ty`. Critical for unsigned: a `U32` constant whose
/// bit 31 is set must become `0xFFFFFFFF` (4294967295), **not** `-1` — the
/// previous revision used `to_i64()` which sign-extends the raw bit pattern
/// and silently flipped such constants.
///
/// Returns `None` for a non-integer constant. A floating-point constant here
/// means an *integer* closure member was admitted from a float-typed
/// operation, which is a classifier bug: `to_i64()` on `F32(11.0)` yields
/// `0`, which is how `11.0f * (float) i` once became a hard zero. Callers
/// must decline the member rather than silently widen garbage.
fn widen_const(c: IrConst, from_ty: IrType) -> Option<IrConst> {
    if !matches!(
        c,
        IrConst::I8(_) | IrConst::I16(_) | IrConst::I32(_) | IrConst::I64(_) | IrConst::Zero
    ) {
        debug_assert!(
            false,
            "iv_widen: non-integer constant {c:?} reached widen_const (from_ty {from_ty:?})"
        );
        return None;
    }
    let raw = c.to_i64().unwrap_or(0) as u64;
    let bits = narrow_bit_width(from_ty);
    let val = if is_unsigned_ty(from_ty) {
        if bits >= 64 {
            raw
        } else {
            raw & ((1u64 << bits) - 1)
        }
    } else if bits >= 64 {
        raw
    } else {
        let shift = 64 - bits;
        ((raw << shift) as i64 >> shift) as u64
    };
    Some(IrConst::I64(val as i64))
}

// ---------------------------------------------------------------------------
// Provenance engine
// ---------------------------------------------------------------------------

/// A provable mathematical value range over the member's *narrow* value
/// domain, in i128 to avoid overflow during range arithmetic. Only meaningful
/// for unsigned (`Ze`) members; signed members rely on the
/// signed-overflow-is-UB theorem instead and keep `range = None`.
#[derive(Debug, Clone, Copy)]
struct Range {
    lo: i128,
    hi: i128,
}

impl Range {
    fn within(&self, ty: IrType) -> bool {
        self.lo >= type_min_i128(ty) && self.hi <= type_max_i128(ty)
    }
}

/// How a closure member is derived from the seed. Every variant names the
/// exact already-admitted operand(s) it consumes — this is the provenance
/// that makes the closure a proof tree rather than a map membership test.
#[derive(Debug, Clone)]
enum MemberKind {
    /// The seed phi itself.
    Seed,
    /// The latch `Add`/`Sub` result (the post-increment value).
    LatchResult,
    /// `y = x op c` with `c` a constant (narrow width before widening).
    /// `const_is_lhs` tells which side holds the constant.
    BinOpConst {
        op: IrBinOp,
        operand: Value,
        c: IrConst,
        const_is_lhs: bool,
    },
    /// `y = x op m` where `m` is another admitted member (signed IVs only).
    BinOpMember { op: IrBinOp, operand: Value },
    /// `y = x op k` for `op ∈ {Shl, AShr, LShr}` with `k` a constant SHIFT
    /// COUNT. The count is never widened (it is a shift amount, not a value);
    /// only the member and the op's type are retyped to wide.
    ///
    /// Soundness: `AShr` is exact for sext (`sext(x >> k) == sext(x) >> k` for
    /// every `x`), and `LShr` is exact for zext, unconditionally. `Shl` is
    /// exact under the signed-overflow-is-UB theorem for signed IVs, and
    /// requires a non-wrapping range proof for unsigned IVs.
    ShiftConst {
        op: IrBinOp,
        operand: Value,
        count: IrConst,
    },
    /// A widening cast of a member (`to_ty` ≥ 32 bits, same signedness).
    /// Dropped after its uses are redirected to the underlying member's wide
    /// value.
    WidenCast { operand: Value },
    /// A narrowing, same-width or pointer cast of a member. Retained and
    /// retyped to a truncation of the wide value (`from_ty = wide`), so its
    /// consumers keep reading the exact narrow bits.
    NarrowCast { operand: Value },
    /// A size-widening cast of a member that *changes* signedness
    /// (`U32→I64` / `I32→U64` — the C frontend produces these when an
    /// unsigned index feeds a signed-width GEP chain, e.g. `unsigned i;
    /// a[i>>1]`, and for `(unsigned long)i` index expressions). The cast's
    /// value equals the widened member on every executed iteration (the
    /// extension semantics follow the 32-bit source's signedness, which is
    /// exactly the member's `ext`; the no-wrap domain — signed-overflow-is-UB
    /// for signed seeds, the counted bound for unsigned seeds — makes the
    /// wide member bit-identical to the narrow one). The cast is therefore
    /// retained and retyped to a same-size `wide_ty → to_ty` cast — a pure
    /// reinterpretation the backend emits as a no-op — and its dest is
    /// enqueued as a chain value so the wide chain below it (the I64 index
    /// scaling and the GEP) is classified and can set `has_addressing`.
    CrossCast { operand: Value },
    /// `y = copy(x)`.
    Copy { operand: Value },
    /// `y = select(cond, a, b)` where `a` is a member and `b` is a const or a
    /// member. The condition is never the IV.
    Select { cond: Operand, a: Value, b: Operand },
}

#[derive(Debug, Clone)]
struct Member {
    value: Value,
    kind: MemberKind,
    /// The member's narrow (pre-widening) IR type.
    orig_ty: IrType,
    ext: ExtKind,
    range: Option<Range>,
}

impl Member {
    fn of(
        value: Value,
        kind: MemberKind,
        orig_ty: IrType,
        ext: ExtKind,
        range: Option<Range>,
    ) -> Self {
        Member {
            value,
            kind,
            orig_ty,
            ext,
            range,
        }
    }
}

/// The other operand of a cmp, and how to widen it.
#[derive(Debug, Clone)]
enum CmpOther {
    /// A constant, widened inline from the cmp's original type.
    Const(IrConst),
    /// A loop-invariant value: a widening cast is hoisted to the preheader.
    /// `from_ty` is the value's defining type (chooses sext vs zext).
    InvariantValue { value: Value, from_ty: IrType },
    /// An already-64-bit value: used directly, no hoist.
    WideValue(Value),
}

/// What to do with one `Cmp` that reads a widened value.
#[derive(Debug, Clone)]
enum CmpAction {
    /// Retype the cmp to the wide type; widen the other operand. The member
    /// side already reads the correct (wide) value and is left untouched.
    Widen {
        cmp_dest: Value,
        member: Value,
        other: CmpOther,
        iv_is_lhs: bool,
        cmp_ty: IrType,
    },
    /// Keep the cmp narrow; insert one truncation of the member's current
    /// value right before it.
    Trunc {
        cmp_dest: Value,
        member: Value,
        iv_is_lhs: bool,
    },
}

/// The complete widening plan for one IV. Built with no mutation; verified
/// against the live IR before anything is applied.
struct WidenPlan {
    phi_dest: Value,
    phi_ty: IrType,
    wide_ty: IrType,
    ext: ExtKind,
    init: Operand,
    latch_dest: Value,
    latch_is_sub: bool,
    step: Operand,
    /// Topological: [0] is the seed; [1] is the latch result when present.
    members: Vec<Member>,
    cmps: Vec<CmpAction>,
    /// (block_idx, member_value) — one truncation per escaping block. Only
    /// members whose consumers expect the narrow value appear here.
    escapes: Vec<(usize, Value)>,
    has_addressing: bool,
}

// ---------------------------------------------------------------------------
// Per-loop driver
// ---------------------------------------------------------------------------

struct PhiCandidate {
    phi_dest: Value,
    phi_ty: IrType,
    init_op: Operand,
    latch_op_dest: Value,
    latch_is_sub: bool,
}

/// Try to widen exactly one IV in `lp`. Returns true if a widening was
/// applied.
fn try_widen_loop(
    func: &mut IrFunction,
    lp: &NaturalLoop,
    cfg: &CfgAnalysis,
    dom: &DominanceChecker,
    debug: bool,
) -> bool {
    let Some(preheader) = lp.find_preheader(&cfg.preds) else {
        return false;
    };
    let Some(latch) = lp.single_latch(&cfg.preds) else {
        return false;
    };
    // A self-loop (header == latch) is the rotated shape; still widenable as
    // long as a preheader exists for the hoists.
    let preheader_label = func.blocks[preheader].label;
    let latch_label = func.blocks[latch].label;

    // Canonicalise chained latch recurrences first (see
    // `canonicalise_chained_latch`): every recogniser below requires the latch
    // to name the phi directly, and an unrolled counted loop never does.
    let chained: Vec<(Value, Value)> = func.blocks[lp.header]
        .instructions
        .iter()
        .filter_map(|inst| match inst {
            Instruction::Phi { dest, ty, incoming }
                if matches!(*ty, IrType::I32 | IrType::U32) && incoming.len() == 2 =>
            {
                incoming
                    .iter()
                    .find(|(_, blk)| *blk == latch_label)
                    .and_then(|(op, _)| match op {
                        Operand::Value(v) => Some((*dest, *v)),
                        Operand::Const(_) => None,
                    })
            }
            _ => None,
        })
        .collect();
    for (phi_dest, latch_val) in chained {
        canonicalise_chained_latch(func, latch, latch_val, phi_dest);
        // A duplicated step (`a[(i + 1) & m]` + `while (i++ < n)`) leaves a
        // value-identical twin ahead of the latch value in the same block;
        // redirecting the back edge to it routes the body read through the
        // intercept's bit-exact And arm (see `canonicalise_latch_twin`).
        canonicalise_latch_twin(func, latch, latch_val, phi_dest);
    }

    let candidates: Vec<PhiCandidate> =
        collect_widenable_phis(func, lp, preheader_label, latch, latch_label);
    if candidates.is_empty() {
        return false;
    }

    let uses = build_use_map(func);

    for cand in &candidates {
        let hoist = PreheaderSite { preheader, dom };
        let Some(plan) = analyze_iv(
            func,
            lp,
            &cfg.preds,
            &cfg.succs,
            &uses,
            &hoist,
            cand.phi_dest,
            cand.phi_ty,
            cand.init_op,
            latch,
            cand.latch_op_dest,
            cand.latch_is_sub,
        ) else {
            continue;
        };
        if !plan.has_addressing {
            continue;
        }
        // The step must be a constant or loop-invariant value — a variant
        // step would need a per-iteration cast (no win) or, worse, a
        // preheader cast of a loop-defined value (invalid SSA). The previous
        // revision checked only the cmp's other operand, not the step.
        if !operand_hoistable_to_preheader(&plan.step, lp, func, plan.phi_dest, &hoist) {
            if debug {
                eprintln!(
                    "[IV-WIDEN] skip phi {:?}: latch step not loop-invariant",
                    cand.phi_dest
                );
            }
            continue;
        }
        if !verify_plan(func, &plan, lp, &hoist) {
            if debug {
                eprintln!(
                    "[IV-WIDEN] skip phi {:?}: plan failed live-IR verification",
                    cand.phi_dest
                );
            }
            continue;
        }
        if apply_widen(func, &plan, preheader, debug) {
            return true;
        }
    }
    false
}

/// Collect narrow phi candidates in the header that have a 2-incoming shape
/// (preheader init + latch step) whose latch value is produced by
/// `Add(phi, step)` / `Add(step, phi)` / `Sub(phi, step)`.
/// Maximum chain links followed by [`canonicalise_chained_latch`].  Unroll
/// factors are small; the cap only bounds the walk on pathological IR.
const LATCH_CHAIN_HOP_LIMIT: usize = 64;

/// Re-spell a CHAINED latch recurrence as a single stride: `phi + total`.
///
/// An unrolled counted loop does not spell its latch `i += 4`.  It spells it
/// `v1 = i+1; v2 = v1+1; v3 = v2+1; i' = v3+1`, because every intermediate is
/// also an addressing offset the body needs (`p[i+1]`, `p[i+2]`, `p[i+3]`).
/// Value numbering will not collapse that chain: re-associating `v2` to `i+2`
/// leaves `v1` live for its own offset use, so the rewrite would add an
/// instruction instead of removing one, and a local simplifier rightly declines
/// it.
///
/// Every recogniser in this pass — `latch_step_uses_phi`, `plan_step_operand`,
/// the `verify_plan` re-derivation and the apply-phase latch rewrite — requires
/// the latch to name the phi directly, so a chained latch is not a candidate at
/// all and the whole loop stays narrow.  Collapsing ONLY the final link
/// canonicalises the recurrence at zero cost:
///
/// * no instruction is added, removed or duplicated — the last link is rewritten
///   in place and the intermediates keep their offset uses;
/// * the value is identical with no bound argument whatsoever, because modular
///   addition is associative: `(((i+1)+1)+1)+1 == i+4` in the u32 ring for every
///   `i`, wrapping or not.  For a signed chain the two spellings differ only on
///   executions whose intermediate step overflows, which C17 6.5/5 makes
///   undefined, so defined behaviour cannot change.
///
/// Returns true when the latch instruction was rewritten.  The chain must be
/// local to the latch block, must consist solely of `Add`/`Sub` against integer
/// constants with the recurrence on the left of a `Sub`, and must terminate at
/// the phi; anything else leaves the IR untouched.
fn canonicalise_chained_latch(
    func: &mut IrFunction,
    latch: usize,
    latch_val: Value,
    phi: Value,
) -> bool {
    if latch_val == phi {
        return false;
    }
    // Collect the chain links: (dest, op, recurrence operand, const).
    let mut links: Vec<(Value, IrBinOp, Value, IrConst)> = Vec::new();
    for inst in &func.blocks[latch].instructions {
        if let Instruction::BinOp {
            dest,
            op: op @ (IrBinOp::Add | IrBinOp::Sub),
            lhs,
            rhs,
            ty: _,
        } = inst
        {
            let (rec, k) = match (lhs, rhs) {
                // `Sub(c, x)` is not an induction recurrence; skip the link.
                (Operand::Value(v), Operand::Const(c)) => (*v, *c),
                (Operand::Const(c), Operand::Value(v)) if *op == IrBinOp::Add => (*v, *c),
                _ => continue,
            };
            links.push((*dest, *op, rec, k));
        }
    }
    let link_of = |v: Value| links.iter().find(|(d, _, _, _)| *d == v).copied();

    // Walk latch_val back to the phi, accumulating the signed stride.
    let mut total: i128 = 0;
    let mut cur = latch_val;
    let mut visited: Vec<Value> = Vec::new();
    while cur != phi {
        if visited.len() >= LATCH_CHAIN_HOP_LIMIT || visited.contains(&cur) {
            return false;
        }
        let Some((_, op, rec, k)) = link_of(cur) else {
            return false;
        };
        let Some(mag) = k.to_i64() else {
            return false;
        };
        let mag = mag as i128;
        total += match op {
            IrBinOp::Add => mag,
            // The recurrence is the left operand, so this link is `rec - mag`.
            _ => -mag,
        };
        visited.push(cur);
        cur = rec;
    }
    // One link is already canonical; a zero stride is not an IV at all.
    if visited.len() < 2 || total == 0 {
        return false;
    }
    let (op, mag) = if total > 0 {
        (IrBinOp::Add, total)
    } else {
        (IrBinOp::Sub, -total)
    };
    // The stride must fit the i64 constant carrier; a chain of huge constants
    // is left alone rather than truncated.
    let Ok(mag) = i64::try_from(mag) else {
        return false;
    };
    for inst in &mut func.blocks[latch].instructions {
        if let Instruction::BinOp {
            dest,
            op: slot_op,
            lhs,
            rhs,
            ty: _,
        } = inst
        {
            if *dest != latch_val {
                continue;
            }
            *slot_op = op;
            *lhs = Operand::Value(phi);
            *rhs = Operand::Const(IrConst::I64(mag));
            return true;
        }
    }
    false
}

/// Canonicalise a DUPLICATED latch step onto its earlier twin.
///
/// When the source spells the step twice — `a[(i + 1) & m]` inside the body
/// plus `while (i++ < n)` in the condition — the frontend emits two
/// value-identical `Add(phi, c)` instructions in the latch block, because
/// iv_widen runs ahead of the CSE that would merge them.  The twin that is
/// NOT the latch value is an ordinary body member: under theorem 5′ it must
/// fold from the seed-inclusive range, which overflows the narrow type at
/// the ring top (`[0, max] + 1`), declining plans whose only body read is
/// the bit-exact `& m` the intercept would happily admit.
///
/// Redirecting the back edge to the earlier twin and deleting the dead
/// original makes the body read the LATCH member instead, routing the shape
/// through the intercept's unconditional And arm.  Value identity needs no
/// bound argument (same op, same operands, same type, pure SSA), and
/// dominance is trivial: the twin is earlier in the same block.
///
/// Returns true when the back edge was redirected.  Declines when the latch
/// value has any use beyond the phi's back-edge incoming (the original is
/// not dead and removing it would strand those uses), when no earlier
/// value-identical twin exists, or when the latch value is not a step of
/// the phi at all.
fn canonicalise_latch_twin(
    func: &mut IrFunction,
    latch: usize,
    latch_val: Value,
    phi: Value,
) -> bool {
    if latch_val == phi {
        return false;
    }
    // Locate the latch instruction and its exact shape.
    let mut latch_pos: Option<usize> = None;
    let mut latch_key: Option<(IrBinOp, Operand, Operand, IrType)> = None;
    for (ii, inst) in func.blocks[latch].instructions.iter().enumerate() {
        if let Instruction::BinOp {
            dest,
            op,
            lhs,
            rhs,
            ty,
        } = inst
        {
            if *dest == latch_val {
                latch_pos = Some(ii);
                latch_key = Some((*op, *lhs, *rhs, *ty));
            }
        }
    }
    let (Some(latch_pos), Some((op, lhs, rhs, ty))) = (latch_pos, latch_key) else {
        return false;
    };
    let reads_phi = matches!(lhs, Operand::Value(v) if v == phi)
        || matches!(rhs, Operand::Value(v) if v == phi);
    if !reads_phi {
        return false;
    }
    // The earlier value-identical twin in the SAME block (dominance is then
    // trivial: same block, earlier position).
    let mut twin: Option<Value> = None;
    for inst in func.blocks[latch].instructions.iter().take(latch_pos) {
        if let Instruction::BinOp {
            dest,
            op: o2,
            lhs: l2,
            rhs: r2,
            ty: t2,
        } = inst
        {
            if *o2 == op && *l2 == lhs && *r2 == rhs && *t2 == ty {
                twin = Some(*dest);
                break;
            }
        }
    }
    let Some(twin) = twin else {
        return false;
    };
    // The latch value must be dead once the back edge moves: its only
    // remaining readers may be the phi's own incoming (redirected below).
    for b in &func.blocks {
        for inst in &b.instructions {
            if let Instruction::Phi { dest, .. } = inst {
                if *dest == phi {
                    continue;
                }
            }
            let mut used = false;
            inst.for_each_used_value(|id| {
                if id == latch_val.0 {
                    used = true;
                }
            });
            if used {
                return false;
            }
        }
        let mut tused = false;
        b.terminator.for_each_used_value(|id| {
            if id == latch_val.0 {
                tused = true;
            }
        });
        if tused {
            return false;
        }
    }
    // Redirect the phi's latch-side incoming to the twin (matched by edge
    // label AND value so a coincidentally-equal init operand on the
    // preheader edge can never be corrupted).
    let latch_label = func.blocks[latch].label;
    for b in &mut func.blocks {
        for inst in &mut b.instructions {
            if let Instruction::Phi { dest, incoming, .. } = inst {
                if *dest != phi {
                    continue;
                }
                for (slot, blk) in incoming.iter_mut() {
                    if *blk == latch_label && matches!(slot, Operand::Value(v) if *v == latch_val) {
                        *slot = Operand::Value(twin);
                    }
                }
            }
        }
    }
    // Remove the dead original (source_spans stay parallel via remove_inst).
    remove_inst(&mut func.blocks[latch], latch_pos);
    true
}

fn collect_widenable_phis(
    func: &IrFunction,
    lp: &NaturalLoop,
    preheader_label: BlockId,
    latch: usize,
    latch_label: BlockId,
) -> Vec<PhiCandidate> {
    let header_block = &func.blocks[lp.header];
    let mut out = Vec::new();
    for inst in &header_block.instructions {
        let Instruction::Phi { dest, ty, incoming } = inst else {
            continue;
        };
        if !matches!(*ty, IrType::I32 | IrType::U32) {
            continue;
        }
        if incoming.len() != 2 {
            continue;
        }
        let mut init_op: Option<Operand> = None;
        let mut latch_op: Option<Operand> = None;
        for (op, blk) in incoming {
            if *blk == preheader_label {
                init_op = Some(*op);
            } else if *blk == latch_label {
                latch_op = Some(*op);
            } else {
                init_op = None;
                break;
            }
        }
        let (Some(init), Some(latch_val_op)) = (init_op, latch_op) else {
            continue;
        };
        let latch_val = match latch_val_op {
            Operand::Value(v) => v,
            _ => continue,
        };
        let Some(is_sub) = latch_step_uses_phi(func, latch, latch_val, *dest) else {
            continue;
        };
        out.push(PhiCandidate {
            phi_dest: *dest,
            phi_ty: *ty,
            init_op: init,
            latch_op_dest: latch_val,
            latch_is_sub: is_sub,
        });
    }
    out
}

/// Returns `Some(is_sub)` if `latch` defines `latch_val = phi op step` with
/// `op ∈ {Add, Sub}` and the phi on the arithmetic left (Add also accepts the
/// commutative order). `Sub(step, phi)` is not an induction recurrence and is
/// rejected.
fn latch_step_uses_phi(
    func: &IrFunction,
    latch: usize,
    latch_val: Value,
    phi: Value,
) -> Option<bool> {
    let latch_block = &func.blocks[latch];
    for inst in &latch_block.instructions {
        if let Instruction::BinOp {
            dest,
            op,
            lhs,
            rhs,
            ty: _,
        } = inst
        {
            if *dest != latch_val {
                continue;
            }
            let lhs_is_phi = matches!(lhs, Operand::Value(v) if *v == phi);
            let rhs_is_phi = matches!(rhs, Operand::Value(v) if *v == phi);
            match op {
                IrBinOp::Add if lhs_is_phi || rhs_is_phi => return Some(false),
                IrBinOp::Sub if lhs_is_phi => return Some(true),
                _ => return None,
            }
        }
    }
    None
}

/// Extract the step operand of the latch op (the operand that is NOT the
/// phi). The caller has already validated the shape via
/// `latch_step_uses_phi`.
fn plan_step_operand(func: &IrFunction, phi_dest: Value, latch_dest: Value) -> Option<Operand> {
    for b in &func.blocks {
        for inst in &b.instructions {
            if let Instruction::BinOp {
                dest,
                op: IrBinOp::Add | IrBinOp::Sub,
                lhs,
                rhs,
                ty: _,
            } = inst
            {
                if *dest != latch_dest {
                    continue;
                }
                let lhs_is_phi = matches!(lhs, Operand::Value(v) if *v == phi_dest);
                let rhs_is_phi = matches!(rhs, Operand::Value(v) if *v == phi_dest);
                if lhs_is_phi {
                    return Some(*rhs);
                }
                if rhs_is_phi {
                    // Only legal for Add (commutative). Sub(step, phi) is
                    // rejected upstream by latch_step_uses_phi.
                    return Some(*lhs);
                }
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Use-map
// ---------------------------------------------------------------------------

/// `value_id -> [(block_idx, instruction_idx)]`; a terminator use is recorded
/// with `instruction_idx == usize::MAX`.
type UseMap = FxHashMap<u32, Vec<(usize, usize)>>;

fn build_use_map(func: &IrFunction) -> UseMap {
    let mut map: UseMap = FxHashMap::default();
    for (bi, b) in func.blocks.iter().enumerate() {
        for (ii, inst) in b.instructions.iter().enumerate() {
            inst.for_each_used_value(|id| {
                map.entry(id).or_default().push((bi, ii));
            });
        }
        b.terminator.for_each_used_value(|id| {
            map.entry(id).or_default().push((bi, usize::MAX));
        });
    }
    map
}

// ---------------------------------------------------------------------------
// Cmp policy
// ---------------------------------------------------------------------------

/// Decide whether the predicate is safe to evaluate at wide width when the IV
/// side is widened by `ext`. See the module header theorems.
fn predicate_ok_wide(pred: IrCmpOp, ext: ExtKind) -> bool {
    match pred {
        IrCmpOp::Eq | IrCmpOp::Ne => true,
        IrCmpOp::Slt | IrCmpOp::Sle | IrCmpOp::Sgt | IrCmpOp::Sge => ext == ExtKind::Se,
        IrCmpOp::Ult | IrCmpOp::Ule | IrCmpOp::Ugt | IrCmpOp::Uge => true,
    }
}

/// Determine how the other operand of a cmp must be widened for a *widenable*
/// predicate. Returns `None` when the other operand cannot be widened
/// compatibly (the caller falls back to a narrow cmp + trunc).
///
/// The extension of the other side must match the member's so both sides live
/// in the same universe: a sext-widened IV compares against sext-widened
/// signed values; a zext-widened IV against zext-widened unsigned values.
fn other_widen_for_pred(ext: ExtKind, other: &Operand, func: &IrFunction) -> Option<CmpOther> {
    match other {
        Operand::Const(c) => Some(CmpOther::Const(*c)),
        Operand::Value(v) => {
            let ty = defining_type(func, *v)?;
            if ty.size() >= 8 {
                // Already 64-bit: no hoist needed.
                return Some(CmpOther::WideValue(*v));
            }
            let matches = match ext {
                ExtKind::Se => !is_unsigned_ty(ty),
                ExtKind::Ze => is_unsigned_ty(ty),
            };
            if !matches {
                return None;
            }
            Some(CmpOther::InvariantValue {
                value: *v,
                from_ty: ty,
            })
        }
    }
}

fn defining_type(func: &IrFunction, v: Value) -> Option<IrType> {
    for b in &func.blocks {
        for inst in &b.instructions {
            if inst.dest().map(|d| d.0) == Some(v.0) {
                return inst.result_type();
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// The provenance engine
// ---------------------------------------------------------------------------

fn analyze_iv(
    func: &IrFunction,
    lp: &NaturalLoop,
    preds: &FlatAdj,
    succs: &FlatAdj,
    uses: &UseMap,
    hoist: &PreheaderSite<'_>,
    phi_dest: Value,
    phi_ty: IrType,
    init: Operand,
    latch: usize,
    latch_dest: Value,
    latch_is_sub: bool,
) -> Option<WidenPlan> {
    let ext = ext_kind_for(phi_ty);
    let wide_ty = wide_ty_for(phi_ty);
    let step = plan_step_operand(func, phi_dest, latch_dest)?;

    // Unsigned IVs need a provable counted-loop bound (theorems 5/5′).
    let (seed_range, intercept): (Option<Range>, bool) = if ext == ExtKind::Ze {
        let proof = prove_counted_bound(
            func,
            lp,
            preds,
            succs,
            uses,
            phi_dest,
            phi_ty,
            latch,
            init,
            latch_is_sub,
            step,
        )?;
        // The exposed-latch intercept activates when the first body pass runs
        // at a seed the proof could not confine AND the proof's range cannot
        // close the exiting latch step: the narrow recurrence then wraps at
        // the ring top where the wide one does not, member values diverge
        // from `zext(narrow)`, and only value-exact uses are admissible. A
        // phi-guarded strict range, or a seed-inclusive range with
        // `hi + mag <= max` (incrementing; `lo - mag >= 0` decrementing),
        // proves no-wrap over the WHOLE trajectory — the intercept stays off
        // and the ordinary closure rules apply.
        let step_val = match step {
            Operand::Const(c) => c.to_i64().unwrap_or(0),
            _ => 0,
        };
        let delta: i128 = if latch_is_sub {
            -(step_val as i128)
        } else {
            step_val as i128
        };
        let max = type_max_i128(phi_ty);
        let wrap_closed = if delta > 0 {
            proof.range.hi + delta <= max
        } else if delta < 0 {
            proof.range.lo + delta >= 0
        } else {
            true
        };
        (Some(proof.range), proof.latch_exposed && !wrap_closed)
    } else {
        (None, false)
    };

    let mut members: Vec<Member> = Vec::new();
    members.push(Member::of(
        phi_dest,
        MemberKind::Seed,
        phi_ty,
        ext,
        seed_range,
    ));

    // The latch result is a member too: `a[i++]`-style uses read it.
    let latch_ty = latch_step_ty(func, latch_dest).unwrap_or(phi_ty);
    members.push(Member::of(
        latch_dest,
        MemberKind::LatchResult,
        latch_ty,
        ext,
        None,
    ));

    let mut cmps: Vec<CmpAction> = Vec::new();
    let mut escapes: Vec<(usize, Value)> = Vec::new();
    let mut has_addressing = false;

    // Worklist of (value, is_narrow_member, value_exact). Narrow members get
    // retyped to wide during apply; chain values are already wide (cast/copy
    // dests that will be aliased to the underlying wide value, or wide chain
    // binops) and only need their uses classified. `value_exact` records the
    // theorem-5′ invariant `wide value == zext(narrow value)` on every
    // executed pass: true for everything outside the intercept, and inside it
    // true only for members whose derivation masks divergence away (bit-exact
    // `& const`) or folds within the narrow type.
    let mut visited: FxHashSet<u32> = FxHashSet::default();
    let mut queue: Vec<(Value, bool, bool)> =
        vec![(phi_dest, true, !intercept), (latch_dest, true, !intercept)];

    while let Some((cur, narrow, cur_exact)) = queue.pop() {
        if !visited.insert(cur.0) {
            continue;
        }
        let cur_ext = ext;
        let cur_is_unsigned = cur_ext == ExtKind::Ze;

        let Some(uselist) = uses.get(&cur.0) else {
            continue; // dead value — dropped casts get collected by DCE
        };
        for &(bi, ii) in uselist {
            if ii == usize::MAX {
                // Terminator reads.
                let term = &func.blocks[bi].terminator;
                match term {
                    Terminator::CondBranch { cond, .. } => {
                        // A branch on a truth value: widening preserves
                        // non-zero-ness, so reading the wide value is exact
                        // — but only while `wide == zext(narrow)`. Under the
                        // intercept a diverged value can flip zero-ness
                        // (narrow 0 vs wide 2^32), so the branch must read
                        // the truncation: an escape repair outside the loop,
                        // a decline inside it.
                        if matches!(cond, Operand::Value(v) if v.0 == cur.0) {
                            if intercept && !cur_exact {
                                narrow_escape_or_bail(lp, bi, cur, narrow, &mut escapes)?;
                            }
                            continue;
                        }
                    }
                    Terminator::Return(Some(op)) => {
                        if matches!(op, Operand::Value(v) if v.0 == cur.0) {
                            narrow_escape_or_bail(lp, bi, cur, narrow, &mut escapes)?;
                            continue;
                        }
                    }
                    Terminator::Switch { val, .. } => {
                        if matches!(val, Operand::Value(v) if v.0 == cur.0) {
                            narrow_escape_or_bail(lp, bi, cur, narrow, &mut escapes)?;
                            continue;
                        }
                    }
                    _ => {}
                }
                continue;
            }

            let inst = &func.blocks[bi].instructions[ii];
            match inst {
                Instruction::BinOp {
                    dest,
                    op,
                    lhs,
                    rhs,
                    ty,
                } => {
                    // The latch op itself: the seed's step recurrence. The op
                    // is rewritten in apply; the latch dest's own uses are
                    // handled when the LatchResult member is processed.
                    if *dest == latch_dest {
                        continue;
                    }
                    let cur_lhs = matches!(lhs, Operand::Value(v) if v.0 == cur.0);
                    let other = if cur_lhs { rhs } else { lhs };

                    // **Integer-closure invariant.** Every widening identity
                    // in this pass (`ext(x op c) == ext(x) op ext(c)`, the
                    // signed-overflow-is-UB theorem, the bitwise theorem) is
                    // stated over *integers*. A float/decimal-typed operation
                    // reading the member would need a value CONVERSION, not a
                    // bit-level reinterpretation, and `widen_const` would turn
                    // its FP constant into a garbage integer. Note the
                    // wide-chain shortcut below keys off `ty.size() >= 8`,
                    // which `F64`/`D64` also satisfy — so this gate must come
                    // first, not after it.
                    if !ty.is_integer() {
                        narrow_escape_or_bail(lp, bi, cur, narrow, &mut escapes)?;
                        continue;
                    }

                    // Wide-type chain ops (already 64-bit): pure pass-through,
                    // nothing to widen — classify their uses. A chain op over
                    // a divergent member stays divergent (it copies the wide
                    // value through), so the exactness flag propagates.
                    if ty.size() >= 8 {
                        match other {
                            Operand::Const(_) => {
                                queue.push((*dest, false, cur_exact));
                                continue;
                            }
                            Operand::Value(ov) => {
                                let other_admitted = members.iter().any(|m| m.value.0 == ov.0);
                                if other_admitted || ov.0 == cur.0 {
                                    queue.push((*dest, false, cur_exact));
                                    continue;
                                }
                            }
                        }
                        narrow_escape_or_bail(lp, bi, cur, narrow, &mut escapes)?;
                        continue;
                    }

                    // Narrow binop on a member: the closure. Membership is
                    // restricted to instructions *inside the loop body*: a
                    // post-loop op gains nothing from widening and would only
                    // force a truncation back at its use; the correct handling
                    // is to let the IV's value escape (truncate at the use).
                    if !lp.body.contains(&bi) {
                        narrow_escape_or_bail(lp, bi, cur, narrow, &mut escapes)?;
                        continue;
                    }
                    match op {
                        IrBinOp::Add | IrBinOp::Sub | IrBinOp::Mul => {
                            if let Operand::Const(c) = other {
                                // Signed members: UB theorem, no range needed.
                                // Unsigned members: need the op provably
                                // non-wrapping over the member's range.
                                if cur_is_unsigned {
                                    let Some(r) = member_range(&members, cur) else {
                                        narrow_escape_or_bail(lp, bi, cur, narrow, &mut escapes)?;
                                        continue;
                                    };
                                    let Some(cr) = const_as_range(*c, *ty) else {
                                        narrow_escape_or_bail(lp, bi, cur, narrow, &mut escapes)?;
                                        continue;
                                    };
                                    let Some(nr) = binop_range(*op, r, cr) else {
                                        narrow_escape_or_bail(lp, bi, cur, narrow, &mut escapes)?;
                                        continue;
                                    };
                                    if !nr.within(*ty) {
                                        narrow_escape_or_bail(lp, bi, cur, narrow, &mut escapes)?;
                                        continue;
                                    }
                                    members.push(Member::of(
                                        *dest,
                                        MemberKind::BinOpConst {
                                            op: *op,
                                            operand: cur,
                                            c: *c,
                                            const_is_lhs: !cur_lhs,
                                        },
                                        *ty,
                                        ext,
                                        Some(nr),
                                    ));
                                    // A fold that stays within the narrow
                                    // type cannot carry divergence (no wrap
                                    // ⇒ wide == zext(narrow)) only if the
                                    // input itself was exact; inherit.
                                    queue.push((*dest, true, cur_exact));
                                    continue;
                                }
                                // Signed: the UB theorem makes every constant
                                // arithmetic op sound.
                                let nr =
                                    match (member_range(&members, cur), const_as_range(*c, *ty)) {
                                        (Some(r), Some(cr)) => binop_range(*op, r, cr),
                                        _ => None,
                                    };
                                members.push(Member::of(
                                    *dest,
                                    MemberKind::BinOpConst {
                                        op: *op,
                                        operand: cur,
                                        c: *c,
                                        const_is_lhs: !cur_lhs,
                                    },
                                    *ty,
                                    ext,
                                    nr,
                                ));
                                queue.push((*dest, true, cur_exact));
                                continue;
                            }
                            // Two-member arithmetic: sound for signed members
                            // (UB theorem); the other operand must be an
                            // admitted member.
                            if let Operand::Value(ov) = other {
                                let other_is_member = members.iter().any(|m| m.value.0 == ov.0);
                                if !cur_is_unsigned && other_is_member {
                                    members.push(Member::of(
                                        *dest,
                                        MemberKind::BinOpMember {
                                            op: *op,
                                            operand: *ov,
                                        },
                                        *ty,
                                        ext,
                                        None,
                                    ));
                                    queue.push((*dest, true, cur_exact));
                                    continue;
                                }
                            }
                            narrow_escape_or_bail(lp, bi, cur, narrow, &mut escapes)?;
                        }
                        IrBinOp::And | IrBinOp::Or | IrBinOp::Xor => {
                            // Bitwise identity (theorem 3): unconditional for
                            // both extensions, any constant.
                            if let Operand::Const(c) = other {
                                // `Or`/`Xor` pass the upper bits of a
                                // divergent wide value through, so under the
                                // intercept they are admissible only on an
                                // exact input. `And` MASKS the divergence
                                // away: `wide & c` depends solely on the low
                                // narrow bits for any `c < 2^w`, so its
                                // result is value-exact for EVERY input —
                                // this is the exposed latch's one admissible
                                // in-loop read (the `a[(i + 1) & m]`
                                // ring-buffer idiom).
                                if intercept && !cur_exact && *op != IrBinOp::And {
                                    narrow_escape_or_bail(lp, bi, cur, narrow, &mut escapes)?;
                                    continue;
                                }
                                // The And arm never depended on the input
                                // range: `x & c ∈ [0, c]` holds for every
                                // unsigned `x`, including the range-None
                                // LatchResult member the twin
                                // canonicalisation routes here.
                                let nr = match (
                                    op,
                                    cur_is_unsigned,
                                    member_range(&members, cur),
                                    const_as_range(*c, *ty),
                                ) {
                                    (IrBinOp::And, true, _, Some(cr)) => {
                                        // x & c ∈ [0, c] (bitwise subset).
                                        Some(Range { lo: 0, hi: cr.hi })
                                    }
                                    (IrBinOp::Or, true, Some(_r), Some(cr)) => {
                                        // x | c ≥ c, and < 2^w.
                                        Some(Range {
                                            lo: cr.lo,
                                            hi: type_max_i128(*ty),
                                        })
                                    }
                                    _ => None,
                                };
                                let member_exact = cur_exact || *op == IrBinOp::And;
                                members.push(Member::of(
                                    *dest,
                                    MemberKind::BinOpConst {
                                        op: *op,
                                        operand: cur,
                                        c: *c,
                                        const_is_lhs: !cur_lhs,
                                    },
                                    *ty,
                                    ext,
                                    nr,
                                ));
                                queue.push((*dest, true, member_exact));
                                continue;
                            }
                            narrow_escape_or_bail(lp, bi, cur, narrow, &mut escapes)?;
                        }
                        IrBinOp::Shl | IrBinOp::AShr | IrBinOp::LShr => {
                            // Under the intercept a shift of a divergent
                            // value moves the upper bits into (LShr) or past
                            // (Shl) the narrow result: neither commutes with
                            // the truncation, so only exact inputs shift.
                            if intercept && !cur_exact {
                                narrow_escape_or_bail(lp, bi, cur, narrow, &mut escapes)?;
                                continue;
                            }
                            // Narrow shift by a constant count. The count is a
                            // shift amount — it is NOT widened; only the
                            // member and the op's type are retyped.
                            //
                            //   AShr is exact under sext for every x (and only
                            //   signed sources produce it);
                            //   LShr is exact under zext for every x (and only
                            //   unsigned sources produce it);
                            //   Shl is exact for signed IVs by the
                            //   signed-overflow-is-UB theorem, and for unsigned
                            //   IVs needs a non-wrapping range proof.
                            let shift_ok = cur_lhs
                                && match op {
                                    IrBinOp::Shl => true,
                                    IrBinOp::AShr => cur_ext == ExtKind::Se,
                                    IrBinOp::LShr => cur_ext == ExtKind::Ze,
                                    _ => false,
                                };
                            let count = match rhs {
                                Operand::Const(c) => *c,
                                _ => IrConst::Zero,
                            };
                            if !shift_ok || !matches!(rhs, Operand::Const(_)) {
                                narrow_escape_or_bail(lp, bi, cur, narrow, &mut escapes)?;
                                continue;
                            }
                            let count_val = count.to_i64().unwrap_or(i64::MAX);
                            if count_val < 0 || count_val >= narrow_bit_width(*ty) as i64 {
                                // Out-of-range shift count: UB in C, but the
                                // conservatively correct choice is to decline.
                                narrow_escape_or_bail(lp, bi, cur, narrow, &mut escapes)?;
                                continue;
                            }
                            // Shl on an unsigned member needs a non-wrapping
                            // range proof over the member's proven range.
                            let nr = if *op == IrBinOp::Shl {
                                if cur_is_unsigned {
                                    let r = match member_range(&members, cur) {
                                        Some(r) => r,
                                        None => {
                                            narrow_escape_or_bail(
                                                lp,
                                                bi,
                                                cur,
                                                narrow,
                                                &mut escapes,
                                            )?;
                                            continue;
                                        }
                                    };
                                    let shifted = Range {
                                        lo: r.lo << count_val,
                                        hi: r.hi << count_val,
                                    };
                                    if !shifted.within(*ty) {
                                        narrow_escape_or_bail(lp, bi, cur, narrow, &mut escapes)?;
                                        continue;
                                    }
                                    Some(shifted)
                                } else {
                                    // Signed: UB theorem — unconditional.
                                    None
                                }
                            } else {
                                None
                            };
                            members.push(Member::of(
                                *dest,
                                MemberKind::ShiftConst {
                                    op: *op,
                                    operand: cur,
                                    count,
                                },
                                *ty,
                                ext,
                                nr,
                            ));
                            queue.push((*dest, true, cur_exact));
                        }
                        _ => {
                            // Division, remainder, BitTest: not
                            // extension-transparent.
                            narrow_escape_or_bail(lp, bi, cur, narrow, &mut escapes)?;
                        }
                    }
                }
                Instruction::Cast {
                    dest,
                    src,
                    from_ty,
                    to_ty,
                } => {
                    if !matches!(src, Operand::Value(v) if v.0 == cur.0) {
                        continue;
                    }
                    // Exposed-latch intercept: a cast of a divergent value
                    // aliases its consumers to the wide bits (a dropped
                    // widening cast) or reinterprets them (a cross cast) —
                    // either way the consumer no longer reads
                    // `zext(narrow)`, which is exactly how the rotated
                    // do-while escape miscompiled (`buf[2^32]` instead of
                    // `buf[0]`). Decline the plan; the loop stays narrow.
                    if intercept && !cur_exact {
                        return None;
                    }
                    // `is_unsigned_ty` answers `false` for every non-integer
                    // type, so a bare `same_sign` test cannot distinguish
                    // `I32 -> I64` (a widening extension) from `I32 -> F32`
                    // (a value *conversion*). Establish integer-ness first.
                    let to_is_int = to_ty.is_integer();
                    let same_sign = is_unsigned_ty(*to_ty) == is_unsigned_ty(*from_ty);
                    // Cross-sign widening cast (U32→I64 / I32→U64). Members
                    // are always 32-bit, so these two pairs are the complete
                    // set of cross-sign widening integer casts. The cast's
                    // value equals the widened member bit-for-bit on every
                    // executed iteration (see MemberKind::CrossCast), so it
                    // is retained as a same-size reinterpretation and the
                    // chain below it is enqueued for classification — the
                    // wide scaling ops and the GEP that set has_addressing
                    // live BELOW the cast, so without the enqueue the whole
                    // `unsigned i; a[i>>1]` family declines (verified on
                    // main @ dcf673d: 32-bit counter survived at -O2).
                    if matches!(
                        (*from_ty, *to_ty),
                        (IrType::U32, IrType::I64) | (IrType::I32, IrType::U64)
                    ) {
                        members.push(Member::of(
                            *dest,
                            MemberKind::CrossCast { operand: cur },
                            *to_ty,
                            ext,
                            None,
                        ));
                        queue.push((*dest, false, cur_exact));
                        continue;
                    }
                    if to_is_int && to_ty.size() >= 4 && *to_ty != IrType::Ptr && same_sign {
                        // Widening *integer* cast (i8→i32, i32→i64, ...): the
                        // dest becomes wide; drop it and alias its uses to the
                        // underlying member.
                        //
                        // `to_is_int` is load-bearing. Without it `F32`
                        // (size 4, `is_unsigned_ty` == false, hence
                        // "same sign" as `I32`) was admitted here, so
                        // `(float) i` was dropped and its consumers were
                        // re-classified as *integer* members of the closure.
                        // `11.0f * (float) i` then became
                        // `Mul(i:I64, widen_const(F32(11.0)) = I64(0))`
                        // — every such product silently evaluated to zero
                        // (gcc.c-torture `20060420-1.c`).
                        members.push(Member::of(
                            *dest,
                            MemberKind::WidenCast { operand: cur },
                            *from_ty,
                            ext,
                            None,
                        ));
                        queue.push((*dest, false, cur_exact));
                        continue;
                    }
                    // Narrowing, same-width (i32→u32 for C's `int < unsigned`
                    // promotion), or pointer cast: retained as a truncation of
                    // the wide value so consumers keep the exact narrow bits.
                    // Not enqueued: its consumers already read a narrow value.
                    members.push(Member::of(
                        *dest,
                        MemberKind::NarrowCast { operand: cur },
                        *to_ty,
                        ext,
                        None,
                    ));
                }
                Instruction::Copy { dest: _, src } => {
                    // A copy of a member is *not* admitted as a closure
                    // member. Aliasing its dest to the wide value (the
                    // previous design) can strand a narrow consumer on a wide
                    // operand once the copy's uses are rewritten, and a
                    // retype would need a widening cast per copy site. Treat
                    // the read as an ordinary use: bail inside the body (no
                    // per-iteration truncation), escape outside it.
                    if matches!(src, Operand::Value(v) if v.0 == cur.0) {
                        narrow_escape_or_bail(lp, bi, cur, narrow, &mut escapes)?;
                    }
                }
                Instruction::Cmp {
                    dest,
                    op,
                    lhs,
                    rhs,
                    ty,
                } => {
                    let lhs_is_cur = matches!(lhs, Operand::Value(v) if v.0 == cur.0);
                    let rhs_is_cur = matches!(rhs, Operand::Value(v) if v.0 == cur.0);
                    if !lhs_is_cur && !rhs_is_cur {
                        continue;
                    }
                    // Integer-closure invariant (see the `BinOp` arm): an
                    // FP-typed compare of an integer member is ill-typed IR
                    // and neither widening nor the `Trunc` repair (which
                    // would emit an int→float conversion) preserves it.
                    if !ty.is_integer() {
                        narrow_escape_or_bail(lp, bi, cur, narrow, &mut escapes)?;
                        continue;
                    }
                    let other = if lhs_is_cur { rhs } else { lhs };
                    if matches!(other, Operand::Value(v) if v.0 == cur.0) {
                        // cmp(member, member): self-comparison — decline.
                        return None;
                    }
                    // Under the intercept the cmp MUST read the truncation:
                    // a wide compare of a divergent value follows a
                    // DIFFERENT trajectory than the narrow program at the
                    // ring top (narrow `0 < n` continues where wide
                    // `2^32 < n` exits). The truncation's low bits ARE the
                    // narrow value, so the guarded trajectory — and with it
                    // the counted proof — survives the widening. (On x86-64
                    // the trunc is free: `cmpl` reads the low half of the
                    // wide register.)
                    if intercept && !cur_exact {
                        cmps.push(CmpAction::Trunc {
                            cmp_dest: *dest,
                            member: cur,
                            iv_is_lhs: lhs_is_cur,
                        });
                        continue;
                    }
                    // A loop-variant other operand cannot be widened (the
                    // cast would have to run per iteration): keep the cmp
                    // narrow with a truncation of the member.
                    if !operand_is_const_or_loop_invariant(other, lp, func, phi_dest) {
                        cmps.push(CmpAction::Trunc {
                            cmp_dest: *dest,
                            member: cur,
                            iv_is_lhs: lhs_is_cur,
                        });
                        continue;
                    }
                    if !predicate_ok_wide(*op, cur_ext) {
                        cmps.push(CmpAction::Trunc {
                            cmp_dest: *dest,
                            member: cur,
                            iv_is_lhs: lhs_is_cur,
                        });
                        continue;
                    }
                    match other_widen_for_pred(cur_ext, other, func) {
                        // A value operand is widened by a cast hoisted to the
                        // end of the preheader, which is only well-formed
                        // when the value is available there. A loop-EXIT
                        // compare may read a value defined after the loop
                        // (a post-loop reload of the bound): keep that cmp
                        // narrow instead of emitting a use before its def.
                        Some(CmpOther::InvariantValue { value, .. })
                            if !operand_hoistable_to_preheader(
                                &Operand::Value(value),
                                lp,
                                func,
                                phi_dest,
                                hoist,
                            ) =>
                        {
                            cmps.push(CmpAction::Trunc {
                                cmp_dest: *dest,
                                member: cur,
                                iv_is_lhs: lhs_is_cur,
                            });
                        }
                        Some(ow) => {
                            cmps.push(CmpAction::Widen {
                                cmp_dest: *dest,
                                member: cur,
                                other: ow,
                                iv_is_lhs: lhs_is_cur,
                                cmp_ty: *ty,
                            });
                        }
                        None => {
                            cmps.push(CmpAction::Trunc {
                                cmp_dest: *dest,
                                member: cur,
                                iv_is_lhs: lhs_is_cur,
                            });
                        }
                    }
                }
                Instruction::GetElementPtr { offset, .. } => {
                    if matches!(offset, Operand::Value(v) if v.0 == cur.0) {
                        // Exposed-latch intercept: a wide address computed
                        // from a value the narrow program wrapped is a
                        // DIFFERENT address — the IVW-RANGE-1 miscompile
                        // itself. Only value-exact members (a bit-exact
                        // `& const` chain, or a fold that stayed inside the
                        // narrow type) may address.
                        if intercept && !cur_exact {
                            return None;
                        }
                        has_addressing = true;
                        continue;
                    }
                }
                Instruction::Intrinsic { .. } => {
                    // An intrinsic index/offset slot is address arithmetic
                    // like a GEP offset: decline it under the intercept on a
                    // divergent member. Any other intrinsic read is an
                    // ordinary narrow use.
                    if intercept && !cur_exact {
                        return None;
                    }
                    narrow_escape_or_bail(lp, bi, cur, narrow, &mut escapes)?;
                }
                Instruction::Select {
                    dest,
                    cond,
                    true_val,
                    false_val,
                    ty,
                } => {
                    let cur_is_true = matches!(true_val, Operand::Value(v) if v.0 == cur.0);
                    let cur_is_false = matches!(false_val, Operand::Value(v) if v.0 == cur.0);
                    if (cur_is_true || cur_is_false) && ty.is_integer() {
                        // Under the intercept a divergent select-data value
                        // propagates the divergence into the result without
                        // a range that could confine it: treat as a narrow
                        // use (escape repair outside, decline inside).
                        if intercept && !cur_exact {
                            narrow_escape_or_bail(lp, bi, cur, narrow, &mut escapes)?;
                            continue;
                        }
                        // A member used as select *data*: admissible when the
                        // other data operand is a const or an admitted member.
                        let other_op = if cur_is_true { false_val } else { true_val };
                        let other_ok = match other_op {
                            Operand::Const(_) => true,
                            Operand::Value(ov) => members.iter().any(|m| m.value.0 == ov.0),
                        };
                        if other_ok {
                            // Select-data membership follows the same
                            // loop-body rule as arithmetic closures.
                            if !lp.body.contains(&bi) {
                                narrow_escape_or_bail(lp, bi, cur, narrow, &mut escapes)?;
                                continue;
                            }
                            let nr = match other_op {
                                Operand::Const(c) => member_range(&members, cur).and_then(|r| {
                                    const_as_range(*c, *ty).map(|cr| Range {
                                        lo: r.lo.min(cr.lo),
                                        hi: r.hi.max(cr.hi),
                                    })
                                }),
                                Operand::Value(ov) => {
                                    let r1 = member_range(&members, cur);
                                    let r2 = member_range(&members, *ov);
                                    match (r1, r2) {
                                        (Some(a), Some(b)) => Some(Range {
                                            lo: a.lo.min(b.lo),
                                            hi: a.hi.max(b.hi),
                                        }),
                                        _ => None,
                                    }
                                }
                            };
                            members.push(Member::of(
                                *dest,
                                MemberKind::Select {
                                    cond: *cond,
                                    a: cur,
                                    b: *other_op,
                                },
                                *ty,
                                ext,
                                nr,
                            ));
                            queue.push((*dest, true, cur_exact));
                            continue;
                        }
                    }
                    // Member as select condition: a truth-value use; widening
                    // preserves non-zero-ness, so no rewrite is needed —
                    // except under the intercept, where a divergent value
                    // can flip zero-ness and the select must read the
                    // truncation like any narrow consumer.
                    if matches!(cond, Operand::Value(v) if v.0 == cur.0) {
                        if intercept && !cur_exact {
                            narrow_escape_or_bail(lp, bi, cur, narrow, &mut escapes)?;
                        }
                        continue;
                    }
                    narrow_escape_or_bail(lp, bi, cur, narrow, &mut escapes)?;
                }
                Instruction::Phi { dest, .. } => {
                    // The seed phi's own incoming reads the latch result —
                    // that is the recurrence itself, not a use to rewrite.
                    if dest.0 == phi_dest.0 {
                        continue;
                    }
                    // Any other phi reading a member (exit-merge or inner
                    // induction): a phi operand is evaluated on the edge, so
                    // its truncation would run per iteration. Decline.
                    return None;
                }
                _ => {
                    // Load/Store/atomics/calls/va_*/memcpy and every other
                    // shape reading the member: a narrow use. The escape
                    // truncation is exact even for a divergent wide value —
                    // its low bits ARE the narrow value — so the intercept
                    // needs no extra gate here.
                    narrow_escape_or_bail(lp, bi, cur, narrow, &mut escapes)?;
                }
            }
        }
    }

    Some(WidenPlan {
        phi_dest,
        phi_ty,
        wide_ty,
        ext,
        init,
        latch_dest,
        latch_is_sub,
        step,
        members,
        cmps,
        escapes,
        has_addressing,
    })
}

// --- small helpers used by the engine ---

fn latch_step_ty(func: &IrFunction, latch_dest: Value) -> Option<IrType> {
    for b in &func.blocks {
        for inst in &b.instructions {
            if let Instruction::BinOp { dest, ty, .. } = inst {
                if *dest == latch_dest {
                    return Some(*ty);
                }
            }
        }
    }
    None
}

fn member_range(members: &[Member], v: Value) -> Option<Range> {
    members
        .iter()
        .find(|m| m.value.0 == v.0)
        .and_then(|m| m.range)
}

fn const_as_range(c: IrConst, ty: IrType) -> Option<Range> {
    let v = widen_const(c, ty)?.to_i64()? as i128;
    Some(Range { lo: v, hi: v })
}

/// Range arithmetic in i128 over unsigned members. `None` = not computable.
fn binop_range(op: IrBinOp, a: Range, b: Range) -> Option<Range> {
    match op {
        IrBinOp::Add => Some(Range {
            lo: a.lo + b.lo,
            hi: a.hi + b.hi,
        }),
        IrBinOp::Sub => Some(Range {
            lo: a.lo - b.hi,
            hi: a.hi - b.lo,
        }),
        IrBinOp::Mul => {
            let candidates = [a.lo * b.lo, a.lo * b.hi, a.hi * b.lo, a.hi * b.hi];
            Some(Range {
                lo: *candidates.iter().min()?,
                hi: *candidates.iter().max()?,
            })
        }
        IrBinOp::And => Some(Range {
            lo: 0,
            hi: b.hi.min(a.hi),
        }),
        IrBinOp::Or => Some(Range {
            lo: a.lo.max(b.lo),
            hi: a.hi.max(b.hi),
        }),
        _ => None,
    }
}

/// A narrow in-loop use of `cur` forces a bail unless the block is outside
/// the loop (then one truncation per escaping block repairs it).
fn narrow_escape_or_bail(
    lp: &NaturalLoop,
    bi: usize,
    cur: Value,
    narrow: bool,
    escapes: &mut Vec<(usize, Value)>,
) -> Option<()> {
    if lp.body.contains(&bi) {
        return None;
    }
    if narrow && !escapes.iter().any(|(b, v)| *b == bi && v.0 == cur.0) {
        escapes.push((bi, cur));
    }
    // Wide (chain) values escaping need no truncation: their consumers
    // already read the wide value.
    Some(())
}

/// The counted-loop proof result (theorems 5/5′): the provable value range of
/// the IV across body passes, and the trajectory class of the exit test that
/// proved it.
#[derive(Debug, Clone, Copy)]
struct CountedProof {
    /// Provable range of the phi's value at body-member evaluations.
    /// phi-guarded proofs are STRICT (post-entry: the seed is tested before
    /// any body work runs); latch-exposed proofs are SEED-INCLUSIVE (the
    /// first body pass runs at the untested seed — a const seed folds into
    /// the range, a runtime seed admits only the full domain).
    range: Range,
    /// True when the exit test does not precede every body pass: a rotated
    /// do-while (cmp in the latch block), a cmp outside the header, or a
    /// header carrying body work ahead of its own test. The range is then
    /// seed-inclusive, and when it cannot close the exiting latch step
    /// (`hi + mag` wraps the narrow type), `analyze_iv` activates the
    /// exposed-latch intercept.
    latch_exposed: bool,
}

/// Resolve the seed operand to a constant, through the single `Copy{src:
/// Const}` level mem2reg leaves for `uint32_t i = 8;` (the init arrives as a
/// copy of the constant, not a bare constant). Any other shape — a load, a
/// param, arithmetic, chained copies — is a runtime seed: `None`. The value
/// is extended by the phi's own type, so an `I32(-1)` carrier for a `U32`
/// seed resolves to `0xFFFFFFFF`.
fn resolve_init_const(func: &IrFunction, init: Operand, phi_ty: IrType) -> Option<i128> {
    let c = match init {
        Operand::Const(c) => c,
        Operand::Value(v) => {
            let (_, inst) = find_def(func, v)?;
            let Instruction::Copy { src, .. } = inst else {
                return None;
            };
            match src {
                Operand::Const(c) => *c,
                _ => return None,
            }
        }
    };
    widen_const(c, phi_ty)?.to_i64().map(|v| v as i128)
}

/// Theorem 5′ trajectory gate: every body block except the header is entered
/// only from inside the body. An outside edge into the middle of the body
/// would run body work on a pass whose IV value no header test gated,
/// invalidating every range argument (strict AND seed-inclusive).
fn body_entry_closed(lp: &NaturalLoop, preds: &FlatAdj) -> bool {
    lp.body.iter().all(|&b| {
        b == lp.header
            || preds
                .row(b)
                .iter()
                .all(|&p| lp.body.contains(&(p as usize)))
    })
}

/// Theorem 5′ trajectory gate: the latch's only in-body successor is the
/// header. A latch that re-enters the body (or itself) steps the recurrence
/// without re-passing the exit test, letting the IV outrun any proven bound
/// (an inner cycle through the latch).
fn latch_on_inner_cycle(lp: &NaturalLoop, succs: &FlatAdj, latch: usize) -> bool {
    succs
        .row(latch)
        .iter()
        .any(|&s| lp.body.contains(&(s as usize)) && s as usize != lp.header)
}

/// Theorem 5′ trajectory gate: the header carries no body work — only phis
/// and the exit test. A header computing member values ahead of its own test
/// evaluates them at the untested seed (the rotated shape), so the loop is
/// latch-exposed even when the cmp nominally sits in the header.
fn header_is_clean(func: &IrFunction, lp: &NaturalLoop) -> bool {
    func.blocks[lp.header]
        .instructions
        .iter()
        .all(|i| matches!(i, Instruction::Phi { .. } | Instruction::Cmp { .. }))
}

/// The negation of an unsigned comparison predicate, for normalising an
/// exit-on-true branch to its continue-condition. `Eq`/`Ne` have no bounding
/// negation (an equality test does not confine a counted trajectory to an
/// interval): `None`.
fn negated_unsigned_pred(op: IrCmpOp) -> Option<IrCmpOp> {
    Some(match op {
        IrCmpOp::Ult => IrCmpOp::Uge,
        IrCmpOp::Ule => IrCmpOp::Ugt,
        IrCmpOp::Ugt => IrCmpOp::Ule,
        IrCmpOp::Uge => IrCmpOp::Ult,
        _ => return None,
    })
}

/// Normalise the CONTINUE condition over the invariant other operand
/// (`phi <cont_op> other`, or the commuted `other <cont_op> phi`) to a strict
/// threshold on the phi.
///
/// Returns `Some((is_upper, threshold))`: continue ⇔ `phi < threshold`
/// (an upper bound, incrementing loops) or ⇔ `phi > threshold` (a lower
/// bound, decrementing loops). `threshold` is `None` for a runtime
/// (loop-invariant value) bound in strict form. Non-strict forms shift the
/// constant by one (`phi <= c` ⇔ `phi < c+1`); shifting a runtime value
/// would need IR materialisation, which this proof does not do, so a
/// non-strict form over a runtime bound declines (`None`).
fn normalise_bound_threshold(
    cont_op: IrCmpOp,
    phi_is_lhs: bool,
    other: Operand,
    phi_ty: IrType,
) -> Option<(bool, Option<i128>)> {
    let const_thr: Option<i128> = match other {
        Operand::Const(c) => widen_const(c, phi_ty)?.to_i64().map(|v| v as i128),
        Operand::Value(_) => None,
    };
    // A non-strict form is only normalisable over a CONST bound.
    let shifted = |k: i128| -> Option<Option<i128>> { const_thr.map(|t| t + k).map(Some) };
    Some(match (cont_op, phi_is_lhs) {
        (IrCmpOp::Ult, true) => (true, const_thr),
        (IrCmpOp::Ugt, false) => (true, const_thr),
        (IrCmpOp::Ule, true) => (true, shifted(1)?),
        (IrCmpOp::Uge, false) => (true, shifted(1)?),
        (IrCmpOp::Ugt, true) => (false, const_thr),
        (IrCmpOp::Ult, false) => (false, const_thr),
        (IrCmpOp::Uge, true) => (false, shifted(-1)?),
        (IrCmpOp::Ule, false) => (false, shifted(-1)?),
        _ => return None,
    })
}

/// The no-wrap room obligation on a normalised CONST exit threshold: the
/// incrementing latch result reaches `thr - 1 + mag` (body values pass the
/// test at `≤ thr-1`) and the decrementing one reaches `thr + 1 - mag`;
/// neither may leave the narrow domain, else the narrow recurrence wraps
/// where the wide one does not. One interval test covers both directions
/// because `thr` is the exit threshold either way. The endpoints cannot
/// overflow: `mag >= 1`, `max = 2^w - 1` with `w <= 32`, all in i128.
fn bound_leaves_room(thr: i128, max: i128, mag: i128) -> bool {
    thr >= mag - 1 && thr <= max - mag + 1
}

/// Theorems 5/5′ for unsigned IVs: prove the loop is a counted loop whose
/// exit test confines the IV to a no-wrap range under a constant step, and
/// classify the trajectory (phi-guarded vs latch-exposed).
///
/// The per-iteration delta is `-step` for a `Sub` latch and `+step` for an
/// `Add` latch.  Writing `mag = |delta|` and `max = 2^w - 1`:
///
/// * The exit cmp must read the phi directly, feed its block's conditional
///   branch, and have exactly ONE arm leaving the loop body — resolved
///   through a label→index map (obligation P): branch targets are `BlockId`
///   LABELS while `lp.body`, `preds` and `succs` are block INDICES, and the
///   two numbering domains diverge after any CFG simplification (a live repro
///   had body index 3 under label 5; comparing the domains directly failed
///   every well-formed loop in real frontend IR). Both branch polarities are
///   accepted: an exit-on-true arm normalises to the negated
///   continue-condition before the bound is read.
/// * Signed predicates (`Slt`/`Sle`/`Sgt`/`Sge`) on an unsigned phi decline
///   the WHOLE proof: `0xFFFFFFFF <s 64` is true, so a signed test confines
///   an unsigned value to no interval at all. (An earlier revision admitted
///   `Slt` here and a unit fixture codified the unsound strict range that
///   followed — a ring-top seed satisfies `phi <s 64` and runs the body
///   outside `[0, 63]`.)
/// * `mag > 1` needs a CONSTANT threshold with room for the final stride
///   (`bound_leaves_room`).  `mag == 1` also accepts a RUNTIME loop-invariant
///   threshold — the no-wrap argument is by construction (every continuing
///   value `< thr ≤ max`, unit steps, latch `≤ max`), not by constant
///   folding.  This is the same discipline SCEV-based compilers apply.
/// * The structural trajectory gates (body-entry closure, no inner cycle
///   through the latch) are unconditional; header cleanliness decides the
///   RANGE CLASS: a phi-guarded proof (`cmp_is_header && !cmp_is_latch &&
///   header_clean`) returns the STRICT post-entry range; a latch-exposed
///   proof (`cmp_is_latch || !cmp_is_header || !header_clean`) returns the
///   SEED-INCLUSIVE range — a const seed (resolved through the single
///   `Copy{src: Const}` level mem2reg leaves) folds into it, a runtime seed
///   admits only the full domain `[0, max]`, whose first arithmetic fold
///   overflows and whose divergent members the exposed-latch intercept in
///   `analyze_iv` confines to value-exact uses.
///
/// Refusing shapes whose latch step can wrap is not merely a missed
/// optimisation.  A narrow index forces the backend to refuse the SIB scale
/// and displacement folds (`scale_ring_is_linear`,
/// `backend/generation.rs`): `add(iv, k)` evaluated in the u32 ring is a
/// different address from `add(iv, k)` in the pointer ring whenever
/// `iv + k` wraps, so the fold is only sound on a pointer-width index.
/// Proving the stride here widens the IV, after which the fold is both legal
/// and free -- the counted loop keeps its `k(%base,%iv)` addressing instead
/// of paying a narrow `leal` per unrolled access.
fn prove_counted_bound(
    func: &IrFunction,
    lp: &NaturalLoop,
    preds: &FlatAdj,
    succs: &FlatAdj,
    uses: &UseMap,
    phi_dest: Value,
    phi_ty: IrType,
    latch: usize,
    init: Operand,
    latch_is_sub: bool,
    step: Operand,
) -> Option<CountedProof> {
    // Constant step only, in either latch spelling (`Add(i, s)` / `Sub(i, s)`),
    // so the true delta carries the latch's sign rather than assuming `s == 1`.
    let step_val = match step {
        Operand::Const(c) => c.to_i64()?,
        _ => return None,
    };
    let delta: i128 = if latch_is_sub {
        -(step_val as i128)
    } else {
        step_val as i128
    };
    if delta == 0 {
        return None;
    }
    let mag: i128 = delta.abs();
    let w = narrow_bit_width(defining_type(func, phi_dest)?);
    let max = (1i128 << w) - 1;

    // Theorem 5′ structural trajectory gates.
    if !body_entry_closed(lp, preds) {
        return None;
    }
    if latch_on_inner_cycle(lp, succs, latch) {
        return None;
    }
    let header_clean = header_is_clean(func, lp);

    // Obligation P: branch targets are BlockId LABELS; the loop body and the
    // adjacency rows are block INDICES. Resolve through this map — never
    // compare the two domains directly.
    let mut label_to_idx: FxHashMap<BlockId, usize> = FxHashMap::default();
    for (i, b) in func.blocks.iter().enumerate() {
        label_to_idx.insert(b.label, i);
    }

    let uselist = uses.get(&phi_dest.0)?;
    for &(bi, ii) in uselist {
        if ii == usize::MAX {
            continue;
        }
        let Instruction::Cmp {
            dest,
            op,
            lhs,
            rhs,
            ty: _,
        } = &func.blocks[bi].instructions[ii]
        else {
            continue;
        };
        // The cmp result must feed its block's conditional branch.
        let Terminator::CondBranch {
            cond,
            true_label,
            false_label,
        } = &func.blocks[bi].terminator
        else {
            continue;
        };
        if !matches!(cond, Operand::Value(v) if v.0 == dest.0) {
            continue;
        }
        let lhs_is_phi = matches!(lhs, Operand::Value(v) if v.0 == phi_dest.0);
        let rhs_is_phi = matches!(rhs, Operand::Value(v) if v.0 == phi_dest.0);
        if !lhs_is_phi && !rhs_is_phi {
            continue;
        }
        // Obligation P (label resolution): exactly one arm leaves the body —
        // that arm is the exit edge and the other is the continue edge. Arms
        // that both stay (an internal routing test) or both leave (ill-formed
        // for a natural loop) prove no exit trajectory.
        let true_idx = *label_to_idx.get(true_label)?;
        let false_idx = *label_to_idx.get(false_label)?;
        let true_in = lp.body.contains(&true_idx);
        let false_in = lp.body.contains(&false_idx);
        if true_in == false_in {
            continue;
        }
        let exits_on_true = !true_in;
        let other = if lhs_is_phi { rhs } else { lhs };
        if !operand_is_const_or_loop_invariant(other, lp, func, phi_dest) {
            continue;
        }
        // A signed predicate over an unsigned phi confines it to no interval
        // (0xFFFFFFFF <s 64 is true): decline the whole proof.
        if matches!(
            op,
            IrCmpOp::Slt | IrCmpOp::Sle | IrCmpOp::Sgt | IrCmpOp::Sge
        ) {
            return None;
        }
        // Normalise to the CONTINUE condition (an exit-on-true arm negates).
        let cont_op = if exits_on_true {
            negated_unsigned_pred(*op)?
        } else {
            *op
        };
        let Some((is_upper, thr)) = normalise_bound_threshold(cont_op, lhs_is_phi, *other, phi_ty)
        else {
            return None;
        };
        // The bound direction must match the step sign.
        if is_upper != (delta > 0) {
            return None;
        }
        // The no-wrap room obligation. A runtime threshold is provable only
        // for a unit step, by construction (every continuing value ≤ max-1
        // under a strict `phi < thr ≤ max` test, latch ≤ max).
        match thr {
            Some(t) => {
                if !bound_leaves_room(t, max, mag) {
                    return None;
                }
            }
            None => {
                if mag > 1 {
                    return None;
                }
            }
        }

        // Trajectory class: the guard confines body work only when it
        // precedes ALL of it — the cmp in a clean header, not the latch.
        let cmp_is_header = bi == lp.header;
        let cmp_is_latch = bi == latch;
        let latch_exposed = cmp_is_latch || !cmp_is_header || !header_clean;

        // Strict (phi-guarded) vs seed-inclusive (latch-exposed) range. The
        // seed-inclusive upper bound is `max(seed, thr-1+mag)`: the untested
        // seed itself AND the tested overshoot (a pass continues at `phi <
        // thr`, so the NEXT body pass runs at up to `thr-1+mag` before its
        // own test). A runtime seed or runtime threshold admits only the
        // full domain — tight enough that the first arithmetic fold overflows
        // and the intercept confines what remains.
        let init_const = if latch_exposed {
            resolve_init_const(func, init, phi_ty)
        } else {
            None
        };
        let range = match (is_upper, latch_exposed) {
            (true, false) => Range {
                lo: 0,
                hi: thr.map(|t| (t - 1).min(max)).unwrap_or(max - 1),
            },
            (true, true) => match (init_const, thr) {
                (Some(c), Some(t)) => Range {
                    lo: 0,
                    hi: c.max(t - 1 + mag).min(max),
                },
                _ => Range { lo: 0, hi: max },
            },
            (false, false) => Range {
                lo: thr.map(|t| (t + 1).max(0)).unwrap_or(1),
                hi: max,
            },
            (false, true) => match (init_const, thr) {
                (Some(c), Some(t)) => Range {
                    lo: c.min(t + 1 - mag).max(0),
                    hi: max,
                },
                _ => Range { lo: 0, hi: max },
            },
        };
        return Some(CountedProof {
            range,
            latch_exposed,
        });
    }
    None
}

// ---------------------------------------------------------------------------
// Pre-mutation verification (provenance asserted)
// ---------------------------------------------------------------------------

/// Re-check every recorded member, cmp and escape against the live IR before
/// any mutation. Analysis and apply run back-to-back so this always passes;
/// it exists to make the provenance explicit and to fail atomically if the IR
/// ever drifts from what the analysis saw.
/// Integer-closure invariant over a whole plan.
///
/// Every widening identity used by this pass — `ext(x op c) == ext(x) op
/// ext(c)`, the signed-overflow-is-UB theorem, the bitwise theorem, and the
/// order-preservation of `sext`/`zext` — is a statement about *integers*.
/// A member that is not integer-typed (or a constant that is not an integer
/// constant) means the classifier admitted a value conversion as if it were
/// a bit-level extension, and `widen_const` would fabricate a garbage
/// integer for it. Reject the entire plan; the loop simply stays narrow.
///
/// Regression anchor: `Cast { I32 -> F32 }` used to pass the `WidenCast`
/// test (`F32.size() == 4`, and `is_unsigned_ty` is `false` for both sides,
/// so the "same sign" test succeeded), after which `11.0f * (float) i` was
/// retyped to `Mul(i:I64, I64(0))` — a silent hard zero.
fn plan_is_integral(func: &IrFunction, plan: &WidenPlan) -> bool {
    let integral_const = |c: &IrConst| {
        matches!(
            c,
            IrConst::I8(_) | IrConst::I16(_) | IrConst::I32(_) | IrConst::I64(_) | IrConst::Zero
        )
    };
    if !plan.phi_ty.is_integer() || !plan.wide_ty.is_integer() {
        return false;
    }
    if let Operand::Const(c) = &plan.init {
        if !integral_const(c) {
            return false;
        }
    }
    if let Operand::Const(c) = &plan.step {
        if !integral_const(c) {
            return false;
        }
    }
    for m in &plan.members {
        // A `NarrowCast` member is deliberately RETAINED as a conversion of
        // the wide value (`Cast { from_ty: wide_ty, to_ty: m.orig_ty }`), so
        // its `orig_ty` is the conversion *target* and may legitimately be
        // non-integer (`(float) i` lands here). Every other member kind
        // names a value that must physically hold the widened integer.
        if matches!(m.kind, MemberKind::NarrowCast { .. }) {
            continue;
        }
        if !m.orig_ty.is_integer() {
            return false;
        }
        match &m.kind {
            MemberKind::BinOpConst { c, .. } if !integral_const(c) => return false,
            MemberKind::Select { b, .. } => {
                if let Operand::Const(c) = b {
                    if !integral_const(c) {
                        return false;
                    }
                }
            }
            _ => {}
        }
        // The live IR must agree: the defining instruction's operating type
        // is what the retype will overwrite with `wide_ty`.
        if let Some((_, inst)) = find_def(func, m.value) {
            let op_ty = match inst {
                Instruction::BinOp { ty, .. } | Instruction::Select { ty, .. } => Some(*ty),
                _ => None,
            };
            if let Some(t) = op_ty {
                if !t.is_integer() {
                    return false;
                }
            }
        }
    }
    for c in &plan.cmps {
        if let CmpAction::Widen { cmp_ty, other, .. } = c {
            if !cmp_ty.is_integer() {
                return false;
            }
            if let CmpOther::Const(k) = other {
                if !integral_const(k) {
                    return false;
                }
            }
        }
    }
    true
}

fn verify_plan(
    func: &IrFunction,
    plan: &WidenPlan,
    lp: &NaturalLoop,
    hoist: &PreheaderSite<'_>,
) -> bool {
    // Integer-closure invariant, re-checked on the *plan* right before any
    // mutation: every widening identity this pass relies on is stated over
    // integers, and `widen_const` cannot represent an FP constant. A single
    // float-typed member is therefore a hard reject, not a partial apply.
    if !plan_is_integral(func, plan) {
        return false;
    }
    // Latch shape: the step operand must still be what we recorded.
    let Some(step) = plan_step_operand(func, plan.phi_dest, plan.latch_dest) else {
        return false;
    };
    let same_step = match (&plan.step, &step) {
        (Operand::Const(a), Operand::Const(b)) => a == b,
        (Operand::Value(a), Operand::Value(b)) => a.0 == b.0,
        _ => false,
    };
    if !same_step {
        return false;
    }

    for m in &plan.members {
        let Some((_, inst)) = find_def(func, m.value) else {
            return false;
        };
        match &m.kind {
            MemberKind::Seed => {
                if !matches!(inst, Instruction::Phi { .. }) {
                    return false;
                }
            }
            MemberKind::LatchResult => {
                if !matches!(
                    inst,
                    Instruction::BinOp {
                        op: IrBinOp::Add | IrBinOp::Sub,
                        ..
                    }
                ) {
                    return false;
                }
            }
            MemberKind::BinOpConst {
                op,
                operand,
                c,
                const_is_lhs,
            } => {
                let Instruction::BinOp {
                    op: aop, lhs, rhs, ..
                } = inst
                else {
                    return false;
                };
                if aop != op {
                    return false;
                }
                let (mop, cop) = if *const_is_lhs {
                    (rhs, lhs)
                } else {
                    (lhs, rhs)
                };
                if !matches!(mop, Operand::Value(v) if v.0 == operand.0) {
                    return false;
                }
                if !matches!(cop, Operand::Const(cc) if cc == c) {
                    return false;
                }
            }
            MemberKind::BinOpMember { op, operand } => {
                let Instruction::BinOp {
                    op: aop, lhs, rhs, ..
                } = inst
                else {
                    return false;
                };
                if aop != op {
                    return false;
                }
                if !matches!(lhs, Operand::Value(v) if v.0 == operand.0)
                    && !matches!(rhs, Operand::Value(v) if v.0 == operand.0)
                {
                    return false;
                }
            }
            MemberKind::ShiftConst { op, operand, count } => {
                let Instruction::BinOp {
                    op: aop, lhs, rhs, ..
                } = inst
                else {
                    return false;
                };
                if aop != op {
                    return false;
                }
                if !matches!(lhs, Operand::Value(v) if v.0 == operand.0) {
                    return false;
                }
                if !matches!(rhs, Operand::Const(c) if c == count) {
                    return false;
                }
            }
            MemberKind::WidenCast { operand }
            | MemberKind::NarrowCast { operand }
            | MemberKind::CrossCast { operand } => {
                let Instruction::Cast { src, .. } = inst else {
                    return false;
                };
                if !matches!(src, Operand::Value(v) if v.0 == operand.0) {
                    return false;
                }
            }
            MemberKind::Copy { operand } => {
                let Instruction::Copy { src, .. } = inst else {
                    return false;
                };
                if !matches!(src, Operand::Value(v) if v.0 == operand.0) {
                    return false;
                }
            }
            MemberKind::Select { cond, a, b } => {
                let Instruction::Select {
                    cond: acond,
                    true_val,
                    false_val,
                    ..
                } = inst
                else {
                    return false;
                };
                if acond != cond {
                    return false;
                }
                if !matches!(true_val, Operand::Value(v) if v.0 == a.0)
                    && !matches!(false_val, Operand::Value(v) if v.0 == a.0)
                {
                    return false;
                }
                if b != true_val && b != false_val {
                    return false;
                }
            }
        }
    }

    // Every value apply_widen casts at the end of the preheader (a widened
    // cmp's invariant operand; the step is checked by the caller) must be
    // available there, or the hoisted cast is a use before its def.
    for c in &plan.cmps {
        if let CmpAction::Widen {
            other: CmpOther::InvariantValue { value, .. },
            ..
        } = c
        {
            if !operand_hoistable_to_preheader(
                &Operand::Value(*value),
                lp,
                func,
                plan.phi_dest,
                hoist,
            ) {
                return false;
            }
        }
    }

    // Cmp targets must still read their recorded member.
    for c in &plan.cmps {
        let (cmp_dest, member) = match c {
            CmpAction::Widen {
                cmp_dest, member, ..
            }
            | CmpAction::Trunc {
                cmp_dest, member, ..
            } => (*cmp_dest, *member),
        };
        let Some((_, inst)) = find_def(func, cmp_dest) else {
            return false;
        };
        let Instruction::Cmp { lhs, rhs, .. } = inst else {
            return false;
        };
        let reads = matches!(lhs, Operand::Value(v) if v.0 == member.0)
            || matches!(rhs, Operand::Value(v) if v.0 == member.0);
        if !reads {
            return false;
        }
    }
    true
}

fn find_def<'a>(func: &'a IrFunction, v: Value) -> Option<(usize, &'a Instruction)> {
    for (bi, b) in func.blocks.iter().enumerate() {
        for inst in &b.instructions {
            if inst.dest().map(|d| d.0) == Some(v.0) {
                return Some((bi, inst));
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Apply (all mutations happen here, in a fixed safe order)
// ---------------------------------------------------------------------------

fn apply_widen(func: &mut IrFunction, plan: &WidenPlan, preheader_idx: usize, debug: bool) -> bool {
    let wide_ty = plan.wide_ty;
    let phi_ty = plan.phi_ty;
    let phi_dest = plan.phi_dest;
    let mut next_id = func.next_value_id;
    let mut fresh = || {
        let v = Value(next_id);
        next_id += 1;
        v
    };

    // 1. Preheader: wide init.
    let wide_init: Operand = match plan.init {
        Operand::Const(c) => Operand::Const(widen_const(c, phi_ty).unwrap_or(c)),
        Operand::Value(v) => {
            let d = fresh();
            push_inst(
                &mut func.blocks[preheader_idx],
                Instruction::Cast {
                    dest: d,
                    src: Operand::Value(v),
                    from_ty: phi_ty,
                    to_ty: wide_ty,
                },
            );
            Operand::Value(d)
        }
    };

    // 2. Preheader: wide step (invariant value only — consts fold inline).
    let wide_step: Operand = match plan.step {
        Operand::Const(c) => Operand::Const(widen_const(c, phi_ty).unwrap_or(c)),
        Operand::Value(v) => {
            let d = fresh();
            push_inst(
                &mut func.blocks[preheader_idx],
                Instruction::Cast {
                    dest: d,
                    src: Operand::Value(v),
                    from_ty: phi_ty,
                    to_ty: wide_ty,
                },
            );
            Operand::Value(d)
        }
    };

    // 3. Seed phi: retype + preheader incoming (matched by edge label so a
    //    coincidentally-equal init operand can never corrupt the latch side).
    rewrite_seed_phi(func, plan, wide_init, preheader_idx);

    // 4. Latch op: retype + step rewrite (both operand orders; the previous
    //    revision unconditionally overwrote `rhs`, corrupting `Add(step, phi)`).
    rewrite_latch(func, plan, wide_step);

    // 5. Retype closure members in topological order (members are appended
    //    in dependency order by the engine).
    for m in &plan.members {
        match &m.kind {
            MemberKind::Seed
            | MemberKind::LatchResult
            | MemberKind::WidenCast { .. }
            | MemberKind::Copy { .. } => {
                // Seed/latch handled above; casts/copies are dropped later.
            }
            MemberKind::NarrowCast { operand } | MemberKind::CrossCast { operand } => {
                // Retain the cast as a conversion of the wide value: a
                // truncation for NarrowCast, a same-size reinterpretation for
                // CrossCast (U64→I64 / I64→U64 emit as no-ops).
                if let Some((bi, ii)) = find_def_mut(func, m.value) {
                    if let Instruction::Cast { src, from_ty, .. } =
                        &mut func.blocks[bi].instructions[ii]
                    {
                        *src = Operand::Value(*operand);
                        *from_ty = wide_ty;
                    }
                }
            }
            MemberKind::BinOpConst {
                op: _,
                operand: _,
                c,
                const_is_lhs,
            } => {
                if let Some((bi, ii)) = find_def_mut(func, m.value) {
                    if let Instruction::BinOp { ty, lhs, rhs, .. } =
                        &mut func.blocks[bi].instructions[ii]
                    {
                        *ty = wide_ty;
                        let cop = if *const_is_lhs { lhs } else { rhs };
                        *cop = Operand::Const(widen_const(*c, m.orig_ty).unwrap_or(*c));
                    }
                }
            }
            MemberKind::BinOpMember { .. } => {
                if let Some((bi, ii)) = find_def_mut(func, m.value) {
                    if let Instruction::BinOp { ty, .. } = &mut func.blocks[bi].instructions[ii] {
                        *ty = wide_ty;
                    }
                }
            }
            MemberKind::ShiftConst { .. } => {
                // Retype the shift to wide; the count is a shift amount and is
                // deliberately left narrow (never widened).
                if let Some((bi, ii)) = find_def_mut(func, m.value) {
                    if let Instruction::BinOp { ty, .. } = &mut func.blocks[bi].instructions[ii] {
                        *ty = wide_ty;
                    }
                }
            }
            MemberKind::Select { .. } => {
                if let Some((bi, ii)) = find_def_mut(func, m.value) {
                    if let Instruction::Select {
                        ty,
                        false_val,
                        true_val,
                        ..
                    } = &mut func.blocks[bi].instructions[ii]
                    {
                        *ty = wide_ty;
                        if let Operand::Const(c) = true_val {
                            *true_val = Operand::Const(widen_const(*c, m.orig_ty).unwrap_or(*c));
                        }
                        if let Operand::Const(c) = false_val {
                            *false_val = Operand::Const(widen_const(*c, m.orig_ty).unwrap_or(*c));
                        }
                    }
                }
            }
        }
    }

    // 6. Drop widening casts and copies: redirect their uses to the
    //    underlying member, then remove if dead (liveness re-checked at the
    //    point of removal so an imprecise analysis can never orphan a value).
    for m in &plan.members {
        if let MemberKind::WidenCast { operand } | MemberKind::Copy { operand } = &m.kind {
            let Some((bi, ii)) = find_def_idx(func, m.value) else {
                continue;
            };
            replace_all_uses_of_value(func, m.value, Operand::Value(*operand));
            let still_used = value_still_used(func, m.value);
            let block = &mut func.blocks[bi];
            if ii < block.instructions.len() {
                if still_used {
                    // Keep a well-typed identity so the IR stays consistent;
                    // DCE will collect it if it becomes dead.
                    if let Instruction::Cast {
                        src,
                        from_ty,
                        to_ty,
                        ..
                    } = &mut block.instructions[ii]
                    {
                        *src = Operand::Value(*operand);
                        *from_ty = wide_ty;
                        *to_ty = wide_ty;
                    }
                    if debug {
                        eprintln!(
                            "[IV-WIDEN] retained cast v{} as wide identity (still used)",
                            m.value.0
                        );
                    }
                } else if matches!(block.instructions[ii], Instruction::Cast { .. })
                    || matches!(block.instructions[ii], Instruction::Copy { .. })
                {
                    remove_inst(block, ii);
                }
            }
        }
    }

    // 7. Cmp rewrites (located by cmp dest — immune to index shifts from the
    //    preheader inserts and the drops above). The member side of a widened
    //    cmp already reads the correct wide value and is left untouched; a
    //    truncated cmp materialises the narrow value of whatever the cmp
    //    currently reads.
    for action in &plan.cmps {
        match action {
            CmpAction::Widen {
                cmp_dest,
                member: _,
                other,
                iv_is_lhs,
                cmp_ty,
            } => {
                let wide_other = match other {
                    CmpOther::Const(c) => Operand::Const(widen_const(*c, *cmp_ty).unwrap_or(*c)),
                    CmpOther::InvariantValue { value, from_ty } => {
                        match find_hoisted_cast(func, preheader_idx, *value, *from_ty, wide_ty) {
                            Some(d) => Operand::Value(d),
                            None => {
                                let d = fresh();
                                push_inst(
                                    &mut func.blocks[preheader_idx],
                                    Instruction::Cast {
                                        dest: d,
                                        src: Operand::Value(*value),
                                        from_ty: *from_ty,
                                        to_ty: wide_ty,
                                    },
                                );
                                Operand::Value(d)
                            }
                        }
                    }
                    CmpOther::WideValue(v) => Operand::Value(*v),
                };
                if let Some((bi, ii)) = find_def_mut(func, *cmp_dest) {
                    if let Instruction::Cmp { ty, lhs, rhs, .. } =
                        &mut func.blocks[bi].instructions[ii]
                    {
                        *ty = wide_ty;
                        if *iv_is_lhs {
                            *rhs = wide_other;
                        } else {
                            *lhs = wide_other;
                        }
                    }
                }
            }
            CmpAction::Trunc {
                cmp_dest,
                member: _,
                iv_is_lhs,
            } => {
                if let Some((bi, ii)) = find_def_idx(func, *cmp_dest) {
                    // Take the operand the cmp CURRENTLY reads on the member
                    // side (it may have been aliased by a cast/copy drop).
                    let (src, other_side, cmp_ty) = {
                        let inst = &func.blocks[bi].instructions[ii];
                        let Instruction::Cmp { lhs, rhs, ty, .. } = inst else {
                            continue;
                        };
                        if *iv_is_lhs {
                            (*lhs, *rhs, *ty)
                        } else {
                            (*rhs, *lhs, *ty)
                        }
                    };
                    let narrow = fresh();
                    let at = ii.min(func.blocks[bi].instructions.len());
                    insert_inst(
                        &mut func.blocks[bi],
                        at,
                        Instruction::Cast {
                            dest: narrow,
                            src,
                            from_ty: wide_ty,
                            to_ty: cmp_ty,
                        },
                    );
                    if let Instruction::Cmp { lhs, rhs, .. } =
                        &mut func.blocks[bi].instructions[at + 1]
                    {
                        if *iv_is_lhs {
                            *lhs = Operand::Value(narrow);
                            *rhs = other_side;
                        } else {
                            *rhs = Operand::Value(narrow);
                            *lhs = other_side;
                        }
                    }
                }
            }
        }
    }

    // 8. Escape truncations — one per escaping block, after the phi prefix.
    for &(bi, member) in &plan.escapes {
        if bi >= func.blocks.len() {
            continue;
        }
        let narrow_ty = member_orig_ty(func, plan, member).unwrap_or(phi_ty);
        let narrow = fresh();
        let insert_at = func.blocks[bi]
            .instructions
            .iter()
            .position(|i| !matches!(i, Instruction::Phi { .. }))
            .unwrap_or(func.blocks[bi].instructions.len());

        // Rewrite the block's own NARROW reads of the member so the new trunc
        // is not self-rewritten. Slots that are type-checked wide (GEP
        // offsets, wide arithmetic, cmp operands — already handled by the
        // CmpAction — cast sources, intrinsic index args) keep reading the
        // wide value: rewriting them would put a 32-bit truncation into a
        // 64-bit slot.
        for inst in func.blocks[bi].instructions.iter_mut() {
            if escape_read_needs_narrow(inst, member, wide_ty) {
                replace_operand_value(inst, member, Operand::Value(narrow));
            }
        }
        replace_terminator_value(
            &mut func.blocks[bi].terminator,
            member,
            Operand::Value(narrow),
        );

        insert_inst(
            &mut func.blocks[bi],
            insert_at,
            Instruction::Cast {
                dest: narrow,
                src: Operand::Value(member),
                from_ty: wide_ty,
                to_ty: narrow_ty,
            },
        );
        if debug {
            eprintln!(
                "[IV-WIDEN] escaping use of v{} in block {} truncated via v{}",
                member.0, bi, narrow.0
            );
        }
    }

    func.next_value_id = next_id;
    if debug {
        eprintln!(
            "[IV-WIDEN] widened phi {:?} (ty {:?} → {:?})",
            phi_dest, phi_ty, wide_ty
        );
    }
    true
}

fn member_orig_ty(func: &IrFunction, plan: &WidenPlan, v: Value) -> Option<IrType> {
    plan.members
        .iter()
        .find(|m| m.value.0 == v.0)
        .map(|m| m.orig_ty)
        .or_else(|| defining_type(func, v))
}

/// Hoist a widening cast of a loop-invariant cmp operand into the preheader.
/// Look up an already-hoisted wide cast of `value` in the preheader.
/// Returns the existing destination if a cast with the same (src, from, to)
/// triple is present, so several cmps sharing one operand reuse one sext/zext.
fn find_hoisted_cast(
    func: &IrFunction,
    preheader_idx: usize,
    value: Value,
    from_ty: IrType,
    to_ty: IrType,
) -> Option<Value> {
    for inst in &func.blocks[preheader_idx].instructions {
        if let Instruction::Cast {
            dest,
            src,
            from_ty: ft,
            to_ty: tt,
        } = inst
        {
            if matches!(src, Operand::Value(v) if v.0 == value.0) && *ft == from_ty && *tt == to_ty
            {
                return Some(*dest);
            }
        }
    }
    None
}

fn rewrite_seed_phi(
    func: &mut IrFunction,
    plan: &WidenPlan,
    wide_init: Operand,
    preheader_idx: usize,
) {
    let preheader_label = func.blocks[preheader_idx].label;
    for b in &mut func.blocks {
        for inst in &mut b.instructions {
            if let Instruction::Phi { dest, ty, incoming } = inst {
                if *dest == plan.phi_dest {
                    *ty = plan.wide_ty;
                    for (op, blk) in incoming.iter_mut() {
                        if *blk == preheader_label {
                            *op = wide_init;
                        }
                    }
                    return;
                }
            }
        }
    }
}

/// Rewrite the latch `Add`/`Sub` to the wide type, replacing ONLY the step
/// operand. Handles `Add(phi, step)`, `Add(step, phi)` and `Sub(phi, step)`;
/// the reversed `Sub(step, phi)` was rejected at candidacy.
fn rewrite_latch(func: &mut IrFunction, plan: &WidenPlan, wide_step: Operand) {
    for b in &mut func.blocks {
        for inst in &mut b.instructions {
            if let Instruction::BinOp {
                dest,
                op,
                lhs,
                rhs,
                ty,
            } = inst
            {
                if *dest != plan.latch_dest {
                    continue;
                }
                if !matches!(*op, IrBinOp::Add | IrBinOp::Sub) {
                    continue;
                }
                *ty = plan.wide_ty;
                let lhs_is_phi = matches!(lhs, Operand::Value(v) if v.0 == plan.phi_dest.0);
                let rhs_is_phi = matches!(rhs, Operand::Value(v) if v.0 == plan.phi_dest.0);
                if lhs_is_phi {
                    *rhs = wide_step;
                } else if rhs_is_phi && !plan.latch_is_sub {
                    // Canonicalise to Add(phi, step) for cleaner scheduling.
                    *lhs = Operand::Value(plan.phi_dest);
                    *rhs = wide_step;
                } else if rhs_is_phi && plan.latch_is_sub {
                    // Defensive: rejected upstream, never reachable.
                    *lhs = wide_step;
                }
                return;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Block mutation helpers (source_spans kept parallel to instructions)
// ---------------------------------------------------------------------------

fn push_inst(block: &mut BasicBlock, inst: Instruction) {
    let fill = block.source_spans.last().copied();
    block.instructions.push(inst);
    if let Some(sp) = fill {
        block.source_spans.push(sp);
    }
}

fn insert_inst(block: &mut BasicBlock, at: usize, inst: Instruction) {
    let at = at.min(block.instructions.len());
    block.instructions.insert(at, inst);
    if block.source_spans.is_empty() {
        return;
    }
    let fill = block
        .source_spans
        .get(at.saturating_sub(1))
        .or_else(|| block.source_spans.last())
        .copied();
    if let Some(sp) = fill {
        block
            .source_spans
            .insert(at.min(block.source_spans.len()), sp);
    }
}

fn remove_inst(block: &mut BasicBlock, ii: usize) {
    block.instructions.remove(ii);
    if ii < block.source_spans.len() {
        block.source_spans.remove(ii);
    }
}

fn find_def_idx(func: &IrFunction, v: Value) -> Option<(usize, usize)> {
    for (bi, b) in func.blocks.iter().enumerate() {
        for (ii, inst) in b.instructions.iter().enumerate() {
            if inst.dest().map(|d| d.0) == Some(v.0) {
                return Some((bi, ii));
            }
        }
    }
    None
}

fn find_def_mut(func: &mut IrFunction, v: Value) -> Option<(usize, usize)> {
    for (bi, b) in func.blocks.iter_mut().enumerate() {
        for (ii, inst) in b.instructions.iter_mut().enumerate() {
            if inst.dest().map(|d| d.0) == Some(v.0) {
                return Some((bi, ii));
            }
        }
    }
    None
}

fn value_still_used(func: &IrFunction, val: Value) -> bool {
    for b in &func.blocks {
        for inst in &b.instructions {
            let mut used = false;
            inst.for_each_used_value(|id| {
                if id == val.0 {
                    used = true;
                }
            });
            if used {
                return true;
            }
        }
        let mut tused = false;
        b.terminator.for_each_used_value(|id| {
            if id == val.0 {
                tused = true;
            }
        });
        if tused {
            return true;
        }
    }
    false
}

fn replace_all_uses_of_value(func: &mut IrFunction, old_val: Value, new_op: Operand) {
    for b in &mut func.blocks {
        for inst in &mut b.instructions {
            replace_operand_value(inst, old_val, new_op);
        }
        replace_terminator_value(&mut b.terminator, old_val, new_op);
    }
}

/// Decide whether `inst`'s read of `member` must be replaced by the narrow
/// escape truncation. Slots type-checked at 64-bit width keep the wide value:
///
/// - GEP offsets and intrinsic index arguments are address arithmetic;
/// - wide binops already read wide operands;
/// - cmp operands are governed by their own `CmpAction` (widened or truncated
///   next to the cmp), never by the block escape;
/// - cast sources stay wide (a retained cast truncates itself; a widened cast
///   is dropped/aliased by step 6).
///
/// Everything else (narrow binops, narrow select data, store values, call
/// arguments, return/switch values) is a narrow consumer and gets the trunc.
/// Whether an operand slot declared as `slot_ty` may read the **widened**
/// member directly instead of the re-truncated narrow value.
///
/// After widening, the member's SSA value physically holds `wide_ty`
/// (`I64`/`U64` on LP64). A reader is only type-correct when its declared
/// operand type is an integer/pointer slot of exactly that width: anything
/// narrower would silently drop the upper half, and anything of a *different
/// kind* (floating point, vector, `Void`) would reinterpret the bits.
///
/// The last case is not hypothetical. `for (i = 0; i < n; ++i) f((float) i)`
/// lowers to `Cast { src: i, from_ty: I32, to_ty: F32 }`, which is **not** a
/// closure member (an int→float conversion is not extension-transparent), so
/// the plan never retypes its `from_ty`. Letting it read the widened `i`
/// produced an `I32`-typed read of an `I64` value; the folder then collapsed
/// the whole conversion to `0.0f` and `K * (float) i` silently evaluated to
/// zero for every loop-carried `i` (gcc.c-torture `20060420-1.c`, and every
/// `sum += a[i] * (float) i` shape).
#[inline]
fn wide_read_is_type_correct(slot_ty: IrType, wide_ty: IrType) -> bool {
    matches!(slot_ty, IrType::Ptr) || (slot_ty.is_integer() && slot_ty.size() == wide_ty.size())
}

fn escape_read_needs_narrow(inst: &Instruction, member: Value, wide_ty: IrType) -> bool {
    let reads = |op: &Operand| matches!(op, Operand::Value(v) if v.0 == member.0);
    let narrow_slot = |ty: &IrType| !wide_read_is_type_correct(*ty, wide_ty);
    match inst {
        // GEP offsets and intrinsic index arguments are address arithmetic:
        // the backend types those slots at pointer width by construction.
        Instruction::GetElementPtr { .. } | Instruction::Intrinsic { .. } => false,
        Instruction::BinOp { ty, lhs, rhs, .. } => narrow_slot(ty) && (reads(lhs) || reads(rhs)),
        // `Cmp`/`Cast`/`Phi` used to be unconditionally wide-read. They are
        // only wide-read when their own declared type says so; a `CmpAction`
        // has already retyped (or re-truncated) every cmp the plan owns, so
        // whatever still reads the member here was not retyped.
        Instruction::Cmp { ty, lhs, rhs, .. } => narrow_slot(ty) && (reads(lhs) || reads(rhs)),
        Instruction::Cast { from_ty, src, .. } => narrow_slot(from_ty) && reads(src),
        // A Copy of a narrow member carries a narrow dest: it must read the
        // truncation, not the wide value.
        Instruction::Copy { .. } => true,
        Instruction::Select {
            ty,
            true_val,
            false_val,
            ..
        } => narrow_slot(ty) && (reads(true_val) || reads(false_val)),
        Instruction::Phi { ty, incoming, .. } => {
            narrow_slot(ty) && incoming.iter().any(|(op, _)| reads(op))
        }
        // Load reads a pointer, not the member; store values, calls, atomics
        // and everything else are narrow consumers.
        _ => true,
    }
}

// ---------------------------------------------------------------------------
// Invariant checks
// ---------------------------------------------------------------------------

fn operand_is_const_or_loop_invariant(
    op: &Operand,
    lp: &NaturalLoop,
    func: &IrFunction,
    phi_dest: Value,
) -> bool {
    match op {
        Operand::Const(_) => true,
        Operand::Value(v) => {
            if *v == phi_dest {
                return false; // cmp(phi, phi) — weird; bail
            }
            is_loop_invariant(v.0, &lp.body, func)
        }
    }
}

/// Where apply_widen materialises hoisted widening casts: appended to the end
/// of the loop preheader.
struct PreheaderSite<'a> {
    preheader: usize,
    dom: &'a DominanceChecker,
}

/// Whether a widening cast of `op` may be appended to the preheader: `op` is a
/// constant, or a loop-invariant value whose defining block dominates the
/// preheader (a def IN the preheader precedes the appended cast).
///
/// Loop invariance alone ("not defined in the body") is not sufficient. The
/// closure also follows members out of the loop, so a loop-EXIT compare can
/// read a value defined after the loop: SQLite's `selectExpander` reloads
/// `pEList->nExpr` in the exit block for `if( k<pEList->nExpr )`, and
/// hoisting that value's sext into the preheader read an undefined register
/// (the `SELECT * FROM (subquery)` column-name miscompile). For an in-loop
/// use the check is implied by SSA dominance (an invariant def dominating a
/// body use dominates the header, hence the preheader), so only exit
/// compares are affected. A value with no defining instruction is
/// ill-formed IR and is conservatively rejected.
fn operand_hoistable_to_preheader(
    op: &Operand,
    lp: &NaturalLoop,
    func: &IrFunction,
    phi_dest: Value,
    site: &PreheaderSite<'_>,
) -> bool {
    if !operand_is_const_or_loop_invariant(op, lp, func, phi_dest) {
        return false;
    }
    match op {
        Operand::Const(_) => true,
        Operand::Value(v) => match find_def(func, *v) {
            Some((bi, _)) => site.dom.dominates(bi, site.preheader),
            None => false,
        },
    }
}

fn is_loop_invariant(val_id: u32, body: &FxHashSet<usize>, func: &IrFunction) -> bool {
    for (bi, b) in func.blocks.iter().enumerate() {
        if body.contains(&bi) {
            for inst in &b.instructions {
                if let Some(d) = inst.dest() {
                    if d.0 == val_id {
                        return false;
                    }
                }
            }
        }
    }
    true
}

// ---------------------------------------------------------------------------
// Tests
//
// The pass is additionally validated end-to-end by
// `tests/regression/run_regression_suite.sh`, the benchmark output gate
// (`scripts/check_benchmark_outputs.sh`) and the benchmark A/B harness; these
// unit tests pin the provenance and soundness contract directly on hand-built
// IR shapes, and every one ends with `verify_function` to assert the result
// is structurally valid SSA.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::types::AddressSpace;

    fn check(func: &mut IrFunction, expected_widened: usize) {
        let n = widen_ivs_in_function(func);
        assert_eq!(
            n, expected_widened,
            "function {:?}: expected {} widened IVs",
            func.name, expected_widened
        );
        let mut violations = Vec::new();
        crate::passes::verify::verify_function(func, "iv_widen_test", &mut violations);
        assert!(
            violations.is_empty(),
            "verify_function found violations: {:?}",
            violations
        );
    }

    /// A 5-block counted loop: B0 preheader, B1 header (phi + optional cmp),
    /// B2 body, B3 latch, B4 exit. `body_insts` is inserted into B2; the
    /// optional `cmp` becomes the header exit test (B2 / B4).
    fn counting_loop(
        name: &str,
        phi_ty: IrType,
        latch_op: IrBinOp,
        body_insts: Vec<Instruction>,
        cmp: Option<Instruction>,
    ) -> IrFunction {
        let mut func = IrFunction::new(name.to_string(), IrType::I32, vec![], false);
        // B0 preheader: init i = 3 (value 0); materialize the array base
        // `gep_inst` hardcodes as Value(50). A base with no defining
        // instruction is an undefined-use violation under the
        // def-dominates-use verifier, so the fixture must define it where
        // the tests claim it lives ("loop-invariant, defined outside").
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![
                Instruction::Copy {
                    dest: Value(0),
                    src: Operand::Const(IrConst::I32(3)),
                },
                Instruction::GlobalAddr {
                    dest: Value(50),
                    name: "arr".to_string(),
                },
            ],
            terminator: Terminator::Branch(BlockId(1)),
            source_spans: Vec::new(),
        });
        // B1 header: phi %1 = phi(init=B0, latch=B3)
        let mut header_insts = vec![Instruction::Phi {
            dest: Value(1),
            ty: phi_ty,
            incoming: vec![
                (Operand::Value(Value(0)), BlockId(0)),
                (Operand::Value(Value(5)), BlockId(3)),
            ],
        }];
        let header_term = if let Some(c) = cmp {
            let d = c.dest().unwrap();
            header_insts.push(c);
            Terminator::CondBranch {
                cond: Operand::Value(d),
                true_label: BlockId(2),
                false_label: BlockId(4),
            }
        } else {
            Terminator::Branch(BlockId(2))
        };
        func.blocks.push(BasicBlock {
            label: BlockId(1),
            instructions: header_insts,
            terminator: header_term,
            source_spans: Vec::new(),
        });
        // B2 body
        func.blocks.push(BasicBlock {
            label: BlockId(2),
            instructions: body_insts,
            terminator: Terminator::Branch(BlockId(3)),
            source_spans: Vec::new(),
        });
        // B3 latch: %5 = phi op const 1
        func.blocks.push(BasicBlock {
            label: BlockId(3),
            instructions: vec![Instruction::BinOp {
                dest: Value(5),
                op: latch_op,
                lhs: Operand::Value(Value(1)),
                rhs: Operand::Const(IrConst::I32(1)),
                ty: phi_ty,
            }],
            terminator: Terminator::Branch(BlockId(1)),
            source_spans: Vec::new(),
        });
        // B4 exit
        func.blocks.push(BasicBlock {
            label: BlockId(4),
            instructions: vec![],
            terminator: Terminator::Return(None),
            source_spans: Vec::new(),
        });
        func.next_value_id = 100;
        func.next_label = 5;
        func
    }

    fn gep_inst(dest: u32, offset: u32, ty: IrType) -> Instruction {
        Instruction::GetElementPtr {
            dest: Value(dest),
            base: Value(50),
            offset: Operand::Value(Value(offset)),
            ty,
        }
    }

    /// `a[i]` byte-array shape: `Cast i32→i64(phi)` feeding a GEP offset.
    #[test]
    fn test_widen_basic_cast_gep() {
        let body = vec![
            Instruction::Cast {
                dest: Value(10),
                src: Operand::Value(Value(1)),
                from_ty: IrType::I32,
                to_ty: IrType::I64,
            },
            gep_inst(11, 10, IrType::I8),
            Instruction::Store {
                volatile: false,
                val: Operand::Const(IrConst::I32(0)),
                ptr: Value(11),
                ty: IrType::I8,
                seg_override: AddressSpace::Default,
            },
        ];
        let mut func = counting_loop("basic", IrType::I32, IrBinOp::Add, body, None);
        check(&mut func, 1);
        let phi = &func.blocks[1].instructions[0];
        assert!(matches!(
            phi,
            Instruction::Phi {
                ty: IrType::I64,
                ..
            }
        ));
        assert!(
            !func.blocks[2]
                .instructions
                .iter()
                .any(|i| matches!(i, Instruction::Cast { .. }))
        );
        assert!(func.blocks[2].instructions.iter().any(
            |i| matches!(i, Instruction::GetElementPtr { offset: Operand::Value(v), .. } if v.0 == 1)
        ));
    }

    /// `a[i]` loop (header exit test `i < 10`) followed by a loop-EXIT compare
    /// `i < n`. `bound_in_exit` places the load of `n` in the exit block (a
    /// post-loop reload, as in SQLite's `selectExpander`); otherwise it is
    /// loaded in the preheader.
    fn exit_cmp_loop(name: &str, bound_in_exit: bool) -> IrFunction {
        let body = vec![
            Instruction::Cast {
                dest: Value(10),
                src: Operand::Value(Value(1)),
                from_ty: IrType::I32,
                to_ty: IrType::I64,
            },
            gep_inst(11, 10, IrType::I8),
            Instruction::Store {
                volatile: false,
                val: Operand::Const(IrConst::I32(0)),
                ptr: Value(11),
                ty: IrType::I8,
                seg_override: AddressSpace::Default,
            },
        ];
        let header_cmp = Instruction::Cmp {
            dest: Value(2),
            op: IrCmpOp::Slt,
            lhs: Operand::Value(Value(1)),
            rhs: Operand::Const(IrConst::I32(10)),
            ty: IrType::I32,
        };
        let mut func = counting_loop(name, IrType::I32, IrBinOp::Add, body, Some(header_cmp));
        let load = Instruction::Load {
            dest: Value(60),
            ptr: Value(50),
            ty: IrType::I32,
            seg_override: AddressSpace::Default,
            volatile: false,
        };
        if bound_in_exit {
            func.blocks[4].instructions.push(load);
        } else {
            func.blocks[0].instructions.push(load);
        }
        func.blocks[4].instructions.push(Instruction::Cmp {
            dest: Value(61),
            op: IrCmpOp::Slt,
            lhs: Operand::Value(Value(1)),
            rhs: Operand::Value(Value(60)),
            ty: IrType::I32,
        });
        func.blocks[4].terminator = Terminator::Return(Some(Operand::Value(Value(61))));
        func
    }

    fn exit_cmp(func: &IrFunction) -> (IrType, Operand, Operand) {
        func.blocks[4]
            .instructions
            .iter()
            .find_map(|i| match i {
                Instruction::Cmp {
                    dest, lhs, rhs, ty, ..
                } if dest.0 == 61 => Some((*ty, *lhs, *rhs)),
                _ => None,
            })
            .expect("exit cmp v61 survives")
    }

    /// LCCC-SQLITE-STAR-EXPAND: the exit compare's bound is defined AFTER
    /// the loop, so its widening cast cannot be hoisted to the preheader.
    /// The IV is still widened; the exit compare stays narrow (I32, fed by a
    /// truncation of the wide IV) and reads the bound directly — and nothing
    /// in the preheader reads v60 (`check` also runs the def-dominates-use
    /// verifier, which rejected the old preheader `sext v60`).
    #[test]
    fn test_exit_cmp_bound_defined_after_loop_stays_narrow() {
        let mut func = exit_cmp_loop("exit_bound_after_loop", true);
        check(&mut func, 1);
        assert!(matches!(
            func.blocks[1].instructions[0],
            Instruction::Phi {
                ty: IrType::I64,
                ..
            }
        ));
        let (ty, lhs, rhs) = exit_cmp(&func);
        assert_eq!(ty, IrType::I32);
        assert!(matches!(rhs, Operand::Value(v) if v.0 == 60));
        let Operand::Value(trunc) = lhs else {
            panic!("exit cmp lhs must be the truncated IV, got {lhs:?}");
        };
        assert!(func.blocks[4].instructions.iter().any(|i| matches!(
            i,
            Instruction::Cast { dest, src: Operand::Value(s), from_ty: IrType::I64, to_ty: IrType::I32 }
                if dest.0 == trunc.0 && s.0 == 1
        )));
        assert!(!func.blocks[0].instructions.iter().any(|i| matches!(
            i,
            Instruction::Cast { src: Operand::Value(s), .. } if s.0 == 60
        )));
    }

    /// Control: the same exit compare with the bound available in the
    /// preheader keeps the established widening (sext hoisted into the
    /// preheader, I64 compare against the wide IV).
    #[test]
    fn test_exit_cmp_bound_in_preheader_is_widened() {
        let mut func = exit_cmp_loop("exit_bound_in_preheader", false);
        check(&mut func, 1);
        let (ty, lhs, rhs) = exit_cmp(&func);
        assert_eq!(ty, IrType::I64);
        assert!(matches!(lhs, Operand::Value(v) if v.0 == 1));
        let Operand::Value(wide) = rhs else {
            panic!("exit cmp rhs must be the hoisted sext, got {rhs:?}");
        };
        assert!(func.blocks[0].instructions.iter().any(|i| matches!(
            i,
            Instruction::Cast { dest, src: Operand::Value(s), from_ty: IrType::I32, to_ty: IrType::I64 }
                if dest.0 == wide.0 && s.0 == 60
        )));
    }

    /// Element-scale chain: `Cast i32→i64(phi); Shl i64 2; GEP`.
    #[test]
    fn test_widen_scaled_int_chain() {
        let body = vec![
            Instruction::Cast {
                dest: Value(10),
                src: Operand::Value(Value(1)),
                from_ty: IrType::I32,
                to_ty: IrType::I64,
            },
            Instruction::BinOp {
                dest: Value(11),
                op: IrBinOp::Shl,
                lhs: Operand::Value(Value(10)),
                rhs: Operand::Const(IrConst::I32(2)),
                ty: IrType::I64,
            },
            gep_inst(12, 11, IrType::I32),
            Instruction::Store {
                volatile: false,
                val: Operand::Const(IrConst::I32(0)),
                ptr: Value(12),
                ty: IrType::I32,
                seg_override: AddressSpace::Default,
            },
        ];
        let mut func = counting_loop("scaled", IrType::I32, IrBinOp::Add, body, None);
        check(&mut func, 1);
        let shl = func.blocks[2]
            .instructions
            .iter()
            .find(|i| {
                matches!(
                    i,
                    Instruction::BinOp {
                        op: IrBinOp::Shl,
                        ..
                    }
                )
            })
            .unwrap();
        assert!(
            matches!(shl, Instruction::BinOp { ty: IrType::I64, lhs: Operand::Value(v), .. } if v.0 == 1)
        );
    }

    /// Closure: `(i & 7)` bitwise member feeding a GEP through a cast.
    #[test]
    fn test_closure_and_mask() {
        let body = vec![
            Instruction::BinOp {
                dest: Value(10),
                op: IrBinOp::And,
                lhs: Operand::Value(Value(1)),
                rhs: Operand::Const(IrConst::I32(7)),
                ty: IrType::I32,
            },
            Instruction::Cast {
                dest: Value(11),
                src: Operand::Value(Value(10)),
                from_ty: IrType::I32,
                to_ty: IrType::I64,
            },
            gep_inst(12, 11, IrType::I32),
            Instruction::Store {
                volatile: false,
                val: Operand::Const(IrConst::I32(0)),
                ptr: Value(12),
                ty: IrType::I32,
                seg_override: AddressSpace::Default,
            },
        ];
        let mut func = counting_loop("andmask", IrType::I32, IrBinOp::Add, body, None);
        check(&mut func, 1);
        let and = func.blocks[2]
            .instructions
            .iter()
            .find(|i| {
                matches!(
                    i,
                    Instruction::BinOp {
                        op: IrBinOp::And,
                        ..
                    }
                )
            })
            .unwrap();
        assert!(matches!(
            and,
            Instruction::BinOp {
                ty: IrType::I64,
                rhs: Operand::Const(IrConst::I64(7)),
                ..
            }
        ));
    }

    /// Closure: `a[i+1]` const-offset member.
    #[test]
    fn test_closure_offset_add() {
        let body = vec![
            Instruction::BinOp {
                dest: Value(10),
                op: IrBinOp::Add,
                lhs: Operand::Value(Value(1)),
                rhs: Operand::Const(IrConst::I32(1)),
                ty: IrType::I32,
            },
            Instruction::Cast {
                dest: Value(11),
                src: Operand::Value(Value(10)),
                from_ty: IrType::I32,
                to_ty: IrType::I64,
            },
            gep_inst(12, 11, IrType::I32),
            Instruction::Store {
                volatile: false,
                val: Operand::Const(IrConst::I32(0)),
                ptr: Value(12),
                ty: IrType::I32,
                seg_override: AddressSpace::Default,
            },
        ];
        let mut func = counting_loop("offset", IrType::I32, IrBinOp::Add, body, None);
        check(&mut func, 1);
        let add = func.blocks[2]
            .instructions
            .iter()
            .find(|i| matches!(i, Instruction::BinOp { op: IrBinOp::Add, dest: Value(d), .. } if *d == 10))
            .unwrap();
        assert!(matches!(
            add,
            Instruction::BinOp {
                ty: IrType::I64,
                rhs: Operand::Const(IrConst::I64(1)),
                ..
            }
        ));
    }

    /// Decrementing IV: `Sub` latch, `a[i-1]` offset.
    #[test]
    fn test_decrementing_iv() {
        let body = vec![
            Instruction::BinOp {
                dest: Value(10),
                op: IrBinOp::Sub,
                lhs: Operand::Value(Value(1)),
                rhs: Operand::Const(IrConst::I32(1)),
                ty: IrType::I32,
            },
            Instruction::Cast {
                dest: Value(11),
                src: Operand::Value(Value(10)),
                from_ty: IrType::I32,
                to_ty: IrType::I64,
            },
            gep_inst(12, 11, IrType::I32),
            Instruction::Store {
                volatile: false,
                val: Operand::Const(IrConst::I32(0)),
                ptr: Value(12),
                ty: IrType::I32,
                seg_override: AddressSpace::Default,
            },
        ];
        let mut func = counting_loop("desc", IrType::I32, IrBinOp::Sub, body, None);
        check(&mut func, 1);
        assert!(matches!(
            &func.blocks[3].instructions[0],
            Instruction::BinOp {
                op: IrBinOp::Sub,
                ty: IrType::I64,
                ..
            }
        ));
    }

    /// Latch `Add(step, phi)` (phi on the RHS): the regression the previous
    /// revision corrupted by always overwriting `rhs`.
    #[test]
    fn test_latch_add_step_phi_order() {
        let mut func = counting_loop("swap", IrType::I32, IrBinOp::Add, vec![], None);
        // Make the latch `%5 = 1 + %1` (step on the LHS).
        func.blocks[3].instructions[0] = Instruction::BinOp {
            dest: Value(5),
            op: IrBinOp::Add,
            lhs: Operand::Const(IrConst::I32(1)),
            rhs: Operand::Value(Value(1)),
            ty: IrType::I32,
        };
        // Give it an addressing use so widening is profitable.
        func.blocks[2].instructions = vec![
            Instruction::Cast {
                dest: Value(10),
                src: Operand::Value(Value(1)),
                from_ty: IrType::I32,
                to_ty: IrType::I64,
            },
            gep_inst(11, 10, IrType::I8),
        ];
        check(&mut func, 1);
        let Instruction::BinOp { lhs, rhs, ty, .. } = &func.blocks[3].instructions[0] else {
            panic!("latch not a binop");
        };
        assert_eq!(*ty, IrType::I64, "latch must be widened");
        assert!(
            matches!(lhs, Operand::Value(v) if v.0 == 1),
            "latch lhs must stay the phi"
        );
        assert!(
            matches!(rhs, Operand::Const(IrConst::I64(1))),
            "latch rhs must be the widened step, not a duplicated phi"
        );
    }

    /// Escape: the `match_len` shape — the IV is also the return value.
    #[test]
    fn test_escape_trunc_return() {
        let cmp = Instruction::Cmp {
            dest: Value(99),
            op: IrCmpOp::Slt,
            lhs: Operand::Value(Value(1)),
            rhs: Operand::Const(IrConst::I32(64)),
            ty: IrType::I32,
        };
        let body = vec![
            Instruction::Cast {
                dest: Value(10),
                src: Operand::Value(Value(1)),
                from_ty: IrType::I32,
                to_ty: IrType::I64,
            },
            gep_inst(11, 10, IrType::I8),
            Instruction::Load {
                dest: Value(12),
                ptr: Value(11),
                ty: IrType::I8,
                volatile: false,
                seg_override: AddressSpace::Default,
            },
        ];
        let mut func = counting_loop("esc", IrType::I32, IrBinOp::Add, body, Some(cmp));
        // Exit block (B4) returns the IV.
        func.blocks[4].terminator = Terminator::Return(Some(Operand::Value(Value(1))));
        check(&mut func, 1);
        let has_trunc = func.blocks[4].instructions.iter().any(|i| {
            matches!(i, Instruction::Cast { src: Operand::Value(v), to_ty: IrType::I32, .. } if v.0 == 1)
        });
        assert!(has_trunc, "exit block must truncate the escaped IV");
        assert!(matches!(
            func.blocks[4].terminator,
            Terminator::Return(Some(Operand::Value(Value(t)))) if t > 100
        ));
    }

    /// The vectorized-counter miscompile shape: one escape block holding BOTH
    /// wide consumers (a GEP offset reading the wide IV) and narrow consumers
    /// (an `And`, a store value). The narrow reads must see ONE truncation of
    /// the IV; the GEP offset must keep reading the wide value — rewriting it
    /// to the 32-bit truncation would put an I32 into a 64-bit slot.
    #[test]
    fn test_escape_block_mixed_wide_narrow_uses() {
        let cmp = Instruction::Cmp {
            dest: Value(99),
            op: IrCmpOp::Slt,
            lhs: Operand::Value(Value(1)),
            rhs: Operand::Const(IrConst::I32(64)),
            ty: IrType::I32,
        };
        let body = vec![
            Instruction::Cast {
                dest: Value(10),
                src: Operand::Value(Value(1)),
                from_ty: IrType::I32,
                to_ty: IrType::I64,
            },
            gep_inst(11, 10, IrType::I8),
            Instruction::Load {
                dest: Value(12),
                ptr: Value(11),
                ty: IrType::I8,
                volatile: false,
                seg_override: AddressSpace::Default,
            },
        ];
        let mut func = counting_loop("mixesc", IrType::I32, IrBinOp::Add, body, Some(cmp));
        // Exit block (B4): narrow And + store value AND a GEP offset reading
        // the IV, then return the And.
        func.blocks[4].instructions = vec![
            Instruction::BinOp {
                dest: Value(20),
                op: IrBinOp::And,
                lhs: Operand::Value(Value(1)),
                rhs: Operand::Const(IrConst::I32(7)),
                ty: IrType::I32,
            },
            gep_inst(21, 1, IrType::I32),
            Instruction::Store {
                ptr: Value(21),
                val: Operand::Value(Value(1)),
                ty: IrType::I32,
                volatile: false,
                seg_override: AddressSpace::Default,
            },
        ];
        func.blocks[4].terminator = Terminator::Return(Some(Operand::Value(Value(20))));
        check(&mut func, 1);

        let b4 = &func.blocks[4];
        // Exactly one truncation of the IV in the exit block.
        let truncs: Vec<&Instruction> = b4
            .instructions
            .iter()
            .filter(|i| {
                matches!(
                    i,
                    Instruction::Cast {
                        src: Operand::Value(v),
                        to_ty: IrType::I32,
                        ..
                    } if v.0 == 1
                )
            })
            .collect();
        assert_eq!(truncs.len(), 1, "exactly one IV truncation in exit block");
        let t = match truncs[0] {
            Instruction::Cast { dest, .. } => dest.0,
            _ => unreachable!(),
        };
        // Narrow consumers read the truncation.
        assert!(b4.instructions.iter().any(|i| matches!(
            i,
            Instruction::BinOp {
                op: IrBinOp::And,
                lhs: Operand::Value(v),
                ..
            } if v.0 == t
        )));
        assert!(b4.instructions.iter().any(|i| matches!(
            i,
            Instruction::Store {
                val: Operand::Value(v),
                ..
            } if v.0 == t
        )));
        // The GEP offset keeps the wide IV.
        assert!(b4.instructions.iter().any(|i| matches!(
            i,
            Instruction::GetElementPtr {
                offset: Operand::Value(v),
                ..
            } if v.0 == 1
        )));
        // No post-loop closure retyping: no wide And of the IV.
        assert!(!b4.instructions.iter().any(|i| matches!(
            i,
            Instruction::BinOp {
                op: IrBinOp::And,
                ty: IrType::I64,
                ..
            }
        )));
    }

    /// Unsigned counted loop: u32 phi, exit `Ult(phi, inv)`, zext chain.
    #[test]
    fn test_unsigned_counted_loop() {
        let cmp = Instruction::Cmp {
            dest: Value(99),
            op: IrCmpOp::Ult,
            lhs: Operand::Value(Value(1)),
            rhs: Operand::Const(IrConst::I32(64)),
            ty: IrType::U32,
        };
        let body = vec![
            Instruction::Cast {
                dest: Value(10),
                src: Operand::Value(Value(1)),
                from_ty: IrType::U32,
                to_ty: IrType::U64,
            },
            gep_inst(11, 10, IrType::I8),
        ];
        let mut func = counting_loop("ucount", IrType::U32, IrBinOp::Add, body, Some(cmp));
        // The latch const must be U32-typed.
        func.blocks[3].instructions[0] = Instruction::BinOp {
            dest: Value(5),
            op: IrBinOp::Add,
            lhs: Operand::Value(Value(1)),
            rhs: Operand::Const(IrConst::I32(1)),
            ty: IrType::U32,
        };
        check(&mut func, 1);
        assert!(matches!(
            &func.blocks[1].instructions[0],
            Instruction::Phi {
                ty: IrType::U64,
                ..
            }
        ));
        let cmp_inst = func.blocks[1]
            .instructions
            .iter()
            .find(|i| matches!(i, Instruction::Cmp { .. }))
            .unwrap();
        assert!(matches!(
            cmp_inst,
            Instruction::Cmp {
                ty: IrType::U64,
                op: IrCmpOp::Ult,
                ..
            }
        ));
    }

    /// Unsigned IV without a counted bound: must NOT widen.
    #[test]
    fn test_unsigned_no_bound_bails() {
        let body = vec![
            Instruction::Cast {
                dest: Value(10),
                src: Operand::Value(Value(1)),
                from_ty: IrType::U32,
                to_ty: IrType::U64,
            },
            gep_inst(11, 10, IrType::I8),
        ];
        let mut func = counting_loop("unbound", IrType::U32, IrBinOp::Add, body, None);
        check(&mut func, 0);
    }

    /// Unsigned IV whose exit cmp uses a SIGNED predicate (`i <s 64` on a
    /// U32 phi): theorem 5′ declines the WHOLE proof. A signed test confines
    /// an unsigned value to no interval at all — a ring-top seed satisfies
    /// `0xFFFFFFFF <s 64`, so the body runs at values outside every unsigned
    /// interval derived from the cmp. The pre-theorem-5′ pass proved `[0, 63]`
    /// from this shape and widened (the former fixture
    /// `test_unsigned_signed_pred_cmp_trunc` codified that unsound behaviour:
    /// it asserted a widened plan whose strict range a ring-top seed
    /// falsifies). Declining is the only sound reading; the cmp-action-level
    /// rule that a signed predicate keeps a narrow cmp (truncation) still
    /// holds for signed IVs and for non-exit cmps.
    #[test]
    fn test_unsigned_signed_pred_bound_declines() {
        let cmp = Instruction::Cmp {
            dest: Value(99),
            op: IrCmpOp::Slt,
            lhs: Operand::Value(Value(1)),
            rhs: Operand::Const(IrConst::I32(64)),
            ty: IrType::U32,
        };
        let body = vec![
            Instruction::Cast {
                dest: Value(10),
                src: Operand::Value(Value(1)),
                from_ty: IrType::U32,
                to_ty: IrType::U64,
            },
            gep_inst(11, 10, IrType::I8),
        ];
        let mut func = counting_loop("usgncmp", IrType::U32, IrBinOp::Add, body, Some(cmp));
        func.blocks[3].instructions[0] = Instruction::BinOp {
            dest: Value(5),
            op: IrBinOp::Add,
            lhs: Operand::Value(Value(1)),
            rhs: Operand::Const(IrConst::I32(1)),
            ty: IrType::U32,
        };
        check(&mut func, 0);
        assert!(matches!(
            &func.blocks[1].instructions[0],
            Instruction::Phi {
                ty: IrType::U32,
                ..
            }
        ));
    }

    /// Signed IV + unsigned predicate with a constant bound: widened
    /// (sext preserves the unsigned order — theorem 4).
    #[test]
    fn test_signed_iv_unsigned_pred_widen() {
        let cmp = Instruction::Cmp {
            dest: Value(99),
            op: IrCmpOp::Ult,
            lhs: Operand::Value(Value(1)),
            rhs: Operand::Const(IrConst::I32(64)),
            ty: IrType::I32,
        };
        let body = vec![
            Instruction::Cast {
                dest: Value(10),
                src: Operand::Value(Value(1)),
                from_ty: IrType::I32,
                to_ty: IrType::I64,
            },
            gep_inst(11, 10, IrType::I8),
        ];
        let mut func = counting_loop("sextucmp", IrType::I32, IrBinOp::Add, body, Some(cmp));
        check(&mut func, 1);
        let cmp_inst = func.blocks[1]
            .instructions
            .iter()
            .find(|i| matches!(i, Instruction::Cmp { .. }))
            .unwrap();
        assert!(matches!(
            cmp_inst,
            Instruction::Cmp {
                ty: IrType::I64,
                op: IrCmpOp::Ult,
                rhs: Operand::Const(IrConst::I64(64)),
                ..
            }
        ));
    }

    /// Loop-variant latch step: must bail (an invariant step is hoisted; a
    /// variant one would be an invalid-SSA preheader cast).
    #[test]
    fn test_variant_step_bails() {
        let mut func = counting_loop("vstep", IrType::I32, IrBinOp::Add, vec![], None);
        // Latch step becomes a value defined in the latch itself (value 7).
        func.blocks[3].instructions = vec![
            Instruction::Copy {
                dest: Value(7),
                src: Operand::Const(IrConst::I32(1)),
            },
            Instruction::BinOp {
                dest: Value(5),
                op: IrBinOp::Add,
                lhs: Operand::Value(Value(1)),
                rhs: Operand::Value(Value(7)),
                ty: IrType::I32,
            },
        ];
        func.blocks[2].instructions = vec![
            Instruction::Cast {
                dest: Value(10),
                src: Operand::Value(Value(1)),
                from_ty: IrType::I32,
                to_ty: IrType::I64,
            },
            gep_inst(11, 10, IrType::I8),
        ];
        check(&mut func, 0);
    }

    /// Invariant (preheader-defined) step: hoisted cast in the preheader.
    #[test]
    fn test_invariant_step_hoisted() {
        let mut func = counting_loop("istep", IrType::I32, IrBinOp::Add, vec![], None);
        // Step is a copy defined in the preheader (value 2).
        func.blocks[0].instructions.push(Instruction::Copy {
            dest: Value(2),
            src: Operand::Const(IrConst::I32(3)),
        });
        func.blocks[3].instructions[0] = Instruction::BinOp {
            dest: Value(5),
            op: IrBinOp::Add,
            lhs: Operand::Value(Value(1)),
            rhs: Operand::Value(Value(2)),
            ty: IrType::I32,
        };
        func.blocks[2].instructions = vec![
            Instruction::Cast {
                dest: Value(10),
                src: Operand::Value(Value(1)),
                from_ty: IrType::I32,
                to_ty: IrType::I64,
            },
            gep_inst(11, 10, IrType::I8),
        ];
        check(&mut func, 1);
        let has_hoist = func.blocks[0].instructions.iter().any(|i| {
            matches!(i, Instruction::Cast { src: Operand::Value(v), from_ty: IrType::I32, to_ty: IrType::I64, .. } if v.0 == 2)
        });
        assert!(has_hoist, "step cast must be hoisted to the preheader");
    }

    /// Two cmps of the same IV in one block: both must be handled without
    /// index skew.
    #[test]
    fn test_multi_cmp_same_block() {
        let c1 = Instruction::Cmp {
            dest: Value(90),
            op: IrCmpOp::Slt,
            lhs: Operand::Value(Value(1)),
            rhs: Operand::Const(IrConst::I32(10)),
            ty: IrType::I32,
        };
        let c2 = Instruction::Cmp {
            dest: Value(91),
            op: IrCmpOp::Sgt,
            lhs: Operand::Value(Value(1)),
            rhs: Operand::Const(IrConst::I32(2)),
            ty: IrType::I32,
        };
        let body = vec![
            Instruction::Cast {
                dest: Value(10),
                src: Operand::Value(Value(1)),
                from_ty: IrType::I32,
                to_ty: IrType::I64,
            },
            gep_inst(11, 10, IrType::I8),
        ];
        let mut func = counting_loop("twocmp", IrType::I32, IrBinOp::Add, body, Some(c1));
        func.blocks[1].instructions.push(c2);
        check(&mut func, 1);
        let cmps: Vec<_> = func.blocks[1]
            .instructions
            .iter()
            .filter(|i| matches!(i, Instruction::Cmp { .. }))
            .collect();
        assert_eq!(cmps.len(), 2, "both cmps must survive");
        assert!(cmps.iter().all(|c| matches!(
            c,
            Instruction::Cmp {
                ty: IrType::I64,
                ..
            }
        )));
    }

    /// Two-member arithmetic (signed): `y = i + (i & 7)`.
    #[test]
    fn test_member_member_binop() {
        let body = vec![
            Instruction::BinOp {
                dest: Value(10),
                op: IrBinOp::And,
                lhs: Operand::Value(Value(1)),
                rhs: Operand::Const(IrConst::I32(7)),
                ty: IrType::I32,
            },
            Instruction::BinOp {
                dest: Value(11),
                op: IrBinOp::Add,
                lhs: Operand::Value(Value(1)),
                rhs: Operand::Value(Value(10)),
                ty: IrType::I32,
            },
            Instruction::Cast {
                dest: Value(12),
                src: Operand::Value(Value(11)),
                from_ty: IrType::I32,
                to_ty: IrType::I64,
            },
            gep_inst(13, 12, IrType::I32),
        ];
        let mut func = counting_loop("twomem", IrType::I32, IrBinOp::Add, body, None);
        check(&mut func, 1);
        let add = func.blocks[2]
            .instructions
            .iter()
            .find(|i| matches!(i, Instruction::BinOp { op: IrBinOp::Add, dest: Value(d), .. } if *d == 11))
            .unwrap();
        assert!(matches!(
            add,
            Instruction::BinOp {
                ty: IrType::I64,
                rhs: Operand::Value(v),
                ..
            } if v.0 == 10
        ));
    }

    /// The previous closure miscompile shape: an op reading the IV whose
    /// other operand is NOT an admitted member must cause a bail.
    #[test]
    fn test_non_member_operand_bails() {
        // `v21 = Copy(7)` (loop-local, not a member), `v20 = Add(phi, v21)`.
        // The Add reads the phi but its other operand is not in the closure:
        // widening it would change semantics. The whole IV must be declined.
        let body = vec![
            Instruction::Copy {
                dest: Value(21),
                src: Operand::Const(IrConst::I32(7)),
            },
            Instruction::BinOp {
                dest: Value(20),
                op: IrBinOp::Add,
                lhs: Operand::Value(Value(1)),
                rhs: Operand::Value(Value(21)),
                ty: IrType::I32,
            },
            Instruction::Cast {
                dest: Value(10),
                src: Operand::Value(Value(1)),
                from_ty: IrType::I32,
                to_ty: IrType::I64,
            },
            gep_inst(11, 10, IrType::I8),
        ];
        let mut func = counting_loop("badmem", IrType::I32, IrBinOp::Add, body, None);
        check(&mut func, 0);
    }

    /// `i8`/`i16` IVs are DECLINED: widening would change the DEFINED 8/16-bit
    /// wrap of the latch truncation (`trunc(add(phi, 1):I32)`), and the
    /// signed-overflow-is-UB theorem does not cover promoted-narrow wrap.
    /// (The C frontend never emits a direct narrow `Add` latch anyway; this
    /// pins the decision on the synthetic shape.)
    #[test]
    fn test_i8_iv() {
        let body = vec![
            Instruction::Cast {
                dest: Value(10),
                src: Operand::Value(Value(1)),
                from_ty: IrType::I8,
                to_ty: IrType::I64,
            },
            gep_inst(11, 10, IrType::I8),
        ];
        let mut func = counting_loop("i8iv", IrType::I8, IrBinOp::Add, body, None);
        func.blocks[0].instructions[0] = Instruction::Copy {
            dest: Value(0),
            src: Operand::Const(IrConst::I8(3)),
        };
        func.blocks[3].instructions[0] = Instruction::BinOp {
            dest: Value(5),
            op: IrBinOp::Add,
            lhs: Operand::Value(Value(1)),
            rhs: Operand::Const(IrConst::I8(1)),
            ty: IrType::I8,
        };
        check(&mut func, 0);
        assert!(matches!(
            &func.blocks[1].instructions[0],
            Instruction::Phi { ty: IrType::I8, .. }
        ));
    }

    /// Narrow `Shl(phi, 1)` feeding a widening cast: the shift is retyped to
    /// wide with its count left narrow, the cast is dropped, and the GEP reads
    /// the shifted member directly.
    #[test]
    fn test_narrow_shift_closure() {
        let body = vec![
            Instruction::BinOp {
                dest: Value(10),
                op: IrBinOp::Shl,
                lhs: Operand::Value(Value(1)),
                rhs: Operand::Const(IrConst::I32(1)),
                ty: IrType::I32,
            },
            Instruction::Cast {
                dest: Value(11),
                src: Operand::Value(Value(10)),
                from_ty: IrType::I32,
                to_ty: IrType::I64,
            },
            gep_inst(12, 11, IrType::I32),
            Instruction::Store {
                volatile: false,
                val: Operand::Const(IrConst::I32(0)),
                ptr: Value(12),
                ty: IrType::I32,
                seg_override: AddressSpace::Default,
            },
        ];
        let mut func = counting_loop("nshift", IrType::I32, IrBinOp::Add, body, None);
        check(&mut func, 1);
        let phi = &func.blocks[1].instructions[0];
        assert!(matches!(
            phi,
            Instruction::Phi {
                ty: IrType::I64,
                ..
            }
        ));
        // The narrow shift is retyped to I64 with its count left as-is.
        assert!(
            func.blocks[2]
                .instructions
                .iter()
                .any(|i| matches!(i, Instruction::BinOp {
                op: IrBinOp::Shl,
                ty: IrType::I64,
                lhs: Operand::Value(v),
                rhs: Operand::Const(IrConst::I32(1)),
                ..
            } if v.0 == 1))
        );
        // The widening cast was dropped; the GEP reads the shifted member.
        assert!(func.blocks[2].instructions.iter().any(
            |i| matches!(i, Instruction::GetElementPtr { offset: Operand::Value(v), .. } if v.0 == 10)
        ));
    }

    /// A shift with an out-of-range count is declined (no widening).
    #[test]
    fn test_shift_bad_count_bails() {
        let body = vec![
            Instruction::BinOp {
                dest: Value(10),
                op: IrBinOp::Shl,
                lhs: Operand::Value(Value(1)),
                rhs: Operand::Const(IrConst::I32(64)),
                ty: IrType::I32,
            },
            Instruction::Cast {
                dest: Value(11),
                src: Operand::Value(Value(10)),
                from_ty: IrType::I32,
                to_ty: IrType::I64,
            },
            gep_inst(12, 11, IrType::I32),
            Instruction::Store {
                volatile: false,
                val: Operand::Const(IrConst::I32(0)),
                ptr: Value(12),
                ty: IrType::I32,
                seg_override: AddressSpace::Default,
            },
        ];
        let mut func = counting_loop("badshift", IrType::I32, IrBinOp::Add, body, None);
        check(&mut func, 0);
    }

    /// Unsigned constants must zero-extend (0xFFFFFFFF → 4294967295, not -1).
    #[test]
    fn test_unsigned_const_widen() {
        assert_eq!(
            widen_const(IrConst::I32(-1), IrType::U32),
            Some(IrConst::I64(0xFFFF_FFFF)),
            "U32 0xFFFFFFFF must zero-extend"
        );
        assert_eq!(
            widen_const(IrConst::I32(-1), IrType::I32),
            Some(IrConst::I64(-1)),
            "I32 -1 must sign-extend"
        );
        assert_eq!(
            widen_const(IrConst::I8(-8), IrType::U8),
            Some(IrConst::I64(0xF8)),
            "U8 0xF8 must zero-extend"
        );
        assert_eq!(
            widen_const(IrConst::I8(-8), IrType::I8),
            Some(IrConst::I64(-8)),
            "I8 -8 must sign-extend"
        );
    }

    /// No addressing use → no widening (nothing to gain).
    #[test]
    fn test_no_addressing_bails() {
        let body: Vec<Instruction> = vec![Instruction::Cmp {
            dest: Value(10),
            op: IrCmpOp::Slt,
            lhs: Operand::Value(Value(1)),
            rhs: Operand::Const(IrConst::I32(10)),
            ty: IrType::I32,
        }];
        let mut func = counting_loop("noaddr", IrType::I32, IrBinOp::Add, body, None);
        check(&mut func, 0);
    }

    /// An exit-merge phi reading the IV must bail.
    #[test]
    fn test_exit_merge_phi_bails() {
        let body = vec![
            Instruction::Cast {
                dest: Value(10),
                src: Operand::Value(Value(1)),
                from_ty: IrType::I32,
                to_ty: IrType::I64,
            },
            gep_inst(11, 10, IrType::I8),
        ];
        let mut func = counting_loop("mergephi", IrType::I32, IrBinOp::Add, body, None);
        func.blocks[4].instructions = vec![Instruction::Phi {
            dest: Value(60),
            ty: IrType::I32,
            incoming: vec![
                (Operand::Value(Value(1)), BlockId(1)),
                (Operand::Const(IrConst::I32(0)), BlockId(0)),
            ],
        }];
        check(&mut func, 0);
    }

    /// `Select` data operands: `sel = select(cond, i, i&7)` widens both arms.
    #[test]
    fn test_select_data_operand() {
        let body = vec![
            Instruction::BinOp {
                dest: Value(10),
                op: IrBinOp::And,
                lhs: Operand::Value(Value(1)),
                rhs: Operand::Const(IrConst::I32(7)),
                ty: IrType::I32,
            },
            Instruction::Select {
                dest: Value(11),
                cond: Operand::Value(Value(70)),
                true_val: Operand::Value(Value(1)),
                false_val: Operand::Value(Value(10)),
                ty: IrType::I32,
            },
            Instruction::Cast {
                dest: Value(12),
                src: Operand::Value(Value(11)),
                from_ty: IrType::I32,
                to_ty: IrType::I64,
            },
            gep_inst(13, 12, IrType::I32),
        ];
        let mut func = counting_loop("sel", IrType::I32, IrBinOp::Add, body, None);
        func.blocks[0].instructions.push(Instruction::Copy {
            dest: Value(70),
            src: Operand::Const(IrConst::I32(1)),
        });
        check(&mut func, 1);
        let sel = func.blocks[2]
            .instructions
            .iter()
            .find(|i| matches!(i, Instruction::Select { .. }))
            .unwrap();
        assert!(matches!(
            sel,
            Instruction::Select {
                ty: IrType::I64,
                true_val: Operand::Value(v),
                ..
            } if v.0 == 1
        ));
    }

    /// `Select` with the member as *condition*: bails (narrow truth use).
    #[test]
    fn test_select_cond_narrow_bails() {
        // The IV is used as a select *condition*: a truth-value use that
        // sext/zext provably preserves (nonzero ⟺ nonzero). The widening
        // proceeds, the select keeps its narrow result type and simply reads
        // the wide condition; no rewrite of the select is required.
        let body = vec![
            Instruction::Select {
                dest: Value(10),
                cond: Operand::Value(Value(1)),
                true_val: Operand::Const(IrConst::I32(1)),
                false_val: Operand::Const(IrConst::I32(0)),
                ty: IrType::I32,
            },
            Instruction::Cast {
                dest: Value(11),
                src: Operand::Value(Value(1)),
                from_ty: IrType::I32,
                to_ty: IrType::I64,
            },
            gep_inst(12, 11, IrType::I8),
        ];
        let mut func = counting_loop("selcond", IrType::I32, IrBinOp::Add, body, None);
        check(&mut func, 1);
        // The select survives untouched (narrow result), reading the wide IV.
        assert!(matches!(
            func.blocks[2].instructions[0],
            Instruction::Select {
                ty: IrType::I32,
                cond: Operand::Value(Value(1)),
                ..
            }
        ));
        // The redundant sext cast was dropped: the GEP now reads the wide phi.
        assert!(func.blocks[2].instructions.iter().any(|i| matches!(
            i,
            Instruction::GetElementPtr {
                offset: Operand::Value(Value(1)),
                ..
            }
        )));
        assert!(!func.blocks[2].instructions.iter().any(|i| matches!(
            i,
            Instruction::Cast {
                dest: Value(11),
                ..
            }
        )));
    }

    /// Rotated self-loop (header == latch): widenable with a preheader.
    #[test]
    fn test_self_loop_rotated() {
        // B0 preheader → B1 (header == latch: phi, exit cmp, add, back-edge),
        // B2 exit.
        let mut func = IrFunction::new("self".to_string(), IrType::I32, vec![], false);
        // Preheader also defines the GEP's array base (v50): the base is
        // loop-invariant, and without a defining instruction the verifier
        // reports the GEP as an undefined-value use.
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![
                Instruction::Copy {
                    dest: Value(0),
                    src: Operand::Const(IrConst::I32(0)),
                },
                Instruction::GlobalAddr {
                    dest: Value(50),
                    name: "arr".to_string(),
                },
            ],
            terminator: Terminator::Branch(BlockId(1)),
            source_spans: Vec::new(),
        });
        func.blocks.push(BasicBlock {
            label: BlockId(1),
            instructions: vec![
                Instruction::Phi {
                    dest: Value(1),
                    ty: IrType::I32,
                    incoming: vec![
                        (Operand::Value(Value(0)), BlockId(0)),
                        (Operand::Value(Value(5)), BlockId(1)),
                    ],
                },
                Instruction::Cast {
                    dest: Value(10),
                    src: Operand::Value(Value(1)),
                    from_ty: IrType::I32,
                    to_ty: IrType::I64,
                },
                gep_inst(11, 10, IrType::I8),
                Instruction::BinOp {
                    dest: Value(5),
                    op: IrBinOp::Add,
                    lhs: Operand::Value(Value(1)),
                    rhs: Operand::Const(IrConst::I32(1)),
                    ty: IrType::I32,
                },
                Instruction::Cmp {
                    dest: Value(99),
                    op: IrCmpOp::Slt,
                    lhs: Operand::Value(Value(1)),
                    rhs: Operand::Const(IrConst::I32(64)),
                    ty: IrType::I32,
                },
            ],
            terminator: Terminator::CondBranch {
                cond: Operand::Value(Value(99)),
                true_label: BlockId(1),
                false_label: BlockId(2),
            },
            source_spans: Vec::new(),
        });
        func.blocks.push(BasicBlock {
            label: BlockId(2),
            instructions: vec![],
            terminator: Terminator::Return(None),
            source_spans: Vec::new(),
        });
        func.next_value_id = 100;
        func.next_label = 3;
        check(&mut func, 1);
        assert!(matches!(
            &func.blocks[1].instructions[0],
            Instruction::Phi {
                ty: IrType::I64,
                ..
            }
        ));
        // Locate the latch Add by dest — earlier indices shift when the
        // redundant sext cast is dropped.
        assert!(func.blocks[1].instructions.iter().any(|i| matches!(
            i,
            Instruction::BinOp {
                dest: Value(5),
                ty: IrType::I64,
                ..
            }
        )));
    }

    /// The latch result (`a[i++]`-style post value) is itself a member: a GEP
    /// reading the post-increment value widens like any other use.
    #[test]
    fn test_post_increment_gep() {
        // Post-increment addressing: the GEP reads the INCREMENTED value
        // (value 5, the step result), not the phi.  The only
        // dominance-legal CFG for that is a rotated self-loop — header
        // == latch, with the step computed in the block before the use —
        // so the fixture is a single-block loop (the shape loop rotation
        // produces for `while (...) { use(*p); p++; }`-style code).  The
        // original fixture read the latch's value from a separate body
        // block: a use before any executing path computes it, invalid
        // SSA the def-dominates-use verifier rejects.
        let mut func = IrFunction::new("postinc".to_string(), IrType::I32, vec![], false);
        // B0 preheader: init i = 3, plus the array base the GEP uses.
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![
                Instruction::Copy {
                    dest: Value(0),
                    src: Operand::Const(IrConst::I32(3)),
                },
                Instruction::GlobalAddr {
                    dest: Value(50),
                    name: "arr".to_string(),
                },
            ],
            terminator: Terminator::Branch(BlockId(1)),
            source_spans: Vec::new(),
        });
        // B1: phi, step, widening cast of the STEP, GEP, guard.
        func.blocks.push(BasicBlock {
            label: BlockId(1),
            instructions: vec![
                Instruction::Phi {
                    dest: Value(1),
                    ty: IrType::I32,
                    incoming: vec![
                        (Operand::Value(Value(0)), BlockId(0)),
                        (Operand::Value(Value(5)), BlockId(1)),
                    ],
                },
                Instruction::BinOp {
                    dest: Value(5),
                    op: IrBinOp::Add,
                    lhs: Operand::Value(Value(1)),
                    rhs: Operand::Const(IrConst::I32(1)),
                    ty: IrType::I32,
                },
                Instruction::Cast {
                    dest: Value(10),
                    src: Operand::Value(Value(5)),
                    from_ty: IrType::I32,
                    to_ty: IrType::I64,
                },
                gep_inst(11, 10, IrType::I8),
                Instruction::Cmp {
                    dest: Value(99),
                    op: IrCmpOp::Slt,
                    lhs: Operand::Value(Value(1)),
                    rhs: Operand::Const(IrConst::I32(64)),
                    ty: IrType::I32,
                },
            ],
            terminator: Terminator::CondBranch {
                cond: Operand::Value(Value(99)),
                true_label: BlockId(1),
                false_label: BlockId(2),
            },
            source_spans: Vec::new(),
        });
        // B2: exit.
        func.blocks.push(BasicBlock {
            label: BlockId(2),
            instructions: vec![],
            terminator: Terminator::Return(Some(Operand::Value(Value(1)))),
            source_spans: Vec::new(),
        });
        func.next_value_id = 100;
        func.next_label = 3;
        check(&mut func, 1);
        // The step was widened in place...
        assert!(matches!(
            &func.blocks[1].instructions[1],
            Instruction::BinOp {
                ty: IrType::I64,
                ..
            }
        ));
        // ...and the post-increment GEP survived, still addressed by the
        // (now wide) step value.
        assert!(func.blocks[1].instructions.iter().any(
            |i| matches!(i, Instruction::GetElementPtr { offset: Operand::Value(v), .. } if v.0 == 5)
        ));
    }

    /// Cross-sign zext chain: `unsigned i; s += a[i >> 1]`. The frontend
    /// emits `LShr(U32) → Cast(U32→I64) → Shl(I64) → GEP`, and the
    /// signedness-changing cast used to stop the closure dead: the cast dest
    /// was never enqueued, so the wide scaling op and the GEP below it were
    /// never scanned, `has_addressing` stayed false, and the IV declined
    /// (verified: the 32-bit counter survived at -O2 on main @ dcf673d). The
    /// cast value is a zext, which equals the widened U64 member on every
    /// executed iteration, so it is retained as a same-size U64→I64
    /// reinterpretation (a no-op on x86) and the chain below it is
    /// classified normally.
    #[test]
    fn test_cross_sign_zext_chain_widens() {
        let cmp = Instruction::Cmp {
            dest: Value(99),
            op: IrCmpOp::Ult,
            lhs: Operand::Value(Value(1)),
            rhs: Operand::Const(IrConst::I32(64)),
            ty: IrType::U32,
        };
        let body = vec![
            Instruction::BinOp {
                dest: Value(10),
                op: IrBinOp::LShr,
                lhs: Operand::Value(Value(1)),
                rhs: Operand::Const(IrConst::I32(1)),
                ty: IrType::U32,
            },
            Instruction::Cast {
                dest: Value(11),
                src: Operand::Value(Value(10)),
                from_ty: IrType::U32,
                to_ty: IrType::I64,
            },
            Instruction::BinOp {
                dest: Value(12),
                op: IrBinOp::Shl,
                lhs: Operand::Value(Value(11)),
                rhs: Operand::Const(IrConst::I64(2)),
                ty: IrType::I64,
            },
            gep_inst(13, 12, IrType::I8),
        ];
        let mut func = counting_loop("xzext", IrType::U32, IrBinOp::Add, body, Some(cmp));
        // The latch const must be U32-typed.
        func.blocks[3].instructions[0] = Instruction::BinOp {
            dest: Value(5),
            op: IrBinOp::Add,
            lhs: Operand::Value(Value(1)),
            rhs: Operand::Const(IrConst::I32(1)),
            ty: IrType::U32,
        };
        check(&mut func, 1);
        assert!(matches!(
            &func.blocks[1].instructions[0],
            Instruction::Phi {
                ty: IrType::U64,
                ..
            }
        ));
        // The retained cast must read the wide member (U64 → I64 bitcast).
        let cast = func.blocks[2]
            .instructions
            .iter()
            .find(|i| {
                matches!(
                    i,
                    Instruction::Cast {
                        dest: Value(11),
                        ..
                    }
                )
            })
            .expect("cross-sign cast retained");
        assert!(matches!(
            cast,
            Instruction::Cast {
                from_ty: IrType::U64,
                to_ty: IrType::I64,
                ..
            }
        ));
    }

    /// Unsigned `Shl` whose range proof shows bits would be shifted out: the
    /// member must be declined (wrapped narrow != wide shift), not widened.
    #[test]
    fn test_unsigned_shl_wrap_bails() {
        // Bound 0xC0000000: the seed range [0, 0xC0000000) shifted left by 1
        // reaches 0x180000000, outside u32 — the wrap proof must reject.
        let cmp = Instruction::Cmp {
            dest: Value(99),
            op: IrCmpOp::Ult,
            lhs: Operand::Value(Value(1)),
            rhs: Operand::Const(IrConst::I32(-0x4000_0000)), // u32 bits 0xC0000000
            ty: IrType::U32,
        };
        let body = vec![
            Instruction::BinOp {
                dest: Value(10),
                op: IrBinOp::Shl,
                lhs: Operand::Value(Value(1)),
                rhs: Operand::Const(IrConst::I32(1)),
                ty: IrType::U32,
            },
            gep_inst(11, 10, IrType::I8),
        ];
        let mut func = counting_loop("ushlwrap", IrType::U32, IrBinOp::Add, body, Some(cmp));
        func.blocks[3].instructions[0] = Instruction::BinOp {
            dest: Value(5),
            op: IrBinOp::Add,
            lhs: Operand::Value(Value(1)),
            rhs: Operand::Const(IrConst::I32(1)),
            ty: IrType::U32,
        };
        check(&mut func, 0);
    }

    /// Unsigned `AShr` cannot arise from C and does not commute with zext:
    /// declined. Signed `LShr` cannot arise from C: declined.
    #[test]
    fn test_shift_foreign_sign_bails() {
        let body_u = vec![
            Instruction::BinOp {
                dest: Value(10),
                op: IrBinOp::AShr,
                lhs: Operand::Value(Value(1)),
                rhs: Operand::Const(IrConst::I32(1)),
                ty: IrType::U32,
            },
            gep_inst(11, 10, IrType::I8),
        ];
        let cmp = Instruction::Cmp {
            dest: Value(99),
            op: IrCmpOp::Ult,
            lhs: Operand::Value(Value(1)),
            rhs: Operand::Const(IrConst::I32(64)),
            ty: IrType::U32,
        };
        let mut func = counting_loop("uashr", IrType::U32, IrBinOp::Add, body_u, Some(cmp));
        func.blocks[3].instructions[0] = Instruction::BinOp {
            dest: Value(5),
            op: IrBinOp::Add,
            lhs: Operand::Value(Value(1)),
            rhs: Operand::Const(IrConst::I32(1)),
            ty: IrType::U32,
        };
        check(&mut func, 0);

        let body_s = vec![
            Instruction::BinOp {
                dest: Value(10),
                op: IrBinOp::LShr,
                lhs: Operand::Value(Value(1)),
                rhs: Operand::Const(IrConst::I32(1)),
                ty: IrType::I32,
            },
            gep_inst(11, 10, IrType::I8),
        ];
        let mut func = counting_loop("slshr", IrType::I32, IrBinOp::Add, body_s, None);
        check(&mut func, 0);
    }

    /// Unsigned IV with a RUNTIME trip bound and a full accumulate body
    /// (`for (i = 0; i < n; i++) s += a[i>>1]`, all runtime): the counted
    /// bound needs no constant fold — any seed < n with unit steps exits at
    /// ≤ n, so no executed value wraps u32. The cross-sign zext chain widens
    /// and the accumulator phi / exit conversion stay untouched.
    #[test]
    fn test_runtime_bound_cross_cast_accum() {
        let mut func = IrFunction::new("rtb".to_string(), IrType::I64, vec![], false);
        // B0 preheader: base ptr v2 = param 0, bound v3 = param 1 (U32),
        // accumulator init v0, IV init v1.
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![
                Instruction::ParamRef {
                    dest: Value(2),
                    param_idx: 0,
                    ty: IrType::Ptr,
                },
                Instruction::ParamRef {
                    dest: Value(3),
                    param_idx: 1,
                    ty: IrType::U32,
                },
                Instruction::Copy {
                    dest: Value(0),
                    src: Operand::Const(IrConst::I64(0)),
                },
                Instruction::Copy {
                    dest: Value(1),
                    src: Operand::Const(IrConst::I32(0)),
                },
            ],
            terminator: Terminator::Branch(BlockId(1)),
            source_spans: Vec::new(),
        });
        // B1 header: accumulator phi v27, IV phi v28, cmp Ult(v28, v3).
        func.blocks.push(BasicBlock {
            label: BlockId(1),
            instructions: vec![
                Instruction::Phi {
                    dest: Value(27),
                    ty: IrType::U32,
                    incoming: vec![
                        (Operand::Value(Value(0)), BlockId(0)),
                        (Operand::Value(Value(22)), BlockId(3)),
                    ],
                },
                Instruction::Phi {
                    dest: Value(28),
                    ty: IrType::U32,
                    incoming: vec![
                        (Operand::Value(Value(1)), BlockId(0)),
                        (Operand::Value(Value(24)), BlockId(3)),
                    ],
                },
                Instruction::Cmp {
                    dest: Value(12),
                    op: IrCmpOp::Ult,
                    lhs: Operand::Value(Value(28)),
                    rhs: Operand::Value(Value(3)),
                    ty: IrType::U32,
                },
            ],
            terminator: Terminator::CondBranch {
                cond: Operand::Value(Value(12)),
                true_label: BlockId(2),
                false_label: BlockId(4),
            },
            source_spans: Vec::new(),
        });
        // B2 body: LShr -> zext -> Shl -> GEP -> Load -> accum Add.
        func.blocks.push(BasicBlock {
            label: BlockId(2),
            instructions: vec![
                Instruction::BinOp {
                    dest: Value(15),
                    op: IrBinOp::LShr,
                    lhs: Operand::Value(Value(28)),
                    rhs: Operand::Const(IrConst::I32(1)),
                    ty: IrType::U32,
                },
                Instruction::Cast {
                    dest: Value(16),
                    src: Operand::Value(Value(15)),
                    from_ty: IrType::U32,
                    to_ty: IrType::I64,
                },
                Instruction::BinOp {
                    dest: Value(18),
                    op: IrBinOp::Shl,
                    lhs: Operand::Value(Value(16)),
                    rhs: Operand::Const(IrConst::I64(2)),
                    ty: IrType::I64,
                },
                Instruction::GetElementPtr {
                    dest: Value(19),
                    base: Value(2),
                    offset: Operand::Value(Value(18)),
                    ty: IrType::Ptr,
                },
                Instruction::Load {
                    dest: Value(20),
                    ptr: Value(19),
                    ty: IrType::U32,
                    seg_override: AddressSpace::Default,
                    volatile: false,
                },
                Instruction::BinOp {
                    dest: Value(22),
                    op: IrBinOp::Add,
                    lhs: Operand::Value(Value(27)),
                    rhs: Operand::Value(Value(20)),
                    ty: IrType::U32,
                },
            ],
            terminator: Terminator::Branch(BlockId(3)),
            source_spans: Vec::new(),
        });
        // B3 latch.
        func.blocks.push(BasicBlock {
            label: BlockId(3),
            instructions: vec![Instruction::BinOp {
                dest: Value(24),
                op: IrBinOp::Add,
                lhs: Operand::Value(Value(28)),
                rhs: Operand::Const(IrConst::I32(1)),
                ty: IrType::U32,
            }],
            terminator: Terminator::Branch(BlockId(1)),
            source_spans: Vec::new(),
        });
        // B4 exit: zext the accumulator for the return.
        func.blocks.push(BasicBlock {
            label: BlockId(4),
            instructions: vec![Instruction::Cast {
                dest: Value(30),
                src: Operand::Value(Value(27)),
                from_ty: IrType::U32,
                to_ty: IrType::I64,
            }],
            terminator: Terminator::Return(Some(Operand::Value(Value(30)))),
            source_spans: Vec::new(),
        });
        func.next_value_id = 100;
        func.next_label = 5;
        check(&mut func, 1);
        assert!(matches!(
            &func.blocks[1].instructions[1],
            Instruction::Phi {
                ty: IrType::U64,
                ..
            }
        ));
        // The accumulator phi must stay U32 (it is not part of the closure).
        assert!(matches!(
            &func.blocks[1].instructions[0],
            Instruction::Phi {
                ty: IrType::U32,
                ..
            }
        ));
        // The cross-sign cast survives as a same-size U64→I64 conversion.
        assert!(func.blocks[2].instructions.iter().any(|i| matches!(
            i,
            Instruction::Cast {
                from_ty: IrType::U64,
                to_ty: IrType::I64,
                ..
            }
        )));
    }
    // ---------------------------------------------------------------------
    // Theorem 5 with a stride: `mag > 1` needs a CONSTANT bound with room for
    // the final stride.  Regression anchor for the unrolled counted loop, whose
    // latch an unroller spells as a chain of unit adds.
    // ---------------------------------------------------------------------

    fn strided_loop(bound: Operand, latch_const: i64, name: &str) -> IrFunction {
        let cmp = Instruction::Cmp {
            dest: Value(99),
            op: IrCmpOp::Ult,
            lhs: Operand::Value(Value(1)),
            rhs: bound,
            ty: IrType::U32,
        };
        let body = vec![
            Instruction::Cast {
                dest: Value(10),
                src: Operand::Value(Value(1)),
                from_ty: IrType::U32,
                to_ty: IrType::U64,
            },
            gep_inst(11, 10, IrType::I8),
        ];
        let mut func = counting_loop(name, IrType::U32, IrBinOp::Add, body, Some(cmp));
        func.blocks[3].instructions[0] = Instruction::BinOp {
            dest: Value(5),
            op: IrBinOp::Add,
            lhs: Operand::Value(Value(1)),
            rhs: Operand::Const(IrConst::I64(latch_const)),
            ty: IrType::U32,
        };
        func
    }

    #[test]
    fn strided_counted_loop_with_a_constant_bound_is_widened() {
        // `i += 4` with `i < 262144`: the latch reaches 262144 <= 2^32-4, so no
        // body value and no latch result can wrap.  This is the shape an
        // unrolled counted loop has, and refusing it is what stranded the SIB
        // displacement fold (histogram, +8.03% Ir before this theorem).
        let mut func = strided_loop(Operand::Const(IrConst::I64(262144)), 4, "strided_const");
        check(&mut func, 1);
        assert!(matches!(
            &func.blocks[1].instructions[0],
            Instruction::Phi {
                ty: IrType::U64,
                ..
            }
        ));
    }

    #[test]
    fn strided_counted_loop_with_a_runtime_bound_stays_narrow() {
        // Same stride, but the bound is a runtime loop-invariant: `n-1+4`
        // exceeds 2^32-1 for any `n > 2^32-4`, which no local argument can
        // exclude.  The unit-step theorem is the tightest case that survives a
        // runtime bound, and a stride must not be waved through on its account.
        let mut func = strided_loop(Operand::Value(Value(60)), 4, "strided_runtime");
        func.blocks[0].instructions.push(Instruction::Copy {
            dest: Value(60),
            src: Operand::Const(IrConst::I32(1000)),
        });
        check(&mut func, 0);
        assert!(matches!(
            &func.blocks[1].instructions[0],
            Instruction::Phi {
                ty: IrType::U32,
                ..
            }
        ));
    }

    #[test]
    fn strided_counted_loop_with_no_room_for_the_stride_stays_narrow() {
        // `i += 4` with `i < 0xFFFFFFFF`: the latch result reaches
        // 0x100000002, which wraps the u32 ring.  The constant bound is present
        // and unsigned, so this is the arithmetic obligation itself failing --
        // `n <= max - mag + 1` -- not a missing proof input.
        let mut func = strided_loop(Operand::Const(IrConst::I64(4294967295)), 4, "strided_tight");
        check(&mut func, 0);
        assert!(matches!(
            &func.blocks[1].instructions[0],
            Instruction::Phi {
                ty: IrType::U32,
                ..
            }
        ));
    }

    #[test]
    fn unit_step_with_a_runtime_bound_still_widens() {
        // Control for the two refusals above: the original theorem must not have
        // been narrowed by the stride work.  `i += 1` with a runtime bound is
        // provable because the latch reaches at most `max`.
        let mut func = strided_loop(Operand::Value(Value(60)), 1, "unit_runtime");
        func.blocks[0].instructions.push(Instruction::Copy {
            dest: Value(60),
            src: Operand::Const(IrConst::I32(1000)),
        });
        check(&mut func, 1);
    }

    #[test]
    fn chained_latch_is_canonicalised_and_then_widened() {
        // The unrolled shape exactly: the latch is `v6=v1+1; v7=v6+1; v8=v7+1;
        // v5=v8+1`, and `v6` is ALSO an addressing offset the body needs, so no
        // value-numbering pass will re-associate the chain (it would add an
        // instruction rather than remove one).  Recognising the recurrence
        // requires re-spelling the final link as `v5 = v1 + 4`, which is
        // value-identical by associativity of modular addition.
        let cmp = Instruction::Cmp {
            dest: Value(99),
            op: IrCmpOp::Ult,
            lhs: Operand::Value(Value(1)),
            rhs: Operand::Const(IrConst::I64(262144)),
            ty: IrType::U32,
        };
        let body = vec![
            Instruction::Cast {
                dest: Value(10),
                src: Operand::Value(Value(1)),
                from_ty: IrType::U32,
                to_ty: IrType::U64,
            },
            gep_inst(11, 10, IrType::I8),
        ];
        let mut func = counting_loop("chained", IrType::U32, IrBinOp::Add, body, Some(cmp));
        let link = |dest: i64, src: i64| Instruction::BinOp {
            dest: Value(dest as u32),
            op: IrBinOp::Add,
            lhs: Operand::Value(Value(src as u32)),
            rhs: Operand::Const(IrConst::I64(1)),
            ty: IrType::U32,
        };
        // One block holds both the chain and the offset use of an intermediate,
        // which is the real unrolled shape (the body block IS the latch block):
        // a link defined in the latch cannot be used from the body, so the
        // fixture would be invalid SSA if the second address lived there.
        func.blocks[3].instructions = vec![
            link(6, 1),
            link(7, 6),
            link(8, 7),
            link(5, 8),
            Instruction::Cast {
                dest: Value(12),
                src: Operand::Value(Value(6)),
                from_ty: IrType::U32,
                to_ty: IrType::U64,
            },
            gep_inst(13, 12, IrType::I8),
        ];
        check(&mut func, 1);
        // The recurrence must have been canonicalised to a single stride, and
        // then widened: the latch add reads the phi directly with step 4.
        let latch_add = func.blocks[3]
            .instructions
            .iter()
            .find(|i| matches!(i, Instruction::BinOp { dest, .. } if *dest == Value(5)))
            .expect("latch add survived");
        match latch_add {
            Instruction::BinOp { lhs, rhs, ty, .. } => {
                assert!(
                    matches!(lhs, Operand::Value(Value(1))),
                    "latch still does not name the phi directly: {latch_add:?}"
                );
                assert!(
                    matches!(rhs, Operand::Const(c) if c.to_i64() == Some(4)),
                    "latch stride was not collapsed to the total: {latch_add:?}"
                );
                assert!(matches!(ty, IrType::U64), "latch not widened: {ty:?}");
            }
            _ => unreachable!(),
        }
    }

    // -----------------------------------------------------------------------
    // Theorem 5′ battery: exposed-latch trajectory gates, the intercept, the
    // label/index Obligation-P pin and the latch-twin canonicalisation.
    // -----------------------------------------------------------------------

    /// Preheader instructions defining `Value(0)` as a RUNTIME seed: a
    /// volatile load (the `ung1` shape — no constant resolution possible).
    fn volatile_seed_insts() -> Vec<Instruction> {
        vec![
            Instruction::GlobalAddr {
                dest: Value(60),
                name: "v_seed".to_string(),
            },
            Instruction::Load {
                dest: Value(0),
                ptr: Value(60),
                ty: IrType::U32,
                seg_override: AddressSpace::Default,
                volatile: true,
            },
        ]
    }

    /// Preheader instructions defining `Value(0)` as a CONST seed through the
    /// single `Copy{src: Const}` level mem2reg leaves for `uint32_t i = c;`.
    fn const_seed_insts(v: i64) -> Vec<Instruction> {
        vec![Instruction::Copy {
            dest: Value(0),
            src: Operand::Const(IrConst::I64(v)),
        }]
    }

    /// Preheader instructions defining `Value(17)` as a runtime loop-invariant
    /// cmp bound (volatile load, hoisted by construction).
    fn volatile_bound_insts() -> Vec<Instruction> {
        vec![
            Instruction::GlobalAddr {
                dest: Value(61),
                name: "v_bound".to_string(),
            },
            Instruction::Load {
                dest: Value(17),
                ptr: Value(61),
                ty: IrType::U32,
                seg_override: AddressSpace::Default,
                volatile: true,
            },
        ]
    }

    /// Rotated do-while (self-loop) builder: B0 preheader (seed + invariant
    /// defs + the GEP base), B1 = header == latch (phi, body insts, latch
    /// step defining `Value(5)`, guard cmp `Value(99)` reading the phi),
    /// B2 exit. LABELS deliberately diverge from indices (label = index + 7),
    /// so every fixture built here also exercises obligation P's label→index
    /// resolution — the pre-fix P rewrite compared the domains directly and
    /// failed every well-formed loop in real frontend IR (fixtures with
    /// label == index cannot catch that class).
    fn rotated_loop(
        name: &str,
        phi_ty: IrType,
        mut seed_insts: Vec<Instruction>,
        body_insts: Vec<Instruction>,
        latch_inst: Instruction,
        cmp: Instruction,
        exit_insts: Vec<Instruction>,
    ) -> IrFunction {
        let mut func = IrFunction::new(name.to_string(), IrType::I32, vec![], false);
        seed_insts.push(Instruction::GlobalAddr {
            dest: Value(50),
            name: "arr".to_string(),
        });
        func.blocks.push(BasicBlock {
            label: BlockId(7),
            instructions: seed_insts,
            terminator: Terminator::Branch(BlockId(8)),
            source_spans: Vec::new(),
        });
        let mut head = vec![Instruction::Phi {
            dest: Value(1),
            ty: phi_ty,
            incoming: vec![
                (Operand::Value(Value(0)), BlockId(7)),
                (Operand::Value(Value(5)), BlockId(8)),
            ],
        }];
        head.extend(body_insts);
        head.push(latch_inst);
        head.push(cmp);
        func.blocks.push(BasicBlock {
            label: BlockId(8),
            instructions: head,
            terminator: Terminator::CondBranch {
                cond: Operand::Value(Value(99)),
                true_label: BlockId(8),
                false_label: BlockId(9),
            },
            source_spans: Vec::new(),
        });
        func.blocks.push(BasicBlock {
            label: BlockId(9),
            instructions: exit_insts,
            terminator: Terminator::Return(None),
            source_spans: Vec::new(),
        });
        func.next_value_id = 100;
        func.next_label = 10;
        func
    }

    /// U32 latch `Value(5) = phi op const` in the rotated builder's spelling.
    fn rotated_latch(op: IrBinOp, k: i64) -> Instruction {
        Instruction::BinOp {
            dest: Value(5),
            op,
            lhs: Operand::Value(Value(1)),
            rhs: Operand::Const(IrConst::I64(k)),
            ty: IrType::U32,
        }
    }

    /// IVW-RANGE-1 (case A of tests/regression/ivwiden_exposed_latch.c):
    /// rotated do-while at a runtime ring-top seed whose latch value escapes
    /// the loop through a Cast feeding a GEP. The exiting pass evaluates the
    /// latch `phi+1` at the untested seed: narrow wraps to 0 where wide does
    /// not, and the pre-fix pass let the GEP keep the wide value (read
    /// `buf[2^32]` instead of `buf[0]` — 99 vs gcc's 11). Theorem 5′:
    /// exposed latch + runtime seed ⇒ intercept; the Cast of the divergent
    /// latch member is the documented Cast-bail ⇒ decline. The in-loop
    /// bit-exact `& 63` read alone would have been admissible.
    #[test]
    fn test_rotated_escape_exit_cast_declines() {
        let mut seed = volatile_seed_insts();
        seed.extend(volatile_bound_insts());
        let body = vec![
            Instruction::BinOp {
                dest: Value(8),
                op: IrBinOp::And,
                lhs: Operand::Value(Value(1)),
                rhs: Operand::Const(IrConst::I64(63)),
                ty: IrType::U32,
            },
            Instruction::Cast {
                dest: Value(9),
                src: Operand::Value(Value(8)),
                from_ty: IrType::U32,
                to_ty: IrType::I64,
            },
            gep_inst(11, 9, IrType::I8),
        ];
        let cmp = Instruction::Cmp {
            dest: Value(99),
            op: IrCmpOp::Ult,
            lhs: Operand::Value(Value(1)),
            rhs: Operand::Value(Value(17)),
            ty: IrType::U32,
        };
        let exit = vec![
            Instruction::Cast {
                dest: Value(20),
                src: Operand::Value(Value(5)),
                from_ty: IrType::U32,
                to_ty: IrType::I64,
            },
            gep_inst(21, 20, IrType::I8),
        ];
        let mut func = rotated_loop(
            "rot_escape",
            IrType::U32,
            seed,
            body,
            rotated_latch(IrBinOp::Add, 1),
            cmp,
            exit,
        );
        check(&mut func, 0);
        assert!(matches!(
            &func.blocks[1].instructions[0],
            Instruction::Phi {
                ty: IrType::U32,
                ..
            }
        ));
    }

    /// Case B of the executed regression (`a[(i + 1) & m]` + `while (i++ <
    /// n)`): the frontend emits TWO value-identical `Add(phi, 1)`s because
    /// iv_widen runs ahead of the merging CSE. `canonicalise_latch_twin`
    /// redirects the back edge to the earlier twin and deletes the dead
    /// original, so the body read becomes a use of the LATCH member and goes
    /// through the intercept's unconditional bit-exact And arm
    /// (`x & c ∈ [0, c]` for EVERY unsigned x — the LatchResult member
    /// legitimately has no input range). The plan widens; the guard cmp reads
    /// a divergent-capable member, so the intercept forces the Trunc action
    /// (the narrow bits ARE the narrow value — the guarded trajectory, and
    /// with it the counted proof, survives the widening).
    #[test]
    fn test_and_const_widens() {
        let mut seed = volatile_seed_insts();
        seed.extend(volatile_bound_insts());
        let body = vec![
            // The twin: value-identical to the latch step, EARLIER in the
            // same block.
            Instruction::BinOp {
                dest: Value(10),
                op: IrBinOp::Add,
                lhs: Operand::Value(Value(1)),
                rhs: Operand::Const(IrConst::I64(1)),
                ty: IrType::U32,
            },
            Instruction::BinOp {
                dest: Value(13),
                op: IrBinOp::And,
                lhs: Operand::Value(Value(10)),
                rhs: Operand::Const(IrConst::I64(63)),
                ty: IrType::U32,
            },
            Instruction::Cast {
                dest: Value(14),
                src: Operand::Value(Value(13)),
                from_ty: IrType::U32,
                to_ty: IrType::I64,
            },
            gep_inst(15, 14, IrType::I8),
        ];
        let cmp = Instruction::Cmp {
            dest: Value(99),
            op: IrCmpOp::Ult,
            lhs: Operand::Value(Value(1)),
            rhs: Operand::Value(Value(17)),
            ty: IrType::U32,
        };
        let mut func = rotated_loop(
            "twin_and",
            IrType::U32,
            seed,
            body,
            rotated_latch(IrBinOp::Add, 1),
            cmp,
            vec![],
        );
        check(&mut func, 1);
        // The phi, the surviving twin and the And are wide.
        assert!(matches!(
            &func.blocks[1].instructions[0],
            Instruction::Phi {
                ty: IrType::U64,
                ..
            }
        ));
        assert!(func.blocks[1].instructions.iter().any(|i| matches!(
            i,
            Instruction::BinOp {
                dest: Value(10),
                ty: IrType::U64,
                ..
            }
        )));
        assert!(func.blocks[1].instructions.iter().any(|i| matches!(
            i,
            Instruction::BinOp {
                dest: Value(13),
                op: IrBinOp::And,
                ty: IrType::U64,
                ..
            }
        )));
        // The dead original latch step was deleted...
        assert!(
            !func.blocks[1]
                .instructions
                .iter()
                .any(|i| i.dest() == Some(Value(5))),
            "the duplicated latch step must be deleted after the redirect"
        );
        // ...and the back edge now reads the twin.
        assert!(matches!(
            &func.blocks[1].instructions[0],
            Instruction::Phi { incoming, .. } if incoming.iter().any(|(op, _)| matches!(op, Operand::Value(v) if *v == Value(10)))
        ));
        // The guard cmp stayed NARROW (intercept-forced Trunc) with a
        // truncation feeding it.
        let cmp_inst = func.blocks[1]
            .instructions
            .iter()
            .find(|i| {
                matches!(
                    i,
                    Instruction::Cmp {
                        dest: Value(99),
                        ..
                    }
                )
            })
            .unwrap();
        assert!(matches!(
            cmp_inst,
            Instruction::Cmp {
                ty: IrType::U32,
                op: IrCmpOp::Ult,
                ..
            }
        ));
        assert!(
            func.blocks[1].instructions.iter().any(|i| matches!(
                i,
                Instruction::Cast {
                    to_ty: IrType::U32,
                    ..
                }
            )),
            "a truncation must feed the narrow guard cmp"
        );
    }

    /// Case C of the executed regression: exposed latch (cmp in the latch
    /// block), RUNTIME ring-top seed, CONST bound, and a body twin
    /// `Add(phi, 2)` feeding the addressing chain. The seed-inclusive fold is
    /// `[0, max] + 2`, which leaves U32 — the twin cannot be admitted and the
    /// plan declines (pre-fix this widened and read `buf[2^32+1]` = 91
    /// instead of `buf[1]` = 21). The stride-2 twin is NOT value-identical to
    /// the unit latch step, so the twin canonicalisation must not fire.
    #[test]
    fn test_exposed_runtime_seed_fold_overflows_and_declines() {
        let body = vec![
            Instruction::BinOp {
                dest: Value(10),
                op: IrBinOp::Add,
                lhs: Operand::Value(Value(1)),
                rhs: Operand::Const(IrConst::I64(2)),
                ty: IrType::U32,
            },
            Instruction::Cast {
                dest: Value(11),
                src: Operand::Value(Value(10)),
                from_ty: IrType::U32,
                to_ty: IrType::I64,
            },
            gep_inst(12, 11, IrType::I8),
        ];
        let cmp = Instruction::Cmp {
            dest: Value(99),
            op: IrCmpOp::Ult,
            lhs: Operand::Value(Value(1)),
            rhs: Operand::Const(IrConst::I64(8)),
            ty: IrType::U32,
        };
        let mut func = rotated_loop(
            "exposed_runtime",
            IrType::U32,
            volatile_seed_insts(),
            body,
            rotated_latch(IrBinOp::Add, 1),
            cmp,
            vec![],
        );
        check(&mut func, 0);
        assert!(matches!(
            &func.blocks[1].instructions[0],
            Instruction::Phi {
                ty: IrType::U32,
                ..
            }
        ));
    }

    /// The const-seed half of the seed-inclusive fold ("selfconst"): same
    /// exposed shape as the decline above, but the seed resolves through the
    /// `Copy{src: Const}` level mem2reg leaves. The fold becomes
    /// `[0, max(0, 8-1+1)] = [0, 8]`, the twin `+2` stays inside U32
    /// (`[2, 10]`), the exiting latch step closes (`8 + 1 <= max`) and the
    /// intercept stays OFF: the loop widens.
    #[test]
    fn test_exposed_const_seed_folds_and_widens() {
        let body = vec![
            Instruction::BinOp {
                dest: Value(10),
                op: IrBinOp::Add,
                lhs: Operand::Value(Value(1)),
                rhs: Operand::Const(IrConst::I64(2)),
                ty: IrType::U32,
            },
            Instruction::Cast {
                dest: Value(11),
                src: Operand::Value(Value(10)),
                from_ty: IrType::U32,
                to_ty: IrType::I64,
            },
            gep_inst(12, 11, IrType::I8),
        ];
        let cmp = Instruction::Cmp {
            dest: Value(99),
            op: IrCmpOp::Ult,
            lhs: Operand::Value(Value(1)),
            rhs: Operand::Const(IrConst::I64(8)),
            ty: IrType::U32,
        };
        let mut func = rotated_loop(
            "exposed_const",
            IrType::U32,
            const_seed_insts(0),
            body,
            rotated_latch(IrBinOp::Add, 1),
            cmp,
            vec![],
        );
        check(&mut func, 1);
        assert!(matches!(
            &func.blocks[1].instructions[0],
            Instruction::Phi {
                ty: IrType::U64,
                ..
            }
        ));
        // The twin member widened with its constant.
        assert!(func.blocks[1].instructions.iter().any(|i| matches!(
            i,
            Instruction::BinOp {
                dest: Value(10),
                ty: IrType::U64,
                rhs: Operand::Const(IrConst::I64(2)),
                ..
            }
        )));
    }

    /// Exit-on-TRUE branch polarity over a stride-2 recurrence
    /// (`for (;;) { use(i & 3); if (i > 6) break; i += 2; }`): the cmp sits
    /// in a non-header block and the EXIT is the true arm, so the
    /// continue-condition is the negation (`i <= 6` ⇔ `i < 7`). The
    /// normalised threshold discharges the room obligation (`7 <= max-1`),
    /// the const seed folds seed-inclusively to `[0, max(0, 7-1+2)] = [0, 8]`
    /// (the trajectory's body values are {0,2,4,6,8} and the exiting latch
    /// reaches 10 — both covered, `8+2 <= max` closes the wrap), and the
    /// loop widens. Pre-theorem-5′ this shape found no bound at all (the
    /// pass matched only continue-on-true spellings).
    #[test]
    fn test_exit_on_true_polarity() {
        let mut func = IrFunction::new("exit_true".to_string(), IrType::I32, vec![], false);
        // Labels diverge from indices (+10) — obligation P coverage.
        func.blocks.push(BasicBlock {
            label: BlockId(10),
            instructions: vec![
                Instruction::Copy {
                    dest: Value(0),
                    src: Operand::Const(IrConst::I64(0)),
                },
                Instruction::GlobalAddr {
                    dest: Value(50),
                    name: "arr".to_string(),
                },
            ],
            terminator: Terminator::Branch(BlockId(11)),
            source_spans: Vec::new(),
        });
        func.blocks.push(BasicBlock {
            label: BlockId(11),
            instructions: vec![Instruction::Phi {
                dest: Value(1),
                ty: IrType::U32,
                incoming: vec![
                    (Operand::Value(Value(0)), BlockId(10)),
                    (Operand::Value(Value(5)), BlockId(14)),
                ],
            }],
            terminator: Terminator::Branch(BlockId(12)),
            source_spans: Vec::new(),
        });
        func.blocks.push(BasicBlock {
            label: BlockId(12),
            instructions: vec![
                Instruction::BinOp {
                    dest: Value(10),
                    op: IrBinOp::And,
                    lhs: Operand::Value(Value(1)),
                    rhs: Operand::Const(IrConst::I64(3)),
                    ty: IrType::U32,
                },
                Instruction::Cast {
                    dest: Value(11),
                    src: Operand::Value(Value(10)),
                    from_ty: IrType::U32,
                    to_ty: IrType::I64,
                },
                gep_inst(12, 11, IrType::I8),
            ],
            terminator: Terminator::Branch(BlockId(13)),
            source_spans: Vec::new(),
        });
        func.blocks.push(BasicBlock {
            label: BlockId(13),
            instructions: vec![Instruction::Cmp {
                dest: Value(99),
                op: IrCmpOp::Ugt,
                lhs: Operand::Value(Value(1)),
                rhs: Operand::Const(IrConst::I64(6)),
                ty: IrType::U32,
            }],
            // EXIT ON TRUE: the true arm leaves the body.
            terminator: Terminator::CondBranch {
                cond: Operand::Value(Value(99)),
                true_label: BlockId(15),
                false_label: BlockId(14),
            },
            source_spans: Vec::new(),
        });
        func.blocks.push(BasicBlock {
            label: BlockId(14),
            instructions: vec![Instruction::BinOp {
                dest: Value(5),
                op: IrBinOp::Add,
                lhs: Operand::Value(Value(1)),
                rhs: Operand::Const(IrConst::I64(2)),
                ty: IrType::U32,
            }],
            terminator: Terminator::Branch(BlockId(11)),
            source_spans: Vec::new(),
        });
        func.blocks.push(BasicBlock {
            label: BlockId(15),
            instructions: vec![],
            terminator: Terminator::Return(None),
            source_spans: Vec::new(),
        });
        func.next_value_id = 100;
        func.next_label = 16;
        check(&mut func, 1);
        assert!(matches!(
            &func.blocks[1].instructions[0],
            Instruction::Phi {
                ty: IrType::U64,
                ..
            }
        ));
        // The cmp widened (const bound, unsigned predicate — the normal
        // action once the polarity normalisation found the bound).
        let cmp_inst = func.blocks[3]
            .instructions
            .iter()
            .find(|i| matches!(i, Instruction::Cmp { .. }))
            .unwrap();
        assert!(matches!(
            cmp_inst,
            Instruction::Cmp {
                ty: IrType::U64,
                op: IrCmpOp::Ugt,
                ..
            }
        ));
    }

    /// Obligation P pin (documented name): the strided guarded counted loop
    /// with EVERY label renumbered +5, so labels and indices diverge in every
    /// block, terminator and phi incoming. The pre-fix P rewrite compared
    /// branch-target labels against the index-based loop body directly and
    /// declined every well-formed loop in real frontend IR (live repro: body
    /// index 3 under label 5). Resolving through the label→index map keeps
    /// the widening.
    #[test]
    fn test_p_edge_resolves_block_labels_not_indices() {
        let mut func = strided_loop(Operand::Const(IrConst::I64(262144)), 4, "p_labels");
        for b in &mut func.blocks {
            b.label = BlockId(b.label.0 + 5);
            match &mut b.terminator {
                Terminator::Branch(l) => *l = BlockId(l.0 + 5),
                Terminator::CondBranch {
                    true_label,
                    false_label,
                    ..
                } => {
                    *true_label = BlockId(true_label.0 + 5);
                    *false_label = BlockId(false_label.0 + 5);
                }
                _ => {}
            }
            for inst in &mut b.instructions {
                if let Instruction::Phi { incoming, .. } = inst {
                    for (_, blk) in incoming.iter_mut() {
                        *blk = BlockId(blk.0 + 5);
                    }
                }
            }
        }
        func.next_label += 5;
        check(&mut func, 1);
        assert!(matches!(
            &func.blocks[1].instructions[0],
            Instruction::Phi {
                ty: IrType::U64,
                ..
            }
        ));
    }

    /// Trajectory gate: a latch with an in-body successor other than the
    /// header sits on an inner cycle — the recurrence steps without
    /// re-passing the exit test and the IV can outrun any proven bound.
    /// Without the gate this loop's clean guarded header cmp would prove
    /// `[0, 63]` and widen.
    #[test]
    fn test_latch_on_inner_cycle_declines() {
        let mut func = IrFunction::new("inner_cycle".to_string(), IrType::I32, vec![], false);
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![
                Instruction::Copy {
                    dest: Value(0),
                    src: Operand::Const(IrConst::I64(0)),
                },
                Instruction::GlobalAddr {
                    dest: Value(50),
                    name: "arr".to_string(),
                },
            ],
            terminator: Terminator::Branch(BlockId(1)),
            source_spans: Vec::new(),
        });
        func.blocks.push(BasicBlock {
            label: BlockId(1),
            instructions: vec![
                Instruction::Phi {
                    dest: Value(1),
                    ty: IrType::U32,
                    incoming: vec![
                        (Operand::Value(Value(0)), BlockId(0)),
                        (Operand::Value(Value(5)), BlockId(3)),
                    ],
                },
                Instruction::Cmp {
                    dest: Value(99),
                    op: IrCmpOp::Ult,
                    lhs: Operand::Value(Value(1)),
                    rhs: Operand::Const(IrConst::I64(64)),
                    ty: IrType::U32,
                },
            ],
            terminator: Terminator::CondBranch {
                cond: Operand::Value(Value(99)),
                true_label: BlockId(2),
                false_label: BlockId(5),
            },
            source_spans: Vec::new(),
        });
        func.blocks.push(BasicBlock {
            label: BlockId(2),
            instructions: vec![
                Instruction::Cast {
                    dest: Value(10),
                    src: Operand::Value(Value(1)),
                    from_ty: IrType::U32,
                    to_ty: IrType::U64,
                },
                gep_inst(11, 10, IrType::I8),
            ],
            terminator: Terminator::Branch(BlockId(3)),
            source_spans: Vec::new(),
        });
        // The latch: steps the IV, then re-enters the body (block 4) on a
        // secondary condition instead of branching straight to the header.
        func.blocks.push(BasicBlock {
            label: BlockId(3),
            instructions: vec![
                Instruction::BinOp {
                    dest: Value(5),
                    op: IrBinOp::Add,
                    lhs: Operand::Value(Value(1)),
                    rhs: Operand::Const(IrConst::I64(1)),
                    ty: IrType::U32,
                },
                Instruction::Cmp {
                    dest: Value(98),
                    op: IrCmpOp::Ult,
                    lhs: Operand::Value(Value(5)),
                    rhs: Operand::Const(IrConst::I64(100)),
                    ty: IrType::U32,
                },
            ],
            terminator: Terminator::CondBranch {
                cond: Operand::Value(Value(98)),
                true_label: BlockId(4),
                false_label: BlockId(1),
            },
            source_spans: Vec::new(),
        });
        func.blocks.push(BasicBlock {
            label: BlockId(4),
            instructions: vec![],
            terminator: Terminator::Branch(BlockId(3)),
            source_spans: Vec::new(),
        });
        func.blocks.push(BasicBlock {
            label: BlockId(5),
            instructions: vec![],
            terminator: Terminator::Return(None),
            source_spans: Vec::new(),
        });
        func.next_value_id = 100;
        func.next_label = 6;
        check(&mut func, 0);
        assert!(matches!(
            &func.blocks[1].instructions[0],
            Instruction::Phi {
                ty: IrType::U32,
                ..
            }
        ));
    }

    /// Trajectory gate: an outside edge into the middle of the body (the
    /// exit block jumps into B2) runs body work on passes no header test
    /// gated — the strict range argument is invalid and the proof declines,
    /// even though the header cmp itself is clean and correctly positioned.
    #[test]
    fn test_body_entry_not_closed_declines() {
        let cmp = Instruction::Cmp {
            dest: Value(99),
            op: IrCmpOp::Ult,
            lhs: Operand::Value(Value(1)),
            rhs: Operand::Const(IrConst::I64(64)),
            ty: IrType::U32,
        };
        let body = vec![
            Instruction::Cast {
                dest: Value(10),
                src: Operand::Value(Value(1)),
                from_ty: IrType::U32,
                to_ty: IrType::U64,
            },
            gep_inst(11, 10, IrType::I8),
        ];
        let mut func = counting_loop("entry_open", IrType::U32, IrBinOp::Add, body, Some(cmp));
        func.blocks[3].instructions[0] = Instruction::BinOp {
            dest: Value(5),
            op: IrBinOp::Add,
            lhs: Operand::Value(Value(1)),
            rhs: Operand::Const(IrConst::I64(1)),
            ty: IrType::U32,
        };
        // The exit block re-enters the BODY block directly (irregular entry),
        // then falls through to a new return block.
        func.blocks[4].terminator = Terminator::Branch(BlockId(2));
        func.blocks.push(BasicBlock {
            label: BlockId(5),
            instructions: vec![],
            terminator: Terminator::Return(None),
            source_spans: Vec::new(),
        });
        func.next_label = 6;
        check(&mut func, 0);
        assert!(matches!(
            &func.blocks[1].instructions[0],
            Instruction::Phi {
                ty: IrType::U32,
                ..
            }
        ));
    }

    /// The gates must not over-refuse: an outer guarded counted loop whose
    /// body CONTAINS a well-formed inner loop still widens (the inner
    /// blocks are entered only from inside the outer body, and the outer
    /// latch's only in-body successor is the outer header). The inner IV has
    /// no addressing use and stays narrow on its own merits.
    #[test]
    fn test_nested_inner_loop_guarded_outer_widens() {
        let mut func = IrFunction::new("nested".to_string(), IrType::I32, vec![], false);
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![
                Instruction::Copy {
                    dest: Value(0),
                    src: Operand::Const(IrConst::I64(0)),
                },
                Instruction::GlobalAddr {
                    dest: Value(50),
                    name: "arr".to_string(),
                },
            ],
            terminator: Terminator::Branch(BlockId(1)),
            source_spans: Vec::new(),
        });
        // Outer header: phi + guard cmp (clean).
        func.blocks.push(BasicBlock {
            label: BlockId(1),
            instructions: vec![
                Instruction::Phi {
                    dest: Value(1),
                    ty: IrType::U32,
                    incoming: vec![
                        (Operand::Value(Value(0)), BlockId(0)),
                        (Operand::Value(Value(5)), BlockId(3)),
                    ],
                },
                Instruction::Cmp {
                    dest: Value(99),
                    op: IrCmpOp::Ult,
                    lhs: Operand::Value(Value(1)),
                    rhs: Operand::Const(IrConst::I64(64)),
                    ty: IrType::U32,
                },
            ],
            terminator: Terminator::CondBranch {
                cond: Operand::Value(Value(99)),
                true_label: BlockId(2),
                false_label: BlockId(7),
            },
            source_spans: Vec::new(),
        });
        // Outer body: addressing read of the outer IV, then the inner loop.
        func.blocks.push(BasicBlock {
            label: BlockId(2),
            instructions: vec![
                Instruction::Cast {
                    dest: Value(10),
                    src: Operand::Value(Value(1)),
                    from_ty: IrType::U32,
                    to_ty: IrType::U64,
                },
                gep_inst(11, 10, IrType::I8),
            ],
            terminator: Terminator::Branch(BlockId(5)),
            source_spans: Vec::new(),
        });
        // Outer latch.
        func.blocks.push(BasicBlock {
            label: BlockId(3),
            instructions: vec![Instruction::BinOp {
                dest: Value(5),
                op: IrBinOp::Add,
                lhs: Operand::Value(Value(1)),
                rhs: Operand::Const(IrConst::I64(1)),
                ty: IrType::U32,
            }],
            terminator: Terminator::Branch(BlockId(1)),
            source_spans: Vec::new(),
        });
        // Inner header (index 4, LABEL 5 — the label/index gap doubles as
        // obligation-P coverage for the inner loop's edges).
        func.blocks.push(BasicBlock {
            label: BlockId(5),
            instructions: vec![
                Instruction::Phi {
                    dest: Value(32),
                    ty: IrType::U32,
                    incoming: vec![
                        (Operand::Const(IrConst::I64(0)), BlockId(2)),
                        (Operand::Value(Value(33)), BlockId(6)),
                    ],
                },
                Instruction::Cmp {
                    dest: Value(96),
                    op: IrCmpOp::Ult,
                    lhs: Operand::Value(Value(32)),
                    rhs: Operand::Const(IrConst::I64(16)),
                    ty: IrType::U32,
                },
            ],
            terminator: Terminator::CondBranch {
                cond: Operand::Value(Value(96)),
                true_label: BlockId(6),
                false_label: BlockId(3),
            },
            source_spans: Vec::new(),
        });
        func.blocks.push(BasicBlock {
            label: BlockId(6),
            instructions: vec![Instruction::BinOp {
                dest: Value(33),
                op: IrBinOp::Add,
                lhs: Operand::Value(Value(32)),
                rhs: Operand::Const(IrConst::I64(1)),
                ty: IrType::U32,
            }],
            terminator: Terminator::Branch(BlockId(5)),
            source_spans: Vec::new(),
        });
        func.blocks.push(BasicBlock {
            label: BlockId(7),
            instructions: vec![],
            terminator: Terminator::Return(None),
            source_spans: Vec::new(),
        });
        func.next_value_id = 100;
        func.next_label = 8;
        check(&mut func, 1);
        // The OUTER IV widened; the inner IV (no addressing use) stayed.
        assert!(matches!(
            &func.blocks[1].instructions[0],
            Instruction::Phi {
                ty: IrType::U64,
                ..
            }
        ));
        assert!(matches!(
            &func.blocks[4].instructions[0],
            Instruction::Phi {
                ty: IrType::U32,
                ..
            }
        ));
    }

    /// The twin canonicalisation's liveness guard: when the latch value has
    /// a use beyond the phi's back-edge incoming (here a loop-exit Cast),
    /// the redirect must NOT happen — deleting the original would strand
    /// that use. The signed plan still widens through both steps (the UB
    /// theorem needs no trajectory), with the original latch intact.
    #[test]
    fn test_twin_with_other_uses_is_not_canonicalised() {
        let mut func = IrFunction::new("twin_live".to_string(), IrType::I32, vec![], false);
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![
                Instruction::Copy {
                    dest: Value(0),
                    src: Operand::Const(IrConst::I32(3)),
                },
                Instruction::GlobalAddr {
                    dest: Value(50),
                    name: "arr".to_string(),
                },
            ],
            terminator: Terminator::Branch(BlockId(1)),
            source_spans: Vec::new(),
        });
        func.blocks.push(BasicBlock {
            label: BlockId(1),
            instructions: vec![
                Instruction::Phi {
                    dest: Value(1),
                    ty: IrType::I32,
                    incoming: vec![
                        (Operand::Value(Value(0)), BlockId(0)),
                        (Operand::Value(Value(5)), BlockId(1)),
                    ],
                },
                // The twin (earlier in the block).
                Instruction::BinOp {
                    dest: Value(10),
                    op: IrBinOp::Add,
                    lhs: Operand::Value(Value(1)),
                    rhs: Operand::Const(IrConst::I32(1)),
                    ty: IrType::I32,
                },
                Instruction::Cast {
                    dest: Value(11),
                    src: Operand::Value(Value(10)),
                    from_ty: IrType::I32,
                    to_ty: IrType::I64,
                },
                gep_inst(12, 11, IrType::I8),
                // The latch step: value-identical, but with an exit use.
                Instruction::BinOp {
                    dest: Value(5),
                    op: IrBinOp::Add,
                    lhs: Operand::Value(Value(1)),
                    rhs: Operand::Const(IrConst::I32(1)),
                    ty: IrType::I32,
                },
                Instruction::Cmp {
                    dest: Value(99),
                    op: IrCmpOp::Slt,
                    lhs: Operand::Value(Value(1)),
                    rhs: Operand::Const(IrConst::I32(64)),
                    ty: IrType::I32,
                },
            ],
            terminator: Terminator::CondBranch {
                cond: Operand::Value(Value(99)),
                true_label: BlockId(1),
                false_label: BlockId(2),
            },
            source_spans: Vec::new(),
        });
        func.blocks.push(BasicBlock {
            label: BlockId(2),
            instructions: vec![
                Instruction::Cast {
                    dest: Value(20),
                    src: Operand::Value(Value(5)),
                    from_ty: IrType::I32,
                    to_ty: IrType::I64,
                },
                gep_inst(21, 20, IrType::I8),
            ],
            terminator: Terminator::Return(None),
            source_spans: Vec::new(),
        });
        func.next_value_id = 100;
        func.next_label = 3;
        check(&mut func, 1);
        // The original latch step survived (it has an exit-block use)...
        assert!(
            func.blocks[1]
                .instructions
                .iter()
                .any(|i| i.dest() == Some(Value(5))),
            "the latch step with other uses must not be deleted"
        );
        // ...and the back edge still names it.
        assert!(matches!(
            &func.blocks[1].instructions[0],
            Instruction::Phi { incoming, .. } if incoming.iter().any(|(op, _)| matches!(op, Operand::Value(v) if *v == Value(5)))
        ));
    }

    /// The intercept's GEP-offset decline in isolation: a GEP reading a
    /// divergent member directly as its offset declines the plan (a wide
    /// address computed from a value the narrow program wrapped is a
    /// different address), while the IDENTICAL shape behind a phi-guarded
    /// header (strict range, intercept off) widens — the decline is
    /// intercept-specific, not a new GEP rule.
    #[test]
    fn test_gep_offset_on_exposed_member_declines() {
        let mut seed = volatile_seed_insts();
        seed.extend(volatile_bound_insts());
        let cmp = Instruction::Cmp {
            dest: Value(99),
            op: IrCmpOp::Ult,
            lhs: Operand::Value(Value(1)),
            rhs: Operand::Value(Value(17)),
            ty: IrType::U32,
        };
        // Exposed: the GEP offset reads the phi itself.
        let mut func = rotated_loop(
            "gep_exposed",
            IrType::U32,
            seed,
            vec![gep_inst(11, 1, IrType::I8)],
            rotated_latch(IrBinOp::Add, 1),
            cmp,
            vec![],
        );
        check(&mut func, 0);

        // Guarded control: counting_loop puts the cmp in a clean header
        // (with a const bound — the runtime-bound spelling has no v17 def
        // in that scaffold).
        let cmp_guarded = Instruction::Cmp {
            dest: Value(99),
            op: IrCmpOp::Ult,
            lhs: Operand::Value(Value(1)),
            rhs: Operand::Const(IrConst::I64(64)),
            ty: IrType::U32,
        };
        let mut guarded = counting_loop(
            "gep_guarded",
            IrType::U32,
            IrBinOp::Add,
            vec![gep_inst(11, 1, IrType::I8)],
            Some(cmp_guarded),
        );
        guarded.blocks[3].instructions[0] = Instruction::BinOp {
            dest: Value(5),
            op: IrBinOp::Add,
            lhs: Operand::Value(Value(1)),
            rhs: Operand::Const(IrConst::I64(1)),
            ty: IrType::U32,
        };
        check(&mut guarded, 1);
    }

    /// The decrementing mirror of the exposed const-seed fold: a rotated
    /// `do { use(i & 7); } while (i-- > 4)` at const seed 10 proves the
    /// seed-inclusive lower range `[min(10, 4+1-1), max] = [4, max]`, the
    /// exiting latch step closes (`4 - 1 >= 0`), the intercept stays off and
    /// the loop widens.
    #[test]
    fn test_exposed_decrement_lower_bound_widens() {
        let body = vec![
            Instruction::BinOp {
                dest: Value(10),
                op: IrBinOp::And,
                lhs: Operand::Value(Value(1)),
                rhs: Operand::Const(IrConst::I64(7)),
                ty: IrType::U32,
            },
            Instruction::Cast {
                dest: Value(11),
                src: Operand::Value(Value(10)),
                from_ty: IrType::U32,
                to_ty: IrType::I64,
            },
            gep_inst(12, 11, IrType::I8),
        ];
        let cmp = Instruction::Cmp {
            dest: Value(99),
            op: IrCmpOp::Ugt,
            lhs: Operand::Value(Value(1)),
            rhs: Operand::Const(IrConst::I64(4)),
            ty: IrType::U32,
        };
        let mut func = rotated_loop(
            "exposed_dec",
            IrType::U32,
            const_seed_insts(10),
            body,
            rotated_latch(IrBinOp::Sub, 1),
            cmp,
            vec![],
        );
        check(&mut func, 1);
        assert!(matches!(
            &func.blocks[1].instructions[0],
            Instruction::Phi {
                ty: IrType::U64,
                ..
            }
        ));
        assert!(func.blocks[1].instructions.iter().any(|i| matches!(
            i,
            Instruction::BinOp {
                dest: Value(5),
                op: IrBinOp::Sub,
                ty: IrType::U64,
                ..
            }
        )));
    }
}
