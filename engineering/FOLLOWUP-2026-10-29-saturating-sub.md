# Saturating subtract, cross-target `-z` diagnostics, and rebase ledger (S62–S70)

## Scope and current ownership

This note records the scalar unsigned-saturating-subtract work, the associated
red-team results, and the independent AArch64/RISC-V linker-diagnostic fix.
The final rebase base is `ms178/lccc` main
`d465f1c9c8a64345b2c99f038a8cd31aada74606`. The final validated compiler is
`3ba50c03519abded657259bbfddd450add635f6e`; the following documentation-only
commit does not change its binaries.

Ownership changed during the rebases. Main now contains the packed U16
saturation implementation, its register-allocation/copy-coalescing plumbing,
the narrow-compare width fixes, and the newer max-select packed optimization.
The old candidate's duplicate packed hunks were therefore dropped. The much
smaller remaining series carries:

- the late scalar `UsubSat32` IR operation and its exact pattern matcher;
- x86-64 and baseline-i686 lowerings, including the secondary-cache fix found
  by the final red-team;
- additional U32, i686, signed-max, different-subtrahend, and runtime
  negative-control coverage while retaining upstream's packed and cross-target
  tests;
- warnings for cross-target dynamic `-z` keywords that are accepted but not
  implemented, plus regression coverage for the generic policy merged upstream;
- a surgical correction of latest main's stale pre-unroll reducer argument,
  which otherwise prevented every Rust target from compiling; and
- the earlier loop/AND/preprocessor work documented in its own follow-ups.

No performance claim below is a whole-compiler ranking. The measured workload
is `tests/benchmark/programs/zlib_ng_slide_hash.c`; static instruction counts
are not timings, and Callgrind Ir is a simulation rather than PMU cycles.

## Matcher legality and deliberate boundaries

`src/passes/saturating_sub.rs` runs after the vectorizers, only for x86-64 and
i686. `CCC_DISABLE_PASSES=satsub` is the exact-token ablation switch. Running
late preserves the input expected by upstream's packed U16 recognizers and
lets the scalar path handle scalar-only x86-64 and i686 without opening the
i686 packed-vector backend.

The accepted graph is an exact compare/subtract/select idiom. Both compare
operands must be the corresponding subtract operands. The condition and the
selected arithmetic result must be single-use; a narrow subtract hidden below
a zero extension must also be single-use. A different subtrahend, nonzero
fallback, shared compare, or shared arithmetic result is rejected. GT/GE with
the subtract in the true arm and LT/LE with it in the false arm are equivalent
at equality because `a - a` and the fallback are both zero.

Full-domain unsigned 32-bit arithmetic needs no range proof. Every signed
compare, signed arithmetic/result, and narrow-roundtrip form requires a common
nonnegative 32-bit domain proof. The only proofs admitted are zero extensions
from U8/U16, copies of such proofs, and constants in `0..=i32::MAX`. A
same-width U32-to-I32 reinterpretation is not a proof. The narrow form also
requires each wide compare operand to be the exact zero extension of its
corresponding narrow subtract operand; narrow stack homes are never passed to
a 32-bit backend operation speculatively.

The previous `u16 x < -1` wide-bound failure is a useful negative example:
wrapping the bound to lane width changes the result. The current width checks
and their four focused tests remain green, but this late scalar pass does not
weaken those checks or claim ownership of the now-upstream packed rewrite.

## Backend lowering and the final cache red-team

x86-64 emits:

```asm
subl   %ecx, %eax
movl   $0, %ecx
cmovcl %ecx, %eax
```

`SUB` sets CF exactly for unsigned `lhs < rhs`; the zero materialization does
not change flags. CMOV is guaranteed on x86-64 and avoids both a branch and an
SSE requirement. The earlier SBB/NOT/AND x86-64 experiment was natively slower
and was not restored.

Baseline i686 has no such CMOV guarantee and emits:

```asm
subl %ecx, %eax
sbbl %ecx, %ecx
notl %ecx
andl %ecx, %eax
```

SBB produces `-CF`; NOT forms the mask `CF-1`. The shorter `sub;sbb;and` form
is wrong and must not return. Packed saturation remains an x86-64/U16 feature;
there is no i686 packed lowering obtained merely by widening a target gate.

The final register-cache review then found a real defect in both new
lowerings. They overwrite RCX/ECX but initially invalidated only the
accumulator cache. The generic `operand_to_reg(..., "rcx")` helper does not
perform the secondary-cache bookkeeping done by `operand_to_rcx`. A later
consumer could therefore reuse a pre-intrinsic value that no longer existed
in RCX. Two direct emitter tests seeded a secondary-cache entry, emitted
`UsubSat32`, and checked its validity:

- before the fix: **2/2 FAIL**, one each for x86-64 and i686, rc=101;
- after adding `invalidate_sec()` in both backends: **2/2 PASS**.

The instructions themselves did not change. All four measured on/off workload
binaries still have exactly the same SHA256 after the fix. This was a focused
correctness fix, not a performance experiment. Other generic RCX-staging call
sites are a separate audit item below, not asserted defects without their own
reproducers.

## Performance evidence

### Final isolated ablation on latest main (S70)

These experiments use the **same final compiler binary in both arms**. The
control wrapper sets exactly `CCC_DISABLE_PASSES=satsub`; the other wrapper
leaves the pass enabled. Thus upstream packed saturation and every unrelated
optimization remain identical. Each run checks stdout, order is randomized
and alternating, and processes are pinned to CPU 0. Intervals are exploratory
bootstrap intervals of the paired median ratio, not a general project CI.
The measurements were repeated after main advanced to `d465f1c9c`; the S68
numbers are not relabeled as final evidence for this different compiler.

| Mode | Rounds | Median on/off | Exploratory 95% interval | Median seconds off/on |
|---|---:|---:|---|---|
| x86-64, SSE/AVX disabled | 31 | **0.9010041577** | [0.8951916548; 0.9050142642] | 0.083530969 / 0.074877663 |
| i686 | 21 | **0.6766923942** | [0.6727157335; 0.6834857691] | 0.176144404 / 0.119424126 |

Ratios below one favor the enabled pass. Bounded Callgrind with identical
cache geometry and checked output gives:

| Mode | Ir off | Ir on | Ir on/off | Bcm on/off |
|---|---:|---:|---:|---:|
| x86-64 scalar | 1,878,486 | 1,681,860 | **0.8953274073** | 2,905 / 2,905 |
| i686 | 3,572,358 | 3,194,112 | **0.8941186746** | 9,997 / 23,086 |

On this final compiler, the SSE2 and AVX2 assemblies and direct production-path
objects are again **byte-identical with `satsub` on and off** for this workload.
There is consequently no honest final late-scalar-pass timing comparison to
report for those two modes. All native and Callgrind receipts, compiler and
wrapper hashes, assembly diffs, and object hashes are retained in S70.

### Historical whole-candidate measurements (S66)

Before the latest upstream integration, S66 compared the complete original
candidate with the preserved S62 before-compiler. These data remain useful
history, but after the rebase they are not isolated claims about only the
remaining scalar patch: the packed implementation is now upstream.

| Mode | Native median after/before | Exploratory 95% interval | Callgrind Ir before → after | Ir ratio |
|---|---:|---|---|---:|
| x86-64 scalar, 31 rounds | 0.8993388222 | [0.8945749604; 0.9051528801] | 1,895,807 → 1,699,181 | 0.8962837462 |
| SSE2, 11 batched rounds | 0.7805462945 | [0.7754698899; 0.8507406449] | 1,207,930 → 1,093,144 | 0.9049729703 |
| AVX2, 31 batched rounds | 0.9689893862 | [0.9555115248; 0.9812114092] | 1,068,497 → 1,068,497 | 1.0 |
| i686, 21 rounds | 0.6814157358 | [0.6771867983; 0.6843274685] | 3,579,332 → 3,201,086 | 0.8943249746 |

In particular, AVX2's historical native result must not be rewritten as an
instruction-count reduction: its Ir ratio is exactly 1.0. Conversely, the
current on/off objects show that the late scalar pass alone is neutral there.

The S66 static `sat_ge` oracle comparison was:

| ISA | LCCC | GCC 16.2 | Clang 23.1.x | ICX 2026.2 | ICC |
|---|---:|---:|---:|---:|---:|
| SSE2 | 38 | 44 | 54 | 29 | — |
| AVX2 | 36 | 45 | 60 | 29 | 68 (`-xAVX2`) |

These are static instruction counts for one function, not a runtime ranking.
The complete oracle-assembly and native receipts remain under the S62/S66
artifact directories; the final isolated JSON, assembly diffs, and Callgrind
manifests are in S70.

## Cross-target `-z` diagnostic contract

AArch64 and RISC-V accepted several options with empty stderr even though
their backends did not apply the requested semantics. The final shared warning
policy covers:

- the six recorded dynamic-flag keywords `origin`, `nodelete`, `nodlopen`,
  `initfirst`, `interpose`, and `nodefaultlib` (the remaining production-code
  addition in this series); and
- generic `global` and unknown/generic keywords through upstream's newly merged
  `z_other_keyword_warnings` helper, with the older focused tests retained.

Plain links warn; `-Wl,--fatal-warnings` rejects them. `separate-code` and
`combreloc` remain warn-free because their outputs were byte-identical to the
default on both cross targets. The merged upstream page-size policy and its
`backend_page_size` argument are preserved rather than duplicated or reverted.

The final dynamic matrix contains **20 AArch64/RISC-V cases**: eight warned
keywords with normal and fatal forms per target plus the two default-keyword
controls per target. Twelve x86-64/i686 controls under fatal warnings still
produce real `ORIGIN`, `NODELETE`, `NOOPEN`, `INITFIRST`, `INTERPOSE`, and
`NODEFLIB` dynamic flags with empty stderr. All four cross default/option
outputs are byte-identical.

PR #1018 and its larger page-size/linker-script diagnostics are now merged in
the final base. The present patch does not claim authorship of that scope; its
unique linker production change is the small dynamic-flag warning addition,
validated alongside—not instead of—the upstream policy.

## Rebase and PR ledger

Main first advanced from `3f732d90d44fe7a1b425574fdf43f6a4b030c927` to
`b8d1cd3c2d72dd774e505e45e325877dc51dcc88`; the mandatory pre-delivery fetch
then advanced it again to `d465f1c9c8a64345b2c99f038a8cd31aada74606`.
Upstream's new optimizer, preprocessor, vectorizer, cross-provisioner, and
benchmark-harness changes were retained. Conflict handling was hunk-specific:

- The lane-plan conflict kept upstream's unsigned-reduction gate and cost;
  `backend-contracts-b` is exactly **426.208 s** both by summation and by the
  predicted field.
- Upstream had already registered `check_u16_saturating_sub.sh` in the local
  mirror and hosted workflow. The old additional registration would have run
  it twice and was dropped. The expanded test body and upstream's cross-target
  leg both remain.
- Packed saturation and width hunks already upstream disappeared from the
  candidate diff instead of being reapplied as duplicate implementations.
- The latest saturation-test conflict retained both upstream's new
  `sat_max_select` checks and this series' `sat_u32` checks. The linker conflict
  retained upstream's generic/page-size policy and added only the dynamic flags.

Reviewed external refs included #1010
`ae01822872aec2d8c231542f78ae0953f2326fab`, #1015
`4eeb19e5b0b979d78ac4b5b6f3ad4a952d24b525`, and #1018
`64e1afed64b24c51b0b78c19c1c0ef3034309043`. #1010 was dynamically equivalent
to then-current main for the probed behavior. The final base now contains the
upstream merge commits for #1015 and #1018 (`c5fdcf759` and `c8956eaa8`), as
well as #1022; their code is accepted as upstream ancestry, not presented as
this patch's independently authored or validated series.

The latest base also exposed a confirmed upstream compile defect. Its new
pre-unroll entry called `strided_i32_reduce::try_transform` with four arguments,
while the exact-element-read reducer has taken three since `55db907ac` removed
`ModuleSymbolFacts`. Both failing files were initially byte-identical to main,
and the first all-targets build stopped with E0061. Removing the stale wrapper
parameter and facts construction—not restoring the obsolete analysis—fixed the
build. The existing vector-remainder gate then passed, including the strided
exact-access watchpoint and the dedicated pre-unroll transformation check.

The **0.75 s Lane-A discrepancy is pre-existing tooling debt**, not a cost of
this patch: `backend-contracts-a` sums to 412.880 s while its predicted field
is 412.130 s, and the same difference existed on the previous main. It must
not be hidden inside the saturating-subtract or linker measurements.

## Validation on the final compiler-code tree

The final code tree used for S70 has these direct receipts:

- Rust 1.99.1 fastbuild, O1, incremental off, all targets, exactly `-j2`:
  PASS in **17m26s** with swap active after the stale-argument correction. The
  preceding unchanged-main E0061 failure and the successful resumed build log
  are both retained.
- All-targets Clippy with `-D warnings`: PASS in **5m45s**;
  `cargo fmt --all -- --check` and `git diff --check`: PASS.
- Focused unit tests: cache regression **2/2**, saturation **5/5**, z-ISA
  module **16/16**, narrow-compare width **4/4**, intrinsic vector-home
  **1/1**. The two new z-ISA tests came from the merged upstream page-size
  policy; the older cross-warning tests were adapted to its new signature.
- Dynamic saturation gate: PASS, including O2/O3, SSE2/AVX2/scalar/i686,
  boundary/tail/overlap and negative controls against GCC 16, plus the
  AArch64/RISC-V compile-only fallback leg.
- Vector-remainder/pre-unroll gate: PASS. Relevant rebase gates for downcount,
  ANDQ, affine folding, macro parity, load/cast, two-block unroll, SLP
  recurrence, unsigned reduction, and include-next: PASS. Static CI parity,
  lane-layout, no-live-Godbolt, cross-provisioner, CPU-publication, and the
  benchmark harness suite: PASS.
- Cross-link matrix: 20/20 warning/default cases and 12/12 implemented x86
  controls PASS.
- S70's patch applies to the main-only temporary index and reproduces the exact
  HEAD tree. The bundle verifies against the same base; patch, source archive,
  logs, and hashes are retained. The final documentation commit receives a new
  tree-exact snapshot rather than reusing the earlier patch hash.

No local full or fast CI runner was invoked. No full-corpus, fuzz, PMU,
Raptor-Lake, or whole-application claim is made. GitHub CI remains required
for those broader integration lanes.

## Remaining work, in priority order

1. **Audit other generic RCX-staging emitters.** The two new `UsubSat32`
   lowerings are fixed and tested. Similar-looking existing intrinsic calls
   must each get a cache-lifetime proof or a direct seeded-cache regression;
   do not mass-edit them on pattern alone.
2. **Keep performance work workload-led.** Extend saturation only for a new
   measured shape with the same operand/use/range proofs. Do not re-time the
   byte-identical SSE2/AVX2 objects, widen i686 packed lowering speculatively,
   or turn static instruction counts into speedup claims.
3. **Keep future linker changes evidence-specific.** #1018 is now merged; do
   not present it as an open unreviewed PR. Any new page-size/script-mode change
   still needs its own GNU/BFD/LLD/mold parity evidence, which the dynamic-flag
   matrix does not provide.
4. **Repair Lane-A accounting separately.** Reconcile the existing 0.75 s
   mismatch from measured gate provenance without perturbing the exact Lane-B
   sum or disguising it as compiler work.
5. **Continue the larger generated-code backlog.** The SQLite/glibc/zlib-ng,
   register-allocation, whole-workload, and real-PMU tasks in the performance
   backlog remain open. This single kernel improvement does not close them.
