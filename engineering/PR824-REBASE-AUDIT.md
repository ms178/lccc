# PR #824: rebased architectural-effects audit

Date: 2026-10-14. Branch: `arena/c48023c2-lccc`.
Current rebase base / freshly fetched main: `3ff36e8592b0d0e26645bf37d6685395b1a6eca5`.
Earlier `fb02007` tables below retain their original stage identity.
The original measurement stage used `d9d0a2b` on the earlier `3c1f893` base;
those retained counters are not relabeled as measurements of this new main.
Remote head `78cd2c5` was fetched, compared and restored onto the rebased tree
before folding. Its copy/spill repairs, 20 native controls, workflow additions
and historical raw-counter summary were preserved, not overwritten.

## Subsequent research/CI correction (current)

- Fresh fetch/rebase still advertises main `3ff36e8592b0d0e26645bf37d6685395b1a6eca5`.
- Per explicit user direction, remove the entire live Compiler Explorer GitHub
  job and the corresponding local survey/diagnostic CI gates. Keep explicit
  research scripts and offline tests; do not substitute another PR gate.
  CI parity now passes **224 commands / 22 contracts**, all five workflow-shell
  checks pass, and three offline policy tests guard the known live CLI paths.
- Actual prior-head hosted rejection: classic ICC `cicc2021100`, warning
  `#10148`, ignored `-march=x86-64-v3`; all 45 canonical survey requests failed.
  This is a diagnosed target-profile failure, not evidence of compiler speed.
  The optional research Haswell dialect adapter and nine-feature compile-only
  canary have offline tests. No real Haswell four-vendor admission is claimed.
  Effective flags are used in remote cache keys; requested flags are retained.
- Rejected the lazy immutable CFI index after measured screening: 49 selected
  Rust tests passed, including scanner-equivalence and invalidation-contract
  tests, but six alternating compiler-work pairs yielded minmax +0.264%, libm
  -0.215%, chain-64 -0.012% simulated Ir; geomean +0.012%. All paired assembly
  was byte-identical, including native/instrumented compilations. No convincing
  overall benefit offsets the whole-file allocation and future invalidation
  obligation. Restored Rust source exactly matches the earlier folded compiler.
  Full counters, hashes, commands, order and timeout policy are retained in
  `evidence/pr824-rebase/boundary-index-screen.json` and its driver.
- The new fast run tested the temporary prototype (4,716 library passes, zero
  failures, eight ignored). Source/controller changes during that run mean it
  cannot certify an immutable final tree. Required local GCC16/multilib/libgcc,
  Rust1.97 and pinned external prerequisites remain unavailable, not waived.
  No full/slow local CI was run. Prior validation below is explicitly historical.

Final restored Rust all-target tests: **4,714 library passes / zero failures /
eight ignored**, plus the bin test. Final Python corpus: **465 tests OK, four
skips**. Rust1.99 formatter and strict Clippy pass
(76 policy names probed, zero unknown). The in-flight fast run ended
**201 passed / 42 failed / 13 skipped**, exit1, and also emitted a partial-comment
shell error after its controller was edited during execution. It is explicitly
**not** an immutable final-tree acceptance stamp. Missing local prerequisites
remain blockers. See `engineering/evidence/pr824-rebase/no-godbolt-fast-validation.json`
and the associated failure excerpts. No full/slow local CI was run.

## Earlier folded-stage follow-through (historical; not merge approval)

All in-flight compiler changes were saved, then rebased cleanly onto main
`3ff36e8` (including PR826). No full/slow local CI ran. The current compiler
source is unchanged from the source instrumented in `latest-main-measurements.json`;
subsequent changes affect the semantic census, its tests and provisioning mocks.

- **Dynamic U64 map repair:** retain the offset-ring proof, select a checked
  packed endpoint for admitted ranges, and fall back to the original scalar
  loop otherwise. The final fused loop has one byte IV, not the earlier two
  counters. Genuine x64 AVX2/SSE2 reach and GCC/scalar differential tests pass
  with `CCC_VERIFY_IR=abort CCC_VALIDATE_SSA=1`. i686 is not covered by that
  focused extraction. An explicitly experimental guard-threshold-32 assembly
  mutation reaches fallback on ordinary defined inputs and matches separately
  GCC-compiled original C; it is not a production policy or enormous-array test.
- **F8 semantic census:** replace displacement grep with a CFG/provenance
  census of the pinned 56-byte Body/280-byte workload. Count direct, copied,
  hoisted and folded mass reads. Reject conflicting/unknown addresses, partial
  GP writes, overlapping spills, unproved frame pointers and literal aliases.
  A further red-team test demonstrated **six previously accepted hidden-effect
  mutants**: calls, indirect calls, unknown register/memory operations, implicit
  string reads and stack pops after address facts are re-seeded in the header.
  The fixed gate rejects them explicitly, rather than relying on accidental
  downstream clobbers. **13 methods pass**; actual shipping nbody still passes,
  with exactly two reads (`48(%rsi)` and `(%r13)`). This is an intraprocedural,
  workload-specific instruction-operand contract, not universal assembly analysis.
- **F10 provisioning:** **7 GCC + 7 swap mock tests pass**. Both privilege
  paths, distribution/PPA policy, multilib package preservation, activation
  confirmation, idempotency and failures are covered without privileged or
  network effects. Readonly EUID is substituted only in a temporary test copy;
  no production bypass was added. Actual four-GiB swap remained active/used.
- **Oracle observability:** preserve actual CE rejection/transport diagnostics
  in structured errors, and publish bounded, escaped, always-on failure notices
  even if a survey step failed. Ordinary warnings remain admissible; ignored
  target options remain failures. Tests cover missing/malformed evidence and
  oversized/control-heavy diagnostics. No vendor is dropped or silently given
  different target flags. That stage's unresolved survey failure was subsequently diagnosed as classic ICC
  rejecting the v3 target; the live CI job is now removed (see above).
- **Latest FAST execution:** **202 passed / 42 failed / 15 skipped**, exit1.
  Rust **4,714 passed / 0 failed / 8 ignored**; Rust1.99 rustfmt and strict
  Clippy pass (76 names, zero unknown compatibility entries). Failures retain
  concrete diagnostics for absent GCC16, multilib/libgcc, Rust1.97 and pinned
  external gas/Arm/kernel tools; they are not waived. Follow-up Python files
  changed during the run, so there is **no immutable final-tree green stamp**.
  Final standalone corpus: **455 tests pass, four skips**; changed helper suites
  and real nbody gate were revalidated independently. CI/local parity passes
  (227 commands/22 contracts). See `latest-fast-validation.json` and all 42
  retained `latest-fast-failure-excerpts.json` entries.

### Fresh latest-main Callgrind evidence and decisions

Independent archived main and candidate were built with genuine Rust1.99,
matched fastbuild settings/flags and different binary hashes. Manifests retain
full counters, source/binary/profile/output hashes, problem-size macros and
native/instrumented output parity. Raw profiles stay in ignored `results/`.
Exact-version GCC16 hosted data below is **earlier-head** data; these fresh local
GCC measurements are honestly identified **Debian GCC12.2**, not GCC16.

| Bounded workload | Candidate/main simulated Ir |
|---|---:|
| nbody | 0.998447 |
| spectral norm | 0.952084 |
| double reduction | 1.000000 |
| ring FIFO | 1.000000 |
| libm rounding | 0.993330 |
| byte reassembly | 1.000000 |
| zlib adler32 | 1.000000 |
| scaled find-bit | 1.000011 |
| LZ4 match extension | 0.988046 |

All nine complete, with native and instrumented parity. The same nine against
GCC12 also complete without failures; full ratios remain in the manifest, not
relabeled as competition-wide superiority.

The checked-in unsigned-map kernel plus its correctness battery and 20,000
repeated calls costs **16,385,881 candidate Ir / 16,204,903 main Ir = 1.011168**;
GCC12 costs 37,370,747. Equal-length executable paths were used. Keeping the
admission guard is a deliberate safety/performance tradeoff, **not a free win**:
remove it only with an equally sound modular-address proof, not a weakened gate.

Whole-compiler simulated-Ir candidate/main: minmax **0.981946**, libm **0.989902**.
The synthetic C operation counts double; exact IR instruction cardinality is
not claimed. Four chain points 16/32/64/128 give **1.048168 /1.050167 /1.047857
/1.045125**; diamond 8/16/32/64 give **1.015425 /1.019510 /1.036848 /1.044599**.
Both arms show superlinear whole-pipeline scaling; the candidate costs more on
these stressors. The larger diamond 128 **baseline** hit the 180s instrumentation
budget (rc -9, no usable profile); the candidate was not attempted. The original
screen remains explicitly incomplete; a narrower follow-up completed the four
bounded points without retrying the timed-out request. One paired observation
per point is a screening measurement, not a statistical wall-clock speed claim.

**F9 remains partial:** immutable IVSR definitions/use indexes and bounded
traversal exist, but these whole-compiler costs do not prove an IVSR-only cause
or a phase-complexity improvement. Broad AVX2/SSE2/NEON plan extraction is not
silently declared complete. Profile attribution/generic Rust symbol folding
also does not independently prove which source phase caused the overhead.
A speculative semantic rewrite without target/ABI coverage is not justified by
these numbers. Future optimization must preserve the refusal and native controls.

The required `--fast` green and fresh four-vendor/current-head hosted green are
still acceptance blockers. No perfected patch, universal superiority or merge
approval is asserted.

## Earlier fb02007 continuation (historical stage)


- Two direct copy regressions were demonstrated before repair: SHA256RNDS2
  hidden-XMM0 bracket input and VPCMPISTRM hidden-XMM0 output. Focused baseline:
  one passed / two failed; the independent first fast CI reproduced both.
- The repair-only Rust stage passed **4,693 tests**, zero failures, eight
  ignored; other targets passed. Rust 1.99 rustfmt and strict Clippy passed
  (76 lint names, zero unknown compatibility entries). This stage preceded
  restoration of the remote's additional copy/spill source; it is not the
  final combined-source all-target acceptance result.
- After restoration, **20/20 native architectural original/illegal-rewrite
  pairs** passed again, including actual SHA execution. Wide minmax differential
  and 20-shape gate passed with SSA validation requested, at five optimization
  levels; unavailable i686 coverage remains explicit.
- Remote Test Suite failure was reproduced locally: the FMA gate expected a
  later diagnostic after the new typed preflight already refused the unsafe
  comparison. The gate now recognizes the precise preflight, checks noinline
  functions individually (strict `<`: eight packed adds; `<=` and `!=`: zero),
  and still runs guard-page/native parity and IR validators. The complete
  focused gate passes. No refusal or positive-control assertion was removed.
- Red-team review found a second harness defect: failure flags inside shell
  command substitutions did not propagate. Compile, execution and output
  mismatches now return nonzero explicitly, with independent mutation tests.
- The failed hosted CE survey used the script's default `gcc-16` header driver
  on an Ubuntu job that did not install it. It now names native `gcc` only as
  the recorded builtin-header provider; CE GCC16 and native Debian GCC16
  identities are kept separate. Fresh CE measurements remain required.
- Python corpus: **436 tests passed, four skips**. Research-publication tests
  exercise missing reports, failures, incomplete coverage and complete actual
  corpus schemas. Workflow shell and CI/local parity pass (224 commands,
  22 registered invocation contracts); whitespace checks pass.
- The final combined-source **`ci_local.sh --fast` completed non-green**:
  **197 passed / 44 failed / 15 skipped**. It passed 4,701 Rust library tests,
  other targets, rustfmt and strict Clippy; PGO I/O 60/60, RISC-V relocation
  172/172 and linked regression 913 cases passed. No slow/full local CI ran.
  Failures were reviewed, not silently waived. Most require genuine GCC16.2,
  i386 development/libgcc objects, pinned gas2.47/Arm XML/kernel tools and
  network hosts unavailable here. The regression's nine failed links explicitly
  reported missing libgcc; linker fixture failures include missing multilib
  and unsupported assembler forms. Exact Rust1.97 also remains unavailable.
- Two non-environment fast failures were subsequently repaired: the added
  minmax preflight's redundant debug-environment read now uses the caller's
  resolved policy (151 reads, unchanged budget151; focused gate passes), and
  documentation now honestly labels an unarchived historical ICX assembly
  artifact instead of requiring its missing runtime path (focused gate passes).
  All-target Rust/fmt/Clippy validation of that final diagnostic-only refactor
  is running separately; the 4,701 result above precedes that last refactor.
- **A genuine map profitability regression is still open:** runtime-U64
  `sub64` and related map shapes lose the packed path because our byte-offset
  no-wrap proof checks the entire unconstrained U64 domain. Native/scalar/GCC
  parity passes before the shape failure, while independently built main passes
  the x86-64 reach checks and then stops at unavailable i686 headers. Do not
  weaken the shape test or drop the offset-wrap proof to turn this green.
  A sound full-width modular-address proof or correctness-gated runtime
  versioning (with scalar fallback for unproven ranges) needs targeted design
  and testing. Narrow pre-widening U32 wrap hazards must remain refused.
- Four GiB swap is active. Genuine pinned **Valgrind3.16.0.GIT** rebuilt
  successfully. Only the GNU bootstrap tools received modern-glibc/timestamp
  compatibility repairs; Valgrind instruction/counter code was not patched.
  Source recipe and all tool/binary/profile hashes are retained. DWARF5 and
  unhandled-rseq warnings remain visible limitations, not suppressed evidence.
- **Fresh current-source instrumentation:** nine targeted workload comparisons
  completed against independently archived/built latest main and against
  identified native GCC12, zero compilation/native/instrumentation/parity
  failures. Flags and bounded problem-size macros match in both arms. These
  measurements preceded the final diagnostic-policy forwarding change only.
- **Fresh local-only static registry:** all45 macro-aware canonical cases
  compiled/analyzed successfully. This validates local script/workload
  contracts, not a fresh GCC/Clang/ICC/ICX comparison. CE remains blocked from
  this sandbox; fresh hosted four-vendor data is still required.
- **Compiler-work Callgrind:** three alternating paired repetitions each on
  minmax/libm, equal-length paths, pinned cache geometry, `.text`-preserving
  debug stripping and native/instrumented assembly-byte parity. Median whole-
  compiler simulated-Ir candidate/main: **0.990303** and **0.995055**. These
  small modeled-work reductions are not compilation wall-clock speedups.

### Fresh nine-case simulated-work screen

Candidate binary SHA256:
`f74774870778ae6b8a58f0bba97652e07d5deabc0d2a77981efbb0a806e37d4c`.
Baseline is independently archived main `fb02007`; both Rust builds used the
same actual compiler, profile and linker flags. Full manifests (source macros,
commands, tool identities, native/instrumented stdout hashes, counters and raw
profile hashes) are retained in `engineering/evidence/pr824-rebase/` as
`current-nine-main.json`, `current-nine-gcc12.json`, `current-compiler-work.json`
and `current-static-local.json`. Raw profiles stay in ignored `results/`.

| Case | Candidate/main Ir | Candidate/GCC12 Ir |
|---|---:|---:|
| nbody | 0.998447 | 1.134678 |
| spectral norm | 0.952083 | 1.115246 |
| double reduction | 1.000000 | 1.125963 |
| ring FIFO | 1.000000 | 2.619365 |
| libm rounding | 0.993328 | 0.966704 |
| byte reassembly | 1.000000 | 3.938086 |
| zlib adler32 | 1.000000 | 0.906326 |
| scaled find-bit | 1.000011 | 1.103144 |
| LZ4 match extension | 0.988045 | 1.102906 |

Whole-program simulated Ir includes startup/libraries and bounded workloads;
these are not cycle measurements, source-default runtime rankings or universal
compiler superiority. Unchanged large losses argue against victory narratives.

### Hosted follow-through

- A supplementary hosted research lane independently archives/builds main,
  records source and binary hashes, and correctness-gates six bounded
  main/candidate Callgrind comparisons. Existing Debian and CE lanes own the
  full 45-case GCC16 instrumentation and four-vendor static/density registry;
  these expensive surveys are not duplicated in the extra research lane.

### Newly recovered hard GCC16 evidence (earlier head, not this head)

Run `38585987349`, check `115928011916`, branch head `78cd2c5`: genuine Debian
**GCC 16.2.0-3**, Rust1.99 lane succeeded. The annotation identity names synthetic
merge checkout `cdf2a53871ca883a983db04bb25a7b5e4017dd53`. All **45/45 workloads**
compiled and matched output; all **45 bounded Callgrind rows** were published.
Complete recovered rows, parity/hashes and identity are retained in
`engineering/evidence/pr824-rebase/hosted-78cd-gcc16.json`.
Callgrind index SHA256: `d1819483624acf88d8118a857bcaf12d4f5727be5bd1d21e19f9a57ef0fdce9b`.
Whole-program simulated-Ir geometric ratio LCCC/GCC16: **1.000095**. Losses
include byte reassembly **4.8962**, spectral norm **3.0299**, ring FIFO **2.4027**,
histogram **1.8975**, strstr **1.7668**. These are bounded simulated work, not
native cycles, a speedup or superiority. Assembly/static density likewise
cannot establish equivalent work, algorithms or floating-point contracts.

F8's semantic nbody mass-load metric, broader vectorizer/refactor questions and
exhaustive swap-helper mocks remain open; neither successful instrumentation
nor this follow-through certifies their completion.

---

The remaining sections retain the historical stage's methods and observations.

## Decisions supported by evidence

1. **Preserve counter domains, not convenient register widths.** The existing
   repair keeps signed I32/I64 reduction bounds, scalar tails and escaping IVs
   in their source/promoted domain. Clamping the limit to the start before
   subtracting avoids overflow for empty signed ranges. Do not revert this to
   an I32 cast or an unchecked subtract. The independent i128 evaluator and
   wide-counter native fixture are complementary checks, not exhaustive proof.
2. **A liveness repair is not a complete copy-cache repair.** Preserving an
   implicit feeder in dead-move elimination does not stop forward substitution
   of a register that an instruction implicitly overwrote. The copy pass and
   spill-home cache now consume the same conservative vector-effect admission.
3. **Kill cached equalities with MAY writes.** Spill homes must account for
   ECX string-index results, the first `xchg` operand, and unmodelled outputs,
   not merely the textual last operand. Reuse the existing GP MAY-write model.
   Its string-index/mask distinction is now exact, and its vector admission
   shares the positive ISA family table instead of duplicating forty names.
4. **Separate low-XMM from upper zeroing.** An initially over-conservative
   implementation broke existing scalar forwarding and YMM-bracket positive
   controls at `vzeroupper`. It remains a local copy barrier, but neither
   observes an old temporary nor changes the low bits of a scalar spill home.
   Retaining these controls is important: safety must not silently disable
   already justified optimizations.
5. **Measure before broad redesign.** No speculative target-neutral vectorizer
   rewrite or claim of universal compiler superiority is justified by this
   evidence. The bounded corpus contains substantial losses as well as wins.

## Newly covered architectural hazards

- String-mask results write XMM0 without naming it, invalidating a preceding
  vector copy or XMM-backed spill home.
- FXSAVE observes the copy bracket's intermediate state; coalescing the bracket
  changes the saved image even if the final live vector values agree.
- A string-mask instruction can define the upper lane before a narrow copy;
  treating it as an undefined/private lane permits an invalid widening.
- Legacy BLENDVPD observes the implicit XMM0 upper-lane mask. Retargeting a
  scalar producer into XMM0 changes that mask despite an unchanged low result.
- String-index outputs invalidate an ECX-backed spill home.
- State-image `q` aliases accepted by GNU as (`fxsaveq`, `fxrstorq`, `xsaveq`,
  `xrstorq`, `xsaveoptq`) receive the same effects as their `64` spellings.
  GNU as rejected `xsavecq`, `xsavesq`, `xrstorsq` and the tested `l` spellings;
  those were not invented as supported aliases.

The native suite now executes **20 original/illegal-rewrite pairs**, including
five new copy/spill controls. On this host every original returned 1 and every
illegal rewrite returned 0; SHA was actually available and executed. Privileged
XSAVES/XRSTORS are modeled but not claimed as executed native coverage.

### Correction to our own negative control

The first experimental retarget assertion used SHA256RNDS2 as an upper-lane
observer. That was too strong: its two message words are in the low 64 bits.
That failed assertion is **not** proof of an upper-lane architectural hazard.
It was replaced by BLENDVPD and a separately executed upper-mask witness.
Three other isolated copy-pass assertions failed against the unmodified
baseline; the archived fourth SHA failure is deliberately excluded as proof.
The complete final test status must be read below, not inferred from source.

## Executed baseline measurements

`engineering/evidence/pr824-rebase/baseline-callgrind.json` retains compact
45-case measurements, source/output/tool hashes, workload flags and own-object
counts. Raw profiles/manifests remain in ignored `results/pr824/`.

- **45/45 bounded Callgrind cases completed**, zero compile, native execution,
  instrumented execution or output-parity failures, against Debian GCC 12.2.
- Flags: `-O2 -march=x86-64-v3 -fwrapv`, matching bounded sizes from
  `tests/benchmark/callgrind_sizes.json`, including shared-source variants.
- Geometry: I1/D1 `32768,8,64`, LL `33554432,16,64`; branch simulation enabled.
- Whole-program simulated Ir geometric ratio LCCC/GCC12: **0.951402**.
  This is **not runtime, cycles, a measured speedup, or a GCC16 comparison**.
- Whole-program losses include byte reassembly **3.9381**, ring FIFO **2.6194**,
  ASCII folding **1.9889**, histogram **1.9067**, strstr **1.8970**.
  Do not replace these losses with an aggregate victory narrative.
- Own-object Ir differs materially from whole-program counts: libm rounding
  **1.0798**, double reduction **1.1268**, nbody **1.1927**, matmul **0.8577**,
  SHA256 **0.9632**. Library outsourcing and different algorithms can affect
  own-object comparisons; no own-object aggregate is used as a merge score.

A pinned GitHub source build made genuine Valgrind available after apt and
release-asset provisioning failed. Source commit:
`971c165fc4335bf04bedf58e0800a12776fce408`; version **3.16.0.GIT**, not a claim
of the latest release. GNU bootstrap tools needed modern-glibc compatibility
repairs; Valgrind itself was not given counter/ISA patches. This older version
warns about DWARF5 and unavailable rseq syscall 334. Both arms still passed
native/instrumented output parity. These warnings limit portability claims.

Compiler-internal Callgrind profiles use three repetitions on minmax/libm,
equal-length binary/output paths, pinned geometry, native/instrumented assembly
hash parity and explicit own-object selection. Debug info alone is stripped
from copied compiler binaries after verifying identical `.text` bytes.
Counters measure simulated compiler work, not compilation wall-clock speed.
Final comparisons are published when their processes complete.

## Fresh hosted baseline and improved publication

At `d9d0a2b`, Clippy, MSRV, CE delta, Debian exact-version corpus and native
benchmark checks succeeded; Test Suite was still pending when sampled.
The Debian annotations independently expose **45 compiled and matching cases**
and **GCC 16.2.0-3**. Report SHA256:
`23a8680edbc8a503e5c67b49aa159355272c638135d9da5a6d90e8b797a7118b`.
Artifact downloads redirect to a disallowed blob host, so status alone is not
being substituted for the available measured annotation data.

The baseline's Git identity was unavailable due to container ownership. The
workflow now authorizes exactly `$GITHUB_WORKSPACE` and checks HEAD against
`$GITHUB_SHA`, not a wildcard safe-directory exception. It also publishes a
separately labeled 45-case Callgrind screen with genuine Debian GCC16 and its
matching builtin headers. The CE job adds four-vendor canonical-registry static
measurements and a minmax loop-density survey, retaining raw artifacts and
bounded check annotations. Density is explicitly a heuristic: matching symbols
and requested flags do not prove equal hot-loop work or ISA/FP-mode equivalence.
Missing/unmeasurable densities are not described as wins.

New measurement commands are mirrored as **slow** local gates. `--fast` skips
those gates; **no full/slow local CI invocation was made**. Standalone bounded
profiling is performance research, not a full local CI run.

## Local acceptance status and remaining limits

- Four GiB swap is active; Rust/cargo/rustfmt/clippy **1.99.0** is installed.
- Formatter, CI/local parity and embedded workflow shell/Python parsing passed.
- Python corpus suite: **430 tests passed, four existing skips**. The diagnostic
  context fixture now supplies its isolated mock's version explicitly; this
  does not install a mock GCC or relax any production GCC16 requirement.
- Rebased native minmax differential passed at five optimization levels on
  baseline and AVX2 targets, including the I64 empty-limit/escaping-IV fixture.
  Shape census: **20 passed**, nine packed 256-bit/32-byte-trip loops at
  **0.125 insn/byte**, eleven refusal shapes. i686 was unavailable locally.
- The earlier fast attempt completed all-target tests before further edits.
  An intermediate follow-through run caught three positive-control regressions;
  those controls were retained and the effect distinctions corrected. The
  current-source fast run and current-head hosted runs must finish before final
  acceptance is asserted.
- **Local `ci_local.sh --fast` is not green:** required GCC16, i386 development
  objects and local Rust1.97 are unavailable. Do not alias GCC12 to GCC16, skip
  mandatory i386 legs, or weaken gates to manufacture acceptance.
- **F8 remains open:** the nbody displacement-only mass-load shape metric is not
  a fail-closed semantic field-access count. Runtime correctness and Callgrind
  do not repair that metric. The broader vectorizer plan/refactor and exhaustive
  swap-helper mocks are also not claimed complete.

This is a bounded, critically checked repair, not a perfection certificate or
an unconditional recommendation to merge. Fresh final-source results supersede
all earlier snapshots.
