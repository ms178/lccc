# LCCC — Lightning Fast Claude's C Compiler

> An optimized fork of [CCC](https://github.com/anthropics/claudes-c-compiler) based on additions made by Lev Kropp (https://www.levkropp.com/lccc/).

---

## What is LCCC?

CCC (Claude's C Compiler) is a zero-dependency C compiler written entirely in Rust by Claude Opus 4.6. Arena.ai Agents
did a good job at improving it further.

It is capable of compiling real projects — gzip, zlib-ng, expat, SQLite, the Linux kernel and glibc — for x86-64, AArch64,
RISC-V 64, and i686, with its own assembler and linker.

LCCC is a performance fork and a personal AI agent research project. It is currently still lacking behind GCC/Clang in many areas.

---

## Licensing

LCCC uses a dual-license model to separate original contributions from CCC-derived code.

**LCCC contributions** (new files, regalloc changes, benchmarks, docs) —
MIT OR Apache-2.0 OR BSD-2-Clause (your choice). See `LICENSE-MIT`, `LICENSE-APACHE`, `LICENSE-BSD`.

**CCC-derived code** (frontend, SSA IR, optimizer, backends, assembler, linker) —
CC0 1.0 Universal (public domain). CCC was released as CC0 by Anthropic.

**Workload-derived benchmark kernels** — retain the per-file upstream license
and provenance (for example GPL/LGPL/MIT/Zlib/public-domain material); they
are documented in [`tests/benchmark/WORKLOAD_PROVENANCE.md`](tests/benchmark/WORKLOAD_PROVENANCE.md).

See [`LICENSING.md`](LICENSING.md) for the full breakdown and per-file guidance.

## Engineering docs

All live performance/RA/codegen research lives in [`engineering/`](engineering/README.md).
Start at [`engineering/agent/README.md`](engineering/agent/README.md).

## Current documentation

Live docs (August 2026): `docs/getting-started.md`, `docs/architecture.md`,
`docs/register-allocator.md`, `lccc-improvements/register-allocation/`
(including `VALIDATION_ZLIB_GZIP_EXPAT.md`). `docs/history/` and
`lccc-improvements/benchmarks/BENCHMARK_RESULTS*.md` are archives.
Canonical bench: `tests/benchmark/run_benchmarks.py`. Repo:
https://github.com/ms178/lccc


# LCCC Performance Report

> **Screening evidence.** Wall-clock, best-of-3, checksums verified against a
> GCC baseline byte-for-byte. Measured in a shared 2-core VM (no PMU); numbers
> rank code-generation work and are *not* microarchitectural claims. Reproduce
> on bare metal before treating any figure as a hardware result.

## Compared revisions

| Compiler | Revision | Notes |
|---|---|---|
| **LCCC (ms178)** | `main` `9ab0d34` + sessions 73–78 (this tree) | segment-aware regalloc, general complete unrolling (nested const-trip loops), VEX 3-operand scalar-FP exploitation, widening I32→I64 reduction vectorization, conditional-sum if-conversion (address-canonicalized arm-load coverage), FMA3 ISA gate, map expression trees, exact peephole liveness |
| **GCC** | 14.2.0 (Debian 14.2.0-19) | external reference |

All compilers at `-O2`, identical sources, identical machine, same run window.
Driver: `tests/benchmark/run_benchmarks.py` (canonical runner): paired median
wall time, randomized compiler order, warm-ups excluded; every binary's
output is checksummed against the GCC baseline (a mismatch disqualifies the
number). Measured 2026-08-24 in the shared 2-core VM (no PMU); screening
evidence only — reproduce on bare metal before treating any figure as a
hardware result.

## Results (ratio = LCCC/GCC; < 1 means LCCC faster)

| Kernel | What it stresses | LCCC/GCC | Verdict |
|---|---|---:|---|
| `fib` | binary recursion → rec2iter | **0.009** (109× faster) | pass |
| `ackermann` | deep recursion | **0.022** (45× faster) | pass |
| `bitops` | integer selection, popcount idioms | **0.603** (1.7× faster) | pass |
| `gzip_crc32` | gzip CRC-32 table loop | **0.862** (1.16× faster) | pass |
| `matmul` | loop-nest FP + reduction FMA | **0.46** (2.17× faster) | pass |
| `hash_table` | pointer chasing | **0.92** | pass |
| `binary_trees` | allocation + recursion | **0.92** | pass |
| `glibc_memcmp` | aligned-word memcmp path | **1.00** | pass |
| `binary_search` | branch-heavy lookup | **1.00** | pass |
| `qsort` | libc branches | **0.97** (1.03× faster) | pass |
| `double_reduction` | two independent FP reductions | **1.00** | pass |
| `fp_memfold_stencil5` | FP stencil memory folding | **0.88** | pass |
| `reduction_vecreg` | register-resident FP reductions | **0.96** | pass |
| `arith_loop` | 32-variable register pressure | **1.242** (1.24× slower) | pass |
| `histogram` | indexed increment/reduction | **0.80** (1.25× faster) | pass |
| `mandelbrot` | FP branch-heavy loop | **1.234** (1.23× slower) | pass |
| `sieve` | branchy int stores | **1.258** (1.26× slower) | pass |
| `expat_xml_scan` | XML name-token scan | **1.686** (1.69× slower) | pass |
| `sqlite_varint` | varint decoder | **1.360** (1.36× slower) | pass |
| `linux_find_bit` | sparse bit search | **0.70** (1.42× slower) | pass |
| `fannkuch` | permutations | **1.391** (1.39× slower) | pass |
| `spectral_norm` | dense FP | **1.301** (1.30× slower) | pass |
| `hash_table` (chase) | pointer chasing | **0.92** | pass |
| `libm_round_family` | libm round intrinsics | **0.411** (2.43× faster) | pass |
| `loop_patterns` | scalar loop transforms | **0.979** (1.02× faster) | pass |
| `nbody` | N-body FP structs | **1.262** (1.26× slower) | pass |
| `tce_sum` | tail-recursive accumulator | **0.961** (1.04× faster) | pass |

**Aggregate: geometric mean ~0.85 (27 pairs, -O2, 5–9-round paired medians,
2026-08-25).** LCCC beats GCC on `fib` (109×), `ackermann` (45×),
`libm_round_family` (2.43×), `loop_patterns` (1.02×), `tce_sum` (1.04×),
and matches on `glibc_memcmp`/`binary_search`/`double_reduction` (1.00×).
The conventional-code geomean (excluding `fib`/`ackermann`) is ~0.95.
Remaining structural gaps (root-caused, tracked for v13): the dominant
systemic gap is **loop rotation** — every benchmark's inner loop pays a
double-jump preheader (`cmp; jge; .Lbody; ...; jmp .Lhead`) that GCC
folds into a single fall-through test-and-branch, costing ~1 branch/iter
across the whole suite. expat_xml_scan (1.69×) is the worst-case: the
FNV-prime IS register-homed (%r14), but the byte-at-a-time XML state
machine pays the rotation penalty plus verbose byte-classification vs
GCC's case-folded `andl $-33; subl $65; cmp $25`. nbody/mandelbrot/spectral
(1.23–1.30×) are FP loops where the rotation penalty compounds with
per-iter pointer arithmetic. sqlite_varint (1.36×) and fannkuch (1.39×)
are serial-state-machine / permutation loops (rotation + branch quality).

### v12 highlights (session 82)

Reconstructs the v11 session's attempted-but-unmerged fixes properly, plus
the widening-accumulator coalescing that makes I64x2 reduction accumulators
register-resident. Five validated fixes + find_max infrastructure:

- **LCG loop tightening (RA precise-span seed)**: `run_with_seed` now
  accepts precise `[start, end)` occupancy spans per register instead of a
  fat `[0, until)` seed. Late call args (printf at function end) no longer
  block caller-saved registers for the entire function, so early loop
  values (LCG seed, IV) get register homes. loop_patterns: **1.294 →
  0.979** (now beats GCC).
- **Fused mul-add constant staging**: 3-operand `imull $imm, %lhs, %eax`
  when lhs is register-homed (drops the leading `movl %lhs, %eax`); `addl
  $imm, %eax` when acc is a constant (drops the 7-byte `movq $imm`
  staging). LCG loop drops 2 instructions + 7 bytes/iteration.
- **Widening reduction accumulator coalescing**: `VecWidenAddI32x4ToI64x2`
  and `VecWidenMaskedAddI32x4ToI64x2` added to `class_of` (class 7 / I64x2)
  and as `legal_consumer` for class 7; `VecBroadcastI32x8`/`VecStoreI32x8`
  exempted from the early-return poison. The Copy-web propagation now
  connects the backedge Copy (dest=phi_acc, src=widen_dest) so the
  accumulator stays classified and reaches the XMM allocator —
  register-homed, no stack round-trip. The widening lowerings were
  restructured (Fix D) to confine scratch to xmm0/xmm1 with in-place
  shuffles so the accumulator's XMM home is never clobbered.
- **Byte-write miscompile fix**: isel Copy lowering now emits
  `movzbl`/`movzwl` for narrow (S8/S16) Copies to registers, matching the
  cast path's "no stale upper bits" principle. A plain `movb $1, %dil` left
  the upper 24 bits of %edi stale (a leftover printf format-string
  pointer); the consumer `movslq %edi` sign-extended garbage. Fixed the
  `flat_short_circuit` regression exposed by the RA reassignment.
- **find_max AVX2 transform wiring (infrastructure landed, detection
  gated)**: the full AVX2 Max transform (broadcast init, `vpmaxsd` lane
  max, dedicated base-matching stride scaler 4→32, `VecHorizontalMaxI32x8`
  reduce) is wired and correct (output matches GCC for 10M-element
  find_max). `VecMaxI32x8`/`VecBroadcastI32x8` added to class 5,
  `VecMaxI32x8`+`VecHorizontalMaxI32x8` as legal consumers, `VecMaxI32x8`
  added to `is_two_operand_binary` so the VecLoad dest is deferred (no
  dead 256-bit store). Detection remains gated on `neon` pending AVX2
  cost-model tuning — the init+reduce overhead exceeds the 8× lane speedup
  for the 10M working set; removing the gate is a one-line v13 change.

### v13 highlights (session 82, continued)

The v12 follow-up work identified **loop rotation** as the systemic gap
affecting every benchmark (the double-jump `cmp; jge; .Lbody; ...;
jmp .Lhead` form costs ~1 branch/iter vs GCC's rotated test-and-branch).
v13 lands the loop-rotation infrastructure and hardens it to correctness
for the canonical single-block body+latch counted-loop form, but ships
it **opt-in** (`CCC_LOOP_ROTATE=1`) because the transform still
miscompiles 24/486 regression tests (multi-exit, nested-loop, and
header-phi-escapes-through-non-Return-terminator shapes). The
infrastructure is in place for a v14 hardening pass.

- **Loop rotation pass** (`src/passes/loop_rotate.rs`, ~530 LOC): a new
  IR-level pass that transforms guard-at-top loops into test-at-bottom
  self-loops. For each single-latch, single-block-body+latch loop with a
  pure-SSA guard cond, it (1) clones the guard cond to the latch with
  phi references rewritten to the post-increment IV value, (2) creates a
  fresh self-loop phi in the body for each header phi (the new IV), (3)
  creates an exit-block merge phi for each header phi with external uses
  (so the accumulator's final value reaches the `Return`/downstream
  users), (4) strips the header phi's stale latch incoming (cfg_simplify
  then collapses it to the preheader value). Runs at -O2+ before
  loop_unroll. Gated `CCC_LOOP_ROTATE=1` (default off) +
  `CCC_NO_LOOP_ROTATE=1` kill-switch. Verified correct on `sum_arr` (tiny
  + 10M-element loop_patterns: output bit-identical to GCC) and emits the
  canonical rotated form (`.Lbody: ...; cmp; jne .Lbody` self-loop).
- **phi-elimination self-loop copy placement** (explored, reverted): the
  self-loop's phi-elim copy must go at the END of the block (before the
  backedge), not in a trampoline — otherwise the trampoline splits the
  self-loop back into a 2-block body+latch with an unconditional backedge,
  reverting the rotation. A fix to `place_copy`/`place_copies` was
  implemented and verified correct on loop_patterns, but it regressed
  `sqlite_varint` (a pre-existing self-loop relied on the trampoline
  split), so it was reverted. v14 will re-land it with a narrower gate
  (only self-loops created by loop_rotate, not pre-existing ones).
- **loop_unroll visibility**: `subst_value_with_operand`,
  `subst_value_in_terminator`, and `rename_inst_dest` promoted to
  `pub(crate)` so the new pass can reuse the proven value-rewriting
  helpers instead of duplicating them.

**v13 status**: zero regressions (486/489, same 4 env-only i686/regparm
failures as v12). The rotation pass is opt-in infrastructure; with it
off, benchmark numbers match v12 (loop_patterns ~0.98, libm 0.411,
tce_sum 0.961). The Godbolt-oracle gap analysis (loop rotation as the
systemic win) is confirmed: enabling `CCC_LOOP_ROTATE=1` on loop_patterns
produces the rotated `jne .Lbody` self-loop form that matches GCC's
shape, but the 24-test miscompile blocks default-enable. v14 target:
harden the rotation to handle multi-exit/nested shapes, re-land the
phi-elim self-loop fix with a narrower gate, then default-enable.

### v14 highlights (session 83)

v14 delivers the **loop-rotation correctness hardening** that unblocks the
canonical counted-loop shape, and resolves the benchmark-metric confusion
that clouded v12/v13 reporting.

- **Exit-merge-phi correctness fix** (the root cause): the v13 pass built
  the exit-merge-phi's test-exit incoming as `Value(new_loop)` — the new
  self-loop phi. But at the latch's CondBranch point that phi still holds
  the **start-of-iteration** value (its backedge writeback only fires on
  the NEXT iteration's entry), so the exit read the sum **before** the
  final `a[i]` was added (off-by-one accumulator). v14 reads
  `latch_operand` (the original header phi's latch incoming — the
  post-iteration value `s_new = s + a[i]`, or `i_next = i + 1`) on the
  test-exit edge. This single fix dropped rotation miscompiles from
  **122/486 → ~24** (24 real + 4 env-only). Verified on `sum_arr`:
  output bit-identical to GCC; the `.LBB4: movl %r8d, %r9d` exit-merge
  copy is now correctly inserted (v13 omitted it).
- **Pipeline reorder: rotation after vectorize.** v13 ran loop_rotate
  BEFORE vectorize, which corrupted the vectorizer's base-dependence
  analysis on the rotated self-loop form
  (`vectorize_iv_dependent_base` SIGSEGV, `simd_vecreg` miscode). v14
  moves loop_rotate to AFTER vectorize+post-unroll+gaddr-cse. Vector
  bodies contain `Vec*` intrinsics which `is_cloneable_pure` rejects,
  so rotation bails on vectorized loops and only fires on the residual
  scalar counted loops the vectorizer left alone.
- **Conservative body guards**: bail if the body contains a
  `Call`/`CallIndirect` (clobbers caller-saved values the exit-merge-phi
  references across the call boundary; also keeps `fib`'s recursive-call
  CFG — spuriously detected as a loop by `find_natural_loops` —
  untouched), any volatile `Load`/`Store` (observable ordering), or an
  `Intrinsic` (XMM phi / vector-reg home mismatch; the vectorizer has
  already run). Fixed `fp_memfold_stencil5` (intrinsic bail).
- **Dead `drop(header_insts)` removed**: was a no-op on a `&T` (references
  are `Copy`) and tripped the `dropping_references` lint under
  `-D warnings`; NLL ends the borrow at last use without it.
- **Metric clarification (resolves the v12 "0.81 faster than 0.63"
  confusion)**: the benchmark runner reports `LCCC/GCC` (ratio > 1 =
  LCCC slower, < 1 = LCCC faster). Prior worklog entries used the inverse
  `GCC/LCCC` (> 1 = LCCC faster), so "nbody 0.81" meant GCC/LCCC = lccc
  is 1.23× slower — the SAME reality as the runner's `1.229`. The "0.31"
  nbody figure was a miscompile (no-op). The README results table is in
  the runner's convention (LCCC/GCC, < 1 = faster) and is authoritative.

**v14 status**: zero regressions (486/489, same 4 env-only i686/regparm
failures as v13). Default-path benchmark numbers match v13 (no-rotation
path unchanged): 10-kernel 9-rep paired medians vs GCC -O2 — `libm_round`
0.413 (2.42× faster), `gzip_crc32` 0.874 (1.14× faster), `loop_patterns`
1.033 (near parity), `nbody` 1.229, `mandelbrot` 1.231, `spectral_norm`
1.302, `expat_xml_scan` 1.728, `sqlite_varint` 1.294, `linux_find_bit`
1.441, `fannkuch` 1.397; geomean 1.127. The FNV prime `1099511628211` is
register-homed (`%r14`, matching GCC's `%r10`) — the v12 spill/reload
concern is closed. With `CCC_LOOP_ROTATE=1`, the canonical `sum_arr`
loop rotates to the test-at-bottom self-loop form bit-identical to GCC.

**v15 roadmap** (the rotation pass is correctness-clean for the canonical
shape; remaining ~24 miscompiles block default-enable): (1) harden
multi-exit / nested-loop / header-phi-escapes-through-non-Return-terminator
shapes so rotation can default-on at -O2+; (2) re-land the phi-elim
self-loop copy-placement fix with a narrower gate (only rotation-created
self-loops, not pre-existing `sqlite_varint`-style ones) to eliminate the
residual double-jump the backend's critical-edge split re-creates; (3)
**IV widening** — keep the loop IV I64-typed in the vectorizer/loop
optimizer so the per-iteration `movslq %i32d, %i64` (seen in loop_patterns
LCG: `addl $1, %r12d; movslq %r12d, %r12`) is eliminated (GCC keeps an
I64 IV); (4) wire the find_max AVX2 `vpmaxsd` reduction transform (the
intrinsics + lowering landed in v11; the reduction analyzer's GEP/IV
stride scaling must fire for the Select-shaped max pattern); (5) integer
`dot_product` `vpmuldq` widening-mul.

### v11 highlights (session 81)

- **FP binary-op staging-copy elimination**: when both operands of a
  scalar FP op are XMM-homed and the destination is a fresh register,
  emit the VEX 3-operand form directly (`vmulsd %src2, %src1, %dest`)
  instead of staging lhs into dest with a redundant `vmovsd %x,%x,%tmp`
  first. Every squared term in the mandelbrot inner loop dropped a
  redundant copy.
- **FP compare-to-branch fusion**: relational float Cmps (Sgt/Sge/Slt/Sle
  and unsigned peers) consumed only by an adjacent CondBranch now skip
  the boolean materialization (setcc + movzbl + testq) and branch on the
  live `ucomisd` flags directly (`ucomisd; ja/jae`). NaN → unordered →
  not-taken = false, matching C99 for all ordered relationals. Eq/Ne
  stay materialized (parity bit). The mandelbrot
  `if (zr*zr+zi*zi > 4.0) break` drops from 6 instructions to 2.
- **`VecMaxI32x8` / `VecHorizontalMaxI32x8` intrinsics + AVX2 lowering**
  landed: `vpmaxsd` lane-max and a `vpshufd`+`vpmaxsd` horizontal reduce
  that preserves sign bits (correct for all-negative data, unlike a
  `vpsrldq` zero-fill reduce). The v12 session wired the full transform
  (see v12 highlights).

### v9 highlights (session 80)

- **Conditional-sum vectorization (masked widening reductions)**:
  `long s = 0; for (...) if (a[i] > K) s += a[i];` now vectorizes as
  GCC's canonical form — vpcmpgtd lane mask, sign-extended through the
  widening pipeline, vpand zero-mask, paddq — via the new
  VecWidenMaskedAddI32x4ToI64x2 composite intrinsic and x86 late
  vectorization (post-if_convert). The conditional-sum kernel: 62ms
  scalar → 38ms cmov → **25ms masked** (2.5×; gcc 21ms).
- **AVX-SSE transition penalty fix in widening codegen**: the widening
  reductions mixed legacy SSE with VEX instructions — after any
  YMM-writing loop every legacy op re-triggered the transition penalty
  (measured 9× on init+map+sum sequences: 318ms → 35ms). All
  widening-loop instructions are now VEX three-operand forms.
- **loop_patterns: 381ms → 99ms (3.9×)**; ratio vs GCC 0.11 → 0.39.

### v8 highlights (sessions 73–78, this tree)

- **VEX 3-operand scalar-FP exploitation** (session 76): the scalar-FP
  emitters' 2-operand staging copies (`movsd %A,%D; vOP %S,%D,%D`) are
  fused/folded into the 3-operand VEX form — nbody 84→68ms (1.22×).
- **Widening I32→I64 reduction vectorization** (session 77):
  `long s += arr[i]` over `int[]` now runs 4 elements/iteration
  (vmovdqu + vpunpckhqdq + vpmovsxdq×2 + paddq) with full I64 lane
  precision; previously 1 element/iteration scalar.
- **Conditional-sum if-conversion** (session 78): the
  `if (arr[i] > 0) s += arr[i]` diamond now converts to cmov — the
  conditional-sum kernel dropped 62→38ms (1.7×). The enabler is
  address-canonicalized arm-load coverage in if_convert (GlobalAddr
  bases canonicalize by symbol; GEP offsets trace through Cast/Shl/Copy
  chains), plus an effective-instruction arm budget that discounts pure
  address-materialization chains the backend folds into one SIB operand.
- **General complete unrolling** (session 75): constant-trip loops of any
  block shape, including bodies containing inner loops, with const-chain
  IV resolution so the fixpoint cascades outer→inner (two miscompiles
  found and fixed by differential testing; nbody output bit-identical to
  GCC -O0).
- **FMA3 ISA gate** (session 73): contraction alone no longer emits
  vfmadd on baseline SSE2 targets (SIGILL on pre-Haswell) — matches GCC.

### v7 highlights (since v6)

- **Agent C's session-75 peephole layer**: 8 new transforms built on a
  new exact CFG-liveness module. Kernel corpus: 325 → 322 instructions
  vs GCC's 264. Largest per-program wins: `gzip_crc32` −26 instructions,
  `spectral_norm` −20, `hash_table` −16, `sqlite_varint` −13, `isort`
  −3.
- **PF-06 (secondary-IV strength reduction)**: `a[i±K]` GEPs with the
  offset `add(iv, const)` are now SIB-folded into
  `disp(%base, %iv, scale)`, eliminating the per-iteration
  `lea X(%iv); cltq; lea 0(,%rax, scale)` chain. Soundness gates:
  base-contains-iv and iv-update-Copy-coalescing detection. Kill switch:
  `CCC_NO_PF06_ADD_PEEL=1`.
- **A miscompile in `reuse_redundant_loads`** (sign-extending-to-64-bit
  loads were rewritten to `movl`, losing the upper 32 sign-extension
  bits) was caught in audit and fixed: copy width now chosen per load
  class (`movq` for `movq/movsbq/movswq/movslq`, `movl` for the 32-bit
  zero-extending class, refused for `movb/movw`).
- See `docs/history/2026-08-24-session75-v7-audit-pf06.md` for the
  full audit notes and the soundness proofs.

## Reading the table honestly

- **Where LCCC wins**: recursion folding (`fib`/`ackermann`/
  `constant_recursion` — TCE + rec2iter; GCC keeps exponential recursion),
  popcount recognition (`bitops` 1.7× via `popcntl`), reduction FMA
  vectorization (`matmul`), the CRC table loop (hoisted PIC base + magic-const
  hoisting; 1.14×), and the whole cluster of integer/branch kernels in the
  0.92–1.10 band.
- **Where GCC still wins**: non-reduction dense FP (`spectral_norm` 4.8×,
  `nbody` 3.8×, `mandelbrot` 1.65× — general loop-nest vectorization is the
  tracked structural gap; the stencil vectorizer covers element-wise shapes
  only), and a codec/parser cluster (`loop_patterns` 2.8×, `find_bit` 1.96×,
  `fannkuch` 1.76×, `expat` 1.67×, `adler32` 1.59× — RA-06 arithmetic-chain copy
  webs and multi-store stencil analysis are the mapped fixes; see
  [`engineering/agent/BACKLOG.md`](engineering/agent/BACKLOG.md) §16.4).
- Every number above was produced by a **correct** binary; correctness is
  enforced before speed is recorded (checksums byte-identical to GCC).

## Where LCCC wins

- **Tail-call elimination (TCE)** and **binary recursion-to-iteration**:
  fib-class recursion runs ~23× faster than GCC (GCC keeps the exponential
  recursion).
- **Reduction vectorization**: simple `sum`/`dot` reductions go 4-wide AVX2
  where GCC `-O2` stays scalar.
- **Profile-guided optimization**: `-fprofile-generate`/`-fprofile-use` fully
  integrated (inlining, unrolling, layout, switch lowering) with no
  PGO-induced regressions on the workload kernels.
- **Peephole layer with exact CFG liveness** (v7): parameter-shuffle
  coalescing, dead pure-write elimination, load+self-test → memory
  compare, accumulator round-trip elimination, redundant-load reuse,
  redundant self-test after logical ops, dead sign-extension narrowing,
  and producer retargeting + store relay. Each rule is gated by a
  skip-key for bisection.

## Root cause of the remaining gap vs GCC

The 2–5× FP deficit is concentrated in three mechanisms, in priority order:

1. **FP values round-trip through the accumulator** (`%rax`/`x0`) on cast and
   copy paths instead of staying in SSE/NEON registers.
2. **No FMA contraction on x86** — GCC fuses `a*b+c` into `vfmadd*`;
   LCCC emits separate mul/add (AArch64 already fuses via `fmadd`).
3. **No loop-nest vectorization** — only the innermost reduction idiom is
   vectorized; matmul/spectral/nbody inner loops stay scalar.

The integer/codec gap (adler32 1.59×, find_bit 1.96×, loop_patterns 2.8×)
is concentrated in two more:

4. **RA-06 (arithmetic-chain copy webs)**: adler32 keeps eight unrolled
   byte temporaries live for the s2 recurrence and spills `s1`/`s2`.
   Needs next-use-aware eviction, not a peephole.
5. **Multi-store stencil analysis**: nbody has six stores across two IVs
   plus field-sensitive load/store disambiguation; sound incremental
   vectorization needs cross-iteration dependence analysis.

## Future work (see `hotspots/` and `ideas/`)

- Struct-by-value ABI and wide (vectorized) aggregate copies.
- Broadening the auto-vectorizer (non-reduction FP loops, FMA fusion).
- Instruction scheduling for the Raptor Lake port/resource model.
- Sample-based PGO and PGO value specialization.
- Use-def-chain shared optimizer context.
- RA-06: next-use-aware eviction with copy-web coalescing for arithmetic
  chains (adler32's eight-byte recurrence).
- Multi-store stencil vectorization (nbody, mandelbrot, spectral_norm).

## Kill switches (for bisection / soundness fallback)

All kill switches are env vars; setting any non-empty value disables the
named pass.

| Switch | Disables |
|---|---|
| `CCC_NO_SEGMENT_RA` | segment-aware register allocator (restores exact fat model) |
| `CCC_NO_STENCIL_VEC` | stencil vectorizer |
| `CCC_NO_DEFER_OVERFLOW_VECREG` | deferred overflow vector register allocation |
| `CCC_NO_SEGMENT_FILL` | segment-aware residual fill (default-on x86-64) |
| `CCC_NO_GLOBAL_ADDR_REMAT` | global-address rematerialization |
| `CCC_NO_MAP_VEC` / `CCC_NO_MAP_VECREG` | map-style vectorization |
| `CCC_NO_PF06_ADD_PEEL` | PF-06 `add(iv, const)` / `sub(iv, const)` SIB displacement peeling (the existing SIB fold without displacement stays on) |
| `CCC_VERIFY_REGALLOC=1` | verifies register-allocation invariants over the correctness corpus (catches RA interference bugs never surfaced by the regression suite) |
| `CCC_DEBUG_COALESCE` / `CCC_TRACE_ALLOC` / `CCC_RA_EXPLAIN` | RA debug tracing |
| `LCCC_DEBUG_VECTORIZE` / `LCCC_WHY_NOT_VECTORIZE` | vectorizer debug tracing |
| `LCCC_DUMP_IR=1` | dumps the post-optimization IR for each function (for inspection) |
| `CCC_NO_GEP_FOLD` | disables all GEP folding (both const-offset and indexed-SIB) — strictest fallback |
