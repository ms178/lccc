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
The i7-14700KF target has **not** been measured. Before accepting a new
runtime improvement, use same-window, same-baseline, output-checked paired
A/B with ≥ 200 ms/arm and agreeing median/min, not historical ratios.

An item with no reproducer does not belong here.

---

## P0 — largest measured gaps

### PF-LZ4-1 · Byte-copy semantics fixed; byte-compare still open
The default-on v2 copy matcher covers both relevant LZ4 loops. The old
unconditional `memmove` rewrite was **incorrect** for forward-overlap smear,
and naively disabling the pass caused a 10.8× scaled-workload slowdown on the
Xeon VM. A guarded memmove fast path now preserves the scalar overlap loop;
25 paired rounds against latest `main` show **no significant runtime change**
(median 0.9946, minimum 1.0057, p=0.2301), and 38 other workload `.text`
sections are unchanged. The separate word-at-a-time byte-compare proposal
remains **open**: require a proof against overread and a target i7-14700KF
paired A/B before shipping. Details: red-team report linked above.

### PF-MB-1 · Mandelbrot hot FP loop refuses vectorization (open)
Current `7ddb770f` VM ratio to GCC is 1.286; candidate and baseline
executable `.text` match exactly. Additional non-IV recurrences need a legal
SSA/exit-value proof before widening; the previous 1.67× figure is historical.
Reproducer: `tests/benchmark/programs/mandelbrot.c`. Done = proven vectorized
loop plus output-verifying paired A/B on the i7-14700KF.

### PF-FB-1 · `linux_find_bit` loop structure (open)
Current `7ddb770f` VM ratio to GCC is 1.318; candidate and baseline
executable `.text` match. `bsfq` is present; the remaining gap is branching
shape, not the loop-copy idiom. The old 1.40× ratio was historical.
Done = classified general CFG improvement with output-checking and target-CPU
paired evidence, or a justified bound showing it cannot improve.

### RA-GLA-04 · GLA Phase 2 — full-identity register remat (sha256 hot)
Phase 1 shipped (`CCC_RA_GLOBAL_LOCATION=1`); source-less remat is default-ON.
The checked Phase-2 *spill-gap* experiment was **not** shipped: paired
`sqlite_varint` and `expat_xml_scan` regressed 1.60% and 4.51% respectively
on the earlier VM despite a few better static counts. The current-main
`sha256_transform` VM ratio to GCC is ~1.175, with identical candidate/main
`.text`; no allocator speedup is claimed. Require post-allocation slot-traffic
feedback before a small, verified rematerialization change. Next: full remat
— register-source spans that rematerialize instead of load.
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
