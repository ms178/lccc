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
                        let Operand::Const(c) = val else {
                            rejected = true;
                            break 'scan;
                        };
                        let mut zero = 0i64;
                        let Some(p) = self.resolve_pointer(*ptr, &mut zero, budget) else {
                            rejected = true;
                            break 'scan;
                        };
                        if p != base {
                            rejected = true;
                            break 'scan;
                        }
                        let bytes = c.to_le_bytes();
                        // `IrConst::to_le_bytes` width must equal the store's
                        // declared type width; otherwise the byte image is
                        // not the bytes the backend would emit.
                        if bytes.len() != ty.size() {
                            rejected = true;
                            break 'scan;
                        }
                        writes.push((zero, bytes));
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
        if let Operand::Value(v) = operand {
            let mut zero = 0i64;
            if let Some(base) = self.resolve_pointer(v, &mut zero, &mut budget) {
                return self.object_image(
                    base,
                    zero,
                    width,
                    read_block,
                    read_index,
                    &mut budget,
                    &mut visiting,
                );
            }
        }
        self.known_vector(
            operand,
            width,
            read_block,
            read_index,
            &mut budget,
            &mut visiting,
        )
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
