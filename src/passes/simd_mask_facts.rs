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

/// Largest vector byte image the proof engine accepts: one 512-bit register.
/// Every LCCC vector width this pass reasons about is 16, 32 or 64 bytes, so
/// a FIXED capacity is the exact bound — not a heuristic.  Keeping it in the
/// type (instead of a `Vec<u8>`) is what makes the whole image engine
/// allocation-free and enables the 512-bit consumers.
const MAX_IMAGE: usize = 64;

/// An exact, fixed-capacity byte image.
///
/// Design decision (red-team round 8): the merged engine represented every
/// proven byte image as `Vec<u8>`.  `combine_byte_logic`, `compare_image`,
/// `known_vector`, `object_image`, `tile_image` and `const_store_bytes`
/// therefore allocated between one and four heap buffers for EVERY candidate
/// instruction, even when the proof immediately refused.  A compiler pass
/// that runs on every function must not allocate on its reject path.
///
/// `ByteImage` is `Copy`, 72 bytes, register-friendly, and derives `PartialEq`
/// over a CANONICAL tail: every constructor zero-fills bytes `[len..]`, so two
/// images are equal iff their visible slices are equal.  Capacity is 64 bytes,
/// which raises the provable width from 32 to 64 and lets the same lattice
/// serve SSE, AVX2 and AVX-512 without a second code path.  No dynamic
/// allocation, no `unsafe`, no pointer arithmetic, no lifetime to audit.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct ByteImage {
    bytes: [u8; MAX_IMAGE],
    len: usize,
}

impl ByteImage {
    #[inline]
    fn empty() -> Self {
        Self {
            bytes: [0u8; MAX_IMAGE],
            len: 0,
        }
    }

    #[inline]
    fn from_slice(src: &[u8]) -> Option<Self> {
        if src.len() > MAX_IMAGE {
            return None;
        }
        let mut out = Self::empty();
        out.bytes[..src.len()].copy_from_slice(src);
        out.len = src.len();
        Some(out)
    }

    #[inline]
    fn filled(width: usize, byte: u8) -> Option<Self> {
        if width > MAX_IMAGE {
            return None;
        }
        let mut out = Self {
            bytes: [byte; MAX_IMAGE],
            len: width,
        };
        // Canonical tail: `PartialEq` must compare only `[0..len)`.
        out.bytes[width..].fill(0);
        Some(out)
    }

    #[inline]
    fn zeros(width: usize) -> Option<Self> {
        Self::filled(width, 0)
    }

    #[inline]
    fn ones(width: usize) -> Option<Self> {
        Self::filled(width, 0xFF)
    }

    #[inline]
    fn as_slice(&self) -> &[u8] {
        &self.bytes[..self.len]
    }

    #[inline]
    fn len(&self) -> usize {
        self.len
    }

    /// Element-wise combination of two exact-width images.  Both must have
    /// the same length; no silent zip truncation (a truncated zip was the
    /// classic way this engine could prove a too-short image).
    #[inline]
    fn zip_map(&self, other: &Self, f: impl Fn(u8, u8) -> u8) -> Option<Self> {
        if self.len != other.len || self.len > MAX_IMAGE {
            return None;
        }
        let mut out = *self;
        for i in 0..self.len {
            out.bytes[i] = f(self.bytes[i], other.bytes[i]);
        }
        Some(out)
    }

    /// Repeat an exact-width lane into a vector of `width` bytes.  A lane
    /// that does not divide the width is refused rather than truncated: a
    /// partial last lane has no defined value in any backend's splat.
    #[inline]
    fn splat(lane: &[u8], width: usize) -> Option<Self> {
        let lane_len = lane.len();
        if width > MAX_IMAGE || lane_len == 0 || width % lane_len != 0 {
            return None;
        }
        let mut out = Self::empty();
        let mut at = 0usize;
        while at < width {
            out.bytes[at..at + lane_len].copy_from_slice(lane);
            at += lane_len;
        }
        out.len = width;
        Some(out)
    }
}

impl core::ops::Deref for ByteImage {
    type Target = [u8];
    #[inline]
    fn deref(&self) -> &[u8] {
        self.as_slice()
    }
}

/// Exact, width-safe scalar decode of an integer `IrConst`.
///
/// **Soundness fix (red-team round 8).**  `IrConst::to_i64()` maps
/// `I128(v) => Some(v as i64)` — it TRUNCATES to the low 64 bits.  The merged
/// engine used it for mask values, compare predicates, GEP offsets and
/// `VecStore` byte offsets.  A constant such as `I128(1 << 64)` therefore
/// decoded as `0`, which turns an out-of-range mask into `AllOff` and an
/// out-of-range byte offset into an in-bounds one.  Both are miscompiles, not
/// merely missed folds.  Refuse any `I128` that does not fit exactly.
#[inline]
fn exact_i64(c: IrConst) -> Option<i64> {
    use IrConst as C;
    Some(match c {
        C::I8(v) => v as i64,
        C::I16(v) => v as i64,
        C::I32(v) => v as i64,
        C::I64(v) => v,
        C::I128(v) => i64::try_from(v).ok()?,
        C::Zero => 0,
        // Floats, decimal bit containers and long double are not masks,
        // predicates or byte offsets: refusing is the only exact verdict.
        C::F32(_) | C::F64(_) | C::D32(_) | C::D64(_) | C::LongDouble(..) => return None,
    })
}

/// Little-endian bytes of an `IrConst`, written into a fixed stack buffer.
/// The common integer/FP spellings are entirely allocation-free; the exotic
/// decimal/long-double containers fall back to `to_le_bytes()` and are then
/// refused by every consumer that needs an exact width.
#[inline]
fn ir_const_bytes(c: IrConst) -> Option<ByteImage> {
    use IrConst as C;
    let mut buf = [0u8; MAX_IMAGE];
    let n = match c {
        C::I8(v) => {
            buf[0] = v as u8;
            1
        }
        C::I16(v) => {
            buf[..2].copy_from_slice(&v.to_le_bytes());
            2
        }
        C::I32(v) => {
            buf[..4].copy_from_slice(&v.to_le_bytes());
            4
        }
        C::I64(v) => {
            buf[..8].copy_from_slice(&v.to_le_bytes());
            8
        }
        C::I128(v) => {
            buf[..16].copy_from_slice(&v.to_le_bytes());
            16
        }
        C::F32(v) => {
            buf[..4].copy_from_slice(&v.to_bits().to_le_bytes());
            4
        }
        C::F64(v) => {
            buf[..8].copy_from_slice(&v.to_bits().to_le_bytes());
            8
        }
        C::D32(v) => {
            buf[..4].copy_from_slice(&v.to_le_bytes());
            4
        }
        C::D64(v) => {
            buf[..8].copy_from_slice(&v.to_le_bytes());
            8
        }
        C::LongDouble(_, bytes) => {
            buf[..16].copy_from_slice(&bytes);
            16
        }
        C::Zero => 1, // `to_le_bytes()` yields exactly one zero byte
    };
    ByteImage::from_slice(&buf[..n])
}

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
        // The legacy `Setzero*` family is available in BOTH the write-through
        // (`dest_ptr`) and the SSA (`dest`) form: `result_dest(dest,
        // dest_ptr)` in the x86 emitter.  An SSA `Setzero*` is exactly the
        // same `ConstantAggregateZero` ISPC folds, so include it here too.
        // Width is checked against the consumer, as for `VecZero*`.
        O::Setzero128 | O::SetzeroPs128 => 16,
        O::Setzero256 => 32,
        O::Setzero512 => 64,
        _ => return None,
    })
}

/// Byte width written by a legacy write-through zero producer.  Kept
/// separate from `zero_vector_width` because `object_image` must classify
/// these as WRITES to an object, not as pure SSA values.  `pxor`/`xorps` +
/// store (16 B) and `vpxor` + store (32 B) are exact on every backend that
/// implements the family; an unrecognized width refuses.
fn dest_ptr_zero_width(op: IntrinsicOp) -> Option<usize> {
    use IntrinsicOp as O;
    Some(match op {
        O::Setzero128 | O::SetzeroPs128 => 16,
        O::Setzero256 => 32,
        O::Setzero512 => 64,
        _ => return None,
    })
}

/// `(vector byte width, lane byte width)` for the SSA packed-integer
/// `VecCmp*` family.  ISPC's `GetMaskFromValue` reaches these through LLVM's
/// constant folding of a compare; LCCC has no constant vector node, so the
/// mask image must be derived here.  Only the sign-INDEPENDENT predicates
/// 0 (`eq`) and 4 (`ne`) are accepted: signed/unsigned `lt`/`le` need the
/// lane's signedness and are deliberately left alone.  `args = [a, b, imm]`
/// (pinned by `vectorize.rs:7653`), with `dest_ptr: None`.
fn const_compare_shape(op: IntrinsicOp) -> Option<(usize, usize)> {
    use IntrinsicOp as O;
    Some(match op {
        O::VecCmpI8x16 => (16, 1),
        O::VecCmpI8x32 => (32, 1),
        O::VecCmpI16x8 => (16, 2),
        O::VecCmpI16x16 => (32, 2),
        O::VecCmpI32x4 => (16, 4),
        O::VecCmpI32x8 => (32, 4),
        O::VecCmpI64x2 => (16, 8),
        O::VecCmpI64x4 => (32, 8),
        _ => return None,
    })
}

/// Classification of a scalar `__mmaskN` constant.  This is a SECOND lattice,
/// distinct from `MaskStatus`'s byte-image lattice: AVX-512 masked memory and
/// mask-compare intrinsics carry their mask in a GPR (`k` register), not as a
/// 16/32-byte vector of `0x00`/`0xFF` lanes.  ISPC's `IntrinsicsOpt` uses
/// `GetMaskStatusFromValue` on LLVM constant vectors; LCCC needs this extra
/// decision procedure for the scalar form.  `Unknown` is fail-closed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ScalarMask {
    AllOff,
    AllOn,
    Mixed(u64),
    Unknown,
}

/// Is this masked-memory op one of the shapes a consumer actually rewrites?
/// Kept as the SAME cheap predicate used by `has_candidate`, so a function
/// containing an unsupported masked op never pays for building `Facts`.
/// The 512-bit forms ARE in scope: `MAX_IMAGE = 64` proves a whole ZMM
/// object, `Setzero512` is the exact 64-byte zero producer (a 32-byte
/// `Setzero256` would leave 32 stale bytes), `Memcpy` copies the 64-byte
/// merge source, and `Loadu512` is the exact all-on substitution.  Refusing
/// a 512-bit form would leave a `kmovq` and an EVEX masked load on the hot
/// path for a mask the compiler already knows is constant.
fn masked_fold_candidate(op: IntrinsicOp) -> bool {
    masked_form(op).is_some()
}

/// Kind of masked intrinsic.  `Merge(usize)` names the argument holding the
/// merge (`old`) vector; `Store(usize)` names the argument holding the value
/// to store.  All-off for a merge form is a plain copy of the merge source,
/// all-off for a zeroing form is an exact-width zero producer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MaskFamily {
    ZeroLoad,
    MergeLoad(usize),
    Store(usize),
    ZeroArith,
    MergeArith(usize),
    /// Only the all-off case is provable: the mask lane count of the
    /// insert/extract forms is not the result lane count, so classifying an
    /// all-on mask here would require a separate per-op hardware proof.
    ZeroOnly,
}

/// `(mask argument index, exact arity, mask lane count, result byte width,
/// family)` for every masked intrinsic this pass rewrites.
///
/// Lane counts come from `include/lcccsimd.h`'s `__mmaskN` declarations and
/// are SAFETY-CRITICAL, not documentation: `scalar_mask` truncates the raw
/// mask to `lanes` bits, so an over-large lane count could turn a mask with
/// an active high lane into `AllOff` and erase a real memory write.  Two of
/// these declarations were WRONG in the merged tree (`vpmaddubsw_maskz512`
/// claimed `__mmask64` but produces 32 WORD lanes, and
/// `vextracti32x4_maskz512` claimed `__mmask16` but has an 8-bit k mask);
/// both are now corrected in `scripts/gen_lcccsimd.py` against GCC 16.2's
/// `avx512bwintrin.h`.  All-off is the only verdict that is lane-count
/// independent (zero is zero at every width); every all-on rewrite below
/// therefore uses an op whose mask width is pinned by a C-level three-way
/// differential test.
fn masked_form(op: IntrinsicOp) -> Option<(usize, usize, u32, usize, MaskFamily)> {
    use IntrinsicOp as O;
    use MaskFamily as F;
    Some(match op {
        // Byte-granular masked memory: one mask bit per byte.
        O::MaskzLoaduEpi8_128 => (0, 2, 16, 16, F::ZeroLoad),
        O::MaskzLoaduEpi8_256 => (0, 2, 32, 32, F::ZeroLoad),
        O::MaskzLoaduEpi8_512 => (0, 2, 64, 64, F::ZeroLoad),
        O::MaskLoaduEpi8_128 => (0, 3, 16, 16, F::MergeLoad(2)),
        O::MaskLoaduEpi8_256 => (0, 3, 32, 32, F::MergeLoad(2)),
        O::MaskLoaduEpi8_512 => (0, 3, 64, 64, F::MergeLoad(2)),
        O::MaskStoreuEpi8_128 => (0, 2, 16, 16, F::Store(1)),
        O::MaskStoreuEpi8_256 => (0, 2, 32, 32, F::Store(1)),
        O::MaskStoreuEpi8_512 => (0, 2, 64, 64, F::Store(1)),
        // Masked arithmetic with a verified unmasked counterpart.
        O::MaskzSet1Epi16_512 => (0, 2, 32, 64, F::ZeroArith),
        O::MaskzSet1Epi32_512 => (0, 2, 16, 64, F::ZeroArith),
        O::MaskzSet1Epi64x512 => (0, 2, 8, 64, F::ZeroArith),
        O::MaskzMaddubsEpi16_512 => (0, 3, 32, 64, F::ZeroArith),
        O::MaskzShuffleEpi8_128 => (0, 3, 16, 16, F::ZeroArith),
        O::MaskShuffleEpi8_128 => (0, 4, 16, 16, F::MergeArith(1)),
        // All-off only: writing the zero producer is exact at every mask
        // width; the corresponding unmasked insert/extract is not a simple
        // drop-the-mask substitution (the imm selects a different lane).
        O::MaskzInsertI64x2 => (0, 4, 8, 64, F::ZeroOnly),
        O::MaskzInsertI32x4 => (0, 4, 16, 64, F::ZeroOnly),
        O::MaskzExtractI32x4 => (0, 3, 8, 16, F::ZeroOnly),
        O::MaskzExtractI64x4 => (0, 3, 8, 32, F::ZeroOnly),
        O::MaskzExtractI64x2 => (0, 3, 8, 16, F::ZeroOnly),
        _ => return None,
    })
}

/// ISPC `ImproveMemoryOps::lCheckMask`: a constant mask whose set bits form a
/// TRUE PREFIX `0..n` can be replaced by a narrower unmasked operation,
/// because the inactive lanes are exactly the untouched tail of the object.
/// Only a prefix is usable here: a suffix or a sparse mask would need a
/// nonzero destination offset, which `fold` cannot express with one
/// instruction (a new GEP value cannot be minted from a pure rewrite).
///
/// `gap_budget` is ISPC `GatherCoalescePass`'s bounded-gap idea.  A budget of
/// ZERO means every byte in `[0, n)` is active, which is the ONLY budget for
/// which dropping the mask is sound: a nonzero budget would copy bytes the
/// hardware left untouched.  The parameter is retained (and exhaustively
/// tested) so the refusal is a measured, explicit decision rather than an
/// implicit assumption.
fn true_prefix_len(bits: u64, lanes: u32, gap_budget: u32) -> Option<usize> {
    if lanes == 0 || lanes > 64 || bits == 0 || gap_budget != 0 {
        return None;
    }
    let full = lane_mask(lanes);
    if bits & !full != 0 {
        return None;
    }
    let n = bits.trailing_ones();
    if n == 0 || n >= lanes || bits >> n != 0 || !bits.wrapping_add(1).is_power_of_two() {
        return None;
    }
    // A prefix that covers EVERY lane is AllOn; `scalar_mask` already
    // classified that, and the caller handles it with a full-width rewrite.
    usize::try_from(n).ok()
}

/// Exact-width zero producer for a write-through destination.  `Setzero512`
/// is required for the 64-byte forms: `Setzero256` writes only 32 bytes.
fn zero_producer_op(width: usize) -> Option<IntrinsicOp> {
    Some(match width {
        16 => IntrinsicOp::Setzero128,
        32 => IntrinsicOp::Setzero256,
        64 => IntrinsicOp::Setzero512,
        _ => return None,
    })
}

/// Which masked-memory op is an all-off-ERASABLE store.  Erasing a store needs
/// no width proof at all (masked-off lanes perform NO memory access), so all
/// three widths (16/32/64) are admitted; the 64-byte load siblings need the
/// full `MAX_IMAGE` proof and are handled by `masked_form` above, not here.
/// The returned value is the NUMBER OF BYTE LANES (16/32/64), which is also
/// the byte width for the `Epi8` family; `fold_removal` passes it to
/// `scalar_mask` as `lanes`.
fn masked_store_shape(op: IntrinsicOp) -> Option<u32> {
    use IntrinsicOp as O;
    Some(match op {
        O::MaskStoreuEpi8_128 => 16,
        O::MaskStoreuEpi8_256 => 32,
        O::MaskStoreuEpi8_512 => 64,
        _ => return None,
    })
}

/// `(vector byte width, lane byte width, lane count, unsigned)` for the
/// scalar-mask packed-integer compares.
///
/// The `unsigned` bit is the ONE fact that makes all eight AVX-512 `vpcmp`
/// predicates foldable: the opcode already encodes the lane signedness
/// (`vpcmpb`/`vpcmpw`/`vpcmpd`/`vpcmpq` are signed, `vpcmpu*` unsigned), and
/// the byte image supplies the exact lane value.  Nothing is inferred from the
/// C carrier or from a host CPU.  The predicate numbering itself is pinned by
/// the 485-case three-way GCC 16.2 / Clang 23.1.5 / LCCC differential in
/// `scripts/check_maskfacts_vl_matrix.py`, not assumed from a manual.
///
/// Round 8: the 512-bit forms are NOW included. `MAX_IMAGE = 64` proves a
/// whole ZMM constant object, and `ScalarMask::Mixed(u64)` plus `lane_mask`
/// already cover exactly 64 lanes, so a constant `vpcmp*b512` can be folded
/// to its 64-bit `kmovq` result with no new opcode family. Excluding them
/// was correct only while the image engine was capped at 32 bytes.
fn mask_compare_shape(op: IntrinsicOp) -> Option<(usize, usize, u32, bool)> {
    use IntrinsicOp as O;
    Some(match op {
        O::CmpeqEpu8Mask128 | O::CmpEpu8Mask128 => (16, 1, 16, true),
        O::CmpEpi8Mask128 => (16, 1, 16, false),
        O::CmpeqEpu8Mask256 | O::CmpEpu8Mask256 => (32, 1, 32, true),
        O::CmpEpi8Mask256 => (32, 1, 32, false),
        O::CmpEpu16Mask128 => (16, 2, 8, true),
        O::CmpEpi16Mask128 => (16, 2, 8, false),
        O::CmpEpu16Mask256 => (32, 2, 16, true),
        O::CmpEpi16Mask256 => (32, 2, 16, false),
        O::CmpEpu32Mask128 => (16, 4, 4, true),
        O::CmpEpi32Mask128 => (16, 4, 4, false),
        O::CmpEpu32Mask256 => (32, 4, 8, true),
        O::CmpEpi32Mask256 => (32, 4, 8, false),
        O::CmpEpu64Mask128 => (16, 8, 2, true),
        O::CmpEpi64Mask128 => (16, 8, 2, false),
        O::CmpEpu64Mask256 => (32, 8, 4, true),
        O::CmpEpi64Mask256 => (32, 8, 4, false),
        // AVX-512: 64 bytes / 64 lanes is the exact u64 mask domain.
        O::CmpeqEpu8Mask512 | O::CmpEpu8Mask512 => (64, 1, 64, true),
        O::CmpEpi8Mask512 => (64, 1, 64, false),
        O::CmpeqEpu16Mask512 | O::CmpEpu16Mask512 => (64, 2, 32, true),
        O::CmpEpi16Mask512 => (64, 2, 32, false),
        O::CmpeqEpu32Mask512 | O::CmpEpu32Mask512 => (64, 4, 16, true),
        O::CmpEpi32Mask512 => (64, 4, 16, false),
        O::CmpeqEpu64Mask512 | O::CmpEpu64Mask512 => (64, 8, 8, true),
        O::CmpEpi64Mask512 => (64, 8, 8, false),
        _ => return None,
    })
}

/// Decode ONE little-endian lane (1/2/4/8 bytes) into `i128`, sign- or
/// zero-extended according to the opcode's signedness.  `i128` is wide enough
/// for every supported lane, so no comparison can wrap and no sign extension
/// can overflow.  A lane longer than 8 bytes is refused instead of guessed.
#[inline]
fn lane_as_i128(lane: &[u8], unsigned: bool) -> Option<i128> {
    if lane.is_empty() || lane.len() > 8 {
        return None;
    }
    let mut value = 0u128;
    for (i, byte) in lane.iter().enumerate() {
        value |= u128::from(*byte) << (8 * i);
    }
    if unsigned {
        return Some(value as i128);
    }
    let bits = lane.len() * 8;
    let sign = 1u128 << (bits - 1);
    Some(if value & sign != 0 {
        (value as i128) - (1i128 << bits)
    } else {
        value as i128
    })
}

/// The eight AVX-512 `vpcmp` predicates (immediate 0..7).  The numbering is
/// pinned by the generated 485-case three-way differential against GCC 16.2
/// and Clang 23.1.5, including a sign-bit boundary pair that makes signed and
/// unsigned LT/LE/GE/GT disagree.  Anything outside 0..7 refuses.
#[inline]
fn compare_predicate(pred: i64, unsigned: bool, a: &[u8], b: &[u8]) -> Option<bool> {
    let x = lane_as_i128(a, unsigned)?;
    let y = lane_as_i128(b, unsigned)?;
    Some(match pred {
        0 => x == y, // EQ
        1 => x < y,  // LT
        2 => x <= y, // LE
        3 => false,  // FALSE
        4 => x != y, // NEQ
        5 => x >= y, // NLT / GE
        6 => x > y,  // NLE / GT
        7 => true,   // TRUE
        _ => return None,
    })
}

/// Bit mask covering exactly `lanes` lanes.  `lanes` is never 0 and never
/// greater than 64 (checked by the shape tables above), so neither shift can
/// overflow.
fn lane_mask(lanes: u32) -> u64 {
    debug_assert!((1..=64).contains(&lanes));
    if lanes == 64 {
        u64::MAX
    } else {
        (1u64 << lanes) - 1
    }
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
/// order hazard (`pcmpeqb`, `pand`, `por`, `pxor`, `pavgb` are symmetric);
/// byte-wise semantics remove every lane-width hazard (a `pand` on I8x16,
/// I32x4 or F64x2 produces the identical 16-byte result).  `Avg` widens to
/// u16 BEFORE adding, so `0xFF + 0xFF + 1` cannot wrap.  Non-commutative
/// compares (`pcmpgtb`), WORD-wise averages (`pavgw`, which carries between
/// bytes) and saturated arithmetic are NOT here: their operand order, carry
/// or saturation rule is backend-specific and would need a separate proof.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ByteOp {
    And,
    Or,
    Xor,
    /// Byte equality -> 0xFF when equal, 0x00 otherwise (`pcmpeqb`).
    Eq,
    /// ISPC "widen-then-average": `(a + b + 1) / 2`, computed in a WIDER
    /// (u16) intermediate so no byte add can overflow.  This is exactly the
    /// x86 `pavgb`/`vpavgb` rounding (round half UP), byte-local and
    /// commutative, so it is lane-width independent on every backend that
    /// implements the family.
    Avg,
}

impl ByteOp {
    #[inline]
    fn apply(self, a: u8, b: u8) -> u8 {
        match self {
            Self::And => a & b,
            Self::Or => a | b,
            Self::Xor => a ^ b,
            Self::Eq => (a == b) as u8 * 0xFF,
            Self::Avg => ((a as u16 + b as u16 + 1) / 2) as u8,
        }
    }
}

/// Exact combination of two OPTIONAL byte images under a `ByteOp`.
///
/// ISPC's `InstructionSimplify` (`lSimplifyBoolVec`) and LLVM's
/// `InstSimplify` both exploit algebraic identities that need no operand
/// value at all.  LCCC's image lattice previously demanded BOTH operand
/// images; a `movemask(xor(v, v))`, `movemask(cmpeq(v, v))`,
/// `movemask(and(v, 0))` or `movemask(or(v, -1))` therefore kept a runtime
/// `pmovmskb` (measured: 4 `pmovmskb` and ~40 instructions in `ident.c`,
/// while GCC 16.2 and Clang 23.1.5 emit four immediates).  Three exact
/// rules close that gap without weakening any proof:
///
/// 1. both images known -> element-wise `ByteOp::apply` (the original rule);
/// 2. `same` (one PURE operand used twice) -> `x^x = 0`, `x==x = 0xFF`,
///    `x&x = x|x = x` (one image at most);
/// 3. one image known -> absorbing constants: `x&0 = 0`, `x|-1 = -1`.
///
/// `None` on either side is a first-class input, never a guess: a rule that
/// cannot decide returns `None` and the caller refuses the fold.  Every rule
/// is byte-local and lane-width independent, so it is exact on x86-64,
/// i686, AArch64 and RISC-V alike.
fn combine_byte_logic(
    kind: ByteOp,
    width: usize,
    a: Option<ByteImage>,
    b: Option<ByteImage>,
    same: bool,
) -> Option<ByteImage> {
    if width == 0 || width > MAX_IMAGE {
        return None;
    }
    if let (Some(x), Some(y)) = (a, b) {
        if x.len() != width || y.len() != width {
            return None;
        }
        return x.zip_map(&y, |p, q| kind.apply(p, q));
    }
    if same {
        return match kind {
            ByteOp::Xor => ByteImage::zeros(width),
            ByteOp::Eq => ByteImage::ones(width),
            // `x avg x == x` for every byte x: (2x+1)/2 == x with no
            // overflow in the widened intermediate.
            ByteOp::And | ByteOp::Or | ByteOp::Avg => a.filter(|v| v.len() == width),
        };
    }
    let known = a.or(b)?;
    if known.len() != width {
        return None;
    }
    match kind {
        ByteOp::And if known.iter().all(|v| *v == 0) => ByteImage::zeros(width),
        ByteOp::Or if known.iter().all(|v| *v == 0xFF) => ByteImage::ones(width),
        _ => None,
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
        O::Pavgb128 => (16, ByteOp::Avg),
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
        O::Pavgb512 => (64, ByteOp::Avg),
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
        // Round 9: the four 512-bit GPR broadcasts write a whole 64-byte
        // `Alloca` (lowering: `LcccSimdClass::Vec512` -> `dest_ptr`).  They
        // were the ONLY reason the 64 constant 512-bit `vpcmp` cases could
        // not be folded: `MAX_IMAGE = 64` is already wide enough, but the
        // write was unclassified so `object_image` refused.  The x86 emitter
        // broadcasts the LOW 8/16/32/64 bits of `%eax`/`%rax`, which is why
        // `object_image` still demands an EXACT-width constant: a widened
        // scalar is never silently truncated here.
        O::SetEpi8_512 => (64, 1),
        O::SetEpi16_512 => (64, 2),
        O::SetEpi32_512 => (64, 4),
        O::SetEpi64x512 => (64, 8),
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
fn const_store_bytes(c: IrConst, ty_size: usize) -> Option<ByteImage> {
    if ty_size == 0 || ty_size > MAX_IMAGE {
        return None;
    }
    // `IrConst::Zero` is the width-independent zero of every integer width.
    if matches!(c, IrConst::Zero) {
        return ByteImage::zeros(ty_size);
    }
    let bytes = ir_const_bytes(c)?;
    if bytes.len() == ty_size {
        return Some(bytes);
    }
    // The backend truncates a wider immediate ONLY to the destination's raw
    // field.  Keep the historical 8-byte ceiling: an `I128`-spelled store is
    // not a proven narrow write on every backend.
    if bytes.len() < ty_size || ty_size > 8 {
        return None;
    }
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
    ByteImage::from_slice(&bytes[..ty_size])
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
    (const_store_bytes(c, n) == ByteImage::from_slice(bytes)).then_some(c)
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
    /// A proven write of `bytes` at `offset`.  `ByteImage` is `Copy`, so this
    /// verdict is itself register-sized: no heap traffic on any audit path.
    Write(i64, ByteImage),
    /// An unprovable memory effect: the whole proof fails closed.
    Reject,
}

/// The immutable facts a mention classifier needs.  `Copy` so it can be
/// threaded through recursive proofs without borrowing the `Facts` snapshot.
#[derive(Clone, Copy)]
struct ImageWindow {
    base: Value,
    size: usize,
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
                || dest_ptr_zero_width(*op).is_some()
                || const_compare_shape(*op).is_some()
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

/// Intrinsics that produce an immutable SSA VALUE (an `dest` result with NO
/// write-through home) and have no observable memory effect of their own.
/// A whole-vector LOAD qualifies: once loaded, the SSA value cannot change
/// between two uses, which is exactly what the algebraic identities need.
/// `is_movemask` is deliberately excluded: it is a pure read of a vector
/// value, but its result is a scalar mask, never a vector operand of a
/// byte-logic/compare instruction.
fn pure_ssa_intrinsic(op: IntrinsicOp) -> bool {
    byte_logic_shape(op).is_some()
        || const_compare_shape(op).is_some()
        || zero_vector_width(op).is_some()
        || splat_shape(op).is_some()
        || whole_vector_load(op)
}

/// Positive proof that an instruction defines an immutable, non-address SSA
/// value.  Used ONLY by `operand_is_pure` for the repeated-operand identities;
/// it is intentionally narrower than `object_image`'s mention classification.
/// Pointer producers (`Alloca`, `GetElementPtr`, `GlobalAddr`, `LabelAddr`,
/// `Cast` to `Ptr`), write-through homes (`dest_ptr: Some(_)`), volatile and
/// atomic memory operations, calls, inline asm, phis and unknown intrinsics
/// all refuse.  A refused operand costs one fold; an accepted volatile
/// address would be a miscompile.
fn pure_ssa_definition(inst: &Instruction) -> bool {
    match inst {
        Instruction::Intrinsic {
            dest: Some(_),
            dest_ptr: None,
            op,
            ..
        } => pure_ssa_intrinsic(*op),
        Instruction::BinOp { .. }
        | Instruction::UnaryOp { .. }
        | Instruction::Cmp { .. }
        | Instruction::Select { .. } => true,
        // A cast to an integer/float/vector type is a value conversion; a
        // cast TO a pointer can smuggle an address back into the operand.
        Instruction::Cast {
            to_ty: IrType::Ptr, ..
        } => false,
        Instruction::Cast { .. } => true,
        Instruction::Load {
            volatile: false,
            ty,
            ..
        } => !matches!(ty, IrType::Ptr),
        _ => false,
    }
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
                    *offset = offset.checked_add(exact_i64(*c)?)?;
                    value = *base;
                }
                _ => return None,
            }
        }
        None
    }

    fn const_offset(op: &Operand) -> Option<i64> {
        match op {
            Operand::Const(c) => exact_i64(*c),
            Operand::Value(_) => None,
        }
    }

    /// Whether an operand is provably ONE immutable bit pattern.
    ///
    /// `x ^ x = 0`, `x == x = all ones`, `x & x = x | x = x` and the
    /// operand-independent `vpcmp` predicates 3/7 all require that dropping
    /// the two reads cannot discard a volatile load, a call or an externally
    /// mutable object.  An SSA value is bit-identical by construction; a
    /// POINTER operand is read BY THE INTRINSIC, so it is safe only when the
    /// address resolves to a plain, non-volatile private `Alloca`: two reads
    /// of the same non-volatile object with no intervening write yield the
    /// same bytes.  Three forms are positively proven:
    ///
    /// 1. an immediate;
    /// 2. a pointer resolved to a plain, non-volatile `Alloca` — the legacy
    ///    write-through stack-home form;
    /// 3. an SSA value whose definition is a known pure value producer,
    ///    reached through `Copy` links (`pure_ssa_definition`).
    ///
    /// Everything else refuses.  The historical `Some(_) => true` fallback
    /// accepted ANY non-`Copy` definition, including `Cast` to a pointer,
    /// `GlobalAddr`, a volatile `Load`, an atomic, or a call result.  An
    /// externally mutable address smuggled in as an `Operand::Value` could
    /// then make `x ^ x` read two different bit patterns; refusing is the
    /// only defensible rule.  A starved budget or a `Copy` cycle also refuses.
    fn operand_is_pure(&self, op: Operand, budget: &mut usize) -> bool {
        let Operand::Value(v) = op else {
            return true; // an immediate has one immutable value
        };
        let mut zero = 0i64;
        if let Some(base) = self.resolve_pointer(v, &mut zero, budget) {
            return matches!(
                self.defs.get(&base),
                Some(Instruction::Alloca {
                    volatile: false,
                    semantic_volatile: false,
                    ..
                })
            );
        }
        let mut cur = v;
        for _ in 0..MAX_DEPTH {
            if *budget == 0 {
                return false;
            }
            *budget -= 1;
            let Some(inst) = self.defs.get(&cur) else {
                return false; // argument / external value: no proof at all
            };
            match inst {
                Instruction::Copy {
                    src: Operand::Value(next),
                    ..
                } => cur = *next,
                _ => return pure_ssa_definition(inst),
            }
        }
        false
    }

    /// Prove the value of a scalar `__mmaskN` operand.  Only an immediate or
    /// a bounded `Copy` chain ending in an immediate is accepted: a runtime
    /// mask, a `Phi`, a `Cast`, a call result or a cycle is `Unknown`.
    ///
    /// Constants are interpreted as the low `lanes` bits of their two's
    /// complement value, so `__mmask16(-1)` is `AllOn` and `0xFFFF0000` on a
    /// 16-lane op is also `AllOn` (the upper bits the hardware ignores are
    /// masked off).  A budget and a depth cap bound the walk; both refuse.
    fn scalar_mask(&self, op: Operand, lanes: u32) -> ScalarMask {
        if !(1..=64).contains(&lanes) {
            return ScalarMask::Unknown;
        }
        let full = lane_mask(lanes);
        let mut budget = MAX_WORK;
        let mut cur = op;
        for _ in 0..MAX_DEPTH {
            if budget == 0 {
                return ScalarMask::Unknown;
            }
            budget -= 1;
            match cur {
                Operand::Const(c) => {
                    let Some(raw) = exact_i64(c) else {
                        return ScalarMask::Unknown; // FP/decimal constants are not masks
                    };
                    let k = (raw as u64) & full;
                    return if k == 0 {
                        ScalarMask::AllOff
                    } else if k == full {
                        ScalarMask::AllOn
                    } else {
                        ScalarMask::Mixed(k)
                    };
                }
                Operand::Value(v) => match self.defs.get(&v) {
                    Some(Instruction::Copy { src, .. }) => cur = *src,
                    _ => return ScalarMask::Unknown,
                },
            }
        }
        ScalarMask::Unknown
    }

    /// Byte image of a `ByteOp` expression, using the shared recursion state
    /// and the ONE `combine_byte_logic` decision procedure.  Both the legacy
    /// write-through forms (`Pand128`, ...) and the SSA forms
    /// (`VecAndI32x4`, ...) converge here, so there is a single place where
    /// soundness is argued and a single place where a new identity is added.
    #[allow(clippy::too_many_arguments)]
    fn byte_logic_image(
        &self,
        kind: ByteOp,
        width: usize,
        args: &[Operand],
        read_block: usize,
        read_index: usize,
        budget: &mut usize,
        visiting: &mut Vec<Value>,
    ) -> Option<ByteImage> {
        if args.len() != 2 || width == 0 || width > MAX_IMAGE {
            return None;
        }
        let same = args[0] == args[1] && self.operand_is_pure(args[0], budget);
        if same {
            // `x^x` and `x==x` need NO operand proof at all; `x&x`/`x|x`
            // need at most one.  Never pay for a second, identical walk.
            let a = match kind {
                ByteOp::Xor | ByteOp::Eq => None,
                ByteOp::And | ByteOp::Or | ByteOp::Avg => {
                    self.operand_image(args[0], width, read_block, read_index, budget, visiting)
                }
            };
            return combine_byte_logic(kind, width, a, None, true);
        }
        let a = self.operand_image(args[0], width, read_block, read_index, budget, visiting);
        let b = self.operand_image(args[1], width, read_block, read_index, budget, visiting);
        combine_byte_logic(kind, width, a, b, false)
    }

    /// Byte image of a constant packed-integer `VecCmp*` (predicate `eq` or
    /// `ne`).  Each lane contributes `0xFF` bytes for a true comparison and
    /// `0x00` for a false one — the exact `-1`/`0` mask the hardware emits,
    /// and the exact input `MaskStatus::from_lanes` expects.  This is
    /// target-neutral: integer equality is bit equality on every backend.
    #[allow(clippy::too_many_arguments)]
    fn compare_image(
        &self,
        op: IntrinsicOp,
        args: &[Operand],
        width: usize,
        read_block: usize,
        read_index: usize,
        budget: &mut usize,
        visiting: &mut Vec<Value>,
    ) -> Option<ByteImage> {
        let (vec_width, lane_bytes) = const_compare_shape(op)?;
        if vec_width != width || args.len() != 3 || lane_bytes == 0 || width % lane_bytes != 0 {
            return None;
        }
        let Operand::Const(pred) = args[2] else {
            return None;
        };
        let invert = match exact_i64(pred)? {
            0 => false, // eq
            4 => true,  // ne = logical negation of eq for integers
            _ => return None,
        };
        let fill = |eq: bool| -> u8 { if eq ^ invert { 0xFF } else { 0x00 } };
        if args[0] == args[1] && self.operand_is_pure(args[0], budget) {
            // `x == x` is all ones and `x != x` is all zeros for integers;
            // no operand image is needed, and no NaN can intrude.
            return ByteImage::filled(width, fill(true));
        }
        let a = self.operand_image(args[0], width, read_block, read_index, budget, visiting)?;
        let b = self.operand_image(args[1], width, read_block, read_index, budget, visiting)?;
        // `ByteImage::as_slice()` is `[0..len)` and every constructor caps
        // len at MAX_IMAGE, so `a.len() == width` already implies
        // `width <= MAX_IMAGE`. The old `width / lane_bytes > MAX_IMAGE`
        // guard was vacuous. State the real bound explicitly and build the
        // result through checked arithmetic + `get_mut` + `from_slice`, so
        // NO path can panic on a malformed image (a bad proof must refuse,
        // never abort the compiler) and no caller slices a private field.
        if a.len() != width || b.len() != width || width > MAX_IMAGE || lane_bytes == 0 {
            return None;
        }
        let mut buf = [0u8; MAX_IMAGE];
        let mut at = 0usize;
        for (x, y) in a.chunks_exact(lane_bytes).zip(b.chunks_exact(lane_bytes)) {
            let end = at.checked_add(lane_bytes)?;
            buf.get_mut(at..end)?.fill(fill(x == y));
            at = end;
        }
        if at != width {
            return None;
        }
        ByteImage::from_slice(&buf[..width])
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
    ) -> Option<ByteImage> {
        if width == 0 || width > MAX_IMAGE {
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
    ) -> Option<ByteImage> {
        if *budget == 0 || visiting.len() >= MAX_DEPTH || width == 0 || width > MAX_IMAGE {
            return None;
        }
        *budget -= 1;
        match operand {
            Operand::Const(c) => ir_const_bytes(c).filter(|img| img.len() == width),
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
                            let mut agreed: Option<ByteImage> = None;
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
                                return ByteImage::zeros(width);
                            }
                            if let Some((vec_width, lane_width)) = splat_shape(*op) {
                                if vec_width != width || args.is_empty() {
                                    return None;
                                }
                                let Operand::Const(c) = args[0] else {
                                    return None;
                                };
                                let lane = ir_const_bytes(c)?;
                                // Exact-width ONLY.  Silently zero-extending
                                // an I8 -1 into an I32 lane would produce a
                                // different sign bit than sign extension;
                                // there is no IR evidence of which the
                                // producer intended, so refuse.
                                if lane.len() != lane_width {
                                    return None;
                                }
                                return ByteImage::splat(lane.as_slice(), width);
                            }
                            if let Some((vec_width, kind)) = byte_logic_shape(*op) {
                                // Vector constant propagation (ISPC
                                // `InstructionSimplify`'s constant-mask
                                // derivation).  Both operands may be SSA
                                // vector values OR pointers to proven
                                // constant objects; `byte_logic_image`
                                // additionally applies the exact algebraic
                                // identities when an operand is runtime
                                // valued.  ONE shared recursion state.
                                if vec_width != width {
                                    return None;
                                }
                                return self.byte_logic_image(
                                    kind, width, args, read_block, read_index, budget, visiting,
                                );
                            }
                            if const_compare_shape(*op).is_some() {
                                return self.compare_image(
                                    *op, args, width, read_block, read_index, budget, visiting,
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
    /// the object in program order, and the per-mention classification lives
    /// in `mention_of` / `memcpy_mention` / `intrinsic_mention`, which return
    /// an `ObjectMention` value instead of returning early.  That is
    /// deliberately structural: when they refuse, control still reaches
    /// `visiting.pop()` below, so a rejected proof can never poison a later
    /// one.  A `Write` is order-sensitive (same block, at or before the read);
    /// a proven `NoWrite` may appear anywhere because it cannot change the
    /// image.
    fn object_image(
        &self,
        base: Value,
        offset: i64,
        len: usize,
        read_block: usize,
        read_index: usize,
        budget: &mut usize,
        visiting: &mut Vec<Value>,
    ) -> Option<ByteImage> {
        if offset < 0 || len == 0 || len > MAX_IMAGE || visiting.contains(&base) {
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
        // `read_block`/`read_index` anchor THIS object's own write scan;
        // nested proofs are anchored separately at the instruction being
        // audited (see `mention_of`'s `at_*` parameters).  Keeping them out
        // of the `Copy` window shrinks it from 32 to 16 bytes and makes it
        // impossible to accidentally re-anchor a nested proof at the outer
        // read.
        let w = ImageWindow { base, size };

        // The mention index was built once in `Facts::new` from the same
        // definition graph.  Auditing only THIS object's mentions removes the
        // quadratic rescan: k mentions instead of every instruction of the
        // function for every read.  No mention can hide, because `root_of`
        // follows unbounded `Copy`/`GEP` chains and refuses cycles.
        if self.escaping.contains(&base.0) {
            visiting.pop();
            return None;
        }

        // The tile is a FIXED stack buffer.  The merged engine collected every
        // write into a `Vec<(i64, Vec<u8>)>` and then tiled it in a second
        // pass; that allocated twice per proof and, worse, refused any two
        // writes to the same byte because "order is not proven".  Order IS
        // proven here: `mentions` is built in program order, and only writes
        // in `read_block` at indices `<= read_index` are accepted, so the
        // sequence below is strictly ascending in instruction index.  Applying
        // them immediately gives exact last-write-wins semantics with zero
        // heap traffic and one pass.
        let mut image = [0u8; MAX_IMAGE];
        let mut filled = [false; MAX_IMAGE];
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
                match self.mention_of(inst, block_id, index, w, budget, visiting) {
                    ObjectMention::NoWrite => {}
                    ObjectMention::Write(off, bytes) => {
                        // A later write, or a write in another block whose
                        // execution order is not proven, could change the
                        // image at the read point.
                        if block_id != read_block || index > read_index {
                            rejected = true;
                            break 'scan;
                        }
                        if !Self::apply_image_write(
                            &w,
                            start,
                            len,
                            off,
                            bytes,
                            &mut image,
                            &mut filled,
                        ) {
                            rejected = true;
                            break 'scan;
                        }
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
        // Exact tiling: every byte of the window must be covered at least
        // once.  Overlapping writes are legal and the LAST one wins, because
        // the caller established strict program order above.
        if filled[..len].iter().any(|f| !*f) {
            return None;
        }
        ByteImage::from_slice(&image[..len])
    }

    /// Apply ONE proven write to the requested window.  Returns `false` for
    /// any shape that is not an exact, in-bounds write, so a buffer overrun
    /// can never be silently truncated into a value proof (this is what
    /// refuses a 32-byte `SetEpi8_256` splat into a 16-byte object and any
    /// mismatched `VecStore*` width).
    fn apply_image_write(
        w: &ImageWindow,
        start: usize,
        len: usize,
        write_offset: i64,
        bytes: ByteImage,
        image: &mut [u8; MAX_IMAGE],
        filled: &mut [bool; MAX_IMAGE],
    ) -> bool {
        let Ok(at) = usize::try_from(write_offset) else {
            return false;
        };
        let Some(write_end) = at.checked_add(bytes.len()) else {
            return false;
        };
        if write_end > w.size {
            return false;
        }
        for (i, byte) in bytes.iter().copied().enumerate() {
            let Some(pos) = at.checked_add(i) else {
                return false;
            };
            if pos < start {
                continue;
            }
            let slot = pos - start;
            if slot >= len {
                continue;
            }
            // Program order proven by the caller: a later write simply
            // overwrites an earlier one.  No `Vec` involved, no ambiguity.
            image[slot] = byte;
            filled[slot] = true;
        }
        true
    }

    /// Classify ONE instruction that mentions the object under audit.  A
    /// value-returning verdict (never an early `return` from `object_image`)
    /// is what makes `visiting.pop()` unconditional.
    fn mention_of(
        &self,
        inst: &Instruction,
        at_block: usize,
        at_index: usize,
        w: ImageWindow,
        budget: &mut usize,
        visiting: &mut Vec<Value>,
    ) -> ObjectMention {
        use ObjectMention::{NoWrite, Reject, Write};
        match inst {
            Instruction::Intrinsic {
                op, args, dest_ptr, ..
            } => self.intrinsic_mention(
                *op, args, *dest_ptr, at_block, at_index, w, budget, visiting,
            ),
            Instruction::Memcpy {
                dest,
                src,
                size: copy_len,
            } => self.memcpy_mention(
                *dest, *src, *copy_len, at_block, at_index, w, budget, visiting,
            ),
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
                    Operand::Value(v) => {
                        self.known_scalar_bytes(*v, ty.size(), at_block, at_index, budget, visiting)
                    }
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
        at_block: usize,
        at_index: usize,
        w: ImageWindow,
        budget: &mut usize,
        visiting: &mut Vec<Value>,
    ) -> ObjectMention {
        use ObjectMention::{NoWrite, Reject, Write};
        if copy_len == 0 || copy_len > MAX_IMAGE {
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
            // ANCHORED NESTED PROOF.  The copy takes a snapshot of `src`
            // at the COPY's program position, not at the outer read's.  Using
            // `w.read_*` here would admit a write to `src` that happens AFTER
            // the copy but BEFORE the outer read -- the classic
            // `d = src; src.a = 5; return d.a` miscompile.  Last-write-wins
            // made that stale-write admission observable, so anchor every
            // nested proof at the instruction being audited.
            let Some(image) = self.object_image(
                s_base, s_off, copy_len, at_block, at_index, budget, visiting,
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
        at_block: usize,
        at_index: usize,
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
            let Some(image) =
                self.known_vector(args[0], store_width, at_block, at_index, budget, visiting)
            else {
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
                self.operand_image(args[0], width, at_block, at_index, budget, visiting)
            else {
                return Reject;
            };
            let chosen = match MaskStatus::from_lanes(&mask, selector_bits) {
                MaskStatus::AllOff => args[1],
                MaskStatus::AllOn => args[2],
                MaskStatus::Mixed(_) | MaskStatus::Unknown => return Reject,
            };
            let Some(image) =
                self.operand_image(chosen, width, at_block, at_index, budget, visiting)
            else {
                return Reject;
            };
            return Write(d_off, image);
        }
        if let Some(zero_width) = dest_ptr_zero_width(op) {
            // ISPC's `ConstantAggregateZero` adaptation for the legacy
            // write-through zero producers.  These have NO data operands,
            // so there is nothing to prove except the destination: the
            // emitted `pxor`/`xorps`/`vpxor` + store writes exactly
            // `zero_width` zero bytes.  `tile_image` still checks that the
            // window is inside the object, so a 32-byte producer into a
            // 16-byte alloca is refused rather than truncated.
            let Some(dp) = dest_ptr else {
                return Reject;
            };
            let mut d_off = 0i64;
            let Some(d_base) = self.resolve_pointer(dp, &mut d_off, budget) else {
                return Reject;
            };
            if d_base != w.base {
                return NoWrite; // writes another object; reads nothing
            }
            return match ByteImage::zeros(zero_width) {
                Some(img) => Write(d_off, img),
                None => Reject,
            };
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
                let Some(img) = self
                    .byte_logic_image(kind, vec_width, args, at_block, at_index, budget, visiting)
                else {
                    return Reject;
                };
                img
            } else {
                // Exact-width splat; refuse widened or non-constant operands
                // (truncation of an `I64` into a dword lane is not provable).
                let Some((vec_width, lane_width)) = dest_ptr_splat_shape(op) else {
                    return Reject;
                };
                if args.len() != 1 || vec_width > MAX_IMAGE {
                    return Reject;
                }
                let Operand::Const(c) = args[0] else {
                    return Reject;
                };
                let Some(lane) = ir_const_bytes(c) else {
                    return Reject;
                };
                if lane.len() != lane_width {
                    return Reject;
                }
                let Some(img) = ByteImage::splat(lane.as_slice(), vec_width) else {
                    return Reject;
                };
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
        if dest_ptr.is_none() && mask_compare_shape(op).is_some() {
            // AVX-512 scalar-mask compares READ two vector operands and write
            // only an SSA scalar `dest`; they cannot change any object's
            // image.  Pin BOTH pointer operands to resolved stack objects so
            // no derived-address access escapes the audit, exactly as the
            // `Blendv*`/`Pand*` "reads a different object" arms do.  The
            // third operand is the immediate predicate, not an address.
            if args.len() != 3 {
                return Reject;
            }
            for arg in args.iter().take(2) {
                if let Operand::Value(p) = arg {
                    let mut zero = 0i64;
                    if self.resolve_pointer(*p, &mut zero, budget).is_none() {
                        return Reject;
                    }
                }
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
    ) -> Option<ByteImage> {
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
    ) -> Option<ByteImage> {
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

/// ISPC `ReplaceMaskedMemOps` + `IntrinsicsOpt` transplant: ONE decision
/// procedure for every masked intrinsic.
///
/// * all-off  -> zero producer (zeroing forms), copy of the merge source
///               (merge forms), or erasure (stores; `fold_removal`);
/// * all-on   -> drop the mask and use the proven unmasked counterpart;
/// * a TRUE PREFIX on a store -> ISPC `lCheckMask`'s narrower unmasked copy,
///               with a ZERO coalescing-gap budget.
///
/// Every rewrite is a single instruction, so the two-phase index contract in
/// `run_function` is preserved.  The destination window is bounds-proven and
/// the SSA `dest` must be unused (a `Memcpy`/zero producer writes memory but
/// produces no SSA vector value).  Mixed masks that are not a true prefix,
/// unknown masks, unsupported widths and ambiguous unmasked forms refuse.
///
/// Kept OUT of `fold`: splitting the rewrite family into its own function
/// keeps each entry point auditable and holds the census ratchet's
/// `fns_over_300` bound without re-baselining anything.
fn fold_masked_intrinsic(facts: &Facts<'_>, inst: &Instruction) -> Option<Instruction> {
    let Instruction::Intrinsic {
        op,
        args,
        dest,
        dest_ptr: Some(dp),
        ..
    } = inst
    else {
        return None;
    };
    let (mask_arg, arity, lanes, width, family) = masked_form(*op)?;
    if args.len() != arity || width == 0 || width > MAX_IMAGE {
        return None;
    }
    if dest.is_some_and(|d| facts.used.contains(&d.0)) {
        return None;
    }
    let status = facts.scalar_mask(args[mask_arg], lanes);

    // A masked LOAD/arith rewrite writes a VECTOR HOME through `dest_ptr`, so
    // it needs the home's bounds proof.  A masked STORE's `dest_ptr` is the
    // USER'S MEMORY ADDRESS (see `LcccSimdClass::PtrStore`): it can be a
    // global, a heap pointer or a parameter, and none of those resolve to an
    // `Alloca`.  Resolving it unconditionally and bailing out used to refuse
    // every all-on masked store to a non-stack object (measured: ON and OFF
    // assembly byte-identical, `kmovq %rax,%k1` + `vmovdqu8 ...{k1}` left in
    // the hot path).  Make the proof LAZY so `unmasked_store` below -- which
    // needs no object proof at all -- can still fire.
    let dest_home: Option<(Value, i64)> = {
        let mut budget = MAX_WORK;
        let mut d_off = 0i64;
        facts
            .resolve_pointer(*dp, &mut d_off, &mut budget)
            .map(|d_base| (d_base, d_off))
    };

    // The ONLY memory-write rewrite in this pass other than erasure.
    // Both windows are independently bounds-proven and distinct
    // (`memcpy` overlap is UB), so no out-of-bounds read or write can
    // be introduced.  `size` is the exact active byte count.
    let memcpy_from = |src: Operand, size: usize| -> Option<Instruction> {
        let (d_base, d_off) = dest_home?;
        if size == 0 || size > MAX_IMAGE || !window_in_bounds(&facts.defs, d_base, d_off, size) {
            return None;
        }
        let Operand::Value(sv) = src else {
            return None;
        };
        let mut b = MAX_WORK;
        let mut s_off = 0i64;
        let s_base = facts.resolve_pointer(sv, &mut s_off, &mut b)?;
        if s_base == d_base || !window_in_bounds(&facts.defs, s_base, s_off, size) {
            return None;
        }
        Some(Instruction::Memcpy {
            dest: *dp,
            src: sv,
            size,
        })
    };
    let zero_here = || -> Option<Instruction> {
        let (d_base, d_off) = dest_home?;
        if !window_in_bounds(&facts.defs, d_base, d_off, width) {
            return None;
        }
        Some(Instruction::Intrinsic {
            dest: None,
            op: zero_producer_op(width)?,
            dest_ptr: Some(*dp),
            args: Vec::new(),
        })
    };
    // All-on loads: only forms whose unmasked counterpart takes the
    // SAME RAW BYTE POINTER are admissible.  128-bit `Loaddqu`, 256-bit
    // `Loadu256Raw` and 512-bit `Loadu512` all dereference a raw pointer.
    // The ordinary `Loadu256` does NOT: it routes through `avx_load_arg`'s
    // vector-HOME heuristics, so it is deliberately never emitted here.
    let unmasked_load = || -> Option<Instruction> {
        let (d_base, d_off) = dest_home?;
        let load = match *op {
            IntrinsicOp::MaskzLoaduEpi8_128 | IntrinsicOp::MaskLoaduEpi8_128 => {
                IntrinsicOp::Loaddqu
            }
            IntrinsicOp::MaskzLoaduEpi8_256 | IntrinsicOp::MaskLoaduEpi8_256 => {
                IntrinsicOp::Loadu256Raw
            }
            IntrinsicOp::MaskzLoaduEpi8_512 | IntrinsicOp::MaskLoaduEpi8_512 => {
                IntrinsicOp::Loadu512
            }
            _ => return None,
        };
        if !matches!(args[1], Operand::Value(_))
            || !window_in_bounds(&facts.defs, d_base, d_off, width)
        {
            return None;
        }
        Some(Instruction::Intrinsic {
            dest: None,
            op: load,
            dest_ptr: Some(*dp),
            args: vec![args[1]],
        })
    };
    // All-on masked arithmetic: drop the mask operand, keep the data
    // operands in the SAME order.  Verified against the emitter's
    // arg conventions (`emit_maskz_set1_512`, `emit_maskz_maddubs_512`,
    // `emit_maskz_shuffle_epi8_128`, `emit_evex_binary_512`,
    // `emit_sse_binary_128`).
    let unmasked_arith = || -> Option<Instruction> {
        let (d_base, d_off) = dest_home?;
        use IntrinsicOp as O;
        let (new_op, new_args) = match *op {
            O::MaskzSet1Epi16_512 => (O::SetEpi16_512, vec![args[1]]),
            O::MaskzSet1Epi32_512 => (O::SetEpi32_512, vec![args[1]]),
            O::MaskzSet1Epi64x512 => (O::SetEpi64x512, vec![args[1]]),
            O::MaskzMaddubsEpi16_512 => (O::Pmaddubsw512, vec![args[1], args[2]]),
            O::MaskzShuffleEpi8_128 => (O::Pshufb128, vec![args[1], args[2]]),
            O::MaskShuffleEpi8_128 => (O::Pshufb128, vec![args[2], args[3]]),
            _ => return None,
        };
        if !window_in_bounds(&facts.defs, d_base, d_off, width) {
            return None;
        }
        Some(Instruction::Intrinsic {
            dest: None,
            op: new_op,
            dest_ptr: Some(*dp),
            args: new_args,
        })
    };

    // All-on masked STORE -> the exact unmasked store intrinsic.  This is
    // the only rewrite that needs NO object proof: the original instruction
    // already performed a FULL-WIDTH write through `dp` with the same value
    // operand, so dropping the all-on `k` mask cannot introduce a fault,
    // change an alias or widen an access.  The emitter loads the vector into
    // a register BEFORE storing (verified for `Storedqu`, `Storeu256` and
    // `Storeu512`), so it is also correct when `dp` overlaps the value's
    // stack home -- a case `Memcpy` must refuse as UB.  `Storedqu`/`Storeu256`/
    // `Storeu512` are the same PtrStore shapes `_mm*_storeu_si*` already use
    // with global, heap and parameter pointers (measured three-way probe).
    let unmasked_store = |value: Operand| -> Option<Instruction> {
        let store = match *op {
            IntrinsicOp::MaskStoreuEpi8_128 => IntrinsicOp::Storedqu,
            IntrinsicOp::MaskStoreuEpi8_256 => IntrinsicOp::Storeu256,
            IntrinsicOp::MaskStoreuEpi8_512 => IntrinsicOp::Storeu512,
            _ => return None,
        };
        Some(Instruction::Intrinsic {
            dest: None,
            op: store,
            dest_ptr: Some(*dp),
            args: vec![value],
        })
    };

    match status {
        ScalarMask::AllOff => match family {
            // `fold_removal` erases all-off stores; returning None
            // here lets that path run.
            MaskFamily::Store(_) => None,
            MaskFamily::MergeLoad(old) | MaskFamily::MergeArith(old) => {
                memcpy_from(args[old], width)
            }
            MaskFamily::ZeroLoad | MaskFamily::ZeroArith | MaskFamily::ZeroOnly => zero_here(),
        },
        ScalarMask::AllOn => match family {
            // Prefer the existing bounds-proven copy for a stack home; fall
            // back to the register store for every other destination.  NEVER
            // fall back for a partial (`Mixed` prefix) mask: that would write
            // the full width and corrupt the inactive lanes.
            MaskFamily::Store(value) => {
                memcpy_from(args[value], width).or_else(|| unmasked_store(args[value]))
            }
            MaskFamily::ZeroLoad | MaskFamily::MergeLoad(_) => unmasked_load(),
            MaskFamily::ZeroArith | MaskFamily::MergeArith(_) => unmasked_arith(),
            MaskFamily::ZeroOnly => None,
        },
        // ISPC `lCheckMask`: `0..n` active byte lanes become an exact
        // n-byte unmasked copy.  gap_budget == 0 is the ONLY sound
        // budget for DROPPING the mask.
        ScalarMask::Mixed(bits) => match family {
            MaskFamily::Store(value) => match true_prefix_len(bits, lanes, 0) {
                Some(n) if n < width as usize => memcpy_from(args[value], n),
                _ => None,
            },
            _ => None,
        },
        ScalarMask::Unknown => None,
    }
}

/// Bit width of an integer IR type.  FP, decimal, pointer, vector and void
/// types refuse: a mask-compare result is an integer bit pattern only.
#[inline]
fn integer_bits(ty: IrType) -> Option<usize> {
    Some(match ty {
        IrType::I8 | IrType::U8 => 8,
        IrType::I16 | IrType::U16 => 16,
        IrType::I32 | IrType::U32 => 32,
        IrType::I64 | IrType::U64 => 64,
        IrType::I128 | IrType::U128 => 128,
        _ => return None,
    })
}

/// The rewrite for one instruction, or `None` when nothing is provable.
fn fold(facts: &Facts<'_>, inst: &Instruction, block: usize, index: usize) -> Option<Instruction> {
    // A scalar `vpcmp` result has EXACTLY `lanes` significant bits: EVEX
    // zeroes every upper `k` bit and `kmov* %k1, %rax` zero-extends into rax.
    // A following U8/U16/U32 -> at-least-as-wide integer cast is therefore a
    // bit-identical no-op, but codegen materialises it as `movzwl %ax, %eax`
    // on EVERY runtime mask compare.  Rewriting it to a Copy removes that
    // roundtrip without inventing a single fact.  Refuse anything narrower
    // than `lanes` (that would be a real truncation), any non-integer type,
    // and any source that is not the DIRECT dest of a mask-compare intrinsic.
    if let Instruction::Cast {
        dest,
        src,
        from_ty,
        to_ty,
    } = inst
    {
        let (Some(from_bits), Some(to_bits)) = (integer_bits(*from_ty), integer_bits(*to_ty))
        else {
            return None;
        };
        if to_bits < from_bits {
            return None;
        }
        let Operand::Value(v) = src else {
            return None;
        };
        let Some(def) = facts.defs.get(v) else {
            return None;
        };
        let Instruction::Intrinsic { op, .. } = **def else {
            return None;
        };
        let (_, _, lanes, _) = mask_compare_shape(op)?;
        if lanes as usize > from_bits {
            return None;
        }
        return Some(Instruction::Copy {
            dest: *dest,
            src: *src,
        });
    }
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

    // ---- ISPC `IntrinsicsOpt` transplant: scalar `__mmaskN` consumers ----
    //
    // (B) A packed-integer compare whose operands are a proven constant
    // image, or the SAME pure operand, produces a scalar mask constant.
    // `eq`(0) and `ne`(4) are the only sign-independent predicates; the
    // image is the SAME byte-lattice proof used by `compare_image`, so there
    // is one decision procedure for both encodings (byte lanes vs bits).
    if let Instruction::Intrinsic {
        op,
        args,
        dest: Some(d),
        dest_ptr: None,
        ..
    } = inst
    {
        if let Some((width, lane_bytes, lanes, unsigned)) = mask_compare_shape(*op) {
            if args.len() != 3 || lane_bytes == 0 || lane_bytes > 8 || width % lane_bytes != 0 {
                return None;
            }
            let Operand::Const(pred) = args[2] else {
                return None;
            };
            // Round 9: ALL eight `vpcmp` predicates are foldable, not just
            // eq/ne.  The opcode's `unsigned` bit plus the exact byte image
            // supplies the lane signedness; nothing is inferred.  Reject an
            // out-of-range immediate before walking any image or def chain.
            let pred = exact_i64(pred)?;
            if !(0..=7).contains(&pred) {
                return None;
            }
            let full = lane_mask(lanes);
            let mut budget = MAX_WORK;
            let bits = if matches!(pred, 3 | 7)
                && facts.operand_is_pure(args[0], &mut budget)
                && facts.operand_is_pure(args[1], &mut budget)
            {
                // `FALSE` (3) and `TRUE` (7) are operand-independent: the
                // hardware ignores both vector inputs.  GCC 16.2 still emits
                // 48 + 48 redundant `vpcmp $3/$7` + `kmovq` pairs in the
                // 485-case matrix, while Clang 23.1.5 folds all of them.
                // Requiring a purity proof for BOTH operands guarantees that
                // eliminating the compare cannot discard a volatile load, a
                // call or a semantically-volatile object: the exact same
                // proof the `x op x` identity below already demands.
                if pred == 3 { 0 } else { full }
            } else if args[0] == args[1] && facts.operand_is_pure(args[0], &mut budget) {
                // `x op x` is lane-independent: evaluate the predicate once
                // on two identical zero lanes.  This also rejects an invalid
                // predicate instead of silently producing a mask.
                let equal = [0u8; 8];
                let lane = &equal[..lane_bytes];
                if compare_predicate(pred, unsigned, lane, lane)? {
                    full
                } else {
                    0
                }
            } else {
                let a = facts.image_of(args[0], width, block, index)?;
                let b = facts.image_of(args[1], width, block, index)?;
                if a.len() != width || b.len() != width {
                    return None;
                }
                let lane_count = a.chunks_exact(lane_bytes).count();
                // A 512-bit byte compare has EXACTLY 64 lanes: `1u64 << i`
                // is defined for i in 0..64 (max shift 63). Rejecting
                // `> 63` silently refused every 64-lane compare; the real
                // bound is the width of the mask register, u64::BITS.
                if lane_count != lanes as usize || lane_count > u64::BITS as usize {
                    return None;
                }
                let mut m = 0u64;
                for (i, (x, y)) in a
                    .chunks_exact(lane_bytes)
                    .zip(b.chunks_exact(lane_bytes))
                    .enumerate()
                {
                    if compare_predicate(pred, unsigned, x, y)? {
                        m |= 1u64 << i;
                    }
                }
                m
            };
            return Some(Instruction::Copy {
                dest: *d,
                src: Operand::Const(IrConst::I64(bits as i64)),
            });
        }
    }

    // (C) ISPC `ReplaceMaskedMemOps` + `IntrinsicsOpt` transplant.  The
    // generalised masked decision procedure lives in its own function; the
    // short-circuit below preserves `fold_removal`'s all-off-store erasure
    // (a refused masked rewrite must not fall into the SSA-dest folds).
    if let Instruction::Intrinsic {
        op,
        dest_ptr: Some(_),
        ..
    } = inst
    {
        if masked_form(*op).is_some() {
            return fold_masked_intrinsic(facts, inst);
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

/// ISPC `IntrinsicsOpt`'s masked-store arm: an all-off masked store performs
/// NO memory access, so the whole instruction (including the `kmov` that
/// would build the k mask) is dead.  Erasure needs a removal channel rather
/// than a replacement, so it is decided separately from `fold` and applied
/// through the span-safe `remove_instructions` helper.  Fail-closed: only the
/// canonical `dest: None, dest_ptr: Some(_)` PtrStore form with exactly two
/// arguments and a PROVEN all-off scalar mask is erasable.
fn fold_removal(facts: &Facts<'_>, inst: &Instruction) -> bool {
    let Instruction::Intrinsic {
        op,
        args,
        dest: None,
        dest_ptr: Some(_),
        ..
    } = inst
    else {
        return false;
    };
    let Some(lanes) = masked_store_shape(*op) else {
        return false;
    };
    args.len() == 2 && facts.scalar_mask(args[0], lanes) == ScalarMask::AllOff
}

pub(crate) fn run(module: &mut IrModule) -> usize {
    module.for_each_function(run_function)
}

fn run_function(func: &mut IrFunction) -> usize {
    let has_candidate = func.blocks.iter().any(|block| {
        block.instructions.iter().any(|inst| {
            if let Instruction::Intrinsic { op, dest_ptr, .. } = inst {
                return match dest_ptr {
                    None => {
                        is_movemask(*op)
                            || blend_shape(*op).is_some()
                            || mask_compare_shape(*op).is_some()
                    }
                    Some(_) => legacy_blend_shape(*op).is_some() || masked_fold_candidate(*op),
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
    let (edits, removals): (Vec<(usize, usize, Instruction)>, Vec<(usize, usize)>) = {
        let facts = Facts::new(func);
        let mut edits = Vec::new();
        let mut removals = Vec::new();
        for (block_id, block) in func.blocks.iter().enumerate() {
            for (index, inst) in block.instructions.iter().enumerate() {
                if let Some(replacement) = fold(&facts, inst, block_id, index) {
                    edits.push((block_id, index, replacement));
                } else if fold_removal(&facts, inst) {
                    removals.push((block_id, index));
                }
            }
        }
        (edits, removals)
    };
    // Every decision above was made against the SAME immutable snapshot, so
    // the indices here are original indices.  Replacements are applied first,
    // then removals through the span-safe helper (which sorts/dedups per
    // block and keeps `source_spans` parallel).  No index can drift.
    let count = edits.len() + removals.len();
    for (block_id, index, replacement) in edits {
        func.blocks[block_id].instructions[index] = replacement;
    }
    if !removals.is_empty() {
        remove_instructions(func, &removals);
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
                    if s == Some(db) || *copy_len == 0 || *copy_len > MAX_IMAGE {
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
                    || dest_ptr_zero_width(*op).is_some()
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
