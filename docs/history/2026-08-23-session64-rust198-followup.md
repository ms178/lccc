# Session 64 — Rust/Cargo 1.98, harness refresh, and follow-up gates

Date: 2026-08-23

## Baseline and continuity

- Latest fetched and rebased upstream main: `99c0c07bf5b069106a9b425087121ec2de90bb49`.
- The previous session's linker, versioning, Oracle, SQLite and glibc work was confirmed already merged by upstream in `87e5db1c` / PR #208.
- Local work is kept relative to the new upstream base. The persistent deliverable is `/home/user/ms178-1.patch`; the artifact ledger is under `/home/user/artifacts`.
- Snapshots made while establishing this session include `S03-session64-upstream-rebase`, `S04-session64-rust198-harness`, `S05-session64-tce-oracle`, `S06-session64-glibc-harness`, and `S07-session64-glibc-pipefail`.

## Completed items

### 1. Rust/Cargo upgraded to 1.98.0

The persisted rustup installation now reports:

```text
cargo 1.98.0 (797e8a9bc 2026-08-05)
rustc 1.98.0 (88d9e12ae 2026-08-18)
rustup 1.29.0
```

`rust-toolchain.toml` pins `1.98.0` with the minimal profile plus `rustfmt` and `clippy`. The installation lives in `/home/user/.cargo` and `/home/user/.rustup`, both inside the persisted workspace rather than `/opt`.

### 2. Build scripts now honor the exact toolchain

`build_lccc_fast.sh` and `build_lccc_o1_j2.sh` prefer the persisted Cargo and export `RUSTUP_TOOLCHAIN=1.98.0`. The LCCC project policy remains unchanged: build the compiler itself at Rust `-O1`; use exactly two Cargo jobs on the constrained machine.

A fastbuild completed successfully with the new toolchain:

```text
fastbuild profile: PASS
compiler binaries: lccc, lccc-x86, lccc-i686, lccc-ld
```

### 3. Arena restore harness was refreshed

`arena_session_restore.sh` no longer reinstalls a disposable stable Rust toolchain under `/opt`. It now:

- recreates/activates the 8 GiB swap file;
- provisions Rust/Cargo 1.98.0 in persisted `/home/user/.rustup` and `/home/user/.cargo`;
- exports the exact toolchain and PATH before building;
- retains the existing multilib/kernel package recovery and executable-bit repair; and
- builds LCCC through the fastbuild wrapper.

This removes the previous downgrade hazard where a harness reset silently restored an older image Cargo.

### 4. TCE regression reproducer was made defined C

`tests/regression/tce_loop_phi_param_retarget.c` used `s + (h & 1)` on the retry path. For the empty string this could form a one-past pointer and dereference it on the next recursive call, making the GCC/LCCC output difference an undefined-behavior artifact rather than a compiler defect.

The retry pointer now advances only when `*s != '\0'`. LCCC and GCC agree at `-O0`, `-O1`, `-O2` and `-O3`; the corrected regression is committed and snapshotted.

### 5. Full native regression corpus is green

After the defined reproducer was installed, the full runner completed:

```text
426 passed, 0 failed, 8 skipped-compare, 0 skipped-run, 434 total
```

The eight `skip-compare` cases are honest GCC-environment/feature skips (real-mode, selected glibc/assembler tests, and the spectral case); they are not LCCC failures. The i686 cases `i686_fused_mul_add_operand_order` and `segment_fill_copy_alias` pass when run through the current `lccc`/`lccc-i686` path with their exact flags.

### 6. Linker suite remains green under Rust 1.98

The complete current linker suite remained:

```text
160 pass, 0 fail, 0 warn, 0 skip
```

This covers the staged interpreter option, version metadata, shared objects, archives, TLS, ICF, `--gc-sections`, `--emit-relocs`, linker scripts, and malformed-input guards.

### 7. glibc workload harness was made restartable and pipefail-safe

The glibc 2.44 harness now has:

- explicit `BINUTILS` naming throughout;
- no `grep -q`/`readelf` SIGPIPE false failure under `set -o pipefail`;
- `GLIBC_REUSE_BUILD=1` recovery mode for post-build gate failures; and
- the existing exact tarball/patch/toolchain identity checks and capability gates.

A clean build/install completed before the post-install pipefail issue was fixed. The repaired reuse mode then completed the structural gate successfully, confirming staged `libc.so.6` contains versioned `_rtld_global_ro@GLIBC_PRIVATE`, `DT_VERNEED`, and a valid `GLIBC_PRIVATE` entry.

The default glibc configuration still deliberately uses:

```text
--disable-multi-arch
--disable-mathvec
--disable-sframe
--enable-kernel=3.2
```

Those are current LCCC capability/host-header limits, not hidden optimizations. They must not be removed without corresponding assembler/linker and runtime evidence.

### 8. Oracle refresh and current performance screen

Compiler Explorer was queried again. Current fixed/moving channels are:

```text
GCC 16.2       cg162
Clang 22.1.0   cclang2210
Clang trunk    cclang_trunk
ICC 2021.10.0  cicc2021100
ICX latest     cicxlatest
```

The seven-round VM screen of gzip CRC, zlib-ng Adler, Expat, SQLite varint, Linux find-bit and glibc memcmp was correctness-clean but still slower than GCC/Clang. Aggregate LCCC/GCC geometric ratio was `1.4330`; fastest-reference geometric ratio was `1.4976`. Important ratios:

| Kernel | LCCC/GCC | Main use |
|---|---:|---|
| gzip CRC | 1.092 | pointer/table addressing |
| zlib-ng Adler | 1.534 | accumulator homes and loop pressure |
| Expat scan | 2.085 | bit-test/classification shape |
| SQLite varint | 1.427 | branch lowering and 27 static spills |
| Linux find-bit | 1.271 | ffs/ANDN/conditional selection |
| glibc memcmp | 1.368 | word-loop RA and load/store scheduling |

All six outputs were checksum-correct. There is no usable perf PMU in this VM, so these are controlled pinned wall-clock screening values only.

## Remaining work — next priority order

### P0 correctness/runtime

1. Run the glibc harness with `GLIBC_RUN_SMOKE=1`. The staged custom-loader executable previously reached the loader but still failed with SIGSEGV 139 before output. Debug the first fault using gdb/strace, compare against GNU ld, and inspect `_r_debug`, `_rtld_global`, loader self-relocations, TLS setup, and the staged `DT_NEEDED`/version tables. Do not weaken the smoke gate.
2. Add a permanent small staged-libc runtime regression once the `_r_debug` fault is understood. It must exercise `malloc`, `printf`, `fma`, versioned imports and a direct `lccc-ld` final link.
3. Convert the eight GCC skips into an explicit capability matrix. A test may remain skipped only with a recorded reason; no skip may hide a compiler failure.
4. Run the full regression runner repeatedly at `-j2` after any linker/RA change. Preserve the exact `426/434` current baseline.

### P1 highest code-generation ROI

5. **SQLite varint:** reduce the 27 LCCC spills and six callee-saved pushes. Start with live-range segments/reload-at-next-use and accumulator homes. Compare `sqlite_get_varint` against GCC 16.2, Clang 22.1, ICC and ICX; require the SQLite workload checksum before timing.
6. **Linux find-bit:** identify why the 159-instruction LCCC helper is much larger than GCC's 34-instruction `andn`/`cmov` form. Check known-bits, conditional selection and bit-scan lowering; preserve baseline ISA safety and test CPUs without BMI1/BMI2.
7. **glibc memcmp:** attack stack traffic and word-loop register pressure only with aligned/unaligned, equal/mismatch-at-each-word and sanitizer coverage. The current kernel is too short/noisy for strong wall-clock conclusions without scaling its work.
8. **Expat:** preserve the existing bit-test correctness contract and investigate classifier inlining/branch layout. Do not copy Clang's large vectorized shapes blindly.
9. **Non-reduction FP loops:** nbody/spectral/mandelbrot remain the largest runtime gap. Prototype ICX-style YMM accumulator plus FMA only behind a strict alias/alignment/FP-contract legality proof; use register pressure in the cost model and keep scalar fallback.
10. **RA segments:** implement a small Wimmer-style reload-at-next-use spike behind a kill switch. Begin with the SQLite/zlib/Expat reproducers, then the full differential corpus. Do not enable graph coloring globally until RA-23 accumulator legality is moved into RA.

### P2 infrastructure/oracles

11. Provision mold/wild and record exact revisions. Preserve the requested mold build settings `-DMOLD_TARGETS='X86_64;I386'`, `-DMOLD_USE_MIMALLOC=OFF`, and `-DMOLD_LTO=OFF`.
12. Keep fixed CE channels and moving trunk/latest channels distinct in manifests. Every result must include resolved ID/name, source hash, flags, local LCCC revision and timestamp.
13. Re-run promising candidates on the real i7-14700KF with pinned P/E-core affinity, fixed governor, SMT policy and PMU counters. VM timing is screening evidence only.
14. Snapshot immediately after every validated source/script/doc change. Keep the patch base synchronized with the latest merged upstream main.

## Non-claims

This session does not claim that LCCC beats GCC, Clang, ICC or ICX on the six measured kernels. It does claim that the build/toolchain/harness baseline is now reproducible, the full native regression corpus has no failures under its documented skips, and the next performance gaps are measured and prioritized rather than guessed.
