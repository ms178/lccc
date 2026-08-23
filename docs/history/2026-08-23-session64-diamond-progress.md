# Session 64 — Diamond-level progress after Rust 1.98 upgrade

Date: 2026-08-23

## Baseline

Latest upstream main was fetched and rebased before work:

```text
99c0c07bf5b069106a9b425087121ec2de90bb49
```

The previous linker/verneed/Oracle/workload patch was already present upstream in `87e5db1c`. New local work is saved relative to `99c0c07b` and snapshot after each validated unit of work.

## Completed this session

### 1. Rust/Cargo 1.98.0 is now the compiler-build baseline

The persisted rustup toolchain reports:

```text
cargo 1.98.0 (797e8a9bc 2026-08-05)
rustc 1.98.0 (88d9e12ae 2026-08-18)
rustup 1.29.0
```

`rust-toolchain.toml` pins channel `1.98.0`, minimal profile, `rustfmt` and `clippy`. The toolchain is installed below `/home/user/.cargo` and `/home/user/.rustup`, both in the persistent workspace.

Validated with the repository's required fastbuild command:

```text
cargo build --profile fastbuild --locked -j 2: PASS
```

### 2. Harness persistence was corrected

`build_lccc_fast.sh`, `build_lccc_o1_j2.sh` and `arena_session_restore.sh` now prefer the persisted Cargo installation and export `RUSTUP_TOOLCHAIN=1.98.0`. The old `/opt` installation path was removed from the restore logic because the harness wipes `/opt` between turns and silently downgraded the next session.

Swap remains an explicit prerequisite: the active `/swapfile` is 8 GiB.

### 3. The TCE differential reproducer is now defined C

The previous `tce_loop_phi_param_retarget.c` could dereference a pointer formed by advancing an empty string. The GCC/LCCC output discrepancy was therefore not a trustworthy compiler oracle. The retry pointer now advances only for a non-empty string.

LCCC and GCC agree across `-O0`, `-O1`, `-O2` and `-O3`. This fixed test was committed and snapshotted.

### 4. Full native regression baseline is green

With Rust 1.98 and the corrected reproducer:

```text
426 passed, 0 failed, 8 skipped-compare, 0 skipped-run, 434 total
```

The eight skips have explicit GCC/environment reasons. The two i686 cases previously reported as failures pass with the current rebuilt `lccc`/`lccc-i686` exact flag paths. Do not remove the skip accounting; it is part of the evidence.

### 5. Linker baseline remains green

The complete linker suite remains:

```text
160 pass, 0 fail, 0 warn, 0 skip
```

This was rerun after the Rust 1.98 rebuild and after the x86 peephole change below.

### 6. New low-risk x86 store-address peephole

The existing `eliminate_rcx_address_copy` handled pointer copies feeding loads but missed the equally common store shape:

```asm
movq %rdx, %rcx
movq %rax, (%rcx)
```

A new G3 case folds it to:

```asm
movq %rax, (%rdx)
```

The transform is deliberately narrow and fail-closed:

- only a plain `(%rcx)` destination is accepted;
- the store source must not read `%rcx`;
- `%rcx` must be dead after the store;
- displacement forms are not rewritten; and
- the existing `CCC_NO_X64_NOHOME_CLASSES`/`rcx_copy` kill-switch path remains available.

Two unit tests cover the positive case and preservation when `%rcx` remains live. The LCCC library test count rose to `1096 passed, 0 failed, 6 ignored`. The SQLite varint static body fell from 294 to 288 counted instructions in the local oracle at `-O2 -fno-inline -march=x86-64-v3`; spills remain 27, so this is a small instruction-count win, not a solved RA gap.

### 7. Fresh performance and Oracle evidence was retained

The current seven-round six-kernel VM screen was checksum-correct. It remains screening-only because the VM has no usable PMU. The post-peephole SQLite-specific run remained correct; its noisy wall-clock ratio is not used to claim a runtime improvement. The static Oracle still identifies the central gap:

```text
LCCC sqlite_get_varint: 288 instructions, 27 spills
GCC 16.2:              116 instructions, 0 spills
Clang 22.1:            115 instructions, 0 spills
ICC 2021.10:           118 instructions, 0 spills
ICX latest:            118 instructions, 0 spills
```

The full raw JSON, assembly and paired timing artifacts are outside the repository under `/home/user/results/`.

## Snapshot ledger for this session

```text
S03-session64-upstream-rebase
S04-session64-rust198-harness
S05-session64-tce-oracle
S06-session64-glibc-harness
S07-session64-glibc-pipefail
S08-session64-followup
S09-session64-rcx-store-fold
```

The canonical patch is always `/home/user/ms178-1.patch`, with per-snapshot copies under `/home/user/artifacts/`.

## Next work, in strict order

### P0 — correctness and staged glibc runtime

1. Run `GLIBC_RUN_SMOKE=1 GLIBC_REUSE_BUILD=1` against the completed staged glibc tree. The prior staged program reached the custom interpreter and then failed before output with SIGSEGV 139. Use gdb/strace, inspect the first faulting instruction and compare the loader's `_r_debug`, `_rtld_global`, TLS, relocation and version state with GNU ld output.
2. Add a permanent staged-libc runtime regression only after the fault is understood. It must use both LCCC driver linking and direct `lccc-ld` linking, exercise `malloc`, `printf`, `fma` and versioned imports, and never overwrite host `/lib`.
3. Keep the eight GCC skips explicit. Convert each into a capability matrix entry with source, flags, reason, and the required external dependency.

### P1 — generated-code performance

4. **RA next-use/reload splitting:** SQLite, zlib-ng and Expat are the first candidates. The current linear scan still consumes fat intervals even though hole-aware segments exist. Prototype a bounded reload-at-next-use mode behind a kill switch. Compare spills, frame references, instructions, runtime and correctness before broadening.
5. **SQLite varint:** the G3 store fold removed six static instructions but did not reduce spills. Attack the 27 spills through allocator/liveness rather than adding more textual special cases.
6. **Linux find-bit:** investigate the current 159-instruction path versus GCC's 34-instruction ANDN/conditional shape. Use known-bits/bit-test reasoning and verify BMI baseline contracts.
7. **glibc memcmp:** scale the kernel and compare equal/mismatch positions before making timing claims; then inspect stack loads, word-loop register pressure and branch shape.
8. **Expat:** use the corrected CE Oracle with `cclang_trunk` as a separately recorded moving comparison. Preserve bit-test NaN/byte semantics and avoid Clang's oversized vectorization shapes.
9. **Non-reduction FP:** prototype ICX-style YMM/FMA accumulation for nbody/spectral/mandelbrot only with FP-contract, alias, alignment, trip-count and register-pressure proofs plus scalar fallback.
10. **Cost model interaction:** every candidate must report top improvements, top regressions, geomean, arithmetic mean, worst regression, code size, compile time and correctness status. A static instruction win with a spill or runtime regression is not accepted as a general win.

### P2 — oracle and infrastructure

11. Provision mold and wild when disk allows. Keep mold's exact requested CMake configuration: `-DMOLD_TARGETS='X86_64;I386'`, `-DMOLD_USE_MIMALLOC=OFF`, `-DMOLD_LTO=OFF`.
12. Keep `cg162`, `cclang2210`, `cclang_trunk`, `cicc2021100` and `cicxlatest` distinct. Record resolved compiler metadata in every report; never treat a moving channel as a fixed release.
13. Re-run only statistically promising candidates on the real i7-14700KF with P/E-core affinity, SMT/governor controls and PMU counters. VM timings are triage evidence, not hardware claims.
14. Snapshot immediately after every validated fix. Re-fetch/rebase `origin/main` before each new research batch and update the snapshot base only when the upstream merge is confirmed.
