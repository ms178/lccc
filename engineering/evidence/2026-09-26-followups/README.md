# Latest-main follow-up: strict reciprocal packing and exposed P0 gaps

**Status (2026-09-26):** The previous `ms178-1.patch` is already merged. This
follow-up was developed against upstream `main` at
`e5bc19118c6ea5a54d5539d4a6cff52c97ab6baa` (refresh/rebase before
submission). The change fixes a measured AVX2 code-generation pathology and
adds regression/benchmark coverage; **it does not fix the open byte-compare,
Mandelbrot, find-bit, or allocator gaps**. None of the runtimes below were
measured on an i7-14700KF. The physical-target gate remains open.

## What changed, and why

1. In the x86 strict computed-reciprocal lane pack, emit `vmovd` and
   `vpinsrd` instead of legacy `movd` and `pinsrd`. This is in the emitter's
   already-guarded AVX2/SSE4.1 branch, not a function-name exception or a
   switchable heuristic. Its four low integer lanes still feed the same
   `vcvtdq2pd` and packed `vdivpd`; the VEX.128 upper-zero behavior is safe
   because the conversion overwrites `ymm0` before its next read. Scalar
   lane additions retain their source order. The fallback for weaker target
   features is unchanged. The final-text VEX promotion pass is **not**
   required for correctness or for the instruction encoding.
2. Tighten the assembly contract for this path, including
   `CCC_NO_VEX_PROMOTE=1`, AVX-only, AVX2-without-SSE4.1, and an
   XMM-allocation-disabled spill fallback. Add a 140-case C differential
   stress test for negative/zero trips, repeated packed chunks, scalar
   remainders, and **mixed negative/positive denominators within one packed
   chunk**. These are generic
   compiler/ISA correctness checks, not benchmark-specific performance
   assertions.
3. Preserve the historical LZ4 benchmark by default, but register a separate
   `MATCH_RICH` source variant (`lz4_match_extend`) and a longer find-bit
   variant (`linux_find_bit_scaled`). Their unchanged source kernels and
   output checks expose previously under-exercised gaps. The default LZ4
   executable's `.text` is byte-for-byte the same as before adding the
   optional input generator.

## Baselines and controls

- Unchanged-main LCCC compiler SHA-256:
  `3f6797b8aeb721e023da1f154763a54100ab732ffb9af7a1fe6a6fb59fc45ce4`.
  Candidate LCCC compiler SHA-256:
  `d071563d5326f5e1d0a8e3f5aa035e0079843868da16abe9cd2ee16484e3f818`.
  A subsequent source edit to the emitter was formatting-only. GCC runtime
  comparator: host GCC 14.2.0, SHA-256
  `a23ecab8ff08f09ad8c80602c2c5df7f49e09c25905cb8975902e101bf72635f`.
- Identical C sources and flags for each compiler arm:
  `-O2 -march=x86-64-v3 -mtune=raptorlake`. CPU 0 was pinned via `taskset`
  on a **virtualized Intel Xeon 2.60 GHz**; 8 GiB of swap was active.
  Per-round compiler order was randomized; outputs were compared on every
  round. The broad run used two excluded warmups, nine paired rounds,
  seed `260928`, and `--strict`. No outliers were removed. The 15-round
  focused spectral run used the same-source/same-host protocol.
- Hardware PMU access was unavailable (`perf` is not installed); wall/child
  CPU clocks and bootstrap intervals are *VM screening*, not cycle counts or
  Raptor Lake performance results. All four remote oracles below are
  **assembly comparisons**, not same-host runtime measurements.

## Output-checked codegen and runtime results

The [41-workload report](full-corpus-ab/results.md),
[per-workload summary with executable `.text` hashes](full-corpus-ab/summary.csv),
and [369 raw paired rounds](full-corpus-ab/paired-rounds.csv) are compact
reviewable evidence. The unabridged `full-corpus-ab/results.json` and
compiled/disassembled artifacts are retained in this workspace; rerunning
`tests/benchmark/run_benchmarks.py` regenerates them. **All 41 workloads
compiled and passed output comparison**; `objcopy --only-section=.text` SHA-256
matches between candidate and main for **40/41**, and their **entire
executable binaries are byte-identical** for those same 40. The *only*
changed executable is `spectral_norm`. Accordingly, timing differences on
those 40 identical binaries are host/noise, not demonstrated compiler
regressions or improvements.

| Workload | Candidate / current-main paired median (95% bootstrap interval) | Candidate / GCC 14 paired median | Meaning |
| --- | ---: | ---: | --- |
| `spectral_norm` (41-workload run) | **0.03679** [0.03582, 0.03820] | 1.02479 | VM gain; still near GCC runtime; i7 result unknown. |
| `lz4_match_extend` | 0.97695 [0.94553, 1.05204] | 1.28963 | No candidate/main gain demonstrated; byte-compare gap remains. |
| `linux_find_bit_scaled` | 1.00165 [0.98580, 1.00611] | 1.39897 | No candidate/main gain demonstrated; CFG/index gap remains. |
| `mandelbrot` | 1.00313 [0.99495, 1.02701] | 1.29839 | Scalar FP gap remains; no candidate/main code difference. |
| `sha256_transform` | 1.02108 [0.99173, 1.04174] | 1.17870 | No demonstrated allocator change or candidate/main regression. |

The independent [15-round focused spectral run](spectral-ab/results.md)
found candidate/main **0.03608** [0.03551, 0.03726] (candidate 240.20 ms,
main 6.5987 s; paired minimum 0.03423), and candidate/GCC 14 **1.02755**.
An earlier assembly-only splice changing **two** `movd` and **six** `pinsrd`
instances to VEX encodings reduced output-checked Xeon-VM time from 6.218 s
to 0.224 s without changing the C source. The separately compiled candidate
then reproduced the direction/magnitude. Its hot-path disassembly has
`vmovd`×2 and `vpinsrd`×6 instead of the legacy spellings, while retaining
`vcvtdq2pd`×2 and `vdivpd`×2. This isolates an encoding problem; the
specific hardware mechanism or magnitude **must not** be assumed on the
14700KF.

The [nine-round scaled P0 pilot](p0-scaled/results.md) independently found
LCCC/GCC 14 ~1.254 for match-rich LZ4 and ~1.387 for scaled find-bit; the
41-workload repeat above found ~1.290 and ~1.399. Both pilot candidate/main
confidence intervals also span 1. A 12,288-pass instrumentation of the
historical LZ4 input had **zero** match-extension entries; the new input had
49,299 matches of length ≥8 in a 24-pass check. The measurements must not be
used to justify unsafe widened reads: both operand `[p,p+8)` ranges need
proof, and `memmove` cannot replace forward-overlapping byte-copy smear.

## Four-oracle code inspection

[`codegen_oracle.py`'s refreshed 12-source ranking](final-oracle/rank.md)
([machine-readable counts](final-oracle/rank.json)) completed with **zero
oracle errors** at `-O2 -march=x86-64-v3`. It compared the candidate with
Compiler Explorer GCC **16.2**, Clang **23.1.0**, ICC **2021.10.0**, and ICX
`latest` (moving channel; exact CE identity recorded in the assembly
manifest). A [separate match-rich LZ4 oracle](match-rich-oracle/rank.md)
([manifest](match-rich-oracle/raw/lz4_compress-all/manifest.json)) used
`-DMATCH_RICH=1 -DPASSES=2048` and also had zero oracle errors.

The [candidate and four-oracle spectral assemblies](final-oracle/raw/spectral_norm-all/lccc.s)
are available as `lccc.s`, `gcc.s`, `clang.s`, `icc.s`, and `icx.s` in the
same directory, with their [compiler/version manifest](final-oracle/raw/spectral_norm-all/manifest.json).
Candidate source assembly has zero legacy `movd`/`pinsrd`, two `vmovd`, six
`vpinsrd`, and two `vdivpd`; GCC 16 and ICX also use packed `vdivpd`, whereas
Clang 23.1 uses scalar `vdivsd` in this source. The static *main*-function
instruction-count ranking still calls spectral “ahead”: it ignores the
outlined hot `mul_AtAv` work and **did not reveal the legacy/VEX issue**.
Neither an instruction count nor an oracle's ISA spelling proves a runtime
result. For Mandelbrot, **all five compared compilers use scalar FP in the
main loop** at these flags; claims that GCC already vectorized it were not
supported by the assembly.

## Correctness, validation, and red-team decisions

- The [mixed-sign strict-reciprocal stress test](validation/stress-mixed-sign.txt)
  passed against GCC 14, unchanged main, and candidate at `-O0/-O2/-O3`,
  and candidate with XMM allocation disabled and with AVX disabled.
  Candidate AVX2 stress assembly contains `vpinsrd`,
  `vcvtdq2pd`, and `vdivpd`. At the four optimization levels, the benchmark
  output gate passed **204/204**, including the new arms.
- `scripts/ci_local.sh --fast`: **74 passed, 0 failed, 3 deliberately skipped
  slow gates**, including passing rustfmt and Clippy. The three slow gates
  passed separately: SSA-validated regression **798 passed, 0 failed,
  13 skipped-compare, 9 skipped-run** (820 total); benchmark outputs all
  optimization levels **204 passed, 0 failed**; four-phase peephole
  whitespace **PASS**. The regression corpus's GCC comparison of
  `spectral_norm` is skipped for a host link issue (`-lm`); the separate
  benchmark gate and 15/9-round output-checked A/B runs compare its output
  against GCC explicitly. The compact [final fast-CI gate
  summary](validation/ci-fast-final.summary.txt) and full slow-gate
  [regression](validation/regression-ssa.txt),
  [benchmark](validation/benchmark-outputs-all-levels.txt), and
  [peephole](validation/peephole-whitespace.txt) transcripts are preserved;
  the complete fast-CI log is retained in this workspace.
- A checked 21-pair, same-assembly-input splice removed the redundant-looking
  `testq` after `andn` in scaled find-bit. Both variants returned `0`; the
  no-test/original VM paired median was **1.013** (0.443 vs 0.437 s), not a
  reliable improvement. No speculative peephole shipped. Find-bit's existing
  scalar-derived-IV optimization did not change this kernel.
- We rejected a byte-compare widening without both-range memory-safety
  proof, an unconditional overlap-blind `memmove`, a blind find-bit peephole,
  a Mandelbrot cross-pixel transform without a valid exit-value/SSA proof,
  and a large allocator rematerialization rewrite. A previous allocator
  prototype regressed `expat_xml_scan` and `sqlite_varint`; the current
  compiler's `sha256_transform` text remains bit-identical in candidate and
  main. These P0 items remain *open*, not claimed as fixed.

## Physical target gate and replay

Run [the guarded i7-14700KF runner](run-on-i7-14700kf.sh) on an actual
physical i7-14700KF **P-core** with active swap. Supply a compiler built
from the unchanged main identified above; the script refuses this Xeon VM,
missing swap, an unavailable/non-P core, and identical compiler binaries.
It uses three same-source compiler arms, randomized paired order, strict
output checks, 21 repetitions, three warmups, and `N=4000` for a separately
scaled spectral arm. Check that both spectral arms reach ≥200 ms; if not,
increase `SPECTRAL_N` and rerun. Example:

```bash
CPU=2 BASE_LCCC=/path/to/e5bc1911-lccc LCCC=/path/to/candidate-lccc \
    bash engineering/evidence/2026-09-26-followups/run-on-i7-14700kf.sh
```

The runner records environment, revision, binary hashes, raw rounds, and a
PMU probe if available. The operator must still verify baseline binary
provenance, minimal CPU interference, output agreement, and median/**minimum**
consistency. If accessible, separately collect actual `perf stat`
cycles/instructions on fixed input and affinity. **Until that run exists,
no i7-14700KF speedup, PMU result, or completion of the other P0 items is
claimed.**
