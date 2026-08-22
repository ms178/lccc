# Session 60 — optimal AArch64 conditional-increment leaves

Date: 2026-08-22

Upstream base: `eff11d5d` (latest merged `ms178/lccc` main).
All LCCC builds used `scripts/build_lccc_fast.sh` only.

## Problem

The SSA CSINC combine selected the right hot instruction but still assigned its
parameters and result to callee-saved registers. A two-parameter leaf therefore
paid a frame, four saves/restores, and copies:

```text
13 instructions, 3 loads, 3 stores
```

GCC 16.1, GCC trunk, and Clang 22.1 all emitted the irreducible:

```asm
cmp  w1, #0
cinc w0, w0, ne
ret
```

## Design

A general x0-x3 allocator is not yet safe: current AArch64 lowering also uses
x0/x1 as implicit accumulators and staging registers. Blindly adding them to
the pool would repeat the class of scratch-clobber bugs already found in call
staging and compare lowering.

Instead, the backend now has a strict fixed-register plan for exactly the
machine-combined conditional-increment leaf family. It is admitted only when:

* there is one block and one Return;
* all parameters are non-aggregate integer AAPCS64 register parameters;
* there is exactly one Add and one Select, plus at most one directly fused Cmp;
* Add is `base + 1` and its result is the selected increment arm;
* base and condition are incoming parameters;
* widths satisfy the existing CSINC legality contract;
* no extra instruction, duplicate ParamRef, Add, Cmp, or Select exists;
* `CCC_NO_CSINC_FOLD` is not set.

The plan keeps parameter `i` in `xi`, keeps the result in `x0`, gives skipped
producers non-stack homes, suppresses the frame, and marks already-correct ABI
homes as pre-stored without emitting moves. A separate condition-width bit
ensures a U32 argument is tested with `cmp wN`, not `cmp xN` (AAPCS64 does not
promise meaningful upper bits for a 32-bit argument).

This is intentionally narrow. Any shape not fully proven falls back to the
existing scratch-aware allocator and framed ABI implementation.

## Result

Local LCCC now emits:

```asm
cmp   w1, #0
csinc w0, w0, w0, eq
ret
```

`cinc w0,w0,ne` is the assembler alias of the same CSINC encoding. Static oracle
results:

| Compiler | Instructions | Loads | Stores | Branches |
|---|---:|---:|---:|---:|
| LCCC fastbuild | 3 | 0 | 0 | 1 |
| ARM64 GCC 16.1 | 3 | 0 | 0 | 1 |
| ARM64 GCC trunk | 3 | 0 | 0 | 1 |
| Clang 22.1 AArch64 | 3 | 0 | 0 | 1 |

This reaches the three-instruction lower bound and therefore ties, rather than
claims to beat, state of the art for this exact leaf. The maintained regression
now rejects any frame, move, spill, or instruction count other than three.

## Validation

```text
Focused CSINC assembly/gate test : PASS
AArch64 CSINC runtime differential: 1 PASS, 0 FAIL
Rust library tests                : 1089 PASS, 6 ignored, 0 FAIL
Correctness suite                 : 50/50 PASS
x86 regression                    : 436 PASS, 0 FAIL
ARM regression                    : 268 PASS, 51 pre-existing FAIL, 68 SKIP
GCC torture first 500             : 454 PASS, 46 retained FAIL
Four fixed GCC torture reproducers: 4/4 PASS under GAS 2.47
Linker suite                      : 160 PASS, 0 FAIL
```

The GCC torture corpus revision remains
`ad82e41f1b997dedd2cf227cb410afd8763d89c8`; assembly validation uses GNU
assembler `2.47.20260726`.

## Performance screening

The fastbuild LCCC driver and integrated `lccc-ld` produced correct binaries for
all four measured corpus entries. Pinned randomized host-VM ratios versus GCC:

```text
aarch64_select_patterns  1.376
Gzip CRC-32              1.085
zlib-ng Adler-32         1.472
Expat XML scan           2.135
geomean                   1.472
```

These are honest host-VM gaps, not AArch64 hardware measurements. The leaf
change is AArch64-only, so its defensible performance evidence is elimination
of ten static instructions and all frame traffic. Real ARM PMU measurements
remain required for cycle claims.

## Next highest-value gap

`select_pressure` is still 31 instructions versus 6 for GCC/Clang because the
general leaf allocator cannot yet preserve several ABI argument registers
through multiple selects. The correct follow-up is machine-IR scratch-clobber
modeling and fixed-register interference, not widening this exact fast path
until it becomes another implicit-register hazard.
