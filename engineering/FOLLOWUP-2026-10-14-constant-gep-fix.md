# Follow-up: constant-GEP correctness repair and Expat evidence repair

## Delivered scope and identity

Upstream base: `e047aa1eb2ae59b3943a64ab7ec5e5dddb89a3ff` (PR #820).
The earlier audit was already merged; only its unmerged validation tail was
rebased. S12 saved in-flight work before that rebase; S13 saved the rebased tail.
Implementation checkpoint S14 is `bc626f2c86af4b9cf4295a342777f9d4e847343d`.
Later checkpoints preserve these fixes plus the measurements below. The final
head, cumulative patch hash and **unchanged-tree fast-CI certification** belong
to the delivered `LCCC-DELIVERY.json` and snapshot ledger, not this document's
self-referential tree hash. No full/slow local CI was run.

**A14-29 and A14-30 are repaired.** With A14-05 closed upstream, the A14 queue
contains 34 entries, **31 open**. This is not a claim to have implemented all
remaining architectural tasks or to have achieved compiler-wide superiority.

## A14-30: cause, surgical repair, and rejected alternatives

The final IR was correct. A constant GEP for `&bl_count[8]` dominated its
consumer in the CFG but followed that consumer in the physical block vector.
`gep_fold_map` already described the address function-wide. However, MachInst
admission consulted only `indexed_gep_map`; its emitter relied on a
`folded_gep_values` marker installed only when the producer was visited.
Consequently the earlier-emitted consumer used a never-written pointer home;
the later producer was then omitted because its address was foldable.

`folded_gep_memory_access` now checks **both** maps, for **Load/Store only**.
Map-managed accesses use the existing map-aware `generate_load`/`generate_store`
path independently of producer emission order. GEP folding, IV widening, LICM,
GVN, register allocation, and the normal MachInst path remain enabled.
A constant-offset unit fixture exercises the same entry → late producer → early
consumer layout as the existing indexed regression, including load/store and
negative controls. The new default `late-constant-gep` fast gate tests an
16-line program with a separate GCC observer that checks every array element,
and the complete licensed two-TU zlib-derived table fixture.

Why not other fixes?

- Disabling IV widening/LICM/GVN changes the triggering IR shape, not the faulty
  backend contract. Those controls were diagnostic, not proof of an optimizer bug.
- All eight peephole kill controls still failed before repair, including complete
  peephole disablement. The native assembler/linker was excluded with GAS linking.
- Disabling MachInst or GEP folding entirely restored output, but would abandon
  unrelated code-quality work. Extending the existing indexed-address admission
  rule is narrower than a new global state mechanism or broad fallback.
- Preseeding mutable folded-value state would affect other emitter consumers.
  The existing immutable function-wide maps already provide the required fact.

### Correctness evidence

Evidence root: `engineering/evidence/2026-10-14-gep-fix/`.

| Check | Result |
|---|---|
| New gate with saved pre-fix compiler | Fails at O2; observer prints `0 0 1`, not `144 0 1` |
| Small + zlib-derived two-TU fixtures | O0/O1/O2/O3/Os pass; independent GAS control passes |
| Full pinned zlib-ng 2.3.3 + recipe patch | **69/69 CTests**, all four round trips pass; runner exit 0 |
| Original full table | SHA-256 `022736ff3362ef3591d1f8fc3b7fede5927c0d25ab0813d7ef8cd95efd6ea242` |
| Post-fix diagnostic matrix | 14/14 default/backend/pass-control arms match the full table |
| Small fixture target controls | i386, baseline x86-64, x86-64-v3/Raptor Lake pass |
| GCC ASan + UBSan, small fixture | Exit 0, no diagnostics |
| Valgrind Memcheck, small and full fixtures | Both exit 0; **0 errors**; full table compared byte-for-byte |
| Rust all-target test phase | 4,565 library tests + 1 integration test pass; 7 ignored |
| Bounded kernel output matrix | 28/28: seven kernels including Adler combine at O0/O1/O2/O3, matched GCC levels; no skips |
| Complete gzip 1.14 rebuild | Both compilers 30/30 upstream tests; six byte-identical streams |

The first fast-CI invocation was deliberately stopped **after** its completed
Rust-test gate to freeze reports/evidence before the final unchanged-tree run.
Its partial log is not a green CI certificate. Final fast CI includes rustfmt,
Clippy, IV-widen, SQLite-derived, indexed-fold, decimal/segment and cross-backend
regressions, plus the new default gate; consult the final receipt for status.

## A14-29: CTest execution metadata is not an outcome

`status="run"` is treated as execution metadata, not an unconditional pass.
The validator reconciles all recognized child and attribute outcomes, rejects
unknown or contradictory outcomes, and checks declared suite counts against
case outcomes. Nonzero suite failures cannot disappear behind passing rows.
Disabled/runtime-skipped counts are checked as subsets of skipped outcomes;
they are not conflated into two independent exact-count assertions.

The self-test now passes **42 checks**, including run+failure/error/skipped,
contradictory status/result, unknown values even alongside failure children,
malformed/contradictory suite counts, and the existing identity/configuration
contracts. Replaying both archived real XML/configuration pairs succeeds.
A **fresh complete Expat 2.8.5 runner exits 0**: both arms pass 4,932 internal
checks, all eight XML workload cases agree, and the summary is complete/pass.
This closes an evidence-parser failure, not an Expat compiler miscompile.

## Callgrind: paired regression check, not a speedup claim

Read and used `scripts/callgrind_corpus.py` and `scripts/callgrind_ab.py`.
Valgrind 3.24.0; fixed, bounded checked-in sizes; equal-length A/B paths;
identical flags, native output checks and instrumented output checks. Manifests
retain source/compiler/ELF hashes, argv, environment, cache geometry and all
counters. All **60 raw profiles** are retained in `callgrind/raw-profiles.tar.xz`.

Three paired rounds compare the repaired compiler directly with the saved
pre-fix compiler. Every generated ELF is **byte-identical** across that pair.

| Kernel | After/before simulated Ir (each of 3 rounds) | After/local GCC 14.2 Ir (context only) |
|---|---:|---:|
| gzip_crc32 | 1.000000 | 0.991134 |
| zlib_ng_adler32 | 1.000000 | 0.907877 |
| expat_siphash24 | 1.000000 | 0.990982 |
| sqlite_varint | 1.000000 | 1.067882 |
| linux_find_bit | 1.000000 | 1.100500 |
| glibc_memcmp | 1.000000 | 1.044363 |

Patch comparison: **arithmetic mean 1.000000; geometric mean 1.000000;
best/worst 1.000000; 0 wins, 0 regressions, 6 ties**. There are no measured
patch regressions above 1%, 2% or 5% in this screen. This does not bound every
program or measure native runtime. Never count the formerly incorrect zlib
program as a valid performance baseline.

GCC context arithmetic/geometric means are 1.017123/1.015153. These are
whole-program instruction-count ratios, including startup/libraries, **not
runtime speedup or slowdown**. The existing Linux/SQLite/memcmp gaps remain
research candidates, not new regressions introduced by this fix. The initial
separate before/after directory names had unequal lengths and tiny startup Ir
drift; they are retained but excluded from the paired patch aggregate.

## Active Godbolt comparison and red-team interpretation

Read and used `scripts/codegen_oracle.py`, its shared assembly analysis, and
`scripts/godbolt.py`; retained the new fixture's assembly and diagnostics.
At `-O2 -march=x86-64-v3 -mtune=raptorlake`, static main instruction counts are
LCCC **30**, GCC 16.2 **61**, Clang 23.1 **37**, ICX alias **59**.
**The smaller LCCC listing is not a speed win:** it retains scalar initialization
and 144 counter updates, whereas the reference compilers replace that counter
recurrence with the constant 144 and use vectorized/unrolled initialization.
This is precisely why raw instruction-count rankings must not substitute for
executed-work and runtime measurements.

ICC 2021.10 rejects the requested v3/Raptor-Lake flags. A separately labelled
`-O2 -march=core-avx2` compatibility probe yields 50 instructions; it is not a
matched Raptor-Lake result. A fresh ICX `--version` query again identifies the
CE latest alias as **2024.0.0, build 2024.0.0.20231017**, not proof of the latest
Intel release. Local tools are explicitly older: GCC 14.2, Clang/lld 19.1.7,
GAS/bfd 2.44, mold 2.37.1. No new mold build was performed; the requested
GCC16.2/GAS2.47/Clang-lld23.1/mold2.42.1 local matrix remains unfulfilled rather
than silently relabelled. CE provided the named GCC/Clang compilation probes.

## Critical review, remaining work and limits

- The repair preserves the useful upstream indexed-GEP design but closes its
  constant-map omission. It does not endorse unrelated upstream choices merely
  because fast CI is green; the 31 remaining A14 findings stay open.
- Broader contracts still include lossy ELF symbol names, ISA/relocation bounds,
  PGO lock/flush ordering, audit scanner gaps, snapshot generation consistency
  and CI resource-state leakage. Their historical evidence remains labelled.
- The paired six-kernel screen and three complete projects are meaningful but
  not exhaustive. No fresh complete SQLite, Linux boot or glibc build/test is
  claimed; SQLite/Linux/glibc coverage here is kernel/gate coverage.
- gzip's runner also produced one-round timings while other work was active.
  They are retained as raw output but **rejected for performance inference**.
- No PMU, native Raptor-Lake P-/E-core/SMT/frequency-controlled throughput, or
  blanket advantage over GCC/Clang/ICC/ICX is claimed. Those acceptance goals
  remain outstanding; instruction simulation cannot establish them.
- Builds used fastbuild **Rust -O1, -j2**, DEBUG=0, INCREMENTAL=0, on the largest
  filesystem with **6 GiB active swap**. Only fast local CI is permitted.
- Source, tests, measurement manifests, raw profiles and this follow-up are in
  the cumulative patch. The publisher commits before saving the patch, source
  archive, bundle and sequential ledger; final recovery is independently checked
  offline. Caches and installed toolchains are not recovery deliverables.
