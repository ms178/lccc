# Joint pixel computation/reduction/sink audit — 2026-11-01

## Status and scope

This supersedes the *component-only* outcome in
`LIGHTPIXEL-AUDIT-2026-11-01.md`. The new implementation vectorizes the complete
integer LightPixel tail, including the packed-by-value result used by Firefox:
byte expansion, wrapping dword multiplication, logical shift, unsigned clamp,
byte-domain maximum, and either a four-byte store or scalar packed return.
It is not a Firefox/browser performance result, nor proof of superiority over
a compiler built with LLVM PR 227303.

Baseline is current main `5545d7125970f068def20f7184734eb9dbc9e595`.
The earlier component report deliberately remains historical evidence.

## Design and legality

The ordinary BB-SLP builder supplies the arithmetic graph and scheduling.
Joint seeds account for compatible narrow sinks and the reduction together;
this is not a LightPixel-specific multiply/clamp opcode. Supported endpoints:

* Three distinct consecutive byte inputs, optionally duplicated/permuted in
  the four-lane image. Read exactly two bytes plus one byte, **not four**.
* Four consecutive byte inputs, read as exactly four bytes.
* Byte fields of one scalar U32, including a permuted field order.
* Four contiguous byte stores, with the reduction in any of the four lanes.
* An OR tree returning four byte-bounded fields as one U32.
* Three-input max and four-input max that preserves an independent alpha.
* Signed integer clamps when their canonical min/max graph proves [0,255].

Narrowing and unsigned-byte max commute only after the range proof. Arbitrary
uint32 multiplication wraps **before** shifting; widening the product without
truncation would be incorrect. SSE4.1 is required; SSE2 retains scalar code.
The target-specific tail estimate includes legacy SSE register copies.

Readback forwarding accepts only exact, nonvolatile, default-segment byte
stores on the same symbolic stream. Unknown calls, inline asm, atomic reads,
fences and volatile accesses stop that proof. Intervening writes/reads and
external uses remain subject to the SLP scheduler's legality checks. Every
retired store, including an overwritten store outside the final four sinks,
is checked against the original vector inputs: a later read of an overwritten
write must not silently become a read of pre-store memory. This has a direct
IR regression test in addition to differential alias fixtures.

The vector register home is 16 bytes; the actual memory footprint is 3 or 4
bytes for a widening load and 4 bytes for a narrow store. Alias metadata tests
explicitly distinguish these sizes. The register-result tail is pure, scalar,
and memory-free, with no XMM result home.

## Validation

Warning-denied fastbuild lccc/lccc-ld builds succeeded, with the required 4 GiB
swap active and no concurrent heavy builds. Full warning-denied Rust library
suite: **7,154 passed, 10 ignored, 0 failed**. Warning-denied all-target
Clippy also passes.

`check_lightpixel.py --require-native`: **6/6 PASS** (O2/O3, SSE2/SSE4.1/AVX2),
65,536 oracle cases per configuration, including full-width scales, exact
three-byte guard-page inputs, output bounds, aliases and volatile accesses.

`check_joint_pixels.py`: **6/6 PASS**, each with 65,536 cases over 15 pixel
kernels and input/output offsets -3 through +4. Additional guarded-page probes
cover three/four-byte inputs, four-byte outputs and RGB24 expansion lengths
0..17. All four alpha positions, independent fourth-channel alpha, reversed
packed-return fields, signed clamp boundaries, retired-store reads,
intervening writes and volatile inputs are covered. Positive SIMD fixtures
require byte-domain reduction codegen, not just equivalent scalar output.
Volatile fixtures require the original three byte reads.

The joint suite also passes all six configurations with Clang 23.1.4 as the
independent reference. The Python SIMD contract suite passes 22 tests. The CI lane-plan checker
passes. Both GCC16 and authenticated hosted-ICX differential CI invocations
are wired into the existing workflow; a hosted pass must still be confirmed
for the new commit. Local ICX is unavailable in this environment.

## Runtime evidence and limits

Raw current-run evidence is in `lightpixel-joint-evidence-2026-11-01.json`.
The benchmark rotates compiler order, checks complete output hashes, and
keeps 24 rounds × 8 kernels × 5 compiler arms = **960 samples**. These are
virtual-host measurements, not statistical confidence intervals.

AVX2 median milliseconds (lower is better):

| Kernel | main | joint candidate | GCC 16.2 | Clang 23.1.4 | Clang + NPOT flag |
|---|---:|---:|---:|---:|---:|
| separate RGB→RGBA | 5.630 | 5.600 | 5.643 | 5.099 | 5.093 |
| packed U32 return | 7.463 | 5.263 | 7.434 | 6.777 | 6.760 |
| four-channel clamp | 4.045 | 3.651 | 3.828 | 3.215 | 3.222 |
| eight-channel clamp | 4.673 | 4.434 | 4.046 | 4.122 | 4.052 |

Packed-return median time is approximately **29.5% lower than main, 29.2%
lower than GCC, and 22.4% lower than Clang**. A separate preceding 12-round
run showed the same direction. The pointer-output kernel shows essentially
no improvement over main and remains about 10% slower than Clang. The larger
clamp workloads also do not establish broad superiority over Clang.

The full AVX2 function bodies contain 21 instructions for original/separate
and 19 for packed return, including `ret`. These counts use a mnemonic parser
that accepts instructions without operands; shorter code is not itself a
runtime win. Clang's NPOT flag is only a comparison configuration: it is NOT
a substitute for building and measuring LLVM PR 227303 itself.

## Chromium source audit: concrete findings, not whole-browser claims

Pinned Chromium revision:
`042b7bd11fd2b37f7d8b445781e6d858d3f56554` (2026-11-01).
Its DEPS pins Skia `3df52f9ec5460b13943441def15d7fcbd2b7e94c` and libyuv
`1e2e3a35993609da1d89ac170b907719d555f4b3`.

1. **skia/ext/convolver.cc**: signed accumulator shift/clamp and alpha raised
   to max(alpha,R,G,B). This motivates the signed and independent-alpha tests.
   The canonical min/max spelling vectorizes completely. The actual
   `ClampTo8` fast-path spelling, `(unsigned)a < 256` followed by a negative
   test, remains scalar in the reduced C witness. That is a demonstrated
   remaining recognition gap, not a claimed win for the original fallback.
2. **skia/ext/convolver_SSE2.cc**: already has explicit SIMD for four pixels,
   saturating dword→word→byte packing and alpha-max shuffles. Improving scalar
   recognition does not establish an improvement to this hot SIMD path.
3. **libyuv/source/row_common.cc**, pinned dependency revision above:
   `RGB24ToARGBRow_C` and `RAWToARGBRow_C` expand exact three-byte pixels to
   four bytes with constant alpha. These are useful exact-boundary and shuffle
   opportunities, but have no max reduction and must not receive fictional
   reduction credit. The RGB24 witness passes guarded differential tests;
   this implementation does not yet recognize it as a joint reduction seed.
4. **ui/gfx/color_conversions.cc, color_utils.cc**: float RGB min/max and color
   transforms are not interchangeable with unsigned byte maxima. Strict NaNs,
   signed zero and rounding preclude applying this integer proof.
5. **ui/gfx/skbitmap_operations.cc**, color-analysis/image-util and media
   video utility/frame sources: further fixed-point, branchy and floating
   pixel transforms warrant separate reduction/loop work. Source inspection
   alone is not a measured optimization or an exhaustive Chromium audit.

Witnesses in `tests/simd/chromium_pixel_shapes.c` are reduced C equivalents,
not a C++ frontend test or a browser workload. The broad Chromium audit,
fast-path clamp normalization, a patched-LLVM comparison and whole-browser
measurements remain unfinished. No universal performance claim is justified.
