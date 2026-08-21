# Session 40 — RA-23 allocator-owned accumulator assignments

Base: latest PR #170 (`76fef786`).

Implemented multiple dependent stages:

1. RA computes `AccumulatorAssignment { value_id, def_point, consume_point }` under target `AccumulatorPolicy`.
2. Assignments verify a single use at exactly `def_point + 1` in liveness program-point order.
3. All four backend prologues receive the assignments from `RegAllocResult` and publish them to stack layout; stack layout no longer independently selects accumulator values.
4. x86/i686 no-home pressure policy consumes the RA analysis API.
5. MachInst recognizes explicit accumulator locations as point-constrained and falls back to mature ISel rather than buffering them beyond the verified consumer.

Validation: build/check clean; `machinst_signed_gep`, copy64, leaf caller-home gates, and 382/382 lccc regressions pass.

Follow-up in the same session added:

- hard program-point verification of every allocator assignment (unique value, exact def, exactly one use, consume=def+1);
- physical-register assignments filter overlapping accumulator assignments deterministically;
- MachInst producer and consumer instructions both fail closed on point-constrained accumulator locations;
- Tier-2 occupancy now includes every IR definition write, including multi-def phi Copies.

The Tier-2 gate remains research-only: huft and SQLite still crash with coloring enabled, proving another alias/live-through edge class remains. Default codegen is clean at 382/382 regressions.
