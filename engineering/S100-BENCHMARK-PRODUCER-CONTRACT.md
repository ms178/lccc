# S100: real benchmark producer completes the S96 consumer contract

Inspection before the fresh45 workload run found that the canonical CLI never
emitted `complete`, whereas the hardened S96 report consumer requires it.
Synthetic report fixtures satisfied the new contract, but the real producer
would be rejected even after a successful complete run. Existing output paths
also retained a previous successful report if a later invocation was interrupted.

Alternatives: do not loosen the consumer to infer completion from row counts,
patch measured JSON by hand, or silently accept stale reports. Add an explicit
producer lifecycle and reuse `tools.corpus.publication.atomic_bytes` rather
than creating another unsafe in-place writer. Publish incomplete state before
execution, checkpoint completed cases, and mark completion only after the
selected-case loop and aggregate/report generation finish. Completion means
attempted coverage, NOT correctness; failed rows and strict exit1 remain.

RED: two success/failure producer subcases lacked the marker; expanded tests
also demonstrate stale-success retention and no incomplete recovery receipt.
GREEN: four new CLI lifecycle tests plus38 prior report/provenance/safety tests
pass (42 total). Includes successful and failed completed runs, fresh and
stale-path interruption, and partial-case preservation. No timing or compiler
speedup claim follows from this reporting repair.

First FAST diagnostic result:257pass/3fail/6skip. Two failures are the S99
inventory mismatch (already repaired); the third required GCC16 C++ multilib
headers/libraries. Installed matching16.2.0-3 packages; focused i386 DSO gate
now1pass/0fail/0warn/0skip across bfd/lld/mold. Rustfmt, MSRV1.97 and strict
Clippy1.99.1 were green in that diagnostic run, but it is NOT a final-tree gate.

Fresh latest-main84 baseline compiler built fastbuild/-O1/-j2 in3m10s. The
initial benchmark invocation was intentionally interrupted before publishing
this validated repair, to keep checkpoint compression from contaminating
measurement. Its incomplete receipt is not final performance evidence.
