# Kernel build provenance, RWX diagnostics, location-counter parsing, and call-result shadowing

This change set reconciles a supplied delta with current `main`. The delta was
retrieved through the GitHub contents API and verified by blob SHA-1/SHA-256.
It did not apply to current `main` at zero fuzz because several paths had
already evolved upstream. The final diff preserves the stronger existing
implementations, carries forward the intended behavior, and applies to the base
commit with `git apply --check --unidiff-zero --whitespace=error-all`.

## Toolchain and build policy

All compiler/linker builds used the `fastbuild` profile, two Cargo jobs,
`-D warnings`, Clang + mold, and a 4 GiB swap file. Verified tools:

- rustc 1.99.1 (`58cf8f9c3`, 2026-10-13)
- GCC 16.2.0
- Clang 23.1.4 (`7a941dfb2877`)
- GNU binutils 2.47 (`2.47.20260726`)
- mold 3.0.0
- qemu-user 11.1.2

ICX 2026.2 was requested but unavailable: the Intel PPA provisioning path failed
with a TLS EOF in this sandbox. No other ICX version was substituted.

## 1. ELF `PT_LOAD` RWX diagnostics

### Design

The linker now inspects completed ELF outputs before any flat-binary conversion.
The diagnostic is based on `PT_LOAD` `p_flags`, not section flags: section flags
are not the kernel's final memory-permission contract. The reader is
allocation-free and streaming:

- validates ELF ident, class, endianness, version, and header size;
- handles ELF32 and ELF64 little-endian headers;
- handles `PN_XNUM` by reading section 0 `sh_info` with bounds checks;
- rejects inconsistent program-header tables;
- uses checked 64-bit arithmetic and never trusts `phoff + phnum * phentsize`;
- performs no I/O when `--no-warn-rwx-segments` suppresses the warning;
- preserves the completed output when `--fatal-warnings` promotes the warning.

The flag is wired through every linker entry point and the common driver path.
The standalone script linker remains the oracle for scripted layouts; the
compiler driver's default layout is measured adaptively because current `main`
does not route `-Wl,-T,<script>` into the script linker.

### Alternatives considered

- Slurping the whole output: simpler, but unnecessary I/O and unbounded memory
  for large images. Rejected in favor of bounded header/table reads.
- Warning on writable+executable sections: wrong ABI layer and false positives
  for debug/metadata sections. Rejected in favor of `PT_LOAD`.
- Deleting the output on fatal warnings: loses the artifact needed for debugging
  and diverges from warning-promotion behavior elsewhere. Rejected.

### Evidence

- `cargo test`: RWX reader unit tests and argument-parsing tests pass.
- `tests/linker/check_rwx_segments.py`: ELF64 and ELF32 matrices pass for
  default, fatal, suppression, last-wins, flat output, driver layout, and GNU ld
  verdict/loadable-byte parity. AArch64 and RISC-V shared-driver plumbing and
  byte-exact suppression pass; qemu-user AArch64/RISC-V freestanding static
  runtime smokes pass when qemu is installed.
- `tests/regression/ld_no_warn_rwx_fixture.sh`: passes.

## 2. Assembler location-counter assignment

### Design

`. = <target>` now lowers to the same `AsmItem::Org` path as `.org`, with zero
fill. A shared `parse_org_target` accepts absolute expressions, symbol-plus/minus
constant targets, parenthesized forms, current-location expressions, and backward
numeric-local targets. It rejects empty/malformed targets, backward movement,
and unsupported symbolic arithmetic instead of silently guessing.

Comparison spellings such as `. ==`, `. !=`, `. <=`, and `. >=` are rejected at
the location-counter intercept rather than being misparsed as assignments.
Ordinary symbol assignment and the existing `.org` fill syntax are preserved.
No ELF-writer change was needed: the existing org/relaxation/fill machinery is
already bounds-checked and was exercised by the new integration matrix.

### Oracle matrix

GNU as 2.47 probes established the accepted/rejected baseline for absolute,
symbolic, dot-relative, numeric-local, parenthesized, and multiplication forms.
The implementation intentionally fail-closes two GAS edge behaviors that are not
safe to emulate silently:

- GAS accepts `. == 8` as a comparison expression statement; lccc rejects it.
- GAS may emit an undefined-symbol warning and assume zero for some symbolic
  org targets; lccc errors instead.

These divergences are documented by the test suite and are safer for a compiler
assembler than silent zero substitution.

### Evidence

- Parser unit tests cover absolute/symbolic forms, dot lowering, comparison
  rejection, malformed input, and shared `.org` parsing.
- `tests/regression/check_dot_assignment.py`: ELF64 and ELF32 pass for macro
  vectors, local labels, dot-relative targets, relaxation, fill bytes, GAS
  byte/symbol parity, and fail-closed negatives.
- Existing `check_asm_directive_contract.py`: 517/517 cells hold, with the
  pre-existing pinned divergences unchanged.

## 3. Kernel build provenance and boot identity

### Design

`build_kernel_full.sh` now preserves an existing `.config` by default. Reconfig
is explicit (`LCCC_RECONFIG=1`) and falls back to a fresh defconfig recipe only
when no package config is available. The script records recipe/config/compiler/
linker/package-config SHA-256 in `.lccc-full-build-config`, refuses silent
configuration drift, re-verifies identity after Kbuild, and builds `bzImage`.

`prepare_kernel_tree.sh` now fingerprints the preparation recipe and compiler
content. A matching stamp is a no-op; a compiler-content change cleans and
re-prepares; a recipe/legacy stamp mismatch refuses to overwrite an owned tree
unless `LCCC_REPREPARE=1` is explicit. Damaged canaries also require explicit
reprepare. Stamps are written atomically and temporary files are cleaned on exit.

`qemu_boot_test.sh` now passes the measured `CONFIG_HZ` to the guest, verifies the
guest `/proc/config.gz` hash against the host config, and gates sched-ext sysfs
verdicts on `CONFIG_SCHED_CLASS_EXT`.

Objtool validation checks the ELF magic directly instead of depending on
`file(1)`, so a shell shim cannot masquerade as the real host validator.
`LCCC_NOOBJTOOL=1` and invalid `LCCC_JOBS` are rejected.

### Evidence

- `tests/regression/check_kernel_provenance.py`: mismatch refusal, no-op,
  config identity, compiler-content invalidation, metadata preservation,
  damaged-tree guard, and explicit reprepare pass.
- `tests/regression/check_kernel_full_harness.py`: deterministic mocked Kbuild
  passes config preservation/reconfig, provenance, real ELF objtool, shim/job
  rejection, drift refusal, post-build identity, and prepare-once behavior.
- `bash -n` and Python compilation pass for the changed scripts/tests.

A full Linux kernel build/boot was not executed in this sandbox; the kernel
changes are validated by the deterministic harness above and by CI wiring.

## 4. Call-result inference and non-callable shadows

### Defect

Current `main` already inferred call-result types with correct block scope, but
`analyze_call` did not constrain the callee to a function or function pointer.
A non-callable local shadow therefore passed sema, and lowering could emit a
direct call to the hidden file-scope function. Reproducer:

```c
extern int f(int);
int g(void) { int f = 0; return f(1); }
```

Before the fix, lccc accepted this and emitted `jmp f@PLT`.

### Fix and evidence

`analyze_call` now requires a function or pointer-to-function callee and emits
a called-object constraint error otherwise (the wording matches Clang; GCC uses
the object name instead of the type). Builtins retain their existing path.

- `tests/regression/fnptr_result_shadows_void_function.c`: passes under lccc,
  GCC 16.2, and Clang 23.1.4 at `-O0` and `-O2` with `-Wall -Wextra -Werror`.
- `tests/regression/check_fnptr_result_shadowing.sh`: positive runtime and
  negative must-reject cells pass across the available compilers.
- Full C regression corpus with the GCC 16.2 oracle: **1305 passed, 0 failed,
  3 skipped-compare, 0 skipped-run** out of 1308. The three skipped comparisons
  are declared oracle gaps where GCC cannot compile the asserted semantics;
  lccc compiled and ran all three successfully.

## CI wiring

The new gates are registered in both `scripts/ci_local.sh` and
`.github/workflows/ci.yml`. The `backend-contracts` shell lane was re-balanced
with `scripts/ci_lane_split.py` using measured costs, and
`.github/ci-lane-plan.json` was regenerated so the hosted shard audit remains
honest:

- `linker-rwx-segments`
- `assembler-dot-assignment`
- `kernel-provenance`
- `kernel-full-harness`
- `fnptr-result-shadowing`

`check_ci_lane_plan.py` and `check_ci_shard_partition.py` pass after the
rebalance.

## Validation commands

```sh
scripts/build_lccc_fast.sh
cargo test --profile fastbuild --locked -j2
cargo clippy --all-targets --profile fastbuild --locked -j2 -- -D warnings
python3 scripts/check_clippy_policy.py
python3 tests/linker/check_rwx_segments.py
bash tests/regression/ld_no_warn_rwx_fixture.sh
python3 tests/regression/check_dot_assignment.py
python3 tests/regression/check_asm_directive_contract.py --lccc target/fastbuild/lccc
python3 tests/regression/check_kernel_provenance.py
python3 tests/regression/check_kernel_full_harness.py
bash tests/regression/check_fnptr_result_shadowing.sh
python3 tests/regression/run_regression.py --lccc target/fastbuild/lccc --gcc gcc-16 -j2
```
