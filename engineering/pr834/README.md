# PR834 follow-up review

Merged baseline: `f07f4c514460e4116a574773cd84b69d185bf65e`.
The merged PR is not rewritten or re-split. Follow-ups are concern-specific.

| Concern | Implemented proof / boundary |
|---|---|
| F1 wide call arguments | Typed external 256-bit witness: original `9 2 3 4`, unfixed optimized `9 2 0 0`; fixed real pipeline passes. Calls observe tracked upper aliases. |
| F2 compiler cost | Default-size all45 compiler timings plus separate bounded program Ir; 10k-instruction narrow-copy stress. Read-monotone caching preserves eager output and invalidates added reads. |
| F3 CI policy | Benchmark gate unchanged; retirement policy moved to its own document; script paths checked; public API collector refuses to invent 20 post-merge observations. |
| F4 workflow parity | Private recursive walker, literal boolean/string continue-on-error, transitive needs skips, always override; unsupported matrix/input/secret semantics rejected. |
| F5 integrity/provenance | Bounded streaming, strict schema, independent 69-file historical scope; corrupt gzip, ancestor symlink, traversal and deleted-file-plus-row mutations. Current corpus linkage is separate from historical hashes. |
| F6 process receipts | Compile/native raw streams and hashed sidecars linked from measurement manifests, including error arms; orchestration-level mocked coverage. |

## Evidence interpretation

A RED/GREEN fixture demonstrates its specific failure and repair; it does not
prove absence of all other defects. The narrow-copy optimization retains eager
refresh for other rewrites and compares exact output against an eager reference
on mixed CFGs. Neither the copy budget nor an existing optimization family is
disabled. Synthetic pass-cost improvements are not whole-compiler or native
benchmark speedup claims. Compiler walltime, compiler-parent Ir and program Ir
are distinct metrics.

The 13 successful hosted checks belong to merged PR834, not these follow-ups.
The 20-post-merge-PR operational dataset is incomplete until those observations
exist. Hosted full/slow CI and target-hardware native/PMU results cannot be
substituted by this VM's local checks. Wide-vector return interoperability and
complete frontend wide-vector ABI support are not established by the call test.

See [call boundary](CALL-UPPER-ABI.md), [refresh proof](VECTOR-REFRESH-COST.md),
[integrity scope](EVIDENCE-INTEGRITY.md), and
[CI/oracle policy](../CI-ORACLE-POLICY.md). Measured trees and final acceptance
must be taken from their explicit receipts, not inferred from this summary.

## Recomputable follow-up capture

[Corpus provenance](CORPUS-PROVENANCE.md) links the exact measured source/tools,
45-form flags, in-repository raw program evidence, compiler timing limitations,
and assembly explanation. Run `python3 scripts/verify_corpus_receipts.py` to
recompute the program `[1,1]` range and verify 3,600 process receipt links.

[Complete measurement results and limitations](MEASUREMENT-RESULTS.md) retain
all45 timing attempts, the interleaved diagnostic and completed stress counters.
