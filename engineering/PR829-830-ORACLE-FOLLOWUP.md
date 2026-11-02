# PR829/830 post-recovery oracle audit — 2026-10-15

## Scope and upstream check

S31 was recovered offline with its original history, HEAD
`790e220c975ca2daede85462d8d04cd1e8e14bb3` and tree
`49d8b4b6b2607aeada34668c0d7b1d735ca63d6e`. A safety branch was made before
fetch/rebase. Git transport, commits/main and a cache-busted Git ref API all
still advertised main `7f9eb225def23bda9370d3349304b9b785554fea` at the first
check in this continuation. Rebase onto that main completed as a no-op.
The final publication must check again; no newer merge is assumed.

This follow-up changes measurement scripts/tests, not the accepted compiler
optimizations. The comparative compiler decisions and unresolved debt remain
in `engineering/PR829-830-COMPARATIVE-AUDIT.md`. Historical 94-case correctness
and 45-case Callgrind records are not represented as fresh runs: the cache
reset removed the final raw supplement and compiler binaries. A rebuilt
compiler has a new hash and requires new evidence.

## Repairs and why they are necessary

* `scripts/benchmark_codegen.py`: an empty GCC include response previously
  became `Path('')`, which is the current directory and passes `is_dir()`.
  Reject empty output before compiling. Also reject duplicate registry IDs
  rather than silently reducing them to a set.
* `scripts/tight_loop_oracle.py`: discover builtin headers from an explicit
  GCC provider (default gcc-16), and pass the same optimization/ISA flags to
  local textual assembly, local integrated assembly and vendors. Missing
  sources, failed local compiles, missing responses and vendor failures are
  incomplete surveys with nonzero status; optional JSON retains errors,
  flags, source hashes and partial histograms. Bound local subprocess time.
  Mixed integer/cascade histogram keys no longer crash sorting. Intermixed
  CLI arguments now work; shrink the exact legacy argparse ratchet.
* Alignment parsing: select the strongest **unconditional** directive in
  either order, distinguish an explicit fill plus maximum-skip operand,
  ignore comment text, and stop at labels/instructions/data-emitting
  directives. Share this small parser between the local and vendor census.
  A loop without an alignment directive is no longer fabricated as log2=3.
  Bounded groups remain labelled cascade, not guaranteed alignment.
* `scripts/loop_density_oracle.py`: retain compilation errors in JSON, mark
  completeness, return failure, and exclude every row from a source with
  incomplete compiler coverage from ranking. Partial diagnostics survive;
  success by a surviving vendor does not turn an outage into an apparent win.
* `scripts/tune_oracle.py compare`: remote failure, empty function assembly,
  or an explicitly requested absent function returns nonzero. Reject empty
  and repeated oracle selections. Still print/save successful arms to aid
  diagnosis instead of silently substituting an empty successful column.

The rejected alternative was to tolerate outages and merely warn: that is
inappropriate for evidence used to accept compiler changes. No live CE call
was added to local/hosted hermetic regression tests. Directive histograms and
cross-compiler innermost-loop density are exploratory static observations,
not matched-loop equivalence, semantic or timing proofs. In particular the
old tight-loop module's per-loop hardware-goal claim was not supportable by
aggregate buckets and has been removed.

## Executed focused validation

* New `tests/corpus/test_oracle_survey_fail_closed.py`: **20 tests pass**.
  Includes successful controls, ISA/header propagation, remote/local/source
  failures, absent functions, incomplete vendor rankings, mixed histogram
  keys, strongest/bounded/fill/comment/data-boundary alignment parsing,
  empty includes and duplicate registry IDs. Subprocesses and vendor requests
  are mocked; no external compiler or network is needed.
* Nine assertions were separately run against the unmodified S31 helper
  implementations: **9 assertion failures, 0 harness errors**. The same nine
  assertions pass against the repaired implementations. Exact names and
  baseline identity are in
  `engineering/evidence/pr829-830/oracle-failure-controls.json`.
* Existing Callgrind contracts: **35 tests pass**; registry regression:
  **6 tests pass**; script import gate: **174 helpers pass**; documentation
  link check and `git diff --check` pass at this checkpoint.
* Rebuild used `scripts/build_lccc_fast.sh`, fastbuild, Rust O1, fixed j2,
  debug=0, incremental=0. The new compiler SHA256 is
  `ca7a022e9ca63539429ab35373807ec1be940b62955e929278dd64c0be585e3f`.
  A frozen copy is used for measurements; build/cache artifacts are not part
  of the source patch. An 8 GiB swap file is enabled and was used by the build.

## Acceptance still required at this checkpoint

Run only `ci_local.sh --fast`, including rustfmt and strict clippy; never run
full/slow local CI. Refresh native witnesses, Valgrind/Callgrind and the
GCC16.2/Clang23.1/latest-ICX CE survey with raw output preserved outside the
excluded cache. Exercise the repaired exploratory tools as well as the
registry-aware survey, with limitations and failures disclosed. Recheck main
before publication, then prove patch application and exact offline recovery.
A checkpoint is not final acceptance: the external final receipt must match
its exact tested tree. No remote push, hardware-speedup claim, universal
optimality claim or claim of eliminating all existing bugs is warranted.

## Post-live self-review and S33 checkpoint

S32's immutable tree `67cbc2343d5fedf7db39547a29e2201e5b837dcd` completed
FAST: **257 passed, 0 failed, 6 skipped**; Rust **4746/0/8 ignored**,
rustfmt, MSRV 1.97 and strict clippy all passed. This is not automatically a
pass for the later S33 script/document changes; final FAST must match the final
tree. The compiler source and frozen compiler hash remain unchanged.

Live use exposed a presentation/measurement-category defect: vendor loops with
no directive were still labelled cascade, and local row labels could print
`2^unaligned`. Preserve **unaligned**, **bounded cascade**, and **unconditional
power-of-two alignment** as distinct categories through parsing, JSON and text.
Also retain a structured failure receipt for a silent local compiler failure.
The focused suite now has **24 passing hermetic controls**. A three-source,
three-vendor live rerun completed with the corrected labels. The original
survey remains archived as original evidence, not silently rewritten.

Fresh native counts are current **94/0**, Clang23 **94/0**, GCC16 **89/5**,
and PR829 **90/4**. PR829 initially had 40 additional link failures because its
older i386 library search did not discover installed GCC16 multilib. Installing
its exact GCC14.2 multilib prerequisite, without changing its source, exposed
its four genuine FENV failures. The failed infrastructure run is retained and
is not counted as 40 semantic miscompiles. The cumulative compiler already
worked with GCC16-only multilib, preserving the earlier portability repair.

The refreshed 45-case current/PR829 Callgrind comparison has **45 Ir ties**.
The i686 byte witness has **82,469,158 / 103,229,153 Ir**, ratio
**0.7988940682289624**. The separate bounded worst15/GCC16 screen has geometric
Ir ratio **1.2471286275411166**, not a runtime speed ratio: important gaps remain.
For example byte-load reassembly is **4.286985** and spectral norm **1.960305**.
The byte accessor already folds to one load; GCC's assembly vectorizes work
across outer rounds, whereas LCCC retains the scalar inner loop. This is not
fixed by guessing a wider load or relaxing memory/control proofs. Same-loop
matching is not established by the exploratory density survey.

Compiler-work profiles cover four sources, three rounds each, with native and
instrumented assembly identity checks. Median Ir ratios span approximately
**0.99704–1.00112**; these are not compilation wall-time improvements. Fresh
Memcheck arms (24 wrapping, 8 readonly, 9 packed reciprocal), 13 real RV64
loader cases and 18 small multi-linker TLS runtime smoke cases pass. Versioned
GCC16/Clang23, Valgrind3.27.1, BFD2.47, lld23.1.3 and a private official mold2.42.1
binary were verified. Mold's smoke is not a source-build/full qualification;
the source-build architecture option remains `-DMOLD_TARGETS='X86_64;I386'`.

The CE version probe reports GCC **16.2.0**, Clang **23.1.0**, and
`cicxlatest` actually **ICX 2024.0.0 (2024.0.0.20231017)**. CE's C list exposes
no newer ICX release. Do not call that a 2026 release merely because the
channel says latest. All three execute the wrapping-control witness correctly.
ICX initially rejects the FENV pragma under its default FP policy; with the
same explicit `-fno-fast-math -ffp-contract=off -frounding-math` flags for all
three, Clang and ICX execute the expected FENV digest, while GCC reproduces its
rounding mismatch. Both attempts and flags are retained. A separate common
Haswell admission canary passes all nine feature families for GCC, Clang, ICX
and ICC Classic; ICC uses its explicit `-xCORE-AVX2` dialect, not a fabricated
x86-64-v3 spelling.

Compiler metadata fingerprints are not compiler-binary hashes. Existing cache
policy detects metadata drift and expires moving-channel entries weekly; it
cannot detect a same-version rebuild or a change within that window. Clarify
those limits in the helper comments instead of claiming an exact build pin.
This continuation started with a cold cache and retains live version probes.
Fresh compact results and hashes are in
`engineering/evidence/pr829-830/fresh-evidence-summary.json`; raw measurements
and the final exact-tree acceptance receipt remain in the external artifact
zone so recording acceptance does not mutate its own tested tree.
