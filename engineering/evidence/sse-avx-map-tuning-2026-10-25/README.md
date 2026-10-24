# SSE/AVX map tuning and checked feature-flag semantics

## GCC feature precedence

GCC 16.2 target queries and predefines were checked, not inferred from a
global last-option rule. See `gcc-feature-order.json`. The native gate now
compares GCC, Clang and LCCC predefines for all four sequences:

| Flags | SSE1 | SSE2 |
|---|---|---|
| `-mno-sse2 -msse` | on | off |
| `-msse -mno-sse2` | on | off |
| `-mno-sse -msse` | on | on |
| `-mno-sse2 -msse2` | on | on |

An SSE1 grant does not override an explicit denial of a different feature,
SSE2. Preventing automatic integer SSE2 instructions under that denial is
correct. Calling it a prohibition on *vector codegen* was too broad: SSE1
includes packed F32 arithmetic. GCC emits `addps` for a restrict F32 addition
loop at O3 under `-mno-sse2 -msse`. LCCC's shared gate still conservatively
misses that SSE1-only packed optimization; no contrary GCC claim is made.

The stronger comparison found another real mismatch: LCCC incorrectly erased
SSE1 for plain `-mno-sse2` or `-msse -mno-sse2`. Preserve that independent
permission, keep legacy SSE2 bulk-copy helpers disabled, and diagnose any
remaining unsupported SSE2 XMM emission. This permits legal scalar SSE1
without silently accepting `addsd`, `movd`, `movdqu`, etc. No x87 fallback is
claimed. Explicit user inline assembly remains an escape hatch. Native tests
pin all flag sequences, legal F32 behavior, scalar 256-byte copies, and the
unsupported-F64 diagnostic. The frozen predecessor fails the predefine gate
and demonstrably emits `addsd` under `-mno-sse2 -msse`.

## Bounded integer product-map unrolling

Small I32/U32 multiply-root maps use two independent packed copies per counted
iteration, at XMM and YMM widths. Admission requires repeated-work cost <= 3;
larger trees and other element types retain their old policy. Existing packed
widths drive alias checks, byte offsets and scalar tails. Per-TU
`CCC_NO_MAP_I32_UNROLL=1` restores the one-copy policy. There is no workload
name dispatch, ISA grant, FP reassociation or new register-allocation mode.

### Paired runtime

Canonical runner, 15 randomized paired rounds, two excluded warmups, CPU 0,
GCC 16.2/Clang 23.1.3 with explicit GCC 16.2 CRT selection, GNU as/ld 2.47,
warning-clean Rust 1.97 fastbuild -j2 for both binaries, 8 GiB swap. All four
arms' outputs and successful exits agree. The predecessor already contains
the legal SSE2 product reconstruction, not an illegal SSE4.1 baseline.
Different pass counts amplify faster profiles; compare within rows only.

| Profile | Passes | Before median | After median | Paired after/before, 95% CI |
|---|---:|---:|---:|---|
| SSE2 | 1,048,576 | 503.98 ms | 425.54 ms | 0.84647 [0.84265, 0.85551] |
| SSE4.1 | 2,097,152 | 536.75 ms | 535.36 ms | 0.99039 [0.95828, 1.00583] |
| AVX2 | 4,194,304 | 661.24 ms | 550.52 ms | 0.83755 [0.78827, 0.85280] |

Additional **1.181x SSE2 / 1.194x AVX2** speedups on this map. SSE4.1 has no
demonstrated change, not an asserted win. SSE2 is consistent with Clang parity
(after/Clang 1.00978 [0.98544, 1.02858]). AVX2 is faster than both references
on this kernel. The JSON reports retain every sample, command, output check,
compiler identity and dispersion warning; no outliers were removed. These
are VM screening results, not PMU or application-geomean claims.

### Correctness and falsification

- 10,605 overlap/length cases per arm: both directions through 17 elements,
  either input aliasing output, identical input streams, negative and zero
  lengths and every length through 97. GCC/Clang and rolled/unrolled SSE2,
  SSE4.1 and AVX2 arms agree. The gate requires twice the packed-product count
  when enabled; the predecessor fails this structural ablation.
- Existing guard-page/high-bit/overflow/live-FP cases, opcode allowlists,
  feature denial/regrant and executable arithmetic/lane-permutation mutants
  pass. Additional macro assertions pin the feature-order answer above.
- Four map-policy units pass, including XMM/YMM small-trip coverage and
  per-worker reset. Two feature-denial units and three instruction-fence
  units pass. Both binaries build with warnings denied.
- Twelve targeted deterministic differential fuzz cases pass, none skipped.
  Entire gzip longest_match assembly remains byte-identical at SSE2,
  SSE4.1 and AVX2, so stack references cannot have increased.
- The final compiler regenerates every timed executable byte-for-byte;
  `i32-identity.json` records hashes. Evidence transcripts normalize trailing
  whitespace. Hosted native gates are wired into codegen CI; broad coverage
  is delegated to hosted CI rather than an expansive local run.
