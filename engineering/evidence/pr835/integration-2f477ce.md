# PR836 integration harness repairs (2026-10-15)

Base: `2f477ce01749824ccab73ead7ccac90abd77fa08`. Compiler under test:
`783e6014b7813efb1113e000eb4fb62f594c5b84`, official fastbuild / Rust O1 / jobs 2.
This is targeted evidence, **not** final exact-tree FAST acceptance.

The first combined FAST run finished 262 PASS / 6 FAIL / 6 SKIP. Repairs here:

* Remove duplicate local/hosted gate invocations rather than inflating coverage.
* Test early loop-bound and late affine ownership independently. A late-owner
  disable must not disable the early owner. Preserve the existing `(early
  unsigned)` diagnostics; count only the requested owner's messages.
* Restore PT_NOTE non-note payload rejection using a shared half-open span
  predicate. The registered oracle entry point had a second missing-name error
  (`SHF_ALLOC`); share the constant and execute that entry point in the unit test.
  Empty spans and exact-touch boundaries must not be reported as intersections.

Targeted results after the repairs:

| Executed check | Result |
| --- | --- |
| `check_pr824_boundaries.py --lccc target/fastbuild/lccc --cross` | 324 PASS, zero failures/skips |
| `check_affine_exit_compare.sh` | PASS: runtime GCC parity and independent owner controls |
| `check_ci_gate_parity.py` | PASS: 239 commands, 24 invocation contracts |
| `test_ci_gate_parity.py` | 73 tests PASS (negative mutants print expected diagnostics) |
| `tests/linker/test_note_runs.py` | 7 tests PASS |
| `run_linker_tests.py --lccc target/fastbuild/lccc --strict` | 304 PASS, zero failures/warnings/skips; bfd oracle |
| shell syntax / Python compilation / `git diff --check` | PASS |

The strict linker gate used GAS 2.47, required i386 support and the provisioned
kernel `relocs` helper, matching FAST's strict environment. This was a targeted
FAST constituent, not full/slow local CI. The x86 `double_reduction` instruction
ratchet remains unresolved at this checkpoint (139 versus a budget of 135).
No budget has been relaxed. Full FAST/fmt/Clippy and fresh performance acceptance
remain outstanding.
