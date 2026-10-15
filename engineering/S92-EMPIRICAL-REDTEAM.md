# S92: rejected BZHI prototype and refreshed empirical audit

**Decision: no runtime optimization accepted. Performance approval remains withheld.**
This extends, rather than replaces, the cumulative PR829/830/831/834 and S86–S91
reviews. At measurement time the latest visible upstream was f07f4c514460e4116a574773cd84b69d185bf65e.
FAST status is recorded by the final delivery receipt; these measurements alone
are not a FAST certificate. Full/slow CI was not run.

## Decisions and alternatives

- Agree with preserving wrapping semantics, CFG/liveness/alias guards and the
  literal/frontend repairs. Memcheck alone missed 276/320 historical fusion
  semantic failures, so clean instrumentation is not correctness proof.
- Reject replacing portable BZHI helpers with opaque inline assembly as an
  optimization. Better opcode selection did not produce better codec runtime.
  `include/bmi2intrin.h` is unchanged. The prototype lives only in the raw
  research archive, not in the production headers.
- Reject raising inliner limits blindly: FSE_encodeSymbol (47 IR / one block)
  and BIT_addBits (27 IR / one block) remain outlined in the observed hot code;
  calling that an inliner bug or removing safety/budget constraints is not
  justified by this profile alone.
- Prefer a future pure, typed bit-mask intrinsic or general legal mask idiom
  recognition with target lowering and allocator-visible operands. Neither
  route has been implemented or validated here. They must cover index low-byte
  semantics, widths, flags, register pressure, no-BMI2 fallback and supported
  target behavior before acceptance.
- Keep GAS post-lock cache validation symmetric with initial validation (S91).
  Its red-before/green-after regression test justified the small repair.

## Patched zstd and measurement controls

Actual ms178/archpkgbuilds d315f524 recipe: zstd1.5.7, source SHA256
5b331d961d6989dc21bb03397fc7a2a4d86bc65a14adc5ffbbce050354e30fd2;
patch SHA256 aa15486d10778c9d00a9b6273101e11feb952aca0b66432a53625dfb67390d8e.
Fresh baseline and candidate static libraries use the same O1/j2-built compiler
(c19756e09c9e176c41a95ea109bf918b28ad84474e187cc8dca903da2ede64ee)
and C flags `-O3 -DNDEBUG -march=x86-64-v3 -fwrapv`. The only intended C
change is forced inclusion of the prototype. The corrected wrapper bypasses
`.S/.s`; its initial erroneous C-header injection into assembly is preserved
as a harness failure, not a compiler failure. The shared API driver was built
with GCC16.2. This is not the recipe's LLVM24/FullLTO/PGO/BOLT build.

Pilot: source/JSON, levels1/3, encode/decode, 15 paired rounds per cell,
1MiB input, randomized arm order, CPU0 affinity, all cells above the 20ms
floor, zero reported correctness failures. The VM has CPUs0/1 available;
frequency/governor, P/E-core and physical-host controls are unavailable.
These are not physical EPYC7763 or Raptor Lake results. Ratios >1 are slower.

| Case | Operation | Candidate/base median | Bootstrap 95% CI |
|---|---|---:|---|
| source-level1 | encode | 1.015191 | [0.990704, 1.083269] |
| source-level1 | decode | 1.004118 | [0.989642, 1.021542] |
| source-level3 | encode | 1.022193 | [0.970215, 1.038676] |
| source-level3 | decode | 0.988780 | [0.969569, 1.008187] |
| json-level1 | encode | 1.009906 | [0.986287, 1.045065] |
| json-level1 | decode | 0.999654 | [0.986984, 1.023099] |
| json-level3 | encode | 1.022668 | [1.008983, 1.039942] |
| json-level3 | decode | 1.009999 | [0.992512, 1.052759] |

| Candidate/base | GM | AM | Best | Worst |
|---|---:|---:|---:|---:|
| compress | 1.017476 | 1.017489 | 1.009906 | 1.022668 |
| decompress | 1.000607 | 1.000638 | 0.988780 | 1.009999 |

Seven intervals cross1; JSON level3 compression is slower with an interval
wholly above1. No win was demonstrated. The small pilot is a rejection screen,
not a replacement for the existing 96-cell broad-options comparison.

Seven standalone Memcheck cases pass (fresh recheck receipt retained):
LCCC64 O0/O2/O3 with x86-64-v3; LCCC64 O2 without BMI2; LCCC32 O2 with and
without BMI2; GCC16 O2 v3. Tests cover 128 pseudorandom sources, indices0–1023,
64-bit index bit40 and a live unsigned comparison across the asm. They are
not exhaustive, omit explicit all-zero/all-one sources and do not execute
non-x86 fallback targets. The initial i386 instrumentation-startup failure
was resolved by installing libc6-dbg for both architectures.

## Callgrind: complete cells, bounded follow-up, and failure accounting

Original 1MiB/four-API-call S86/GCC profiles completed levels1/3 C/D (8 cells,
each with Memcheck and independent GCC-decoder compression checks). Level19
S86 compression exceeded the 240s timeout; that report remains `complete:false`.
The separately labeled **64KiB/one-call** level19 follow-up completed four
cells with Memcheck and decoder checks. Never pool those input sizes/counts.

| Scope | Operation | LCCC Ir | GCC Ir | Ratio |
|---|---|---:|---:|---:|
| 1MiB / 4 calls, L3 | encode | 215154302 | 109065149 | 1.972714 |
| 1MiB / 4 calls, L3 | decode | 73388734 | 49086346 | 1.495095 |
| 64KiB / 1 call, L19 | encode | 395362898 | 199499725 | 1.981772 |
| 64KiB / 1 call, L19 | decode | 1204395 | 807998 | 1.490592 |

Level3 baseline compression exclusive Ir: doubleFast_noDict_generic91.89M
(42.71%), FSE_encodeSymbol42.16M (19.60%), encodeSequences19.94M,
BIT_addBits14.29M and ZSTD_count9.75M. Level3 decompression is dominated by
ZSTD_decompressSequences_default (64.62M; 88.05%). Indirect branch counts are
not a census of all calls/returns; no causal switch-dispatch ablation was done.

A separate fresh-library base/BZHI comparison, same 1MiB/four-call level3
scope, completed four profiles and Memcheck controls:

| Compression event | Baseline | Prototype | Ratio |
|---|---:|---:|---:|
| Ir | 215154302 | 219624566 | 1.020777 |
| Dr | 71387633 | 76757769 | 1.075225 |
| Dw | 34129201 | 39499401 | 1.157349 |
| Bc | 11150872 | 11150872 | 1.000000 |
| Bi | 1036470 | 1036470 | 1.000000 |

Decompression Ir/Dr/Dw are identical between these arms. Increased compression
memory accesses support investigating opaque-asm/register costs, but do not
prove which allocator transformation caused them. Simulated events are not
hardware cycles or PMU measurements.

## All45 refreshed Godbolt shapes

Ran existing `scripts/benchmark_codegen.py` with local flags
`-O2 -march=haswell -fwrapv` and GCC16 header-provider include path.
Oracles: cg162, cclang2310, cicc2021100, cicxlatest. Initial survey had12 failed
rows, primarily HTTP429. A full retry after cooldown completed44/45 rows
without oracle errors: all45 local compiles and179/180 vendor compile results.
The remaining ICC percpu_ptr_walk result rejects `__seg_gs` syntax (exit2),
not a rate-limit failure. Neither traversal `complete:true` nor exit0 output
from an arbitrary vendor proves semantic correctness.

The retry uses CE GCC16.2, Clang23.1.0, ICC2021.10.0 and the ICX latest alias.
The prior version probe of that ICX alias identified2024.0.0, not vendor-latest
2026; no newer deployment is certified. Local Clang is separately23.1.3.
The current retry logic's short backoff does not honor Retry-After; that
potential robustness improvement was inspected, not implemented or claimed fixed.

Static instruction census, **not runtime speed**, for every shape follows.
Bodies/flags/raw responses are retained in the archive; differing inlining and
library choices limit cross-vendor instruction-count comparisons.

| Shape | LCCC | GCC16.2 | Clang23.1 | ICC | ICX alias |
|---|---:|---:|---:|---:|---:|
| aarch64_select_patterns | 67 | 49 | 58 | 74 | 63 |
| ackermann | 9 | 134 | 32 | 196 | 35 |
| arith_loop | 198 | 224 | 236 | 486 | 251 |
| ascii_case_fold | 111 | 158 | 107 | 165 | 121 |
| binary_search | 72 | 51 | 93 | 60 | 101 |
| binary_trees | 145 | 1475 | 151 | 612 | 139 |
| bitops | 79 | 92 | 90 | 126 | 112 |
| byte_load_reassembly | 153 | 131 | 202 | 218 | 63 |
| chacha20_block | 193 | 271 | 257 | 2278 | 138 |
| constant_recursion | 10 | 135 | 32 | 196 | 35 |
| double_reduction | 135 | 94 | 133 | 145 | 149 |
| expat_siphash24 | 847 | 312 | 496 | 756 | 515 |
| expat_xml_scan | 219 | 172 | 222 | 322 | 327 |
| fannkuch | 146 | 142 | 227 | 229 | 268 |
| fib | 31 | 246 | 32 | 162 | 35 |
| glibc_memcmp | 203 | 157 | 227 | 315 | 311 |
| glibc_strstr | 195 | 179 | 319 | 226 | 261 |
| gzip_crc32 | 73 | 50 | 82 | 60 | 100 |
| hash_table | 141 | 121 | 140 | 158 | 142 |
| histogram | 64 | 76 | 107 | 132 | 124 |
| libm_round_family | 159 | 173 | 183 | 180 | 125 |
| linux_find_bit | 148 | 144 | 118 | 187 | 121 |
| linux_find_bit_scaled | 148 | 144 | 118 | 187 | 121 |
| linux_rbtree | 235 | 173 | 189 | 189 | 274 |
| loop_patterns | 226 | 179 | 361 | 296 | 245 |
| lz4_compress | 350 | 295 | 421 | 442 | 363 |
| lz4_match_extend | 366 | 306 | 414 | 446 | 338 |
| mandelbrot | 56 | 48 | 59 | 59 | 64 |
| matmul | 94 | 82 | 164 | 134 | 131 |
| nbody | 260 | 215 | 266 | 782 | 531 |
| percpu_ptr_walk | 141 | 58 | 97 | failed | 102 |
| qsort | 31 | 26 | 43 | 43 | 52 |
| ring_fifo | 56 | 13 | 57 | 68 | 90 |
| sha256_transform | 272 | 213 | 232 | 410 | 862 |
| sieve | 55 | 51 | 345 | 192 | 376 |
| spectral_norm | 207 | 176 | 365 | 261 | 247 |
| sqlite_varint | 397 | 327 | 373 | 470 | 428 |
| strlen_bench | 196 | 139 | 226 | 204 | 481 |
| struct_copy | 135 | 80 | 76 | 183 | 82 |
| switch_dispatch | 86 | 68 | 80 | 76 | 87 |
| tce_sum | 11 | 10 | 10 | 51 | 13 |
| tls_seg_access | 51 | 36 | 55 | 62 | 64 |
| zlib_ng_adler32 | 275 | 156 | 148 | 244 | 147 |
| zlib_ng_adler32_combine | 290 | 316 | 495 | 289 | 646 |
| zstd_count | 164 | 91 | 121 | 106 | 152 |

## Broader objectives and evidence boundaries

Existing qualified28 native timings against GCC: GM1.057436, AM1.069925;
against main: GM1.011297, AM1.012337. Full45 includes a launch-floor distortion.
Broad zstd96/GCC compression GM1.445530/AM1.463961; decompression
GM1.307746/AM1.324858. These remain historical controlled measurements, not
new runs or improvements. See the S86 follow-through report for each original
cell and separately reported31-pair follow-ups. Fann/main +25.53%, XML +4.38%
and nbody +2.99% are not waived. The present rejected experiment does not
resolve those deficits or the overall runtime objective.

Raw research: persisted `artifacts/S92-EXPERIMENT-RAW.tar.xz` with adjacent
SHA256 receipt and per-member manifest; summaries under
`engineering/evidence/redteam92`. Archive includes both failed/partial and
successful runs, prototype, corrected wrapper, input bytes, drivers, profile
scripts, static oracle artifacts and build logs. Production changes remain
only previously validated repairs and these accurately scoped reports.
Pinned GAS/objdump2.47.20260726 for x86/AArch64 and ArmA64 XML2026-09 provisioning
completed successfully. Six GiB swap was active. No external expert approval,
all-target coverage, universal optimality or zero-defect claim is made.
