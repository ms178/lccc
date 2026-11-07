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
//! This module supplies the lattice and three exact consumers:
//!
//! 1. **Movemask fold** — `Pmovmskb128/256`, `MovemaskPs/Pd128/256` on a known
//!    vector image become an `I32` constant (ISPC `InstructionSimplify`'s
//!    `lSimplifyCall` / `IntrinsicsOpt`'s mask-instruction arm).
//! 2. **Blend triviality** — a `VecBlendv*` with an all-on or all-off mask, or
//!    with identical arms, becomes a `Copy` (ISPC `IntrinsicsOpt`'s blend arm
//!    and `InstructionSimplify`'s `lSimplifySelect`).
//! 3. **Private-image reclamation** — after (1) or (2) removes the last
//!    reader of a private constant object, its initialization writes are
//!    deleted.  Without this the fold saves the `pmovmskb`/NEON reduction but
//!    leaves the whole constant image in the instruction stream, which is not
//!    a win.  ISPC does not need it because LLVM constant vectors are SSA
//!    values with no emitted initialization.
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
//!   length, including variable offsets — see `base_of_value`) is in the SAME
//!   basic block as the read, at or before the read;
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

    /// Sound *all-on / all-off* classification that is independent of the
    /// lane width: if every byte's sign bit agrees, every lane of every
    /// packed width agrees too.  Used for blends, whose masks are consumed
    /// byte-wise by `pblendvb`/`vpblendvb` and dword-wise by `vblendvps`.
    /// A disagreement is `Unknown`, never a guess at a lane boundary.
    ///
    /// This is deliberately conservative: a well-formed dword mask such as
    /// `0x80000000` per lane has mixed payload bytes and is reported
    /// `Unknown`.  Failing closed is cheaper than maintaining a per-intrinsic
    /// lane-granularity table for ten blend ops across four backends.
    pub(crate) fn from_bytes(image: &[u8]) -> Self {
        if image.is_empty() {
            return Self::Unknown;
        }
        let any = image.iter().any(|b| b & 0x80 != 0);
        let all = image.iter().all(|b| b & 0x80 != 0);
        if all {
            Self::AllOn
        } else if !any {
            Self::AllOff
        } else {
            Self::Unknown
        }
    }
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

/// `(is a blend, byte width of its mask)` for the `VecBlendv*` family.
fn blend_width(op: IntrinsicOp) -> Option<usize> {
    use IntrinsicOp as O;
    Some(match op {
        O::VecBlendvF32x4
        | O::VecBlendvF64x2
        | O::VecBlendvI32x4
        | O::VecBlendvI8x16
        | O::VecBlendvI16x8 => 16,
        O::VecBlendvF32x8
        | O::VecBlendvF64x4
        | O::VecBlendvI32x8
        | O::VecBlendvI8x32
        | O::VecBlendvI16x16 => 32,
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

/// Pre-pass snapshot of the function's SSA definitions.  Kept immutable
/// during the rewrite so a fold cannot change the evidence of another fold.
/// Definitions are BORROWED, not cloned: `Facts::new` is O(instructions) with
/// zero heap payload copies, and the two-phase rewrite drops the borrow
/// before touching `IrFunction`.
struct Facts<'a> {
    func: &'a IrFunction,
    defs: FxHashMap<Value, &'a Instruction>,
}

impl<'a> Facts<'a> {
    fn new(func: &'a IrFunction) -> Self {
        let mut defs = FxHashMap::default();
        for block in &func.blocks {
            for inst in &block.instructions {
                if let Some(dest) = inst.dest() {
                    defs.insert(dest, inst);
                }
            }
        }
        Self { func, defs }
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
        if let Some(cached) = memo.get(&v) {
            return *cached;
        }
        if !visiting.insert(v) {
            return None; // provenance cycle: refuse
        }
        let result = match self.defs.get(&Value(v)) {
            Some(Instruction::Alloca { dest, .. }) => Some(dest.0),
            Some(Instruction::Copy {
                src: Operand::Value(next),
                ..
            }) => self.base_of_value(next.0, memo, visiting),
            Some(Instruction::GetElementPtr { base, .. }) => {
                self.base_of_value(base.0, memo, visiting)
            }
            _ => None,
        };
        visiting.remove(&v);
        memo.insert(v, result);
        result
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

    /// Prove that `base`'s byte window `[offset, offset + len)` is a
    /// compile-time constant and return that image.  This is the adaptation
    /// layer ISPC's LLVM-based analysis does not need.
    #[allow(clippy::too_many_arguments)]
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

        // Reachability table for the WHOLE function.  Unlike
        // `resolve_pointer`, this follows variable-offset GEPs and unbounded
        // chains, so no address mention can hide from the escape audit.
        let mut memo: FxHashMap<u32, Option<u32>> = FxHashMap::default();
        let mut cycle: FxHashSet<u32> = FxHashSet::default();

        // (byte offset, bytes) writes, in program order.  Only the requested
        // window is materialized, but EVERY write to the object is audited.
        let mut writes: Vec<(i64, Vec<u8>)> = Vec::new();
        let mut rejected = false;

        'scan: for (block_id, block) in self.func.blocks.iter().enumerate() {
            for (index, inst) in block.instructions.iter().enumerate() {
                if *budget == 0 {
                    rejected = true;
                    break 'scan;
                }
                *budget -= 1;

                let mut touches = false;
                for id in inst.used_values() {
                    if self.base_of_value(Value(id).0, &mut memo, &mut cycle) == Some(base.0) {
                        touches = true;
                        break;
                    }
                }
                if !touches {
                    continue;
                }
                // Every mention of the address must be in the read's block
                // and at or before the read; a later one could be a write.
                if block_id != read_block || index > read_index {
                    rejected = true;
                    break 'scan;
                }

                match inst {
                    // Pure address derivations.  Variable-offset GEPs are
                    // rejected by the escape audit below (`base` may still be
                    // reachable, but the derived address is not provably
                    // inside the object), so we only allow constant ones.
                    Instruction::GetElementPtr {
                        offset: Operand::Const(_),
                        ..
                    }
                    | Instruction::Copy { .. } => {}
                    Instruction::GetElementPtr { .. } => rejected = true,
                    Instruction::Load {
                        ptr,
                        ty,
                        volatile: false,
                        ..
                    } => {
                        // A non-volatile scalar read cannot change the image.
                        // It is sound to allow because NO `IrConst` can encode
                        // a stack address: a value loaded from a constant
                        // object cannot be turned back into a pointer that
                        // aliases it.  Pin the pointer to THIS object and the
                        // read window inside it, or refuse.  This is the
                        // i686 `Load I128` aggregate-copy idiom.
                        let mut zero = 0i64;
                        let Some(p) = self.resolve_pointer(*ptr, &mut zero, budget) else {
                            rejected = true;
                            break 'scan;
                        };
                        if p != base {
                            rejected = true;
                            break 'scan;
                        }
                        let Ok(s) = usize::try_from(zero) else {
                            rejected = true;
                            break 'scan;
                        };
                        let Some(end) = s.checked_add(ty.size()) else {
                            rejected = true;
                            break 'scan;
                        };
                        if end > size {
                            rejected = true;
                            break 'scan;
                        }
                    }
                    // A volatile scalar load is an observable read: refuse.
                    Instruction::Load { .. } => rejected = true,
                    Instruction::Store {
                        val,
                        ptr,
                        ty,
                        volatile,
                        ..
                    } => {
                        if *volatile {
                            rejected = true;
                            break 'scan;
                        }
                        let mut zero = 0i64;
                        let Some(p) = self.resolve_pointer(*ptr, &mut zero, budget) else {
                            rejected = true;
                            break 'scan;
                        };
                        if p != base {
                            rejected = true;
                            break 'scan;
                        }
                        let bytes = match val {
                            Operand::Const(c) => const_store_bytes(*c, ty.size()),
                            // i686's aggregate-copy lowering: the store's
                            // value is a scalar Load from ANOTHER proven
                            // constant object, not an `IrConst`.
                            Operand::Value(v) => self.known_scalar_bytes(
                                *v,
                                ty.size(),
                                read_block,
                                read_index,
                                budget,
                                visiting,
                            ),
                        };
                        let Some(bytes) = bytes else {
                            rejected = true;
                            break 'scan;
                        };
                        writes.push((zero, bytes));
                    }
                    Instruction::Memcpy {
                        dest,
                        src,
                        size: copy_len,
                    } => {
                        // `vector_temp_promotion` and the aggregate-copy
                        // lowering insert a whole-object `Memcpy` between a
                        // vector temporary and the movemask's own stack slot
                        // (measured for `_mm_movemask_ps` / `_mm_movemask_pd`).
                        // Treat it as a PROVEN write or a pure read, never as
                        // an unknown memory effect, and prove both ends.
                        if *copy_len == 0 || *copy_len > 32 {
                            rejected = true;
                            break 'scan;
                        }
                        let mut d_off = 0i64;
                        let Some(d_base) = self.resolve_pointer(*dest, &mut d_off, budget) else {
                            rejected = true;
                            break 'scan;
                        };
                        let mut s_off = 0i64;
                        let Some(s_base) = self.resolve_pointer(*src, &mut s_off, budget) else {
                            rejected = true;
                            break 'scan;
                        };
                        if d_base == base {
                            // A write to this object.  A self/overlapping
                            // copy is refused: memcpy byte ORDER over an
                            // overlapping range is not a value proof.
                            if s_base == base {
                                rejected = true;
                                break 'scan;
                            }
                            let Some(image) = self.object_image(
                                s_base, s_off, *copy_len, read_block, read_index, budget, visiting,
                            ) else {
                                rejected = true;
                                break 'scan;
                            };
                            writes.push((d_off, image));
                        } else if s_base == base {
                            // A pure read: it cannot change the image.  Pin
                            // the read window inside the object, so an
                            // out-of-bounds copy cannot become "harmless" by
                            // silence.
                            // NB: no early `return` inside this scan — the
                            // `visiting.pop()` at the end must always run, or
                            // a rejected proof poisons every later one.
                            let Ok(s) = usize::try_from(s_off) else {
                                rejected = true;
                                break 'scan;
                            };
                            let Some(end) = s.checked_add(*copy_len) else {
                                rejected = true;
                                break 'scan;
                            };
                            if end > size {
                                rejected = true;
                                break 'scan;
                            }
                        } else {
                            rejected = true;
                            break 'scan;
                        }
                    }
                    Instruction::Intrinsic {
                        op, args, dest_ptr, ..
                    } => {
                        if let Some(store_width) = whole_store_width(*op) {
                            // args = [value, ptr, byte_offset]; some
                            // producers omit the trailing zero offset.
                            if args.len() < 2 {
                                rejected = true;
                                break 'scan;
                            }
                            let byte_offset = match args.get(2) {
                                Some(off) => match Self::const_offset(off) {
                                    Some(o) => o,
                                    None => {
                                        rejected = true;
                                        break 'scan;
                                    }
                                },
                                None => 0,
                            };
                            let Operand::Value(pp) = args[1] else {
                                rejected = true;
                                break 'scan;
                            };
                            let mut zero = 0i64;
                            let Some(p) = self.resolve_pointer(pp, &mut zero, budget) else {
                                rejected = true;
                                break 'scan;
                            };
                            if p != base || (*dest_ptr).is_some_and(|d| d != pp) {
                                rejected = true;
                                break 'scan;
                            }
                            let Some(image) = self.known_vector(
                                args[0],
                                store_width,
                                read_block,
                                read_index,
                                budget,
                                visiting,
                            ) else {
                                rejected = true;
                                break 'scan;
                            };
                            writes.push((zero.saturating_add(byte_offset), image));
                        } else if byte_logic_shape(*op).is_some()
                            || dest_ptr_splat_shape(*op).is_some()
                        {
                            // Legacy SSE/AVX2 form: the intrinsic WRITES a
                            // computed constant vector into the object named
                            // by `dest_ptr`.  Both source images must be
                            // provable with the SHARED recursion state.
                            let Some(dp) = *dest_ptr else {
                                rejected = true;
                                break 'scan;
                            };
                            let mut d_off = 0i64;
                            let Some(d_base) = self.resolve_pointer(dp, &mut d_off, budget) else {
                                rejected = true;
                                break 'scan;
                            };
                            if d_base != base {
                                // This intrinsic writes a DIFFERENT object
                                // and merely READS this one through its
                                // pointer arguments.  A read cannot change
                                // this object's image; pin every pointer
                                // argument to a resolved stack object so no
                                // derived-address access escapes the audit.
                                for arg in args {
                                    if let Operand::Value(p) = arg {
                                        let mut zero = 0i64;
                                        if self.resolve_pointer(*p, &mut zero, budget).is_none() {
                                            rejected = true;
                                            break 'scan;
                                        }
                                    }
                                }
                                continue;
                            }
                            let image = if let Some((vec_width, kind)) = byte_logic_shape(*op) {
                                if args.len() != 2 || vec_width == 0 || vec_width > 32 {
                                    rejected = true;
                                    break 'scan;
                                }
                                let Some(a) = self.operand_image(
                                    args[0], vec_width, read_block, read_index, budget, visiting,
                                ) else {
                                    rejected = true;
                                    break 'scan;
                                };
                                let Some(b) = self.operand_image(
                                    args[1], vec_width, read_block, read_index, budget, visiting,
                                ) else {
                                    rejected = true;
                                    break 'scan;
                                };
                                a.into_iter()
                                    .zip(b)
                                    .map(|(x, y)| kind.apply(x, y))
                                    .collect::<Vec<u8>>()
                            } else {
                                // Exact-width splat; refuse widened or
                                // non-constant operands (truncation of an
                                // `I64` into a dword lane is not provable).
                                let Some((vec_width, lane_width)) = dest_ptr_splat_shape(*op)
                                else {
                                    rejected = true;
                                    break 'scan;
                                };
                                if args.len() != 1 || vec_width > 32 {
                                    rejected = true;
                                    break 'scan;
                                }
                                let Operand::Const(c) = args[0] else {
                                    rejected = true;
                                    break 'scan;
                                };
                                let lane = c.to_le_bytes();
                                if lane.len() != lane_width {
                                    rejected = true;
                                    break 'scan;
                                }
                                let mut img = Vec::with_capacity(vec_width);
                                while img.len() < vec_width {
                                    img.extend_from_slice(&lane);
                                }
                                img.truncate(vec_width);
                                img
                            };
                            writes.push((d_off, image));
                        } else if whole_vector_load(*op) {
                            // A whole-register load is a pure read.  Pin its
                            // pointer to THIS object and its offset to a
                            // constant, or reject (an unknown memory effect
                            // through a derived address is not a read we can
                            // account for).
                            let (Some(Operand::Value(p)), Some(_)) =
                                (args.first(), args.get(1).and_then(Self::const_offset))
                            else {
                                rejected = true;
                                break 'scan;
                            };
                            let mut zero = 0i64;
                            if self.resolve_pointer(*p, &mut zero, budget) != Some(base) {
                                rejected = true;
                                break 'scan;
                            }
                        } else if is_movemask(*op) {
                            let Some(Operand::Value(p)) = args.first() else {
                                rejected = true;
                                break 'scan;
                            };
                            let mut zero = 0i64;
                            if self.resolve_pointer(*p, &mut zero, budget) != Some(base) {
                                rejected = true;
                                break 'scan;
                            }
                        } else {
                            // Unknown memory effect through this address.
                            rejected = true;
                        }
                    }
                    // A scalar `Load`, phi, select, cast, call, inline asm,
                    // atomic, memcpy, return or arithmetic mention loses
                    // provenance, may read, or may write.  Fail closed.
                    _ => rejected = true,
                }
                if rejected {
                    break 'scan;
                }
            }
            // Terminators are audited too: returning or branching on the
            // address lets it escape.
            for id in block.terminator.used_values() {
                if self.base_of_value(Value(id).0, &mut memo, &mut cycle) == Some(base.0) {
                    rejected = true;
                    break 'scan;
                }
            }
        }

        visiting.pop();
        if rejected {
            return None;
        }

        // Exact tiling of the requested window: no gap, no overlap.  Bytes
        // outside the window are ignored, but a write that overlaps the
        // window twice is refused because store ORDER is not proven.
        let mut image = vec![None::<u8>; len];
        for (write_offset, bytes) in writes {
            let Ok(w) = usize::try_from(write_offset) else {
                return None;
            };
            // A write that extends past the alloca's declared size is a
            // buffer overrun, not a value proof.  Refuse it instead of
            // silently truncating the trailing bytes away (this is what
            // catches a 32-byte `SetEpi8_256` splat into a 16-byte object,
            // and any mismatched `VecStore*` width).
            let Some(write_end) = w.checked_add(bytes.len()) else {
                return None;
            };
            if write_end > size {
                return None;
            }
            for (i, byte) in bytes.into_iter().enumerate() {
                let Some(at) = w.checked_add(i) else {
                    return None;
                };
                if at < start {
                    continue;
                }
                let slot = at - start;
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

    if let Some(width) = blend_width(*op) {
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
        return match MaskStatus::from_bytes(&image) {
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
            matches!(inst, Instruction::Intrinsic { op, dest_ptr: None, .. }
                if is_movemask(*op) || blend_width(*op).is_some())
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
        total += remove_instructions(func, &dead);
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

    let mut veto: FxHashSet<u32> = FxHashSet::default();
    // (block, index, base) for instructions that are legal to delete.  A
    // candidate is only emitted if its base survives the veto.
    let mut candidates: Vec<(usize, usize, u32)> = Vec::new();
    let mut work = MAX_WORK;

    for (block_id, block) in facts.func.blocks.iter().enumerate() {
        for (index, inst) in block.instructions.iter().enumerate() {
            if work == 0 {
                // Budget exhausted: refuse every candidate this round.
                veto.extend(bases.iter().copied());
                break;
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
                ..
            } = inst
            {
                if byte_logic_shape(*op).is_some() || dest_ptr_splat_shape(*op).is_some() {
                    let d = facts
                        .base_of_value(dp.0, &mut memo, &mut cycle)
                        .filter(|b| bases.contains(b));
                    if let Some(db) = d {
                        candidates.push((block_id, index, db));
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
