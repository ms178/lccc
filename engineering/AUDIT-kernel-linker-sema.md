# Kernel build provenance, RWX diagnostics, location-counter parsing, and call-result shadowing

This change set reconciles a supplied delta with current `main`. The delta was
retrieved through the GitHub contents API and verified by blob SHA-1/SHA-256.
It did not apply to current `main` at zero fuzz because several paths had
already evolved upstream. The final diff preserves the stronger existing
implementations, carries forward the intended behavior, and applies to the base
commit with `git apply --check --unidiff-zero --whitespace=error-all`.

## Reconciliation of the 49-commit upstream series

The latest `ms178-1.patch` fetched through the GitHub contents/blob API is a
49-commit mbox (2,209,177 bytes, blob `1a3486c5…`, SHA-256 `7fba9ea7…`). It
does not apply at zero fuzz to current `main` — and patch 0001 does not apply
at zero fuzz even to its own declared base `0ff1d3f12` — so a forced `git am`
was rejected. Instead the series was reconciled by path and content:

- At the original audit snapshot, all **222 paths** touched by the 49 patches
  were a subset of the **227 changed files** in open PR #1051 (`bd55a8e1`).
  The refresh at `99c1eba6` (base `60a2e2c1`) finds 221 of those paths in its
  **226-file diff**; the remaining path, `.github/ci-lane-plan.json`, is already
  identical on that PR's base and therefore no longer appears in its diff.
  All 222 paths exist in the PR tree. This is path-coverage evidence, not a
  claim that every file is byte-identical.
- Spot-checked final blobs from PR #1051 against the series' added content;
  all four sampled blob IDs are unchanged at `99c1eba6`. These paths are
  subsumed by PR #1051 and are not duplicated in this branch:

  | PR #1051 path (subsumed upstream) | Comparison against series content |
  |---|---|
  | `scripts/diffrun.sh` | byte-identical |
  | `src/backend/linker_common/binary_input.rs` | byte-identical |
  | `tests/regression/ebp_data_register_stores_i686.c` | byte-identical |
  | `src/backend/riscv/linker/relax.rs` | strict superset (extra overflow-hardening) |
- The remaining i686/optimize/linker fixes (patches 36–48) are carried by
  PR #1051; the sampled relaxation file has additional overflow hardening.
  Duplicating that work here would create a competing copy.

The orthogonal kernel-provenance / RWX / dot-assignment / sema delta was
merged as #1052. This follow-up keeps only the measured RWX-reader and
location-counter-parser improvements plus their audit, rebased onto `620e7e4b`.

## Toolchain and build policy

All compiler/linker builds used the `fastbuild` profile, two Cargo jobs,
`-D warnings`, Clang + mold, and a 4 GiB swap file. Verified tools:

- rustc 1.99.1 (`58cf8f9c3`, 2026-10-13)
- GCC 16.2.0
- Clang 23.1.4 (`7a941dfb2877`)
- GNU binutils 2.47 (`2.47.20260726`)
- mold 3.0.0
- qemu-user 11.1.2

ICX 2026.2 was requested but unavailable locally: the Intel PPA provisioning
path failed with a TLS EOF in this sandbox. No other ICX version was
substituted. The hosted `Authenticated Intel ICX 2026.2 integration` CI job
provisions ICX itself and passes on this branch.

## 1. ELF `PT_LOAD` RWX diagnostics

### Design

The linker now inspects completed ELF outputs before any flat-binary conversion.
The diagnostic is based on `PT_LOAD` `p_flags`, not section flags: section flags
are not the kernel's final memory-permission contract. The reader is
bounded-memory and chunked:

- validates ELF ident, class, endianness, version, and header size;
- handles ELF32 and ELF64 little-endian headers;
- handles `PN_XNUM` by reading section 0 `sh_info` with bounds checks;
- rejects inconsistent program-header tables;
- uses checked 64-bit arithmetic and never trusts `phoff + phnum * phentsize`;
- seeks once to the program-header table and reads it in bounded chunks of at
  most 64 KiB (one oversized `phentsize` still fits in a single chunk), so a
  table of N headers costs O(N / chunk) reads instead of N seek+read pairs;
- stops at the first RWX `PT_LOAD` without reading the rest of the table;
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

### Measured performance

The first implementation issued one `seek` + one `read` per program header.
That is correct but pays two syscalls per segment; a kernel image with hundreds
of PHDRs turns the diagnostic into a syscall storm. The final implementation
seeks once and reads the contiguous table in chunks capped at 64 KiB, stopping
at the first RWX hit.

Two independent measurements pin the contract:

1. **Exact I/O-count unit tests** (`batching_bounds_io_and_crosses_chunks`,
   `extended_phnum`, `oversized_phentsize_is_one_entry_per_chunk`): a
   4096-header table takes exactly 4 reads for ELF32 and 6 for ELF64, with
   3 seeks; an early hit takes 3 reads. `PN_XNUM` adds exactly one read/seek,
   and an oversized `phentsize = u16::MAX` takes one read per entry. The tests
   also pin last-chunk detection and fail-closed truncation.
2. **End-to-end A/B timing** against the exact base `620e7e4b`, with both
   variants rebuilt by the same `fastbuild` script/toolchain, CPU-pinned,
   8 warmups and 80 randomized-interleaved samples per variant. Every timed
   invocation succeeded without diagnostics and produced identical bytes.

   | workload | base median | optimized median | median speedup | bootstrap 95% CI |
   |---|---:|---:|---:|---:|
   | link, 4096 `PT_LOAD` headers | 9.82 ms | 8.44 ms | **1.16×** | 1.147–1.177× |
   | assemble, 20,000 symbolic `.org` targets | 213.61 ms | 205.63 ms | **1.04×** | 1.023–1.055× |

   Output SHA-256 prefixes are `94598bf0…` (link) and `5f24b59f…` (assemble).
   The interval is a 5,000-resample bootstrap of the median ratio. The
   assembler workload is end-to-end and noisier (one long optimized-sample
   outlier raised its mean), so the claim is the modest median improvement,
   not a broad compiler-speed claim. Normal images use the single-chunk path;
   suppression still performs zero I/O.

## 2. Assembler location-counter assignment

### Design

`. = <target>` now lowers to the same `AsmItem::Org` path as `.org`, with zero
fill. A shared `parse_org_target` accepts absolute expressions, symbol-plus/minus
constant targets, parenthesized forms, current-location expressions, and backward
numeric-local targets. It rejects empty/malformed targets, backward movement,
and unsupported symbolic arithmetic instead of silently guessing.

Subtractive addends are negated with `checked_neg` on the parsed integer instead
of allocating a `-(...)` string and re-parsing it. This removes one allocation
and one parse per symbolic `.org`/`. =` target, and keeps the `i64::MIN`
boundary fail-closed (`base - -9223372036854775808` is rejected, not wrapped).
Unit tests pin double negatives, parenthesized products, and the overflow edge.

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
- Full C regression corpus with the GCC 16.2 oracle on base `620e7e4b`:
  **1307 passed, 0 failed, 3 skipped-compare, 0 skipped-run** out of 1310.
  The three skipped comparisons are declared oracle gaps where GCC cannot
  compile the asserted semantics; lccc compiled and ran all three successfully.

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

## Validation on base `620e7e4b`

The rebased tree passed 7152 workspace tests (0 failed, 13 ignored), a
warning-free `fastbuild` build with `-D warnings` and 4 GiB swap, Clippy with
`-D warnings`, the 76-lint policy probe, rustfmt, and the doc-link gate.
The assembler directive contract held all 517 cells (14 pinned divergences).
CI structure checks passed: lane plan (2516 s), disjoint fast-contract shards
(86/96 gates), workflow shell (12 files), and local/hosted parity (481 commands,
27 invocation contracts). The targeted RWX, dot/org, kernel-provenance,
kernel-full, call-result-shadowing, and suppression gates all passed.

## Validation commands

```sh
scripts/build_lccc_fast.sh
cargo test --workspace --profile fastbuild --locked -j2
cargo clippy --workspace --all-targets --profile fastbuild --locked -j2 -- -D warnings
python3 scripts/check_clippy_policy.py
cargo fmt --all -- --check
python3 scripts/check_doc_links.py
python3 tests/linker/check_rwx_segments.py
bash tests/regression/ld_no_warn_rwx_fixture.sh
python3 tests/regression/check_dot_assignment.py
python3 tests/regression/check_asm_directive_contract.py --lccc target/fastbuild/lccc
python3 tests/regression/check_kernel_provenance.py
python3 tests/regression/check_kernel_full_harness.py
bash tests/regression/check_fnptr_result_shadowing.sh
python3 tests/regression/check_ci_lane_plan.py
python3 tests/regression/check_ci_shard_partition.py
python3 scripts/check_ci_workflow_shell.py
python3 scripts/check_ci_gate_parity.py
python3 tests/regression/run_regression.py --lccc target/fastbuild/lccc --gcc gcc-16 -j2
```
