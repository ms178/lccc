# PR832 review follow-up

Base: 63e783b15586eca86f306bd75a52b82c57304bbb, whose source tree is exactly
S48's accepted 282be93bfedca7e27b2e6ced04e33b8eddcedf35. Historical commits and
receipts remain historical. Changes are small attributable follow-up commits;
rewriting the already-merged PR into a fictitious independently-tested stack
would not improve provenance.

## Initial disposition (execution evidence to follow)

- F1: documentation gap accepted; accidental removal allegation rejected.
  The owner explicitly retired live CE merge gates. Controller self-tests and
  local GCC differentials are NOT replacement GCC/Clang/ICX execution coverage.
- F2: conservative cost worth measuring. Ordinal-only refinement rejected:
  moving a loop load to a preheader crosses ordering events on earlier loop
  iterations, even if the load text precedes the acquire in its own block.
- F3: missing mechanical verification accepted. Hash consistency cannot prove
  historical measurement truth or reproduce original timestamps/hardware.
  Preserve old bytes and distinguish integrity, replay, and historical records.
- F4: repeated scans accepted as compile-time overhead; use an immutable scoped
  definition index, not a stale pass-wide cache across IR mutations. Duplicate
  definitions should fail closed in all build profiles, not just assert.
- F5: proposed equality assertion is WRONG: failed speculative planning normally
  advances packed.next. Recycling unreachable speculative IDs is legitimate.
  Test/refactor the commit-or-discard boundary rather than asserting no work.
- F6: compiler-side fixture integration accepted. Macro-rich assembly must be
  expanded before peephole testing; feeding .macro directives unchanged would
  only test the opaque-directive barrier, not the expanded instructions.
- F7: block-count-only assertion is insufficient: edges can change without new
  blocks. Check block identities AND terminators in assertions-enabled builds.
- F8: merged history cannot be retroactively split. Follow-up commits separate
  tests, production changes, CI and evidence; no single unreviewable rewrite.
- F9: explicit scalar-result documentation accepted (classification was correct).
- F10: review had no local execution. This agent's executed results will be
  recorded independently; no invented expert approval or numeric quality score.

## Corpus scope

All 45 registered forms are in scope. The supplied EPYC/KVM report ranks
wall-time versus GCC, not against current main and not on Raptor Lake.
Its worst 15, descending ratio: chacha20_block, byte_load_reassembly,
spectral_norm, sieve, sha256_transform, binary_trees, expat_xml_scan,
lz4_match_extend, sqlite_varint, fannkuch, hash_table, tls_seg_access,
mandelbrot, strlen_bench, linux_rbtree. Bounded Callgrind ranks may differ;
never substitute a small-input Ir rank for this native-time screen.

## Rejected work

Keep the earlier rejected boundary-index prototype and its measurements.
No new optimization is accepted merely because it looks cheaper in assembly.

## Executed follow-up, before final acceptance

* F3: `verify_evidence_hashes.py` verifies 69 retained historical files, their
  stored SHA-256 and gzip-decoded hashes, plus the original RAW-LOGS receipts.
  The read-only FAST/hosted gate and corruption/missing/duplicate/path tests
  pass. README distinguishes exact Git retrieval from experimental replay;
  incomplete historical command records remain an explicit limitation.
* F4: copy-root resolution now borrows one lazily constructed definition index
  per immutable reduction analysis. Duplicate SSA definitions reject in every
  profile. Tests cover duplicates outside the loop and rebuilding after mutation.
  This improves lookup complexity, NOT a measured compiler wall-time claim.
* F5: rejected-DAG test intentionally allocates a broadcast before failure,
  proves the private counter advanced and caller IR/counter stayed unchanged.
  The suggested equality assertion would reject this legitimate case.
* F7: assertions-enabled IVSR checks labels, terminator kinds and successor
  lists after each transform. Value-operand changes remain legal. Unit negatives
  catch same-count edge, kind and identity changes. Normal-profile tests do not
  claim execution of the assertions-enabled build; hosted debug remains required.
* F6: GCC's exact fixtures, including deliberate mutants, now execute before
  and after the real peephole pipeline. GNU as expands the macro fixture;
  emitted-address filtering removes inactive `.if` arms. All 40 fixture
  functions are retained, with an independent optimizing positive control.
  **The first run was RED:** XSAVE-upper and scalar-merge/implicit-blend mutant
  results changed from 0 to 1. These were real wrong-code bugs, not incorrect
  expectations that the original functions must always produce different text.
* The final VEX promoter now shares architectural upper-state effects with FP
  liveness: implicit state-image observations count; flag-only VEX instructions
  are not destination kills; unresolved transfers fail closed. Positive controls
  retain promotion after a proven full overwrite and for non-temporal/immediate
  instructions. Known explicit extraction/move forms are admitted explicitly.
* Scalar-copy propagation distinguishes a VEX scalar DATA input from its
  128-bit MERGE input. A private self-merge bracket remains optimizable; a narrow
  identity cannot replace an externally observed upper lane. The former
  suffix-only width test (incorrectly expecting 64/32 for the merge input) is
  corrected. One old copy-propagation expectation now retains its unproved
  merge feeder; this is not advertised as cost-free. All-45 A/B checks must
  establish the actual code-generation impact before delivery.
* Development library rerun: **4754 passed / 0 failed / 8 ignored**. Earlier
  red and intermediate failing runs are retained separately. This is not the
  final immutable FAST receipt.

## CI work reduction without removing corpus coverage

`bench.yml` is now a reusable lane called by `ci.yml` after `build`. It verifies
both commit and SHA-256 of that job's compiler artifact instead of installing
Rust and rebuilding the same compiler. Both runners are Ubuntu 24.04. The full
45-workload corpus, GCC16 comparator, nine timed rounds, correctness/static
quality gates and result artifacts remain. The aggregate requires the benchmark
lane. Caller-owned triggers/cancellation prevent duplicate push/PR benchmarks.
No free-tier concurrency limit or hosted elapsed-time saving is asserted without
an actual hosted run.

The parity checker follows reachable local reusable workflows, propagates caller
conditions/soft failures, rejects cycles/missing callees and does not credit an
uncalled `workflow_call`. Tests reject duplicate triggers, unverified artifacts,
rebuilds and corpus selection. Existing unrelated red-team/oracle jobs survive.
71 parity tests, 8 DAG/reachability tests and the parity command pass locally.

## F2 measured scope, not a license to weaken ordering

A separate-TU probe uses four ordinary global loads: two before and two after an
acquire atomic load, with a 20,000-iteration driver. Both LCCC main and GCC16 keep
all four inside the ordered loop. A guarded do/while positive control WITHOUT
the atomic hoists all four in LCCC; GCC additionally collapses the recurrence.
The initial for-loop probe did not hoist in LCCC even without ordering, so its
result is retained but cannot isolate the new ordering restriction. The corrected
control matters: a register-count or ordinal-only argument is not causal proof.

Each arm has actual assembly, correctness output, compiler/header identities,
fixed-geometry Callgrind counters and source. These single-thread diagnostic
arms have different synchronization semantics and are NOT interchangeable in a
concurrent program. No barrier removal or ordinal-only refinement is accepted.

## Measurement evidence improvement found during this audit

The existing Callgrind controller retained counters and stdout/stderr hashes but
not all raw process streams. New runs now also retain exact compiler, native and
instrumented argv, stdout and stderr, including failures and embedded NUL bytes.
Old streams are not retroactively invented; the fresh GCC baseline is rerun
with this recording. 46 mocked controller/corpus tests pass, including stale
stream removal on infrastructure failure. This does not change timed programs,
inputs, comparator flags, correctness checks or geometry.

## Final development checks before immutable FAST

After exact merge-width accounting, a whole-function wide-reader approximation
retained three unnecessary copies in libm leaf helpers. The first A/B sweep
reported +1 whole-program Ir (171495 vs 171494), with unchanged hot-loop Ir.
This was not silently waived: an immediate-return-only proof now excludes earlier
unreachable readers, while any intervening observer or backedge retains the
conservative check. Positive/negative regressions cover both. Library tests:
**4756 pass / 0 fail / 8 ignored**; raw-controller tests: **47 pass**.

The pinned final sweep covers all45, with three alternating pairs each:
**all45 candidate/main Ir ranges equal [1,1]**; all supplied worst15 ELFs are
byte-identical. The separate `PR832-CORPUS-STATUS.md` records every form and the
still-open optimization work. This is correctness hardening with preserved
measured Ir, not a claim to have fixed fifteen native-performance deficits.

CI's reusable lane changes the benchmark check's display context; repository
branch-protection rules, if they name the old standalone context, require owner
verification. `Test Suite` still aggregates every lane. Hosted execution has not
been asserted locally.

## First immutable FAST found the remaining copy-budget regression

On 9121f51, FAST was **252 pass / 1 fail / 6 slow skips**. The failing
vector-copy gate measured 49 copies against its unchanged budget of 48.
All runtime comparisons in that gate remained correct; strict Clippy, fmt,
and 303 strict linker tests passed. Assertions-enabled IVSR: 43/43 pass.
This is retained as a failed acceptance, not a deliverable stamp.

The extra copies feed scalar VEX MERGE operands. They cannot be forwarded
merely because the arithmetic uses a scalar: another instruction may observe
the merged high bits. The new refinement admits non-ABI temporaries only
when straight-line uses extract at most the established low element and
CFG liveness then proves the complete XMM value dead. Full-width observers,
state images, escaping merges, backedges, unknown transfers, wider extraction
and ABI argument/return registers have negative controls. The authoritative
call annotations are not broadened by trusting the old unannotated-call
staging heuristic. The liveness cache is refreshed after each rewrite.

This restores **48 copies, zero GP relay pairs**, without changing the
budget or removing any tests: vector-copy gate 47 pass / 0 fail / 0 skip;
library 4758 pass / 0 fail / 8 ignored. New all45 measurements and a new
immutable FAST receipt are required for this changed compiler.

The required fresh sweep on compiler source **75b74f1** is now complete:
all45 correct in three alternating pairs, all Ir ranges **[1,1]**, 44 identical
ELFs including every supplied worst15. Tool/controller/source identities are
identical before/after. The separate pinned Main/GCC16 baseline remains valid:
neither Main's binary, its source, the workload inputs nor the pinned tools
changed. It is not mislabelled as a new GCC run. Strict Clippy preflight is green.
The first failed FAST's swap-usage delta was a volatile resource counter, not a
source/tool change; the next identity controller records swap size/priority,
not changing occupancy. Both original identity files remain available.
