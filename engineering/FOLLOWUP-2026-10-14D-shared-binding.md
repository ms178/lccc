# 2026-10-14D: shared-binding audit, validation and handoff

Upstream base: `3ff36e8592b0d0e26645bf37d6685395b1a6eca5` (fresh fetch).
PR #827 incorporated every S12 source/test/evidence change. Replaying those
commits would duplicate already-merged work. The subsequent PR #826 changes
are also retained; all prior patch snapshots remain preserved.

Red-team review of the still-incomplete A14-17 positive controls found that
R_RISCV_64 relocations to default-visible definitions were made RELATIVE,
preventing ELF symbol preemption; undefined imports were lost. Absolute local
symbols incorrectly got a load-base adjustment. This extension emits symbolic
R_RISCV_64 entries with signed addends, retains RELATIVE only for non-preemptible
section addresses, and preserves absolute values. GOT binding and exported
visibility obey the same classification. Visibility constraints from all
references/definitions merge using the shared gABI ordering helper.

Surgical validation passed on the rebuilt implementation. The final exact-tree
fast-CI verdict and stamp are recorded externally in `LCCC-DELIVERY.json` and
`/home/user/artifacts/ci-fast-current.txt`; this document is frozen before that run, so
publishing a result does not invalidate the content-addressed CI stamp. Compiler builds remain fastbuild/O1/j2 with 4 GiB active swap;
local validation is surgical gates followed by fast CI only, never slow/full.

## Rebase to PR #826

The newer main adds relocation width/ULEB128/branch-range validation, stronger
PGO lock/descriptor tests, and independent-build comparison evidence. These
changes are retained. The sole content conflict was the ELF fixture signature:
its new payload/ULEB/no-symbol/displacement parameters and this patch's absolute
symbol parameters are now combined, without changing existing defaults.

A further red-team finding is repaired here: the old shared emitter's constant
`R_RISCV_GLOB_DAT = 6` is not an RV64 address relocation (6 is TLS_DTPMOD32).
Both GOT imports and data pointers now emit the psABI's R_RISCV_64 (2). Local
GOT bookkeeping keys are resolved locally instead of looking up fabricated
names in .dynsym; the redundant first GOT write pass is removed.

## Executed checks (pre-strength checkpoint)

- Fresh upstream `3ff36e85` built in its own worktree/target directory:
  shared-binding matrix **10 PASS / 206 FAIL**. Candidate **216 PASS / 0 FAIL**.
- Supplemental Debian LLD **19.1.7**: **216 PASS / 0 FAIL** on that matrix.
  This is not evidence from the requested LLD 23.1; it is not presented as such.
- Inherited expanded relocation contract: **466 PASS / 0 FAIL / 0 SKIP**.
- Inherited PGO error/locking/descriptor/concurrency suite: **72 PASS / 0 FAIL**.
- New compiler-free verifier red-team suite: **10 tests passed**. Mutations
  cover symbolic versus RELATIVE/TLS relocation, wrong symbol/addend, hidden
  exports, absolute load-base errors, GOT headers/HI20, combined rebased fixture
  parameters, unavailable tools, and optimized Python. The gate refuses `-O`.
- CI/local parity: **220 commands / 22 invocation contracts**.
- Audit census held without raising its baseline. `relocate_address64` isolates
  dynamic-binding policy instead of growing the generic relocation dispatcher;
  ordinary executable linking avoids the added shared-symbol hash lookup.

### Corrections to our own experiment

`shared-binding-initial.*` preserves the initial result (LCCC 216 passed, LLD
179 passed / 37 failed), which was **not** 37 demonstrated linker bugs. The
verifier incorrectly assumed the requested GOT cell was always the first slot.
It now locates the cell through relocation metadata or its absolute value and
checks its HI20 address independently. LLD also retains an undefined protected
weak dynsym entry while resolving references to zero; that is not an exported
definition. Both encodings are accepted, but SHN_UNDEF/value-zero metadata and
actual zero-based resolution remain mandatory. Corrected results are separate.

## Fresh performance evidence and scope

The independent upstream/candidate sweep compiled all 58 benchmark-directory
sources at O0/O1/O2/O3: **232/232 assembly outputs byte-identical**, no failures.
This directory includes unregistered studies; it is not 58 full applications.

Callgrind was rerun for the six registered gzip/zlib-ng/Expat/SQLite/Linux/glibc
**kernels**, using `callgrind_corpus.py`'s explicit bounded sizes. Every upstream
versus candidate pair had identical simulated Ir and matching output. These
are not complete package builds, target-hardware timings, or proof that LCCC
beats GCC. The separate distro-GCC experiment and CE static comparisons expose
remaining gaps. Raw Callgrind records are preserved with commands, hashes,
versions, correctness checks and cache/branch counters.

Relevant tools were read before use: the cache/version-aware `godbolt` helpers,
`codegen_oracle`, `oracle_asm`, `loop_density_oracle`, `tune_oracle`, and the
Callgrind pair/corpus runners. `tight_loop_oracle` was inspected but not used as
evidence: its local compile lacks the remote `--march` flag and its include
path is hard-coded; its histograms do not establish corresponding-loop work.
The density survey similarly labels selected-loop equivalence **unverified**.
The SipHash per-function comparison reports GCC's missing standalone function,
not a zero-sized win. Tune-probe assembly for Raptor Lake is retained.

## Still open / not claimed

- RISC-V dynamic TLS, shared CALL_PLT binding, and nonzero GOT_HI20 addends
  are outside this matrix. A14-17 remains partial, not complete psABI support.
- AArch64 visibility and other-target relocation audits remain separate work.
- Full package builds of all six requested projects and source-built
  mold2.42.1/BFD2.47 linker timing comparisons are not supplied by this bounded
  correctness patch. GAS/objdump 2.47 are the fast-CI oracles. Exact local
  GCC16.2/Clang23.1/LLD23.1 validation was subsequently completed below.
- No 14700KF or PMU is available. Callgrind is simulation. CE supplies pinned
  GCC16.2/Clang23.1 and the recorded ICX channel, not local native timings;
  ICC's core-avx2 run is separate and is not declared ISA-equivalent to v3.
- PGO lock/error fixes are upstream and newly retested, not a novel part of
  this delta. EINTR is diagnosed, not retried; partial I/O has no rollback.
- Snapshot transaction/budget hardening and the remaining audit queue are not
  silently marked closed. No claim of universal optimality or zero defects.

## Actual dynamic-loader validation

`check_riscv_shared_loader.py` additionally compiled real PIC C objects with
Clang 19, linked their DSOs with each independently built LCCC, and executed
LLD-linked PIE test drivers through QEMU and the RV64 glibc dynamic loader.
Candidate **13 PASS / 0 FAIL**, upstream **3 PASS / 10 FAIL**. Both stored data
pointers and GOT-based `get_ptr()` results are checked against executable
interposition, private value 42, missing-weak zero, and absolute 256. This is
execution evidence, not only an ELF parser or simulated loader. Exact commands,
source, runner assembly, statuses and tool/loader hashes are in `loader-*.json`.
These supplemental tools are not mislabeled Clang/LLD 23.1.

The first runtime ABS experiment was itself wrong: same-TU inline `.set`
allowed LLVM's assembler to fold the intended symbolic use into sym0+256 GOT
and literal data, instead of canonical symbol+0 relocations. Initial failures
remain archived. Final ABS cases use separate definition objects, preserving
the intended references, and test default, weak and protected bindings. Local
ABS remains covered by the independent ELF matrix, not claimed as a runtime
case. The null-symbol/nonzero-addend behavior remains separate open work;
LLD19 rejected the initial object rather than providing a valid agreement arm.

All six measured upstream/candidate executable hashes and **all recorded
Callgrind event totals**, not just Ir, match. Real dynamic-loader failures
therefore improve without claiming an x86 generated-code speedup. Individual
comparison values and reproduction commands are in the evidence README.

Fresh CE version probes identify GCC **16.2.0**, Clang **23.1.0** at
`ea7d852a70e8bdfaf601d6626a760f9771b2c4b4`, and ICX channel **2024.0.0.20231017**.
The moving channel's name is not a claim that this is Intel's newest release.
ICC is the recorded **2021.10.0** channel with separate core-avx2 flags.

## Final strengthened gate (supersedes earlier checkpoint counts)

A subsequent red-team pass added cross-TU weak/strong reference and definition
orders, absolute-over-section overrides, real symbols impersonating internal GOT
keys, and duplicate/unnamed local symbols at distinct locations. Nine concrete
failures were reproduced on the previous repair before changing code; LLD
passed those counterexamples. The final matrix has **240 cases per tool**:

- Final candidate: **240 PASS / 0 FAIL**; supplemental LLD19: **240 PASS / 0 FAIL**.
- Independently built upstream: **10 PASS / 230 FAIL** on the same final matrix.
- Final inherited relocation **466/466**, PGO I/O **72/72**, real loader **13/13**.

Repairs: an unresolved strong reference upgrades an earlier weak reference;
a strong ABS definition clears the weak definition's old section affiliation;
internal GOT keys use a NUL-prefixed namespace (impossible for an ELF symbol
name) and include section/value to distinguish equal or unnamed locals. This
also protects executable-linker uses of the shared key helper.

`*-final` evidence and `build-provenance-final.json` describe the final binary.
Earlier named evidence is retained as checkpoint history, not relabeled. The
58-source/four-level sweep was rerun: **232/232 byte-identical assemblies**.
Both six-kernel Callgrind experiments were rerun; all baseline executable hashes
and simulated counters again match. `callgrind-final/` preserves raw counters;
`oracle-local-revalidation.json` proves the final compiler emits exactly the six
saved CE-comparison local bodies with their original matched flags.

## Exact local oracle pins completed

Compiler Explorer's Linux binary archives supplied **GCC 16.2.0**, **Clang
23.1.0** and **LLD 23.1.0**, with archive URLs/SHA-256 and executable hashes in
`pinned-tool-provenance.json`. Clang and LLD report LLVM commit
`ea7d852a70e8bdfaf601d6626a760f9771b2c4b4`. Both compilers successfully linked
and ran 64-bit and `-m32` canaries. These are not relabeled distro tools.

- Exact LLD23 shared-binding matrix: **240/240**, alongside candidate **240/240**.
- Real loader tests with Clang23 objects and LLD23 PIEs: candidate **13/13**,
  independently built upstream **3/13**.
- Fresh Callgrind comparisons against local **GCC16.2** and **Clang23.1**:
  all **six/six correctness and measurement pairs** completed in each arm.
  This remains bounded kernel simulation, not full-package or hardware timing.
  The pinned comparisons do not support universal superiority: candidate Ir
  exceeds Clang's in all six workloads; it exceeds GCC's in four of six.

An initial fast-CI attempt encountered eight missing-GCC16 prerequisite
failures, not compiler verdict failures. No checks were weakened: install the
pinned compiler and export `GCC` to its executable. All eight affected gates
then passed surgical re-execution. The final full **fast-only** command is:

```bash
GCC=/home/user/.cache/oracles/gcc-16.2.0/bin/gcc \
PATH=/home/user/.cache/oracles/bin:$PATH \
CARGO_PROFILE_FASTBUILD_DEBUG=0 CARGO_INCREMENTAL=0 \
bash scripts/ci_local.sh --fast
```

`oracles/bin/gcc-16` is a symlink to the genuine GCC16.2 executable, not a
version wrapper. Default distro `gcc`/Rust linker selection is not replaced.
Strict Clippy separately passed on the final Rust implementation (76 policy
lint names probed, none unknown). The frozen-tree CI receipt remains external.
