//! AVX2/SSE2 vectorization pass for matmul-style loops.
//!
//! Recognizes innermost loops with stride-1 double-precision accumulation patterns
//! and transforms them into FmaF64x4 (AVX2, default) or FmaF64x2 (SSE2) intrinsics.
//!
//! Target pattern (matmul j-loop):
//! ```c
//! for (int j = 0; j < N; j++)
//!     C[i][j] += A[i][k] * B[k][j];
//! ```
//!
//! ## AVX2 Transformation (default, 4-wide):
//! ```c
//! for (int j = 0; j < N/4; j++)  // Loop N/4 times
//!     FmaF64x4(&C[i][j*4], &A[i][k], &B[k][j*4]);  // Process 4 elements per iteration
//! ```
//!
//! ## SSE2 Transformation (with LCCC_FORCE_SSE2=1, 2-wide):
//! ```c
//! for (int j = 0; j < N/2; j++)  // Loop N/2 times
//!     FmaF64x2(&C[i][j*2], &A[i][k], &B[k][j*2]);  // Process 2 elements per iteration
//! ```
//!
//! ## Transformation Details
//!
//! ### AVX2 (4-wide, default):
//! 1. **Loop Bound**: Modified from `j < N` to `j < N/4`
//!    - For constant N: divide by 4 at compile time
//!    - For dynamic N: insert `udiv` instruction to compute N/4
//!    - Modifies ALL comparisons involving IV-derived values in the loop
//!
//! 2. **Array Indexing**: Changed from `j` to `j*4`
//!    - Inserts multiply instructions before GEPs: `offset' = offset * 4`
//!    - Ensures iteration j accesses elements [j*4..j*7] instead of [j, j+1]
//!    - Backend generates stride-32 addressing (4 doubles × 8 bytes)
//!
//! 3. **Induction Variable**: Keeps incrementing by 1
//!    - Backend-friendly: `j++` instead of `j += 4`
//!    - Combined with 4× offset, produces correct element access
//!
//! 4. **AVX2 Code Generation**:
//!    - `vbroadcastsd`: broadcast A[i][k] scalar to 4 lanes
//!    - `vmovupd`: load 4 doubles from B[k][j*4]
//!    - `vmulpd`: packed multiply (4 doubles)
//!    - `vaddpd`: packed add with C[i][j*4]
//!    - `vmovupd`: store 4 results back
//!
//! ### SSE2 (2-wide, with LCCC_FORCE_SSE2=1):
//! 1. **Loop Bound**: Modified from `j < N` to `j < N/2`
//! 2. **Array Indexing**: Changed from `j` to `j*2` (stride-16)
//! 3. **SSE2 Code Generation**:
//!    - `movsd` + `unpcklpd`: broadcast A[i][k] scalar
//!    - `movupd`: load 2 doubles from B[k][j*2]
//!    - `mulpd`: packed multiply (2 doubles)
//!    - `addpd`: packed add with C[i][j*2]
//!    - `movupd`: store 2 results back
//!
//! ## Remainder Loops
//!
//! Remainder loops are automatically inserted to handle cases where N is not divisible by the vector width:
//! - AVX2 (4-wide): Handles N % 4 ∈ {1, 2, 3} with scalar remainder loop
//! - SSE2 (2-wide): Handles N % 2 = 1 with scalar remainder loop
//!
//! Example for N=255 with AVX2:
//! - Vectorized loop: 63 iterations processing indices [0..251] (4 elements each)
//! - Remainder loop: 3 iterations processing indices [252, 253, 254] (scalar)
//!
//! ## Limitations
//!
//! - Only handles matmul-style patterns (load, multiply, add, store)
//! - Requires innermost loop with IV-based indexing
//!
//! ## Environment Variables
//!
//! - `LCCC_FORCE_SSE2=1`: Force SSE2 2-wide vectorization instead of AVX2 4-wide
//! - `LCCC_FORCE_AVX2=1`: Explicitly enable AVX2 (default behavior, provided for clarity)
//! - `LCCC_DEBUG_VECTORIZE=1`: Enable debug output for vectorization pass

use crate::common::fp_contract::FpContract;
use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::common::types::{AddressSpace, IrType};
use crate::ir::analysis::CfgAnalysis;
use crate::ir::instruction::{BasicBlock, BlockId, Instruction, Operand, Terminator, Value};
use crate::ir::intrinsics::IntrinsicOp;
use crate::ir::ops::{IrBinOp, IrCmpOp, IrUnaryOp};
use crate::ir::reexports::{IrConst, IrFunction};
use crate::passes::loop_analysis;

// Per-loop rejection reason for the "why was this not vectorized" diagnostic
// (project goal §57). The analysis functions record the most specific reason
// they bail out with; `vectorize_with_analysis` prints it when either
// `LCCC_DEBUG_VECTORIZE=1` (full trace) or `LCCC_WHY_NOT_VECTORIZE=1`
// (one-line-per-loop summary) is set. Purely diagnostic — never changes
// codegen.
//
// A `///` doc comment cannot attach to a `thread_local!` invocation (the macro
// expands to items the comment never reaches), so this is a plain comment.
thread_local! {
    static REJECT_REASON: std::cell::RefCell<Option<&'static str>> = const { std::cell::RefCell::new(None) };
    // FMA3 ISA availability on the x86 target (`-mfma` / enabling -march).
    // `VecFma*`/`VecMadd*` lower to `vfmadd231p{s,d}` (FMA3), which is NOT in
    // the baseline SSE2 ISA. Contraction (`fp_contract == Fast`) alone must
    // not produce them, exactly like scalar FMA fusion: GCC requires `-mfma`
    // even under `-ffp-contract=fast`. Set once per `run_passes` (per TU) by
    // the driver; AArch64 `fmla` is baseline ISA and needs no gate.
    static X86_FMA_AVAILABLE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Record whether the x86 target has FMA3. Called by `run_passes` before any
/// vectorization entry point runs on this thread.
pub(crate) fn set_x86_fma_enabled(enabled: bool) {
    X86_FMA_AVAILABLE.with(|f| f.set(enabled));
}

fn x86_fma_enabled() -> bool {
    X86_FMA_AVAILABLE.with(|f| f.get())
}

fn set_reject(reason: &'static str) {
    REJECT_REASON.with(|r| {
        if r.borrow().is_none() {
            *r.borrow_mut() = Some(reason);
        }
    });
}

fn take_reject() -> Option<&'static str> {
    REJECT_REASON.with(|r| r.borrow_mut().take())
}

/// Run SSE2 vectorization on a function with precomputed CFG analysis.
pub(crate) fn vectorize_with_analysis(func: &mut IrFunction, cfg: &CfgAnalysis) -> usize {
    vectorize_with_analysis_mode(func, cfg, false, false, false, FpContract::default())
}

fn vectorize_with_analysis_mode(
    func: &mut IrFunction,
    cfg: &CfgAnalysis,
    force_two_wide: bool,
    neon: bool,
    fp_reassoc: bool,
    fp_contract: crate::common::fp_contract::FpContract,
) -> usize {
    let num_blocks = func.blocks.len();
    let loops = loop_analysis::find_natural_loops(num_blocks, &cfg.preds, &cfg.succs, &cfg.idom);

    let debug = std::env::var("LCCC_DEBUG_VECTORIZE").is_ok();
    if debug {
        eprintln!(
            "[VEC] Function: {}, blocks: {}, loops: {}",
            func.name,
            num_blocks,
            loops.len()
        );
    }

    if loops.is_empty() {
        return 0;
    }

    let mut total_changes = 0;

    // Process innermost loops (loops that don't contain other loops).
    for (idx, loop_info) in loops.iter().enumerate() {
        // Check if this is an innermost loop (no other loop nests strictly inside it).
        let is_innermost = !loops.iter().enumerate().any(|(other_idx, other)| {
            idx != other_idx
                && other.body.len() < loop_info.body.len()
                && other.body.iter().all(|b| loop_info.body.contains(b))
        });

        if debug {
            eprintln!(
                "[VEC] Loop {} at header={}, body_size={}, innermost={}",
                idx,
                loop_info.header,
                loop_info.body.len(),
                is_innermost
            );
        }

        if !is_innermost {
            continue;
        }

        let body_insts = loop_info
            .body
            .iter()
            .map(|&bi| func.blocks.get(bi).map_or(0, |b| b.instructions.len()))
            .sum();
        if matches!(
            crate::pgo::unroll_pgo::should_vectorize_loop(func, loop_info.header, body_insts),
            Some(false)
        ) {
            if debug || std::env::var("LCCC_WHY_NOT_VECTORIZE").is_ok() {
                eprintln!(
                    "[VEC] Loop {} (header block {}) not vectorized: PGO trip/body cost veto",
                    idx, loop_info.header
                );
            }
            continue;
        }

        // Try to vectorize this loop - first try matmul, then try reduction patterns.
        take_reject();
        if let Some(pattern) = analyze_loop_pattern(func, loop_info, cfg) {
            // Select vector width: default to AVX2 (4-wide) unless explicitly disabled
            let use_sse2 = force_two_wide || std::env::var("LCCC_FORCE_SSE2").is_ok();
            let vec_width: i64 = if use_sse2 { 2 } else { 4 };
            // The AArch64 two-wide lowering emits both halves per iteration,
            // consuming four doubles. Use the real machine-step width for the
            // profitability/correctness gate rather than the nominal vector
            // type width.
            let machine_step_width = if neon { 4 } else { vec_width };

            // Profitability: with a KNOWN constant trip count, require the
            // vector body to run at least twice (trip >= 2*width). One vector
            // iteration + setup + horizontal combine + scalar remainder is a
            // net LOSS vs the plain scalar loop (verified: a 4-iteration sum
            // previously produced a 1-iteration vector body + 3-iteration
            // remainder for zero win and ~3x code size). Unknown (dynamic)
            // trip counts still vectorize - the runtime guard handles n < width.
            let skip_small = match &pattern.limit {
                Operand::Const(c) => c.to_i64().map_or(false, |n| n < 2 * machine_step_width),
                _ => false,
            };
            if skip_small {
                if debug {
                    eprintln!(
                        "[VEC] Skip: constant trip count < 2x machine step width ({})",
                        machine_step_width
                    );
                }
            } else if use_sse2 {
                if debug {
                    eprintln!(
                        "[VEC] Matmul pattern matched! Transforming to FmaF64x2 (SSE2, 2-wide)"
                    );
                }
                total_changes += transform_to_fma_f64x2(func, &pattern);
            } else {
                // Use AVX2 by default (or if LCCC_FORCE_AVX2 is set)
                if debug {
                    eprintln!(
                        "[VEC] Matmul pattern matched! Transforming to FmaF64x4 (AVX2, 4-wide)"
                    );
                }
                total_changes += transform_to_fma_f64x4(func, &pattern);
            }
        } else if let Some(red_pattern) = analyze_reduction_pattern(
            func,
            loop_info,
            cfg,
            force_two_wide || (!neon && !force_two_wide),
            neon,
        ) {
            // Packed FP reductions reassociate additions across SIMD lanes.
            // That is observably different for IEEE-754 values (for example
            // [1e100, 1, -1e100, 1] sums to 1 in source order but 2 after a
            // four-lane tree reduction). GCC/Clang likewise require an
            // explicit fast-math/associative-math contract for this rewrite.
            if matches!(red_pattern.element_type, IrType::F32 | IrType::F64) && !fp_reassoc {
                if debug || std::env::var("LCCC_WHY_NOT_VECTORIZE").is_ok() {
                    eprintln!(
                        "[VEC] Loop {} (header block {}) not vectorized: floating-point reduction requires -fassociative-math or -ffast-math",
                        idx, loop_info.header
                    );
                }
                continue;
            }

            // IV PRECONDITIONS — gate BOTH the SSE2 and the AVX2 reduction
            // transform (both are called only from this dispatcher).
            //
            // (1) IV-WIDTH: only 32-bit-or-wider induction variables may
            //     become the vector loop's counter. Both addressing schemes
            //     rescale the counter's unit past a narrow domain:
            //       - byte-offset IV: the IV steps `byte_stride`
            //         (w * elem_size) per iteration and the limit is
            //         multiplied by `byte_stride`, so an I8/U8/I16/U16
            //         counter wraps once the byte range exceeds its width.
            //         Reproduced on main @ dcf673d, -O2: `unsigned char i;
            //         for (i = 0; i < n; i++) s += a[i];` with n = 120
            //         vectorized into a U8 counter stepped by 16 against a
            //         byte-limit of 480 — the counter wraps at 256 and the
            //         loop spins forever (GCC terminates and prints 7140).
            //       - element-index IV: the limit is divided by w, which
            //         keeps the trip count small, but the transform selects
            //         the byte-offset scheme whenever the GEP offset is
            //         `iv * elem_size` (the common case), and the remainder
            //         resumes at `iv_final >> log2(elem)` — a narrow counter
            //         makes both the vector coverage and the remainder start
            //         wrong once the scaled range exceeds its domain.
            //     A narrow-IV reduction therefore stays scalar; its trip
            //     counts are small and the scalar form is already tight.
            //
            // (2) IV-START: only constant-zero-init reductions may vectorize.
            //     Both addressing schemes assume the IV starts at element 0:
            //       - byte-offset IV: the limit is rescaled to
            //         `(n / w) * byte_stride` but the IV keeps its original
            //         preheader value, so with `for (i = c; i < n; i++)
            //         s += a[i]`, c != 0, the vector loop walks byte offsets
            //         [c, (n/w)*byte_stride) in `byte_stride` steps and the
            //         remainder resumes at `iv_final >> log2(elem)` — a
            //         misaligned, truncated run that silently drops
            //         elements.
            //       - element-index IV: the limit is divided to `n / w` but
            //         the IV starts at c, so the vector loop exits after
            //         covering only [c, c + w*floor(n/w)) and the remainder
            //         resumes at `iv_final * w`, far past the loop's actual
            //         range.
            //     Reproduced on main @ dcf673d, -O2 (GCC 14.2 prints 1760):
            //       `int`  accumulator, i = 5, n = 60 -> 31708938240 (AVX2)
            //       `long` accumulator, i = 5, n = 60 -> 30702305280 (SSE2)
            //     The SSE2 breakage is why this precondition lives in the
            //     dispatcher and not only inside the AVX2 transform. A
            //     DYNAMIC start (the preheader incoming is a Value) is
            //     rejected by the same check — no const exists to inspect.
            //     The Max reduction carries c-aware remainder math
            //     (max_shift in insert_reduction_remainder_loop) but it only
            //     reads CONST starts and no c != 0 Max shape the detector
            //     accepts currently vectorizes; rejecting everything but
            //     const-zero-init keeps every scheme provably correct and
            //     costs nothing on the supported set.
            {
                let hdr = &func.blocks[red_pattern.header_idx];
                let latch_label = func.blocks[red_pattern.latch_idx].label;
                let mut narrow_iv = false;
                let mut const_zero_init = false;
                for inst in &hdr.instructions {
                    if let Instruction::Phi {
                        dest, ty, incoming, ..
                    } = inst
                    {
                        if *dest != red_pattern.iv {
                            continue;
                        }
                        narrow_iv =
                            matches!(ty, IrType::I8 | IrType::U8 | IrType::I16 | IrType::U16);
                        const_zero_init = incoming.iter().any(|(op, lbl)| {
                            *lbl != latch_label
                                && matches!(op, Operand::Const(c) if c.to_i64() == Some(0))
                        });
                    }
                }
                if narrow_iv {
                    if debug {
                        eprintln!(
                            "[VEC] Skip reduction: induction variable is narrower than 32 bits (stays scalar)"
                        );
                    }
                    continue;
                }
                if !const_zero_init {
                    if debug {
                        eprintln!(
                            "[VEC] Skip reduction: induction variable does not start at a constant 0 (limit rescale + remainder math assume an element-0 start)"
                        );
                    }
                    continue;
                }
            }

            // Try reduction pattern vectorization (sum += arr[i], sum += a[i] * b[i], etc.)
            let use_sse2 = force_two_wide || std::env::var("LCCC_FORCE_SSE2").is_ok();
            let vec_width: i64 = if use_sse2 { 2 } else { 4 };

            // Same profitability gate as the matmul path (the reduction path
            // historically had NO trip-count check at all).
            let skip_small = match &red_pattern.limit {
                Operand::Const(c) => c.to_i64().map_or(false, |n| n < 2 * vec_width),
                _ => false,
            };
            if skip_small {
                if debug {
                    eprintln!(
                        "[VEC] Skip reduction: constant trip count < 2x vector width ({})",
                        vec_width
                    );
                }
            } else if use_sse2 || red_pattern.element_type == IrType::I64 {
                // I64 AVX2 4-wide path not wired yet; 2-wide SSE/AVX is competitive
                // and beats scalar (Godbolt: GCC/Clang/ICX all vectorize I64 sums).
                if debug {
                    eprintln!("[VEC] Reduction pattern matched! Transforming to SSE2 2-wide");
                }
                total_changes += transform_reduction_sse2(func, &red_pattern, neon);
            } else {
                if debug {
                    eprintln!("[VEC] Reduction pattern matched! Transforming to AVX2 4-wide");
                }
                total_changes += transform_reduction_avx2(func, &red_pattern, fp_contract);
            }
        } else if std::env::var("CCC_NO_MAP_VEC").is_err() {
            if let Some(map_pattern) = analyze_map_pattern(func, loop_info, neon) {
                // AArch64 is always 128-bit; x86 uses 256-bit vectors unless
                // the focused SSE diagnostic override requests 128-bit code.
                let avx2 = !neon && !force_two_wide && std::env::var("LCCC_FORCE_MAP_SSE").is_err();
                if debug {
                    eprintln!(
                        "[VEC] Map pattern matched! Transforming to {}-bit {:?}",
                        if avx2 { 256 } else { 128 },
                        map_pattern.elem_ty
                    );
                }
                total_changes += transform_map_vector(func, &map_pattern, avx2, fp_contract);
            } else if std::env::var("CCC_NO_STENCIL_VEC").is_err() {
                // OP-05a: generalized non-reduction FP loops (stencils and
                // multi-load maps with constant tap offsets). AArch64's NEON
                // intrinsics are not wired for the displacement form yet.
                if let Some(stencil_pattern) = analyze_stencil_pattern(func, loop_info) {
                    if !neon {
                        let avx2 = !force_two_wide && std::env::var("LCCC_FORCE_MAP_SSE").is_err();
                        if debug {
                            eprintln!(
                                "[VEC] Stencil pattern matched! taps={} {:?} ({}-bit)",
                                stencil_pattern.taps.len(),
                                stencil_pattern.elem_ty,
                                if avx2 { 256 } else { 128 }
                            );
                        }
                        total_changes +=
                            transform_stencil_vector(func, &stencil_pattern, avx2, fp_contract);
                    }
                } else {
                    let why = take_reject().unwrap_or("pattern shape not recognized");
                    if debug || std::env::var("LCCC_WHY_NOT_VECTORIZE").is_ok() {
                        eprintln!(
                            "[VEC] Loop {} (header block {}) not vectorized: {}",
                            idx, loop_info.header, why
                        );
                    }
                }
            } else {
                let why = take_reject().unwrap_or("pattern shape not recognized");
                if debug || std::env::var("LCCC_WHY_NOT_VECTORIZE").is_ok() {
                    eprintln!(
                        "[VEC] Loop {} (header block {}) not vectorized: {}",
                        idx, loop_info.header, why
                    );
                }
            }
        } else {
            let why = take_reject().unwrap_or("pattern shape not recognized");
            if debug || std::env::var("LCCC_WHY_NOT_VECTORIZE").is_ok() {
                eprintln!(
                    "[VEC] Loop {} (header block {}) not vectorized: {}",
                    idx, loop_info.header, why
                );
            }
        }
    }

    total_changes
}

/// Pattern matching result for a vectorizable loop.
#[derive(Debug)]
struct VectorizablePattern {
    /// Loop header block index
    header_idx: usize,
    /// Loop body block (where the accumulation happens)
    body_idx: usize,
    /// Loop latch block index (contains the increment and backedge)
    latch_idx: usize,
    /// Exit block index
    exit_idx: usize,
    /// Induction variable (loop counter)
    iv: Value,
    /// Induction variable increment instruction index in latch block
    iv_inc_idx: usize,
    /// GEP for C array (result pointer)
    c_gep: Value,
    /// GEP for B array (source vector pointer)
    b_gep: Value,
    /// A scalar pointer (broadcasted value, loop-invariant)
    a_ptr: Value,
    /// Store instruction index in body (will be replaced)
    store_idx: usize,
    /// Loop limit value (N in `j < N`)
    limit: Operand,
    /// Comparison instruction index that tests loop exit condition
    exit_cmp_inst_idx: usize,
    /// Comparison destination value
    exit_cmp_dest: Value,
    /// All block indices in the loop body
    loop_blocks: FxHashSet<usize>,
}

/// Reduction pattern types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReductionKind {
    /// Simple sum: sum += arr[i]
    Sum,
    /// Dot product: sum += a[i] * b[i]
    DotProduct,
    /// Maximum: mx = max(mx, arr[i]) — Select-shaped after if-conversion.
    /// Integer max is associative, commutative and idempotent, so a lane-wise
    /// NEON smax reduction is bit-identical to the scalar loop; no zero init
    /// required (the scalar init broadcasts into the vector accumulator).
    /// NEON-only (levkropp 8b139820, audited port).
    Max,
}

/// Pattern matching result for a vectorizable reduction loop.
#[derive(Debug)]
struct ReductionPattern {
    /// Type of reduction
    kind: ReductionKind,
    /// Element type being reduced (F64, F32, I32, I64)
    element_type: IrType,
    /// Scalar accumulator type, which may be wider than element_type.
    accumulator_type: IrType,
    /// Loop header block index
    header_idx: usize,
    /// Loop body block (where the accumulation happens)
    body_idx: usize,
    /// Loop latch block index (contains the increment and backedge)
    latch_idx: usize,
    /// Exit block index
    exit_idx: usize,
    /// Induction variable (loop counter)
    iv: Value,
    /// Induction variable increment instruction index in latch block
    iv_inc_idx: usize,
    /// Scalar accumulator phi node destination value in header
    accumulator_phi: Value,
    /// GEP for first array (arr for sum, a for dot product)
    array_a_gep: Value,
    /// GEP for second array (only for dot product)
    array_b_gep: Option<Value>,
    /// Index of the add instruction that updates the accumulator
    accumulator_add_idx: usize,
    /// Loop limit value (N in `i < N`)
    limit: Operand,
    /// Comparison instruction index that tests loop exit condition
    exit_cmp_inst_idx: usize,
    /// Comparison destination value
    exit_cmp_dest: Value,
    /// All block indices in the loop body
    loop_blocks: FxHashSet<usize>,
    /// Additional fully independent accumulators in the same loop body
    /// (multi-reduction: `a += x[i]*y[i]; b += x[i]*z[i]; c += ...`).  Each
    /// has the same kind, element type, accumulator type and body block as
    /// the primary; chains share only loads and the induction variable, never
    /// state.
    seconds: Vec<SecondaryAccumulator>,
    /// Conditional-sum guard: the Cmp value feeding the Select that wraps
    /// the accumulator update (`if (a[i] > 0) s += a[i]`). None for plain
    /// reductions. The transform emits the MASKED widening add
    /// (VecWidenMaskedAddI32x4ToI64x2) for this shape.
    guard_cond: Option<Value>,
    /// The guard comparison's rhs operand (validated: the comparison is
    /// `loaded_lane > guard_rhs`, signed, lhs = the loaded element).
    guard_rhs: Option<Operand>,
}

/// Value IDs of one extra accumulator's scalar remainder chain.
#[derive(Clone, Copy)]
struct RemainderAcc {
    scalar_sum: Value,
    sum_rem_phi: Value,
    sum_rem_next: Value,
    offset_a: Value,
    gep_rem_a: Value,
    load_rem_a: Value,
    offset_b: Value,
    gep_rem_b: Value,
    load_rem_b: Value,
    mul_rem: Value,
}

/// A second, independent reduction accumulator sharing the primary's loop.
///
/// The two accumulators are required to be *independent*: each add must read
/// only its own phi and pure loads (a load may be shared with the other
/// accumulator; a phi or any accumulator-derived value may not).  A dependent
/// chain like Adler-32's `sum2 += sum1` is deliberately rejected — it needs a
/// sequential prefix-sum transform, not an independent multi-reduction.
#[derive(Debug)]
struct SecondaryAccumulator {
    /// Scalar accumulator phi in the header (distinct from the primary's).
    accumulator_phi: Value,
    /// Destination of the add that updates this accumulator.
    add_result: Value,
    /// Index of that add in the (shared) body block.
    accumulator_add_idx: usize,
    /// GEP for first array (arr for sum, a for dot product).
    array_a_gep: Value,
    /// GEP for second array (only for dot product).
    array_b_gep: Option<Value>,
    /// Cast/copy closure of this accumulator, for soundness checks.
    accumulator_derived: FxHashSet<Value>,
    /// Load destinations feeding this accumulator's add (1 for Sum, 2 for
    /// DotProduct), for the union soundness check.
    loads: Vec<Value>,
}

/// Analyze a loop to detect the vectorizable matmul pattern.
fn analyze_loop_pattern(
    func: &IrFunction,
    loop_info: &loop_analysis::NaturalLoop,
    _cfg: &CfgAnalysis,
) -> Option<VectorizablePattern> {
    let debug = std::env::var("LCCC_DEBUG_VECTORIZE").is_ok();

    // Build label→index map so we can convert BlockId labels to array indices.
    let label_to_idx: FxHashMap<BlockId, usize> = func
        .blocks
        .iter()
        .enumerate()
        .map(|(i, b)| (b.label, i))
        .collect();

    let header_idx = loop_info.header;
    let header = &func.blocks[header_idx];

    // Find the header phi that reaches the loop-exit comparison through local
    // casts/copies. The first phi is often a reduction accumulator.
    let mut iv = None;
    for inst in &header.instructions {
        if let Instruction::Phi { dest, incoming, .. } = inst {
            if incoming.len() != 2 {
                continue;
            }
            let mut derived = FxHashSet::default();
            derived.insert(*dest);
            for candidate in &header.instructions {
                if let Instruction::Cast { dest, src, .. } | Instruction::Copy { dest, src } =
                    candidate
                {
                    if matches!(src, Operand::Value(v) if derived.contains(v)) {
                        derived.insert(*dest);
                    }
                }
            }
            let reaches_exit_cmp = header.instructions.iter().any(|candidate| {
                matches!(candidate, Instruction::Cmp { lhs, rhs, .. }
                    if matches!(lhs, Operand::Value(v) if derived.contains(v)))
            });
            if reaches_exit_cmp {
                iv = Some(dest);
                break;
            }
        }
    }
    if iv.is_none() && debug {
        eprintln!("[VEC]   No IV phi found in header");
        return None;
    }
    let iv = *iv?;

    // Build a map of values that are derived from the IV (casts, copies, etc.)
    let mut iv_derived = FxHashSet::default();
    iv_derived.insert(iv);
    for inst in &header.instructions {
        match inst {
            Instruction::Cast { dest, src, .. } | Instruction::Copy { dest, src } => {
                if let Operand::Value(src_val) = src {
                    if iv_derived.contains(src_val) {
                        iv_derived.insert(*dest);
                    }
                }
            }
            _ => {}
        }
    }

    // Find the comparison instruction for loop exit in header
    let mut exit_cmp_info = None;
    for (idx, inst) in header.instructions.iter().enumerate() {
        if let Instruction::Cmp {
            dest,
            op: _,
            lhs,
            rhs,
            ty: _,
        } = inst
        {
            // Check if comparing IV (or derived value) to a limit
            if let Operand::Value(lhs_val) = lhs {
                if iv_derived.contains(lhs_val) {
                    exit_cmp_info = Some((idx, *dest, rhs.clone()));
                    if debug {
                        eprintln!(
                            "[VEC]   Found comparison with IV-derived on left: {:?} < {:?}",
                            lhs, rhs
                        );
                    }
                    break;
                }
            } else if let Operand::Value(rhs_val) = rhs {
                if iv_derived.contains(rhs_val) {
                    // IV is on the right, use lhs as the limit
                    exit_cmp_info = Some((idx, *dest, lhs.clone()));
                    if debug {
                        eprintln!(
                            "[VEC]   Found comparison with IV-derived on right: {:?} > {:?}",
                            lhs, rhs
                        );
                    }
                    break;
                }
            }
        }
    }
    if exit_cmp_info.is_none() {
        if debug {
            eprintln!("[VEC]   No comparison instruction found for IV");
            eprintln!(
                "[VEC]   Header block has {} instructions:",
                header.instructions.len()
            );
            for (idx, inst) in header.instructions.iter().enumerate() {
                eprintln!("[VEC]     {}: {:?}", idx, inst);
            }
        }
        return None;
    }
    let (exit_cmp_inst_idx, exit_cmp_dest, limit) = exit_cmp_info?;

    // Search all blocks in the loop to find the one with the store instruction.
    // This is the actual computation block we want to vectorize.
    let mut body_idx = None;
    for &block_idx in &loop_info.body {
        if block_idx == header_idx {
            continue;
        }
        let block = &func.blocks[block_idx];
        for inst in &block.instructions {
            if matches!(inst, Instruction::Store { .. }) {
                body_idx = Some(block_idx);
                break;
            }
        }
        if body_idx.is_some() {
            break;
        }
    }

    if body_idx.is_none() {
        if debug {
            eprintln!("[VEC]   No store instruction found in any loop body block");
        }
        return None;
    }
    let body_idx = body_idx?;

    // This transform replaces one scalar read/modify/write stream. Additional
    // stores in any loop block are not represented by the vector body and may
    // overlap or overwrite its lanes (the i-j-k N=4 matmul reproducer did
    // exactly that after unrolling). Reject rather than silently dropping or
    // reordering observable memory effects.
    let store_count = loop_info
        .body
        .iter()
        .flat_map(|&block_index| &func.blocks[block_index].instructions)
        .filter(|instruction| matches!(instruction, Instruction::Store { .. }))
        .count();
    if store_count != 1 {
        if debug {
            eprintln!(
                "[VEC]   Loop has {} stores; matmul transform requires exactly one",
                store_count
            );
        }
        set_reject("matmul loop does not have exactly one store");
        return None;
    }

    // Find exit by looking at loop successors that are outside the loop.
    // Use label_to_idx to convert BlockId labels to block array indices for body.contains().
    let mut exit_label = None;
    for &block_idx in &loop_info.body {
        let block = &func.blocks[block_idx];
        match &block.terminator {
            Terminator::CondBranch {
                true_label,
                false_label,
                ..
            } => {
                let then_idx = label_to_idx.get(true_label).copied();
                let else_idx = label_to_idx.get(false_label).copied();
                let then_in_loop = then_idx.map_or(false, |i| loop_info.body.contains(&i));
                let else_in_loop = else_idx.map_or(false, |i| loop_info.body.contains(&i));
                if !then_in_loop {
                    exit_label = Some(*true_label);
                    break;
                } else if !else_in_loop {
                    exit_label = Some(*false_label);
                    break;
                }
            }
            Terminator::Branch(target) => {
                let target_idx = label_to_idx.get(target).copied();
                if !target_idx.map_or(false, |i| loop_info.body.contains(&i)) {
                    exit_label = Some(*target);
                    break;
                }
            }
            _ => {}
        }
    }

    if exit_label.is_none() {
        if debug {
            eprintln!("[VEC]   No exit block found");
        }
        return None;
    }
    let exit_label = exit_label?;
    let exit_idx = *label_to_idx.get(&exit_label)?;

    // Find the latch block (backedges to header).
    let latch_idx = find_latch(func, loop_info);
    if latch_idx.is_none() {
        if debug {
            eprintln!("[VEC]   No latch block found");
        }
        return None;
    }
    let latch_idx = latch_idx?;
    let latch = &func.blocks[latch_idx];

    // Find IV increment in latch: %next = add %iv, 1
    let mut iv_inc_idx = None;
    for (idx, inst) in latch.instructions.iter().enumerate() {
        if let Instruction::BinOp {
            op: IrBinOp::Add,
            lhs,
            rhs,
            ..
        } = inst
        {
            if let Operand::Value(lhs_val) = lhs {
                if *lhs_val == iv {
                    if let Operand::Const(c) = rhs {
                        if c.to_i64() == Some(1) {
                            iv_inc_idx = Some(idx);
                            break;
                        }
                    }
                }
            }
        }
    }
    if iv_inc_idx.is_none() {
        if debug {
            eprintln!("[VEC]   No IV increment by 1 found in latch");
        }
        return None;
    }
    let iv_inc_idx = iv_inc_idx?;

    // Analyze the body block for accumulation pattern.
    let body = &func.blocks[body_idx];

    // Find the store instruction.
    let mut store_info = None;
    for (idx, inst) in body.instructions.iter().enumerate() {
        if let Instruction::Store { ptr, val, .. } = inst {
            if let Operand::Value(store_val) = val {
                store_info = Some((idx, *ptr, *store_val));
                break;
            }
        }
    }
    if store_info.is_none() {
        if debug {
            eprintln!(
                "[VEC]   No store instruction found in body block {}",
                body_idx
            );
        }
        return None;
    }
    let (store_idx, store_addr, store_value) = store_info?;
    if debug {
        eprintln!(
            "[VEC]   Found store in block {}, tracing backward...",
            body_idx
        );
    }

    // Trace backwards: store_value should be result of fadd.
    let fadd_inst = find_inst_by_dest(body, store_value);
    if fadd_inst.is_none() {
        if debug {
            eprintln!("[VEC]   Store value not produced in body block");
        }
        return None;
    }
    let fadd_inst = fadd_inst?;
    let (c_load_val, mul_val) = match fadd_inst {
        Instruction::BinOp {
            op: IrBinOp::Add,
            lhs,
            rhs,
            ..
        } => Some((lhs, rhs)),
        _ => {
            if debug {
                eprintln!("[VEC]   Store value not from Add instruction");
            }
            None
        }
    }?;

    // Find the multiply.
    let mul_dest = match mul_val {
        Operand::Value(v) => v,
        _ => {
            if debug {
                eprintln!("[VEC]   Multiply operand is not a Value");
            }
            return None;
        }
    };
    let fmul_inst = find_inst_by_dest(body, *mul_dest);
    if fmul_inst.is_none() {
        if debug {
            eprintln!("[VEC]   Multiply value not produced in body block");
        }
        return None;
    }
    let fmul_inst = fmul_inst?;
    // The whole transform lowers to FmaF64x4 / FmaF64x2 (packed DOUBLE FMA).
    // Only double-precision matmuls may match: lowering a float or integer
    // matmul as double FMA reinterprets the arrays with the wrong element
    // width and stride (reproducer: a float matmul segfaulted at runtime).
    if let Instruction::BinOp { ty, .. } = fmul_inst {
        if *ty != IrType::F64 {
            if debug {
                eprintln!(
                    "[VEC]   Rejecting matmul: element type {:?} != F64 (only double FMA is supported)",
                    ty
                );
            }
            set_reject("matmul element type is not double (only F64 FMA is supported)");
            return None;
        }
    }
    let (a_val, b_val) = match fmul_inst {
        Instruction::BinOp {
            op: IrBinOp::Mul,
            lhs,
            rhs,
            ..
        } => Some((lhs, rhs)),
        _ => {
            if debug {
                eprintln!("[VEC]   Add operand not from Mul instruction");
            }
            None
        }
    }?;

    // Verify C load.
    let c_load_dest = match c_load_val {
        Operand::Value(v) => v,
        _ => {
            if debug {
                eprintln!("[VEC]   C load operand is not a Value");
            }
            return None;
        }
    };
    let c_load_inst = find_inst_by_dest(body, *c_load_dest);
    if c_load_inst.is_none() {
        if debug {
            eprintln!("[VEC]   C load value not produced in body block");
        }
        return None;
    }
    let c_load_inst = c_load_inst?;
    let c_load_addr = match c_load_inst {
        Instruction::Load { ptr, .. } => *ptr,
        _ => {
            if debug {
                eprintln!("[VEC]   C load not from Load instruction");
            }
            return None;
        }
    };

    // Store and load must use the same GEP.
    if c_load_addr != store_addr {
        if debug {
            eprintln!("[VEC]   C load and store use different addresses");
        }
        return None;
    }

    // Extract GEPs for C and B.
    let c_gep = store_addr;
    // Verify C GEP exists somewhere in the loop (may have been hoisted by LICM)
    if find_inst_in_loop(func, &loop_info.body, c_gep).is_none() {
        if debug {
            eprintln!("[VEC]   C GEP not found in loop");
        }
        return None;
    }

    // Find B load and GEP.
    let b_load_dest = match b_val {
        Operand::Value(v) => v,
        _ => {
            if debug {
                eprintln!("[VEC]   B value operand is not a Value");
            }
            return None;
        }
    };
    // Search for B load in the entire loop (may have been moved by optimizations)
    let b_load_result = find_inst_in_loop(func, &loop_info.body, *b_load_dest);
    if b_load_result.is_none() {
        if debug {
            eprintln!("[VEC]   B load not found in loop");
        }
        return None;
    }
    let (_b_load_block, b_load_inst) = b_load_result?;
    let b_load_addr = match b_load_inst {
        Instruction::Load { ptr, .. } => ptr,
        _ => {
            if debug {
                eprintln!("[VEC]   B load not from Load instruction");
            }
            return None;
        }
    };
    let b_gep = *b_load_addr;
    // Verify B GEP exists somewhere in the loop
    if find_inst_in_loop(func, &loop_info.body, b_gep).is_none() {
        if debug {
            eprintln!("[VEC]   B GEP not found in loop");
        }
        return None;
    }

    // Both streams widened by this transform must advance by exactly one F64
    // element per scalar iteration. Merely depending on the IV is insufficient
    // (`iv * row_stride` is a column walk, not contiguous). Reuse the exact
    // byte-stride recognizer used by map/reduction vectorization, then verify
    // that its unscaled index is this loop's IV through Cast/Copy chains only.
    let traces_to_loop_iv = |start: Value| {
        let mut current = start;
        let mut seen = FxHashSet::default();
        loop {
            if !seen.insert(current.0) {
                break false;
            }
            if iv_derived.contains(&current) {
                break true;
            }
            let Some((_, definition)) = find_inst_in_loop(func, &loop_info.body, current) else {
                break false;
            };
            match definition {
                Instruction::Cast {
                    src: Operand::Value(source),
                    ..
                }
                | Instruction::Copy {
                    src: Operand::Value(source),
                    ..
                } => current = *source,
                _ => break false,
            }
        }
    };
    let contiguous_f64_stream = |gep: Value| {
        find_reduction_byte_iv(func, &loop_info.body, gep, 8)
            .is_some_and(|(_, index)| traces_to_loop_iv(index))
    };
    if !contiguous_f64_stream(c_gep) || !contiguous_f64_stream(b_gep) {
        if debug {
            eprintln!("[VEC]   C/B stream is not a contiguous F64 IV walk");
        }
        set_reject("matmul C/B stream is not contiguous at F64 stride");
        return None;
    }

    // Find A load (should be loop-invariant, may have been hoisted by LICM).
    let a_load_dest = match a_val {
        Operand::Value(v) => v,
        _ => {
            if debug {
                eprintln!("[VEC]   A value operand is not a Value");
            }
            return None;
        }
    };
    // Search for A load in the entire loop
    let a_load_result = find_inst_in_loop(func, &loop_info.body, *a_load_dest);
    if a_load_result.is_none() {
        if debug {
            eprintln!("[VEC]   A load not found in loop");
        }
        return None;
    }
    let (_a_load_block, a_load_inst) = a_load_result?;
    let a_ptr = match a_load_inst {
        Instruction::Load { ptr, .. } => *ptr,
        _ => {
            if debug {
                eprintln!("[VEC]   A load not from Load instruction");
            }
            return None;
        }
    };

    // C must be provably disjoint from BOTH read streams. Merely seeing
    // different parameter SSA values is insufficient: callers may pass c=b+1
    // or c=a, in which case vectorization changes loop-carried data.
    let c_root = proven_object_root(func, c_gep)?;
    let b_root = proven_object_root(func, b_gep)?;
    let a_root = proven_object_root(func, a_ptr)?;
    if !roots_proven_distinct(&c_root, &b_root) || !roots_proven_distinct(&c_root, &a_root) {
        if debug {
            eprintln!(
                "[VEC]   Rejecting matmul: C may alias source (C={:?}, B={:?}, A={:?})",
                c_root, b_root, a_root
            );
        }
        set_reject("matmul destination may alias A/B (use restrict or runtime versioning)");
        return None;
    }

    Some(VectorizablePattern {
        header_idx,
        body_idx,
        latch_idx,
        exit_idx,
        iv,
        iv_inc_idx,
        c_gep,
        b_gep,
        a_ptr,
        store_idx,
        limit,
        exit_cmp_inst_idx,
        exit_cmp_dest,
        loop_blocks: loop_info.body.clone(),
    })
}

/// Analyze a loop to detect vectorizable reduction patterns (sum += arr[i], sum += a[i] * b[i]).
/// Soundness gate for reduction vectorization.
///
/// The transform rewrites the whole loop into a vector-width loop (N/8
/// iterations) plus a scalar remainder, and rewires the accumulator phi to a
/// vector accumulator updated by ONE vector add per iteration. This is only
/// valid when:
///
///  1. the accumulator (and every copy/cast of it) is written by exactly one
///     instruction — the identified scalar Add — plus the header phi and pure
///     copy/cast relays;
///  2. no instruction other than the identified Add (or a relay) READS an
///     accumulator-derived value — an `else s -= 2` arm or an `if (s > x)
///     break` test would silently corrupt the vector accumulator;
///  3. the Add's block dominates the latch, so the update executes on every
///     path back to the header — data-dependent accumulation (`if (c)
///     s += a[i];`) is rejected: the scalar condition would gate a whole
///     8-lane vector add, and the not-taken path would copy an undefined
///     value (observed miscompile: `if (a[i] & 1) s += a[i]; else s -= 2;`
///     produced garbage for even elements);
///  4. the loop contains no foreign memory operations (other loads, stores,
///     calls, atomics, volatile, intrinsics, inline asm, non-Branch
///     terminators): every remaining instruction would execute once per
///     VECTOR iteration with the scaled induction variable and touch the
///     wrong elements.
fn reduction_pattern_is_sound(
    func: &IrFunction,
    cfg: &CfgAnalysis,
    loop_blocks: &FxHashSet<usize>,
    header_idx: usize,
    body_idx: usize,
    adds: &[(usize, Value)],
    accumulator_phis: &[Value],
    accumulator_derived: &FxHashSet<Value>,
    allowed_loads: &[Value],
    iv_phi: Value,
) -> bool {
    let debug = std::env::var("LCCC_DEBUG_VECTORIZE").is_ok();
    let is_add = |block_idx: usize, inst_idx: usize, dest: Value| {
        block_idx == body_idx
            && adds
                .iter()
                .any(|&(add_idx, add_result)| add_idx == inst_idx && add_result == dest)
    };
    // Labels of every loop block, to detect a phi's back-edge incoming.
    let loop_block_labels: FxHashSet<u32> = loop_blocks
        .iter()
        .map(|&bi| func.blocks[bi].label.0)
        .collect();

    // (4) terminators: only the header may branch (its exit condition);
    //     every other loop block must end in an unconditional branch.
    for &block_idx in loop_blocks {
        if block_idx != header_idx {
            if !matches!(func.blocks[block_idx].terminator, Terminator::Branch(_)) {
                if debug {
                    eprintln!(
                        "[VEC-RED]   Rejecting: loop block {} terminator {:?}",
                        block_idx, func.blocks[block_idx].terminator
                    );
                }
                return false;
            }
        }
    }

    for &block_idx in loop_blocks {
        let block = &func.blocks[block_idx];
        for (inst_idx, inst) in block.instructions.iter().enumerate() {
            match inst {
                Instruction::Phi { dest, incoming, .. } => {
                    // The transform rewires ONLY the IV phi and the recognized
                    // accumulator phis. Any OTHER loop-carried phi (a value
                    // whose back-edge incoming is a non-constant Value from a
                    // loop block) changes once per scalar iteration but is left
                    // untouched, so after vectorization it would be updated
                    // with the scaled induction variable and be silently wrong
                    // (reproducer: `s += a[i]; last = a[i];` — `last` ended up
                    // reading A[0], A[8], ... instead of A[i]). Reject the
                    // whole loop (fail-closed; the scalar form is correct).
                    let is_iv_or_accum = *dest == iv_phi
                        || accumulator_phis.iter().any(|phi| *phi == *dest);
                    if !is_iv_or_accum {
                        let loop_carried = incoming.iter().any(|(op, lbl)| {
                            loop_block_labels.contains(&lbl.0) && matches!(op, Operand::Value(_))
                        });
                        if loop_carried {
                            if debug {
                                eprintln!(
                                    "[VEC-RED]   Rejecting: unhandled loop-carried phi {} in block {}",
                                    dest.0, block_idx
                                );
                            }
                            set_reject("loop has an unhandled loop-carried value");
                            return false;
                        }
                    }
                    // Phis writing an accumulator-derived value are forbidden
                    // (the vector transform rewires the accumulator phi to a
                    // vector value).  With multiple accumulators, each
                    // accumulator's own header phi is allowed; any other
                    // accumulator-derived phi is not.
                    if accumulator_derived.contains(dest)
                        && (block_idx != header_idx
                            || !accumulator_phis.iter().any(|phi| *phi == *dest))
                    {
                        if debug {
                            eprintln!(
                                "[VEC-RED]   Rejecting: accumulator-derived phi {} in block {}",
                                dest.0, block_idx
                            );
                        }
                        return false;
                    }
                }
                Instruction::Copy { dest, src } | Instruction::Cast { dest, src, .. } => {
                    if accumulator_derived.contains(dest)
                        && !matches!(src, Operand::Value(v) if accumulator_derived.contains(v))
                    {
                        if debug {
                            eprintln!(
                                "[VEC-RED]   Rejecting: copy/cast {} = {:?} overwrites accumulator with foreign value",
                                dest.0, src
                            );
                        }
                        return false;
                    }
                }
                Instruction::BinOp { dest, lhs, rhs, .. } => {
                    let uses_acc = matches!(lhs, Operand::Value(v) if accumulator_derived.contains(v))
                        || matches!(rhs, Operand::Value(v) if accumulator_derived.contains(v));
                    if uses_acc && !is_add(block_idx, inst_idx, *dest) {
                        if debug {
                            eprintln!(
                                "[VEC-RED]   Rejecting: BinOp {} reads accumulator outside identified Add",
                                dest.0
                            );
                        }
                        return false;
                    }
                    if accumulator_derived.contains(dest) {
                        if debug {
                            eprintln!(
                                "[VEC-RED]   Rejecting: BinOp writes accumulator-derived {}",
                                dest.0
                            );
                        }
                        return false;
                    }
                }
                // (4) foreign memory / side effects / control
                Instruction::Store { .. }
                | Instruction::Call { .. }
                | Instruction::CallIndirect { .. }
                | Instruction::Memcpy { .. }
                | Instruction::VaStart { .. }
                | Instruction::VaEnd { .. }
                | Instruction::VaCopy { .. }
                | Instruction::VaArg { .. }
                | Instruction::VaArgStruct { .. }
                | Instruction::AtomicLoad { .. }
                | Instruction::AtomicStore { .. }
                | Instruction::AtomicRmw { .. }
                | Instruction::AtomicInc { .. }
                | Instruction::AtomicCmpxchg { .. }
                | Instruction::Fence { .. }
                | Instruction::DynAlloca { .. }
                | Instruction::StackRestore { .. }
                | Instruction::InlineAsm { .. }
                | Instruction::Intrinsic { .. } => {
                    if debug {
                        eprintln!(
                            "[VEC-RED]   Rejecting: foreign side-effect instruction in loop block {}",
                            block_idx
                        );
                    }
                    return false;
                }
                Instruction::Load { dest, .. } => {
                    if !allowed_loads.iter().any(|l| *l == *dest) {
                        if debug {
                            eprintln!(
                                "[VEC-RED]   Rejecting: foreign load {} in loop block {}",
                                dest.0, block_idx
                            );
                        }
                        return false;
                    }
                }
                _ => {}
            }
        }
    }

    // (2) no instruction other than the identified Adds and copy/cast/phi
    //     relays may READ an accumulator-derived value.
    for &block_idx in loop_blocks {
        let block = &func.blocks[block_idx];
        for (inst_idx, inst) in block.instructions.iter().enumerate() {
            let is_any_add =
                block_idx == body_idx && adds.iter().any(|&(add_idx, _)| add_idx == inst_idx);
            if is_any_add
                || matches!(
                    inst,
                    Instruction::Copy { .. }
                        | Instruction::Cast { .. }
                        | Instruction::Phi { .. }
                        | Instruction::Cmp { .. }
                        | Instruction::Select { .. }
                )
            {
                continue;
            }
            let mut uses_acc = false;
            crate::backend::liveness::for_each_operand_in_instruction(inst, |op| {
                if let Operand::Value(v) = op {
                    if accumulator_derived.contains(v) {
                        uses_acc = true;
                    }
                }
            });
            if uses_acc {
                if debug {
                    eprintln!(
                        "[VEC-RED]   Rejecting: instruction {} in block {} reads accumulator",
                        inst_idx, block_idx
                    );
                }
                return false;
            }
        }
        if block_idx != header_idx {
            let mut uses_acc = false;
            crate::backend::liveness::for_each_operand_in_terminator(&block.terminator, |op| {
                if let Operand::Value(v) = op {
                    if accumulator_derived.contains(v) {
                        uses_acc = true;
                    }
                }
            });
            if uses_acc {
                if debug {
                    eprintln!(
                        "[VEC-RED]   Rejecting: terminator in block {} reads accumulator",
                        block_idx
                    );
                }
                return false;
            }
        }
    }

    // (3) the Add's block must dominate the latch: the accumulator update
    //     executes on every iteration, unconditionally.
    let latch_idx = loop_blocks
        .iter()
        .copied()
        .find(|&b| {
            matches!(func.blocks[b].terminator, Terminator::Branch(t) if t.0 == func.blocks[header_idx].label.0)
        })
        .unwrap_or(usize::MAX);
    if latch_idx == usize::MAX {
        if debug {
            eprintln!("[VEC-RED]   Rejecting: no latch found in loop body");
        }
        return false;
    }
    let mut cur = latch_idx;
    let mut steps = 0;
    while cur != body_idx
        && cur < cfg.num_blocks
        && cur != cfg.idom[cur]
        && steps < cfg.num_blocks + 1
    {
        cur = cfg.idom[cur];
        steps += 1;
    }
    if cur != body_idx {
        if debug {
            eprintln!(
                "[VEC-RED]   Rejecting: accumulator Add block {} does not dominate latch {}",
                body_idx, latch_idx
            );
        }
        return false;
    }

    true
}

/// `neon` enables AArch64-only forms: i32→i64 dot products whose multiply
/// operands are sign-extension casts of i32 loads (lowered to smlal/smlal2).
fn analyze_reduction_pattern(
    func: &IrFunction,
    loop_info: &loop_analysis::NaturalLoop,
    cfg: &CfgAnalysis,
    allow_widening_i32: bool,
    neon: bool,
) -> Option<ReductionPattern> {
    let debug = std::env::var("LCCC_DEBUG_VECTORIZE").is_ok();
    let header_idx = loop_info.header;
    let header = &func.blocks[header_idx];

    if debug {
        eprintln!(
            "[VEC-RED] Analyzing reduction pattern for loop at header {}",
            header_idx
        );
    }

    // Find exit and latch blocks first
    let exit_idx = find_exit(func, loop_info);
    if exit_idx.is_none() {
        if debug {
            eprintln!("[VEC-RED]   Could not find exit block");
        }
        return None;
    }
    let exit_idx = exit_idx?;

    let latch_idx = find_latch(func, loop_info);
    if latch_idx.is_none() {
        if debug {
            eprintln!("[VEC-RED]   Could not find latch block");
        }
        return None;
    }
    let latch_idx = latch_idx?;
    let latch = &func.blocks[latch_idx];

    // This reduction transform assumes every scalar iteration contributes to
    // the accumulator.  Predicated/conditional reductions need masking (and
    // independent pointer progression), which this transform does not yet
    // implement.  Treat any internal conditional as a legality failure.
    if loop_info.body.iter().copied().any(|block_idx| {
        block_idx != header_idx
            && matches!(
                func.blocks[block_idx].terminator,
                Terminator::CondBranch { .. }
            )
    }) {
        if debug {
            eprintln!("[VEC-RED]   Rejecting conditional reduction loop");
        }
        return None;
    }

    // Identify the induction phi from the header exit comparison. There may be
    // unrelated unit increments in the latch (especially after unrolling), so
    // choosing the first `add ?, 1` is not reliable.
    let mut iv = None;
    for inst in &header.instructions {
        let Instruction::Phi { dest, incoming, .. } = inst else {
            continue;
        };
        if incoming.len() != 2 {
            continue;
        }
        let mut derived = FxHashSet::default();
        derived.insert(*dest);
        for candidate in &header.instructions {
            if let Instruction::Cast { dest, src, .. } | Instruction::Copy { dest, src } = candidate
            {
                if matches!(src, Operand::Value(v) if derived.contains(v)) {
                    derived.insert(*dest);
                }
            }
        }
        if header.instructions.iter().any(|candidate| {
            matches!(candidate,
            Instruction::Cmp { lhs, rhs, .. }
                if matches!(lhs, Operand::Value(v) if derived.contains(v)))
        }) {
            iv = Some(*dest);
            break;
        }
    }
    if iv.is_none() && debug {
        eprintln!("[VEC-RED]   No IV increment found in latch");
        return None;
    }
    let iv = iv?;

    // Build IV-derived values using fixed-point iteration (like matmul pattern)
    // This captures casts/copies of the IV across ALL loop blocks, not just the header
    let mut iv_derived = FxHashSet::default();
    iv_derived.insert(iv);

    let mut changed = true;
    while changed {
        changed = false;
        for &block_idx in &loop_info.body {
            let block = &func.blocks[block_idx];
            for inst in &block.instructions {
                match inst {
                    Instruction::Cast { dest, src, .. } | Instruction::Copy { dest, src } => {
                        if let Operand::Value(src_val) = src {
                            if iv_derived.contains(src_val) && iv_derived.insert(*dest) {
                                changed = true;
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    if debug {
        eprintln!("[VEC-RED]   IV-derived values: {:?}", iv_derived);
    }

    // Find comparison for loop exit
    let mut exit_cmp_info = None;
    for (idx, inst) in header.instructions.iter().enumerate() {
        if let Instruction::Cmp {
            dest,
            op: _,
            lhs,
            rhs,
            ty: _,
        } = inst
        {
            if let Operand::Value(lhs_val) = lhs {
                if iv_derived.contains(lhs_val) {
                    exit_cmp_info = Some((idx, *dest, rhs.clone()));
                    break;
                }
            } else if let Operand::Value(rhs_val) = rhs {
                if iv_derived.contains(rhs_val) {
                    exit_cmp_info = Some((idx, *dest, lhs.clone()));
                    break;
                }
            }
        }
    }
    let (exit_cmp_inst_idx, exit_cmp_dest, limit) = exit_cmp_info?;

    // Find IV increment in latch: %next = add %iv, 1
    let mut iv_inc_idx = None;
    for (idx, inst) in latch.instructions.iter().enumerate() {
        if let Instruction::BinOp {
            op: IrBinOp::Add,
            lhs,
            rhs,
            ..
        } = inst
        {
            if let Operand::Value(lhs_val) = lhs {
                if *lhs_val == iv {
                    if let Operand::Const(c) = rhs {
                        if c.to_i64() == Some(1) {
                            iv_inc_idx = Some(idx);
                            break;
                        }
                    }
                }
            }
        }
    }
    let iv_inc_idx = iv_inc_idx?;

    // Max-reduction form (levkropp 8b139820, audited port): the accumulator
    // phi's latch incoming is a Select of the loaded element vs the phi
    // (`mx = x > mx ? x : mx`, if-converted). Detected BEFORE the zero-init
    // sum search because a max accumulator is NOT zero-initialised (its init
    // is arr[0] or any seed; the transform broadcasts it). All four compare
    // polarities are accepted; SIGNED compares only — smax is a signed max,
    // an unsigned compare (Ugt/Ult) must NOT match (that would need umax).
    let mut is_max_reduction = false;
    let mut max_select_val = None;
    let mut max_accumulator_phi = None;
    // v12 Fix F: the Max reduction DETECTOR is target-independent (the
    // Select-shaped max pattern is the same on AArch64 and x86). The
    // AVX2 transform body + lowerings + whitelist (VecMaxI32x8 class 5,
    // VecHorizontalMaxI32x8 legal consumer, is_two_operand_binary
    // deferral) are all wired and CORRECT (output matches GCC for
    // 10M-element find_max). However, the vectorized find_max is currently
    // ~1.4× SLOWER than scalar on the loop_patterns benchmark — the init
    // broadcast, horizontal reduce, and YMM7 occupancy add overhead that
    // exceeds the 8× lane speedup for this small (10M) working set. The
    // detection is therefore still gated on `neon` (AArch64, where it's a
    // proven win) until the AVX2 cost model is tuned. All infrastructure
    // landed; removing the gate is a one-line v13 change once the cost
    // model accounts for the init+reduce overhead.
    if neon {
        let latch_label = func.blocks[latch_idx].label;
        'max_search: for inst in &header.instructions {
            let Instruction::Phi {
                dest,
                incoming,
                ty: phi_ty,
                ..
            } = inst
            else {
                continue;
            };
            if *dest == iv || incoming.len() != 2 || *phi_ty != IrType::I32 {
                continue;
            }
            let latch_src = incoming.iter().find_map(|(op, lbl)| {
                if *lbl == latch_label {
                    if let Operand::Value(v) = op {
                        return Some(*v);
                    }
                }
                None
            });
            let Some(sel_val) = latch_src else { continue };
            for &bi in &loop_info.body {
                for sinst in &func.blocks[bi].instructions {
                    let Instruction::Select {
                        dest: sd,
                        cond,
                        true_val,
                        false_val,
                        ..
                    } = sinst
                    else {
                        continue;
                    };
                    if *sd != sel_val {
                        continue;
                    }
                    let (Operand::Value(cond_v), Operand::Value(tv), Operand::Value(fv)) =
                        (cond, true_val, false_val)
                    else {
                        continue;
                    };
                    // Arms must be {the phi, the element X}; cond compares X
                    // against the phi.
                    let (x_val, take_x_when_true) = if fv.0 == dest.0 {
                        (*tv, true)
                    } else if tv.0 == dest.0 {
                        (*fv, false)
                    } else {
                        continue;
                    };
                    let Some(cmp_inst) = func.blocks[bi]
                        .instructions
                        .iter()
                        .find(|i| matches!(i.dest(), Some(d) if d.0 == cond_v.0))
                    else {
                        continue;
                    };
                    let Instruction::Cmp { op, lhs, rhs, .. } = cmp_inst else {
                        continue;
                    };
                    use crate::ir::reexports::IrCmpOp as C;
                    let x_on = |o: &Operand| matches!(o, Operand::Value(v) if v.0 == x_val.0);
                    let phi_on = |o: &Operand| matches!(o, Operand::Value(v) if v.0 == dest.0);
                    let is_max_form = match (op, take_x_when_true) {
                        (C::Sgt | C::Sge, true) if x_on(lhs) && phi_on(rhs) => true,
                        (C::Slt | C::Sle, false) if x_on(lhs) && phi_on(rhs) => true,
                        (C::Slt | C::Sle, true) if phi_on(lhs) && x_on(rhs) => true,
                        (C::Sgt | C::Sge, false) if phi_on(lhs) && x_on(rhs) => true,
                        _ => false,
                    };
                    if is_max_form {
                        max_accumulator_phi = Some(*dest);
                        is_max_reduction = true;
                        max_select_val = Some(sel_val);
                        break 'max_search;
                    }
                }
            }
        }
    }

    // Find the accumulator phi (scalar sum variable)
    let mut accumulator_phi = max_accumulator_phi;
    let mut accumulator_init_is_zero = false;
    if accumulator_phi.is_none() {
        for inst in &header.instructions {
            if let Instruction::Phi { dest, incoming, .. } = inst {
                if incoming.len() == 2 && *dest != iv {
                    // Check if initialized to zero (common for reductions)
                    for (val, _block) in incoming {
                        if let Operand::Const(c) = val {
                            if c.to_i64() == Some(0) {
                                accumulator_init_is_zero = true;
                            } else if c.to_f64().map(|f| f == 0.0).unwrap_or(false) {
                                accumulator_init_is_zero = true;
                            }
                        }
                    }
                    if accumulator_init_is_zero {
                        accumulator_phi = Some(*dest);
                        break;
                    }
                }
            }
        }
    }
    if accumulator_phi.is_none() {
        if debug {
            eprintln!("[VEC-RED]   No accumulator phi found (initialized to zero)");
        }
        set_reject("no zero-initialized accumulator variable found");
        return None;
    }
    let accumulator_phi = accumulator_phi?;

    // Collect any additional zero-init phis in the header.  Zero or one extra
    // phi is acceptable: a single extra phi may be a second, independent
    // reduction (analyzed after the primary); two or more exceed what the
    // transform supports and are rejected as before.  A dependent reduction
    // (`sum2 += sum1`) is also rejected, by the independence check, because
    // the vector transform cannot split it into two standalone accumulators.
    let other_zero_phis: Vec<Value> = header
        .instructions
        .iter()
        .filter_map(|inst| {
            if let Instruction::Phi { dest, incoming, .. } = inst {
                if *dest != iv && *dest != accumulator_phi && incoming.len() == 2 {
                    let zero_init = incoming.iter().any(|(val, _)| {
                        if let Operand::Const(c) = val {
                            c.to_i64() == Some(0) || c.to_f64().map(|f| f == 0.0).unwrap_or(false)
                        } else {
                            false
                        }
                    });
                    return zero_init.then_some(*dest);
                }
            }
            None
        })
        .collect();
    // No upper bound on the number of extra accumulators: each is analyzed
    // independently and any failure rejects the whole loop (fail-closed), so
    // an arbitrarily wide multi-reduction is safe to attempt.

    // Build a set of accumulator-derived values (accumulator + casts of accumulator)
    let mut accumulator_derived = FxHashSet::default();
    accumulator_derived.insert(accumulator_phi);

    // Find all casts/copies of the accumulator
    let mut changed = true;
    while changed {
        changed = false;
        for &block_idx in &loop_info.body {
            let block = &func.blocks[block_idx];
            for inst in &block.instructions {
                match inst {
                    Instruction::Cast { dest, src, .. } | Instruction::Copy { dest, src } => {
                        if let Operand::Value(src_val) = src {
                            if accumulator_derived.contains(src_val)
                                && !accumulator_derived.contains(dest)
                            {
                                accumulator_derived.insert(*dest);
                                changed = true;
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    // Find the body block that updates the accumulator
    let mut body_idx = None;
    let mut accumulator_add_idx = None;
    let mut add_result = None;
    // Conditional-sum guard (if_convert output): the Select wrapping the
    // accumulator update. `s' = Select(cond, s + x, s)` — the guard Cmp's
    // value id; validated after the added-value chain resolves.  The
    // operator may ALSO be written the other way round,
    // `s' = Select(cond, s, s + x)` (the `if (a[i] & 1) continue; s +=
    // a[i];` shape, where the kept value is on the TRUE arm).  Both denote
    // a guarded update; only the add-on-TRUE arm is expressible by the
    // masked widening intrinsic, so a swapped guard is recorded and later
    // rejected.
    let mut guard_select_cond: Option<Value> = None;
    let mut guard_rhs: Option<Operand> = None;
    let mut guard_swapped = false;
    for &block_idx in &loop_info.body {
        if block_idx == header_idx {
            continue;
        }
        let block = &func.blocks[block_idx];
        if debug {
            eprintln!(
                "[VEC-RED]   Searching block {} for accumulator update (accumulator_phi = {})",
                block_idx, accumulator_phi.0
            );
            eprintln!(
                "[VEC-RED]   Accumulator-derived values: {:?}",
                accumulator_derived
            );
            for (idx, inst) in block.instructions.iter().enumerate() {
                eprintln!("[VEC-RED]     {}: {:?}", idx, inst);
            }
        }
        for (idx, inst) in block.instructions.iter().enumerate() {
            // Max reduction: the accumulator update IS the Select.
            if is_max_reduction {
                if let Instruction::Select { dest, .. } = inst {
                    if Some(*dest) == max_select_val {
                        body_idx = Some(block_idx);
                        accumulator_add_idx = Some(idx);
                        add_result = Some(*dest);
                        break;
                    }
                }
                continue;
            }
            if let Instruction::BinOp {
                dest,
                op: IrBinOp::Add,
                lhs,
                rhs,
                ..
            } = inst
            {
                // Check if this is accumulator += something (allowing casts)
                let lhs_is_acc = if let Operand::Value(v) = lhs {
                    accumulator_derived.contains(v)
                } else {
                    false
                };
                let rhs_is_acc = if let Operand::Value(v) = rhs {
                    accumulator_derived.contains(v)
                } else {
                    false
                };

                if lhs_is_acc || rhs_is_acc {
                    body_idx = Some(block_idx);
                    accumulator_add_idx = Some(idx);
                    add_result = Some(*dest);
                    // Conditional-sum detection: if this Add's result feeds
                    // a Select whose other arm is the accumulator phi, the
                    // update is GUARDED. Record the guard's cond value; the
                    // plain unguarded path must NOT run (it would drop the
                    // guard and miscompile), so validation failure later
                    // rejects the loop entirely.
                    for sin in block.instructions.iter().skip(idx + 1) {
                        if let Instruction::Select {
                            cond,
                            true_val,
                            false_val,
                            ..
                        } = sin
                        {
                            let tv_is_add = matches!(true_val, Operand::Value(v) if v.0 == dest.0);
                            let fv_is_phi = matches!(
                                false_val,
                                Operand::Value(v) if v.0 == accumulator_phi.0
                            );
                            // Swapped form: `Select(cond, s, s + x)` (the
                            // `if (a[i] & 1) continue; s += a[i]` shape). The
                            // kept value is the phi, the guarded add is on the
                            // FALSE arm. Still a guarded update; record the
                            // cond and mark it swapped so the validator
                            // rejects the unexpressible form instead of
                            // silently dropping the guard.
                            let fv_is_add = matches!(
                                false_val,
                                Operand::Value(v) if v.0 == dest.0
                            );
                            let tv_is_phi = matches!(
                                true_val,
                                Operand::Value(v) if v.0 == accumulator_phi.0
                            );
                            if tv_is_add && fv_is_phi {
                                if let Operand::Value(cv) = cond {
                                    guard_select_cond = Some(*cv);
                                }
                                break;
                            }
                            if fv_is_add && tv_is_phi {
                                if let Operand::Value(cv) = cond {
                                    guard_select_cond = Some(*cv);
                                    guard_swapped = true;
                                }
                                break;
                            }
                        }
                    }
                    break;
                }
            }
        }
        if body_idx.is_some() {
            break;
        }
    }
    if body_idx.is_none() {
        if debug {
            eprintln!("[VEC-RED]   No accumulator update found");
        }
        return None;
    }
    let body_idx = body_idx?;
    let accumulator_add_idx = accumulator_add_idx?;
    let add_result = add_result?;
    let body = &func.blocks[body_idx];
    let add_inst = &body.instructions[accumulator_add_idx];

    // Max-reduction early return: the update is the Select; the non-phi arm
    // must be an I32 load whose GEP marches with the IV, and the marching
    // pointer's preheader init must start exactly at element iv_init
    // (coverage legality: the vector loop covers [c, c+4*iters), and the
    // remainder resumes from the marching pointer's position; a pointer
    // starting anywhere else would silently skip or re-read elements).
    if is_max_reduction {
        let Instruction::Select {
            true_val,
            false_val,
            ..
        } = add_inst
        else {
            return None;
        };
        let x_val = match (true_val, false_val) {
            (Operand::Value(tv), Operand::Value(fv)) => {
                if fv.0 == accumulator_phi.0 {
                    *tv
                } else if tv.0 == accumulator_phi.0 {
                    *fv
                } else {
                    return None;
                }
            }
            _ => return None,
        };
        let Some(x_inst) = body
            .instructions
            .iter()
            .find(|i| matches!(i.dest(), Some(d) if d.0 == x_val.0))
        else {
            return None;
        };
        let Instruction::Load {
            ptr,
            ty: IrType::I32,
            ..
        } = x_inst
        else {
            return None;
        };
        let array_gep = *ptr;
        // The access must be either an IV-indexed GEP or a marching-pointer
        // phi stepping exactly ONE element (4 bytes) per iteration — any
        // other stride would be silently re-scaled wrong by the vec_width
        // transform (levkropp's elem_size-extended gep_uses_iv, split into
        // a dedicated helper here to keep the shared gep_uses_iv signature
        // and its loop-invariant-base hardening untouched).
        let marches_one_element = || -> bool {
            for &bi in &loop_info.body {
                for pinst in &func.blocks[bi].instructions {
                    if let Instruction::Phi { dest, incoming, .. } = pinst {
                        if *dest != array_gep {
                            continue;
                        }
                        for (op, _) in incoming {
                            if let Operand::Value(v) = op {
                                let steps =
                                    func.blocks.iter().flat_map(|b| &b.instructions).any(|i| {
                                        matches!(i, Instruction::GetElementPtr {
                                        dest: d,
                                        base,
                                        offset: Operand::Const(c),
                                        ..
                                    } if *d == *v && base.0 == array_gep.0
                                        && c.to_i64() == Some(4))
                                    });
                                if steps {
                                    return true;
                                }
                            }
                        }
                    }
                }
            }
            false
        };
        if !gep_uses_iv(func, &loop_info.body, array_gep, iv, &iv_derived, 4)
            && !marches_one_element()
        {
            if debug {
                eprintln!("[VEC-RED]   Max-reduction array GEP doesn't use IV");
            }
            return None;
        }
        let latch_label = func.blocks[latch_idx].label;
        let mut iv_init = None;
        for inst in &header.instructions {
            if let Instruction::Phi { dest, incoming, .. } = inst {
                if *dest == iv {
                    for (op, lbl) in incoming {
                        if *lbl != latch_label {
                            if let Operand::Const(c) = op {
                                iv_init = c.to_i64();
                            }
                        }
                    }
                }
            }
        }
        let Some(iv_init) = iv_init else {
            if debug {
                eprintln!("[VEC-RED]   Max-reduction IV init is not a constant");
            }
            return None;
        };
        // STRICT shape requirement (levkropp's original check, kept strict
        // on purpose): the array access must be a marching-pointer phi whose
        // preheader incoming is a constant-offset GEP at exactly iv_init*4
        // bytes (element c). Coverage legality depends on it: the vector
        // loop covers elements [c, c + 4*iters) via the marching pointer,
        // and the remainder resumes at (iv_final - c)*4 relative to the
        // SAME element-c base. A direct IV-indexed GEP would be re-scaled
        // by vec_width in the transform and start reading at element c*4
        // instead of c — silently wrong for any c != 0 — so that shape is
        // rejected outright rather than special-cased.
        let mut ptr_init_ok = false;
        for inst in &header.instructions {
            if let Instruction::Phi { dest, incoming, .. } = inst {
                if *dest != array_gep {
                    continue;
                }
                for (op, lbl) in incoming {
                    if *lbl == latch_label {
                        continue;
                    }
                    let Operand::Value(init_v) = op else { continue };
                    let init_gep = func
                        .blocks
                        .iter()
                        .flat_map(|b| &b.instructions)
                        .find_map(|i| {
                            if let Instruction::GetElementPtr {
                                dest,
                                offset: Operand::Const(off),
                                ..
                            } = i
                            {
                                if *dest == *init_v {
                                    return off.to_i64();
                                }
                            }
                            None
                        });
                    if init_gep == Some(iv_init * 4) {
                        ptr_init_ok = true;
                    }
                }
            }
        }
        if !ptr_init_ok {
            if debug {
                eprintln!("[VEC-RED]   Max-reduction pointer/IV init mismatch");
            }
            return None;
        }
        if debug {
            eprintln!("[VEC-RED]   Max reduction detected: mx = max(mx, load(arr[iv]))");
        }
        return Some(ReductionPattern {
            kind: ReductionKind::Max,
            seconds: Vec::new(),
            guard_cond: None,
            guard_rhs: None,
            element_type: IrType::I32,
            accumulator_type: IrType::I32,
            header_idx,
            body_idx,
            latch_idx,
            exit_idx,
            iv,
            iv_inc_idx,
            accumulator_phi,
            array_a_gep: array_gep,
            array_b_gep: None,
            accumulator_add_idx,
            limit,
            exit_cmp_inst_idx,
            exit_cmp_dest,
            loop_blocks: loop_info.body.clone(),
        });
    }

    // Get the element type from the add instruction
    let element_type = match add_inst {
        Instruction::BinOp { ty, .. } => *ty,
        _ => return None,
    };

    // Verify element type is vectorizable (F64, F32, I32, I64)
    if !matches!(
        element_type,
        IrType::F64 | IrType::F32 | IrType::I32 | IrType::I64
    ) {
        if debug {
            eprintln!("[VEC-RED]   Unsupported element type: {:?}", element_type);
        }
        set_reject("accumulator element type is not F64/F32/I32/I64");
        return None;
    }

    // Determine what is being added to the accumulator
    let (lhs_val, rhs_val) = match add_inst {
        Instruction::BinOp { lhs, rhs, .. } => {
            let lhs_v = if let Operand::Value(v) = lhs {
                Some(*v)
            } else {
                None
            };
            let rhs_v = if let Operand::Value(v) = rhs {
                Some(*v)
            } else {
                None
            };
            (lhs_v, rhs_v)
        }
        _ => (None, None),
    };

    // The non-accumulator operand is what we're adding (check accumulator_derived set)
    let added_value = if lhs_val.is_some() && accumulator_derived.contains(&lhs_val.unwrap()) {
        rhs_val?
    } else if rhs_val.is_some() && accumulator_derived.contains(&rhs_val.unwrap()) {
        lhs_val?
    } else {
        if debug {
            eprintln!("[VEC-RED]   Add instruction doesn't use accumulator or derived value");
        }
        return None;
    };

    // Check if added_value is a load (simple sum) or multiply (dot product)
    let added_inst = find_inst_by_dest(body, added_value)?;

    if debug {
        eprintln!(
            "[VEC-RED]   Added value {} is produced by: {:?}",
            added_value.0, added_inst
        );
    }

    // Conditional-sum guard validation. A Select wrapping the accumulator
    // update was detected in the search; the guarded vector form is only
    // expressible when the guard compares the LOADED element (the root of
    // the added-value chain) with signed greater-than against any operand:
    //   Select(a[iv] > rhs, acc + sext(a[iv]), acc)
    // The added-value chain's root load: Load directly, or Cast(Load).
    // Anything else (or no Select at all when one was detected) stays
    // scalar — the unguarded vector form would silently drop the guard.
    let mut select_guard_cond: Option<Value> = None;
    if let Some(gcv) = guard_select_cond {
        // Swapped form (`Select(cond, s, s + x)`): the kept value is on the
        // TRUE arm, so the guarded add is conditional on the NOT-taken path.
        // The masked widening intrinsic only encodes add-when-cond-true, so
        // this shape must stay scalar. Reject before the loader-root check
        // (which would otherwise reject it anyway, but with a misleading
        // message about `loaded > rhs`).
        if guard_swapped {
            if debug {
                eprintln!(
                    "[VEC-RED]   Rejecting: conditional-sum guard has the add on the FALSE arm (staying scalar)"
                );
            }
            set_reject("conditional-sum guard has the add on the false arm (not expressible)");
            return None;
        }
        // Resolve the chain root: the loaded I32 element.
        let chain_root = match added_inst {
            Instruction::Load { dest, .. } => Some(*dest),
            Instruction::Cast {
                src: Operand::Value(sv),
                ..
            } => {
                // The cast's source must be (or feed) the load; accept the
                // direct-load case and one intervening cast chain link.
                if let Some(src_inst) = find_inst_by_dest(body, *sv) {
                    match src_inst {
                        Instruction::Load { dest, .. } => Some(*dest),
                        _ => None,
                    }
                } else {
                    None
                }
            }
            _ => None,
        };
        let mut valid = false;
        if let Some(root_load) = chain_root {
            'gc: for &bi in &loop_info.body {
                for inst in &func.blocks[bi].instructions {
                    if let Instruction::Cmp {
                        dest, op, lhs, rhs, ..
                    } = inst
                    {
                        if dest.0 == gcv.0
                            && *op == IrCmpOp::Sgt
                            && matches!(lhs, Operand::Value(lv) if lv.0 == root_load.0)
                        {
                            select_guard_cond = Some(gcv);
                            guard_rhs = Some(*rhs);
                            valid = true;
                            break 'gc;
                        }
                    }
                }
            }
        }
        if !valid {
            if debug {
                eprintln!(
                    "[VEC-RED]   Rejecting: conditional-sum guard is not `loaded > rhs` (staying scalar)"
                );
            }
            set_reject("conditional-sum guard shape not expressible (needs loaded > rhs)");
            return None;
        }
    }

    let mut primary_loads: Vec<Value> = Vec::new();
    let mut pattern = match added_inst {
        // Simple sum pattern: sum += arr[i]
        Instruction::Load { ptr, ty, .. } => {
            let array_gep = *ptr;
            // Verify GEP uses IV (canonical unit stride)
            if !gep_uses_iv(
                func,
                &loop_info.body,
                array_gep,
                iv,
                &iv_derived,
                reduction_element_size(*ty).unwrap_or(0),
            ) {
                if debug {
                    eprintln!("[VEC-RED]   Array GEP doesn't use IV");
                }
                set_reject("array index is not based on the loop induction variable");
                return None;
            }

            if debug {
                eprintln!(
                    "[VEC-RED]   Simple sum pattern detected: {:?} += load(arr[iv])",
                    element_type
                );
            }

            primary_loads = vec![added_value];

            ReductionPattern {
                kind: ReductionKind::Sum,
                element_type,
                accumulator_type: element_type,
                header_idx,
                body_idx,
                latch_idx,
                exit_idx,
                iv,
                iv_inc_idx,
                accumulator_phi,
                array_a_gep: array_gep,
                array_b_gep: None,
                accumulator_add_idx,
                limit,
                exit_cmp_inst_idx,
                exit_cmp_dest,
                loop_blocks: loop_info.body.clone(),
                seconds: Vec::new(),
                guard_cond: select_guard_cond,
                guard_rhs,
            }
        }

        // Handle cast followed by load (e.g., long sum += (long)arr[i] where arr is int[])
        Instruction::Cast {
            src,
            from_ty,
            to_ty,
            ..
        } => {
            // The cast should be widening the element type to match the accumulator
            if *to_ty != element_type {
                if debug {
                    eprintln!(
                        "[VEC-RED]   Cast type mismatch: cast to {:?} but accumulator is {:?}",
                        to_ty, element_type
                    );
                }
                return None;
            }

            // Check if the source of the cast is a load
            let cast_src_val = if let Operand::Value(v) = src {
                *v
            } else {
                return None;
            };
            let cast_src_inst = find_inst_by_dest(body, cast_src_val)?;

            if let Instruction::Load {
                ptr, ty: load_ty, ..
            } = cast_src_inst
            {
                if *load_ty != *from_ty {
                    if debug {
                        eprintln!(
                            "[VEC-RED]   Load type {:?} doesn't match cast from_ty {:?}",
                            load_ty, from_ty
                        );
                    }
                    return None;
                }

                let array_gep = *ptr;
                // Verify GEP uses IV (canonical unit stride)
                if !gep_uses_iv(
                    func,
                    &loop_info.body,
                    array_gep,
                    iv,
                    &iv_derived,
                    reduction_element_size(*load_ty).unwrap_or(0),
                ) {
                    if debug {
                        eprintln!("[VEC-RED]   Array GEP doesn't use IV");
                    }
                    return None;
                }

                if debug {
                    eprintln!(
                        "[VEC-RED]   Simple sum pattern with cast detected: {:?} += ({:?})load(arr[iv])",
                        element_type, from_ty
                    );
                }

                // The cast must be a redundant no-op (from_ty == accumulator
                // type). Anything else — widening (`long += int`), narrowing,
                // or cross-kind (`float += int`, `int += float`) — changes the
                // per-element add semantics in a way a single-lane-type packed
                // add cannot reproduce. The old check only rejected widening,
                // so `float s += (float)int_arr[i]` slipped through, vectorized
                // as I32 adds on a F32 accumulator, and returned 0.0.
                //
                // ONE exception: signed I32 -> I64 widening when the target
                // provides a true widening load+add. AArch64 NEON
                // VecLoadWidenI32ToI64x2 (ldr + saddlp/smlal) and x86
                // VecWidenAddI32x4ToI64x2 (vmovdqu + vpmovsxdq×2 +
                // vextracti128 + paddq×2) both keep full I64 precision per
                // lane. allow_widening_i32 is set for both targets; the x86
                // AVX2 path additionally requires the AVX feature for
                // vpmovsxdq's VEX encoding.
                let allowed_widen =
                    allow_widening_i32 && element_type == IrType::I64 && *from_ty == IrType::I32;
                if *from_ty != element_type && !allowed_widen {
                    if debug {
                        eprintln!(
                            "[VEC-RED]   Rejecting: cast from {:?} to accumulator type {:?} (only redundant casts are vectorizable)",
                            from_ty, element_type
                        );
                    }
                    set_reject("widening/narrowing reduction cast not vectorizable");
                    return None;
                }

                primary_loads = vec![cast_src_val];

                ReductionPattern {
                    kind: ReductionKind::Sum,
                    element_type: *from_ty, // Use the actual array element type
                    accumulator_type: element_type,
                    header_idx,
                    body_idx,
                    latch_idx,
                    exit_idx,
                    iv,
                    iv_inc_idx,
                    accumulator_phi,
                    array_a_gep: array_gep,
                    array_b_gep: None,
                    accumulator_add_idx,
                    limit,
                    exit_cmp_inst_idx,
                    exit_cmp_dest,
                    loop_blocks: loop_info.body.clone(),
                    seconds: Vec::new(),
                    guard_cond: select_guard_cond,
                    guard_rhs,
                }
            } else {
                if debug {
                    eprintln!("[VEC-RED]   Cast source is not a load: {:?}", cast_src_inst);
                }
                return None;
            }
        }

        // Dot product pattern: sum += a[i] * b[i]
        Instruction::BinOp {
            op: IrBinOp::Mul,
            lhs,
            rhs,
            ..
        } => {
            // Both operands of multiply should be loads, or (NEON i32→i64 dot)
            // sign-extension casts of i32 loads: (long)a[i] * (long)b[i].
            let mul_lhs_val = if let Operand::Value(v) = lhs {
                *v
            } else {
                return None;
            };
            let mul_rhs_val = if let Operand::Value(v) = rhs {
                *v
            } else {
                return None;
            };

            let mul_lhs_inst = find_inst_by_dest(body, mul_lhs_val)?;
            let mul_rhs_inst = find_inst_by_dest(body, mul_rhs_val)?;

            // Resolve one multiply operand to its array GEP, tracking whether
            // it came through an i32→i64 sign-extension cast of a load.
            let mut widened_i32 = false;
            let operand_gep = |inst: &Instruction, widened: &mut bool| -> Option<Value> {
                match inst {
                    Instruction::Load { ptr, .. } => Some(*ptr),
                    Instruction::Cast {
                        src,
                        from_ty: IrType::I32,
                        to_ty: IrType::I64,
                        ..
                    } if neon => {
                        let src_val = if let Operand::Value(v) = src {
                            *v
                        } else {
                            return None;
                        };
                        if let Some(Instruction::Load { ptr, .. }) =
                            find_inst_by_dest(body, src_val)
                        {
                            *widened = true;
                            Some(*ptr)
                        } else {
                            None
                        }
                    }
                    _ => None,
                }
            };

            let array_a_gep = operand_gep(mul_lhs_inst, &mut widened_i32)?;
            let array_b_gep = operand_gep(mul_rhs_inst, &mut widened_i32)?;

            // A widened dot must widen BOTH operands (C usual arithmetic
            // conversions) and accumulate in i64; mixed forms are rejected.
            let (element_type, accumulator_type) = if widened_i32 {
                let both_widened = matches!(mul_lhs_inst, Instruction::Cast { .. })
                    && matches!(mul_rhs_inst, Instruction::Cast { .. });
                if !both_widened || element_type != IrType::I64 {
                    if debug {
                        eprintln!("[VEC-RED]   Rejecting partially widened dot product");
                    }
                    return None;
                }
                (IrType::I32, IrType::I64)
            } else {
                (element_type, element_type)
            };

            // Verify both GEPs use IV (canonical unit stride)
            if !gep_uses_iv(
                func,
                &loop_info.body,
                array_a_gep,
                iv,
                &iv_derived,
                reduction_element_size(element_type).unwrap_or(0),
            ) || !gep_uses_iv(
                func,
                &loop_info.body,
                array_b_gep,
                iv,
                &iv_derived,
                reduction_element_size(element_type).unwrap_or(0),
            ) {
                if debug {
                    eprintln!("[VEC-RED]   Array GEPs don't use IV");
                }
                set_reject("array index is not based on the loop induction variable");
                return None;
            }

            if debug {
                eprintln!(
                    "[VEC-RED]   Dot product pattern detected: {:?} += load(a[iv]) * load(b[iv])",
                    element_type
                );
            }

            primary_loads = vec![mul_lhs_val, mul_rhs_val];

            ReductionPattern {
                kind: ReductionKind::DotProduct,
                element_type,
                accumulator_type,
                header_idx,
                body_idx,
                latch_idx,
                exit_idx,
                iv,
                iv_inc_idx,
                accumulator_phi,
                array_a_gep,
                array_b_gep: Some(array_b_gep),
                accumulator_add_idx,
                limit,
                exit_cmp_inst_idx,
                exit_cmp_dest,
                loop_blocks: loop_info.body.clone(),
                seconds: Vec::new(),
                guard_cond: select_guard_cond,
                guard_rhs,
            }
        }

        _ => {
            if debug {
                eprintln!("[VEC-RED]   Unsupported accumulator update pattern");
            }
            set_reject("accumulator update is not sum += x[i] or sum += x[i]*y[i]");
            return None;
        }
    };

    // Multi-reduction: analyze every extra zero-init phi as an additional,
    // fully independent accumulator.  Each must match the primary's kind/type/
    // body block and be disjoint from every previously accepted chain, or the
    // whole loop stays scalar (same as the historical single-reduction
    // behavior).  `prior_derived` is the running union of accepted chains.
    {
        let mut prior_derived = accumulator_derived.clone();
        for &secondary_phi in &other_zero_phis {
            let sec = analyze_secondary_accumulator(
                func,
                loop_info,
                cfg,
                &pattern,
                &prior_derived,
                secondary_phi,
            )?;
            if debug {
                eprintln!(
                    "[VEC-RED]   Multi-reduction: extra accumulator phi {} (add @{} in body {})",
                    sec.accumulator_phi.0, sec.accumulator_add_idx, body_idx,
                );
            }
            prior_derived.extend(sec.accumulator_derived.iter().copied());
            pattern.seconds.push(sec);
        }
    }

    // One soundness check over the union of accumulators.  With a single
    // accumulator this is exactly the historical per-kind check (same add,
    // phi, derived set and loads); with more it additionally permits each
    // extra accumulator's loads and add.
    {
        let mut adds = vec![(pattern.accumulator_add_idx, add_result)];
        let mut phis = vec![pattern.accumulator_phi];
        let mut derived = accumulator_derived.clone();
        let mut loads = primary_loads.clone();
        for sec in &pattern.seconds {
            adds.push((sec.accumulator_add_idx, sec.add_result));
            phis.push(sec.accumulator_phi);
            derived.extend(sec.accumulator_derived.iter().copied());
            loads.extend(sec.loads.iter().copied());
        }
        if !reduction_pattern_is_sound(
            func,
            cfg,
            &loop_info.body,
            header_idx,
            body_idx,
            &adds,
            &phis,
            &derived,
            &loads,
            iv,
        ) {
            if debug {
                eprintln!("[VEC-RED]   Rejecting unsound reduction");
            }
            set_reject("reduction rejected: side effects / control flow / foreign memory in loop");
            return None;
        }
    }

    Some(pattern)
}

/// Analyze a second, independent reduction accumulator sharing the primary's
/// loop.  Returns None (fail-closed) unless the accumulator is a standalone
/// `sum += x[i]` / `sum += x[i] * y[i]` chain of the same kind and type in the
/// same body block, whose add reads neither the primary accumulator chain nor
/// any state the primary writes, and the union of both chains is sound.
///
/// Independence is the key constraint: the two accumulators may share loads
/// (`b += v[i]*w[i]` reuses the `v[i]` load of `a += u[i]*v[i]` — a load is
/// pure), but must not share any accumulator-derived SSA value.  A dependent
/// chain like Adler-32's `sum2 += sum1` is rejected here and stays scalar.
fn analyze_secondary_accumulator(
    func: &IrFunction,
    loop_info: &loop_analysis::NaturalLoop,
    cfg: &CfgAnalysis,
    primary: &ReductionPattern,
    primary_derived: &FxHashSet<Value>,
    secondary_phi: Value,
) -> Option<SecondaryAccumulator> {
    let debug = std::env::var("LCCC_DEBUG_VECTORIZE").is_ok();

    // Widening multi-reductions are not supported: they would need two
    // widening load+add chains and the x86 paths have no such form.  The
    // historical behavior (reject the whole loop) is preserved.
    if primary.element_type != primary.accumulator_type {
        if debug {
            eprintln!("[VEC-RED]   Rejecting multi-reduction: widening accumulator not supported");
        }
        set_reject("multi-reduction widening accumulator not supported");
        return None;
    }

    // Cast/copy closure of the secondary accumulator (mirrors the primary).
    let mut acc_derived: FxHashSet<Value> = FxHashSet::default();
    acc_derived.insert(secondary_phi);
    let mut changed = true;
    while changed {
        changed = false;
        for &block_idx in &loop_info.body {
            for inst in &func.blocks[block_idx].instructions {
                if let Instruction::Cast { dest, src, .. } | Instruction::Copy { dest, src } = inst
                {
                    if let Operand::Value(src_val) = src {
                        if acc_derived.contains(src_val) && !acc_derived.contains(dest) {
                            acc_derived.insert(*dest);
                            changed = true;
                        }
                    }
                }
            }
        }
    }

    // Independence: the two chains must not share any accumulator-derived
    // value.  A dependent reduction (`sum2 += sum1`) shares the primary phi
    // and is rejected here, keeping the loop scalar.
    if acc_derived.iter().any(|v| primary_derived.contains(v)) {
        if debug {
            eprintln!(
                "[VEC-RED]   Rejecting multi-reduction: dependent accumulator (sum2 += sum1)"
            );
        }
        set_reject("dependent reduction accumulator");
        return None;
    }

    let body = &func.blocks[primary.body_idx];

    // Locate the add that updates the secondary accumulator in the same body.
    let mut acc_add_idx = None;
    let mut add_result = None;
    for (idx, inst) in body.instructions.iter().enumerate() {
        if let Instruction::BinOp {
            dest,
            op: IrBinOp::Add,
            lhs,
            rhs,
            ty,
            ..
        } = inst
        {
            let lhs_is = matches!(lhs, Operand::Value(v) if acc_derived.contains(v));
            let rhs_is = matches!(rhs, Operand::Value(v) if acc_derived.contains(v));
            if !lhs_is && !rhs_is {
                continue;
            }
            if *ty != primary.accumulator_type {
                if debug {
                    eprintln!(
                        "[VEC-RED]   Rejecting multi-reduction: second accumulator type {:?} != {:?}",
                        ty, primary.accumulator_type
                    );
                }
                set_reject("second accumulator type mismatch");
                return None;
            }
            acc_add_idx = Some(idx);
            add_result = Some(*dest);
            break;
        }
    }
    let (acc_add_idx, add_result) = (acc_add_idx?, add_result?);

    // The non-accumulator operand is the added value; it must not be the
    // primary accumulator chain (another dependent-reduction check).
    let add_inst = &body.instructions[acc_add_idx];
    let (lhs_val, rhs_val) = match add_inst {
        Instruction::BinOp { lhs, rhs, .. } => (
            match lhs {
                Operand::Value(v) => Some(*v),
                _ => None,
            },
            match rhs {
                Operand::Value(v) => Some(*v),
                _ => None,
            },
        ),
        _ => return None,
    };
    let added_value = if lhs_val.is_some_and(|v| acc_derived.contains(&v)) {
        rhs_val?
    } else if rhs_val.is_some_and(|v| acc_derived.contains(&v)) {
        lhs_val?
    } else {
        return None;
    };
    if primary_derived.contains(&added_value) {
        set_reject("dependent reduction accumulator");
        return None;
    }
    let added_inst = find_inst_by_dest(body, added_value)?;

    // IV-derived closure (recomputed; same shape as the primary analyzer).
    let mut iv_derived: FxHashSet<Value> = FxHashSet::default();
    iv_derived.insert(primary.iv);
    let mut changed = true;
    while changed {
        changed = false;
        for &block_idx in &loop_info.body {
            for inst in &func.blocks[block_idx].instructions {
                if let Instruction::Cast { dest, src, .. } | Instruction::Copy { dest, src } = inst
                {
                    if let Operand::Value(src_val) = src {
                        if iv_derived.contains(src_val) && iv_derived.insert(*dest) {
                            changed = true;
                        }
                    }
                }
            }
        }
    }

    // Match the secondary shape to the primary kind.
    let (array_a_gep, array_b_gep, loads): (Value, Option<Value>, Vec<Value>) =
        match (primary.kind, added_inst) {
            (ReductionKind::Sum, Instruction::Load { ptr, ty, .. }) => {
                if *ty != primary.element_type {
                    set_reject("second accumulator element type mismatch");
                    return None;
                }
                if !gep_uses_iv(
                    func,
                    &loop_info.body,
                    *ptr,
                    primary.iv,
                    &iv_derived,
                    reduction_element_size(primary.element_type).unwrap_or(0),
                ) {
                    set_reject("second accumulator array index not based on IV");
                    return None;
                }
                (*ptr, None, vec![added_value])
            }
            (
                ReductionKind::DotProduct,
                Instruction::BinOp {
                    op: IrBinOp::Mul,
                    lhs,
                    rhs,
                    ty,
                    ..
                },
            ) => {
                if *ty != primary.accumulator_type {
                    set_reject("second accumulator element type mismatch");
                    return None;
                }
                let (Operand::Value(lv), Operand::Value(rv)) = (lhs, rhs) else {
                    return None;
                };
                let li = find_inst_by_dest(body, *lv)?;
                let ri = find_inst_by_dest(body, *rv)?;
                let (Instruction::Load { ptr: ap, .. }, Instruction::Load { ptr: bp, .. }) =
                    (li, ri)
                else {
                    set_reject("second dot product operands not loads");
                    return None;
                };
                if !gep_uses_iv(
                    func,
                    &loop_info.body,
                    *ap,
                    primary.iv,
                    &iv_derived,
                    reduction_element_size(primary.element_type).unwrap_or(0),
                ) || !gep_uses_iv(
                    func,
                    &loop_info.body,
                    *bp,
                    primary.iv,
                    &iv_derived,
                    reduction_element_size(primary.element_type).unwrap_or(0),
                ) {
                    set_reject("second dot product array index not based on IV");
                    return None;
                }
                (*ap, Some(*bp), vec![*lv, *rv])
            }
            _ => {
                if debug {
                    eprintln!(
                        "[VEC-RED]   Rejecting multi-reduction: second accumulator kind mismatch"
                    );
                }
                set_reject("second accumulator kind mismatch");
                return None;
            }
        };

    // The union soundness check (both adds, both phis, both derived sets,
    // both load sets) runs in the caller once the whole pattern is known.
    Some(SecondaryAccumulator {
        accumulator_phi: secondary_phi,
        add_result,
        accumulator_add_idx: acc_add_idx,
        array_a_gep,
        array_b_gep,
        accumulator_derived: acc_derived,
        loads,
    })
}

/// All (array_a_gep, array_b_gep) pairs of a reduction pattern, including the
/// optional second accumulator.  Every transform-wide sweep over "the arrays
/// of this loop" (contiguity precondition, byte-IV strength reduction, and
/// element-index GEP scaling) must cover both accumulators or the second one
/// would read at the wrong stride.
fn reduction_array_geps(pattern: &ReductionPattern) -> Vec<(Value, Option<Value>)> {
    let mut geps = vec![(pattern.array_a_gep, pattern.array_b_gep)];
    for sec in &pattern.seconds {
        geps.push((sec.array_a_gep, sec.array_b_gep));
    }
    geps
}

/// Is `op` a register-resident vector load (the `VecLoad*` family)?  These
/// have `dest: Some`, `dest_ptr: None`, and read memory through args[0..2]
/// (`base` + byte offset in the byte-IV form, or `GEP + 0` in the
/// element-index form).
fn is_vector_load_op(op: IntrinsicOp) -> bool {
    use IntrinsicOp as O;
    matches!(
        op,
        O::VecLoadF64x4
            | O::VecLoadF64x2
            | O::VecLoadI32x8
            | O::VecLoadI32x4
            | O::VecLoadF32x8
            | O::VecLoadF32x4
            | O::VecLoadWidenI32ToI64x2
            | O::VecLoadI64x2
            | O::VecLoadI64x4
    )
}

/// Canonical address identity of a vector-load operand pair.
///
/// The byte-IV form is `(base, byte_iv)` and is already canonical.  The
/// element-index form is `(gep, 0)`; resolve the GEP to `(base, offset)` —
/// tracing the transform-inserted `mul(orig, vec_width)` back to the original
/// element offset — so two loads of the same array share a key even when the
/// frontend duplicated the GEP into distinct SSA ids.
/// Canonical identity of a load's base pointer.  Distinct `GlobalAddr` SSA
/// values naming the SAME symbol are one object (the frontend emits a fresh
/// `GlobalAddr` per source use, and GlobalAddr CSE deliberately keeps
/// variable-index bases site-local), so they canonicalize to the symbol name.
/// Every other pointer is identified by its SSA value, which is already
/// object-unique for allocas and parameters.
#[derive(Clone, PartialEq, Eq, Hash)]
enum LoadBaseKey {
    Symbol(String),
    Value(u32),
}

fn canonical_load_base(defs: &FxHashMap<u32, &Instruction>, base: Value) -> LoadBaseKey {
    match defs.get(&base.0) {
        Some(Instruction::GlobalAddr { name, .. }) => LoadBaseKey::Symbol(name.clone()),
        _ => LoadBaseKey::Value(base.0),
    }
}

fn vector_load_key(
    body: &BasicBlock,
    defs: &FxHashMap<u32, &Instruction>,
    base: &Operand,
    off: &Operand,
) -> Option<(LoadBaseKey, u32)> {
    match (base, off) {
        (Operand::Value(b), Operand::Value(o)) => Some((canonical_load_base(defs, *b), o.0)),
        (Operand::Value(gep), Operand::Const(_)) => {
            let g = find_inst_by_dest(body, *gep)?;
            if let Instruction::GetElementPtr {
                base: gb,
                offset: Operand::Value(go),
                ..
            } = g
            {
                let orig = match find_inst_by_dest(body, *go) {
                    Some(Instruction::BinOp {
                        op: IrBinOp::Mul,
                        lhs,
                        rhs,
                        ..
                    }) => match (lhs, rhs) {
                        (Operand::Value(v), Operand::Const(_))
                        | (Operand::Const(_), Operand::Value(v)) => v.0,
                        _ => go.0,
                    },
                    _ => go.0,
                };
                Some((canonical_load_base(defs, *gb), orig))
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Merge duplicate vector loads inside a transformed reduction loop.
///
/// Two `VecLoad*` intrinsics with the same op and the same canonical address
/// read identical bytes, and the reduction soundness check forbids any store
/// between them, so a shared array — `b += v*w` after `a += u*v`, or
/// `sum += x*x` — needs only one load.  Uses of the duplicate SSA value are
/// rewritten to the canonical (earlier) load, which dominates them, and the
/// duplicate instruction is removed.
fn deduplicate_vector_loads(func: &mut IrFunction, loop_blocks: &FxHashSet<usize>) -> usize {
    // Function-wide def map: the load bases (`GlobalAddr`/`ParamRef` roots)
    // are commonly defined in the preheader or entry block, outside the loop
    // the loads live in.
    let mut defs: FxHashMap<u32, &Instruction> = FxHashMap::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Some(dest) = inst.dest() {
                defs.insert(dest.0, inst);
            }
        }
    }

    let mut seen: FxHashMap<(IntrinsicOp, LoadBaseKey, u32), Value> = FxHashMap::default();
    let mut replace: FxHashMap<u32, u32> = FxHashMap::default();
    let mut removals: FxHashMap<usize, Vec<usize>> = FxHashMap::default();

    for &bi in loop_blocks {
        let block = &func.blocks[bi];
        for (ii, inst) in block.instructions.iter().enumerate() {
            let Instruction::Intrinsic {
                dest: Some(d),
                op,
                dest_ptr: None,
                args,
            } = inst
            else {
                continue;
            };
            if !is_vector_load_op(*op) || args.len() != 2 {
                continue;
            }
            let Some(key) = vector_load_key(block, &defs, &args[0], &args[1]) else {
                continue;
            };
            let full = (*op, key.0, key.1);
            match seen.entry(full) {
                std::collections::hash_map::Entry::Vacant(entry) => {
                    entry.insert(*d);
                }
                std::collections::hash_map::Entry::Occupied(entry) => {
                    let canonical = *entry.get();
                    replace.insert(d.0, canonical.0);
                    removals.entry(bi).or_default().push(ii);
                }
            }
        }
    }

    if replace.is_empty() {
        return 0;
    }

    // Rewrite uses function-wide; every use of the duplicate is dominated by
    // the earlier canonical load in the same block.
    for block in func.blocks.iter_mut() {
        for inst in &mut block.instructions {
            crate::passes::tail_call_elim::replace_values_in_inst(inst, &replace);
        }
        match &mut block.terminator {
            Terminator::CondBranch { cond, .. } => {
                if let Operand::Value(v) = cond {
                    if let Some(&to) = replace.get(&v.0) {
                        *v = Value(to);
                    }
                }
            }
            Terminator::Switch { val, .. } => {
                if let Operand::Value(v) = val {
                    if let Some(&to) = replace.get(&v.0) {
                        *v = Value(to);
                    }
                }
            }
            Terminator::Return(Some(Operand::Value(v))) => {
                if let Some(&to) = replace.get(&v.0) {
                    *v = Value(to);
                }
            }
            _ => {}
        }
    }

    // Remove the duplicate instructions (and keep source spans in lockstep).
    let mut total = 0usize;
    for (bi, mut idxs) in removals {
        idxs.sort_unstable();
        idxs.dedup();
        let block = &mut func.blocks[bi];
        let mut next = idxs.iter().copied();
        let mut next_remove = next.next();
        let mut idx = 0usize;
        block.instructions.retain(|_| {
            let cur = idx;
            idx += 1;
            if Some(cur) == next_remove {
                next_remove = next.next();
                false
            } else {
                true
            }
        });
        if !block.source_spans.is_empty() {
            let mut next2 = idxs.iter().copied();
            let mut next_remove2 = next2.next();
            let mut idx2 = 0usize;
            block.source_spans.retain(|_| {
                let cur = idx2;
                idx2 += 1;
                if Some(cur) == next_remove2 {
                    next_remove2 = next2.next();
                    false
                } else {
                    true
                }
            });
        }
        total += idxs.len();
    }
    total
}

/// Rewrite one reduction accumulator's body (scalar `+=` → register vector
/// chain) and rewire its header phi to the vector accumulator.  Shared by the
/// AVX2 and SSE2 transforms; the single-accumulator fast path is intentionally
/// NOT routed through here so its emitted IR stays bit-identical.
///
/// Returns `(init_zero_value, vec_sum_value, change_count)`.  The caller must
/// process accumulators in DESCENDING `add_idx` order: inserting/removing
/// instructions at a higher index never shifts a lower one still to patch.
#[allow(clippy::too_many_arguments)]
fn rewrite_reduction_body(
    func: &mut IrFunction,
    header_idx: usize,
    body_idx: usize,
    latch_idx: usize,
    kind: ReductionKind,
    use_byte_iv: bool,
    vec_load_op: IntrinsicOp,
    vec_add_op: IntrinsicOp,
    vec_zero_op: IntrinsicOp,
    vec_mul_op: Option<IntrinsicOp>,
    use_fma: bool,
    vec_fma_op: Option<IntrinsicOp>,
    phi: Value,
    add_idx: usize,
    array_a_gep: Value,
    array_b_gep: Option<Value>,
    byte_iv_a: Option<(Value, Value)>,
    byte_iv_b: Option<(Value, Value)>,
    next_val_id: &mut u32,
) -> (Value, Value, usize) {
    // SSA ids in the same order the single-accumulator paths allocate them:
    // init_zero first (it lands in the entry block), then loads, then the sum.
    let init_zero_value = Value(*next_val_id);
    *next_val_id += 1;
    let vec_load_a = Value(*next_val_id);
    *next_val_id += 1;
    let vec_load_b = Value(*next_val_id);
    *next_val_id += 1;
    let vec_mul = Value(*next_val_id);
    *next_val_id += 1;
    let vec_sum_value = Value(*next_val_id);
    *next_val_id += 1;

    let mut changes = 0usize;

    // Initialize the vector accumulator to zero in the entry block.
    {
        let entry_block = &mut func.blocks[0];
        let zero_inst = Instruction::Intrinsic {
            dest: Some(init_zero_value),
            op: vec_zero_op,
            dest_ptr: None,
            args: vec![],
        };
        entry_block.instructions.push(zero_inst);
    }
    changes += 1;

    // Rewire this accumulator's phi: the zero-constant entry edge becomes the
    // vector zero; the backedge becomes the new vector sum.
    let latch_label = func.blocks[latch_idx].label;
    {
        let header_block = &mut func.blocks[header_idx];
        for inst in header_block.instructions.iter_mut() {
            if let Instruction::Phi { dest, incoming, .. } = inst {
                if *dest != phi {
                    continue;
                }
                for (val, label) in incoming.iter_mut() {
                    if matches!(
                        val,
                        Operand::Const(IrConst::F32(_))
                            | Operand::Const(IrConst::F64(_))
                            | Operand::Const(IrConst::I32(0))
                            | Operand::Const(IrConst::I64(0))
                            | Operand::Const(IrConst::Zero)
                    ) {
                        *val = Operand::Value(init_zero_value);
                    }
                    if *label == latch_label {
                        *val = Operand::Value(vec_sum_value);
                    }
                }
            }
        }
    }

    let (a_base, a_off) = match (use_byte_iv, &byte_iv_a) {
        (true, Some((base, off))) => (Operand::Value(*base), Operand::Value(*off)),
        _ => (Operand::Value(array_a_gep), Operand::Const(IrConst::I64(0))),
    };
    let (b_base, b_off) = match (use_byte_iv, &byte_iv_b) {
        (true, Some((base, off))) => (Operand::Value(*base), Operand::Value(*off)),
        _ => (
            Operand::Value(array_b_gep.unwrap_or(array_a_gep)),
            Operand::Const(IrConst::I64(0)),
        ),
    };

    let body_block = &mut func.blocks[body_idx];
    match kind {
        // Max never reaches the secondary-accumulator emitter: the Max
        // detector never records extra accumulators (seconds is empty).
        ReductionKind::Max => {
            unreachable!("max reductions never carry a secondary accumulator")
        }
        ReductionKind::Sum => {
            let load_inst = Instruction::Intrinsic {
                dest: Some(vec_load_a),
                op: vec_load_op,
                dest_ptr: None,
                args: vec![a_base, a_off],
            };
            let add_inst = Instruction::Intrinsic {
                dest: Some(vec_sum_value),
                op: vec_add_op,
                dest_ptr: None,
                args: vec![Operand::Value(phi), Operand::Value(vec_load_a)],
            };
            body_block.instructions.insert(add_idx, load_inst);
            body_block.instructions.insert(add_idx + 1, add_inst);
            body_block.instructions.remove(add_idx + 2);
            changes += 2;
        }
        ReductionKind::DotProduct => {
            if use_fma {
                let fma_inst = Instruction::Intrinsic {
                    dest: Some(vec_sum_value),
                    op: vec_fma_op.expect("dot product FMA requires a vector FMA op"),
                    dest_ptr: None,
                    args: vec![Operand::Value(phi), a_base, a_off, b_base, b_off],
                };
                body_block.instructions.insert(add_idx, fma_inst);
                body_block.instructions.remove(add_idx + 1);
                changes += 1;
            } else {
                let load_a_inst = Instruction::Intrinsic {
                    dest: Some(vec_load_a),
                    op: vec_load_op,
                    dest_ptr: None,
                    args: vec![a_base, a_off],
                };
                let load_b_inst = Instruction::Intrinsic {
                    dest: Some(vec_load_b),
                    op: vec_load_op,
                    dest_ptr: None,
                    args: vec![b_base, b_off],
                };
                let mul_inst = Instruction::Intrinsic {
                    dest: Some(vec_mul),
                    op: vec_mul_op.expect("dot product requires a vector multiply op"),
                    dest_ptr: None,
                    args: vec![Operand::Value(vec_load_a), Operand::Value(vec_load_b)],
                };
                let add_inst = Instruction::Intrinsic {
                    dest: Some(vec_sum_value),
                    op: vec_add_op,
                    dest_ptr: None,
                    args: vec![Operand::Value(phi), Operand::Value(vec_mul)],
                };
                body_block.instructions.insert(add_idx, load_a_inst);
                body_block.instructions.insert(add_idx + 1, load_b_inst);
                body_block.instructions.insert(add_idx + 2, mul_inst);
                body_block.instructions.insert(add_idx + 3, add_inst);
                body_block.instructions.remove(add_idx + 4);
                changes += 4;
            }
        }
    }

    (init_zero_value, vec_sum_value, changes)
}

/// Rewrite every use of `acc_id` OUTSIDE the reduction's loop blocks to
/// `replacement` (the scalar remainder result).  The vector accumulator only
/// lives inside the loop; every outside reader must see the reduced scalar.
/// Returns the number of rewritten uses (diagnostic only).
fn rewrite_accumulator_uses_outside_loop(
    func: &mut IrFunction,
    loop_blocks: &FxHashSet<usize>,
    acc_id: u32,
    replacement: Value,
) -> usize {
    let mut updates = 0usize;
    let mut replace_in_operand = |op: &mut Operand| -> bool {
        if let Operand::Value(v) = op {
            if v.0 == acc_id {
                *v = replacement;
                return true;
            }
        }
        false
    };
    for (bi, block) in func.blocks.iter_mut().enumerate() {
        if loop_blocks.contains(&bi) {
            continue; // the vector accumulator lives and is consumed here
        }
        for inst in &mut block.instructions {
            match inst {
                Instruction::Copy { src, .. } => {
                    if replace_in_operand(src) {
                        updates += 1;
                    }
                }
                Instruction::Store { val, .. } => {
                    if replace_in_operand(val) {
                        updates += 1;
                    }
                }
                Instruction::BinOp { lhs, rhs, .. } => {
                    if replace_in_operand(lhs) {
                        updates += 1;
                    }
                    if replace_in_operand(rhs) {
                        updates += 1;
                    }
                }
                Instruction::Cmp { lhs, rhs, .. } => {
                    if replace_in_operand(lhs) {
                        updates += 1;
                    }
                    if replace_in_operand(rhs) {
                        updates += 1;
                    }
                }
                Instruction::UnaryOp { src, .. } | Instruction::Cast { src, .. } => {
                    if replace_in_operand(src) {
                        updates += 1;
                    }
                }
                Instruction::Call { info, .. } | Instruction::CallIndirect { info, .. } => {
                    for a in &mut info.args {
                        if replace_in_operand(a) {
                            updates += 1;
                        }
                    }
                }
                Instruction::Phi { incoming, .. } => {
                    for (op, _) in incoming {
                        if replace_in_operand(op) {
                            updates += 1;
                        }
                    }
                }
                Instruction::Select {
                    cond,
                    true_val,
                    false_val,
                    ..
                } => {
                    if replace_in_operand(cond) {
                        updates += 1;
                    }
                    if replace_in_operand(true_val) {
                        updates += 1;
                    }
                    if replace_in_operand(false_val) {
                        updates += 1;
                    }
                }
                _ => {}
            }
        }
        match &mut block.terminator {
            Terminator::Return(Some(op)) => {
                if replace_in_operand(op) {
                    updates += 1;
                }
            }
            Terminator::CondBranch { cond, .. } => {
                if replace_in_operand(cond) {
                    updates += 1;
                }
            }
            Terminator::Switch { val, .. } => {
                if replace_in_operand(val) {
                    updates += 1;
                }
            }
            _ => {}
        }
    }
    updates
}

/// Identity of a pointer's statically-known base object.
///
/// Parameter roots are retained even when they are not `restrict`: this lets
/// `roots_proven_distinct` use the contract if either side is restrict while
/// correctly treating two ordinary pointer parameters as possibly aliasing.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ProvenObjectRoot {
    Global(String),
    Alloca(u32),
    Param { index: usize, noalias: bool },
}

/// Follow pointer-preserving IR operations to a global, local alloca, or
/// parameter. Loads and arbitrary arithmetic stop the proof. The walk is
/// deliberately bounded/cycle-safe because malformed copy cycles must make an
/// optimization fail closed, never hang compilation.
fn proven_object_root(func: &IrFunction, start: Value) -> Option<ProvenObjectRoot> {
    let mut cur = start;
    let mut seen = FxHashSet::default();
    for _ in 0..128 {
        if !seen.insert(cur) {
            return None;
        }
        let defining = func.blocks.iter().find_map(|block| {
            block
                .instructions
                .iter()
                .find(|inst| inst.dest() == Some(cur))
        })?;
        match defining {
            Instruction::GlobalAddr { name, .. } => {
                return Some(ProvenObjectRoot::Global(name.clone()));
            }
            Instruction::Alloca { dest, .. } => {
                return Some(ProvenObjectRoot::Alloca(dest.0));
            }
            Instruction::ParamRef { param_idx, .. } => {
                let noalias = func.params.get(*param_idx).is_some_and(|p| p.noalias);
                return Some(ProvenObjectRoot::Param {
                    index: *param_idx,
                    noalias,
                });
            }
            Instruction::GetElementPtr { base, .. } => cur = *base,
            Instruction::Copy {
                src: Operand::Value(src),
                ..
            }
            | Instruction::Cast {
                src: Operand::Value(src),
                ..
            } => cur = *src,
            _ => return None,
        }
    }
    None
}

/// C object identity and restrict rules sufficient for vectorization legality.
/// Distinct globals/allocas cannot overlap. Parameter pointers are only
/// disjoint when at least one carries `restrict`; otherwise different SSA
/// value numbers prove nothing (the old implementation made exactly that
/// unsound inference and miscompiled shifted in-place maps/matmuls).
fn roots_proven_distinct(a: &ProvenObjectRoot, b: &ProvenObjectRoot) -> bool {
    if a == b {
        return false;
    }
    match (a, b) {
        (
            ProvenObjectRoot::Param {
                noalias: a_noalias, ..
            },
            ProvenObjectRoot::Param {
                noalias: b_noalias, ..
            },
        ) => *a_noalias || *b_noalias,
        (ProvenObjectRoot::Param { noalias, .. }, _)
        | (_, ProvenObjectRoot::Param { noalias, .. }) => *noalias,
        _ => true,
    }
}

/// Pattern matching result for a vectorizable one-source store loop:
/// `dst[i] = src[i]`, `src[i] * scale`, `src[i] + offset`, or the full
/// `src[i] * scale + offset`, with loop-invariant scalar operands.
#[derive(Debug)]
struct MapPattern {
    /// Loop header block index
    header_idx: usize,
    /// Loop body block (contains the store)
    body_idx: usize,
    /// Loop latch block index
    latch_idx: usize,
    /// Exit block index
    exit_idx: usize,
    /// Induction variable (loop counter)
    iv: Value,
    /// Signed or unsigned 32/64-bit induction type.
    iv_ty: IrType,
    /// Element type shared by the source load and destination store.
    elem_ty: IrType,
    /// Loop limit value (N in `i < N`)
    limit: Operand,
    /// Normalized strict-less operation that tests the loop exit condition.
    exit_cmp_op: IrCmpOp,
    /// GEP for the destination store (dst[iv])
    dst_gep: Value,
    /// GEPs for the source load streams (src[iv]); all advance by the same
    /// byte induction variable.
    src_geps: Vec<Value>,
    /// The elementwise expression stored to dst[iv].
    expr: MapExpr,
    /// All block indices in the loop body
    loop_blocks: FxHashSet<usize>,
}

/// Elementwise map expression tree (OP-05a). Leaves are loop loads (by
/// stream index) or loop-invariant scalars; internal nodes are FP Add/Sub/
/// Mul/Div, integer Add/Mul, or FP Sqrt. The legacy affine family
/// (`src[i]`, `src[i]*s`, `src[i]+o`, `src[i]*s+o`) is the depth-1 subset of
/// this tree. Lane-exact by construction: every vector intrinsic computes
/// the same IEEE operation per lane as the scalar original; the only
/// semantics-affecting rewrite is the optional mul+add -> fused madd
/// contraction under `-ffp-contract=fast` (same contract as the affine
/// path).
#[derive(Debug, Clone)]
enum MapExpr {
    /// Load from source stream `i` (index into `MapPattern::src_geps`).
    Load(usize),
    /// Loop-invariant scalar operand (broadcast in the preheader).
    Invariant(Operand),
    /// Binary operation; `ty` was checked to equal the element type.
    BinOp(IrBinOp, Box<MapExpr>, Box<MapExpr>),
    /// Scalar sqrt over a subexpression (FP only).
    Sqrt(Box<MapExpr>),
}

impl MapExpr {
    fn node_count(&self) -> usize {
        match self {
            MapExpr::Load(_) | MapExpr::Invariant(_) => 1,
            MapExpr::BinOp(_, l, r) => 1 + l.node_count() + r.node_count(),
            MapExpr::Sqrt(x) => 1 + x.node_count(),
        }
    }
}

/// Emission context for the elementwise map tree (OP-05a). Owns the fresh
/// value counter and accumulates preheader broadcasts (hoisted, hoisted-once
/// per invariant operand) and the packed-body instruction list.
struct MapEmitCtx<'a> {
    src_bases: &'a [Value],
    byte_iv: Value,
    load_op: IntrinsicOp,
    broadcast_op: IntrinsicOp,
    sqrt_op: Option<IntrinsicOp>,
    madd_op: Option<IntrinsicOp>,
    bin_op: &'a dyn Fn(&IrBinOp) -> Option<IntrinsicOp>,
    broadcast_cache: Vec<(String, Value)>,
    preheader_insts: Vec<Instruction>,
    vec_insts: Vec<Instruction>,
    next_val_id: u32,
    changes: &'a mut usize,
}

impl<'a> MapEmitCtx<'a> {
    fn fresh(&mut self) -> Value {
        let v = Value(self.next_val_id);
        self.next_val_id += 1;
        v
    }

    fn emit(&mut self, expr: &MapExpr) -> Option<Value> {
        match expr {
            MapExpr::Load(stream) => {
                let dest = self.fresh();
                self.vec_insts.push(Instruction::Intrinsic {
                    dest: Some(dest),
                    op: self.load_op,
                    dest_ptr: None,
                    args: vec![
                        Operand::Value(self.src_bases[*stream]),
                        Operand::Value(self.byte_iv),
                    ],
                });
                Some(dest)
            }
            MapExpr::Invariant(operand) => {
                let key = format!("{:?}", operand);
                if let Some(&(_, cached)) = self.broadcast_cache.iter().find(|(k, _)| *k == key) {
                    return Some(cached);
                }
                let dest = self.fresh();
                // Broadcasts live in the PREHEADER so they are hoisted out
                // of the packed loop (register-allocated once).
                self.preheader_insts.push(Instruction::Intrinsic {
                    dest: Some(dest),
                    op: self.broadcast_op,
                    dest_ptr: None,
                    args: vec![operand.clone()],
                });
                self.broadcast_cache.push((key, dest));
                Some(dest)
            }
            MapExpr::Sqrt(x) => {
                let inner = self.emit(x)?;
                let sqrt_op = self.sqrt_op?;
                let dest = self.fresh();
                self.vec_insts.push(Instruction::Intrinsic {
                    dest: Some(dest),
                    op: sqrt_op,
                    dest_ptr: None,
                    args: vec![Operand::Value(inner)],
                });
                Some(dest)
            }
            MapExpr::BinOp(op, l, r) => {
                // Contract-legal fusion (same gate as the affine path): a
                // mul feeding an add fuses into the 3-operand madd when the
                // other addend is not itself a mul (a*b + c*d has no single
                // madd form).
                if let (Some(madd), IrBinOp::Add, MapExpr::BinOp(IrBinOp::Mul, ml, mr)) =
                    (self.madd_op, op, &**l)
                {
                    if !matches!(&**r, MapExpr::BinOp(IrBinOp::Mul, _, _)) {
                        let lv = self.emit(ml)?;
                        let rv = self.emit(mr)?;
                        let av = self.emit(r)?;
                        let dest = self.fresh();
                        self.vec_insts.push(Instruction::Intrinsic {
                            dest: Some(dest),
                            op: madd,
                            dest_ptr: None,
                            args: vec![Operand::Value(lv), Operand::Value(rv), Operand::Value(av)],
                        });
                        return Some(dest);
                    }
                }
                let lv = self.emit(l)?;
                let rv = self.emit(r)?;
                let vec_op = (self.bin_op)(op)?;
                let dest = self.fresh();
                self.vec_insts.push(Instruction::Intrinsic {
                    dest: Some(dest),
                    op: vec_op,
                    dest_ptr: None,
                    args: vec![Operand::Value(lv), Operand::Value(rv)],
                });
                Some(dest)
            }
        }
    }
}

/// Analyze a one-source copy/scale/add/affine store loop.
///
/// Strict legality: the loop must be straight-line (no internal conditionals),
/// contain exactly one load and one store, only simple non-trapping arithmetic,
/// and the two GEP base pointers must be provably distinct (separate globals
/// or allocas) so the vector store cannot clobber a not-yet-read source lane.
fn analyze_map_pattern(
    func: &IrFunction,
    loop_info: &loop_analysis::NaturalLoop,
    neon: bool,
) -> Option<MapPattern> {
    let debug = std::env::var("LCCC_DEBUG_VECTORIZE").is_ok();
    let header_idx = loop_info.header;
    let header = &func.blocks[header_idx];

    let exit_idx = find_exit(func, loop_info)?;
    let latch_idx = find_latch(func, loop_info)?;
    // The transform redirects the false edge to the scalar remainder.  Require
    // the canonical while-loop shape rather than guessing branch polarity.
    if !matches!(header.terminator,
        Terminator::CondBranch { false_label, .. }
            if false_label == func.blocks[exit_idx].label)
    {
        return None;
    }

    // Reject loops with internal conditionals (predication not supported).
    if loop_info.body.iter().copied().any(|block_idx| {
        block_idx != header_idx
            && matches!(
                func.blocks[block_idx].terminator,
                Terminator::CondBranch { .. }
            )
    }) {
        return None;
    }

    // Identify the induction phi from the header exit comparison.
    let mut iv = None;
    for inst in &header.instructions {
        let Instruction::Phi { dest, incoming, ty } = inst else {
            continue;
        };
        if incoming.len() != 2
            || !matches!(*ty, IrType::I32 | IrType::U32 | IrType::I64 | IrType::U64)
        {
            continue;
        };
        let mut derived = FxHashSet::default();
        derived.insert(*dest);
        for candidate in &header.instructions {
            if let Instruction::Cast { dest, src, .. } | Instruction::Copy { dest, src } = candidate
            {
                if matches!(src, Operand::Value(v) if derived.contains(v)) {
                    derived.insert(*dest);
                }
            }
        }
        if header.instructions.iter().any(|candidate| {
            matches!(candidate,
            Instruction::Cmp { lhs, rhs, .. }
                if matches!(lhs, Operand::Value(v) if derived.contains(v)))
        }) {
            iv = Some((*dest, *ty));
            break;
        }
    }
    let (iv, iv_ty) = iv?;

    // IV-derived values across the loop (casts/copies), fixed-point.
    let mut iv_derived = FxHashSet::default();
    iv_derived.insert(iv);
    let mut changed = true;
    while changed {
        changed = false;
        for &block_idx in &loop_info.body {
            for inst in &func.blocks[block_idx].instructions {
                if let Instruction::Cast { dest, src, .. } | Instruction::Copy { dest, src } = inst
                {
                    if let Operand::Value(src_val) = src {
                        if iv_derived.contains(src_val) && iv_derived.insert(*dest) {
                            changed = true;
                        }
                    }
                }
            }
        }
    }

    // Exit comparison and limit.
    let mut exit_cmp_info = None;
    for inst in &header.instructions {
        if let Instruction::Cmp {
            op, lhs, rhs, ty, ..
        } = inst
        {
            if *ty != iv_ty {
                continue;
            }
            if let Operand::Value(lhs_val) = lhs {
                if iv_derived.contains(lhs_val) && matches!(*op, IrCmpOp::Slt | IrCmpOp::Ult) {
                    exit_cmp_info = Some((*op, rhs.clone()));
                    break;
                }
            } else if let Operand::Value(rhs_val) = rhs {
                if iv_derived.contains(rhs_val) {
                    let normalized = match op {
                        IrCmpOp::Sgt => Some(IrCmpOp::Slt),
                        IrCmpOp::Ugt => Some(IrCmpOp::Ult),
                        _ => None,
                    };
                    if let Some(normalized) = normalized {
                        exit_cmp_info = Some((normalized, lhs.clone()));
                        break;
                    }
                }
            }
        }
    }
    let (exit_cmp_op, limit) = exit_cmp_info?;
    if matches!(&limit, Operand::Value(v)
        if find_inst_in_loop(func, &loop_info.body, *v).is_some())
    {
        set_reject("map trip count is not loop-invariant");
        return None;
    }

    // Constant trip counts of 4 or fewer are better left scalar.
    if let Operand::Const(c) = &limit {
        if c.to_i64().map_or(false, |n| n <= 4) {
            return None;
        }
    }

    // IV increment by 1 must exist in the latch.
    let latch = &func.blocks[latch_idx];
    let has_unit_increment = latch.instructions.iter().any(|inst| {
        matches!(inst, Instruction::BinOp { op: IrBinOp::Add, lhs, rhs, .. }
            if matches!(lhs, Operand::Value(v) if *v == iv)
                && matches!(rhs, Operand::Const(c) if c.to_i64() == Some(1)))
    });
    if !has_unit_increment {
        return None;
    }

    // Scan the loop for loads/stores: exactly one store, up to four source
    // load streams, and every other instruction must be simple,
    // side-effect-free arithmetic. Integer div/rem can fault and keep the
    // loop scalar; FP division is IEEE-defined elementwise (no trap) and is
    // parsed into the map expression tree.
    const MAP_MAX_STREAMS: usize = 4;
    let mut load_infos: Vec<(usize, Value, Value, IrType)> = Vec::new();
    let mut store_info = None; // (block, ptr value, stored val, element type)
    for &block_idx in &loop_info.body {
        for inst in &func.blocks[block_idx].instructions {
            match inst {
                Instruction::Load { dest, ptr, ty, .. } => {
                    // Packed I32/U32/F32/F64 all have native forms.
                    if !matches!(*ty, IrType::I32 | IrType::U32 | IrType::F32 | IrType::F64)
                        || load_infos.len() >= MAP_MAX_STREAMS
                    {
                        return None;
                    }
                    load_infos.push((block_idx, *dest, *ptr, *ty));
                }
                Instruction::Store { val, ptr, ty, .. } => {
                    if !matches!(*ty, IrType::I32 | IrType::U32 | IrType::F32 | IrType::F64)
                        || store_info.is_some()
                    {
                        return None;
                    }
                    let Operand::Value(store_val) = val else {
                        return None;
                    };
                    store_info = Some((block_idx, *ptr, *store_val, *ty));
                }
                Instruction::BinOp { op, ty, .. } if op.can_trap() && !ty.is_float() => {
                    return None;
                }
                // Scalar sqrt lowers to a pure intrinsic; the map tree
                // parser understands it (FP only).
                Instruction::Intrinsic {
                    op: IntrinsicOp::SqrtF32 | IntrinsicOp::SqrtF64,
                    ..
                } => {}
                Instruction::Phi { .. }
                | Instruction::BinOp { .. }
                | Instruction::UnaryOp { .. }
                | Instruction::Cmp { .. }
                | Instruction::Cast { .. }
                | Instruction::Copy { .. }
                | Instruction::GetElementPtr { .. }
                | Instruction::GlobalAddr { .. } => {}
                _ => return None,
            }
        }
    }
    let (body_idx, dst_gep, store_val, elem_ty) = store_info?;
    // At least one source stream is required (a pure store of a loop
    // invariant is not a map).
    if load_infos.is_empty() {
        return None;
    }
    if load_infos.iter().any(|&(_, _, _, ty)| ty != elem_ty) {
        return None;
    }

    let elem_size = match elem_ty {
        IrType::F64 | IrType::I64 | IrType::U64 => 8,
        IrType::F32 | IrType::I32 | IrType::U32 => 4,
        _ => {
            set_reject("map element type not vectorizable");
            return None;
        }
    };
    // The destination GEP must be indexed by the IV (canonical unit stride).
    if !gep_uses_iv(func, &loop_info.body, dst_gep, iv, &iv_derived, elem_size) {
        if debug {
            eprintln!("[VEC-MAP]   GEPs don't use IV");
        }
        return None;
    }
    if find_reduction_byte_iv(func, &loop_info.body, dst_gep, elem_size).is_none() {
        set_reject("map access is not a contiguous element-size stride");
        return None;
    }
    // Every source stream must be IV-indexed and contiguous as well.
    for &(_, _, src_gep, _) in &load_infos {
        if !gep_uses_iv(func, &loop_info.body, src_gep, iv, &iv_derived, elem_size)
            || find_reduction_byte_iv(func, &loop_info.body, src_gep, elem_size).is_none()
        {
            set_reject("map source access is not a contiguous element-size stride");
            return None;
        }
    }

    // Loop-invariance of an operand: constant, or defined outside the loop and
    // not derived from the IV.
    let is_invariant = |op: &Operand| -> bool {
        match op {
            Operand::Const(_) => true,
            Operand::Value(v) => {
                !iv_derived.contains(v) && find_inst_in_loop(func, &loop_info.body, *v).is_none()
            }
        }
    };

    // Parse the stored value as an elementwise expression tree over the load
    // streams and loop invariants (OP-05a). The legacy affine family is the
    // depth-1 subset. Bounded: small trees only, so the vector body stays
    // compact and register pressure bounded.
    let load_dests: Vec<Value> = load_infos.iter().map(|&(_, d, _, _)| d).collect();
    // NEON lowers VecAdd/VecMul (fadd/fmul 2d/4s) but has no lowering for
    // VecSub/VecDiv/VecSqrt yet — restrict those tree nodes to x86 targets
    // (fail-closed: a NEON loop containing them stays scalar).
    let allow_ext_fp_ops = !neon;
    let parse_tree = |value: Value, depth: usize| -> Option<MapExpr> {
        parse_map_expr(
            func,
            &loop_info.body,
            &load_dests,
            &iv_derived,
            &is_invariant,
            &elem_ty,
            value,
            depth,
            allow_ext_fp_ops,
        )
    };
    let expr = if load_dests.len() == 1 && store_val == load_dests[0] {
        MapExpr::Load(0)
    } else {
        parse_tree(store_val, 0)?
    };
    const MAP_MAX_NODES: usize = 12;
    if expr.node_count() > MAP_MAX_NODES {
        set_reject("map expression tree too large");
        return None;
    }
    // Every load stream must appear in the tree (a stream read but not
    // stored would leave a live scalar load the transform cannot remove).
    for (idx, _) in load_dests.iter().enumerate() {
        if !expr_uses_stream(&expr, idx) {
            if load_dests.len() == 1 {
                // The single legacy stream may feed dead code that DCE owns;
                // the affine path tolerated this shape.
                continue;
            }
            return None;
        }
    }

    // Alias safety: identify the complete pointer roots. Different SSA value
    // numbers are NOT an alias proof; ordinary pointer parameters may overlap.
    // Distinct restrict parameters and distinct globals/allocas are legal.
    // Only WRITE-vs-READ aliasing matters: source streams only read, so they
    // may alias each other freely; each source must be disjoint from the
    // destination (or the exact same GEP for lane-local in-place maps).
    let dst_root = proven_object_root(func, dst_gep)?;
    for &(_, _, src_gep, _) in &load_infos {
        let src_root = proven_object_root(func, src_gep)?;
        // Exact in-place maps are lane-local and safe (`a[i] = a[i] * s + b`).
        // Otherwise require disjoint complete-object roots; merely seeing
        // different SSA pointer values is not an alias proof.
        if !geps_proven_identical(func, &loop_info.body, src_gep, dst_gep)
            && !roots_proven_distinct(&dst_root, &src_root)
        {
            if debug {
                eprintln!(
                    "[VEC-MAP]   GEP {:?}/{:?} bases not provably distinct: {:?} vs {:?}",
                    dst_gep, src_gep, dst_root, src_root
                );
            }
            set_reject("map source/destination may alias (use restrict or exact in-place access)");
            return None;
        }
    }

    if debug {
        eprintln!("[VEC-MAP]   Map pattern detected: {:?}", expr);
    }

    Some(MapPattern {
        header_idx,
        body_idx,
        latch_idx,
        exit_idx,
        iv,
        iv_ty,
        elem_ty,
        limit,
        exit_cmp_op,
        dst_gep,
        src_geps: load_infos.iter().map(|&(_, _, g, _)| g).collect(),
        expr,
        loop_blocks: loop_info.body.clone(),
    })
}

/// Recursive parser for the elementwise map expression (OP-05a).
///
/// Recognizes, bounded by `depth`: stream loads, loop-invariant scalars,
/// FP Add/Sub/Mul/Div, integer Add/Mul, and FP Sqrt intrinsics. IV-derived
/// values and anything defined in the loop outside this grammar fail closed.
#[allow(clippy::too_many_arguments)]
fn parse_map_expr(
    func: &IrFunction,
    loop_blocks: &FxHashSet<usize>,
    load_dests: &[Value],
    iv_derived: &FxHashSet<Value>,
    is_invariant: &dyn Fn(&Operand) -> bool,
    elem_ty: &IrType,
    value: Value,
    depth: usize,
    allow_ext_fp_ops: bool,
) -> Option<MapExpr> {
    // Depth guard is a secondary bound; the analyzer's node-count cap is
    // the primary size limit. 6 allows sqrt(mul+add) / div nesting (the
    // nbody/spectral elementwise shapes) while keeping the tree compact.
    if depth > 6 {
        return None;
    }
    if let Some(idx) = load_dests.iter().position(|&d| d == value) {
        return Some(MapExpr::Load(idx));
    }
    let (_, inst) = find_inst_in_loop(func, loop_blocks, value)?;
    match inst {
        Instruction::BinOp {
            op, lhs, rhs, ty, ..
        } => {
            if *ty != *elem_ty {
                return None;
            }
            let fp = ty.is_float();
            match op {
                IrBinOp::Add | IrBinOp::Mul => {}
                // Sub/Div (and Sqrt below) lower to VecSub/VecDiv/VecSqrt,
                // which only the x86 backend implements today.
                IrBinOp::Sub | IrBinOp::SDiv if fp && allow_ext_fp_ops => {}
                _ => return None,
            }
            let l = parse_map_operand(
                func,
                loop_blocks,
                load_dests,
                iv_derived,
                is_invariant,
                elem_ty,
                lhs,
                depth + 1,
                allow_ext_fp_ops,
            )?;
            let r = parse_map_operand(
                func,
                loop_blocks,
                load_dests,
                iv_derived,
                is_invariant,
                elem_ty,
                rhs,
                depth + 1,
                allow_ext_fp_ops,
            )?;
            Some(MapExpr::BinOp(*op, Box::new(l), Box::new(r)))
        }
        Instruction::Intrinsic {
            op: IntrinsicOp::SqrtF64,
            args,
            ..
        } if *elem_ty == IrType::F64 && allow_ext_fp_ops => {
            let Operand::Value(v) = &args[0] else {
                return None;
            };
            Some(MapExpr::Sqrt(Box::new(parse_map_expr(
                func,
                loop_blocks,
                load_dests,
                iv_derived,
                is_invariant,
                elem_ty,
                *v,
                depth + 1,
                allow_ext_fp_ops,
            )?)))
        }
        Instruction::Intrinsic {
            op: IntrinsicOp::SqrtF32,
            args,
            ..
        } if *elem_ty == IrType::F32 && allow_ext_fp_ops => {
            let Operand::Value(v) = &args[0] else {
                return None;
            };
            Some(MapExpr::Sqrt(Box::new(parse_map_expr(
                func,
                loop_blocks,
                load_dests,
                iv_derived,
                is_invariant,
                elem_ty,
                *v,
                depth + 1,
                allow_ext_fp_ops,
            )?)))
        }
        _ => None,
    }
}

#[allow(clippy::too_many_arguments)]
fn parse_map_operand(
    func: &IrFunction,
    loop_blocks: &FxHashSet<usize>,
    load_dests: &[Value],
    iv_derived: &FxHashSet<Value>,
    is_invariant: &dyn Fn(&Operand) -> bool,
    elem_ty: &IrType,
    operand: &Operand,
    depth: usize,
    allow_ext_fp_ops: bool,
) -> Option<MapExpr> {
    if is_invariant(operand) {
        return Some(MapExpr::Invariant(operand.clone()));
    }
    let Operand::Value(v) = operand else {
        return None;
    };
    parse_map_expr(
        func,
        loop_blocks,
        load_dests,
        iv_derived,
        is_invariant,
        elem_ty,
        *v,
        depth,
        allow_ext_fp_ops,
    )
}

/// Every BinOp/Sqrt node in the tree must have a lowering before the
/// transform starts mutating the function (fail-closed pre-validation).
fn map_tree_ops_available(
    expr: &MapExpr,
    bin_op: &dyn Fn(&IrBinOp) -> Option<IntrinsicOp>,
    sqrt_op: Option<IntrinsicOp>,
) -> bool {
    match expr {
        MapExpr::Load(_) | MapExpr::Invariant(_) => true,
        MapExpr::BinOp(op, l, r) => {
            bin_op(op).is_some()
                && map_tree_ops_available(l, bin_op, sqrt_op)
                && map_tree_ops_available(r, bin_op, sqrt_op)
        }
        MapExpr::Sqrt(x) => sqrt_op.is_some() && map_tree_ops_available(x, bin_op, sqrt_op),
    }
}

fn expr_uses_stream(expr: &MapExpr, stream: usize) -> bool {
    match expr {
        MapExpr::Load(i) => *i == stream,
        MapExpr::Invariant(_) => false,
        MapExpr::BinOp(_, l, r) => expr_uses_stream(l, stream) || expr_uses_stream(r, stream),
        MapExpr::Sqrt(x) => expr_uses_stream(x, stream),
    }
}

/// Prove two loop GEPs denote the same address in every iteration.  Frontend
/// lowering commonly emits separate GEP SSA destinations for an exact in-place
/// load/store while sharing the base and byte-offset operands.
fn geps_proven_identical(
    func: &IrFunction,
    loop_blocks: &FxHashSet<usize>,
    a: Value,
    b: Value,
) -> bool {
    if a == b {
        return true;
    }
    let find = |target: Value| {
        for &block_idx in loop_blocks {
            for inst in &func.blocks[block_idx].instructions {
                if let Instruction::GetElementPtr {
                    dest, base, offset, ..
                } = inst
                {
                    if *dest == target {
                        return Some((*base, offset.clone()));
                    }
                }
            }
        }
        None
    };
    match (find(a), find(b)) {
        (Some((a_base, Operand::Value(a_offset))), Some((b_base, Operand::Value(b_offset)))) => {
            a_base == b_base && a_offset == b_offset
        }
        (Some((a_base, Operand::Const(a_offset))), Some((b_base, Operand::Const(b_offset)))) => {
            a_base == b_base && a_offset.to_i64() == b_offset.to_i64()
        }
        _ => false,
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Stencil vectorization (OP-05a): generalized non-reduction FP loops.
//
// Target shape (one store, N loads from one base at constant element
// offsets, expression tree of adds/muls over the loads and invariants):
//
//     for (i = s; i + k < n; ++i)
//         dst[i] = w0*src[i+o0] + w1*src[i+o1] + ... + C;
//
// This is the fp_memfold_stencil5 / nbody-force / spectral-kernel class the
// pattern-based matmul and single-load map vectorizers reject.
//
// Exactness contract: the vector body mirrors the scalar expression TREE
// one node at a time (VecAdd for Add, VecMul for Mul, broadcast for
// invariants, VecLoad per tap), so every lane computes exactly the scalar
// op sequence for its element — no reassociation, no fast-math required.
// Sub is emitted as Add(x, Mul(y, -1)) which is bit-exact in IEEE-754
// (subtraction IS addition of the negation; negation is exact).
// ═══════════════════════════════════════════════════════════════════════════

/// One stencil input tap: a load at `iv + disp/elem` from the shared base.
#[derive(Debug, Clone)]
struct StencilTap {
    /// The Load instruction's dest value.
    load: Value,
    /// Byte displacement from `base + iv*elem` (must fit an i32 SIB disp).
    disp_bytes: i64,
}

/// A matched stencil loop.
#[derive(Debug)]
struct StencilPattern {
    header_idx: usize,
    body_idx: usize,
    latch_idx: usize,
    exit_idx: usize,
    loop_blocks: FxHashSet<usize>,
    /// Element-counter phi in the header.
    iv: Value,
    iv_ty: IrType,
    /// Constant initial value of the IV (element units).
    iv_start: i64,
    /// The exit comparison is `(iv + cmp_k) op limit` (normalized Slt/Ult).
    cmp_k: i64,
    limit: Operand,
    exit_cmp_op: IrCmpOp,
    elem_ty: IrType,
    /// Shared source root of every tap GEP.
    src_base: Value,
    /// Destination store root.
    dst_base: Value,
    /// Byte displacement of the store (usually 0).
    dst_disp: i64,
    taps: Vec<StencilTap>,
    /// The stored value (the expression tree root).
    store_val: Value,
    store_gep: Value,
}

/// Affine form `a*iv + b` over the loop IV, in BYTE units (GEP offsets).
#[derive(Debug, Clone, Copy)]
struct IvAffine {
    scale: i64,
    const_bytes: i64,
}

/// Match `v == iv` (element units, scale 1, const 0) through the GEP offset
/// expression: Cast(widen)/Copy chains, Shl/Mul by a constant, and
/// Add/Sub of constants — i.e. `((iv ± k) * elem)` byte-offset shapes.
///
/// Returns the affine coefficients, or `None` when the expression depends
/// on anything but the IV and constants.
fn affine_iv_offset(
    func: &IrFunction,
    loop_blocks: &FxHashSet<usize>,
    v: Value,
    iv: Value,
    depth: u32,
) -> Option<IvAffine> {
    if depth > 8 {
        return None;
    }
    if v == iv {
        return Some(IvAffine {
            scale: 1,
            const_bytes: 0,
        });
    }
    // Definition lookup (loop-local values only; the IV chain lives there).
    let mut def = None;
    for &block_idx in loop_blocks {
        for inst in &func.blocks[block_idx].instructions {
            if inst.dest() == Some(v) {
                def = Some(inst);
            }
        }
    }
    let def = def?;
    match def {
        Instruction::Copy { src, .. } => match src {
            Operand::Value(sv) => affine_iv_offset(func, loop_blocks, *sv, iv, depth + 1),
            Operand::Const(c) => Some(IvAffine {
                scale: 0,
                const_bytes: c.to_i64()?,
            }),
        },
        // Widening casts of an integer IV preserve the affine coefficients
        // (sign/zero extension of a*iv + b for a,b in the source width).
        Instruction::Cast { src, from_ty, .. } if from_ty.is_integer() => match src {
            Operand::Value(sv) => affine_iv_offset(func, loop_blocks, *sv, iv, depth + 1),
            Operand::Const(c) => Some(IvAffine {
                scale: 0,
                const_bytes: c.to_i64()?,
            }),
        },
        Instruction::BinOp {
            op: IrBinOp::Shl,
            lhs,
            rhs,
            ..
        } => {
            let k = match rhs {
                Operand::Const(c) => c.to_i64()?,
                _ => return None,
            };
            if !(0..=16).contains(&k) {
                return None;
            }
            let inner = match lhs {
                Operand::Value(lv) => affine_iv_offset(func, loop_blocks, *lv, iv, depth + 1)?,
                Operand::Const(c) => IvAffine {
                    scale: 0,
                    const_bytes: c.to_i64()?,
                },
            };
            Some(IvAffine {
                scale: inner.scale.checked_shl(k as u32)?,
                const_bytes: inner.const_bytes.checked_shl(k as u32)?,
            })
        }
        Instruction::BinOp {
            op: IrBinOp::Mul,
            lhs,
            rhs,
            ..
        } => {
            let (val_op, k) = match (lhs, rhs) {
                (Operand::Value(v), Operand::Const(c)) => (*v, c.to_i64()?),
                (Operand::Const(c), Operand::Value(v)) => (*v, c.to_i64()?),
                _ => return None,
            };
            let inner = affine_iv_offset(func, loop_blocks, val_op, iv, depth + 1)?;
            Some(IvAffine {
                scale: inner.scale.checked_mul(k)?,
                const_bytes: inner.const_bytes.checked_mul(k)?,
            })
        }
        Instruction::BinOp {
            op: IrBinOp::Add,
            lhs,
            rhs,
            ..
        } => {
            let l = affine_operand(func, loop_blocks, lhs, iv, depth + 1)?;
            let r = affine_operand(func, loop_blocks, rhs, iv, depth + 1)?;
            Some(IvAffine {
                scale: l.scale.checked_add(r.scale)?,
                const_bytes: l.const_bytes.checked_add(r.const_bytes)?,
            })
        }
        Instruction::BinOp {
            op: IrBinOp::Sub,
            lhs,
            rhs,
            ..
        } => {
            let l = affine_operand(func, loop_blocks, lhs, iv, depth + 1)?;
            let r = affine_operand(func, loop_blocks, rhs, iv, depth + 1)?;
            Some(IvAffine {
                scale: l.scale.checked_sub(r.scale)?,
                const_bytes: l.const_bytes.checked_sub(r.const_bytes)?,
            })
        }
        _ => None,
    }
}

/// [`affine_iv_offset`] for an operand (constant or value).
fn affine_operand(
    func: &IrFunction,
    loop_blocks: &FxHashSet<usize>,
    op: &Operand,
    iv: Value,
    depth: u32,
) -> Option<IvAffine> {
    match op {
        Operand::Const(c) => Some(IvAffine {
            scale: 0,
            const_bytes: c.to_i64()?,
        }),
        Operand::Value(v) => affine_iv_offset(func, loop_blocks, *v, iv, depth),
    }
}

/// Analyze a loop for the stencil pattern. See the section comment for the
/// shape and the exactness contract.
fn analyze_stencil_pattern(
    func: &IrFunction,
    loop_info: &loop_analysis::NaturalLoop,
) -> Option<StencilPattern> {
    let debug = std::env::var("LCCC_DEBUG_VECTORIZE").is_ok();
    let header_idx = loop_info.header;
    let header = &func.blocks[header_idx];

    let exit_idx = find_exit(func, loop_info)?;
    let latch_idx = find_latch(func, loop_info)?;
    // Canonical while-loop shape (the transform redirects the false edge).
    if !matches!(header.terminator,
        Terminator::CondBranch { false_label, .. }
            if false_label == func.blocks[exit_idx].label)
    {
        set_reject("stencil loop is not a canonical while loop");
        return None;
    }
    // No internal conditionals (predication not supported).
    if loop_info.body.iter().copied().any(|block_idx| {
        block_idx != header_idx
            && matches!(
                func.blocks[block_idx].terminator,
                Terminator::CondBranch { .. }
            )
    }) {
        set_reject("stencil loop has internal conditionals");
        return None;
    }

    // ── IV + exit compare: `(iv + k) op limit` ────────────────────────────
    // The IV phi: 2 incomings, preheader constant start, latch +1. The
    // compare operand may be the phi itself (k = 0) or Add(phi, k).
    let mut iv_info: Option<(Value, IrType, i64, Operand, IrCmpOp)> = None;
    for inst in &header.instructions {
        let Instruction::Phi { dest, incoming, ty } = inst else {
            continue;
        };
        if incoming.len() != 2
            || !matches!(*ty, IrType::I32 | IrType::U32 | IrType::I64 | IrType::U64)
        {
            continue;
        }
        // Values transitively derived from the phi by Cast/Copy.
        let mut derived = FxHashSet::default();
        derived.insert(*dest);
        for candidate in &header.instructions {
            if let Instruction::Cast { dest, src, .. } | Instruction::Copy { dest, src } = candidate
            {
                if matches!(src, Operand::Value(v) if derived.contains(v)) {
                    derived.insert(*dest);
                }
            }
        }
        // Values of the form Add(derived, k).
        let mut derived_k: Vec<(Value, i64)> = vec![(*dest, 0)];
        for candidate in &header.instructions {
            if let Instruction::BinOp {
                dest: d,
                op: IrBinOp::Add,
                lhs,
                rhs,
                ..
            } = candidate
            {
                if let (Operand::Value(v), Operand::Const(c)) = (lhs, rhs) {
                    if derived.contains(v) {
                        if let Some(k) = c.to_i64() {
                            derived_k.push((*d, k));
                        }
                    }
                }
                if let (Operand::Const(c), Operand::Value(v)) = (lhs, rhs) {
                    if derived.contains(v) {
                        if let Some(k) = c.to_i64() {
                            derived_k.push((*d, k));
                        }
                    }
                }
            }
        }
        for cand in &header.instructions {
            if let Instruction::Cmp {
                op,
                lhs,
                rhs,
                ty: cmp_ty,
                ..
            } = cand
            {
                if *cmp_ty != *ty {
                    continue;
                }
                for (dv, k) in &derived_k {
                    let (norm_op, limit) = if matches!(lhs, Operand::Value(v) if v == dv) {
                        (*op, rhs.clone())
                    } else if matches!(rhs, Operand::Value(v) if v == dv) {
                        match op {
                            IrCmpOp::Sgt => (IrCmpOp::Slt, lhs.clone()),
                            IrCmpOp::Ugt => (IrCmpOp::Ult, lhs.clone()),
                            _ => continue,
                        }
                    } else {
                        continue;
                    };
                    if !matches!(norm_op, IrCmpOp::Slt | IrCmpOp::Ult) {
                        continue;
                    }
                    // The limit must be loop-invariant.
                    if matches!(&limit, Operand::Value(v)
                        if find_inst_in_loop(func, &loop_info.body, *v).is_some())
                    {
                        continue;
                    }
                    if iv_info.is_none() {
                        iv_info = Some((*dest, *ty, *k, limit, norm_op));
                    }
                }
            }
        }
    }
    let (iv, iv_ty, cmp_k, limit, exit_cmp_op) = iv_info?;

    // IV start: the phi's non-latch incoming must be a constant.
    let latch_label = func.blocks[latch_idx].label;
    let mut iv_start = None;
    for inst in &header.instructions {
        if let Instruction::Phi { dest, incoming, .. } = inst {
            if *dest == iv {
                for (op, pred) in incoming {
                    if *pred != latch_label {
                        if let Operand::Const(c) = op {
                            iv_start = c.to_i64();
                        }
                    }
                }
            }
        }
    }
    let iv_start = iv_start?;

    // Unit increment in the latch.
    let latch = &func.blocks[latch_idx];
    if !latch.instructions.iter().any(|inst| {
        matches!(inst, Instruction::BinOp { op: IrBinOp::Add, lhs, rhs, .. }
            if matches!(lhs, Operand::Value(v) if *v == iv)
                && matches!(rhs, Operand::Const(c) if c.to_i64() == Some(1)))
    }) {
        set_reject("stencil IV does not increment by 1");
        return None;
    }

    // ── Body scan: one store, N loads, only simple arithmetic ────────────
    let elem_ty = {
        let mut store_ty = None;
        let mut load_tys: Vec<IrType> = Vec::new();
        for &block_idx in &loop_info.body {
            for inst in &func.blocks[block_idx].instructions {
                match inst {
                    Instruction::Load { ty, .. } => load_tys.push(*ty),
                    Instruction::Store { ty, .. } => {
                        if store_ty.is_some() {
                            set_reject("stencil loop has multiple stores");
                            return None;
                        }
                        store_ty = Some(*ty);
                    }
                    Instruction::BinOp { op, .. } if op.can_trap() => {
                        set_reject("stencil loop has a trapping operation");
                        return None;
                    }
                    Instruction::Phi { .. }
                    | Instruction::BinOp { .. }
                    | Instruction::UnaryOp { .. }
                    | Instruction::Cmp { .. }
                    | Instruction::Cast { .. }
                    | Instruction::Copy { .. }
                    | Instruction::Select { .. }
                    | Instruction::GetElementPtr { .. }
                    | Instruction::GlobalAddr { .. } => {}
                    _ => {
                        set_reject("stencil loop contains an unsupported instruction");
                        return None;
                    }
                }
            }
        }
        let ty = store_ty?;
        if !matches!(ty, IrType::F32 | IrType::F64) {
            set_reject("stencil element type is not scalar FP");
            return None;
        }
        for lt in &load_tys {
            if *lt != ty {
                set_reject("stencil loads mix element types");
                return None;
            }
        }
        ty
    };
    let elem_size: i64 = if elem_ty == IrType::F64 { 8 } else { 4 };

    // Locate the store and every load; map each to (base, affine).
    let mut store_info = None;
    // (load dest, gep, base, affine)
    let mut load_infos: Vec<(Value, Value, Value, IvAffine)> = Vec::new();
    for &block_idx in &loop_info.body {
        for inst in &func.blocks[block_idx].instructions {
            match inst {
                Instruction::Store { val, ptr, .. } => {
                    let Operand::Value(store_val) = val else {
                        return None;
                    };
                    let store_gep = *ptr;
                    store_info = Some((*store_val, store_gep, block_idx));
                }
                Instruction::Load { dest, ptr, .. } => {
                    let gep = *ptr;
                    let mut found = None;
                    for &b2 in &loop_info.body {
                        for inst2 in &func.blocks[b2].instructions {
                            if let Instruction::GetElementPtr {
                                dest: gd,
                                base,
                                offset,
                                ..
                            } = inst2
                            {
                                if *gd == gep {
                                    found = Some((
                                        *base,
                                        affine_operand(func, &loop_info.body, offset, iv, 0),
                                    ));
                                }
                            }
                        }
                    }
                    let (base, affine) = found?;
                    let affine = affine?;
                    if affine.scale != elem_size {
                        set_reject("stencil access is not a contiguous element stride");
                        return None;
                    }
                    load_infos.push((*dest, gep, base, affine));
                }
                _ => {}
            }
        }
    }
    let (store_val, store_gep, body_idx) = store_info?;
    if load_infos.is_empty() {
        set_reject("stencil loop has no loads");
        return None;
    }
    // Store GEP affine.
    let mut store_affine = None;
    for &b2 in &loop_info.body {
        for inst2 in &func.blocks[b2].instructions {
            if let Instruction::GetElementPtr {
                dest: gd,
                base,
                offset,
                ..
            } = inst2
            {
                if *gd == store_gep {
                    store_affine =
                        Some((*base, affine_operand(func, &loop_info.body, offset, iv, 0)));
                }
            }
        }
    }
    let (dst_base, store_affine) = store_affine?;
    let store_affine = store_affine?;
    if store_affine.scale != elem_size {
        set_reject("stencil store is not a contiguous element stride");
        return None;
    }
    let dst_disp = store_affine.const_bytes;

    // All loads must share ONE base root.
    let src_base = load_infos[0].2;
    if load_infos.iter().any(|(_, _, b, _)| *b != src_base) {
        set_reject("stencil taps use multiple source bases");
        return None;
    }

    // Displacements must fit the x86 SIB disp32.
    let disp_ok = |d: i64| d.unsigned_abs() <= 0x7FFF_FFF0;
    if !disp_ok(dst_disp)
        || load_infos
            .iter()
            .any(|(_, _, _, a)| !disp_ok(a.const_bytes))
    {
        set_reject("stencil displacement exceeds the SIB disp32 range");
        return None;
    }

    // Alias safety: distinct proven roots, or an exact lane-local in-place
    // update (store disp 0 AND every tap disp 0).
    let dst_root = proven_object_root(func, dst_base)?;
    let src_root = proven_object_root(func, src_base)?;
    let lane_local = dst_disp == 0 && load_infos.iter().all(|(_, _, _, a)| a.const_bytes == 0);
    if dst_base != src_base && !roots_proven_distinct(&dst_root, &src_root) && !lane_local {
        set_reject("stencil source/destination may alias (use restrict)");
        return None;
    }

    // Constant trip counts of 4 or fewer are better left scalar (mirrors
    // the map gate).
    if let Operand::Const(c) = &limit {
        let trip = c.to_i64()? - cmp_k - iv_start;
        if trip <= 4 {
            return None;
        }
    }

    let pattern = StencilPattern {
        header_idx,
        body_idx,
        latch_idx,
        exit_idx,
        loop_blocks: loop_info.body.clone(),
        iv,
        iv_ty,
        iv_start,
        cmp_k,
        limit,
        exit_cmp_op,
        elem_ty,
        src_base,
        dst_base,
        dst_disp,
        taps: load_infos
            .into_iter()
            .map(|(load, _gep, _base, affine)| StencilTap {
                load,
                disp_bytes: affine.const_bytes,
            })
            .collect(),
        store_val,
        store_gep,
    };
    if debug {
        eprintln!(
            "[VEC-STENCIL] matched: taps={} elem={:?} start={} k={} dst_disp={} disps={:?}",
            pattern.taps.len(),
            pattern.elem_ty,
            pattern.iv_start,
            pattern.cmp_k,
            pattern.dst_disp,
            pattern
                .taps
                .iter()
                .map(|t| t.disp_bytes)
                .collect::<Vec<_>>()
        );
    }
    Some(pattern)
}

/// Transform a matched stencil loop. Returns the number of IR changes.
fn transform_stencil_vector(
    func: &mut IrFunction,
    pattern: &StencilPattern,
    avx2: bool,
    fp_contract: crate::common::fp_contract::FpContract,
) -> usize {
    let debug = std::env::var("LCCC_DEBUG_VECTORIZE").is_ok();
    let mut changes = 0usize;
    let vec_width: u64 = match (pattern.elem_ty, avx2) {
        (IrType::F64, true) => 4,
        (IrType::F64, false) => 2,
        (_, true) => 8,
        (_, false) => 4,
    };

    // A zero-iteration vector loop plus scalar remainder only adds overhead.
    if let Operand::Const(c) = &pattern.limit {
        let trip = c.to_i64().unwrap_or(0) - pattern.cmp_k - pattern.iv_start;
        if trip <= vec_width as i64 {
            return 0;
        }
    }

    let mut next_val_id = func.next_value_id;
    let mut next_label = func.next_label.max(
        func.blocks
            .iter()
            .map(|b| b.label.0)
            .max()
            .unwrap_or(0)
            .saturating_add(1),
    );

    // Preheader: the unique out-of-loop branch to the header.
    let Some(preheader_idx) = func.blocks.iter().enumerate().find_map(|(idx, block)| {
        if pattern.loop_blocks.contains(&idx) {
            return None;
        }
        matches!(block.terminator, Terminator::Branch(label)
            if label == func.blocks[pattern.header_idx].label)
        .then_some(idx)
    }) else {
        return 0;
    };
    let preheader_label = func.blocks[preheader_idx].label;
    let latch_label = func.blocks[pattern.latch_idx].label;

    let elem_size: i64 = if pattern.elem_ty == IrType::F64 { 8 } else { 4 };
    let int_const = |n: i64| match pattern.iv_ty {
        IrType::I32 | IrType::U32 => IrConst::I32(n as i32),
        _ => IrConst::I64(n),
    };

    // ── Vector trip count and the rewritten bound ─────────────────────────
    // trip = limit − (cmp_k + iv_start); bound = trip/W + cmp_k + iv_start
    // so the (iv + cmp_k) < bound form runs floor(trip/W) iterations.
    let trip_val = Value(next_val_id);
    next_val_id += 1;
    let trip_operand = match &pattern.limit {
        Operand::Const(c) => {
            let n = c.to_i64().unwrap_or(0) - pattern.cmp_k - pattern.iv_start;
            Operand::Const(int_const(n))
        }
        Operand::Value(limit_val) => {
            func.blocks[preheader_idx]
                .instructions
                .push(Instruction::BinOp {
                    dest: trip_val,
                    op: IrBinOp::Sub,
                    lhs: Operand::Value(*limit_val),
                    rhs: Operand::Const(int_const(pattern.cmp_k + pattern.iv_start)),
                    ty: pattern.iv_ty,
                });
            changes += 1;
            Operand::Value(trip_val)
        }
    };

    let vec_trip = Value(next_val_id);
    next_val_id += 1;
    let shift = vec_width.trailing_zeros() as i64;
    if pattern.exit_cmp_op == IrCmpOp::Slt {
        // Signed division by 2^k rounded toward zero:
        //   (n + ((n >> (bits-1)) & (2^k-1))) >> k
        let sign = Value(next_val_id);
        next_val_id += 1;
        let bias = Value(next_val_id);
        next_val_id += 1;
        let adjusted = Value(next_val_id);
        next_val_id += 1;
        let bits = if matches!(pattern.iv_ty, IrType::I32 | IrType::U32) {
            31
        } else {
            63
        };
        for inst in [
            Instruction::BinOp {
                dest: sign,
                op: IrBinOp::AShr,
                lhs: trip_operand.clone(),
                rhs: Operand::Const(int_const(bits)),
                ty: pattern.iv_ty,
            },
            Instruction::BinOp {
                dest: bias,
                op: IrBinOp::And,
                lhs: Operand::Value(sign),
                rhs: Operand::Const(int_const(vec_width as i64 - 1)),
                ty: pattern.iv_ty,
            },
            Instruction::BinOp {
                dest: adjusted,
                op: IrBinOp::Add,
                lhs: trip_operand.clone(),
                rhs: Operand::Value(bias),
                ty: pattern.iv_ty,
            },
            Instruction::BinOp {
                dest: vec_trip,
                op: IrBinOp::AShr,
                lhs: Operand::Value(adjusted),
                rhs: Operand::Const(int_const(shift)),
                ty: pattern.iv_ty,
            },
        ] {
            func.blocks[preheader_idx].instructions.push(inst);
            changes += 1;
        }
    } else {
        func.blocks[preheader_idx]
            .instructions
            .push(Instruction::BinOp {
                dest: vec_trip,
                op: IrBinOp::LShr,
                lhs: trip_operand.clone(),
                rhs: Operand::Const(int_const(shift)),
                ty: pattern.iv_ty,
            });
        changes += 1;
    }
    // bound = vec_trip + (cmp_k + iv_start)
    let bound = Value(next_val_id);
    next_val_id += 1;
    func.blocks[preheader_idx]
        .instructions
        .push(Instruction::BinOp {
            dest: bound,
            op: IrBinOp::Add,
            lhs: Operand::Value(vec_trip),
            rhs: Operand::Const(int_const(pattern.cmp_k + pattern.iv_start)),
            ty: pattern.iv_ty,
        });
    changes += 1;

    // Rewrite the header compare's limit operand. The compare's IV-derived
    // side is (iv + cmp_k) (or the phi itself); replace the OTHER side.
    {
        let header = &mut func.blocks[pattern.header_idx];
        let mut iv_side = FxHashSet::default();
        iv_side.insert(pattern.iv);
        for cand in &header.instructions {
            if let Instruction::Cast { dest, src, .. } | Instruction::Copy { dest, src } = cand {
                if matches!(src, Operand::Value(v) if iv_side.contains(v)) {
                    iv_side.insert(*dest);
                }
            }
        }
        for cand in &header.instructions {
            if let Instruction::BinOp {
                dest,
                op: IrBinOp::Add,
                lhs,
                rhs,
                ..
            } = cand
            {
                let lhs_iv = matches!(lhs, Operand::Value(v) if iv_side.contains(v));
                let rhs_iv = matches!(rhs, Operand::Value(v) if iv_side.contains(v));
                if lhs_iv != rhs_iv {
                    iv_side.insert(*dest);
                }
            }
        }
        for inst in header.instructions.iter_mut() {
            if let Instruction::Cmp { lhs, rhs, .. } = inst {
                let lhs_iv = matches!(lhs, Operand::Value(v) if iv_side.contains(v));
                let rhs_iv = matches!(rhs, Operand::Value(v) if iv_side.contains(v));
                if lhs_iv && !rhs_iv {
                    *rhs = Operand::Value(bound);
                    changes += 1;
                } else if rhs_iv && !lhs_iv {
                    *lhs = Operand::Value(bound);
                    changes += 1;
                }
            }
        }
    }

    // ── Byte IV: element-accurate vector addressing ────────────────────────
    // byte_iv starts at iv_start*elem (the first scalar iteration's byte
    // offset from the GEP bases) and advances width*elem per iteration.
    let byte_iv = Value(next_val_id);
    next_val_id += 1;
    let byte_iv_next = Value(next_val_id);
    next_val_id += 1;
    let phi_pos = func.blocks[pattern.header_idx]
        .instructions
        .iter()
        .position(|inst| !matches!(inst, Instruction::Phi { .. }))
        .unwrap_or(func.blocks[pattern.header_idx].instructions.len());
    func.blocks[pattern.header_idx].instructions.insert(
        phi_pos,
        Instruction::Phi {
            dest: byte_iv,
            ty: IrType::I64,
            incoming: vec![
                (
                    Operand::Const(IrConst::I64(pattern.iv_start * elem_size)),
                    preheader_label,
                ),
                (Operand::Value(byte_iv_next), latch_label),
            ],
        },
    );
    func.blocks[pattern.latch_idx]
        .instructions
        .push(Instruction::BinOp {
            dest: byte_iv_next,
            op: IrBinOp::Add,
            lhs: Operand::Value(byte_iv),
            rhs: Operand::Const(IrConst::I64(elem_size * vec_width as i64)),
            ty: IrType::I64,
        });
    changes += 2;

    // ── Vector body ────────────────────────────────────────────────────────
    let (load_op, add_op, mul_op, store_op, broadcast_op) = match (pattern.elem_ty, avx2) {
        (IrType::F64, true) => (
            IntrinsicOp::VecLoadF64x4,
            IntrinsicOp::VecAddF64x4,
            IntrinsicOp::VecMulF64x4,
            IntrinsicOp::VecStoreF64x4,
            IntrinsicOp::VecBroadcastF64x4,
        ),
        (IrType::F64, false) => (
            IntrinsicOp::VecLoadF64x2,
            IntrinsicOp::VecAddF64x2,
            IntrinsicOp::VecMulF64x2,
            IntrinsicOp::VecStoreF64x2,
            IntrinsicOp::VecBroadcastF64x2,
        ),
        (IrType::F32, true) => (
            IntrinsicOp::VecLoadF32x8,
            IntrinsicOp::VecAddF32x8,
            IntrinsicOp::VecMulF32x8,
            IntrinsicOp::VecStoreF32x8,
            IntrinsicOp::VecBroadcastF32x8,
        ),
        _ => (
            IntrinsicOp::VecLoadF32x4,
            IntrinsicOp::VecAddF32x4,
            IntrinsicOp::VecMulF32x4,
            IntrinsicOp::VecStoreF32x4,
            IntrinsicOp::VecBroadcastF32x4,
        ),
    };
    let madd_op = if fp_contract == FpContract::Fast && avx2 && x86_fma_enabled() {
        match pattern.elem_ty {
            IrType::F64 => Some(IntrinsicOp::VecMaddF64x4),
            IrType::F32 => Some(IntrinsicOp::VecMaddF32x8),
            _ => None,
        }
    } else {
        None
    };

    // Load each tap once (displacement folded into the memory operand).
    let mut tap_vecs: FxHashMap<u32, Value> = FxHashMap::default();
    let mut vec_insts: Vec<Instruction> = Vec::new();
    for tap in &pattern.taps {
        let v = Value(next_val_id);
        next_val_id += 1;
        vec_insts.push(Instruction::Intrinsic {
            dest: Some(v),
            op: load_op,
            dest_ptr: None,
            args: vec![
                Operand::Value(pattern.src_base),
                Operand::Value(byte_iv),
                Operand::Const(IrConst::I64(tap.disp_bytes)),
            ],
        });
        changes += 1;
        tap_vecs.insert(tap.load.0, v);
    }

    // Mirror the scalar expression tree in the vector domain.
    let Some(result) = emit_stencil_expr(
        func,
        pattern,
        &tap_vecs,
        &add_op,
        &mul_op,
        madd_op,
        broadcast_op,
        preheader_idx,
        &mut next_val_id,
        &mut vec_insts,
        &mut changes,
    ) else {
        // Fail soft: revert the bound rewrite so the scalar loop keeps its
        // original trip count, and drop the byte-IV plumbing.
        if debug {
            eprintln!("[VEC-STENCIL] expression mirror failed; reverting");
        }
        let header = &mut func.blocks[pattern.header_idx];
        for inst in header.instructions.iter_mut() {
            if let Instruction::Cmp { lhs, rhs, .. } = inst {
                if matches!(lhs, Operand::Value(v) if *v == bound) {
                    *lhs = pattern.limit.clone();
                } else if matches!(rhs, Operand::Value(v) if *v == bound) {
                    *rhs = pattern.limit.clone();
                }
            }
        }
        func.blocks[pattern.header_idx]
            .instructions
            .retain(|i| !matches!(i, Instruction::Phi { dest, .. } if *dest == byte_iv));
        func.blocks[pattern.latch_idx]
            .instructions
            .retain(|i| !matches!(i, Instruction::BinOp { dest, .. } if *dest == byte_iv_next));
        func.next_value_id = next_val_id;
        func.next_label = next_label;
        return changes;
    };

    // Store the vector result (displacement folded for dst_disp ≠ 0).
    vec_insts.push(Instruction::Intrinsic {
        dest: None,
        op: store_op,
        dest_ptr: Some(pattern.dst_base),
        args: vec![
            Operand::Value(result),
            Operand::Value(pattern.dst_base),
            Operand::Value(byte_iv),
            Operand::Const(IrConst::I64(pattern.dst_disp)),
        ],
    });
    changes += 1;

    // Replace the scalar store with the vector sequence.
    {
        let body = &mut func.blocks[pattern.body_idx];
        let Some(store_pos) = body.instructions.iter().position(
            |inst| matches!(inst, Instruction::Store { ptr, .. } if *ptr == pattern.store_gep),
        ) else {
            if debug {
                eprintln!("[VEC-STENCIL] store not found at transform time");
            }
            func.next_value_id = next_val_id;
            func.next_label = next_label;
            return changes;
        };
        let inserted = vec_insts.len();
        for (i, inst) in vec_insts.into_iter().enumerate() {
            body.instructions.insert(store_pos + i, inst);
        }
        body.instructions.remove(store_pos + inserted);
        changes += inserted;
    }

    changes +=
        insert_stencil_remainder_loop(func, pattern, vec_width, &mut next_val_id, &mut next_label);

    func.next_value_id = next_val_id;
    func.next_label = next_label;
    if debug {
        eprintln!("[VEC-STENCIL] transform complete: {} changes", changes);
    }
    changes
}

/// Mirror one scalar expression node of the stencil body in the vector
/// domain. See the stencil section comment for the exactness contract.
#[allow(clippy::too_many_arguments)]
fn emit_stencil_expr(
    func: &mut IrFunction,
    pattern: &StencilPattern,
    tap_vecs: &FxHashMap<u32, Value>,
    add_op: &IntrinsicOp,
    mul_op: &IntrinsicOp,
    madd_op: Option<IntrinsicOp>,
    broadcast_op: IntrinsicOp,
    preheader_idx: usize,
    next_val_id: &mut u32,
    vec_insts: &mut Vec<Instruction>,
    changes: &mut usize,
) -> Option<Value> {
    // Collect the loop-local def map up front (the scalar body is not
    // mutated by this emitter).
    let mut defs: FxHashMap<u32, Instruction> = FxHashMap::default();
    for &b in &pattern.loop_blocks {
        for inst in &func.blocks[b].instructions {
            if let Some(d) = inst.dest() {
                defs.insert(d.0, inst.clone());
            }
        }
    }

    fn broadcast(
        op: &Operand,
        func: &mut IrFunction,
        broadcast_op: IntrinsicOp,
        preheader_idx: usize,
        next_val_id: &mut u32,
        changes: &mut usize,
        cache: &mut FxHashMap<u64, Value>,
    ) -> Option<Value> {
        let key = match op {
            Operand::Value(v) => v.0 as u64,
            Operand::Const(c) => u32::MAX as u64 + (c.to_i64().unwrap_or(0) as u64),
        };
        if let Some(&b) = cache.get(&key) {
            return Some(b);
        }
        let dest = Value(*next_val_id);
        *next_val_id += 1;
        func.blocks[preheader_idx]
            .instructions
            .push(Instruction::Intrinsic {
                dest: Some(dest),
                op: broadcast_op,
                dest_ptr: None,
                args: vec![op.clone()],
            });
        *changes += 1;
        cache.insert(key, dest);
        Some(dest)
    }

    fn neg_one_const(elem_ty: IrType) -> Operand {
        Operand::Const(if elem_ty == IrType::F64 {
            IrConst::F64(-1.0)
        } else {
            IrConst::F32(-1.0)
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn emit_recursive(
        node: Value,
        func: &mut IrFunction,
        defs: &FxHashMap<u32, Instruction>,
        tap_vecs: &FxHashMap<u32, Value>,
        elem_ty: IrType,
        add_op: &IntrinsicOp,
        mul_op: &IntrinsicOp,
        madd_op: Option<IntrinsicOp>,
        broadcast_op: IntrinsicOp,
        preheader_idx: usize,
        next_val_id: &mut u32,
        vec_insts: &mut Vec<Instruction>,
        changes: &mut usize,
        cache: &mut FxHashMap<u64, Value>,
    ) -> Option<Value> {
        // Tap leaf: the already-loaded vector.
        if let Some(&v) = tap_vecs.get(&node.0) {
            return Some(v);
        }
        let is_invariant = |v: Value| !tap_vecs.contains_key(&v.0) && !defs.contains_key(&v.0);
        let def = defs.get(&node.0)?;
        match def {
            Instruction::BinOp { op, lhs, rhs, .. } => match op {
                IrBinOp::Add | IrBinOp::Sub => {
                    let lv_node = match lhs {
                        Operand::Value(v) => *v,
                        _ => return None,
                    };
                    let rv_node = match rhs {
                        Operand::Value(v) => *v,
                        _ => return None,
                    };
                    // FMA contraction (fast contract only):
                    // Add(Mul(tap, w), z) → VecMadd(tap, w, z).
                    if *op == IrBinOp::Add {
                        if let Some(madd) = madd_op {
                            if let Some(Instruction::BinOp {
                                op: IrBinOp::Mul,
                                lhs: mlhs,
                                rhs: mrhs,
                                ..
                            }) = defs.get(&lv_node.0)
                            {
                                if let (Operand::Value(tap), Operand::Value(w)) = (mlhs, mrhs) {
                                    if tap_vecs.contains_key(&tap.0) && is_invariant(*w) {
                                        let tv = *tap_vecs.get(&tap.0)?;
                                        let wv = broadcast(
                                            &Operand::Value(*w),
                                            func,
                                            broadcast_op,
                                            preheader_idx,
                                            next_val_id,
                                            changes,
                                            cache,
                                        )?;
                                        let zv = emit_recursive(
                                            rv_node,
                                            func,
                                            defs,
                                            tap_vecs,
                                            elem_ty,
                                            add_op,
                                            mul_op,
                                            madd_op,
                                            broadcast_op,
                                            preheader_idx,
                                            next_val_id,
                                            vec_insts,
                                            changes,
                                            cache,
                                        )?;
                                        let dest = Value(*next_val_id);
                                        *next_val_id += 1;
                                        vec_insts.push(Instruction::Intrinsic {
                                            dest: Some(dest),
                                            op: madd,
                                            dest_ptr: None,
                                            args: vec![
                                                Operand::Value(tv),
                                                Operand::Value(wv),
                                                Operand::Value(zv),
                                            ],
                                        });
                                        *changes += 1;
                                        return Some(dest);
                                    }
                                }
                            }
                        }
                    }
                    let lv = emit_recursive(
                        lv_node,
                        func,
                        defs,
                        tap_vecs,
                        elem_ty,
                        add_op,
                        mul_op,
                        madd_op,
                        broadcast_op,
                        preheader_idx,
                        next_val_id,
                        vec_insts,
                        changes,
                        cache,
                    )?;
                    let rv = emit_recursive(
                        rv_node,
                        func,
                        defs,
                        tap_vecs,
                        elem_ty,
                        add_op,
                        mul_op,
                        madd_op,
                        broadcast_op,
                        preheader_idx,
                        next_val_id,
                        vec_insts,
                        changes,
                        cache,
                    )?;
                    if *op == IrBinOp::Add {
                        let dest = Value(*next_val_id);
                        *next_val_id += 1;
                        vec_insts.push(Instruction::Intrinsic {
                            dest: Some(dest),
                            op: *add_op,
                            dest_ptr: None,
                            args: vec![Operand::Value(lv), Operand::Value(rv)],
                        });
                        *changes += 1;
                        Some(dest)
                    } else {
                        // a - b == a + (b * -1), bit-exact in IEEE-754.
                        let neg = broadcast(
                            &neg_one_const(elem_ty),
                            func,
                            broadcast_op,
                            preheader_idx,
                            next_val_id,
                            changes,
                            cache,
                        )?;
                        let negated = Value(*next_val_id);
                        *next_val_id += 1;
                        vec_insts.push(Instruction::Intrinsic {
                            dest: Some(negated),
                            op: *mul_op,
                            dest_ptr: None,
                            args: vec![Operand::Value(rv), Operand::Value(neg)],
                        });
                        *changes += 1;
                        let dest = Value(*next_val_id);
                        *next_val_id += 1;
                        vec_insts.push(Instruction::Intrinsic {
                            dest: Some(dest),
                            op: *add_op,
                            dest_ptr: None,
                            args: vec![Operand::Value(lv), Operand::Value(negated)],
                        });
                        *changes += 1;
                        Some(dest)
                    }
                }
                IrBinOp::Mul => {
                    // Exactly one side tap-derived, the other invariant.
                    let (tap_node, inv_op) = match (lhs, rhs) {
                        (Operand::Value(l), r) if !is_invariant(*l) => (*l, r),
                        (l, Operand::Value(r)) if !is_invariant(*r) => (*r, l),
                        _ => return None,
                    };
                    match inv_op {
                        Operand::Value(w) if is_invariant(*w) => {}
                        Operand::Const(_) => {}
                        _ => return None,
                    }
                    let tv = emit_recursive(
                        tap_node,
                        func,
                        defs,
                        tap_vecs,
                        elem_ty,
                        add_op,
                        mul_op,
                        madd_op,
                        broadcast_op,
                        preheader_idx,
                        next_val_id,
                        vec_insts,
                        changes,
                        cache,
                    )?;
                    let wv = broadcast(
                        inv_op,
                        func,
                        broadcast_op,
                        preheader_idx,
                        next_val_id,
                        changes,
                        cache,
                    )?;
                    let dest = Value(*next_val_id);
                    *next_val_id += 1;
                    vec_insts.push(Instruction::Intrinsic {
                        dest: Some(dest),
                        op: *mul_op,
                        dest_ptr: None,
                        args: vec![Operand::Value(tv), Operand::Value(wv)],
                    });
                    *changes += 1;
                    Some(dest)
                }
                _ => None,
            },
            Instruction::UnaryOp {
                op, src: operand, ..
            } => match op {
                IrUnaryOp::Neg => {
                    let v_node = match operand {
                        Operand::Value(v) => *v,
                        _ => return None,
                    };
                    let v = emit_recursive(
                        v_node,
                        func,
                        defs,
                        tap_vecs,
                        elem_ty,
                        add_op,
                        mul_op,
                        madd_op,
                        broadcast_op,
                        preheader_idx,
                        next_val_id,
                        vec_insts,
                        changes,
                        cache,
                    )?;
                    let neg = broadcast(
                        &neg_one_const(elem_ty),
                        func,
                        broadcast_op,
                        preheader_idx,
                        next_val_id,
                        changes,
                        cache,
                    )?;
                    let dest = Value(*next_val_id);
                    *next_val_id += 1;
                    vec_insts.push(Instruction::Intrinsic {
                        dest: Some(dest),
                        op: *mul_op,
                        dest_ptr: None,
                        args: vec![Operand::Value(v), Operand::Value(neg)],
                    });
                    *changes += 1;
                    Some(dest)
                }
                _ => None,
            },
            // Any other loop-local definition (Select, Copy, Load, Call,
            // Phi, ...) can NEVER be soundly broadcast: the broadcast lands
            // in the preheader, but the value is defined inside the loop —
            // a use-before-def dominance violation (the broadcast reads the
            // value's uninitialized home and is then reused for every
            // iteration). True invariant leaves never reach this match (the
            // `defs.get(node)?` above already bailed on them; the per-arm
            // `is_invariant` checks broadcast operands directly), so this
            // arm's historical "invariant leaf" comment was wrong: it only
            // ever matched loop-local defs. Fail soft — return None so the
            // stencil transform declines the pattern and the scalar loop
            // runs.
            _ => None,
        }
    }

    let mut broadcast_cache: FxHashMap<u64, Value> = FxHashMap::default();
    emit_recursive(
        pattern.store_val,
        func,
        &defs,
        tap_vecs,
        pattern.elem_ty,
        add_op,
        mul_op,
        madd_op,
        broadcast_op,
        preheader_idx,
        next_val_id,
        vec_insts,
        changes,
        &mut broadcast_cache,
    )
}

/// Insert an exact scalar remainder for a vectorized stencil: a fresh loop
/// over the unprocessed tail elements that recomputes the ORIGINAL
/// expression (same ops, same order) from the tap GEP shapes.
fn insert_stencil_remainder_loop(
    func: &mut IrFunction,
    pattern: &StencilPattern,
    vec_width: u64,
    next_val_id: &mut u32,
    next_label: &mut u32,
) -> usize {
    let mut changes = 0usize;
    let elem_size: i64 = if pattern.elem_ty == IrType::F64 { 8 } else { 4 };
    let int_const = |n: i64| match pattern.iv_ty {
        IrType::I32 | IrType::U32 => IrConst::I32(n as i32),
        _ => IrConst::I64(n),
    };

    let vec_exit_label = BlockId(*next_label);
    *next_label += 1;
    let rem_header_label = BlockId(*next_label);
    *next_label += 1;
    let rem_body_label = BlockId(*next_label);
    *next_label += 1;
    let rem_latch_label = BlockId(*next_label);
    *next_label += 1;

    let i_rem_start = Value(*next_val_id);
    *next_val_id += 1;
    let i_rem_iv = Value(*next_val_id);
    *next_val_id += 1;
    let i_rem_iv_next = Value(*next_val_id);
    *next_val_id += 1;
    let i_rem_cmp = Value(*next_val_id);
    *next_val_id += 1;
    let i_rem_cmp_lhs = Value(*next_val_id);
    *next_val_id += 1;

    // Redirect the vector loop's exit edge to the remainder entry.
    if let Terminator::CondBranch { false_label, .. } =
        &mut func.blocks[pattern.header_idx].terminator
    {
        *false_label = vec_exit_label;
    }

    // Remainder start element = iv_start + (vector-iv − iv_start) * width.
    // The header phi at vector-loop exit holds the next unprocessed scalar
    // IV under unit stepping; the byte-parallel lanes cover the width-fold
    // span from there.
    let delta = Value(*next_val_id);
    *next_val_id += 1;
    let scaled = Value(*next_val_id);
    *next_val_id += 1;
    let vec_exit_block = BasicBlock {
        label: vec_exit_label,
        instructions: vec![
            Instruction::BinOp {
                dest: delta,
                op: IrBinOp::Sub,
                lhs: Operand::Value(pattern.iv),
                rhs: Operand::Const(int_const(pattern.iv_start)),
                ty: pattern.iv_ty,
            },
            Instruction::BinOp {
                dest: scaled,
                op: IrBinOp::Mul,
                lhs: Operand::Value(delta),
                rhs: Operand::Const(int_const(vec_width as i64)),
                ty: pattern.iv_ty,
            },
            Instruction::BinOp {
                dest: i_rem_start,
                op: IrBinOp::Add,
                lhs: Operand::Value(scaled),
                rhs: Operand::Const(int_const(pattern.iv_start)),
                ty: pattern.iv_ty,
            },
        ],
        terminator: Terminator::Branch(rem_header_label),
        source_spans: vec![],
    };

    // Remainder header: the ORIGINAL compare shape (iv + cmp_k) op limit.
    let rem_header_block = BasicBlock {
        label: rem_header_label,
        instructions: vec![
            Instruction::Phi {
                dest: i_rem_iv,
                ty: pattern.iv_ty,
                incoming: vec![
                    (Operand::Value(i_rem_start), vec_exit_label),
                    (Operand::Value(i_rem_iv_next), rem_latch_label),
                ],
            },
            Instruction::BinOp {
                dest: i_rem_cmp_lhs,
                op: IrBinOp::Add,
                lhs: Operand::Value(i_rem_iv),
                rhs: Operand::Const(int_const(pattern.cmp_k)),
                ty: pattern.iv_ty,
            },
            Instruction::Cmp {
                dest: i_rem_cmp,
                op: pattern.exit_cmp_op,
                lhs: Operand::Value(i_rem_cmp_lhs),
                rhs: pattern.limit.clone(),
                ty: pattern.iv_ty,
            },
        ],
        terminator: Terminator::CondBranch {
            cond: Operand::Value(i_rem_cmp),
            true_label: rem_body_label,
            false_label: func.blocks[pattern.exit_idx].label,
        },
        source_spans: vec![],
    };

    // Remainder body: recompute the original expression from the taps.
    let mut body_insts: Vec<Instruction> = Vec::new();
    let cast_v = Value(*next_val_id);
    *next_val_id += 1;
    if matches!(pattern.iv_ty, IrType::I32 | IrType::U32) {
        body_insts.push(Instruction::Cast {
            dest: cast_v,
            src: Operand::Value(i_rem_iv),
            from_ty: pattern.iv_ty,
            to_ty: IrType::I64,
        });
    }
    let byte_index = if matches!(pattern.iv_ty, IrType::I32 | IrType::U32) {
        cast_v
    } else {
        i_rem_iv
    };

    let mut tap_scalar_ptrs: FxHashMap<u32, Value> = FxHashMap::default();
    for tap in &pattern.taps {
        let scaled_v = Value(*next_val_id);
        *next_val_id += 1;
        let offset_v = Value(*next_val_id);
        *next_val_id += 1;
        let gep_v = Value(*next_val_id);
        *next_val_id += 1;
        body_insts.push(Instruction::BinOp {
            dest: scaled_v,
            op: IrBinOp::Mul,
            lhs: Operand::Value(byte_index),
            rhs: Operand::Const(IrConst::I64(elem_size)),
            ty: IrType::I64,
        });
        body_insts.push(Instruction::BinOp {
            dest: offset_v,
            op: IrBinOp::Add,
            lhs: Operand::Value(scaled_v),
            rhs: Operand::Const(IrConst::I64(tap.disp_bytes)),
            ty: IrType::I64,
        });
        body_insts.push(Instruction::GetElementPtr {
            dest: gep_v,
            base: pattern.src_base,
            offset: Operand::Value(offset_v),
            ty: pattern.elem_ty,
        });
        let load_v = Value(*next_val_id);
        *next_val_id += 1;
        body_insts.push(Instruction::Load {
            volatile: false,
            dest: load_v,
            ptr: gep_v,
            ty: pattern.elem_ty,
            seg_override: AddressSpace::Default,
        });
        tap_scalar_ptrs.insert(tap.load.0, load_v);
    }
    // Store address.
    let dst_scaled = Value(*next_val_id);
    *next_val_id += 1;
    let dst_offset = Value(*next_val_id);
    *next_val_id += 1;
    let dst_gep_v = Value(*next_val_id);
    *next_val_id += 1;
    body_insts.push(Instruction::BinOp {
        dest: dst_scaled,
        op: IrBinOp::Mul,
        lhs: Operand::Value(byte_index),
        rhs: Operand::Const(IrConst::I64(elem_size)),
        ty: IrType::I64,
    });
    body_insts.push(Instruction::BinOp {
        dest: dst_offset,
        op: IrBinOp::Add,
        lhs: Operand::Value(dst_scaled),
        rhs: Operand::Const(IrConst::I64(pattern.dst_disp)),
        ty: IrType::I64,
    });
    body_insts.push(Instruction::GetElementPtr {
        dest: dst_gep_v,
        base: pattern.dst_base,
        offset: Operand::Value(dst_offset),
        ty: pattern.elem_ty,
    });

    // Mirror the expression tree in the scalar domain with the fresh tap
    // loads (same ops, same order — an exact copy with remapped leaves).
    let mut remap: FxHashMap<u32, Value> = tap_scalar_ptrs;
    let mut defs: FxHashMap<u32, Instruction> = FxHashMap::default();
    for &b in &pattern.loop_blocks {
        for inst in &func.blocks[b].instructions {
            if let Some(d) = inst.dest() {
                defs.insert(d.0, inst.clone());
            }
        }
    }
    fn remap_expr(
        node: Value,
        defs: &FxHashMap<u32, Instruction>,
        remap: &mut FxHashMap<u32, Value>,
        next_val_id: &mut u32,
        out: &mut Vec<Instruction>,
    ) -> Option<Value> {
        if let Some(&v) = remap.get(&node.0) {
            return Some(v);
        }
        let def = defs.get(&node.0)?.clone();
        match def {
            Instruction::BinOp {
                op, lhs, rhs, ty, ..
            } => {
                let map_op = |o: &Operand,
                              remap: &mut FxHashMap<u32, Value>,
                              next_val_id: &mut u32,
                              out: &mut Vec<Instruction>|
                 -> Option<Operand> {
                    match o {
                        Operand::Value(v) => {
                            remap_expr(*v, defs, remap, next_val_id, out).map(Operand::Value)
                        }
                        c @ Operand::Const(_) => Some(c.clone()),
                    }
                };
                let l = map_op(&lhs, remap, next_val_id, out)?;
                let r = map_op(&rhs, remap, next_val_id, out)?;
                let dest = Value(*next_val_id);
                *next_val_id += 1;
                out.push(Instruction::BinOp {
                    dest,
                    op,
                    lhs: l,
                    rhs: r,
                    ty,
                });
                remap.insert(node.0, dest);
                Some(dest)
            }
            Instruction::UnaryOp {
                op,
                src: operand,
                ty,
                ..
            } => {
                let o = match operand {
                    Operand::Value(v) => {
                        Operand::Value(remap_expr(v, defs, remap, next_val_id, out)?)
                    }
                    c @ Operand::Const(_) => c.clone(),
                };
                let dest = Value(*next_val_id);
                *next_val_id += 1;
                out.push(Instruction::UnaryOp {
                    dest,
                    op,
                    src: o,
                    ty,
                });
                remap.insert(node.0, dest);
                Some(dest)
            }
            // Invariant leaf: reuse the original value directly.
            _ => {
                remap.insert(node.0, node);
                Some(node)
            }
        }
    }
    let Some(result) = remap_expr(
        pattern.store_val,
        &defs,
        &mut remap,
        next_val_id,
        &mut body_insts,
    ) else {
        // Without a remainder the transform would be unsound; the caller
        // already replaced the store, so report failure (0 added blocks).
        return 0;
    };
    body_insts.push(Instruction::Store {
        val: Operand::Value(result),
        ptr: dst_gep_v,
        ty: pattern.elem_ty,
        seg_override: AddressSpace::Default,
        volatile: false,
    });

    let rem_body_block = BasicBlock {
        label: rem_body_label,
        instructions: body_insts,
        terminator: Terminator::Branch(rem_latch_label),
        source_spans: vec![],
    };
    let rem_latch_block = BasicBlock {
        label: rem_latch_label,
        instructions: vec![Instruction::BinOp {
            dest: i_rem_iv_next,
            op: IrBinOp::Add,
            lhs: Operand::Value(i_rem_iv),
            rhs: Operand::Const(int_const(1)),
            ty: pattern.iv_ty,
        }],
        terminator: Terminator::Branch(rem_header_label),
        source_spans: vec![],
    };

    // The remainder blocks are appended at the end; the final layout pass
    // re-orders everything in reverse post-order.
    func.blocks.push(vec_exit_block);
    func.blocks.push(rem_header_block);
    func.blocks.push(rem_body_block);
    func.blocks.push(rem_latch_block);
    changes += 4;
    changes
}

/// Check if a GEP uses the induction variable.
fn gep_uses_iv(
    func: &IrFunction,
    loop_blocks: &FxHashSet<usize>,
    gep: Value,
    iv: Value,
    iv_derived: &FxHashSet<Value>,
    elem_size: u32,
) -> bool {
    // Find the GEP instruction in the loop
    for &block_idx in loop_blocks {
        let block = &func.blocks[block_idx];
        for inst in &block.instructions {
            if let Instruction::GetElementPtr {
                dest, base, offset, ..
            } = inst
            {
                if *dest == gep {
                    // The reduction transform models the access as
                    // `invariant_base + iv * element_size` and later rewrites
                    // the IV to step in bytes. That model is WRONG when the
                    // GEP's BASE itself depends on the IV — multi-level GEPs
                    // like C[i][i] lower to
                    //     row  = C + (i << 11)      ; IV-dependent base!
                    //     addr = row + (i << 3)     ; looks like stride-8
                    // and after the byte-stride rewrite the row computation
                    // scales with the byte IV (i<<11 over 0..2048 = 4 MB):
                    // out-of-bounds segfault (256x256 matmul diagonal-sum
                    // reproducer). Fail closed: the base must be defined
                    // outside the loop or be a loop-invariant address.
                    if !gep_base_is_loop_invariant(func, loop_blocks, *base, iv, iv_derived) {
                        return false;
                    }
                    // The packed VecLoad consumes elem_size*vec_width
                    // CONTIGUOUS bytes per vector iteration, and the transform
                    // re-scales the whole offset chain by vec_width. That is
                    // only semantics-preserving when the scalar byte offset is
                    // exactly `iv * elem_size` (unit-stride stream of
                    // elements). Struct fields (`s[i].f`, byte stride 12) or
                    // any other affine-but-non-contiguous shape would load
                    // adjacent fields as if they were consecutive elements
                    // (struct_array_sum miscompile) — and the byte-IV bound
                    // rewrite over/under-covers the array. Require the
                    // canonical shape; anything else stays scalar.
                    return match offset {
                        Operand::Value(v) => {
                            offset_is_canonical_unit_stride(func, *v, iv, iv_derived, elem_size)
                        }
                        _ => false,
                    };
                }
            }
        }
    }
    false
}

/// True when `v` is exactly the induction variable scaled by the element
/// size: `iv`, `sext/zext/copy(iv)`, `(cast(iv)) * elem_size`,
/// `elem_size * (cast(iv))`, or `(cast(iv)) << log2(elem_size)`.
///
/// GEP offsets in this IR are BYTE offsets (the remainder loop lowers
/// `a[i]` to `GEP(base, i*elem_size)`), so offset == raw iv means a
/// one-byte stride, which is only the canonical shape for elem_size == 1
/// (never vectorizable) — it is rejected here.
///
/// Every accepted form is closed under the two rewrites the transforms
/// perform: the vec_width scaling (`offset * w` covers w consecutive
/// elements) and the byte-IV relatch (the offset equals the byte-stepping
/// IV). Both require stride == elem_size exactly; this is the single gate
/// that keeps the contiguous-stream model honest.
fn offset_is_canonical_unit_stride(
    func: &IrFunction,
    v: Value,
    iv: Value,
    iv_derived: &FxHashSet<Value>,
    elem_size: u32,
) -> bool {
    if elem_size < 2 || !elem_size.is_power_of_two() {
        return false;
    }
    // Small iterative worklist instead of recursion: the chains are short
    // (cast/copy around a mul/shl), 8 hops is far beyond any real shape.
    let mut cur = v;
    for _ in 0..8 {
        if cur == iv || iv_derived.contains(&cur) {
            // A raw iv/cast-of-iv offset (no mul/shl) is a one-byte stride,
            // which is not the canonical elem_size shape for elem_size >= 2.
            return false;
        }
        let mut def: Option<&Instruction> = None;
        for block in &func.blocks {
            let found = block.instructions.iter().find(|i| i.dest() == Some(cur));
            if found.is_some() {
                def = found;
                break;
            }
        }
        let inst = match def {
            Some(i) => i,
            None => return false,
        };
        match inst {
            Instruction::Copy {
                src: Operand::Value(s),
                ..
            } => {
                cur = *s;
            }
            Instruction::Cast {
                src: Operand::Value(s),
                from_ty,
                to_ty,
                ..
            } => {
                // Accept only widening casts (sext/zext of the same value);
                // narrowing casts change the value — fail closed.
                if !(from_ty.is_integer() && to_ty.is_integer() && to_ty.size() >= from_ty.size()) {
                    return false;
                }
                cur = *s;
            }
            Instruction::BinOp {
                op: IrBinOp::Mul,
                lhs: Operand::Value(a),
                rhs: Operand::Const(c),
                ..
            }
            | Instruction::BinOp {
                op: IrBinOp::Mul,
                lhs: Operand::Const(c),
                rhs: Operand::Value(a),
                ..
            } => {
                return c.to_i64() == Some(elem_size as i64)
                    && scaled_operand_is_iv(func, *a, iv, iv_derived);
            }
            Instruction::BinOp {
                op: IrBinOp::Shl,
                lhs: Operand::Value(a),
                rhs: Operand::Const(c),
                ..
            } => {
                return c.to_i64() == Some(elem_size.trailing_zeros() as i64)
                    && scaled_operand_is_iv(func, *a, iv, iv_derived);
            }
            _ => return false,
        }
    }
    false
}

/// True when `v` is the IV or reaches it through widening casts / copies
/// only. Used as the scaled operand of the canonical `iv * elem_size` /
/// `iv << log2(elem_size)` offset forms; any other derivation fails closed.
fn scaled_operand_is_iv(
    func: &IrFunction,
    v: Value,
    iv: Value,
    iv_derived: &FxHashSet<Value>,
) -> bool {
    let mut cur = v;
    for _ in 0..8 {
        if cur == iv {
            return true;
        }
        // iv_derived holds IV-plus-constant shapes (iv+1 etc.), which are NOT
        // the canonical operand — only exact cast chains qualify beyond the
        // IV itself.
        let mut def: Option<&Instruction> = None;
        for block in &func.blocks {
            let found = block.instructions.iter().find(|i| i.dest() == Some(cur));
            if found.is_some() {
                def = found;
                break;
            }
        }
        let inst = match def {
            Some(i) => i,
            None => return false,
        };
        match inst {
            Instruction::Copy {
                src: Operand::Value(s),
                ..
            } => {
                cur = *s;
            }
            Instruction::Cast {
                src: Operand::Value(s),
                from_ty,
                to_ty,
                ..
            } => {
                // Accept only widening casts (sext/zext of the same value);
                // narrowing casts change the value — fail closed.
                if !(from_ty.is_integer() && to_ty.is_integer() && to_ty.size() >= from_ty.size()) {
                    return false;
                }
                cur = *s;
            }
            _ => return false,
        }
    }
    false
}

/// A GEP base is loop-invariant when its defining instruction lives outside
/// the loop, or is a loop-materialized but constant address (GlobalAddr,
/// Alloca). Anything defined inside the loop — in particular another GEP or
/// arithmetic that involves the IV — disqualifies the access from the
/// contiguous-stream reduction model. Chains of Copy/GlobalAddr are followed;
/// unknown shapes fail closed.
fn gep_base_is_loop_invariant(
    func: &IrFunction,
    loop_blocks: &FxHashSet<usize>,
    base: Value,
    iv: Value,
    iv_derived: &FxHashSet<Value>,
) -> bool {
    if base == iv || iv_derived.contains(&base) {
        return false;
    }
    for &block_idx in loop_blocks {
        for inst in &func.blocks[block_idx].instructions {
            if inst.dest() == Some(base) {
                return match inst {
                    // Constant addresses are invariant regardless of where
                    // they are materialized.
                    Instruction::GlobalAddr { .. } | Instruction::Alloca { .. } => true,
                    Instruction::Copy {
                        src: Operand::Value(v),
                        ..
                    } => gep_base_is_loop_invariant(func, loop_blocks, *v, iv, iv_derived),
                    // GEP/BinOp/Cast/Load/Phi defined IN the loop: treat as
                    // variant (fail closed).
                    _ => false,
                };
            }
        }
    }
    // Not defined in any loop block: defined before the loop — invariant.
    true
}

/// Find exit block (first successor outside the loop).
fn find_exit(func: &IrFunction, loop_info: &loop_analysis::NaturalLoop) -> Option<usize> {
    let debug = std::env::var("LCCC_DEBUG_VECTORIZE").is_ok();

    // Build a mapping from BlockId to block index
    let mut label_to_idx = FxHashMap::default();
    for (idx, block) in func.blocks.iter().enumerate() {
        label_to_idx.insert(block.label.0, idx);
    }

    if debug {
        eprintln!(
            "[VEC-RED]   find_exit: loop body contains block indices: {:?}",
            loop_info.body
        );
    }

    for &block_idx in &loop_info.body {
        let block = &func.blocks[block_idx];
        if debug {
            eprintln!(
                "[VEC-RED]   Block[{}] (label={}) terminator: {:?}",
                block_idx, block.label.0, block.terminator
            );
        }
        match &block.terminator {
            Terminator::CondBranch {
                true_label,
                false_label,
                ..
            } => {
                // Convert BlockId to block index for comparison
                let true_idx = label_to_idx.get(&true_label.0).copied();
                let false_idx = label_to_idx.get(&false_label.0).copied();

                let then_in_loop = true_idx
                    .map(|idx| loop_info.body.contains(&idx))
                    .unwrap_or(false);
                let else_in_loop = false_idx
                    .map(|idx| loop_info.body.contains(&idx))
                    .unwrap_or(false);

                if debug {
                    eprintln!(
                        "[VEC-RED]   Block[{}] CondBranch: true=BlockId({}) [idx={:?}] (in_loop={}), false=BlockId({}) [idx={:?}] (in_loop={})",
                        block_idx,
                        true_label.0,
                        true_idx,
                        then_in_loop,
                        false_label.0,
                        false_idx,
                        else_in_loop
                    );
                }

                if then_in_loop && !else_in_loop {
                    return false_idx;
                } else if !then_in_loop && else_in_loop {
                    return true_idx;
                }
            }
            _ => {}
        }
    }
    None
}

/// Find the latch block (block with backedge to header).
fn find_latch(func: &IrFunction, loop_info: &loop_analysis::NaturalLoop) -> Option<usize> {
    let debug = std::env::var("LCCC_DEBUG_VECTORIZE").is_ok();

    // Get the actual BlockId of the header (not just the index)
    let header_label = func.blocks[loop_info.header].label;

    if debug {
        eprintln!(
            "[VEC-RED]   find_latch: looking for backedge to header BlockId({})",
            header_label.0
        );
    }

    for &block_idx in &loop_info.body {
        let block = &func.blocks[block_idx];
        if debug {
            eprintln!(
                "[VEC-RED]   Block[{}] (label={}) terminator: {:?}",
                block_idx, block.label.0, block.terminator
            );
        }
        match &block.terminator {
            Terminator::Branch(target) if *target == header_label => {
                if debug {
                    eprintln!("[VEC-RED]   Found latch: block[{}]", block_idx);
                }
                return Some(block_idx);
            }
            Terminator::CondBranch {
                true_label,
                false_label,
                ..
            } => {
                if *true_label == header_label || *false_label == header_label {
                    if debug {
                        eprintln!("[VEC-RED]   Found latch: block[{}]", block_idx);
                    }
                    return Some(block_idx);
                }
            }
            _ => {}
        }
    }
    None
}

/// Find an instruction by its destination value in a single block.
fn find_inst_by_dest(block: &BasicBlock, dest: Value) -> Option<&Instruction> {
    for inst in &block.instructions {
        if inst.dest() == Some(dest) {
            return Some(inst);
        }
    }
    None
}

/// Find an instruction by its destination value, searching all loop blocks.
fn find_inst_in_loop<'a>(
    func: &'a IrFunction,
    loop_blocks: &FxHashSet<usize>,
    dest: Value,
) -> Option<(usize, &'a Instruction)> {
    for &block_idx in loop_blocks {
        let block = &func.blocks[block_idx];
        for inst in &block.instructions {
            if inst.dest() == Some(dest) {
                return Some((block_idx, inst));
            }
        }
    }
    None
}

/// Element size in bytes for a vectorizable reduction element type.
fn reduction_element_size(ty: IrType) -> Option<u32> {
    match ty {
        IrType::F64 => Some(8),
        IrType::F32 => Some(4),
        IrType::I32 => Some(4),
        IrType::I64 => Some(8),
        _ => None,
    }
}

/// For a GEP `base + (iv * elem_size)`, return `(base, byte_iv)` where
/// `byte_iv` is the I64 cast of the induction variable (i.e. the value that
/// equals the byte offset once the IV steps by `elem_size * vec_width` bytes
/// instead of one element). Returns `None` when the offset is not the
/// canonical `shl/mul(iv_cast, elem_size)` shape, so the caller can fall back
/// to the element-index scheme (scaled GEP offsets + `iv * vec_width`
/// remainder start).
/// Is `offset_val` exactly `some_index * elem_size` (as `shl` by log2 or an
/// explicit `mul`)?
///
/// Contiguity precondition for the element-index vectorization scheme, which
/// scales this offset by the vector width. That is sound only when successive
/// elements are ADJACENT. An array-of-structs access steps `sizeof(struct)`
/// instead -- `sum += arr[i].i` over `struct S { int i, j, k; }` has a 12-byte
/// stride, so eight vector lanes are not eight consecutive `.i` fields.
/// Scaling it produced `i * 12 * 8` and silently returned the wrong sum
/// (6 instead of 12 for a 3-element array).
fn offset_is_index_times(
    func: &IrFunction,
    loop_blocks: &FxHashSet<usize>,
    offset_val: Value,
    elem_size: u32,
) -> bool {
    if elem_size == 0 {
        return false;
    }
    let log2 = elem_size.trailing_zeros();
    let pow2 = elem_size.is_power_of_two();
    for &block_idx in loop_blocks {
        for inst in &func.blocks[block_idx].instructions {
            let dest = match inst.dest() {
                Some(d) => d,
                None => continue,
            };
            if dest != offset_val {
                continue;
            }
            return match inst {
                Instruction::BinOp {
                    op: IrBinOp::Shl,
                    rhs: Operand::Const(c),
                    ..
                } => pow2 && c.to_i64() == Some(log2 as i64),
                Instruction::BinOp {
                    op: IrBinOp::Mul,
                    lhs,
                    rhs,
                    ..
                } => {
                    let k = match (lhs, rhs) {
                        (Operand::Const(c), _) | (_, Operand::Const(c)) => c.to_i64(),
                        _ => None,
                    };
                    k == Some(elem_size as i64)
                }
                _ => false,
            };
        }
    }
    false
}

fn find_reduction_byte_iv(
    func: &IrFunction,
    loop_blocks: &FxHashSet<usize>,
    gep_val: Value,
    elem_size: u32,
) -> Option<(Value, Value)> {
    let log2 = elem_size.trailing_zeros(); // 4 -> 2, 8 -> 3
    for &block_idx in loop_blocks {
        let block = &func.blocks[block_idx];
        for inst in &block.instructions {
            if let Instruction::GetElementPtr {
                dest, base, offset, ..
            } = inst
            {
                if *dest != gep_val {
                    continue;
                }
                let off_val = match offset {
                    Operand::Value(v) => *v,
                    _ => return None, // constant offset: not IV-based
                };
                for &b2 in loop_blocks {
                    for inst2 in &func.blocks[b2].instructions {
                        if inst2.dest() != Some(off_val) {
                            continue;
                        }
                        match inst2 {
                            Instruction::BinOp {
                                op: IrBinOp::Shl,
                                lhs,
                                rhs,
                                ..
                            } => {
                                if let (Operand::Value(v), Operand::Const(c)) = (lhs, rhs) {
                                    if c.to_i64() == Some(log2 as i64) {
                                        return Some((*base, *v));
                                    }
                                }
                            }
                            Instruction::BinOp {
                                op: IrBinOp::Mul,
                                lhs,
                                rhs,
                                ..
                            } => {
                                let iv_op = match (lhs, rhs) {
                                    (Operand::Value(v), Operand::Const(c)) => c
                                        .to_i64()
                                        .and_then(|k| (k == elem_size as i64).then_some(*v)),
                                    (Operand::Const(c), Operand::Value(v)) => c
                                        .to_i64()
                                        .and_then(|k| (k == elem_size as i64).then_some(*v)),
                                    _ => None,
                                };
                                if let Some(v) = iv_op {
                                    return Some((*base, v));
                                }
                            }
                            _ => {}
                        }
                    }
                }
                // Offset exists but is not a clean shl/mul by elem_size.
                return None;
            }
        }
    }
    None
}

/// Verify that a GEP uses the IV as offset (with or without scale).
fn verify_gep_pattern(block: &BasicBlock, gep_val: Value, iv: Value) -> Option<()> {
    let debug = std::env::var("LCCC_DEBUG_VECTORIZE").is_ok();
    let gep_inst = find_inst_by_dest(block, gep_val);
    if gep_inst.is_none() {
        if debug {
            eprintln!("[VEC]     GEP instruction not found in block");
        }
        return None;
    }
    let gep_inst = gep_inst?;
    match gep_inst {
        Instruction::GetElementPtr { offset, .. } => {
            match offset {
                Operand::Value(v) if *v == iv => {
                    if debug {
                        eprintln!("[VEC]     GEP uses IV directly");
                    }
                    Some(())
                }
                Operand::Value(idx_val) => {
                    // Check for multiply: %idx = mul %iv, scale
                    let mul_inst = find_inst_by_dest(block, *idx_val);
                    if mul_inst.is_none() {
                        if debug {
                            eprintln!("[VEC]     GEP offset value not found in block");
                        }
                        return None;
                    }
                    let mul_inst = mul_inst?;
                    match mul_inst {
                        Instruction::BinOp {
                            op: IrBinOp::Mul,
                            lhs,
                            rhs,
                            ..
                        } => {
                            let lhs_matches = if let Operand::Value(v) = lhs {
                                *v == iv
                            } else {
                                false
                            };
                            let rhs_matches = if let Operand::Value(v) = rhs {
                                *v == iv
                            } else {
                                false
                            };
                            if lhs_matches || rhs_matches {
                                if debug {
                                    eprintln!("[VEC]     GEP uses IV * scale");
                                }
                                Some(())
                            } else {
                                if debug {
                                    eprintln!("[VEC]     GEP offset multiply doesn't involve IV");
                                }
                                None
                            }
                        }
                        _ => {
                            if debug {
                                eprintln!("[VEC]     GEP offset not from Mul instruction");
                            }
                            None
                        }
                    }
                }
                _ => {
                    if debug {
                        eprintln!("[VEC]     GEP offset is not a Value");
                    }
                    None
                }
            }
        }
        _ => {
            if debug {
                eprintln!("[VEC]     Instruction is not GetElementPtr");
            }
            None
        }
    }
}

/// Extract base pointers for C and B arrays from the pattern's GEP values.
/// Returns (c_base, a_ptr, b_base) for use in remainder loop.
fn extract_base_pointers(
    func: &IrFunction,
    pattern: &VectorizablePattern,
) -> (Value, Value, Value) {
    let mut c_base = None;
    let mut b_base = None;

    // Scan all loop blocks to find GEP instructions that define c_gep and b_gep
    for &block_idx in &pattern.loop_blocks {
        let block = &func.blocks[block_idx];
        for inst in &block.instructions {
            if let Instruction::GetElementPtr { dest, base, .. } = inst {
                if *dest == pattern.c_gep {
                    c_base = Some(*base);
                }
                if *dest == pattern.b_gep {
                    b_base = Some(*base);
                }
            }
        }
    }

    let c_base = c_base.expect("Could not find C array base pointer");
    let b_base = b_base.expect("Could not find B array base pointer");

    (c_base, pattern.a_ptr, b_base)
}

/// Insert remainder loop to handle N % vec_width elements after vectorized loop.
///
/// Creates new CFG structure:
/// [vec_header] ⇄ [vec_body] → [vec_latch] → [vec_exit]
///                                             ↓
///                                    [remainder_header] ⇄ [remainder_body] → [remainder_latch]
///                                             ↓
///                                          [exit]
///
/// Returns the number of changes made (typically 4 for the 4 new blocks).
fn insert_remainder_loop(
    func: &mut IrFunction,
    pattern: &VectorizablePattern,
    vec_width: usize,
    next_val_id: &mut u32,
    next_label: &mut u32,
) -> usize {
    let debug = std::env::var("LCCC_DEBUG_VECTORIZE").is_ok();

    // Extract base pointers for arrays
    let (c_base, a_ptr, b_base) = extract_base_pointers(func, pattern);

    // Allocate new block IDs
    let vec_exit_label = BlockId(*next_label);
    *next_label += 1;
    let remainder_header_label = BlockId(*next_label);
    *next_label += 1;
    let remainder_body_label = BlockId(*next_label);
    *next_label += 1;
    let remainder_latch_label = BlockId(*next_label);
    *next_label += 1;

    // Allocate new value IDs
    let j_rem_start = Value(*next_val_id);
    *next_val_id += 1;
    let j_rem_iv = Value(*next_val_id);
    *next_val_id += 1;
    let j_rem_iv_next = Value(*next_val_id);
    *next_val_id += 1;
    let j_rem_cmp = Value(*next_val_id);
    *next_val_id += 1;
    let j_rem_cast = Value(*next_val_id);
    *next_val_id += 1;
    let j_rem_offset = Value(*next_val_id);
    *next_val_id += 1;
    let c_rem_gep = Value(*next_val_id);
    *next_val_id += 1;
    let b_rem_gep = Value(*next_val_id);
    *next_val_id += 1;
    let c_rem_load = Value(*next_val_id);
    *next_val_id += 1;
    let a_rem_load = Value(*next_val_id);
    *next_val_id += 1;
    let b_rem_load = Value(*next_val_id);
    *next_val_id += 1;
    let mul_result = Value(*next_val_id);
    *next_val_id += 1;
    let add_result = Value(*next_val_id);
    *next_val_id += 1;

    if debug {
        eprintln!("[VEC] Creating remainder loop blocks...");
        eprintln!("[VEC]   vec_exit (BlockId({}))", vec_exit_label.0);
        eprintln!(
            "[VEC]   remainder_header (BlockId({}))",
            remainder_header_label.0
        );
        eprintln!(
            "[VEC]   remainder_body (BlockId({}))",
            remainder_body_label.0
        );
        eprintln!(
            "[VEC]   remainder_latch (BlockId({}))",
            remainder_latch_label.0
        );
    }

    // Step 1: Modify vectorized header to exit to vec_exit instead of original exit
    // Find the header block and change its false_label (exit branch) to vec_exit
    let header_block = &mut func.blocks[pattern.header_idx];
    if let Terminator::CondBranch {
        true_label: _,
        false_label,
        ..
    } = &mut header_block.terminator
    {
        if debug {
            eprintln!(
                "[VEC]   Redirecting header exit {} → {}",
                false_label, vec_exit_label
            );
        }
        *false_label = vec_exit_label;
    }

    // Step 2: Create vec_exit block.
    // Convert the loop's final IV into the element index where the scalar
    // remainder must start. The IV representation differs by scheme:
    //   - width 2 (legacy SSE2): IV is an ELEMENT index stepping 1 with the
    //     GEP stride doubled -> start = IV * 2.
    //   - width 4 (two-NEON / two-XMM hoisted FmaF64x2): IV is a GROUP index
    //     stepping 1, each group covering 4 doubles (GEP stride x4, loop
    //     limit divided by 4) -> start = IV * 4. Using the byte-offset
    //     formula here computed IV >> 3 = 0, so the scalar remainder
    //     re-accumulated the ENTIRE row on top of the vector result —
    //     matmul C[i][j] came out exactly 2x too large on AArch64 at -O2
    //     (x86 masked it: its FmaF64x2Hoisted was a no-op stub, so the
    //     remainder redid all the work correctly but unvectorized).
    //   - width 16 (quad FmaF64x4, AVX2): IV is a BYTE offset stepping 128
    //     -> start = IV / 8.
    let rem_start_inst = match vec_width {
        2 => Instruction::BinOp {
            dest: j_rem_start,
            op: IrBinOp::Mul,
            lhs: Operand::Value(pattern.iv), // Final element index
            rhs: Operand::Const(IrConst::I32(2)),
            ty: IrType::I32,
        },
        4 => Instruction::BinOp {
            dest: j_rem_start,
            op: IrBinOp::Mul,
            lhs: Operand::Value(pattern.iv), // Final 4-element group index
            rhs: Operand::Const(IrConst::I32(4)),
            ty: IrType::I32,
        },
        _ => Instruction::BinOp {
            dest: j_rem_start,
            op: IrBinOp::LShr,
            lhs: Operand::Value(pattern.iv), // Final non-negative byte offset
            rhs: Operand::Const(IrConst::I32(3)), // log2(sizeof(double))
            ty: IrType::I32,
        },
    };

    let vec_exit_block = BasicBlock {
        label: vec_exit_label,
        instructions: vec![rem_start_inst],
        terminator: Terminator::Branch(remainder_header_label),
        source_spans: vec![],
    };

    // Step 3: Create remainder_header block
    // Phi node for remainder IV and comparison
    let remainder_header_block = BasicBlock {
        label: remainder_header_label,
        instructions: vec![
            Instruction::Phi {
                dest: j_rem_iv,
                ty: IrType::I32,
                incoming: vec![
                    (Operand::Value(j_rem_start), vec_exit_label),
                    (Operand::Value(j_rem_iv_next), remainder_latch_label),
                ],
            },
            Instruction::Cmp {
                dest: j_rem_cmp,
                op: IrCmpOp::Slt, // Signed less-than
                lhs: Operand::Value(j_rem_iv),
                rhs: pattern.limit, // Original N
                ty: IrType::I32,
            },
        ],
        terminator: Terminator::CondBranch {
            cond: Operand::Value(j_rem_cmp),
            true_label: remainder_body_label,
            false_label: func.blocks[pattern.exit_idx].label,
        },
        source_spans: vec![],
    };

    // Step 4: Create remainder_body block
    // Scalar FMA: C[i][j] += A[i][k] * B[k][j]
    let remainder_body_block = BasicBlock {
        label: remainder_body_label,
        instructions: vec![
            // Cast j to i64
            Instruction::Cast {
                dest: j_rem_cast,
                src: Operand::Value(j_rem_iv),
                from_ty: IrType::I32,
                to_ty: IrType::I64,
            },
            // Compute byte offset: j * 8
            Instruction::BinOp {
                dest: j_rem_offset,
                op: IrBinOp::Mul,
                lhs: Operand::Value(j_rem_cast),
                rhs: Operand::Const(IrConst::I64(8)),
                ty: IrType::I64,
            },
            // GEP for C[i][j]
            Instruction::GetElementPtr {
                dest: c_rem_gep,
                base: c_base,
                offset: Operand::Value(j_rem_offset),
                ty: IrType::F64, // Element type, not pointer type
            },
            // GEP for B[k][j]
            Instruction::GetElementPtr {
                dest: b_rem_gep,
                base: b_base,
                offset: Operand::Value(j_rem_offset),
                ty: IrType::F64, // Element type, not pointer type
            },
            // Load C[i][j]
            Instruction::Load {
                volatile: false,
                dest: c_rem_load,
                ptr: c_rem_gep,
                ty: IrType::F64,
                seg_override: AddressSpace::Default,
            },
            // Load A[i][k]
            Instruction::Load {
                volatile: false,
                dest: a_rem_load,
                ptr: a_ptr,
                ty: IrType::F64,
                seg_override: AddressSpace::Default,
            },
            // Load B[k][j]
            Instruction::Load {
                volatile: false,
                dest: b_rem_load,
                ptr: b_rem_gep,
                ty: IrType::F64,
                seg_override: AddressSpace::Default,
            },
            // Multiply A * B
            Instruction::BinOp {
                dest: mul_result,
                op: IrBinOp::Mul, // Type-generic multiply, determined by ty field
                lhs: Operand::Value(a_rem_load),
                rhs: Operand::Value(b_rem_load),
                ty: IrType::F64,
            },
            // Add C + (A * B)
            Instruction::BinOp {
                dest: add_result,
                op: IrBinOp::Add, // Type-generic add, determined by ty field
                lhs: Operand::Value(c_rem_load),
                rhs: Operand::Value(mul_result),
                ty: IrType::F64,
            },
            // Store result back to C[i][j]
            Instruction::Store {
                volatile: false,
                val: Operand::Value(add_result),
                ptr: c_rem_gep,
                ty: IrType::F64,
                seg_override: AddressSpace::Default,
            },
        ],
        terminator: Terminator::Branch(remainder_latch_label),
        source_spans: vec![],
    };

    // Step 5: Create remainder_latch block
    // Increment j and loop back
    let remainder_latch_block = BasicBlock {
        label: remainder_latch_label,
        instructions: vec![Instruction::BinOp {
            dest: j_rem_iv_next,
            op: IrBinOp::Add,
            lhs: Operand::Value(j_rem_iv),
            rhs: Operand::Const(IrConst::I32(1)),
            ty: IrType::I32,
        }],
        terminator: Terminator::Branch(remainder_header_label),
        source_spans: vec![],
    };

    // Step 6: Add all new blocks to the function
    func.blocks.push(vec_exit_block);
    func.blocks.push(remainder_header_block);
    func.blocks.push(remainder_body_block);
    func.blocks.push(remainder_latch_block);

    if debug {
        eprintln!(
            "[VEC] Remainder phi: [(Value({}), BlockId({})), (Value({}), BlockId({}))]",
            j_rem_start.0, vec_exit_label.0, j_rem_iv_next.0, remainder_latch_label.0
        );
        eprintln!("[VEC] Transformation complete: 4 blocks added");
    }

    4 // 4 new blocks added
}

/// Transform the loop to use FmaF64x2 intrinsics.
fn transform_to_fma_f64x2(func: &mut IrFunction, pattern: &VectorizablePattern) -> usize {
    let debug = std::env::var("LCCC_DEBUG_VECTORIZE").is_ok();
    let mut changes = 0;

    // Keep track of the next available Value and BlockId
    let mut next_val_id = func.next_value_id;
    let mut next_label = func.next_label;
    // Defensive: never trust func.next_label alone — some IR producers can
    // leave it stale relative to the blocks actually present. Allocating
    // below an existing label duplicates it and corrupts the CFG
    // (lea_sib_fold -O2 self-loop regression).
    let max_present_label = func.blocks.iter().map(|b| b.label.0).max().unwrap_or(0);
    next_label = std::cmp::max(next_label, max_present_label + 1);

    // Restrict all IV/GEP tracing and modifications to the innermost loop blocks only.
    let innermost_blocks: FxHashSet<usize> =
        [pattern.header_idx, pattern.body_idx, pattern.latch_idx]
            .iter()
            .copied()
            .collect();

    // Build a set of IV-derived values (for finding all IV-related comparisons)
    // Start with the IV from the header, but also trace back from the GEPs to find
    // the actual j-loop IV
    let mut iv_derived = FxHashSet::default();
    iv_derived.insert(pattern.iv);

    // Find the IV used in the B and C GEPs by tracing backwards
    // The GEPs use an offset that's derived from the j-loop IV
    let mut gep_ivs = FxHashSet::default();
    for &block_idx in &innermost_blocks {
        let block = &func.blocks[block_idx];
        for inst in &block.instructions {
            if let Instruction::GetElementPtr {
                dest,
                base: _,
                offset,
                ty: _,
            } = inst
            {
                if *dest == pattern.b_gep || *dest == pattern.c_gep {
                    // This GEP is for B or C array - trace its offset back to find the IV
                    if let Operand::Value(offset_val) = offset {
                        gep_ivs.insert(*offset_val);
                        if debug {
                            eprintln!(
                                "[VEC]   GEP Value({}) uses offset Value({})",
                                dest.0, offset_val.0
                            );
                        }
                    }
                }
            }
        }
    }

    // Trace gep_ivs back through multiplies and casts to find the original IVs
    let mut changed = true;
    while changed {
        changed = false;
        for &block_idx in &innermost_blocks {
            let block = &func.blocks[block_idx];
            for inst in &block.instructions {
                match inst {
                    Instruction::BinOp {
                        dest,
                        op: _,
                        lhs,
                        rhs,
                        ty: _,
                    } => {
                        if gep_ivs.contains(dest) {
                            if let Operand::Value(lhs_val) = lhs {
                                if gep_ivs.insert(*lhs_val) {
                                    changed = true;
                                }
                            }
                            if let Operand::Value(rhs_val) = rhs {
                                if gep_ivs.insert(*rhs_val) {
                                    changed = true;
                                }
                            }
                        }
                    }
                    Instruction::Cast { dest, src, .. } | Instruction::Copy { dest, src } => {
                        if gep_ivs.contains(dest) {
                            if let Operand::Value(src_val) = src {
                                if gep_ivs.insert(*src_val) {
                                    changed = true;
                                }
                            }
                        }
                    }
                    Instruction::Phi { dest, .. } => {
                        if gep_ivs.contains(dest) {
                            // This is likely the j-loop IV!
                            iv_derived.insert(*dest);
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    // Now collect all values derived from ANY of these IVs
    changed = true;
    while changed {
        changed = false;
        for &block_idx in &innermost_blocks {
            let block = &func.blocks[block_idx];
            for inst in &block.instructions {
                match inst {
                    Instruction::Cast { dest, src, .. } | Instruction::Copy { dest, src } => {
                        if let Operand::Value(src_val) = src {
                            if (iv_derived.contains(src_val) || gep_ivs.contains(src_val))
                                && iv_derived.insert(*dest)
                            {
                                changed = true;
                            }
                        }
                    }
                    Instruction::BinOp {
                        dest,
                        op: IrBinOp::Add,
                        lhs,
                        ..
                    } => {
                        // IV increment (j = j + 1)
                        if let Operand::Value(lhs_val) = lhs {
                            if iv_derived.contains(lhs_val) && iv_derived.insert(*dest) {
                                changed = true;
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    if debug {
        eprintln!(
            "[VEC]   IV-derived values (including j-loop IV): {:?}",
            iv_derived
        );
        eprintln!("[VEC]   GEP offset chain: {:?}", gep_ivs);
        eprintln!("[VEC]   Loop contains blocks: {:?}", pattern.loop_blocks);
    }

    // Step 1: Modify ALL comparisons in the loop that compare IV-derived values against the limit
    // This ensures we catch the actual loop exit condition regardless of loop transformations
    {
        // Process two two-lane NEON vectors per iteration (four doubles).
        let halved_limit = match &pattern.limit {
            Operand::Const(IrConst::I32(n)) => Operand::Const(IrConst::I32(*n / 4)),
            Operand::Const(IrConst::I64(n)) => Operand::Const(IrConst::I64(*n / 4)),
            Operand::Value(limit_val) => {
                // Dynamic limit: insert division in header
                let div_dest = Value(next_val_id);
                next_val_id += 1;

                let limit_ty = match &func.blocks[pattern.header_idx].instructions
                    [pattern.exit_cmp_inst_idx]
                {
                    Instruction::Cmp { ty, .. } => *ty,
                    _ => IrType::I64,
                };

                let div_inst = Instruction::BinOp {
                    dest: div_dest,
                    op: IrBinOp::UDiv,
                    lhs: Operand::Value(*limit_val),
                    rhs: Operand::Const(match limit_ty {
                        IrType::I32 => IrConst::I32(4),
                        IrType::I64 => IrConst::I64(4),
                        _ => IrConst::I64(4),
                    }),
                    ty: limit_ty,
                };

                func.blocks[pattern.header_idx]
                    .instructions
                    .insert(pattern.exit_cmp_inst_idx, div_inst);

                if debug {
                    eprintln!(
                        "[VEC]   Inserted division for dynamic limit: Value({})",
                        div_dest.0
                    );
                }

                Operand::Value(div_dest)
            }
            _ => {
                if debug {
                    eprintln!("[VEC]   Unsupported limit type");
                }
                return 0;
            }
        };

        // Modify the exit comparison in the header block only (not outer loop comparisons)
        for block_idx in [pattern.header_idx] {
            let block = &mut func.blocks[block_idx];

            if debug {
                // First pass: log all comparisons in this block
                for inst in &block.instructions {
                    if let Instruction::Cmp {
                        dest,
                        op: _,
                        lhs,
                        rhs,
                        ty: _,
                    } = inst
                    {
                        eprintln!(
                            "[VEC]   Block {} has comparison: {:?} <cmp> {:?}, dest={:?}",
                            block_idx, lhs, rhs, dest
                        );
                    }
                }
            }

            for inst in &mut block.instructions {
                if let Instruction::Cmp {
                    dest,
                    op: _,
                    lhs,
                    rhs,
                    ty: _,
                } = inst
                {
                    // Check if this compares an IV-derived value against something
                    let modifies_lhs = if let Operand::Value(lhs_val) = lhs {
                        iv_derived.contains(lhs_val)
                    } else {
                        false
                    };

                    let modifies_rhs = if let Operand::Value(rhs_val) = rhs {
                        iv_derived.contains(rhs_val)
                    } else {
                        false
                    };

                    if modifies_lhs || modifies_rhs {
                        if debug {
                            eprintln!("[VEC]   -> This is an IV comparison (will modify)");
                        }

                        // Modify the comparison to use halved limit
                        if modifies_lhs {
                            *rhs = halved_limit.clone();
                            changes += 1;
                            if debug {
                                eprintln!(
                                    "[VEC]   -> Modified comparison RHS to {:?}",
                                    halved_limit
                                );
                            }
                        } else if modifies_rhs {
                            *lhs = halved_limit.clone();
                            changes += 1;
                            if debug {
                                eprintln!(
                                    "[VEC]   -> Modified comparison LHS to {:?}",
                                    halved_limit
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    // Step 2: Modify GEP offset calculation from IV*8 to IV*32.
    // Handle both explicit multiplies and strength-reduced pointer increments
    {
        let mut found_any_mul = false;
        let mut modified_any_increment = false;

        // First, try to find and modify explicit IV * 8 multiplies
        for &block_idx in &innermost_blocks {
            let block = &mut func.blocks[block_idx];
            for inst in &mut block.instructions {
                if let Instruction::BinOp {
                    dest,
                    op: IrBinOp::Mul,
                    lhs,
                    rhs,
                    ty: _,
                } = inst
                {
                    found_any_mul = true;
                    // Check if this multiply involves IV-derived values and scale factor 8
                    let lhs_is_iv_derived = if let Operand::Value(v) = lhs {
                        iv_derived.contains(v)
                    } else {
                        false
                    };
                    let rhs_is_8 = matches!(
                        rhs,
                        Operand::Const(IrConst::I64(8)) | Operand::Const(IrConst::I32(8))
                    );

                    if debug && (lhs_is_iv_derived || rhs_is_8) {
                        eprintln!(
                            "[VEC]   Found multiply in block {}: Value({}) = {:?} * {:?}, lhs_is_iv_derived={}, rhs_is_8={}",
                            block_idx, dest.0, lhs, rhs, lhs_is_iv_derived, rhs_is_8
                        );
                    }

                    if lhs_is_iv_derived && rhs_is_8 {
                        // Change 8 to 32 (process 4 doubles).
                        *rhs = match rhs {
                            Operand::Const(IrConst::I64(_)) => Operand::Const(IrConst::I64(32)),
                            Operand::Const(IrConst::I32(_)) => Operand::Const(IrConst::I32(32)),
                            _ => Operand::Const(IrConst::I64(32)),
                        };
                        changes += 1;
                        modified_any_increment = true;
                        if debug {
                            eprintln!(
                                "[VEC]   Changed GEP stride from 8 to 16 for Value({})",
                                dest.0
                            );
                        }
                    }
                }
            }
        }

        // If no IV*8 multiplies found, the loop is strength-reduced
        // Look for pointer increments (ptr + 8) and change them to (ptr + 16)
        if !modified_any_increment {
            if debug {
                eprintln!(
                    "[VEC]   No IV*8 multiplies found, searching for strength-reduced pointer increments"
                );
            }

            // Build a set of pointer values that are used in GEPs for C and B arrays
            let mut pointer_values = FxHashSet::default();
            pointer_values.insert(pattern.c_gep);
            pointer_values.insert(pattern.b_gep);

            // Track pointer values through the loop (they flow through adds and GEPs)
            for &block_idx in &innermost_blocks {
                let block = &func.blocks[block_idx];
                for inst in &block.instructions {
                    match inst {
                        Instruction::GetElementPtr { dest, base, .. } => {
                            if pointer_values.contains(base) || pointer_values.contains(dest) {
                                pointer_values.insert(*dest);
                                pointer_values.insert(*base);
                            }
                        }
                        Instruction::BinOp {
                            dest,
                            op: IrBinOp::Add,
                            lhs,
                            rhs: _,
                            ty: _,
                        } => {
                            if let Operand::Value(lhs_val) = lhs {
                                if pointer_values.contains(lhs_val) {
                                    pointer_values.insert(*dest);
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }

            if debug {
                eprintln!(
                    "[VEC]   Tracked pointer values: {} pointers",
                    pointer_values.len()
                );
            }

            // Now modify all pointer increments by 8 to increment by 16
            for &block_idx in &innermost_blocks {
                let block = &mut func.blocks[block_idx];
                for inst in &mut block.instructions {
                    if let Instruction::BinOp {
                        dest,
                        op: IrBinOp::Add,
                        lhs,
                        rhs,
                        ty: _,
                    } = inst
                    {
                        // Check if incrementing by 8
                        let is_8 = matches!(
                            rhs,
                            Operand::Const(IrConst::I64(8)) | Operand::Const(IrConst::I32(8))
                        );

                        if debug && is_8 {
                            let is_pointer = if let Operand::Value(lhs_val) = lhs {
                                pointer_values.contains(lhs_val)
                            } else {
                                false
                            };
                            eprintln!(
                                "[VEC]   Block {} has add by 8: Value({}) = {:?} + 8, is_pointer={}",
                                block_idx, dest.0, lhs, is_pointer
                            );
                        }

                        // Check if this is a pointer increment
                        let is_pointer_add = if let Operand::Value(lhs_val) = lhs {
                            pointer_values.contains(lhs_val)
                        } else {
                            false
                        };

                        if is_pointer_add && is_8 {
                            *rhs = match rhs {
                                Operand::Const(IrConst::I64(_)) => Operand::Const(IrConst::I64(32)),
                                Operand::Const(IrConst::I32(_)) => Operand::Const(IrConst::I32(32)),
                                _ => Operand::Const(IrConst::I64(32)),
                            };
                            changes += 1;
                            modified_any_increment = true;
                            if debug {
                                eprintln!(
                                    "[VEC]   -> Changed pointer increment from 8 to 16 for Value({})",
                                    dest.0
                                );
                            }
                        }
                    }
                }
            }

            if debug && !modified_any_increment {
                eprintln!("[VEC]   No add-by-8 found in IR - GEPs likely using IV directly");
                eprintln!("[VEC]   Will modify GEP offsets to use IV*2 instead of IV");
            }

            // Collect GEPs that need to be modified (two-pass to avoid borrow issues)
            let mut geps_to_modify = Vec::with_capacity(16);
            for &block_idx in &innermost_blocks {
                let block = &func.blocks[block_idx];
                for (inst_idx, inst) in block.instructions.iter().enumerate() {
                    if let Instruction::GetElementPtr {
                        dest,
                        base: _,
                        offset,
                        ty,
                    } = inst
                    {
                        // Check if this is a GEP for B or C array
                        if *dest == pattern.b_gep || *dest == pattern.c_gep {
                            if let Operand::Value(offset_val) = offset {
                                geps_to_modify.push((block_idx, inst_idx, *offset_val, *ty));
                                if debug {
                                    eprintln!(
                                        "[VEC]   Found GEP in block {} inst {}: Value({}) with offset Value({})",
                                        block_idx, inst_idx, dest.0, offset_val.0
                                    );
                                }
                            }
                        }
                    }
                }
            }

            // Now modify the GEPs by inserting mul instructions and updating offsets
            // Process in reverse order to avoid index shifting issues
            for (block_idx, inst_idx, offset_val, gep_ty) in geps_to_modify.into_iter().rev() {
                let mul_dest = Value(next_val_id);
                next_val_id += 1;

                // Determine the type for the multiply (use I64 for pointer offsets)
                let offset_ty = IrType::I64;

                // Create mul instruction: mul_dest = offset * 2
                let mul_inst = Instruction::BinOp {
                    dest: mul_dest,
                    op: IrBinOp::Mul,
                    lhs: Operand::Value(offset_val),
                    rhs: Operand::Const(IrConst::I64(4)),
                    ty: offset_ty,
                };

                // Insert mul before the GEP
                func.blocks[block_idx]
                    .instructions
                    .insert(inst_idx, mul_inst);

                // Update the GEP's offset (now at inst_idx + 1 due to insertion)
                if let Instruction::GetElementPtr { offset, .. } =
                    &mut func.blocks[block_idx].instructions[inst_idx + 1]
                {
                    *offset = Operand::Value(mul_dest);
                    changes += 1;
                    modified_any_increment = true;
                    if debug {
                        eprintln!(
                            "[VEC]   Inserted mul and updated GEP offset: Value({}) = Value({}) * 2",
                            mul_dest.0, offset_val.0
                        );
                    }
                }
            }

            if debug && !modified_any_increment {
                eprintln!("[VEC]   Warning: Could not modify GEP offsets");
            }
        }
    }

    // Hoist the loop-invariant A[i][k] scalar broadcast to the preheader.
    // AArch64 codegen keeps it in v15, outside the allocator's v16-v31 pool.
    if let Some(preheader_idx) = func.blocks.iter().enumerate().find_map(|(idx, block)| {
        if pattern.loop_blocks.contains(&idx) {
            return None;
        }
        matches!(block.terminator, Terminator::Branch(label)
            if label == func.blocks[pattern.header_idx].label)
        .then_some(idx)
    }) {
        func.blocks[preheader_idx]
            .instructions
            .push(Instruction::Intrinsic {
                dest: None,
                op: IntrinsicOp::BroadcastLoadF64,
                dest_ptr: None,
                args: vec![Operand::Value(pattern.a_ptr)],
            });
        changes += 1;
    }

    // Step 3: Replace the body accumulation with a hoisted FmaF64x2.
    {
        let body = &mut func.blocks[pattern.body_idx];

        // Create FmaF64x2 intrinsic: writes directly to memory, no dest value.
        let intrinsic = Instruction::Intrinsic {
            dest: None,
            op: IntrinsicOp::FmaF64x2Hoisted,
            dest_ptr: Some(pattern.c_gep),
            args: vec![Operand::Value(pattern.b_gep)],
        };

        // Find the store instruction by scanning (store_idx may be stale after Step 2 insertions).
        let store_pos = body.instructions.iter().position(
            |inst| matches!(inst, Instruction::Store { ptr, .. } if *ptr == pattern.c_gep),
        );

        if let Some(pos) = store_pos {
            body.instructions.insert(pos, intrinsic);
            body.instructions.remove(pos + 1);
        } else {
            body.instructions.push(intrinsic);
        }
        changes += 1;
        if debug {
            eprintln!(
                "[VEC]   Inserted hoisted FmaF64x2 intrinsic, dest_ptr=Value({}), B=Value({})",
                pattern.c_gep.0, pattern.b_gep.0
            );
        }

        // The old load/mul/add instructions are now dead. DCE will clean them up.
    }

    // Step 4: Create remainder loop blocks for N % 2 != 0
    {
        let remainder_changes = insert_remainder_loop(
            func,
            pattern,
            4, // two NEON vectors per iteration
            &mut next_val_id,
            &mut next_label,
        );
        changes += remainder_changes;
        if debug {
            eprintln!(
                "[VEC]   Added remainder loop: {} blocks created",
                remainder_changes / 4
            );
        }
    }

    // Update the function's next_value_id and next_label
    func.next_value_id = next_val_id;
    func.next_label = next_label;

    changes
}

/// Transform loop to use AVX2 FmaF64x4 intrinsic (4-wide, 256-bit).
/// Same pattern as SSE2 but processes 4 elements per iteration instead of 2.
fn transform_to_fma_f64x4(func: &mut IrFunction, pattern: &VectorizablePattern) -> usize {
    let debug = std::env::var("LCCC_DEBUG_VECTORIZE").is_ok();
    let mut changes = 0;

    // Keep track of the next available Value and BlockId
    let mut next_val_id = func.next_value_id;
    let mut next_label = func.next_label;
    // Defensive: never trust func.next_label alone — some IR producers can
    // leave it stale relative to the blocks actually present. Allocating
    // below an existing label duplicates it and corrupts the CFG
    // (lea_sib_fold -O2 self-loop regression).
    let max_present_label = func.blocks.iter().map(|b| b.label.0).max().unwrap_or(0);
    next_label = std::cmp::max(next_label, max_present_label + 1);

    // Restrict all IV/GEP tracing and modifications to the innermost loop blocks only.
    // This prevents accidentally modifying comparisons or GEPs in outer loops.
    let innermost_blocks: FxHashSet<usize> =
        [pattern.header_idx, pattern.body_idx, pattern.latch_idx]
            .iter()
            .copied()
            .collect();

    // Build a set of IV-derived values (for finding all IV-related comparisons)
    // Start with the IV from the header, but also trace back from the GEPs to find
    // the actual j-loop IV
    let mut iv_derived = FxHashSet::default();
    iv_derived.insert(pattern.iv);

    // Find the IV used in the B and C GEPs by tracing backwards
    // The GEPs use an offset that's derived from the j-loop IV
    let mut gep_ivs = FxHashSet::default();
    for &block_idx in &innermost_blocks {
        let block = &func.blocks[block_idx];
        for inst in &block.instructions {
            if let Instruction::GetElementPtr {
                dest,
                base: _,
                offset,
                ty: _,
            } = inst
            {
                if *dest == pattern.b_gep || *dest == pattern.c_gep {
                    // This GEP is for B or C array - trace its offset back to find the IV
                    if let Operand::Value(offset_val) = offset {
                        gep_ivs.insert(*offset_val);
                        if debug {
                            eprintln!(
                                "[VEC]   GEP Value({}) uses offset Value({})",
                                dest.0, offset_val.0
                            );
                        }
                    }
                }
            }
        }
    }

    // Trace gep_ivs back through multiplies and casts to find the original IVs
    let mut changed = true;
    while changed {
        changed = false;
        for &block_idx in &innermost_blocks {
            let block = &func.blocks[block_idx];
            for inst in &block.instructions {
                match inst {
                    Instruction::BinOp {
                        dest,
                        op: _,
                        lhs,
                        rhs,
                        ty: _,
                    } => {
                        if gep_ivs.contains(dest) {
                            if let Operand::Value(lhs_val) = lhs {
                                if gep_ivs.insert(*lhs_val) {
                                    changed = true;
                                }
                            }
                            if let Operand::Value(rhs_val) = rhs {
                                if gep_ivs.insert(*rhs_val) {
                                    changed = true;
                                }
                            }
                        }
                    }
                    Instruction::Cast { dest, src, .. } | Instruction::Copy { dest, src } => {
                        if gep_ivs.contains(dest) {
                            if let Operand::Value(src_val) = src {
                                if gep_ivs.insert(*src_val) {
                                    changed = true;
                                }
                            }
                        }
                    }
                    Instruction::Phi { dest, .. } => {
                        if gep_ivs.contains(dest) {
                            // This is likely the j-loop IV!
                            iv_derived.insert(*dest);
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    // Now collect all values derived from ANY of these IVs
    changed = true;
    while changed {
        changed = false;
        for &block_idx in &innermost_blocks {
            let block = &func.blocks[block_idx];
            for inst in &block.instructions {
                match inst {
                    Instruction::Cast { dest, src, .. } | Instruction::Copy { dest, src } => {
                        if let Operand::Value(src_val) = src {
                            if (iv_derived.contains(src_val) || gep_ivs.contains(src_val))
                                && iv_derived.insert(*dest)
                            {
                                changed = true;
                            }
                        }
                    }
                    Instruction::BinOp {
                        dest,
                        op: IrBinOp::Add,
                        lhs,
                        ..
                    } => {
                        // IV increment (j = j + 1)
                        if let Operand::Value(lhs_val) = lhs {
                            if iv_derived.contains(lhs_val) && iv_derived.insert(*dest) {
                                changed = true;
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    if debug {
        eprintln!(
            "[VEC]   IV-derived values (including j-loop IV): {:?}",
            iv_derived
        );
        eprintln!("[VEC]   GEP offset chain: {:?}", gep_ivs);
        eprintln!("[VEC]   Loop contains blocks: {:?}", pattern.loop_blocks);
    }

    // Step 1: Convert loop IV from element index to byte offset.
    // Instead of j=0..N/4 with GEP offset j*32, use byte_off=0..N*8 step 32.
    // This eliminates the multiply in the inner loop (the biggest single win: -8 instructions).
    //
    // Changes:
    //   - Loop limit: N/4 → N*8 (byte limit)
    //   - IV increment: +1 → +32 (bytes per AVX2 iteration = 4 doubles × 8 bytes)
    //   - GEP offset multiply: eliminated (IV IS the byte offset)
    {
        // First, create the byte limit. The vector loop steps by 32 bytes
        // (4 doubles) per iteration, so it must run exactly floor(N/4)
        // iterations: byte_limit = (N/16)*128 = (N & ~3)*8. Using N*8 here
        // instead runs ceil(N/4) iterations — the final iteration reads and
        // writes up to 3 doubles past the end of the array (OOB), and the
        // scalar remainder loop then never fires, silently skipping the tail
        // (reproducer: N=255 matmul clobbered element 255 and skipped
        // elements 252..254, producing a wrong sum).
        let byte_limit = match &pattern.limit {
            Operand::Const(IrConst::I32(n)) => Operand::Const(IrConst::I32((*n / 16) * 128)),
            Operand::Const(IrConst::I64(n)) => Operand::Const(IrConst::I64((*n / 16) * 128)),
            Operand::Value(limit_val) => {
                // Dynamic limit: byte_limit = (N/16)*128, hoisted into the header.
                let limit_ty = match &func.blocks[pattern.header_idx].instructions
                    [pattern.exit_cmp_inst_idx]
                {
                    Instruction::Cmp { ty, .. } => *ty,
                    _ => IrType::I64,
                };

                // N / 16 (quad FMA width)
                let div_dest = Value(next_val_id);
                next_val_id += 1;
                let div_inst = Instruction::BinOp {
                    dest: div_dest,
                    op: IrBinOp::UDiv,
                    lhs: Operand::Value(*limit_val),
                    rhs: Operand::Const(match limit_ty {
                        IrType::I32 => IrConst::I32(16),
                        IrType::I64 => IrConst::I64(16),
                        _ => IrConst::I64(16),
                    }),
                    ty: limit_ty,
                };
                func.blocks[pattern.header_idx]
                    .instructions
                    .insert(pattern.exit_cmp_inst_idx, div_inst);

                // (N/16) * 128
                let mul_dest = Value(next_val_id);
                next_val_id += 1;
                let mul_inst = Instruction::BinOp {
                    dest: mul_dest,
                    op: IrBinOp::Mul,
                    lhs: Operand::Value(div_dest),
                    rhs: Operand::Const(match limit_ty {
                        IrType::I32 => IrConst::I32(128),
                        IrType::I64 => IrConst::I64(128),
                        _ => IrConst::I64(128),
                    }),
                    ty: limit_ty,
                };
                func.blocks[pattern.header_idx]
                    .instructions
                    .insert(pattern.exit_cmp_inst_idx + 1, mul_inst);

                if debug {
                    eprintln!(
                        "[VEC]   Inserted (N/16)*128 dynamic byte limit: Value({})",
                        mul_dest.0
                    );
                }

                Operand::Value(mul_dest)
            }
            _ => {
                if debug {
                    eprintln!("[VEC]   Unsupported limit type");
                }
                return 0;
            }
        };

        // Modify the exit comparison in the header block only (not outer loop comparisons)
        for block_idx in [pattern.header_idx] {
            let block = &mut func.blocks[block_idx];

            if debug {
                // First pass: log all comparisons in this block
                for inst in &block.instructions {
                    if let Instruction::Cmp {
                        dest,
                        op: _,
                        lhs,
                        rhs,
                        ty: _,
                    } = inst
                    {
                        eprintln!(
                            "[VEC]   Block {} has comparison: {:?} <cmp> {:?}, dest={:?}",
                            block_idx, lhs, rhs, dest
                        );
                    }
                }
            }

            for inst in &mut block.instructions {
                if let Instruction::Cmp {
                    dest,
                    op: _,
                    lhs,
                    rhs,
                    ty: _,
                } = inst
                {
                    // Check if this compares an IV-derived value against something
                    let modifies_lhs = if let Operand::Value(lhs_val) = lhs {
                        iv_derived.contains(lhs_val)
                    } else {
                        false
                    };

                    let modifies_rhs = if let Operand::Value(rhs_val) = rhs {
                        iv_derived.contains(rhs_val)
                    } else {
                        false
                    };

                    if modifies_lhs || modifies_rhs {
                        if debug {
                            eprintln!("[VEC]   -> This is an IV comparison (will modify)");
                        }

                        // Modify the comparison to use byte limit (N * 8)
                        if modifies_lhs {
                            *rhs = byte_limit.clone();
                            changes += 1;
                            if debug {
                                eprintln!("[VEC]   -> Modified comparison RHS to {:?}", byte_limit);
                            }
                        } else if modifies_rhs {
                            *lhs = byte_limit.clone();
                            changes += 1;
                            if debug {
                                eprintln!("[VEC]   -> Modified comparison LHS to {:?}", byte_limit);
                            }
                        }
                    }
                }
            }
        }
    }

    // Step 2: Convert GEP offset to use byte-offset IV directly.
    // The IV now represents a byte offset (0, 32, 64, ...) instead of an element index.
    // Replace IV*8 multiplies with a Copy (the IV IS the byte offset).
    // Also change the IV increment from +1 to +32.
    {
        // 2a: Eliminate the IV * 8 multiply by replacing it with a Copy of the IV.
        // The multiply's source (the IV or a cast of it) already holds the byte offset.
        let mut eliminated_mul = false;
        for &block_idx in &innermost_blocks {
            let block = &mut func.blocks[block_idx];
            for inst in &mut block.instructions {
                if let Instruction::BinOp {
                    dest,
                    op: IrBinOp::Mul,
                    lhs,
                    rhs,
                    ty,
                } = inst
                {
                    let lhs_is_iv = if let Operand::Value(v) = lhs {
                        iv_derived.contains(v) || gep_ivs.contains(v)
                    } else {
                        false
                    };
                    let rhs_is_8 = matches!(
                        rhs,
                        Operand::Const(IrConst::I64(8)) | Operand::Const(IrConst::I32(8))
                    );

                    if lhs_is_iv && rhs_is_8 {
                        // Replace multiply with a Copy: the IV already holds the byte offset
                        let mul_dest = *dest;
                        let src = lhs.clone();
                        *inst = Instruction::Copy {
                            dest: mul_dest,
                            src,
                        };
                        eliminated_mul = true;
                        changes += 1;
                        if debug {
                            eprintln!(
                                "[VEC]   Eliminated IV*8 multiply for Value({}) (IV is now byte offset)",
                                mul_dest.0
                            );
                        }
                    }
                }
                // Also handle Shl by 3 (= multiply by 8)
                if let Instruction::BinOp {
                    dest,
                    op: IrBinOp::Shl,
                    lhs,
                    rhs,
                    ty: _,
                } = inst
                {
                    let lhs_is_iv = if let Operand::Value(v) = lhs {
                        iv_derived.contains(v) || gep_ivs.contains(v)
                    } else {
                        false
                    };
                    let rhs_is_3 = matches!(
                        rhs,
                        Operand::Const(IrConst::I64(3)) | Operand::Const(IrConst::I32(3))
                    );

                    if lhs_is_iv && rhs_is_3 {
                        let shl_dest = *dest;
                        let src = lhs.clone();
                        *inst = Instruction::Copy {
                            dest: shl_dest,
                            src,
                        };
                        eliminated_mul = true;
                        changes += 1;
                        if debug {
                            eprintln!(
                                "[VEC]   Eliminated IV<<3 shift for Value({}) (IV is now byte offset)",
                                shl_dest.0
                            );
                        }
                    }
                }
            }
        }

        // 2b: Change the IV increment from +1 to +64 (8 doubles × 8 bytes).
        // Dual FmaF64x4 processes two consecutive 4-wide chunks per iteration.
        {
            let latch = &mut func.blocks[pattern.latch_idx];
            if pattern.iv_inc_idx < latch.instructions.len() {
                if let Instruction::BinOp {
                    op: IrBinOp::Add,
                    rhs,
                    ty,
                    ..
                } = &mut latch.instructions[pattern.iv_inc_idx]
                {
                    *rhs = match rhs {
                        Operand::Const(IrConst::I32(_)) => Operand::Const(IrConst::I32(128)),
                        _ => Operand::Const(IrConst::I64(128)),
                    };
                    // Promote IV increment to I64 to eliminate movslq in backend.
                    // The IV is byte offset (0..2048), always non-negative, so I64 is safe
                    // and avoids sign-extension per iteration.
                    *ty = IrType::I64;
                    changes += 1;
                    if debug {
                        eprintln!("[VEC]   Changed IV increment from +1 to +128 (quad FMA) and promoted to I64");
                    }
                }
            }
        }

        if debug && !eliminated_mul {
            eprintln!("[VEC]   Warning: Could not find IV*8 multiply to eliminate");
        }

        // 2c: Promote IV Phi from I32 to I64 to eliminate movslq per iteration.
        // Original loop: j is i32, but byte offset j*8 needs i64 for SIB addressing.
        // Backend would emit movslq %r12d, %r10 per iteration. By making IV I64,
        // we keep it in 64-bit register from start, saving 2 movslq per iter.
        {
            let header = &mut func.blocks[pattern.header_idx];
            for inst in &mut header.instructions {
                if let Instruction::Phi { dest, ty, incoming } = inst {
                    if *dest == pattern.iv {
                        if *ty == IrType::I32 {
                            *ty = IrType::I64;
                            // Promote incoming consts from I32 to I64
                            for (op, _) in incoming.iter_mut() {
                                if let Operand::Const(IrConst::I32(v)) = op {
                                    *op = Operand::Const(IrConst::I64(*v as i64));
                                }
                            }
                            changes += 1;
                            if debug {
                                eprintln!(
                                    "[VEC]   Promoted IV Phi Value({}) from I32 to I64",
                                    dest.0
                                );
                            }
                        }
                        break;
                    }
                }
            }

            // Also promote any Cast I32->I64 of IV-derived values to Copy (eliminates movslq)
            for &block_idx in &innermost_blocks {
                let block = &mut func.blocks[block_idx];
                for inst in &mut block.instructions {
                    if let Instruction::Cast {
                        dest,
                        src,
                        from_ty: IrType::I32,
                        to_ty: IrType::I64,
                    } = inst
                    {
                        if let Operand::Value(v) = src {
                            if iv_derived.contains(v) || *v == pattern.iv {
                                // Replace Cast with Copy: IV is now I64, no conversion needed
                                let cast_dest = *dest;
                                let cast_src = src.clone();
                                *inst = Instruction::Copy {
                                    dest: cast_dest,
                                    src: cast_src,
                                };
                                changes += 1;
                                if debug {
                                    eprintln!(
                                        "[VEC]   Promoted Cast I32->I64 for Value({}) to Copy (IV now I64)",
                                        cast_dest.0
                                    );
                                }
                            }
                        }
                    }
                }
            }

            // Promote comparison type to I64 if it compares IV
            for block_idx in [pattern.header_idx] {
                let block = &mut func.blocks[block_idx];
                for inst in &mut block.instructions {
                    if let Instruction::Cmp { lhs, rhs, ty, .. } = inst {
                        let lhs_is_iv = if let Operand::Value(v) = lhs {
                            iv_derived.contains(v) || *v == pattern.iv
                        } else {
                            false
                        };
                        let rhs_is_iv = if let Operand::Value(v) = rhs {
                            iv_derived.contains(v) || *v == pattern.iv
                        } else {
                            false
                        };
                        if lhs_is_iv || rhs_is_iv {
                            if *ty == IrType::I32 {
                                *ty = IrType::I64;
                                // Also promote const RHS/LHS if needed
                                if let Operand::Const(IrConst::I32(v)) = rhs {
                                    *rhs = Operand::Const(IrConst::I64(*v as i64));
                                }
                                if let Operand::Const(IrConst::I32(v)) = lhs {
                                    *lhs = Operand::Const(IrConst::I64(*v as i64));
                                }
                                changes += 1;
                                if debug {
                                    eprintln!("[VEC]   Promoted IV comparison to I64");
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // Hoist the loop-invariant A[i][k] scalar broadcast to the preheader.
    // Backend keeps it in ymm1, reused across all 4 FMA lanes (saves 3× vmovsd+vbroadcastsd per iter).
    if let Some(preheader_idx) = func.blocks.iter().enumerate().find_map(|(idx, block)| {
        if pattern.loop_blocks.contains(&idx) {
            return None;
        }
        matches!(block.terminator, Terminator::Branch(label)
            if label == func.blocks[pattern.header_idx].label)
        .then_some(idx)
    }) {
        func.blocks[preheader_idx]
            .instructions
            .push(Instruction::Intrinsic {
                dest: None,
                op: IntrinsicOp::BroadcastLoadF64,
                dest_ptr: None,
                args: vec![Operand::Value(pattern.a_ptr)],
            });
        changes += 1;
        if debug {
            eprintln!(
                "[VEC]   Hoisted BroadcastLoadF64 for A ptr Value({}) into preheader block {}",
                pattern.a_ptr.0, preheader_idx
            );
        }
    }

    // Step 3: Replace the body with FOUR FmaF64x4HoistedSIB (SIB + hoisted broadcast).
    // This is the optimal form: SIB eliminates GEP leaq/movq overhead, hoisted
    // broadcast eliminates vmovsd+vbroadcastsd per chunk.
    // Chunks at byte offsets IV+{0,32,64,96}. Each chunk does:
    //   vmovupd (C_base+off), %ymm0
    //   vfmadd231pd (B_base+off), %ymm1, %ymm0
    //   vmovupd %ymm0, (C_base+off)
    // where %ymm1 already holds A[i][k] from BroadcastLoadF64 in preheader.
    // Total per iteration: 1 movslq (IV conv) + 3 adds (off+32/64/96) + 12 FMA insns
    // vs previous 11 leaq + 8 movq + 12 FMA = 31 insns. Matches GCC SIB pattern.
    {
        let body = &mut func.blocks[pattern.body_idx];
        let store_pos = body.instructions.iter().position(
            |inst| matches!(inst, Instruction::Store { ptr, .. } if *ptr == pattern.c_gep),
        );

        // Extract base pointers and byte offset from existing GEPs.
        // b_base = B row base (B + k*256*8), c_base = C row base (C + i*256*8)
        // b_off = c_off = j*8 (shared IV-derived offset)
        let (b_base, b_off, c_base) = {
            let mut b_base = pattern.b_gep;
            let mut b_off = Operand::Const(IrConst::I64(0));
            let mut c_base = pattern.c_gep;
            for inst in &body.instructions {
                if let Instruction::GetElementPtr {
                    dest, base, offset, ..
                } = inst
                {
                    if *dest == pattern.b_gep {
                        b_base = *base;
                        b_off = offset.clone();
                    }
                    if *dest == pattern.c_gep {
                        c_base = *base;
                        // c_off should equal b_off, but we keep b_off as canonical
                    }
                }
            }
            (b_base, b_off, c_base)
        };

        let mut prelude: Vec<Instruction> = Vec::new();

        // Quad SIB with displacement: all 4 chunks share same byte offset (j*8)
        // plus disp 0,32,64,96 encoded as 4th arg. No extra BinOp Adds needed.
        // This yields optimal asm:
        //   vmovupd (%rbx,%r10), %ymm0 / vfmadd  (%r14,%r10), %ymm1, %ymm0 / vmovupd %ymm0, (%rbx,%r10)
        //   vmovupd 32(%rbx,%r10), %ymm0 / vfmadd 32(%r14,%r10), %ymm1, %ymm0 / ...
        //   vmovupd 64(%rbx,%r10), %ymm0 / ...
        //   vmovupd 96(%rbx,%r10), %ymm0 / ...
        // Total: 1 movslq + 12 FMA insns + loop control, no leaq/addq for offsets.
        for chunk in 0u64..4 {
            let disp = chunk * 32;
            if disp == 0 {
                prelude.push(Instruction::Intrinsic {
                    dest: None,
                    op: IntrinsicOp::FmaF64x4HoistedSIB,
                    dest_ptr: None,
                    args: vec![
                        Operand::Value(c_base),
                        Operand::Value(b_base),
                        b_off.clone(),
                    ],
                });
            } else {
                prelude.push(Instruction::Intrinsic {
                    dest: None,
                    op: IntrinsicOp::FmaF64x4HoistedSIB,
                    dest_ptr: None,
                    args: vec![
                        Operand::Value(c_base),
                        Operand::Value(b_base),
                        b_off.clone(),
                        Operand::Const(IrConst::I64(disp as i64)),
                    ],
                });
            }
        }

        if let Some(pos) = store_pos {
            body.instructions.remove(pos);
            for (k, inst) in prelude.into_iter().enumerate() {
                body.instructions.insert(pos + k, inst);
            }
        } else {
            body.instructions.extend(prelude);
        }
        changes += 4;
        if debug {
            eprintln!("[VEC]   Inserted quad FmaF64x4HoistedSIB intrinsics (SIB + hoisted broadcast, step 128)");
        }
    }

    // Step 4: Create remainder loop blocks for N % 4 != 0
    {
        let remainder_changes = insert_remainder_loop(
            func,
            pattern,
            16, // quad FmaF64x4 = 16 doubles per iteration
            &mut next_val_id,
            &mut next_label,
        );
        changes += remainder_changes;
        if debug {
            eprintln!(
                "[VEC]   Added remainder loop: {} blocks created",
                remainder_changes / 4
            );
        }
    }

    // Update the function's next_value_id and next_label
    func.next_value_id = next_val_id;
    func.next_label = next_label;

    changes
}

/// Insert remainder loop for reduction patterns.
/// Creates 4 blocks:
/// - vec_exit: Performs horizontal reduction and computes remainder start index
/// - remainder_header: Loop header with IV phi and accumulator phi
/// - remainder_body: Scalar reduction operation
/// - remainder_latch: IV increment
/// Resolve a reduction GEP to its base pointer (the GEP lives in the body
/// block); fall back to the GEP itself when not found.
fn reduction_gep_base(func: &IrFunction, body_idx: usize, gep: Value) -> Value {
    let body_block = &func.blocks[body_idx];
    for inst in &body_block.instructions {
        if let Instruction::GetElementPtr { dest, base, .. } = inst {
            if *dest == gep {
                return *base;
            }
        }
    }
    gep
}

/// Repair uses of the loop counter that ESCAPE a vectorized loop.
///
/// # The defect
///
/// Vectorizing redefines what the counter counts. Under the byte-offset scheme
/// it steps `elem_size * vec_width` BYTES per iteration; under the
/// element-index scheme the trip count is divided so it numbers VECTOR
/// iterations. Inside the loop only addresses read it, so neither is visible
/// there. A use AFTER the loop reads a number that is no longer the element
/// index:
///
/// ```c
/// for (; n < max; n++) acc += v[n];
/// return acc + (n >> 2);
/// ```
///
/// returned 528 (byte scheme, `n` == 128) or 497 (element scheme, `n` == 4)
/// where GCC 16.2, Clang 23.1, ICC and ICX all return 504 (`n` == 32). A
/// silent wrong answer -- not a crash -- on `for (i = 0; i < n; i++) ...;`
/// followed by any use of `i`.
///
/// # The repair, and why it is free
///
/// The transform already builds a scalar remainder loop whose induction
/// variable counts ELEMENTS: its preheader converts the vector counter back to
/// an element index (`v27 >> 2` for 4-byte elements), and it steps by one
/// until it reaches the ORIGINAL trip bound. Its value on exit is therefore
/// exactly the final value the source-level counter would have had.
///
/// So the correct value already exists in the function and no arithmetic needs
/// to be synthesized: the escaping uses simply have to name it. This costs
/// **zero instructions** and keeps the loop vectorized -- strictly better than
/// declining to vectorize, and better than materializing
/// `trips * vec_width + remainder` in the exit block, which would add
/// instructions AND have to be kept in agreement with the remainder loop on
/// every exit path.
///
/// # Why the value dominates every escaping use
///
/// The vector loop's only exit goes to the remainder preheader, and the
/// remainder loop's only exit goes to the original loop exit. Every path that
/// leaves the loop nest therefore passes through the remainder header, so its
/// phi dominates everything downstream. The rewrite is restricted to blocks
/// that existed before the remainder was created (`outside_labels`), which
/// excludes the remainder's own blocks -- their use of the vector counter is
/// the legitimate byte-to-element conversion and must not be touched.
fn rewire_escaping_iv_uses(
    func: &mut IrFunction,
    pattern: &ReductionPattern,
    rem_iv: Option<Value>,
    outside_labels: &[BlockId],
    debug: bool,
) -> usize {
    let iv = pattern.iv;
    let Some(rem_iv) = rem_iv else {
        // No remainder loop was built, so there is no element-counting value
        // to point at. This cannot happen for the element types the reduction
        // path accepts (the only early return in
        // `insert_reduction_remainder_loop` rejects types this transform never
        // reaches), but a wrong answer is not an acceptable failure mode for a
        // "cannot happen".
        debug_assert!(
            false,
            "vectorized a reduction without a remainder loop; an escaping \
             counter would read the vector counter"
        );
        return 0;
    };

    let mut rewrites = 0usize;
    for label in outside_labels {
        let Some(bi) = func.blocks.iter().position(|b| b.label == *label) else {
            continue;
        };
        for inst in func.blocks[bi].instructions.iter_mut() {
            let mut hit = false;
            inst.for_each_used_value(|v| {
                if v == iv.0 {
                    hit = true;
                }
            });
            if hit {
                rewrite_value_use(inst, iv, rem_iv);
                rewrites += 1;
            }
        }
        if terminator_uses_value(&func.blocks[bi].terminator, iv) {
            rewrite_terminator_use(&mut func.blocks[bi].terminator, iv, rem_iv);
            rewrites += 1;
        }
    }

    if debug && rewrites > 0 {
        eprintln!(
            "[VEC-RED]   Rewired {} escaping use(s) of counter v{} to the \
             remainder IV v{}",
            rewrites, iv.0, rem_iv.0
        );
    }
    rewrites
}

/// Replace every use of `old` in `inst` with `new`.
fn rewrite_value_use(inst: &mut Instruction, old: Value, new: Value) {
    inst.for_each_operand_mut(|op| {
        if matches!(op, Operand::Value(v) if *v == old) {
            *op = Operand::Value(new);
        }
    });
    // Operand-shaped fields are covered above; the pointer-like fields that
    // hold a bare `Value` are not.
    match inst {
        Instruction::Load { ptr, .. } => {
            if *ptr == old {
                *ptr = new;
            }
        }
        Instruction::GetElementPtr { base, .. } => {
            if *base == old {
                *base = new;
            }
        }
        _ => {}
    }
}

fn terminator_uses_value(term: &Terminator, v: Value) -> bool {
    let is = |op: &Operand| matches!(op, Operand::Value(x) if *x == v);
    match term {
        Terminator::Return(Some(op)) => is(op),
        Terminator::CondBranch { cond, .. } => is(cond),
        Terminator::Switch { val, .. } => is(val),
        Terminator::IndirectBranch { target, .. } => is(target),
        _ => false,
    }
}

fn rewrite_terminator_use(term: &mut Terminator, old: Value, new: Value) {
    let mut fix = |op: &mut Operand| {
        if matches!(op, Operand::Value(x) if *x == old) {
            *op = Operand::Value(new);
        }
    };
    match term {
        Terminator::Return(Some(op)) => fix(op),
        Terminator::CondBranch { cond, .. } => fix(cond),
        Terminator::Switch { val, .. } => fix(val),
        Terminator::IndirectBranch { target, .. } => fix(target),
        _ => {}
    }
}

fn insert_reduction_remainder_loop(
    func: &mut IrFunction,
    pattern: &ReductionPattern,
    vec_width: u64,
    horizontal_intrinsic: IntrinsicOp,
    vec_sum_value: Value, // Accumulated vector SSA value
    byte_offset_iv: bool,
    second_acc: Option<Value>, // Second vector accumulator phi (NEON smlal2 half)
    seconds: &[SecondaryAccumulator], // Extra independent accumulators (multi-reduction)
    next_val_id: &mut u32,
    next_label: &mut u32,
    // Out: the remainder loop's induction variable, which counts ELEMENTS and
    // runs to the original trip count. It is the correct final value of the
    // source-level loop counter, so the caller can rewire uses that escape the
    // loop to it. See `rewire_escaping_iv_uses`.
    rem_iv_out: &mut Option<Value>,
) -> usize {
    let debug = std::env::var("LCCC_DEBUG_VECTORIZE").is_ok();

    // Get element size in bytes
    let element_size: i64 = match pattern.element_type {
        IrType::F64 => 8,
        IrType::F32 => 4,
        IrType::I32 => 4,
        IrType::I64 => 8,
        _ => return 0,
    };

    // Extract base pointers for arrays from the GEP instructions
    let array_a_base = {
        let body_block = &func.blocks[pattern.body_idx];
        let mut base = None;
        for inst in &body_block.instructions {
            if let Instruction::GetElementPtr { dest, base: b, .. } = inst {
                if *dest == pattern.array_a_gep {
                    base = Some(*b);
                    break;
                }
            }
        }
        // Max reductions access through a marching-pointer phi. The remainder
        // must address relative to the phi's PREHEADER incoming — the pointer
        // at element c — NOT the phi itself: as an SSA value read in
        // vec_exit, the phi holds the END-of-vector-loop pointer, so using
        // it double-counts the vector coverage (observed: reads at element
        // ~2x the intended index, out-of-bounds for large n).
        if base.is_none() && pattern.kind == ReductionKind::Max {
            let latch_label = func.blocks[pattern.latch_idx].label;
            for inst in &func.blocks[pattern.header_idx].instructions {
                if let Instruction::Phi { dest, incoming, .. } = inst {
                    if *dest == pattern.array_a_gep {
                        for (op, lbl) in incoming {
                            if *lbl != latch_label {
                                if let Operand::Value(v) = op {
                                    base = Some(*v);
                                }
                            }
                        }
                    }
                }
            }
        }
        let result = base.unwrap_or(pattern.array_a_gep);
        if std::env::var("LCCC_DEBUG_VECTORIZE").is_ok() {
            eprintln!(
                "[VEC-RED] array_a_base = Value({}), array_a_gep = Value({})",
                result.0, pattern.array_a_gep.0
            );
        }
        result
    };

    let array_b_base = pattern.array_b_gep.and_then(|gep| {
        let body_block = &func.blocks[pattern.body_idx];
        for inst in &body_block.instructions {
            if let Instruction::GetElementPtr { dest, base, .. } = inst {
                if *dest == gep {
                    return Some(*base);
                }
            }
        }
        Some(gep)
    });

    // Allocate new block IDs
    let vec_exit_label = BlockId(*next_label);
    *next_label += 1;
    let remainder_header_label = BlockId(*next_label);
    *next_label += 1;
    let remainder_body_label = BlockId(*next_label);
    *next_label += 1;
    let remainder_latch_label = BlockId(*next_label);
    *next_label += 1;

    // Allocate new value IDs
    let scalar_sum = Value(*next_val_id);
    *next_val_id += 1;
    let i_rem_start = Value(*next_val_id);
    *next_val_id += 1;
    // Max-with-shifted-IV temporaries (unused unless max_shift != 0).
    let i_rem_start_pre = Value(*next_val_id);
    *next_val_id += 1;
    let max_limit_adj = Value(*next_val_id);
    *next_val_id += 1;
    // Max with IV init c != 0: the vector loop covered elements
    // [c, c + w*(lim - c)) through the marching pointer, and the remainder
    // addresses RELATIVE to element c (the pointer's preheader value). Its
    // start index is w*(iv_final - c) and its limit is n - c.
    let max_shift: i64 = if pattern.kind == ReductionKind::Max {
        let latch_label = func.blocks[pattern.latch_idx].label;
        let mut c = 0i64;
        for inst in &func.blocks[pattern.header_idx].instructions {
            if let Instruction::Phi { dest, incoming, .. } = inst {
                if *dest == pattern.iv {
                    for (op, lbl) in incoming {
                        if *lbl != latch_label {
                            if let Operand::Const(k) = op {
                                c = k.to_i64().unwrap_or(0);
                            }
                        }
                    }
                }
            }
        }
        c
    } else {
        0
    };
    let i_rem_iv = Value(*next_val_id);
    *next_val_id += 1;
    *rem_iv_out = Some(i_rem_iv);
    let i_rem_iv_next = Value(*next_val_id);
    *next_val_id += 1;
    let i_rem_cmp = Value(*next_val_id);
    *next_val_id += 1;
    let i_rem_cast = Value(*next_val_id);
    *next_val_id += 1;
    let offset_a = Value(*next_val_id);
    *next_val_id += 1;
    let gep_rem_a = Value(*next_val_id);
    *next_val_id += 1;
    let load_rem_a = Value(*next_val_id);
    *next_val_id += 1;
    let load_rem_a_acc = Value(*next_val_id);
    *next_val_id += 1;
    let sum_rem_phi = Value(*next_val_id);
    *next_val_id += 1;
    let sum_rem_next = Value(*next_val_id);
    *next_val_id += 1;

    // Additional values for dot product
    let (offset_b, gep_rem_b, load_rem_b, mul_rem) = if pattern.kind == ReductionKind::DotProduct {
        let vals = (
            Value(*next_val_id),
            Value(*next_val_id + 1),
            Value(*next_val_id + 2),
            Value(*next_val_id + 3),
        );
        *next_val_id += 4;
        vals
    } else {
        (Value(0), Value(0), Value(0), Value(0))
    };
    let load_rem_b_acc = Value(*next_val_id);
    *next_val_id += 1;

    // Each additional accumulator gets its own scalar remainder chain.
    let mut extras: Vec<RemainderAcc> = Vec::with_capacity(seconds.len());
    for _ in seconds {
        let mut acc = RemainderAcc {
            scalar_sum: Value(*next_val_id),
            sum_rem_phi: Value(*next_val_id + 1),
            sum_rem_next: Value(*next_val_id + 2),
            offset_a: Value(*next_val_id + 3),
            gep_rem_a: Value(*next_val_id + 4),
            load_rem_a: Value(*next_val_id + 5),
            offset_b: Value(*next_val_id + 6),
            gep_rem_b: Value(*next_val_id + 7),
            load_rem_b: Value(*next_val_id + 8),
            mul_rem: Value(*next_val_id + 9),
        };
        *next_val_id += 10;
        if pattern.kind != ReductionKind::DotProduct {
            acc.offset_b = Value(0);
            acc.gep_rem_b = Value(0);
            acc.load_rem_b = Value(0);
            acc.mul_rem = Value(0);
        }
        extras.push(acc);
    }

    if debug {
        eprintln!("[VEC-RED] Creating remainder loop blocks...");
        eprintln!("[VEC-RED]   vec_exit (BlockId({}))", vec_exit_label.0);
        eprintln!(
            "[VEC-RED]   remainder_header (BlockId({}))",
            remainder_header_label.0
        );
        eprintln!(
            "[VEC-RED]   remainder_body (BlockId({}))",
            remainder_body_label.0
        );
        eprintln!(
            "[VEC-RED]   remainder_latch (BlockId({}))",
            remainder_latch_label.0
        );
    }

    // Step 1: Redirect vectorized header to vec_exit instead of original exit
    let header_block = &mut func.blocks[pattern.header_idx];
    if let Terminator::CondBranch { false_label, .. } = &mut header_block.terminator {
        if debug {
            eprintln!(
                "[VEC-RED]   Redirecting header exit {} → {}",
                false_label, vec_exit_label
            );
        }
        *false_label = vec_exit_label;
    }

    // Step 2: Create vec_exit block
    // Performs horizontal reduction and computes remainder start index

    // Map to register-based horizontal reduction intrinsic
    let vec_horizontal_op = match horizontal_intrinsic {
        IntrinsicOp::HorizontalAddF64x4 => IntrinsicOp::VecHorizontalAddF64x4,
        IntrinsicOp::HorizontalAddF64x2 => IntrinsicOp::VecHorizontalAddF64x2,
        IntrinsicOp::HorizontalAddI32x8 => IntrinsicOp::VecHorizontalAddI32x8,
        IntrinsicOp::HorizontalAddI32x4 => IntrinsicOp::VecHorizontalAddI32x4,
        _ => horizontal_intrinsic, // Fallback
    };

    let vec_exit_block = {
        // When a second NEON accumulator was used (smlal/smlal2 split), fold
        // the two vector accumulators together before the horizontal reduce.
        let (horiz_src, mut prefix) = if let Some(acc2) = second_acc {
            let combined = Value(*next_val_id);
            *next_val_id += 1;
            (
                combined,
                vec![Instruction::Intrinsic {
                    dest: Some(combined),
                    op: IntrinsicOp::VecAddI64x2,
                    dest_ptr: None,
                    args: vec![
                        Operand::Value(pattern.accumulator_phi),
                        Operand::Value(acc2),
                    ],
                }],
            )
        } else {
            (pattern.accumulator_phi, Vec::new())
        };
        let mut instructions = Vec::with_capacity(16);
        instructions.append(&mut prefix);
        // Horizontal reduction: scalar_sum = reduce(vec_accumulator)
        // Use the accumulator PHI (not vec_sum_value) so that when the
        // vectorized loop has 0 iterations, we reduce the initial zero
        // vector instead of an uninitialized temporary.
        instructions.push(Instruction::Intrinsic {
            dest: Some(scalar_sum),
            op: vec_horizontal_op,
            dest_ptr: None,
            args: vec![Operand::Value(horiz_src)],
        });
        // Each additional accumulator reduces to its own scalar
        // (multi-reduction); its vector phi is rewired the same way as the
        // primary's and is only live inside the loop.
        for (sec, acc) in seconds.iter().zip(extras.iter()) {
            instructions.push(Instruction::Intrinsic {
                dest: Some(acc.scalar_sum),
                op: vec_horizontal_op,
                dest_ptr: None,
                args: vec![Operand::Value(sec.accumulator_phi)],
            });
        }
        // Compute starting index for the scalar remainder loop.
        //   byte-offset IV: start = byte_iv_final >> log2(element_size)
        //   element-index IV: start = iv_final * vec_width
        // Vector byte IVs are normalized at zero, increase by a positive
        // power-of-two stride, and only reach this block from the loop's
        // non-negative exit.  Use that proof directly rather than leaving
        // a late signed divide for x86 codegen.
        instructions.push(if byte_offset_iv {
            Instruction::BinOp {
                dest: i_rem_start,
                op: IrBinOp::LShr,
                lhs: Operand::Value(pattern.iv),
                rhs: Operand::Const(IrConst::I32(element_size.trailing_zeros() as i32)),
                ty: IrType::I32,
            }
        } else {
            Instruction::BinOp {
                dest: if max_shift != 0 {
                    i_rem_start_pre
                } else {
                    i_rem_start
                },
                op: IrBinOp::Mul,
                lhs: Operand::Value(pattern.iv),
                rhs: Operand::Const(IrConst::I32(vec_width as i32)),
                ty: IrType::I32,
            }
        });
        // Max with IV init c != 0: start = w*iv_final - w*c (relative to
        // element c), and a dynamic limit becomes n - c.
        if max_shift != 0 {
            instructions.push(Instruction::BinOp {
                dest: i_rem_start,
                op: IrBinOp::Sub,
                lhs: Operand::Value(i_rem_start_pre),
                rhs: Operand::Const(IrConst::I32((max_shift * vec_width as i64) as i32)),
                ty: IrType::I32,
            });
            if let Operand::Value(lv) = &pattern.limit {
                instructions.push(Instruction::BinOp {
                    dest: max_limit_adj,
                    op: IrBinOp::Sub,
                    lhs: Operand::Value(*lv),
                    rhs: Operand::Const(IrConst::I32(max_shift as i32)),
                    ty: IrType::I32,
                });
            }
        }
        BasicBlock {
            label: vec_exit_label,
            instructions,
            terminator: Terminator::Branch(remainder_header_label),
            source_spans: vec![],
        }
    };

    // Step 3: Create remainder_header block
    let mut remainder_header_instructions = vec![
        // IV phi
        Instruction::Phi {
            dest: i_rem_iv,
            ty: IrType::I32,
            incoming: vec![
                (Operand::Value(i_rem_start), vec_exit_label),
                (Operand::Value(i_rem_iv_next), remainder_latch_label),
            ],
        },
        // Accumulator phi (receives scalar_sum from horizontal reduction!)
        Instruction::Phi {
            dest: sum_rem_phi,
            ty: pattern.accumulator_type,
            incoming: vec![
                (Operand::Value(scalar_sum), vec_exit_label),
                (Operand::Value(sum_rem_next), remainder_latch_label),
            ],
        },
    ];
    // Extra accumulator phis (multi-reduction): same shape, own result.
    for acc in &extras {
        remainder_header_instructions.push(Instruction::Phi {
            dest: acc.sum_rem_phi,
            ty: pattern.accumulator_type,
            incoming: vec![
                (Operand::Value(acc.scalar_sum), vec_exit_label),
                (Operand::Value(acc.sum_rem_next), remainder_latch_label),
            ],
        });
    }
    // Comparison
    remainder_header_instructions.push(Instruction::Cmp {
        dest: i_rem_cmp,
        op: IrCmpOp::Slt,
        lhs: Operand::Value(i_rem_iv),
        rhs: if max_shift != 0 {
            // Max with IV init c != 0: the remainder counts RELATIVE to
            // element c, so the bound is n - c (folded for constants, the
            // vec_exit Sub for dynamic limits).
            match &pattern.limit {
                Operand::Const(IrConst::I32(n)) => {
                    Operand::Const(IrConst::I32(n - max_shift as i32))
                }
                Operand::Const(IrConst::I64(n)) => Operand::Const(IrConst::I64(n - max_shift)),
                Operand::Value(_) => Operand::Value(max_limit_adj),
                other => other.clone(),
            }
        } else {
            pattern.limit.clone() // ORIGINAL limit (not divided)
        },
        ty: IrType::I32,
    });

    let remainder_header_block = BasicBlock {
        label: remainder_header_label,
        instructions: remainder_header_instructions,
        terminator: Terminator::CondBranch {
            cond: Operand::Value(i_rem_cmp),
            true_label: remainder_body_label,
            false_label: func.blocks[pattern.exit_idx].label,
        },
        source_spans: vec![],
    };

    // Step 4: Create remainder_body block
    let mut remainder_body_instructions = vec![
        // Cast i to i64
        Instruction::Cast {
            dest: i_rem_cast,
            src: Operand::Value(i_rem_iv),
            from_ty: IrType::I32,
            to_ty: IrType::I64,
        },
        // Compute offset for array A: offset = i * element_size
        Instruction::BinOp {
            dest: offset_a,
            op: IrBinOp::Mul,
            lhs: Operand::Value(i_rem_cast),
            rhs: Operand::Const(IrConst::I64(element_size)),
            ty: IrType::I64,
        },
        // GEP to array_a[i]
        Instruction::GetElementPtr {
            dest: gep_rem_a,
            base: array_a_base,
            offset: Operand::Value(offset_a),
            ty: pattern.element_type,
        },
        // Load array_a[i]
        Instruction::Load {
            volatile: false,
            dest: load_rem_a,
            ptr: gep_rem_a,
            ty: pattern.element_type,
            seg_override: AddressSpace::Default,
        },
    ];

    let scalar_a = if pattern.element_type != pattern.accumulator_type {
        remainder_body_instructions.push(Instruction::Cast {
            dest: load_rem_a_acc,
            src: Operand::Value(load_rem_a),
            from_ty: pattern.element_type,
            to_ty: pattern.accumulator_type,
        });
        load_rem_a_acc
    } else {
        load_rem_a
    };

    // Add pattern-specific operations
    match pattern.kind {
        ReductionKind::Sum => {
            if let Some(guard_rhs) = &pattern.guard_rhs {
                // Conditional sum: `if (a[i] > rhs) sum += a[i]` — mirror
                // the guard in the scalar tail (element-type compare on the
                // UNCAST loaded value, matching the original C semantics).
                let cmp_rem = Value(*next_val_id);
                *next_val_id += 1;
                let add_rem = Value(*next_val_id);
                *next_val_id += 1;
                remainder_body_instructions.push(Instruction::Cmp {
                    dest: cmp_rem,
                    op: IrCmpOp::Sgt,
                    lhs: Operand::Value(load_rem_a),
                    rhs: guard_rhs.clone(),
                    ty: pattern.element_type,
                });
                remainder_body_instructions.push(Instruction::BinOp {
                    dest: add_rem,
                    op: IrBinOp::Add,
                    lhs: Operand::Value(sum_rem_phi),
                    rhs: Operand::Value(scalar_a),
                    ty: pattern.accumulator_type,
                });
                remainder_body_instructions.push(Instruction::Select {
                    dest: sum_rem_next,
                    cond: Operand::Value(cmp_rem),
                    true_val: Operand::Value(add_rem),
                    false_val: Operand::Value(sum_rem_phi),
                    ty: pattern.accumulator_type,
                });
            } else {
                // Simple sum: sum += array_a[i]
                remainder_body_instructions.push(Instruction::BinOp {
                    dest: sum_rem_next,
                    op: IrBinOp::Add,
                    lhs: Operand::Value(sum_rem_phi),
                    rhs: Operand::Value(scalar_a),
                    ty: pattern.accumulator_type,
                });
            }
        }
        ReductionKind::Max => {
            // mx = max(mx, x) as a scalar Select: take x when x > mx.
            // Signed compare matches the detector's signed-only gate.
            let cmp_rem = Value(*next_val_id);
            *next_val_id += 1;
            remainder_body_instructions.push(Instruction::Cmp {
                dest: cmp_rem,
                op: IrCmpOp::Sgt,
                lhs: Operand::Value(scalar_a),
                rhs: Operand::Value(sum_rem_phi),
                ty: pattern.accumulator_type,
            });
            remainder_body_instructions.push(Instruction::Select {
                dest: sum_rem_next,
                cond: Operand::Value(cmp_rem),
                true_val: Operand::Value(scalar_a),
                false_val: Operand::Value(sum_rem_phi),
                ty: pattern.accumulator_type,
            });
        }
        ReductionKind::DotProduct => {
            // Dot product: sum += a[i] * b[i]
            remainder_body_instructions.extend_from_slice(&[
                // Compute offset for array B (same as A)
                Instruction::BinOp {
                    dest: offset_b,
                    op: IrBinOp::Mul,
                    lhs: Operand::Value(i_rem_cast),
                    rhs: Operand::Const(IrConst::I64(element_size)),
                    ty: IrType::I64,
                },
                // GEP to array_b[i]
                Instruction::GetElementPtr {
                    dest: gep_rem_b,
                    base: array_b_base.unwrap(),
                    offset: Operand::Value(offset_b),
                    ty: pattern.element_type,
                },
                // Load array_b[i]
                Instruction::Load {
                    volatile: false,
                    dest: load_rem_b,
                    ptr: gep_rem_b,
                    ty: pattern.element_type,
                    seg_override: AddressSpace::Default,
                },
            ]);
            let scalar_b = if pattern.element_type != pattern.accumulator_type {
                remainder_body_instructions.push(Instruction::Cast {
                    dest: load_rem_b_acc,
                    src: Operand::Value(load_rem_b),
                    from_ty: pattern.element_type,
                    to_ty: pattern.accumulator_type,
                });
                load_rem_b_acc
            } else {
                load_rem_b
            };
            remainder_body_instructions.extend_from_slice(&[
                // Multiply a[i] * b[i]
                Instruction::BinOp {
                    dest: mul_rem,
                    op: IrBinOp::Mul,
                    lhs: Operand::Value(scalar_a),
                    rhs: Operand::Value(scalar_b),
                    ty: pattern.accumulator_type,
                },
                // Add to accumulator
                Instruction::BinOp {
                    dest: sum_rem_next,
                    op: IrBinOp::Add,
                    lhs: Operand::Value(sum_rem_phi),
                    rhs: Operand::Value(mul_rem),
                    ty: pattern.accumulator_type,
                },
            ]);
        }
    }

    // Extra accumulators' scalar chains (multi-reduction).  Same shape as the
    // primary; the analyzer guarantees element_type == accumulator_type here,
    // so no per-element casts are needed.
    for (sec, acc) in seconds.iter().zip(extras.iter()) {
        let base_a = reduction_gep_base(func, pattern.body_idx, sec.array_a_gep);
        remainder_body_instructions.extend_from_slice(&[
            Instruction::BinOp {
                dest: acc.offset_a,
                op: IrBinOp::Mul,
                lhs: Operand::Value(i_rem_cast),
                rhs: Operand::Const(IrConst::I64(element_size)),
                ty: IrType::I64,
            },
            Instruction::GetElementPtr {
                dest: acc.gep_rem_a,
                base: base_a,
                offset: Operand::Value(acc.offset_a),
                ty: pattern.element_type,
            },
            Instruction::Load {
                volatile: false,
                dest: acc.load_rem_a,
                ptr: acc.gep_rem_a,
                ty: pattern.element_type,
                seg_override: AddressSpace::Default,
            },
        ]);
        match pattern.kind {
            ReductionKind::Max => {
                unreachable!("max reductions never carry a secondary accumulator")
            }
            ReductionKind::Sum => {
                remainder_body_instructions.push(Instruction::BinOp {
                    dest: acc.sum_rem_next,
                    op: IrBinOp::Add,
                    lhs: Operand::Value(acc.sum_rem_phi),
                    rhs: Operand::Value(acc.load_rem_a),
                    ty: pattern.accumulator_type,
                });
            }
            ReductionKind::DotProduct => {
                let base_b = reduction_gep_base(
                    func,
                    pattern.body_idx,
                    sec.array_b_gep.unwrap_or(sec.array_a_gep),
                );
                remainder_body_instructions.extend_from_slice(&[
                    Instruction::BinOp {
                        dest: acc.offset_b,
                        op: IrBinOp::Mul,
                        lhs: Operand::Value(i_rem_cast),
                        rhs: Operand::Const(IrConst::I64(element_size)),
                        ty: IrType::I64,
                    },
                    Instruction::GetElementPtr {
                        dest: acc.gep_rem_b,
                        base: base_b,
                        offset: Operand::Value(acc.offset_b),
                        ty: pattern.element_type,
                    },
                    Instruction::Load {
                        volatile: false,
                        dest: acc.load_rem_b,
                        ptr: acc.gep_rem_b,
                        ty: pattern.element_type,
                        seg_override: AddressSpace::Default,
                    },
                    Instruction::BinOp {
                        dest: acc.mul_rem,
                        op: IrBinOp::Mul,
                        lhs: Operand::Value(acc.load_rem_a),
                        rhs: Operand::Value(acc.load_rem_b),
                        ty: pattern.accumulator_type,
                    },
                    Instruction::BinOp {
                        dest: acc.sum_rem_next,
                        op: IrBinOp::Add,
                        lhs: Operand::Value(acc.sum_rem_phi),
                        rhs: Operand::Value(acc.mul_rem),
                        ty: pattern.accumulator_type,
                    },
                ]);
            }
        }
    }

    let remainder_body_block = BasicBlock {
        label: remainder_body_label,
        instructions: remainder_body_instructions,
        terminator: Terminator::Branch(remainder_latch_label),
        source_spans: vec![],
    };

    // Step 5: Create remainder_latch block
    let remainder_latch_block = BasicBlock {
        label: remainder_latch_label,
        instructions: vec![Instruction::BinOp {
            dest: i_rem_iv_next,
            op: IrBinOp::Add,
            lhs: Operand::Value(i_rem_iv),
            rhs: Operand::Const(IrConst::I32(1)),
            ty: IrType::I32,
        }],
        terminator: Terminator::Branch(remainder_header_label),
        source_spans: vec![],
    };

    // Step 6: Add all new blocks to the function
    let original_block_count = func.blocks.len();
    func.blocks.push(vec_exit_block);
    func.blocks.push(remainder_header_block);
    func.blocks.push(remainder_body_block);
    func.blocks.push(remainder_latch_block);

    // Step 7: Replace uses of the original scalar accumulator(s) with the
    // scalar remainder results.  After vectorization each accumulator phi in
    // the header holds a VECTOR value; EVERY use outside this loop must read
    // the reduced scalar instead (otherwise the consumer prints the vector's
    // stack address — lea_sib_fold / reduction_two_sums regressions).
    {
        let primary_updates = rewrite_accumulator_uses_outside_loop(
            func,
            &pattern.loop_blocks,
            pattern.accumulator_phi.0,
            sum_rem_phi,
        );
        for (sec, acc) in seconds.iter().zip(extras.iter()) {
            rewrite_accumulator_uses_outside_loop(
                func,
                &pattern.loop_blocks,
                sec.accumulator_phi.0,
                acc.sum_rem_phi,
            );
        }
        if debug {
            eprintln!(
                "[VEC-RED]   Rewired {} uses of accumulator outside the loop",
                primary_updates
            );
        }
    }

    if debug {
        eprintln!("[VEC-RED] Remainder loop complete: 4 blocks added");
    }

    4 // 4 new blocks added
}

/// Transform reduction loop to use AVX2 256-bit vectorization (4×F64, 8×I32, etc.).
fn transform_reduction_avx2(
    func: &mut IrFunction,
    pattern: &ReductionPattern,
    fp_contract: crate::common::fp_contract::FpContract,
) -> usize {
    // CONTIGUITY PRECONDITION -- checked BEFORE any IR is touched.
    //
    // The element-index scheme scales each GEP's byte offset by the vector
    // width, which is correct only when successive elements are ADJACENT, i.e.
    // the offset is `index * elem_size`. An array-of-structs access steps
    // `sizeof(struct)`: `sum += arr[i].i` over `struct S { int i, j, k; }` has
    // a 12-byte stride, so eight lanes are not eight consecutive `.i` fields.
    // Scaling produced `i * 12 * 8` and silently returned 6 instead of 12.
    //
    // This must be a precondition rather than a bail-out inside the rewrite:
    // aborting midway leaves the loop half-transformed, which is worse than
    // either outcome.

    {
        let elem_size = reduction_element_size(pattern.element_type).unwrap_or(0) as u32;
        let all_geps = reduction_array_geps(pattern);
        for &block_idx in &pattern.loop_blocks {
            for inst in &func.blocks[block_idx].instructions {
                if let Instruction::GetElementPtr { dest, offset, .. } = inst {
                    let is_reduction_gep = all_geps
                        .iter()
                        .any(|&(a, b)| *dest == a || Some(*dest) == b);
                    if is_reduction_gep {
                        if let Operand::Value(offset_val) = offset {
                            if !offset_is_index_times(
                                func,
                                &pattern.loop_blocks,
                                *offset_val,
                                elem_size,
                            ) {
                                return 0;
                            }
                        }
                    }
                }
            }
        }
    }

    let debug = std::env::var("LCCC_DEBUG_VECTORIZE").is_ok();
    let mut changes = 0;

    // Keep track of the next available Value and BlockId
    let mut next_val_id = func.next_value_id;
    let mut next_label = func.next_label;
    // Defensive: never trust func.next_label alone — some IR producers can
    // leave it stale relative to the blocks actually present. Allocating
    // below an existing label duplicates it and corrupts the CFG
    // (lea_sib_fold -O2 self-loop regression).
    let max_present_label = func.blocks.iter().map(|b| b.label.0).max().unwrap_or(0);
    next_label = std::cmp::max(next_label, max_present_label + 1);

    // Determine vector width and intrinsics based on element type
    // NOTE: These are only used for pattern matching - the actual transform uses Vec* variants
    // The widening I32→I64 reduction consumes FOUR I32 lanes per iteration
    // (VecWidenAddI32x4ToI64x2) and reduces into an I64x2 accumulator; its
    // horizontal intrinsic is the I64x2 hadd.
    let widening_i64 =
        pattern.accumulator_type == IrType::I64 && pattern.element_type == IrType::I32;
    let (vec_width, _load_intrinsic, _add_intrinsic, _mul_intrinsic, horizontal_intrinsic) =
        match pattern.element_type {
            IrType::F64 => (
                4u64,
                IntrinsicOp::LoadF64x4, // Legacy - not used in register-based transform
                IntrinsicOp::AddF64x4,  // Legacy - not used in register-based transform
                Some(IntrinsicOp::MulF64x4), // Legacy - not used in register-based transform
                IntrinsicOp::HorizontalAddF64x4, // Used for remainder loop intrinsic selection
            ),
            IrType::I32 if widening_i64 => (
                4u64,
                IntrinsicOp::VecWidenAddI32x4ToI64x2,
                IntrinsicOp::VecWidenAddI32x4ToI64x2,
                None,
                IntrinsicOp::VecHorizontalAddI64x2,
            ),
            // v12 Fix F: Max reduction (find_max). 8-wide lane max with a
            // horizontal max-reduce. The transform body (below, replacing the
            // old `unreachable!`) broadcasts the scalar init, lane-wise maxes
            // each loaded vector against the accumulator, and scales the
            // marching-pointer step from 4 → 32 bytes.
            IrType::I32 if pattern.kind == ReductionKind::Max => (
                8u64,
                IntrinsicOp::VecLoadI32x8,
                IntrinsicOp::VecMaxI32x8,
                None,
                IntrinsicOp::VecHorizontalMaxI32x8,
            ),
            IrType::I32 => (
                8u64,
                IntrinsicOp::LoadI32x8,
                IntrinsicOp::AddI32x8,
                Some(IntrinsicOp::VecMulI32x8),
                IntrinsicOp::HorizontalAddI32x8,
            ),
            IrType::F32 => (
                8u64,
                IntrinsicOp::VecLoadF32x8,
                IntrinsicOp::VecAddF32x8,
                Some(IntrinsicOp::VecMulF32x8),
                IntrinsicOp::VecHorizontalAddF32x8, // pass-through (no legacy F32 form)
            ),
            _ => {
                if debug {
                    eprintln!(
                        "[VEC-RED] Unsupported type for AVX2: {:?}",
                        pattern.element_type
                    );
                }
                return 0;
            }
        };

    if debug {
        eprintln!("[VEC-RED] Transforming reduction to AVX2:");
        eprintln!("[VEC-RED]   Kind: {:?}", pattern.kind);
        eprintln!("[VEC-RED]   Type: {:?}", pattern.element_type);
        eprintln!("[VEC-RED]   Vec width: {}", vec_width);

        eprintln!("[VEC-RED] === LOOP STRUCTURE BEFORE TRANSFORM ===");
        eprintln!(
            "[VEC-RED]   Header: {}, Body: {}, Latch: {}, Exit: {}",
            pattern.header_idx, pattern.body_idx, pattern.latch_idx, pattern.exit_idx
        );

        // Print header terminator
        eprintln!(
            "[VEC-RED]   Header terminator: {:?}",
            func.blocks[pattern.header_idx].terminator
        );

        // Print body terminator
        eprintln!(
            "[VEC-RED]   Body terminator: {:?}",
            func.blocks[pattern.body_idx].terminator
        );

        // Print latch terminator
        eprintln!(
            "[VEC-RED]   Latch terminator: {:?}",
            func.blocks[pattern.latch_idx].terminator
        );

        // Print IV and limit
        eprintln!(
            "[VEC-RED]   IV: {}, Limit: {:?}",
            pattern.iv.0, pattern.limit
        );
    }

    // Build IV-derived values using fixed-point iteration
    let mut iv_derived = FxHashSet::default();
    iv_derived.insert(pattern.iv);

    let mut changed = true;
    while changed {
        changed = false;
        for &block_idx in &pattern.loop_blocks {
            let block = &func.blocks[block_idx];
            for inst in &block.instructions {
                match inst {
                    Instruction::Cast { dest, src, .. } | Instruction::Copy { dest, src } => {
                        if let Operand::Value(src_val) = src {
                            if iv_derived.contains(src_val) && iv_derived.insert(*dest) {
                                changed = true;
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    // byte-offset IV strength reduction (mirrors the matmul path). The IV
    // steps `elem_sz * vec_width` bytes per iteration instead of one element,
    // so the GEP offset IS the byte IV (no per-iteration shl/leaq/scale) and
    // VecLoad can use `(base, byte_iv)` addressing directly. Falls back to the
    // element-index scheme when the offset chain is not the canonical
    // `shl/mul(iv_cast, elem_sz)` shape.
    let elem_sz = reduction_element_size(pattern.element_type).unwrap_or(0) as u64;
    let byte_stride = elem_sz * vec_width;
    let byte_iv_a = find_reduction_byte_iv(
        func,
        &pattern.loop_blocks,
        pattern.array_a_gep,
        elem_sz as u32,
    );
    let byte_iv_b = pattern
        .array_b_gep
        .and_then(|g| find_reduction_byte_iv(func, &pattern.loop_blocks, g, elem_sz as u32));
    let mut use_byte_iv =
        byte_iv_a.is_some() && (pattern.array_b_gep.is_none() || byte_iv_b.is_some());

    // Every additional accumulator must have the byte-IV shape on all of its
    // arrays too; otherwise the whole loop uses the element-index scheme
    // (scaled GEP offsets), which is correct but one LEA per access heavier.
    for sec in &pattern.seconds {
        let sec_byte_iv_a =
            find_reduction_byte_iv(func, &pattern.loop_blocks, sec.array_a_gep, elem_sz as u32);
        let sec_byte_iv_b = sec
            .array_b_gep
            .and_then(|g| find_reduction_byte_iv(func, &pattern.loop_blocks, g, elem_sz as u32));
        use_byte_iv &=
            sec_byte_iv_a.is_some() && (sec.array_b_gep.is_none() || sec_byte_iv_b.is_some());
    }

    // Step 1: Divide loop bound by vector width (byte limit under byte-offset IV)
    let divided_limit = match &pattern.limit {
        Operand::Const(IrConst::I32(n)) => {
            if use_byte_iv {
                Operand::Const(IrConst::I32((*n / vec_width as i32) * byte_stride as i32))
            } else {
                Operand::Const(IrConst::I32(*n / vec_width as i32))
            }
        }
        Operand::Const(IrConst::I64(n)) => {
            if use_byte_iv {
                Operand::Const(IrConst::I64((*n / vec_width as i64) * byte_stride as i64))
            } else {
                Operand::Const(IrConst::I64(*n / vec_width as i64))
            }
        }
        Operand::Value(limit_val) => {
            // Dynamic limit: insert division
            let div_dest = Value(next_val_id);
            next_val_id += 1;

            let limit_ty =
                match &func.blocks[pattern.header_idx].instructions[pattern.exit_cmp_inst_idx] {
                    Instruction::Cmp { ty, .. } => *ty,
                    _ => IrType::I64,
                };

            let div_inst = Instruction::BinOp {
                dest: div_dest,
                op: IrBinOp::UDiv,
                lhs: Operand::Value(*limit_val),
                rhs: Operand::Const(match limit_ty {
                    IrType::I32 => IrConst::I32(vec_width as i32),
                    IrType::I64 => IrConst::I64(vec_width as i64),
                    _ => IrConst::I64(vec_width as i64),
                }),
                ty: limit_ty,
            };

            // Insert before comparison
            func.blocks[pattern.header_idx]
                .instructions
                .insert(pattern.exit_cmp_inst_idx, div_inst);
            changes += 1;

            if debug {
                eprintln!(
                    "[VEC-RED]   Inserted division for dynamic limit: Value({})",
                    div_dest.0
                );
            }

            if use_byte_iv {
                let mul_dest = Value(next_val_id);
                next_val_id += 1;
                let mul_inst = Instruction::BinOp {
                    dest: mul_dest,
                    op: IrBinOp::Mul,
                    lhs: Operand::Value(div_dest),
                    rhs: Operand::Const(match limit_ty {
                        IrType::I32 => IrConst::I32(byte_stride as i32),
                        IrType::I64 => IrConst::I64(byte_stride as i64),
                        _ => IrConst::I64(byte_stride as i64),
                    }),
                    ty: limit_ty,
                };
                func.blocks[pattern.header_idx]
                    .instructions
                    .insert(pattern.exit_cmp_inst_idx + 1, mul_inst);
                changes += 1;
                Operand::Value(mul_dest)
            } else {
                Operand::Value(div_dest)
            }
        }
        _ => {
            if debug {
                eprintln!("[VEC-RED]   Unsupported limit type");
            }
            return 0;
        }
    };

    // Modify all comparisons that use IV-derived values
    for &block_idx in &pattern.loop_blocks {
        let block = &mut func.blocks[block_idx];
        for inst in &mut block.instructions {
            if let Instruction::Cmp { lhs, rhs, op, .. } = inst {
                if debug {
                    eprintln!("[VEC-RED]   CMP before: {:?} {:?} {:?}", lhs, op, rhs);
                }

                let modifies_lhs = if let Operand::Value(lhs_val) = lhs {
                    iv_derived.contains(lhs_val)
                } else {
                    false
                };

                let modifies_rhs = if let Operand::Value(rhs_val) = rhs {
                    iv_derived.contains(rhs_val)
                } else {
                    false
                };

                if modifies_lhs {
                    *rhs = divided_limit.clone();
                    changes += 1;
                    if debug {
                        eprintln!(
                            "[VEC-RED]   CMP after:  {:?} {:?} {:?} (modified RHS)",
                            lhs, op, rhs
                        );
                    }
                } else if modifies_rhs {
                    *lhs = divided_limit.clone();
                    changes += 1;
                    if debug {
                        eprintln!(
                            "[VEC-RED]   CMP after:  {:?} {:?} {:?} (modified LHS)",
                            lhs, op, rhs
                        );
                    }
                }
            }
        }
    }

    // byte-offset IV — the latch increment steps by `byte_stride` bytes.
    if use_byte_iv {
        let latch = &mut func.blocks[pattern.latch_idx];
        if pattern.iv_inc_idx < latch.instructions.len() {
            if let Instruction::BinOp {
                op: IrBinOp::Add,
                rhs,
                ..
            } = &mut latch.instructions[pattern.iv_inc_idx]
            {
                *rhs = match rhs {
                    Operand::Const(IrConst::I32(_)) => {
                        Operand::Const(IrConst::I32(byte_stride as i32))
                    }
                    _ => Operand::Const(IrConst::I64(byte_stride as i64)),
                };
                changes += 1;
                if debug {
                    eprintln!(
                        "[VEC-RED]   Changed IV increment to byte stride {}",
                        byte_stride
                    );
                }
            }
        }
    }

    // Step 2: Scale array indexing by vector width (element-index scheme only;
    // the byte-offset IV already covers vec_width elements per iteration).
    if !use_byte_iv {
        // Each GEP's byte offset (element_index * elem_size) must be multiplied by
        // vec_width so one vector iteration covers vec_width consecutive elements.
        // Collect ALL matching GEPs first — a dot product has two (array A and
        // array B) in the same block, and the old `break` after the first match
        // left array B at scalar stride, loading the wrong elements (miscompile:
        // dot(x,y,n) returned garbage for both n=255 and n=256).
        let mut geps_to_scale: Vec<(usize, usize, Value)> = Vec::new();
        let all_geps = reduction_array_geps(pattern);
        for &block_idx in &pattern.loop_blocks {
            let block = &func.blocks[block_idx];
            for (inst_idx, inst) in block.instructions.iter().enumerate() {
                if let Instruction::GetElementPtr { dest, offset, .. } = inst {
                    let is_reduction_gep = all_geps
                        .iter()
                        .any(|&(a, b)| *dest == a || Some(*dest) == b);
                    if is_reduction_gep {
                        if let Operand::Value(offset_val) = offset {
                            geps_to_scale.push((block_idx, inst_idx, *offset_val));
                        }
                    }
                }
            }
        }
        // Apply in reverse order so earlier insertions don't shift later indices.
        for (block_idx, inst_idx, offset_val) in geps_to_scale.into_iter().rev() {
            let mul_dest = Value(next_val_id);
            next_val_id += 1;

            let mul_inst = Instruction::BinOp {
                dest: mul_dest,
                op: IrBinOp::Mul,
                lhs: Operand::Value(offset_val),
                rhs: Operand::Const(IrConst::I64(vec_width as i64)),
                ty: IrType::I64,
            };

            func.blocks[block_idx]
                .instructions
                .insert(inst_idx, mul_inst);
            changes += 1;

            if let Instruction::GetElementPtr { offset, .. } =
                &mut func.blocks[block_idx].instructions[inst_idx + 1]
            {
                *offset = Operand::Value(mul_dest);
            }

            if debug {
                eprintln!("[VEC-RED]   Scaled GEP offset by {}", vec_width);
            }
        }
    }

    // Step 3: Transform loop body - register-based vector operations
    // Vector values are SSA values that live in stack slots (backend keeps in registers when possible)
    let vec_sum_value: Value; // The accumulated vector value

    if pattern.seconds.is_empty() {
        match pattern.kind {
            // v12 Fix F: AVX2 Max reduction (find_max). The detector (now
            // un-gated) fires for the Select-shaped max pattern; the
            // dispatcher above selects VecMaxI32x8 + VecHorizontalMaxI32x8.
            // The body mirrors the proven SSE2/NEON Max transform
            // (vectorize.rs ~9724-9890): broadcast the scalar init into all
            // 8 lanes, lane-wise vpmaxsd each loaded vector against the
            // accumulator, and scale the marching-pointer step from 4 → 32
            // bytes (vec_width × elem_size). The dedicated base-matching
            // scaler (not the shared dest-matching one) is what makes the
            // marching-pointer PHI form vectorize correctly.
            ReductionKind::Max => {
                let init_bcast = Value(next_val_id);
                next_val_id += 1;
                let vec_load = Value(next_val_id);
                next_val_id += 1;
                vec_sum_value = Value(next_val_id);
                next_val_id += 1;

                let latch_label = func.blocks[pattern.latch_idx].label;

                // Read the phi edges FIRST (fail-closed), then rewire:
                // backedge → max result, preheader → broadcast of scalar init.
                let mut preheader_label = None;
                let mut init_operand = None;
                {
                    let header_block = &func.blocks[pattern.header_idx];
                    for inst in &header_block.instructions {
                        if let Instruction::Phi { dest, incoming, .. } = inst {
                            if *dest == pattern.accumulator_phi {
                                for (val, label) in incoming {
                                    if *label != latch_label {
                                        preheader_label = Some(*label);
                                        init_operand = Some(val.clone());
                                    }
                                }
                            }
                        }
                    }
                }
                let (Some(preheader_label), Some(init_operand)) = (preheader_label, init_operand)
                else {
                    if debug {
                        eprintln!("[VEC-RED]   Max: no preheader edge on accumulator phi");
                    }
                    return changes;
                };
                {
                    let header_block = &mut func.blocks[pattern.header_idx];
                    for inst in header_block.instructions.iter_mut() {
                        if let Instruction::Phi { dest, incoming, .. } = inst {
                            if *dest == pattern.accumulator_phi {
                                for (val, label) in incoming.iter_mut() {
                                    if *label == latch_label {
                                        *val = Operand::Value(vec_sum_value);
                                    } else {
                                        *val = Operand::Value(init_bcast);
                                    }
                                }
                            }
                        }
                    }
                }

                // Broadcast the scalar init into all 8 lanes, in the preheader.
                if let Some(pre_idx) = func.blocks.iter().position(|b| b.label == preheader_label) {
                    func.blocks[pre_idx]
                        .instructions
                        .push(Instruction::Intrinsic {
                            dest: Some(init_bcast),
                            op: IntrinsicOp::VecBroadcastI32x8,
                            dest_ptr: None,
                            args: vec![init_operand],
                        });
                    changes += 1;
                }

                // Body: 8-wide load + lane-wise vpmaxsd, replacing the
                // Select and its feeding Cmp.
                {
                    let body_block = &mut func.blocks[pattern.body_idx];
                    let mut sel_cmp_val = None;
                    if let Instruction::Select {
                        cond: Operand::Value(cv),
                        ..
                    } = &body_block.instructions[pattern.accumulator_add_idx]
                    {
                        sel_cmp_val = Some(*cv);
                    }
                    let (base, off) = match (use_byte_iv, &byte_iv_a) {
                        (true, Some((b, o))) => (Operand::Value(*b), Operand::Value(*o)),
                        _ => (
                            Operand::Value(pattern.array_a_gep),
                            Operand::Const(IrConst::I64(0)),
                        ),
                    };
                    body_block.instructions.insert(
                        pattern.accumulator_add_idx,
                        Instruction::Intrinsic {
                            dest: Some(vec_load),
                            op: IntrinsicOp::VecLoadI32x8,
                            dest_ptr: None,
                            args: vec![base, off],
                        },
                    );
                    body_block.instructions.insert(
                        pattern.accumulator_add_idx + 1,
                        Instruction::Intrinsic {
                            dest: Some(vec_sum_value),
                            op: IntrinsicOp::VecMaxI32x8,
                            dest_ptr: None,
                            args: vec![
                                Operand::Value(pattern.accumulator_phi),
                                Operand::Value(vec_load),
                            ],
                        },
                    );
                    // Remove the old Select (now shifted to +2).
                    body_block
                        .instructions
                        .remove(pattern.accumulator_add_idx + 2);
                    changes += 2;
                    if let Some(cv) = sel_cmp_val {
                        if let Some(cp) = body_block
                            .instructions
                            .iter()
                            .position(|i| matches!(i.dest(), Some(d) if d.0 == cv.0))
                        {
                            body_block.instructions.remove(cp);
                        }
                    }
                }

                // Scale the marching pointer's latch step from one element
                // (4 bytes) to vec_width elements (32 = 8 × 4): the detector
                // proved the access is a one-element marching-pointer phi,
                // and the vector body now consumes 8 lanes per iteration.
                // The shared dest-matching stride scaler (above) does NOT
                // fire for the marching-pointer PHI form (array_a_gep IS the
                // phi, not a GEP); this dedicated base-matching scaler —
                // ported from the SSE2 Max transform — is the fix.
                {
                    let mut scaled = false;
                    for &bi in &pattern.loop_blocks {
                        let block = &mut func.blocks[bi];
                        for inst in block.instructions.iter_mut() {
                            if let Instruction::GetElementPtr {
                                base,
                                offset: offset @ Operand::Const(_),
                                ..
                            } = inst
                            {
                                if base.0 == pattern.array_a_gep.0 {
                                    if let Operand::Const(c) = offset {
                                        if c.to_i64() == Some(4) {
                                            *offset = Operand::Const(IrConst::I64(32));
                                            scaled = true;
                                        }
                                    }
                                }
                            }
                        }
                    }
                    debug_assert!(
                        scaled,
                        "max transform requires the marching-pointer step GEP"
                    );
                    if scaled {
                        changes += 1;
                    }
                }

                if debug {
                    eprintln!("[VEC-RED]   Transformed max body: load + vpmaxsd (AVX2 8-wide)");
                }
            }
            ReductionKind::Sum => {
                // Simple sum: sum += arr[i]
                // Register-based flow: vec_zero → vec_load → vec_add → horizontal_add

                // Map to register-based intrinsics based on element type.
                // The WIDENING case (I32 loads accumulated into an I64
                // accumulator) uses the composite VecWidenAddI32x4ToI64x2
                // intrinsic: one instruction = load 4×I32 + sign-extend +
                // paddq into the I64x2 accumulator. The emitted body then
                // carries a single intrinsic instead of a load/add pair.
                let widening_i64 =
                    pattern.accumulator_type == IrType::I64 && pattern.element_type == IrType::I32;
                let (vec_load_op, vec_add_op, vec_zero_op) = match pattern.element_type {
                    IrType::F64 => (
                        IntrinsicOp::VecLoadF64x4,
                        IntrinsicOp::VecAddF64x4,
                        IntrinsicOp::VecZeroF64x4,
                    ),
                    IrType::I32 if widening_i64 => (
                        IntrinsicOp::VecWidenAddI32x4ToI64x2,
                        IntrinsicOp::VecWidenAddI32x4ToI64x2,
                        IntrinsicOp::VecZeroI64x2,
                    ),
                    IrType::I32 => (
                        IntrinsicOp::VecLoadI32x8,
                        IntrinsicOp::VecAddI32x8,
                        IntrinsicOp::VecZeroI32x8,
                    ),
                    IrType::F32 => (
                        IntrinsicOp::VecLoadF32x8,
                        IntrinsicOp::VecAddF32x8,
                        IntrinsicOp::VecZeroF32x8,
                    ),
                    _ => panic!("Unsupported AVX2 element type: {:?}", pattern.element_type),
                };

                if debug {
                    eprintln!(
                        "[VEC-RED]   Using register-based intrinsics: {:?}, {:?}",
                        vec_load_op, vec_add_op
                    );
                }

                // Value IDs for vector operations (SSA values)
                // IMPORTANT: Create init_zero_value FIRST because it's inserted in entry block (block 0)
                // which comes before the loop body. This ensures SSA IDs are in program order.
                let init_zero_value = Value(next_val_id);
                next_val_id += 1;
                let vec_load = Value(next_val_id);
                next_val_id += 1;
                vec_sum_value = Value(next_val_id);
                next_val_id += 1;

                // Initialize vector accumulator to zero in entry block
                let entry_block = &mut func.blocks[0];
                let zero_inst = Instruction::Intrinsic {
                    dest: Some(init_zero_value),
                    op: vec_zero_op,
                    dest_ptr: None,
                    args: vec![],
                };
                entry_block.instructions.push(zero_inst);
                changes += 1;

                // Update the accumulator PHI to use init_zero_value as the entry predecessor
                let header_block = &mut func.blocks[pattern.header_idx];
                for inst in header_block.instructions.iter_mut() {
                    if let Instruction::Phi { dest, incoming, .. } = inst {
                        if *dest == pattern.accumulator_phi {
                            for (val, _) in incoming.iter_mut() {
                                if matches!(
                                    val,
                                    Operand::Const(IrConst::F32(_))
                                        | Operand::Const(IrConst::F64(_))
                                        | Operand::Const(IrConst::I32(0))
                                        | Operand::Const(IrConst::I64(0))
                                        | Operand::Const(IrConst::Zero)
                                ) {
                                    *val = Operand::Value(init_zero_value);
                                }
                            }
                        }
                    }
                }

                // Get latch label before taking mutable references
                let latch_label = func.blocks[pattern.latch_idx].label;

                // Transform loop body with register-based operations
                {
                    let body_block = &mut func.blocks[pattern.body_idx];

                    let (base, off) = match (use_byte_iv, &byte_iv_a) {
                        (true, Some((b, o))) => (Operand::Value(*b), Operand::Value(*o)),
                        _ => (
                            Operand::Value(pattern.array_a_gep),
                            Operand::Const(IrConst::I64(0)),
                        ),
                    };

                    if widening_i64 && pattern.guard_cond.is_some() {
                        // CONDITIONAL (guarded) widening: the composite
                        // masked intrinsic carries the guard's rhs operand;
                        // the lowering builds the per-lane compare mask
                        // internally. Splice: remove the Select first (it
                        // sits after the Add), then replace the Add.
                        let guard_rhs = pattern
                            .guard_rhs
                            .clone()
                            .unwrap_or(Operand::Const(IrConst::I32(0)));
                        let masked_inst = Instruction::Intrinsic {
                            dest: Some(vec_sum_value),
                            op: IntrinsicOp::VecWidenMaskedAddI32x4ToI64x2,
                            dest_ptr: None,
                            args: vec![
                                Operand::Value(pattern.accumulator_phi),
                                base,
                                off,
                                guard_rhs,
                            ],
                        };
                        // The Add's result id (the Select's true_val),
                        // read BEFORE any splice shifts indices.
                        let add_result_id = body_block
                            .instructions
                            .get(pattern.accumulator_add_idx)
                            .and_then(|i| i.dest())
                            .map(|d| d.0);
                        // Locate the Select (true_val == the Add's result)
                        // to remove it together with the old Add.
                        let mut select_idx = None;
                        for (sidx, sin) in body_block.instructions.iter().enumerate() {
                            if let Instruction::Select { true_val, .. } = sin {
                                if matches!(
                                    true_val,
                                    Operand::Value(tv) if Some(tv.0) == add_result_id
                                ) {
                                    select_idx = Some(sidx);
                                    break;
                                }
                            }
                        }
                        if let Some(si) = select_idx {
                            body_block.instructions.remove(si);
                        }
                        body_block
                            .instructions
                            .insert(pattern.accumulator_add_idx, masked_inst);
                        body_block
                            .instructions
                            .remove(pattern.accumulator_add_idx + 1);
                        changes += 1;
                    } else if widening_i64 {
                        // One composite intrinsic: acc += sext(load4×I32).
                        let widen_inst = Instruction::Intrinsic {
                            dest: Some(vec_sum_value),
                            op: vec_load_op,
                            dest_ptr: None,
                            args: vec![Operand::Value(pattern.accumulator_phi), base, off],
                        };
                        body_block
                            .instructions
                            .insert(pattern.accumulator_add_idx, widen_inst);
                        body_block
                            .instructions
                            .remove(pattern.accumulator_add_idx + 1);
                        changes += 1;
                    } else {
                        // Vector load from array → SSA value
                        let load_inst = Instruction::Intrinsic {
                            dest: Some(vec_load),
                            op: vec_load_op,
                            dest_ptr: None,
                            args: vec![base, off],
                        };

                        // Vector add: accumulator + loaded vector → new accumulator
                        let add_inst = Instruction::Intrinsic {
                            dest: Some(vec_sum_value),
                            op: vec_add_op,
                            dest_ptr: None,
                            args: vec![
                                Operand::Value(pattern.accumulator_phi), // Current accumulator (PHI)
                                Operand::Value(vec_load),                // Loaded vector
                            ],
                        };

                        // Insert vector instructions and remove old scalar add
                        body_block
                            .instructions
                            .insert(pattern.accumulator_add_idx, load_inst);
                        body_block
                            .instructions
                            .insert(pattern.accumulator_add_idx + 1, add_inst);
                        body_block
                            .instructions
                            .remove(pattern.accumulator_add_idx + 2);
                        changes += 2;
                    }

                    // Debug: Log vector accumulator flow
                    if debug {
                        eprintln!("[VEC-RED-DEBUG] Vec load SSA value: {}", vec_load.0);
                        eprintln!(
                            "[VEC-RED-DEBUG] Vector accumulator SSA value: {}",
                            vec_sum_value.0
                        );
                        eprintln!(
                            "[VEC-RED-DEBUG] Accumulator PHI: {}",
                            pattern.accumulator_phi.0
                        );
                        eprintln!("[VEC-RED-DEBUG] Entry init value: {}", init_zero_value.0);
                        eprintln!("[VEC-RED-DEBUG] Vector add op: {:?}", vec_add_op);
                    }
                }

                // Update the PHI's backedge to use vec_sum_value
                {
                    let header_block = &mut func.blocks[pattern.header_idx];
                    for inst in header_block.instructions.iter_mut() {
                        if let Instruction::Phi { dest, incoming, .. } = inst {
                            if *dest == pattern.accumulator_phi {
                                for (val, label) in incoming.iter_mut() {
                                    if *label == latch_label {
                                        *val = Operand::Value(vec_sum_value);
                                    }
                                }
                            }
                        }
                    }
                }

                if debug {
                    eprintln!(
                        "[VEC-RED]   Transformed sum body: vec_load + vec_add (register-based)"
                    );
                }
            }
            ReductionKind::DotProduct => {
                // Dot product: sum += a[i] * b[i]
                // Register-based: vec_load_a, vec_load_b, vec_mul, vec_add (all SSA)

                // Value IDs for vector operations
                let vec_load_a = Value(next_val_id);
                next_val_id += 1;
                let vec_load_b = Value(next_val_id);
                next_val_id += 1;
                let vec_mul = Value(next_val_id);
                next_val_id += 1;
                vec_sum_value = Value(next_val_id);
                next_val_id += 1;

                // Map to register-based intrinsics based on element type
                let (vec_load_op, vec_mul_op, vec_add_op, vec_zero_op, vec_fma_op) =
                    match pattern.element_type {
                        IrType::F64 => (
                            IntrinsicOp::VecLoadF64x4,
                            IntrinsicOp::VecMulF64x4,
                            IntrinsicOp::VecAddF64x4,
                            IntrinsicOp::VecZeroF64x4,
                            IntrinsicOp::VecFmaF64x4,
                        ),
                        IrType::F32 => (
                            IntrinsicOp::VecLoadF32x8,
                            IntrinsicOp::VecMulF32x8,
                            IntrinsicOp::VecAddF32x8,
                            IntrinsicOp::VecZeroF32x8,
                            IntrinsicOp::VecFmaF32x8,
                        ),
                        IrType::I32 => (
                            IntrinsicOp::VecLoadI32x8,
                            IntrinsicOp::VecMulI32x8,
                            IntrinsicOp::VecAddI32x8,
                            IntrinsicOp::VecZeroI32x8,
                            IntrinsicOp::VecMulI32x8,
                        ),
                        _ => {
                            if debug {
                                eprintln!(
                                    "[VEC-RED] Unsupported AVX2 dot product type: {:?}",
                                    pattern.element_type
                                );
                            }
                            return changes;
                        }
                    };

                if debug {
                    eprintln!("[VEC-RED]   Using register-based dot product intrinsics");
                }

                // Initialize vector accumulator to zero in entry block
                let entry_block = &mut func.blocks[0];
                let init_zero_value = Value(next_val_id);
                next_val_id += 1;
                let zero_inst = Instruction::Intrinsic {
                    dest: Some(init_zero_value),
                    op: vec_zero_op,
                    dest_ptr: None,
                    args: vec![],
                };
                entry_block.instructions.push(zero_inst);
                changes += 1;

                // Update the accumulator PHI to use init_zero_value as the entry predecessor
                let header_block = &mut func.blocks[pattern.header_idx];
                for inst in header_block.instructions.iter_mut() {
                    if let Instruction::Phi { dest, incoming, .. } = inst {
                        if *dest == pattern.accumulator_phi {
                            for (val, _) in incoming.iter_mut() {
                                if matches!(
                                    val,
                                    Operand::Const(IrConst::F32(_))
                                        | Operand::Const(IrConst::F64(_))
                                        | Operand::Const(IrConst::I32(0))
                                        | Operand::Const(IrConst::I64(0))
                                        | Operand::Const(IrConst::Zero)
                                ) {
                                    *val = Operand::Value(init_zero_value);
                                }
                            }
                        }
                    }
                }

                let (a_base, a_off) = match (use_byte_iv, &byte_iv_a) {
                    (true, Some((base, off))) => (Operand::Value(*base), Operand::Value(*off)),
                    _ => (
                        Operand::Value(pattern.array_a_gep),
                        Operand::Const(IrConst::I64(0)),
                    ),
                };
                let (b_base, b_off) = match (use_byte_iv, &byte_iv_b) {
                    (true, Some((base, off))) => (Operand::Value(*base), Operand::Value(*off)),
                    _ => (
                        Operand::Value(pattern.array_b_gep.unwrap()),
                        Operand::Const(IrConst::I64(0)),
                    ),
                };
                let body_block = &mut func.blocks[pattern.body_idx];

                if fp_contract == FpContract::Fast
                    && x86_fma_enabled()
                    && matches!(pattern.element_type, IrType::F32 | IrType::F64)
                {
                    let fma_inst = Instruction::Intrinsic {
                        dest: Some(vec_sum_value),
                        op: vec_fma_op,
                        dest_ptr: None,
                        args: vec![
                            Operand::Value(pattern.accumulator_phi),
                            a_base,
                            a_off,
                            b_base,
                            b_off,
                        ],
                    };
                    body_block
                        .instructions
                        .insert(pattern.accumulator_add_idx, fma_inst);
                    body_block
                        .instructions
                        .remove(pattern.accumulator_add_idx + 1);
                    changes += 1;
                } else {
                    let load_a_inst = Instruction::Intrinsic {
                        dest: Some(vec_load_a),
                        op: vec_load_op,
                        dest_ptr: None,
                        args: vec![a_base, a_off],
                    };
                    let load_b_inst = Instruction::Intrinsic {
                        dest: Some(vec_load_b),
                        op: vec_load_op,
                        dest_ptr: None,
                        args: vec![b_base, b_off],
                    };
                    let mul_inst = Instruction::Intrinsic {
                        dest: Some(vec_mul),
                        op: vec_mul_op,
                        dest_ptr: None,
                        args: vec![Operand::Value(vec_load_a), Operand::Value(vec_load_b)],
                    };
                    let add_inst = Instruction::Intrinsic {
                        dest: Some(vec_sum_value),
                        op: vec_add_op,
                        dest_ptr: None,
                        args: vec![
                            Operand::Value(pattern.accumulator_phi),
                            Operand::Value(vec_mul),
                        ],
                    };
                    body_block
                        .instructions
                        .insert(pattern.accumulator_add_idx, load_a_inst);
                    body_block
                        .instructions
                        .insert(pattern.accumulator_add_idx + 1, load_b_inst);
                    body_block
                        .instructions
                        .insert(pattern.accumulator_add_idx + 2, mul_inst);
                    body_block
                        .instructions
                        .insert(pattern.accumulator_add_idx + 3, add_inst);
                    body_block
                        .instructions
                        .remove(pattern.accumulator_add_idx + 4);
                    changes += 4;
                }

                // Get latch label before taking a mutable reference to header
                let latch_label = func.blocks[pattern.latch_idx].label;

                // Update the PHI's backedge to use vec_sum_value
                let header_block = &mut func.blocks[pattern.header_idx];
                for inst in header_block.instructions.iter_mut() {
                    if let Instruction::Phi { dest, incoming, .. } = inst {
                        if *dest == pattern.accumulator_phi {
                            for (val, label) in incoming.iter_mut() {
                                if *label == latch_label {
                                    *val = Operand::Value(vec_sum_value);
                                }
                            }
                        }
                    }
                }

                if debug {
                    eprintln!(
                        "[VEC-RED]   Transformed dot product body: load_a + load_b + mul + add"
                    );
                }
            }
        }
    } else {
        // Multi-reduction: two independent accumulators.  Rewrite each body in
        // DESCENDING add-index order so the higher-index insertions/removals
        // never shift a lower index still to be patched.
        let (vec_load_op, vec_mul_op, vec_add_op, vec_zero_op, vec_fma_op, use_fma) =
            match (pattern.kind, pattern.element_type) {
                (ReductionKind::Sum, IrType::F64) => (
                    IntrinsicOp::VecLoadF64x4,
                    None,
                    IntrinsicOp::VecAddF64x4,
                    IntrinsicOp::VecZeroF64x4,
                    None,
                    false,
                ),
                (ReductionKind::Sum, IrType::I32) => (
                    IntrinsicOp::VecLoadI32x8,
                    None,
                    IntrinsicOp::VecAddI32x8,
                    IntrinsicOp::VecZeroI32x8,
                    None,
                    false,
                ),
                (ReductionKind::Sum, IrType::F32) => (
                    IntrinsicOp::VecLoadF32x8,
                    None,
                    IntrinsicOp::VecAddF32x8,
                    IntrinsicOp::VecZeroF32x8,
                    None,
                    false,
                ),
                (ReductionKind::DotProduct, IrType::F64) => (
                    IntrinsicOp::VecLoadF64x4,
                    Some(IntrinsicOp::VecMulF64x4),
                    IntrinsicOp::VecAddF64x4,
                    IntrinsicOp::VecZeroF64x4,
                    Some(IntrinsicOp::VecFmaF64x4),
                    fp_contract == FpContract::Fast,
                ),
                (ReductionKind::DotProduct, IrType::F32) => (
                    IntrinsicOp::VecLoadF32x8,
                    Some(IntrinsicOp::VecMulF32x8),
                    IntrinsicOp::VecAddF32x8,
                    IntrinsicOp::VecZeroF32x8,
                    Some(IntrinsicOp::VecFmaF32x8),
                    fp_contract == FpContract::Fast,
                ),
                (ReductionKind::DotProduct, IrType::I32) => (
                    IntrinsicOp::VecLoadI32x8,
                    Some(IntrinsicOp::VecMulI32x8),
                    IntrinsicOp::VecAddI32x8,
                    IntrinsicOp::VecZeroI32x8,
                    None,
                    false,
                ),
                _ => return 0,
            };

        let mut order: Vec<(
            Value,
            usize,
            Value,
            Option<Value>,
            Option<(Value, Value)>,
            Option<(Value, Value)>,
            bool,
        )> = Vec::with_capacity(1 + pattern.seconds.len());
        order.push((
            pattern.accumulator_phi,
            pattern.accumulator_add_idx,
            pattern.array_a_gep,
            pattern.array_b_gep,
            byte_iv_a,
            byte_iv_b,
            true,
        ));
        for sec in &pattern.seconds {
            let sec_byte_iv_a =
                find_reduction_byte_iv(func, &pattern.loop_blocks, sec.array_a_gep, elem_sz as u32);
            let sec_byte_iv_b = sec.array_b_gep.and_then(|g| {
                find_reduction_byte_iv(func, &pattern.loop_blocks, g, elem_sz as u32)
            });
            order.push((
                sec.accumulator_phi,
                sec.accumulator_add_idx,
                sec.array_a_gep,
                sec.array_b_gep,
                sec_byte_iv_a,
                sec_byte_iv_b,
                false,
            ));
        }
        order.sort_by_key(|&(_, add_idx, ..)| std::cmp::Reverse(add_idx));

        let mut primary_sum = Value(u32::MAX);
        for (phi, add_idx, a_gep, b_gep, ba, bb, is_primary) in order {
            let (_init, sum, n) = rewrite_reduction_body(
                func,
                pattern.header_idx,
                pattern.body_idx,
                pattern.latch_idx,
                pattern.kind,
                use_byte_iv,
                vec_load_op,
                vec_add_op,
                vec_zero_op,
                vec_mul_op,
                use_fma,
                vec_fma_op,
                phi,
                add_idx,
                a_gep,
                b_gep,
                ba,
                bb,
                &mut next_val_id,
            );
            changes += n;
            if is_primary {
                primary_sum = sum;
            }
        }
        debug_assert!(
            primary_sum.0 != u32::MAX,
            "primary accumulator not rewritten"
        );
        vec_sum_value = primary_sum;
    }

    // Merge duplicate vector loads (a shared array is loaded once per
    // iteration; `sum += x*x` and `b += v*w` after `a += u*v` both benefit).
    changes += deduplicate_vector_loads(func, &pattern.loop_blocks);

    // Step 4: Create remainder loop.
    //
    // Snapshot the labels of every block OUTSIDE the loop first. The remainder
    // blocks are created below and legitimately consume the vector IV (the
    // preheader converts the byte counter back to an element index), so they
    // must be excluded from the escaping-use rewrite that follows.
    let outside_labels: Vec<BlockId> = func
        .blocks
        .iter()
        .enumerate()
        .filter(|(bi, _)| !pattern.loop_blocks.contains(bi))
        .map(|(_, b)| b.label)
        .collect();

    let mut rem_iv: Option<Value> = None;
    let mut rem_iv_unused: Option<Value> = None;
    let remainder_changes = insert_reduction_remainder_loop(
        func,
        pattern,
        vec_width,
        horizontal_intrinsic,
        vec_sum_value, // Pass the vector accumulator SSA value
        use_byte_iv,
        None,             // AVX2 path uses a single NEON accumulator
        &pattern.seconds, // Extra independent accumulators, if any
        &mut next_val_id,
        &mut next_label,
        &mut rem_iv,
    );
    changes += remainder_changes;

    if debug {
        eprintln!(
            "[VEC-RED]   Added remainder loop: {} blocks created",
            remainder_changes / 4
        );
    }

    // Step 5: repair uses of the loop counter that ESCAPE the loop.
    changes += rewire_escaping_iv_uses(func, pattern, rem_iv, &outside_labels, debug);

    // Update the function's next_value_id and next_label
    func.next_value_id = next_val_id;
    func.next_label = next_label;

    changes
}

/// Transform reduction loop to use SSE2 128-bit vectorization (2×F64, 4×I32, etc.).
fn transform_reduction_sse2(
    func: &mut IrFunction,
    pattern: &ReductionPattern,
    neon: bool,
) -> usize {
    let debug = std::env::var("LCCC_DEBUG_VECTORIZE").is_ok();
    let mut changes = 0;

    // Keep track of the next available Value and BlockId
    let mut next_val_id = func.next_value_id;
    let mut next_label = func.next_label;
    // Defensive: never trust func.next_label alone — some IR producers can
    // leave it stale relative to the blocks actually present. Allocating
    // below an existing label duplicates it and corrupts the CFG
    // (lea_sib_fold -O2 self-loop regression).
    let max_present_label = func.blocks.iter().map(|b| b.label.0).max().unwrap_or(0);
    next_label = std::cmp::max(next_label, max_present_label + 1);

    // Determine vector width and intrinsics based on element type (SSE2 = half of AVX2)
    let (vec_width, load_intrinsic, add_intrinsic, mul_intrinsic, horizontal_intrinsic) =
        match pattern.element_type {
            IrType::F64 => (
                2u64,
                IntrinsicOp::LoadF64x2,
                IntrinsicOp::AddF64x2,
                Some(IntrinsicOp::MulF64x2),
                IntrinsicOp::HorizontalAddF64x2,
            ),
            // NEON 4-wide i32 max reduction: lane-wise smax + smaxv reduce.
            IrType::I32 if pattern.kind == ReductionKind::Max => {
                if !neon {
                    // No packed max path on the x86 side of this transform
                    // (pmaxsd exists but the epilogue lacks a horizontal-max;
                    // detector already gates on neon, this is defense).
                    if debug {
                        eprintln!("[VEC-RED] Max reduction requires NEON");
                    }
                    return 0;
                }
                (
                    4u64,
                    IntrinsicOp::VecLoadI32x4,
                    IntrinsicOp::VecSmaxI32x4,
                    None,
                    IntrinsicOp::VecHorizontalMaxI32x4,
                )
            }
            // NEON 4-wide i32→i64 forms: sadalp for sums, smlal/smlal2 for dots.
            IrType::I32 if pattern.accumulator_type == IrType::I64 && neon => (
                4u64,
                IntrinsicOp::VecLoadI32x4,
                match pattern.kind {
                    ReductionKind::Sum => IntrinsicOp::VecSadalpI32x4,
                    ReductionKind::DotProduct => IntrinsicOp::VecSmlalLoI32x4,
                    ReductionKind::Max => unreachable!("max has an I32 accumulator"),
                },
                None,
                IntrinsicOp::VecHorizontalAddI64x2,
            ),
            IrType::I32 if pattern.accumulator_type == IrType::I64 => (
                2u64,
                IntrinsicOp::VecLoadWidenI32ToI64x2,
                IntrinsicOp::VecAddI64x2,
                Some(IntrinsicOp::VecMulI64x2),
                IntrinsicOp::VecHorizontalAddI64x2,
            ),
            IrType::I32 => (
                4u64,
                IntrinsicOp::LoadI32x4,
                IntrinsicOp::AddI32x4,
                Some(IntrinsicOp::VecMulI32x4),
                IntrinsicOp::HorizontalAddI32x4,
            ),
            IrType::I64 => (
                2u64,
                IntrinsicOp::VecLoadI64x2,
                IntrinsicOp::VecAddI64x2,
                Some(IntrinsicOp::VecMulI64x2),
                IntrinsicOp::VecHorizontalAddI64x2,
            ),
            IrType::F32 => (
                4u64,
                IntrinsicOp::VecLoadF32x4,
                IntrinsicOp::VecAddF32x4,
                Some(IntrinsicOp::VecMulF32x4),
                IntrinsicOp::VecHorizontalAddF32x4, // pass-through (no legacy F32 form)
            ),
            _ => {
                if debug {
                    eprintln!(
                        "[VEC-RED] Unsupported type for SSE2: {:?}",
                        pattern.element_type
                    );
                }
                return 0;
            }
        };

    if debug {
        eprintln!("[VEC-RED] Transforming reduction to SSE2:");
        eprintln!("[VEC-RED]   Kind: {:?}", pattern.kind);
        eprintln!("[VEC-RED]   Type: {:?}", pattern.element_type);
        eprintln!("[VEC-RED]   Vec width: {}", vec_width);
    }

    // Build IV-derived values using fixed-point iteration
    let mut iv_derived = FxHashSet::default();
    iv_derived.insert(pattern.iv);

    let mut changed = true;
    while changed {
        changed = false;
        for &block_idx in &pattern.loop_blocks {
            let block = &func.blocks[block_idx];
            for inst in &block.instructions {
                match inst {
                    Instruction::Cast { dest, src, .. } | Instruction::Copy { dest, src } => {
                        if let Operand::Value(src_val) = src {
                            if iv_derived.contains(src_val) && iv_derived.insert(*dest) {
                                changed = true;
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    // byte-offset IV strength reduction (mirrors the matmul path). The IV
    // steps `elem_sz * vec_width` bytes per iteration instead of one element,
    // so the GEP offset IS the byte IV (no per-iteration shl/leaq/scale) and
    // VecLoad can use `(base, byte_iv)` addressing directly. Falls back to the
    // element-index scheme when the offset chain is not the canonical
    // `shl/mul(iv_cast, elem_sz)` shape.
    let elem_sz = reduction_element_size(pattern.element_type).unwrap_or(0) as u64;
    let byte_stride = elem_sz * vec_width;
    let byte_iv_a = find_reduction_byte_iv(
        func,
        &pattern.loop_blocks,
        pattern.array_a_gep,
        elem_sz as u32,
    );
    let byte_iv_b = pattern
        .array_b_gep
        .and_then(|g| find_reduction_byte_iv(func, &pattern.loop_blocks, g, elem_sz as u32));
    let mut use_byte_iv =
        byte_iv_a.is_some() && (pattern.array_b_gep.is_none() || byte_iv_b.is_some());
    // Every additional accumulator must have the byte-IV shape on all of its
    // arrays too; otherwise the whole loop uses the element-index scheme
    // (scaled GEP offsets), which is correct but one LEA per access heavier.
    for sec in &pattern.seconds {
        let sec_byte_iv_a =
            find_reduction_byte_iv(func, &pattern.loop_blocks, sec.array_a_gep, elem_sz as u32);
        let sec_byte_iv_b = sec
            .array_b_gep
            .and_then(|g| find_reduction_byte_iv(func, &pattern.loop_blocks, g, elem_sz as u32));
        use_byte_iv &=
            sec_byte_iv_a.is_some() && (sec.array_b_gep.is_none() || sec_byte_iv_b.is_some());
    }

    // Step 1: Divide loop bound by vector width (byte limit under byte-offset IV)
    let divided_limit = match &pattern.limit {
        Operand::Const(IrConst::I32(n)) => {
            if use_byte_iv {
                Operand::Const(IrConst::I32((*n / vec_width as i32) * byte_stride as i32))
            } else {
                Operand::Const(IrConst::I32(*n / vec_width as i32))
            }
        }
        Operand::Const(IrConst::I64(n)) => {
            if use_byte_iv {
                Operand::Const(IrConst::I64((*n / vec_width as i64) * byte_stride as i64))
            } else {
                Operand::Const(IrConst::I64(*n / vec_width as i64))
            }
        }
        Operand::Value(limit_val) => {
            // Dynamic limit: insert division
            let div_dest = Value(next_val_id);
            next_val_id += 1;

            let limit_ty =
                match &func.blocks[pattern.header_idx].instructions[pattern.exit_cmp_inst_idx] {
                    Instruction::Cmp { ty, .. } => *ty,
                    _ => IrType::I64,
                };

            let div_inst = Instruction::BinOp {
                dest: div_dest,
                op: IrBinOp::UDiv,
                lhs: Operand::Value(*limit_val),
                rhs: Operand::Const(match limit_ty {
                    IrType::I32 => IrConst::I32(vec_width as i32),
                    IrType::I64 => IrConst::I64(vec_width as i64),
                    _ => IrConst::I64(vec_width as i64),
                }),
                ty: limit_ty,
            };

            // Insert before comparison
            func.blocks[pattern.header_idx]
                .instructions
                .insert(pattern.exit_cmp_inst_idx, div_inst);
            changes += 1;

            if debug {
                eprintln!(
                    "[VEC-RED]   Inserted division for dynamic limit: Value({})",
                    div_dest.0
                );
            }

            if use_byte_iv {
                let mul_dest = Value(next_val_id);
                next_val_id += 1;
                let mul_inst = Instruction::BinOp {
                    dest: mul_dest,
                    op: IrBinOp::Mul,
                    lhs: Operand::Value(div_dest),
                    rhs: Operand::Const(match limit_ty {
                        IrType::I32 => IrConst::I32(byte_stride as i32),
                        IrType::I64 => IrConst::I64(byte_stride as i64),
                        _ => IrConst::I64(byte_stride as i64),
                    }),
                    ty: limit_ty,
                };
                func.blocks[pattern.header_idx]
                    .instructions
                    .insert(pattern.exit_cmp_inst_idx + 1, mul_inst);
                changes += 1;
                Operand::Value(mul_dest)
            } else {
                Operand::Value(div_dest)
            }
        }
        _ => {
            if debug {
                eprintln!("[VEC-RED]   Unsupported limit type");
            }
            return 0;
        }
    };

    // Modify all comparisons that use IV-derived values
    for &block_idx in &pattern.loop_blocks {
        let block = &mut func.blocks[block_idx];
        for inst in &mut block.instructions {
            if let Instruction::Cmp { lhs, rhs, op, .. } = inst {
                if debug {
                    eprintln!("[VEC-RED]   CMP before: {:?} {:?} {:?}", lhs, op, rhs);
                }

                let modifies_lhs = if let Operand::Value(lhs_val) = lhs {
                    iv_derived.contains(lhs_val)
                } else {
                    false
                };

                let modifies_rhs = if let Operand::Value(rhs_val) = rhs {
                    iv_derived.contains(rhs_val)
                } else {
                    false
                };

                if modifies_lhs {
                    *rhs = divided_limit.clone();
                    changes += 1;
                    if debug {
                        eprintln!(
                            "[VEC-RED]   CMP after:  {:?} {:?} {:?} (modified RHS)",
                            lhs, op, rhs
                        );
                    }
                } else if modifies_rhs {
                    *lhs = divided_limit.clone();
                    changes += 1;
                    if debug {
                        eprintln!(
                            "[VEC-RED]   CMP after:  {:?} {:?} {:?} (modified LHS)",
                            lhs, op, rhs
                        );
                    }
                }
            }
        }
    }

    // byte-offset IV — the latch increment steps by `byte_stride` bytes.
    if use_byte_iv {
        let latch = &mut func.blocks[pattern.latch_idx];
        if pattern.iv_inc_idx < latch.instructions.len() {
            if let Instruction::BinOp {
                op: IrBinOp::Add,
                rhs,
                ..
            } = &mut latch.instructions[pattern.iv_inc_idx]
            {
                *rhs = match rhs {
                    Operand::Const(IrConst::I32(_)) => {
                        Operand::Const(IrConst::I32(byte_stride as i32))
                    }
                    _ => Operand::Const(IrConst::I64(byte_stride as i64)),
                };
                changes += 1;
                if debug {
                    eprintln!(
                        "[VEC-RED]   Changed IV increment to byte stride {}",
                        byte_stride
                    );
                }
            }
        }
    }

    // Step 2: Scale array indexing by vector width (element-index scheme only;
    // the byte-offset IV already covers vec_width elements per iteration).
    if !use_byte_iv {
        // Each GEP's byte offset (element_index * elem_size) must be multiplied by
        // vec_width so one vector iteration covers vec_width consecutive elements.
        // Collect ALL matching GEPs first — a dot product has two (array A and
        // array B) in the same block, and the old `break` after the first match
        // left array B at scalar stride, loading the wrong elements.
        let mut geps_to_scale: Vec<(usize, usize, Value)> = Vec::new();
        let all_geps = reduction_array_geps(pattern);
        for &block_idx in &pattern.loop_blocks {
            let block = &func.blocks[block_idx];
            for (inst_idx, inst) in block.instructions.iter().enumerate() {
                if let Instruction::GetElementPtr { dest, offset, .. } = inst {
                    let is_reduction_gep = all_geps
                        .iter()
                        .any(|&(a, b)| *dest == a || Some(*dest) == b);
                    if is_reduction_gep {
                        if let Operand::Value(offset_val) = offset {
                            geps_to_scale.push((block_idx, inst_idx, *offset_val));
                        }
                    }
                }
            }
        }
        // Apply in reverse order so earlier insertions don't shift later indices.
        for (block_idx, inst_idx, offset_val) in geps_to_scale.into_iter().rev() {
            let mul_dest = Value(next_val_id);
            next_val_id += 1;

            let mul_inst = Instruction::BinOp {
                dest: mul_dest,
                op: IrBinOp::Mul,
                lhs: Operand::Value(offset_val),
                rhs: Operand::Const(IrConst::I64(vec_width as i64)),
                ty: IrType::I64,
            };

            func.blocks[block_idx]
                .instructions
                .insert(inst_idx, mul_inst);
            changes += 1;

            if let Instruction::GetElementPtr { offset, .. } =
                &mut func.blocks[block_idx].instructions[inst_idx + 1]
            {
                *offset = Operand::Value(mul_dest);
            }

            if debug {
                eprintln!("[VEC-RED]   Scaled GEP offset by {}", vec_width);
            }
        }
    }

    // Step 3: Transform loop body - replace scalar operations with vector intrinsics
    // Use register-based SSA values for vector operations (no stack allocations)
    let vec_sum_value: Value;
    // NEON smlal dot products split lanes across two 2×I64 accumulators;
    // this carries the second accumulator's header phi to the epilogue.
    let mut second_acc: Option<Value> = None;

    if pattern.seconds.is_empty() {
        match pattern.kind {
            // NEON 4-wide i32 max: acc = smax(acc, load4). The scalar init value
            // (the phi's preheader incoming) is broadcast into the vector
            // accumulator, so zero vector iterations still reduce to the scalar
            // init (smaxv of 4 equal lanes = the init).
            ReductionKind::Max => {
                let init_bcast = Value(next_val_id);
                next_val_id += 1;
                let vec_load = Value(next_val_id);
                next_val_id += 1;
                vec_sum_value = Value(next_val_id);
                next_val_id += 1;

                let latch_label = func.blocks[pattern.latch_idx].label;

                // Read the phi edges FIRST (fail-closed: abort before any
                // mutation if the shape is unexpected), then rewire: backedge ->
                // smax result, preheader -> broadcast of the scalar init.
                let mut preheader_label = None;
                let mut init_operand = None;
                {
                    let header_block = &func.blocks[pattern.header_idx];
                    for inst in &header_block.instructions {
                        if let Instruction::Phi { dest, incoming, .. } = inst {
                            if *dest == pattern.accumulator_phi {
                                for (val, label) in incoming {
                                    if *label != latch_label {
                                        preheader_label = Some(*label);
                                        init_operand = Some(val.clone());
                                    }
                                }
                            }
                        }
                    }
                }
                let (Some(preheader_label), Some(init_operand)) = (preheader_label, init_operand)
                else {
                    if debug {
                        eprintln!("[VEC-RED]   Max: no preheader edge on accumulator phi");
                    }
                    return changes; // nothing mutated yet
                };
                {
                    let header_block = &mut func.blocks[pattern.header_idx];
                    for inst in header_block.instructions.iter_mut() {
                        if let Instruction::Phi { dest, incoming, .. } = inst {
                            if *dest == pattern.accumulator_phi {
                                for (val, label) in incoming.iter_mut() {
                                    if *label == latch_label {
                                        *val = Operand::Value(vec_sum_value);
                                    } else {
                                        *val = Operand::Value(init_bcast);
                                    }
                                }
                            }
                        }
                    }
                }

                // Broadcast the scalar init into all 4 lanes, in the preheader
                // (the init operand dominates that block's terminator).
                if let Some(pre_idx) = func.blocks.iter().position(|b| b.label == preheader_label) {
                    func.blocks[pre_idx]
                        .instructions
                        .push(Instruction::Intrinsic {
                            dest: Some(init_bcast),
                            op: IntrinsicOp::VecBroadcastI32x4,
                            dest_ptr: None,
                            args: vec![init_operand],
                        });
                    changes += 1;
                }

                // Body: 4-wide load + lane-wise smax, replacing the Select and
                // its feeding Cmp. Capture the cmp by VALUE first; remove AFTER
                // the insert/remove (its index may precede the Select's).
                {
                    let body_block = &mut func.blocks[pattern.body_idx];
                    let mut sel_cmp_val = None;
                    if let Instruction::Select {
                        cond: Operand::Value(cv),
                        ..
                    } = &body_block.instructions[pattern.accumulator_add_idx]
                    {
                        sel_cmp_val = Some(*cv);
                    }
                    let (base, off) = match (use_byte_iv, &byte_iv_a) {
                        (true, Some((b, o))) => (Operand::Value(*b), Operand::Value(*o)),
                        _ => (
                            Operand::Value(pattern.array_a_gep),
                            Operand::Const(IrConst::I64(0)),
                        ),
                    };
                    body_block.instructions.insert(
                        pattern.accumulator_add_idx,
                        Instruction::Intrinsic {
                            dest: Some(vec_load),
                            op: IntrinsicOp::VecLoadI32x4,
                            dest_ptr: None,
                            args: vec![base, off],
                        },
                    );
                    body_block.instructions.insert(
                        pattern.accumulator_add_idx + 1,
                        Instruction::Intrinsic {
                            dest: Some(vec_sum_value),
                            op: IntrinsicOp::VecSmaxI32x4,
                            dest_ptr: None,
                            args: vec![
                                Operand::Value(pattern.accumulator_phi),
                                Operand::Value(vec_load),
                            ],
                        },
                    );
                    // Remove the old Select (now shifted to +2).
                    body_block
                        .instructions
                        .remove(pattern.accumulator_add_idx + 2);
                    changes += 2;
                    if let Some(cv) = sel_cmp_val {
                        if let Some(cp) = body_block
                            .instructions
                            .iter()
                            .position(|i| matches!(i.dest(), Some(d) if d.0 == cv.0))
                        {
                            body_block.instructions.remove(cp);
                        }
                    }
                }

                // Scale the marching pointer's latch step from one element
                // (4 bytes) to vec_width elements (16): the detector proved the
                // access is a one-element marching-pointer phi, and the vector
                // body now consumes 4 lanes per iteration. Without this the
                // pointer advances 4 bytes per vector iteration and every load
                // after the first reads 3 stale lanes (find_max returned a[0]-
                // adjacent garbage instead of the max).
                {
                    let mut scaled = false;
                    for &bi in &pattern.loop_blocks {
                        let block = &mut func.blocks[bi];
                        for inst in block.instructions.iter_mut() {
                            if let Instruction::GetElementPtr {
                                base,
                                offset: offset @ Operand::Const(_),
                                ..
                            } = inst
                            {
                                if base.0 == pattern.array_a_gep.0 {
                                    if let Operand::Const(c) = offset {
                                        if c.to_i64() == Some(4) {
                                            *offset = Operand::Const(IrConst::I64(16));
                                            scaled = true;
                                        }
                                    }
                                }
                            }
                        }
                    }
                    debug_assert!(
                        scaled,
                        "max transform requires the marching-pointer step GEP"
                    );
                    if scaled {
                        changes += 1;
                    }
                }

                if debug {
                    eprintln!("[VEC-RED]   Transformed max body: load + smax (NEON 4-wide)");
                }
            }
            ReductionKind::Sum => {
                // Simple sum: sum += arr[i]
                // Map old intrinsics to new register-based intrinsics
                let (vec_load_op, vec_add_op, vec_zero_op) = match load_intrinsic {
                    IntrinsicOp::LoadF64x2 => (
                        IntrinsicOp::VecLoadF64x2,
                        IntrinsicOp::VecAddF64x2,
                        IntrinsicOp::VecZeroF64x2,
                    ),
                    IntrinsicOp::LoadI32x4 => (
                        IntrinsicOp::VecLoadI32x4,
                        IntrinsicOp::VecAddI32x4,
                        IntrinsicOp::VecZeroI32x4,
                    ),
                    IntrinsicOp::VecLoadF32x4 => (
                        IntrinsicOp::VecLoadF32x4,
                        IntrinsicOp::VecAddF32x4,
                        IntrinsicOp::VecZeroF32x4,
                    ),
                    IntrinsicOp::VecLoadWidenI32ToI64x2 => (
                        IntrinsicOp::VecLoadWidenI32ToI64x2,
                        IntrinsicOp::VecAddI64x2,
                        IntrinsicOp::VecZeroI64x2,
                    ),
                    IntrinsicOp::VecLoadI64x2 => (
                        IntrinsicOp::VecLoadI64x2,
                        IntrinsicOp::VecAddI64x2,
                        IntrinsicOp::VecZeroI64x2,
                    ),
                    // NEON 4-wide i32→i64 sum: load 4×I32, accumulate pairs (sadalp).
                    IntrinsicOp::VecLoadI32x4 => (
                        IntrinsicOp::VecLoadI32x4,
                        IntrinsicOp::VecSadalpI32x4,
                        IntrinsicOp::VecZeroI64x2,
                    ),
                    _ => panic!("Unsupported SSE2 load intrinsic: {:?}", load_intrinsic),
                };

                // Create SSA values for vector operations
                // IMPORTANT: Create init_zero_value FIRST (appears in entry block)
                let init_zero_value = Value(next_val_id);
                next_val_id += 1;
                let vec_load = Value(next_val_id);
                next_val_id += 1;
                vec_sum_value = Value(next_val_id);
                next_val_id += 1;

                // Initialize vector accumulator to zero in entry block
                let entry_block = &mut func.blocks[0];
                let zero_inst = Instruction::Intrinsic {
                    dest: Some(init_zero_value),
                    op: vec_zero_op,
                    dest_ptr: None,
                    args: vec![],
                };
                entry_block.instructions.push(zero_inst);
                changes += 1;

                if debug {
                    eprintln!(
                        "[VEC-RED]   Created SSA values: vec_load={}, vec_sum={}",
                        vec_load.0, vec_sum_value.0
                    );
                }

                // Update the accumulator PHI to use init_zero_value as the entry predecessor
                let header_block = &mut func.blocks[pattern.header_idx];
                for inst in header_block.instructions.iter_mut() {
                    if let Instruction::Phi { dest, incoming, .. } = inst {
                        if *dest == pattern.accumulator_phi {
                            for (val, _) in incoming.iter_mut() {
                                if matches!(
                                    val,
                                    Operand::Const(IrConst::F32(_))
                                        | Operand::Const(IrConst::F64(_))
                                        | Operand::Const(IrConst::I32(0))
                                        | Operand::Const(IrConst::I64(0))
                                        | Operand::Const(IrConst::Zero)
                                ) {
                                    *val = Operand::Value(init_zero_value);
                                }
                            }
                        }
                    }
                }

                // Get latch label before taking mutable references
                let latch_label = func.blocks[pattern.latch_idx].label;

                // Create vector load (returns SSA value)
                let load_inst = Instruction::Intrinsic {
                    dest: Some(vec_load),
                    op: vec_load_op,
                    dest_ptr: None,
                    args: {
                        let (base, off) = match (use_byte_iv, &byte_iv_a) {
                            (true, Some((b, o))) => (Operand::Value(*b), Operand::Value(*o)),
                            _ => (
                                Operand::Value(pattern.array_a_gep),
                                Operand::Const(IrConst::I64(0)),
                            ),
                        };
                        vec![base, off]
                    },
                };

                // Create vector add (accumulate) - reads from PHI, produces new value
                let add_inst = Instruction::Intrinsic {
                    dest: Some(vec_sum_value),
                    op: vec_add_op,
                    dest_ptr: None,
                    args: vec![
                        Operand::Value(pattern.accumulator_phi),
                        Operand::Value(vec_load),
                    ],
                };

                // Insert instructions and remove old scalar add
                {
                    let body_block = &mut func.blocks[pattern.body_idx];
                    body_block
                        .instructions
                        .insert(pattern.accumulator_add_idx, load_inst);
                    body_block
                        .instructions
                        .insert(pattern.accumulator_add_idx + 1, add_inst);
                    body_block
                        .instructions
                        .remove(pattern.accumulator_add_idx + 2);
                    changes += 2;

                    // Debug: Log vector accumulator flow
                    if debug {
                        eprintln!(
                            "[VEC-RED-DEBUG-SSE2] Vector accumulator SSA value: {}",
                            vec_sum_value.0
                        );
                        eprintln!(
                            "[VEC-RED-DEBUG-SSE2] Accumulator PHI: {}",
                            pattern.accumulator_phi.0
                        );
                        eprintln!(
                            "[VEC-RED-DEBUG-SSE2] Entry init value: {}",
                            init_zero_value.0
                        );
                        eprintln!("[VEC-RED-DEBUG-SSE2] Vector add op: {:?}", vec_add_op);
                    }
                }

                // Update the PHI's backedge to use vec_sum_value
                {
                    let header_block = &mut func.blocks[pattern.header_idx];
                    for inst in header_block.instructions.iter_mut() {
                        if let Instruction::Phi { dest, incoming, .. } = inst {
                            if *dest == pattern.accumulator_phi {
                                for (val, label) in incoming.iter_mut() {
                                    if *label == latch_label {
                                        *val = Operand::Value(vec_sum_value);
                                    }
                                }
                            }
                        }
                    }
                }

                if debug {
                    eprintln!("[VEC-RED]   Transformed sum body: load + add (register-based)");
                }
            }
            // NEON 4-wide i32→i64 dot product: load both arrays 4×I32, then
            // smlal (lanes 0-1) + smlal2 (lanes 2-3) into two independent 2×I64
            // accumulators to break the loop-carried dependency chain.
            ReductionKind::DotProduct
                if pattern.element_type == IrType::I32
                    && pattern.accumulator_type == IrType::I64
                    && load_intrinsic == IntrinsicOp::VecLoadI32x4 =>
            {
                let vec_load_a = Value(next_val_id);
                next_val_id += 1;
                let vec_load_b = Value(next_val_id);
                next_val_id += 1;
                let acc0_next = Value(next_val_id);
                next_val_id += 1;
                let acc1_next = Value(next_val_id);
                next_val_id += 1;
                let init_zero0 = Value(next_val_id);
                next_val_id += 1;
                let init_zero1 = Value(next_val_id);
                next_val_id += 1;
                let acc1_phi = Value(next_val_id);
                next_val_id += 1;
                vec_sum_value = acc0_next;
                second_acc = Some(acc1_phi);

                // Zero both vector accumulators in the entry block.
                {
                    let entry_block = &mut func.blocks[0];
                    for init in [init_zero0, init_zero1] {
                        entry_block.instructions.push(Instruction::Intrinsic {
                            dest: Some(init),
                            op: IntrinsicOp::VecZeroI64x2,
                            dest_ptr: None,
                            args: vec![],
                        });
                    }
                    changes += 2;
                }

                let latch_label = func.blocks[pattern.latch_idx].label;

                // Rewire the original accumulator phi (acc0): entry → zero vector,
                // backedge → smlal result. Remember the preheader label for acc1.
                let mut preheader_label = None;
                {
                    let header_block = &mut func.blocks[pattern.header_idx];
                    for inst in header_block.instructions.iter_mut() {
                        if let Instruction::Phi { dest, incoming, .. } = inst {
                            if *dest == pattern.accumulator_phi {
                                for (val, label) in incoming.iter_mut() {
                                    if *label == latch_label {
                                        *val = Operand::Value(acc0_next);
                                    } else {
                                        preheader_label = Some(*label);
                                        if matches!(val, Operand::Const(_)) {
                                            *val = Operand::Value(init_zero0);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                let Some(preheader_label) = preheader_label else {
                    if debug {
                        eprintln!("[VEC-RED]   NEON dot: no preheader edge on accumulator phi");
                    }
                    return changes;
                };

                // Insert the second accumulator phi after the existing header phis.
                {
                    let header_block = &mut func.blocks[pattern.header_idx];
                    let insert_pos = header_block
                        .instructions
                        .iter()
                        .rposition(|inst| matches!(inst, Instruction::Phi { .. }))
                        .map(|p| p + 1)
                        .unwrap_or(0);
                    header_block.instructions.insert(
                        insert_pos,
                        Instruction::Phi {
                            dest: acc1_phi,
                            ty: IrType::I64,
                            incoming: vec![
                                (Operand::Value(init_zero1), preheader_label),
                                (Operand::Value(acc1_next), latch_label),
                            ],
                        },
                    );
                    changes += 1;
                }

                // Body: two 4-wide loads + smlal/smlal2, replacing the scalar add.
                {
                    let body_block = &mut func.blocks[pattern.body_idx];
                    body_block.instructions.insert(
                        pattern.accumulator_add_idx,
                        Instruction::Intrinsic {
                            dest: Some(vec_load_a),
                            op: IntrinsicOp::VecLoadI32x4,
                            dest_ptr: None,
                            args: vec![
                                Operand::Value(pattern.array_a_gep),
                                Operand::Const(IrConst::I64(0)),
                            ],
                        },
                    );
                    body_block.instructions.insert(
                        pattern.accumulator_add_idx + 1,
                        Instruction::Intrinsic {
                            dest: Some(vec_load_b),
                            op: IntrinsicOp::VecLoadI32x4,
                            dest_ptr: None,
                            args: vec![
                                Operand::Value(pattern.array_b_gep.unwrap()),
                                Operand::Const(IrConst::I64(0)),
                            ],
                        },
                    );
                    body_block.instructions.insert(
                        pattern.accumulator_add_idx + 2,
                        Instruction::Intrinsic {
                            dest: Some(acc0_next),
                            op: IntrinsicOp::VecSmlalLoI32x4,
                            dest_ptr: None,
                            args: vec![
                                Operand::Value(pattern.accumulator_phi),
                                Operand::Value(vec_load_a),
                                Operand::Value(vec_load_b),
                            ],
                        },
                    );
                    body_block.instructions.insert(
                        pattern.accumulator_add_idx + 3,
                        Instruction::Intrinsic {
                            dest: Some(acc1_next),
                            op: IntrinsicOp::VecSmlalHiI32x4,
                            dest_ptr: None,
                            args: vec![
                                Operand::Value(acc1_phi),
                                Operand::Value(vec_load_a),
                                Operand::Value(vec_load_b),
                            ],
                        },
                    );
                    // Remove the old scalar add; the scalar mul/casts/loads are now
                    // dead and cleaned up by DCE.
                    body_block
                        .instructions
                        .remove(pattern.accumulator_add_idx + 4);
                    changes += 4;
                }

                if debug {
                    eprintln!(
                        "[VEC-RED]   Transformed dot product body: load_a + load_b + smlal + smlal2 (NEON 4-wide)"
                    );
                }
            }
            ReductionKind::DotProduct => {
                // Dot product: sum += a[i] * b[i]
                // Note: Only F64 dot products are supported (no integer multiply intrinsic yet)
                // Create SSA values for vector operations
                let vec_load_a = Value(next_val_id);
                next_val_id += 1;
                let vec_load_b = Value(next_val_id);
                next_val_id += 1;
                let vec_mul = Value(next_val_id);
                next_val_id += 1;
                vec_sum_value = Value(next_val_id);
                next_val_id += 1;

                // Map old intrinsics to new register-based intrinsics (F64 and F32)
                let (vec_load_op, vec_mul_op, vec_add_op, vec_zero_op) = match load_intrinsic {
                    IntrinsicOp::LoadF64x2 => (
                        IntrinsicOp::VecLoadF64x2,
                        IntrinsicOp::VecMulF64x2,
                        IntrinsicOp::VecAddF64x2,
                        IntrinsicOp::VecZeroF64x2,
                    ),
                    IntrinsicOp::VecLoadF32x4 => (
                        IntrinsicOp::VecLoadF32x4,
                        IntrinsicOp::VecMulF32x4,
                        IntrinsicOp::VecAddF32x4,
                        IntrinsicOp::VecZeroF32x4,
                    ),
                    IntrinsicOp::VecLoadWidenI32ToI64x2 => (
                        IntrinsicOp::VecLoadWidenI32ToI64x2,
                        IntrinsicOp::VecMulI64x2,
                        IntrinsicOp::VecAddI64x2,
                        IntrinsicOp::VecZeroI64x2,
                    ),
                    IntrinsicOp::LoadI32x4 => (
                        IntrinsicOp::VecLoadI32x4,
                        IntrinsicOp::VecMulI32x4,
                        IntrinsicOp::VecAddI32x4,
                        IntrinsicOp::VecZeroI32x4,
                    ),
                    IntrinsicOp::VecLoadI64x2 => (
                        IntrinsicOp::VecLoadI64x2,
                        IntrinsicOp::VecMulI64x2,
                        IntrinsicOp::VecAddI64x2,
                        IntrinsicOp::VecZeroI64x2,
                    ),
                    _ => {
                        if debug {
                            eprintln!(
                                "[VEC-RED] Unsupported SSE2 dot product type: {:?}",
                                load_intrinsic
                            );
                        }
                        return changes;
                    }
                };

                // Initialize vector accumulator to zero in entry block
                let entry_block = &mut func.blocks[0];
                let init_zero_value = Value(next_val_id);
                next_val_id += 1;
                let zero_inst = Instruction::Intrinsic {
                    dest: Some(init_zero_value),
                    op: vec_zero_op,
                    dest_ptr: None,
                    args: vec![],
                };
                entry_block.instructions.push(zero_inst);
                changes += 1;

                if debug {
                    eprintln!(
                        "[VEC-RED]   Created SSA values: vec_load_a={}, vec_load_b={}, vec_mul={}, vec_sum={}",
                        vec_load_a.0, vec_load_b.0, vec_mul.0, vec_sum_value.0
                    );
                }

                // Update the accumulator PHI to use init_zero_value as the entry predecessor
                let header_block = &mut func.blocks[pattern.header_idx];
                for inst in header_block.instructions.iter_mut() {
                    if let Instruction::Phi { dest, incoming, .. } = inst {
                        if *dest == pattern.accumulator_phi {
                            for (val, _) in incoming.iter_mut() {
                                if matches!(
                                    val,
                                    Operand::Const(IrConst::F32(_))
                                        | Operand::Const(IrConst::F64(_))
                                        | Operand::Const(IrConst::I32(0))
                                        | Operand::Const(IrConst::I64(0))
                                        | Operand::Const(IrConst::Zero)
                                ) {
                                    *val = Operand::Value(init_zero_value);
                                }
                            }
                        }
                    }
                }

                // Get latch label before taking mutable references
                let latch_label = func.blocks[pattern.latch_idx].label;

                // Create vector loads (return SSA values)
                let load_a_inst = Instruction::Intrinsic {
                    dest: Some(vec_load_a),
                    op: vec_load_op,
                    dest_ptr: None,
                    args: {
                        let (base, off) = match (use_byte_iv, &byte_iv_a) {
                            (true, Some((b, o))) => (Operand::Value(*b), Operand::Value(*o)),
                            _ => (
                                Operand::Value(pattern.array_a_gep),
                                Operand::Const(IrConst::I64(0)),
                            ),
                        };
                        vec![base, off]
                    },
                };

                let load_b_inst = Instruction::Intrinsic {
                    dest: Some(vec_load_b),
                    op: vec_load_op,
                    dest_ptr: None,
                    args: {
                        let (base, off) = match (use_byte_iv, &byte_iv_b) {
                            (true, Some((b, o))) => (Operand::Value(*b), Operand::Value(*o)),
                            _ => (
                                Operand::Value(pattern.array_b_gep.unwrap()),
                                Operand::Const(IrConst::I64(0)),
                            ),
                        };
                        vec![base, off]
                    },
                };

                // Create vector multiply (element-wise)
                let mul_inst = Instruction::Intrinsic {
                    dest: Some(vec_mul),
                    op: vec_mul_op,
                    dest_ptr: None,
                    args: vec![Operand::Value(vec_load_a), Operand::Value(vec_load_b)],
                };

                // Create vector add (accumulate)
                let add_inst = Instruction::Intrinsic {
                    dest: Some(vec_sum_value),
                    op: vec_add_op,
                    dest_ptr: None,
                    args: vec![
                        Operand::Value(pattern.accumulator_phi),
                        Operand::Value(vec_mul),
                    ],
                };

                // Insert all instructions and remove old scalar operations
                {
                    let body_block = &mut func.blocks[pattern.body_idx];
                    body_block
                        .instructions
                        .insert(pattern.accumulator_add_idx, load_a_inst);
                    body_block
                        .instructions
                        .insert(pattern.accumulator_add_idx + 1, load_b_inst);
                    body_block
                        .instructions
                        .insert(pattern.accumulator_add_idx + 2, mul_inst);
                    body_block
                        .instructions
                        .insert(pattern.accumulator_add_idx + 3, add_inst);

                    // Remove the dead scalar add (now after the 4 inserted vector
                    // ops). The dead scalar multiply that fed it becomes unreachable
                    // and is cleaned up by DCE. The old code removed TWO
                    // instructions at accumulator_add_idx + 4, which accidentally
                    // deleted the latch's induction-variable increment once the
                    // byte-offset IV stopped inserting per-GEP multiplies
                    // (infinite loop / undefined IV on dot products).
                    body_block
                        .instructions
                        .remove(pattern.accumulator_add_idx + 4);
                    changes += 4;
                }

                // Update the PHI's backedge to use vec_sum_value
                {
                    let header_block = &mut func.blocks[pattern.header_idx];
                    for inst in header_block.instructions.iter_mut() {
                        if let Instruction::Phi { dest, incoming, .. } = inst {
                            if *dest == pattern.accumulator_phi {
                                for (val, label) in incoming.iter_mut() {
                                    if *label == latch_label {
                                        *val = Operand::Value(vec_sum_value);
                                    }
                                }
                            }
                        }
                    }
                }

                if debug {
                    eprintln!(
                        "[VEC-RED]   Transformed dot product body: load_a + load_b + mul + add (register-based)"
                    );
                }
            }
        }
    } else {
        // Multi-reduction (SSE2/NEON 2-wide): mirror the AVX2 path, deriving
        // the vector ops from `load_intrinsic` exactly like the single
        // accumulator arms do.  Process in descending add-index order.
        let (vec_load_op, vec_mul_op, vec_add_op, vec_zero_op) = match load_intrinsic {
            IntrinsicOp::LoadF64x2 => (
                IntrinsicOp::VecLoadF64x2,
                Some(IntrinsicOp::VecMulF64x2),
                IntrinsicOp::VecAddF64x2,
                IntrinsicOp::VecZeroF64x2,
            ),
            IntrinsicOp::VecLoadF32x4 => (
                IntrinsicOp::VecLoadF32x4,
                Some(IntrinsicOp::VecMulF32x4),
                IntrinsicOp::VecAddF32x4,
                IntrinsicOp::VecZeroF32x4,
            ),
            IntrinsicOp::LoadI32x4 => (
                IntrinsicOp::VecLoadI32x4,
                Some(IntrinsicOp::VecMulI32x4),
                IntrinsicOp::VecAddI32x4,
                IntrinsicOp::VecZeroI32x4,
            ),
            IntrinsicOp::VecLoadI64x2 => (
                IntrinsicOp::VecLoadI64x2,
                Some(IntrinsicOp::VecMulI64x2),
                IntrinsicOp::VecAddI64x2,
                IntrinsicOp::VecZeroI64x2,
            ),
            // Widening/NEON-only forms never reach the multi path: the
            // analyzer rejects widening multi-reductions up front.
            _ => return 0,
        };

        let mut order: Vec<(
            Value,
            usize,
            Value,
            Option<Value>,
            Option<(Value, Value)>,
            Option<(Value, Value)>,
            bool,
        )> = Vec::with_capacity(1 + pattern.seconds.len());
        order.push((
            pattern.accumulator_phi,
            pattern.accumulator_add_idx,
            pattern.array_a_gep,
            pattern.array_b_gep,
            byte_iv_a,
            byte_iv_b,
            true,
        ));
        for sec in &pattern.seconds {
            let sec_byte_iv_a =
                find_reduction_byte_iv(func, &pattern.loop_blocks, sec.array_a_gep, elem_sz as u32);
            let sec_byte_iv_b = sec.array_b_gep.and_then(|g| {
                find_reduction_byte_iv(func, &pattern.loop_blocks, g, elem_sz as u32)
            });
            order.push((
                sec.accumulator_phi,
                sec.accumulator_add_idx,
                sec.array_a_gep,
                sec.array_b_gep,
                sec_byte_iv_a,
                sec_byte_iv_b,
                false,
            ));
        }
        order.sort_by_key(|&(_, add_idx, ..)| std::cmp::Reverse(add_idx));

        let mut primary_sum = Value(u32::MAX);
        for (phi, add_idx, a_gep, b_gep, ba, bb, is_primary) in order {
            let (_init, sum, n) = rewrite_reduction_body(
                func,
                pattern.header_idx,
                pattern.body_idx,
                pattern.latch_idx,
                pattern.kind,
                use_byte_iv,
                vec_load_op,
                vec_add_op,
                vec_zero_op,
                vec_mul_op,
                false,
                None,
                phi,
                add_idx,
                a_gep,
                b_gep,
                ba,
                bb,
                &mut next_val_id,
            );
            changes += n;
            if is_primary {
                primary_sum = sum;
            }
        }
        debug_assert!(
            primary_sum.0 != u32::MAX,
            "primary accumulator not rewritten"
        );
        vec_sum_value = primary_sum;
    }

    // Merge duplicate vector loads (a shared array is loaded once per
    // iteration; `sum += x*x` and `b += v*w` after `a += u*v` both benefit).
    changes += deduplicate_vector_loads(func, &pattern.loop_blocks);

    // Step 4: Create remainder loop.
    //
    // Snapshot the labels of every block OUTSIDE the loop first; see the AVX2
    // path and `rewire_escaping_iv_uses` for why. Both vector paths need this:
    // the counter is redefined by the transform regardless of which vector
    // width was chosen, and an i64 reduction (SSE2, 2-wide) leaked a BYTE
    // count exactly as the i32 one (AVX2, 8-wide) did.
    let outside_labels: Vec<BlockId> = func
        .blocks
        .iter()
        .enumerate()
        .filter(|(bi, _)| !pattern.loop_blocks.contains(bi))
        .map(|(_, b)| b.label)
        .collect();

    let mut rem_iv: Option<Value> = None;
    let remainder_changes = insert_reduction_remainder_loop(
        func,
        pattern,
        vec_width,
        horizontal_intrinsic,
        vec_sum_value, // Pass the vector accumulator SSA value
        use_byte_iv,
        second_acc,       // Second NEON accumulator phi (smlal2 half), if any
        &pattern.seconds, // Extra independent accumulators, if any
        &mut next_val_id,
        &mut next_label,
        &mut rem_iv,
    );
    changes += remainder_changes;

    if debug {
        eprintln!(
            "[VEC-RED]   Added remainder loop: {} blocks created",
            remainder_changes / 4
        );
    }

    // Step 5: repair uses of the loop counter that ESCAPE the loop.
    changes += rewire_escaping_iv_uses(func, pattern, rem_iv, &outside_labels, debug);

    // Update the function's next_value_id and next_label
    func.next_value_id = next_val_id;
    func.next_label = next_label;

    changes
}

/// Transform a legal one-source store loop to packed 128/256-bit form.  The
/// optional scale and offset operations stay optional: copies become exactly
/// load/store, while scale/add/affine maps emit only their source operations.
fn transform_map_vector(
    func: &mut IrFunction,
    pattern: &MapPattern,
    avx2: bool,
    fp_contract: crate::common::fp_contract::FpContract,
) -> usize {
    let debug = std::env::var("LCCC_DEBUG_VECTORIZE").is_ok();
    let mut changes = 0;
    let vec_width: u64 = match (pattern.elem_ty, avx2) {
        (IrType::F64, true) => 4,
        (IrType::F64, false) => 2,
        (IrType::I64 | IrType::U64, _) => 2,
        (_, true) => 8,
        (_, false) => 4,
    };

    // A zero-iteration vector loop plus scalar remainder only adds overhead.
    if matches!(&pattern.limit, Operand::Const(c)
        if c.to_i64().map_or(false, |n| n <= vec_width as i64))
    {
        return 0;
    }

    let mut next_val_id = func.next_value_id;
    let mut next_label = func.next_label.max(
        func.blocks
            .iter()
            .map(|b| b.label.0)
            .max()
            .unwrap_or(0)
            .saturating_add(1),
    );

    // Locate the unique preheader before mutating anything.  Broadcasts live
    // there and are register-allocated across the packed loop.
    let Some(preheader_idx) = func.blocks.iter().enumerate().find_map(|(idx, block)| {
        if pattern.loop_blocks.contains(&idx) {
            return None;
        }
        matches!(block.terminator, Terminator::Branch(label)
            if label == func.blocks[pattern.header_idx].label)
        .then_some(idx)
    }) else {
        if debug {
            eprintln!("[VEC-MAP]   No preheader found; bailing");
        }
        return 0;
    };

    let elem_size = if pattern.elem_ty == IrType::F64 { 8 } else { 4 };
    let mut src_bases = Vec::with_capacity(pattern.src_geps.len());
    for &src_gep in &pattern.src_geps {
        let Some((base, _)) =
            find_reduction_byte_iv(func, &pattern.loop_blocks, src_gep, elem_size)
        else {
            return 0;
        };
        src_bases.push(base);
    }
    let Some((dst_base, _)) =
        find_reduction_byte_iv(func, &pattern.loop_blocks, pattern.dst_gep, elem_size)
    else {
        return 0;
    };

    // Build IV-derived values for comparison rewriting.
    let mut iv_derived = FxHashSet::default();
    iv_derived.insert(pattern.iv);
    let mut changed = true;
    while changed {
        changed = false;
        for &block_idx in &pattern.loop_blocks {
            for inst in &func.blocks[block_idx].instructions {
                if let Instruction::Cast { dest, src, .. } | Instruction::Copy { dest, src } = inst
                {
                    if let Operand::Value(src_val) = src {
                        if iv_derived.contains(src_val) && iv_derived.insert(*dest) {
                            changed = true;
                        }
                    }
                }
            }
        }
    }

    // Divide the loop bound by the packed width (constant folded or dynamic).
    let divided_limit = match &pattern.limit {
        Operand::Const(IrConst::I32(n)) => Operand::Const(IrConst::I32(*n / vec_width as i32)),
        Operand::Const(IrConst::I64(n)) => Operand::Const(IrConst::I64(*n / vec_width as i64)),
        Operand::Value(limit_val) => {
            let shift = vec_width.trailing_zeros() as i64;
            let int_const = |n: i64| match pattern.iv_ty {
                IrType::I32 | IrType::U32 => IrConst::I32(n as i32),
                _ => IrConst::I64(n),
            };
            let mut quotient_insts = Vec::new();
            let div_dest = if pattern.exit_cmp_op == IrCmpOp::Slt {
                // Signed division by 2^k, rounded toward zero:
                //   (n + ((n >> (bits-1)) & (2^k-1))) >> k
                // This preserves negative/zero-trip semantics without putting
                // a costly `idiv` in the vector loop or its preheader.
                let sign = Value(next_val_id);
                next_val_id += 1;
                let bias = Value(next_val_id);
                next_val_id += 1;
                let adjusted = Value(next_val_id);
                next_val_id += 1;
                let quotient = Value(next_val_id);
                next_val_id += 1;
                quotient_insts.push(Instruction::BinOp {
                    dest: sign,
                    op: IrBinOp::AShr,
                    lhs: Operand::Value(*limit_val),
                    rhs: Operand::Const(int_const(
                        if matches!(pattern.iv_ty, IrType::I32 | IrType::U32) {
                            31
                        } else {
                            63
                        },
                    )),
                    ty: pattern.iv_ty,
                });
                quotient_insts.push(Instruction::BinOp {
                    dest: bias,
                    op: IrBinOp::And,
                    lhs: Operand::Value(sign),
                    rhs: Operand::Const(int_const(vec_width as i64 - 1)),
                    ty: pattern.iv_ty,
                });
                quotient_insts.push(Instruction::BinOp {
                    dest: adjusted,
                    op: IrBinOp::Add,
                    lhs: Operand::Value(*limit_val),
                    rhs: Operand::Value(bias),
                    ty: pattern.iv_ty,
                });
                quotient_insts.push(Instruction::BinOp {
                    dest: quotient,
                    op: IrBinOp::AShr,
                    lhs: Operand::Value(adjusted),
                    rhs: Operand::Const(int_const(shift)),
                    ty: pattern.iv_ty,
                });
                quotient
            } else {
                let quotient = Value(next_val_id);
                next_val_id += 1;
                quotient_insts.push(Instruction::BinOp {
                    dest: quotient,
                    op: IrBinOp::LShr,
                    lhs: Operand::Value(*limit_val),
                    rhs: Operand::Const(int_const(shift)),
                    ty: pattern.iv_ty,
                });
                quotient
            };
            changes += quotient_insts.len();
            func.blocks[preheader_idx]
                .instructions
                .extend(quotient_insts);
            Operand::Value(div_dest)
        }
        _ => return 0,
    };

    // Rewrite the canonical IV < limit test to IV < floor(limit / width).
    for &block_idx in &pattern.loop_blocks {
        for inst in &mut func.blocks[block_idx].instructions {
            if let Instruction::Cmp { lhs, rhs, .. } = inst {
                let modifies_lhs = matches!(lhs, Operand::Value(v) if iv_derived.contains(v));
                let modifies_rhs = matches!(rhs, Operand::Value(v) if iv_derived.contains(v));
                if modifies_lhs {
                    *rhs = divided_limit.clone();
                    changes += 1;
                } else if modifies_rhs {
                    *lhs = divided_limit.clone();
                    changes += 1;
                }
            }
        }
    }

    // Carry an independent I64 byte offset through the packed loop.  The
    // source-language IV remains an element counter for its exact signed or
    // unsigned exit semantics and for scalar-remainder recovery; the byte IV
    // gives both memory streams one SIB/NEON indexed address with no per-
    // iteration cast, multiply, or GEP materialization.
    let byte_iv = Value(next_val_id);
    next_val_id += 1;
    let byte_iv_next = Value(next_val_id);
    next_val_id += 1;
    let preheader_label = func.blocks[preheader_idx].label;
    let latch_label = func.blocks[pattern.latch_idx].label;
    let phi_pos = func.blocks[pattern.header_idx]
        .instructions
        .iter()
        .position(|inst| !matches!(inst, Instruction::Phi { .. }))
        .unwrap_or(func.blocks[pattern.header_idx].instructions.len());
    func.blocks[pattern.header_idx].instructions.insert(
        phi_pos,
        Instruction::Phi {
            dest: byte_iv,
            ty: IrType::I64,
            incoming: vec![
                (Operand::Const(IrConst::I64(0)), preheader_label),
                (Operand::Value(byte_iv_next), latch_label),
            ],
        },
    );
    func.blocks[pattern.latch_idx]
        .instructions
        .push(Instruction::BinOp {
            dest: byte_iv_next,
            op: IrBinOp::Add,
            lhs: Operand::Value(byte_iv),
            rhs: Operand::Const(IrConst::I64((elem_size as u64 * vec_width) as i64)),
            ty: IrType::I64,
        });
    changes += 2;

    let dst_address = (dst_base, Operand::Value(byte_iv));

    let broadcast_op = match (pattern.elem_ty, avx2) {
        (IrType::F64, true) => IntrinsicOp::VecBroadcastF64x4,
        (IrType::F64, false) => IntrinsicOp::VecBroadcastF64x2,
        (IrType::F32, true) => IntrinsicOp::VecBroadcastF32x8,
        (IrType::F32, false) => IntrinsicOp::VecBroadcastF32x4,
        (IrType::I32 | IrType::U32, true) => IntrinsicOp::VecBroadcastI32x8,
        (IrType::I32 | IrType::U32, false) => IntrinsicOp::VecBroadcastI32x4,
        (IrType::I64 | IrType::U64, _) => IntrinsicOp::VecBroadcastI64x2,
        _ => return 0,
    };

    let load_op = match (pattern.elem_ty, avx2) {
        (IrType::F64, true) => IntrinsicOp::VecLoadF64x4,
        (IrType::F64, false) => IntrinsicOp::VecLoadF64x2,
        (IrType::F32, true) => IntrinsicOp::VecLoadF32x8,
        (IrType::F32, false) => IntrinsicOp::VecLoadF32x4,
        (IrType::I32 | IrType::U32, true) => IntrinsicOp::VecLoadI32x8,
        (IrType::I32 | IrType::U32, false) => IntrinsicOp::VecLoadI32x4,
        (IrType::I64 | IrType::U64, _) => IntrinsicOp::VecLoadI64x2,
        _ => return 0,
    };
    let store_op = match (pattern.elem_ty, avx2) {
        (IrType::F64, true) => IntrinsicOp::VecStoreF64x4,
        (IrType::F64, false) => IntrinsicOp::VecStoreF64x2,
        (IrType::F32, true) => IntrinsicOp::VecStoreF32x8,
        (IrType::F32, false) => IntrinsicOp::VecStoreF32x4,
        (IrType::I32 | IrType::U32, true) => IntrinsicOp::VecStoreI32x8,
        (IrType::I32 | IrType::U32, false) => IntrinsicOp::VecStoreI32x4,
        (IrType::I64 | IrType::U64, _) => IntrinsicOp::VecStoreI64x2,
        _ => return 0,
    };
    let bin_op = |op: &IrBinOp| -> Option<IntrinsicOp> {
        match (pattern.elem_ty, avx2, op) {
            (IrType::F64, true, IrBinOp::Add) => Some(IntrinsicOp::VecAddF64x4),
            (IrType::F64, false, IrBinOp::Add) => Some(IntrinsicOp::VecAddF64x2),
            (IrType::F64, true, IrBinOp::Sub) => Some(IntrinsicOp::VecSubF64x4),
            (IrType::F64, false, IrBinOp::Sub) => Some(IntrinsicOp::VecSubF64x2),
            (IrType::F64, true, IrBinOp::Mul) => Some(IntrinsicOp::VecMulF64x4),
            (IrType::F64, false, IrBinOp::Mul) => Some(IntrinsicOp::VecMulF64x2),
            (IrType::F64, true, IrBinOp::SDiv) => Some(IntrinsicOp::VecDivF64x4),
            (IrType::F64, false, IrBinOp::SDiv) => Some(IntrinsicOp::VecDivF64x2),
            (IrType::F32, true, IrBinOp::Add) => Some(IntrinsicOp::VecAddF32x8),
            (IrType::F32, false, IrBinOp::Add) => Some(IntrinsicOp::VecAddF32x4),
            (IrType::F32, true, IrBinOp::Sub) => Some(IntrinsicOp::VecSubF32x8),
            (IrType::F32, false, IrBinOp::Sub) => Some(IntrinsicOp::VecSubF32x4),
            (IrType::F32, true, IrBinOp::Mul) => Some(IntrinsicOp::VecMulF32x8),
            (IrType::F32, false, IrBinOp::Mul) => Some(IntrinsicOp::VecMulF32x4),
            (IrType::F32, true, IrBinOp::SDiv) => Some(IntrinsicOp::VecDivF32x8),
            (IrType::F32, false, IrBinOp::SDiv) => Some(IntrinsicOp::VecDivF32x4),
            (IrType::I32 | IrType::U32, true, IrBinOp::Add) => Some(IntrinsicOp::VecAddI32x8),
            (IrType::I32 | IrType::U32, false, IrBinOp::Add) => Some(IntrinsicOp::VecAddI32x4),
            (IrType::I32 | IrType::U32, true, IrBinOp::Mul) => Some(IntrinsicOp::VecMulI32x8),
            (IrType::I32 | IrType::U32, false, IrBinOp::Mul) => Some(IntrinsicOp::VecMulI32x4),
            (IrType::I64 | IrType::U64, _, IrBinOp::Add) => Some(IntrinsicOp::VecAddI64x2),
            (IrType::I64 | IrType::U64, _, IrBinOp::Mul) => Some(IntrinsicOp::VecMulI64x2),
            _ => None,
        }
    };
    // Sqrt is only available for FP element types; integer trees never
    // contain Sqrt (the parser gates it), so `None` is fine there.
    let sqrt_op = match (pattern.elem_ty, avx2) {
        (IrType::F64, true) => Some(IntrinsicOp::VecSqrtF64x4),
        (IrType::F64, false) => Some(IntrinsicOp::VecSqrtF64x2),
        (IrType::F32, true) => Some(IntrinsicOp::VecSqrtF32x8),
        (IrType::F32, false) => Some(IntrinsicOp::VecSqrtF32x4),
        _ => None,
    };
    let madd_op = if fp_contract == FpContract::Fast && avx2 && x86_fma_enabled() {
        match pattern.elem_ty {
            IrType::F64 => Some(IntrinsicOp::VecMaddF64x4),
            IrType::F32 => Some(IntrinsicOp::VecMaddF32x8),
            _ => None,
        }
    } else {
        None
    };

    // Fail-closed BEFORE any mutation: every operation the tree needs must
    // have a lowering. A mid-transform bail after the preheader/byte-IV
    // rewrite would leave `func.next_value_id` stale and corrupt every
    // later pass (bit_idioms indexed out of bounds on exactly this).
    if !map_tree_ops_available(&pattern.expr, &bin_op, sqrt_op) {
        if debug {
            eprintln!("[VEC-MAP]   Tree requires an op with no vector lowering");
        }
        return 0;
    }

    // Replace the scalar store with only the packed operations present in the
    // source expression.  DCE removes the now-unreachable scalar dataflow.
    {
        let mut ctx = MapEmitCtx {
            src_bases: &src_bases,
            byte_iv,
            load_op,
            broadcast_op,
            sqrt_op,
            madd_op,
            bin_op: &bin_op,
            broadcast_cache: Vec::new(),
            preheader_insts: Vec::new(),
            vec_insts: Vec::new(),
            next_val_id,
            changes: &mut changes,
        };
        let Some(current) = ctx.emit(&pattern.expr) else {
            if debug {
                eprintln!("[VEC-MAP]   Tree emission failed");
            }
            return changes;
        };
        let store_args = vec![
            Operand::Value(current),
            Operand::Value(dst_address.0),
            dst_address.1.clone(),
        ];
        ctx.vec_insts.push(Instruction::Intrinsic {
            dest: None,
            op: store_op,
            dest_ptr: Some(dst_address.0),
            args: store_args,
        });
        // Broadcasts live in the preheader so they are hoisted out of the
        // packed loop (register-allocated once, reused across iterations).
        func.blocks[preheader_idx]
            .instructions
            .extend(ctx.preheader_insts);
        next_val_id = ctx.next_val_id;
        let mut vec_insts = ctx.vec_insts;

        let body = &mut func.blocks[pattern.body_idx];
        let Some(store_pos) = body.instructions.iter().position(
            |inst| matches!(inst, Instruction::Store { ptr, .. } if *ptr == pattern.dst_gep),
        ) else {
            if debug {
                eprintln!("[VEC-MAP]   Store not found at transform time");
            }
            return changes;
        };
        let inserted = vec_insts.len();
        for (i, inst) in vec_insts.drain(..).enumerate() {
            body.instructions.insert(store_pos + i, inst);
        }
        body.instructions.remove(store_pos + inserted);
        changes += inserted;
    }

    changes +=
        insert_map_remainder_loop(func, pattern, vec_width, &mut next_val_id, &mut next_label);

    func.next_value_id = next_val_id;
    func.next_label = next_label;

    if debug {
        eprintln!("[VEC-MAP]   Map transform complete: {} changes", changes);
    }
    changes
}

/// Insert an exact scalar remainder for a vectorized map.  It mirrors the
/// source element/IV widths and optional operations rather than assuming the
/// old fixed I32/F32 `mul + add` shape.
fn insert_map_remainder_loop(
    func: &mut IrFunction,
    pattern: &MapPattern,
    vec_width: u64,
    next_val_id: &mut u32,
    next_label: &mut u32,
) -> usize {
    let mut src_bases: Vec<Option<Value>> = vec![None; pattern.src_geps.len()];
    let mut dst_base = None;
    for &block_idx in &pattern.loop_blocks {
        for inst in &func.blocks[block_idx].instructions {
            if let Instruction::GetElementPtr { dest, base, .. } = inst {
                if let Some(slot) = pattern.src_geps.iter().position(|g| *g == *dest) {
                    src_bases[slot] = Some(*base);
                }
                if *dest == pattern.dst_gep {
                    dst_base = Some(*base);
                }
            }
        }
    }
    let Some(dst_base) = dst_base else {
        return 0;
    };
    let src_bases: Vec<Value> = match src_bases.into_iter().collect::<Option<Vec<_>>>() {
        Some(v) => v,
        None => return 0,
    };

    let vec_exit_label = BlockId(*next_label);
    *next_label += 1;
    let remainder_header_label = BlockId(*next_label);
    *next_label += 1;
    let remainder_body_label = BlockId(*next_label);
    *next_label += 1;
    let remainder_latch_label = BlockId(*next_label);
    *next_label += 1;

    let mut fresh = || {
        let value = Value(*next_val_id);
        *next_val_id += 1;
        value
    };
    let i_rem_start = fresh();
    let i_rem_iv = fresh();
    let i_rem_iv_next = fresh();
    let i_rem_cmp = fresh();
    let i_rem_cast = matches!(pattern.iv_ty, IrType::I32 | IrType::U32).then(|| fresh());
    let offset_v = fresh();
    let gep_dst = fresh();

    // Redirect the vectorized header's known false/exit edge.
    if let Terminator::CondBranch { false_label, .. } =
        &mut func.blocks[pattern.header_idx].terminator
    {
        *false_label = vec_exit_label;
    }

    let iv_width_const = match pattern.iv_ty {
        IrType::I32 | IrType::U32 => IrConst::I32(vec_width as i32),
        _ => IrConst::I64(vec_width as i64),
    };
    let one = match pattern.iv_ty {
        IrType::I32 | IrType::U32 => IrConst::I32(1),
        _ => IrConst::I64(1),
    };

    let vec_exit_block = BasicBlock {
        label: vec_exit_label,
        instructions: vec![Instruction::BinOp {
            dest: i_rem_start,
            op: IrBinOp::Mul,
            lhs: Operand::Value(pattern.iv),
            rhs: Operand::Const(iv_width_const),
            ty: pattern.iv_ty,
        }],
        terminator: Terminator::Branch(remainder_header_label),
        source_spans: vec![],
    };

    let remainder_header_block = BasicBlock {
        label: remainder_header_label,
        instructions: vec![
            Instruction::Phi {
                dest: i_rem_iv,
                ty: pattern.iv_ty,
                incoming: vec![
                    (Operand::Value(i_rem_start), vec_exit_label),
                    (Operand::Value(i_rem_iv_next), remainder_latch_label),
                ],
            },
            Instruction::Cmp {
                dest: i_rem_cmp,
                op: pattern.exit_cmp_op,
                lhs: Operand::Value(i_rem_iv),
                rhs: pattern.limit.clone(),
                ty: pattern.iv_ty,
            },
        ],
        terminator: Terminator::CondBranch {
            cond: Operand::Value(i_rem_cmp),
            true_label: remainder_body_label,
            false_label: func.blocks[pattern.exit_idx].label,
        },
        source_spans: vec![],
    };

    let mut remainder_insts = Vec::new();
    let byte_index = if let Some(cast) = i_rem_cast {
        remainder_insts.push(Instruction::Cast {
            dest: cast,
            src: Operand::Value(i_rem_iv),
            from_ty: pattern.iv_ty,
            to_ty: IrType::I64,
        });
        cast
    } else {
        i_rem_iv
    };
    let elem_bytes = if pattern.elem_ty == IrType::F64 { 8 } else { 4 };
    remainder_insts.push(Instruction::BinOp {
        dest: offset_v,
        op: IrBinOp::Mul,
        lhs: Operand::Value(byte_index),
        rhs: Operand::Const(IrConst::I64(elem_bytes)),
        ty: IrType::I64,
    });
    remainder_insts.push(Instruction::GetElementPtr {
        dest: gep_dst,
        base: dst_base,
        offset: Operand::Value(offset_v),
        ty: pattern.elem_ty,
    });
    // Per-stream GEP + load, then the scalar mirror of the expression tree.
    // Invariant leaves are already available as operands (no instruction);
    // Sqrt re-emits the scalar SqrtF32/SqrtF64 intrinsic, and BinOps re-emit
    // the identical scalar operation — lane-exact by construction.
    let mut next_val_local = *next_val_id;
    let Some(scalar_result) = emit_map_scalar_tree(
        &pattern.expr,
        &src_bases,
        pattern,
        Operand::Value(offset_v),
        &mut remainder_insts,
        &mut next_val_local,
    ) else {
        return 0;
    };
    *next_val_id = next_val_local;
    remainder_insts.push(Instruction::Store {
        volatile: false,
        val: scalar_result,
        ptr: gep_dst,
        ty: pattern.elem_ty,
        seg_override: AddressSpace::Default,
    });

    let remainder_body_block = BasicBlock {
        label: remainder_body_label,
        instructions: remainder_insts,
        terminator: Terminator::Branch(remainder_latch_label),
        source_spans: vec![],
    };

    let remainder_latch_block = BasicBlock {
        label: remainder_latch_label,
        instructions: vec![Instruction::BinOp {
            dest: i_rem_iv_next,
            op: IrBinOp::Add,
            lhs: Operand::Value(i_rem_iv),
            rhs: Operand::Const(one),
            ty: pattern.iv_ty,
        }],
        terminator: Terminator::Branch(remainder_header_label),
        source_spans: vec![],
    };

    func.blocks.push(vec_exit_block);
    func.blocks.push(remainder_header_block);
    func.blocks.push(remainder_body_block);
    func.blocks.push(remainder_latch_block);

    4
}

/// Scalar mirror of the map expression tree for the exact remainder loop
/// (OP-05a). Emits one instruction per tree node — the identical scalar
/// operation the original loop performed — so tail elements observe the same
/// arithmetic (and, without `-ffp-contract=fast`, the same rounding) as the
/// pre-vectorization code.
fn emit_map_scalar_tree(
    expr: &MapExpr,
    src_bases: &[Value],
    pattern: &MapPattern,
    byte_offset: Operand,
    remainder_insts: &mut Vec<Instruction>,
    next_val_id: &mut u32,
) -> Option<Operand> {
    match expr {
        MapExpr::Load(stream) => {
            let gep = {
                let v = Value(*next_val_id);
                *next_val_id += 1;
                v
            };
            let load = {
                let v = Value(*next_val_id);
                *next_val_id += 1;
                v
            };
            remainder_insts.push(Instruction::GetElementPtr {
                dest: gep,
                base: src_bases[*stream],
                offset: byte_offset.clone(),
                ty: pattern.elem_ty,
            });
            remainder_insts.push(Instruction::Load {
                volatile: false,
                dest: load,
                ptr: gep,
                ty: pattern.elem_ty,
                seg_override: AddressSpace::Default,
            });
            Some(Operand::Value(load))
        }
        MapExpr::Invariant(operand) => Some(operand.clone()),
        MapExpr::Sqrt(x) => {
            let inner = emit_map_scalar_tree(
                x,
                src_bases,
                pattern,
                byte_offset,
                remainder_insts,
                next_val_id,
            )?;
            let dest = {
                let v = Value(*next_val_id);
                *next_val_id += 1;
                v
            };
            let sqrt_op = match pattern.elem_ty {
                IrType::F64 => IntrinsicOp::SqrtF64,
                IrType::F32 => IntrinsicOp::SqrtF32,
                _ => return None,
            };
            remainder_insts.push(Instruction::Intrinsic {
                dest: Some(dest),
                op: sqrt_op,
                dest_ptr: None,
                args: vec![inner],
            });
            Some(Operand::Value(dest))
        }
        MapExpr::BinOp(op, l, r) => {
            let lhs = emit_map_scalar_tree(
                l,
                src_bases,
                pattern,
                byte_offset.clone(),
                remainder_insts,
                next_val_id,
            )?;
            let rhs = emit_map_scalar_tree(
                r,
                src_bases,
                pattern,
                byte_offset,
                remainder_insts,
                next_val_id,
            )?;
            let dest = {
                let v = Value(*next_val_id);
                *next_val_id += 1;
                v
            };
            remainder_insts.push(Instruction::BinOp {
                dest,
                op: *op,
                lhs,
                rhs,
                ty: pattern.elem_ty,
            });
            Some(Operand::Value(dest))
        }
    }
}

/// Recognize a fixed-width squared-distance expression and pack its independent
/// lane work.  Reassociation is a hard precondition because the horizontal
/// reduction tree need not match the source's left-associated additions.
/// The profitable widths are complete 256-bit F32x8/F64x4 vectors. Narrower
/// and partial vectors retain scalar code (for example p50's three-double
/// distance) rather than paying a horizontal-reduction loss or reading past
/// the object boundary.
fn transform_fixed_distance_slp(func: &mut IrFunction) -> usize {
    if func.blocks.len() != 1 || std::env::var("CCC_NO_FIXED_SLP").is_ok() {
        return 0;
    }
    let block = &func.blocks[0];
    // The packed intrinsic performs its loads at the replacement site, just
    // before the return.  A call, store, atomic, or other ordered operation
    // anywhere in this straight-line block could change the source objects (or
    // make the original access order observable) between the scalar loads and
    // that site.  Restrict this specialized SLP fold to pure address/scalar
    // computation plus ordinary loads.  This conservative whitelist also makes
    // future side-effecting IR variants reject by default.
    if block.instructions.iter().any(|inst| {
        !matches!(
            inst,
            Instruction::Alloca { .. }
                | Instruction::Load { .. }
                | Instruction::BinOp { .. }
                | Instruction::UnaryOp { .. }
                | Instruction::Cmp { .. }
                | Instruction::GetElementPtr { .. }
                | Instruction::Cast { .. }
                | Instruction::Copy { .. }
                | Instruction::GlobalAddr { .. }
                | Instruction::Phi { .. }
                | Instruction::LabelAddr { .. }
                | Instruction::Select { .. }
                | Instruction::ParamRef { .. }
        )
    }) {
        return 0;
    }
    let ty = func.return_type;
    if !matches!(ty, IrType::F32 | IrType::F64) {
        return 0;
    }
    let Operand::Value(root) = (match &block.terminator {
        Terminator::Return(Some(value)) => value.clone(),
        _ => return 0,
    }) else {
        return 0;
    };

    fn collect_add_terms(
        block: &BasicBlock,
        value: Value,
        ty: IrType,
        terms: &mut Vec<Value>,
    ) -> bool {
        if terms.len() > 8 {
            return false;
        }
        if let Some(Instruction::BinOp {
            op: IrBinOp::Add,
            lhs: Operand::Value(lhs),
            rhs: Operand::Value(rhs),
            ty: add_ty,
            ..
        }) = find_inst_by_dest(block, value)
        {
            if *add_ty == ty {
                return collect_add_terms(block, *lhs, ty, terms)
                    && collect_add_terms(block, *rhs, ty, terms);
            }
        }
        terms.push(value);
        true
    }

    fn load_address(block: &BasicBlock, value: Value, ty: IrType) -> Option<(Value, i64)> {
        let Instruction::Load {
            ptr,
            ty: load_ty,
            seg_override,
            ..
        } = find_inst_by_dest(block, value)?
        else {
            return None;
        };
        // The fixed-distance intrinsic currently emits ordinary (%gpr) vector
        // loads.  Folding a __seg_fs/__seg_gs scalar load would silently drop
        // its architectural address-space prefix and read different memory.
        if *load_ty != ty || *seg_override != AddressSpace::Default {
            return None;
        }
        if let Some(Instruction::GetElementPtr {
            base,
            offset: Operand::Const(offset),
            ..
        }) = find_inst_by_dest(block, *ptr)
        {
            Some((*base, offset.to_i64()?))
        } else {
            Some((*ptr, 0))
        }
    }

    let mut terms = Vec::new();
    if !collect_add_terms(block, root, ty, &mut terms) {
        return 0;
    }
    let lanes = terms.len();
    // Four F32 or two F64 lanes do not amortize the baseline horizontal
    // sequence and remain scalar. Full 256-bit F32x8/F64x4 vectors are the
    // measured profitable widths on x86-64-v3.
    let width_supported = matches!((ty, lanes), (IrType::F32, 8) | (IrType::F64, 4));
    if !width_supported {
        return 0;
    }

    let mut lane_addresses = Vec::with_capacity(lanes);
    for term in terms {
        let Some(Instruction::BinOp {
            op: IrBinOp::Mul,
            lhs: Operand::Value(lhs),
            rhs: Operand::Value(rhs),
            ty: mul_ty,
            ..
        }) = find_inst_by_dest(block, term)
        else {
            return 0;
        };
        if *mul_ty != ty || lhs != rhs {
            return 0;
        }
        let Some(Instruction::BinOp {
            op: IrBinOp::Sub,
            lhs: Operand::Value(a),
            rhs: Operand::Value(b),
            ty: sub_ty,
            ..
        }) = find_inst_by_dest(block, *lhs)
        else {
            return 0;
        };
        if *sub_ty != ty {
            return 0;
        }
        let Some(a_addr) = load_address(block, *a, ty) else {
            return 0;
        };
        let Some(b_addr) = load_address(block, *b, ty) else {
            return 0;
        };
        lane_addresses.push((a_addr, b_addr));
    }
    lane_addresses.sort_by_key(|((_, offset), _)| *offset);
    let a_base = lane_addresses[0].0 .0;
    let b_base = lane_addresses[0].1 .0;
    let elem_size = if ty == IrType::F64 { 8 } else { 4 };
    for (lane, ((this_a, a_offset), (this_b, b_offset))) in lane_addresses.iter().enumerate() {
        let expected = lane as i64 * elem_size;
        if *this_a != a_base || *this_b != b_base || *a_offset != expected || *b_offset != expected
        {
            return 0;
        }
    }

    let op = match (ty, lanes) {
        (IrType::F32, 8) => IntrinsicOp::FixedDistanceF32x8,
        (IrType::F64, 4) => IntrinsicOp::FixedDistanceF64x4,
        _ => return 0,
    };
    let scalar = Value(func.next_value_id);
    func.next_value_id += 1;
    let block = &mut func.blocks[0];
    block.instructions.push(Instruction::Intrinsic {
        dest: Some(scalar),
        op,
        dest_ptr: None,
        args: vec![Operand::Value(a_base), Operand::Value(b_base)],
    });
    block.terminator = Terminator::Return(Some(Operand::Value(scalar)));
    1
}

/// Run x86 vectorization under strict floating-point semantics.
pub(crate) fn vectorize_function(func: &mut IrFunction) -> usize {
    vectorize_function_mode(func, false, FpContract::default(), false)
}

/// `-ffp-contract=fast` without `-fassociative-math`: FP reductions keep
/// their scalar-order legality gate (reassociation still off), but map
/// expression trees may contract mul+add into fused madd — the same
/// distinction GCC makes (`-ffp-contract=fast` alone contracts loops;
/// reduction reassociation needs `-fassociative-math`).
pub(crate) fn vectorize_function_contract(func: &mut IrFunction) -> usize {
    vectorize_function_mode(func, false, FpContract::Fast, false)
}

/// Run x86 vectorization with reassociation but without FMA contraction.
pub(crate) fn vectorize_function_reassoc(func: &mut IrFunction) -> usize {
    vectorize_function_mode(func, true, FpContract::default(), true)
}

/// Preserve the established loop-vectorization policy while withholding the
/// fixed-width 256-bit SLP fold when the requested ISA has no AVX.
pub(crate) fn vectorize_function_reassoc_without_fixed_slp(func: &mut IrFunction) -> usize {
    vectorize_function_mode(func, true, FpContract::default(), false)
}

/// Run x86 vectorization with explicit reassociation and fast contraction.
pub(crate) fn vectorize_function_fast_math(func: &mut IrFunction) -> usize {
    vectorize_function_mode(func, true, FpContract::Fast, true)
}

/// Fast-math counterpart used for baseline x86-64 targets without AVX.
pub(crate) fn vectorize_function_fast_math_without_fixed_slp(func: &mut IrFunction) -> usize {
    vectorize_function_mode(func, true, FpContract::Fast, false)
}

fn vectorize_function_mode(
    func: &mut IrFunction,
    fp_reassoc: bool,
    fp_contract: FpContract,
    enable_fixed_slp: bool,
) -> usize {
    // A DynAlloca changes the live stack extent at run time. The current vector
    // loop/remainder builder assumes fixed alloca addresses when it rewrites
    // GEPs and carries scalar reductions, which can misaddress a VLA
    // (reproducer: tests/regression/vla_dynamic_stack.c). This is a scoped
    // legality check, not a global pass disable: functions without dynamic
    // stack allocation retain full vectorization.
    if func.blocks.iter().any(|block| {
        block
            .instructions
            .iter()
            .any(|inst| matches!(inst, Instruction::DynAlloca { .. }))
    }) {
        return 0;
    }

    // Volatile memory accesses are observable side effects: vectorizing a
    // loop that reads or writes volatile objects changes the number, width,
    // and order of memory accesses (C11 6.7.3p7: "Any attempt to refer to a
    // volatile object through a non-volatile lvalue is undefined behaviour"
    // — a vector load is exactly such an access). Bail out for any function
    // whose loop loads/stores chain to a volatile alloca.
    // (Reproducer: tests/correctness volatile_access — volatile int array
    // sum produced garbage at -O2 when vectorized.)
    if func_has_volatile_loop_access(func) {
        return 0;
    }

    let slp_changes = if fp_reassoc && enable_fixed_slp {
        transform_fixed_distance_slp(func)
    } else {
        0
    };
    if func.blocks.len() < 2 {
        if slp_changes > 0 {
            crate::passes::dce::eliminate_dead_code(func);
        }
        return slp_changes;
    }

    let cfg = CfgAnalysis::build(func);
    let loop_changes =
        vectorize_with_analysis_mode(func, &cfg, false, false, fp_reassoc, fp_contract);
    let changes = slp_changes + loop_changes;

    // The transforms replace the scalar accumulation with vector intrinsics but
    // leave the orphaned scalar load/mul/GEP/offset chain behind for the global
    // DCE pass — which runs AFTER IVSR in the pipeline. IVSR therefore
    // strength-reduces those already-dead GEPs into loop-carried pointer
    // increments (e.g. `leaq 256(%rdi), %rdi` every iteration) that the later
    // DCE cannot remove (the increment and the phi only reference each other).
    // Sweep the dead chain HERE, before IVSR runs, so no dead pointer
    // increments are ever materialized.
    if changes > 0 {
        crate::passes::dce::eliminate_dead_code(func);
    }
    changes
}

/// True if any Load/Store in `func` chains (through GEP/Cast/Copy) to an
/// alloca declared `volatile` or `semantic_volatile`. Such loops must never be
/// vectorized: vector loads/stores would observe or mutate volatile objects
/// with different width/count/order than the source program.
fn func_has_volatile_loop_access(func: &IrFunction) -> bool {
    use crate::ir::instruction::Instruction;

    let mut volatile_allocas: FxHashSet<u32> = FxHashSet::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Instruction::Alloca {
                dest,
                volatile,
                semantic_volatile,
                ..
            } = inst
            {
                if *volatile || *semantic_volatile {
                    volatile_allocas.insert(dest.0);
                }
            }
        }
    }
    if volatile_allocas.is_empty() {
        return false;
    }

    // Union-find style root map: pointer value -> alloca root it derives from.
    let mut parent: FxHashMap<u32, u32> = FxHashMap::default();
    fn find(parent: &mut FxHashMap<u32, u32>, v: u32) -> u32 {
        let mut root = v;
        while let Some(&p) = parent.get(&root) {
            if p == root {
                break;
            }
            root = p;
        }
        // Path compression.
        let mut cur = v;
        while let Some(&p) = parent.get(&cur) {
            if p == root {
                break;
            }
            parent.insert(cur, root);
            cur = p;
        }
        root
    }
    for block in &func.blocks {
        for inst in &block.instructions {
            let (dest, base): (Option<u32>, Option<u32>) = match inst {
                Instruction::GetElementPtr { dest, base, .. } => (Some(dest.0), Some(base.0)),
                Instruction::Cast { dest, src, .. } => {
                    if let Operand::Value(v) = src {
                        (Some(dest.0), Some(v.0))
                    } else {
                        (None, None)
                    }
                }
                Instruction::Copy { dest, src } => {
                    if let Operand::Value(v) = src {
                        (Some(dest.0), Some(v.0))
                    } else {
                        (None, None)
                    }
                }
                _ => (None, None),
            };
            if let (Some(d), Some(b)) = (dest, base) {
                let rb = find(&mut parent, b);
                parent.insert(d, rb);
                // The alloca itself is its own root.
                parent.entry(rb).or_insert(rb);
            }
        }
    }

    // Any Load/Store whose pointer chains to a volatile alloca disqualifies
    // the whole function from vectorization.
    for block in &func.blocks {
        for inst in &block.instructions {
            let ptr: Option<u32> = match inst {
                Instruction::Load { ptr, .. } | Instruction::Store { ptr, .. } => Some(ptr.0),
                _ => None,
            };
            if let Some(p) = ptr {
                let root = find(&mut parent, p);
                if volatile_allocas.contains(&root) {
                    return true;
                }
            }
        }
    }
    false
}

/// AArch64 NEON has 128-bit vectors, matching the two-wide F64 transform.
/// Passes neon=true so AArch64-only widening forms (sadalp, smlal/smlal2)
/// may be emitted; other backends reject those intrinsics.
pub(crate) fn vectorize_function_two_wide(func: &mut IrFunction) -> usize {
    let cfg = CfgAnalysis::build(func);
    vectorize_with_analysis_mode(func, &cfg, true, true, false, FpContract::default())
}

/// Late AArch64 vectorization (levkropp 8ef1978f concept): reduction loops
/// whose accumulator update only becomes Select-shaped after the main loop's
/// if_convert phase (max reductions, conditional sums). The early vectorizer
/// runs before if-conversion and rejects those loops as "conditional"; this
/// second pass catches the converted form. Runs the SAME analysis — patterns
/// already vectorized early contain Vec* intrinsics, which the analyzers
/// reject structurally (no scalar Load/BinOp/Select shape), so re-running is
/// idempotent and cannot double-transform.
pub(crate) fn vectorize_function_two_wide_late(func: &mut IrFunction) -> usize {
    let cfg = CfgAnalysis::build(func);
    vectorize_with_analysis_mode(func, &cfg, true, true, false, FpContract::default())
}

/// x86 late rerun (post-if_convert): catches Select-shaped conditional
/// reductions the early vectorizer could not see. AVX2 form; strict FP
/// contract (integer reductions need no contract).
pub(crate) fn vectorize_function_late(func: &mut IrFunction) -> usize {
    let cfg = CfgAnalysis::build(func);
    vectorize_with_analysis_mode(func, &cfg, false, false, false, FpContract::default())
}

pub(crate) fn vectorize_function_two_wide_fast_math(func: &mut IrFunction) -> usize {
    let cfg = CfgAnalysis::build(func);
    vectorize_with_analysis_mode(func, &cfg, true, true, true, FpContract::default())
}

#[cfg(test)]
mod fixed_distance_slp_tests {
    use super::*;

    fn make_f64x4(seg_override: AddressSpace, trailing_store: bool) -> IrFunction {
        let mut func = IrFunction::new("fixed_f64x4".into(), IrType::F64, vec![], false);
        let mut instructions = Vec::new();
        let mut next = 2u32;
        let mut fresh = || {
            let value = Value(next);
            next += 1;
            value
        };
        let mut terms = Vec::new();
        for lane in 0..4 {
            let a_ptr = fresh();
            instructions.push(Instruction::GetElementPtr {
                dest: a_ptr,
                base: Value(0),
                offset: Operand::Const(IrConst::I64(lane * 8)),
                ty: IrType::F64,
            });
            let a = fresh();
            instructions.push(Instruction::Load {
                volatile: false,
                dest: a,
                ptr: a_ptr,
                ty: IrType::F64,
                seg_override,
            });
            let b_ptr = fresh();
            instructions.push(Instruction::GetElementPtr {
                dest: b_ptr,
                base: Value(1),
                offset: Operand::Const(IrConst::I64(lane * 8)),
                ty: IrType::F64,
            });
            let b = fresh();
            instructions.push(Instruction::Load {
                volatile: false,
                dest: b,
                ptr: b_ptr,
                ty: IrType::F64,
                seg_override,
            });
            let delta = fresh();
            instructions.push(Instruction::BinOp {
                dest: delta,
                op: IrBinOp::Sub,
                lhs: Operand::Value(a),
                rhs: Operand::Value(b),
                ty: IrType::F64,
            });
            let square = fresh();
            instructions.push(Instruction::BinOp {
                dest: square,
                op: IrBinOp::Mul,
                lhs: Operand::Value(delta),
                rhs: Operand::Value(delta),
                ty: IrType::F64,
            });
            terms.push(square);
        }
        let lhs = fresh();
        instructions.push(Instruction::BinOp {
            dest: lhs,
            op: IrBinOp::Add,
            lhs: Operand::Value(terms[0]),
            rhs: Operand::Value(terms[1]),
            ty: IrType::F64,
        });
        let rhs = fresh();
        instructions.push(Instruction::BinOp {
            dest: rhs,
            op: IrBinOp::Add,
            lhs: Operand::Value(terms[2]),
            rhs: Operand::Value(terms[3]),
            ty: IrType::F64,
        });
        let root = fresh();
        instructions.push(Instruction::BinOp {
            dest: root,
            op: IrBinOp::Add,
            lhs: Operand::Value(lhs),
            rhs: Operand::Value(rhs),
            ty: IrType::F64,
        });
        if trailing_store {
            instructions.push(Instruction::Store {
                volatile: false,
                val: Operand::Const(IrConst::F64(0.0)),
                ptr: Value(0),
                ty: IrType::F64,
                seg_override: AddressSpace::Default,
            });
        }
        func.next_value_id = next;
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions,
            terminator: Terminator::Return(Some(Operand::Value(root))),
            source_spans: vec![],
        });
        func
    }

    #[test]
    fn fixed_distance_packs_default_address_space() {
        let mut func = make_f64x4(AddressSpace::Default, false);
        assert_eq!(transform_fixed_distance_slp(&mut func), 1);
        assert!(func.blocks[0].instructions.iter().any(|inst| matches!(
            inst,
            Instruction::Intrinsic {
                op: IntrinsicOp::FixedDistanceF64x4,
                ..
            }
        )));
    }

    #[test]
    fn fixed_distance_rejects_segment_address_space() {
        let mut func = make_f64x4(AddressSpace::SegGs, false);
        assert_eq!(transform_fixed_distance_slp(&mut func), 0);
    }

    #[test]
    fn fixed_distance_does_not_sink_loads_across_store() {
        let mut func = make_f64x4(AddressSpace::Default, true);
        assert_eq!(transform_fixed_distance_slp(&mut func), 0);
    }
}
