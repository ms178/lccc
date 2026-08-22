# Session 57 — levkropp delta audit and hardened AArch64 CSINC fold

> **Superseded by Session 58:** the assembly-text CSINC implementation described
> here was removed and replaced by an SSA-level machine combine. See
> `2026-08-22-session58-full-lev-semantic-review.md` for the active design and
> validation.

Date: 2026-08-22

Upstream base: `bbd907bff857df3b93c0f74c5d1ddf4619dd6bfa`

Candidate fork tip audited: `lev/main` at `1e3d8c844d9ada4e6dee5db7064e5ed58d669dfe`

## Scope and result

The fork is 100 commits ahead of its merge base but 414 commits behind current
`ms178/lccc`; wholesale merging is therefore neither reviewable nor safe. The
latest fork commits were inspected as algorithms, then compared against the
current upstream implementation.

One useful code-generation idea was adopted, but not its implementation:
levkropp `24200ab4` recognizes

```asm
add  wTmp, wCount, #1
...
csel wCount, wTmp, wCount, ne
```

and emits:

```asm
csinc wCount, wCount, wCount, eq
```

The upstream implementation in this session adds materially stronger physical-
register liveness checks and five focused tests. On the integrated `sieve.c`,
the fold also exposes a redundant reload to existing cleanup, reducing the hot
counting loop by two dynamically repeated instructions:

```diff
- add   w5, w27, #1
- ldr   x0, [sp, #32]
  cmp   x0, #0
- csel  w27, w5, w27, ne
+ csinc w27, w27, w27, eq
```

`CCC_NO_CSINC_FOLD=1` is the A/B and bisection gate.

## Red-team findings in the candidate implementation

The original `24200ab4` text-level proof was not safe enough to merge:

1. It did not reject a use of the increment temporary between `add` and `csel`.
2. Its forward-only liveness scan could not see a read reached through a
   backward CFG edge.
3. It treated `ret` as transparent even though AArch64 `ret` implicitly returns
   `x0`.
4. It stopped at calls without modeling `x0`–`x7` as implicit argument uses.
5. It reasoned about physical registers as if assembly text retained SSA
   properties.

The adopted version requires a same-block producer/consumer, rejects every
intervening temporary use and base overwrite, rejects every explicit live-out
mention, models call/return implicit uses, rejects indirect branches, and checks
that each post-select backward edge reaches the increment definition before any
read of its temporary. If the select itself overwrites the temporary, the
live-out proof is unnecessary.

## Other latest fork commits

### `1e3d8c84` — void-pointer arithmetic / dead stores

Not cherry-picked.

* The IR-side failure is already prevented on current upstream by the
  default-closed escape policy in `aggregate_copy_forward`: an unmodeled
  pointer-arithmetic `BinOp` makes the alloca root escape, so its initializer
  stores cannot be deleted. The existing `void_pointer_arith` differential test
  passes.
* The proposed ARM change modifies `track_sp_bases`, but that helper currently
  has no callers. Current upstream deliberately rejected the fork's
  derived-base GDSE and uses a conservative per-function bail-out whenever an
  SP-derived address is materialized. Applying the patch would therefore alter
  dead code and would not fix an active path.
* The candidate commit reported 49/50 correctness and 21/23 progressive tests;
  current upstream produces 50/50 and 22/22 runnable progressive passes.

A future cleanup may remove the now-unused `track_sp_bases` helper after
confirming no planned pass depends on it, rather than patching dormant code.

### `a08f6768` — backedge PRE

Deferred. The idea is promising for Mandelbrot, but the 518-line pass arrives
without focused unit tests and duplicates operand-rewrite logic instead of using
the complete centralized IR visitors. It must be re-derived on current IR,
validated under `CCC_VALIDATE_SSA`, and fuzzed with zero-trip, multi-latch,
critical-edge, and floating-point semantic cases before adoption.

### Reverted recursive inlining (`49b7e085` / `c9285e2f`)

Correctly left reverted by the fork itself; no action.

## Validation evidence

Host constraints and reproducibility:

* 1.9 GiB RAM; an 8 GiB `/swapfile` was activated before Rust builds.
* LCCC built through `scripts/build_lccc_fast.sh`: Cargo `fastbuild`, Rust
  `opt-level=1`, no LTO, incremental, exactly two jobs.
* Cross execution used Debian AArch64 GCC 14.2 and QEMU 10.0.11.

Passing gates:

* focused CSINC tests: **5/5**;
* Rust library tests: **1084 passed, 0 failed, 6 ignored** (1090 total after the
  new tests);
* correctness differential suite: **50/50**;
* progressive suite: **22 passed, 0 failed**, SQLite full skipped because the
  amalgamation was absent;
* AArch64 `sieve.c`: LCCC object assembled and linked with
  `aarch64-linux-gnu-gcc`; QEMU output exactly matched AArch64 GCC:
  `primes up to 10000000: 664579`;
* the project Godbolt oracle queried Compiler Explorer ARM64 GCC 16.1
  (`carm64g1610`, `-O2`): GCC uses the `cinc` alias for the same conditional
  increment, while LCCC now emits canonical `csinc`; both dumps and provenance
  are saved under `engineering/evidence/godbolt/session57/`;
* full AArch64 regression A/B with and without `CCC_NO_CSINC_FOLD` was identical:
  **255 passed, 56 failed, 67 skipped** in both configurations. The 56 failures
  are pre-existing ARM-backend/test-filter debt, not hidden by this change.

QEMU wall-time medians were noisy and slightly negative (optimized/base about
1.02x over 15 interleaved runs), so they are explicitly **not** claimed as
performance evidence. The defensible evidence in this PMU-less environment is
semantic differential execution plus the exact hot-loop instruction reduction.
Real AArch64 hardware counters remain required before quoting a runtime win.

## Follow-up work, prioritized

1. **Real AArch64 measurement:** run interleaved pinned trials of sieve on a
   recent core and collect cycles, instructions, branches, IPC, and front-end /
   back-end stall metrics. Accept the fold based on broad non-regression and
   retired-instruction reduction, not QEMU timing.
2. **Move the combine earlier:** teach the machine-selection layer an SSA-aware
   `select(cond, add(x,1), x) -> csinc` combine. This would remove text-CFG
   reasoning and may prevent creation of the temporary entirely. Keep the
   peephole as a late fallback until coverage is equal.
3. **ARM regression classification:** split the 56 current failures into
   x86-only tests that the ARM driver should skip, unsupported features, real
   compile/assembly defects, and runtime miscompilations. Turn the driver strict
   for the resulting ARM-supported corpus.
4. **Backedge PRE research spike:** reimplement `a08f6768` with centralized use
   rewriting and at least zero-trip, multiple-latch, nested-loop, FP-contract,
   and SSA-validator tests. Compare Mandelbrot assembly and QEMU instruction
   traces before requesting hardware measurements.
5. **Dormant code cleanup:** audit and remove unused `track_sp_bases` /
   `parse_base_addr` machinery if no active optimization is intended to use it.
6. **Current fork delta:** continue newest-first review, but use patch-equivalence
   and current-upstream behavior rather than commit count; many old fork ideas
   have already been independently adopted or superseded.
