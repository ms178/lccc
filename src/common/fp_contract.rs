//! Floating-point contraction contract (C11 6.5p8 / FP_CONTRACT pragma).
//!
//! `-ffp-contract` selects how `a*b + c` may collapse into a single-rounding
//! FMA:
//!
//! * [`FpContract::Off`] — never contract. Every intermediate rounds
//!   exactly as the abstract machine specifies.
//! * [`FpContract::OnExpr`] — contract only when the multiply and the add
//!   come from the SAME source expression. The frontend tags every FP
//!   Mul/Add/Sub result with its statement-root id
//!   (`IrFunction::fp_expr_tags`); the backend fuses two ops only when
//!   their tags match. This is GCC's `-ffp-contract=on` semantics: the
//!   leaf `x = a*b + c` fuses, while `t = a*b; s += t;` (separate
//!   statements) never does. Optimization-pass-created values carry no
//!   tag and fail closed.
//! * [`FpContract::Fast`] — contract freely across SSA values
//!   (`-ffp-contract=fast`, implied by `-ffast-math`). Changes numerics;
//!   single rounding per fused op.

use crate::common::fx_hash::FxHashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FpContract {
    /// Never contract (conservative default; matches pre-OP-36 behaviour).
    #[default]
    Off,
    /// Contract within one source expression only (GCC `on` semantics).
    OnExpr,
    /// Contract freely (GCC `fast`; `-ffast-math` implies this).
    Fast,
}

impl FpContract {
    /// Whether backend mul+add fusion is allowed for a candidate pair with
    /// the given expression tags (`None` = untagged: pass-generated or
    /// inlined value).
    #[inline]
    pub fn fuse_pair(self, mul_tag: Option<u64>, add_tag: Option<u64>) -> bool {
        match self {
            FpContract::Fast => true,
            FpContract::OnExpr => match (mul_tag, add_tag) {
                (Some(m), Some(a)) => m == a,
                _ => false,
            },
            FpContract::Off => false,
        }
    }
}

/// Per-function expression tags: FP arithmetic value → statement-root id.
/// Populated by the frontend lowering (one fresh root per statement); used
/// by [`FpContract::OnExpr`] to keep contraction inside one expression.
/// Optimization passes create untagged values, which fail closed.
pub type FpExprTags = FxHashMap<u32, u64>;
