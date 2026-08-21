# 2026-08-21 session 37 — RA-24 explicit register pointer locations

Base: `a8d1b7814e58974f75e70261db3316a30b8eab71` (PR #164).

## Structural defect

`CodegenState::resolve_slot_addr` encoded a register-resident pointer with no stack slot as `SlotAddr::Indirect(StackSlot(0))`. Correctness depended on every Indirect consumer first consulting a separate backend `reg_assignments` map. A forgotten check silently dereferenced frame offset zero (saved frame/return state) and exhaustive matching could not catch it.

## Implementation

- `CodegenState` now stores the exact `ValueId -> PhysReg` map supplied by stack layout.
- `SlotAddr::Reg(PhysReg)` is a first-class location; the dummy offset is deleted.
- `ArchCodegen::emit_reg_to_addr` materializes the physical pointer in each target's dedicated address scratch: `%rcx`, `%ecx`, `x9`, or `t5`.
- Shared scalar/i128 load/store, constant GEP, variable GEP, and memcpy dispatch consume the exact register.
- Shared ARM/RISC-V soft-F128 gains an explicit physical-home primitive.
- x86 x87 raw/f64/F128 paths, returns, scalar FP direct paths, and offset folds consume register addresses.
- i686 scalar, pair, x87, direct compare, offset, and `rep movsb` paths consume register addresses directly.
- AArch64 keeps direct `[xN]` FP forms and scaled/unscaled offset forms where encodable.
- RISC-V stages register addresses in `t5` for scalar/offset accesses.

Rust exhaustiveness checking exposed **51 consumer matches**. Every one received real architecture behavior; no panic/dummy fallback was added.

## Validation

- 985 unit tests passed, 6 ignored.
- 50/50 correctness.
- 378/378 lccc-only regressions.
- 600/600 phi CFG differential.
- 540/540 i686 alias differential.
- Dedicated native runtime covers register-pointer scalar, i64, FP, offset, and 64-byte copy operations.
- AArch64, RISC-V, and i686 compile the same scalar/wide/FP/F128/memcpy corpus.
- Static gate rejects any reintroduction of `SlotAddr::Indirect(StackSlot(0))`.
- gzip 1.14: 30/30; `longest_match` remains 330 instructions / 118 stack references.
- Godbolt pointer-load oracle: LCCC 3 instructions versus GCC 16.2/Clang 22.1/ICC/ICX 2. RA-24 preserves the direct `12(%rdi)` memory operand; the remaining extra return-register move is now isolated as the next accumulator-location/return-coalescing gap rather than falsely credited to address resolution.

RA-24 is an enabler rather than a claimed runtime speedup: generated code is intended to preserve existing direct-register forms while converting a silent convention into a compiler-checked location contract. RA-23 remains the next prerequisite for enabling broad hole-aware graph coloring and split allocation.
