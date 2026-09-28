# CFG red-team review — 2026-09-28

## Decision and scope

**Do not replace `src/passes/cfg_simplify.rs` wholesale with Agent A's proposal.**
Use the small, regression-tested correction in this patch. This review covers
that supplied proposal and the corresponding current implementation, not a
completed audit of every compiler subsystem. It does not establish performance
supremacy, production readiness, or correctness of all input programs.

Upstream base: `6f8ace9c0bf98bb2c4df671342d89f983e0ab4c1` (`ms178/lccc/main`).
The proposal was pasted text, not an independently versioned attachment. No
claim is made that its entire replacement file was built. Its closure mutability
issue was separately reproduced with rustc; the patch is built against the real
repository APIs and keeps the existing CFG tests and optimizations.

## Landed correctness correction

`IrConst::to_i64()` explicitly truncates `I128`. Three CFG equality paths used
that conversion: local phi constant resolution, global phi constant resolution,
and `consts_equal_for_phi`. Thus `I128(0)` and `I128(1 << 64)` were considered
equal although their truth values differ. This can license a wrong branch or
replace a nontrivial phi. The fix uses `to_i128()` in **all three** paths.

Why not take Agent A's narrower integer-variant-only equality wholesale?

* Full-width numeric comparison retains the existing cross-width integer rule
  (e.g. `I32(0)` versus `I64(0)`) without the lossy conversion.
* Existing exact hash keys are structural representations, not hash digests.
  `ConstHashKey::F64(u64)` distinguishes signed zero and NaN payloads. Keeping
  this equality is safe and avoids dropping valid floating-phi simplifications.
* Floating constants are still not introduced into integer-only phi resolution.
  No reassociation, FP exception, or cast semantics are changed by this patch.
* No allocator, scheduler, vectorizer, or unrelated pass is changed.

The two new negative unit tests fail on the unmodified implementation. The
floating-bit-identity test already passes there, demonstrating why blanket
rejection of all floating constants is unnecessarily pessimistic. With the
fix, all 15 CFG tests pass, including local and global resolution checks.

`tests/regression/cfg_phi_i128_high_bits.c` adds end-to-end path, truth-value,
and high/low-half observations at O0–O3. **It also passes before this fix**:
other pipeline passes can prevent the vulnerable IR shape in this C example.
It is integration coverage, not the red/green proof; the unit tests are that
proof. Do not relabel it as a demonstrated source-level miscompile.

## Agent A: function-by-function findings

| Area | Assessment | Disposition |
|---|---|---|
| Strict integer phi equality | Avoids the demonstrated I128 truncation; unnecessarily loses supported cross-width equality | Apply full-width comparison, not wholesale replacement |
| Floating phi equality | Rejects even identical F32/F64 bit patterns; the warning about numeric conversion is valid, but exact enum hash keys are not collision-prone hashes | Preserve exact existing equality; signed-zero/NaN tests |
| `resolve_value` Copy/Phi/Select | Cycle set and depth bound are conservative; removing Cmp/Cast resolution also removes existing kernel-specific opportunities | Keep current resolver in this patch; type-aware Cmp/Cast audit remains P0 |
| Resolution complexity | Depth 32 bounds stack depth, not total work. A DAG with repeated identical phi operands can repeatedly resolve the same predecessor expression; no memo table | Prototype tri-state memoization with explicit work budget before adopting |
| `clean_removed_edges` | Actual-edge check correctly avoids mistaking “multiple predecessors” for membership; preserves surviving asm-goto edges | Useful design. Source block is still found by linear scan despite existing label-index map; not a linear-time implementation |
| Direct Switch folding | Width-masked comparison is better than untyped i64 comparison | Current O0 integrity path already masks; optimized path still requires a unified typed helper and tests |
| Jump threading | Rejecting final destinations with phis is conservative and avoids many edge-value collisions | A safe fallback, not a demonstrated code-quality win; retains fewer threading opportunities |
| Forwarding chains | Terminates on cycles, but allocates a visited set and walks suffixes for every start | Long chains have quadratic aggregate work; use path compression with explicit cycle states |
| Reachability roots | Entry, static initializer labels, reachable LabelAddr and asm-goto edges are represented | Broadly appropriate. Need structural tests for each root, including unreachable address-producing instructions |
| Trivial phis | Avoids self-copy for a self-referential phi | Useful defensive improvement; current implementation needs this test before changing its handling of degenerate IR |
| Disjoint fusions | Batched disjoint fusions are reasonable and permit one-level label substitution | Current main already batches fusions. Do not claim this proposal invents batching relative to this base |
| Missing fusion phi input | Candidate check avoids inventing an incoming value; `expect` is then justified by no intervening phi mutation | Current fallback to arbitrary input/zero is technical debt; add verifier-backed negative tests before removal |
| Asm-goto fusion | Conservatively blocks both asm-goto-containing predecessors and successors and protects goto targets | Safe bias, possibly misses legal fusions; no demonstrated speedup |
| Absorbed blocks | Removes immediately after remapping | Current pass defers removal to next reachability round; immediate removal is a possible compile-time cleanup, not itself evidence of a runtime bug |
| Source spans | Pads successor spans and previously empty predecessor spans | Does not repair an already partially populated predecessor span vector. Specify the input invariant; test valid empty/full combinations |
| `remap_terminator` closure | `let mut remap` is unnecessary: captured map is only read | `-D warnings` rejects unused mut. Independently reproduced with rustc 1.98.1; entire proposal not compiled |
| Test coverage | Nine hand-built examples, several with undefined inputs or unreachable pairs | Good unit isolation, insufficient for pass-wide semantic assurance; misses root/asm/span cases and adversarial complexity |

### Important inherited hazards, not fixed by rejecting Agent A

1. The current Cmp/Cast/Select resolvers still use i64-based evaluation in other
   contexts. Merely fixing phi equality does **not** make 128-bit comparisons or
   arbitrary casts correct. A typed evaluator must reject unsupported widths
   instead of guessing, with signed/unsigned boundary tests.
2. `IrConst::is_zero` currently uses the approximate f64 field of LongDouble.
   A nonzero binary128 value may underflow to f64 zero. Agent A calls
   `is_nonzero` directly, so its claim of conservative constant evaluation does
   not solve that inherited issue. Test full-precision truthiness, including
   signed zero and subnormals, at the constant representation boundary.
3. Current branch/switch phi cleanup is not consistently conditioned on a
   surviving asm-goto edge. Construct legal IR that contains both edge kinds;
   require the verifier and backend to agree on predecessor semantics.
4. A depth guard alone does not bound total recursive resolution work. Current
   local Select recursion also needs an explicit cycle/budget audit.

These are source-review findings and targeted follow-up requirements, not
claims that every one has a reproduced user-program failure in this session.

## Validation-tool defect found during the audit

`tests/fuzz/phi_cfg_fuzz.py` accepted matching nonzero exit statuses, matching
signals, and even two `TIMEOUT` results with empty output as a passing result,
then removed the source reproducer. Generated `main` is supposed to return
zero. The patch classifies any execution failure as `runtime` and preserves
its source. Matching successful stdout is still required for a pass.

`tests/fuzz/test_phi_cfg_fuzz.py` mocks compiler/run results and tests successful
cleanup, equal failures, one-sided failures, and successful-output mismatch.
The original verdict logic fails seven subcases; the corrected logic passes.
This is a correctness-oracle fix, not a generated-code optimization.

## CI and snapshot hardening discovered while validating

The first fast CI run passed 90 gates; Clippy was killed by the kernel OOM
killer, confirmed in dmesg. Swap was expanded from 2 to 8 GiB without relaxing
`-j2`, fastbuild, or warnings-as-errors. A subsequent run exposed an unrelated
flaky ARX gate: `awk ... | grep -q pshufd` under `pipefail` sometimes reports
failure when grep finds its match but awk receives SIGPIPE. A 20,000-line
matching stream reproduced exit **141** with the old predicate and **0** with
the consuming predicate. The gate now drains the stream and self-tests both
matching and nonmatching large streams. All six ARX configurations pass after
this fix. These are gate fixes, not compiler SIMD changes.

The snapshot archive also included Python `__pycache__` directories generated
by validation. The tar invocation now excludes `__pycache__` and `*.pyc`;
archive contents are checked after snapshot creation. The flat patch never
included these caches. The new Python unit-test file is explicitly unignored
so the repository's broad `test_*` rule cannot silently omit it from patches.

## Measured code-generation evidence

Existing `scripts/godbolt.py audit` resolved the requested pins successfully:
GCC 16.2 (`cg162`), Clang 23.1.0 (`cclang2310`), ICC 2021.10.0
(`cicc2021100`), and ICX moving channel (`cicxlatest`). ICX latest is not an
immutable compiler revision; preserve the returned manifest and assembly.

`codegen_oracle.py` was actually run on the existing workload-derived kernels,
with `-O3 -march=x86-64-v3`, whole translation units. Static instruction counts:

| Kernel | LCCC | GCC 16.2 | Clang 23.1 | ICC | ICX |
|---|---:|---:|---:|---:|---:|
| gzip CRC32 | 73 | 34 | 82 | 60 | 100 |
| zlib-ng Adler32 | 332 | 129 | 148 | 240 | 147 |
| Expat scanner | 222 | 232 | 228 | 322 | 336 |

These are **not runtimes**, retired instructions, or speedup ratios. Counts
include initialization and self-checks; different inlining/unrolling/constant
folding choices change them. The oracle's spill count is a syntactic heuristic,
not a register-allocator event counter. LCCC/GCC Adler32 stack reservations in
the saved assembly are 584/24 bytes; their causes need per-function live-range
and spill analysis, not just instruction-count ranking. Expat's heuristic
spill counts are LCCC 17 versus GCC 1 despite the smaller LCCC instruction count.

For the phi correction, emitted assembly is **byte-identical before/after**
for all three workloads at each of O0/O1/O2/O3 (12 comparisons). This establishes
no generated-code change for those exact inputs/flags. It is stronger than a
noisy same-code wall-clock comparison, but says nothing about compile-time
cost or workloads not checked. No runtime gain is claimed.

The repo already contains these extracts and provenance records, including
SQLite, Linux and glibc; this session reused them rather than inventing duplicate
extracts or claiming to have revalidated their old upstream checksums.

## Environment and limits

* VM: Intel Xeon model 106, 2 exposed logical CPUs, approximately 1.9 GiB RAM;
  **not** the user's i7-14700KF. Kernel `6.1.158+`, microcode reported `0x1`.
* Created and activated `/swapfile.lccc`, 2 GiB, then a second 6 GiB file
  after the first Clippy run exhausted memory (kernel OOM log confirmed).
  `/proc/swaps` showed actual use. Both must be recreated after a VM reset.
* Rust stable resolved to rustc 1.98.1. Every compiler build uses `fastbuild`,
  opt-level 1 and `-j2`; test debuginfo and incremental state were disabled
  to reduce memory. No release-LTO build was substituted.
* Local GCC is Debian 14.2.0, local linker initially bfd 2.44. The remote
  comparison pins are distinct from this local execution oracle. CI provisions
  its own GAS 2.47 oracle. Do not report remote compilers as locally executed.
* No PMU data was collected. No i7 cycles/IPC/cache or P/E-core results exist.
* `tools/linker/setup_oracles.sh` already pins bfd 2.47, mold 2.42.1,
  lld 23.1.x. Mold's exact `MOLD_TARGETS='X86_64;I386'` option was additionally
  checked against the v2.42.1 upstream CMakeLists. No redundant pin rewrite,
  mold build, linker superiority claim, or full-linker benchmark was performed.
* Snapshot payload is kept below the workspace cap: toolchains/builds and live
  `.git` storage are in excluded `target/`; source, flat patches, source archive,
  clone-tested bundle and evidence are in the persisted workspace. A restored
  `.git` symlink may be dangling: recover from the saved bundle, not that link.

## Reproduction

```sh
./scripts/build_lccc_fast.sh
RUSTFLAGS="$(cat target/lccc-rustflags)" \
 CARGO_PROFILE_FASTBUILD_DEBUG=0 CARGO_INCREMENTAL=0 \
 cargo test --profile fastbuild --lib -j2 passes::cfg_simplify::tests
python3 -m unittest discover -s tests/fuzz -p test_phi_cfg_fuzz.py
CARGO_PROFILE_FASTBUILD_DEBUG=0 CARGO_INCREMENTAL=0 CI_LOCAL_JOBS=2 \
 ./scripts/ci_local.sh --fast
for w in gzip_crc32 zlib_ng_adler32 expat_xml_scan; do
  bash scripts/check_benchmark_outputs.sh "$w"
done
CCC_VALIDATE_SSA=1 python3 tests/fuzz/phi_cfg_fuzz.py \
 --ccc "$PWD/target/fastbuild/lccc" --gcc /usr/bin/gcc \
 --seeds 0:32 --levels O0,O2,O3 --jobs 2 --out target/phi-fuzz
python3 scripts/godbolt.py audit
python3 scripts/codegen_oracle.py \
 tests/benchmark/programs/gzip_crc32.c \
 tests/benchmark/programs/zlib_ng_adler32.c \
 tests/benchmark/programs/expat_xml_scan.c --all-functions \
 --local target/fastbuild/lccc --artifact-dir target/cfg-oracle \
 --json target/cfg-oracle.json --markdown target/cfg-oracle.md
```

The C integration test must additionally be compiled/executed at O0–O3 with
both compilers and stdout compared; SSA verification was enabled for LCCC.
The full slow CI suite is separate from `--fast`; do not infer its result.

## Prioritized continuation, with acceptance criteria

| Priority | Work | Evidence / acceptance criterion |
|---|---|---|
| P0 | Typed Cmp/Cast/Select and Switch evaluation | Differential boundary matrix for I8–I128, signed/unsigned, FP cast boundaries; no narrowing before comparison; verifier green |
| P0 | LongDouble truth predicate | Binary128/x87 representable tiny nonzero constants, +/-0, infinities and NaNs; constant and C execution tests per backend |
| P0 | Asm-goto edge-aware phi repair | Unit cases where removed terminator edge survives as asm edge; phi input retained, SSA verifier and C asm-goto tests pass |
| P1 | Remove invented phi fusion inputs | Malformed input rejected conservatively or diagnosed; no `incoming[0]`/zero fallback; valid fusion corpus unchanged |
| P1 | Resolver budgets/memoization | Repeated-operand DAG, cycles, deep chains; deterministic bounded work counters; no superlinear blow-up in shared DAGs |
| P1 | Forwarding path compression | 10/100/1000/10000-block chains and cycles; exact graph oracle; linear work trend and bounded allocations |
| P1 | Add fuzzer-verdict unit tests to mirrored CI | Update hosted workflow, local gate, and parity metadata together; adversarial tests run automatically |
| P1 | Adler32 live-range/spill root cause | Current 584-byte frame vs GCC 24; identify exact spill homes, loop state and phase interaction; isolate without changing ABI |
| P1 | Baseline on the actual i7 | Randomized paired runs pinned separately to P/E cores, warmups, confidence intervals, PMU counters where permitted |
| P2 | Full upstream workload builds | zlib-ng, gzip, Expat first; prove complete test suites before broadening kernel/glibc claims |
| P2 | Whole-codebase audit | Inventory frontend-to-linker invariants and owners; current document is not a substitute |

For VM work, use exact assembly identity when possible; otherwise deterministic
ROI dynamic instruction counts with correctness gates, explicit startup exclusion,
and repeated counts. Callgrind's simulated cache/branch events are not Raptor
Lake measurements, nor are instruction counts uop counts. Avoid absolute
“zero-variance” claims for libc/ASLR/concurrent programs. Existing `callgrind_ab.py`
and `icount_ab.py` docstrings overstate some of these guarantees and need a
measurement-contract review before numbers from them are treated as hardware
predictions. Preserve per-workload regressions; no geomean can replace them.

## Snapshot recovery

The session uses the repository's already-hardened `scripts/lccc-snapshot.sh`,
which is derived from the requested Kontinuität script but atomically stages
artifacts, clone-tests its bundle, and records CI status rather than swallowing
errors. Snapshots after focused validation are explicitly UNGATED with respect
to **full CI**, not presented as CI-complete. Final fast-only stamps remain
PARTIAL-NOT-DELIVERABLE under that script's stricter full-CI policy.

Persisted names: `/home/user/ms178-1.patch`,
`/home/user/artifacts/ms178-1.SNN-<slug>.patch`, `lccc-src.tar.gz`,
`lccc.bundle`, and `SNAPSHOT_LEDGER.md`. The source archive has no build output.
Recover to a fresh directory with `git clone /home/user/artifacts/lccc.bundle
/home/user/lccc-restored`; install stable Rust, enable swap, and use fastbuild.
Before starting another session, fetch upstream main and rebase the restored
commits; a successful rebase still requires repeating validation and refreshing
the recorded base. No cross-session background agent or guaranteed automatic
execution is implied.
