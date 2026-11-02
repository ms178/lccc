# Incoming MEMORY objects: ABI proof and measured capture removal

## Change and legality

The caller's SysV x86-64 MEMORY argument is already the callee's by-value
parameter object. Creating another object, copying every rounded eightbyte and
then aligning its address at each use is unnecessary when the incoming alignment
satisfies the local object's requirement. Bind the parameter alloca to a positive
virtual-frame slot instead: RBP + 16 + argument offset, or entry RSP + 8 + offset
in FPO mode. The existing frame operand emitter handles pushes/frame size.

Only `StructStack` and `LargeStructStack` qualify. Scalar ParamRefs, by-reference
classes, incomplete/mismatched storage, absent allocas, misaligned offsets,
non-power-of-two alignments and unrepresentable extents fail closed. The lookup
uses `param_alloca_values`, never the ordinal position of an alloca. Incoming
homes never enter local slot pools and never receive a runtime align-up.
Function reset clears the map; functions without candidate classes avoid the
extra IR scans. Mutation, volatile accesses and escaped addresses still refer
to one callee-owned object. Outgoing areas are below the frame; dynamic
realignment and alloca retain the frame pointer through the existing FPO veto.

The audit exposed a separate existing ABI bug. An aligned typedef can raise an
aggregate PARAMETER OBJECT's alignment without changing stack argument layout.
Previously that request was folded into `struct_align` and displaced incoming
objects after a stack-passed integer. Keep carrier alignment in `struct_align`,
put the stronger object requirement in `param_align`, and take their maximum
for the alloca. Objects needing stronger alignment keep their private aligned
home. The old compiler fails the fixture's `aligned_object` check (exit 6);
GCC 16.2 agrees with the correction. Clang 23.1.3 uses the same ABI positions
but does not honor this stronger typedef alignment on aggregate parameters;
the fixture checks its underlying alignment and checks GCC/LCCC's stronger
object guarantee separately, without skipping the value/ABI comparison.

## Evidence

`engineering/evidence/incoming-memory-2026-10-24/` retains representative
kernels, source/assembly hashes, counts and raw timing pairs. The compiler
baseline is the merged main tree at the start of this work. Both binaries use
the fastbuild profile; swap was enabled. Matrix commands:

```
lccc -O2 -mavx2 -mfma -S tests/regression/vector_ext/matrix.c -o matrix.s
gcc-16 -O2 -mavx2 -mfma -S tests/regression/vector_ext/matrix.c -o gcc.s
clang-23 -O2 -mavx2 -mfma -ffp-contract=fast -S tests/regression/vector_ext/matrix.c -o clang.s
```

Count instruction lines within `.type ..., @function` / `.size` boundaries,
including `ret`, excluding labels/directives/comments and the `tern_count` data
object. There are 377 common functions:

| compiler | instructions |
| --- | ---: |
| LCCC before | 2582 |
| LCCC after | 2537 |
| GCC 16.2 | 2441 |
| Clang 23.1.3 | 2133 |

| changed function | before | after | GCC | Clang |
| --- | ---: | ---: | ---: | ---: |
| cf_wrap_phi | 32 | 10 | 6 | 12 |
| nine | 20 | 12 | 9 | 15 |
| nine4 | 17 | 11 | 9 | 9 |
| s4i_add | 25 | 16 | 11 | 9 |

The wrapper frame drops from 152 to 40 bytes. The separate 24-byte scalar
`sum_big` fixture drops from 12 to 4 instructions and from a 24-byte local
frame to no local frame. All four matrix improvements have tightened ceilings.

The reproducible, CPU-pinned, call-inclusive benchmark alternates old/new order,
discards a warmup pair and checks every final lane bit. Twenty-one pairs of
30 million dependent calls on this virtualized Xeon host give:

- `cf_wrap_phi`: median before/after **2.0817704848**, ratio MAD **0.0199516153**.
  This is a real measured win for the specified wrapper microkernel, not a
  whole-program or cross-architecture claim.
- `sum_big`: median before/after **0.9904893824**, ratio MAD **0.0107644707**.
  No meaningful runtime improvement established for the small scalar kernel.

```
python3 scripts/benchmark_incoming_byval.py --kernel cf_wrap_phi \
  --before before-full.s --after after-full.s --cc gcc-16 \
  --samples 21 --iterations 30000000 --out timing-wrapper.json
```

The wrapper driver reproduces the matrix's actual 64-byte record (32-byte vector
PLUS integer tag). A mistaken vector-only record uses a different ABI; the
benchmark checksum rejected that initial driver before retaining any timings.
Branch labels are made private per A/B kernel. Four compiler-free contracts
cover extraction, missing kernels, budget rejection and label independence.

## Correctness work

The ABI gate covers ten behaviors, with 41 signed input seeds, both optimization
levels, both frame modes, both capture-switch states and all four compiler
pairings. GCC and Clang each pass 32 native executable pairings. Behaviors
include source-object isolation, mutation through an escaped address, repeated
address identity, volatile object accesses, packed fields, GP exhaustion, sret,
variadic arguments, dynamic alloca and outgoing 64-byte realignment.

Local full suite: 6168 library tests and one linker test passed; ten existing
library and three existing documentation tests remain ignored. Strict all-target
Clippy passed. Eleven vector runtime levels pass with the tighter ceilings;
GNU vector alignment has 26 passing groups; variadic over-alignment has four
pairings times 25 runs; the AVX ABI differential has 192 passing Clang legs.
CI/local gate parity passes (378 commands, 27 invocation contracts).

The new invocation-owned `CCC_NO_INCOMING_BYVAL_HOME` switch is deliberately
retained to isolate this structural transformation in ABI and runtime A/B
checks. The census baseline increases by this ONE documented knob; no panic,
unsafe, ignored-test, large-function or undocumented-knob budget increases.
The authenticated ICX lane runs the same incoming-object gate with ICX 2026.2;
local ICX is not installed and no local ICX result is claimed.

## Remaining requested scope — not claimed complete

This change does NOT constitute the complete Intel/AMD instruction-manual audit
requested alongside it. Instruction-set metadata and publication dates do not
prove operand semantics, exceptions, masking or feature legality.

The current official Intel instruction reference is 325383-093US, September
2026, linked by the index updated October 9:
https://cdrdv2-public.intel.com/929352/325383-093-sdm-vol-2abcd.pdf .
The reference-data checkout is Intel XED release v2026.08.23, commit
`0bcb6237345c5066726dcc08b3d87928df3b5b26`; it predates that SDM, so it cannot
alone establish a complete current-manual universe. AMD volume 4 publication
metadata identifies 26568 rev 3.27 (July 2026); the full current instruction
text has not been audited. The earlier older-PDF/viewer-only evidence must not
be relabeled as a current semantic review.

The outstanding ledger must separately account for manual/version/page,
encoding and operands, supported/explicitly unsupported status, lowering and
encoder paths, target-feature/OS-state checks, masking/fault suppression,
upper-register writes, rounding/NaN/exception behavior and executable negative
and differential tests. Current GCC/LLVM development-source inspiration is
also not executable GCC-17/LLVM-git performance evidence. Neither a complete
manual audit nor superiority over all compilers is claimed here.
