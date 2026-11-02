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
    /// The contraction default for C compilation: [`FpContract::Fast`],
    /// matching GCC's C default (`-ffp-contract=fast` in GNU dialects since
    /// 4.6). Verified against the godbolt oracles on a cross-statement
    /// reduction (`t = a[i]*b[i]; s = s + t;`, -O3 -march=x86-64-v3):
    /// gcc16.2 and icx fuse to vfmadd by default; only clang (whose C
    /// default is `on`) keeps the separate pair. The conservative
    /// [`FpContract::Off`] default lost every scalar FMA opportunity
    /// against the reference compiler (dot8: 46 instructions vs gcc's 21)
    /// while changing numerics away from GCC's, not toward it.
    /// Explicit `-ffp-contract={on,off}` flags always win; `Off` remains
    /// the enum `Default` for internal/test callers that want the
    /// fail-closed sentinel.
    pub const fn c_language_default() -> Self {
        FpContract::Fast
    }

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
