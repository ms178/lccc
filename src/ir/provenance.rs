//! Pointer-provenance model for optimizer root tracers.
//!
//! [`root_leaf`], [`chain_step`], and [`produces_pointer`] are the single
//! home of the rules every alias-analysis root tracer must obey. Both
//! `vectorize::proven_object_root` and `loop_idiom::object_root_inner` are
//! written against this module so the leaf conditions cannot drift apart.
//!
//! # The IR's de-facto provenance model
//!
//! The frontend erases all integer↔pointer conversions: `emit_implicit_cast`
//! (`src/ir/lowering/expr.rs`) returns the source operand unchanged whenever
//! either side is `Ptr`, so `(char *)raw` and `(u64)p` lower to the *same
//! SSA value* (later plumbing may wrap it in an untyped [`Instruction::Copy`],
//! which is equally transparent). No production pass synthesizes an
//! integer↔pointer [`Instruction::Cast`] either — only test fixtures do.
//!
//! Consequences, each load-bearing for the rules below:
//!
//! 1. **Pointer-ness is a leaf property.** A chain's value designates an
//!    object only if the chain *ends* at a pointer-typed leaf
//!    ([`Instruction::ParamRef`] with `ty == Ptr`, [`Instruction::Alloca`],
//!    [`Instruction::GlobalAddr`]). An integer-typed leaf
//!    (`ParamRef { ty: U64 }`, integer arithmetic) names no object: the
//!    integer may coincidentally equal any live address, so presenting a
//!    root for it would license `memcpy`-vs-`memmove` and alias-versioning
//!    decisions on a guess. This is the same reason LLVM gives `inttoptr` a
//!    may-alias-anything result.
//! 2. **`Copy` is transparent and must be followed.** Gating `Copy` on
//!    "known pointer" is unimplementable without whole-chain type inference
//!    (`Copy` carries no type; [`Instruction::result_type`] returns `None`
//!    for it by design) and unnecessary: `Copy` is semantically the identity,
//!    so the chain's pointer-ness is *exactly* the leaf's. Following `Copy`
//!    and checking the leaf is equivalent in strength to any inference.
//! 3. **Arithmetic preserves the root; offsets need no provenance.**
//!    `p + k` (or `p - k`, or a `GEP`) with an integer `k` stays inside `p`'s
//!    object, and going out of bounds is UB (C17 6.5.6p8) — the optimizer may
//!    assume it does not happen. This holds for *any* integer `k`, including
//!    `ptrtoint`-derived and loaded ones: wild offsets are a UB question, not
//!    an aliasing question. Only `ptr + ptr` (two rooted sides) is
//!    meaningless — invalid C, fail closed — as is `int - ptr` (not valid
//!    pointer arithmetic; only integer laundering produces it).
//! 4. **`Cast` follows only `Ptr -> Ptr`.** Any other cast shape ends the
//!    proof. This blocks nothing the frontend can produce (see above) and
//!    closes the hole for synthesized/test IR.
//!
//! # What is deliberately NOT covered here
//!
//! `loop_memory_promote` and `passes::alias` run their own root/tag schemes,
//! but both are immune to the leaf-typing question *by construction*: they
//! never claim cross-root disjointness involving parameters
//! (`loop_memory_promote::disjoint` requires two `ALLOCA`/`GLOBAL` tags;
//! `alias::forms_disjoint` returns false for differing roots), and their
//! same-root reasoning is pure address arithmetic on one base value, which
//! needs no provenance. They were audited, not assumed — see the P1
//! follow-up record.
//!
//! [`Instruction::Copy`]: super::instruction::Instruction::Copy
//! [`Instruction::Cast`]: super::instruction::Instruction::Cast
//! [`Instruction::ParamRef`]: super::instruction::Instruction::ParamRef
//! [`Instruction::Alloca`]: super::instruction::Instruction::Alloca
//! [`Instruction::GlobalAddr`]: super::instruction::Instruction::GlobalAddr
//! [`Instruction::result_type`]: super::instruction::Instruction::result_type

use super::instruction::{Instruction, Operand, Value};
use crate::common::types::IrType;

/// What a single definition contributes as an alias-analysis root leaf.
///
/// Only pointer-typed leaves name objects. Everything else — integer leaves,
/// loads, calls, arithmetic — is [`RootLeaf::NotLeaf`]: the chain either
/// continues through it ([`chain_step`]) or the proof ends.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RootLeaf {
    Global(String),
    Alloca(u32),
    Param(usize),
    NotLeaf,
}

/// Classify one definition as a provenance leaf, or not a leaf.
///
/// A [`Instruction::ParamRef`] is a leaf **only** when its IR type is
/// [`IrType::Ptr`]: an integer parameter converted to a pointer (the
/// frontend erases the conversion — see the module docs) names no object
/// and must not present a `Param` root. [`Instruction::Alloca`] and
/// [`Instruction::GlobalAddr`] values are always addresses.
pub fn root_leaf(inst: &Instruction) -> RootLeaf {
    match inst {
        Instruction::GlobalAddr { name, .. } => RootLeaf::Global(name.clone()),
        Instruction::Alloca { dest, .. } => RootLeaf::Alloca(dest.0),
        Instruction::ParamRef { param_idx, ty, .. } if *ty == IrType::Ptr => {
            RootLeaf::Param(*param_idx)
        }
        _ => RootLeaf::NotLeaf,
    }
}

/// One step of provenance-chain following.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChainStep {
    /// Continue the proof at this value.
    Follow(Value),
    /// This instruction is opaque to provenance; the tracer either applies
    /// its own pass-specific rule (phi march, pointer arithmetic) or ends
    /// the proof.
    Stop,
}

/// Follow exactly the provenance-preserving chain operations.
///
/// * [`Instruction::GetElementPtr`] continues at its base.
/// * [`Instruction::Copy`] is transparent (see the module docs).
/// * [`Instruction::Cast`] continues **only** for `Ptr -> Ptr` with a value
///   source; integer↔pointer conversions end the proof.
///
/// Everything else — loads, calls, arithmetic, phis — is [`ChainStep::Stop`].
/// Arithmetic and phis are intentionally *not* resolved here: the two
/// tracers have legitimately different rules for them (the vectorizer fails
/// closed on both; loop_idiom accepts a vetted bump phi and `ptr ± int`
/// with the [`produces_pointer`] side check), and sharing the leaf + chain
/// core while keeping those local is what prevents both drift and
/// over-generalization.
pub fn chain_step(inst: &Instruction) -> ChainStep {
    match inst {
        Instruction::GetElementPtr { base, .. } => ChainStep::Follow(*base),
        Instruction::Copy {
            src: Operand::Value(v),
            ..
        } => ChainStep::Follow(*v),
        Instruction::Cast {
            src: Operand::Value(v),
            from_ty,
            to_ty,
            ..
        } if *from_ty == IrType::Ptr && *to_ty == IrType::Ptr => ChainStep::Follow(*v),
        _ => ChainStep::Stop,
    }
}

/// Does this definition produce a value of pointer type?
///
/// Conservative-`true`: only definitions the canonical
/// [`Instruction::result_type`] types as [`IrType::Ptr`] return true.
/// Transparent ([`Instruction::Copy`], `None`) and untyped/unknown producers
/// return false — they are *not* known pointers.
///
/// The single consumer is loop_idiom's `Add`/`Sub` side check: an unrooted
/// operand that is a *known* pointer (`ptr + ptr`, loaded pointer used as an
/// offset) rejects the root, because adding two addresses is meaningless.
/// Any other unrooted operand is an integer offset or UB in valid IR (C17
/// 6.5.6 admits only `ptr ± int` / `int + ptr`), so it cannot disturb the
/// rooted side's object identity. Note the check inspects the operand's
/// *immediate* definition only: looking through `Copy` would misclassify the
/// valid `p + (int)q` idiom (whose `ptrtoint` is an erased `Copy` of a
/// pointer-typed value) as pointer-plus-pointer.
pub fn produces_pointer(inst: &Instruction) -> bool {
    inst.result_type() == Some(IrType::Ptr)
}

#[cfg(test)]
mod tests {
    use super::super::instruction::Instruction;
    use super::super::ops::IrBinOp;
    use super::super::reexports::IrConst;
    use super::*;
    use crate::common::types::IrType;

    fn param(idx: usize, ty: IrType) -> Instruction {
        Instruction::ParamRef {
            dest: Value(idx as u32),
            param_idx: idx,
            ty,
        }
    }

    #[test]
    fn leaf_typing_matrix() {
        // Pointer params root; integer params never do (P1: laundered
        // `(T *)(uintptr_t)raw` presents a Param root otherwise).
        assert_eq!(root_leaf(&param(0, IrType::Ptr)), RootLeaf::Param(0));
        assert_eq!(root_leaf(&param(1, IrType::U64)), RootLeaf::NotLeaf);
        assert_eq!(root_leaf(&param(2, IrType::I32)), RootLeaf::NotLeaf);
        assert_eq!(
            root_leaf(&Instruction::Alloca {
                dest: Value(9),
                ty: IrType::U8,
                size: 16,
                align: 1,
                volatile: false,
                semantic_volatile: false,
            }),
            RootLeaf::Alloca(9)
        );
        assert_eq!(
            root_leaf(&Instruction::GlobalAddr {
                dest: Value(10),
                name: "g".into(),
            }),
            RootLeaf::Global("g".into())
        );
        // Loads/calls/arithmetic are never leaves, whatever their type.
        assert_eq!(
            root_leaf(&Instruction::Load {
                dest: Value(11),
                ptr: Value(9),
                ty: IrType::Ptr,
                seg_override: crate::common::types::AddressSpace::Default,
                volatile: false,
            }),
            RootLeaf::NotLeaf
        );
    }

    #[test]
    fn cast_gate_matrix() {
        let cast = |from_ty, to_ty| Instruction::Cast {
            dest: Value(20),
            src: Operand::Value(Value(1)),
            from_ty,
            to_ty,
        };
        // Only Ptr -> Ptr continues the proof.
        assert_eq!(
            chain_step(&cast(IrType::Ptr, IrType::Ptr)),
            ChainStep::Follow(Value(1))
        );
        assert_eq!(chain_step(&cast(IrType::U64, IrType::Ptr)), ChainStep::Stop);
        assert_eq!(chain_step(&cast(IrType::Ptr, IrType::U64)), ChainStep::Stop);
        assert_eq!(chain_step(&cast(IrType::I32, IrType::I64)), ChainStep::Stop);
        // Const-source casts have no chain to follow.
        assert_eq!(
            chain_step(&Instruction::Cast {
                dest: Value(21),
                src: Operand::Const(IrConst::I64(0)),
                from_ty: IrType::Ptr,
                to_ty: IrType::Ptr,
            }),
            ChainStep::Stop
        );
        // Copy is transparent; GEP continues at its base.
        assert_eq!(
            chain_step(&Instruction::Copy {
                dest: Value(22),
                src: Operand::Value(Value(3)),
            }),
            ChainStep::Follow(Value(3))
        );
        assert_eq!(
            chain_step(&Instruction::GetElementPtr {
                dest: Value(23),
                base: Value(4),
                offset: Operand::Const(IrConst::I64(0)),
                ty: IrType::Ptr,
            }),
            ChainStep::Follow(Value(4))
        );
        // Arithmetic/phi/load/call all stop here (pass-specific rules apply).
        assert_eq!(
            chain_step(&Instruction::BinOp {
                dest: Value(24),
                op: IrBinOp::Add,
                lhs: Operand::Value(Value(4)),
                rhs: Operand::Const(IrConst::I64(8)),
                ty: IrType::U64,
            }),
            ChainStep::Stop
        );
    }

    #[test]
    fn produces_pointer_matrix() {
        assert!(produces_pointer(&param(0, IrType::Ptr)));
        assert!(!produces_pointer(&param(0, IrType::U64)));
        assert!(produces_pointer(&Instruction::GetElementPtr {
            dest: Value(23),
            base: Value(4),
            offset: Operand::Const(IrConst::I64(0)),
            ty: IrType::I8, // element type is irrelevant: the dest is an address
        }));
        assert!(!produces_pointer(&Instruction::Copy {
            dest: Value(22),
            src: Operand::Value(Value(3)),
        }));
        assert!(!produces_pointer(&Instruction::BinOp {
            dest: Value(24),
            op: IrBinOp::Add,
            lhs: Operand::Value(Value(4)),
            rhs: Operand::Const(IrConst::I64(8)),
            ty: IrType::U64,
        }));
        assert!(produces_pointer(&Instruction::Load {
            dest: Value(11),
            ptr: Value(9),
            ty: IrType::Ptr,
            seg_override: crate::common::types::AddressSpace::Default,
            volatile: false,
        }));
        assert!(!produces_pointer(&Instruction::Cmp {
            dest: Value(12),
            op: crate::ir::ops::IrCmpOp::Ult,
            lhs: Operand::Value(Value(1)),
            rhs: Operand::Const(IrConst::I32(16)),
            ty: IrType::I32,
        }));
    }
}
