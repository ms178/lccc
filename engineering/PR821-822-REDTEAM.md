# PR #821 / #822 — executable red-team audit, 2026-10-14

## Scope and source identity

Start: `984960ae82cfc596eacef46ea614fe2a5e221c7a` (both PRs merged).
After the owner's upstream-update request: rebased/restored onto
`3ff36e8592b0d0e26645bf37d6685395b1a6eca5`. PR #823 already contains our
F5 fix byte-for-byte; reverse patch checking proved this. Do not reapply it.
The final delivery's metadata identifies the actual base/head/tree and CI status.

This is a bounded correctness/integration repair, not a claim that the compiler
has been audited in its entirety or that it outperforms all competitors.
`ci_local.sh --fast` only is authorized locally; no full/slow CI was requested.
Builds use fastbuild (O1, no LTO). The build helper fixes Cargo jobs at two;
the existing local test helper independently reduces its cold unit-test build
parallelism on hosts below 6 GiB. A real 4 GiB swapfile was activated and used;
another 4 GiB was enabled before final validation (8 GiB total). The final run
explicitly sets DEBUG=0, INCREMENTAL=0 and two jobs to bound build memory.

## Review decisions and additional findings

| Finding | Assessment | Change / evidence |
|---|---|---|
| F1, missing AArch64 user-asm markers | Agree, reproduced in all three real C paths before editing | AArch64 extended asm overrides the existing marker hook; naked and file-scope emission use a target hook. Only user text is bracketed. Setup/writeback stay outside. |
| F2, visibility versus dynsym | Agree, but do not claim a wholesale visibility fix | Remove the name-only side set and limit **shared ADR** to local allocated section-relative symbols. Hidden/protected globals now diagnose in either TU order. Other relocation kinds retain their contracts. |
| F3, LOCAL+UNDEF/symbol zero | Confirmed at linker level, not merely by an arithmetic model | Native link originally succeeded; independently decoded target becomes B instead of zero. Checked resolution now requires actual object/section mapping and rejects UNDEF, reserved indices, nonallocated and TLS sections. Zero-valued mapped addresses remain valid. |
| F4, optimized Python/environment | Agree; retained unchanged | Existing optimized-import and scope-mutation tests are retained. |
| F5, optional hosted execution | Agree; already upstream via PR #823 | Hosted `--run`, local compiler-only capability, exact per-side contracts and missing-QEMU-before-compilation test retained. |
| Additional: FP input register homes | New test exposed a real panic, even for ordinary `double` input | Emit correctly sized FMOV from authoritative GP/FP register homes instead of requiring a stack slot. |
| Additional: 128-bit asm input home | New QEMU test returned 4 at O0: high lane was loaded from the pointer's spill slot | Distinguish pointer-to-vector from direct, scalar-I128 and vector homes; dereference pointer homes. Preserve explicitly bound x17 across temporary address materialization. |
| Additional: i386 PLT-only dynamic table | Fast CI with readelf 2.47 exposed missing DT_RELENT | Emit one DT_RELENT=8 whenever either REL or JMPREL exists. Assert presence/value over all 512 descriptor combinations; inspect ELF tags directly and execute both ABI modes. |
| Additional: PR822 JUnit producer model | Disagree with the completeness of the old 44-test evidence | A **real CTest 3.31.6** report uses `status="fail"` and separate disabled/runtime-skip counters. The parser now accepts the genuine report without allowing failures/skips to become success. 49 compiler-free checks, including negative mutations; same outcome under `python -O`. |

The review's restriction on `status="run"` plus `<skipped/>` is **not** relaxed:
our measured producer emits runtime skip as `notrun`. That was a good request
for producer evidence, not a reason to guess. Unknown/contradictory outcomes
still fail. The captured XML and its replaying CTest input are checked in next
to `junit_check.py`; temporary build paths in the capture are not dependencies.

The review's historic network/toolchain limitations do not apply to this VM.
Official Rust 1.99.0 was installed; GCC 16.2.0-3 was obtained from Debian sid.
No tool is relabelled to satisfy an oracle pin. GNU AArch64 ld also accepts the
symbol-zero ADR fixture, so its exit status alone is **not** the correctness
oracle: decoded instruction semantics and load-bias cancellation are the test.

## Design and red-team considerations

* Do not disable LICM, IV widening, MachInst or constant folding. Main's
  function-wide constant/indexed GEP admission gate is retained.
* No generic "function has inline asm" exemption. The new real-C tests check
  ordinary/naked/file-scope user SIMD, FP code before/after it in a TU, FP code
  in the same function, input setup and output writeback. Native assembly and
  available GNU AArch64 assembly both consume the comment syntax.
* FP input home tests observe raw F32/F64/integer bits, including negative zero,
  and both halves of a vector. Register width matters: F32 uses S/W, F64 D/X;
  a 128-bit move uses the whole SIMD register, not its D view.
* The new temporary-address path must not overwrite a fixed `x17` operand.
  The common input-loading interface supplies the operand set; other targets
  ignore it and retain their existing behavior. Only the explicitly occupied
  scratch case needs a balanced, 16-byte-aligned spill/restore.
* Shared ADR resolution is intentionally narrow. It returns `Option<u64>`,
  not a guessed address plus a name-set predicate; checks additions; validates
  output indices; retains object identity. No global change to NONE or other
  relocations. Error tests preserve an existing output sentinel.
* Full AArch64 ELF visibility, interposition and GOT/PLT binding remain separate
  debt. This patch neither exports a hidden symbol correctly in all modes nor
  claims that it does. It prevents that defect from becoming the proof for the
  newly admitted shared-ADR cases.
* JUnit failures remain failures even if the reference fails identically. A
  parser acceptance test is not a passing workload test.

## Measured code-quality screen (not physical speed)

Existing `godbolt.py` and `codegen_oracle.py` were read and used, rather than
adding another HTTP client. The audit resolves GCC 16.2, Clang 23.1.0, ICC
2021.10.0 and the ICX latest channel. ICX's raw metadata is in the manifest.
ICC rejects `-march=x86-64-v3`; the initial run correctly failed rather than
silently ignoring a warning. A second run showed that LCCC rejects the old
`core-avx2` spelling. The successful survey uses `-march=core-avx2` remotely
and its Haswell target spelling locally, with both flag sets recorded.

Static whole-translation-unit instruction counts, `-O2`, **not runtime**:

| Extracted workload | LCCC | GCC 16.2 | Clang 23.1 | ICC | ICX | Fewest static instructions |
|---|---:|---:|---:|---:|---:|---|
| gzip CRC32 | 73 | 50 | 82 | 60 | 100 | GCC |
| zlib-ng Adler32 | 276 | 156 | 148 | 244 | 147 | ICX |
| Expat XML scan | 221 | 172 | 222 | 322 | 327 | GCC |

All are the existing licensed corpus extracts, not whole applications. The
static total includes harness code and cannot establish which kernel is faster.
LCCC does **not** beat the best compiler on these static totals. In particular,
Adler's stack-reference count (15 versus ICX's 3) warrants a pressure/unrolling
investigation; it is not by itself a measured hot-path spill count.

`callgrind_corpus.py` uses bounded checked-in problem sizes and identical
`-O2 -march=x86-64-v3 -fwrapv` workload flags against local GCC 16.2. Program
output is checked before and during instrumentation. Model geometry is fixed
and recorded; this Xeon VM is **not** the owner's Raptor Lake target.

| Workload | LCCC simulated Ir | GCC simulated Ir | LCCC/GCC |
|---|---:|---:|---:|
| gzip CRC32 | 802020 | 809055 | 0.991305 |
| zlib-ng Adler32 | 464853 | 506189 | 0.918339 |
| Expat XML scan | 1667730 | 1435301 | 1.161937 |

Arithmetic mean of the three Ir ratios: **1.023860**; geometric mean: **1.018899**.
Best: **0.918339** (Adler); worst: **1.161937** (Expat). No unchanged workload.

These are whole-program **simulated instructions**, including startup/libc;
no percentages here are claimed as speedups. Expat is the worst remaining
screened gap, while zlib has fewer dynamic instructions despite more static
instructions. That directly argues against optimizing solely for assembly size.
All six candidate/reference binaries also completed Memcheck with zero errors.
This is a competition baseline, not an A/B claim for the ARM correctness patch.

Two later replays of each identical executable agree exactly with each other,
but each has 155 more Ir than the original invocation (whole-process context
matters). Their ranges, not a fabricated zero-variance speedup, are retained in
`repeated-measurements.tar.gz`. The hot gzip kernel's self Ir remains 524410.
A separate Clang 23.1.3 comparison gives LCCC/Clang Ir ratios 1.327083 (gzip),
1.035666 (Adler), 1.218094 (Expat). Thus a GCC-only "win" is not a win against
the competition set. No local ICC/ICX runtime measurement was made.

Function attribution sharpens the Expat hypothesis: LCCC's UTF-8 helper takes
676176 self Ir versus GCC's 690384, while `main` takes 845242 versus 591483;
the larger deficit is outside the helper, not proof its decoder is inferior.
The assembly shows LCCC's loop hash repeatedly homed at `40(%rsp)`, whereas
GCC keeps loop state and the multiplication constant in registers across the
private helper call. Investigate loop-phi homes and internal-call clobber
summaries before wholesale vectorization or changing the decoder. Clang inlines
the helper; GCC does not, so "always inline" is not a supported conclusion.

All **58 benchmark program assemblies** are byte-identical between the frozen
measurement compiler and the later input-home repair build, under the same
O2/v3 flags (`codegen-identity.json`). This is an x86 nonregression control,
not proof that compiler binaries or untested targets are identical.

Raw counter profiles, per-case command/hash manifests and all oracle assembly
are in `evidence/pr821-822-redteam/measurements.tar.gz`. Failed oracle attempts
are retained as receipts, not mixed into the successful table.

## Real workloads and tool provisioning

Expat 2.8.5: matching GCC16/LCCC CTest identity/configuration, one aggregate
upstream test in each build, plus all canonical and rejection XML cases passed.
zlib-ng 2.3.3 with the pinned archpkgbuilds patch: **69/69 CTest tests** and
compression roundtrip passed, including maketrees. These used the frozen x86
compiler identified in their manifests, not an unsupported ARM speed claim.
`workloads.tar.gz` retains their logs, versions, flags, feature configurations
and output-hash checks; generated executables are deliberately not in the patch.

The newer readelf also exposed a real i386 metadata defect: a PLT-only dynamic
executable lacked `DT_RELENT`. Replacing only the old output's optional DT_DEBUG
slot with DT_RELENT=8 made the previously hidden JUMP_SLOTs readable, isolating
the cause from the symbol-routing code. The descriptor fix is shared by i386
executable/DSO emission. The existing runtime gate plus direct ELF tag assertions
passes with both modes executed, not skipped.

The initial fast-CI run encountered missing GCC16/multilib. After GCC16 install,
the old BFD 2.44 plus new glibc 2.43 produced crashing i386 dynamic executables,
including `int main(void){return 0;}` with both GCC14 and GCC16. Static execution
worked. Upgrading the **real** system BFD to 2.47 made freshly linked dynamic
PIE and non-PIE controls execute normally. No compiler skip or version wrapper
was added to hide that infrastructure failure. Clang/lld 23.1.3 were also
provisioned from Debian's official packages. The repo's mold provisioner already
pins 2.42.1 and `-DMOLD_TARGETS='X86_64;I386'`; that exact CMake variable was
rechecked against upstream v2.42.1. No mold build/performance result is claimed.

### Fast-CI oracle repairs (not compiler changes)

Two further linker-suite failures were reproduced and attributed before editing
production code. `note_run_merge` counted a seven-byte allocated `.rodata` payload
as an alignment hole, incorrectly demanding one PT_NOTE instead of two. The
independent oracle now distinguishes real file payload from padding, rejects a
PT_NOTE covering non-note payload, requires exactly-one coverage, and has six
boundary controls. The emitter was not changed.

GCC16 hoisted a TLSLD module-base lookup before the fixture's ARCH_SET_FS inline
syscall. The original LCCC **and GNU ld** executables both SIGSEGV; the fixture
violated the compiler's stable-TLS-base assumption. TLS access now lives in a
separate TU, reached after initialization through an ABI-aligned entry shim.
Both GD and LD relocations must be present, GNU ld must link/run successfully,
and poisoning gd_a's TLS initializer must yield exit 1 and `0`, not a signal.
GCC16 and Clang23 each pass both models, both GNU controls and both poison
controls. No TLS relaxation code was changed. See `linker-fixtures-{before,after}.txt`.

## Reproduction

```sh
scripts/ensure_swap.sh
scripts/build_lccc_fast.sh
python3 tests/regression/check_arm_address_contract.py
python3 tests/regression/check_arm_popcount_contract.py --run
bash tests/regression/check_late_const_gep.sh
python3 tests/workloads/expat-2.8.5/junit_check.py --self-test
python3 scripts/test_ci_gate_parity.py
CARGO_PROFILE_FASTBUILD_DEBUG=0 CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=2 \
  CI_LOCAL_JOBS=2 RUSTFLAGS="$(cat target/lccc-rustflags)" \
  scripts/ci_local.sh --fast             # NOT full/slow CI
```

CTest producer replay (the failing case deliberately makes CTest nonzero):
copy `CTestTestfile.outcomes.cmake` to a private directory as `CTestTestfile.cmake`,
then run `ctest --test-dir DIR --output-junit report.xml`. Inspect all four cases;
nonzero CTest does not mean the XML capture failed.

Oracle and Callgrind command lines, tool banners and compiler binary hashes are
recorded in the archived manifests. For a new comparison, build/freeze the local
compiler first; do not race measurements against replacement of its executable.

## Follow-up priorities and acceptance criteria

1. Focused ARM validation passed: 667 address checks, 10 popcount and 5 input-home
   QEMU executions plus a poisoned-x17 negative control. Complete exact-source fast CI after the last edit; use the
   delivery metadata rather than this document as the CI receipt.
   Missing GCC16/multilib/tool pins are infrastructure failures, not compiler
   passes. Do not downgrade pins or add skips to make a green summary.
2. AArch64 visibility: extend the common merged-visibility infrastructure through
   ARM global symbols and every emitter/binding consumer. Test hidden/internal
   lookup exclusion, protected internal binding, default/weak interposition,
   pointer identity, TU order and same-name locals with an independent linker
   and native/QEMU execution. Only then consider expanding shared ADR support.
3. General inline-asm constraint/alias audit: size metadata currently overloads
   I128 for vector address-backed inputs; a typed by-value/by-address distinction
   would make that contract explicit. Include 8-byte vectors, read/write vector
   outputs, every specific-register alias and allocation exhaustion. Do not
   extrapolate this patch's tested 16-byte input path into complete asm support.
4. Expat dynamic instruction gap: attribution identifies main/loop-state homes
   rather than a slower UTF-8 helper. Prototype proved internal-call clobber
   summaries and register-resident loop phis; compare branch trees and bounds. Use
   corpus-independent validation and preserve UTF-8 rejection behavior.
5. Adler code size/pressure gap: separate harness and hot loop; examine unrolling
   and live intervals before changing the allocator. Fewer total static
   instructions is not sufficient when the bounded dynamic count is better.
6. Validate any performance prototype on pinned P-cores/E-cores of the actual
   i7-14700KF. Collect PMU, cycles, thermal/frequency, affinity and repeated paired
   timing evidence. No PMU/Raptor Lake result was obtained in this VM.
7. JUnit configuration normalization predates these PRs and removes some
   path-bearing switches; audit semantic `-include`/`-imacros` changes separately.
   Retain measured producer fixtures and fail-closed mutation tests.
8. Full/slow hosted CI and expanded real-workload runs remain hosted validation
   duties. Fast CI includes the pinned MSRV check; that is not a substitute for
   hosted multi-platform validation. A local fast pass must not be called full CI.

## Snapshot continuity

An actual mid-conversation reset deleted caches/builds. S01 survived and its
F5 change was already merged upstream. F1 was reconstructed from the recorded
edits, then checkpointed against new main. Subsequent checkpoints materialize
patch, source archive, original-session bundle and restore metadata atomically.
Toolchains/Git object caches are excluded from persistence to stay below the
workspace cap. The tracked source archive and patch are the recovery authority;
never assume that a binary or a background process survived a new turn.
