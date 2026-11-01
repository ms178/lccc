# Unified PR audit — 2026-11-01

## Latest branch validation (2026-11-01; supersedes historical status below)

The unified changes were rebased onto `origin/main` at `9e93005b8` after main
absorbed the earlier PR commits. On the resulting pre-rebase-validation tree
`7fe29e478`, hosted CI run `41315532314` completed successfully, including
assertion-enabled units, all three frontend/ABI shards, both codegen/runtime
shards, assembler/linker contracts, both benchmark shards, and the aggregate
benchmark report. The Debian exact-version oracle (`41315532322`), relocation
contracts (`41315532407`), IPA/LRS (`41315532403`) and targeted research
(`41315532335`) also completed successfully. Targeted research's explicitly
optional patched-LLVM comparison was skipped, not passed. These runs are for
`7fe29e478`, before the latest-main rebase; the rebased result must be retested.

The hosted CI lane reports warning-denied builds, formatting, Clippy, MSRV,
GCC 16.2 / Clang 23.1.4 contracts and authenticated ICX 2026.2 integration.
Local `cargo test --offline --locked --profile fastbuild` separately passed
7,445 tests, with 10 ignored and 0 failures; it is not a replacement for the
hosted multilib/cross-target suites.


## Hosted-only validation update

At the user's direction, the local full-suite process was stopped before
completion; no local CI pass is claimed for this revision. Latest main
`734290ddb` was fetched and the merge-preserving rebase completed cleanly.
Compiler fixes were pushed as `26bb52730` to PR #1056. All further validation
and defect follow-up is delegated to GitHub Actions.

The last completed main CI run (`41308567453`) took 15 minutes: its build job
ran 3m32s, with substantial downstream queueing. The existing contract shards
already run independently; increasing shard count blindly would consume more
setup time and runner slots. Two low-risk efficiency changes preserve coverage:

- Superseded PR research runs now share a per-PR concurrency group and cancel,
  rather than continuing a 45-minute obsolete experiment per commit. Explicit
  manual measurement runs retain independent groups and are not canceled by PRs.
- The shared compiler artifact disables redundant upload compression because
  its payload is already gzip-compressed. Hash/commit verification is unchanged.

## Latest integration checkpoint (supersedes older revision status below)

- All five hosted workflows for `6d2b5c78a` completed successfully, including
  Targeted compiler research (`41308567423`). The frozen local fast suite at
  that revision passed **543 gates, 0 failed, 9 explicitly skipped**. These
  results do **not** validate the newer tree.
- Rebased with `--rebase-merges` onto upstream main
  `734290ddbb3c1556d7c7f4c0d30312d876486b5c`, which merged #1063.
  The existing inline, SLP, and x86 intrinsic work was preserved exactly.
- #1063 permits narrow integer bit-modify IR, but its ARM and RISC-V emitters
  reduced indices modulo register width instead of IR width. The independently
  reproduced `U8 BitSet(0, 8)` counterexample requires 1, not 0. The correction
  passes actual `IrType` into the ARM helpers, declines narrow register-direct
  emission before any write, reduces narrow dynamic indices explicitly, and
  derives the RISC-V mask from the actual IR width. Existing 32/64-bit
  instruction shapes are retained. IR documentation now names all accepted
  widths. New direct-emitter unit tests cover both operations and signed and
  unsigned 8/16/32/64-bit types. Rust 1.99.1 fastbuild with warnings denied:
  3 focused tests passed, then **7448 library tests passed / 0 failed / 10
  ignored**. GNU 2.47 plus QEMU 11.1.2 executed the actual captured emitter
  streams: **480 AArch64 and 80 RISC-V cases passed**, with relocation-free
  static ELF witnesses. Indices include 0, W-1, W, W+1 and maximum unsigned
  values; ARM additionally exercises immediate 4095. Results are compared at
  the IR width, not as a claim about upper-register normalization.
  A final test-only assertion tightening (matching the whole immediate shift,
  not an ambiguous suffix) is included in the next full-suite run.
- The rebase exposed an overwritten census literal count: upstream introduced
  `CCC_NO_BIT_SELECT`, so the measured count is 551, not 550. The documented
  baseline is restored to 551; the other census metrics do not grow.
- Re-read updated open heads #1060 (`5b68bdae5`) and #1062 (`0994c3e23`).
  The wider-constant splat optimization is already present here, including
  stricter lane-width and truncated zero/all-ones handling. This branch already
  refuses both register-preservation contracts. Do not copy their broad SIMD
  literal fallback over the consumer-specific safety guards. Do not silently
  accept `force_align_arg_pointer` on the assertion that frame offsets are
  aligned: that does not establish alignment for a misaligned incoming stack.
  Existing explicit refusal is retained until realignment is implemented.
- The first new test build exhausted the filesystem (8.3 GB of incremental
  caches). Only disposable `target/fastbuild/incremental` contents were removed;
  no sources, toolchains, snapshots, or benchmark evidence were removed. A
  fresh serial modern-toolchain build succeeded. The external raw-ELF harness
  also needed executable file permissions; after fixing that harness, both
  actual-emitter QEMU runs passed. Full fast-suite and hosted validation of the
  updated revision remain required. No production-readiness claim or
  current-revision CI success is asserted.

## Scope and acceptance status (historical integration record)

The requested consolidation now covers #1050 (`a62b7d708`), #1051 (updated to
`130f9e0cb`), #1060 (`d26a1ab63`), and #1056, on `arena/b673aa4d-lccc`. All three other heads are ancestors
of this branch. `git fetch origin main` and `git rebase --rebase-merges
origin/main` were run against main `013983d6a`. The renewed all-open-PR request adds the newly opened #1060; its
modern-toolchain ABI review is recorded below.

**This is an in-progress audit, not a production-readiness certificate.** The
previous #1056 hosted run was red. Full local gates and a new hosted run are
required before acceptance. Historical benchmark and component reports are
not evidence that every program on the unified tree is correct or faster.
No independent expert approval is asserted.

## Defects exposed by integration and adversarial verification

### 1. Literal SIMD lanes were treated as addresses on the SSE2 multiply path

Generic literal-lane SLP eligibility was unsound: the SSE2 dword multiply
consumer interprets a non-homed vector argument as a memory address, whereas
some other consumers know how to synthesize constants. The real SHA-256
regression faulted while dereferencing the literal 1664525. The fix admits
literals by **consumer capability**, retains the all-ones compare idiom, and
does not pretend that an unsupported literal operand is a spill address.

`slp_dword_constants.c` and `check_slp_dword_constants.sh` exercise hoisted
loads, require the intended vector instruction, compare outputs, and retain
the no-SLP arm. An earlier fixture was rejected as vacuous and a helper named
like a compiler builtin was renamed rather than relying on builtin resolution.
The warning-denied build, constant gate, BB-SLP v5 and reassociation gates pass.
SHA-256 checksum: `054db5f638d89d8b` on baseline x86-64 and v3.

### 2. Inliner ABI homes need provenance and shape, not alloca order

The merged inliner resolves homes by parameter identity/data flow, validates
ABI shape, and allocates collision-free fresh IDs. A same-typed leading local
is not identity proof. Tests cover a leading local, mismatched ABI-home shape,
and stale `next_value_id`. The ABI differential gate passes 12 rows
(O1/O2/O3/Os across i128 switches, Float128 conversions and SSE1-only vec8),
with an explicit assertion that the i128 parameter is actually inlined.

A tentative aggregate-ParamRef change was withdrawn: the crashing `bits` and
`mbits` helpers contain **no ParamRef** in their initial IR. Its small runtime
probe also passed on the old compiler, so it was not credited as a regression
or used to justify a speculative production change.

### 3. Destructive phi coalescing assigned a register to an unmaterialized Alloca

The real `vector_small_packed_wrappers_m32.c` failed with SIGSEGV at O2, while
GCC 16.2 and the inline-disabled build printed
`small vector packed wrappers a23f76a3732b5dd3`. Disabling assembly peepholes did
not repair it. Skipping `bits,mbits` or disabling phi coalescing did repair it;
skipping only `mix` or the MMX wrappers did not.

A ptrace breakpoint on `ret` established that call 1 had a valid sret buffer,
but call 2 passed zero. The false predecessor passed EBP before any defining
LEA; the return-buffer LEA appeared only after the call. The callee's `ret $4`
and the caller's reserve-32/add-28 stack cleanup were correct, so changing
cleanup would have attacked the wrong boundary.

`Alloca` produces a rematerializable frame address, **not an emitted register
definition**. Phi-home inheritance bypassed ordinary eligibility and gave
that address the phi's register. Uses before the latch copy read an
uninitialized register. The shared candidate detector now rejects an actual
Alloca source definition. It does not disable phi coalescing globally or
change ordinary memory operand handling. A materialized Copy of an Alloca
remains eligible; the Part-2 chain's actual source definition, rather than its
underlying root, is what matters.

Rust tests pin both the refusal and materialized-pointer positive controls.
The rebuilt compiler passes the real m32 fixture with the GCC checksum above.
The diagnostic trace harness also passes. This is a shared allocator fix,
not an i686-only operand-order workaround; cross-target execution is still a
separate validation obligation.

### 4. Merged simplify tests had an obsolete call signature

Nine test/wrapper calls lacked the thirteenth argument of
`try_simplify_with_types`. The repairs were compiled in the first full unified
library run: **7,426 passed, 10 ignored, 0 failed**. The latest #1051 correction
was merged as well; its overlapping repairs produced no duplicate arguments.

### 5. Bounded SIMD metadata validation was duplicated and truncated I128

Four new internal-control decoders duplicated constant/range handling.
`IrConst::to_i64` truncates I128, allowing a malformed value with high bits set
to masquerade as a valid shuffle/control byte. A shared checked decoder now
converts through I128 and checked I64 conversion before enforcing the exact
range. Missing operands, SSA values, floating constants, out-of-range values
and high-bit I128 aliases must fail closed. Unit tests enumerate all valid
controls and those invalid classes. This does not weaken target-feature or
operand-count assertions.

Deduplication removes seven production panic/expect sites. Relative to the
fetched main census, production panic sites fall **881 → 877**, hot panic
sites stay 101, unsafe sites stay 25, long functions (>300 lines) fall
233 → 232, unique knobs are 550 after the new ABI escape hatch and undocumented knobs fall 264 → 262.
The integrated baseline is regenerated against that upstream baseline;
there is no ratchet increase relative to main. The internal tail-call marker
is no longer mislabeled a user configuration knob.

## Validation receipts so far

All compiler builds use the fastbuild profile, two Cargo jobs, warnings denied,
and the active 4 GiB swap. Heavy builds are serial.

- Warning-denied compiler/linker build: PASS.
- SLP literal constants, BB-SLP v5, latency reassociation: PASS.
- Real i686 small packed-wrapper fixture: PASS after the allocator fix.
- Inliner ABI/home gate (12 differential rows plus actual-inline coverage): PASS.
- PIE executable/ET_DYN and struct-by-value execution gate: PASS.
- LightPixel and joint-pixel differentials: each 6/6 configurations PASS
  (SSE2/SSE4.1/AVX2 × O2/O3).
- First unified Rust library run: 7,426 passed / 10 ignored / 0 failed.
- Warning-denied all-target Clippy: PASS.
- Python SIMD harness suite: 22/22 PASS.
- Census, documentation links and generated CI lane-plan checks: PASS.

These receipts are staged validation, not a claim that a later tree edit was
already exercised. The subsequent complete local run must validate the final
materialized-copy positive controls and bounded-control tests on one unchanged
tree. Hosted checks, complete regression/corpus results, cross execution and
fresh unified timing receipts remain outstanding at the time of this note.

## Remaining boundaries and limits

The follow-up documents remain evidence inventories, not completed checklists.
In particular, exact Chromium ClampTo8 fastguard recognition, a build of the
comparison LLVM PR, broad browser relevance, external ICX execution and
architecture-specific runtime coverage must not be inferred from an integer
pixel microbenchmark. The existing codegen-mode follow-up records its landed
non-PIC fix and its GCC/kill-switch runtime regression separately from its
remaining profitability questions. No unsupported memory-width, alignment,
strict-FP, overflow or whole-browser performance claim is made here.

## Renewed all-open-PR audit: modern ABI evidence

The main compiler and all comparison oracles use the hash-cache's Rust 1.99.1,
GCC 16.2, Clang 23.1.4 and binutils 2.47. Unversioned C/C++ compiler commands
are explicitly routed to those versions. No performance conclusion uses the
system GCC 12 or the separately declared Rust compatibility floor.

The initial #1060 table was based on GCC 12. GCC 16.2 honors
`no_callee_saved_registers` and `preserve_none` on both x86 targets. On x86-64,
`preserve_none` also changes integer argument registers (R12/R13). Ignoring
these attributes is unsafe in an lccc caller of a GCC callee, even if an lccc
callee preserves more registers than required. The table now refuses them
until function types and call-clobber lowering implement those contracts.

The oracle now compiles identically named functions in separate translation
units; differently named twins permitted incidental register choices to look
like ABI differences. An explicit EBX/ESI/EDI clobber probe exposes register
preservation independently of arithmetic register pressure. It also proves
that `force_align_arg_pointer` can affect a real x86-64 prologue; it cannot be
classified as universally inert. All 30 table cells match modern GCC after
these changes. Malformed ABI argument lists are tested in both GNU syntaxes
and word sizes; an initially missed C23 `regparm(1,2)` case was detected and
corrected before acceptance.

`CCC_ALLOW_UNSUPPORTED_ABI` is the explicit, warning-producing compatibility
escape introduced by #1060. Presence enables it; absence (the default) refuses
unsupported ABI/layout declarations. It is **not** an interoperability mode or
a production-safety guarantee. Objects built under it must not rely on the
ignored foreign calling convention, preservation contract, layout or stack
realignment promise, including assembly and external-library boundaries.
This new, documented switch adds one literal knob, not an undocumented one.

Additional integration evidence before adding #1060: SSA validation corpus
**1,326 passed, 0 failed, 3 skipped comparisons, 0 skipped executions**;
AArch64/RV64 freestanding execution **52/52 passed**; real i686 packed-wrapper
matrix **15/15 passed** across O0/O1/O2/O3/Os, default/no-phi/no-inline.
The skipped GCC floor-float comparison was traced to a missing `-lm` sidecar
and repaired; it must be re-run, not silently counted as a comparison pass.

The first complete-local attempt exposed two environment/header boundaries:
Memcheck needed the matching cached libc debug package (installed and its
startup probe repaired), while i686 Linux UAPI headers need Debian's shared
multiarch `asm/` tree. The latter is a compiler search-path defect, not a
reason to install random headers or weaken the trapv test. The fix admits the
shared tree only when its explicit `gnu/stubs-32.h` support marker exists
inside the selected sysroot, retains i386 precedence, and adds a real UAPI
width regression.

Local `ci_local.sh --fast --skip-msrv` selects the cached modern toolchain
suite without invoking the older compatibility-floor compiler. This selection
never emits a full/fast acceptance stamp. The default local runner and hosted
MSRV job are unchanged; hosted CI must still pass before delivery. Gate
selection tests prove that the compiler-contract gates remain selected.


## Follow-through: corrected tests and refreshed PR heads

The audit refreshed #1060 through `07992c547`; #1050 `a62b7d708` and
#1051 `130f9e0cb` are also ancestors of the unified branch. Main remains
`013983d6a` at the latest fetch. The following are measured integration fixes,
not assertions of universal completion:

- All-target Rust: **7,429 library tests + 1 linker-binary test PASS**, 10
  intentionally ignored. The materialized-Alloca-copy positive control had
  replaced instruction 2 (removing Copy7) rather than instruction 1 (BinOp6).
  The fixture now asserts the replaced destination and retains both the
  positive coalescing expectation and the Alloca refusal control.
  Warning-denied all-target Clippy also passes.
- The independent shell attribute gate now requires preservation-convention
  refusals, rather than its obsolete ignored-attribute expectations. Escape
  hatch assertions remain. `regparm(3)` on x86-64 is explicitly ignored.
- The cached modern GCC does not ship libubsan. The RA harness uses GCC's
  native trap instrumentation, **not an older runtime or a sanitizer skip**.
  All 20 generated kernels pass GCC O0/O1/O2/O3 and sanitized O2. Intentional
  signed-overflow and float-cast-overflow controls must terminate with SIGILL.
- Sanitizing the call-staging reference exposed real fixture UB: negative
  signed left shifts and overflowing signed hash postprocessing. Negative
  constants are now constructed with representable multiplication/negation;
  variable wide-bit assembly uses an unsigned carrier; hash postprocessing
  uses modular unsigned arithmetic. Both reference translation units are now
  instrumented. **35/35 target builds and 10,825 ABI call comparisons PASS**
  (pinned cases + six 120-function seeds, O0/O1/O2/O3/Os).
- The hosted fast-contract shard caught negative frontend tests masquerading
  as accepted IR fixtures. The latest #1060 supplies the purpose-built
  bit-field IR fixture. The unsupported preserve-none fixture is replaced
  with accepted native direct/forwarded/indirect calls; foreign-ABI refusal
  remains independently pinned. **78-file abort-mode IR sweep PASS**; the
  new native fixture also passes GCC/LCCC, m32/m64, O0/O2 with
  `-Wall -Wextra -Werror` (**eight compile/execute pairs**).
- Historical simple `x+x` vector probes emit one packed add plus return;
  the offset-memset probe matches GCC 16.2 at four instructions. These limited
  non-reproductions do not justify speculative backend changes or close all
  context-sensitive profitability questions.
- The ICX hosted lane now provisions multilib before the exact GCC installer,
  installs GCC 16.2 and Clang 23.1.4, retains the Clang installer's exported
  executable, and verifies the pair before selecting unversioned aliases.

The first full modern-selected local run started before these test fixes; it
records failures for the obsolete shell expectations and missing libubsan.
Those gates pass independently after correction. An unchanged-tree clean
rerun, fresh unified timing, current-head hosted CI, and the remaining browser
and follow-up work are **not yet certified** by the receipts above. Focused
validated `ms178-1.patch` snapshots 04 through 08 record their exact commits
and these limitations rather than manufacturing a canonical local pass stamp.

## Harness and v6 fixture repair receipts (2026-11-01)

- Complete corpus rerun: 1,097 tests, six skips, 107.423 seconds, PASS;
  external log `/home/user/unified-corpus-contracts-v6.log`. The subsequent
  strengthened v6 assembly assertion and negative control also pass.
- Callgrind object IDs share the `ob`/`cob` namespace, but only `ob` changes
  current object context. The streaming parser now handles either definition
  order, exact integer counts and malformed/missing-count errors. Five
  synthetic tests pass. The real layout gate counts 96,397,664 selected-layout
  versus 96,973,245 historical-layout transfers, with matching output and
  13 static jumps each (the previous zero count was a parser defect).
- Source ratchets enumerate Git's source set rather than recursively scanning
  ignored source archives. CI expectation tests include the actual ICX and
  optional LLVM-builder contracts. Hosted/local parity passes: 493 commands,
  27 invocation contracts. Ordinary compiler-research evidence is unconditional;
  the expensive patched-LLVM comparison remains opt-in.
- The v6 C fixture had actual signed overflow in add, subtract, add8 and the
  equality-select false arm, despite claiming defined wraparound. Those kernels
  now use unsigned arithmetic. The XOR expected-value expression also lacked
  parentheses and could accept incorrect results; a deliberately zeroed XOR
  lane now fails the repaired check. GCC 16.2 compiles with `-Werror`, and the
  complete fixture passes undefined-behavior/float-cast-overflow trap sanitizers.
  This sanitizer oracle is now part of the regression gate.
- The apparent constant-add stream-fold regression was an over-specific shape
  expectation: literal pooling permits folding either memory input. Retain the
  original literal fixture and require packed memory arithmetic in at most four
  instructions (including return). Add a noinline runtime-broadcast fixture
  that must fold the input stream, with no separate vector load; it passes
  1,000 constants across four edge-value lanes. Both strict assembly contracts
  pass. GCC 16.2's corresponding literal function uses five instructions at
  `-O2 -march=x86-64-v3`; this is an instruction-count observation, not a runtime
  speedup claim. No compiler optimizer change was necessary for this gate.
- Offline environment provisioning: the Arm September-2026 archive was verified
  and installed through its official installer; kernel relocs v7.3 was rebuilt
  through its official source-pin verification. Modern GCC's local m32 wrapper
  now adds the shared Debian UAPI include only without explicit sysroot or
  nostdinc policies, honoring the last ABI switch. Its genuine GCC 16 lib32
  runtime is selected, and the throw/catch C++ probe runs successfully.

Outstanding: the full modern fast run remains 517 pass / 18 fail / 16 skip
before these repairs; no frozen clean rerun or green hosted acceptance yet.
GNU tool provisioning is still in progress (Texinfo documentation generation is
unavailable locally; `MAKEINFO=true` suppresses manuals, not tool validation).
The open-PR refresh additionally found #1061 and #1062; their unique changes
still require audit/integration without deleting retained improvements. Latest
main remains `013983d6a0c94d9cabb700aa8fdde5c3cfa16e88`. Actual patched-LLVM
comparison and fresh whole-computation runtime measurements remain unrun.

## Open-PR refresh and measured follow-up (2026-11-01)

Reviewed #1050/#1051/#1060 plus newly opened #1061/#1062 at fetched heads
`a62b7d708`, `130f9e0cb`, `e7d7de9af`, `66f5d7088`, `673baafc6`.
The older-base trees must not replace this unified tree: their inverse diffs
remove retained ABI safety, literal-consumer whitelisting, i686 alloca address
handling and regression coverage. The i686 sret-coalescing work in #1061 is
already present; its unique source-span cleanup repair is now integrated.
The unique wider-constant splat optimization in #1062 is adapted without moving
or deleting the retained literal-operand emitter. Narrower integer constants
still decline the fast path (generic sign extension); wider ones truncate to
consuming lane width. Additional zero/all-ones truncation cases use register
idioms instead of pool reads. Two table-driven unit tests cover lane widths
1/2/4/8 and vector widths 16/32. The additional #1060 diagnostic-trap cleanup
fix is integrated; its SIGPIPE concern was already fixed with a draining grep.

Validation receipts:
- Rust 1.99.1 fastbuild: 7,432 library tests PASS, ten ignored; linker unit
  PASS. Clippy `--all-targets -- -D warnings` and fresh fastbuild binaries PASS.
  Log: `/home/user/unified-rust-pr1062-serial.log`.
- The first two-job attempt exhausted available swap; it was stopped, not
  counted as a pass. Serial validation completed successfully with the 4 GiB
  swap active. No legacy compiler was substituted.
- SSE2/SSE4.1/AVX2 dword-constant runtime/codegen at O2/O3 PASS (six pairs).
  The v6 gate including the sanitized modern GCC oracle PASS. The original
  i686 narrow-output fixture compiles under `CCC_VERIFY_IR=1 -O2 -g`.
- Both complete LightPixel and joint-pixel differential/legality suites PASS
  at O2/O3 across SSE2, SSE4.1 and AVX2. All benchmark hashes agree with the
  independent O0 reference.
- 1,617 diagnostic red-team contracts and exact GCC 16.2 severity/column parity
  PASS; 315 scripts pass the pipefail/SIGPIPE audit. Invocation parity remains
  493 commands / 27 contracts. All four failure-annotation unit tests PASS.
- Native and AArch64 binutils 2.47 now pass their official provisioner fast
  paths with valid source/binary provenance, and kernel relocs v7.3 passes its
  official verified-source stamp fast path. `MAKEFLAGS=MAKEINFO=true` was
  needed to suppress manual regeneration; plain MAKEINFO was not sufficient.

Fresh 12-round, single-CPU, order-balanced medians are recorded in
`engineering/unified-pixel-summary-2026-11-01.json`. The by-value kernel takes
3.724 ms with LCCC versus 4.853 ms GCC 16.2 / 4.672 ms Clang 23.1.4.
The separate-buffer kernel takes 3.640 ms versus 3.823 ms GCC / 3.380 ms Clang:
LCCC LOSES to Clang on that measurement. The original source's complete
compute/reduce/store is measured, not merely its packed arithmetic. The emitted
vector path has a longer widening/multiply/narrow/reduction dependency chain
than Clang's scalar path; no unsupported narrowing, out-of-bounds load or
floating conversion was introduced to manufacture a win. These data disprove a
universal significant-superiority claim; tiny min/max differences are not
claimed as meaningful wins. No ICX or patched-LLVM timing is claimed locally.

Full frozen modern fast-suite rerun and current-head hosted acceptance remain
required; earlier hosted results do not certify these additional changes.

## Frozen-run result, hosted green milestone and final harness follow-up

At source milestone `9327e12a1`, all five hosted workflows completed successfully:
CI `41304114851`, research `41304114858`, Debian `41304114833`, relocation
`41304114816`, IPA/LRS `41304114904`. Main CI includes authenticated ICX 2026.2
integration, all frontend/ABI/codegen/runtime shards and assembler/linker tests.
The expensive opt-in patched-LLVM job was skipped, NOT passed. The research
artifact is ID `13083673184` (`compiler-research`, 8,087,395 bytes); local download
failed at the external artifact-storage host, so no unseen numeric ICX timing
is asserted here.

The frozen modern-selected local fast run completed **542 PASS / 1 FAIL /
9 SKIP** (`/home/user/unified-ci-frozen-pr1062.log`). The only failing test was
`i386_dso_emit_semantics`: its fixture replaced `LD_LIBRARY_PATH`, hiding the
cache-selected GCC 16 lib32 C++ runtime before the exception test executed.
The two DSO semantic batteries now prepend their own directory while preserving
explicit inherited compiler-runtime paths. Two additional unit contracts pin
fixture-first order, environment isolation and no implicit cwd for unset/empty
paths. Actual strict full linker suite after correction: **341 PASS / 0 FAIL /
0 WARN / 0 SKIP** (`/home/user/unified-linker-final.log`), with BFD and mold
oracles. No leg was disabled and no exception assertion was relaxed.

The latest #1060 startup-annotation concern is also addressed: one EXIT reporter
is installed before compiler discovery and temp setup. Missing-compiler and
failed-mktemp negative controls preserve exit 1 and 7 and emit annotations.
All six annotation contracts pass; 1,617 real GCC 16.2 diagnostic red-team
contracts still pass. Full corpus after these harness changes: **1,099 tests
PASS / six skips** (`/home/user/unified-corpus-final-1099.log`). The earlier
frozen whole-tree run is not relabeled as a clean run of these changes.

Paired-round bootstrap of the committed pixel timings (10,000 resamples,
seed 63122730) gives a by-value elapsed-time change interval of -24.04% to
-21.89% versus GCC and -21.16% to -18.30% versus Clang. The separate-buffer
loss versus Clang persists at +5.08% to +9.91%. These intervals describe only
the 12 measured rounds, not systematic cloud-host effects.

PR #1063 opened during the frozen run, at head `1a78be8ae`. Its new producer
accepts I8/U8/I16/U16, but the AArch64 and RV64 bit-modify emitters reduce indices
only modulo 32/64. Independently assembling the exact isolated emitter sequence
and executing under QEMU confirms `BitSet(U8(0),8)` yields 0 rather than the
stated IR result 1, on BOTH targets. This is an emitter/model counterexample,
not a claim that the entire PR was built. Review posted at
https://github.com/ms178/lccc/pull/1063#issuecomment-6557857349 . Do not import
that uncorrected feature or discard its valuable tests to manufacture acceptance.
Its corrected integration and the remaining Chromium/LLVM work are outstanding;
there is no universal-superiority or exhaustive-production-certification claim.
## Final focused review and local evidence (2026-11-01)

### #1056 / #1062 compared with #1051

The review is against the fetched PR head snapshots, not author descriptions.
The retained #1056 SLP changes preserve the prior literal-operand emitter and
its refusal cases; the #1051 baseline remains the comparison point for the
pre-existing optimizer/ABI improvements. #1062's wider-constant splat change
is not retained: all eight workload shapes produced identical output hashes,
but its candidate-vs-#1056 24-round medians varied in both directions. On this
single pinned virtual CPU (`-O2 -mavx2`), the candidate was +0.39% on the
separate-buffer kernel, +0.25% on the by-value kernel, +0.73% on four-lane
wide arithmetic, +0.34%/+0.85% on four-lane unsigned min/max, +0.38% on
8-lane wide arithmetic, +0.37% on 8-lane unsigned min, and -0.74% on 8-lane
unsigned max. Those sub-percent, mixed-direction observations do not establish
a speedup or regression; the optimization was reverted rather than promoted
from codegen plausibility alone. Raw samples and compiler identities are in
`/tmp/lightpixel-splat-comparison.json` (ephemeral local evidence, not a
repository artifact).

The focused ABI comparison remeasured 30 convention/target cells against GCC
16.2, including caller-visible preservation, argument assignment, stack
realignment, and return behavior; all supported cells passed. Unsupported
foreign ABI contracts remain refused by default, with the documented explicit
compatibility escape warning-producing. The `CCC_ALLOW_UNSUPPORTED_ABI` knob
accounts for the single corresponding census literal increase. Correctness
probes and the census do not imply interoperability for ignored conventions.

### Current-tree checks and boundaries

- `cargo test --offline --locked --profile fastbuild`: **7,445 passed, 10
  ignored, 0 failed**.
- `python3 tests/regression/abi_attribute_matrix.py`: **30/30 GCC 16.2 cells
  passed**; `python3 scripts/audit_census.py --check` and `cargo fmt --all --
  --check`: PASS.
- Targeted SLP dword-constant, BB-SLP, LightPixel, joint-pixel and AoS-pair
  codegen/runtime checks passed in earlier focused runs. These are correctness
  receipts, not performance claims.
- The local broad CI attempt is **incomplete, not green**: `empty-aggregate-abi`
  and `vector-abi-boundary` passed, then `aggregate-call-abi` could not find
  i686 libc headers because the invocation lacked `LCCC_SYSROOT`; the run was
  stopped during `sysv-aggregate-abi`. This environment has no complete system
  32-bit runtime setup. Do not count this as a compiler regression or as an
  acceptance pass.
- `gh pr view 1056` reports PR #1056 open against `main`, but its head is
  `arena/b673aa4d-lccc` at `d8ded5e9`, not this working branch. Its retrieved
  checks are successful on that separate head; they do not certify this tree.
  Current-head hosted checks and a clean broad local run remain outstanding.
