# Follow-up: 2026-10-14C relocation audit

## Baseline and preservation

Upstream fetched from https://github.com/ms178/lccc.git: `3c1f893480dc81f0fdc2bbf7973761c7266d6639`.
The previous session's seven commits have been incorporated upstream (PR #822,
then reconciled with PR #821). Direct comparison confirmed identical JUnit
validator and all previous session evidence files. The GEP changes are a
superset, including both sets of tests. Replaying the old commits duplicated
already-merged tests; the research branch therefore starts at current main,
with the original history retained in `preserved-session-before-refresh` and
the previous delivery archives. No in-flight source edits were lost.

A14-14 and A14-19 were closed upstream: do not duplicate those fixes. This
session targets the still-open A14-17 relocation-dispatch contract, starting
with malformed ELF negative controls and supported relocation positive controls.
The final status and measurement results will replace this in-progress note.

## Measurement constraints

4 GiB swap activated before builds on the 2-vCPU / 1.9-GiB VM. Compiler builds
use fastbuild, opt-level 1, j2, no incremental/debug data (memory budget).
Only surgical gates and `ci_local.sh --fast` will run locally, never full/slow CI.
Valgrind/Callgrind measurements are simulated instruction/cache counts, not
14700KF cycles or evidence of superiority over other compilers. Godbolt results
will retain actual compiler IDs, flags, outputs and errors; moving aliases do
not certify recent vendor releases.

## Packaging observation

The initial empty-session save correctly refused an empty Git bundle, but only
after publication had started. This is another instance of open A14-32. The
pre-refresh patch was preserved separately before attempting any snapshot.
Do not interpret this failed save as a gated delivery.
