# Large-snapshot barrier scan: bounded audit and measurements

## Decision and invariant

`forward_large_memcpy_loads` accepts only loads in the copy's block, strictly
following the copy, and collects them by increasing instruction index. The
half-open intervals from the copy's successor to each load are nested; their
union ends just before the last load. One scan of this union has exactly the
same barrier decision as scanning every interval. The barrier predicate is
unchanged: writes, calls and opaque writes (`may_write_memory`), fences,
atomic loads and volatile loads block the rewrite. In particular, a barrier
between two loads must still block it, whereas one after the last load need
not. Loads are nonempty before `last().unwrap()`; the result is never used
when collection rejected another block or an earlier load.

This removes repeated scans, not all quadratic work in the pass: derived
pointer discovery and instruction insertion have separate costs. It does not
weaken alias, volatile, memory-order or overlap guards, recover the known AVX
round-trip regression, or claim program-runtime improvements.

## Checks

* Fastbuild with warnings denied and active 4 GiB swap passed. Rust unit tests:
  snapshot forwarding 5/5, vector temporary promotion 44/44. The new test
  crosses five barrier kinds (store, call, fence, atomic load, volatile load)
  with four positions (before the copy, before first load, between two loads,
  after last load). A real source mutation replacing `last()` by `first()`
  failed at the between-load store case; restoring the source gave 5/5.
* A separate exhaustive model enumerated 261,625 sequences of 1–11 neutral,
  load, and barrier slots with at least one load: old and new decisions
  agreed; a first-load-only mutant differed on 112,373 sequences. This model
  is not a substitute for the production-code mutation test.
* `check_avx256_fp_contracts.py` passed 72 build/runtime/frame checks with
  LCCC, GCC 16.2 and Clang 23.1.5 on x86-64/i686. The alias fixture passed
  AArch64 and RV64 QEMU at five optimization levels per target. Default-policy
  assembly comparison of the benchmark kernel corpus: 24/24 identical. These
  are bounded functional checks, not hardware timing or concurrency proofs.

## Compiler-cost experiment and important pass-order qualification

Environment: KVM virtualized Intel Xeon 2.60 GHz, two vCPUs, 3.8 GiB RAM,
4 GiB swap; Valgrind/Callgrind 3.19.0, Rust 1.99.1 fastbuild (`-O1`, no LTO,
debug info disabled). GCC and Clang came from the SHA-verified
`harness-cache-8`. A local Intel ICX provisioning attempt failed at TLS because
its Intel host is outside this sandbox's allowed network destinations.

Generate a deterministic source with eight copies of the same function:

```sh
python3 - <<'PY'
from pathlib import Path
s = ['struct S { unsigned a[128]; };', 'extern void fill(struct S *);']
for i in range(8):
    terms = ' + '.join(f'b.a[{j}]' for j in range(128))
    s.append(f'unsigned snapshot_many_{i}(void) {{ struct S a, b; fill(&a); b=a; return {terms}; }}')
Path('/tmp/snapshot-many-8.c').write_text('\n'.join(s) + '\n')
PY
# Build the base and candidate with identical fastbuild flags; keep both
# executables outside target before rebuilding. For each executable:
CCC_NO_AGG_COPY_FWD=1 valgrind --tool=callgrind \
  --callgrind-out-file=/tmp/snapshot.cg COMPILER -O2 -S \
  /tmp/snapshot-many-8.c -o /tmp/snapshot.s
callgrind_annotate --threshold=100 /tmp/snapshot.cg
```

With default policy the earlier `aggregate_copy_forward` eliminates the copy
before the changed pass, so this source does **not** exercise the change.
Three default-policy Callgrind repetitions were approximately 2.04258 billion
instructions before and 2.04330 billion after; there is no evidence of a
default-policy improvement. Initial IR showed 40,387 versus 633 hypothetical
prefix inspections, but it is from an earlier phase and is not an observed
count in this pass. Do not use that figure as a real-pass measurement.

For the diagnostic `CCC_NO_AGG_COPY_FWD=1` configuration, IR dumps show the
copy survives to the post-structural-inline phase, where this pass removes it.
One Callgrind run measured 2,104,069,640 total instructions before and
2,098,528,206 after; the pass's own attributed instructions were 4,595,409 and
668,273 respectively. Output assembly was byte-identical. Twenty randomized
paired samples pinned to one vCPU, after two warmups, had medians 0.154141 s
before and 0.152645 s after; median paired difference was -0.002713 s, with
17/20 pairs favoring the candidate and overlapping ranges. This synthetic
isolated-pass test is not a general speedup claim or a program benchmark.

## Remaining work

Run the full fast gate and hosted CI on the same final tree, check/rebase onto
current main, and retain a gated snapshot. Investigate the default-pipeline
AVX round-trip regression and realistic sources reaching this pass. Complete
native AArch64/RV64 contention and real libatomic interoperability, and
measure GCC/Clang/ICX-generated programs on the requested physical host before
asserting performance or architecture-wide correctness. The broader backlog
and exhaustive line-by-line audit remain open.
