# TASK-MS-08 — consolidate the CCC_ environment knobs into RaConfig

IDs: MS-08 · Priority: **P1** · Base: f657de55 · Hygiene: mixed-polarity
env parsing is a standing foot-gun (one kill switch was once wired
inverted).

## Objective

`regalloc.rs` reads 33 distinct `CCC_*` env names, `live_range.rs` 6,
`x86/codegen/prologue.rs` 11 (v3-audit counts) — mixed polarity (`CCC_NO_*` vs `CCC_*`), parsed with
ad-hoc string logic in hot paths. Consolidate into a single `RaConfig`
struct built once per driver invocation, threaded explicitly; keep the env
vars as the *construction* surface (backward compatible) but remove
re-parsing from the hot path and give every knob one owner.

## Files

`src/backend/regalloc.rs`, `src/backend/live_range.rs`,
`src/backend/x86/codegen/prologue.rs`, `src/driver/`.

## Acceptance

- Byte-identical assembly on the kernel corpus + regression suite with
  default env (pure refactor).
- One grep (`RaConfig`) lists every knob, its polarity, and its default;
  `cargo test --lib` includes polarity unit tests for every switch.
- No `std::env::var` calls in per-function code paths.

## Validation battery

`cargo test --lib` · regression corpus (byte-identical gate) ·
`CCC_TRACE_ALLOCSTATS` output unchanged · kill-switch smoke: every switch
flips its documented behavior.

## Do not

- Do not change any default while moving the knob.
- Do not delete `CCC_NO_*` aliases (bisection muscle memory).
- Coordinate with TASK-RA-06A — serialize edits to `regalloc.rs`.
