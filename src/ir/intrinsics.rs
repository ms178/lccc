/// Target-independent intrinsic operations.
///
/// These represent SIMD, crypto, math, and hardware intrinsics that the IR
/// can express without target-specific details. Each backend emits the
/// appropriate native instructions for its architecture.
///
/// Organized by ISA extension:
/// - Fences/barriers (Lfence, Mfence, Sfence, Pause)
/// - Non-temporal stores (Movnti, Movntdq, etc.)
/// - SSE2 packed integer ops (Pcmpeqb, Paddw, Psubd, etc.)
/// - SSE2 shuffle/pack/unpack (Pshufd, Packssdw, Punpcklbw, etc.)
/// - SSE2/SSE4.1 insert/extract (Pinsrw, Pextrw, Pinsrd, etc.)
/// - AES-NI (Aesenc, Aesdec, etc.)
/// - CLMUL (Pclmulqdq)
/// - CRC32
/// - Scalar math (SqrtF32, SqrtF64, FabsF32, FabsF64)
/// - Frame/return address builtins

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IntrinsicOp {
    /// Memory fence operations (no dest, no args beyond optional ptr)
    Lfence,
    Mfence,
    Sfence,
    Pause,
    Vzeroupper,
    Clflush,
    /// Non-temporal stores: movnti (32-bit), movnti64 (64-bit), movntdq (128-bit), movntpd (128-bit double)
    Movnti,
    Movnti64,
    Movntdq,
    Movntpd,
    /// Load/store 128-bit unaligned
    Loaddqu,
    Storedqu,
    /// Compare equal packed bytes (16 bytes)
    Pcmpeqb128,
    /// Compare equal packed dwords (4x32)
    Pcmpeqd128,
    /// Subtract packed unsigned saturated bytes
    Psubusb128,
    /// Subtract packed signed saturated bytes
    Psubsb128,
    /// Bitwise OR/AND/XOR on 128-bit
    Por128,
    Pand128,
    Pxor128,
    /// Packed single-precision float ops (addps/subps/mulps).
    /// xorps/andps/orps reuse the integer Pxor/Pand/Por ops (bitwise identical).
    AddPs128,
    SubPs128,
    MulPs128,
    /// Packed double-precision float ops (addpd/subpd/mulpd).
    AddPd128,
    SubPd128,
    MulPd128,
    /// Packed 32x32->64 multiplies: pmuludq (SSE2, unsigned),
    /// pmuldq (SSE4.1, signed), pmulld (SSE4.1, low 32).
    Pmuludq128,
    Pmuldq128,
    Pmulld128,
    /// Free 128-bit reinterpret cast: the value is passed through unchanged.
    CastReinterpret128,
    /// Move byte mask (pmovmskb) - returns i32
    Pmovmskb128,
    /// Set all bytes to value (splat)
    SetEpi8,
    /// Set all dwords to value (splat)
    SetEpi32,
    /// CRC32 accumulate
    Crc32_8,
    Crc32_16,
    Crc32_32,
    Crc32_64,
    /// __builtin_ia32_rdtsc() - 64-bit timestamp counter (EDX:EAX)
    Rdtsc,
    /// __builtin_ia32_rdtscp(&aux) - rdtscp, stores IA32_TSC_AUX (ecx) to aux
    Rdtscp,
    /// __builtin_frame_address(0) - returns current frame pointer
    FrameAddress,
    /// __builtin_return_address(0) - returns current return address
    ReturnAddress,
    /// GCC's local-frame setjmp primitive. Args: user buffer pointer.
    BuiltinSetjmp,
    /// GCC's local-frame longjmp primitive. Args: user buffer pointer, value.
    BuiltinLongjmp,
    /// __builtin_apply_args support: emits the target-specific save-area size
    /// in bytes (x86-64: 6 int arg regs + al + 8 XMM arg regs = 184; i686:
    /// 16-byte incoming-register block + the caller's stack argument area).
    /// dest = size in bytes.
    ApplyArgsAreaSize,
    /// __builtin_apply_args(): snapshot the incoming argument registers (and
    /// on i686 the caller's stack argument area) into the save area.
    /// dest_ptr = save area.  Reads arg registers; writes only the area.
    SaveApplyArgs,
    /// __builtin_apply(func, args, size): restore argument registers from the
    /// save area (i686: re-stage `size` bytes of stack arguments), perform the
    /// indirect call, and capture the result into the result area.
    /// Args: [func_ptr, save_area, result_area, size].
    DoBuiltinApply,
    /// __builtin_return(block): load the current function's return value from
    /// the result block produced by __builtin_apply and return.
    /// Args: [result_area].  Terminates the block.
    RestoreApplyResult,
    /// __builtin_thread_pointer() - returns thread pointer (TLS base address)
    ThreadPointer,
    /// Scalar square root: sqrtsd/sqrtss on x86, fsqrt on ARM/RISC-V
    /// args[0] = input float value; dest = sqrt result
    SqrtF32,
    SqrtF64,
    /// Scalar absolute value: bitwise AND with sign mask on x86, fabs on ARM/RISC-V
    /// args[0] = input float value; dest = |x|
    FabsF32,
    FabsF64,
    /// C99 fma()/fmaf(): dest = args[0] * args[1] + args[2] with a SINGLE
    /// rounding. Must lower to a hardware FMA instruction (vfmadd on x86);
    /// splitting into Mul+Add would double-round and is a correctness bug,
    /// not a performance choice (glibc's s_fma.c/s_fmaf.c via the ms178
    /// math-use-builtins-fma.h are the canonical consumers).
    FmaScalarF32,
    FmaScalarF64,
    /// SSE4.1/AVX scalar directed rounding, payload = the ROUNDSS/ROUNDSD
    /// imm8 (GCC-verified at -O2 -march=x86-64-v3):
    ///   floor = 9, ceil = 10, trunc = 11,
    ///   rint = 4 (dynamic MXCSR mode, inexact ALLOWED — C99 rint),
    ///   nearbyint = 12 (dynamic mode, inexact SUPPRESSED — C99 nearbyint),
    ///   roundeven = 8 (ties-to-even, inexact suppressed — C23 roundeven).
    /// Inline expansion is a CORRECTNESS requirement for glibc self-hosting:
    /// its generic s_floor.c/s_trunc.c/... define floor() AS __builtin_floor
    /// under USE_*_BUILTIN, so lowering to a libm call recurses into the
    /// function being compiled.
    RoundScalarF32(u8),
    RoundScalarF64(u8),
    /// copysign(x, y)/copysignf: magnitude of x with the sign bit of y,
    /// pure SSE bit ops (and/andn/or) — never a libm call (glibc's
    /// s_copysign.c defines copysign() AS __builtin_copysign).
    CopysignF32,
    CopysignF64,
    /// _Float128 fabs: clear sign bit 127 (inline bit ops, no libgcc call).
    F128Fabs,
    /// _Float128 negate: toggle sign bit 127 (NOT an integer negation).
    F128Neg,
    /// _Float128 copysign: x magnitude combined with y sign bit 127.
    F128Copysign,
    /// long double (80-bit x87) fabs: clear bit 79 in the 10-byte slot.
    LDFabs,
    /// long double (80-bit x87) copysign: x magnitude with y sign bit 79.
    LDCopysign,
    /// Packed double FMA for vectorized matmul inner loop.
    /// Computes: *dest_ptr[0..2] += broadcast(*args[0]) * *args[1][0..2]
    /// dest_ptr: pointer to 2×F64 accumulator (read+write, 16 bytes)
    /// args[0]: pointer to scalar F64 (broadcast to both SSE lanes)
    /// args[1]: pointer to 2×F64 (one SSE register worth)
    /// NOT pure: modifies memory at dest_ptr.
    FmaF64x2,
    /// Two-wide FMA using a broadcast previously loaded by BroadcastLoadF64.
    /// dest_ptr is C and args[0] is the B vector pointer.
    FmaF64x2Hoisted,
    /// Packed double FMA for AVX2 4-wide vectorized loops.
    /// Computes: *dest_ptr[0..4] += broadcast(*args[0]) * *args[1][0..4]
    /// dest_ptr: pointer to 4×F64 accumulator (read+write, 32 bytes)
    /// args[0]: pointer to scalar F64 (broadcast to all 4 lanes)
    /// args[1]: pointer to 4×F64 source vector
    /// NOT pure: modifies memory at dest_ptr.
    FmaF64x4,
    /// Like FmaF64x4, but the A[i][k] broadcast has been hoisted out of the
    /// inner loop. The codegen assumes ymm1 already holds the broadcast value
    /// (set by a preceding BroadcastLoadF64 instruction).
    /// dest_ptr = C pointer (read+write, 4×F64)
    /// args[0] = B pointer (4×F64)  (no A pointer needed)
    FmaF64x4Hoisted,
    /// Load a scalar F64 from a pointer and broadcast to all 4 lanes of ymm1.
    /// Placed before the vectorized j-loop to hoist the A[i][k] broadcast.
    /// args[0] = pointer to scalar F64
    BroadcastLoadF64,
    /// FMA with SIB addressing: uses base + byte_offset for B and C accesses.
    /// This eliminates the GEP address computation from the inner loop by
    /// using x86 SIB addressing directly: vmovupd (%base, %offset), %ymm0.
    /// args[0] = A pointer (scalar F64, broadcast inside)
    /// args[1] = C base pointer (row base, loop-invariant in j-loop)
    /// args[2] = B base pointer (row base, loop-invariant in j-loop)
    /// args[3] = byte offset (the j-loop IV, increments by 32 each iteration)
    FmaF64x4SIB,
    /// Like FmaF64x4SIB but with A broadcast hoisted to ymm1.
    /// Eliminates both GEP address computation AND broadcast load from inner loop.
    /// args[0] = C base pointer (row base, loop-invariant)
    /// args[1] = B base pointer (row base, loop-invariant)
    /// args[2] = byte offset (j-loop IV, shared across chunks)
    /// args[3] = optional displacement (0,32,64,96) for quad unroll — if present,
    ///           emits disp(%base,%off) SIB, avoiding extra leaq/addq.
    ///           If absent, behaves like 3-arg form (offset already includes chunk).
    /// ymm1 must already hold broadcasted A[i][k] (from BroadcastLoadF64).
    FmaF64x4HoistedSIB,

    // --- Vector loads for reduction patterns ---
    /// Load 4 packed doubles (256-bit unaligned): vmovupd
    /// args[0] = base pointer, args[1] = byte offset
    /// dest_ptr = result ptr (32 bytes, 4×F64)
    LoadF64x4,
    /// Load 2 packed doubles (128-bit unaligned): movupd
    /// args[0] = base pointer, args[1] = byte offset
    /// dest_ptr = result ptr (16 bytes, 2×F64)
    LoadF64x2,
    /// Load 8 packed 32-bit ints (256-bit unaligned): vmovdqu
    /// args[0] = base pointer, args[1] = byte offset
    /// dest_ptr = result ptr (32 bytes, 8×I32)
    LoadI32x8,
    /// Load 4 packed 32-bit ints (128-bit unaligned): movdqu
    /// args[0] = base pointer, args[1] = byte offset
    /// dest_ptr = result ptr (16 bytes, 4×I32)
    LoadI32x4,

    // --- Vector arithmetic for reductions ---
    /// Packed 4×F64 add (256-bit): vaddpd
    /// args[0] = src1 ptr, args[1] = src2 ptr; dest_ptr = result ptr
    AddF64x4,
    /// Packed 2×F64 add (128-bit): addpd
    /// args[0] = src1 ptr, args[1] = src2 ptr; dest_ptr = result ptr
    AddF64x2,
    /// Packed 4×F64 multiply (256-bit): vmulpd
    /// args[0] = src1 ptr, args[1] = src2 ptr; dest_ptr = result ptr
    MulF64x4,
    /// Packed 2×F64 multiply (128-bit): mulpd
    /// args[0] = src1 ptr, args[1] = src2 ptr; dest_ptr = result ptr
    MulF64x2,
    /// Packed 8×I32 add (256-bit): vpaddd
    /// args[0] = src1 ptr, args[1] = src2 ptr; dest_ptr = result ptr
    AddI32x8,
    /// Packed 4×I32 add (128-bit): paddd
    /// args[0] = src1 ptr, args[1] = src2 ptr; dest_ptr = result ptr
    AddI32x4,

    // --- Horizontal reduction (vector → scalar) ---
    /// Horizontal add 4×F64 → 1×F64 (AVX2)
    /// Reduces {a, b, c, d} to a+b+c+d
    /// args[0] = src vector ptr (32 bytes); dest = scalar F64
    HorizontalAddF64x4,
    /// Horizontal add 2×F64 → 1×F64 (SSE2)
    /// Reduces {a, b} to a+b
    /// args[0] = src vector ptr (16 bytes); dest = scalar F64
    HorizontalAddF64x2,
    /// Horizontal add 8×I32 → 1×I32 (AVX2)
    /// Reduces {a,b,c,d,e,f,g,h} to a+b+c+d+e+f+g+h
    /// args[0] = src vector ptr (32 bytes); dest = scalar I32
    HorizontalAddI32x8,
    /// Horizontal add 4×I32 → 1×I32 (SSE2)
    /// Reduces {a,b,c,d} to a+b+c+d
    /// args[0] = src vector ptr (16 bytes); dest = scalar I32
    HorizontalAddI32x4,

    // --- Register-based vector operations for reductions (SSA-friendly) ---
    /// Vector load: %dest_vec = load_vector(base_ptr, offset) - AVX2 4×F64
    /// Returns vector value in SSA, lives in %ymm register
    /// args[0] = base pointer, args[1] = byte offset; dest = vector value
    VecLoadF64x4,
    /// Vector load: %dest_vec = load_vector(base_ptr, offset) - SSE2 2×F64
    /// Returns vector value in SSA, lives in %xmm register
    /// args[0] = base pointer, args[1] = byte offset; dest = vector value
    VecLoadF64x2,
    /// Vector load: %dest_vec = load_vector(base_ptr, offset) - AVX2 8×I32
    /// args[0] = base pointer, args[1] = byte offset; dest = vector value
    VecLoadI32x8,
    /// Vector load: %dest_vec = load_vector(base_ptr, offset) - SSE2 4×I32
    /// args[0] = base pointer, args[1] = byte offset; dest = vector value
    VecLoadI32x4,
    /// Vector load: %dest_vec = load_vector(base_ptr, offset) - AVX2 8×F32
    /// args[0] = base pointer, args[1] = byte offset; dest = vector value
    VecLoadF32x8,
    /// Vector load: %dest_vec = load_vector(base_ptr, offset) - SSE2 4×F32
    /// args[0] = base pointer, args[1] = byte offset; dest = vector value
    VecLoadF32x4,

    /// Vector add: %dest_vec = %src1_vec + %src2_vec - AVX2 4×F64
    /// args[0] = src1 vector value, args[1] = src2 vector value; dest = result vector
    VecAddF64x4,
    /// Vector add: %dest_vec = %src1_vec + %src2_vec - SSE2 2×F64
    /// args[0] = src1 vector value, args[1] = src2 vector value; dest = result vector
    VecAddF64x2,
    /// Vector multiply: %dest_vec = %src1_vec * %src2_vec - AVX2 4×F64
    /// args[0] = src1 vector value, args[1] = src2 vector value; dest = result vector
    VecMulF64x4,
    /// Vector multiply: %dest_vec = %src1_vec * %src2_vec - SSE2 2×F64
    /// args[0] = src1 vector value, args[1] = src2 vector value; dest = result vector
    VecMulF64x2,
    /// Fused reduction step over two memory vectors, AVX2 4×F64.
    /// args = [accumulator, a_base, a_byte_offset, b_base, b_byte_offset].
    VecFmaF64x4,
    /// Contract-legal affine map: input * scale + bias, AVX 4×F64.
    VecMaddF64x4,
    /// Vector add: %dest_vec = %src1_vec + %src2_vec - AVX2 8×I32
    /// args[0] = src1 vector value, args[1] = src2 vector value; dest = result vector
    VecAddI32x8,
    /// Vector add: %dest_vec = %src1_vec + %src2_vec - SSE2 4×I32
    /// args[0] = src1 vector value, args[1] = src2 vector value; dest = result vector
    VecAddI32x4,
    /// Vector add: %dest_vec = %src1_vec + %src2_vec - AVX2 8×F32
    VecAddF32x8,
    /// Vector add: %dest_vec = %src1_vec + %src2_vec - SSE2 4×F32
    VecAddF32x4,
    /// Vector multiply: %dest_vec = %src1_vec * %src2_vec - AVX2 8×F32
    VecMulF32x8,
    /// Fused reduction step over two memory vectors, AVX2 8×F32.
    /// args = [accumulator, a_base, a_byte_offset, b_base, b_byte_offset].
    VecFmaF32x8,
    /// Contract-legal affine map: input * scale + bias, AVX 8×F32.
    VecMaddF32x8,
    /// Fixed-width squared distance reduced directly to the scalar FP ABI
    /// result. args = [a_base, b_base]. Reassociation must be enabled.
    FixedDistanceF32x8,
    FixedDistanceF64x4,
    /// Broadcast scalar f32 to 8 lanes (vbroadcastss).
    VecBroadcastF32x8,
    /// Broadcast scalar f32 to 4 lanes.
    VecBroadcastF32x4,
    /// Broadcast scalar f64 to 4 lanes (vbroadcastsd).
    VecBroadcastF64x4,
    /// Broadcast scalar f64 to 2 lanes.
    VecBroadcastF64x2,
    /// Vector multiply: %dest_vec = %src1_vec * %src2_vec - SSE2 4×F32
    VecMulF32x4,
    /// Vector subtract: %dest_vec = %src1_vec - %src2_vec - AVX2 4×F64.
    /// args[0] = src1 vector value, args[1] = src2 vector value (non-commutative).
    VecSubF64x4,
    /// Vector subtract: %dest_vec = %src1_vec - %src2_vec - SSE2 2×F64.
    VecSubF64x2,
    /// Vector subtract: %dest_vec = %src1_vec - %src2_vec - AVX2 8×F32.
    VecSubF32x8,
    /// Vector subtract: %dest_vec = %src1_vec - %src2_vec - SSE2 4×F32.
    VecSubF32x4,
    /// Vector divide: %dest_vec = %src1_vec / %src2_vec - AVX2 4×F64.
    /// Elementwise IEEE division; identical per-lane results to scalar div.
    VecDivF64x4,
    /// Vector divide: %dest_vec = %src1_vec / %src2_vec - SSE2 2×F64.
    VecDivF64x2,
    /// Vector divide: %dest_vec = %src1_vec / %src2_vec - AVX2 8×F32.
    VecDivF32x8,
    /// Vector divide: %dest_vec = %src1_vec / %src2_vec - SSE2 4×F32.
    VecDivF32x4,
    /// Vector square root: %dest_vec = sqrt(%src_vec) - AVX2 4×F64.
    /// args[0] = source vector value; dest = result vector.
    VecSqrtF64x4,
    /// Vector square root: %dest_vec = sqrt(%src_vec) - SSE2 2×F64.
    VecSqrtF64x2,
    /// Vector square root: %dest_vec = sqrt(%src_vec) - AVX2 8×F32.
    VecSqrtF32x8,
    /// Vector square root: %dest_vec = sqrt(%src_vec) - SSE2 4×F32.
    VecSqrtF32x4,
    /// Packed IEEE minimum with x86 `MINPS/MINPD` operand semantics:
    /// `dest[i] = args[0][i] < args[1][i] ? args[0][i] : args[1][i]`.
    /// The SECOND operand is returned whenever a lane is unordered (NaN)
    /// or both lanes are zero of either sign — exactly the C expression
    /// `a < b ? a : b`, so the vectorizer's exact (non-fast-math) min/max
    /// recognition is sound only when it preserves this operand order.
    /// NOT commutative. AVX 8×F32.
    VecMinF32x8,
    /// Packed minimum (see `VecMinF32x8`) - SSE 4×F32.
    VecMinF32x4,
    /// Packed minimum (see `VecMinF32x8`) - AVX 4×F64.
    VecMinF64x4,
    /// Packed minimum (see `VecMinF32x8`) - SSE2 2×F64.
    VecMinF64x2,
    /// Packed maximum with `MAXPS/MAXPD` semantics:
    /// `dest[i] = args[0][i] > args[1][i] ? args[0][i] : args[1][i]`
    /// (second operand on unordered / both-zero). NOT commutative. AVX 8×F32.
    VecMaxF32x8,
    /// Packed maximum (see `VecMaxF32x8`) - SSE 4×F32.
    VecMaxF32x4,
    /// Packed maximum (see `VecMaxF32x8`) - AVX 4×F64.
    VecMaxF64x4,
    /// Packed maximum (see `VecMaxF32x8`) - SSE2 2×F64.
    VecMaxF64x2,
    /// Packed FP compare producing an all-ones/all-zeros lane mask:
    /// `dest[i] = (args[0][i] PRED args[1][i]) ? ~0 : 0`. `args[2]` is the
    /// `CMPPS` predicate immediate (0 = EQ_OQ, 1 = LT_OS, 2 = LE_OS,
    /// 4 = NEQ_UQ; the SSE2-baseline subset, so the 128-bit form never needs
    /// AVX's extended predicates). AVX 8×F32.
    VecCmpF32x8,
    /// Packed FP compare (see `VecCmpF32x8`) - SSE 4×F32.
    VecCmpF32x4,
    /// Packed FP compare (see `VecCmpF32x8`) - AVX 4×F64.
    VecCmpF64x4,
    /// Packed FP compare (see `VecCmpF32x8`) - SSE2 2×F64.
    VecCmpF64x2,
    /// Lane-mask select: `dest[i] = mask[i] ? args[1][i] : args[0][i]` with
    /// `args = [false_vec, true_vec, mask]` (the `VBLENDVPS src1, src2, mask`
    /// operand order). The 128-bit forms lower to the SSE2-baseline
    /// `andps/andnps/orps` triple, so no SSE4.1 dependency is introduced.
    /// AVX 8×F32.
    VecBlendvF32x8,
    /// Lane-mask select (see `VecBlendvF32x8`) - SSE 4×F32.
    VecBlendvF32x4,
    /// Lane-mask select (see `VecBlendvF32x8`) - AVX 4×F64.
    VecBlendvF64x4,
    /// Lane-mask select (see `VecBlendvF32x8`) - SSE2 2×F64.
    VecBlendvF64x2,
    /// Widening reduction step: dest(I64x2 accumulator) += sign-extend of
    /// 4×I32 loaded from (base, byte_offset). One intrinsic = load 4 I32s,
    /// widen lanes 0..1 and 2..3 to two I64x2 halves, add both into the
    /// accumulator. x86 lowering: vmovdqu + vpmovsxdq×2 + vextracti128 +
    /// paddq×2. Full I64 precision per lane — `long s += (long)int_arr[i]`
    /// over 4 elements/iteration.
    VecWidenAddI32x4ToI64x2,
    /// Masked widening reduction step: dest(I64x2 accumulator) += sext of
    /// 4×I32 loaded from (base, byte_offset) WHERE the lane satisfies
    /// `lane > guard_rhs`. args = [accumulator, base, byte_offset,
    /// guard_rhs]. The x86 lowering builds the per-lane I32 mask with
    /// vpcmpgtd, sign-extends it through the same vpmovsxdq lane geometry
    /// (all-ones/all-zeros I64 masks), and vpand zero-masks the widened
    /// values before the paddq folds — exact lane-guarded I64 accumulation
    /// (GCC's canonical conditional-reduction form).
    VecWidenMaskedAddI32x4ToI64x2,
    /// Masked equal-width reduction step: dest(I32x8 accumulator) +=
    /// 8×I32 loaded from (base, byte_offset) WHERE the lane satisfies
    /// `lane > guard_rhs` (signed).  args = [accumulator, base,
    /// byte_offset, guard_rhs].  The x86 lowering builds the per-lane I32
    /// mask with vpcmpgtd (lanes > rhs), vpand zero-masks the loaded lanes
    /// and vpaddd folds them into the accumulator — the equal-width sibling
    /// of VecWidenMaskedAddI32x4ToI64x2.  Without it the non-widening
    /// conditional-sum transform silently DROPPED the Select guard
    /// (`if (a[i] > 0) s += a[i]` summed every element; regression:
    /// tests/regression/vector_guard_sum.c).
    VecMaskedAddI32x8,
    /// Vector multiply: %dest_vec = %src1_vec * %src2_vec - 4×I32
    /// args[0] = src1 vector value, args[1] = src2 vector value; dest = result vector
    VecMulI32x4,
    VecMulI32x8,
    /// NEON smax: per-lane signed max of two 4xI32 vectors (args[0], args[1]).
    /// Integer max is associative, commutative, and idempotent, so lane-order
    /// reduction matches sequential scalar max bit-for-bit (levkropp 8b139820).
    VecSmaxI32x4,
    /// NEON smaxv: horizontal signed max of 4xI32 lanes; dest = scalar I32.
    VecHorizontalMaxI32x4,
    /// Broadcast a scalar I32 to all 4 lanes: %dest_vec = {x, x, x, x}
    /// args[0] = scalar I32 value; dest = result vector
    VecBroadcastI32x4,
    VecBroadcastI32x8,
    /// Vector store: store 4×I32 vector to memory.
    /// dest_ptr = destination pointer; args[0] = source vector value.
    VecStoreI32x4,
    VecStoreI32x8,
    /// Store 8×F32 (vmovups ymm).
    VecStoreF32x8,
    /// Store 4×F32.
    VecStoreF32x4,
    /// Store 4×F64 (vmovupd ymm).
    VecStoreF64x4,
    /// Store 2×F64 (movupd xmm).
    VecStoreF64x2,
    /// Load two signed I32 lanes and widen to two I64 lanes.
    VecLoadWidenI32ToI64x2,
    /// Load two I64 lanes (movdqu).
    VecLoadI64x2,
    VecAddI64x2,
    VecMulI64x2,
    VecStoreI64x2,
    VecBroadcastI64x2,
    VecLoadI64x4,
    VecAddI64x4,
    VecHorizontalAddI64x4,
    VecZeroI64x4,

    /// NEON sadalp: sign-extend 4×I32 lanes and accumulate adjacent pairs into
    /// a 2×I64 accumulator: dest = args[0] + pairwise_sums(args[1]).
    /// Integer addition is associative, so this matches sequential scalar order.
    VecSadalpI32x4,
    /// NEON smlal (low half): dest = args[0] + widen_mul(low2(args[1]), low2(args[2]))
    /// where args[1]/args[2] are 4×I32 vectors and the accumulator is 2×I64.
    VecSmlalLoI32x4,
    /// NEON smlal2 (high half): same as VecSmlalLoI32x4 but for lanes 2-3.
    VecSmlalHiI32x4,

    /// Horizontal reduction: %scalar = horizontal_add(%vec) - AVX2 4×F64 → F64
    /// args[0] = source vector value; dest = scalar F64 result
    VecHorizontalAddF64x4,
    /// Horizontal reduction: %scalar = horizontal_add(%vec) - SSE2 2×F64 → F64
    /// args[0] = source vector value; dest = scalar F64 result
    VecHorizontalAddF64x2,
    /// Horizontal reduction: %scalar = horizontal_add(%vec) - AVX2 8×I32 → I32
    /// args[0] = source vector value; dest = scalar I32 result
    VecHorizontalAddI32x8,
    /// AVX2 lane-wise signed max: %dest_vec = max(%src1_vec, %src2_vec) per
    /// lane, 8×I32 (vpmaxsd). args[0] = src1, args[1] = src2; dest = result.
    /// Signed max is associative, commutative, and idempotent, so lane-wise
    /// reduction matches the scalar max bit-for-bit (same property NEON's
    /// smax relies on for the I32x4 path).
    VecMaxI32x8,
    /// AVX2 horizontal signed max: %scalar = max over all 8 I32 lanes of the
    /// source vector. x86 has no single smaxv; the lowering folds the 8 lanes
    /// with vextracti128 + vpmaxsd + vpshufd + vpmaxsd pairs, ending in vmovd
    /// to the dest GPR. args[0] = source vector value; dest = scalar I32.
    VecHorizontalMaxI32x8,
    /// Horizontal reduction: %scalar = horizontal_add(%vec) - SSE2 4×I32 → I32
    /// args[0] = source vector value; dest = scalar I32 result
    VecHorizontalAddI32x4,
    /// Horizontal reduction: %scalar = horizontal_add(%vec) - AVX2 8×F32 → F32
    VecHorizontalAddF32x8,
    /// Horizontal reduction: %scalar = horizontal_add(%vec) - SSE2 4×F32 → F32
    VecHorizontalAddF32x4,
    VecHorizontalAddI64x2,

    /// Vector zero: %dest_vec = {0.0, 0.0, 0.0, 0.0} - AVX2 4×F64
    /// No args; dest = zero vector
    VecZeroF64x4,
    /// Vector zero: %dest_vec = {0.0, 0.0} - SSE2 2×F64
    /// No args; dest = zero vector
    VecZeroF64x2,
    /// Vector zero: %dest_vec = {0, 0, 0, 0, 0, 0, 0, 0} - AVX2 8×I32
    /// No args; dest = zero vector
    VecZeroI32x8,
    /// Vector zero: %dest_vec = {0, 0, 0, 0} - SSE2 4×I32
    /// No args; dest = zero vector
    VecZeroI32x4,
    /// Vector zero: AVX2 8×F32
    VecZeroF32x8,
    /// Vector zero: SSE2 4×F32
    VecZeroF32x4,
    VecZeroI64x2,
    /// AES-NI: aesenc (single round encrypt)
    /// args[0] = state ptr, args[1] = round key ptr; dest_ptr = result ptr
    Aesenc128,
    /// AES-NI: aesenclast (final round encrypt)
    Aesenclast128,
    /// AES-NI: aesdec (single round decrypt)
    Aesdec128,
    /// AES-NI: aesdeclast (final round decrypt)
    Aesdeclast128,
    /// AES-NI: aesimc (inverse mix columns)
    /// args[0] = input ptr; dest_ptr = result ptr
    Aesimc128,
    /// AES-NI: aeskeygenassist with immediate
    /// args[0] = input ptr, args[1] = imm8; dest_ptr = result ptr
    Aeskeygenassist128,
    /// CLMUL: pclmulqdq with immediate
    /// args[0] = src1 ptr, args[1] = src2 ptr, args[2] = imm8; dest_ptr = result ptr
    Pclmulqdq128,
    /// SSE2 byte shift left (PSLLDQ): shift by imm8 bytes
    /// args[0] = src ptr, args[1] = imm8; dest_ptr = result ptr
    Pslldqi128,
    /// SSE2 byte shift right (PSRLDQ): shift by imm8 bytes
    Psrldqi128,
    /// SSE2 bit shift left per 64-bit lane (PSLLQ)
    /// args[0] = src ptr, args[1] = count; dest_ptr = result ptr
    Psllqi128,
    /// SSE2 bit shift right per 64-bit lane (PSRLQ)
    Psrlqi128,
    /// SSE2 shuffle 32-bit integers (PSHUFD)
    /// args[0] = src ptr, args[1] = imm8; dest_ptr = result ptr
    Pshufd128,
    /// Load low 64 bits, zero upper (MOVQ)
    /// args[0] = src ptr; dest_ptr = result ptr
    Loadldi128,

    // --- SSE2 packed 16-bit integer operations ---
    /// Packed 16-bit add (PADDW)
    Paddw128,
    /// Packed 16-bit subtract (PSUBW)
    Psubw128,
    /// Packed 8-bit add (PADDB)
    Paddb128,
    /// Packed 8-bit subtract (PSUBB)
    Psubb128,
    /// Packed unsigned 16-bit saturating subtract (PSUBUSW)
    Psubusw128,
    /// Sum of absolute differences of unsigned bytes (PSADBW)
    Psadbw128,
    /// Packed low 16-bit multiply (PMULLW)
    Pmullw128,
    /// Packed multiply unsigned/signed bytes, horizontal add pairs (PMADDUBSW, SSSE3)
    Pmaddubsw128,
    /// Packed 16-bit horizontal add (PHADDW, SSSE3)
    Phaddw128,
    /// Packed 32-bit horizontal add (PHADDD, SSSE3)
    Phaddd128,
    /// Packed byte shuffle (PSHUFB, SSSE3)
    Pshufb128,
    /// Packed absolute value bytes (PABSB, SSSE3)
    Pabsb128,
    /// Packed absolute value words (PABSW, SSSE3)
    Pabsw128,
    /// Packed absolute value dwords (PABSD, SSSE3)
    Pabsd128,
    /// Concatenate shift-in bytes (PALIGNR, SSSE3)
    Palignr128,
    /// Packed max unsigned bytes (PMAXUB)
    Pmaxub128,
    /// Packed min unsigned bytes (PMINUB)
    Pminub128,
    /// Packed variable blend bytes (PBLENDVB, SSE4.1)
    Pblendvb128,
    /// Packed blend 16-bit words with immediate (PBLENDW, SSE4.1)
    Pblendw128,
    /// Packed zero-extend 8-bit to 16-bit (PMOVZXBW, SSE4.1)
    Pmovzxbw128,
    /// Packed zero-extend 16-bit to 32-bit (PMOVZXWD, SSE4.1)
    Pmovzxwd128,
    /// Packed 16-bit variable shift left (PSLLW)
    Psllw128,
    /// Packed 16-bit variable shift right (PSRLW)
    Psrlw128,
    /// Packed 16-bit multiply high (PMULHW)
    Pmulhw128,
    /// Packed 16-bit multiply-add to 32-bit (PMADDWD)
    Pmaddwd128,
    /// Packed 16-bit compare greater-than (PCMPGTW)
    Pcmpgtw128,
    /// Packed 8-bit compare greater-than (PCMPGTB)
    Pcmpgtb128,
    /// Packed 16-bit shift left by imm (PSLLW)
    Psllwi128,
    /// Packed 16-bit shift right logical by imm (PSRLW)
    Psrlwi128,
    /// Packed 16-bit shift right arithmetic by imm (PSRAW)
    Psrawi128,
    /// Packed 32-bit shift right arithmetic by imm (PSRAD)
    Psradi128,
    /// Packed 32-bit shift left by imm (PSLLD)
    Pslldi128,
    /// Packed 32-bit shift right logical by imm (PSRLD)
    Psrldi128,

    // --- SSE2 packed 32-bit integer operations ---
    /// Packed 32-bit add (PADDD)
    Paddd128,
    /// Packed 32-bit subtract (PSUBD)
    Psubd128,

    // --- SSE2 pack/unpack operations ---
    /// Pack 32-bit to 16-bit signed saturate (PACKSSDW)
    Packssdw128,
    /// Pack 16-bit to 8-bit signed saturate (PACKSSWB)
    Packsswb128,
    /// Pack 16-bit to 8-bit unsigned saturate (PACKUSWB)
    Packuswb128,
    /// Unpack and interleave low 8-bit (PUNPCKLBW)
    Punpcklbw128,
    /// Unpack and interleave high 8-bit (PUNPCKHBW)
    Punpckhbw128,
    /// Unpack and interleave low 16-bit (PUNPCKLWD)
    Punpcklwd128,
    /// Unpack and interleave high 16-bit (PUNPCKHWD)
    Punpckhwd128,

    // --- SSE2 set/insert/extract/convert operations ---
    /// Set all 16-bit lanes to value (splat)
    SetEpi16,
    /// Insert 16-bit value at lane (PINSRW)
    Pinsrw128,
    /// Extract 16-bit value at lane (PEXTRW) - returns scalar i32
    Pextrw128,
    /// Store low 64 bits to memory (MOVQ store)
    Storeldi128,
    /// Convert low 32-bit of __m128i to int (MOVD) - returns scalar i32
    Cvtsi128Si32,
    /// Convert int to __m128i with zero extension (MOVD)
    Cvtsi32Si128,
    /// Convert low 64-bit of __m128i to long long - returns scalar i64
    Cvtsi128Si64,
    /// Shuffle low 16-bit integers (PSHUFLW)
    Pshuflw128,
    /// Shuffle high 16-bit integers (PSHUFHW)
    Pshufhw128,

    // --- SSE4.1 insert/extract operations ---
    /// Insert 32-bit value at lane (PINSRD)
    Pinsrd128,
    /// Extract 32-bit value at lane (PEXTRD) - returns scalar i32
    Pextrd128,
    /// Insert 8-bit value at lane (PINSRB)
    Pinsrb128,
    /// Extract 8-bit value at lane (PEXTRB) - returns scalar i32
    Pextrb128,
    /// Insert 64-bit value at lane (PINSRQ)
    Pinsrq128,
    /// Extract 64-bit value at lane (PEXTRQ) - returns scalar i64
    Pextrq128,

    // --- AVX2 256-bit integer operations (lowered to v* ymm instructions) ---
    Loadu256,
    Storeu256,
    Load256,
    Store256,
    Paddb256,
    Paddw256,
    Paddd256,
    Psubb256,
    Psubw256,
    Psubusw256,
    Psadbw256,
    Pmaddubsw256,
    Pmaddwd256,
    Pcmpeqb256,
    Pcmpgtb256,
    Pmovmskb256,
    Pshufb256,
    Pabsb256,
    Pabsw256,
    Pmaxub256,
    Pminub256,
    Pxor256,
    Por256,
    Pand256,
    Psllidi256,
    Psrlidi256,
    Psllwi256,
    Psrlwi256,
    Broadcast128to256,
    Zext128to256,
    Cast256to128,
    Insert128to256,
    SetEpi16_256,
    SetEpi32_256,
    SetEpi64x256,
    SetEpi8_256,
    /// VPERMD: permute 8 x i32 by variable index (chunkset_avx2)
    Permutevar8x32,

    // --- AVX-VNNI (VEX, Raptor Lake+) ---
    Dpbusd128,
    Dpbusds128,
    Dpwusd128,
    Dpwusds128,
    Dpbusd256,
    Dpbusds256,
    Dpwusd256,
    Dpwusds256,
    // --- AVX-VNNI-INT8 ---
    Dpbssd128,
    Dpbssds128,
    Dpbsud128,
    Dpbsuds128,
    Dpbuud128,
    Dpbuuds128,
    Dpbssd256,
    Dpbssds256,
    Dpbsud256,
    Dpbsuds256,
    Dpbuud256,
    Dpbuuds256,
    // --- AVX-VNNI-INT16 (vpdpwusd/s shared with AVX-VNNI) ---
    Dpwuud128,
    Dpwuuds128,
    Dpwssd128,
    Dpwssds128,
    Dpwuud256,
    Dpwuuds256,
    Dpwssd256,
    Dpwssds256,
    // --- GFNI ---
    Gf2p8mulb128,
    Gf2p8affineqb128,
    Gf2p8affineinvqb128,
    // --- VAES 256-bit + VPCLMULQDQ 256-bit ---
    Aesenc256,
    Aesenclast256,
    Aesdec256,
    Aesdeclast256,
    Vpclmulqdq256,

    // --- SSE2 ops previously left as scalar header loops ---
    Paddusb128,
    Paddsb128,
    Paddusw128,
    Paddsw128,
    Psubsw128,
    Pandn128,
    Pcmpeqw128,
    Pcmpgtd128,
    Pavgb128,
    Pavgw128,
    Pminsw128,
    Pmaxsw128,
    Pmulhuw128,
    Paddq128,
    Psubq128,
    Punpckldq128,
    Punpckhdq128,
    Punpcklqdq128,
    Punpckhqdq128,
    Setzero128,
    Testz128,

    // --- AVX / AVX2 ops previously left as scalar header loops ---
    Pmulld256,
    Psubd256,
    Paddq256,
    Psubq256,
    Pandn256,
    Pcmpeqd256,
    Pcmpeqq256,
    Pcmpgtd256,
    Pcmpgtq256,
    Extracti128,
    Setzero256,
    AddPs256,
    SubPs256,
    MulPs256,
    AddPd256,
    SubPd256,
    MulPd256,
    LoaduPs256,
    StoreuPs256,
    LoaduPd256,
    StoreuPd256,
    Permute2x128,
    Permute4x64,
    Pshufd256,
    Punpcklbw256,
    Punpckhbw256,
    Punpcklwd256,
    Punpckhwd256,
    Punpckldq256,
    Punpckhdq256,
    Punpcklqdq256,
    Punpckhqdq256,
    Pslldqi256,
    Psrldqi256,
    Psllqi256,
    Psrlqi256,
    Pmullw256,
    Pmulhw256,
    Pminsd256,
    Pmaxsd256,
    Pmovzxbw256,
    Pmovzxbd256,
    Pmovzxwd256,
    Pmovsxbw256,
    Pmovsxbd256,
    Pmovsxwd256,
    Psrawi256,
    Psradi256,
    Packssdw256,
    Packuswb256,
    Phaddw256,
    Phaddd256,
    Pabsd256,
    Pmuludq256,
    // === AVX-512 (512-bit) packed integer ops ===
    Paddb512,
    Paddw512,
    Paddd512,
    Paddq512,
    Psubb512,
    Psubw512,
    Psubd512,
    Psubq512,
    Paddsb512,
    Paddsw512,
    Paddusb512,
    Paddusw512,
    Psubsb512,
    Psubsw512,
    Psubusb512,
    Psubusw512,
    Pavgb512,
    Pavgw512,
    Pmaxub512,
    Pminub512,
    Pmaxuw512,
    Pminuw512,
    Pmaxsb512,
    Pminsb512,
    Pmaxsw512,
    Pminsw512,
    Pmaxsd512,
    Pminsd512,
    Pmaxud512,
    Pminud512,
    Pmaxsq512,
    Pminsq512,
    Pmaxuq512,
    Pminuq512,
    Pcmpeqd512,
    Pcmpeqq512,
    Pcmpgtb512,
    Pcmpgtw512,
    Pcmpgtd512,
    Pcmpgtq512,
    Psadbw512,
    Pmaddubsw512,
    Pmaddwd512,
    Pmullw512,
    Pmulhw512,
    Pmulhuw512,
    Pmulld512,
    Pmuludq512,
    Pxor512,
    Por512,
    Pand512,
    Pandn512,
    Pshufb512,
    Pabsb512,
    Pabsw512,
    Pabsd512,
    Pabsq512,
    Punpcklbw512,
    Punpcklwd512,
    Punpckldq512,
    Punpcklqdq512,
    Punpckhbw512,
    Punpckhwd512,
    Punpckhdq512,
    Punpckhqdq512,
    Packsswb512,
    Packuswb512,
    Packssdw512,
    Packusdw512,
    Pshufd512,
    Pshuflw512,
    Pshufhw512,
    Psllwi512,
    Psrlwi512,
    Psrawi512,
    Psllidi512,
    Psrlidi512,
    Psradi512,
    Psllqi512,
    Psrlqi512,
    Psraqi512,
    Palignr512,
    Pmovzxbw512,
    Pmovzxbd512,
    Pmovzxbq512,
    Pmovzxwd512,
    Pmovzxwq512,
    Pmovzxdq512,
    Pmovsxbw512,
    Pmovsxbd512,
    Pmovsxbq512,
    Pmovsxwd512,
    Pmovsxwq512,
    Pmovsxdq512,
    Popcntb512,
    Popcntw512,
    Popcntd512,
    Popcntq512,
    TernaryLogic128,
    TernaryLogic256,
    TernaryLogic512,
    // 512-bit insert/extract/permute
    InsertI32x4,
    InsertI64x2,
    InsertI32x8,
    InsertI64x4,
    ExtractI32x4,
    ExtractI64x2,
    ExtractI32x8,
    ExtractI64x4,
    PermutexvarEp32,
    PermutexvarEp64,
    // 512-bit broadcast/set/cast
    BroadcastI32x4,
    BroadcastI64x2,
    BroadcastI32x8,
    BroadcastI64x4,
    SetEpi8_512,
    SetEpi16_512,
    SetEpi32_512,
    SetEpi64x512,
    Zext128to512,
    Cast512to256,
    Cast128to512,
    // === AVX-512 mask ops (mask = scalar u64 in IR) ===
    CmpeqEpu8Mask128,
    CmpeqEpu8Mask256,
    CmpeqEpu8Mask512,
    CmpeqEpu16Mask512,
    CmpeqEpu32Mask512,
    CmpeqEpu64Mask512,
    CmpEpi8Mask128,
    CmpEpi8Mask256,
    CmpEpi8Mask512,
    CmpEpu8Mask128,
    CmpEpu8Mask256,
    CmpEpu8Mask512,
    CmpEpi32Mask128,
    CmpEpi32Mask256,
    CmpEpi32Mask512,
    CmpEpu32Mask128,
    CmpEpu32Mask256,
    CmpEpu32Mask512,
    CmpEpi64Mask128,
    CmpEpi64Mask256,
    CmpEpi64Mask512,
    CmpEpu64Mask128,
    CmpEpu64Mask256,
    CmpEpu64Mask512,
    CmpEpi16Mask128,
    CmpEpi16Mask256,
    CmpEpi16Mask512,
    CmpEpu16Mask128,
    CmpEpu16Mask256,
    CmpEpu16Mask512,
    Loadu512,
    Storeu512,
    MaskzLoaduEpi8_128,
    MaskzLoaduEpi8_256,
    MaskzLoaduEpi8_512,
    MaskzLoaduEpi32_512,
    MaskzLoaduEpi64_512,
    MaskLoaduEpi8_128,
    MaskLoaduEpi8_256,
    MaskLoaduEpi8_512,
    MaskStoreuEpi8_128,
    MaskStoreuEpi8_256,
    MaskStoreuEpi8_512,
    MaskzMaddubsEpi16_512,
    MaskzSet1Epi16_512,
    MaskzSet1Epi32_512,
    MaskzSet1Epi64x512,
    MaskzInsertI64x2,
    MaskzInsertI32x4,
    MaskzExtractI32x4,
    MaskzExtractI64x4,
    MaskzExtractI64x2,
    MaskzShuffleEpi8_128,
    MaskShuffleEpi8_128,
    ReduceAddEpu32_512,
    Vpdpbusd512,
    Vpdpbusds512,
    Vpclmulqdq512,
    // === 128-bit FP ops (SSE) — complements existing AddPs128/AddPd128 etc. ===
    DivPs128,
    MinPs128,
    MaxPs128,
    SqrtPs128,
    RcpPs128,
    RsqrtPs128,
    CmpPs128,
    ShufPs128,
    UnpcklPs128,
    UnpckhPs128,
    MovemaskPs128,
    CvtPs2Ep32_128,
    CvtEp32ToPs128,
    CvttPs2Ep32_128,
    CvtPs2Pd128,
    CvtPd2Ps128,
    DivPd128,
    MinPd128,
    MaxPd128,
    SqrtPd128,
    CmpPd128,
    ShufPd128,
    UnpcklPd128,
    UnpckhPd128,
    MovemaskPd128,
    CvtPd2Ep32_128,
    CvtEp32ToPd128,
    CvttPd2Ep32_128,
    CvtSs2Si128,
    CvtSi2Ss128,
    CvtSs2Sd128,
    CvtSd2Ss128,
    CvtSi2Sd128,
    CvtSi2Ss64_128,
    CvtSi2Sd64_128,
    CvtSd2Si128,
    Movss128,
    Movsd128,
    // === 256-bit FP ops (AVX) ===
    DivPs256,
    MinPs256,
    MaxPs256,
    SqrtPs256,
    CmpPs256,
    ShufPs256,
    UnpcklPs256,
    UnpckhPs256,
    MovemaskPs256,
    CvtPs2Ep32_256,
    CvtEp32ToPs256,
    CvttPs2Ep32_256,
    CvtPs2Pd256,
    CvtPd2Ps256,
    DivPd256,
    MinPd256,
    MaxPd256,
    SqrtPd256,
    CmpPd256,
    ShufPd256,
    UnpcklPd256,
    UnpckhPd256,
    MovemaskPd256,
    CvtPd2Ep32_256,
    CvtEp32ToPd256,
    CvttPd2Ep32_256,
    VpermilPs128,
    VpermilPs256,
    /// 256-bit variable-index permute: `vpermilps %idx, %src, %dst`.
    VpermilvarPs256,
    /// 256-bit variable-index permute (double): `vpermilpd %idx, %src, %dst`.
    VpermilvarPd256,
    Vperm2f128,
    Vinsertf128,
    Vextractf128,
    Vbroadcastss,
    Vbroadcastsd,
    TestzPs128,
    TestzPs256,
    RoundPs128,
    RoundPs256,
    RoundPd128,
    RoundPd256,
    BlendPs128,
    BlendPd128,
    BlendPs256,
    BlendPd256,
    BlendvPs128,
    BlendvPd128,
    BlendvPs256,
    BlendvPd256,
    DpPs128,
    DpPd128,
    InsertPs128,
    ExtractPs128,
    InsertPd128,
    ExtractPd128,
    HaddPs128,
    HsubPs128,
    AddsubPs128,
    HaddPd128,
    HsubPd128,
    AddsubPd128,
    HaddPs256,
    HsubPs256,
    AddsubPs256,
    Movddup128,
    Movsldup128,
    Movshdup128,
    FmaSs,
    FmaSd,
    FmaPs132,
    FmaPs213,
    FmaPs231,
    FmaPd132,
    FmaPd213,
    FmaPd231,
    FmaPs132v256,
    FmaPs213v256,
    FmaPs231v256,
    FmaPd132v256,
    FmaPd213v256,
    FmaPd231v256,
    // === 512-bit FP ===
    AddPs512,
    SubPs512,
    MulPs512,
    DivPs512,
    MinPs512,
    MaxPs512,
    AddPd512,
    SubPd512,
    MulPd512,
    DivPd512,
    MinPd512,
    MaxPd512,
    SqrtPs512,
    SqrtPd512,
    CmpPs512,
    CmpPd512,
    CvtPs2Pd512,
    CvtPd2Ps512,
    CvtEp32_2Ps512,
    CvtPs2Ep32_512,
    CvttPs2Ep32_512,
    CvtEp32_2Pd512,
    CvtPd2Ep32_512,
    CvttPd2Ep32_512,
    FmaPs132v512,
    FmaPs213v512,
    FmaPs231v512,
    FmaPd132v512,
    FmaPd213v512,
    FmaPd231v512,
}

impl IntrinsicOp {
    /// Width in bytes of the vector/XMM result produced by this intrinsic:
    /// 128-bit SSE results → Some(16), 256-bit AVX/AVX2 results → Some(32).
    /// Returns None for ops whose `dest` is scalar (GPR/x87), that produce no
    /// value (fences, stores, control), or that are handled by the F128 path.
    ///
    /// NOTE: ops whose result is a SCALAR must never be listed here even if
    /// their name sounds vector-ish (HorizontalAdd*/VecHorizontalAdd* return
    /// I32/F64 scalars; Pmovmskb*/Cvtsi128Si*/Pextr*/Crc32* are scalar too).
    /// Misclassifying them as vectors corrupts scalar values (volatile_access
    /// regression: sum reloaded via leaq instead of movq).
    ///
    /// This is the single authority for "does this intrinsic write a vector
    /// home slot" — used by the stack-layout pre-scan to protect user-level
    /// SSE/AVX intrinsic results from block-local slot reuse. Previously only
    /// the auto-vectorizer's internal Vec* ops were recognized, so real code
    /// (zlib-ng adler32_ssse3, _mm256_mul_ps chains, ...) got 8-byte slots
    /// with 16/32-byte stores overflowing into neighbours and reusable slots
    /// being read back after corruption (vector_defer_multidef_slot and
    /// simd_avx2_256 regressions).
    pub fn vector_result_width(&self) -> Option<u32> {
        use IntrinsicOp::*;
        match self {
            // ---- 256-bit AVX/AVX2 results ----
            Loadu256 | Load256
            | Paddb256 | Paddw256 | Paddd256 | Psubb256 | Psubw256 | Psubusw256
            | Psadbw256 | Pmaddubsw256 | Pmaddwd256 | Pcmpeqb256 | Pcmpgtb256
            | Pshufb256 | Pabsb256 | Pabsw256 | Pmaxub256
            | Pminub256 | Pxor256 | Por256 | Pand256 | Psllidi256 | Psrlidi256
            | Psllwi256 | Psrlwi256 | Broadcast128to256 | Zext128to256
            | Insert128to256 | SetEpi16_256 | SetEpi32_256 | SetEpi64x256 | SetEpi8_256
            | Dpbusd256 | Dpbssd256 | Dpwuud256 | Aesenc256 | Vpclmulqdq256
            | FmaF64x4 | FmaF64x4Hoisted | BroadcastLoadF64 | FmaF64x4SIB | FmaF64x4HoistedSIB
            | LoadF64x4 | LoadI32x8 | AddF64x4 | MulF64x4 | AddI32x8
            | VecLoadF64x4 | VecLoadI32x8 | VecAddF64x4 | VecMulF64x4 | VecFmaF64x4 | VecMaddF64x4 | VecBroadcastF64x4 | VecAddI32x8 | VecMulI32x8 | VecBroadcastI32x8 | VecMaxI32x8
            | VecSubF64x4 | VecDivF64x4 | VecSqrtF64x4
            | VecSubF32x8 | VecDivF32x8 | VecSqrtF32x8
            | VecZeroF64x4 | VecZeroI32x8 | VecLoadF32x8 | VecAddF32x8
            | VecMulF32x8 | VecFmaF32x8 | VecMaddF32x8
            | VecBroadcastF32x8 | VecZeroF32x8
            | VecMinF32x8 | VecMaxF32x8 | VecCmpF32x8 | VecBlendvF32x8
            | VecMinF64x4 | VecMaxF64x4 | VecCmpF64x4 | VecBlendvF64x4
            // Newly wired AVX/AVX2 ops (previously scalar header loops)
            | Pmulld256 | Psubd256 | Paddq256 | Psubq256 | Pandn256
            | Pcmpeqd256 | Pcmpeqq256 | Pcmpgtd256 | Pcmpgtq256
            | Setzero256 | AddPs256 | SubPs256 | MulPs256 | AddPd256
            | SubPd256 | MulPd256 | LoaduPs256 | StoreuPs256 | LoaduPd256
            | StoreuPd256 | Permute2x128 | Permute4x64 | Permutevar8x32 | Pshufd256
            | Punpcklbw256 | Punpckhbw256 | Punpcklwd256 | Punpckhwd256
            | Punpckldq256 | Punpckhdq256 | Punpcklqdq256 | Punpckhqdq256
            | Pslldqi256 | Psrldqi256 | Psllqi256 | Psrlqi256
            | Pmullw256 | Pmulhw256 | Pminsd256 | Pmaxsd256
            | Pmovzxbw256 | Pmovzxbd256 | Pmovzxwd256
            | Pmovsxbw256 | Pmovsxbd256 | Pmovsxwd256
            | Psrawi256 | Psradi256 | Packssdw256 | Packuswb256
            | Phaddw256 | Phaddd256 | Pabsd256 | Pmuludq256 => Some(32),
            // ---- 128-bit SSE/SSE2/SSSE3/SSE4 results ----
            Loaddqu | Loadldi128 | Pcmpeqb128 | Pcmpeqd128 | Psubusb128
            | Psubsb128 | Por128 | Pand128 | Pxor128 | AddPs128 | SubPs128
            | MulPs128 | AddPd128 | SubPd128 | MulPd128 | Pmuludq128
            | Pmuldq128 | Pmulld128 | CastReinterpret128 | SetEpi8 | SetEpi16
            | SetEpi32 | Pslldqi128 | Psrldqi128 | Psllqi128 | Psrlqi128
            | Pshufd128 | Paddw128 | Psubw128 | Paddb128 | Psubb128
            | Psubusw128 | Psadbw128 | Pmullw128 | Pmaddubsw128 | Phaddw128
            | Phaddd128 | Pshufb128 | Pabsb128 | Pabsw128 | Pabsd128
            | Palignr128 | Pmaxub128 | Pminub128 | Pblendvb128 | Pblendw128 | Pmovzxbw128
            | Pmovzxwd128 | Psllw128 | Psrlw128 | Pmulhw128 | Pmaddwd128
            | Pcmpgtw128 | Pcmpgtb128 | Psllwi128 | Psrlwi128 | Psrawi128
            | Psradi128 | Pslldi128 | Psrldi128 | Paddd128 | Psubd128
            | Packssdw128 | Packsswb128 | Packuswb128 | Punpcklbw128
            | Punpckhbw128 | Punpcklwd128 | Punpckhwd128 | Pinsrw128
            | Cvtsi32Si128 | Pshuflw128 | Pshufhw128 | Pinsrd128 | Pinsrb128
            | Pinsrq128 | Cast256to128 | Dpbusd128 | Dpbssd128 | Dpwuud128
            | Gf2p8mulb128 | Aesenc128 | Aesenclast128 | Aesdec128
            | Aesdeclast128 | Aesimc128 | Aeskeygenassist128 | Pclmulqdq128
            | FmaF64x2 | LoadF64x2 | LoadI32x4 | AddF64x2 | MulF64x2
            | AddI32x4 | VecLoadF64x2 | VecLoadI32x4 | VecAddF64x2
            | VecMulF64x2 | VecBroadcastF64x2 | VecAddI32x4 | VecZeroF64x2
            | VecSubF64x2 | VecDivF64x2 | VecSqrtF64x2
            | VecZeroI32x4 | VecLoadF32x4 | VecAddF32x4 | VecMulF32x4 | VecBroadcastF32x4 | VecZeroF32x4
            | VecSubF32x4 | VecDivF32x4 | VecSqrtF32x4
            | VecMinF32x4 | VecMaxF32x4 | VecCmpF32x4 | VecBlendvF32x4
            | VecMinF64x2 | VecMaxF64x2 | VecCmpF64x2 | VecBlendvF64x2
            | VecWidenAddI32x4ToI64x2
            | VecWidenMaskedAddI32x4ToI64x2
            | VecMaskedAddI32x8
            | VecLoadWidenI32ToI64x2 | VecLoadI64x2 | VecAddI64x2 | VecMulI64x2 | VecStoreI64x2 | VecBroadcastI64x2 | VecZeroI64x2 | VecLoadI64x4 | VecAddI64x4 | VecHorizontalAddI64x4 | VecZeroI64x4
            | VecMulI32x4 | VecBroadcastI32x4 | VecSmaxI32x4
            | Paddusb128 | Paddsb128 | Paddusw128 | Paddsw128 | Psubsw128
            | Pandn128 | Pcmpeqw128 | Pcmpgtd128 | Pavgb128 | Pavgw128
            | Pminsw128 | Pmaxsw128 | Pmulhuw128 | Paddq128 | Psubq128
            | Punpckldq128 | Punpckhdq128 | Punpcklqdq128 | Punpckhqdq128
            | Setzero128 | Extracti128
            => Some(16),
            // Everything else produces a scalar GPR/x87 result, no result, or
            // is an F128 helper handled by the dedicated f128 slot path.
            _ => None,
        }
    }

    /// Returns true if this intrinsic is a pure function (no side effects, result depends
    /// only on inputs). Pure intrinsics can be dead-code eliminated if their result is unused.
    pub fn is_pure(&self) -> bool {
        matches!(
            self,
            IntrinsicOp::SqrtF32 | IntrinsicOp::SqrtF64 |
            IntrinsicOp::FabsF32 | IntrinsicOp::FabsF64 |
            IntrinsicOp::FmaScalarF32 | IntrinsicOp::FmaScalarF64 |
            IntrinsicOp::RoundScalarF32(_) | IntrinsicOp::RoundScalarF64(_) |
            IntrinsicOp::CopysignF32 | IntrinsicOp::CopysignF64 |
            IntrinsicOp::F128Fabs | IntrinsicOp::F128Neg | IntrinsicOp::F128Copysign |
            IntrinsicOp::VecWidenAddI32x4ToI64x2
            | IntrinsicOp::VecWidenMaskedAddI32x4ToI64x2
            | IntrinsicOp::VecMaskedAddI32x8 |
            IntrinsicOp::LDFabs | IntrinsicOp::LDCopysign |
            IntrinsicOp::Aesenc128 | IntrinsicOp::Aesenclast128 |
            IntrinsicOp::Aesdec128 | IntrinsicOp::Aesdeclast128 |
            IntrinsicOp::Aesimc128 | IntrinsicOp::Aeskeygenassist128 |
            IntrinsicOp::Pclmulqdq128 |
            IntrinsicOp::Pslldqi128 | IntrinsicOp::Psrldqi128 |
            IntrinsicOp::Psllqi128 | IntrinsicOp::Psrlqi128 |
            IntrinsicOp::Pshufd128 |
            // SSE2 packed operations are all pure
            IntrinsicOp::Psubsb128 |
            IntrinsicOp::Paddw128 | IntrinsicOp::Psubw128 |
            IntrinsicOp::Pmulhw128 | IntrinsicOp::Pmaddwd128 |
            IntrinsicOp::Pcmpgtw128 | IntrinsicOp::Pcmpgtb128 |
            IntrinsicOp::Psllwi128 | IntrinsicOp::Psrlwi128 |
            IntrinsicOp::Psrawi128 | IntrinsicOp::Psradi128 |
            IntrinsicOp::Pslldi128 | IntrinsicOp::Psrldi128 |
            IntrinsicOp::Paddd128 | IntrinsicOp::Psubd128 |
            IntrinsicOp::Packssdw128 | IntrinsicOp::Packsswb128 | IntrinsicOp::Packuswb128 |
            IntrinsicOp::Punpcklbw128 | IntrinsicOp::Punpckhbw128 |
            IntrinsicOp::Punpcklwd128 | IntrinsicOp::Punpckhwd128 |
            IntrinsicOp::SetEpi16 | IntrinsicOp::Pinsrw128 |
            IntrinsicOp::Pextrw128 | IntrinsicOp::Cvtsi128Si32 |
            IntrinsicOp::Cvtsi32Si128 | IntrinsicOp::Cvtsi128Si64 |
            IntrinsicOp::Pshuflw128 | IntrinsicOp::Pshufhw128 |
            // SSE4.1 insert/extract are pure
            IntrinsicOp::Pinsrd128 | IntrinsicOp::Pextrd128 |
            IntrinsicOp::Pinsrb128 | IntrinsicOp::Pextrb128 |
            IntrinsicOp::Pinsrq128 | IntrinsicOp::Pextrq128 |
            // Vector reduction operations are pure
            IntrinsicOp::LoadF64x4 | IntrinsicOp::LoadF64x2 |
            IntrinsicOp::LoadI32x8 | IntrinsicOp::LoadI32x4 |
            IntrinsicOp::AddF64x4 | IntrinsicOp::AddF64x2 |
            IntrinsicOp::MulF64x4 | IntrinsicOp::MulF64x2 |
            IntrinsicOp::AddI32x8 | IntrinsicOp::AddI32x4 |
            IntrinsicOp::HorizontalAddF64x4 | IntrinsicOp::HorizontalAddF64x2 |
            IntrinsicOp::HorizontalAddI32x8 | IntrinsicOp::HorizontalAddI32x4
            | IntrinsicOp::VecMaxI32x8
            | IntrinsicOp::VecHorizontalMaxI32x8
        ) ||
        // The modern vectorizer's value-producing families (splats, zeros,
        // non-volatile vector loads, packed arithmetic) are pure: they read
        // no state beyond their operands. Without this, an orphaned
        // VecBroadcast left behind by a vectorizer bailout was DCE-rooted
        // as "side-effecting", reached codegen with no register or slot
        // home, and ICEd the intrinsic emitter (`gd[i] = gd[i]*k + c` over
        // a global array). VecStoreI64x2 is the one memory-writing op that
        // produces_vector_value() also lists (kept there for slot sizing);
        // it is excluded here -- stores are never pure.
        (self.produces_vector_value() && !matches!(self, IntrinsicOp::VecStoreI64x2))
    }

    /// Returns true if this intrinsic produces a 128/256-bit vector value
    /// (as opposed to a scalar).  These values cannot live in a GPR; backends
    /// either home them on the stack or allocate SIMD registers for them.
    pub fn produces_vector_value(&self) -> bool {
        matches!(
            self,
            IntrinsicOp::VecZeroF64x4
                | IntrinsicOp::VecZeroF64x2
                | IntrinsicOp::VecZeroI32x8
                | IntrinsicOp::VecZeroI32x4
                | IntrinsicOp::VecZeroF32x8
                | IntrinsicOp::VecZeroF32x4
                | IntrinsicOp::VecLoadF64x4
                | IntrinsicOp::VecLoadF64x2
                | IntrinsicOp::VecLoadI32x8
                | IntrinsicOp::VecLoadI32x4
                | IntrinsicOp::VecLoadF32x8
                | IntrinsicOp::VecLoadF32x4
                | IntrinsicOp::VecAddF64x4
                | IntrinsicOp::VecAddF64x2
                | IntrinsicOp::VecAddI32x8
                | IntrinsicOp::VecMaxI32x8
                | IntrinsicOp::VecAddI32x4
                | IntrinsicOp::VecAddF32x8
                | IntrinsicOp::VecAddF32x4
                | IntrinsicOp::VecMulF64x4
                | IntrinsicOp::VecMulF64x2
                | IntrinsicOp::VecSubF64x4
                | IntrinsicOp::VecSubF64x2
                | IntrinsicOp::VecDivF64x4
                | IntrinsicOp::VecDivF64x2
                | IntrinsicOp::VecSqrtF64x4
                | IntrinsicOp::VecSqrtF64x2
                | IntrinsicOp::VecBroadcastF64x4
                | IntrinsicOp::VecBroadcastF64x2
                | IntrinsicOp::VecMulF32x8
                | IntrinsicOp::VecMulF32x4
                | IntrinsicOp::VecSubF32x8
                | IntrinsicOp::VecSubF32x4
                | IntrinsicOp::VecDivF32x8
                | IntrinsicOp::VecDivF32x4
                | IntrinsicOp::VecSqrtF32x8
                | IntrinsicOp::VecSqrtF32x4
                | IntrinsicOp::VecWidenAddI32x4ToI64x2
                | IntrinsicOp::VecWidenMaskedAddI32x4ToI64x2
                | IntrinsicOp::VecMaskedAddI32x8
                | IntrinsicOp::VecBroadcastF32x8
                | IntrinsicOp::VecBroadcastF32x4
                | IntrinsicOp::VecFmaF64x4
                | IntrinsicOp::VecFmaF32x8
                | IntrinsicOp::VecMaddF64x4
                | IntrinsicOp::VecMaddF32x8
                | IntrinsicOp::VecLoadWidenI32ToI64x2
                | IntrinsicOp::VecLoadI64x2
                | IntrinsicOp::VecAddI64x2
                | IntrinsicOp::VecMulI64x2
                | IntrinsicOp::VecStoreI64x2
                | IntrinsicOp::VecBroadcastI64x2
                | IntrinsicOp::VecZeroI64x2
                | IntrinsicOp::VecLoadI64x4
                | IntrinsicOp::VecAddI64x4
                | IntrinsicOp::VecHorizontalAddI64x4
                | IntrinsicOp::VecZeroI64x4
                | IntrinsicOp::VecMulI32x4
                | IntrinsicOp::VecMulI32x8
                | IntrinsicOp::VecBroadcastI32x4
                | IntrinsicOp::VecBroadcastI32x8
                | IntrinsicOp::VecSadalpI32x4
                | IntrinsicOp::VecSmlalLoI32x4
                | IntrinsicOp::VecSmlalHiI32x4
                | IntrinsicOp::VecSmaxI32x4
                | IntrinsicOp::VecMinF32x8
                | IntrinsicOp::VecMinF32x4
                | IntrinsicOp::VecMinF64x4
                | IntrinsicOp::VecMinF64x2
                | IntrinsicOp::VecMaxF32x8
                | IntrinsicOp::VecMaxF32x4
                | IntrinsicOp::VecMaxF64x4
                | IntrinsicOp::VecMaxF64x2
                | IntrinsicOp::VecCmpF32x8
                | IntrinsicOp::VecCmpF32x4
                | IntrinsicOp::VecCmpF64x4
                | IntrinsicOp::VecCmpF64x2
                | IntrinsicOp::VecBlendvF32x8
                | IntrinsicOp::VecBlendvF32x4
                | IntrinsicOp::VecBlendvF64x4
                | IntrinsicOp::VecBlendvF64x2
        )
    }
}
