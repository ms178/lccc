//! IR operation enums: binary, unary, comparison, and atomic operations.
//!
//! Each enum carries its own evaluation methods (eval_i64, eval_i128, eval_f64)
//! for use by constant folding and simplification passes.

/// Atomic read-modify-write operations.
#[derive(Debug, Clone, Copy)]
pub enum AtomicRmwOp {
    /// Add: *ptr += val
    Add,
    /// Sub: *ptr -= val
    Sub,
    /// And: *ptr &= val
    And,
    /// Or: *ptr |= val
    Or,
    /// Xor: *ptr ^= val
    Xor,
    /// Nand: *ptr = ~(*ptr & val)
    Nand,
    /// Exchange: *ptr = val (returns old value)
    Xchg,
    /// Test and set: *ptr = 1 (returns old value)
    TestAndSet,
}

/// Memory ordering for atomic operations.
#[derive(Debug, Clone, Copy)]
pub enum AtomicOrdering {
    Relaxed,
    Acquire,
    Release,
    AcqRel,
    SeqCst,
}

/// Binary operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IrBinOp {
    Add,
    Sub,
    Mul,
    SDiv,
    UDiv,
    SRem,
    URem,
    And,
    Or,
    Xor,
    Shl,
    AShr,
    LShr,
    /// Boolean bit test: `(lhs >> rhs) & 1`.  The result is I32 and lhs/rhs are
    /// always integer.  Backends may lower this to a native BT-family
    /// instruction or to the portable shift/AND sequence.  This is the
    /// cross-target canonical form for classifier chains such as Expat name
    /// scanning, instead of leaving the pattern in text peepholes.
    BitTest,
    /// Bitwise rotate (funnel shift by a whole register): `RotateLeft(x, n)`
    /// is `(x << n) | (x >> (W - n))` and `RotateRight(x, n)` is
    /// `(x >> n) | (x << (W - n))`, where **W is the width of the operation's
    /// IR type**, not the width of the container the constant folder happens
    /// to evaluate in.  Both operands and the result are integer.
    ///
    /// This is the cross-target canonical form for the ARX rotate idiom that
    /// every hash and cipher spells out portably (`ROTL32`/`ROTR` in ChaCha20,
    /// SHA-256, MD5, BLAKE2, siphash, the kernel's `rol32`/`ror32`).  Left as
    /// a shift/or triple it costs three instructions, two of them on the
    /// critical path through a temporary, where x86 `rol`/`ror`, AArch64
    /// `ror`/`extr` and RISC-V Zbb `rol`/`ror` each cost one.  Backends
    /// without a native rotate lower it to the portable sequence, so
    /// recognition never costs correctness — only the missed single
    /// instruction.
    ///
    /// The rotate amount is taken modulo W (x86 masks the count to 5/6 bits
    /// in hardware; the IR makes that explicit so a count of 0 or >= W folds
    /// to the identity rather than depending on an ISA's masking rules).
    RotateLeft,
    RotateRight,
}

impl IrBinOp {
    /// True for the two rotate forms, whose semantics depend on the
    /// operation's *type width* rather than only on the operand values.
    pub fn is_rotate(self) -> bool {
        matches!(self, IrBinOp::RotateLeft | IrBinOp::RotateRight)
    }
}

/// Rotate the low `bits` bits of `value` by `amount`, reducing `amount`
/// modulo `bits`.
///
/// Rotates are the one integer operation whose result cannot be recovered by
/// evaluating in a wider container and truncating afterwards: rotating
/// `0x8000_0001` left by one gives `0x0000_0003` at 32 bits, but the 64-bit
/// rotate yields `0x1_0000_0002`, whose low half is `0x0000_0002`.  Bits that
/// leave the top of the *narrow* value must re-enter at its bottom, so any
/// width-typed consumer (constant folding at I8/I16/I32, i128 lowering) must
/// route through here with the operation's real width instead of reusing
/// `IrBinOp::eval_i64`/`eval_i128`.
///
/// `bits == 0` (a `Void`-typed operation, which cannot occur in valid IR)
/// yields 0 rather than shifting by the container width, which Rust defines as
/// a panic in debug builds.
pub fn rotate_within_bits(value: u128, amount: u128, bits: u32, left: bool) -> u128 {
    if bits == 0 || bits > 128 {
        return 0;
    }
    let mask = if bits >= 128 {
        u128::MAX
    } else {
        (1u128 << bits) - 1
    };
    let value = value & mask;
    let amount = (amount % u128::from(bits)) as u32;
    if amount == 0 {
        return value;
    }
    // Both halves stay inside `mask`: the left shift drops the bits that the
    // right shift brings back in, and vice versa.
    if left {
        ((value << amount) | (value >> (bits - amount))) & mask
    } else {
        ((value >> amount) | (value << (bits - amount))) & mask
    }
}

impl IrBinOp {
    /// Returns true if this operation is commutative (a op b == b op a).
    pub fn is_commutative(self) -> bool {
        matches!(
            self,
            IrBinOp::Add | IrBinOp::Mul | IrBinOp::And | IrBinOp::Or | IrBinOp::Xor
        )
    }

    /// Returns true if this operation can trap at runtime (e.g., divide by zero causes SIGFPE).
    /// Such operations must not be speculatively executed by if-conversion.
    pub fn can_trap(self) -> bool {
        matches!(
            self,
            IrBinOp::SDiv | IrBinOp::UDiv | IrBinOp::SRem | IrBinOp::URem
        )
    }

    /// Evaluate this binary operation on two i64 operands using wrapping arithmetic.
    ///
    /// Signed operations use Rust's native i64 arithmetic.
    /// Unsigned operations (UDiv, URem, LShr) reinterpret the bits as u64.
    /// Returns None for division/remainder by zero.
    pub fn eval_i64(self, lhs: i64, rhs: i64) -> Option<i64> {
        Some(match self {
            IrBinOp::Add => lhs.wrapping_add(rhs),
            IrBinOp::Sub => lhs.wrapping_sub(rhs),
            IrBinOp::Mul => lhs.wrapping_mul(rhs),
            IrBinOp::And => lhs & rhs,
            IrBinOp::Or => lhs | rhs,
            IrBinOp::Xor => lhs ^ rhs,
            IrBinOp::Shl => lhs.wrapping_shl(rhs as u32),
            IrBinOp::AShr => lhs.wrapping_shr(rhs as u32),
            IrBinOp::LShr => (lhs as u64).wrapping_shr(rhs as u32) as i64,
            IrBinOp::BitTest => {
                // The canonical operation is produced by an i32-typed recognizer
                // after integer promotion; evaluate it as C's `(x >> i) & 1`.
                (((lhs as u64) >> (rhs as u32)) & 1) as i64
            }
            // Container-width rotate.  A narrower IR type (I8/I16/I32) MUST
            // not be folded through here: rotating at 64 bits and truncating
            // is not the same value (see `rotate_within_bits`).
            // `eval_binop_const`/`fold_binop` intercept both rotate forms and
            // pass the operation's real width instead.
            IrBinOp::RotateLeft => {
                rotate_within_bits(lhs as u64 as u128, rhs as u64 as u128, 64, true) as i64
            }
            IrBinOp::RotateRight => {
                rotate_within_bits(lhs as u64 as u128, rhs as u64 as u128, 64, false) as i64
            }
            IrBinOp::SDiv => {
                if rhs == 0 {
                    return None;
                }
                lhs.wrapping_div(rhs)
            }
            IrBinOp::UDiv => {
                if rhs == 0 {
                    return None;
                }
                ((lhs as u64).wrapping_div(rhs as u64)) as i64
            }
            IrBinOp::SRem => {
                if rhs == 0 {
                    return None;
                }
                lhs.wrapping_rem(rhs)
            }
            IrBinOp::URem => {
                if rhs == 0 {
                    return None;
                }
                ((lhs as u64).wrapping_rem(rhs as u64)) as i64
            }
        })
    }

    /// Evaluate this binary operation on two i128 operands using wrapping arithmetic.
    ///
    /// Unsigned operations (UDiv, URem, LShr) reinterpret the bits as u128.
    /// Returns None for division/remainder by zero.
    pub fn eval_i128(self, lhs: i128, rhs: i128) -> Option<i128> {
        Some(match self {
            IrBinOp::Add => lhs.wrapping_add(rhs),
            IrBinOp::Sub => lhs.wrapping_sub(rhs),
            IrBinOp::Mul => lhs.wrapping_mul(rhs),
            IrBinOp::And => lhs & rhs,
            IrBinOp::Or => lhs | rhs,
            IrBinOp::Xor => lhs ^ rhs,
            IrBinOp::Shl => lhs.wrapping_shl(rhs as u32),
            IrBinOp::AShr => lhs.wrapping_shr(rhs as u32),
            IrBinOp::LShr => (lhs as u128).wrapping_shr(rhs as u32) as i128,
            IrBinOp::BitTest => (((lhs as u128) >> (rhs as u32)) & 1) as i128,
            // Container-width (128-bit) rotate; see the note on the i64 arms —
            // narrower IR types are folded by `fold_binop` with their own
            // width, never through here.
            IrBinOp::RotateLeft => rotate_within_bits(lhs as u128, rhs as u128, 128, true) as i128,
            IrBinOp::RotateRight => {
                rotate_within_bits(lhs as u128, rhs as u128, 128, false) as i128
            }
            IrBinOp::SDiv => {
                if rhs == 0 {
                    return None;
                }
                lhs.wrapping_div(rhs)
            }
            IrBinOp::UDiv => {
                if rhs == 0 {
                    return None;
                }
                (lhs as u128).wrapping_div(rhs as u128) as i128
            }
            IrBinOp::SRem => {
                if rhs == 0 {
                    return None;
                }
                lhs.wrapping_rem(rhs)
            }
            IrBinOp::URem => {
                if rhs == 0 {
                    return None;
                }
                (lhs as u128).wrapping_rem(rhs as u128) as i128
            }
        })
    }
}

/// Unary operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IrUnaryOp {
    Neg,
    Not,
    Clz,
    Ctz,
    Bswap,
    BitReverse,
    Popcount,
    /// __builtin_constant_p: returns 1 if operand is a compile-time constant, 0 otherwise.
    /// Lowered as an IR instruction so it can be resolved after inlining and constant propagation.
    IsConstant,
}

/// Comparison operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IrCmpOp {
    Eq,
    Ne,
    Slt,
    Sle,
    Sgt,
    Sge,
    Ult,
    Ule,
    Ugt,
    Uge,
}

impl IrCmpOp {
    /// Evaluate this comparison on two i64 operands.
    ///
    /// Signed comparisons use Rust's native i64 ordering.
    /// Unsigned comparisons reinterpret the bits as u64.
    pub fn eval_i64(self, lhs: i64, rhs: i64) -> bool {
        match self {
            IrCmpOp::Eq => lhs == rhs,
            IrCmpOp::Ne => lhs != rhs,
            IrCmpOp::Slt => lhs < rhs,
            IrCmpOp::Sle => lhs <= rhs,
            IrCmpOp::Sgt => lhs > rhs,
            IrCmpOp::Sge => lhs >= rhs,
            IrCmpOp::Ult => (lhs as u64) < (rhs as u64),
            IrCmpOp::Ule => (lhs as u64) <= (rhs as u64),
            IrCmpOp::Ugt => (lhs as u64) > (rhs as u64),
            IrCmpOp::Uge => (lhs as u64) >= (rhs as u64),
        }
    }

    /// Evaluate this comparison on two i128 operands.
    ///
    /// Signed comparisons use Rust's native i128 ordering.
    /// Unsigned comparisons reinterpret the bits as u128.
    pub fn eval_i128(self, lhs: i128, rhs: i128) -> bool {
        match self {
            IrCmpOp::Eq => lhs == rhs,
            IrCmpOp::Ne => lhs != rhs,
            IrCmpOp::Slt => lhs < rhs,
            IrCmpOp::Sle => lhs <= rhs,
            IrCmpOp::Sgt => lhs > rhs,
            IrCmpOp::Sge => lhs >= rhs,
            IrCmpOp::Ult => (lhs as u128) < (rhs as u128),
            IrCmpOp::Ule => (lhs as u128) <= (rhs as u128),
            IrCmpOp::Ugt => (lhs as u128) > (rhs as u128),
            IrCmpOp::Uge => (lhs as u128) >= (rhs as u128),
        }
    }

    /// Evaluate this comparison on two f64 operands using IEEE 754 semantics.
    ///
    /// For floats, signed and unsigned comparison variants are equivalent since
    /// IEEE 754 defines a total ordering (NaN comparisons return false for
    /// ordered ops, true for Ne).
    pub fn eval_f64(self, lhs: f64, rhs: f64) -> bool {
        match self {
            IrCmpOp::Eq => lhs == rhs,
            IrCmpOp::Ne => lhs != rhs,
            IrCmpOp::Slt | IrCmpOp::Ult => lhs < rhs,
            IrCmpOp::Sle | IrCmpOp::Ule => lhs <= rhs,
            IrCmpOp::Sgt | IrCmpOp::Ugt => lhs > rhs,
            IrCmpOp::Sge | IrCmpOp::Uge => lhs >= rhs,
        }
    }
}
