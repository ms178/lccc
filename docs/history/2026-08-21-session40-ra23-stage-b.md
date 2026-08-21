# Session 40 — RA-23 allocator-owned accumulator assignments

Base: latest PR #170 (`76fef786`).

Implemented multiple dependent stages:

1. RA computes `AccumulatorAssignment { value_id, def_point, consume_point }` under target `AccumulatorPolicy`.
2. Assignments verify a single use at exactly `def_point + 1` in liveness program-point order.
3. All four backend prologues receive the assignments from `RegAllocResult` and publish them to stack layout; stack layout no longer independently selects accumulator values.
4. x86/i686 no-home pressure policy consumes the RA analysis API.
5. MachInst recognizes explicit accumulator locations as point-constrained and falls back to mature ISel rather than buffering them beyond the verified consumer.

Validation: build/check clean; `machinst_signed_gep`, copy64, leaf caller-home gates, and 382/382 lccc regressions pass.

Remaining completion work is explicit MachInst accumulator operands, hard program-point verification, then Tier-2 definition-store interference and default enablement.
