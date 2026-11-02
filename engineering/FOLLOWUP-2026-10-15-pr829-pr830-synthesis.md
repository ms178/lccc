# PR829 / PR830: adversarial audit and measured synthesis

## Conclusion and identities

**Keep PR829's correctness repairs, adopt PR830's pinned-SIB fix and useful
vectorizer organization, and repair the additional defects found by this audit.**
Neither original PR is an acceptable substitute for the synthesis. In particular,
passing the original Rust suites did not establish the missing architectural,
wrapping-arithmetic, floating-environment, or dynamic-loader contracts.

- Upstream main: `7f9eb225def23bda9370d3349304b9b785554fea`; fresh fetch on
  2026-10-15 at 10:16 UTC still returned this commit.
- PR829: `7110d5af8fc2eb9cf64d7907af4b8ed0bed24acd`. Its tree exactly matched
  the saved S32 implementation. It is not an independent third implementation.
- PR830: `d508ea1c97df104c6d35d35d63749c1fe9af519b`, merged; its tree matched
  the main checkout used here.
- Final measured compiler source: S38 (`943af435`); its binary SHA256 and every
  measurement's identities are in `pr829-830/summary.json` (archived byte-exact
  out of the tree in S59; sha256 in [ARCHIVED-BULK.md](evidence/ARCHIVED-BULK.md)).
  Subsequent report/harness-only commits do not change compiler production code.
- Canonical cumulative deliverable: `/home/user/ms178-1.patch`, against that main.
  The final receipt, not an earlier checkpoint, is authoritative for the final
  tree, patch hash, apply check, and matching **fast-CI** gate.

The initial local fast-CI attempt was unsuccessful and deliberately stopped.
It is retained as negative evidence, not relabeled green. The
[current integration audit](FOLLOWUP-2026-10-15-pr831-pr832.md) and its matching
snapshot receipt supersede this historical report for final validation.
Full/slow CI was not run locally.

## What was accepted, rejected, and why

| Area | Decision | Evidence and boundary |
|---|---|---|
| Pinned SIB operands | Retain PR830's fix | The corresponding regression failed on PR829. Protect pinned address operands rather than relying on incidental register allocation. |
| Implicit register/memory effects | Retain PR829's MAY/MUST distinctions, implicit outputs, VEX upper-zero regions, symbol-home invalidation and spill-home protections | 17 adversarial tests transplanted onto PR830: 2 passed, 15 failed. Gather masks, partial vector definitions, CLZERO, implicit string outputs, and stale LEA homes require different read/write and kill semantics. These are targeted architectural tests, not proof of every ISA instruction or privileged execution. |
| Vectorizer Copy roots | Keep bounded identity walking, but only admit the new min/max Copy roots in the late stage | An early-stage version regressed `loop_patterns` by 2,054 simulated instructions (+0.2898%) versus PR829. Late-only admission removes that regression. The walk is bounded to eight hops and declines uncertain identities. Preserve PR829's wide-integer range restrictions. |
| Signed wrapping and IV strength reduction | Retain real modular/range proofs, not signed-UB assumptions under `-fwrapv` | Original PR830 crashed on the wrapping-domain witness. Exact GCC/Clang comparisons, two ABIs, and Memcheck validate the repaired domains. Low-optimization tail-recursion correctness uses a bounded stack-safe input; optimized performance inputs are not reduced by that change. |
| Masked signed IV ranges | Add the missing nonnegative-mask range fact | Under `-fwrapv`, `x & c` lies in `[0,c]` for either extension when the normalized mask is nonnegative. The pass had retained this fact only for unsigned members, so `(i & 7) + 1` lost its proof. Keeping the fact enables existing TLS forwarding without relaxing overflow checks. Negative masks, overflowing additions, OR and XOR are negative controls. |
| Strict FP / recurrence | Retain PR829's strict packed-reciprocal and FP-environment protections; additionally fence FP backedge PRE | A rounding-mode witness failed on PR829 and the initial synthesis, but passed on PR830 and Clang. Disabling backedge PRE alone repaired it. Equality of operands across iterations is insufficient when rounding mode or exception timing changes. The new fence is FP-only; integer PRE remains enabled, including in mixed loops. |
| ELF32 dynamic tags | Emit `DT_RELENT` when either ordinary REL relocations or PLT REL relocations exist | The no-ordinary-REL / nonempty-PLT case was missing the entry size. The repair has a 512-combination Rust test and native i386/x86-64 NOTYPE regressions. |
| RV64 shared relocation/binding | Retain PR829's symbol-identity, visibility/preemption and signed-addend repairs | PR830 passed only 10 of its 240 binding cases; the LLD control passed all 240. Synthesis and LLD passed 480/480 combined checks. Loader tests improved from PR830's 3/13 to synthesis's 13/13 under QEMU. |
| PT_NOTE layout | **Reject and revert this audit's proposed production rewrite** | The oracle incorrectly treated a short gap as padding even when it contained allocated `.rodata`. A real GCC16 ELF had seven bytes of ordinary data separating note runs. Two PT_NOTE segments were correct. Fix the oracle, require exactly one covering segment per note, and test padding versus data gaps of 1/4/7/8 bytes. |
| Measurement machinery | Fail closed on output, process and artifact errors | Preserve stdout bytes via SHA256; nonzero exits, signals, timeout/overflow metadata, failed/changed warmups and partial runs cannot become successful comparisons. Remove stale executables before compilation and require a newly produced executable. Preserve exact sign-test probabilities instead of a rounded normal approximation. |

The prior PR824 architectural work is retained, including its negative evidence
and rejected performance experiments. For example, a four-way readonly reduction
reduced simulated Ir but regressed native median time by 6.84% in its earlier
15-pair experiment; it was not accepted. A two-way alternative was inconclusive.
No instruction-count reduction is automatically treated as a native speedup.

### Two important self-audit corrections

1. The PT_NOTE proposal was a production change motivated by a faulty oracle.
   The final production note emitter is restored to the previously validated
   implementation; only the oracle and its tests needed changing.
2. The FP contraction test mixed incompatible contexts. The fixture that observes
   FENV must forbid contraction even with `-ffp-contract=fast`; a separate fixture
   explicitly opting out of observable FENV must actually fuse. Both are now
   asserted. The bitwise numeric and exception checks were retained, not weakened.
   GCC16 `-O2` without `-frounding-math` was not treated as a strict FENV oracle.
   The invalid early pseudo-pass bisection is retained but is not causal evidence.

## Final Callgrind corpus: all 45 registered shapes

All **180 candidate/reference comparisons passed**, with no failed or omitted
workload. Each arm uses the same checked-in bounded sizing, semantic workload
flags, `-O2 -march=x86-64-v3 -fwrapv`, and appropriate trailing libraries.
The corpus preserves distinct scaled `linux_find_bit` and repeat-rich
`lz4_match_extend` identities rather than collapsing them into their base source.

| Reference | Candidate/reference geometric mean simulated Ir |
|---|---:|
| PR829 | 0.995247 |
| PR830 | 0.987462 |
| Debian GCC16.2 | 0.985283 |
| Clang23.1.3 | 1.020511 |

Lower means fewer **whole-program simulated instructions**, not lower wall time.
The final compiler was used for a fresh complete four-reference matrix after the
signed-mask repair. Against PR829, **43 workloads have identical Ir**. The only
changes are `struct_copy` (0.903349, about 9.67% fewer Ir) and `tls_seg_access`
(0.893364, about 10.66% fewer Ir).

Configured and verified profile geometry: 32-KiB, 8-way, 64-byte-line I1 and D1;
32-MiB, 16-way, 64-byte-line last-level cache. Valgrind's host-cache discovery
warning does not override the geometry recorded in the actual profiles.
Cache misses and branch counters are retained, but are not PMU measurements.

Earlier v1/v2 matrices are historical experiments, not the final matrix. Before
the later TLS repair, a separate replay established 180/180 byte-identical
executables across the production note-emitter reversion. That proof is limited
to those exact inputs and flags; it is not a general linker-equivalence theorem.

## Native measurements: improvements and remaining gaps

Host: an **Intel Xeon 2.60-GHz VM**, not the requested physical 14700KF or EPYC7763.
No usable PMU counters. Runs were pinned to CPU0, randomized/interleaved by round,
with seven timed rounds and one warmup per compiler/workload. No samples or MAD
outliers were deleted. There was no concurrent compiler build or instrumentation
during native timing; light editing/contract checks occurred during the long
screen, while the focused confirmations ran without concurrent maintenance.

Native flags were common `-O2 -march=x86-64-v3 -fwrapv -ffp-contract=off` at normal
registry sizes. Thus these are **not the same input sizes or contraction setting
as the bounded Callgrind screen**. They must not be presented as one causal model.
All 45 shapes passed exact-output and successful-exit checks across all five local
compiler arms. Warmups and all timed observations were independently revalidated
with the strengthened gate.

The TLS repair came after the first native matrix. Exact command replay proved
that the other **44 complete executables were byte-identical**. Only TLS was
remeasured, across all five arms with the same flags and seven-round policy.
The summary explicitly records each row's measurement epoch and binary hashes.
It does not silently reuse the invalidated TLS measurements.

| Native subset | n | vs PR829 | vs PR830 | vs GCC16.2 | vs Clang23.1.3 |
|---|---:|---:|---:|---:|---:|
| All registered shapes | 45 | 0.9687 | 0.9857 | 0.8166 | 0.7556 |
| Workload-derived tags | 18 | 0.9459 | 0.9991 | 1.0313 | 1.0077 |
| Synthetic tags | 27 | 0.9843 | 0.9769 | 0.6990 | 0.6236 |
| Original worst-15 screening selection | 15 | 1.0109* | 0.9877 | 1.1606 | 1.0951 |

These are descriptive geometric means of paired median elapsed-time ratios;
less than one favors the synthesis. **The all-45 result is strongly affected by
synthetic recurrence specialization**, including the very short optimized `fib`
program. It does not establish general superiority over GCC or Clang.

\* All 15 executables are byte-identical to PR829: their observed timing variation
is noise/environment, not a compiler regression. More generally, the final
native corpus contains 43 byte-identical PR829 pairs and 26 byte-identical PR830
pairs. Their measured ratios must not be used to claim compiler improvements.
The CSV and JSON retain both the observations and the identity flags.

The 18 workload-derived kernels still average **3.13% more elapsed time than GCC**
and **0.77% more than Clang** in this VM screen. Their per-kernel confidence
intervals are exploratory and not multiplicity-corrected. The original 15-case
selection still trails GCC by about 16.1%; this is not an EPYC measurement.

### Focused independent confirmations

| Change | 25-pair median before → after | Elapsed-time reduction | Exact two-sided sign-test p |
|---|---:|---:|---:|
| `struct_copy`, N=16,000,000, PR829 → synthesis | 194.180 → 158.846 ms | 18.20% | 0.000156522 |
| TLS, PASSES=4,000,000, pre-mask-proof synthesis → final | 451.882 → 135.678 ms | 69.97% | 5.96046e-8 |

Three warmups; alternating A/B order; successful exits and exact stdout checked
on every run. Minimum and median agree on direction. Both exact tests would also
pass a two-comparison Bonferroni threshold; they are focused confirmations after
exploratory screening, not evidence that every kernel improved.

The calibrated and ordinary-size final TLS binaries are **byte-identical to
PR830's binaries**. The gain restores PR830's useful code generation while
retaining the wrapping-domain checks that PR830 lacked. The assembly removes
three repeated sign extensions and permits the existing loop-carried load
forwarding, eliminating its store/load dependency chain. No workload-name special
case or relaxed overflow semantics was introduced.

Historical paired JSON files retain the earlier tool's rounded normal-approximation
p field. `summary.json` recomputes the exact binomial probability from the retained
pair signs; the updated tool stores full precision and preserves raw sample
precision. A stored old `0.0` is not claimed to be an actual zero probability.

### Original worst-15: current VM ratios against external compilers

| Workload | vs GCC16.2 | vs Clang23.1.3 |
|---|---:|---:|
| byte_load_reassembly | 1.800 | 1.994 |
| spectral_norm | 1.414 | 0.741 |
| expat_siphash24 | 0.916 | 0.983 |
| sha256_transform | 1.029 | 0.883 |
| expat_xml_scan | 1.261 | 1.313 |
| linux_find_bit_scaled | 1.196 | 1.258 |
| linux_find_bit | 1.174 | 1.149 |
| sieve | 1.048 | 1.136 |
| binary_trees | 1.167 | 1.072 |
| lz4_match_extend | 0.989 | 1.033 |
| mandelbrot | 1.100 | 1.026 |
| sqlite_varint | 1.291 | 1.037 |
| zlib_ng_adler32_combine | 1.014 | 1.089 |
| strlen_bench | 1.078 | 1.088 |
| switch_dispatch | 1.171 | 1.013 |

These losses remain visible rather than being hidden by the synthetic aggregate.
All 45 rows, identity flags and per-comparison intervals are available in
[native-all45.csv](evidence/pr829-830/native-all45.csv) and the summary JSON.

## Compiler-work tradeoffs

Callgrind profiled compilation itself, then GNU as independently assembled the
output. These are simulated compiler instructions and text bytes, not compile
wall time or release/LTO throughput. Both LCCC arms were fastbuild/O1 builds.

| Input | Compiler Ir ratio vs PR830 | Text bytes PR830 → synthesis |
|---|---:|---:|
| byte_load_reassembly | 0.8777 | 784 → 741 |
| expat_xml_scan | 1.0222 | 895 → 882 |
| sha256_transform | 1.2413 | 1427 → 1427 |
| spectral_norm | 0.8615 | 1009 → 1009 |

The SHA compilation-work regression is real in this experiment and is not hidden
by the correctness gains or runtime aggregate. It does not justify removing
architectural-effect or arithmetic-soundness checks. A typed shared effect model
remains architectural work; this patch does not claim to complete that migration,
eliminate every optimizer cost, or exhaustively audit every function.

## Tools, Intel comparison, and validation scope

Installed/used: Debian GCC/G++16.2.0-3 with multilib; Clang/LLD23.1.3;
GAS/bfd2.47; Valgrind3.27.1; Rust/cargo1.99.0, rustfmt1.10.0, clippy0.1.99;
4 GiB activated swap (actual usage recorded). LCCC and lccc-ld were built using
**fastbuild, O1, two jobs, no LTO**, with debug info/incremental disabled for the
constrained VM. Mold2.42.1 was built for exactly `MOLD_TARGETS='X86_64;I386'`, O1,
two jobs; it was a correctness oracle, not a linker-speed reference. RV64 loader
checks used QEMU10.0.13 and the matching sysroot; kernel relocs was pinned to v6.12.

The existing Compiler Explorer/oracle scripts were used, not replaced by guessed
compiler flags. Latest available CE channel identities were executed and retained:
ICX reported 2024.0.0.20231017; classic ICC channel metadata was 2021.10.0, with its
reported backend identity retained. **These are latest available CE channels,
not a claim to be Intel's latest commercial release.**

All 45 unchanged source shapes were submitted to both Intel channels: **ICX
accepted 45/45; ICC accepted 44/45 and failed `percpu_ptr_walk` on GNU `__seg_gs`.**
That failure remains a feature gap, not a hidden skip or successful comparison.
The Intel survey is static code-generation/feature evidence, not native Intel
throughput. For classic ICC, `-fp-model precise` still allowed FMA; the adapter
adds `-no-fma` for contraction-off. A Haswell feature/contraction canary and six
adapter contracts validated the actual dialect rather than merely accepting exit 0.

Executed validation includes:

- Final compiler build and **4,735 Rust tests passed**, zero failed, eight ignored.
- **303 modern linker cases passed**, zero failures/warnings/skips, with bfd/lld/mold.
- FP recurrence exact bits/exceptions at O0–O3 on x86-64 and x86-64-v3; strict
  no-contraction and separate permitted-contraction controls; x87/jump-thread checks.
- Wrapping domains over optimization levels and two ABIs; 24 exact-oracle/ABI/domain
  Memcheck cases; new masked arithmetic controls; TLS dimensions 2/8/64/257 with
  default and no-IV-widen execution plus Memcheck; GCC16/Clang23 independent checks.
- Strict packed reciprocal: 1,260 bitwise cases in each tested configuration,
  plus native Memcheck; retained readonly-domain and trapping-environment checks.
- RV64 shared-binding matrix and real loader tests described above; final Rust
  unit coverage also exercises the symbol/relocation implementation.
- 20 benchmark fail-closed/statistical contracts; five binary-attribution contracts;
  dedicated note-run oracle tests; workflow/local parity and script-import checks.
- Final unchanged-tree **`ci_local.sh --fast`**, rustfmt and strict Clippy are the
  publication gate recorded by the final receipt. No earlier pass stamp is reused.

Ignored Rust tests and slow gates are not reported as executed. This is not a
full fuzz campaign, a proof that all compiler bugs are gone, an exhaustive glibc
or kernel build, physical 14700KF/EPYC validation, or universal compiler superiority.

## Reproduction and durable evidence

- The all-45 CSV and exact native replay are in the patch. The machine-readable
  `pr829-830/summary.json` and the compressed raw native/correctness JSON
  (`*.json.gz`) were archived byte-exact in S59; paths and sha256 are in
  [ARCHIVED-BULK.md](evidence/ARCHIVED-BULK.md).
- `artifacts/pr829-830-raw.tar.xz` contains the full experiment records: raw
  Callgrind profiles, benchmark binaries/disassembly, original and rejected
  experiments, identities, commands and diagnostics. Every included member is
  SHA256-verified before atomic publication; the adjacent receipt gives the
  archive hash. Frozen compiler executables are omitted with their hashes retained;
  source/history and build recipes are delivered instead.
- `scripts/summarize_pr829_830.py` regenerates the summary and checks completeness,
  successful outputs, final compiler identity and the native replay before combining
  measurement epochs. `scripts/verify_corpus_binaries.py` and
  `scripts/replay_native_corpus.py` reject stale-source attribution.
- `scripts/archive_pr829_830_evidence.py` reproduces the raw archive;
  `scripts/merge_checkpoint_archives.py` preserves old checkpoint TAR streams
  byte-for-byte with verification before removal. The combined-through-S37 archive
  has its own manifest and receipt. No named patch history was discarded.
- `LCCC-RECOVER.py`, the source archive, upstream-base-dependent session bundle,
  canonical patch and snapshot ledger remain the recovery entry points. Do not
  rely on `.cache` surviving a harness wipe.

Example measurement commands (run native timing without concurrent builds):

```sh
export PATH=/home/user/.cache/oracle-bin:$PATH
CARGO_PROFILE_FASTBUILD_DEBUG=0 CARGO_INCREMENTAL=0 \
CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER=gcc bash scripts/build_lccc_fast.sh
python3 scripts/callgrind_corpus.py --mine target/fastbuild/lccc \
  --ref gcc-16 --gcc-include-command gcc-16 --out /path/to/fresh-cg
python3 tests/benchmark/run_benchmarks.py --compilers lccc,gcc,clang \
  --gcc gcc-16 --clang clang-23 --opt=-O2 --cflag=-march=x86-64-v3 \
  --cflag=-fwrapv --cflag=-ffp-contract=off --reps 7 --warmup 1 --cpu auto \
  --strict --json /path/to/native.json --artifact-dir /path/to/native-artifacts
# Local CI policy: fast only; hosted CI owns full/slow execution.
bash scripts/ci_local.sh --fast
```
