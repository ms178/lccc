# 2026-08-18 — `-Os` post-inline size policy and kernel setup follow-up

## Scope and checkpoint

This checkpoint fixes a size-profile pipeline bug exposed by the patched Linux
6.18.44 x86 boot setup.  `-Os` used the size-aware inliner in the primary
optimization phase, but the later post-recursion cleanup invoked unrestricted
inlining.  Medium helpers were consequently cloned again after structural
cleanup, expanding already oversized `.code16gcc` setup objects.

Repository base: `c72889b03bb3d32d6e06851991b74b26a84eebb6`
(`origin/main` as fetched on 2026-08-18).  The preceding local checkpoint is
`2391c109b1abc67dade9290337869e33da8662ee`.

The current kernel tree is `/home/user/linux-6.18.44`.  It contains all 26
applicable `linux-cachymod-6.18` package patches.  This checkpoint deliberately
does not relax Linux's 64-sector setup limit.

## Implemented policy

Both inline phases now retain the size profile at `-Os`/`-Oz`.  The inliner no
longer treats size optimization as "first pass only"; instead it admits bounded
high-value cases:

1. tiny and small helpers handled by the existing first pass;
2. `always_inline` helpers required by C attributes and kernel section/asm
   semantics;
3. a static helper with one module-wide call site, where eliminating the
   standalone body is normally a net size win;
4. an acyclic helper called from a loop, preserving per-iteration call savings;
5. a loop helper called from another loop function but outside the caller's
   loop; and
6. a loop helper called inside a loop only when its current IR body has at most
   64 instructions.

The 64-instruction nested-loop cap is checked before the one-call-site
exemption.  This ordering matters: zlib-ng's large Adler-32 loop has a cold call
that profitably specializes, after which the remaining hot call appears unique;
letting uniqueness override the cap merges the large checksum loop into the
outer benchmark loop and produces severe spills.

During one size-aware inliner invocation, an ordinary body larger than the
small-static-loop threshold is cloned at most once per caller.  This limits
repeated medium-body expansion while preserving one specialization
opportunity.  The nested-loop cap remains the cross-invocation safety rule,
because the primary and post-structural inline phases have separate invocation
state.

The API and local terminology is now `run_size_optimized`/`size_optimized`,
not `run_small_only`/`small_only`, matching the actual behavior.

## New regressions

- `tests/regression/os_postinline_size_policy.c` supplies a medium static helper
  with three semantically checked call sites.
- `tests/regression/check_os_postinline_size_policy.sh` compiles that fixture for
  i686 at `-Os` and requires the helper plus all three calls to survive.
- `tests/regression/check_os_nested_loop_inline_policy.sh` guards both sides of
  the nested-loop threshold:
  - zlib-ng Adler-32 must retain exactly one outlined hot checksum call while
    fully inlining its `len_16`/`len_64` tail helpers;
  - the bounded `hash`, `insert`, and `lookup` helpers in the hash-table corpus
    must have no surviving calls in the generated assembly.

The hash-table assertions were added after a deliberately more conservative
candidate left all three helpers outlined.  That candidate saved 342 executable
`.text` bytes but caused a statistically clear 4.9% paired slowdown
(95% bootstrap interval 3.8% to 5.9%).  The bounded exception restores the hot
shape without admitting the large Adler-32 body.

## Validation

LCCC itself was rebuilt through `scripts/build_lccc_fast.sh`: Rust `-O1`, two
Cargo jobs, fastbuild profile, active 4 GiB swap.  The functional rebuild took
23.24 seconds.  After adding the explicit `always_inline` semantic exemption,
the final source was rebuilt again in 23.24 seconds.

Correctness and structural validation:

- Rust library suite: **853 passed, 0 failed, 6 ignored**.
- C/assembly regression suite with `CCC=target/fastbuild/lccc`:
  **336 passed, 0 failed**.
- Both new structural scripts pass independently and through the full runner.
- `git diff --check` passes.

An initial attempt to ask `run_regression.sh` for `--help` was invalid because
that script has no help mode; it used its missing default `target/release/lccc`
and produced only compile failures.  It is not a product failure and is not
validation evidence.  The corrected explicit-`CCC` run above is authoritative.

## Paired generated-code evidence

The final comparison used the repository's paired benchmark runner at `-Os`
with current LCCC versus the exact preserved pre-fix compiler.  It used nine
randomized AB/BA rounds, two excluded warm-ups, CPU 0 pinning, retained every
sample, and required output equality.  This VM does not expose a usable PMU, so
these are controlled wall-clock screening results rather than hardware-counter
claims.

All **25/25** workloads passed correctness.

- geometric-mean current/pre-fix runtime ratio: **0.9821**;
- arithmetic mean: **0.9869**;
- spectral norm: **0.584**, 95% interval **[0.578, 0.592]**;
- hash table: **1.000**, 95% interval **[0.967, 1.058]**;
- no statistically clear hash regression remains.

A separate nine-round focused hash run measured **0.9929** with interval
**[0.9502, 1.0230]**.  Its variance is too high for a speedup claim, but the
assembly has no `hash`, `insert`, or `lookup` calls and both paired runs reject
the earlier clear 1.049 regression.

The timed compiler preceded the final explicit `always_inline` exemption.  A
correctness-only replay with the final rebuilt compiler passed all 25 workloads,
and every final executable was byte-for-byte identical to its timed counterpart.
Thus the timing and size evidence applies to the committed source rather than to
merely similar assembly.

Aggregate executable `.text` is **42,947 bytes** current versus **43,119 bytes**
pre-fix, a **172-byte reduction**.  The only changed workloads are:

| workload | current | pre-fix | delta |
|---|---:|---:|---:|
| spectral norm | 1,928 | 1,624 | +304 |
| hash table | 1,922 | 1,892 | +30 |
| zlib-ng Adler-32 | 2,171 | 2,263 | -92 |
| SQLite varint | 3,357 | 3,771 | -414 |

Primary retained evidence:

- `/home/user/results/postinline-size-ab/policy6-final-benchmark-os.json`
- `/home/user/results/postinline-size-ab/policy6-final-benchmark-os.md`
- `/home/user/results/postinline-size-ab/policy6-final-artifacts/`
- `/home/user/results/postinline-size-ab/policy6-final-text-size.json`
- `/home/user/results/postinline-size-ab/policy6-final-source-correctness.json`
- `/home/user/results/postinline-size-ab/policy6-final-source-byte-identity.json`
- `/home/user/results/postinline-size-ab/policy6-hash-runtime.json`
- `/home/user/results/postinline-size-ab/policy6-final-source-regression.log`

## Authentic kernel setup replay

All 24 authentic kbuild compile commands from
`/home/user/lccc-setup-postinline-build-clean-config.log` were replayed with the
current `lccc-i686`; every object compiled.  A strict link with the unmodified
Linux `setup.ld` correctly failed its 64-sector assertion.  A diagnostic link
changed only the two size assertions and produced:

- `.text = 30,431` bytes;
- `setup_size = 0x9000`;
- `setup_sects = 0x48` (**72 sectors**);
- `_end = 0x99c0`.

The nested-loop/hash refinement does not alter the preceding 72-sector kernel
result.  After the final `always_inline` exemption, all 24 setup objects were
recompiled and were byte-for-byte identical to the objects used for that link.
Evidence is retained under `/home/user/results/postinline-size-ab/kernel-policy6/`,
including every object, both byte-identity manifests, strict and relaxed link
logs, section sizes, and layout symbols.

The policy therefore fixes the post-inline size defect safely, but it does
**not** by itself solve the kernel's 64-sector blocker.  No boot claim is made.

## Prioritized unfinished work

1. **Reduce setup from 72 to at most 64 sectors without relaxing the linker
   script.**  The target is not merely 8 raw sectors: `setup_size` rounds to a
   4 KiB boundary, so `_end` must cross below the next boundary required by the
   genuine script.
2. **Diff the 24 current objects against the preserved GCC and LCCC baselines.**
   Rank functions by attributable bytes and repeated instruction forms rather
   than tuning another global inline threshold.
3. **Pursue i686 low-risk code-size/code-quality gaps.**  The earlier kernel
   audit identifies accumulator shuttling, repeated GEP/address construction,
   callee-save push/pop churn, and regparm argument staging as the largest
   structural gaps.  Validate each candidate on representative setup objects,
   the regression suite, and the paired user-space corpus.
4. **Repeat the strict authentic setup link after every measured improvement.**
   Relaxed links are diagnostics only.
5. **After setup fits, build the complete patched kernel with LCCC and
   `lccc-ld`, then boot a serial-console initramfs under QEMU.**  Exercise the
   relevant custom patch functionality where the virtual hardware permits and
   clearly list functionality requiring real hardware.
6. **Complete the requested compiler/linker oracle work.**  Keep the user's
   GCC 16.2/binutils 2.47 preferences and Clang/lld, ICC, ICX comparisons
   reproducible; use the exact mold i686 CMake preset if mold is needed.
7. **Obtain real-hardware PMU evidence later.**  Current paired wall-clock data
   is the strongest available VM proxy, not a substitute for cycles,
   instructions, branches, cache misses, and spill-load/store counters.
