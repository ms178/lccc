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

### Focused-ablation census entry

Hosted CI caught the missing audit-census registration for the new I32
unroll ablation. Register exactly that one documented name (+1 knob); no
panic, unsafe, long-function or undocumented-knob allowance is increased.
The independent switch is justified by the retained paired measurements and
required native structural ablation, not a new optimizer mode or default.

## Strict floating-point map tuning

Short F32/F64 Add/Sub/Mul trees now use two packed copies per counted x86
iteration. The repeated-work budget is five (e.g. three loads plus multiply
and add). Division, sqrt, rounding, comparisons, selects, explicit fused
operations, larger trees and NEON keep the one-copy policy. The existing
small-trip shrink rule also applies to two-wide F64. No expression is
reassociated and no contraction permission changes. A separate per-TU
`CCC_NO_MAP_FP_UNROLL=1` ablation restores one copy. Register exactly this
second measured ablation (+1 knob) without changing any other audit ceiling.

The new F32/F64 benchmark IDs share one deterministic 512-element source;
this is an original synthetic throughput kernel, not an upstream application
extract. Four arrays occupy 8 KiB (F32) or 16 KiB (F64). One input element
changes per pass, the kernel is noinline, and every output bit enters the
final hash. Both variants explicitly use `-ffp-contract=off`, including in
hosted CI. A bounded 128-pass configuration covers Callgrind tooling.

### Paired floating-point runtime

Same modern tools, canonical randomization, 15 paired rounds, two excluded
warmups, exact output/exit checks and no removed outliers as above. The
predecessor already contains the integer tuning and corrected flag handling.
SSE2/SSE4.1 use 4,194,304 passes; AVX2 uses 8,388,608 to keep every measured
arm above 200 ms. All arm minima exceed 248 ms. Compare within rows only.

| Profile | Type | Paired new/before, 95% CI | Reciprocal speedup |
|---|---|---|---:|
| SSE2 | F32 | **0.91797 [0.90832, 0.96096]** | **1.089x** |
| SSE2 | F64 | **0.87348 [0.86672, 0.88877]** | **1.145x** |
| SSE4.1 | F32 | **0.93440 [0.92404, 0.96125]** | **1.070x** |
| SSE4.1 | F64 | **0.86669 [0.85660, 0.86885]** | **1.154x** |
| AVX2 | F32 | **0.83102 [0.80166, 0.84080]** | **1.203x** |
| AVX2 | F64 | **0.79077 [0.78661, 0.83121]** | **1.265x** |

These are per-kernel, within-profile improvements, not an application geomean.
SSE2/SSE4.1 are consistent with Clang parity for both types. GCC still leads
F32 by approximately 3-4% at XMM width and 6% at YMM width; it also leads
YMM F64 by approximately 3%. No universal optimality or erased GCC gap is
claimed. The three full FP JSON reports retain compiler/binary identities,
commands, all samples, paired intervals and dispersion warnings. One-round
screens were not accepted as evidence of an effect.

### Strict-FP native boundaries

The guard fixture checks every length -3..97 at unmapped page endpoints and
five forward/backward/identical-output alias offsets, for 606 cases per arm.
Inputs exercise single-rounding cancellation witnesses, signed zero, minimum
subnormal and normal values, underflow and overflow. Full initialized-buffer
hashes also include a scalar FP argument kept live across the packed loop.
GCC/Clang and both packed/rolled widths agree bit-for-bit with successful
exits. Independent GCC executables compiled with contraction enabled must
produce different outputs for BOTH precisions: real native FMA mutants kill
any attempted weakening of the multiply-then-add contract.

At O2 the native structural gate requires precisely twice the packed multiply
count when unrolling is enabled, separately for F32/F64 and SSE2/SSE4.1/AVX2.
The frozen predecessor fails this ablation. Additional O0/O1/O3/Os runs at all
three profiles check bytes and exits without pretending vectorization is
mandatory in optimization modes that disable it. Five warning-clean focused
policy units cover admission, small trips and per-worker switch reset. The
new native gate is wired into hosted and targeted local codegen CI, and both
benchmark IDs are explicit members of the hosted benchmark-shard partition.

Final FP checks: **40 native arms / 24,240 cases**, both real FMA mutants
killed, 12 additional deterministic differential-fuzz cases passed without
skips, and the existing integer product/alias/feature gates still passed.
All six measured FP executables and all three measured integer executables
regenerate byte-identically with the final compiler; gzip assembly is also
unchanged on all three profiles. The full audit census holds with exactly
the two documented new ablation names and no other increased ceiling.

The final shared-admission cleanup combines the two identical two-copy
branches without changing either type's guard or short-circuit ablation
behavior. Both binaries rebuilt warning-clean, all focused units and native
gates passed again, and all nine timed ELF images plus gzip assembly remain
byte-identical. The final compiler's separate fingerprint is recorded beside
the original measured-compiler/source identities, not substituted into the
original timing reports.
