# 2026-08-21 session 38 — explicit accumulator homes and AB-14 parameter-order fix

Base follow-up: `207c26f0f67010e22ebc513c4d31312e34b93bb1` (PR #166).

## RA-23 structural slice

RA-24 made register pointer homes explicit but accumulator-only values still lived in a parallel `immediately_consumed` set. This round introduces one non-stack scalar location contract:

```rust
ExplicitLocation::Reg(PhysReg)
ExplicitLocation::Accumulator
```

Stack layout publishes accumulator candidates first and then overlays real allocated registers, so a physical assignment always wins. `resolve_slot_addr` consumes only `ExplicitLocation::Reg`; x86 store/materialization paths query `is_accumulator_location`. The duplicate `reg_assigned_locations` and `CodegenState::immediately_consumed` fields are removed.

This is a real RA-23 prerequisite, but not falsely marked complete: `compute_immediately_consumed` and `is_safe_sole_consumer` still determine accumulator legality from backend load order. They must move into RA before Tier-2 graph coloring can become default.

## AB-14 root cause and fix

The long-double reproducer was not an x87 arithmetic defect. Every backend still maps parameter homes by nth entry-block Alloca. Safe-leaf DCE removed dead parameter 0's Alloca while parameter 1's live F128 Alloca survived, shifting parameter 1 into slot zero. x86 then copied `%rdi` (the pointer) into the long-double home and loaded it with `fldt`.

DCE now preserves the complete positional parameter-Alloca prefix through the last live home. A wholly dead suffix is still removed, preserving the six-instruction copy64 optimization.

Runtime regression:

```c
void wr(long double *p, long double v) { p[1] = v; }
```

now writes and reads 7.5L correctly. A unit test pins the dead-param0/live-param1 prefix invariant.

Adjacent non-volatile F128 Load→Store pairs are additionally emitted as an exact 16-byte memcpy on every backend. This removes x87 truncate/reload intermediates and reduces the x86 function from 15 to 8 instructions; GCC/Clang/ICC/ICX emit 3. The remaining gap is the prologue copy of the stack-passed long double into its positional home.

## Validation

- 986 unit tests passed, 6 ignored.
- 50/50 correctness.
- 380/380 lccc-only regressions.
- 600/600 phi CFG differential.
- 540/540 i686 alias differential.
- RA-24 all-backend pointer-location regression remains green.
- The location consolidation is intended to be codegen-neutral; gzip remains at the validated 330 instructions / 118 stack references.
