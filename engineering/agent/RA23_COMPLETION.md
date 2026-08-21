# RA-23 completion plan — allocator-owned accumulator locations

Base: `d57ef0866d68d7290d2bd9225d7dcb9166905c7b`.

## Non-negotiable invariants

1. Every live value has exactly one durable home (`Reg` or `Stack`), or an allocator-issued `Accumulator` assignment with an explicit producer→consumer clobber-free proof.
2. MachInst and mature ISel consume the same location table; neither infers location from missing slots.
3. Codegen-introduced stores are represented in slot interference before Tier-2 coloring.
4. Parameter ABI hints outrank transient values; no leaf-frame or copy64 regression.
5. gzip `longest_match` stack references cannot increase.

## Dependency-ordered implementation

### A. Allocator-owned accumulator contract — INTEGRATED

- `AccumulatorPolicy` is part of `RegAllocConfig`.
- RA emits verified `{value, def_point, consume_point}` assignments.
- Every backend passes the allocator result to stack layout; stack layout no longer recomputes candidates.
- x86/i686 no-home policy consumes the RA analysis API.
- Remaining deletion step: retire the compatibility implementation in copy_coalescing after its tests move to regalloc.

### B. Unified location consumption — IN PROGRESS

- Allocator assignments now publish through `ExplicitLocation`.
- MachInst explicitly rejects point-constrained accumulator values and falls back to mature ISel, preventing buffered lifetime extension.
- Next: native MachInst accumulator operands and the program-point location verifier.

### C. Tier-2 interference correctness

- Add definition-store points and closed boundaries to slot interference.
- Treat accumulator assignments as non-stack, but their durable fallback (when required by replay/calls) as a normal segment.
- Reproduce and eliminate huft/SQLite collisions before enabling coloring.
- Enable by default behind `CCC_NO_TIER2_GRAPH` only after differential gates pass.

### D. Additional RA structural work

1. RA-05: consume hole-aware segments in the scan rather than fat intervals.
2. RA-06: split at pressure/call boundaries and reload at next use, preserving SSA/phi edges.
3. RA-11: build immutable per-function range metadata once and share it across allocation waves/verifiers.
4. RA-19: centralize target PhysReg metadata (name, width, ABI class, clobbers) and delete duplicated numeric maps.

## Gates after every stage

- Unit, 380+ regression, 50 correctness.
- 600 phi CFG and 540 i686 alias differential.
- MachInst signed-GEP/fallback replay.
- huft, SQLite, Expat, zlib-ng, gzip 30/30.
- gzip stack references and kernel real-mode corpus.
- Godbolt GCC 16.2, Clang 22.1, ICC 2021.10, ICX latest.
