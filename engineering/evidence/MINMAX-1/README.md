# MINMAX-1 — integer min/max reduction vectorization on x86-64

**Status:** landed 2026-09-29 · gates `minmax-reduction` + `hot-loop-metric`
· evidence below is measured with the pinned Godbolt oracles and the local
LCCC, `-O3 -march=x86-64-v3` unless a table says otherwise.

---

## 1. The defect

```c
int max_i32(const int *a, int n) {
    int m = a[0];
    for (int i = 0; i < n; i++) if (a[i] > m) m = a[i];
    return m;
}
```

stayed **scalar** on x86-64 at every optimization level. Three independent
reasons, each sufficient on its own:

1. **The reduction does not exist when the vectorizer looks.** `if (a[i] > m)
   m = a[i];` is a branch diamond until `if_convert` runs, which happens
   *after* the main vectorizer. The pipeline therefore has a **late rerun**
   whose entire purpose is to catch Select-shaped reductions — and that rerun
   was gated on

   ```rust
   if matches!(target, crate::backend::Target::Aarch64) && ... {
   ```

   while the comment three lines above it promised *"AArch64 reruns the
   two-wide NEON pass; **x86-64 reruns the AVX2 pass**"*. The finished
   `VecMaxI32x8` (`vpmaxsd`) + `VecHorizontalMaxI32x8` machinery, and the
   whole late pass, were dead code on x86-64.

2. **`Min` did not exist.** `ReductionKind` had `Sum`, `DotProduct`, `Max`
   (NEON-only). No `Min`, no `VecHorizontalMinI32x8` intrinsic, no
   `vpminsd` reduction path.

3. **The detector rejected the ordinary loop shape.** The min/max detector
   required the array access to be a *marching-pointer phi* whose preheader
   value starts at element `iv_init`. `for (i = 0; i < n; i++)` over a
   pointer argument lowers to an **IV-indexed GEP**, which was rejected
   outright — i.e. the shape every real loop has.

## 2. The fix

| change | where |
|---|---|
| ONE dispatch table shared by the early and late vectorizer runs, opened to x86-64 | `src/passes/mod.rs` (`vectorize_entry`) |
| `ReductionKind::Min` + dual compare forms in the detector | `src/passes/vectorize.rs` |
| AVX2 opcode row: `VecMinI32x8` + `VecHorizontalMinI32x8` | `src/passes/vectorize.rs` |
| `vpminsd` lane op and the 3-fold horizontal min (`vextracti128` + two `vpshufd` lane permutes) | `src/backend/x86/codegen/intrinsics.rs` |
| `VecHorizontalMinI32x8` intrinsic + regalloc op classes | `src/ir/intrinsics.rs`, `src/backend/regalloc.rs` |
| SSE2/NEON path refuses `Min` (no horizontal min at 128 bits) — fail-closed, stays scalar | `src/passes/vectorize.rs` |
| IV-indexed GEP admitted when `iv_init == 0` | `src/passes/vectorize.rs` |

**Why `iv_init == 0` only.** The vectorizer rescales the loop's offset chain
by `vec_width`. For a marching pointer the pointer itself starts at element
`c`, so coverage is `[c, c + w·iters)` and the remainder resumes relative to
element `c` (`max_shift`). An IV-indexed GEP rescaled the same way starts at
element `c·w` — silently wrong for any `c ≠ 0`. At `c == 0` both the byte-IV
strength reduction and the scaled-GEP fallback start at element 0, so the
vector loop covers `[0, w·iters)` and the remainder resumes at `w·iters`:
nothing is skipped or re-read. `c ≠ 0` keeps the marching-pointer
requirement (`max_start1` in the shape fixture exercises it and vectorizes).

## 3. Measured effect

### 3.1 The target kernel, against every pinned oracle

Steady-state density (instructions retired per input byte in the innermost
non-composite loop — `scripts/hot_loop_metric.py`), `-O3 -march=x86-64-v3`:

| compiler | insns | bytes/trip | insn/byte | vector |
|---|---|---|---|---|
| **LCCC after** | **4** | 32 | **0.1250** | 256-bit `vpmaxsd` |
| LCCC after the min/max fix alone | 10 | 32 | 0.3125 | 256-bit `vpmaxsd` |
| LCCC before (scalar) | 6 | 4 | 1.5000 | scalar `cmov` |
| **clang 23.1** | 7 | 128 | **0.0547** | 256-bit, 4× unrolled |
| gcc 16.2 | 4 | 32 | 0.1250 | 256-bit |
| icx (latest) | 4 | 32 | 0.1250 | 256-bit |
| icc 2021.10 | 9 | 16 | 0.5625 | 128-bit |

A **12×** improvement on the kernel (scalar → packed), landing **level with
GCC 16.2 and ICX** and behind clang only because clang unrolls the packed
body 4× (7 insns / 128 B) — the one remaining gap, and the same unroll
follow-up the case-fold kernels already carry.

Static whole-function counts for the same source, for the record:
lccc 56, icx 33, clang 69, icc 70, gcc 16.2 73.

### 3.2 Shapes that vectorize

| shape | insn/byte |
|---|---|
| `max_i32`, `min_i32` | 0.1250 |
| operands swapped (`m < a[i]`) | 0.1250 |
| `<=` / `>=` spellings | 0.1250 |
| loop starting at `i = 1` (`c = 1`, marching pointer) | 0.1562 |
| constant non-zero seed (`INT_MIN + 1`) | 0.1250 |

### 3.3 Shapes that must stay scalar (pinned, contract 3 of the gate)

| shape | why |
|---|---|
| `unsigned` compare | `vpmaxsd`/`vpminsd` are signed-only (`vpmaxud` is SSE4.1) |
| `short` elements | no packed min/max at 16-bit lanes yet (MINMAX-3) |
| `float` elements | `min`/`max` is not bit-identical under NaN: needs fast-math, like every other FP reduction |
| `if (a[i] > 1000) continue;` | not every iteration contributes (needs masking) |
| a second accumulator in the loop | see §4 |

### 3.4 Corpus effect of opening the late vectorizer on x86-64

Same-binary A/B (`CCC_DISABLE_PASSES=latevec` = pre-change behaviour),
`-O3 -march=x86-64-v3`, all 51 benchmark programs:

| benchmark::function | before | after |
|---|---|---|
| `zlib_ng_adler32::main` | 0.4375 (14 / 32 B) | **0.3750 (12 / 32 B)** |
| `loop_patterns::main` | 0.1875 (6 / 32 B) | **0.1562 (5 / 32 B)** |
| `bitops::main` | 62 insns (scalar) | **59 insns** |

Everything else is byte-identical: the late rerun is idempotent on loops the
early pass already transformed (they contain `Vec*` intrinsics, which the
scalar-shape analyzers reject).

## 3.5 A second defect the measurement exposed: a `divq` in the hot loop

Once min/max vectorized, its steady-state body measured 10 instructions per
32 bytes against GCC's 4 — and six of the ten were this:

```asm
      vpmaxsd (%rbx,%r8), %ymm2, %ymm2     ; the actual work
      addq $32, %r8
      movq %r10, %rax                      ; n
      xorl %edx, %edx
      movl $8, %ecx
      divq %rcx                            ; n / 8 -- EVERY ITERATION
      movq %rax, %rdi
      shlq $5, %rdi                        ; * 32
      cmpq %rdi, %r8
      jl .LBB2
```

`transform_reduction_avx2`/`sse2` divide a dynamic loop bound by the vector
width by inserting a `UDiv` **into the loop header**. LICM runs before the
vectorizer (and the late rerun runs after it), so nothing hoisted it again.
Sum reductions happened to be fine — their limit arithmetic got
strength-reduced to `shrl`/`shll` and hoisted — but the min/max path reaches
this code with an I64 limit and materialised a real 64-bit `divq` in the
steady state: 0.3125 insn/byte against GCC's 0.1250.

Fixed in both transforms: the division (and the byte-stride scaling that goes
with it) is now emitted **once in the preheader**, and `UDiv` by a power-of-two
width becomes `LShr` (exact for the unsigned division `UDiv` denotes; every
vector width is a power of two), with a header fallback when the loop has no
single preheader — correct either way, only slower. Result: 10 → 4
instructions per trip, and LCCC now matches GCC and ICX exactly.

## 3.6 Why the late rerun is scoped to min/max (measured, not assumed)

The unrestricted rerun is a **net loss** on the one kernel where it does more
than min/max. `zlib_ng_adler32`, `-O3 -march=x86-64-v3`,
`valgrind --tool=callgrind`, whole program, identical stdout in both arms:

| | retired instructions (Ir) | static insns | stack refs |
|---|---|---|---|
| late rerun off (pre-change) | 396,349,832 | 335 | 32 |
| unrestricted late rerun | **400,499,785 (+1.05 %)** | **370 (+10.4 %)** | **44 (+37.5 %)** |
| late rerun scoped to min/max (shipped) | 396,339,981 | **332** | 32 |

…while the *steady-state* loop of that same kernel is denser with the extra
transform (14 -> 12 insns per 32 bytes). Denser hot loop, slower program: the
prologue, the accumulator traffic and the spills are not paid back on this
workload. `set_late_minmax_only` therefore admits only
`ReductionKind::{Min, Max}` in the rerun (matmul, strict-recip, byte-count,
Adler epic, map and stencil arms are all skipped), which keeps the min/max win
and leaves every other kernel byte-identical — and adler32 ends up *better*
than before (332 static instructions, 0.3750 insn/byte from the preheader
fix). Recorded as MINMAX-4: re-open with a profitability guard.

## 4. The miscompile this produced, and the refusal that fixes it

Opening the late pass made `mnmax_kernel` — min, max **and** sum in one loop
— return `sum == 0` for **every n below the vector width**, at every
optimization level, on both baselines. Cause: the min/max pattern models
exactly ONE accumulator (`seconds` is empty and the secondary-accumulator
emitter is `unreachable!` for `Max`), so the transform rewrote the loop with
the sum accumulator unmodelled.

The detector now refuses any loop whose header carries a second phi whose
**latch** incoming is computed inside the loop body — the signature of an
accumulator this pattern does not model. The preheader incoming is not
examined: that edge carries the seed (a constant or a pre-loop load), which
identifies nothing.

This is a *capability gap with a guard rail*, not a fix, and it is recorded
as **MINMAX-2** in `backlog.md`: `moving_stats` (sum + min + max over a
`short` window) is the corpus's clearest victim.

## 5. Gates

* `tests/regression/check_minmax_reduction.sh`
  * **contract 1** — randomized differential correctness vs the GCC oracle:
    every length 1..80 (both sides of every vector-width and remainder
    boundary) × 4 input distributions (narrow, full-range, `INT_MIN`/`INT_MAX`
    only, heavy duplicates) × interior and tail extremes, plus large lengths.
    Run two ways: whole program by LCCC, and **LCCC kernels + oracle
    harness**, so a miscompile cannot hide in harness-side codegen.
  * **contract 2** — `scripts/check_minmax_shapes.py`: the 7 admitted shapes
    must measure as packed 256-bit steady-state loops; the 7 refused shapes
    must measure as scalar. A capability increase moves a row on purpose.
  * **contract 3** — the refused shapes, driven by
    `tests/regression/minmax_refused_main.c`, must agree with the oracle at
    `-O1/-O2/-O3` × `x86-64`/`x86-64-v3`.
  * **contract 4** — i686 (no AVX2 min/max) agrees with the oracle.
* `scripts/test_hot_loop_metric.py` — nine known-answer cases for the
  measurement instrument used above (see §6).

## 6. The instrument had to be fixed first (and it flattered LCCC)

`scripts/hot_loop_metric.py` is the tool this project's own policy
(`engineering/STATE.md`) names for ranking loop kernels. It had two defects,
both of which made LCCC look *better* than it is — i.e. exactly the wrong
direction for a tool that decides where to spend effort:

1. **"Backedge" meant "branch to a textually earlier label."** The remainder
   guard's `jge .LBB7` in a vectorized LCCC kernel jumps backwards in the
   listing but forward in the CFG. The resulting "loop" *contains* the real
   inner loop, so its static instruction count was charged against a trip
   that executes the inner loop `VF` times: `matmul` reported **0.0117**
   insn/byte (24 insns / 2048 B) where the true steady-state cost is
   **0.1172** (15 insns / 128 B) — and the LCCC-vs-GCC verdict inverted.
2. **Step recovery only recognised a pointer advance on a register that
   appears in a memory operand.** LCCC's packed reductions advance a separate
   index (`addl $128, %r8d`, with `movslq %r8d, %r9` for the address), so the
   packed loop measured as "step unknown", was skipped, and the **scalar
   remainder** was reported instead — a vectorized kernel recorded as
   scalar.

Both are fixed with a real CFG: basic blocks, iterative dominators, natural
loops (`backedge = edge b → h` where `h` dominates `b`), nesting
classification, `composite` loops carrying **no** density at all (a loop that
encloses another loop has no static bytes-per-trip), and address-advance
recovery that follows one level of `movslq`/`lea` scaling.

The corrected `matmul` comparison (captured in `tests/asm/hot-loop/`):

| compiler | insns | bytes/trip | insn/byte |
|---|---|---|---|
| **LCCC** | 15 | 128 | **0.1172** |
| GCC 16.2 | 6 | 32 | 0.1875 |

… where `codegen_oracle.py --rank` reports the same function as *"63
instructions behind GCC"*. Static ranking rewards refusing to vectorize, and
the corpus is mostly loops.

## 7. Not done

* MINMAX-2 (multi-accumulator), MINMAX-3 (16-bit / unsigned lanes),
  RED-WIDEN-1 (widening reductions: `int s; s += (short)a[i]`) — all in
  `backlog.md`, all measured there.
* LCCC's packed min/max loop is not unrolled (4 insns / 32 B vs clang's
  7 / 128 B, i.e. 0.1250 vs 0.0547). Unrolling the packed body is the single
  remaining gap on this kernel and is the same follow-up the case-fold
  kernels already carry.
* The preheader-division fix touched every dynamic-limit reduction in both
  the AVX2 and the SSE2 transform. The corpus A/B shows no kernel getting
  worse and none of the existing sum/dot loops changing (their limit
  arithmetic was already hoisted); the shapes that gain are the ones that
  used to divide inside the loop.
