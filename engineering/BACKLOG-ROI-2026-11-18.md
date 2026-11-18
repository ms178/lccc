# LCCC / lccc-ld: ten-item performance queue, 2026-11-18

Scope: the performance worklist, not a replacement for the conformance roadmap in
[`backlog.md`](../backlog.md). Checked against `origin/main` at
`4d8fd153fd4d1c7f1ea21182f95a520b6e48b063` (PR #1198). The
[Nov 10 dynamic sweep](BACKLOG-ROI-DYNAMIC-2026-11-10.md) protects existing
contracts; 16 passing ratchets did not close 16 performance projects. This is a
new ranking. No production optimizer change or compiler speedup is claimed.

**Decision.** Start with bounded match extension and the heap comparator:
we have current, output-checked runtime gaps and concrete compiler witnesses for
both. Start MB-20 step 5 first on the **compiler-throughput** track. Review
metric-pair sharing before changing either scoped leg. Screen allocators
without a dependency change. Do not use a faster compiler as compensation for
slower generated code.

**How to reorder.** Start with a named backlog item and a current output-checked
witness. Prefer repeatable generated-code gaps over tool latency, and bounded,
independently testable candidates over broad redesigns. Prioritize credible
opportunities to remove >10% of workload time; a baseline gap is *headroom*,
not a forecast. Among compile-only ideas prefer directly removable work over
inclusive hotspots, then price implementation cost, risk and representative
coverage. A stale datapoint or a missing tool makes a probe, not an implementation
mandate. Re-rank on the same workloads and flags after a rebase. S/M/L below
mean a narrow probe or patch / a cross-pass change / a multi-stage project.

## Evidence that changed the order

[Machine-readable screening receipts](evidence/backlog-roi-2026-11-18/screening.json)
retain raw paired wall samples, checksums, identities, and profile counts. The
frozen compiler was built with Rust 1.99.1, `fastbuild` (`-O1`, no LTO, `-j2`),
Clang driver + cached mold and 4 GiB active swap (used during the build).
Its SHA-256 is `f262b2ecf8de71ad6450a85804b5cdd12e3937e2603ce22c43602cf9e2abfaa8`.
The binary was built at `6439940` (tooling-only delta from the base); the first
run used the Huffman source later committed in `1d68c74`, identified by content
hash. Do not substitute a later binary or call a debug-dump repair an optimization.

### Generated-program wall time

KVM guest, Xeon 2.60 GHz model exposed, two vCPUs, about 3.8 GiB RAM, CPU 0
pinned; Debian 12, glibc 2.36, compiler defaults for linking (LCCC built-in
linker; GCC/Clang external GNU ld 2.47.20260726 available on PATH). GCC 16.2.0
and Clang 23.1.5 (`392dfff97d29b59d77e5cac6d4b6ef2585fe7117`) are the local
references, not a proxy for ICC/ICX or for the owner’s i7-14700KF. Hardware PMU
was **not probed**; no cycles, cache-miss or branch-miss conclusions. Native
process wall time includes startup. `-O2 -march=x86-64-v3`, 12 rounds, two
excluded warmups, shuffled arm order, seed 20261118; no removed outliers.
Ratios are paired medians, LCCC/reference (lower is better), with paired
bootstrap intervals in the JSON. All seven checksums agreed.

| Workload (source-derived extracts) | LCCC median | vs GCC | vs Clang | Reading |
|---|---:|---:|---:|---|
| `zlib_ng_compare256` | 7.02 ms | 0.983 | 1.309 | Too short in this run; amplified in a separate run below |
| `zlib_ng_huffman_heap` | 157.54 ms | 1.167 | 1.207 | 20.7% longer than Clang [1.200, 1.217]; not full deflate |
| `gzip_longest_match` | 105.95 ms | 0.897 | 0.986 | Keep as a regression sentinel; old gap not reproduced here |
| `expat_utf8_content` | 49.81 ms | 1.029 | 1.010 | Narrow, small gap; use malformed-input controls |
| `sqlite_vdbe_serial` | 15.18 ms | 1.088 | 1.078 | Short; not bytecode dispatch or a DB benchmark |
| `linux_xxhash64` | 4.36 ms | 0.988 | 0.991 | Short; no tuning decision |
| `glibc_dl_hash` | 12.22 ms | 0.952 | 0.956 | Short; not dynamic-loader startup |

Geometric/arithmetic means: **1.0114/1.0147** vs GCC (best gzip 0.897, worst
Huffman 1.167) and **1.0737/1.0803** vs the fastest available reference for
each row (best glibc/GCC 0.952, worst compare256/Clang 1.309). Sensitivity: on
only the three rows with both LCCC and GCC medians ≥20 ms, geomean **1.0249**.
This selection is not representative of applications or distributions. The
rechecks below are separate; do not blend them into the seven-row aggregates.

- **Amplified compare256** (`-DPASSES=524288` only; 24 rounds, two warmups):
  245.01 / 264.25 / 165.60 ms, LCCC/GCC **0.9205** [0.912, 0.933],
  LCCC/Clang **1.4790** [1.465, 1.489]. Clang's emitted loop has eight copies
  of the 8-byte compare and retains early exits; LCCC's has one copy and a
  loop-limit branch every eight bytes. This is a candidate explanation, not a
  measured attribution; the whole-program ratio can also price caller/layout.
- **Huffman source-only control** (`-DPASSES=1536`, `-O2`/v3, 24 alternating
  pairs, equal stdout/status on every run): adding `always_inline` only to
  `smaller` removed five static call sites (three in `huffman_merge`, two in
  `pqdownheap`), grew the `size` text total from 4,894 to 5,062 B and changed
  the median 304.29 → 280.40 ms (**0.9215 forced/original**, minimum ratio
  0.8910, paired sign-test p=1.19e−7). This isolates a useful direction on one
  slice, not a shippable change. The existing inline trace shows three
  `pqdownheap` and three `smaller` inlines into `huffman_merge`, then
  `Cleaned capped caller 'huffman_merge': 495 -> 313 instructions`; optimized
  IR and assembly still contain `smaller` calls. Investigate the growth cap and
  the live-range effects separately; do not force all leaf inlines.
- **`nbody` recheck**, canonical input, 24 rounds, two warmups: 198.75 / 214.46
  / 195.80 ms, LCCC/GCC **0.9274** [0.924, 0.932], LCCC/Clang **1.0227**
  [1.013, 1.026]. The October MB-44 1.44×/1.49× screen is not a current
  priority under these settings. No causal fix or all-target parity is implied.
- **Static comparison only:** `godbolt.py compare` ran locally on seven source
  rows with GCC and Clang. `longest_match` is inlined by all three, so the
  emitted `main` is the comparison unit; the Huffman counts (244/264/204 for
  LCCC/GCC/Clang) omit callees and cannot rank the work. Compare256's
  15/16/57 static counts actually invert the runtime ranking against Clang.

### Compiler work, a different metric

Valgrind **3.19.0** Callgrind, one CPU, `LCCC_PEEPHOLE_PAIR_PARALLEL=0`,
compiler `-O1 -S` (no assembling/linking). Mixed-40 is the repository generator
with seed 20261024; the other TU is the Huffman extract at default defines.
A/A on compare256 and gzip (two rounds per TU, both orders) held exact assembly
and stdout, with Ir ratios in [0.99999733, 1.00000234]. This sizes the simulated
work noise only; it says nothing about native compile latency.

| TU | Total simulated Ir | Peephole inclusive | Metric pair inclusive | `hash_function` self | `LabelRefs::build` inclusive |
|---|---:|---:|---:|---:|---:|
| mixed-40 | 6,764,409,752 | 65.36% | 42.10% | 2.63% | 2.38% |
| Huffman | 1,274,924,249 | 52.56% | 34.05% | 1.62% | 2.27% |

Inclusive costs overlap. None are available savings or wall-time predictions.
The 32.5% historical `LabelRefs` problem is not the present marginal benefit.
The exact lazy index, allocation-before-filter work and #1198 decoding changes
are already in the base. MB-20 is still open: its **<2% label build and ≤35%
peephole** closure targets are unmet on these inputs. GDB reached `hash_function`
via `analyse_function` → `FileLiveness::new` → `LazyDeadness::dead_after` →
`fold_lea_into_memory_op` from a metric leg. This verifies the call path, not
that every solve can be reused or that a host bottleneck is proven.

The x86-64 raw-dump race was real: with distinct metric candidates the literal
`CCC_DUMP_ASM` path could not preserve either tagged fragment. The new positive
test reproduced both missing files before `568abc7`; after it, four backends
passed the 50-pair scheduling/identity contract, and the 49-input x86 benchmark
and kernel `-O2 -march=x86-64-v4` assembly comparison was 49 identical, zero
failures. With the repaired diagnostic, the **last** mixed-40 pair shares only
7/170–178 raw lines; the **last** Huffman pair shares 75/402. These are two
samples of one pair each, not population coverage. They argue for a census and a
cheap decode-only proposal, not copying mutable prefix state.

## Ranked work

| # | Backlog anchor and next test | Expected coverage / stop rule | Effort and acceptance |
|---:|---|---|---|
| 1 | **MB-44 / §8 PF-LZ4-1 / RH-10: bounded match extension.** In compare256 trace C `memcpy` load → IR loads/first mismatch → x86 compare/branch and bit scan → disassembly. Price same-source candidate with a bounded unroll or chunk compare plus first-difference refinement. | 1.479× Clang on amplified input means ~32% headroom to parity on that input, not gzip/deflate savings. Compare early/late/all-equal and realistic compression mix; retain LZ4/gzip as independent shapes. | M. Check every offset and alignment, guard pages/tails, overlapping read-only ranges, exact first differing byte and zero-bit-scan legality. No load past the proved readable bound or hoisting volatile/conditionally valid loads. Demand a real paired win, generated code evidence and no regression on gzip/golden controls. Reject unrolling everything or simply cloning Clang. |
| 2 | **MB-44 / §8 PERF-5 and RA pressure: hot inner comparator/caller.** Use inline trace and RA explanation on the remaining `smaller` calls; test bounded loop-weighted benefit vs call/pressure/growth. Price a post-inline cleanup separately (nonnegative IVs, avoid gratuitous extends). | 1.207× Clang at ordinary size; source-only inlining shows ~7.8% lower median on amplified input, with code growth. Could generalize to small static predicates in codecs and scanners; no projection to whole gzip. | M. Use structural/cost/tie oracles, held-out loop/non-loop, aliased, cold, interposable and recursive cases, debug inline ranges, and actual zlib-ng compress/decompress corpus. Run inflated and ordinary sizes; reject a global growth-cap bump or name-based rule. |
| 3 | **MB-20 step 5a: incremental per-function liveness invalidation.** Add a mutation/fresh-solve oracle before moving cache ownership; see design below. | Direct hash self cost 1.62–2.63% on these TUs before edit bookkeeping, with potential additional wins only if measured. Broad compiler coverage, **no codegen improvement expected**. | M/high correctness risk. `-O0/-O1/-O2/-Os`, debug/CFI/asm/switch and neutralization negative tests; exact assembly and layout winner; two-input Ir/3% ratchet plus replicated native paired compile wall. Fallback on untracked dependencies. No general dirty-pass skipping in this change. |
| 4 | **MB-20 remaining pair work: exact common-prefix decode feasibility.** Count all distinct pairs and classify shareable prepass time; prototype read-only local classification only after review. | Inclusive pair 34–42% is not removable. Existing full-string exact dedup already handles identical inputs. Shared work bounded by actual decode cost of a safely identical prefix, and often short. | S census / M implementation. Require both optimized legs and chosen winner byte-identical, unchanged diagnostic artifacts and schedule/panic semantics; use four-backend pair tests, single/multi-CPU wall and serial Ir. Stop if copy/clone/worker overhead eats the small eligible gain. |
| 5 | **MB-20/22: system vs mimalloc vs jemalloc compiler allocator screen.** Use the same frozen executable (preloaded DSOs first), resolve `malloc`/`free`/alignment bindings for every arm once off-timer; no target-program preload. | No measured allocator headroom here. Small experiment prices broad compiler workloads and pair contention; allocation counts are not latency and neither arm is pre-selected. | S screen / M if adopted. Same loader/env, separate RSS/page-fault and runtime measurements, pin DSO versions/digests; see protocol. Require byte identity and material paired wall gain that survives an integrated `fastbuild` A/B; separate dependency commit only after evidence. |
| 6 | **MB-44 / §8 WR-COND-RETEST: scalar parser/dispatch diagnostics.** Amplify SQLite serial decode without changing data distribution, then compare IR bounds, width/sign extensions, branch fanout and layout with Expat UTF-8 as an independent control. | SQLite 1.088× GCC at 15 ms; Expat 1.029× at 50 ms. Confirm one profitable generic shape; a good case can reach parsers beyond SQLite/Expat, but no VDBE claim. | S probe / M candidate. Oracle every serial type, sign boundary, legal and malformed UTF-8/token boundary; preserve short reads and alias rules. Do not equate a lookup of constant values with dispatch executing arbitrary case bodies; no broad jump-threading rewrite on static differences. |
| 7 | **§15 RH-10 → RH-04/12 or RH-11: add a missing hot shape, then rank homing.** First source-derived zlib-ng CRC with an existing gzip CRC control; then a real SQLite dispatch slice, not a rename of the serial decoder. Inventory existing Adler/match/token fixtures first. | Benchmark coverage now, not instant throughput. Choose one register-home or missing-intrinsic target only when the real profile reaches it. | S per fixture, M/L for ABI. Pin recipe and upstream commit, hash and license; include independent scalar oracle, feature/ABI/escape and mixed GCC↔LCCC TU tests, instruction and paired wall evidence. Reject new SIMD ISA or a vector class rewrite just for synthetic instruction counts. |
| 8 | **MB-13/14/33: same-object linker comparison.** Use [`run_linker_tests.py`](../tests/linker/run_linker_tests.py) for feature fixtures and [`real_workloads.py`](../tests/linker/real_workloads.py) to replay the exact object/CRT link invocation. Add representative package links and test startup/text locality separately from link duration. | Cache-mode bug fixed, ELF32/ELF64 smoke passed with cached mold. No complete local matrix or native link-speed gap yet; prioritize artifact equivalence over timing. | S/M once provisioned. Require pinned bfd, lld and native mold; check object order, scripts, TLS/weak/COMDAT/IFUNC/RELR/PIE and rejection cases. Compare relocations and program outputs, not whole-file bytes across distinct linkers. The package runner's grouped medians are a screen; interleave timed links before claiming a gain. |
| 9 | **MB-44 / SV-06: strict FP loop follow-up, parked pending a fresh larger gap.** Recheck nbody/matmul/spectral_norm on the physical host, trace reduction order, spills and address recurrence. Price affine IV or load folding before a vectorizer rewrite. | nbody no longer shows the historical 44–49% deficit in this VM screen; keep it as a control. Priority rises only for witnessed strict-FP or address-motion gaps. | M/L. Bit-exact/correctly specified FP oracles (NaNs, signed zero, denormals, rounding/exception environment as applicable), separately labelled contracted and relaxed flags. No reassociation or implicit fast-math. |
| 10 | **MB-30: physical-host cost model.** Use measured basic-block evidence to price one scheduling/RA choice at a time, P/E-specific, before publishing a generated table. | Cross-corpus potential; blocked on calibrated 14700KF partitions/counters. Static models narrow hypotheses, not host/throughput proof. | L. Record CPUID/features (no assumed AVX-512), P/E/SMT affinity, clocks and perf-event availability. Require separate held-out codegen workloads, no >1% unexplained regression, and reproducible CPU-specific gain; do not fit a policy on the evaluation set. |

### Review gate for #3: ownership before invalidation

1. Keep existing fresh solver as the reference: force a **no memo lookup** full
   solve and compare all observable answers (`known`, live-in/out, use/def and
   upper-32-zero), including the masked hypothetical analysis. A comparison
   between two lookups in the old memo is not an independent oracle. Use the
   existing `CCC_VERIFY_LIVENESS_SPAN` checker for `refresh_span` as a starting
   point, not as proof of all cache keys. Count hits, invalidations and full
   scans for test witnesses; require a hit in an unchanged sibling as well as a
   miss after an in-function metadata-only change.
2. Enumerate **all** writes to `LineStore` and `LineInfo`: replacements, insertions
   and reindexing, `mark_nop`, debug hiding/reinsertion, pinned/volatile bits,
   metadata overlays, call/return marker scans, labels and jump-table/section
   rows, and neutralizing `(range, mask)` queries. In the current code the
   span hash folds the cached raw text digest **and** `LineInfo`; text-only edit
   counters miss metadata changes. A CFI span is not automatically independent:
   audit lookahead across `.cfi_endproc`, jump-table ownership, and effects of
   sibling layout or structure. Test insertions before/inside a function and
   ambiguous or missing markers (conservative fallback).
3. Prefer a per-peephole-invocation owner with span identity, structural epoch,
   and exact semantic edit generations; mark changes at an audited mutation API.
   Initially invalidate all affected state on structural/unknown/global edits;
   an opaque `&mut [LineInfo]` borrow not observed by the owner must not reuse a
   cached result. Ordinary, neutralized and upper-zero queries need distinct
   audited keys. Keep independent ownership per candidate/thread. Compare exact
   old-hash/fresh-solve answers at review time; test no cache leakage across TUs,
   reruns or switches. Retain uncapped terminating worklist behavior.
4. Reject pointer-identity keys, a global mutable cache, text-only generations,
   dirty bits set only by `replace_line`, probabilistic equality as a safety
   proof and skip-on-opcode heuristics in this patch. A narrow experiment may
   lose cross-invocation memo hits: count that too. Stop if tracking, cold miss
   cost or `-g` makes the change negative. Incremental invalidation is not
   permission to change which transformations see a fixpoint.

### Review gate for #4: what is actually shareable

Instrument exact pair sizes, prefix lines/bytes, identical-pair rate and the
independent prepass/classification cost on more than the last two fragments.
In [`metric_pair.rs`](../src/backend/metric_pair.rs) exact equal `String`s are
already deduped. Otherwise auto scheduling selects scoped parallel work at
combined length ≥1024 bytes, subject to available threads. Each closure captures
`RaConfig` and size policy; tags are TLS and counters are process-global. Audit
input/environment ordering before the proposed cut, including pushf/extension
prepasses, line limits, inline-asm/volatile pins, marker/label maps, TLS caches,
and `CCC_DUMP_ASM`, trace and timing sinks. Analyze both raw and identical
post-prepass prefixes. Sharing only immutable, position-independent decode
records might be legal; move unique mutable `LineStore`/`LineInfo`, CFG, liveness,
pass state, offsets and diagnostic tag ownership into each leg. Keep the
existing fallback on suffix-dependent classification or debug modes until
proven. Never splice one leg’s optimized prefix into the other, move a mutable
cache across scoped workers or reduce dual exploration to win a timing test.
Require equality of raw dumps, both post-optimization strings *before* scoring,
final output and tie outcomes. Check actual path counters for each arm and retain
parallel on/off/auto, one-thread fallback, multi-thread panic propagation and
repeat determinism. Test both tiny and profitable-to-split pairs. Before/after
compile latency must include preparation and worker cost; serial Ir alone cannot
justify losing parallel wall time.

### Allocator screen and common acceptance

- Predeclare arms, source hashes, minimum duration and sample counts. For #5
  include small header-heavy TUs, mixed-40/100 and codecs; serial on one CPU
  and default pairs on at least two real available CPUs. Begin with A/A; seed
  balanced arm order, warm up, interleave ≥24 pairs, replicate and retain all
  samples. Use at least ~200 ms per arm or controlled batches with identical
  inputs. Keep link/codegen flags, path lengths, libc and linker constant and
  save RSS, minor/major faults, allocator and thread counts off the timed path
  if the probe perturbs it. No `LD_PRELOAD` leakage to child oracles (start
  with `-S`); test complete compiler/driver invocations separately before
  adoption. Strong evidence would be ≥3% native compile geomean gain with the
  paired interval away from 1, unchanged artifacts and no meaningful tail/RSS
  regression. If effects wash out or are limited to a small partial workload,
  keep the system allocator. Allocation-count tooling measures *why* only.
- Every identity-only candidate gets exact `.s` comparison (corpus/kernels,
  asm-diff witnesses; document any separately canonicalized PGO PID), native
  correctness, baseline/candidate Callgrind twice on the pinned TUs, and
  **distinct** paired native compile times. Use
  [`measure_label_ref_cost.py`](../scripts/measure_label_ref_cost.py) for native
  mixed TU A/B and [`measure_peephole_pair.py`](../scripts/measure_peephole_pair.py)
  for schedule telemetry. A 3% Ir ratchet detects regressions; it is not a
  speed threshold. Require audits/census, directive-law, claim-fidelity, fmt,
  zero Clippy warnings, Rust lib tests and a matching fast CI stamp before push.
- For generated code, keep per-workload and best/worst, arithmetic and geometric
  means with defined denominators, paired CIs, hashes, flags, affinity and raw
  samples. Investigate >1% regression, demand strong review >2%, reject >5%
  without explicit scoped justification; prefer broad or >10% replicated
  workload wins. Separate compiler work, linker wall time, and program runtime.
  Record assembler object/relocations and final binary in addition to IR and
  selected asm; static body counts, QEMU and Callgrind do not validate cycles.
- Keep independent validation inputs and a default-off/old-algorithm control
  until refuted alternatives are documented. Strict semantics and ABI always
  win. For research: LLVM's [MemorySSA update contract](https://github.com/llvm/llvm-project/blob/llvmorg-23.1.5/llvm/docs/MemorySSA.md#invalidation-and-updating)
  illustrates edit-cost and dependency bookkeeping (not a solution for text
  liveness); [Alive2](https://github.com/AliveToolkit/alive2) is a useful model
  for bounded refinement of small IR rules, not a verifier for LCCC ASM/ABI.
  [uiCA](https://github.com/andreas-abel/uiCA) can help explain Intel hot-block
  throughput under its cache-hit assumption, not price misses or full apps.
  [mimalloc's design](https://github.com/microsoft/mimalloc) explains locality
  and thread-local free lists; it does not predict this compiler's winner.

## Reproduction and tooling limits

Use a frozen executable and an isolated output directory; substitute a real
eligible CPU for `0` when necessary. In a restored Arena environment:

```bash
source engineering/verification/agent-env.sh
export LCCC_BUILD_JOBS=2 CARGO_PROFILE_FASTBUILD_DEBUG=0
out=$PWD/target/roi-20261118
mkdir -p "$out"
base=$out/lccc-main
if [[ ! -x $base ]]; then
  # Build LCCC alone from the comparison revision; record a new identity.
  bash scripts/build_lccc_fast.sh
  cp target/fastbuild/lccc "$base"
fi
sha256sum "$base"
ids=zlib_ng_compare256,zlib_ng_huffman_heap,gzip_longest_match,expat_utf8_content,sqlite_vdbe_serial,linux_xxhash64,glibc_dl_hash
flags=(--compilers lccc,gcc,clang --lccc "$base" --gcc gcc-16 --clang clang-23
       --opt=-O2 --cflag=-march=x86-64-v3 --warmup 2 --cpu 0 --seed 20261118
       --no-pmu-probe --strict)
python3 tests/benchmark/run_benchmarks.py "${flags[@]}" --only "$ids" --reps 12 \
  --json "$out/runtime.json" --artifact-dir "$out/runtime-artifacts"
python3 tests/benchmark/run_benchmarks.py "${flags[@]}" --only zlib_ng_compare256 \
  --reps 24 --cflag=-DPASSES=524288 --json "$out/compare256-amplified.json"
python3 tests/benchmark/run_benchmarks.py "${flags[@]}" --only nbody \
  --reps 24 --json "$out/nbody-recheck.json"

src=tests/benchmark/programs/zlib_ng_huffman_heap.c
for arm in original forced; do
  cp "$src" "$out/huffman-probe.c"
  if [[ $arm = forced ]]; then
    sed -i 's/^static inline int smaller(/static inline __attribute__((always_inline)) int smaller(/' \
      "$out/huffman-probe.c"
  fi
  "$base" -O2 -march=x86-64-v3 -DPASSES=1536 "$out/huffman-probe.c" \
    -o "$out/huffman-$arm.bin"
done
# Source-only experiment; do not change the registry source.
taskset -c 0 python3 scripts/paired_ab.py --bin-a "$out/huffman-original.bin" \
  --bin-b "$out/huffman-forced.bin" --rounds 24 --json "$out/huffman-forced.json"

PYTHONPATH=scripts python3 -c 'from measure_label_ref_cost import mixed_source; print(mixed_source(40, 20261024), end="")' \
  > "$out/mixed-40.c"
export LCCC_PEEPHOLE_PAIR_PARALLEL=0
taskset -c 0 python3 scripts/callgrind_compiler.py --mine "$base" --ref "$base" \
  --source tests/benchmark/programs/zlib_ng_compare256.c \
  --source tests/benchmark/programs/gzip_longest_match.c --flags=-O1 --rounds 2 \
  --out "$out/compiler-aa"
for tu in "$out/mixed-40.c" "$src"; do
  name=$(basename "$tu" .c)
  taskset -c 0 valgrind --tool=callgrind --callgrind-out-file="$out/$name.cg" \
    "$base" -O1 -S "$tu" -o "$out/$name-profile.s"
  callgrind_annotate --inclusive=no "$out/$name.cg" > "$out/$name-self.txt"
  callgrind_annotate --inclusive=yes "$out/$name.cg" > "$out/$name-inclusive.txt"
done
unset LCCC_PEEPHOLE_PAIR_PARALLEL

python3 scripts/godbolt.py compare "$src" --local "$base" --oracles '' --offline \
  --local-ref gcc="$(command -v gcc-16)" --local-ref clang="$(command -v clang-23)" \
  --flags='-O2 -march=x86-64-v3' --function huffman_merge \
  --artifact-dir "$out/huffman-asm" --json "$out/huffman-asm.json"
CCC_INLINE_DEBUG=1 CCC_INLINE_DEBUG_DETAIL=1 CCC_DUMP_IR=huffman_merge \
  "$base" -O2 -march=x86-64-v3 -S "$src" -o "$out/huffman-trace.s" \
  2> "$out/huffman-trace.log"
```

For fresh full-matrix **static research**, extend the existing
[`benchmark_codegen_survey.py`](../scripts/benchmark_codegen_survey.py) and
[`godbolt.py`](../scripts/godbolt.py) path rather than another parser/cache.
Use known aliases `cg162,cclang2310,cicxlatest,cicc2021100` only after checking
actual banners or an exact cache hit (a Clang 23.1.0 CE row is not a 23.1.5
local row). Preserve failures and absent symbols; hold flags, preprocessing,
work done in callees and codegen features comparable. For execution use local
native versions or the authenticated hosted ICX lane, not CE instruction
counts. See [oracle policy](CI-ORACLE-POLICY.md); live CE is **not** a PR gate.

Provisioning state: pinned `harness-cache-8` supplied GCC 16.2, GAS/bfd
2.47.20260726, Clang 23.1.5, GDB 18.1, QEMU 11.1.3, mold 3.0.0 cache and
Valgrind 3.19.0. Rust/Cargo 1.100.0, rustfmt 1.11.0 and Clippy 0.1.100 were
installed separately from the [Git mirror commit](https://github.com/kelexine/rust-toolchain/tree/1d200adbbc96c6bbd7b5f33cc7d936a137305b6b/toolchains/1.100.0)
with the mirror's whole-archive SHA-256, not a changed CI pin or an independently
verified official signature. An API blob read was truncated; fetching Git
objects by pinned hashes and checking the joined archive succeeded.
Clang 23.1.6 metadata was queried, but no 23.1.6 binary, lld 23.1.x, ICX, or
legacy ICC was run locally. Sid/Intel/CE endpoints were unreachable here;
offline consistency is not live freshness. Local Valgrind is not the 3.28.0
research pin. See [linker version/build policy](../tools/linker/README.md):
for native timing use source-mode mold 3.0.0 (`mold-cli`, Rust features
`x86_64,i386,system-allocator`, not a CMake `MOLD_TARGETS` option). This
session exercised warm-cache switching offline and real release-cache
ELF32/ELF64 linking. The v3.0.0 source at `8de38c35a2df` was retrieved, but
`cargo fetch --offline --locked` could not resolve the locked `mimalloc_rust`
Git dependency; no native mold executable was tested. Record the resolved
GNU ld, lld, mold and lccc-ld identity per comparison; do not silently substitute.

## Corpus and pipeline follow-up

- **Provenance and coverage.** The starting recipes were checked at
  [archpkgbuilds `9ff5a16`](https://github.com/ms178/archpkgbuilds/tree/9ff5a16c095080c591922f2c26849c196a053221/packages):
  gzip 1.14, zlib-ng 2.3.3, Expat 2.9.0, SQLite 3.54, zstd 1.5.7 and a
  pinned glibc snapshot; verify the exact Linux cachymod minor before using
  it as a source pin. Recipes are not source authentication. The new
  [Huffman entry](../tests/benchmark/WORKLOAD_PROVENANCE.md) preserves the
  upstream `trees.c` frequency/depth rules with separate optimal-cost,
  structural and tie oracles. It does not emit a code or test bit-length
  limiting. Keep existing gzip chain match, zlib-ng Adler/compare/hash slide,
  Expat UTF-8, SQLite serial, Linux xxHash and glibc loader hash as different
  shapes. Add CRC, real bytecode dispatch, then Linux bit scan/tree and glibc
  string/loader witnesses only where they add coverage; bind full gzip,
  zlib-ng and Expat package validation to successful extract experiments.
  Expand the golden set by hot shape (entropy/match: zstd/xz; crypto: OpenSSL;
  SIMD media/filtering: FFmpeg/libjpeg-turbo/libpng; mixed parser/dispatch and
  multi-TU: curl/Git/nginx/Redis/PostgreSQL/binutils/BusyBox). Keep source
  archives and large outputs in a cache, not in Git.
- **FE/preprocessor/driver.** Check preprocessing output, macro/include and
  option-effect ledgers before crediting a runtime difference; compare original
  source and saved preprocessed TU, not cross-driver header differences. Audit
  compiler latency separately on header-heavy and unity TUs; no current
  evidence for a parser rewrite or broad `-O0` pass removal (MB-21 needs a
  fresh `-O0/-O1`, `-g` recheck after MB-20).
- **IR and analyses.** For #1/#2/#6 trace lowering, memory legality, range/IV
  facts and pass order via existing IR dumps and differential tests; capture
  first missed transform. Audit CFG/alias analysis invalidation before motion;
  whole-program/LTO (MB-15), blanket rotation and new alias precision are
  deferred without a same-input causal witness. Keep the rejected designs in
  [DECISIONS.md](DECISIONS.md).
- **Target lowering, RA, scheduling, assembler.** Use RA spill explanations,
  hot-block measurements, feature-negative controls and pinned GAS diffs
  (MB-43). Confirm actual machine bytes/relocations; instructions absent due to
  inlining or changed ABI are not wins. RH-04/12 are the same vector-parameter
  boundary, not independent additive savings.
- **Linker/runtime and builds/tests.** Measure same-object linking and native
  startup independently; compare runtime with identical libc/flags and real
  outputs. Never claim a default PIE, LTO or security feature closure via a
  benchmark. Enable swap and use the most-free drive for reproducible
  fastbuilds, save every validated patch immediately, replay the base-to-head
  delta, run fast CI then hosted PR gates; report incomplete and skipped
  checks rather than borrowing the previous PR's green status.

## Delivery audit

- **Compiler-work reports.** Independent controls falsified both cross-arm
  and cross-round output equality. A failed source/tool/geometry preflight
  also used to leave a previous successful `summary.json` visible. Publish
  the new incomplete receipt under the existing lock before preflight; keep
  the byte comparisons and record stdout hashes rather than substituting hash
  equality for byte equality. The offline controls and a real two-order
  Valgrind A/A passed. No compiler hot path changes here.
- **Huffman correctness.** On x86-64 and i686, LCCC, GCC 16.2 and Clang
  23.1.5 each compile and run at `-O0/-O1/-O2/-O3/-Os` with warnings as
  errors. Test every active count from 2 to 286 at total `UINT16_MAX`, with
  an independent closed-form weighted cost. Reject impossible roots, too few
  leaves, wrong weights and broken parents. `PASSES` must fit a positive 32-bit
  integer; invalid build defines are compile-fail controls. GDB's layout view
  confirms a four-byte node and the first merge's parent/root values. LCCC
  output ran under Memcheck with zero errors. The authenticated ICX 2026.2
  lane runs the same contract on x86-64, not a substituted local compiler.
- **Keep input validation out of the timing shape.** The benchmark has no
  input histogram API; its generator guarantees two positive weights and
  never approaches the 16-bit total limit. Test this separately before calling
  the heap. Keep the malformed-root guard in the cold tree verifier. A fused
  startup histogram check took 1.0299× Clang elapsed time on an isolated
  32-pair repeat (sign p=0.0021), despite unchanged optimized hot-function
  bytes. An outlined check measured 1.0187×, versus 0.9749× for the root-only
  change on 24 pairs each. The selected source rebuilt to the same bytes as
  the measured root-only arm for LCCC, GCC and Clang. Its LCCC and GCC median
  ratios were 0.9981× and 0.9995× (p=0.84 and p=0.54). Linked code addresses
  moved; these are VM workload-shape checks, not causal cycle estimates or an
  LCCC speedup. Raw pairs are in the ignored `artifacts/snapshots/roi-20261118-huffman-audit-*.json`
  receipts.
- **Diagnostic and linker evidence.** The four-backend scheduling contract
  ran with one and two available CPUs; the three x86-64 dump files are now
  also compared byte-for-byte between serial and forced scheduling. Both mold
  setup paths revalidate the installed release executable against the digest
  extracted from the pinned archive, before running its version command. A
  same-banner replacement must not run. LLD's pin is matched to its actual
  version field, not a version mentioned later in a banner. This audit does
  not certify a native mold build or alter the known i686 symbol policy.
