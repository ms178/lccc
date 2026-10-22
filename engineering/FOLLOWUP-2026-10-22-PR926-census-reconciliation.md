# PR #926 follow-up: census reconciliation and function-size audit

Date: 2026-10-22. This records the source-count attribution and the structural changes made after rebasing PR #926 onto `32769db4`.

## Findings and decisions

- The committed census baseline was already stale at source commit `857d11c8`. Running the current `scripts/audit_census.py` scanner over archived source trees measured the following:

  | Metric | `origin/main` | Source at `857d11c8` | Source after this follow-up | Previous branch baseline |
  | --- | ---: | ---: | ---: | ---: |
  | Production panic sites | 837 | 836 | 836 | 835 |
  | Functions over 300 lines | 229 | 230 | 229 | 229 |
  | Distinct environment-knob literals | 516 | 517 | 516 | 515 |

  Therefore the panic count is not a newly introduced site: the branch has one fewer measured panic site than `main`, and the source at `857d11c8` already measured 836. The old baseline undercounted it by one. Likewise, after removing the branch-only `LCCC_DEBUG_AGGCOPY` debug switch, the current distinct-knob set matches `main`; the old baseline undercounted upstream/source by one. `CCC_DISABLE_PASSES` is not a new knob.

- Removed the one-off `LCCC_DEBUG_AGGCOPY` environment switch and its trace. No test or CI gate depended on it; retaining it would add a distinct knob solely for optional diagnostics. The aggregate-copy pass remains covered by its IR and code-generation checks, and can still be disabled with the existing `CCC_DISABLE_PASSES=aggcopy16` control.
- Extracted the 257-line consumer-width match from `pointer_vector_arg_width` into `pointer_vector_arg_width_consumer`. The conversion, extension, and consumer match arms and their return conditions were moved without changing classification logic. This takes the function-size metric back to its existing 229 limit.
- Extracted the AArch64 `MulAddF64x2Hoisted` emission body into `emit_mul_add_f64x2_hoisted`. The same address selection, registers, loads, multiply/add sequence, and stores are emitted; this brings the source count of functions over 1,000 lines down from 28 to 27.

## Baseline update

`scripts/audit_census_baseline.json` now records the measured source state: `panic_sites=836`, `knob_literals=516`, and the improved `fns_over_1000=27`. The undocumented-knob ratchet was tightened from 275 to the measured 273. Other limits remain unchanged. These are source-attributed corrections and ratchet tightenings, not a waiver for a new panic site or a new knob.

## Verification

- A byte-for-byte comparison confirms the extracted vector consumer-width match is identical to the pre-refactor table.
- A source check confirms all seven AArch64 instruction templates are unchanged and remain in the same order after extraction.
- `python3 scripts/audit_census.py --check` passes with all nine metrics at or below the updated baseline.
- `bash tests/regression/check_env_test_hygiene.sh` passes (146 pass-pipeline environment reads against the 147 ratchet).
- `git diff --check` passes.
- This sandbox has no `cargo`, `rustc`, or `rustfmt`; Rust compilation and regression tests must be confirmed by PR CI. No performance improvement is claimed for these structural extractions, and no matched benchmark was run for them.
