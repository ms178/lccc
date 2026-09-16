# Subsystem handbooks

Per-subsystem contract files: what the code guarantees, which knobs are
is-SBE (compiled, graded), which defect classes are already fixed (do not
re-litigate), and where the invariant-preserving extension points are.
Read the matching file *before* touching the subsystem; update the file
in the same delta as the behaviour change, never with a substitute
per-session document (those live in [`../journal/`](../journal/README.md)).

| File | Scope |
|------|-------|
| [`cpu-model.md`](cpu-model.md) | x86 tuning rows, decision functions and their consumers, verified policy (mispredict penalty, movbe movsb ERMS, movbe, march resolution), stop-conditions |
| [`x86-apx-evex.md`](x86-apx-evex.md) | APX/EVEX encoder contract (`-mapx`/`-mapxf` gate, GAS 2.47 oracle, reloc pairing, reject list) |
| [`i686.md`](i686.md) | 32-bit backend: landed infra (red-teamer, `synth_mul`, div/divrem helpers), fixed defect classes, measured perf-gap ordering (x87↔GP pairs, leaf prologue) |
| [`optimizer.md`](optimizer.md) | Optimizer tier/cost-model surface, vectorizer/vec-interleave discipline, FP-minmax interpreter rule |
| [`codegen-x86.md`](codegen-x86.md) | x86-64 codegen pipeline contract |
| [`frontend-sema.md`](frontend-sema.md) | Frontend/sema pipeline contract |
| [`register-allocation.md`](register-allocation.md) | Thirteen-phase RA pipeline, per-phase kill switches, accepted-hazard catalog |

Cross-references: optics/product gaps live in
[`../../docs/roadmap.md`](../../docs/roadmap.md); measured negatives and
do-not-retry grounds live in [`../DECISIONS.md`](../DECISIONS.md); session
history in [`../journal/`](../journal/README.md).
