# Main 1dd907def integration: preserve both lazy builtins and diagnostics

Status: merge validation in progress, not acceptance or production readiness.

The validated 045c1c3b5/ac28 follow-up cohort is recorded separately in
NESTED-TRAMPOLINE-AND-MS-FRAME-REPAIR-2026-11-02.md. A subsequent fetch found
main 1dd907def, including lazy file/time builtins, ownership masks, host-zone
rendering and conditional-result carrier repairs. Those source changes need
new evidence; the ac28 totals are not attributed to this merge.

## Semantic conflict resolution

Keep upstream's single builtin table, lazy file stack, timestamp cache and
compiler/user ownership masks. Keep this branch's lexical expansion-effect
queue (pragmas interleaved with date/time diagnostics), presumed diagnostic
filename and per-definition fast date/time warning tag.

The old eager timestamp computation at every preprocessing entry is removed,
not retained beside the new lazy cache. Both stored date/time expansions and
lazy timestamp expansions queue a diagnostic at actual use. A user replacement,
probe, unused argument or stringification must not acquire an invented builtin
warning; the existing GCC differential warning matrix remains mandatory.

Upstream deliberately adopts Clang's SOURCE_DATE_EPOCH rule for timestamp,
where GCC instead uses the physical file's mtime. Preserve both oracle cohorts:
physical mtime without the epoch (GCC, Clang, LCCC), and deterministic epoch
with the epoch (Clang, LCCC). This is an explicit semantic split, not dropping
the old physical-file test or pretending that GCC implements Clang's rule.

## A proved merge interaction, not a hypothetical lead

The first merged fastbuild compiler was executed before the restoration fix.
`/home/user/1dd-builtin-restore-before.log` records:

| compiler-owned builtin after push/temporary redefine/pop | merged LCCC | GCC 16.2 |
| --- | --- | --- |
| DATE visible in `-dM` | yes (wrong) | no |
| COUNTER still defined | no (wrong) | yes |
| TIMESTAMP still defined | no (wrong) | yes |

A replacement-list-only save cannot represent a lazy builtin with no table
entry, nor restore the compiler/user ownership and suppression state.
`SavedMacroDefinition` now snapshots those properties as well as the optional
replacement list. Restoration writes the saved state directly, rather than
calling source `define`/`undefine` operations that would change its origin.
For compiler FILE restored inside a header, the replacement uses the current
presumed filename, not the path at the push site. The counter value itself is
not rolled back.

The expanded regression checks all eight builtins in compiler-owned,
undefined and user-owned states, in both preprocessing and macro-dump modes,
plus FILE restoration inside a header. The existing warning/error promotion,
pragma ordering, argument-side-effect and dump-origin checks remain present.

## CI merge: coverage union before rebalance

Taking either conflicted workflow wholesale would lose gates. Retain the
branch's authenticated Intel ABI/mitigation and combined Microsoft ABI gates;
add all three upstream gates (builtin scope, host zones, conditional carriers).
A Python Counter comparison proved that every executable workflow line and its
multiplicity from both parents remains in the result. This check excludes
comments but does not merely compare step-name sets.

Join and re-split frontend with `scripts/ci_lane_split.py`, using the parents'
recorded measured costs, not invented measurements. There are 153 gates across
three shards, predicted 318.5 s each (prediction, not measured hosted latency).
The structural lane-plan and disjoint fast-shard audits both pass. Census holds
without changing this branch's caps; upstream itself adds its host-time FFI
sites and corresponding unsafe baseline, 25 to 27. No extra unsafe is added by
the merge repair.

## Still open

Record the final integration-gate results after the serialized process exits.
Current-source boot-only size, full Linux/objtool/QEMU, all inventoried warning
and option families, remaining mitigation/ABI boundaries and green hosted CI
remain required. No universal correctness, zero-warning kernel, or kernel-wide
performance claim follows from these focused repairs.

## Executed post-merge validation

The serialized integration process exited zero (`ALL LATEST-MAIN INTEGRATION
GATES PASS`), logs `/home/user/1dd-final-*.log`:

* 7679 library tests pass, 0 fail, 10 existing ignores; execution 14.64 s;
* fastbuild all binaries pass (1m00s);
* 332 object/runtime/frame contracts pass;
* builtin-scope and conditional-carrier gates pass;
* host-zone sweep: 420 observations, zero failures;
* GCC date/time warning/origin matrix passes; the strengthened final replay
  passes **688 checks**, including user-body equality, nested counter saves
  without resetting its value, and diagnostic filename independence;
* mandatory 32/64-bit full-hash mask, parity, CLZ and four-target DWARF pass;
* all-target Clippy warnings-denied passes (2m49s);
* strict authentic usercopy/configured objtool passes;
* census, lane plan, fast-shard partition and parent coverage-union checks pass.

Binary identities:

* lccc: `c74ac25fd3a8d4578eb83cb60c63cc217bb9d437a7478fabd9996c344f28eb0a`
* lccc-i686: `872115c466cabbe97b6e5f98aacedc544a53d52e1374ca8661b921566a2a7418`
* lccc-ld: `8939c9d6dc1e7b1ec02ffd43d2ab1d2d647e1c63895e12c5069d87fe961f1cfc`

This validates the merged compiler code, not a fresh whole-kernel boot. The
acceptance gaps above remain; proceed with boot-only before full kernel/QEMU.
