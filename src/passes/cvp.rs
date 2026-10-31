//! Correlated value propagation (dominance-scoped predicate folding).
//!
//! Conditional control flow is a source of facts: on the true edge of
//! `br (x <u 10)`, `x` is known to lie in `[0, 9]`; on the false edge of
//! `br (v == 0)`, `v` is known non-zero.  Every block dominated by such an
//! edge inherits the fact, and any later compare of the same value against a
//! constant whose outcome is *implied* by the accumulated facts is redundant.
//!
//! GCC (VRP / evrp / dom-threading) and LLVM (CorrelatedValuePropagation,
//! LazyValueInfo) both perform this analysis; LCCC did not, so redundant
//! checks survived to the backend.  Two concrete shapes motivated the pass:
//!
//!  * Linux `find_next_bit`: the `__ffs` tree is recognised as `Ctz`, but the
//!    recogniser must guard the zero case (`v == 0 ? 63 : ctz(v)`) because
//!    the C function is well defined for zero.  The tree is only ever reached
//!    through `while (!value)`, so `value != 0` on that path and the guard
//!    (`test/sete/movzbl/test/cmov` ×2, ten instructions) is dead.
//!  * Post-inlining chains such as `if (p) f(p)` with `f` starting `if (!p)`,
//!    SQLite/Expat byte classifiers that re-test a range already tested by the
//!    enclosing branch, and `switch` cases that re-compare the selector.
//!
//! # Facts
//!
//! A fact is `(value, bits, interval-set)`: the value of `value` (interpreted
//! as a `bits`-wide integer) is contained in the interval set.  Interval sets
//! are kept in the **unsigned** domain `[0, 2^bits)`; signed comparisons are
//! mapped by splitting at the sign boundary, so signed and unsigned facts
//! about the same value compose (`x >s -1 && x <s 100` ⇒ `x <u 100`).
//!
//! Edge facts come from `CondBranch` on a `Cmp value-vs-constant` (and from
//! the boolean condition value itself, so a re-branch on the same `_Bool`
//! folds) and from `Switch` (`val == case` on a case edge, `val != every
//! case` on the default edge).
//!
//! A fact holds in block `S` when **every** predecessor edge of `S` carries
//! it (single-predecessor blocks are the common special case; the per-edge
//! sets are unioned).  Facts of `S` then hold in every block `S` dominates.
//! The pass walks the dominator tree with an explicit fact stack, so
//! gathering is `O(blocks + facts)`.
//!
//! # Folds
//!
//! * `Cmp value-vs-constant` whose truth set contains / is disjoint from the
//!   fact set → canonical boolean constant (`IrConst::I32(0|1)`, as SCCP).
//! * `Select` whose condition is a known boolean → the selected arm.
//! * A *use* of `S = select C,A,B` in a block where `C` is a known boolean
//!   → the selected arm (pure rename, no code motion: the arm dominates `S`
//!   which dominates the use).  This is the select-before-branch order that
//!   jump-threading cannot see — the select dominates the branch, so no
//!   predecessor edge predicts it — and lets DCE + sinking delete selects
//!   duplicated across correlated diamonds (libm `trunc`: the inlined
//!   round-to-even select feeds both arms of `if (x >= 0)` on its own
//!   condition).  Phi incomings are never rewritten (edge-flavored facts);
//!   `IsConstant` operands are skipped so `__builtin_constant_p` keeps its
//!   `-O0` answer under optimization.
//! * `CondBranch` whose condition is known → `Branch`, with the dropped
//!   successor's phi incomings for this block removed.  Removing edges can
//!   only enlarge dominance, so facts computed before the edit stay valid
//!   for the rest of the walk; `cfg_simplify` later deletes unreachable
//!   blocks.
//!
//! The pass never invents values, never widens a type, and is idempotent.
//! Pass name for `CCC_DISABLE_PASSES`: `"cvp"`.

use crate::common::fx_hash::{FxHashMap, FxHashSet};

use crate::common::types::IrType;
use crate::ir::analysis::{
    build_cfg, build_dom_tree_children, build_label_map, compute_dominators,
};
use crate::ir::reexports::{
    BlockId, Instruction, IrBinOp, IrCmpOp, IrConst, IrFunction, Operand, Terminator, Value,
};

/// A sorted, disjoint, non-adjacent list of closed intervals in `[0, max]`.
type Set = Vec<(i128, i128)>;

/// Carrier width used for facts about boolean condition values themselves.
const BOOL_BITS: u32 = 64;

fn bits_of(ty: IrType) -> Option<u32> {
    match ty {
        IrType::I8 | IrType::U8 => Some(8),
        IrType::I16 | IrType::U16 => Some(16),
        IrType::I32 | IrType::U32 => Some(32),
        IrType::I64 | IrType::U64 => Some(64),
        IrType::Ptr => Some((crate::common::types::target_ptr_size() * 8) as u32),
        _ => None,
    }
}

fn umax(bits: u32) -> i128 {
    (1i128 << bits) - 1
}

/// Normalise an integer constant to the unsigned `bits`-wide domain.
fn to_unsigned(c: i128, bits: u32) -> i128 {
    c & umax(bits)
}

fn normalize(mut v: Set) -> Set {
    v.retain(|(a, b)| a <= b);
    v.sort_unstable();
    let mut out: Set = Vec::with_capacity(v.len());
    for (a, b) in v {
        if let Some(last) = out.last_mut() {
            if a <= last.1 + 1 {
                last.1 = last.1.max(b);
                continue;
            }
        }
        out.push((a, b));
    }
    out
}

fn complement(s: &Set, bits: u32) -> Set {
    let max = umax(bits);
    let mut out = Vec::new();
    let mut cur = 0i128;
    for &(a, b) in s {
        if a > cur {
            out.push((cur, a - 1));
        }
        cur = b + 1;
    }
    if cur <= max {
        out.push((cur, max));
    }
    out
}

fn intersect(x: &Set, y: &Set) -> Set {
    let mut out = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < x.len() && j < y.len() {
        let lo = x[i].0.max(y[j].0);
        let hi = x[i].1.min(y[j].1);
        if lo <= hi {
            out.push((lo, hi));
        }
        if x[i].1 < y[j].1 {
            i += 1;
        } else {
            j += 1;
        }
    }
    out
}

/// Peel conversions that preserve the zero/nonzero partition exactly.
fn zero_equivalent_root(
    mut value: Value,
    casts: &FxHashMap<Value, (Operand, IrType, IrType)>,
) -> Value {
    // Same-width integer reinterpretations and widenings preserve zero in both
    // directions. Truncations do not: e.g. 0x100 -> u8 is zero. Bound the walk
    // defensively even though valid SSA cast chains are acyclic.
    for _ in 0..32 {
        let Some(&(Operand::Value(src), from_ty, to_ty)) = casts.get(&value) else {
            break;
        };
        if !from_ty.is_integer() || !to_ty.is_integer() || to_ty.size() < from_ty.size() {
            break;
        }
        value = src;
    }
    value
}

/// Whether any active predicate excludes zero for `value`. Zero/nonzero is
/// representation-invariant across zero-preserving integer casts, so this also
/// consumes BOOL_BITS facts attached to direct `CondBranch(value)`.
fn proves_nonzero(
    facts: &[Fact],
    value: Value,
    casts: &FxHashMap<Value, (Operand, IrType, IrType)>,
) -> bool {
    let root = zero_equivalent_root(value, casts);
    facts.iter().any(|f| {
        zero_equivalent_root(f.value, casts) == root
            && !f.set.is_empty()
            && f.set.iter().all(|&(lo, hi)| !(lo <= 0 && 0 <= hi))
    })
}

/// Resolve an operand to the integer constant it must hold, looking through
/// copies and constant casts, together with the bit width it is seen at.
/// Frontends routinely materialise small literals as `Cast(Const)` pairs, so a
/// literal-only test would miss the `x | 1` guard entirely.
fn integer_mask(bits: u32) -> u128 {
    u128::MAX >> (128 - bits)
}

fn resolved_constant(
    func: &IrFunction,
    defs: &FxHashMap<Value, (usize, usize)>,
    mut operand: Operand,
) -> Option<(u128, u32)> {
    // A single budget covers both copies and casts. The old recursive cast
    // walk reset its budget at each cast and could exhaust the compiler stack.
    let mut conversions: Vec<(IrType, IrType)> = Vec::new();
    for _ in 0..64 {
        match operand {
            Operand::Const(c) => {
                let mut n = c.to_i128()? as u128;
                let mut bits = 128;
                for (from, to) in conversions.into_iter().rev() {
                    let from_bits = (from.size() * 8) as u32;
                    let to_bits = (to.size() * 8) as u32;
                    n &= integer_mask(from_bits);
                    if from.is_signed()
                        && to_bits > from_bits
                        && n & (1u128 << (from_bits - 1)) != 0
                    {
                        n |= !integer_mask(from_bits);
                    }
                    n &= integer_mask(to_bits);
                    bits = to_bits;
                }
                return Some((n, bits));
            }
            Operand::Value(v) => {
                let &(bi, ii) = defs.get(&v)?;
                match &func.blocks[bi].instructions[ii] {
                    Instruction::Cast {
                        src,
                        from_ty,
                        to_ty,
                        ..
                    } if from_ty.is_integer() && to_ty.is_integer() => {
                        conversions.push((*from_ty, *to_ty));
                        operand = *src;
                    }
                    Instruction::Copy { src, .. } => operand = *src,
                    _ => return None,
                }
            }
        }
    }
    None
}

/// Whether `value`'s own defining operation returns a nonzero result on every
/// path, with no help from a dominating predicate. The canonical source form is
/// the portable zero guard `x | C` with `C != 0` (`__builtin_clz(x | 1)`,
/// `__builtin_ctz(x | 0x80000000)`): the OR forces a bit on, so the result is
/// nonzero for every `x`, including the `x == 0` case the guard exists for.
/// Count intrinsics consume this fact to pick their branch-free lowering, so it
/// is worth deriving from the definition instead of only from predicates.
///
/// Copies and zero-preserving casts are transparent; nothing else is. In
/// particular `And(x, C)` is not admitted: a bitwise AND can still clear every
/// bit, so it proves nothing about zero.
fn def_proves_nonzero(
    func: &IrFunction,
    defs: &FxHashMap<Value, (usize, usize)>,
    casts: &FxHashMap<Value, (Operand, IrType, IrType)>,
    value: Value,
) -> bool {
    // OR is nonzero if either operand is. Search both sides without recursion,
    // and cap work for malformed or adversarially deep SSA graphs.
    let mut pending = vec![value];
    let mut seen = FxHashSet::default();
    while let Some(value) = pending.pop() {
        let value = zero_equivalent_root(value, casts);
        if !seen.insert(value) {
            continue;
        }
        if seen.len() > 256 {
            return false;
        }
        let Some(&(bi, ii)) = defs.get(&value) else {
            continue;
        };
        match &func.blocks[bi].instructions[ii] {
            Instruction::Copy {
                src: Operand::Value(next),
                ..
            } => pending.push(*next),
            Instruction::BinOp {
                op: crate::ir::reexports::IrBinOp::Or,
                lhs,
                rhs,
                ty,
                ..
            } if ty.is_integer() => {
                let constant_nonzero = |o: &Operand| {
                    resolved_constant(func, defs, *o).is_some_and(|(n, w)| {
                        let bits = w.min((ty.size() * 8) as u32);
                        n & integer_mask(bits) != 0
                    })
                };
                if constant_nonzero(lhs) || constant_nonzero(rhs) {
                    return true;
                }
                if let Operand::Value(v) = lhs {
                    pending.push(*v);
                }
                if let Operand::Value(v) = rhs {
                    pending.push(*v);
                }
            }
            _ => {}
        }
    }
    false
}

/// Portable count recognizers preserve zero -> W-1 by setting an extremal
/// bit: bit 0 for CLZ, bit W-1 for CTZ. Under a dominating nonzero fact about
/// x that bit cannot change its count, so the OR can go. Its arithmetic
/// width must equal the intrinsic width; casts of the OR are transparent only
/// when they preserve that width. Never use a fact about a pre-truncation root.
fn guarded_bitcount_input(
    func: &IrFunction,
    defs: &FxHashMap<Value, (usize, usize)>,
    casts: &FxHashMap<Value, (Operand, IrType, IrType)>,
    facts: &[Fact],
    mut src: Operand,
    op: crate::ir::reexports::IrUnaryOp,
    ty: IrType,
) -> Option<Operand> {
    if !matches!(ty, IrType::I32 | IrType::U32 | IrType::I64 | IrType::U64) {
        return None;
    }
    let bit = if matches!(
        op,
        crate::ir::reexports::IrUnaryOp::Ctz | crate::ir::reexports::IrUnaryOp::CtzNonZero
    ) {
        1u64 << (ty.size() * 8 - 1)
    } else {
        1
    };
    for _ in 0..32 {
        let Operand::Value(v) = src else {
            return None;
        };
        let &(bi, ii) = defs.get(&v)?;
        match &func.blocks[bi].instructions[ii] {
            Instruction::Copy { src: next, .. } => src = *next,
            Instruction::Cast {
                src: next,
                from_ty,
                to_ty,
                ..
            } if from_ty.is_integer()
                && to_ty.is_integer()
                && from_ty.size() == ty.size()
                && to_ty.size() == ty.size() =>
            {
                src = *next
            }
            Instruction::BinOp {
                op: crate::ir::reexports::IrBinOp::Or,
                lhs,
                rhs,
                ty: or_ty,
                ..
            } if or_ty.is_integer() && or_ty.size() == ty.size() => {
                let is_bit = |o: Operand| matches!(o, Operand::Const(c) if c.to_i128().map(|n| n as u64) == Some(bit));
                let x = if is_bit(*rhs) {
                    *lhs
                } else if is_bit(*lhs) {
                    *rhs
                } else {
                    return None;
                };
                return match x {
                    Operand::Value(v) if proves_nonzero(facts, v, casts) => Some(x),
                    _ => None,
                };
            }
            _ => return None,
        }
    }
    None
}

fn is_subset(sub: &Set, sup: &Set) -> bool {
    // Both normalised: every `sub` interval must sit inside one `sup` interval.
    let mut j = 0;
    for &(a, b) in sub {
        while j < sup.len() && sup[j].1 < a {
            j += 1;
        }
        if j >= sup.len() || sup[j].0 > a || sup[j].1 < b {
            return false;
        }
    }
    true
}

/// Signed closed interval → unsigned interval set (split at the sign bit).
fn signed_interval(a: i128, b: i128, bits: u32) -> Set {
    if a > b {
        return Vec::new();
    }
    let m = 1i128 << bits;
    if a >= 0 {
        vec![(a, b)]
    } else if b < 0 {
        vec![(a + m, b + m)]
    } else {
        vec![(0, b), (a + m, m - 1)]
    }
}

/// The set of `bits`-wide values `x` for which `x OP c` is true.
fn truth_set(op: IrCmpOp, c: i128, bits: u32) -> Set {
    let max = umax(bits);
    let cu = to_unsigned(c, bits);
    let smin = -(1i128 << (bits - 1));
    let smax = (1i128 << (bits - 1)) - 1;
    // Sign-extend the constant for signed orderings.
    let cs = if cu > smax { cu - (1i128 << bits) } else { cu };
    let raw = match op {
        IrCmpOp::Eq => vec![(cu, cu)],
        IrCmpOp::Ne => return normalize(complement(&vec![(cu, cu)], bits)),
        IrCmpOp::Ult => vec![(0, cu - 1)],
        IrCmpOp::Ule => vec![(0, cu)],
        IrCmpOp::Ugt => vec![(cu + 1, max)],
        IrCmpOp::Uge => vec![(cu, max)],
        IrCmpOp::Slt => signed_interval(smin, cs - 1, bits),
        IrCmpOp::Sle => signed_interval(smin, cs, bits),
        IrCmpOp::Sgt => signed_interval(cs + 1, smax, bits),
        IrCmpOp::Sge => signed_interval(cs, smax, bits),
    };
    normalize(raw)
}

/// True when `op` provably evaluates to a never-null address at runtime:
/// the address of a defined symbol, a frame alloca, a code label, or the
/// stack pointer — possibly through Copy residue left by earlier passes.
/// C11 gives every object a distinct, non-null address (6.5.3.2, 7.19),
/// and frame/stack addresses derive from %rsp, so comparing any of them
/// against NULL has a compile-time answer.  Deliberately NOT covered:
/// casts (an inttoptr round-trip can re-enter 0), GEPs (offset arithmetic
/// on a null base is C undefined behavior, but the IR-level value is not
/// provably non-null without UB reasoning), and GetStaticChain (zero for
/// functions without a static chain).
fn never_null_address(
    func: &IrFunction,
    defs: &FxHashMap<Value, (usize, usize)>,
    defined_syms: &FxHashSet<String>,
    op: &Operand,
    depth: u8,
) -> bool {
    let Operand::Value(v) = op else {
        return false;
    };
    let Some(&(bi, ii)) = defs.get(v) else {
        return false;
    };
    match &func.blocks[bi].instructions[ii] {
        // GlobalAddr is never-null ONLY for a symbol this module defines
        // (or strongly externs): an ELF weak-undefined symbol resolves to
        // address zero at static-link time, and `&weak_opt == 0` is a real
        // configuration-idiom.  The caller passes the module's
        // defined-nonweak set (see defined_nonweak_symbols).
        Instruction::GlobalAddr { name, .. } => defined_syms.contains(name),
        Instruction::Alloca { .. }
        | Instruction::DynAlloca { .. }
        | Instruction::LabelAddr { .. }
        | Instruction::StackSave { .. } => true,
        Instruction::Copy { src, .. } if depth > 0 => {
            never_null_address(func, defs, defined_syms, src, depth - 1)
        }
        _ => false,
    }
}

fn is_null_const(op: &Operand) -> bool {
    matches!(op, Operand::Const(k) if k.to_i128() == Some(0))
}

/// The compile-time verdict of comparing a never-null address against
/// NULL.  Pointer comparisons are unsigned by ABI, so every unsigned
/// predicate has an answer; signed predicates on pointers never occur
/// from C lowering and stay unfolded by choice.
///
/// Truth table for `p OP 0` with p != 0 guaranteed (the never-null law):
///   Eq  -> false   (p is not 0)
///   Ne  -> true
///   Ult -> false   (no unsigned value is < 0)
///   Ule -> false   (p u<= 0 would force p == 0, excluded)
///   Ugt -> true    (every nonzero unsigned value is > 0)
///   Uge -> true    (every value is u>= 0)
///
/// The swapped direction `0 OP p` is canonicalized with `swap_op` BEFORE
/// the table lookup — `0 u< p` is true while `p u< 0` is false, so a
/// shared table without canonicalization answers two of the four
/// ordering cases backwards.
fn null_compare_verdict(
    func: &IrFunction,
    defs: &FxHashMap<Value, (usize, usize)>,
    defined_syms: &FxHashSet<String>,
    op: IrCmpOp,
    lhs: &Operand,
    rhs: &Operand,
) -> Option<bool> {
    if never_null_address(func, defs, defined_syms, lhs, 4) && is_null_const(rhs) {
        return match op {
            IrCmpOp::Eq | IrCmpOp::Ult | IrCmpOp::Ule => Some(false),
            IrCmpOp::Ne | IrCmpOp::Ugt | IrCmpOp::Uge => Some(true),
            _ => None,
        };
    }
    if never_null_address(func, defs, defined_syms, rhs, 4) && is_null_const(lhs) {
        return match swap_op(op) {
            IrCmpOp::Eq | IrCmpOp::Ult | IrCmpOp::Ule => Some(false),
            IrCmpOp::Ne | IrCmpOp::Ugt | IrCmpOp::Uge => Some(true),
            _ => None,
        };
    }
    None
}

fn swap_op(op: IrCmpOp) -> IrCmpOp {
    match op {
        IrCmpOp::Eq => IrCmpOp::Eq,
        IrCmpOp::Ne => IrCmpOp::Ne,
        IrCmpOp::Slt => IrCmpOp::Sgt,
        IrCmpOp::Sle => IrCmpOp::Sge,
        IrCmpOp::Sgt => IrCmpOp::Slt,
        IrCmpOp::Sge => IrCmpOp::Sle,
        IrCmpOp::Ult => IrCmpOp::Ugt,
        IrCmpOp::Ule => IrCmpOp::Uge,
        IrCmpOp::Ugt => IrCmpOp::Ult,
        IrCmpOp::Uge => IrCmpOp::Ule,
    }
}

/// `value OP const` canonical form of a compare, if exactly one side is an
/// integer constant.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Pred {
    value: Value,
    op: IrCmpOp,
    c: i128,
    bits: u32,
}

fn canonical_pred(op: IrCmpOp, lhs: &Operand, rhs: &Operand, ty: IrType) -> Option<Pred> {
    let bits = bits_of(ty)?;
    match (lhs, rhs) {
        (Operand::Value(v), Operand::Const(k)) => Some(Pred {
            value: *v,
            op,
            c: k.to_i128()?,
            bits,
        }),
        (Operand::Const(k), Operand::Value(v)) => Some(Pred {
            value: *v,
            op: swap_op(op),
            c: k.to_i128()?,
            bits,
        }),
        _ => None,
    }
}

/// A fact that holds on a CFG edge: `value ∈ set` (as a `bits`-wide integer).
#[derive(Clone, Debug, PartialEq, Eq)]
struct Fact {
    value: Value,
    bits: u32,
    set: Set,
}

impl Fact {
    fn from_pred(p: Pred, truth: bool) -> Fact {
        let mut set = truth_set(p.op, p.c, p.bits);
        if !truth {
            set = complement(&set, p.bits);
        }
        Fact {
            value: p.value,
            bits: p.bits,
            set,
        }
    }
}

/// Facts carried by the edge `from → to_label`.
fn edge_facts(
    func: &IrFunction,
    defs: &FxHashMap<Value, (usize, usize)>,
    copies: &FxHashMap<Value, Operand>,
    from: usize,
    to_label: BlockId,
) -> Vec<Fact> {
    let mut out = Vec::new();
    match &func.blocks[from].terminator {
        Terminator::CondBranch {
            cond: Operand::Value(cv),
            true_label,
            false_label,
        } => {
            if true_label == false_label {
                return out;
            }
            let truth = if *true_label == to_label {
                true
            } else if *false_label == to_label {
                false
            } else {
                return out;
            };
            // The boolean itself is known on this edge (its carrier type
            // does not matter: the consumer only asks "zero or not").
            out.push(Fact {
                value: *cv,
                bits: BOOL_BITS,
                set: if truth {
                    vec![(1, umax(BOOL_BITS))]
                } else {
                    vec![(0, 0)]
                },
            });
            // Resolve copies (GVN/CSE residue between a compare and its
            // branch) to the compared value: a copy preserves every bit, so
            // the truth established for the branch condition holds identically
            // for the compare's destination, and the predicate fact (when the
            // compare canonicalizes) holds for the predicate.  Without this a
            // `br (copy-of-cmp)` edge carries no usable fact — the float
            // compares never canonicalize, so the value fact is their ONLY
            // channel (libm `trunc`: `br %6`, `%6 = copy %30`, `%30` the
            // `x >= 0.0` compare feeding the inlined select).
            let mut resolved = *cv;
            for _ in 0..16 {
                match copies.get(&resolved) {
                    Some(Operand::Value(v)) => resolved = *v,
                    _ => break,
                }
            }
            if resolved != *cv {
                out.push(Fact {
                    value: resolved,
                    bits: BOOL_BITS,
                    set: if truth {
                        vec![(1, umax(BOOL_BITS))]
                    } else {
                        vec![(0, 0)]
                    },
                });
            }
            if let Some(&(bi, ii)) = defs.get(&resolved) {
                if let Instruction::Cmp {
                    ref op,
                    ref lhs,
                    ref rhs,
                    ref ty,
                    ..
                } = func.blocks[bi].instructions[ii]
                {
                    if let Some(p) = canonical_pred(*op, lhs, rhs, *ty) {
                        out.push(Fact::from_pred(p, truth));
                    }
                }
            }
        }
        Terminator::Switch {
            val: Operand::Value(v),
            cases,
            default,
            ty,
        } => {
            let Some(bits) = bits_of(*ty) else { return out };
            if *default == to_label {
                // Default edge: val is none of the case values that do not
                // also target the default block.
                let excluded: Set = normalize(
                    cases
                        .iter()
                        .filter(|(_, l)| *l != to_label)
                        .map(|(c, _)| {
                            let u = to_unsigned(*c as i128, bits);
                            (u, u)
                        })
                        .collect(),
                );
                out.push(Fact {
                    value: *v,
                    bits,
                    set: complement(&excluded, bits),
                });
            } else {
                let hits: Set = normalize(
                    cases
                        .iter()
                        .filter(|(_, l)| *l == to_label)
                        .map(|(c, _)| {
                            let u = to_unsigned(*c as i128, bits);
                            (u, u)
                        })
                        .collect(),
                );
                if !hits.is_empty() {
                    out.push(Fact {
                        value: *v,
                        bits,
                        set: hits,
                    });
                }
            }
        }
        _ => {}
    }
    out
}

/// Facts that hold on entry to `blk`: for each value, the union over all
/// incoming edges of that edge's fact — kept only if every edge has one.
fn block_entry_facts(
    func: &IrFunction,
    defs: &FxHashMap<Value, (usize, usize)>,
    copies: &FxHashMap<Value, Operand>,
    preds: &[u32],
    blk: usize,
) -> Vec<Fact> {
    if preds.is_empty() {
        return Vec::new();
    }
    let label = func.blocks[blk].label;
    let mut acc: Option<Vec<Fact>> = None;
    for &p in preds {
        let ef = edge_facts(func, defs, copies, p as usize, label);
        acc = Some(match acc {
            None => ef,
            Some(prev) => {
                let mut merged = Vec::new();
                for f in prev {
                    for g in ef.iter().filter(|g| g.value == f.value && g.bits == f.bits) {
                        let mut u = f.set.clone();
                        u.extend_from_slice(&g.set);
                        merged.push(Fact {
                            value: f.value,
                            bits: f.bits,
                            set: normalize(u),
                        });
                    }
                }
                merged
            }
        });
        if acc.as_ref().is_some_and(Vec::is_empty) {
            break;
        }
    }
    acc.unwrap_or_default()
}

/// Known interval set for `value` given the active facts.
fn known_set(stack: &[Fact], value: Value, bits: u32) -> Option<Set> {
    let mut acc: Option<Set> = None;
    for f in stack.iter().filter(|f| f.value == value && f.bits == bits) {
        acc = Some(match acc {
            None => f.set.clone(),
            Some(prev) => intersect(&prev, &f.set),
        });
    }
    acc
}

/// Decide `value OP c` under the active facts.
fn decide_pred(stack: &[Fact], p: Pred) -> Option<bool> {
    let known = known_set(stack, p.value, p.bits)?;
    if known.is_empty() {
        // Contradictory facts: the block is unreachable.  Leave it alone —
        // nothing here is observable, and cfg_simplify owns dead code.
        return None;
    }
    let t = truth_set(p.op, p.c, p.bits);
    if is_subset(&known, &t) {
        Some(true)
    } else if intersect(&known, &t).is_empty() {
        Some(false)
    } else {
        None
    }
}

/// Decide the truth of a boolean operand under the active facts.
///
/// `cond` values defined by a `Copy` are looked through recursively (depth
/// capped): this covers BOTH the values this pass itself folds — a decided
/// `Cmp` becomes `Copy { src: bool_const }` at its own def site, and every
/// later `Select`/branch conditioned on it must see the constant — and
/// ordinary copy chains left by earlier passes. Without the look-through,
/// folding a guard `Cmp` would erase the very fact its consumers need
/// (reproduced: full_function_folds_guard_and_branch folded 2/4 instead of
/// 4/4 — the select and the branch after the folded guard stayed put).
fn decide_bool(
    stack: &[Fact],
    func: &IrFunction,
    defs: &FxHashMap<Value, (usize, usize)>,
    defined_syms: &FxHashSet<String>,
    cond: &Operand,
) -> Option<bool> {
    #[allow(clippy::too_many_arguments)]
    fn go(
        stack: &[Fact],
        func: &IrFunction,
        defs: &FxHashMap<Value, (usize, usize)>,
        defined_syms: &FxHashSet<String>,
        cond: &Operand,
        depth: u8,
    ) -> Option<bool> {
        match cond {
            Operand::Const(k) => k.to_i128().map(|v| v != 0),
            Operand::Value(v) => {
                if let Some(s) = known_set(stack, *v, BOOL_BITS) {
                    if s.as_slice() == [(0, 0)] {
                        return Some(false);
                    }
                    if !s.is_empty() && s[0].0 >= 1 {
                        return Some(true);
                    }
                }
                if depth == 0 {
                    return None;
                }
                let &(bi, ii) = defs.get(v)?;
                match &func.blocks[bi].instructions[ii] {
                    Instruction::Copy { src, .. } => {
                        go(stack, func, defs, defined_syms, src, depth - 1)
                    }
                    Instruction::Cmp {
                        op, lhs, rhs, ty, ..
                    } => decide_pred(stack, canonical_pred(*op, lhs, rhs, *ty)?)
                        .or_else(|| null_compare_verdict(func, defs, defined_syms, *op, lhs, rhs)),
                    _ => None,
                }
            }
        }
    }
    go(stack, func, defs, defined_syms, cond, 16)
}

fn bool_const(b: bool) -> Operand {
    Operand::Const(IrConst::I32(b as i32))
}

/// If `op` uses a select whose condition the fact stack decides, the selected
/// arm; otherwise `None`.  The arm is dominated by the select which dominates
/// the use, so the rename never moves code and the arm is always available.
fn decided_select_arm(
    op: &Operand,
    stack: &[Fact],
    func: &IrFunction,
    defs: &FxHashMap<Value, (usize, usize)>,
    defined_syms: &FxHashSet<String>,
    selects: &FxHashMap<Value, (Operand, Operand, Operand)>,
) -> Option<Operand> {
    if let Operand::Value(v) = op {
        if let Some((cond, true_val, false_val)) = selects.get(v) {
            if let Some(t) = decide_bool(stack, func, defs, defined_syms, cond) {
                return Some(if t { *true_val } else { *false_val });
            }
        }
    }
    None
}

/// The constant `op` is known to equal under the active facts, spelled for a
/// consumer that reads it as `ty`; `None` unless the facts pin `op` to one
/// value of exactly `ty`'s width.
///
/// Facts are `bits`-wide unsigned interval sets, so the singleton is
/// re-signed for `IrConst::from_i64` (which owns the per-type constant
/// spelling, e.g. `U32` in a zero-extended `I64` payload). A 64-bit constant
/// outside the sign-extended imm32 range is refused: materialising it costs a
/// `movabsq` at every use, while the register it replaces is already there.
fn singleton_const(stack: &[Fact], op: &Operand, ty: IrType) -> Option<Operand> {
    let Operand::Value(v) = op else {
        return None;
    };
    let bits = bits_of(ty)?;
    // A direct `CondBranch(v)` edge only records "zero or not" on a 64-bit
    // carrier. Zero is zero at every width, so a `{0}` boolean fact pins a
    // narrower use too (`if (a) ...; else g(a)` → `g(0)`); a nonzero one does
    // not, because the carrier's extension of `v` is unknown.
    let zero_edge = bits < BOOL_BITS
        && known_set(stack, *v, BOOL_BITS).is_some_and(|s| s.as_slice() == [(0, 0)]);
    let lo = if zero_edge {
        0
    } else {
        let known = known_set(stack, *v, bits)?;
        let &[(lo, hi)] = known.as_slice() else {
            return None;
        };
        if lo != hi {
            return None;
        }
        lo
    };
    let signed = if lo >= 1i128 << (bits - 1) {
        lo - (1i128 << bits)
    } else {
        lo
    };
    let value = i64::try_from(signed).ok()?;
    if bits == 64 && i32::try_from(value).is_err() {
        return None;
    }
    Some(Operand::Const(IrConst::from_i64(value, ty)))
}

/// Integer-register slot of each argument under the 64-bit register
/// conventions, for the scalar integer/pointer arguments among the first six
/// (the smallest register-argument count of SysV x86-64, AAPCS64 and the
/// RISC-V LP64 ABI). Floats take no integer slot; the first aggregate or
/// wide scalar ends the mapping, since its slot use is ABI-specific.
/// Empty on 32-bit targets, whose arguments live on the stack.
fn gp_arg_slots(args: impl Iterator<Item = (IrType, bool)>) -> Vec<Option<usize>> {
    let mut out = Vec::new();
    if crate::common::types::target_is_32bit() {
        return out;
    }
    let mut next = 0usize;
    let mut open = true;
    for (ty, aggregate) in args {
        let scalar_int = !aggregate && (ty.is_integer() || ty == IrType::Ptr) && ty.size() <= 8;
        if aggregate || (!scalar_int && !ty.is_float()) {
            open = false;
        }
        out.push((open && scalar_int && next < 6).then_some(next));
        if scalar_int {
            next += 1;
        }
    }
    out
}

/// Parameters that arrive in an integer argument register, by slot
/// (see [`gp_arg_slots`]).
fn in_place_params(func: &IrFunction) -> FxHashMap<Value, usize> {
    let slots = gp_arg_slots(func.params.iter().map(|p| (p.ty, p.struct_size.is_some())));
    let mut out = FxHashMap::default();
    for b in &func.blocks {
        for inst in &b.instructions {
            if let Instruction::ParamRef {
                dest, param_idx, ..
            } = inst
            {
                if let Some(Some(s)) = slots.get(*param_idx) {
                    out.insert(*dest, *s);
                }
            }
        }
    }
    out
}

/// Replace uses the active facts pin to a single constant (`x == 7` on this
/// path: `g(x)` → `g(7)`), at operand positions that accept an immediate.
///
/// The win is the register, not the arithmetic: on the `a == 0` arm of
/// `if (a) return g3(a, b, c); return g3(b, a, c);` the second argument
/// becomes `xorl %esi, %esi`, so `a` is no longer live into that arm and the
/// copy that kept it there disappears (GCC's DOM / LLVM's GVN equality
/// propagation; GCC 16.2, Clang 23.1.4 and ICX 2026.2 all emit the `xorl`).
///
/// Positions: scalar call arguments (typed by `arg_types`; aggregates and
/// the indirect callee stay), both operands of add/sub/mul/and/or/xor, the
/// LEFT operand of division, remainder and shifts (a known-zero divisor or
/// out-of-range count must never reach the constant folder as a literal),
/// compare operands, stored values, select arms and the returned value.
/// Never phis (edge-flavored), never `IsConstant` (`__builtin_constant_p`
/// keeps its unoptimized answer), never pointers, asm operands or intrinsic
/// arguments.
fn substitute_singletons(
    inst: &mut Instruction,
    stack: &[Fact],
    in_place: &FxHashMap<Value, usize>,
) -> usize {
    let mut n = 0usize;
    let mut sub = |op: &mut Operand, ty: IrType| {
        if let Some(c) = singleton_const(stack, op, ty) {
            *op = c;
            n += 1;
        }
    };
    match inst {
        Instruction::Call { info, .. } | Instruction::CallIndirect { info, .. } => {
            let slots = gp_arg_slots(info.arg_types.iter().enumerate().map(|(k, ty)| {
                (
                    *ty,
                    info.struct_arg_sizes.get(k).is_some_and(Option::is_some),
                )
            }));
            for (k, arg) in info.args.iter_mut().enumerate() {
                let aggregate = info.struct_arg_sizes.get(k).is_some_and(Option::is_some);
                // A parameter forwarded in its own integer slot is already in
                // the argument register: the constant would cost a `xorl`/
                // `movl` the register makes free (`else v2(c, y)` with
                // `c == 0`).
                let forwarded = matches!(arg, Operand::Value(v)
                    if slots.get(k).copied().flatten().is_some_and(|s| in_place.get(v) == Some(&s)));
                if let (false, false, Some(&ty)) = (aggregate, forwarded, info.arg_types.get(k)) {
                    sub(arg, ty);
                }
            }
        }
        Instruction::BinOp {
            op, lhs, rhs, ty, ..
        } => {
            use IrBinOp as B;
            let ty = *ty;
            match op {
                B::Add | B::Sub | B::Mul | B::And | B::Or | B::Xor => {
                    sub(lhs, ty);
                    sub(rhs, ty);
                }
                B::SDiv | B::UDiv | B::SRem | B::URem | B::Shl | B::AShr | B::LShr => {
                    sub(lhs, ty);
                }
                _ => {}
            }
        }
        Instruction::Cmp { lhs, rhs, ty, .. } => {
            let ty = *ty;
            sub(lhs, ty);
            sub(rhs, ty);
        }
        Instruction::Store { val, ty, .. } => sub(val, *ty),
        Instruction::Select {
            true_val,
            false_val,
            ty,
            ..
        } => {
            let ty = *ty;
            sub(true_val, ty);
            sub(false_val, ty);
        }
        _ => {}
    }
    n
}

/// Remove `pred_label` from the phi incomings of block `target`.
fn remove_phi_incoming(func: &mut IrFunction, target: usize, pred_label: BlockId) {
    for inst in &mut func.blocks[target].instructions {
        if let Instruction::Phi { incoming, .. } = inst {
            incoming.retain(|(_, l)| *l != pred_label);
        }
    }
}

/// Symbols whose address is provably non-null at static-link time: every
/// data global this module DEFINES (extern declarations excluded — a weak
/// undefined symbol resolves to address zero and `&weak_opt == 0` is a
/// real idiom), plus every function with a body in this module.  Weak
/// spellings are excluded even when a definition exists (preemption and
/// the weak-undef-to-zero binding make the null comparison meaningful).
/// GCC folds non-weak addresses the same way; weak guards it keeps.
pub fn defined_nonweak_symbols(module: &crate::ir::reexports::IrModule) -> FxHashSet<String> {
    let mut set: FxHashSet<String> = FxHashSet::default();
    for g in &module.globals {
        if g.is_weak {
            continue;
        }
        if g.is_extern && !g.is_common {
            continue;
        }
        set.insert(g.name.clone());
    }
    for f in &module.functions {
        if !f.is_declaration && !f.is_weak {
            set.insert(f.name.clone());
        }
    }
    // A weak extern-declaration attribute wins over any set membership
    // above (name spelled weak somewhere in this TU = the guard is real).
    for (name, is_weak, _) in &module.symbol_attrs {
        if *is_weak {
            set.remove(name);
        }
    }
    set
}

/// Run correlated value propagation with NO module knowledge: the
/// never-null-address law abstains on every GlobalAddr (weak-safe
/// default).  Tests and pass-external callers use this;
/// the pass driver uses [`run_function_with_symbols`].
pub fn run_function(func: &mut IrFunction) -> usize {
    run_function_with_symbols(func, &FxHashSet::default())
}

/// Run correlated value propagation on one function with the module's
/// defined-nonweak symbol set (see [`defined_nonweak_symbols`]).
/// Promote `Clz`/`Ctz` to their nonzero-contract forms when the operand's own
/// definition proves it nonzero, with no predicate and no CFG involved.
///
/// This is the portable zero-guard shape (`__builtin_clz(x | 1)`,
/// `__builtin_ctz(x | 0x80000000)`): setting any constant bit makes the operand
/// nonzero for every input, including the zero input the guard exists for. The
/// count then feeds the branch-free BSR/BSF lowering instead of a
/// test/jump diamond, so it matters even in a straight-line leaf function that
/// the predicate-driven body of CVP never looks at (it requires two blocks).
///
/// Only the opcode changes; the guard instruction stays and keeps its value.
/// Running twice is a no-op because the promoted ops no longer match.
fn promote_guarded_counts(func: &mut IrFunction) -> usize {
    let mut defs: FxHashMap<Value, (usize, usize)> = FxHashMap::default();
    let mut casts: FxHashMap<Value, (Operand, IrType, IrType)> = FxHashMap::default();
    for (bi, b) in func.blocks.iter().enumerate() {
        for (ii, inst) in b.instructions.iter().enumerate() {
            if let Some(dest) = inst.dest() {
                defs.insert(dest, (bi, ii));
            }
            if let Instruction::Cast {
                dest,
                src,
                from_ty,
                to_ty,
            } = inst
            {
                casts.insert(*dest, (*src, *from_ty, *to_ty));
            }
        }
    }
    let promoted: Vec<(Value, usize, usize)> = {
        let mut v = Vec::new();
        for (bi, block) in func.blocks.iter().enumerate() {
            for (ii, inst) in block.instructions.iter().enumerate() {
                let Instruction::UnaryOp {
                    dest,
                    op: crate::ir::reexports::IrUnaryOp::Clz | crate::ir::reexports::IrUnaryOp::Ctz,
                    src: Operand::Value(src),
                    ..
                } = inst
                else {
                    continue;
                };
                if def_proves_nonzero(func, &defs, &casts, *src) {
                    v.push((*dest, bi, ii));
                }
            }
        }
        v
    };
    for (_, bi, ii) in &promoted {
        let Instruction::UnaryOp { op, .. } = &mut func.blocks[*bi].instructions[*ii] else {
            continue;
        };
        *op = match *op {
            crate::ir::reexports::IrUnaryOp::Clz => crate::ir::reexports::IrUnaryOp::ClzNonZero,
            _ => crate::ir::reexports::IrUnaryOp::CtzNonZero,
        };
    }
    promoted.len()
}

pub fn run_function_with_symbols(func: &mut IrFunction, defined_syms: &FxHashSet<String>) -> usize {
    let defined_syms = defined_syms;
    // Straight-line functions have no predicates to propagate, but the
    // definition-based zero-guard proof below needs no CFG at all: run it first
    // so single-block leaves get the branch-free count lowering too.
    let mut changes = promote_guarded_counts(func);
    let n = func.blocks.len();
    if n < 2 {
        return changes;
    }
    let label_to_idx = build_label_map(func);
    let (preds, succs) = build_cfg(func, &label_to_idx);
    let idom = compute_dominators(n, &preds, &succs);
    let in_place = in_place_params(func);
    let children = build_dom_tree_children(n, &idom);

    // Value → definition maps used by predicate and zero-equivalence queries.
    let mut defs: FxHashMap<Value, (usize, usize)> = FxHashMap::default();
    let mut casts: FxHashMap<Value, (Operand, IrType, IrType)> = FxHashMap::default();
    // Copy chains (branch conditions through GVN residue) and select triples
    // (correlated-select use rewriting).  Both keyed by SSA dest (unique).
    let mut copies: FxHashMap<Value, Operand> = FxHashMap::default();
    let mut selects: FxHashMap<Value, (Operand, Operand, Operand)> = FxHashMap::default();
    for (bi, b) in func.blocks.iter().enumerate() {
        for (ii, inst) in b.instructions.iter().enumerate() {
            // `defs` covers EVERY value definition, not just compares: the
            // never-null-address law must see the GlobalAddr/Alloca/LabelAddr
            // producer behind a null test.  Consumers that only care about
            // compares guard with `if let Instruction::Cmp`, so the wider
            // map is behavior-identical for them.
            if let Some(dest) = inst.dest() {
                defs.insert(dest, (bi, ii));
            }
            match inst {
                Instruction::Copy { dest, src } => {
                    copies.insert(*dest, *src);
                }
                Instruction::Select {
                    dest,
                    cond,
                    true_val,
                    false_val,
                    ..
                } => {
                    selects.insert(*dest, (*cond, *true_val, *false_val));
                }
                Instruction::Cast {
                    dest,
                    src,
                    from_ty,
                    to_ty,
                } => {
                    casts.insert(*dest, (*src, *from_ty, *to_ty));
                }
                _ => {}
            }
        }
    }

    // Entry facts per block, computed on the pristine CFG.
    let entry_facts: Vec<Vec<Fact>> = (0..n)
        .map(|b| block_entry_facts(func, &defs, &copies, preds.row(b), b))
        .collect();

    let mut stack: Vec<Fact> = entry_facts[0].clone();
    // Explicit DFS over the dominator tree: (block, next child index).
    let mut work: Vec<(usize, usize)> = vec![(0, 0)];
    let mut pushed: Vec<usize> = vec![entry_facts[0].len()];

    while let Some(&mut (blk, ref mut next)) = work.last_mut() {
        if *next == 0 {
            // First visit: fold inside this block under the active facts.
            //
            // Correlated-select USE rewriting runs BEFORE definition
            // folding: once a select folds to `Copy(arm)` its triple is
            // gone from the instruction stream, and uses rewritten here
            // directly (rather than via the copy) converge in one visit.
            // Phi incomings are edge-flavored, never block facts: skipped.
            // `IsConstant` operands are skipped so `__builtin_constant_p`
            // keeps its `-O0` answer under optimization.
            if !selects.is_empty() {
                let mut i = 0;
                while i < func.blocks[blk].instructions.len() {
                    let skip = matches!(
                        func.blocks[blk].instructions[i],
                        Instruction::Phi { .. }
                            | Instruction::UnaryOp {
                                op: crate::ir::reexports::IrUnaryOp::IsConstant,
                                ..
                            }
                    );
                    if !skip {
                        // Clone-rewrite-writeback: decide_bool borrows
                        // `func` immutably, so the rewrite computes on an
                        // owned clone and the slot is only touched after.
                        let mut new_inst = func.blocks[blk].instructions[i].clone();
                        let mut n = 0usize;
                        new_inst.for_each_operand_mut(|op| {
                            if let Some(arm) =
                                decided_select_arm(op, &stack, func, &defs, defined_syms, &selects)
                            {
                                *op = arm;
                                n += 1;
                            }
                        });
                        if n > 0 {
                            // A rewritten select's own triple changed:
                            // refresh its map entry so later blocks rewrite
                            // to the current arm in the same visit.  (A stale
                            // arm would still be value-correct at every
                            // dominated use by SSA immutability — the old arm
                            // equals the new one wherever this block's facts
                            // hold, which is everywhere the select's uses
                            // are — but freshness converges in one visit
                            // instead of two and keeps the map truthful.)
                            if let Instruction::Select {
                                dest,
                                cond,
                                true_val,
                                false_val,
                                ..
                            } = &new_inst
                            {
                                selects.insert(*dest, (*cond, *true_val, *false_val));
                            }
                            func.blocks[blk].instructions[i] = new_inst;
                            changes += n;
                        }
                    }
                    i += 1;
                }
                // Terminator operands (branch conditions, return values).
                let mut new_term = func.blocks[blk].terminator.clone();
                let mut n = 0usize;
                new_term.for_each_operand_mut(|op| {
                    if let Some(arm) =
                        decided_select_arm(op, &stack, func, &defs, defined_syms, &selects)
                    {
                        *op = arm;
                        n += 1;
                    }
                });
                if n > 0 {
                    func.blocks[blk].terminator = new_term;
                    changes += n;
                }
            }
            let mut i = 0;
            while i < func.blocks[blk].instructions.len() {
                let repl = match &func.blocks[blk].instructions[i] {
                    Instruction::Cmp {
                        dest,
                        op,
                        lhs,
                        rhs,
                        ty,
                    } => canonical_pred(*op, lhs, rhs, *ty)
                        .and_then(|p| decide_pred(&stack, p))
                        .or_else(|| null_compare_verdict(func, &defs, defined_syms, *op, lhs, rhs))
                        .map(|t| Instruction::Copy {
                            dest: *dest,
                            src: bool_const(t),
                        }),
                    Instruction::Select {
                        dest,
                        cond,
                        true_val,
                        false_val,
                        ..
                    } => decide_bool(&stack, func, &defs, defined_syms, cond).map(|t| {
                        Instruction::Copy {
                            dest: *dest,
                            src: if t { *true_val } else { *false_val },
                        }
                    }),
                    Instruction::UnaryOp {
                        dest,
                        op:
                            op @ (crate::ir::reexports::IrUnaryOp::Clz
                            | crate::ir::reexports::IrUnaryOp::ClzNonZero
                            | crate::ir::reexports::IrUnaryOp::Ctz
                            | crate::ir::reexports::IrUnaryOp::CtzNonZero),
                        src,
                        ty,
                    } if let Some(src) =
                        guarded_bitcount_input(func, &defs, &casts, &stack, *src, *op, *ty) =>
                    {
                        Some(Instruction::UnaryOp {
                            dest: *dest,
                            op: if matches!(
                                op,
                                crate::ir::reexports::IrUnaryOp::Clz
                                    | crate::ir::reexports::IrUnaryOp::ClzNonZero
                            ) {
                                crate::ir::reexports::IrUnaryOp::ClzNonZero
                            } else {
                                crate::ir::reexports::IrUnaryOp::CtzNonZero
                            },
                            src,
                            ty: *ty,
                        })
                    }
                    Instruction::UnaryOp {
                        dest,
                        op:
                            op @ (crate::ir::reexports::IrUnaryOp::Clz
                            | crate::ir::reexports::IrUnaryOp::Ctz),
                        src: Operand::Value(src),
                        ty,
                    } if proves_nonzero(&stack, *src, &casts)
                        || def_proves_nonzero(func, &defs, &casts, *src) =>
                    {
                        Some(Instruction::UnaryOp {
                            dest: *dest,
                            op: match op {
                                crate::ir::reexports::IrUnaryOp::Clz => {
                                    crate::ir::reexports::IrUnaryOp::ClzNonZero
                                }
                                _ => crate::ir::reexports::IrUnaryOp::CtzNonZero,
                            },
                            src: Operand::Value(*src),
                            ty: *ty,
                        })
                    }
                    _ => None,
                };
                if let Some(r) = repl {
                    // The fold REPLACES the Cmp with its decided constant at
                    // the same (bi, ii), so the defs entry stays valid — it
                    // now maps to a `Copy { src: bool_const }`, which
                    // decide_bool looks through. Erasing the entry here would
                    // hide the folded fact from later consumers in this very
                    // block (select cond / branch cond follow the guard).
                    func.blocks[blk].instructions[i] = r;
                    changes += 1;
                }
                i += 1;
            }
            let decided = match &func.blocks[blk].terminator {
                Terminator::CondBranch {
                    cond,
                    true_label,
                    false_label,
                } if true_label != false_label => {
                    decide_bool(&stack, func, &defs, defined_syms, cond).map(|t| {
                        if t {
                            (*true_label, *false_label)
                        } else {
                            (*false_label, *true_label)
                        }
                    })
                }
                _ => None,
            };
            if let Some((keep, drop)) = decided {
                let this_label = func.blocks[blk].label;
                if let Some(&di) = label_to_idx.get(&drop) {
                    remove_phi_incoming(func, di, this_label);
                }
                func.blocks[blk].terminator = Terminator::Branch(keep);
                changes += 1;
            }
            // Singleton substitution (see `substitute_singletons`) runs LAST:
            // the folds above decide compares, selects and branches from
            // `Value` operands, and a compare whose operand had already been
            // replaced by its constant (`cmp eq v, 1` -> `cmp eq 1, 1`) would
            // escape them for the rest of this visit.
            let return_type = func.return_type;
            let block = &mut func.blocks[blk];
            for inst in &mut block.instructions {
                changes += substitute_singletons(inst, &stack, &in_place);
            }
            if let Terminator::Return(Some(op)) = &mut block.terminator {
                if let Some(c) = singleton_const(&stack, op, return_type) {
                    *op = c;
                    changes += 1;
                }
            }
        }
        if *next < children[blk].len() {
            let child = children[blk][*next];
            *next += 1;
            let f = &entry_facts[child];
            stack.extend(f.iter().cloned());
            pushed.push(f.len());
            work.push((child, 0));
        } else {
            work.pop();
            let k = pushed.pop().unwrap_or(0);
            let new_len = stack.len().saturating_sub(k);
            stack.truncate(new_len);
        }
    }
    changes
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::reexports::{BasicBlock, CallInfo, IrBinOp, IrUnaryOp};

    #[test]
    fn constant_cast_resolution_respects_source_width_and_signedness() {
        let mut f = IrFunction::new("constants".into(), IrType::Void, vec![], false);
        f.blocks = vec![mk(
            0,
            vec![
                Instruction::Cast {
                    dest: Value(1),
                    src: Operand::Const(IrConst::I32(256)),
                    from_ty: IrType::U8,
                    to_ty: IrType::U32,
                },
                Instruction::Cast {
                    dest: Value(2),
                    src: Operand::Const(IrConst::I32(128)),
                    from_ty: IrType::I8,
                    to_ty: IrType::I32,
                },
                Instruction::Cast {
                    dest: Value(3),
                    src: Operand::Const(IrConst::I32(255)),
                    from_ty: IrType::U8,
                    to_ty: IrType::U32,
                },
                Instruction::Cast {
                    dest: Value(4),
                    src: Operand::Const(IrConst::I64(1 << 40)),
                    from_ty: IrType::U64,
                    to_ty: IrType::U8,
                },
            ],
            Terminator::Return(None),
        )];
        let defs = (0..4).map(|i| (Value((i + 1) as u32), (0, i))).collect();
        assert_eq!(
            resolved_constant(&f, &defs, Operand::Value(Value(1))),
            Some((0, 32))
        );
        assert_eq!(
            resolved_constant(&f, &defs, Operand::Value(Value(2))),
            Some((0xffff_ff80, 32))
        );
        assert_eq!(
            resolved_constant(&f, &defs, Operand::Value(Value(3))),
            Some((255, 32))
        );
        assert_eq!(
            resolved_constant(&f, &defs, Operand::Value(Value(4))),
            Some((0, 8))
        );
    }

    #[test]
    fn nonzero_definition_search_checks_both_or_operands_and_is_bounded() {
        let mut f = IrFunction::new("or_search".into(), IrType::Void, vec![], false);
        let mut instructions = vec![
            Instruction::BinOp {
                dest: Value(2),
                op: IrBinOp::Or,
                lhs: Operand::Value(Value(0)),
                rhs: Operand::Const(IrConst::I32(1)),
                ty: IrType::U32,
            },
            Instruction::BinOp {
                dest: Value(3),
                op: IrBinOp::Or,
                lhs: Operand::Value(Value(1)),
                rhs: Operand::Value(Value(2)),
                ty: IrType::U32,
            },
            Instruction::Cast {
                dest: Value(4),
                src: Operand::Const(IrConst::I32(256)),
                from_ty: IrType::U8,
                to_ty: IrType::U32,
            },
            Instruction::BinOp {
                dest: Value(5),
                op: IrBinOp::Or,
                lhs: Operand::Value(Value(0)),
                rhs: Operand::Value(Value(4)),
                ty: IrType::U32,
            },
            Instruction::BinOp {
                dest: Value(6),
                op: IrBinOp::Or,
                lhs: Operand::Value(Value(0)),
                rhs: Operand::Const(IrConst::I128(i128::MIN)),
                ty: IrType::U128,
            },
        ];
        // A long OR chain must fail closed without recursive stack growth.
        for id in 10..310 {
            instructions.push(Instruction::BinOp {
                dest: Value(id),
                op: IrBinOp::Or,
                lhs: Operand::Value(Value(if id == 10 { 0 } else { id - 1 })),
                rhs: Operand::Const(IrConst::I32(0)),
                ty: IrType::U32,
            });
        }
        f.blocks = vec![mk(0, instructions, Terminator::Return(None))];
        let defs = f.blocks[0]
            .instructions
            .iter()
            .enumerate()
            .filter_map(|(i, inst)| inst.dest().map(|v| (v, (0, i))))
            .collect();
        let casts = FxHashMap::default();
        assert!(def_proves_nonzero(&f, &defs, &casts, Value(3)));
        assert!(!def_proves_nonzero(&f, &defs, &casts, Value(5)));
        assert!(def_proves_nonzero(&f, &defs, &casts, Value(6)));
        assert!(!def_proves_nonzero(&f, &defs, &casts, Value(309)));
    }

    #[test]
    fn truth_sets_signed_and_unsigned() {
        assert_eq!(truth_set(IrCmpOp::Ult, 10, 32), vec![(0, 9)]);
        assert_eq!(truth_set(IrCmpOp::Ne, 0, 8), vec![(1, 255)]);
        assert_eq!(truth_set(IrCmpOp::Slt, 0, 8), vec![(128, 255)]);
        assert_eq!(truth_set(IrCmpOp::Sgt, -1, 8), vec![(0, 127)]);
        assert_eq!(truth_set(IrCmpOp::Sle, 5, 8), vec![(0, 5), (128, 255)]);
        assert!(truth_set(IrCmpOp::Ult, 0, 16).is_empty());
        assert_eq!(truth_set(IrCmpOp::Uge, 0, 16), vec![(0, 65535)]);
    }

    #[test]
    fn complement_and_intersection() {
        let s = truth_set(IrCmpOp::Eq, 7, 8);
        assert_eq!(complement(&s, 8), vec![(0, 6), (8, 255)]);
        let a = truth_set(IrCmpOp::Sge, 0, 8);
        let b = truth_set(IrCmpOp::Ult, 100, 8);
        assert_eq!(intersect(&a, &b), vec![(0, 99)]);
        assert!(is_subset(&vec![(3, 4), (10, 10)], &vec![(0, 5), (8, 12)]));
        assert!(!is_subset(&vec![(3, 6)], &vec![(0, 5), (8, 12)]));
    }

    fn pred(op: IrCmpOp, c: i128) -> Pred {
        Pred {
            value: Value(1),
            op,
            c,
            bits: 64,
        }
    }

    #[test]
    fn nonzero_fact_folds_zero_compare() {
        let stack = vec![Fact::from_pred(pred(IrCmpOp::Eq, 0), false)];
        assert_eq!(decide_pred(&stack, pred(IrCmpOp::Eq, 0)), Some(false));
        assert_eq!(decide_pred(&stack, pred(IrCmpOp::Ne, 0)), Some(true));
        assert_eq!(decide_pred(&stack, pred(IrCmpOp::Ugt, 0)), Some(true));
        assert_eq!(decide_pred(&stack, pred(IrCmpOp::Ult, 5)), None);
    }

    #[test]
    fn nonzero_edge_specializes_defined_bitcounts_only_in_dominated_region() {
        // b0: br v, b1, b2
        // b1 is dominated by the nonzero edge, so defined-zero Clz/Ctz can use
        // their branchless nonzero forms. b2 deliberately retains Ctz: the
        // false edge proves zero and changing its semantics would be invalid.
        let v = Value(0);
        let mut f = IrFunction::new("bitcounts".to_string(), IrType::U64, vec![], false);
        f.blocks = vec![
            mk(
                0,
                vec![],
                Terminator::CondBranch {
                    cond: Operand::Value(v),
                    true_label: BlockId(1),
                    false_label: BlockId(2),
                },
            ),
            mk(
                1,
                vec![
                    Instruction::UnaryOp {
                        dest: Value(1),
                        op: crate::ir::reexports::IrUnaryOp::Clz,
                        src: Operand::Value(v),
                        ty: IrType::U64,
                    },
                    // Builtin lowering commonly inserts a signedness-only
                    // cast between the guarded value and bitcount operand.
                    Instruction::Cast {
                        dest: Value(4),
                        src: Operand::Value(v),
                        from_ty: IrType::U64,
                        to_ty: IrType::I64,
                    },
                    Instruction::UnaryOp {
                        dest: Value(2),
                        op: crate::ir::reexports::IrUnaryOp::Ctz,
                        src: Operand::Value(Value(4)),
                        ty: IrType::I64,
                    },
                ],
                Terminator::Return(Some(Operand::Value(Value(2)))),
            ),
            mk(
                2,
                vec![Instruction::UnaryOp {
                    dest: Value(3),
                    op: crate::ir::reexports::IrUnaryOp::Ctz,
                    src: Operand::Value(v),
                    ty: IrType::U64,
                }],
                Terminator::Return(Some(Operand::Value(Value(3)))),
            ),
        ];
        f.next_value_id = 5;
        assert_eq!(run_function_with_symbols(&mut f, &bench_syms()), 2);
        assert!(matches!(
            f.blocks[1].instructions[0],
            Instruction::UnaryOp {
                op: crate::ir::reexports::IrUnaryOp::ClzNonZero,
                ..
            }
        ));
        assert!(matches!(
            f.blocks[1].instructions[2],
            Instruction::UnaryOp {
                op: crate::ir::reexports::IrUnaryOp::CtzNonZero,
                ..
            }
        ));
        assert!(matches!(
            f.blocks[2].instructions[0],
            Instruction::UnaryOp {
                op: crate::ir::reexports::IrUnaryOp::Ctz,
                ..
            }
        ));
        assert_eq!(
            run_function_with_symbols(&mut f, &bench_syms()),
            0,
            "specialization is idempotent"
        );
    }

    #[test]
    fn empty_fact_set_does_not_prove_nonzero() {
        let facts = vec![Fact {
            value: Value(1),
            bits: 8,
            set: Vec::new(),
        }];
        assert!(!proves_nonzero(&facts, Value(1), &FxHashMap::default()));
    }

    #[test]
    fn zero_equivalent_cast_roots_accept_widenings_and_reject_truncations() {
        let mut casts = FxHashMap::default();
        casts.insert(
            Value(2),
            (Operand::Value(Value(1)), IrType::U32, IrType::I64),
        );
        casts.insert(
            Value(3),
            (Operand::Value(Value(2)), IrType::I64, IrType::U8),
        );
        assert_eq!(zero_equivalent_root(Value(2), &casts), Value(1));
        assert_eq!(zero_equivalent_root(Value(3), &casts), Value(3));
    }

    #[test]
    fn signed_unsigned_facts_compose() {
        let stack = vec![
            Fact::from_pred(pred(IrCmpOp::Sgt, -1), true),
            Fact::from_pred(pred(IrCmpOp::Slt, 100), true),
        ];
        assert_eq!(decide_pred(&stack, pred(IrCmpOp::Ult, 100)), Some(true));
        assert_eq!(decide_pred(&stack, pred(IrCmpOp::Uge, 100)), Some(false));
        assert_eq!(decide_pred(&stack, pred(IrCmpOp::Eq, 50)), None);
        // `x <s 100` alone says nothing about `x <u 100` (negatives wrap high).
        let stack2 = vec![Fact::from_pred(pred(IrCmpOp::Slt, 100), true)];
        assert_eq!(decide_pred(&stack2, pred(IrCmpOp::Ult, 100)), None);
    }

    #[test]
    fn mismatched_width_is_not_used() {
        let stack = vec![Fact::from_pred(pred(IrCmpOp::Eq, 0), false)];
        let narrow = Pred {
            value: Value(1),
            op: IrCmpOp::Eq,
            c: 0,
            bits: 32,
        };
        assert_eq!(decide_pred(&stack, narrow), None);
    }

    /// The symbol set the null-compare tests compile against: the pinned
    /// global is defined in the test module's "TU".
    fn bench_syms() -> FxHashSet<String> {
        let mut set = FxHashSet::default();
        set.insert("bench_data".to_string());
        set
    }

    fn global_addr_null_compare_case(op: IrCmpOp, swap: bool) -> IrFunction {
        // b0: v0 = &global; v1 = (v0 OP NULL); br v1, b1, b2
        let lhs = if swap {
            Operand::Const(IrConst::I64(0))
        } else {
            Operand::Value(Value(0))
        };
        let rhs = if swap {
            Operand::Value(Value(0))
        } else {
            Operand::Const(IrConst::I64(0))
        };
        let mut f = IrFunction::new("nullcmp".to_string(), IrType::U64, vec![], false);
        f.blocks = vec![
            mk(
                0,
                vec![
                    Instruction::GlobalAddr {
                        dest: Value(0),
                        name: "bench_data".to_string(),
                    },
                    Instruction::Cmp {
                        dest: Value(1),
                        op,
                        lhs,
                        rhs,
                        ty: IrType::U64,
                    },
                ],
                Terminator::CondBranch {
                    cond: Operand::Value(Value(1)),
                    true_label: BlockId(1),
                    false_label: BlockId(2),
                },
            ),
            mk(
                1,
                vec![],
                Terminator::Return(Some(Operand::Const(IrConst::I64(1)))),
            ),
            mk(
                2,
                vec![],
                Terminator::Return(Some(Operand::Const(IrConst::I64(2)))),
            ),
        ];
        f.next_value_id = 3;
        f
    }

    /// EXPECTED[op] for `p OP NULL` with p provably never null.
    fn expected_verdict(op: IrCmpOp) -> bool {
        match op {
            IrCmpOp::Eq | IrCmpOp::Ult | IrCmpOp::Ule => false,
            IrCmpOp::Ne | IrCmpOp::Ugt | IrCmpOp::Uge => true,
            _ => unreachable!("test only covers unsigned/eq predicates"),
        }
    }

    #[test]
    fn null_compare_matrix_all_12_cases() {
        // The full truth table: 6 unsigned/eq predicates x both operand
        // directions.  Regression: Ule/Ugt were answered backwards for
        // `p OP NULL`, and the swapped direction reused the un-swapped
        // table (2 of 4 ordering cases inverted).  A wrong fold here is a
        // straight miscompile of `if (p > NULL)` style code.
        for op in [
            IrCmpOp::Eq,
            IrCmpOp::Ne,
            IrCmpOp::Ult,
            IrCmpOp::Ule,
            IrCmpOp::Ugt,
            IrCmpOp::Uge,
        ] {
            for swap in [false, true] {
                let want = expected_verdict(if swap { swap_op(op) } else { op });
                let mut f = global_addr_null_compare_case(op, swap);
                run_function_with_symbols(&mut f, &bench_syms());
                let ok = matches!(
                    f.blocks[0].instructions[1],
                    Instruction::Copy {
                        src: Operand::Const(IrConst::I32(c)),
                        ..
                    } if c == want as i32
                );
                assert!(
                    ok,
                    "{op:?} swap={swap}: expected fold to {want}, IR: {:?}",
                    f.blocks[0].instructions[1]
                );
                // The guard must prune to the arm the verdict selects.
                let want_block = if want { BlockId(1) } else { BlockId(2) };
                assert!(
                    matches!(f.blocks[0].terminator, Terminator::Branch(t) if t == want_block),
                    "{op:?} swap={swap}: guard must prune to block {want_block:?}"
                );
            }
        }
    }

    #[test]
    fn global_addr_never_null_folds_eq_null_compare_and_prunes_guard() {
        // &global == NULL is always false (C11 6.5.3.2: every object has a
        // distinct non-null address), so the compare folds to 0 and the
        // guard prunes to the false edge.
        let mut f = global_addr_null_compare_case(IrCmpOp::Eq, false);
        let n = run_function_with_symbols(&mut f, &bench_syms());
        assert!(
            n >= 1,
            "null-compare against &global must fold (folded {n})"
        );
        assert!(matches!(
            f.blocks[0].instructions[1],
            Instruction::Copy {
                src: Operand::Const(IrConst::I32(0)),
                ..
            }
        ));
        // The guard is now decidable and must have pruned to the false edge.
        assert!(matches!(
            f.blocks[0].terminator,
            Terminator::Branch(BlockId(2))
        ));
    }

    #[test]
    fn global_addr_null_law_reaches_through_gvn_copy_residue() {
        // v0 = &global; v2 = copy v0; v3 = (v2 == NULL) — the copy chain
        // (GVN residue) must not hide the never-null fact.
        let mut f = global_addr_null_compare_case(IrCmpOp::Eq, false);
        f.blocks[0].instructions.insert(
            1,
            Instruction::Copy {
                dest: Value(3),
                src: Operand::Value(Value(0)),
            },
        );
        f.blocks[0].instructions[2] = Instruction::Cmp {
            dest: Value(1),
            op: IrCmpOp::Eq,
            lhs: Operand::Value(Value(3)),
            rhs: Operand::Const(IrConst::I64(0)),
            ty: IrType::U64,
        };
        f.next_value_id = 4;
        run_function_with_symbols(&mut f, &bench_syms());
        assert!(matches!(
            f.blocks[0].instructions[2],
            Instruction::Copy {
                src: Operand::Const(IrConst::I32(0)),
                ..
            }
        ));
    }

    #[test]
    fn runtime_value_null_compare_stays_untouched() {
        // A null test on a function PARAMETER has no compile-time verdict;
        // the law must not fold it (soundness of the never-null premise).
        let mut f = global_addr_null_compare_case(IrCmpOp::Eq, false);
        f.blocks[0].instructions[0] = Instruction::Cmp {
            dest: Value(1),
            op: IrCmpOp::Eq,
            lhs: Operand::Value(Value(4)), // undefined/param-ish value
            rhs: Operand::Const(IrConst::I64(0)),
            ty: IrType::U64,
        };
        f.next_value_id = 5;
        let n = run_function_with_symbols(&mut f, &bench_syms());
        assert_eq!(n, 0, "no fold without a never-null producer");
        assert!(matches!(
            f.blocks[0].instructions[0],
            Instruction::Cmp { .. }
        ));
    }

    fn mk(id: u32, insts: Vec<Instruction>, term: Terminator) -> BasicBlock {
        BasicBlock {
            label: BlockId(id),
            instructions: insts,
            terminator: term,
            source_spans: Vec::new(),
        }
    }

    /// Build `return clz/ctz(<guard>)` as a ONE-block leaf, the shape the
    /// frontend emits for `__builtin_clz(x | 1)` and the shape CVP's own body
    /// skips (it needs two blocks). `const_guard` picks the portable zero guard
    /// `x | C`, `via_cast` materialises `C` as `Cast(Const)` like the real
    /// frontend, and `ty` sets the count width.
    fn guarded_count_leaf(
        op: IrUnaryOp,
        guard: IrConst,
        via_cast: bool,
        ty: IrType,
        wrap_in_trunc: bool,
    ) -> IrFunction {
        let mut insts = vec![Instruction::ParamRef {
            dest: Value(1),
            param_idx: 0,
            ty,
        }];
        let mut next = 2u32;
        let guard_op = if via_cast {
            // v2 = (ty)1 (a Cast of a literal, as the C frontend builds it)
            let literal = IrConst::I64(guard.to_i128().unwrap_or(0) as i64);
            insts.push(Instruction::Cast {
                dest: Value(next),
                src: Operand::Const(literal),
                from_ty: ty,
                to_ty: ty,
            });
            next += 1;
            Operand::Value(Value(next - 1))
        } else {
            Operand::Const(guard)
        };
        let safe = next;
        insts.push(Instruction::BinOp {
            dest: Value(safe),
            op: IrBinOp::Or,
            lhs: Operand::Value(Value(1)),
            rhs: guard_op,
            ty,
        });
        next += 1;
        let counted = next;
        insts.push(Instruction::UnaryOp {
            dest: Value(counted),
            op,
            src: Operand::Value(Value(safe)),
            ty,
        });
        next += 1;
        let ret = if wrap_in_trunc {
            // (u8)(count ^ (W-1)) + 1, the reviewed byte_bit_length wrapper:
            // the XOR/ADD pair must not confuse the promotion.
            let mask = next;
            insts.push(Instruction::BinOp {
                dest: Value(mask),
                op: IrBinOp::Xor,
                lhs: Operand::Value(Value(counted)),
                rhs: Operand::Const(IrConst::I32(31)),
                ty: IrType::I32,
            });
            next += 1;
            let plus = next;
            insts.push(Instruction::BinOp {
                dest: Value(plus),
                op: IrBinOp::Add,
                lhs: Operand::Value(Value(mask)),
                rhs: Operand::Const(IrConst::I32(1)),
                ty: IrType::I32,
            });
            next += 1;
            Operand::Value(Value(plus))
        } else {
            Operand::Value(Value(counted))
        };
        let mut f = IrFunction::new("guarded_count".to_string(), ty, vec![], false);
        f.next_value_id = next;
        f.blocks = vec![mk(0, insts, Terminator::Return(Some(ret)))];
        f
    }

    fn count_op(f: &IrFunction) -> IrUnaryOp {
        f.blocks[0]
            .instructions
            .iter()
            .find_map(|i| match i {
                Instruction::UnaryOp { op, .. } => Some(*op),
                _ => None,
            })
            .expect("count instruction")
    }

    #[test]
    fn single_block_zero_guard_is_promoted() {
        // clz(x | 1) really is nonzero: the OR forces bit 0 on for every x.
        let mut f = guarded_count_leaf(IrUnaryOp::Clz, IrConst::I64(1), false, IrType::U32, false);
        assert_eq!(count_op(&f), IrUnaryOp::Clz);
        assert_eq!(promote_guarded_counts(&mut f), 1);
        assert_eq!(count_op(&f), IrUnaryOp::ClzNonZero);
        // The guard instruction is NOT removed: only the opcode changes, so
        // the promoted count keeps the exact value of the original.
        assert!(matches!(
            f.blocks[0].instructions[1],
            Instruction::BinOp {
                op: IrBinOp::Or,
                ..
            }
        ));
        // Idempotent.
        assert_eq!(promote_guarded_counts(&mut f), 0);
    }

    #[test]
    fn frontend_cast_guard_still_promotes() {
        // The C frontend materialises small literals as Cast(Const): the proof
        // must look through that Cast, not only at a bare Const operand.
        let mut f = guarded_count_leaf(
            IrUnaryOp::Ctz,
            IrConst::I64(0x8000_0000),
            true,
            IrType::U32,
            false,
        );
        assert_eq!(promote_guarded_counts(&mut f), 1);
        assert_eq!(count_op(&f), IrUnaryOp::CtzNonZero);
    }

    #[test]
    fn wrapped_bit_length_keeps_its_promotion() {
        // (u8)((clz(x|1)^31)+1) — the reviewed byte_bit_length shape. The XOR
        // and ADD above the count must not hide the guard proof.
        let mut f = guarded_count_leaf(IrUnaryOp::Clz, IrConst::I64(1), true, IrType::U32, true);
        assert_eq!(promote_guarded_counts(&mut f), 1);
        assert_eq!(count_op(&f), IrUnaryOp::ClzNonZero);
    }

    #[test]
    fn zeroing_guards_are_not_promoted() {
        // A guard constant of zero sets no bit: `x | 0 == x` can still be zero,
        // so promoting would hand the lowering an input it may not assume.
        let mut f = guarded_count_leaf(IrUnaryOp::Clz, IrConst::I64(0), false, IrType::U32, false);
        assert_eq!(promote_guarded_counts(&mut f), 0);
        assert_eq!(count_op(&f), IrUnaryOp::Clz);
        // An `And` with a nonzero constant proves nothing either: it can clear
        // every bit of x.
        let mut g = IrFunction::new("and_guard".to_string(), IrType::U32, vec![], false);
        g.next_value_id = 5;
        g.blocks = vec![mk(
            0,
            vec![
                Instruction::ParamRef {
                    dest: Value(1),
                    param_idx: 0,
                    ty: IrType::U32,
                },
                Instruction::BinOp {
                    dest: Value(2),
                    op: IrBinOp::And,
                    lhs: Operand::Value(Value(1)),
                    rhs: Operand::Const(IrConst::I32(0x100)),
                    ty: IrType::U32,
                },
                Instruction::UnaryOp {
                    dest: Value(3),
                    op: IrUnaryOp::Ctz,
                    src: Operand::Value(Value(2)),
                    ty: IrType::U32,
                },
            ],
            Terminator::Return(Some(Operand::Value(Value(3)))),
        )];
        assert_eq!(promote_guarded_counts(&mut g), 0);
        assert_eq!(count_op(&g), IrUnaryOp::Ctz);
    }

    #[test]
    fn bare_param_has_no_definition_proof() {
        // No guard at all: ctz(x) must keep its defined-zero contract.
        let mut f = IrFunction::new("bare".to_string(), IrType::U32, vec![], false);
        f.next_value_id = 3;
        f.blocks = vec![mk(
            0,
            vec![
                Instruction::ParamRef {
                    dest: Value(1),
                    param_idx: 0,
                    ty: IrType::U32,
                },
                Instruction::UnaryOp {
                    dest: Value(2),
                    op: IrUnaryOp::Ctz,
                    src: Operand::Value(Value(1)),
                    ty: IrType::U32,
                },
            ],
            Terminator::Return(Some(Operand::Value(Value(2)))),
        )];
        assert_eq!(promote_guarded_counts(&mut f), 0);
    }

    #[test]
    fn nested_or_guard_is_promoted() {
        // or(or(x, 0), 1): the proof may sit one level down.
        let mut f = IrFunction::new("nested".to_string(), IrType::U32, vec![], false);
        f.next_value_id = 6;
        f.blocks = vec![mk(
            0,
            vec![
                Instruction::ParamRef {
                    dest: Value(1),
                    param_idx: 0,
                    ty: IrType::U32,
                },
                Instruction::BinOp {
                    dest: Value(2),
                    op: IrBinOp::Or,
                    lhs: Operand::Value(Value(1)),
                    rhs: Operand::Const(IrConst::I32(0)),
                    ty: IrType::U32,
                },
                Instruction::BinOp {
                    dest: Value(3),
                    op: IrBinOp::Or,
                    lhs: Operand::Value(Value(2)),
                    rhs: Operand::Const(IrConst::I32(4)),
                    ty: IrType::U32,
                },
                Instruction::UnaryOp {
                    dest: Value(4),
                    op: IrUnaryOp::Clz,
                    src: Operand::Value(Value(3)),
                    ty: IrType::U32,
                },
            ],
            Terminator::Return(Some(Operand::Value(Value(4)))),
        )];
        assert_eq!(promote_guarded_counts(&mut f), 1);
        assert_eq!(count_op(&f), IrUnaryOp::ClzNonZero);
    }

    fn cmp0(d: u32, op: IrCmpOp) -> Instruction {
        Instruction::Cmp {
            dest: Value(d),
            op,
            lhs: Operand::Value(Value(0)),
            rhs: Operand::Const(IrConst::I64(0)),
            ty: IrType::U64,
        }
    }

    fn add1(d: u32, lhs: Operand) -> Instruction {
        Instruction::BinOp {
            dest: Value(d),
            op: crate::ir::reexports::IrBinOp::Add,
            lhs,
            rhs: Operand::Const(IrConst::I64(1)),
            ty: IrType::U64,
        }
    }

    fn sel(d: u32, cond: u32, t: Operand, f: Operand) -> Instruction {
        Instruction::Select {
            dest: Value(d),
            cond: Operand::Value(Value(cond)),
            true_val: t,
            false_val: f,
            ty: IrType::U64,
        }
    }

    #[test]
    fn select_use_under_correlated_branch_rewrites_to_known_arm() {
        // The select-before-branch order: b0 computes `s = select c,63,v0`
        // then branches on `c`; both arms use `s` and each sees its arm.
        // b1 covers an instruction operand, b2 a terminator (ret) operand.
        let v0 = Operand::Value(Value(0));
        let mut f = IrFunction::new("t".to_string(), IrType::U64, vec![], false);
        f.blocks = vec![
            mk(
                0,
                vec![
                    cmp0(1, IrCmpOp::Eq),
                    sel(3, 1, Operand::Const(IrConst::I64(63)), v0),
                ],
                Terminator::CondBranch {
                    cond: Operand::Value(Value(1)),
                    true_label: BlockId(1),
                    false_label: BlockId(2),
                },
            ),
            mk(
                1,
                vec![add1(4, Operand::Value(Value(3)))],
                Terminator::Return(Some(Operand::Value(Value(4)))),
            ),
            mk(
                2,
                vec![],
                Terminator::Return(Some(Operand::Value(Value(3)))),
            ),
        ];
        f.next_value_id = 5;
        let n = run_function_with_symbols(&mut f, &bench_syms());
        assert_eq!(n, 2, "one instruction use, one terminator use");
        assert!(matches!(
            f.blocks[1].instructions[0],
            Instruction::BinOp {
                lhs: Operand::Const(IrConst::I64(63)),
                ..
            }
        ));
        assert!(matches!(
            f.blocks[2].terminator,
            Terminator::Return(Some(Operand::Value(Value(0))))
        ));
        assert_eq!(
            run_function_with_symbols(&mut f, &bench_syms()),
            0,
            "idempotent"
        );
    }

    #[test]
    fn select_use_behind_copy_branch_condition_rewrites() {
        // GVN residue between the compare and its branch (`br %6`,
        // `%6 = copy %30`) must not hide the edge fact: the float
        // compares never canonicalize, so the resolved value fact on
        // the compare destination is their only channel (libm trunc).
        let v0 = Operand::Value(Value(0));
        let mut f = IrFunction::new("t".to_string(), IrType::U64, vec![], false);
        f.blocks = vec![
            mk(
                0,
                vec![
                    cmp0(30, IrCmpOp::Eq),
                    sel(3, 30, Operand::Const(IrConst::I64(63)), v0),
                    Instruction::Copy {
                        dest: Value(6),
                        src: Operand::Value(Value(30)),
                    },
                ],
                Terminator::CondBranch {
                    cond: Operand::Value(Value(6)),
                    true_label: BlockId(1),
                    false_label: BlockId(2),
                },
            ),
            mk(
                1,
                vec![add1(4, Operand::Value(Value(3)))],
                Terminator::Return(Some(Operand::Value(Value(4)))),
            ),
            mk(
                2,
                vec![],
                Terminator::Return(Some(Operand::Value(Value(3)))),
            ),
        ];
        f.next_value_id = 31;
        let n = run_function_with_symbols(&mut f, &bench_syms());
        assert_eq!(n, 2, "edge fact resolves through the copy");
        assert!(matches!(
            f.blocks[1].instructions[0],
            Instruction::BinOp {
                lhs: Operand::Const(IrConst::I64(63)),
                ..
            }
        ));
        assert!(matches!(
            f.blocks[2].terminator,
            Terminator::Return(Some(Operand::Value(Value(0))))
        ));
        assert_eq!(
            run_function_with_symbols(&mut f, &bench_syms()),
            0,
            "idempotent"
        );
    }

    #[test]
    fn select_use_under_unknown_branch_is_kept() {
        // A merge dominated by neither edge decides nothing: the use of
        // the select in b3 keeps the select.
        let v0 = Operand::Value(Value(0));
        let mut f = IrFunction::new("t".to_string(), IrType::U64, vec![], false);
        f.blocks = vec![
            mk(
                0,
                vec![
                    cmp0(1, IrCmpOp::Eq),
                    sel(3, 1, Operand::Const(IrConst::I64(63)), v0),
                ],
                Terminator::CondBranch {
                    cond: Operand::Value(Value(1)),
                    true_label: BlockId(1),
                    false_label: BlockId(2),
                },
            ),
            mk(1, vec![], Terminator::Branch(BlockId(3))),
            mk(2, vec![], Terminator::Branch(BlockId(3))),
            mk(
                3,
                vec![add1(4, Operand::Value(Value(3)))],
                Terminator::Return(Some(Operand::Value(Value(4)))),
            ),
        ];
        f.next_value_id = 5;
        assert_eq!(
            run_function_with_symbols(&mut f, &bench_syms()),
            0,
            "merge decides nothing"
        );
        assert!(matches!(
            f.blocks[3].instructions[0],
            Instruction::BinOp {
                lhs: Operand::Value(Value(3)),
                ..
            }
        ));
    }

    #[test]
    fn phi_incoming_of_decided_select_is_kept() {
        // Phi incomings are edge-flavored, never block facts: even in a
        // block whose facts decide the select's condition, an incoming
        // naming the select is left alone (conservative and documented).
        let v0 = Operand::Value(Value(0));
        let mut f = IrFunction::new("t".to_string(), IrType::U64, vec![], false);
        f.blocks = vec![
            mk(
                0,
                vec![
                    cmp0(1, IrCmpOp::Eq),
                    sel(3, 1, Operand::Const(IrConst::I64(63)), v0),
                ],
                Terminator::CondBranch {
                    cond: Operand::Value(Value(1)),
                    true_label: BlockId(1),
                    false_label: BlockId(2),
                },
            ),
            mk(
                1,
                vec![Instruction::Phi {
                    dest: Value(5),
                    incoming: vec![(Operand::Value(Value(3)), BlockId(0))],
                    ty: IrType::U64,
                }],
                Terminator::Return(Some(Operand::Value(Value(5)))),
            ),
            mk(
                2,
                vec![],
                Terminator::Return(Some(Operand::Const(IrConst::I64(0)))),
            ),
        ];
        f.next_value_id = 6;
        assert_eq!(
            run_function_with_symbols(&mut f, &bench_syms()),
            0,
            "phi incomings are never rewritten"
        );
        if let Instruction::Phi { incoming, .. } = &f.blocks[1].instructions[0] {
            assert_eq!(incoming, &vec![(Operand::Value(Value(3)), BlockId(0))]);
        } else {
            panic!("phi survived");
        }
    }

    #[test]
    fn nested_select_arms_rewrite_to_fresh_arms_in_one_visit() {
        // S2's own arm is rewritten under b1's facts before any later
        // block reads S2's triple: uses in b3/b4 see the fresh arm in
        // the SAME run (no second fixpoint iteration needed).
        // b0: c = cmp eq v0,0; S1 = select c,63,v0; br c,b1,b2
        // b1: d2 = cmp ne v0,v0 (undecidable: keeps def-folds out);
        //     S2 = select d2,S1,7; br d2,b3,b4
        // b3: ret S2  (→ 63)   b4: ret S2  (→ 7)   b2: ret 0
        let v0 = Operand::Value(Value(0));
        let mut f = IrFunction::new("t".to_string(), IrType::U64, vec![], false);
        f.blocks = vec![
            mk(
                0,
                vec![
                    cmp0(1, IrCmpOp::Eq),
                    sel(3, 1, Operand::Const(IrConst::I64(63)), v0),
                ],
                Terminator::CondBranch {
                    cond: Operand::Value(Value(1)),
                    true_label: BlockId(1),
                    false_label: BlockId(2),
                },
            ),
            mk(
                1,
                vec![
                    Instruction::Cmp {
                        dest: Value(6),
                        op: IrCmpOp::Ne,
                        lhs: Operand::Value(Value(0)),
                        rhs: Operand::Value(Value(0)),
                        ty: IrType::U64,
                    },
                    sel(
                        7,
                        6,
                        Operand::Value(Value(3)),
                        Operand::Const(IrConst::I64(7)),
                    ),
                ],
                Terminator::CondBranch {
                    cond: Operand::Value(Value(6)),
                    true_label: BlockId(3),
                    false_label: BlockId(4),
                },
            ),
            mk(
                2,
                vec![],
                Terminator::Return(Some(Operand::Const(IrConst::I64(0)))),
            ),
            mk(
                3,
                vec![],
                Terminator::Return(Some(Operand::Value(Value(7)))),
            ),
            mk(
                4,
                vec![],
                Terminator::Return(Some(Operand::Value(Value(7)))),
            ),
        ];
        f.next_value_id = 8;
        let n = run_function_with_symbols(&mut f, &bench_syms());
        // Plus singleton substitution of d2's operands: b1 lies on b0's
        // `v0 == 0` edge, so `cmp ne v0, v0` reads `cmp ne 0, 0` (the select
        // rewrites are the subject; d2 stays a Cmp, so no def-fold interferes).
        assert_eq!(n, 5, "S2 arm, b3 use, b4 use, d2 lhs, d2 rhs");
        assert!(matches!(
            f.blocks[1].instructions[0],
            Instruction::Cmp {
                lhs: Operand::Const(IrConst::I64(0)),
                rhs: Operand::Const(IrConst::I64(0)),
                ..
            }
        ));
        assert!(matches!(
            f.blocks[1].instructions[1],
            Instruction::Select {
                true_val: Operand::Const(IrConst::I64(63)),
                ..
            }
        ));
        assert!(matches!(
            f.blocks[3].terminator,
            Terminator::Return(Some(Operand::Const(IrConst::I64(63))))
        ));
        assert!(matches!(
            f.blocks[4].terminator,
            Terminator::Return(Some(Operand::Const(IrConst::I64(7))))
        ));
        assert_eq!(
            run_function_with_symbols(&mut f, &bench_syms()),
            0,
            "idempotent"
        );
    }

    /// `b0: c = cmp ne v, 0; br c, b1, b2` with `b2` (the `v == 0` arm)
    /// holding `insts` and returning `ret`.
    fn zero_arm(insts: Vec<Instruction>, ret: Operand) -> IrFunction {
        let mut f = IrFunction::new("t".to_string(), IrType::U64, vec![], false);
        f.blocks = vec![
            mk(
                0,
                vec![cmp0(1, IrCmpOp::Ne)],
                Terminator::CondBranch {
                    cond: Operand::Value(Value(1)),
                    true_label: BlockId(1),
                    false_label: BlockId(2),
                },
            ),
            mk(
                1,
                vec![],
                Terminator::Return(Some(Operand::Value(Value(0)))),
            ),
            mk(2, insts, Terminator::Return(Some(ret))),
        ];
        f.next_value_id = 20;
        f
    }

    fn call_g(dest: u32, args: Vec<Operand>, arg_types: Vec<IrType>) -> Instruction {
        let n = args.len();
        Instruction::Call {
            func: "g".to_string(),
            info: CallInfo {
                dest: Some(Value(dest)),
                args,
                arg_types,
                return_type: IrType::U64,
                struct_arg_sizes: vec![None; n],
                ..CallInfo::default()
            },
        }
    }

    #[test]
    fn singleton_fact_substitutes_call_args_compares_and_return() {
        let v = Operand::Value(Value(0));
        let mut f = zero_arm(
            vec![
                call_g(
                    5,
                    vec![Operand::Value(Value(9)), v],
                    vec![IrType::U64, IrType::U64],
                ),
                Instruction::Cmp {
                    dest: Value(6),
                    op: IrCmpOp::Ult,
                    lhs: Operand::Value(Value(9)),
                    rhs: v,
                    ty: IrType::U64,
                },
            ],
            v,
        );
        let n = run_function_with_symbols(&mut f, &bench_syms());
        let Instruction::Call { info, .. } = &f.blocks[2].instructions[0] else {
            panic!("call moved");
        };
        assert!(matches!(info.args[1], Operand::Const(IrConst::I64(0))));
        assert!(matches!(info.args[0], Operand::Value(Value(9))));
        assert!(matches!(
            f.blocks[2].terminator,
            Terminator::Return(Some(Operand::Const(IrConst::I64(0))))
        ));
        // The v != 0 arm learns nothing singleton: its `ret v` stays.
        assert!(matches!(
            f.blocks[1].terminator,
            Terminator::Return(Some(Operand::Value(Value(0))))
        ));
        assert!(n >= 3, "call arg, compare operand and return: {n}");
    }

    #[test]
    fn direct_branch_zero_edge_pins_a_narrow_use() {
        // `int w6(int a, ..) { if (a) return g3(a, ..); return g3(b, a, ..); }`
        // branches on `a` itself: the false edge knows `a == 0` at any width;
        // the true edge (nonzero on a 64-bit carrier) pins nothing.
        let mut f = IrFunction::new("t".to_string(), IrType::I32, vec![], false);
        let arm = |id: u32| {
            mk(
                id,
                vec![call_g(
                    5 + id,
                    vec![Operand::Value(Value(0))],
                    vec![IrType::I32],
                )],
                Terminator::Return(Some(Operand::Value(Value(5 + id)))),
            )
        };
        f.blocks = vec![
            mk(
                0,
                vec![],
                Terminator::CondBranch {
                    cond: Operand::Value(Value(0)),
                    true_label: BlockId(1),
                    false_label: BlockId(2),
                },
            ),
            arm(1),
            arm(2),
        ];
        f.next_value_id = 20;
        run_function_with_symbols(&mut f, &bench_syms());
        let arg = |b: usize| match &f.blocks[b].instructions[0] {
            Instruction::Call { info, .. } => info.args[0],
            other => panic!("call moved: {other:?}"),
        };
        assert!(
            matches!(arg(2), Operand::Const(IrConst::I32(0))),
            "{:?}",
            arg(2)
        );
        assert!(matches!(arg(1), Operand::Value(Value(0))), "{:?}", arg(1));
    }

    #[test]
    fn forwarded_parameter_keeps_its_register() {
        // `void w4(int c, int x, int y) { if (c) v2(x, y); else v2(c, y); }`:
        // on the `c == 0` arm `c` is parameter 0 passed as argument 0, already
        // in the argument register, so the constant would cost an `xorl`. The
        // same value passed in ANOTHER slot is substituted.
        let param = |ty| crate::ir::reexports::IrParam {
            ty,
            noalias: false,
            struct_size: None,
            struct_align: None,
            param_align: None,
            struct_eightbyte_classes: Vec::new(),
            riscv_float_class: None,
            is_f128_sse: false,
        };
        let mut f = IrFunction::new(
            "w4".to_string(),
            IrType::I32,
            vec![param(IrType::I32), param(IrType::F64), param(IrType::I32)],
            false,
        );
        let c = || Operand::Value(Value(0));
        let y = || Operand::Value(Value(2));
        f.blocks = vec![
            mk(
                0,
                vec![
                    Instruction::ParamRef {
                        dest: Value(0),
                        param_idx: 0,
                        ty: IrType::I32,
                    },
                    Instruction::ParamRef {
                        dest: Value(2),
                        param_idx: 2,
                        ty: IrType::I32,
                    },
                ],
                Terminator::CondBranch {
                    cond: c(),
                    true_label: BlockId(1),
                    false_label: BlockId(2),
                },
            ),
            mk(1, vec![], Terminator::Return(Some(y()))),
            mk(
                2,
                vec![
                    // A double ahead of `c` takes no integer slot: `c` is
                    // still integer argument 0.
                    call_g(
                        5,
                        vec![Operand::Const(IrConst::F64(1.0)), c(), y()],
                        vec![IrType::F64, IrType::I32, IrType::I32],
                    ),
                    call_g(6, vec![y(), c()], vec![IrType::I32, IrType::I32]),
                ],
                Terminator::Return(Some(y())),
            ),
        ];
        f.next_value_id = 20;
        run_function_with_symbols(&mut f, &bench_syms());
        let args = |i: usize| match &f.blocks[2].instructions[i] {
            Instruction::Call { info, .. } => info.args.clone(),
            other => panic!("call moved: {other:?}"),
        };
        assert!(
            matches!(args(0)[1], Operand::Value(Value(0))),
            "{:?}",
            args(0)
        );
        assert!(
            matches!(args(1)[1], Operand::Const(IrConst::I32(0))),
            "{:?}",
            args(1)
        );
    }

    #[test]
    fn singleton_never_reaches_divisors_shift_counts_aggregates_or_phis() {
        let v = Operand::Value(Value(0));
        let x = Operand::Value(Value(9));
        let bin = |d: u32, op: IrBinOp, lhs: Operand, rhs: Operand| Instruction::BinOp {
            dest: Value(d),
            op,
            lhs,
            rhs,
            ty: IrType::U64,
        };
        let mut agg = call_g(8, vec![v], vec![IrType::U64]);
        if let Instruction::Call { info, .. } = &mut agg {
            info.struct_arg_sizes = vec![Some(8)];
        }
        let mut f = zero_arm(
            vec![
                bin(5, IrBinOp::UDiv, x, v),
                bin(6, IrBinOp::Shl, x, v),
                bin(7, IrBinOp::SRem, v, x),
                agg,
            ],
            x,
        );
        run_function_with_symbols(&mut f, &bench_syms());
        let insts = &f.blocks[2].instructions;
        assert!(matches!(
            insts[0],
            Instruction::BinOp {
                rhs: Operand::Value(Value(0)),
                ..
            }
        ));
        assert!(matches!(
            insts[1],
            Instruction::BinOp {
                rhs: Operand::Value(Value(0)),
                ..
            }
        ));
        // The dividend is ordinary data: substituted.
        assert!(matches!(
            insts[2],
            Instruction::BinOp {
                lhs: Operand::Const(IrConst::I64(0)),
                ..
            }
        ));
        let Instruction::Call { info, .. } = &insts[3] else {
            panic!("call moved");
        };
        assert!(matches!(info.args[0], Operand::Value(Value(0))));
    }

    #[test]
    fn singleton_respects_width_and_the_imm32_budget() {
        // A 32-bit fact says nothing about a 64-bit use of the same value.
        let mut f = zero_arm(
            vec![call_g(5, vec![Operand::Value(Value(0))], vec![IrType::U64])],
            Operand::Const(IrConst::I64(1)),
        );
        if let Instruction::Cmp { ty, rhs, .. } = &mut f.blocks[0].instructions[0] {
            *ty = IrType::U32;
            *rhs = Operand::Const(IrConst::I32(0));
        }
        run_function_with_symbols(&mut f, &bench_syms());
        let Instruction::Call { info, .. } = &f.blocks[2].instructions[0] else {
            panic!("call moved");
        };
        assert!(matches!(info.args[0], Operand::Value(Value(0))));
        // `v == 2^40` pins v, but a movabs per use costs more than the register.
        let mut f = zero_arm(
            vec![call_g(5, vec![Operand::Value(Value(0))], vec![IrType::U64])],
            Operand::Const(IrConst::I64(1)),
        );
        if let Instruction::Cmp { rhs, .. } = &mut f.blocks[0].instructions[0] {
            *rhs = Operand::Const(IrConst::I64(1 << 40));
        }
        run_function_with_symbols(&mut f, &bench_syms());
        let Instruction::Call { info, .. } = &f.blocks[2].instructions[0] else {
            panic!("call moved");
        };
        assert!(matches!(info.args[0], Operand::Value(Value(0))));
        // A negative 32-bit singleton is re-signed into the type's spelling.
        let mut f = zero_arm(
            vec![call_g(5, vec![Operand::Value(Value(0))], vec![IrType::I32])],
            Operand::Const(IrConst::I64(1)),
        );
        if let Instruction::Cmp { ty, rhs, .. } = &mut f.blocks[0].instructions[0] {
            *ty = IrType::I32;
            *rhs = Operand::Const(IrConst::I32(-5));
        }
        run_function_with_symbols(&mut f, &bench_syms());
        let Instruction::Call { info, .. } = &f.blocks[2].instructions[0] else {
            panic!("call moved");
        };
        assert!(
            matches!(info.args[0], Operand::Const(IrConst::I32(-5))),
            "{:?}",
            info.args[0]
        );
    }

    #[test]
    fn full_function_folds_guard_and_branch() {
        // b0: c = cmp eq v, 0 ; br c, b1, b2
        // b1: ret v
        // b2: g = cmp eq v, 0 ; s = select g, 63, v ; c2 = cmp ne v, 0 ; br c2, b3, b4
        // b3: ret s      b4: ret 0
        let v = Value(0);
        let mut f = IrFunction::new("t".to_string(), IrType::U64, vec![], false);
        f.blocks = vec![
            mk(
                0,
                vec![cmp0(1, IrCmpOp::Eq)],
                Terminator::CondBranch {
                    cond: Operand::Value(Value(1)),
                    true_label: BlockId(1),
                    false_label: BlockId(2),
                },
            ),
            mk(1, vec![], Terminator::Return(Some(Operand::Value(v)))),
            mk(
                2,
                vec![
                    cmp0(2, IrCmpOp::Eq),
                    Instruction::Select {
                        dest: Value(3),
                        cond: Operand::Value(Value(2)),
                        true_val: Operand::Const(IrConst::I64(63)),
                        false_val: Operand::Value(v),
                        ty: IrType::U64,
                    },
                    cmp0(4, IrCmpOp::Ne),
                ],
                Terminator::CondBranch {
                    cond: Operand::Value(Value(4)),
                    true_label: BlockId(3),
                    false_label: BlockId(4),
                },
            ),
            mk(
                3,
                vec![],
                Terminator::Return(Some(Operand::Value(Value(3)))),
            ),
            mk(
                4,
                vec![],
                Terminator::Return(Some(Operand::Const(IrConst::I64(0)))),
            ),
        ];
        f.next_value_id = 5;
        let n = run_function_with_symbols(&mut f, &bench_syms());
        // guard cmp, select, second cmp, branch, the correlated-select use
        // rewrite of `ret s` in b3 (the guard is false there, so s is v), and
        // the singleton substitution of `ret v` in b1 (`v == 0` there).
        assert_eq!(
            n, 6,
            "guard cmp, select, second cmp, branch, ret use, singleton ret"
        );
        assert!(matches!(
            f.blocks[1].terminator,
            Terminator::Return(Some(Operand::Const(IrConst::I64(0))))
        ));
        assert!(matches!(
            f.blocks[2].instructions[0],
            Instruction::Copy {
                src: Operand::Const(IrConst::I32(0)),
                ..
            }
        ));
        assert!(matches!(
            f.blocks[2].instructions[1],
            Instruction::Copy {
                src: Operand::Value(Value(0)),
                ..
            }
        ));
        assert!(matches!(
            f.blocks[2].terminator,
            Terminator::Branch(BlockId(3))
        ));
        assert!(matches!(
            f.blocks[3].terminator,
            Terminator::Return(Some(Operand::Value(Value(0))))
        ));
        assert_eq!(
            run_function_with_symbols(&mut f, &bench_syms()),
            0,
            "idempotent"
        );
    }

    #[test]
    fn two_agreeing_edges_carry_the_fact_and_phi_is_maintained() {
        // b0: c = cmp ne v, 0 ; br c, b3, b1
        // b1: c1 = cmp ne v, 0 ; br c1, b3, b2      (second edge, same fact)
        // b2: ret 0
        // b3: p = phi [1,b0],[2,b1] ; g = cmp eq v,0 ; br g, b4, b5
        // b4: ret 9   b5: ret p
        let v = Value(0);
        let cmpne = |d: u32| cmp0(d, IrCmpOp::Ne);
        let mut f = IrFunction::new("t".to_string(), IrType::U64, vec![], false);
        f.blocks = vec![
            mk(
                0,
                vec![cmpne(1)],
                Terminator::CondBranch {
                    cond: Operand::Value(Value(1)),
                    true_label: BlockId(3),
                    false_label: BlockId(1),
                },
            ),
            mk(
                1,
                vec![cmpne(2)],
                Terminator::CondBranch {
                    cond: Operand::Value(Value(2)),
                    true_label: BlockId(3),
                    false_label: BlockId(2),
                },
            ),
            mk(
                2,
                vec![],
                Terminator::Return(Some(Operand::Const(IrConst::I64(0)))),
            ),
            mk(
                3,
                vec![
                    Instruction::Phi {
                        dest: Value(3),
                        ty: IrType::U64,
                        incoming: vec![
                            (Operand::Const(IrConst::I64(1)), BlockId(0)),
                            (Operand::Const(IrConst::I64(2)), BlockId(1)),
                        ],
                    },
                    cmp0(4, IrCmpOp::Eq),
                ],
                Terminator::CondBranch {
                    cond: Operand::Value(Value(4)),
                    true_label: BlockId(4),
                    false_label: BlockId(5),
                },
            ),
            mk(
                4,
                vec![],
                Terminator::Return(Some(Operand::Const(IrConst::I64(9)))),
            ),
            mk(5, vec![], Terminator::Return(Some(Operand::Value(v)))),
        ];
        f.next_value_id = 5;
        let n = run_function_with_symbols(&mut f, &bench_syms());
        // b1 is on the false edge of (v != 0): v == 0 there, so its own
        // re-test folds to false and its branch becomes `br b2`; b3's phi
        // loses the b1 incoming.  b3 is then reached only from b0 with
        // v != 0, so the guard folds and the branch goes to b5.
        assert_eq!(n, 4);
        assert!(matches!(
            f.blocks[1].terminator,
            Terminator::Branch(BlockId(2))
        ));
        assert!(matches!(
            f.blocks[3].terminator,
            Terminator::Branch(BlockId(5))
        ));
        if let Instruction::Phi { incoming, .. } = &f.blocks[3].instructions[0] {
            assert_eq!(incoming.len(), 1);
            assert_eq!(incoming[0].1, BlockId(0));
        } else {
            panic!("phi expected");
        }
    }

    #[test]
    fn switch_edges_carry_case_facts() {
        // b0: switch v [1→b1, 2→b2] default b3
        // b1: c = cmp eq v, 1 ; ret c        (true)
        // b2: c = cmp eq v, 1 ; ret c        (false)
        // b3: c = cmp eq v, 2 ; ret c        (false: default excludes 1 and 2)
        let v = Value(0);
        let mut f = IrFunction::new("t".to_string(), IrType::I32, vec![], false);
        let cmpk = |d: u32, k: i32| Instruction::Cmp {
            dest: Value(d),
            op: IrCmpOp::Eq,
            lhs: Operand::Value(v),
            rhs: Operand::Const(IrConst::I32(k)),
            ty: IrType::I32,
        };
        f.blocks = vec![
            mk(
                0,
                vec![],
                Terminator::Switch {
                    val: Operand::Value(v),
                    cases: vec![(1, BlockId(1)), (2, BlockId(2))],
                    default: BlockId(3),
                    ty: IrType::I32,
                },
            ),
            mk(
                1,
                vec![cmpk(1, 1)],
                Terminator::Return(Some(Operand::Value(Value(1)))),
            ),
            mk(
                2,
                vec![cmpk(2, 1)],
                Terminator::Return(Some(Operand::Value(Value(2)))),
            ),
            mk(
                3,
                vec![cmpk(3, 2)],
                Terminator::Return(Some(Operand::Value(Value(3)))),
            ),
        ];
        f.next_value_id = 4;
        assert_eq!(run_function_with_symbols(&mut f, &bench_syms()), 3);
        let val = |b: usize| match &f.blocks[b].instructions[0] {
            Instruction::Copy {
                src: Operand::Const(IrConst::I32(k)),
                ..
            } => *k,
            other => panic!("unexpected {other:?}"),
        };
        assert_eq!((val(1), val(2), val(3)), (1, 0, 0));
    }
}

#[cfg(test)]
mod safe_clz_review_tests {
    use super::*;
    use crate::ir::reexports::{BasicBlock, IrUnaryOp};
    #[test]
    fn nonzero_or_bypass_is_dominance_and_width_scoped() {
        for (op, bit) in [
            (IrUnaryOp::ClzNonZero, 1),
            (IrUnaryOp::CtzNonZero, 1i64 << 31),
        ] {
            for truncate in [false, true] {
                let v = |n| Operand::Value(Value(n));
                let mut f = IrFunction::new("guarded_safe_clz".into(), IrType::U32, vec![], false);
                let mut entry = vec![Instruction::ParamRef {
                    dest: Value(0),
                    param_idx: 0,
                    ty: IrType::U64,
                }];
                entry.push(Instruction::Cast {
                    dest: Value(1),
                    src: v(0),
                    from_ty: IrType::U64,
                    to_ty: IrType::U32,
                });
                let block = |n, ins, term| BasicBlock {
                    label: BlockId(n),
                    instructions: ins,
                    source_spans: vec![],
                    terminator: term,
                };
                let body = |base| {
                    vec![
                        Instruction::BinOp {
                            dest: Value(base),
                            op: IrBinOp::Or,
                            lhs: v(1),
                            rhs: Operand::Const(IrConst::I64(bit)),
                            ty: IrType::U32,
                        },
                        Instruction::UnaryOp {
                            dest: Value(base + 1),
                            op,
                            src: v(base),
                            ty: IrType::U32,
                        },
                    ]
                };
                f.blocks = vec![
                    block(
                        0,
                        entry,
                        Terminator::CondBranch {
                            cond: v(if truncate { 0 } else { 1 }),
                            true_label: BlockId(1),
                            false_label: BlockId(2),
                        },
                    ),
                    block(1, body(2), Terminator::Return(Some(v(3)))),
                    block(2, body(4), Terminator::Return(Some(v(5)))),
                ];
                // Branching on the U32 value itself, the false edge pins it to
                // zero: b2's `or v1, bit` becomes `or 0, bit` (singleton
                // substitution) alongside b1's bypass rewrite.  Branching on
                // the U64 source pins nothing about the truncation's width.
                assert_eq!(run_function(&mut f), 2 * usize::from(!truncate));
                let b2_lhs = match &f.blocks[2].instructions[0] {
                    Instruction::BinOp { lhs, .. } => *lhs,
                    other => panic!("or moved: {other:?}"),
                };
                if truncate {
                    assert_eq!(b2_lhs, v(1));
                } else {
                    assert!(matches!(b2_lhs, Operand::Const(c) if c.to_i64() == Some(0)));
                }
                let expected = if truncate { v(2) } else { v(1) };
                assert!(
                    matches!(f.blocks[1].instructions[1],Instruction::UnaryOp {src,..} if src==expected)
                );
                assert!(
                    matches!(f.blocks[2].instructions[1],Instruction::UnaryOp {src,..} if src==v(4))
                );
                assert_eq!(run_function(&mut f), 0);
            }
        }
    }
}
