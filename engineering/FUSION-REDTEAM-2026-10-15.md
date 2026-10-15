# Compare/branch fusion red team — 2026-10-15

## Status

S84 (`fd2aba49`, upstream base `f07f4c51`) passed 4,800 Rust library tests
(8 ignored), but is **not accepted**. Five executable counterexamples then
invalidated its inherited late boolean-spill fusion. Candidate/control builds
were stopped before publication of a new compiler or benchmark results.

## Executed counterexamples

The exact production late-fuser body was extracted, changing only Rust
visibility and the hash-map implementation. Original and transformed assembly
were assembled/linked with Debian GCC 16.2; each was exercised over 64 pairs of
signed boundary inputs (the comparisons in these fixtures are unsigned).
All five drivers returned Memcheck exit 0 with no diagnostics.

| Observable state the rewrite loses | Wrong pairs / 64 |
|---|---:|
| RAX Boolean result | 64 |
| TEST flags used after the conditional branch | 56 |
| A stack slot read after the branch | 64 |
| The fact SETcc writes CL, not AL | 28 |
| A non-stack destination store | 64 |

Artifacts: `engineering/evidence/redteam85-fusion/`. The replay script uses
frozen pre-fix sources, not whatever source happens to be in the working tree.
These are native tests of the isolated transform, **not** full-frontend C
reproductions. Rust regressions exercise the replacement directly.

## Adjudication and repair

- **Agree** with keeping the adjacent GP-forwarding opcode/width/pin safeguards
  and preserving boolean materialization bundles for their owning pass.
- **Disagree** with the old "corpus-validated" exception for deleting a live
  AL/RAX boolean. Passing examples cannot justify altering defined semantics.
- **Disagree** with maintaining a second, raw-text implementation whose own
  comment promises a stack-use census that its main rewrite does not perform.
- The late pass now reconstructs all five categories of pin metadata and calls
  the shared CFG-aware fusion implementation. No partial raw-text phi shortcut
  remains. Unknown facts refuse the rewrite.
- The shared pass gates **all** shapes on GP deadness, flags, and complete CFG
  facts. A matching nearby reload is not a dead-store proof: deleted stores now
  require CFG-aware frame-slot deadness over the **full store width**, not only
  the number of meaningful boolean bytes.
- Whole-file liveness facts are constructed lazily and reused; after a successful
  rewrite, the edited function's facts are refreshed. This avoids introducing
  a fresh whole-file analysis for each candidate.
- Frame-slot analysis now treats mixed RBP/RSP coordinates as opaque unless a
  future implementation supplies a proven frame delta. Distinct base register
  spellings do not establish non-aliasing. Register analysis remains available.

Two positive spill fixtures now define their successors and kill RAX before
returning; their fusion assertions remain. The old phi fixture and legacy live
RAX test asserted unsafe deletion, so they now require preservation instead.
This is an intentional rejection of an unsound optimization contract, not a
relaxation of semantics to make the forwarding change pass.

## Validation limits

The safety repair's full library suite is currently running. S84's green suite
is **not** validation of this repair. No fresh FAST, Clippy, 45-case native,
Callgrind, or zstd performance result has yet been obtained on repaired source.
No performance gain, universal approval, or target-hardware result is claimed.

### First repair run

Full units: **4,795 passed / 8 failed / 8 ignored**, exit 101. The new five-case
safety regression, volatile/inline/unknown-CFG regression, and mixed-frame alias
regression passed. Six failures were flag/condition-code positives that returned
the supposedly dead RAX boolean; one used RBP as an unestablished frame pointer.
Their fixtures now explicitly kill RAX with flag-neutral MOV before every return
and establish a real frame where needed. Negative flag fixtures get the same
RAX kills so their refusals cannot pass vacuously on GP liveness instead.

The remaining signed-byte spill positive exposed another pass-order issue:
`fold_load_test_into_cmp` ran before fusion and erased the reload. The shared
fusion now gets first ownership immediately before that weaker fold, preserving
all existing skip controls. The subsequent S85 run passed 4,803 units (eight ignored); S86 passed 4,806.
Completed performance results are in
[the measurement report](MEASUREMENTS-S86-FOLLOWTHROUGH-2026-10-15.md).

The script inventory covered 223 Python/shell entry points; all Python files
parsed successfully. This is a syntax/purpose inventory, not a semantic proof of
all tooling. The measurement map distinguishes runtime, static, simulated, and
external-oracle evidence and records the FAST-only boundary.

## Frozen replay verification (follow-through)

The replay now resolves frozen inputs relative to its own location, discovers
`rustc` through PATH, and requires an explicit output directory. It no longer
writes generated binaries or reports into the committed evidence directory.
Run `python3 engineering/evidence/redteam85-fusion/probe.py --out /tmp/fusion-replay`.
A fresh execution reproduced exactly 64/56/64/28/64 mismatches (276/320), with
all five Memcheck invocations clean. The script asserts this red-witness census;
a successful replay means the historical defect was reproduced, not accepted.
