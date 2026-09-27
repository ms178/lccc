# Notes

## Dependency map observed

`SSA lowering -> GLA (pre-phi-elimination planner) -> stack layout / physical RA -> phi copy materialization -> x86 address and instruction selection -> emitted assembly -> regression and benchmark runners`.

At `-O0`, `CodegenOptions::disable_regalloc` keeps physical register allocation disabled because phi elimination can create non-SSA multi-definition value IDs.  Stack homes are consequently authoritative.  Global Location Allocation and optimized physical allocation are not implicated in this failure.

## Root cause

The direct accumulator-address load in `memory.rs` uses `acc_has_verified(ptr)` as a proof that `%rax` is the load base.  That proof is valid for optimized SSA IR, but not at `-O0`: phi elimination can reuse a numeric ID for more than one definition.  A newly parked earlier definition can therefore pass the cache check and emit a dereference through a stale `%rax` base.

## Legality

The new gate is `disable_regalloc`, the existing non-SSA `-O0` contract.  It changes neither ABI locations nor argument order: it selects the existing stack-home address materialization instead of the accumulator-only address form.  Optimized SSA paths retain the fast path.
