# Session 48 — register-resident integer reduction accumulators

**Base:** `5f2d37e7a6bd0e50080d7cc4d78316930dbc117f` (PR #188)

## Why this remained valid after the latest merges

PR #187 added two independent reductions per loop, but its generated I32
accumulators still lived in stack slots. On the tracked `double_reduction`
workload the default and `CCC_NO_REDUCTION_VECREG=1` assembly were identical:
four packed accumulator updates read a frame slot and every update wrote it
back. The existing reduction-home collector covered only F32/F64 classes.
This was session 47's highest-ROI follow-up and remained untouched by PR #188.

## Implementation

The fail-closed x86 reduction collector now recognizes three audited integer
classes in addition to its four FP classes:

- AVX2 I32x8 zero/load/add/multiply/horizontal;
- SSE2 I32x4 zero/load/add/multiply/horizontal;
- SSE2 I64x2 zero/load/add/multiply/horizontal.

Only the loop-carried copy web is retained. Transient loads and products remain
on the established deferred/stack path, calls and unknown intrinsics still
reject the function, and width classes cannot merge.

The corresponding x86 emitters were completed before admission:

- integer zero producers initialize an assigned XMM/YMM home directly;
- I32/I64 horizontal reductions use the width-aware register loaders;
- the software-lane I64x2 multiply uses register-aware operand loaders;
- `VecMulI64x2` is declared an XMM2 scratch clobber.

The scratch audit exposed a separate gating bug: when XMM2 was quarantined,
`xmm_regs` correctly began at PhysReg 21/XMM3, but `x86_fp_pool` tested only
whether the first register was exactly PhysReg 20. It therefore disabled the
entire x86 SIMD allocator in precisely the functions requiring the quarantine.
The target check now accepts any first register in the x86 SIMD range 20..=33.
The I64 dot structural gate requires an XMM3+ accumulator and therefore pins
both the quarantine and this corrected target detection.

## Generated code

`double_reduction.c::main`, treatment versus the same compiler with
`CCC_NO_REDUCTION_VECREG=1`:

| Metric | Register homes | Stack control | Delta |
|---|---:|---:|---:|
| Static instructions | 215 | 227 | -12 |
| All frame references | 39 | 63 | -24 |
| Vector frame accesses | 2 | 22 | -20 |
| Packed adds reading accumulator slots | 0 | 4 | -4 |

The four independent accumulators update YMM6/YMM7 and YMM10/YMM11 in place.
The four loop-invariant zeros are initialized directly in YMM2..YMM5. Forced
SSE2 I32 and native I64x2 sum/dot kernels likewise update XMM homes in place.
The kill switch reproduces the pre-patch assembly exactly.

## Runtime evidence

The reusable `benchmark_reduction_vecreg_ab.py` now accepts an expected output
and kernel description, allowing the existing treatment/control protocol to
screen integer reductions without a duplicate harness.

A 24-pair CPU-0-pinned VM screen of the complete deterministic
`double_reduction` workload measured:

- register/stack geometric mean: **0.725076**;
- bootstrap 95% interval: **0.600703 .. 0.880672**;
- register path faster in **23/24** pairs;
- medians: 95.20 ms register, 124.12 ms stack.

The repository benchmark runner's independent 11-pair screen measured current
LCCC at **0.895× GCC** (1.12× faster, interval 0.875..0.905). These are VM
wall-time results only; no PMU is available. Raw samples and disassemblies are
under `/home/user/artifacts/session48-i32-reduction-vecreg-timing.json` and
`/home/user/artifacts/session48-double-reduction-oracle/`.

The live GCC/Clang/ICC/ICX oracle still exposes whole-function static debt:
LCCC 221 instructions versus GCC 92, Clang 133, ICC 212, and ICX 150. The
integer-home change owns only 12 instructions and 24 frame references; array
initialization, scalar remainder setup, duplicated shared loads, and general
loop-control quality remain separate work. The retained oracle is under
`/home/user/artifacts/session48-double-reduction-ce/`.

## Correctness and safety audit

- Register admission remains exact-width and copy-web-only.
- Unknown intrinsics, inline assembly, memcpy, and call-spanning intervals
  remain fail-closed.
- XMM2 is removed from the pool whenever I64 lane multiplication uses it as
  scratch; no live value can be silently overwritten.
- Horizontal reducers consult allocator assignments before any pointer-style
  fallback, preventing vector bits from being interpreted as an address.
- Unassigned/kill-switch code generation is byte-identical to the original
  `double_reduction` assembly.
- Integer arithmetic order within each lane and the existing horizontal tree
  are unchanged; only storage location changes.
- No new unchecked indexing, pointer arithmetic, alias assumption, or shared
  mutable compiler state was introduced.

## Coverage

- New `check_integer_reduction_vecreg_codegen.sh`: AVX2 four-accumulator I32,
  forced-SSE2 I32, I64 sum, and I64 dot treatment/control assembly.
- Existing multi-reduction, single FP reduction, and GCC-differential integer
  runtime tests remain mandatory.
- The i686 fused-multiply-add regression no longer depends on `printf`, so a
  host without 32-bit libc can still compile it and report the missing ELF
  interpreter honestly rather than an unrelated undefined-symbol failure.
- The Tier-2 graph gate now accepts equal enabled/control frames: newer RA can
  leave no colorable huft values, while any frame growth still fails.
- All changed Rust files are formatted with rustfmt 1.9.0 using the 2024 Style
  Guide edition.

## Final validation

- Fastbuild (`-O1`, no LTO, `-j2`) and active 8 GiB swap.
- Rust library tests: **1075 passed, 0 failed, 6 ignored**.
- Differential corpus: **381 passed, 0 failed, 7 GCC-oracle skips, 2 honest
  host-i386-loader skips** across 390 tests.
- Shell runtime + structural suite: **400 passed, 0 failed, 2 host skips**.
- Intrinsic differential suite: **3/3 passed**; focused F128, blend, AVX2,
  forced-SSE2, I64, FP reduction, and multi-reduction gates all pass.
- Pinned GNU gzip 1.14: **30/30 tests** in treatment and control plus exact
  round trips; three-round VM treatment/control geomean **1.000239** (neutral,
  no whole-project speed claim).
- `cargo fmt --all -- --style-edition 2024` was run; targeted 2024-style checks
  pass for every changed Rust file.

## Remaining valid follow-up

1. Strict non-contracted FP dot and integer dot products still materialize a
   transient `VecLoad` in a frame slot before `vmulps/pd` or software I64
   multiply. Extend forwarding only with exact source-liveness and alias proof.
2. Multi-reduction shared-load dedup remains valid: `a*b` plus `a*c` loads `a`
   twice. CSE must preserve address-space, offset, and memory-clobber legality.
3. More than two accumulators remain intentionally rejected until a measured
   pressure/profitability model exists.
4. Repeat the VM screens on the requested i7-14700KF with fixed P-core affinity
   and PMU counters.
