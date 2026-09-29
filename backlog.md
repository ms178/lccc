# LCCC Engineering Backlog

Live triage queues for measurable runtime/quality work. This file holds
**open items and their evidence**; session narratives, landed-work
history and measurement method live at the paths below — add history
there, not here.

- Sessions / measuremethod: [`engineering/journal/`](engineering/journal/README.md)
  (worst-10/worst-12 triage screens: `2026-09-W2.md`; daily entries tagged
  per-workstream: per-day edit).
- Method doctrine: [`engineering/agent/RULES.md`](engineering/agent/RULES.md).
- Per-area canonical knobs/kill-switches:
  [`engineering/subsystems/`](engineering/subsystems/README.md).
- The measured oracle baselines: [`engineering/evidence/benchmarks/`](engineering/evidence/README.md#benchmarks-and-comparisons).
- Deep research: [`engineering/DECISIONS.md`](engineering/DECISIONS.md) (measured
  negative-space, do-not-retry grounds), [`ideas/`](ideas/README.md).

Last broad triage rebuild: **2026-09-16**, fictional base `8ca2fd4`.
The historical numbers below are from that earlier screen, not the current
baseline. On **2026-09-25**, `main` at `7ddb770f` was remeasured in 39
randomized paired, output-checked VM workloads against the candidate and GCC;
the only changed executable `.text` was LZ4. The sound byte-copy loop-idiom
fix retains ~10.8× speed over disabling that idiom on a separately scaled
LZ4 screen, while preserving forward-overlap semantics. See
[`engineering/evidence/2026-09-25-redteam/loop-idiom-redteam/README.md`](engineering/evidence/2026-09-25-redteam/loop-idiom-redteam/README.md).
On **2026-09-26**, follow-up work was based on upstream `e5bc1911`. A
41-workload randomized three-arm Xeon-VM screen found changed executable
`.text` only in `spectral_norm`; the paired 15-round focused result is
candidate/current-main **0.0361** (240 ms / 6.60 s), with exact benchmark
output agreement. The fix replaces the AVX2 strict-reciprocal lane pack's
legacy SSE instructions with VEX forms. The newly added match-rich LZ4 and
scaled find-bit arms reveal gaps hidden by the original short/no-match inputs.
The evidence and remaining P0 limitations are in
[`engineering/evidence/2026-09-26-followups/README.md`](engineering/evidence/2026-09-26-followups/README.md).
The i7-14700KF target has **not** been measured. Before accepting a new
**target-hardware** runtime improvement, use same-window, same-baseline,
output-checked paired A/B with ≥ 200 ms/arm and agreeing median/min, not
historical ratios.

An item with no reproducer does not belong here.

---

## P0 — largest measured gaps

### PF-SN-1 · AVX2 strict-reciprocal pack, target confirmation pending
Current main `e5bc1911` emits legacy `movd`/`pinsrd` immediately before
`vcvtdq2pd`/`vdivpd`. The follow-up replaces these with VEX forms **only
when AVX2 is available**; a focused 15-round, output-checked Xeon-VM screen
found candidate/main 0.0361 (240 ms vs 6.60 s), and a 41-workload scan found
changed `.text` only in `spectral_norm`. This is a reproducible VM result,
**not** an i7-14700KF timing. Target P-core validation is pending; use the
runner and evidence linked above. Negative-trip, repeated-vector, scalar-tail,
changing-sign divisor, and XMM-disabled tests protect semantics.

### PF-LZ4-1 · Byte-copy semantics fixed; byte-compare still open
The default-on v2 copy matcher covers both relevant LZ4 loops. The old
unconditional `memmove` rewrite was **incorrect** for forward-overlap smear,
and naively disabling the pass caused a 10.8× scaled-workload slowdown on the
Xeon VM. A guarded memmove fast path now preserves the scalar overlap loop.
The original input executed **zero** match extensions in an instrumented
12,288-pass run; its near-parity time is *not* evidence for byte-compare parity.
A separately registered match-rich arm exercises 49,299 long matches in
24 passes. On current main, the 9-round scaled Xeon-VM LCCC/GCC 14 ratio is
~1.29; candidate/main `.text` is identical, so the byte-compare gap remains
**open**. Widening a compare without proving **both** operand ranges valid
can overread near an object boundary. No speculative widening or incorrect
copy rewrite shipped. Require sanitizer/page-edge and overlap tests, a range
proof, all four oracles, and an output-checked target i7 paired A/B.

### PF-MB-1 · Mandelbrot hot scalar FP loop (open)
Current `e5bc1911` 9-round Xeon-VM ratio to GCC 14 is ~1.30; candidate and
main `.text` match. GCC 16, Clang 23.1, ICC, and ICX oracles **also use scalar
FP instructions** in the compared main loop; static AVX-instruction counts
do not prove vectorization. Further cross-pixel widening needs legal SSA/exit
proof, and scalar loop/scheduling improvements require paired measurements.
Reproducer: `tests/benchmark/programs/mandelbrot.c`. Done = output-verifying
paired A/B with a proven general change on the i7-14700KF.

### PF-FB-1 · `linux_find_bit` loop structure (open)
Current `e5bc1911` scaled 9-round Xeon-VM ratio to GCC 14 is ~1.40;
candidate/main `.text` matches. `bsfq` is present; the open question is
branch/index structure, not the loop-copy idiom. A 21-round output-checked,
one-instruction assembly splice removing an apparently redundant post-`andn`
`test` gave median no-test/original 1.013 on this VM; **not** a win, so no
peephole was shipped. Done = classified, semantics-proven general CFG change
with output-checking and target-CPU paired evidence, or a justified bound.

### RA-GLA-04 · GLA Phase 2 — full-identity register remat (sha256 hot)
Phase 1 shipped (`CCC_RA_GLOBAL_LOCATION=1`); source-less remat is default-ON.
The checked Phase-2 *spill-gap* experiment was **not** shipped: paired
`sqlite_varint` and `expat_xml_scan` regressed 1.60% and 4.51% respectively
on an earlier VM despite a few better static counts. On current `e5bc1911`,
`sha256_transform` is ~1.18× GCC 14 on the 9-round Xeon-VM screen;
candidate/main `.text` is identical and its runtime ratio's interval spans
1.0. **No allocator speedup or regression is established.** Require
post-allocation slot-traffic feedback and a small, verified general change
before reconsidering full register-source-spanning remat.
**2026-09-28: the slot-traffic feedback prerequisite now exists** —
`scripts/stack_census.py --per-slot N` reports per-slot load/store/addr
traffic, and the census shows `sha256_transform` has **0 spill** and 12
`csave` references, so the Phase-2 remat experiment cannot pay for itself on
this function in its current shape. Re-open only for a function the census
shows as genuinely spill-dominated.
Oracle targets: `sha256_transform ≤ 1.5×`, epilogue ref-count −50 %.
Design refs: [`engineering/DECISIONS.md`](engineering/DECISIONS.md)
RA-GLA-01/02/03. Aligns with **R3** (RA span supply + phi/ORI lowering —
skip the 1.93× sha256 without tripling the web count; compare
GHASH/mulpack for the multi-source shape rule).

## Tier 1 — measured, largest first

### ZERO-REM-1 · **LANDED 2026-09-28** — dead vectorizer remainder loops
The map vectorizer emitted its scalar mirror (the `N % W` tail loop, which
doubles as the runtime dependence guard's fallback) even when a constant trip
count made it unreachable: 17 instructions of guard, resume-index arithmetic,
materialised stream pointers and a dead scalar body **per vectorized loop**.
`transform_map_vector` now omits it when `N % packed_width == 0`,
`pattern.guarded_streams` is empty and the escaping counter can be re-pointed
at the trip count (materialised in the preheader with the mirror's own
`Cast`-to-`iv_ty` form, so it works in bare-`Value` use positions too).
Measured at `-O2` on upstream `main` `93f2a43b` (A/B against the kill
switch): `sha256_transform` **151 → 135** insns / 20 → 19 rrmov / 1 → 0
stkref, a 16-element `unsigned` copy 61 → 28, `shape_u32_exact` 39 → 18.
Pinned-oracle targets for the same function: GCC 16.2 = 142, Clang 23.1 =
129. Kill switch `CCC_NO_MAP_ZERO_REM=1`.
Evidence: [`engineering/evidence/ZERO-REM-1/README.md`](engineering/evidence/ZERO-REM-1/README.md);
gates `tests/regression/check_vec_dead_remainder.sh` +
`tests/regression/check_vec_remainder_shapes.py` +
`tests/regression/vec_dead_remainder.c` and the assembly-shape fixture
`tests/regression/vec_shapes/vec_dead_remainder_shapes.c` (a subdirectory, so
`run_regression.py`'s program corpus does not try to link it).
Not done: the reduction and stencil vectorizers build their own tails; the same
constant-trip-count reasoning applies there.

### SPILL-01 · **TOOL LANDED 2026-09-28** — causal stack-reference census
`ra_quality_census` counted stack references; nothing said *why* they exist.
`RegAllocResult` now publishes the allocator's own `eligible` set and
`src/backend/stack_layout/slot_census.rs` turns it into a per-slot cause
(`alloca`/`address`/`wide`/`spill`/`nongpr`/`temp`, opt-in `CCC_SLOT_CENSUS=1`);
`scripts/stack_census.py` joins it with the POST-PEEPHOLE assembly and
attributes every emitted stack reference (plus `csave`/`argout`/`incoming`).
Corpus gate met: **98.74 %** coverage (1251/1267) vs the 95 % criterion.
Finding that re-orders the queue: **44.4 % of stack references are
callee-save save/restore**, 23.8 % structural (alloca/address/wide/nongpr) and
only **10.7 % are spills** — and `sha256_transform`, the RA-PRESSURE-3 target,
has **zero** spill references.
Evidence: [`engineering/evidence/SPILL-01/README.md`](engineering/evidence/SPILL-01/README.md).

### RA-CSAVE-1 · **NEW 2026-09-28** — callee-save save/restore traffic
Opened from the SPILL-01 census: 562 of 1267 stack references (44.4 %) are
callee-saved registers being saved on entry and restored on exit, more than
spills and allocas combined. Every register taken from the callee-saved pool
costs two stack references per call. Before touching eviction policy: measure
how many of those registers are *used* after allocation (`[RA-STATS]
callee-homes=`) versus saved defensively, and whether the corpus's hot
functions would rather spill a caller-saved value. Done = a census-driven,
output-checked change that reduces `csave` references on the corpus without an
instruction-count regression, or a documented bound.

### RA-PRESSURE-3 · **RE-SCOPED 2026-09-28** — sha256 round loop
The historical framing ("land a generic phi-copy cycle resolver") is
obsolete: a general resolver already ships (Kahn decomposition in
`plan_edge_copies`, exhaustive semantic-equivalence test over EVERY
parallel-copy graph for n=2..5, default-on policy, and
`check_phi_acyclic_order.sh` pinning rotation < legacy statically).
Re-measured at `-O2` on `6f8ace9c`: `sha256_transform` is **148 insns /
14 rrmov / 10 stkref** (GCC 142/19/8, Clang 129/24/26) — not the
historical 218 insns / 64 spills, and lccc now has FEWER stack
references than Clang. The residual gap is instruction selection and
*physical* rotation, not copy resolution.
New sub-problems: (1) express the rotation as a register permutation
(rotate-by-immediate / `rorx`-class) rather than copies — check
`ui: alu/phi` admission, see `span_recurrence`; (2) 2×/4× unroll so the
rotation runs once per body; (3) close the 19-insn gap to Clang.
Negative space unchanged: the valve count-coupled supply is falsified
(W2 09-11); RA-06 intra-block splitting-measurement is recorded dead
(W1 09-05).

### RA-PRESSURE-4 · Aggregate/struct register pressure — `struct_copy` (1.15–1.45×)
156 insns vs clang 76 (**77 spills**). `CCC_NO_AGGREGATE_SPLIT` escape.
Done = ≤1.05× on Raptor-Lake report.

### PF-CHACHA-1 · chacha20 ARX schedule — remaining ICX gap (118 insns vs icx 74 @-O2, icx 63 @v3)
Refreshed 09-27 (oracle, static counts): PR #638's ARX work already took
the `-O2` body from the old 592 and the v3 body from 222 down to **118**
(oracle: lccc 118 < clang 176 < gcc16.2 180; icc 1104). The remaining gap
is ICX only (74 @`-O2`, 63 @v3): the v3 ARX-lane schedule (superword diag +
horizontal ARX; W2 09-08) plus ICX's tighter quarter-round folding at base.
Done = v3 closer to ICX's 63, `-O2` static ≤1.15× of ICX's 74 (runtime
parity on the author's box no longer counts as done — keep static and
runtime numbers separate).

### PF-SCHEDULER-1 · sha256 message-schedule vectorization
GCC vectorizes the sliding-window expansion even at -O2 (`m[i-2..i-16]`
SIGTA → SSE). Blocker: strided/sliding recurrence — unaligned vector
loads at constrained offsets + sliding-window SLP (`vec_arx` /
`arx_vectorize` first-light: `s10/next/solve/split` → v-slp flags).
Done = schedule vectorized at -O2, kernel ≤1.15×.

### PF-TLS-1 · TLS segment access — **DIRECT LOCAL-EXEC FORM LANDED (2026-09-28)**
Legal Local-Exec accesses now lower to ONE instruction,
`%fs:symbol@TPOFF(+N)`, instead of base materialization + access.
Measured at `-O2` on `6f8ace9c` → this tree (`ra_quality_census`):
`set_all` 54 → **36** insns, `sum_all` 41 → **28**, a 3-store `set`
12 → **6** (byte-identical to GCC), benchmark `tls_pass` 34 → **30**
(GCC 37, Clang 31), whole focused TU 123 → 91.
Legality rule (measured against GCC, not guessed): Local-Exec is a
link-time constant, so it is used for every executable — PIE included —
but **never** for `-shared`, and only for a symbol this module owns
(extern TLS keeps Initial-Exec, as GCC does). The backend therefore
carries a new `shared_lib` flag: `pic_mode` alone cannot tell `-fPIC`
from `-shared`, and the two have opposite legality here.
Two prerequisites were repaired first, both reproducible on the review
baseline: (1) the assembler dropped the relocation modifier for
`sym@MOD+N` (`R_X86_64_32S` against a symbol literally named
`sym@TPOFF` — a silent wrong address; lccc's relocations now match GNU
as 2.47 exactly); (2) `lccc -fPIC -shared` could not link ANY
`static __thread` ("R_X86_64_TPOFF32 ... can not be used when making a
shared object") because shared output used Local-Exec — it now uses
Initial-Exec, verified by a `dlopen` round-trip.
Runtime: **not established** — the scaled `tls_seg_access` screen
(`-DPASSES=20000000U`, 7 interleaved rounds, output identical to GCC)
has a per-binary spread of 0.67–0.94 s, so the before/after medians are
inside the noise. UNVERIFIED ON TARGET.
Evidence: [`engineering/evidence/PF-TLS-1/README.md`](engineering/evidence/PF-TLS-1/README.md);
gates `tests/regression/check_tls_model_selection.sh` +
`tests/regression/tls_local_exec_direct.c`.
Remaining: two of three `&tls_slots` computations still not CSE'd; a
variable-index TLS read still pays the base (unavoidable); `%gs:` and
i686 coverage not attempted.

### PF-CLS-1 · Byte-classifier chains (expat_xml_scan 1.31–1.37×)
**Re-measured 2026-09-28 at `-O2`: `expat_utf8_name_length` is 70 insns
vs gcc 79** — lccc is now statically AHEAD, so the 1.31–1.37× runtime
gap is pure branch/layout behaviour and instruction-count work here is
wasted effort. Use `scripts/callgrind_ab.py` (deterministic `Ir`,
I1/LL misses, branch mispredictions) or a target-CPU PMU run; this
host's wall-clock noise floor is ~15 %.
Historically: 79 vs gcc 78 insns — branch-prediction/scheduling-bound,
not insn count. Structural block: `a||b||c` last-member critical edge + shared
increment block starve if-conversion (two attempts reverted W1 09-01g).
**Split the critical edge on the last member first**; the counting
spelling `pred && n++` needs the same `Select`s as the boolean one.
Sub-items: (1) length-table load DCE; (2) char-class increment-starved
`vectorize` falling back to `if_convert` (1.17×): sequence-first
vectorization.

## Tier 2 — MachInst / isel

### MI-CLOBBER-1 · Clobber modelling, then lower `Call` (7.8 % of insns)
Correct-by-rejection today (the flush boundary *is* the clobber model).
`CallTyped` carries declared clobbers (W1 09-02e); extend per-instruction
clobbers before lowering `Call` — adding `Call` uses before that is a
miscompile.

### MI-XMM-1 · Vector/FP register class (1.3 %)
119 rejected `Store(float)`. MachInst models only the 16 GP families.

### MI-PARAM-1 · Remaining `ParamRef` (1.1 %)
No-code subset landed (963 → 96). Remainder is emissive: alloca-homed
params and stack-passed args; the fallback reads the parameter's
**incoming** register even when a caller-saved pre-store of a different
parameter aliased the name.

### MI-ROTATE-1 · Sub-word rotates
Native `RotateLeft/Right` landed (W2 09-07). Open: sub-word rotates
whose complements meet only at the narrow width (truncation-aware
pattern / a consuming `Cast(i32→u16)` rewrite). Count-staging polish is
cosmetic; do not chase.

### MI-ENCODE-1 · Encoding-level differential
Suite at 7 layers/36 tests incl. execution vs GAS 2.47; byte-level arm
via `scripts/insndiff.py` / `encoding_diff.py` + GAS 2.47 (INF-GAS-1).

## Tier 3 — verifier / infra

### FE-RELRO-1 · CLOSED — multi-level pointer const leaked into `.data.rel.ro` placement (2026-09-27)
The parser OR-ed `decl_flag::POINTER_CONST` across every star of a
declarator, so `short * const * gp` (outer object **writable**) was
classified read-only and `classify_global` routed it to `.data.rel.ro`;
RELRO page-protection then made the first run-time store SIGSEGV
(Csmith seed 20260928, differential vs GCC at -O0; wild store to the
globals page, gdb-confirmed read-only mapping at run time). Fix:
each star iteration now *clears* the flag, so only the identifier-
adjacent (outermost) level's const survives — which also fixes
cross-declarator leakage (`int * const a, *b;` made `b` read-only).
Regression: `tests/regression/pointer_const_multi_level_relro.c`
(SIGSEGVs on the pre-fix build; byte-compares vs GCC).
Do-not-reopen: the single-star `T *const p` case is
`pointer_const_data_rel_ro.c` (kernel zstd contract); the volatile
sibling (`short * volatile * gp` marks the declaration volatile —
conservative, safe, missed-opt only) is deliberately NOT changed in
the same commit; measure before touching it.

### VER-PHIARITY-1 · CLOSED — φ-arity contract is two-sided: reachable ⊆ named ⊆ static (2026-09-27, refined after corpus run)
phi elimination materializes one edge copy per (reachable pred,
incoming) pair and never visits unreachable blocks, so (a) a statically
present but dead predecessor legally has no φ incoming — the frontend
lowering leaves such dead edges routinely — and (b) an entry for a dead
predecessor is inert (the edge copy sits on a path that never runs;
switch/loop lowering produces these, e.g. switch_dispatch's 16-pred
join). The first cut of the reachable contract enforced EQUALITY with
the reachable set and false-positived on 15 corpus files, all of the
`extra [dead-pred]` class. Final contract, identical in BOTH checkers:
every REACHABLE predecessor must be named (missing = live-edge defect,
the miscompile class), and only real CFG predecessors may be named
(stray = carelessly retargeted edge); an entry for a dead predecessor
sits legally in the gap. GLA check 7 (src/backend/location_alloc/
verifier.rs) and the `CCC_VALIDATE_SSA` PHI-ARITY check (src/passes/
mod.rs) implement it with unit tests for both sides. Do not weaken the
reachable-preds coverage side: that is the real defect detector
(Csmith 20260981's compile abort). Also added a
`lowering:entry` validator checkpoint (at -O0 the optimizer loop is
empty, so without it a frontend IR defect was only visible at
`backend:pre-eliminate_phis`) and dump-before-validate ordering there.

### PHI-SINK-1 · CLOSED (superseding entry) — composed with #645; profitability = live phi home (2026-09-27, S66)
The unguarded sink regressed `spectral_norm` (289 > 287 budget): sinking the
accumulator's add across the body/latch boundary separated it from its mul
and lccc lost `vfmadd231sd`. Final rule: sink ONLY when the phi home is
live across the window — an instruction between definition and relay (same
block) or in the strictly-dominated region reads or writes the home (the
rotation's `f <- e` copy). Dead-home accumulator shapes are refused; the
allocator coalesces them anyway and the fold would only distance the
computation from its operands. Both behaviors unit-pinned
(`computed_incoming_folds_to_its_copy_slot`, `dead_home_computed_incoming_is_not_sunk`,
`fold_sinks_definition_without_touching_readers` — rewritten after it was
found to pass vacuously under refusal). Composed with #645 (Agent B's
rotation lags + residency guard + lea addend fold — audit:
`engineering/AUDIT-PR645-reassoc-latency.md`): rot 55/3 → 53/3, sha256
144→142, corpus −5 insns/−5 rrmov with 183/186 functions identical, full
lib 3621/0. Gate ratchet: escape-off census pinned to the exact composed
shape (k_i ≤ 53 / k_s ≤ 3); the beat-gcc and beat-legacy-by-10 contract
unchanged. Do-not-reopen: the predicate gates profitability only —
correctness always stays with the RA's disjoint-interval coalescing rule.

### CC-O0CALL-1 · **CLOSED 2026-09-28** — secondary-cache (`%rcx`) clobber by GP argument 4
The previously proposed cause (`-O0` accumulator-address loads trusting a
non-SSA cache hit) is **falsified**: applying that guard leaves Csmith
20260945 SIGSEGVing, and its regression test passes on the broken
baseline, so it does not discriminate.
Actual cause: `%rcx` is both the secondary value cache and SysV's fourth
GP argument register. Argument 4 staged a scalar into `%rcx` without
invalidating the cached pointer identity, so the next (aggregate)
argument rematerialized the "pointer" from `%rcx` and dereferenced the
scalar that had just been written — `movq %rcx,%rax; movq (%rax),%r8`
with a NULL base. Independent of the peephole and of GLA (also fails
with `CCC_NO_PEEPHOLE=1` and `CCC_RA_GLOBAL_LOCATION=0`).
Fix: invalidate the secondary cache after a GP argument writes ABI slot
3 and before the next argument is staged, for every classification that
can write it (scalar, i128/i64 register pairs, by-value aggregates, both
mixed struct classes); the next-iteration placement also covers the
early-`continue` staging paths.
Evidence: reduced reproducer
`tests/regression/call_secondary_cache_clobber.c` (8 ABI shapes), gate
`tests/regression/check_call_secondary_cache.sh` (20 output-checked
arms: `-O0..-O3/-Os` × {default, no-peephole, no-GLA, both}), the full
Csmith seed 20260945 now prints `checksum = 39DEBCF9` / exit 0 like GCC,
and GCC `-fsanitize=address,undefined` is clean on the reduced case.
Historical reproducer kept:
`artifacts/repros/miscompile_csmith_20260945_-O0.c`.

### INF-HARNESS-1 · Subtract startup / scale short benchmarks
lz4's 1.94× hid a 10× work gap (~2 ms fixed cost in a 7 ms measure).
Report rule: no benchmark with fixed cost > 10 % of measured time
reported bare; short kernels scaled or decomposed.

### INF-BENCHGATE-1 · Benchmark output gate unconditional in CI
`scripts/check_benchmark_outputs.sh` runs every `tests/benchmark/
programs/*.c` at all four levels (~4 min, no timing) — a miscompile
once passed all regression tests because those kernels were only ever
built by the timing harness.

### INF-FRESHCLONE-1 · CI builds from a fresh clone
Historical failure mode: files missing from a commit while the author's
tree was fine.

### INF-GAS-1 · GAS 2.47 re-provision per session
`scripts/ensure_gas_247.sh` — caches/snapshots do not persist it;
`/usr/bin/as` 2.44 is **not** an oracle.

### INF-LINK-1 · Linker oracle pins
lld 23.1 (`release/23.x`), mold 2.42.1, bfd 2.47 — verify versions
before trusting a prebuilt (W3: a stale `wild` 0.7.0 produced two false
lccc failures). Setup: `tools/linker/setup_oracles.sh`,
`tests/linker/setup_oracles.sh`.

---

## Closed this cycle (do not re-open without new evidence)
<!-- durable here for grep-ability; narrative in engineering/journal/2026-09-W2.md and 2026-09-W3.md -->

VER-DOM-1 (def-dominates-use in the IR verifier — landed as
`src/passes/verify.rs` checks 7 & 8: SSA single definition and
def-dominates-use over an interval-encoded dominator tree, with the
diamond/loop/terminator test family in `verify/tests.rs`; the 2026-09-27
triage found this item stale — the dominance check has been in place
since the IR-verifier birth arc);

`__builtin_memcpy` → native Memcpy + the two latent aliasing fixes it
exposed; chacha20/ARX 10.01× → ~1.05×; `iv_widen` constant-scale firing
(sieve −21.6 %); order-preserving block layout; `CCC_VERIFY_IR` 0
violations/6 levels; machine-level loop inversion (memchr → parity);
S05 inline-single-site-static; S06 cmp-branch-fusion (4 holes, 16
tests); S07 ifcombine profitability (+4.7 % lz4); S09 vectorizer
cond-store use-without-def; S10 vectorizer IV-live-out (`wsum=511`);
bool-pair tail-jmp; loop-memset (lz4 −1.75 % Ir); native rotates; imm32
bit-pattern hoisting; duplicate `GlobalAddr` merge (expat −23.3 %,
lz4 −15.3 %); RA web-wide in-loop-use supply (+3.63/+4.33 % sha256,A/B)
+ boolean-only valve supply; phi acyclic copy order (opt-in
`CCC_PHI_ACYCLIC_ORDER=1`); SROA split `0.0`/`-0.0` bit-exact zero
(CG-07); epilogue-suffix sharing (CG-09); OP-42 transactional sinking;
RA mode-6 stays opt-in (RA-28/28b); TLS CSE merge (3→2).
