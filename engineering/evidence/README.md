# Evidence

Frozen, data-bearing artifacts. **Retention rule:** everything here is
either (a) an oracle baseline (the `godbolt/corpus/` assembly dumps the
scoreboard compares against), (b) a pinned measurement record (workload
kernels, A/B numbers, session-numbered screening dirs), or (c) a
methodology document. Evidence is point-in-time by definition — treat
numbers as screening records, not current claims; the live state is
[`../STATE.md`](../STATE.md) and the root `README.md` performance table.

| Path | What |
|------|------|
| `godbolt/` | Compiler Explorer oracle corpus (gcc16.2 / clang 22.1 / icx), scoreboard, methodology |
| `workloads/` | gzip / zlib-ng / Expat LCCC vs GCC measurements |
| `benchmarks/2026-08-28-1b3994e7/` | canonical 33-kernel screening run (raw JSON + verified merge) behind the root README table |
| `aarch64/session59..61/` | AArch64 torture + VM screening records (frozen) |
| `simd-fp-oracle.md` | SIMD/FP oracle audit methodology + 2026-08-18 record |
| `phi-acyclic-copy-order-2026-09-11/` | paired A/B behind landing `CCC_PHI_ACYCLIC_ORDER` opt-in (sha256 5.08% slower, p=0.0003) **and** the noise-floor record showing byte-identical arms can reach p=0.0164. Superseded on disposition by the row below; kept as the frozen record of the pre-rebase base `4f527199` |
| `ra-web-inloop-use-2026-09-11/` | amplified paired factorial behind landing the web-wide in-loop-use supply (`CCC_NO_WEB_INLOOP_USE`): sha256 **+3.63% / +4.33%**, p=0.0000, median and min agreeing in both replicates — **and** withdrawing the phi-resolver default, which on the rebased base `25ed36de` costs −1.99% / −0.71% on top of it. Carries the `rot()` isolation matrix for upstream's `evict_short_k` escape, the corrected gcc oracle counts (142 insns / 8 stkref, not the bogus 240/31 a silent whole-file fallback produced), and the withdrawal of an un-amplified +8.21% median. **Session 17 addendum:** the same directory now carries the audit of that fix — an 807-TU byte differential (`corpus-differential.txt`, via the new `scripts/differential_corpus.sh`) that found a **40.53% `lz4_compress` regression** at identical instruction count, the trace-level mechanism (admission-cap veto on v212 remcost 100 handing the pressure to a cost-blind valve that spilled v133 remcost 1110), and the decoupled design that ships instead: 13 → 10 TUs touched, `lz4_compress` byte-identical to base, `sha256_transform` and `linux_rbtree` byte-identical to the coupled build so every number above still holds |
| `ms09-peephole-utf8-audit-2026-09-06.md` | bounded historic/publication audit, live UTF-8 repair, and validation record |
| `godbolt/ms09-inline-asm-utf8.c` | noinline inline-asm companion used for the successful four-oracle MS-09 screen |
| `pmu/` | (future) hardware PMU snapshots from the 14700KF metal runner (MS-14) |


---

## Worst-10 before the 2026-09 triage cycle

<!-- folded from `session/worst10-before.md` (consolidation 2026-09-16; JSONs stay in `session/`) -->

| # | Benchmark | LCCC (ms) | GCC (ms) | LCCC/GCC | Status |
|--:|---|--:|--:|--:|---|
| 1 | `tls_seg_access` | 24.2 | 10.9 | 2.209× | ok |
| 2 | `spectral_norm` | 306.0 | 201.5 | 1.515× | ok |
| 3 | `arith_loop` | 139.8 | 99.3 | 1.409× | ok |
| 4 | `glibc_memcmp` | 12.1 | 8.8 | 1.379× | ok |
| 5 | `expat_xml_scan` | 50.9 | 40.1 | 1.269× | ok |
| 6 | `loop_patterns` | 88.5 | 71.5 | 1.237× | ok |
| 7 | `fannkuch` | 3043.7 | 2473.4 | 1.234× | ok |
| 8 | `aarch64_select_patterns` | 166.1 | 128.6 | 1.216× | ok |
| 9 | `sqlite_varint` | 32.4 | 26.2 | 1.204× | ok |
| 10 | `sieve` | 62.1 | 55.2 | 1.124× | ok |

Geomean LCCC/GCC (all 33 kernels): before 0.692

## PR832 retained artifacts: integrity is not re-execution

Run **`python3 scripts/verify_evidence_hashes.py`** (all three preserved inventories). The read-only, exclusive
FAST gate checks all 69 retained `engineering/` files changed between 7f9eb225
and merged main 63e783b, including compressed streams and the original decoded
hashes in `pr828/RAW-LOGS.json`. Its inventory is
`pr832-integrity.json`; every file is individually named, sized and hashed.
Corruption, missing files, duplicate entries, escaping paths and symlinks fail.
CI does not regenerate or refresh the inventory.

These are historical records, not current performance claims. Hash agreement
proves byte consistency, NOT that an experiment was performed or that its
conclusion is correct. Some old dumps lack complete original command/environment
records; those are **not independently regenerable experiments**. We will not
invent missing commands, tool identities, dates, or human review provenance.

Exact retrieval of each historical dump is possible from its pinned Git object:
`git show 63e783b15586eca86f306bd75a52b82c57304bbb:PATH > recovered-file`.
Use each `files[].path` in `pr832-integrity.json`; this retrieves frozen bytes,
it does not rerun the experiment. `.gz` files are ordinary gzip streams;
`gzip -dc FILE` reconstructs the original log, including failed tests.

Re-execution recipes (produce NEW evidence; timestamps/hardware may differ):

* Library: `RUSTFLAGS="$(cat target/lccc-rustflags)" cargo test --profile fastbuild --locked -j 2 --lib`.
* Local acceptance: `bash scripts/ci_local.sh --fast`; never relabel a new run
  as an old `*-red.log` or `*-green.log`.
* Complete bounded Ir screen: `python3 scripts/callgrind_corpus.py --mine target/fastbuild/lccc --ref gcc-16 --gcc-include-command gcc-16 --out results/new-corpus`.
  All 45 registered forms and checked-in sizes are required without `--only`.
* Optional external compilers: `python3 scripts/codegen_oracle.py SOURCE --no-local --oracles gcc16.2,clang23.1,icx --flags '-O2 -march=x86-64-v3 -fwrapv' --artifact-dir results/new-oracle --json results/new-oracle/manifest.json`.
  Record the actual source, macro sizes and accepted flags; this template does
  not recreate historical parameters omitted from a record.

Forward policy: new experiments keep the exact script/argv, source/tree and tool
identities, raw outputs/counters, failure records and all input-size overrides.
Large new raw sweeps live in delivery/CI artifacts; do not add another bulk
binary/JSON dump to the compiler patch just to advertise a favorable result.

# Evidence integrity and reproducibility

## PR832 inventory (merged commit 31e5acb8)

Run from the repository root:

```sh
python3 scripts/verify_evidence_hashes.py
python3 -m unittest discover -s tests/corpus -p test_pr832_audit_contracts.py -v
```

`pr832/ARTIFACTS.json` enumerates all 61 added/changed evidence files in PR832
(2,730,890 bytes), with their upstream byte lengths and SHA256 hashes. The
inventory was generated by reading both each current file and `git show
31e5acb8:<path>` and requiring byte equality. The verifier also recomputes the
uncompressed hashes recorded for the ten logs in `pr828/RAW-LOGS.json`.

This gate is an **integrity check**, not a claim that historical measurements
were reproduced. The other historical JSON can reference absolute compiler,
profile or executable paths that were not committed. Hashing that JSON does
not verify the missing external objects. Exact producer command lines were
not uniformly recorded for every original dump; the inventory says so for
each artifact. Inventing those commands would make provenance worse. Existing
historical reports explain experiments, but do not make every dump exactly
regenerable. File identity also does not prove that a measurement's conclusion
is sound. New PR832-audit measurements retain actual commands, source/compiler
hashes, options, outputs and completeness receipts outside the source tree.

## Re-run the relevant test and measurement families

Use fastbuild/O1 and exactly two Cargo jobs. Set up swap before building.
These commands regenerate **new evidence**, not byte-identical old timestamps,
PIDs, compiler versions or profiles:

```sh
CARGO_PROFILE_FASTBUILD_DEBUG=0 CARGO_INCREMENTAL=0 scripts/build_lccc_fast.sh
python3 scripts/check_licm_ordering.py --gcc gcc-16 --report results/ordering.json
python3 scripts/check_strict_packed_recip.py --gcc gcc-16 --clang clang-23 --memcheck --json results/packed.json
python3 scripts/compare_optimizer_witnesses.py --compiler candidate=target/fastbuild/lccc --out results/native
python3 scripts/callgrind_corpus.py --mine target/fastbuild/lccc --ref /absolute/path/to/frozen-baseline --gcc-include-command gcc-16 --out results/callgrind
python3 scripts/benchmark_codegen.py --local target/fastbuild/lccc --gcc gcc-16 --oracles gcc16.2,clang,icx --flags='-O2 -march=x86-64-v3 -fwrapv -fno-fast-math -ffp-contract=off -frounding-math' --out results/codegen
python3 scripts/oracle_delta_gate.py --strict --report results/oracle-delta.json
scripts/ci_local.sh --fast
```

The registry-wide commands intentionally omit `--only`. The strict FP-policy
survey is distinct from default-option surveys. Vendor instruction counts are
not runtime results or semantic proofs; Callgrind Ir is not hardware cycles.
Use `callgrind_compiler.py` for compiler-work measurements, not program runtime.
`tight_loop_oracle.py` surveys alignment directives; `loop_density_oracle.py`
estimates static loop density without proving matched-loop work;
`tune_oracle.py` compares tuning profiles; `oracle_asm.py` exposes source
assembly; `oracle_delta_gate.py` checks a pinned, small static-delta corpus.
None substitutes for all-corpus output tests or native FENV/trap witnesses.

The live CE workflow is scheduled/manual and uses strict failure semantics.
PR CI retains hermetic oracle self-tests and native GCC differential/output
checks. Network availability is not silently counted as successful PR-time
oracle coverage. Local full/slow CI is not part of this research protocol.


## PR835 import boundary and deduplication

`pr835-import-integrity.json` inventories the additional imported historical
bytes from PR835 head `4216f897460be1eef08cc0f24e8b6016cd738283`.
It is checked by the same FAST/hosted integrity gate. Its seven `deduplicated`
records map former names to byte-identical retained files, with byte lengths
and SHA256 values verified by the gate. No historical payload was rewritten.
The pinned original commit and offline recovery bundle preserve every original
path; copying the listed retained file reconstructs any removed duplicate.

These are **historical evidence**, not runs of the newly combined compiler.
Importing or hashing them does not close historical “in progress”/“pending”
findings. Current synthesis validation and performance must identify its own
source tree, compiler ELF, commands, outputs and measurement limitations.
