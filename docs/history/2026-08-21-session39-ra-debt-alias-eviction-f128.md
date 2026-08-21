# 2026-08-21 session 39 — RA debt cleanup, shared alias proof, exact F128 forwarding

Base: `75d16f90469ad1f92d07a9444c9ecc55a9992eb3` (PR #168).

Delivered:

1. RA-12: delete the zero-writer `exchange_eviction` policy bit. Eviction defaults explicitly to proven mode 3; mode 5 remains environment opt-in only.
2. RA-25: loop-memory promotion delegates affine separation to shared `alias::forms_disjoint`; all subtraction/end arithmetic is checked and overflow fails closed.
3. AB-14 follow-up: adjacent exact F128 Load→Store emits a 16-byte backend memcpy, reducing x86 from 15 to 8 instructions while preserving all payload bits.

A full RA-23 replacement experiment was implemented and vetoed: assigning durable stack homes and deleting consumer-order legality passed compilation but caused caller-home/frame regressions; enabling Tier-2 coloring still crashed huft/SQLite. Those unsafe changes were reverted. The validated explicit-location implementation remains intact.

Validation: 989 unit tests passed, 6 ignored; 382/382 lccc regressions. The complete suite runs after rebuilding the fastbuild binary.
