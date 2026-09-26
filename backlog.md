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
Oracle targets: `sha256_transform ≤ 1.5×`, epilogue ref-count −50 %.
Design refs: [`engineering/DECISIONS.md`](engineering/DECISIONS.md)
RA-GLA-01/02/03. Aligns with **R3** (RA span supply + phi/ORI lowering —
skip the 1.93× sha256 without tripling the web count; compare
GHASH/mulpack for the multi-source shape rule).

## Tier 1 — measured, largest first

### RA-PRESSURE-3 · Phi-copy cycle resolution — sha256 round loop (1.73–1.80×)
218 insns vs clang 126 (83 loads/25 stores/**64 spills**, 15 branches).
Sub-problems (kill-switch separately): (1) express the cyclic rotation
web as register permutation — check `ui: alu/phi` admission, see
`span_recurrence`; (2) 2×/4× unroll so the rotation runs once per body.
Negative space: the valve count-coupled supply is falsified (W2 09-11);
RA-06 intra-block splitting-measurement is recorded dead (W1 09-05).

### RA-PRESSURE-4 · Aggregate/struct register pressure — `struct_copy` (1.15–1.45×)
156 insns vs clang 76 (**77 spills**). `CCC_NO_AGGREGATE_SPLIT` escape.
Done = ≤1.05× on Raptor-Lake report.

### PF-CHACHA-1 · chacha20 v3 ARX schedule (222 insns vs icx 63)
plain-`-O2` at parity; the `-march=x86-64-v3` ARX-lane schedule is the
gap (superword diag + horizontal ARX; W2 09-08). Done = v3 ARX closer
to ICX, plain-`-O2` ≤1.05×.

### PF-SCHEDULER-1 · sha256 message-schedule vectorization
GCC vectorizes the sliding-window expansion even at -O2 (`m[i-2..i-16]`
SIGTA → SSE). Blocker: strided/sliding recurrence — unaligned vector
loads at constrained offsets + sliding-window SLP (`vec_arx` /
`arx_vectorize` first-light: `s10/next/solve/split` → v-slp flags).
Done = schedule vectorized at -O2, kernel ≤1.15×.

### PF-TLS-1 · TLS segment access — remaining dups
2.19× → 1.06× landed. Remaining: two of three `&tls_slots` computations
not CSE'd; prologue may stage the thread pointer. Next step: direct
`%fs:symbol@TPOFF` lowering. Done = ≤1.05×, every dynamic-offset TLS
read one `%fs:` operand.

### PF-CLS-1 · Byte-classifier chains (expat_xml_scan 1.31–1.37×)
79 vs gcc 78 insns — branch-prediction/scheduling-bound, not insn
count. Structural block: `a||b||c` last-member critical edge + shared
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

### VER-DOM-1 · Def-dominates-use in the IR verifier
Six structural properties clean; dominance is the uncovered one (how an
SSA violation once shipped). Cooper-Harvey-Kennedy over RPO next.

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
<!-- durable here for grep-ability; narrative in engineering/journal/2026-09-W2.md -->

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
