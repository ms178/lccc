//! Compile-time SIMD mask facts, transplanted from Intel ISPC's
//! `GetMaskFromValue` / `GetMaskStatusFromValue` lattice
//! (`src/llvmutil.cpp:1920`, `src/llvmutil.h:32`, pinned at ISPC `ae2ae73`,
//! 2026-11-06).  See `engineering/ISPC-TRANSPLANT-AUDIT-2026-11-07.md` for
//! the full inventory and the transferability decisions.
//!
//! ISPC classifies an execution mask as `all_on`, `all_off`, `mixed` or
//! `unknown` and uses that one lattice in five different passes:
//! `InstructionSimplify` (constant `__movmsk`, all-on/all-off selects),
//! `IntrinsicsOpt` (blends and AVX masked loads/stores),
//! `ImproveMemoryOps` (forward/backward gather legality),
//! `MaskedStoreOpt` and `ReplaceMaskedMemOps`.  LCCC had no equivalent: a
//! `__builtin_ia32_pmovmskb128` on a compile-time-constant vector compiled to
//! sixteen stack stores, a `movdqu` and a `pmovmskb` on x86-64, a 17-instruction
//! NEON sequence on AArch64, and a software reduction on RISC-V, where GCC 16.2
//! and Clang 23.1.5 emit one `movl $mask, %eax`.
//!
//! This module supplies the lattice and five exact consumers:
//!
//! 1. **Movemask fold** — `Pmovmskb128/256`, `MovemaskPs/Pd128/256` on a known
//!    vector image become an `I32` constant (ISPC `InstructionSimplify`'s
//!    `lSimplifyCall` / `IntrinsicsOpt`'s mask-instruction arm).
//! 2. **Blend triviality** — a `VecBlendv*` with a lane-exact all-on/all-off
//!    mask, or with identical arms, becomes a `Copy`; the legacy
//!    `Blendv*128/256` `dest_ptr` forms emitted by the C frontend become a
//!    bounds-proven `Memcpy` (ISPC `IntrinsicsOpt`'s blend arm and
//!    `InstructionSimplify`'s `lSimplifySelect`).
//! 3. **Load-of-constant** — a non-volatile scalar `Load` from a proven
//!    constant window becomes a bit-exact `IrConst` (ISPC
//!    `ImproveMemoryOps` / `InstructionSimplify`).  `run` re-runs the
//!    canonical `constant_fold` afterwards, because this hook is scheduled
//!    after every other folder in the pipeline.
//! 4. **Computed-vector propagation** — byte-wise commutative `And`/`Or`/`Xor`
//!    /`Eq` chains and exact-width splats are proved recursively, with one
//!    shared work budget and cycle set.
//! 5. **Private-image reclamation** — after (1)-(3) remove the last reader of
//!    a private constant object, its initialization writes are deleted.
//!    Without this the fold saves the `pmovmskb`/NEON reduction but leaves the
//!    whole constant image in the instruction stream, which is not a win.
//!    ISPC does not need it because LLVM constant vectors are SSA values with
//!    no emitted initialization.
//!
//! ## Adapting the lattice to LCCC's IR
//!
//! ISPC's masks are LLVM `ConstantVector`/`ConstantDataVector` SSA values, so
//! `GetMaskFromValue` only inspects a constant node.  LCCC materializes a
//! constant vector as a *stack object* initialized by scalar constant stores
//! (and vector temporaries as `VecStore*` of a `VecLoad*`).  The lattice here
//! therefore adds a **constant-object proof** that ISPC does not need.  It is
//! deliberately stricter than `const_array_promote` and fail-closed:
//!
//! * the object is a non-volatile, non-semantic-volatile `Alloca`;
//! * every mention of its address (through `Copy` and `GEP` chains of ANY
//!   length, including variable offsets — see `root_of`) is found by a
//!   once-per-function mention index; every proven WRITE must be in the SAME
//!   basic block as the read and at or before it, while a proven pure read
//!   may appear anywhere (a read cannot change the image);
//! * a `Phi`/`Select`/`Cast`/call/inline-asm/atomic/scalar-load mention of the
//!   address rejects the candidate, so provenance can never be lost and no
//!   unknown pointer can alias the object;
//! * every write is either a scalar `Store` of an `IrConst` or a recognized
//!   whole-register `VecStore*` whose source vector has a known image;
//! * the requested byte window is tiled **exactly once** — a gap or an
//!   overlap rejects, so the image is a value, not a guess;
//! * the recursion is bounded (`MAX_DEPTH`, `MAX_WORK`) and cycle-checked.
//!
//! Nothing here changes memory, introduces a new IR opcode, or assumes a
//! target: the fold produces one `I32` constant, which all four backends lower
//! identically.  That is also why this pass *improves* AArch64 and RISC-V most:
//! it deletes the software `pmovmskb` sequence entirely.  The
//! `CCC_DISABLE_PASSES` name is `maskfacts`.
use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::common::types::{AddressSpace, IrType};
use crate::ir::reexports::{
    Instruction, IntrinsicOp, IrConst, IrFunction, IrModule, Operand, Value,
};

use super::vector_temp_promotion::{whole_vector_load, whole_vector_store_width};

/// Recursion depth cap for SSA image walks.  Real chains (constant store ->
/// `VecLoad*` -> `VecStore*` -> movemask) are 2-4 links.  A longer chain
/// returns `Unknown`, which is always safe.
const MAX_DEPTH: usize = 16;
/// Per-proof instruction/derivation work budget.  A large function simply
/// declines the optimization instead of becoming quadratic in the optimizer.
const MAX_WORK: usize = 4096;

/// ISPC's `MaskStatus`, with the exact lane bits retained for `mixed`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MaskStatus {
    AllOn,
    AllOff,
    Mixed(u32),
    Unknown,
}

impl MaskStatus {
    /// Sign bit of each fixed-width lane, lane `i` contributing bit `i`.
    /// `lane_bits` is the lane width in BITS (8, 16, 32 or 64); the image must
    /// be an exact whole number of lanes and at most 32 lanes wide.
    ///
    /// Lanes are little-endian (`IrConst::to_le_bytes` uses `to_le_bytes` and
    /// `f32::to_bits`), so the sign bit is bit 7 of the LAST byte of the lane
    /// for every supported width: this is exact for I8/I16/I32/I64 and for the
    /// IEEE-754 sign bit of F32/F64.
    pub(crate) fn from_lanes(image: &[u8], lane_bits: usize) -> Self {
        let lane_bytes = lane_bits / 8;
        if lane_bytes == 0 || lane_bits % 8 != 0 || image.len() % lane_bytes != 0 {
            return Self::Unknown;
        }
        let lanes = image.len() / lane_bytes;
        if lanes == 0 || lanes > 32 {
            return Self::Unknown;
        }
        let mut bits = 0u32;
        for (i, lane) in image.chunks_exact(lane_bytes).enumerate() {
            if lane[lane_bytes - 1] & 0x80 != 0 {
                // `i < lanes <= 32`, so `i <= 31`; no shift overflow.
                bits |= 1u32 << i;
            }
        }
        let full = if lanes == 32 {
            u32::MAX
        } else {
            (1u32 << lanes) - 1
        };
        if bits == 0 {
            Self::AllOff
        } else if bits == full {
            Self::AllOn
        } else {
            Self::Mixed(bits)
        }
    }

    // NOTE: there is deliberately ONE classification entry point.  A former
    // `from_bytes` helper returned `Unknown` for a mixed mask while
    // `from_lanes(_, 8)` returns `Mixed(bits)`; two verdict granularities for
    // the same lattice were a design smell and made blend classification
    // needlessly conservative.  `from_lanes(image, 8)` is exactly the old
    // byte-wise test for AllOn/AllOff, and `from_lanes(image, 32/64)` is the
    // exact dword/qword selector test.  Callers that only care about
    // AllOn/AllOff treat `Mixed` as "no fold", so the behaviour is identical
    // where it matters and strictly stronger where it can be proved.
}

/// `(number of lanes, lane width in bits)` for a movemask intrinsic.
fn movemask_lanes(op: IntrinsicOp) -> Option<(usize, usize)> {
    use IntrinsicOp as O;
    Some(match op {
        O::Pmovmskb128 => (16, 8),
        O::Pmovmskb256 => (32, 8),
        O::MovemaskPs128 => (4, 32),
        O::MovemaskPd128 => (2, 64),
        O::MovemaskPs256 => (8, 32),
        O::MovemaskPd256 => (4, 64),
        _ => return None,
    })
}

fn is_movemask(op: IntrinsicOp) -> bool {
    movemask_lanes(op).is_some()
}

/// Byte width of a zero-vector producer.  ISPC treats a
/// `ConstantAggregateZero` mask as `all_off`; LCCC has no vector constant, so
/// the `VecZero*` family is the equivalent.  The width MUST be checked: a
/// 128-bit zero used by a 256-bit movemask leaves the upper half undefined on
/// legacy SSE, and claiming 32 zero bytes would be a miscompile.  Returning
/// `None` for a width mismatch is fail-closed.
fn zero_vector_width(op: IntrinsicOp) -> Option<usize> {
    use IntrinsicOp as O;
    Some(match op {
        O::VecZeroI32x4 | O::VecZeroI64x2 | O::VecZeroF32x4 | O::VecZeroF64x2 => 16,
        O::VecZeroI32x8 | O::VecZeroI64x4 | O::VecZeroF32x8 | O::VecZeroF64x4 => 32,
        _ => return None,
    })
}

/// `(byte width of the resulting vector, byte width of the splatted lane)`
/// for a broadcast/splat producer whose single data operand is a constant.
fn splat_shape(op: IntrinsicOp) -> Option<(usize, usize)> {
    use IntrinsicOp as O;
    Some(match op {
        O::SetEpi8 => (16, 1),
        O::SetEpi16 => (16, 2),
        O::SetEpi32 => (16, 4),
        O::SetEpi8_256 => (32, 1),
        O::SetEpi16_256 => (32, 2),
        O::SetEpi32_256 => (32, 4),
        O::SetEpi64x256 => (32, 8),
        O::VecBroadcastI8x16 => (16, 1),
        O::VecBroadcastI16x8 => (16, 2),
        O::VecBroadcastI32x4 => (16, 4),
        O::VecBroadcastI64x2 => (16, 8),
        O::VecBroadcastF32x4 => (16, 4),
        O::VecBroadcastF64x2 => (16, 8),
        O::VecBroadcastI8x32 => (32, 1),
        O::VecBroadcastI16x16 => (32, 2),
        O::VecBroadcastI32x8 => (32, 4),
        O::VecBroadcastI64x4 => (32, 8),
        O::VecBroadcastF32x8 => (32, 4),
        O::VecBroadcastF64x4 => (32, 8),
        _ => return None,
    })
}

/// A byte-wise, COMMUTATIVE, lane-width-independent vector operation.
///
/// Deliberately restricted to operations whose result byte depends only on
/// the two corresponding input bytes.  Commutativity removes every operand-
/// order hazard (`pcmpeqb`, `pand`, `por`, `pxor` are symmetric); byte-wise
/// semantics remove every lane-width hazard (a `pand` on I8x16, I32x4 or
/// F64x2 produces the identical 16-byte result).  Non-commutative compares
/// (`pcmpgtb`) and saturated arithmetic are NOT here: their operand order or
/// saturation rule is backend-specific and would need a separate proof.
#[derive(Clone, Copy)]
enum ByteOp {
    And,
    Or,
    Xor,
    /// Byte equality -> 0xFF when equal, 0x00 otherwise (`pcmpeqb`).
    Eq,
}

impl ByteOp {
    #[inline]
    fn apply(self, a: u8, b: u8) -> u8 {
        match self {
            Self::And => a & b,
            Self::Or => a | b,
            Self::Xor => a ^ b,
            Self::Eq => (a == b) as u8 * 0xFF,
        }
    }
}

/// `(result byte width, operation)` for the constant-propagatable vector
/// ops.  Both the legacy SSE `dest_ptr` forms (`Pand128`, ...) and the modern
/// SSA forms (`VecAndI32x4`, ...) are included; the SAME byte function is
/// exact on x86-64, i686, AArch64 and RISC-V because these are bitwise
/// register operations.
fn byte_logic_shape(op: IntrinsicOp) -> Option<(usize, ByteOp)> {
    use IntrinsicOp as O;
    Some(match op {
        O::Pand128 | O::VecAndI8x16 | O::VecAndI16x8 | O::VecAndI32x4 | O::VecAndI64x2 => {
            (16, ByteOp::And)
        }
        O::Por128 | O::VecOrI8x16 | O::VecOrI16x8 | O::VecOrI32x4 | O::VecOrI64x2 => {
            (16, ByteOp::Or)
        }
        O::Pxor128
        | O::VecXorI8x16
        | O::VecXorI16x8
        | O::VecXorI32x4
        | O::VecXorI64x2
        | O::VecXorF32x4
        | O::VecXorF64x2 => (16, ByteOp::Xor),
        O::Pcmpeqb128 => (16, ByteOp::Eq),
        O::Pand256 | O::VecAndI8x32 | O::VecAndI16x16 | O::VecAndI32x8 | O::VecAndI64x4 => {
            (32, ByteOp::And)
        }
        O::Por256 | O::VecOrI8x32 | O::VecOrI16x16 | O::VecOrI32x8 | O::VecOrI64x4 => {
            (32, ByteOp::Or)
        }
        O::Pxor256
        | O::VecXorI8x32
        | O::VecXorI16x16
        | O::VecXorI32x8
        | O::VecXorI64x4
        | O::VecXorF32x8
        | O::VecXorF64x4 => (32, ByteOp::Xor),
        O::Pcmpeqb256 => (32, ByteOp::Eq),
        _ => return None,
    })
}

/// Legacy `SetEpi*` splats that WRITE their result through `dest_ptr`.
/// Kept separate from `splat_shape` (which also covers the SSA-value
/// `VecBroadcast*` family): those do not write memory and must not be
/// mistaken for a store by `object_image`.
fn dest_ptr_splat_shape(op: IntrinsicOp) -> Option<(usize, usize)> {
    use IntrinsicOp as O;
    Some(match op {
        O::SetEpi8 => (16, 1),
        O::SetEpi16 => (16, 2),
        O::SetEpi32 => (16, 4),
        O::SetEpi8_256 => (32, 1),
        O::SetEpi16_256 => (32, 2),
        O::SetEpi32_256 => (32, 4),
        O::SetEpi64x256 => (32, 8),
        _ => return None,
    })
}

/// `(mask byte width, selector lane bits)` for the LEGACY `Blendv*128/256`
/// intrinsics emitted by the C `_mm_blendv_ps/pd` family.  Their argument
/// order is `[mask, false_arm, true_arm]` — confirmed by the emitters
/// (`intrinsics_simd.rs:1758` comment "operands (mask, a, b)"): AllOff keeps
/// `a`, AllOn takes `b`.  `Pblendvb128` is deliberately NOT here: it uses a
/// DIFFERENT order (`[a, b, mask]`, asserted at `intrinsics.rs:3322`) and
/// would need a separate proof.
fn legacy_blend_shape(op: IntrinsicOp) -> Option<(usize, usize)> {
    use IntrinsicOp as O;
    Some(match op {
        O::BlendvPs128 => (16, 32),
        O::BlendvPd128 => (16, 64),
        O::BlendvPs256 => (32, 32),
        O::BlendvPd256 => (32, 64),
        _ => return None,
    })
}

/// `(mask byte width, selector lane bits)` for the `VecBlendv*` family.
///
/// The selector lane width is NOT the payload lane width — it is the width of
/// the mask field the hardware actually reads:
///
/// * `vblendvps` (F32x4/F32x8/I32x4/I32x8) keys on bit 31 of each DWORD, so
///   the low three bytes of a dword mask are don't-cares.
/// * `vblendvpd` (F64x2/F64x4) keys on bit 63 of each QWORD.
/// * `vpblendvb` (I8x*/I16x*) keys on bit 7 of EVERY byte.  The I16 forms
///   are `vpblendvb` too: the backend requires both bytes of a word to agree,
///   so byte-granular classification is the exact sufficient condition.
///
/// Reading only the last byte of each selector lane is therefore exact in
/// little-endian IR byte images (`IrConst::to_le_bytes`), and it folds masks
/// like `_mm_set1_epi32(INT_MIN)` that a byte-wise test wrongly calls mixed.
fn blend_shape(op: IntrinsicOp) -> Option<(usize, usize)> {
    use IntrinsicOp as O;
    Some(match op {
        O::VecBlendvF32x4 | O::VecBlendvI32x4 => (16, 32),
        O::VecBlendvF32x8 | O::VecBlendvI32x8 => (32, 32),
        O::VecBlendvF64x2 => (16, 64),
        O::VecBlendvF64x4 => (32, 64),
        O::VecBlendvI8x16 | O::VecBlendvI16x8 => (16, 8),
        O::VecBlendvI8x32 | O::VecBlendvI16x16 => (32, 8),
        _ => return None,
    })
}

/// Byte width written by a recognized whole-register `VecStore*`.
///
/// `VecStoreI32x4Pair` is deliberately EXCLUDED: `vector_temp_promotion`
/// answers `Some(8)` for it because it is a HALF-wide `movq` of the low 8
/// bytes of a 16-byte XMM source (see the x86 emitter and
/// `src/passes/alias.rs:155`).  Treating its `args[0]` as an 8-byte vector
/// *value* would conflate "low half of a 16-byte vector" with "an 8-byte
/// vector", which is not a proof.  Refusing the whole object is fail-closed
/// and costs nothing: this op never carries the constant images we fold.
fn whole_store_width(op: IntrinsicOp) -> Option<usize> {
    if matches!(op, IntrinsicOp::VecStoreI32x4Pair) {
        return None;
    }
    whole_vector_store_width(op)
}

/// Bytes a scalar `Store` writes, given its declared `IrType` width.
///
/// LCCC's front end frequently spells a narrow byte store as
/// `Store { val: Const(I64(255)), ty: U8 }` (measured in the 32-lane
/// `_mm256_setr_epi8` lowering).  The x86 emitter's `direct_store_imm`
/// documents the contract: for destinations narrower than 8 bytes the
/// immediate is truncated to the destination's raw field.  Blindly taking
/// all eight `to_le_bytes()` would claim an 8-byte write where the backend
/// writes one byte and would leave 7/8 of the image unproven.
///
/// To stay backend-independent we accept a truncation ONLY when it is
/// unambiguous: the bytes beyond `ty_size` must be the exact zero- or
/// sign-extension of the retained low part.  Then every backend agrees on
/// the value, and the retained bytes are its little-endian image.  Anything
/// else (a float in an integer slot, `I128` narrowed to 8, a value whose
/// high bytes are neither 0x00 nor 0xFF) refuses.
fn const_store_bytes(c: IrConst, ty_size: usize) -> Option<Vec<u8>> {
    if ty_size == 0 {
        return None;
    }
    // `IrConst::Zero` is the width-independent zero of every integer width.
    if matches!(c, IrConst::Zero) {
        return Some(vec![0u8; ty_size]);
    }
    let bytes = c.to_le_bytes();
    if bytes.len() == ty_size {
        return Some(bytes);
    }
    if bytes.len() < ty_size || ty_size > 8 {
        return None;
    }
    let low = &bytes[..ty_size];
    // Both extension forms are exact: the front end emits `I64(255)` for a
    // `U8` byte store (zero-extended) and `I64(-1)` for the same byte after
    // a signed literal (sign-extended).  Both truncate to the low byte.
    // A mixed high tail (e.g. `I64(0x1FF)`) has no single value at the
    // narrowed width and is refused.
    let zero_ext = bytes[ty_size..].iter().all(|b| *b == 0x00);
    let sign_ext = bytes[ty_size..].iter().all(|b| *b == 0xFF);
    if !zero_ext && !sign_ext {
        return None;
    }
    Some(low.to_vec())
}

/// Rebuild an `IrConst` from the exact little-endian byte image of a proven
/// constant object (ISPC `ImproveMemoryOps` / `InstructionSimplify`'s
/// load-of-constant).  The constructed constant is VERIFIED by round-tripping
/// it through `const_store_bytes` at the load's declared width: if the chosen
/// spelling does not serialize back to EXACTLY `bytes`, the fold refuses.
///
/// That check is not decoration.  LCCC spells unsigned sub-64-bit constants as
/// zero-extended `IrConst::I64` (`IrConst::from_i64`, `constants.rs:640`) so
/// `to_i64()` does not sign-extend them; a narrow `I8`/`I32` spelling would
/// silently change `U8(255)` into `-1`.
///
/// Widths above 8 bytes are deliberately OUT of scope.  A measured CI
/// regression (`sse34_encoding_emission`, rc=3) showed that folding a
/// `Load I128` into `Copy { Const(I128) }` lets the late constant folder
/// substitute that constant into an `InlineAsm` `"x"` input, which the x86
/// backend materialises as ZERO (`movl $0,%eax; movq %rax,%xmm0`) instead of
/// the 128-bit value.  The IR fold was correct; the backend path is not.
/// Fail closed: only byte/word/dword/qword loads, which are proven in
/// `fx6.c` and `loads.c`, are folded.  BID decimal containers (`D32`/`D64`),
/// `F128` (target-dependent 12/16-byte x87 or binary128 with a lossy `f64`
/// field) and `Ptr` (no `IrConst` encodes a stack address) are refused too.
fn const_from_bytes(ty: IrType, bytes: &[u8]) -> Option<IrConst> {
    use IrType as T;
    let n = ty.size();
    // `> 8` is the fail-closed backend-materialisation boundary above.
    if n == 0 || n > 8 || bytes.len() != n {
        return None;
    }
    let mut buf = [0u8; 8];
    buf[..n].copy_from_slice(bytes);
    let c = match ty {
        T::F32 => IrConst::F32(f32::from_bits(u32::from_le_bytes(
            buf[..4].try_into().ok()?,
        ))),
        T::F64 => IrConst::F64(f64::from_bits(u64::from_le_bytes(
            buf[..8].try_into().ok()?,
        ))),
        // D32/D64 (C23 BID decimal bit containers) and F128 are refused:
        // their `Copy { Const }` lowering goes through target-specific helper
        // paths that this pass has not proved.  Fail closed rather than
        // assume a bit container is interchangeable with its bit pattern.
        T::I8 | T::U8 | T::I16 | T::U16 | T::I32 | T::U32 | T::I64 | T::U64 => {
            let mut low = [0u8; 8];
            low[..n].copy_from_slice(&buf[..n]);
            IrConst::from_i64(i64::from_le_bytes(low), ty)
        }
        // Ptr / F128 / Void: no exact, target-independent constant spelling.
        _ => return None,
    };
    // Bit-exactness proof, not a guess: NaN payloads, -0.0, sign bits and
    // unsigned zero-extension all have to survive the round trip.
    (const_store_bytes(c, n) == Some(bytes.to_vec())).then_some(c)
}

/// Is `[off, off+len)` a fully in-bounds window of a plain `Alloca`?  Used to
/// prove a `Memcpy` rewrite cannot read or write past an object.
fn window_in_bounds(
    defs: &FxHashMap<Value, &Instruction>,
    base: Value,
    off: i64,
    len: usize,
) -> bool {
    let Ok(at) = usize::try_from(off) else {
        return false;
    };
    let Some(Instruction::Alloca {
        size,
        volatile,
        semantic_volatile,
        ..
    }) = defs.get(&base)
    else {
        return false;
    };
    !*volatile && !*semantic_volatile && at.checked_add(len).is_some_and(|end| end <= *size)
}

/// Pre-pass snapshot of the function's SSA definitions.  Kept immutable
/// during the rewrite so a fold cannot change the evidence of another fold.
/// Definitions are BORROWED, not cloned: `Facts::new` is O(instructions) with
/// zero heap payload copies, and the two-phase rewrite drops the borrow
/// before touching `IrFunction`.
struct Facts<'a> {
    func: &'a IrFunction,
    defs: FxHashMap<Value, &'a Instruction>,
    /// Every SSA value that is actually READ.  `for_each_used_value` excludes
    /// destinations, so this is exactly the set that must be protected when
    /// an instruction is deleted or replaced.  Built once per snapshot and
    /// shared by `fold` and `dead_private_writes` — one source of truth.
    used: FxHashSet<u32>,
    /// `(block, index)` of every instruction that mentions a value derived
    /// from each alloca, in program order.  Built ONCE.  `object_image` audits
    /// only these mentions instead of rescanning the whole function for every
    /// read: O(n + k) analysis instead of O(n^2).
    mentions: FxHashMap<u32, Vec<(usize, usize)>>,
    /// Allocas whose address appears in a terminator (i.e. escapes).
    escaping: FxHashSet<u32>,
    /// Allocas with at least one recognized constant write.  A cheap gate so
    /// an ordinary scalar load never pays for a full image proof.
    constant_bases: FxHashSet<u32>,
}

/// Verdict for ONE instruction that mentions a constant object under audit.
/// Returning a value (instead of `return`ing from `object_image`) is a safety
/// invariant: every refusal still reaches `visiting.pop()`.
enum ObjectMention {
    /// Pure read or address derivation: the object's image is unchanged.
    NoWrite,
    /// A proven write of `bytes` at `offset`.
    Write(i64, Vec<u8>),
    /// An unprovable memory effect: the whole proof fails closed.
    Reject,
}

/// The immutable facts a mention classifier needs.  `Copy` so it can be
/// threaded through recursive proofs without borrowing the `Facts` snapshot.
#[derive(Clone, Copy)]
struct ImageWindow {
    base: Value,
    size: usize,
    read_block: usize,
    read_index: usize,
}

/// Definition-graph root of an SSA value, following `Copy` and
/// `GetElementPtr` (ANY offset, including variable) with an unbounded walk,
/// memoization and cycle detection.  A free function so `Facts::new` can build
/// the mention index before a `Facts` value exists.  Only these two opcodes
/// preserve provenance: a phi, select, cast, load or call result deliberately
/// does not, which is what makes the non-escape proof fail closed.
fn root_of(
    v: u32,
    defs: &FxHashMap<Value, &Instruction>,
    memo: &mut FxHashMap<u32, Option<u32>>,
    visiting: &mut FxHashSet<u32>,
) -> Option<u32> {
    if let Some(cached) = memo.get(&v) {
        return *cached;
    }
    if !visiting.insert(v) {
        return None; // provenance cycle: refuse
    }
    let result = match defs.get(&Value(v)) {
        Some(Instruction::Alloca { dest, .. }) => Some(dest.0),
        Some(Instruction::Copy {
            src: Operand::Value(next),
            ..
        }) => root_of(next.0, defs, memo, visiting),
        Some(Instruction::GetElementPtr { base, .. }) => root_of(base.0, defs, memo, visiting),
        _ => None,
    };
    visiting.remove(&v);
    memo.insert(v, result);
    result
}

/// Is this instruction a recognized CONSTANT producer that writes an object
/// image?  Used only as a CHEAP PRE-FILTER for load folding; it is never
/// trusted as a proof.  `object_image` still audits every mention and refuses
/// any runtime write, out-of-bounds store or escape.
fn is_constant_write(inst: &Instruction) -> bool {
    match inst {
        Instruction::Store {
            val: Operand::Const(_),
            volatile: false,
            ..
        }
        | Instruction::Memcpy { .. } => true,
        Instruction::Intrinsic { op, .. } => {
            whole_store_width(*op).is_some()
                || byte_logic_shape(*op).is_some()
                || dest_ptr_splat_shape(*op).is_some()
                || legacy_blend_shape(*op).is_some()
        }
        _ => false,
    }
}

/// Build the per-alloca mention index, escape set and constant-write hint in
/// ONE linear pass.  Every definition is classified exactly once; a deep or
/// cyclic `Copy`/`GEP` chain cannot make a use invisible because `root_of`
/// memoizes roots and refuses cycles.
fn build_mention_index(
    func: &IrFunction,
    defs: &FxHashMap<Value, &Instruction>,
) -> (
    FxHashMap<u32, Vec<(usize, usize)>>,
    FxHashSet<u32>,
    FxHashSet<u32>,
) {
    let mut mentions: FxHashMap<u32, Vec<(usize, usize)>> = FxHashMap::default();
    let mut escaping = FxHashSet::default();
    let mut constant_bases = FxHashSet::default();
    let mut memo: FxHashMap<u32, Option<u32>> = FxHashMap::default();
    let mut cycle: FxHashSet<u32> = FxHashSet::default();
    let mut touched: Vec<u32> = Vec::with_capacity(4);
    for (block_id, block) in func.blocks.iter().enumerate() {
        for (index, inst) in block.instructions.iter().enumerate() {
            touched.clear();
            for id in inst.used_values() {
                if let Some(b) = root_of(id, defs, &mut memo, &mut cycle) {
                    if !touched.contains(&b) {
                        touched.push(b);
                    }
                }
            }
            for b in &touched {
                mentions.entry(*b).or_default().push((block_id, index));
                if is_constant_write(inst) {
                    constant_bases.insert(*b);
                }
            }
        }
        for id in block.terminator.used_values() {
            if let Some(b) = root_of(id, defs, &mut memo, &mut cycle) {
                escaping.insert(b);
            }
        }
    }
    (mentions, escaping, constant_bases)
}

impl<'a> Facts<'a> {
    fn new(func: &'a IrFunction) -> Self {
        let mut defs = FxHashMap::default();
        let mut used = FxHashSet::default();
        for block in &func.blocks {
            for inst in &block.instructions {
                if let Some(dest) = inst.dest() {
                    defs.insert(dest, inst);
                }
                used.extend(inst.used_values());
            }
            used.extend(block.terminator.used_values());
        }
        let (mentions, escaping, constant_bases) = build_mention_index(func, &defs);
        Self {
            func,
            defs,
            used,
            mentions,
            escaping,
            constant_bases,
        }
    }

    /// Cheap gate for load folding: does this pointer resolve to an object
    /// that has at least one recognized constant write?  O(MAX_DEPTH).
    fn maybe_constant_load(&self, ptr: Value) -> bool {
        let mut off = 0i64;
        let mut budget = MAX_DEPTH;
        self.resolve_pointer(ptr, &mut off, &mut budget)
            .is_some_and(|base| self.constant_bases.contains(&base.0))
    }

    /// Resolve an SSA value to a stack-object base plus a constant BYTE
    /// offset.  Only `Copy` and constant-offset `GEP` preserve provenance;
    /// a phi, select, cast, load or call result deliberately does not.
    /// LCCC's `GetElementPtr` is `dest = base + offset` in BYTES
    /// (`src/backend/x86/codegen/isel.rs:1343`), so no element scaling is
    /// applied — the same convention `const_array_promote` uses.
    fn resolve_pointer(
        &self,
        mut value: Value,
        offset: &mut i64,
        budget: &mut usize,
    ) -> Option<Value> {
        for _ in 0..MAX_DEPTH {
            if *budget == 0 {
                return None;
            }
            *budget -= 1;
            match self.defs.get(&value) {
                Some(Instruction::Alloca { .. }) => return Some(value),
                Some(Instruction::Copy {
                    src: Operand::Value(next),
                    ..
                }) => value = *next,
                Some(Instruction::GetElementPtr {
                    base,
                    offset: Operand::Const(c),
                    ..
                }) => {
                    *offset = offset.checked_add(c.to_i64()?)?;
                    value = *base;
                }
                _ => return None,
            }
        }
        None
    }

    fn const_offset(op: &Operand) -> Option<i64> {
        match op {
            Operand::Const(c) => c.to_i64(),
            Operand::Value(_) => None,
        }
    }

    /// Which alloca, if any, this value is derived from — following `Copy`
    /// and `GetElementPtr` of ANY offset (including variable), with NO depth
    /// cap.  Used for non-escape auditing, where a long or variable-offset
    /// chain must still be attributed to its root rather than silently
    /// ignored.  Memoized: O(definitions), cycle-safe.
    fn base_of_value(
        &self,
        v: u32,
        memo: &mut FxHashMap<u32, Option<u32>>,
        visiting: &mut FxHashSet<u32>,
    ) -> Option<u32> {
        root_of(v, &self.defs, memo, visiting)
    }

    /// Byte image of a non-vector SSA value produced by a scalar `Load` from
    /// a proven constant object.  This is the i686 aggregate-copy idiom:
    /// `Load { ty: I128 }` from a 16-byte constant object, then
    /// `Store { val: loaded, ty: I128 }` into the vector temporary.
    ///
    /// Only `Load`s of a whole scalar type are accepted; the width must equal
    /// the store's declared width, the pointer must resolve to an `Alloca`,
    /// and the source window must itself be provable.  A volatile load, a
    /// computed value, or a source whose image is not constant refuses.
    #[allow(clippy::too_many_arguments)]
    fn known_scalar_bytes(
        &self,
        v: Value,
        width: usize,
        read_block: usize,
        read_index: usize,
        budget: &mut usize,
        visiting: &mut Vec<Value>,
    ) -> Option<Vec<u8>> {
        if width == 0 || width > 32 {
            return None;
        }
        // The i686 front end inserts a `Copy` between the `Load` and the
        // storing `Store` (measured: `Copy v84 <- v214`).  Follow Copy links
        // with a bounded, cycle-safe walk; anything else refuses.
        let mut cur = v;
        let (ptr, ty) = 'resolve: {
            for _ in 0..MAX_DEPTH {
                let inst = *self.defs.get(&cur)?;
                match inst {
                    Instruction::Copy {
                        src: Operand::Value(next),
                        ..
                    } => {
                        if *budget == 0 {
                            return None;
                        }
                        *budget -= 1;
                        cur = *next;
                    }
                    Instruction::Load {
                        ptr,
                        ty,
                        volatile: false,
                        ..
                    } => break 'resolve (*ptr, *ty),
                    _ => return None,
                }
            }
            // A Copy cycle cannot resolve to a Load: refuse.
            return None;
        };
        if ty.size() != width {
            return None;
        }
        let mut zero = 0i64;
        let base = self.resolve_pointer(ptr, &mut zero, budget)?;
        self.object_image(base, zero, width, read_block, read_index, budget, visiting)
    }

    /// Known byte image of an *SSA vector value* (not an address).
    #[allow(clippy::too_many_arguments)]
    fn known_vector(
        &self,
        operand: Operand,
        width: usize,
        read_block: usize,
        read_index: usize,
        budget: &mut usize,
        visiting: &mut Vec<Value>,
    ) -> Option<Vec<u8>> {
        if *budget == 0 || visiting.len() >= MAX_DEPTH || width == 0 || width > 32 {
            return None;
        }
        *budget -= 1;
        match operand {
            Operand::Const(c) => {
                let bytes = c.to_le_bytes();
                (bytes.len() == width).then_some(bytes)
            }
            Operand::Value(v) => {
                if visiting.contains(&v) {
                    return None;
                }
                visiting.push(v);
                let result = (|| {
                    let inst = *self.defs.get(&v)?;
                    match inst {
                        Instruction::Copy { src, .. } => {
                            self.known_vector(*src, width, read_block, read_index, budget, visiting)
                        }
                        Instruction::Phi { incoming, .. } => {
                            // Only PURE values can reach here: any memory
                            // image is audited by `object_image`, which
                            // rejects mentions outside the read's block.
                            // A phi of equal pure images is therefore a
                            // value, independent of dominance.
                            let mut agreed: Option<Vec<u8>> = None;
                            for (op, _) in incoming {
                                let image = self.known_vector(
                                    *op, width, read_block, read_index, budget, visiting,
                                )?;
                                match &agreed {
                                    None => agreed = Some(image),
                                    Some(first) if *first == image => {}
                                    Some(_) => return None,
                                }
                            }
                            agreed
                        }
                        Instruction::Intrinsic {
                            op,
                            args,
                            dest_ptr: None,
                            ..
                        } => {
                            if zero_vector_width(*op) == Some(width) {
                                return Some(vec![0u8; width]);
                            }
                            if let Some((vec_width, lane_width)) = splat_shape(*op) {
                                if vec_width != width || args.is_empty() {
                                    return None;
                                }
                                let Operand::Const(c) = args[0] else {
                                    return None;
                                };
                                let lane = c.to_le_bytes();
                                // Exact-width ONLY.  Silently zero-extending
                                // an I8 -1 into an I32 lane would produce a
                                // different sign bit than sign extension;
                                // there is no IR evidence of which the
                                // producer intended, so refuse.
                                if lane.len() != lane_width {
                                    return None;
                                }
                                let mut image = Vec::with_capacity(width);
                                while image.len() < width {
                                    image.extend_from_slice(&lane);
                                }
                                image.truncate(width);
                                return Some(image);
                            }
                            if let Some((vec_width, kind)) = byte_logic_shape(*op) {
                                // Vector constant propagation (ISPC
                                // `InstructionSimplify`'s constant-mask
                                // derivation).  Both operands may be SSA
                                // vector values OR pointers to proven
                                // constant objects; `operand_image` handles
                                // both with the SHARED recursion state.
                                if vec_width != width || args.len() != 2 {
                                    return None;
                                }
                                let a = self.operand_image(
                                    args[0], width, read_block, read_index, budget, visiting,
                                )?;
                                let b = self.operand_image(
                                    args[1], width, read_block, read_index, budget, visiting,
                                )?;
                                return Some(
                                    a.into_iter()
                                        .zip(b)
                                        .map(|(x, y)| kind.apply(x, y))
                                        .collect(),
                                );
                            }
                            if whole_vector_load(*op) && args.len() >= 2 {
                                let byte_offset = Self::const_offset(&args[1])?;
                                let Operand::Value(p) = args[0] else {
                                    return None;
                                };
                                let mut zero = 0i64;
                                let base = self.resolve_pointer(p, &mut zero, budget)?;
                                return self.object_image(
                                    base,
                                    zero.saturating_add(byte_offset),
                                    width,
                                    read_block,
                                    read_index,
                                    budget,
                                    visiting,
                                );
                            }
                            None
                        }
                        _ => None,
                    }
                })();
                visiting.pop();
                result
            }
        }
    }

    /// Prove a constant byte image for `[offset, offset+len)` inside one
    /// private `Alloca`.  This is the adaptation layer ISPC's LLVM-based
    /// analysis does not need.  The mention index classifies EVERY mention of

    /// classification lives in `mention_of` / `memcpy_mention` /
    /// `intrinsic_mention`, which return an `ObjectMention` value instead of
    /// returning early.  That is deliberately structural: when they refuse,
    /// control still reaches `visiting.pop()` below, so a rejected proof can
    /// never poison a later one.
    fn object_image(
        &self,
        base: Value,
        offset: i64,
        len: usize,
        read_block: usize,
        read_index: usize,
        budget: &mut usize,
        visiting: &mut Vec<Value>,
    ) -> Option<Vec<u8>> {
        if offset < 0 || len == 0 || len > 32 || visiting.contains(&base) {
            return None;
        }
        let (size, volatile, semantic_volatile) = match self.defs.get(&base) {
            Some(Instruction::Alloca {
                size,
                volatile,
                semantic_volatile,
                ..
            }) => (*size, *volatile, *semantic_volatile),
            _ => return None,
        };
        if volatile || semantic_volatile {
            return None;
        }
        let start = usize::try_from(offset).ok()?;
        if start.checked_add(len)? > size {
            return None;
        }
        visiting.push(base);
        let w = ImageWindow {
            base,
            size,
            read_block,
            read_index,
        };

        // The mention index was built once in `Facts::new` from the same
        // definition graph.  Auditing only THIS object's mentions removes the
        // quadratic rescan: k mentions instead of every instruction of the
        // function for every read.  No mention can hide, because `root_of`
        // follows unbounded `Copy`/`GEP` chains and refuses cycles.
        if self.escaping.contains(&base.0) {
            visiting.pop();
            return None;
        }

        // (byte offset, bytes) writes, in program order.  Only the requested
        // window is materialized, but EVERY write to the object is audited.
        let mut writes: Vec<(i64, Vec<u8>)> = Vec::new();
        let mut rejected = false;

        if let Some(mentions) = self.mentions.get(&base.0) {
            'scan: for &(block_id, index) in mentions {
                if *budget == 0 {
                    rejected = true;
                    break 'scan;
                }
                *budget -= 1;
                let Some(inst) = self
                    .func
                    .blocks
                    .get(block_id)
                    .and_then(|b| b.instructions.get(index))
                else {
                    rejected = true;
                    break 'scan;
                };
                // Only a WRITE is order-sensitive.  A proven `NoWrite`
                // mention (a pure read or address derivation) cannot change
                // the image, so it is harmless anywhere in the CFG -- and in
                // particular AFTER the read.  Requiring every mention to be
                // at-or-before the read was over-conservative: it refused
                // `movemask(and(A,B)) + movemask(cmpeq(A,B))` because the
                // second computed op reads A after the first movemask.
                match self.mention_of(inst, w, budget, visiting) {
                    ObjectMention::NoWrite => {}
                    ObjectMention::Write(off, bytes) => {
                        // A later write, or a write in another block whose
                        // execution order is not proven, could change the
                        // image at the read point.
                        if block_id != read_block || index > read_index {
                            rejected = true;
                            break 'scan;
                        }
                        writes.push((off, bytes));
                    }
                    ObjectMention::Reject => {
                        rejected = true;
                        break 'scan;
                    }
                }
            }
        }

        visiting.pop();
        if rejected {
            return None;
        }
        self.tile_image(w, start, len, writes)
    }

    /// Overlay the proven writes onto the requested window.  Kept separate so
    /// `object_image` stays a scan, not a scan AND a tiling algorithm.
    fn tile_image(
        &self,
        w: ImageWindow,
        start: usize,
        len: usize,
        writes: Vec<(i64, Vec<u8>)>,
    ) -> Option<Vec<u8>> {
        // Exact tiling: no gap, no overlap.  Bytes outside the window are
        // ignored, but a write that overlaps the window twice is refused
        // because store ORDER is not proven.
        let mut image = vec![None::<u8>; len];
        for (write_offset, bytes) in writes {
            let Ok(at) = usize::try_from(write_offset) else {
                return None;
            };
            // A write that extends past the alloca's declared size is a
            // buffer overrun, not a value proof.  Refuse it instead of
            // silently truncating the trailing bytes away (this is what
            // catches a 32-byte `SetEpi8_256` splat into a 16-byte object,
            // and any mismatched `VecStore*` width).
            let Some(write_end) = at.checked_add(bytes.len()) else {
                return None;
            };
            if write_end > w.size {
                return None;
            }
            for (i, byte) in bytes.into_iter().enumerate() {
                let Some(pos) = at.checked_add(i) else {
                    return None;
                };
                if pos < start {
                    continue;
                }
                let slot = pos - start;
                if slot >= len {
                    continue;
                }
                if image[slot].replace(byte).is_some() {
                    return None; // two writes to one byte: order not proven
                }
            }
        }
        image.into_iter().collect()
    }

    /// Classify ONE instruction that mentions the object under audit.  A
    /// value-returning verdict (never an early `return` from `object_image`)
    /// is what makes `visiting.pop()` unconditional.
    fn mention_of(
        &self,
        inst: &Instruction,
        w: ImageWindow,
        budget: &mut usize,
        visiting: &mut Vec<Value>,
    ) -> ObjectMention {
        use ObjectMention::{NoWrite, Reject, Write};
        match inst {
            Instruction::Intrinsic {
                op, args, dest_ptr, ..
            } => self.intrinsic_mention(*op, args, *dest_ptr, w, budget, visiting),
            Instruction::Memcpy {
                dest,
                src,
                size: copy_len,
            } => self.memcpy_mention(*dest, *src, *copy_len, w, budget, visiting),
            // Pure address derivations.  Variable-offset GEPs are rejected
            // by the escape audit in `object_image` (`base` may still be
            // reachable, but the derived address is not provably inside the
            // object), so only constant ones are allowed here.
            Instruction::GetElementPtr {
                offset: Operand::Const(_),
                ..
            }
            | Instruction::Copy { .. } => NoWrite,
            Instruction::GetElementPtr { .. } => Reject,
            Instruction::Load {
                ptr,
                ty,
                volatile: false,
                ..
            } => {
                // A non-volatile scalar read cannot change the image.  It is
                // sound to allow because NO `IrConst` can encode a stack
                // address: a value loaded from a constant object cannot be
                // turned back into a pointer that aliases it.  Pin the
                // pointer to THIS object and the read window inside it, or
                // refuse.  This is the i686 `Load I128` aggregate idiom.
                let mut zero = 0i64;
                let Some(p) = self.resolve_pointer(*ptr, &mut zero, budget) else {
                    return Reject;
                };
                if p != w.base {
                    return Reject;
                }
                let Ok(s) = usize::try_from(zero) else {
                    return Reject;
                };
                let Some(end) = s.checked_add(ty.size()) else {
                    return Reject;
                };
                if end > w.size {
                    return Reject;
                }
                NoWrite
            }
            // A volatile scalar load is an observable read: refuse.
            Instruction::Load { .. } => Reject,
            Instruction::Store {
                val,
                ptr,
                ty,
                volatile,
                ..
            } => {
                if *volatile {
                    return Reject;
                }
                let mut zero = 0i64;
                let Some(p) = self.resolve_pointer(*ptr, &mut zero, budget) else {
                    return Reject;
                };
                if p != w.base {
                    return Reject;
                }
                let bytes = match val {
                    Operand::Const(c) => const_store_bytes(*c, ty.size()),
                    // i686's aggregate-copy lowering: the store's value is a
                    // scalar Load from ANOTHER proven constant object, not an
                    // `IrConst`.
                    Operand::Value(v) => self.known_scalar_bytes(
                        *v,
                        ty.size(),
                        w.read_block,
                        w.read_index,
                        budget,
                        visiting,
                    ),
                };
                let Some(bytes) = bytes else {
                    return Reject;
                };
                Write(zero, bytes)
            }
            // A phi, select, cast, call, inline asm, atomic or arithmetic
            // mention loses provenance, may read, or may write.  Fail closed.
            _ => Reject,
        }
    }

    /// `Memcpy` is BOTH a write to `dest` and a read of `src`.  A
    /// self/overlapping copy is refused: memcpy byte ORDER over an
    /// overlapping range is not a value proof.
    fn memcpy_mention(
        &self,
        dest: Value,
        src: Value,
        copy_len: usize,
        w: ImageWindow,
        budget: &mut usize,
        visiting: &mut Vec<Value>,
    ) -> ObjectMention {
        use ObjectMention::{NoWrite, Reject, Write};
        if copy_len == 0 || copy_len > 32 {
            return Reject;
        }
        let mut d_off = 0i64;
        let Some(d_base) = self.resolve_pointer(dest, &mut d_off, budget) else {
            return Reject;
        };
        let mut s_off = 0i64;
        let Some(s_base) = self.resolve_pointer(src, &mut s_off, budget) else {
            return Reject;
        };
        if d_base == w.base {
            if s_base == w.base {
                return Reject;
            }
            let Some(image) = self.object_image(
                s_base,
                s_off,
                copy_len,
                w.read_block,
                w.read_index,
                budget,
                visiting,
            ) else {
                return Reject;
            };
            return Write(d_off, image);
        }
        if s_base == w.base {
            // A pure read: it cannot change the image.  Pin the read window
            // inside the object, so an out-of-bounds copy cannot become
            // "harmless" by silence.
            let Ok(s) = usize::try_from(s_off) else {
                return Reject;
            };
            let Some(end) = s.checked_add(copy_len) else {
                return Reject;
            };
            if end > w.size {
                return Reject;
            }
            return NoWrite;
        }
        Reject
    }

    /// Intrinsics that touch the object: whole-register vector stores,
    /// byte-wise computed-vector writes through `dest_ptr`, `SetEpi*`
    /// splats, whole-register loads and movemask reads.
    fn intrinsic_mention(
        &self,
        op: IntrinsicOp,
        args: &[Operand],
        dest_ptr: Option<Value>,
        w: ImageWindow,
        budget: &mut usize,
        visiting: &mut Vec<Value>,
    ) -> ObjectMention {
        use ObjectMention::{NoWrite, Reject, Write};
        if let Some(store_width) = whole_store_width(op) {
            // args = [value, ptr, byte_offset]; some producers omit the
            // trailing zero offset.
            if args.len() < 2 {
                return Reject;
            }
            let byte_offset = match args.get(2) {
                Some(off) => match Self::const_offset(off) {
                    Some(o) => o,
                    None => return Reject,
                },
                None => 0,
            };
            let Operand::Value(pp) = args[1] else {
                return Reject;
            };
            let mut zero = 0i64;
            let Some(p) = self.resolve_pointer(pp, &mut zero, budget) else {
                return Reject;
            };
            if p != w.base || dest_ptr.is_some_and(|d| d != pp) {
                return Reject;
            }
            let Some(image) = self.known_vector(
                args[0],
                store_width,
                w.read_block,
                w.read_index,
                budget,
                visiting,
            ) else {
                return Reject;
            };
            return Write(zero.saturating_add(byte_offset), image);
        }
        if let Some((width, selector_bits)) = legacy_blend_shape(op) {
            // A legacy `Blendv*128/256` writes its result through
            // `dest_ptr`.  Prove the mask image, then prove the SELECTED arm
            // image.  A mixed selector cannot be reduced to one constant
            // image, so refuse (the fold rewrite handles the all-on/off
            // cases by emitting a Memcpy instead).
            let Some(dp) = dest_ptr else {
                return Reject;
            };
            let mut d_off = 0i64;
            let Some(d_base) = self.resolve_pointer(dp, &mut d_off, budget) else {
                return Reject;
            };
            if d_base != w.base {
                // It writes a different object and only reads this one
                // through its three pointer args.  Pin every pointer arg.
                for arg in args {
                    if let Operand::Value(p) = arg {
                        let mut zero = 0i64;
                        if self.resolve_pointer(*p, &mut zero, budget).is_none() {
                            return Reject;
                        }
                    }
                }
                return NoWrite;
            }
            if args.len() != 3 {
                return Reject;
            }
            let Some(mask) =
                self.operand_image(args[0], width, w.read_block, w.read_index, budget, visiting)
            else {
                return Reject;
            };
            let chosen = match MaskStatus::from_lanes(&mask, selector_bits) {
                MaskStatus::AllOff => args[1],
                MaskStatus::AllOn => args[2],
                MaskStatus::Mixed(_) | MaskStatus::Unknown => return Reject,
            };
            let Some(image) =
                self.operand_image(chosen, width, w.read_block, w.read_index, budget, visiting)
            else {
                return Reject;
            };
            return Write(d_off, image);
        }
        if byte_logic_shape(op).is_some() || dest_ptr_splat_shape(op).is_some() {
            // Legacy SSE/AVX2 form: the intrinsic WRITES a computed constant
            // vector into the object named by `dest_ptr`.  Both source images
            // must be provable with the SHARED recursion state.
            let Some(dp) = dest_ptr else {
                return Reject;
            };
            let mut d_off = 0i64;
            let Some(d_base) = self.resolve_pointer(dp, &mut d_off, budget) else {
                return Reject;
            };
            if d_base != w.base {
                // This intrinsic writes a DIFFERENT object and merely READS
                // this one through its pointer arguments.  A read cannot
                // change this object's image; pin every pointer argument to a
                // resolved stack object so no derived-address access escapes
                // the audit.
                for arg in args {
                    if let Operand::Value(p) = arg {
                        let mut zero = 0i64;
                        if self.resolve_pointer(*p, &mut zero, budget).is_none() {
                            return Reject;
                        }
                    }
                }
                return NoWrite;
            }
            let image = if let Some((vec_width, kind)) = byte_logic_shape(op) {
                if args.len() != 2 || vec_width == 0 || vec_width > 32 {
                    return Reject;
                }
                let Some(a) = self.operand_image(
                    args[0],
                    vec_width,
                    w.read_block,
                    w.read_index,
                    budget,
                    visiting,
                ) else {
                    return Reject;
                };
                let Some(b) = self.operand_image(
                    args[1],
                    vec_width,
                    w.read_block,
                    w.read_index,
                    budget,
                    visiting,
                ) else {
                    return Reject;
                };
                a.into_iter()
                    .zip(b)
                    .map(|(x, y)| kind.apply(x, y))
                    .collect::<Vec<u8>>()
            } else {
                // Exact-width splat; refuse widened or non-constant operands
                // (truncation of an `I64` into a dword lane is not provable).
                let Some((vec_width, lane_width)) = dest_ptr_splat_shape(op) else {
                    return Reject;
                };
                if args.len() != 1 || vec_width > 32 {
                    return Reject;
                }
                let Operand::Const(c) = args[0] else {
                    return Reject;
                };
                let lane = c.to_le_bytes();
                if lane.len() != lane_width {
                    return Reject;
                }
                let mut img = Vec::with_capacity(vec_width);
                while img.len() < vec_width {
                    img.extend_from_slice(&lane);
                }
                img.truncate(vec_width);
                img
            };
            return Write(d_off, image);
        }
        if whole_vector_load(op) {
            // A whole-register load is a pure read.  Pin its pointer to THIS
            // object and its offset to a constant, or reject (an unknown
            // memory effect through a derived address is not a read we can
            // account for).
            let (Some(Operand::Value(p)), Some(_)) =
                (args.first(), args.get(1).and_then(Self::const_offset))
            else {
                return Reject;
            };
            let mut zero = 0i64;
            if self.resolve_pointer(*p, &mut zero, budget) != Some(w.base) {
                return Reject;
            }
            return NoWrite;
        }
        if is_movemask(op) {
            let Some(Operand::Value(p)) = args.first() else {
                return Reject;
            };
            let mut zero = 0i64;
            if self.resolve_pointer(*p, &mut zero, budget) != Some(w.base) {
                return Reject;
            }
            return NoWrite;
        }
        // Unknown memory effect through this address.
        Reject
    }

    /// Known image of the operand of a movemask / blend mask: either an SSA
    /// vector value or a pointer to a proven constant stack object.
    fn image_of(
        &self,
        operand: Operand,
        width: usize,
        read_block: usize,
        read_index: usize,
    ) -> Option<Vec<u8>> {
        let mut budget = MAX_WORK;
        let mut visiting = Vec::with_capacity(MAX_DEPTH);
        self.operand_image(
            operand,
            width,
            read_block,
            read_index,
            &mut budget,
            &mut visiting,
        )
    }

    /// `image_of` with SHARED recursion state, so a nested vector expression
    /// cannot restart the work budget or lose the cycle set.  This is what
    /// makes constant propagation through `Pand128`/`VecAnd*` chains sound.
    #[allow(clippy::too_many_arguments)]
    fn operand_image(
        &self,
        operand: Operand,
        width: usize,
        read_block: usize,
        read_index: usize,
        budget: &mut usize,
        visiting: &mut Vec<Value>,
    ) -> Option<Vec<u8>> {
        if let Operand::Value(v) = operand {
            let mut zero = 0i64;
            if let Some(base) = self.resolve_pointer(v, &mut zero, budget) {
                return self
                    .object_image(base, zero, width, read_block, read_index, budget, visiting);
            }
        }
        self.known_vector(operand, width, read_block, read_index, budget, visiting)
    }
}

/// The rewrite for one instruction, or `None` when nothing is provable.
fn fold(facts: &Facts<'_>, inst: &Instruction, block: usize, index: usize) -> Option<Instruction> {
    // ISPC `ImproveMemoryOps`: a scalar `Load` from a proven private constant
    // object is itself a constant.  `maybe_constant_load` is an O(MAX_DEPTH)
    // gate, so ordinary functions pay nothing for this consumer.
    if let Instruction::Load {
        dest,
        ptr,
        ty,
        volatile: false,
        seg_override: AddressSpace::Default,
        ..
    } = inst
    {
        if !facts.maybe_constant_load(*ptr) {
            return None;
        }
        let width = ty.size();
        // Do not even pay for an image proof on a width the backend cannot
        // materialise as a `Copy { Const }` (see `const_from_bytes`).
        if width == 0 || width > 8 {
            return None;
        }
        let image = facts.image_of(Operand::Value(*ptr), width, block, index)?;
        let c = const_from_bytes(*ty, &image)?;
        return Some(Instruction::Copy {
            dest: *dest,
            src: Operand::Const(c),
        });
    }

    // Legacy `Blendv*128/256` intrinsics write their result THROUGH
    // `dest_ptr`.  When the selector is provably all-off or all-on, the
    // whole blend is exactly a 16/32-byte copy of one arm: replace it with
    // `Memcpy`, which removes the blendvps/blendvpd/vblendv* instruction.
    // The intrinsic's SSA `dest` is deliberately NOT replaced with a value —
    // refuse whenever it has any user, so no dangling SSA reference can arise.
    if let Instruction::Intrinsic {
        op,
        args,
        dest_ptr: Some(dp),
        dest,
        ..
    } = inst
    {
        if let Some((width, selector_bits)) = legacy_blend_shape(*op) {
            if dest.is_some_and(|d| facts.used.contains(&d.0)) || args.len() != 3 {
                return None;
            }
            let mask = facts.image_of(args[0], width, block, index)?;
            let chosen = match MaskStatus::from_lanes(&mask, selector_bits) {
                MaskStatus::AllOff => args[1],
                MaskStatus::AllOn => args[2],
                MaskStatus::Mixed(_) | MaskStatus::Unknown => return None,
            };
            let Operand::Value(src) = chosen else {
                return None;
            };
            // A `Memcpy` is only sound when BOTH windows are in bounds and
            // the objects are distinct (memcpy overlap is not a value proof).
            // `image_of` above proved the MASK; it says nothing about the
            // SELECTED ARM, which may be a runtime value in a smaller object.
            // Emitting an unchecked copy would be a buffer over-read.
            let mut budget = MAX_WORK;
            let mut d_off = 0i64;
            let mut s_off = 0i64;
            let Some(d_base) = facts.resolve_pointer(*dp, &mut d_off, &mut budget) else {
                return None;
            };
            let Some(s_base) = facts.resolve_pointer(src, &mut s_off, &mut budget) else {
                return None;
            };
            if d_base == s_base
                || !window_in_bounds(&facts.defs, d_base, d_off, width)
                || !window_in_bounds(&facts.defs, s_base, s_off, width)
            {
                return None;
            }
            return Some(Instruction::Memcpy {
                dest: *dp,
                src,
                size: width,
            });
        }
    }

    let Instruction::Intrinsic {
        dest,
        op,
        args,
        dest_ptr: None,
        ..
    } = inst
    else {
        return None;
    };
    let dest = (*dest)?;

    if let Some((width, selector_bits)) = blend_shape(*op) {
        // args = [false_vec, true_vec, mask]; pinned by
        // `emit_sse_blendv_128`'s `assert!(args.len() == 3, "expects false,
        // true, mask")`.  ISPC's blend arm.
        if args.len() != 3 {
            return None;
        }
        if args[0] == args[1] {
            return Some(Instruction::Copy { dest, src: args[0] });
        }
        let image = facts.image_of(args[2], width, block, index)?;
        // Classify by the selector lane the hardware actually reads, not by
        // every byte: a `0x80000000` dword mask is ALL-ON, not mixed.
        return match MaskStatus::from_lanes(&image, selector_bits) {
            MaskStatus::AllOff => Some(Instruction::Copy { dest, src: args[0] }),
            MaskStatus::AllOn => Some(Instruction::Copy { dest, src: args[1] }),
            MaskStatus::Mixed(_) | MaskStatus::Unknown => None,
        };
    }

    let (lanes, lane_bits) = movemask_lanes(*op)?;
    let width = lanes * (lane_bits / 8);
    let image = facts.image_of(args.first().copied()?, width, block, index)?;
    let bits = match MaskStatus::from_lanes(&image, lane_bits) {
        MaskStatus::AllOff => 0u32,
        MaskStatus::AllOn => {
            if lanes == 32 {
                u32::MAX
            } else {
                (1u32 << lanes) - 1
            }
        }
        MaskStatus::Mixed(bits) => bits,
        MaskStatus::Unknown => return None,
    };
    // `pmovmskb`/`movmskps` produce a 32-bit GPR result.  `I32` is the exact
    // IR type; the all-on 32-lane case is `u32::MAX`, whose bit pattern as
    // `i32` is -1, exactly what `vpmovmskb %ymm, %eax` leaves in `%eax`.
    Some(Instruction::Copy {
        dest,
        src: Operand::Const(IrConst::I32(bits as i32)),
    })
}

pub(crate) fn run(module: &mut IrModule) -> usize {
    module.for_each_function(run_function)
}

fn run_function(func: &mut IrFunction) -> usize {
    let has_candidate = func.blocks.iter().any(|block| {
        block.instructions.iter().any(|inst| {
            if let Instruction::Intrinsic { op, dest_ptr, .. } = inst {
                return match dest_ptr {
                    None => is_movemask(*op) || blend_shape(*op).is_some(),
                    Some(_) => legacy_blend_shape(*op).is_some(),
                };
            }
            // A non-volatile, default-address-space scalar load may be a
            // load-of-constant.  `maybe_constant_load` gates the expensive
            // proof; this gate only decides whether to build `Facts` at all.
            matches!(
                inst,
                Instruction::Load {
                    volatile: false,
                    seg_override: AddressSpace::Default,
                    ..
                }
            )
        })
    });
    if !has_candidate {
        return 0;
    }
    // Two-phase: every decision is made against the same immutable snapshot,
    // and the `Facts` borrow of `func` ends before the first mutation.
    let edits: Vec<(usize, usize, Instruction)> = {
        let facts = Facts::new(func);
        let mut edits = Vec::new();
        for (block_id, block) in func.blocks.iter().enumerate() {
            for (index, inst) in block.instructions.iter().enumerate() {
                if let Some(replacement) = fold(&facts, inst, block_id, index) {
                    edits.push((block_id, index, replacement));
                }
            }
        }
        edits
    };
    let count = edits.len();
    for (block_id, index, replacement) in edits {
        func.blocks[block_id].instructions[index] = replacement;
    }
    if count > 0 {
        prune_dead_private_writes(func);
        // ORDERING FIX.  This hook runs at phase 11e-pre, AFTER every
        // scheduled `constant_fold::run` in the pipeline.  A load folded
        // here creates a NEW `IrConst` that flows into casts and arithmetic
        // no later folder will see — measured on the all-on dword blend
        // probe: `Cast { Const(F32(5.0)), F32 -> F64 }` survived into
        // codegen as `movss .LCFP + cvtss2sd`, while GCC 16.2 and Clang
        // 23.1.5 emit one `movsd`.  Re-run the canonical local folder so
        // the evidence consumer's output is fully simplified.
        //
        // SCOPED to this function (not the whole module) and REFUSED when
        // the function contains `InlineAsm`: inline asm is opaque to the
        // optimizer, and propagating a late constant into an asm operand is
        // a measured miscompile (`sse34_encoding_emission`).  No new verify
        // hooks are introduced.
        let opaque_asm = func.blocks.iter().any(|b| {
            b.instructions
                .iter()
                .any(|i| matches!(i, Instruction::InlineAsm { .. }))
        });
        if !opaque_asm {
            super::constant_fold::fold_function(func);
        }
    }
    count
}

/// After a fold removes the last reader, a private constant object's
/// initialization is dead.  Delete it so the backend emits no residual
/// traffic.  This is a SELF-CONTAINED proof — it re-derives non-escape from
/// scratch and does not trust `object_image`'s verdict.
///
/// Legality, for one alloca `A` (all four conditions, fail-closed):
///
/// 1. `A` is a non-volatile, non-semantic-volatile `Alloca`.
/// 2. Every instruction that mentions a value derived from `A` is exactly
///    one of: (a) a non-volatile scalar `Store` whose pointer derives from
///    `A` and which mentions no other object; (b) a recognized whole-register
///    `VecStore*` writing to `A`, with a matching `dest_ptr` and no other
///    object mentioned; (c) a `Copy` / `GetElementPtr` whose `dest` also
///    derives from `A` (a pure address derivation).  A scalar/vector load, a
///    call, a phi, a cast, an unknown intrinsic, an atomic, inline asm, a
///    volatile/partial store, or any other instruction VETOES `A`.
/// 3. No terminator mentions a value derived from `A`.
/// 4. `base_of_value` follows `Copy`/`GEP` chains of ANY length (including
///    variable offsets) with cycle detection, so a deep derivation cannot
///    make its uses invisible.
///
/// Under (1)-(4) nothing can observe `A`'s contents: (c) only creates more
/// addresses, (a)/(b) only write, and no read or escape exists.  Removing
/// (a)/(b) is therefore semantics-preserving on every backend.
///
/// Rounds are needed because deleting a `VecStore*` makes the producing
/// `VecLoad*` dead; `dce` removes it, which in turn makes the SOURCE object's
/// image dead.  Each round must delete at least one instruction, so the loop
/// is bounded by `MAX_DEPTH` and terminates.
fn prune_dead_private_writes(func: &mut IrFunction) -> usize {
    let mut total = 0usize;
    for _ in 0..MAX_DEPTH {
        let dead: Vec<(usize, usize)> = {
            let facts = Facts::new(func);
            dead_private_writes(&facts)
        };
        if dead.is_empty() {
            break;
        }
        let removed = remove_instructions(func, &dead);
        total += removed;
        if removed == 0 {
            // No progress: do not spin for the remaining rounds.  This is the
            // fixpoint's termination guarantee on top of MAX_DEPTH.
            break;
        }
        // Derivations whose only users were the deleted writes are now dead.
        super::dce::eliminate_dead_code(func);
    }
    total
}

/// One round of the dead-write decision.  Returns `(block, index)` pairs in
/// ascending order, ready for a single stable deletion pass.
fn dead_private_writes(facts: &Facts<'_>) -> Vec<(usize, usize)> {
    let mut bases: FxHashSet<u32> = FxHashSet::default();
    let mut memo: FxHashMap<u32, Option<u32>> = FxHashMap::default();
    let mut cycle: FxHashSet<u32> = FxHashSet::default();
    for block in &facts.func.blocks {
        for inst in &block.instructions {
            if let Instruction::Alloca {
                dest,
                volatile: false,
                semantic_volatile: false,
                ..
            } = inst
            {
                bases.insert(dest.0);
            }
        }
    }
    if bases.is_empty() {
        return Vec::new();
    }

    // `remove_instructions` has no dangling-use check.  Legacy SSE/AVX2
    // intrinsics carry BOTH `dest_ptr` and an SSA `dest`, so deleting one
    // whose `dest` still has a user would leave an undefined value.  Use the
    // shared `Facts::used` set (built once, excludes destinations).
    let used_anywhere = &facts.used;

    let mut veto: FxHashSet<u32> = FxHashSet::default();
    // (block, index, base) for instructions that are legal to delete.  A
    // candidate is only emitted if its base survives the veto.
    let mut candidates: Vec<(usize, usize, u32)> = Vec::new();
    let mut work = MAX_WORK;

    'outer: for (block_id, block) in facts.func.blocks.iter().enumerate() {
        for (index, inst) in block.instructions.iter().enumerate() {
            if work == 0 {
                // Budget exhausted: refuse every candidate this round and
                // stop walking the function (a bare `break` here used to
                // resume the next block with zero work, burning cycles).
                veto.extend(bases.iter().copied());
                break 'outer;
            }
            work -= 1;

            // `Memcpy` is BOTH a write to `dest` and a read of `src`, so it
            // needs a two-sided verdict that the generic "one write to one
            // object" classifier cannot express.  Deleting it is legal only
            // when `dest`'s object has no reader; `src`'s object must keep
            // its image until this copy is gone, hence the veto.  A
            // bounded fixpoint then reclaims the source in a later round.
            if let Instruction::Memcpy {
                dest,
                src,
                size: copy_len,
            } = inst
            {
                let d = facts
                    .base_of_value(dest.0, &mut memo, &mut cycle)
                    .filter(|b| bases.contains(b));
                let s = facts
                    .base_of_value(src.0, &mut memo, &mut cycle)
                    .filter(|b| bases.contains(b));
                if let Some(db) = d {
                    if s == Some(db) || *copy_len == 0 || *copy_len > 32 {
                        veto.insert(db); // self-copy or unaudited width
                    } else {
                        candidates.push((block_id, index, db));
                    }
                }
                if let Some(sb) = s {
                    veto.insert(sb); // the source is READ by this copy
                }
                continue;
            }

            // Legacy SSE/AVX2 intrinsics write a computed vector THROUGH
            // `dest_ptr` and may read their source objects.  Like `Memcpy`
            // this needs a two-sided verdict: the destination write is a
            // deletion candidate, every pointer argument is a read that
            // vetoes its object until this instruction is gone, and the
            // bounded fixpoint reclaims the sources in a later round.
            if let Instruction::Intrinsic {
                op,
                args,
                dest_ptr: Some(dp),
                dest,
                ..
            } = inst
            {
                if byte_logic_shape(*op).is_some()
                    || dest_ptr_splat_shape(*op).is_some()
                    || legacy_blend_shape(*op).is_some()
                {
                    let d = facts
                        .base_of_value(dp.0, &mut memo, &mut cycle)
                        .filter(|b| bases.contains(b));
                    // `remove_instructions` has no dangling-use check.  If
                    // this intrinsic's SSA result still has a user, deleting
                    // it would leave an undefined value: veto its object
                    // instead of nominating the instruction.
                    let dest_live = dest.is_some_and(|v| used_anywhere.contains(&v.0));
                    if let Some(db) = d {
                        if dest_live {
                            veto.insert(db);
                        } else {
                            candidates.push((block_id, index, db));
                        }
                    }
                    for arg in args {
                        if let Operand::Value(v) = arg {
                            if let Some(sb) = facts
                                .base_of_value(v.0, &mut memo, &mut cycle)
                                .filter(|b| bases.contains(b))
                            {
                                veto.insert(sb);
                            }
                        }
                    }
                    continue;
                }
            }

            let mut touched: Vec<u32> = Vec::with_capacity(2);
            for id in inst.used_values() {
                if let Some(b) = facts.base_of_value(Value(id).0, &mut memo, &mut cycle) {
                    if bases.contains(&b) && !touched.contains(&b) {
                        touched.push(b);
                    }
                }
            }
            if touched.is_empty() {
                continue;
            }

            // Classify the instruction ONCE, then apply the verdict to each
            // touched base.  `write_base` is `Some` only for a clean write to
            // a single object; `derivation_base` only for pure address math.
            let write_base: Option<u32> = match inst {
                Instruction::Store {
                    ptr,
                    volatile: false,
                    ..
                } => facts.base_of_value(ptr.0, &mut memo, &mut cycle),
                Instruction::Intrinsic {
                    op, args, dest_ptr, ..
                } if whole_store_width(*op).is_some() && args.len() >= 2 => match args[1] {
                    Operand::Value(p) => {
                        let b = facts.base_of_value(p.0, &mut memo, &mut cycle);
                        let dest_ok = dest_ptr
                            .is_none_or(|d| facts.base_of_value(d.0, &mut memo, &mut cycle) == b);
                        if dest_ok { b } else { None }
                    }
                    _ => None,
                },
                _ => None,
            };
            let derivation_base: Option<u32> = match inst {
                Instruction::Copy { dest, .. } | Instruction::GetElementPtr { dest, .. } => {
                    facts.base_of_value(dest.0, &mut memo, &mut cycle)
                }
                _ => None,
            };

            for b in &touched {
                if touched.len() == 1 && write_base == Some(*b) {
                    candidates.push((block_id, index, *b));
                } else if derivation_base == Some(*b) {
                    // Pure derivation: keep it now; `dce` removes it once
                    // its users are gone.  It is NOT a reader.
                } else {
                    veto.insert(*b);
                }
            }
        }

        // A terminator that names the address lets it escape.
        for id in block.terminator.used_values() {
            if let Some(b) = facts.base_of_value(Value(id).0, &mut memo, &mut cycle) {
                if bases.contains(&b) {
                    veto.insert(b);
                }
            }
        }
    }

    candidates
        .into_iter()
        .filter_map(|(block_id, index, base)| (!veto.contains(&base)).then_some((block_id, index)))
        .collect()
}

/// Stable deletion of `(block, index)` pairs, keeping `source_spans`
/// parallel.  `BasicBlock`'s documented invariant is that `source_spans` is
/// either empty or exactly `instructions.len()` long.
fn remove_instructions(func: &mut IrFunction, dead: &[(usize, usize)]) -> usize {
    let mut per_block: FxHashMap<usize, Vec<usize>> = FxHashMap::default();
    for (block_id, index) in dead {
        per_block.entry(*block_id).or_default().push(*index);
    }
    for idxs in per_block.values_mut() {
        idxs.sort_unstable();
        idxs.dedup();
    }
    let mut removed = 0usize;
    for (block_id, idxs) in per_block {
        let Some(block) = func.blocks.get_mut(block_id) else {
            continue;
        };
        let before = block.instructions.len();
        let span = std::mem::take(&mut block.source_spans);
        let parallel = span.len() == before;
        let mut keep: Vec<Instruction> = Vec::with_capacity(before);
        let mut new_span: Vec<_> = Vec::with_capacity(before);
        let mut di = 0usize;
        for (i, inst) in block.instructions.drain(..).enumerate() {
            if di < idxs.len() && idxs[di] == i {
                di += 1;
                removed += 1;
                continue;
            }
            if parallel {
                new_span.push(span[i].clone());
            }
            keep.push(inst);
        }
        block.instructions = keep;
        block.source_spans = if parallel { new_span } else { span };
    }
    removed
}

#[cfg(test)]
#[path = "simd_mask_facts_tests.rs"]
mod tests;
