# PR833 red-CI audit — measured repair and latest-main integration

## Result and boundaries

The reported `affine_control_proof` mismatch is reproduced and repaired without
weakening comparison. The missing fixture semantics and stale workflow-policy
tests were real defects in my earlier delivery. A separate pre-existing
non-UTF-8 temporary-path panic is repaired, and a narrowly proved signed-mask
range optimization recovers a measured TLS regression. This is **not** a claim
that all compiler bugs are eliminated or that the compiler universally wins.

Latest-main integration includes PR834, base
`f07f4c514460e4116a574773cd84b69d185bf65e`. The compiler build is commit
`fd086d197c70fc9bbae282a6def6ac957e190c79`, tree
`b89f950b280662c7783890bd22df4af5740be68e`, Rust/source subtree
`a4387baad64c13f7be9ee562728890e4ec73b9fd`. Subsequent research-tool and document
commits do not change that compiler source. The final publication receipt and
FAST stamp identify the complete delivered tree, avoiding a self-referential
source-document hash.

**Rebased compiler acceptance:** `ci_local.sh --fast`: **261 passed, 0 failed,
6 intentionally skipped**. Library tests: **4803 passed, 0 failed, 8 ignored**;
the separate binary test also passes. Rustfmt, strict Clippy (76 lint names,
0 unknown) and the declared Rust1.97 MSRV check pass. The final publication must
additionally carry a matching FAST stamp for its documentation/tooling tree.
No full/slow local CI or unrequested remote push was performed.

Compact machine-readable measurements: [`evidence/pr833/summary.json`](evidence/pr833/summary.json).
Delivery artifacts under `artifacts/pr833/` retain raw profiles, command/output
receipts, compiler identities and negative controls; the final archive manifests
identify their stored bytes. These are not substitutes for the remaining hosted
CI jobs.

## 1. Concrete red-CI diagnosis

PR833 head `446d4e8eb1335475df626af4265d9f42da007fa1` has the exact S39 source
tree `c690f9997bf49f2516b4733ebbf47e01ec247a5e`. Initial main was
`63e783b15586eca86f306bd75a52b82c57304bbb`.

### Missing semantics in a discovered fixture

The general regression runner defaults to `-O2`. The affine fixture deliberately
uses signed wrapping but lacked its `.flags` sidecar. The actual `compile_one`
path reproduced LCCC `affine-control-proof 5 97` versus GCC16.2
`affine-control-proof 2 11`, with GCC returning1. Adding the fixture-specific
`-O2 -fwrapv` gives `5 97`/rc0 in both arms, including SSA validation. Removing
that semantics contract again reproduces the reported failure. The independent
Python signed-32-bit ring model gives five iterations for the optional guard
and nine iterations/seven true body comparisons for the wrapping guard.

This is **not evidence of a GCC bug**. Clang23 at O0 with signed-overflow UBSan
reports `2147483646 + 2` at fixture line10:19. GCC16's no-wrap UBSan builds at
O0 and O2 instead produce `2 11`/rc1 without a diagnostic; absence of a sanitizer
report does not make the source defined. All three live CE execution oracles
agree on the explicit wrapping version.

Two older sidecars, `ivw_wrapv_range.flags` and `wrapv_domains.flags`, contained
only `-fwrapv`. Sidecars replace defaults, so those accidentally selected O0.
They now explicitly include O2. The new signed-mask fixture also has its own
wrapping sidecar. Final filtered runs of all four fixtures pass in normal and
SSA modes: eight actual runner comparisons, not just hand-built executables.

There is no global wrapping override, disabled comparison, suppressed reference
exit status, `continue-on-error`, or optimizer disablement masquerading as a fix.

### Stale research-policy tests

The old Python suite reproduced two failures among551 tests: it categorically
banned the scheduled CE lane and asserted that the preserved Callgrind workflow
did not exist. The intended policy is scheduled/manual live CE, separate from
required PR gates. The repaired YAML contracts mutation-test automatic triggers,
missing strictness, hidden errors and missing artifact uploads. The research
Callgrind lane is manual-only; native differential/output gates remain strict.
The command-parity check passes234 commands and22 registered invocation
contracts. The final live strict delta experiment covers9 cases with no errors
or threshold failures; its static metric is not a runtime claim.

Public annotations identify the SSA mismatch and Python failure. Job metadata
places the assertion-enabled failure in step13, `Regression corpus
(debug-assertions compiler)`, rather than in its earlier build/unit steps.
Downloading full hosted logs returned HTTP403/admin-required; that job's public
annotation is only exit1. The sidecar fix applies to its shared runner, but I do
not invent the missing console log, assert that it had no other failing fixture,
or claim that its complete hosted run was reproduced locally.

## 2. Temporary-file API: genuine panic removed, not rebaselined away

The initial repaired-tree FAST run was **260 passed / 1 failed / 6 skipped**:
`panic_sites=891` exceeded baseline889. Inspection also reproduced a real
pre-existing worker panic with an invalid-byte `TMPDIR`. The CLI returns1, not
101, so testing only an exit-code convention would miss the internal panic.

Changes:

- `src/common/temp_files.rs:265,298`: remove unused `make_temp_path` and the
  infallible `TempFile::new`; retain fallible reservation and make `to_str`
  return `io::Result<&str>` rather than `expect` or lossy conversion.
- `src/driver/pipeline.rs:1328`: collect/propagate conversion failures.
- `src/backend/common.rs:106,115,750`: propagate allocation failure, pass native
  paths to GCC's `Command::arg`, and make the optional-feature discovery test
  fallible rather than adding counted unwraps.
- `tests/regression/check_temp_symlink.sh`: exercise both C and assembly through
  an invalid-byte TMPDIR. Require an ordinary diagnostic, no panic/internal-error
  text, no output executable and no leaked leases. Preserve the read-only-dir
  and symlink/victim controls.

Seven lifecycle units pass. The rebuilt compiler passes the security regression;
the old compiler fails the new negative control for the intended reason.
`cargo check --profile fastbuild --locked -j2 --tests --features gcc_assembler,gcc_m16`
also passes, including test-only optional-feature code. Census is now **888**,
and the baseline is tightened889→888. The scanner and its test-boundary rules
were not weakened or rearranged to hide production sites.

## 3. Signed-mask range proof and causal measurement

`src/passes/iv_widen.rs:1535` previously retained the range of an AND mask only
for unsigned values. After normalization to the operand type, a **nonnegative
constant mask clears the sign bit**, proving `0 <= (x & mask) <= mask` even for
a signed input. This permits the existing downstream addition-overflow proof;
it does not license arbitrary signed wrapping arithmetic to widen.

Raw-IR cases cover masks0/7/INT_MAX−1/INT_MAX/−1/−8/INT_MIN, ordinary bias1 and
INT_MAX bias, including conservative refusals where no overflow proof is
available. Focused IVW units:61 pass. The executable fixture contains eight
kernels checked against unsigned modular references, checksum770224292.

`scripts/check_signed_mask_oracles.py` makes the complete experiment reproducible:
GCC16, Clang23, LCCC enabled and LCCC with `CCC_NO_IVWIDEN=1`, each at m32/m64 and
O1/O2/O3/Os. **All32 configurations pass Memcheck**. Its command/status/checksum
contracts fail closed on compilation errors, nonzero reference exits,
Memcheck errors, timeouts, missing output and incomplete matrices.

The isolated pre-PR834 45-case A/B changes **only TLS**:

- TLS Ir: **237886 → 212486**, ratio **0.893226**, **10.68% fewer** simulated
  instructions; the other44 cases have identical Ir.
- Full-corpus geometric ratio: **0.997493913** (about0.25% fewer Ir, not a large
  corpus-wide speedup).
- An independent focused run saves the same25400 Ir with different startup
  totals,214428/239828. Do not conflate absolute counts from different runs.

This recovers a regression caused by the necessary wrapping-safety restrictions.
The final candidate ties latest main on TLS; it does not improve TLS by10.68%
relative to latest main.

## 4. Final-source correctness and performance

Both candidate and latest-main compilers were independently built with
**fastbuild/Rust O1, exactly two Cargo jobs**, frozen and hashed. Candidate SHA256:
`ade7c12a8796fc10e8f5dfc94d522e8e4676c1700dbd73bb33e3a8568c443c37`.
Latest-main SHA256:
`f27767dfd04ff08995b76e0c008640751d976aff480df03beb1b19efa35da016`.

### Correctness is the hard constraint

The same native witness matrix gives **candidate94/94**, **latest main76/94**.
Under explicit `-fwrapv`, main emits `affine-control-proof 2 95`/rc1, segfaults
on a wrapping-domain witness, and returns43 for the first-trap FP witness at
O2/O3. These are distinct from the undefined-behavior GCC comparison above.
The wrapping, integer-width, RISC-V relocation and FENV repairs retained from
the earlier integration must not be discarded to improve a benchmark rank.
Strict packed reciprocal tests pass **9 configurations ×1260 bitwise cases**,
including Memcheck and pass-disable controls. FP trap-order checks run natively.

### Complete45-case bounded comparisons

All45 output comparisons pass in each sweep. The bounded sizes are committed
in `tests/benchmark/callgrind_sizes.json`. Ratios below1 mean fewer whole-program
Callgrind Ir, including startup/libraries, **not faster hardware execution**.

| Comparison | Geometric candidate/reference Ir |
|---|---:|
| Latest main, common x86-64-v3 | 0.999589556 |
| GCC16.2, common x86-64-v3 | 0.987468718 |
| GCC16.2, each compiler's default target | 0.900233639 |
| Latest main, common x86-64-v3 + znver3 tuning | 0.999593080 |
| GCC16.2, common x86-64-v3 + znver3 tuning | 0.992791851 |

The two additional Zen3-tuned sweeps also cover all45 cases with passing
output comparisons; selecting `-mtune=znver3` does not turn this VM into an
EPYC hardware measurement. The latest-main aggregate is effectively neutral,
with both wins and losses.
For example, byte reassembly is0.8750 and spectral norm0.9122 relative to main,
while sieve is1.1187 and fannkuch1.0541. Byte reassembly remains4.2868 relative
to GCC-v3 in this experiment. These are remaining optimization opportunities,
not grounds for claiming universal superiority. Broader nonlinear/dynamic-step
range proofs for sieve were not guessed into this correctness repair.

### Requested worst15: do not mix metrics

The first column below is the user's historical EPYC9V45 timing screen from
10:18:56Z, revision6b6723d, binary484301fac0ca92de…. Its compiler options and
binary/hardware timings were not independently reproduced here. It is not a
measurement of this final binary, nor proof of performance on an EPYC7763.
The other columns are new bounded simulated instruction counts.

| Case | Supplied EPYC time ratio (historical) | Final/main Ir | Final/GCC16-v3 Ir |
|---|---:|---:|---:|
| `chacha20_block` | 2.0116 | 1.0000 | 0.5099 |
| `byte_load_reassembly` | 1.7987 | 0.8750 | 4.2868 |
| `spectral_norm` | 1.4297 | 0.9122 | 1.8978 |
| `sieve` | 1.2591 | 1.1187 | 1.0540 |
| `sha256_transform` | 1.2344 | 1.0067 | 0.9492 |
| `binary_trees` | 1.1989 | 1.0000 | 1.0791 |
| `expat_xml_scan` | 1.1709 | 0.9900 | 1.1501 |
| `lz4_match_extend` | 1.1369 | 1.0000 | 1.2421 |
| `sqlite_varint` | 1.0792 | 1.0218 | 1.0902 |
| `fannkuch` | 1.0548 | 1.0541 | 1.1804 |
| `hash_table` | 1.0532 | 1.0000 | 1.0464 |
| `tls_seg_access` | 1.0518 | 1.0000 | 0.9721 |
| `mandelbrot` | 1.0427 | 1.0000 | 1.0884 |
| `strlen_bench` | 1.0378 | 1.0093 | 1.1386 |
| `linux_rbtree` | 1.0362 | 1.0000 | 1.2533 |

Chacha's reversed ranking illustrates why fewer Ir cannot be advertised as an
EPYC speedup. No hardware CPU PMU is exposed: `perf_event_open` fails ENOENT;
only software/tracepoint-style event sources are present.

### Other applicable oracle shapes

- `benchmark_codegen.py`: all45 registered cases, including the distinct LZ4
  match-rich macro configuration, against **GCC16.2, Clang23.1 and ICX**.
  The strict static survey explicitly uses `-fwrapv -fno-fast-math
  -ffp-contract=off -frounding-math`; it is distinct from the Callgrind defaults.
- `tune_oracle.py`: common x86-64-v3/`-mtune=znver3` probes for shifts, rotates,
  popcount, and copy thresholds, all three vendors.
- `tight_loop_oracle.py` and `loop_density_oracle.py`: requested14 source files
  plus a separately parameterized LZ4 match-rich case, all three vendors,
  complete/no errors. Density explicitly marks loop/work equivalence false.
  Chacha/SHA can select a short initialization loop rather than the rounds;
  their attractive heuristic ratios are **not accepted performance evidence**.
- `callgrind_compiler.py`: three alternating paired rounds per source, frozen
  equal-length paths, identical native/instrumented assembly required. Median
  compiler-work Ir ratios versus main: TLS0.999870, sieve1.050014,
  byte-reassembly0.878488. These are compiler work, not program runtime or
  compilation wall time.
- Classic ICC2021.10 is additionally checked on TLS with the explicit common
  Haswell request; the oracle records ICC's `-xCORE-AVX2` dialect. A first
  `core-avx2` request was correctly rejected by LCCC and retained as failed
  evidence, not counted as coverage. The Haswell retry includes LCCC and all
  four vendor arms and passes; it is separate from the v3 surveys.
- `ld_feature_census.py`: bfd2.47/lld23.1.3/mold2.42.1/lccc-ld, dynamic and static
  TLS: eight successful link/run comparisons. ELF layout differences are
  recorded; this is not whole-linker qualification or byte-identical ELF.

Local tools: GCC16.2.0-3, Clang/LLD23.1.3, binutils2.47, Valgrind3.27.1,
Rust/Cargo1.99.0, rustfmt1.10.0, clippy0.1.99; 8GiB swap was active. FAST also
provisions its SHA-pinned upstream GAS/objdump2.47 pair. Rust binaries link with
GCC/bfd, not an unreported mold build. Mold is the official2.42.1 prebuilt;
the custom x86/i686-only source option is `-DMOLD_TARGETS='X86_64;I386'`.

CE identities were actually executed: cg162 reports16.2.0; cclang2310 reports
23.1.0; **cicxlatest reports Intel2024.0.0 (20231017), macro20240000**. The fresh
compiler list exposes no newer numbered ICX. This is the latest available CE
alias, not a claim to have used the latest Intel product release. Same-session
remote compilations may be reused through the validated oracle cache.

## 5. Critical architectural review and rebase decisions

| Follow-up | Decision and evidence |
|---|---|
| F1 live CE | Keep scheduled/manual strict delta lane; do not make network availability a successful PR gate. Actual9-case strict run has no errors/failures. |
| F2 LICM ordering | Keep loop-wide atomic/fence barriers. Ordinal-only motion is unsound across backedges; see the defined concurrent counterexample below. |
| F3 evidence | Keep both original immutable inventories (69 and61 files, overlapping), original raw-log hashes, bounded decoding, duplicate-key/path/symlink checks. Never regenerate hashes just to pass. |
| F4 Copy roots | Keep latest main's stronger whole-index duplicate rejection and lazy once-per-analysis map. Preserve Copy-only identity, cycle/depth tests and parameter handling. |
| F5 transactional plans | Keep owned `StrictPackedPlan` (`vectorize.rs:19087`). A failed private builder may consume private IDs; discard its instructions and IDs together, not assert the counter never advanced. |
| F6 architectural pipeline | Preserve both sides' actual peephole-pipeline fixture tests and positive optimizing controls, plus the independently found narrow-GPR DSE repair. Raw assembler round trips alone are insufficient. |
| F7 CFG invariance | Check both structural label/terminator/edge shape and cached-analysis successors (`iv_strength_reduce.rs:281,313`); retain both test modules. Block count alone is insufficient. |
| F8 reviewability | Use the review order below. Most cumulative patch lines are retained historical evidence, not new optimizer code. |
| F9 packed naming | Document one scalar F64 result from four source-order products; the preserved implementation uses exactly128-bit I32x4 denominator values, not an accidental packed-F64 return. |
| F10 provenance | Bind measurements to the actual frozen compiler SHA, build tree, source subtree, flags and tool identities; final receipts identify the delivered/gated tree separately. |

PR834 merged during the audit. I retain its shared upper-state effect model
(`fp_liveness.rs:1496`, `vex_promote.rs`) and scalar-merge/private-temporary
proofs in `vector_copy.rs`; those strengthen my earlier implementation.
Both sets of tests survive. The historical41-commit S45 chain is preserved in
recovery artifacts; the cumulative tree was replayed onto latest main as one
integration commit, rather than repeatedly reapplying overlapping historical
implementations and silently dropping one side.

### F2: measured conservatism, not an unsafe relaxation

`check_licm_ordering.py --callgrind` runs a release/acquire handshake with four
ordinary global values. A load textually before the first acquire is after the
previous iteration's acquire on the next trip. Both GCC and LCCC pass8 rounds.
The deliberately hoisted mutant returns20 instead of60 and fails for that
semantic reason, not a build error. Both compilers hoist all four reads in the
plain guarded-do-while control and retain four inside the acquire loop.

For100000 uncontended shape iterations:

| Compiler | Plain Ir | Acquire Ir |
|---|---:|---:|
| GCC16 | 120835 | 1020835 |
| LCCC | 520829 | 1320824 |

This difference includes the acquire load and other optimizer choices: GCC
collapses the plain recurrence, while LCCC retains its four-instruction loop.
It is **not isolated fence latency**. A more precise memory/control proof could
recover opportunities; a local instruction-order exception cannot.

## 6. Review order and exact reproduction

1. **Immediate CI semantics:** the four wrapping `.flags` files;
   `test_wrapping_regression_contract.py`, `test_benchmark_output_gate.py`,
   workflow-policy tests and `.github/workflows/oracle-delta.yml`.
2. **New range rule:** `iv_widen.rs`, its raw-IR tests,
   `ivw_signed_mask.c`, `check_ivwiden_exposed_latch.sh`,
   `check_signed_mask_oracles.py` and its matrix/verdict tests.
3. **Fallible temp lifecycle:** `temp_files.rs`, driver/backend call sites,
   common README and `check_temp_symlink.sh`.
4. **Preserved compiler fixes:** signed affine/wrapping proofs in `simplify.rs`,
   `loop_bound.rs`, `iv_strength_reduce.rs`, strict FP/packed planning in
   `vectorize.rs` and the x86 emitter, positive architectural effects in
   `fp_liveness.rs`, `vector_copy.rs`, `dead_writes.rs`, `store_forwarding.rs`,
   `helpers.rs` and operand `types.rs`. Review RISC-V linker changes separately
   with their binding/loader/relocation tests.
5. **Publication and evidence:** `compact_session_artifacts.py`, its14 tests,
   `verify_evidence_hashes.py`, both evidence-contract suites and the summary.
   Large older evidence directories remain historical, not newly rerun claims.

Use the frozen candidate/main binaries recorded in the delivery, or rebuild
from the pinned commits with the same tools/flags. Representative commands:

```sh
CARGO_PROFILE_FASTBUILD_DEBUG=0 CARGO_INCREMENTAL=1 scripts/build_lccc_fast.sh
scripts/ci_local.sh --fast
python3 scripts/compare_optimizer_witnesses.py --compiler candidate="$CANDIDATE" --out results/native
python3 scripts/check_signed_mask_oracles.py --lccc "$CANDIDATE" --gcc gcc-16 --clang clang-23 --work-dir results/mask --json results/mask.json
python3 scripts/check_strict_packed_recip.py --lccc "$CANDIDATE" --gcc gcc-16 --clang clang-23 --memcheck --json results/packed.json
python3 scripts/check_licm_ordering.py --lccc "$CANDIDATE" --gcc gcc-16 --callgrind --report results/ordering.json
python3 scripts/callgrind_corpus.py --mine "$CANDIDATE" --ref "$MAIN" --gcc-include-command gcc-16 --out results/main
python3 scripts/callgrind_corpus.py --mine "$CANDIDATE" --ref gcc-16 --gcc-include-command gcc-16 --out results/gcc-v3
python3 scripts/callgrind_corpus.py --mine "$CANDIDATE" --ref gcc-16 --gcc-include-command gcc-16 --target-flags='' --out results/gcc-default
python3 scripts/callgrind_corpus.py --mine "$CANDIDATE" --ref gcc-16 --gcc-include-command gcc-16 --target-flags='-march=x86-64-v3 -mtune=znver3' --out results/znver3-gcc
python3 scripts/callgrind_compiler.py --mine "$CANDIDATE" --ref "$MAIN" --source tests/benchmark/programs/tls_seg_access.c --source tests/benchmark/programs/sieve.c --source tests/benchmark/programs/byte_load_reassembly.c --rounds 3 --out results/compiler-work
python3 scripts/benchmark_codegen.py --local "$CANDIDATE" --gcc gcc-16 --oracles gcc16.2,clang,icx --flags='-O2 -march=x86-64-v3 -fwrapv -fno-fast-math -ffp-contract=off -frounding-math' --out results/vendor
python3 scripts/oracle_delta_gate.py --local "$CANDIDATE" --strict --report results/delta.json
```

The measurement commands intentionally omit `--only`: they cover the complete
registered45-case corpus. Preserve each output directory, not just its aggregate.
Do not run `ci_local.sh --slow` or a full local CI run under this research policy.

## 7. Recovery defect found by actually recovering

The first real offline restore of the compacted snapshot failed: updating
`BUNDLE_ARCHIVE.json` alone did not update the standalone consumer's
`SESSION_BUNDLE.json` locator. `publish_bundle_location` now authenticates the
current bundle member and updates both archive fields; the old archive remains
until both pointers are published. An actual-consumer unit and malformed-session
preservation test join the14-test compactor suite. Staged and installed S42
**offline exact-history** restores then passed.

Checkpoint deltas are authenticated and byte-compared before loose duplicates
are removed. The final delivery must repeat a clean-patch application and the
installed standalone offline restore; its recovery receipt is authoritative.
A hash inventory proves integrity, not execution truth. Historical dumps with
missing original command/environment data remain an explicit reproduction gap.

## Remaining debt

- Hosted full/assertion-enabled CI remains to run after upload; no remote green
  status is claimed from the local FAST result.
- No reproduction of the supplied EPYC timings, no PMU cycles/uops, and no claim
  that every function or every one of the45 cases improved. The overall
  latest-main Ir ratio is effectively neutral despite useful individual wins.
- Sieve's remaining range-proof opportunity, GCC's advantage on byte reassembly
  and other SIMD kernels, and compiler-work regressions need separate proved
  changes and hardware evaluation. Do not undo wrapping/FENV safety to hide them.
- Static function/loop identity, alignment and instruction density are screening
  information, not matched-work performance proofs.
- Previously known ignored tests and existing audit budgets are not evidence of
  a bug-free compiler. The work fixes demonstrated, high-confidence defects;
  it does not certify the entire language/backend/linker surface.
