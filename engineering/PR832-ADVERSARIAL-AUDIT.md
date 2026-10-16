# PR832 versus S34 — empirical red-team audit (in progress)

## Identities and scope

PR832 `31e5acb8c315e7fa2b30a7630f0d58dd79a37149` is merged in main
`63e783b15586eca86f306bd75a52b82c57304bbb`. Preserved S34 is
`b9629da366b8a65b3831c50e8c4b666a3d666be9`; its exact history, checkpoint,
acceptance receipt and compiler are retained. The cumulative work was rebased
onto the actual new main, not layered over the old base as duplicate upstream
changes. Conflict resolution retains PR832's region-aware FP liveness,
fail-closed memory effects, dominance reuse and shared CI build, and S34's
wrapping/FENV/ARM/i386/measurement protections.

The supplied EPYC9V45 report names GitHub test-merge revision `6b6723d1`.
Fetching that commit proves its tree is **identical** to PR832 and merged main:
`282be93bfedca7e27b2e6ced04e33b8eddcedf35`. Source provenance is established;
its binary/options/EPYC timings are not locally reproduced. The supplied
worst-15 ratios are retained in `evidence/pr832/requested-screening.json`. The local host remains a two-vCPU
Intel VM with 8 GiB swap; instruction counts are simulated by Valgrind3.27.1.
Compiler builds use fastbuild/O1/j2. `rustup update stable --no-self-update`
confirmed installed Rust1.99.0 is current; Debian GCC16.2.0-3 is installed.

## Measured first comparison

Pristine PR832 was built in its own source and Cargo-target directories. The
same native matrix gives **76/94 pass**, versus S34's **94/94**. Failures are
16 integer/control witnesses (including four SIGSEGV executions) and two
first-FP-trap witnesses. They are not missing libraries or compile failures.
The initial rebased synthesis restores **94/94**. This directly contradicts
any reading of "every path fails closed" as general optimizer soundness.

All 45 bounded S34/PR832 paired Callgrind cases complete and match outputs.
Geometric Ir ratio is **1.0023542646**. S34 wins spectral norm (**0.942039**),
byte reassembly (**0.875018**), nbody (**0.958244**); PR832 wins several other
cases. This is a mixed result, not universal superiority. The initial
PR832-style wide packed synthesis is **1.0044266957** versus PR832 and
**1.0020678797** versus S34. It restores correctness but costs **6.15%** Ir
on spectral norm versus S34; a narrower synthesis must be tested before
choosing the packed implementation. Neither candidate is accepted merely
because the native matrix passes.

The complete registry-wide GCC16.2/Clang23.1/ICX codegen survey has **45/45
complete, zero failures**, with explicit common non-fast FP flags. Whole-TU
instruction counts are not matched-loop work or runtime superiority.

## Review-AI findings: agreement and disagreement

- **F1 agree:** a self-test does not execute CE. Add an explicit weekly/manual
  strict CE workflow retaining reports; keep network outages out of required
  PR coverage. The main workflow explains the separation. Hermetic parity
  checks pass after restoring all preserved gates to actual hosted steps.
- **F2 partial agreement:** the barrier needs measurement, not an ordinal-only
  relaxation. Guarded do-while probes show **0** global reads inside the plain
  loop versus **4** with acquire, for both GCC16 and PR832. An eight-round,
  race-free release/acquire handshake passes. A deliberately hoisted control
  returns **20 instead of 60**, proving the test detects invalid cross-iteration
  motion. A load textually before an acquire can still follow the previous
  iteration's acquire. Keep the sound barrier without a stronger proof.
  Initial ordinary-for probes retained four reads even without acquire because
  the existing must-execute rule declines zero-trip-body speculation; they
  were not misreported as barrier cost.
- **F3 agree on provenance, not the binary count:** the actual changed evidence
  inventory has **61 files / 2,730,890 bytes, 22 gzip files, zero ELF files**.
  Git binary classification is not an executable count. The new integrity gate
  authenticates every inventoried file and the ten original raw-log hashes.
  Exact producer commands for all historical dumps were not recorded: this
  cannot honestly be retroactively repaired. The evidence README distinguishes
  integrity, reproducible new experiments and unavailable historical objects.
- **F4 agree:** replace repeated full-function searches with a lazy immutable
  definition index per read-only analyzer. Duplicate definitions fail closed
  in release as well as tests. A once-per-whole-pass cache would become stale
  after vectorizer transformations; do not implement that shortcut.
- **F5 disagree with the proposed assertion:** failed speculation legitimately
  advances its private counter. Equality with the committed counter would
  fire on valid fallback. Instead only an owned successful plan can export
  IDs/instructions; failure drops both. Test partial allocation, rejection,
  successful unique IDs and result ownership.
- **F6 agree:** feed both committed architectural fixture families through the
  real peephole pipeline, with an active positive canary and native comparison
  against the originals. GCC is the assembler/driver, not the optimizer under
  test. Keep the existing architectural negative controls too.
- **F7 strengthen:** block count alone does not detect edge rewrites. Check
  actual adjacency before/after IVSR in debug builds and test a same-count
  changed-edge negative control in ordinary fastbuild unit tests.
- **F8 partial agreement:** upstream is already merged; retroactively splitting
  its PR is not an available action. Keep new work in reviewable commits and
  separate source changes from external measurement archives. Do not re-add
  upstream's evidence as new changes.
- **F9 agree:** document the scalar F64 result at the intrinsic variant. An
  input vector width must not be inferred to be its result allocation width.
- **F10 agree:** these are actual local builds/native/Callgrind executions, not
  review-only or inferred CI results. Final exact-tree FAST/fmt/clippy remains
  required; full/slow local CI is forbidden and has not been run.

## Review order and unfinished acceptance

1. S34 safety preservation: `loop_bound`, signed-affine/rotation/IVSR guards,
   shared FENV guards including backedge PRE, ARM input homes and i386 tags.
2. Packed plan, min/max definition index and IVSR graph contract in `src/passes`.
3. Compiler-side fixture execution in `vector_copy.rs`.
4. Ordering/evidence harnesses, their negative controls, and CI wiring.
5. Actual comparative profiles and rejected variants in external artifacts.

Remaining: validate focused Rust tests, evaluate a narrower packed plan,
refresh final compiler measurements, inspect worst15 assembly/oracles, complete
exact-tree FAST/fmt/clippy, recheck main and prove final patch/restore identity.
No claim of exhaustive bug elimination or universally optimal code is made.

## Additional reproduced defect: temporary directory lifetime

The compiler-side architectural fixture test initially failed on its **second**
fixture at file creation, before assembly or execution. `TempFile::drop` removed
an empty process-private directory while a `OnceLock` retained its path. The
same two-generation caller against pristine main's module panics (exit101);
the repaired module succeeds. Five O1 module tests cover 32 successive private
generations, an unmaterialized sibling guard, 16 parallel writers, retained
files and externally owned parents. This is a pre-existing resource-lifetime
bug, not a demonstrated peephole miscompile.

Managed temporary paths now hold shared directory leases. Only the last lease
can reap its empty directory; a new generation gets a fresh securely-created
path, never an adopted/recreated stale name. Debug/unmanaged paths keep a
separate persistent directory. Cleanup never recursively removes kept files.
The six packed-plan/Copy-index/CFG-contract unit tests also pass. Full compiler
fixture execution and final gates must be repeated after the lifetime repair.

## S37 intermediate result — not accepted

The I32x4 denominator plan passes six plan/index tests, the native 94/94
matrix, nine 9,108-comparison packed fixture executions, and nine 1,260-case
bitwise/Memcheck arms. The existing no-promotion assembly assertion initially
caught two legacy broadcast-result copies. Direct emission to the owned typed
XMM home removes them without weakening the assertion or changing arbitrary
legacy instructions. The 45-case narrow/S34 simulated-Ir geometric ratio is
1.0000221890; spectral is 0.9682659904, versus the wide variant's 1.0615272.
This is a selected architectural improvement, not universal superiority.

**New blocking F6 finding:** after the TempFile repair, the real pipeline
removes the live `movsd %xmm6,272(%rsp)` feeding `fxrstor64 (%rsp)`. The fixture
adapter now explicitly expands its limited GAS macro grammar and compares
GAS-original, expanded-original and optimized execution. Both originals agree;
the optimized program fails. Disabling phase 5 makes the full fixture pass.
The phase-5 never-read-store pass assumes arbitrary inline-asm frame accesses
fit 64 bytes, independently of PR832's stronger FP-liveness footprint model.
The repair and exact-tree FAST acceptance are still pending at this checkpoint.

The ordering harness now optionally profiles 100,000 uncontended iterations.
GCC16 plain/acquire whole-program Ir: 120,835 / 1,020,835; narrow LCCC:
520,843 / 1,320,838. These include the acquire load and different optimization
shapes, not isolated barrier latency, contention or hardware timing. The
8-round handshake and deliberately incorrect hoisted control still distinguish
60 from 20. Run `scripts/check_licm_ordering.py --callgrind --report FILE`.

The scheduled CE gate's actual `--strict --report` CLI was executed against
the frozen wide compiler: no errors or baseline failures. Narrow final-tree
execution remains pending. Evidence verification now checks malformed schemas,
duplicate manifests and every gzip CRC; 61 artifact hashes, 22 gzip streams
and 10 decompressed-log hashes pass. Twenty-one hermetic audit tests pass.
Our own three historical bulk archives (9,903,698 bytes) are externalized
losslessly; see `evidence/pr824-redteam/EXTERNAL-RAW.md`. This addresses source
bloat without deleting evidence or pretending historical commands are known.

## Frame-read repair validated (S38 interim)

The shared architectural read-extent contract now also governs the independent
never-read-store pass. Unsupported/state-image extents make its function opaque;
known scalar/vector extents still permit unrelated dead stores to disappear.
This replaces three arbitrary 16/32/64-byte read ceilings, not PR832's sound
separation between possible reads and guaranteed overwrites. The unbounded-floor
endpoint comparison is widened to i64 so a valid i32 displacement plus a store
width cannot wrap. Direct tests cover FXRSTOR/XRSTOR, unsupported instructions,
ZMM overlap, YMM non-overlap, MXCSR's four-byte extent and the i32 endpoint.

The complete architectural fixture now passes through the real pipeline, with
both expanded and original GAS controls agreeing. Six packed-plan tests, five
TempFile tests, the CFG contract and both new frame tests pass. Production build,
final profiles and full FAST acceptance still follow. The defective independent
DSE ceiling also exists in base `7f9eb225`: it was missed by **both S34 and PR832**,
not introduced by narrowing. Approval of the FP-liveness helper alone was not
proof that every consumer had adopted its contract.

Offline recovery independently reconstructs the S37 head/tree and, using the
saved upstream bridge and S34 bundle, the exact S34 head/tree and old base —
without accessing the live audit repository or a remote. The checked-in
`verify_preserved_checkpoint.py` reproduces the second proof from an already
restored current repository. The GAS-fixture adapter now rejects repeated else
branches and substitutes complete parameter tokens; all 23 hermetic tests pass.

## Temporary-file follow-up — security and ownership

Further review found two gaps. A kept-but-uncreated path must retain its parent
(the first lease repair only retained parents made nonempty by actual files).
A per-generation atomic retention flag now transfers that cleanup responsibility
on guard drop; six standalone O1 lifecycle tests pass, including a write after
handoff. Changing `set_keep` back to false before drop still permits cleanup.

More seriously, a read-only TMPDIR can contain a pre-placed symlink whose target
is writable. The inherited private-directory failure fallback warns and then
writes through that symlink. An actual pre-repair compiler fails the strengthened
existing `check_temp_symlink.sh` gate by overwriting its sentinel. The standalone
causal probe likewise prints `victim=overwritten` before and `victim=safe` after
the repair. Private-path reservation no longer falls back to flat names.
Production assembler/driver paths use fallible `TempFile::try_new`, propagating
an ordinary diagnostic; optional debug dumps return None. The convenience
`new` retains its signature but panics on reservation failure. Timestamp-based
naming is no longer described as a security secret: atomic creation and ownership
checks, not assumed unpredictability, provide the protection. Full rebuilt-CLI
and exact-tree acceptance are recorded separately, not inferred from the module
tests. The new readonly attack arm reports a skip when elevated privileges
bypass directory permissions rather than pretending to exercise that case.
