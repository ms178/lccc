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
    /// always integer.  The index is taken modulo the operand width W, as
    /// every native form does (x86 BT, AArch64 LSRV, RISC-V SRL/SRLW): an
    /// index of W or more comes from an undefined C shift, which may sit in
    /// dead code and must still compile, and folding agrees with codegen.  Backends may lower this to a native BT-family
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
    /// Zero the high bits from an index on: `Bzhi(x, n)` with `k = n & 0xFF`
    /// is `x` when `k >= W` and `x & ((1 << k) - 1)` otherwise -- exactly
    /// x86 BMI2 `bzhi` and the `_bzhi_u32`/`_bzhi_u64` intrinsics (Intel SDM
    /// "BZHI"; GCC's `__builtin_ia32_bzhi_si/_di`).  Only the low eight bits
    /// of the index are read, and an index of W or more is the identity, so
    /// the operation is total; it is NOT the C expression
    /// `x & ((1 << n) - 1)`, whose shift is undefined from W on.
    ///
    /// Typed I32/U32/I64/U64 only.  Evaluating in a wider container and
    /// truncating to W is exact: for `W <= k < container` the container
    /// result `x & ((1 << k) - 1)` keeps every one of x's low W bits, and for
    /// larger k both forms return x.  Only the x86-64 frontend creates it,
    /// from the BMI2 builtins when BMI2 is enabled; no pass synthesizes it.
    Bzhi,
    /// Set one bit: `BitSet(x, n)` is `x | (1 << (n mod W))`, W the width of
    /// the operation's IR type -- x86 `bts`, AArch64 `mov`+`lsl`+`orr` (there
    /// is no variable-immediate BFI), RISC-V `sll`+`or`.  This is the
    /// cross-target canonical form of the C idiom `x | 1u << n`, which
    /// otherwise costs an instruction for the constant, one for the shift and
    /// one for the OR, with the shift pinned to `%cl` on x86.  The index is
    /// taken modulo W exactly as [`IrBinOp::BitTest`]'s is, so an index of W
    /// or more -- an undefined C shift that may sit in dead code -- still
    /// compiles and still agrees with the constant folder.
    ///
    /// Typed 8/16/32/64-bit integers, and the bit index is the second operand:
    /// the operation is NOT commutative.  Passes synthesize it from the
    /// shift/OR pair, so every backend must lower it; the portable lowering
    /// is `lhs | (1 << (rhs mod W))` at the operation's width, which is why
    /// a 64-bit operation may not appear on a 32-bit target (the wide-int
    /// pair path has no bit-set lowering).
    BitSet,
    /// Clear one bit: `BitClear(x, n)` is `x & ~(1 << (n mod W))`, the
    /// canonical form of `x & ~(1u << n)` -- x86 `btr`, and the portable
    /// `x & ~(1 << (rhs mod W))` elsewhere.  Same typing, index semantics,
    /// backend obligation and non-commutativity as [`IrBinOp::BitSet`].
    BitClear,
    /// High half of the double-width product: `UMulHigh(a, b)` is
    /// `(zext(a) * zext(b)) >> W` and `SMulHigh(a, b)` is
    /// `(sext(a) * sext(b)) >> W` (arithmetic), W the width of the
    /// operation's IR type -- GCC's `MULT_HIGHPART_EXPR`, LLVM's
    /// `mulhu`/`mulhs`, x86 one-operand `mul`/`imul` (`%rdx`), AArch64
    /// `umulh`/`smulh`, RISC-V `mulhu`/`mulh`.
    ///
    /// Typed I64/U64 only, and only on 64-bit targets: `div_by_const`
    /// creates it for the magic-number expansion of 64-bit division, which
    /// otherwise needs a 128-bit product.  Spelled that way the function
    /// carries I128 values, and on x86-64 a function with any I128 value
    /// loses `%r8`, `%r9`, `%rdi`, `%rsi` and `%rdx` from its register pool
    /// for its whole body (the i128 emitters use them as scratch) -- one
    /// `n / 5` in a loop preheader spilled the 20-symbol decode loop of zstd's
    /// `HUF_decompress4X1_usingDTable_internal_fast_c_loop`.  Like the
    /// rotates and `Bzhi` its value depends on W, so it cannot be evaluated
    /// in a wider container and truncated: use [`mul_high_within_bits`].
    UMulHigh,
    SMulHigh,
}

/// High half of the `2 * bits`-wide product of the low `bits` bits of `lhs`
/// and `rhs` (see [`IrBinOp::UMulHigh`]), sign- or zero-extended from `bits`
/// according to `signed`, for `1 <= bits <= 64`.  Callers truncate the result
/// to the operation's type.
pub fn mul_high_within_bits(lhs: i64, rhs: i64, bits: u32, signed: bool) -> i64 {
    debug_assert!((1..=64).contains(&bits));
    let bits = bits.clamp(1, 64);
    let shift = 64 - bits;
    if signed {
        let a = i128::from((lhs << shift) >> shift);
        let b = i128::from((rhs << shift) >> shift);
        ((a * b) >> bits) as i64
    } else {
        let a = u128::from(((lhs as u64) << shift) >> shift);
        let b = u128::from(((rhs as u64) << shift) >> shift);
        ((a * b) >> bits) as i64
    }
}

/// The backend arm for an [`IrBinOp::UMulHigh`]/[`IrBinOp::SMulHigh`] it
/// never receives: `div_by_const` creates them only I64/U64-typed and only
/// for 64-bit targets, so i686 or a narrower type is a broken invariant, not
/// an input error.  One shared site keeps the invariant and its message.
#[cold]
pub fn mul_high_unsupported_here() -> ! {
    unreachable!("UMulHigh/SMulHigh are created only I64/U64-typed for 64-bit targets")
}

impl IrBinOp {
    /// True for the multiply-high forms ([`IrBinOp::UMulHigh`]).
    pub fn is_mul_high(self) -> bool {
        matches!(self, IrBinOp::UMulHigh | IrBinOp::SMulHigh)
    }

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

/// `Bzhi` at container width `bits`: keep the low `index & 0xFF` bits of
/// `value`, or all of them when that is `bits` or more (see
/// [`IrBinOp::Bzhi`]).  Callers truncate the result to the operation's type.
pub fn zero_high_bits_from(value: u128, index: u128, bits: u32) -> u128 {
    let k = (index & 0xFF) as u32;
    if k >= bits || k >= 128 {
        value
    } else {
        value & ((1u128 << k) - 1)
    }
}

/// `BitSet`/`BitClear` at container width `bits`: force bit `index mod bits`
/// of `value` to `set` (`true`) or clear, keeping only the low `bits` bits of
/// the container so a sign-extended operand does not leak into the result
/// (see [`IrBinOp::BitSet`]).  Callers truncate the result to the operation's
/// type.  `bits` is 1..=128 and the index is reduced here, so an index of W
/// or more -- an undefined C shift that may sit in dead code -- is defined and
/// matches what the hardware does with the same index.
pub fn bit_op_within_bits(value: u128, index: u128, bits: u32, set: bool) -> u128 {
    debug_assert!(bits >= 1 && bits <= 128);
    let mask = if bits >= 128 {
        u128::MAX
    } else {
        (1u128 << bits) - 1
    };
    let bit = 1u128 << (index % bits as u128);
    if set {
        (value | bit) & mask
    } else {
        (value & !bit) & mask
    }
}

/// The non-x86-64 backends' arm for [`IrBinOp::Bzhi`]: only the x86-64
/// frontend creates it (from the BMI2 builtins) and no pass synthesizes it,
/// so reaching another backend is a broken invariant, not an input error.
/// One shared site keeps the invariant and its message in one place.
#[cold]
pub fn bzhi_outside_x86_64() -> ! {
    unreachable!(
        "Bzhi is created only by the x86-64 frontend (BMI2 builtins); no pass synthesizes it"
    )
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
                ((lhs as u64).wrapping_shr(rhs as u32) & 1) as i64
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
            IrBinOp::Bzhi => zero_high_bits_from(lhs as u64 as u128, rhs as u64 as u128, 64) as i64,
            // Container-width bit set/clear; a narrower IR type is folded by
            // `fold_binop` with its own width, never through here.
            IrBinOp::BitSet => {
                bit_op_within_bits(lhs as u64 as u128, rhs as u64 as u128, 64, true) as i64
            }
            IrBinOp::BitClear => {
                bit_op_within_bits(lhs as u64 as u128, rhs as u64 as u128, 64, false) as i64
            }
            IrBinOp::UMulHigh => mul_high_within_bits(lhs, rhs, 64, false),
            IrBinOp::SMulHigh => mul_high_within_bits(lhs, rhs, 64, true),
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
            IrBinOp::BitTest => ((lhs as u128).wrapping_shr(rhs as u32) & 1) as i128,
            // Container-width (128-bit) rotate; see the note on the i64 arms —
            // narrower IR types are folded by `fold_binop` with their own
            // width, never through here.
            IrBinOp::RotateLeft => rotate_within_bits(lhs as u128, rhs as u128, 128, true) as i128,
            IrBinOp::RotateRight => {
                rotate_within_bits(lhs as u128, rhs as u128, 128, false) as i128
            }
            IrBinOp::Bzhi => zero_high_bits_from(lhs as u128, rhs as u128, 128) as i128,
            IrBinOp::BitSet => bit_op_within_bits(lhs as u128, rhs as u128, 128, true) as i128,
            IrBinOp::BitClear => bit_op_within_bits(lhs as u128, rhs as u128, 128, false) as i128,
            // No 128-bit form exists (I64/U64 only); refuse rather than
            // evaluate at the wrong width.
            IrBinOp::UMulHigh | IrBinOp::SMulHigh => return None,
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
    /// Defined for zero: returns the operand bit width.
    Clz,
    /// Operand is proven nonzero. This distinction lets baseline x86 use BSR
    /// without a zero fixup while preserving `Clz(0) == width` for internal IR.
    ClzNonZero,
    /// Defined for zero: returns the operand bit width.
    Ctz,
    /// Operand is proven nonzero. This distinction lets baseline x86 use BSF
    /// without a zero fixup while preserving `Ctz(0) == width` for internal IR.
    CtzNonZero,
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

#[cfg(test)]
mod mul_high_tests {
    use super::*;

    /// Reference: the high `bits` of the `2 * bits`-wide product, computed
    /// with full-width i128/u128 arithmetic on the extended operands.
    fn reference(a: i64, b: i64, bits: u32, signed: bool) -> i64 {
        let shift = 64 - bits;
        if signed {
            let a = i128::from((a << shift) >> shift);
            let b = i128::from((b << shift) >> shift);
            ((a * b) >> bits) as i64
        } else {
            let a = u128::from(((a as u64) << shift) >> shift);
            let b = u128::from(((b as u64) << shift) >> shift);
            ((a * b) >> bits) as i64
        }
    }

    #[test]
    fn mul_high_matches_x86_mul_and_imul() {
        let edges = [
            0i64,
            1,
            -1,
            2,
            3,
            5,
            7,
            i64::MAX,
            i64::MIN,
            i64::MIN + 1,
            0x5555_5555_5555_5556,
            0xCCCC_CCCC_CCCC_CCCDu64 as i64,
            0x0123_4567_89AB_CDEF,
            u32::MAX as i64,
        ];
        for &a in &edges {
            for &b in &edges {
                let u = u128::from(a as u64) * u128::from(b as u64);
                let s = i128::from(a) * i128::from(b);
                assert_eq!(IrBinOp::UMulHigh.eval_i64(a, b), Some((u >> 64) as i64));
                assert_eq!(IrBinOp::SMulHigh.eval_i64(a, b), Some((s >> 64) as i64));
                // Multiplication is symmetric even though the op is not
                // flagged commutative.
                assert_eq!(
                    IrBinOp::UMulHigh.eval_i64(a, b),
                    IrBinOp::UMulHigh.eval_i64(b, a)
                );
                // No 128-bit form: refusing beats evaluating at the wrong W.
                assert_eq!(IrBinOp::UMulHigh.eval_i128(a.into(), b.into()), None);
                assert_eq!(IrBinOp::SMulHigh.eval_i128(a.into(), b.into()), None);
                for bits in [8u32, 16, 32, 64] {
                    for signed in [false, true] {
                        assert_eq!(
                            mul_high_within_bits(a, b, bits, signed),
                            reference(a, b, bits, signed),
                            "a={a:#x} b={b:#x} bits={bits} signed={signed}"
                        );
                    }
                }
            }
        }
        // A narrow width is NOT the 64-bit high half truncated.
        assert_eq!(mul_high_within_bits(0xFF, 0xFF, 8, false), 0xFE);
        assert_eq!(mul_high_within_bits(-1, -1, 8, true), 0);
        assert!(IrBinOp::UMulHigh.is_mul_high() && !IrBinOp::Mul.is_mul_high());
    }
}
