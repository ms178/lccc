# FP loop header lookup, SSE2 store widths and RV64 immediate fields

## Baseline and procedure

Started from merged `ea061b5d34028740fda1ab77fcce7559a2755362` (including the
preceding label-reference work). No broad local CI rerun: targeted checks and
measurements run locally, and GitHub CI owns the broad suite. Local builds use
Rust 1.97.0's verified transport cache, `fastbuild -j2`, and active 8 GiB swap.
Hosted CI must check the current stable Rust separately. Reference compilers
are GCC 16.2.0 and Clang 23.1.3; GNU assembler/linker/inspection tools are the
verified-source 2.47.20260726 build. No local ICX payload or PMU is available.

## MB-20: another quadratic prefix scan

`promote_loop_invariant_fp_load` searched every line preceding each jump for
its first matching label, including forward jumps that could never be loop
back-edges. Cache a label index lazily once per invocation, retaining the first
occurrence and requiring its position to precede the jump. This pass changes
only instruction text, not label identities, positions or classifications.

The initial 15-pair CPU-pinned experiment (two excluded warmups) measured:

| Functions | Before median, seconds | After median, seconds | Before/after |
| --- | ---: | ---: | ---: |
| 40 | 0.243943 | 0.234882 | 1.039 |
| 120 | 0.841302 | 0.744582 | 1.130 |

Callgrind on the same fixed inputs, with identical output assembly:

| Functions | Before Ir | After Ir | Reduction |
| --- | ---: | ---: | ---: |
| 40 | 3,224,400,549 | 3,101,676,030 | 3.81% |
| 120 | 10,781,171,046 | 9,608,985,532 | 10.87% |

The indexed pass's inclusive 40-function cost is now about 0.12%, compared
with the preceding profile's approximately 4%. The peephole still consumes
72.33% in this new profile, so the ≤35% MB-20 criterion remains **open**.
These are compiler-cost numbers, not generated-program speedups. Both inputs
satisfy the bounded 3% Ir non-regression check. The measured x86-phase compiler
and final compiler are recorded separately by hash; adding the independent
RV64 range helper does not turn the earlier binary into the final binary.

A separate 15-pair confirmation using the final compiler (including the RV64
helper) measured 0.242943 → 0.229257 s for 40 functions (1.060×) and
0.842614 → 0.744973 s for 120 (1.131×). Its raw samples and final binary hash
are in `final-compile-cost.json`; the earlier phase and its profiles are kept
rather than silently relabelled as the final executable.

All 94 assembly outputs are byte-identical on the final compiler, with zero
compile failures. This includes gzip `longest_match`; the sampled gzip runtime
ELF is also byte-identical to baseline, so its stack traffic cannot increase.

## SSE2 correctness: a partial store is not a full value definition

The same pass matched `StoreRbp { reg: 0, offset, .. }` without checking size,
then rewrote the store as `movq %rax, %xmm2`. A byte/word/dword memory store
preserves the other slot bytes. The later MOVSD reads all eight bytes, which
cannot be reconstructed from the full RAX value. Restrict the preheader store
to `MoveSize::Q`. Also require both edited participants to be unpinned before
committing either replacement. Preserve the existing slot-stability,
single-entry and FP-liveness proofs and the textbook positive control.

Before those fixes, both new production-pass regressions fail: a byte store
becomes a full GP-to-XMM move, and a pinned preheader store is rewritten.
Afterward all four partial-store variants (AL/AH/AX/EAX) and both pinned
participants are preserved. A native x86-64 SSE2 test executes the partial
stores followed by MOVSD/MOVQ and confirms the preserved bytes against an
independent integer model. It is an executed instruction-contract model,
not a claim that arbitrary hand-written assembly enters the C frontend.

## RV64 manual review: do not mask an immediate into another instruction

The official RV64I text at the source hash retained in `manual-source.json`
uses six shift bits; W forms reserve encodings with imm[5] nonzero. The
assembler cast signed immediates to `u32` and masked them, silently accepting
negative, overflowing and out-of-range source operands. Validate before the
cast for SLLI/SRLI/SRAI/RORI and their W forms and immediate aliases. Register
count shifts retain their existing semantics.

All 384 legal canonical encodings agree byte-for-byte with Clang 23.1.3.
Both assemblers reject all 64 invalid canonical inputs. The previous compiler
accepted all 64; running the new oracle harness on that binary fails all 64,
so the harness is itself mutation-checked. Rust tests additionally cover
128 invalid canonical/alias cases, all valid encoding fields and register-count
controls. A 24-function C compile covers i128 shifts across 0/1/31/32/63/64/65/127
without generating an invalid RV64 immediate. This is assembly/codegen evidence,
not RISC-V runtime execution or a full extension/ABI/linker certification.

## Mutation and focused local outcomes

* All 26 loop-hoist tests pass, including the new native model and both
  correctness regressions; all three header-index tests and all three RV64
  immediate-range tests pass.
* Rebuilding the index at every jump fails the budget (256 versus 1);
  accepting forward targets fails the old-scan equivalence test. Removing the
  RV64 signed-range check fails its negative test. Restored tests pass.
* `lccc`, `lccc-ld` and the RV64 driver build with fastbuild and two jobs.
* No feature flag, warning suppression, fixture weakening or audit-budget
  increase is used. Hosted broad results must come from the new PR's head.

## SSE2 runtime triage: output-checked, amplified, no claimed speedup

The original roughly-sevenfold gap is not established by the three workloads
below, and this change does not resolve it. An initial default-size screen was
too short for firm conclusions, so it is not used as performance evidence.
The retained canonical-runner experiment uses `-O2 -march=x86-64`, randomized
CPU-pinned 15-pair rounds and two excluded warmups. All four arms (baseline,
candidate, GCC, Clang) have every timed sample above 200 ms; outputs and exits
agree. Sizes are explicitly recorded, not hidden compiler special cases.

| Workload | Size override | Candidate/GCC paired median | Candidate/Clang paired median |
| --- | --- | ---: | ---: |
| matmul | N=1024 | 1.021 | 1.017 |
| spectral_norm | N=3072 | 1.275 | 1.008 |
| gzip_longest_match | PASSES=131072 | 1.080 | 1.140 |

Candidate and baseline ELFs are identical for all three inputs. Their paired
ratios' confidence intervals include 1; this is not a program-runtime win.
The GCC spectral-norm inner loop uses packed `cvtdq2pd/divpd/mulpd` followed by
ordered scalar additions, while LCCC retains scalar `cvtsi2sd/divsd/mulsd`.
That is a concrete next vectorizer hypothesis: pack independent terms while
preserving reduction order. It is not permission to reassociate FP sums or
to infer causality from instruction count alone. No ICX runtime comparison,
all-workload superiority or hardware-counter conclusion is claimed.

Compact records and raw samples are in
`engineering/evidence/fp-loop-headers-2026-10-24/`. Larger binary/profile and
raw-run artifacts remain under `/home/user/artifacts/audit/`. Reproduce runtime
rows with `tests/benchmark/run_benchmarks.py`, selecting the named workload,
all three compiler keys, the labelled baseline arm, the flags/override above,
`--reps 15 --warmup 2 --strict --no-pmu-probe`, and JSON/artifact destinations.

## Architecture/manual boundary

The live cross-architecture ledger is
[`../subsystems/instruction-audit.md`](../subsystems/instruction-audit.md).
The x86-64 and RV64 contracts above are new bounded audit work. i686,
AArch64, the remaining Intel/AMD/Arm full text, other instruction families,
vectorizer/ABI/assembler/linker interactions and target cost models remain
explicitly open. No PDF extraction count or green CI converts those obligations
into completed semantic reviews.
