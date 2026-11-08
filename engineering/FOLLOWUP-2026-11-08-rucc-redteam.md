# RUCC red-team audit follow-up — 2026-11-08

## Scope and identities

- LCCC under audit: `ms178/lccc` main/base `dcd1822c4c9f07ed4284eb96d0347a6bf97e03c1`, branch `arena/0fc3839e-lccc`.
- Competitor: `tamnd/rucc` `4f9f5d857349ffd8ddfdcdbd79d6dadb87dc50e9` (Apache-2.0), shallow clone at `/home/user/rucc-review`.
- Toolchains: exclusively the hash-verified `ms178/lccc` `harness-cache-8` tag (`d66c5ac881e2168790b8e228dd548f025ab83e59`), restored by `scripts/restore_agent_toolchain_cache.py`; 12 files verified. Rust/Cargo 1.99.1, GCC 16.2.0, Clang 23.1.5, binutils 2.47.20260726, mold 3.0.0, GDB 18.1, QEMU user 11.1.3.
- Host: two vCPUs, 3.8 GiB RAM, exactly 4 GiB active `/swapfile`, Intel Xeon @ 2.60 GHz virtual host. **No physical i7-14700KF and no hardware PMU evidence.**
- LCCC build policy: `fastbuild`, Rust `-O1`, LTO off, incremental, 256 codegen units, debug=0, exactly `-j2`, cached Clang driver + mold. Baseline build completed in 3m46s; baseline binaries preserved in `/home/user/lccc-baseline-dcd1822/`.
- Rucc audit build: release profile overridden to `-O1`, LTO off, 256 codegen units, debug=0, incremental, `-j2`, mold; completed in 1m44s. crates.io is unreachable, so all 13 locked dependencies were replaced by GitHub source pins whose manifest versions match the lock exactly (`/home/user/rucc-deps/PINS.txt`). This is **not** the official release asset and **not** a crates.io checksum-identical build. The GitHub release asset route returned an HTML redirect outside the permitted hosts and was discarded.

## What was measured

Correctness screening ran the existing `tests/benchmark/run_benchmarks.py` corpus against LCCC, GCC 16.2, Clang 23.1.5 and the pinned rucc build. Twelve workload-derived kernels passed output equivalence for all four compilers: gzip CRC-32, zlib-ng Adler-32, Expat XML scan, Expat SipHash-2-4, SQLite varint, SQLite VDBE serial, Linux find-bit, Linux rbtree, glibc memcmp, glibc strstr, gzip longest-match and SHA-256 transform.

Timed screening used `-O2`, seven measured rounds, two excluded warm-ups, CPU0 pinning, randomized compiler order, retained MAD outliers, wall clock plus child CPU time, and no PMU probe. Raw report: `/home/user/rucc-timed.json` (676,555 bytes, SHA-256 recorded in `engineering/evidence/2026-11-08-rucc-audit/summary.json`); artifacts: `/home/user/rucc-timed-artifacts/`. The committed summary is deliberately below the 100 KiB bulk-evidence threshold.

Median wall times (ms) and paired LCCC/reference ratios from the runner:

| Workload | LCCC | GCC | Clang | rucc | LCCC/GCC | LCCC/Clang | LCCC/rucc |
|---|---:|---:|---:|---:|---:|---:|---:|
| gzip_crc32 | 153.62 | 169.21 | 170.33 | 169.27 | 0.904 | 0.897 | 0.902 |
| zlib_ng_adler32 | 40.15 | 39.81 | 37.73 | 41.98 | 1.014 | 1.063 | 0.956 |
| expat_xml_scan | 45.69 | 43.05 | 40.40 | 41.99 | 1.083 | 1.143 | 1.117 |
| expat_siphash24 | 264.70 | 301.66 | 280.66 | 333.96 | 0.869 | 0.981 | 0.784 |
| sqlite_varint | 29.61 | 27.33 | 28.16 | 27.76 | 1.084 | 1.062 | 1.021 |
| linux_find_bit | 16.47 | 15.39 | 15.10 | 15.02 | 1.067 | 1.091 | 1.114 |
| glibc_memcmp | 11.25 | 10.91 | 10.92 | 10.53 | 1.026 | 1.030 | 1.092 |
| sha256_transform | 296.12 | 313.39 | 349.43 | 420.54 | 0.995 | 0.931 | 0.812 |
| linux_rbtree | 16.86 | 16.66 | 16.44 | 16.32 | 0.981 | 1.001 | 1.044 |
| glibc_strstr | 4227.53 | 4234.68 | 4507.57 | 4522.51 | 0.998 | 0.935 | 0.932 |
| gzip_longest_match | 159.68 | 154.61 | 144.37 | 157.58 | 1.032 | 1.122 | 1.019 |
| sqlite_vdbe_serial | 21.57 | 17.77 | 17.51 | 18.27 | 1.221 | 1.233 | 1.189 |

Aggregate paired-median geometric means: LCCC/GCC **1.0192**, LCCC/Clang **1.0365**, LCCC/rucc **0.9911**. LCCC is faster than rucc on 5/12 and slower on 7/12; the near-unit aggregate is **not** a decisive overall win. The defensible conclusion is workload-dependent: LCCC is materially ahead on SHA-256 and SipHash ARX kernels and glibc strstr, while rucc/Clang/GCC are ahead on SQLite VDBE serial, Expat XML scan and Linux find-bit. Short cases (<20 ms) carry launch/scheduler noise; several runs have >5% CV and retained outliers. These are VM screening numbers, not Raptor Lake microarchitectural claims.

Static disassembly was retained per compiler/workload. Instruction counts are structural only, not dynamic counts. Examples: LCCC lowers Linux's generic `__ffs` decision tree to `bsf`, while rucc emits the branch/cmov tree; LCCC nevertheless loses the find-bit workload, so the gap is not explained by that lowering alone. LCCC's `scan_all_records` rematerializes the FNV prime inside the byte loop and keeps the outer record index in a stack slot; GCC/Clang/rucc differ in hoisting and register pressure. `gzip_longest_match` main is 385 LCCC instructions vs 320–324 for the references. These are leads, not yet validated optimizations.

## Concrete defect found and fixed

LCCC returned unconditional `0` for every `__has_feature` / `__has_extension` in both `#if` and ordinary code. GCC 16.2 and Clang 23.1.5 answer `1` for implemented C11/C23 capabilities, tls, asm-goto extensions and active UBSan rows; headers such as FreeBSD's `sys/cdefs.h` and Linux feature probes use these answers to select code paths. The blanket zero silently selects fallbacks.

The fix adds `src/frontend/preprocessor/feature_queries.rs`: a GCC-16.2-mode capability table gated on LCCC's *implemented* capabilities, wired through the driver-selected dialect, `-pedantic-errors` (not `-Werror=pedantic`), and active UBSan checks. It handles armoured spellings (`__c_atomic__`), macro aliases, `#if` and code-line contexts. Rows for unimplemented deprecated/unavailable/enumerator attribute diagnostics and absent address/hwaddress/leak/thread sanitizers deliberately remain `0`. Regression coverage is in `tests/regression/check_frontend_preprocessor_rules.sh` section 10 and Rust unit tests; the measured GCC/Clang matrix is retained in `engineering/evidence/2026-11-08-rucc-audit/feature-query-oracle.json`.

The capability-row approach was informed by rucc's Apache-2.0 GNU matrix and its truthfulness rule ("answering untruthfully is worse than answering no"). **No rucc source was copied.** LCCC is CC0; a future verbatim Apache-2.0 transplant must carry its own license/notice rather than being silently relicensed.

Also fixed the fastbuild wrapper's explicit job override: `LCCC_BUILD_JOBS=2` is no longer silently reduced by the low-RAM heuristic, and `ci_local.sh`/hosted Build now request it explicitly. The default unattended path remains memory-bounded; the research path uses the installed 4 GiB swap as an OOM backstop, not a speed claim.

## Rucc ideas worth keeping / not worth wholesale porting

- Worth adopting as discipline: capability queries must be truth-gated; rewrite rules should carry proof status and a solver-required CI lane; unverified/bounded-width rules must be inventoried separately. Rucc's verifier CI installs Z3 5.1.0, sets `RUCC_REQUIRE_SOLVER=1`, and checks rule directories against `docs/UNVERIFIED.md`; nine rules are proved only at 4/8 bits. Its build scripts generate matchers but do not prove rules.
- LCCC already has extensive proof, differential, fuzz, peephole-invariance and benchmark-output infrastructure. Wholesale porting rucc's optimizer/e-graph/register allocator is not justified by this evidence and would be high-risk. Targeted transplants should be driven by a reproducer and an A/B measurement.
- Rucc's optional e-graph rewriter is not the default pipeline; default selection is driven by `-Zrewriter=` and the level pipeline. Do not describe rucc as "e-graph optimized by default" without tracing the selected arm.
- Rucc's safety monitor and litmus/replay harnesses are interesting correctness assets, but they depend on rucc IR/runtime conventions; port the *test shapes* first, not the monitor wholesale.

## Known gaps and limitations

- LCCC accepts but does not yet diagnose `deprecated("msg")`, `unavailable("msg")` and enumerator attributes in the probes run here; the corresponding feature rows stay `0` until sema implements them. `unavailable` use should be an error, not a silent accept.
- ICX/ICC, LLVM `ld.lld`, `clang++`, `qemu-x86_64` and a working `-m32` environment are absent from the verified cache/sandbox. Compiler Explorer failed with three TLS EOFs. No ICX/lld/CE ranking is claimed.
- mold 3.0.0 is a Rust/Cargo build; the historical CMake `MOLD_TARGETS` preset no longer exists. The x86-only feature set is `x86_64,i386,system-allocator`. Linker-oracle setup still does not reuse the verified cached mold and checks existence rather than version in places; this remains a prioritized script fix.
- Rucc's official release binary was not obtained; the audit binary is a pinned source build with GitHub dependency substitutions. Dependency tags match locked versions but are not crates.io archive checksums.
- No compiler miscompile was established in the twelve-kernel screening. Screening is not exhaustive correctness proof.
- Full `ms178/archpkgbuilds` project builds (gzip, zlib-ng, expat, SQLite, Linux, glibc) were not run; the existing workload-derived excerpts were used. Full-project extraction/build is remaining work.

## Prioritized remaining work

1. Validate the feature-query fix and job-override change with targeted tests, `ci_local.sh --fast`, and hosted PR CI; snapshot each validated tree into `/home/user/ms178-1.patch`.
2. Reduce the SQLite VDBE serial gap: reproduce the inner FNV byte loop, test constant hoist/rematerialization and outer-index register pressure, and require output-equivalence plus paired A/B timing before landing.
3. Reduce Linux find-bit overhead: investigate the unnecessary callee-saved `%r14` push/pop and indexed load folding in the zero-word loop; keep the existing `bsf` lowering.
4. Investigate Expat XML scan and gzip longest-match branch/layout and inlining differences against Clang/GCC.
5. Implement deprecated/unavailable/enumerator attribute diagnostics, then flip the truthful feature rows to `1` with differential tests.
6. Make linker-oracle setup reuse the hash-verified cached mold/binutils and assert exact preferred versions; keep the X86/i686 Cargo feature preset and record revisions.
7. Extend the corpus with pinned `ms178/archpkgbuilds` excerpts/full builds, especially zlib-ng, gzip, expat, SQLite, Linux and glibc, preserving provenance and license boundaries.
8. Re-run the timed corpus on the physical i7-14700KF with verified PMU counters before making any Raptor Lake or "beats GCC/Clang/ICX" claim.
9. Consider porting rucc's solver-required rule-proof CI shape and bounded-width inventory to LCCC's existing proof gates, only where it closes a demonstrated coverage gap.

## Reproduction pointers

- Cache restore: `python3 scripts/restore_agent_toolchain_cache.py --restore` (lock: `engineering/toolchains/agent-cache-lock.json`).
- LCCC build: `source /home/user/.cache/lccc/agent-toolchains/env.sh && LCCC_BUILD_JOBS=2 ./scripts/build_lccc_fast.sh`.
- Correctness/timing: `tests/benchmark/run_benchmarks.py --candidate rucc=/home/user/rucc-audit-bin/rucc-4f9f5d85 --compilers lccc,gcc,clang,rucc --reps 7 --warmup 2 --cpu 0 --no-pmu-probe ...`.
- Feature gate: `CCC=target/fastbuild/lccc bash tests/regression/check_frontend_preprocessor_rules.sh`.
- Local CI mirror: `./scripts/ci_local.sh --fast`; snapshot: `scripts/lccc-snapshot.sh` into `/home/user/ms178-1.patch`.
