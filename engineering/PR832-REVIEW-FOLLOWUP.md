# Historical PR832 follow-up, delivered as PR834

This summarizes the completed earlier delivery. It is not the current PR834
review status. The original chronological work log, including intermediate
failures and superseded counts, is preserved byte-for-byte in
[the historical log](history/PR832-REVIEW-LOG.md).
Current work is indexed in [the PR834 follow-up](pr834/README.md).

## Delivered identity and results

- Delivery commit: `c52bcf03815d961585fb7176af12e0307c1bb992`.
- Delivery tree: `2fd6467d25c88064e3cede0083822b9d5ad6d40c`.
- PR834 merge: `f07f4c514460e4116a574773cd84b69d185bf65e`, the same tree.
- Final local FAST: 253 passed, 0 failed, 6 slow gates skipped.
- Library: 4,758 passed, 0 failed, 8 ignored.
- Assertion-enabled IVSR: 43 passed. Strict linker: 303 passed.
- Vector-copy gate: 47 passed; 48-copy budget unchanged; zero GP relay pairs.

These are historical receipts, not validation of subsequent follow-ups.
The initial failed FAST run (252/1/6, 49 copies) remains in the work log;
it is not the final result and was not excused by raising the budget.

## What the measurements establish

The earlier final compiler measurements used source `75b74f1`, not the later
documentation-only delivery commit. Three alternating pairs covered all 45
registered forms: every simulated program-Ir ratio was [1,1], and 44 ELF pairs
were identical, including the supplied worst 15. These are bounded Callgrind
results, not native CPU-cycle or speedup measurements.

The measured-source/delivery-source distinction is intentional. It must not
be erased by relabelling historical runs. Current follow-ups supply their own
measured-tree, tool, argv and raw-data linkage.

## Limits carried forward

- The supplied worst-15 native deficits were not eliminated.
- No rejected fannkuch regression waiver is reinstated.
- Raptor Lake/14700KF native/PMU validation remains separate work.
- Offline policy tests are not live GCC/Clang/ICX oracle coverage.
- The new native wide-call witness found a defect not covered by the earlier
  successful tests; the old green result is not a claim of zero bugs.

See [oracle policy](CI-ORACLE-POLICY.md) for the intentionally retired network
merge gates and the unchanged benchmark gate's actual failure semantics.
