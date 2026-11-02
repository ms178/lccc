# LightPixel / unsigned dword SLP audit — 2026-11-01

## Scope and outcome

**Historical component-only report.** See
[the joint graph follow-up](LIGHTPIXEL-JOINT-AUDIT-2026-11-01.md) for the
subsequent complete integer tail implementation and current measurements.
The results below describe the earlier implementation, not the current one.

This is a **component improvement, not a completed implementation of LLVM's
three-lane reduction/store profitability optimization**. The exact RGB clamp
and alpha-max shape remains scalar in lccc. No Firefox/rasterflood-svg browser
benchmark was run. We cannot claim a Firefox speedup or overall superiority
to the named compilers.

The measured improvement is general unsigned 32-bit min/max lowering for BB
SLP: native `pminud/pmaxud` at 128 bits with SSE4.1 and `vpminud/vpmaxud` at 256
bits with AVX2, rather than sign-bit flips plus compare/blend. The SSE2
fallback and the signedness checks remain intact. This also improves the
four/eight-channel widened multiply/logical-shift/clamp subproblem.

Baseline: main `60a2e2c1a7c649a7eb68f7e5ac93206c51357904` (rebased before work).
Raw receipts, assembly bodies, compiler versions and all 1,920 timing samples (two runs)
are in `lightpixel-evidence-2026-11-01.json`.

## Motivating sources

- [LLVM PR 227303](https://github.com/llvm/llvm-project/pull/227303),
  “[SLP] Account for reduction consumers in profitability”: evaluate the
  complete reduction and its compatible consecutive-store consumers rather
  than rejecting the three-lane tree and four-lane store in isolation. This
  does not enable non-power-of-two vectorization by default, and does not
  justify store credit across intervening accesses or nonconsecutive sinks.
- [LLVM issue 46312](https://github.com/llvm/llvm-project/issues/46312),
  missed unsigned min/max SLP in Firefox's lighting tail.
- [Firefox FilterNodeSoftware.cpp at bc1221a6f3f7468380629ca90b4e40ce045ab8ae](https://github.com/mozilla-firefox/firefox/blob/bc1221a6f3f7468380629ca90b4e40ce045ab8ae/gfx/2d/FilterNodeSoftware.cpp#L3750-L3790):
  `SpecularLightingSoftware::LightPixel` still multiplies the B/G/R byte
  components by `specularNHi`, shifts by the output precision, clamps to
  255U and sets alpha to their unsigned maximum. The source uses a local
  packed color union and returns the packed value. The fixture isolates the
  precision-15 integer tail; it does not reproduce lighting geometry,
  floating-point normalization, the power cache, or the browser workload.

The product is **uint32 wrapping multiplication**, even for an arbitrary
full-width scale. Widening the product without truncating before the shift
would change the answer. A three-byte source cannot be replaced by a
four-byte load merely because the destination has four bytes.

Clang 23.1.4 was tested both normally and with
`-mllvm -slp-vectorize-non-power-of-2`. These are released-compiler tests, not
a build of the proposed LLVM patch. Both produced identical instruction
counts for these fixtures. Intel PPA and Godbolt are outside this sandbox's
network allowlist; no local ICX or remote Godbolt result is claimed. The
existing authenticated ICX 2026.2 CI lane runs the new correctness gate.

## Implementation and safety boundaries

- Four unsigned min/max intrinsic opcodes carry explicit 16/32-byte home,
  result and purity metadata, with register-allocation, operand homing,
  commutative memory folding and x86 lowering wired together.
- The four-lane x86 lowering asserts SSE4.1 permission; the eight-lane
  lowering asserts AVX2. Selection checks the **comparison's signedness**,
  not just lane width. Top-bit-set U32 values must never use signed min/max.
- AArch64 has the matching 128-bit `umin/umax` lowerings. Its existing
  unsupported-256-bit path remains unsupported. No non-x86 SLP expansion or
  native AArch64 performance claim is made.
- Existing scheduling, volatile, alias, external-use and profitability
  restrictions are not relaxed. No speculative extra input byte is loaded.
- The ordinary loop-map min/max selector is unchanged; the new selection
  is in BB SLP only. There is no whole-program or all-target optimality claim.

## Correctness and regression gates

`check_lightpixel.py` compiles the **harness and independent oracle with the
reference compiler at O0**, and compiles only the candidate kernels with the
compiler under test. Its oracle uses uint64 multiplication and an explicit
low-32-bit mask before the shift. It checks all outputs, not a lossy summary.
Each invocation tests O2/O3 at SSE2-only, SSE4.1-without-AVX and AVX2.

Each of the six executables runs 65,536 input/scale cases across 12 kernels:

- 16 fixed scales, all 256 values in each byte position via permutations;
  the remainder uses a deterministic full-width xorshift stream;
- scale boundaries include zero, Q15 neighbors, `UINT32_MAX`, the sign-bit
  boundary, and multiplication-wrap boundaries;
- in-place, separate-source and packed-value RGB/alpha variants;
- input objects of exactly three bytes and four-byte outputs placed at both
  edges of accessible pages, surrounded by inaccessible pages;
- seven overlapping source/destination placements, four interfering-store
  aliases, scattered stores with checked padding, and volatile byte accesses;
- four/eight-lane full-U32 min/max and wrapping multiply/shift/clamp, including
  operands on opposite sides of the signed boundary.

`--require-native` additionally requires native unsigned min/max in the
widened kernels, refuses SSE4.1 unsigned min/max in SSE2-only output, and
checks that the volatile control retains exactly six byte reads/four byte
writes. These are positive and negative codegen gates, not timing gates.

Local results:

- All six configurations pass for baseline, candidate, GCC 16.2.0,
  Clang 23.1.4 and Clang 23.1.4 with non-power-of-two SLP enabled.
- Candidate also passes with the harness built by Clang 23.1.4.
- GCC 16.2 UBSan: all 65,536 cases pass without a diagnostic.
- Warning-denied fastbuild lccc/lccc-ld, Rust units: **7,148 passed,
  10 ignored**; warning-denied Clippy passes.
- Python SIMD runner contracts: **22 passed**.
- Existing scoreboard: **134 checks pass**, including injected clock failure.
- Existing AoS default and guarded mixed-sign stride-four suites: **63 pass**.
- CI lane-plan self-test: **18 rules / 25 fixtures pass**; workflow partition
  validation passes. No existing test expectation was relaxed.

This is adversarial regression coverage, not exhaustive enumeration of all
2^32 scales, all RGB triples, all alias graphs, or all execution environments.

## Generated code, O2 / AVX2

Counts are static AT&T instruction lines, including `ret`, excluding labels,
directives and comments. They are not micro-op counts or timing predictions.

| Kernel | lccc before | lccc after | GCC 16.2 | Clang 23.1.4 |
|---|---:|---:|---:|---:|
| in-place RGB/alpha | 33 | 33 | 27 | 25 |
| separate RGB/alpha | 31 | 31 | 29 | 25 |
| packed-value RGB/alpha | 37 | 37 | 32 | 30 |
| widened clamp, 4 lanes | 13 | 10 | 9 | 8 |
| unsigned min/max, 4 lanes (each) | 10 | 4 | 4 | 4 |
| widened clamp, 8 lanes | 14 | 11 | 10 | 9 |
| unsigned min/max, 8 lanes (each) | 11 | 5 | 5 | 5 |

The four-lane min is now one load, one memory-folded `vpminud`, one store and
return, matching the two reference compilers' instruction count. The widened
clamp still has extra setup instructions compared with both references.

## Runtime method and limits

`benchmark_lightpixel.py` builds the driver with GCC 16.2 at O2 and links
separately compiled kernel objects (no LTO). Every measured hash must match a
separate GCC O0 kernel object. All arms use O2/AVX2. Each sample performs
2,097,152 external calls over cache-resident data; input generation, call and
hashing machinery are common to every arm. There is one warm-up per arm and
kernel, then 24 rounds, reversing compiler order on alternate rounds, pinned
to the first available CPU. Hardware advertises Intel Xeon @ 2.60GHz.

This measures tiny isolated kernels plus common call/input overhead on a
shared virtual host. Frequency, neighboring tenants and cache effects are
not controlled. There are no hardware-counter or confidence-interval claims.
The table reports medians in milliseconds, not browser throughput.

| Kernel | Before | After | GCC 16.2 | Clang 23.1.4 | After/before |
|---|---:|---:|---:|---:|---:|
| separate RGB/alpha | 5.652 | 5.661 | 5.658 | 5.080 | 1.002 |
| packed-value RGB/alpha | 7.449 | 7.450 | 7.427 | 6.758 | 1.000 |
| widened clamp4 | 4.028 | 3.823 | 3.853 | 3.230 | 0.949 |
| umin4 | 3.600 | 2.639 | 2.658 | 2.620 | 0.733 |
| umax4 | 3.688 | 2.591 | 2.641 | 2.645 | 0.702 |
| widened clamp8 | 4.856 | 4.023 | 4.012 | 4.050 | 0.829 |
| umin8 | 4.034 | 3.133 | 3.267 | 3.077 | 0.777 |
| umax8 | 4.067 | 3.436 | 3.408 | 3.414 | 0.845 |

The component kernels improved against lccc's baseline in this run. Native
min/max reaches roughly reference-compiler performance, not a convincing
win over them. The exact RGB/alpha functions have identical assembly before
and after; their small timing changes are noise, **not improvements**.
Clang remains faster on these exact scalar shapes. NPOT-Clang timings and
all raw per-round samples are retained in the JSON, including outliers.
A preceding independent 24-round run is retained as `timing_replicate`;
its generated candidate assembly is byte-for-byte identical. Clamp4's
measured reduction varies from about 20% in that run to about 5% in the final
run. That variability is another reason not to generalize these timings.

## Remaining work before claiming the motivating optimization is solved

1. Represent three useful widened lanes without a speculative fourth byte
   load, and lower narrowing/packing without scalar round-trips.
2. Form and cost the horizontal unsigned max and compatible four-byte store
   as one graph, preserving external uses and original memory ordering.
3. Credit only proven compatible store consumers; keep scattered,
   interfering, overlapping and volatile negative cases protected.
4. Measure a real Firefox/rasterflood-svg build/workload, plus the existing
   kernel corpus, rather than extrapolating from the component benchmark.
