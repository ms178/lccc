# REGEHR 2124 transplant: missed-alarm testing and Minotaur-lite for LCCC

Source: Regehr & Livinskii, its authors, "Looking for Missed Alarm Bugs in a Formal
Verification Tool" (2024-09-04), plus the Alive2, intel/yarpgen and
Minotaur projects it discusses.

This document records what was transplanted, what was deliberately not,
how each piece is validated, and what it found. It also records the
red-team audit of the untrusted agent patch this work started from —
several of its components were salvaged only after real defects were
found and fixed, and its documentation claims did not survive contact
with a build. **Trust nothing in this tree that is not backed by a
runnable test; every claim below names its evidence.**

## What the blog post actually says (distilled)

Alive2 proves (or refutes) that an LLVM transform is a *refinement*.
The dangerous failure is the **missed alarm**: Alive2 says "refines"
for a transform that does not. Testing for missed alarms needs pairs of
programs that are *known* to be inequivalent, presented so the tool is
asked "does A refine B?" and must answer no.

* **Method 1 (mutation pairs).** YARPGen generates UB-free programs;
  a UB-preserving mutation produces a sibling; compiling and *running*
  both filters out pairs where the mutation preserved behaviour. The
  surviving pairs are known-inequivalent: Alive2 must reject
  refinement between them, at scale, or a missed alarm has been found.
* **Method 2 (Minotaur).** A superoptimiser cuts a slice out of a real
  program, treats the slice (with its backward slice) as the spec, and
  enumerates candidate replacements with *symbolic constants*
  (an ∃∀ solver query). Each candidate must be **proven** a refinement
  by Alive2 and **proven cheaper** by llvm-mca before it is applied.
  A missed alarm anywhere in that chain becomes a miscompile of a large
  real program — which ordinary test suites then catch. The verifier is
  thus stressed by the superoptimiser's output, not by hand-written
  cases.
* **Outcome at LLVM scale:** very few missed alarms were found. The
  authors' standing suspicion: constructs neither generator stresses
  (function attributes were named) are where the remaining risk lives,
  and characterising *state-space coverage* of such testing is open.

## Transplants (what LCCC took, and why)

LCCC has no Alive2; the "verifier" under test is LCCC's own optimiser
and the "alarm" is its observable behaviour versus a reference. The
transplant keeps the *structure* of both methods and replaces the
proof oracle with the strongest check available at each layer.

### 1. `scripts/mutpair.py` + `fuzz_diff.py --engine mutpair` (Method 1, expression scale)

Generates a UB-free-by-construction `uint32` function plus a one-step
UB-preserving mutation, evaluates both under a bit-exact Python model,
and keeps pairs with a *known* relationship (inequivalent, or known
equivalent as controls). The emitted TU prints both result columns;
every compiler/opt-level must reproduce the model's answers, and must
not collapse a known-inequivalent pair.

Red-team fixes over the salvaged draft (each is test-pinned):

* Operand-swap mutations on **shifts** produced unmasked shift counts
  (C UB the Python model cannot see — ≈1.1 % of generated pairs). Swap
  now excludes `%`, `<<`, `>>`; `validate_ub_free` structurally audits
  every emitted pair; validator negative controls must reject planted
  UB. A 2295-pair prober found 0 violations after the fix.
* `judge_pair` re-materialises the source per invocation (a fake
  optimiser in the self-test rewrites its input in place).
* `--self-test` requires a deliberately collapsing fake optimiser to be
  caught — the harness is itself tested for missed alarms.

Evidence: `scripts/test_mutpair.py` (17 tests), self-test,
`fuzz_diff --engine mutpair` 120/120 vs GCC `-O0..-O3`, standalone
model-oracle runs vs LCCC and GCC.

### 2. `scripts/yarpgen_mutpair.py` (Method 1, whole-program scale)

For each seed: original YARPGen program and its `--mutate=exprs`
sibling (YARPGen's own UB-preserving mutation). The pair is kept only
if the final-state hashes **provably differ** under the reference
(GCC `-O0`/`-O2` must agree with each other first — consensus oracle);
LCCC must then preserve the difference at every requested `-O` level.
`--self-test` plants a collapsing fake reference and requires
detection. `scripts/test_yarpgen_mutpair.py` covers the pair/filter
logic (5 tests). CI gates: `yarpgen-mutpair-tests`, `yarpgen-pair-smoke`
(SKIP-clean without a provisioned YARPGen).

### 3. `scripts/slice_superopt.py` (Minotaur-lite, offline only)

Cost-bounded enumerative synthesis over small integer slices with
observational-equivalence pruning. The Alive2 role is played by
**exhaustive 8-bit truth tables** (no sampling: at the checked width a
missed alarm is impossible), re-gated at 16/32/64 bits on
boundary+random vectors before anything is reported. Constants are
solved by pool enumeration — deliberately *not* a general ∃∀ solver,
so the tool is complete only for the candidate spaces it enumerates;
variable-shift specs are flagged as masked-shift (hardware) semantics,
not C. The llvm-mca role is played by a documented op-weight cost
proxy: **a reported win is a hypothesis** until `bench_kernels.py`
confirms it on the target. Output is a reviewable candidate table;
nothing is auto-applied to codegen.

### 4. YARPGen infrastructure, resurrected and pinned

* `scripts/ensure_yarpgen.sh`: probe-gated provisioning of
  intel/yarpgen at `1adb290f453505838f3aa33dc571f292b0c1810f`
  (the previously referenced fork is gone/private).
* `fuzz_diff.py --engine yarpgen` fixed: collect **all** generated
  `.c`/`.h` (missing `init.h` silently changed semantics), `--seed=N`,
  `--max-array-dims=3` (the generated code indexes `[i][j][k]`), only
  `.c` files as TUs, and full multi-file repro saving.

### 5. Benchmark kernels (the "large real programs" leg, miniaturised)

`tests/bench/k_*.c`: crc32 slice-8 (zlib-ng shape), base64 decode,
jhash, SQLite varint decode, UTF-8 validation, Horspool search —
checksum-verified against GCC by `bench_kernels.py`, screened on this
VM (geomean 0.795× vs GCC 14.2; VM numbers are screening-only, no PMU).

### 6. Statement-expression / min-max typing (the attribute-axis lesson)

The blog's open suspicion — *unstressed construct classes hide the
bugs* — paid off immediately in an adjacent form: the fixed bugs below
both lived in corners random testing under-samples (constant `_Bool`
conversions with zero low bytes; GNU statement expressions whose tail
conditional mixes arm types, i.e. the kernel `min`/`max` idiom). The
planned extension along this axis (attributes/qualifiers as a
first-class differential dimension) is recorded in the FOLLOWUP.

## What it found (all fixed, all regression-pinned)

1. **`(_Bool)` constant fold truncated before normalising.**
   `(_Bool)256`, `(_Bool)67106816ULL` folded to 0 (C11 6.3.1.2 wants 1
   for any nonzero scalar). Dual of corpus case #1. Fixed at four
   folding sites; `tests/regression/regress_const_bool_cast_zero_lowbyte.c`.
   Found by YARPGen seed 20261023 (diverging global `arr_55`).
2. **Statement-expression tail conditional typed as its then-arm.**
   `get_expr_ctype_with_scope` returned the then-branch type instead of
   the C11 6.5.15 composite type, truncating every mixed-type
   `min`/`max` statement expression containing a declaration. YARPGen
   seed 20261020: loop init collapsed (0 trips at `-O1`), guarded
   stores skipped at `-O2`; counter-instrumented `-O0` also diverged
   (5 vs 30 inner iterations) while the final checksum coincidentally
   matched. Fixed by composing both arms via
   `CType::conditional_composite_type`;
   `tests/regression/regress_stmt_expr_conditional_composite_type.c`.
3. **Unary minus on `unsigned int` constants folded modulo 2^64.**
   Both constant folders negated the I64 carrier instead of reducing
   modulo 2^32 (C11 6.5.3.3p1 + 6.3.1.3p2): `-(3844603080U)` folded to
   18446744069864948536, flipping a folded `min()`. The sibling `~`
   fold already had the width treatment. Found by
   `yarpgen_mutpair.py` seed 101 (the pair machinery earning its
   keep); regression `regress_const_neg_unsigned_wrap.c`.
4. **Pre-existing stale peephole fact** (found while hardening the
   salvaged redundant-sign-extension pass): the `movq`→`movl`
   narrowing rewrite left `sext32` set for the redefined register, so
   a later self-`movslq` could be deleted on a stale fact. The
   hardening clears extension facts at every rewrite site; a
   mutation-style negative control (removing the clear) fails the new
   unit test. The pass itself (base64: `movsbq` 36→24, +12 % wall A/B
   vs its `CCC_NO_SEXT` kill switch, VM screening) is salvaged and
   hardened: low-subregister-exact consumers, parenthesis-aware
   destination classification, conservative clearing on parse failure.

Post-fix evidence: seeds 20261019/20261020/20261023 match GCC at
`-O0..-O3`/`-Os`; YARPGen 10-seed sweep 7 PASS / 2 SKIP / 1 timeout
(seed 20261019 compiles correctly standalone in 83 s on this 2-CPU VM —
a compile-time item, tracked in the FOLLOWUP); full lib test suite
5315 passed / 0 failed.

## Salvage ledger (untrusted input patch)

| Component | Verdict |
|---|---|
| mutpair engine core | salvaged after fixing shift-swap UB + judge rewrite flaw |
| slice superoptimiser | salvaged; masked-shift semantics now flagged, JSON-tagged |
| fuzz_diff wiring | salvaged |
| bench kernels | salvaged; checksums re-verified vs GCC; prior perf ratios did **not** reproduce and are not cited |
| redundant-ext peephole | salvaged after removing a duplicate `#[test]` (did not compile) and closing three stale-fact miscompile holes (one pre-existing on main) |
| alu_torture oracle fixes | salvaged as UB hygiene; the claimed `-O2` divergence does **not** reproduce under GCC 14.2 — kept because `clz(0)`/cast punning are genuinely UB, matching the file's existing ctz guards |
| patch's own docs/claims | rejected ("no Rust", gate counts) — replaced by this document and per-commit evidence |

## Deliberately not transplanted (yet)

* A real ∃∀/SMT superoptimiser loop and an Alive2-style verifier for
  LCCC IR. The offline enumerative tool covers the small-slice niche
  with zero missed alarms at 8 bits; an SMT-backed loop is the next
  step only if slice mining shows wins worth automating.
* Attribute/qualifier differential axis (the blog's named gap) —
  designed in the FOLLOWUP; the two bugs above argument it is the
  right next investment.
