---
layout: doc
title: Optimization Passes
description: The CCC/LCCC optimizer pass pipeline — what runs, in what order, and why.
prev_page:
  title: Register Allocator
  url: /docs/register-allocator
next_page:
  title: Benchmarks
  url: /docs/benchmarks
---

# Optimization Passes
{:.doc-subtitle}
LCCC inherits CCC's optimizer. **Tiers are real** (authoritative: `src/passes/README.md`):

- `-O0`: skip the pipeline except mandatory inline-asm symbol resolution
- `-O1`: mem2reg, constant folding, copy-prop, DCE
- `-O2`: full scalar/IPO pipeline
- `-O3`: `-O2` plus size-increasing unroll
- `-Os` / `-Oz`: size; `-Oz` also disables inlining

Older text that claimed “all `-O` levels run the same pipeline” is **obsolete**.

The `-O` flags still set `__OPTIMIZE__` / `__OPTIMIZE_SIZE__` for kernel-style `#ifdef`s.

## Passes added since this page was written (August 2026)

This page predates several production passes; `src/passes/README.md` and
`src/passes/mod.rs` are authoritative. Landed since:

- **DSE** (`dse.rs`) — same-block dead-store elimination with closed-alloca
  escape analysis and byte-range kills (`CCC_NO_DSE`).
- **Backedge PRE** — integer recurrences default-on (1.14× measured);
  FP variants gated (`CCC_BEPRE_FP=1` research).
- **GlobalAddr CSE** (`global_addr_cse.rs`) — oracle-derived placement:
  cold singletons branch-local, loop addresses to the innermost preheader,
  dominating defs reused, derived variable-index bases site-local.
- **Stencil vectorizer** — constant-tap affine loops, bit-exact vs scalar
  (`CCC_NO_STENCIL_VEC`).
- **Map expression trees** — elementwise FP/int map loops (`CCC_NO_MAP_VEC`).
- **Widening + masked conditional-sum reductions** — I32→I64 `paddq`
  pipelines, VEX-only bodies (9× AVX-SSE transition penalty avoided).
- **Loop rotation** (`loop_rotate.rs`) — opt-in (`CCC_LOOP_ROTATE=1`),
  runs after vectorize; default-enable pending hardening.
- **General complete unrolling** — nested/multi-block constant-trip loops
  with FP-aware expansion budget.
- **Tri-state FP contraction** — `FpContract { Off, OnExpr, Fast }`,
  default Off (GCC `gnu*` parity); FMA emission requires the FMA3 feature.

## Pass Order

The optimizer runs up to three full iterations. Each iteration runs this pipeline:

```
1.  CFG simplification      — remove dead blocks, thread jump chains, simplify branches
2.  Copy propagation        — replace uses of copies with original values
2a. Division-by-constant    — replace idiv/div with multiply-shift (iter 0 only)
2b. Integer narrowing       — shrink operation widths (e.g. i64 add → i32 add)
3.  Algebraic simplification — strength reduction, identity removal
4.  Constant folding        — evaluate constant expressions at compile time
5.  GVN + LICM + IVSR       — shared CFG analysis (computed once per function):
    ├── GVN                 — global value numbering / CSE across dominated blocks
    ├── LICM                — loop-invariant code motion
    └── IVSR                — induction variable strength reduction
7.  If-conversion           — convert branch+phi diamonds to cmov/Select
8.  Copy propagation        — clean up copies created by GVN, simplify, LICM
9.  Dead code elimination   — remove dead instructions (excluded from convergence check)
10. CFG simplification      — clean up after DCE empties blocks
10.5 IPCP                   — interprocedural constant propagation (all iterations)
```

**Phase 0** (before the loop):
- Tail-call elimination (TCE) — converts self-recursive tail calls to loops
- Function inlining (including the **post-structural** inlining round after
  TCE/rec2iter)
- `recursion_to_iter` (rec2iter) — non-tail binary recursion to iterative
  accumulators
- mem2reg
- Initial constant-fold/copy-prop round
- **Vectorization** (iteration 0 only) — transforms reduction loops to AVX2/SSE2 SIMD

**Phase 11** (after the loop):
- dead static function elimination — removes `static inline` functions that
  became unreferenced after inlining
- **Outline switch** — large cold switches lowered out of line when beneficial

Additional passes in the pipeline (see `src/passes/`): `resolve_asm` (inline-asm
symbol resolution), `vector_temp_promotion` (SIMD temp slot promotion), and the
standalone `univsr` (universal induction-variable strength reduction) pass that
runs under explicit control.

**PGO** (`src/pgo/`) wraps the optimizer: with `-fprofile-generate` it
instruments the post-optimization IR; with `-fprofile-use` it drives inlining,
loop unrolling, block/function layout, switch lowering, and indirect-call
devirtualization from a runtime profile. See the PGO section below.

## Convergence

Iterations stop when:
- No pass made any change (fixpoint reached), **or**
- This iteration made fewer than 1/20th the changes of the first iteration (diminishing returns).

DCE changes are excluded from the convergence check because DCE's large removal counts inflate the first-iteration baseline and cause premature exit.

## Pass Interactions

The passes form a dependency graph. LCCC uses a `should_run!` macro to skip passes when their upstream passes made no changes in the previous iteration:

| Pass | Only re-runs if... |
|------|-------------------|
| CFG simplify | constfold changed (constant branches) or DCE changed (empty blocks) |
| Copy prop | CFG, GVN, LICM, or if-convert changed |
| Simplify | Copy prop or narrowing changed |
| Constfold | Copy prop, narrowing, simplify, or if-convert changed |
| GVN | CFG, copy prop, or simplify changed |
| LICM | CFG, copy prop, or GVN changed |
| DCE | GVN, LICM, if-convert, or copy prop2 changed |

## Key Passes

### GVN (Global Value Numbering)

Eliminates redundant computations across basic blocks. If two blocks both compute `a + b` where `a` and `b` have the same value, GVN replaces the second with a copy of the first. Operates on `BinOp`, `UnaryOp`, `Cmp`, `Cast`, `GetElementPtr`, and `Load` instructions within the dominator tree.

GVN, LICM, and IVSR share a single `CfgAnalysis` (dominator tree + loop nesting) per function per iteration, saving significant compile time on large translation units.

### LICM (Loop-Invariant Code Motion)

Hoists computations out of loops when their operands don't change within the loop. Identifies natural loops via the dominator tree, then moves invariant instructions to loop preheaders. Critical for inner-loop performance on code like matrix multiply.

### IPCP (Interprocedural Constant Propagation)

When a function is always called with the same constant argument, IPCP specializes the function for that constant and folds the resulting dead branches. Important for Linux kernel code (`IS_ENABLED()`, `cpucap_is_possible()` chains) where `static inline` wrappers gate large blocks of dead code.

### Division-by-Constant

Replaces `idiv`/`div` instructions (20–90 cycle latency on modern CPUs) with multiply-and-shift sequences. Runs only on the first iteration, before narrowing and constant folding can further simplify the expanded sequence. Disabled on i686 where 64-bit multiply overflow semantics differ.

## Disabling Passes

For debugging, individual passes can be disabled:

```bash
CCC_DISABLE_PASSES="gvn,licm" ./target/fastbuild/lccc input.c -o output
CCC_DISABLE_PASSES="all"      ./target/fastbuild/lccc input.c -o output
```

Pass names: `cfg`, `copyprop`, `narrow`, `simplify`, `constfold`, `gvn`, `licm`, `ifconv`, `dce`, `ipcp`, `inline`, `ivsr`, `divconst`.

Timing data is available via:

```bash
CCC_TIME_PASSES=1 ./target/fastbuild/lccc input.c -o output 2>&1 | grep PASS
```

## LCCC-Specific Passes

LCCC adds two optimization passes that run before CCC's main optimizer loop.

### Tail-Call Elimination (`tce`)

Converts self-recursive tail calls to back-edge branches. A tail call is a recursive call whose result is returned immediately — `return f(args)` with no further computation.

```c
// Before: 10M stack frames
long sum(int n, long acc) {
    if (n <= 0) return acc;
    return sum(n - 1, acc + n);
}

// After TCE: tight counted loop (identical to GCC output)
long sum(int n, long acc) {
loop:
    if (n <= 0) return acc;
    acc += n; n -= 1; goto loop;
}
```

TCE runs once after inlining, before the main optimization loop, so that LICM, IVSR, and GVN can subsequently optimize the resulting loop.

**Pass name:** `tce` (disable with `CCC_DISABLE_PASSES=tce`)

**Implementation:** [`src/passes/tail_call_elim.rs`](https://github.com/ms178/lccc/blob/main/src/passes/tail_call_elim.rs)

### Phi-Copy Stack Slot Coalescing (backend)

This is a backend optimization in `src/backend/stack_layout/copy_coalescing.rs`, not a pass in the traditional sense. It runs during stack layout, before code generation.

When CCC's phi elimination lowers SSA phi nodes to Copy instructions, it creates separate stack slots for the phi destination and its backedge update value. For a 32-variable loop, this generates ~20 redundant stack-to-stack `movq` pairs per iteration.

LCCC detects the phi-copy pattern — where the source is defined and killed in the backedge block — and aliases the source to use the phi destination's wider-live slot. The Copy becomes a same-slot no-op and is dropped by `generate_copy`.

**Result:** `arith_loop` (32 variables): 550 → 507 assembly lines; 0.124s → 0.104s.

## LCCC-Specific: Reduction Vectorization

**Added in Phase 8** — LCCC detects and transforms reduction loops into AVX2/SSE2 SIMD operations.

### What Gets Vectorized

Simple reduction patterns:
```c
// Sum reduction
double sum = 0.0;
for (int i = 0; i < n; i++)
    sum += arr[i];

// Dot product
double dot = 0.0;
for (int i = 0; i < n; i++)
    dot += a[i] * b[i];
```

### Transformation Strategy

1. **Pattern detection**: Identifies loops with a scalar accumulator PHI and a single reduction operation
2. **Loop splitting**: Divides loop bound by vector width (4 for AVX2, 2 for SSE2)
3. **Vector body**: Replaces scalar ops with vector intrinsics (VecLoad, VecAdd, VecMul)
4. **Horizontal reduction**: Extracts scalar from final vector (`vextractf128` + `vunpckhpd` + `vaddsd`)
5. **Remainder loop**: Handles `N % vec_width != 0` with scalar operations
6. **Correct return**: Exit block returns scalar from remainder loop, not vector from main loop

### Backend Implementation

Vector values are treated as first-class SSA values that:
- Get unique, never-reused stack slots (protected from slot recycler)
- Are excluded from GPR allocation (forced to stack)
- Use direct slot access in intrinsics (no pointer indirection)
- Support vector-to-vector Copy via ymm/xmm registers

### Generated Code (AVX2 Example)

```asm
vxorpd %ymm0, %ymm0, %ymm0          # Zero vector accumulator
.loop:
    vmovupd (%rax,%rcx), %ymm0      # Load 4 doubles
    vaddpd %ymm1, %ymm0, %ymm0      # Add 4 doubles
    ; loop back...

; Horizontal reduction
vextractf128 $1, %ymm0, %xmm1       # Extract high 128 bits
vaddpd %xmm1, %xmm0, %xmm0          # Add high + low (2 doubles each)
vunpckhpd %xmm0, %xmm0, %xmm1       # Unpack high double
vaddsd %xmm1, %xmm0, %xmm0          # Final scalar

; Remainder loop (scalar)
.remainder:
    movsd (%rbx,%r13,8), %xmm0      # Load single element
    addsd -24(%rbp), %xmm0          # Add to scalar accumulator
    ; loop back...

; Return scalar result
movsd -24(%rbp), %xmm0              # Return scalar (not vector!)
```

### Why It Beats GCC

GCC's auto-vectorizer is conservative on simple reductions:
- Worries about aliasing even with clear array indexing
- Considers the pattern "too simple" to benefit
- Falls back to 2× scalar loop unrolling

LCCC's pattern-based approach:
- Explicitly targets common reduction idioms
- Aggressively transforms when pattern matches
- Generates complete vectorization (4× for AVX2 vs GCC's 2× unroll)

**Result**: LCCC vectorizes patterns GCC -O3 leaves scalar, achieving ~2.7× speedup.

### Debug Flags

```bash
LCCC_DEBUG_VECTORIZE=1    # Show vectorization transformations
LCCC_DEBUG_PROTECT=1      # Show stack slot protection decisions
```

---

## PGO (Profile-Guided Optimization)

PGO is implemented in `src/pgo/` and wraps the pass pipeline. It uses the
GCC-compatible flag surface: `-fprofile-generate[=dir]`,
`-fprofile-use[=dir]`, `-fbranch-probabilities`, `-fauto-profile`, and
`-fprofile-update=atomic|single`.

### Profile generation (instrumentation)

`instrument.rs` instruments the *post-optimization* IR so no optimization pass
ever sees a counter. For each function it:

1. builds the CFG (with a virtual EXIT node for `return`-terminated blocks);
2. chooses the edges to count with a **maximum-weight directed arborescence**
   (backedges and cycle edges stay off the counter path; every non-entry node,
   including loop latches, keeps exactly one incoming tree edge);
3. emits a single `incq sym+off(%rip)` per counted edge (or a `lock`-prefixed
   increment under `-fprofile-update=atomic`), splitting critical edges so a
   counter never sits between a fused `cmp` and its branch;
4. instruments **indirect-call value profiling** (top-4 callee slots per site).

At runtime the counters are dumped to `lccc-<unit>-<pid>.profraw` files (or the
`LCCC_PROFILE_FILE` / `LLVM_PROFILE_FILE` override) and merged deterministically.

### Profile use

`profile.rs` loads the `.profraw` text format (multi-file merge, version-tolerant
parsing, per-file corruption tolerance) and `derive_block_counts` recovers every
block count and tree-edge count from the instrumented edges by **flow
conservation**.

**Identity & drift:** profiles are keyed by a PRE-pass CFG fingerprint (stable
across generate/use). A POST-pass fingerprint detects CFG drift from PGO-guided
transforms; drifted functions keep their stable entry count for hot/cold
*sections* but get **no** stale edge-derived data (fail-closed).

**Profile summary** (`summary.rs`): an LLVM `ProfileSummaryInfo` analogue. The
hot/cold thresholds come from the count distribution, and `has_spread()` tests
whether one function uniquely dominates the runner-up — the gate that keeps a
**flat** profile from perturbing the hot path.

### Consumers

- **Inlining** (`inline_pgo.rs`): percentile hot/cold classification,
  label-independent entry-count-ratio force-inline for hot loop sites, bounded
  force-inline budget, and a working hotness threshold multiplier.
- **Loop unrolling** (`unroll_pgo.rs`): derived trip count (`backedge / entry`)
  drives whether hot high-trip loops unroll and cold low-trip loops do not.
- **Layout** (`layout.rs`): hot/cold function sections, profile-driven switch
  lowering (dominant case hoisted out of a jump table; cold switches forced to a
  compare chain), switch-case ordering, and conservative
  (preserve-source-order) block layout that never perturbs register allocation.
- **Devirtualization** (`promote.rs`): cost-aware indirect-call promotion — a
  site whose top target is ≥95% of calls is already BTB-predicted and left
  indirect; genuinely multi-valued sites are promoted to a guarded direct call.
- **Backend branch inversion**: the hot successor of a `CondBranch` is made the
  physical fall-through so the hot path takes no branch, without reordering
  blocks.

### Debug / tuning knobs

```bash
CCC_INLINE_DEBUG=1          # explain inliner decisions
LCCC_DEBUG_LAYOUT=1         # layout decisions
LCCC_DEBUG_PROMOTE=1        # devirtualization decisions
LCCC_PGO_NO_LAYOUT=1        # disable PGO layout
LCCC_PGO_PROMOTE_STABLE=95  # top-share % treated as "already predicted"
LCCC_PGO_HOT_FRAC / LCCC_PGO_COLD_FRAC  # summary percentiles
```

The PGO A/B harness — `tests/benchmark/run_pgo_ab.py` — builds plain and
profile-guided binaries for lccc and a reference compiler, verifies differential
output equivalence, and reports paired bootstrap-CI speedups.
