# Resource isolation and rounded RV64 AUIPC displacement checks

Base: `3ff36e8592b0d0e26645bf37d6685395b1a6eca5`, PR826 including PR827.
S31 preserved the rebased CI changes before additional implementation.
Historical S23 evidence is not certification of this tree.

## Decisions and implementation

* A14-34: use subshell gate isolation, rather than a fragile save/restore list
  that must handle every return path and distinguish unset/empty/exported state.
  Share the resource policy; do not infer the job budget from debug-info choice.
  Explicit CI_LOCAL_JOBS wins over CARGO_BUILD_JOBS. Missing/invalid RAM chooses
  one job. Preserve explicit debug/incremental preferences. Propagate reporter
  self-test failures and reject repetition counts that Bash arithmetic could
  wrap into a false zero-test success. The new permanent test runs in local fast
  and hosted CI and has an invocation parity contract. No lint is disabled.
* Agree with upstream's input-contribution bounds (merged-buffer bounds are
  insufficient), rejection of unknown relocations in shared mode, and encoding
  the final ULEB difference in its original reserved width. Intermediate label
  values need not fit that field. Widened ULEB subtraction and modular XLEN
  PC-relative arithmetic implement different contracts and should stay distinct.
* Agree with holding the PGO lock through fclose: stdio may defer all writes.
  Checked I/O plus deterministic contention, ingestion and mutation controls
  are stronger evidence than a lucky parallel run. Do not replay these fixes.
* The direct-branch checks did not cover rounded AUIPC displacements. Reproduced
  CALL_PLT +2147481600 linking successfully but decoding as -2147485696, a 2^32
  error. PCREL_HI20 similarly truncated. A generic signed-32 guard is wrong:
  AUIPC plus signed LO12 spans **[-2147485696, 2147481599]**. The helper checks
  this asymmetric range before masking/mutation and retains modulo-64 address
  and addend arithmetic. It is used for PCREL_HI20, CALL_PLT, GOT_HI20 and
  TLS_GOT_HI20. Existing GD-to-LE call relaxation is not reinterpreted.
  Odd immediates remain legal: PC-relative data can be byte-aligned; JALR clears
  its target bit zero in hardware rather than discarding an immediate bit.

## Evidence and scope

`engineering/evidence/2026-10-14-ci-resources/` contains the original isolation
failure and permanent-test output. The four tests exercise 64 RAM/environment
configurations, isolated gates, both orders, caller-state preservation, failure
propagation and invalid/overflow counts. Fresh warning-denied compiler build:
fastbuild/-O1/-j2, stable Rust 1.99.0, debug=0/incremental=0, 6 GiB swap active.

Fresh upstream replay before the AUIPC extension: RISC-V **466/0/0**;
PGO **72/0/0**, including exact production ingestion, not just a raw-file parser;
PR826 harness red-team **9 tests pass**. No full/slow CI was requested or run.

`engineering/evidence/2026-10-14-auipc-range/` records before/after and independent
LLVM linked-binary oracle evidence. Added ELF cases keep symbols within their
input sections (S=P) and use the addend to reach the boundaries. Both ET_EXEC
and ET_DYN, local and hidden-global symbols, exact reconstructed immediates,
opcode/register preservation and adjacent contribution sentinels are checked.
Before the fix, the added cases produce **288 passes / 64 failures**. After the
fix, the complete ELF suite is **818 PASS / 0 FAIL / 0 SKIP**, and all **six**
displacement unit tests pass. Full ELF records are retained as `.json.gz`. Unit tests
also cover all 4096 low-field values at five high-field anchors and XLEN wrap.
The complete local test/fast-CI status belongs to the delivery manifest and
current validation logs, not to an old build or a copied certificate.

## Remaining acceptance scope

A14-20 and A14-34 meet their listed targeted acceptance contracts. A14-17 stays
open for its wider cross-architecture contract. In particular, this work does
not certify TLS model selection, shared-symbol preemption, orphan low-part
lookup, or AArch64 hidden/protected export semantics (A14-35). Those need their
own implementation and executable controls. The backlog now has 35 entries,
28 open; this is not a claim that the compiler is perfected or warning-free
under every possible configuration.

No generated-code speedup is claimed. Native Callgrind controls, when recorded,
are simulated whole-program counters, not physical Raptor Lake timing. Oracle
IDs, effective versions and target flags must be read from the fresh manifests;
CE's `icxlatest` alias is not proof of Intel's newest public release.

## Fresh oracle and regression controls

* Twelve LLVM linked-binary boundary checks pass with exact AUIPC/JALR words
  and specific range diagnostics. `-Wl,-v` records **LLD 23.1.0**. Fixed-width
  instructions and disabled relaxation prevent a compressed-JALR fixture from
  accidentally relocating into the next instruction.
* GCC **16.2.0**, Clang **23.1.0**, CE `icxlatest` (**2024.0.0**, not claimed to
  be Intel's latest public release), and ICC **2021.10.0** execute the bounded
  gzip CRC32 control and agree with rebuilt LCCC: `6d5b5c2b`. ICC uses recorded
  `-march=core-avx2` rather than its unsupported v3 spelling; its deprecation
  remark is retained. This integer correctness control is not a runtime ranking
  or a claim of identical general FP/ISA policies. GCC's CE version banner
  identifies its bundled binutils as 2.44, not the requested 2.47 linker oracle.
* Six native workloads, three paired Callgrind rounds: **18 Ir ratios = 1.0**,
  **18 byte-identical generated ELFs**. Arithmetic/geometric mean and best/worst
  are all **1.0**; zero wins, zero regressions, six tied workloads. Both compiler
  binaries were independently built and have different recorded fingerprints.
  Twelve Memcheck runs (six workloads, both builds) report zero errors.
  Full raw Callgrind counters and manifests are preserved losslessly compressed.
  Valgrind's host-cache discovery warnings are retained; all raw headers confirm
  the requested 32 KiB I1/D1 and 32 MiB, 16-way LL simulation.
* Bash syntax and rustfmt pass; invocation parity is **219 commands / 23
  contracts**, with **71 parity tests passing**. Final fast CI and strict Clippy
  are separate gates; the delivery manifest is the authority for their status.
