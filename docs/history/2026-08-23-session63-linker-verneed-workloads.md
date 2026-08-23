# Session 63 — linker version metadata, staged libc workload, and oracle hardening

Date: 2026-08-23

## Continuity and snapshot

- Upstream base at the start of this session: `33956afb9ee355e8bc5818bfad92b8d02f6ab5de` (`ms178/lccc` `main`).
- The worktree was rebased/reset to the latest fetched `origin/main` before any session edits.
- The first saved patch was `S01-session63-linker-verneed`; after this document is added, run the snapshot script once more so the canonical `/home/user/ms178-1.patch` also contains this follow-up record.
- The next agent must not change `/home/user/artifacts/.base_ref` casually: it is the exact base for the cumulative patch.

The authoritative recovery command remains:

```bash
cd /home/user/lccc
./scripts/lccc-snapshot.sh session63-followup "record session 63 results and next research gates"
```

The snapshot script commits pending work, refreshes `/home/user/ms178-1.patch`, writes the per-session patch under `/home/user/artifacts`, refreshes the source tarball and bundle, and verifies patch applicability.

## Environment established

- Host: constrained 2 GiB VM, two available CPUs, no usable hardware PMU/perf installation.
- An 8 GiB `/swapfile` was created with `mkswap` and successfully activated with `swapon`; the current `free -h`/`swapon --show` output showed it active. The disposable 4 GiB workspace data file was removed to avoid wasting persisted snapshot capacity.
- `cargo build --profile fastbuild --locked -j 2` is the required development build. The compiler itself is Rust `opt-level=1`; this is separate from the target C optimization flag.
- Rust 1.85, Clang 19, CMake, Ninja, bison, gawk, texinfo, gettext, zstd and binutils 2.47 were installed for the session.
- Pinned GNU binutils 2.47 was built under `/home/user/tools/binutils-2.47`. The exact archive digest was recorded in the session artifacts.

## Landed in LCCC

### 1. Selectable ELF interpreter

`--dynamic-linker=PATH`, its separated form, `-dynamic-linker`, `-I`, and the `-Wl,` spellings are now preserved by the common linker argument parser and the standalone `lccc-ld` driver. The x86 executable emitter receives one NUL-terminated interpreter buffer and uses it consistently for layout, `PT_INTERP`, and `.interp`.

Previously the option was accepted and discarded while the emitter silently hard-wired `/lib64/ld-linux-x86-64.so.2`. That made a staged/sysroot binary execute against the host loader even when the build command visibly requested another loader.

### 2. Shared-object version requirements

The x86 shared-object emitter now emits consumer-side GNU version metadata:

- `.gnu.version_r` (`SHT_GNU_VERNEED`);
- `DT_VERNEED` and `DT_VERNEEDNUM`;
- deterministic per-library/per-version indices;
- correctly sized `.dynstr` before `vna_name` offsets are captured; and
- versioned `.gnu.version` entries for undefined imports.

This fixed the malformed staged-glibc metadata where ld.so reported a bogus version such as `G`. The implementation also keeps provider-side `.gnu.version_d` and consumer-side `.gnu.version_r` index spaces distinct.

The multi-node version-script path was corrected so `LIBV_2.0 { ... } LIBV_1.0;` emits `Cnt: 2` and `Parent 1: LIBV_1.0`. A needs-only DSO gets a valid base verdef and version table rather than a zero/invalid string offset. The `dynstr_size` ordering bug and the `local: *`/version-node branch ordering bug were both caught by direct ELF inspection.

### 3. `.symver` default aliases

The common ELF64 symbol-registration pass now installs an unversioned alias for a defined `public@@VERSION` symbol. This is required for ordinary references inside the same DSO: GNU ld binds the plain `public` reference to the default `@@` definition. Without it, glibc's `libc_pic.os.clean` left `memcpy` unresolved and acquired an accidental host-libc dependency.

### 4. Oracle correctness

`scripts/godbolt.py` and `scripts/codegen_oracle.py` now:

- recognize GCC/LLVM clone suffixes such as `.constprop.0`, `.isra.0`, `.part.0`, `.cold`, and `llvm.*` without accepting unrelated symbols;
- stop a requested function at `.size`/`.cfi_endproc`; and
- fail loudly when a requested function was inlined or removed instead of producing a misleading zero-instruction “win”.

`tests/scripts/test_oracle_function_matching.py` covers exact labels, clone labels, unrelated symbols, missing functions and whole-TU behavior. The offline test ran successfully.

### 5. Regression-runner path normalization

`tests/regression/run_regression.py` now resolves the LCCC and path-like GCC arguments before worker threads start. The old invocation `--lccc target/fastbuild/lccc` launched workers from temporary directories and reported every test as an ENOENT failure even though the compiler existed.

## Workload results

### SQLite 3.53.4

The source archive downloaded from the `packages/sqlite/PKGBUILD` URL had SHA-256:

```text
d18fa15aec74d8c17e1463f861095adc01b5ad190256acb4f91d22f0368d232b
```

The current recipe lists `2d7b032b...`, which does not match the byte-identical archive retrieved twice in the prior provenance audit. This discrepancy is recorded, not hidden.

The recipe's `sqlite-lemon-system-template.patch` did not apply with `--fuzz=0` to the 3.53.4 release tarball because the context changed (`argv[0]` and line movement). A release-specific equivalent was added as:

```text
tests/workloads/sqlite-3.53.4/sqlite-lemon-system-template-3.53.4.patch
```

The new `tests/workloads/sqlite-3.53.4/run.sh` refuses an archive digest mismatch, records the stale recipe-patch status, applies the exact compatibility patch, builds SQLite with LCCC, compiles the shell with LCCC, links a second shell directly with `lccc-ld`, and executes a deterministic SQL workload. It was run successfully from a clean work directory:

```text
SQLite 3.53.4: PASS
LCCC compile + lccc-ld final link: PASS
LCCC driver output == lccc-ld output: PASS
```

The clean-run manifest is retained outside the repository under the workload results. The SQLite build is intentionally not copied into the LCCC git worktree or into `ms178-1.patch`.

### glibc 2.44

The official tarball SHA-256 was:

```text
37f600f2bef3c5e8300147059568b2a2e40a7ad6ccc65ce942556d49429cc667
```

The current `toolchain-stable/glibc/ms178-glibc.patch` applied cleanly with `--fuzz=0` to the official `glibc-2.44.tar.xz`. The full scalar x86_64 glibc build and staged install completed with LCCC and the companion lccc-ld selected by the build environment.

The build used these capability gates, which must remain explicit in all future claims:

```text
--disable-multi-arch   # current LCCC linker has no IFUNC support
--disable-mathvec      # current LCCC cannot assemble glibc AVX-512 libmvec sources
--disable-sframe       # current LCCC assembler does not emit SFrame v2
--enable-kernel=3.2    # host headers do not advertise the recipe's 6.18 baseline
```

The glibc link initially exposed the shared-linker defects above. After the fixes, `libc.so.6` structurally contains:

```text
_rtld_global_ro@GLIBC_PRIVATE
DT_VERNEED / DT_VERNEEDNUM
.gnu.version_r -> ld-1-x86-64.so.2: GLIBC_2.2.5, GLIBC_2.3, GLIBC_2.35, GLIBC_PRIVATE
```

`tests/workloads/glibc-2.44/build_lccc.sh` was added as the reproducible, non-destructive staged build gate. Its first invocation found and fixed a shell variable typo (`$BIN` versus `$BINUTILS`) before this follow-up was written. Because the user requested no more validation in the current turn, rerun the script in the next session rather than treating the post-fix script itself as already clean.

A custom-interpreter smoke executable was linked both with the LCCC driver and directly with lccc-ld. `PT_INTERP` correctly pointed at the staged loader. Runtime execution still exits with SIGSEGV 139 before producing output. This is intentionally **not** a glibc runtime pass. The next session must debug the remaining loader/startup failure; the structural version-table fix is validated independently by ELF inspection and the 160-case linker suite.

## Validation observed this session

These are screening facts from the constrained VM, not Raptor Lake PMU evidence:

```text
LCCC fastbuild build (-O1, -j2)              PASS
Rust library tests                            1094 passed, 0 failed, 6 ignored
Standalone/linker regression suite            160 passed, 0 failed
Oracle function-extraction offline tests       5 passed, 0 failed
SQLite 3.53.4 clean workload gate             PASS
Six-kernel paired LCCC/GCC screen              all 6 correct; LCCC slower on all
```

The six-kernel five-round paired screen was:

| Kernel | LCCC/GCC median ratio | Interpretation |
|---|---:|---|
| gzip CRC | 1.071 | LCCC slower |
| zlib-ng Adler | 1.531 | LCCC slower |
| Expat scan | 2.061 | LCCC slower |
| SQLite varint | 1.867 | LCCC slower |
| Linux find-bit | 1.338 | LCCC slower |
| glibc memcmp | 1.526 | LCCC slower |

The machine has no installed usable `perf`, so these are pinned paired wall-clock screening data with raw samples and assembly artifacts, not counter-backed hardware claims. The static CE oracle at `-O2 -fno-inline -march=x86-64-v3` showed the remaining code-generation gaps clearly: LCCC had 294 instructions/27 spills for the SQLite varint helper versus 115–118 instructions and no spills for Clang/ICC/ICX; Linux find-bit was 159 versus 34 GCC instructions; and glibc memcmp was 281 versus 26 GCC instructions. These are prioritization evidence, not a claim that static instruction count alone predicts runtime.

The correctly invoked full C regression runner reached 423 passed, 3 failed, 8 skipped-compare, 0 skipped-run out of 434. The three failures were:

```text
i686_fused_mul_add_operand_order   compile/run path needs exact lccc-i686 validation
segment_fill_copy_alias            compile/run path needs exact lccc-i686 validation
tce_loop_phi_param_retarget         LCCC output differs from GCC reference
```

Do not call the full regression suite green until those are investigated with their exact `.flags` and the intended i686 driver. The earlier 0/434 result was only the runner's relative-path bug and must not be retained as evidence.

## Next-session priority order

### P0 — finish correctness before performance claims

1. **Re-run and debug the staged glibc runtime SIGSEGV.** Use a debugger/strace-equivalent if available, inspect the first faulting instruction, and compare the custom loader's `_r_debug`, `_rtld_global`, TLS, relocation, and symbol-version setup with GNU ld output. Check whether the failure is in LCCC's loader self-relocation, the `LINKER_DEFINED_SYMBOLS` suppression list, or the custom glibc build assumptions. Keep the custom interpreter smoke as a hard gate; do not weaken it to make the report green.
2. **Run `tests/workloads/glibc-2.44/build_lccc.sh` after the `$BINUTILS` typo fix.** Keep the clean tarball/patch checks and retain the manifest. If another script defect appears, fix the script, rerun only the affected gate, then snapshot immediately.
3. **Investigate the three real regression failures.** Use `target/fastbuild/lccc-i686` for the `-m32` cases, preserve their flags, collect assembly and exact exit status, and use GCC only as a differential oracle. For TCE, reduce the mismatch to the first hash/phi edge and inspect generated code; do not mark it expected.
4. **Run the complete linker suite with `target/fastbuild/lccc-x86` and keep it at 160/160.** The direct `lccc-ld` smoke and custom-interpreter option need a dedicated permanent regression so this new feature cannot regress silently.

### P1 — highest measured code-generation gaps

5. **SQLite varint:** reduce the 27 LCCC spills and six callee-saved pushes. Start with the live-range/segment RA and accumulator/home model, then re-run the CE four-compiler oracle and the SQLite clean workload. Do not copy a compiler's branch pattern without proving C semantics and measuring the real shell workload.
6. **Linux find-bit:** compare the current 159-instruction LCCC path against GCC's 34-instruction `andn`/`cmov` form. Determine why the existing ANDN fusion is insufficient and whether a known-bits/zero-test/ctz lowering can produce the shorter shape without introducing an illegal target dependency.
7. **glibc memcmp and Expat:** target stack homes and branch/bit-test lowering only after a focused reproducer, assembly diff, correctness fuzz, and the paired workload gate agree.
8. **Real SQLite, zlib-ng, gzip and Expat projects:** graduate from kernels to complete upstream builds with the existing workload runners. Record compile flags, source/archive digests, generated binary sizes, checksums, timings, and every failed test.

### P2 — oracle and infrastructure completion

9. **Oracle channel policy:** CE currently exposes `cg162`, `cclang2210`, `cicc2021100`, and `cicxlatest`. Re-query the CE list at the next session start and record the resolved IDs/names in each manifest. Decide explicitly whether `cclang_trunk` supersedes the fixed 22.1 channel; never label a moving channel as reproducible without recording its resolved metadata.
10. **Linker oracle provisioning:** run `tests/linker/setup_oracles.sh` when disk headroom permits. Preserve the requested mold fast preset exactly: `-DMOLD_TARGETS='X86_64;I386'`, `-DMOLD_USE_MIMALLOC=OFF`, and `-DMOLD_LTO=OFF`; build wild's `wild-linker` package only. Record git revisions in `tools/bin/ORACLE_REVISIONS.txt`.
11. **PMU limitation:** `perf` is unavailable in this VM. Continue using CPU affinity, randomized paired order, warmups, raw samples, median plus variance/bootstrap intervals, checksum gating, and assembly/cost reports. Do not invent Top-Down metrics. Re-run the promising candidates on the real i7-14700KF before claiming hardware wins.
12. **Snapshot after each validated change.** Use a distinct slug (`session63-*`, then `session64-*`), keep the cumulative artifact ledger, and ensure `ms178-1.patch` remains applicable to the recorded upstream base.

## Explicit non-claims

- No claim was made that LCCC beats GCC/Clang/ICC/ICX on the six workload kernels; it did not in the VM screen.
- No claim was made that the staged glibc runtime is working; it still SIGSEGVs.
- No claim was made that the full 434-test regression corpus is green; three failures remain.
- No hardware-counter or Raptor Lake performance claim is possible in this environment.
