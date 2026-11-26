# SQLite 3.54.1 full-project workload

See the dated [validation record](RESULTS.md) for observed pass/fail results
and measurement limits; the scripts here are the reproducible gates.

This is a *bounded* end-to-end gate, not a claim that the entire SQLite/Tcl
suite ran or that VM timings predict a Raptor Lake CPU. It uses the official
SQLite release **tag** `version-3.54.1` from the project's GitHub mirror,
checks the pinned ZIP SHA-256 and the upstream `manifest.uuid`, and builds the
SQLite CLI and static library with **both** LCCC and GCC 16.2 at the same C
optimization/target flags. The `lccc-ld` binary independently links a third
CLI from LCCC objects. The active swap file is mandatory; the repository's
`ensure_swap.sh` now provisions **8 GiB**, since 4 GiB was measured insufficient
for the monolithic Rust lib-test link on a small host.

`ms178/archpkgbuilds` supplies the pinned SQLite recipe and its compile-time
flags. Its `PKGBUILD` at `0516be29c004aa2a64e8a66ba84a3eda154ccd6e`
targets **3.54, not 3.54.1**: its Lemon patch has the old source layout. The
runner checks the recipe and patch hashes, records the failed exact dry run,
then applies the equivalent release-specific [`lemon-system-template.patch`](lemon-system-template.patch)
with `--fuzz=0`. The recipe's Clang PGO/BOLT stages are deliberately *not*
used to compile SQLite with LCCC. Archive and generated files stay outside Git.

## Reproduce

```sh
# Run once (sparse extraction of the package recipe only):
git clone --depth 1 --filter=blob:none --sparse \
  https://github.com/ms178/archpkgbuilds.git /home/user/archpkgbuilds
git -C /home/user/archpkgbuilds sparse-checkout set packages/sqlite
git -C /home/user/archpkgbuilds fetch --depth 1 origin \
  0516be29c004aa2a64e8a66ba84a3eda154ccd6e
git -C /home/user/archpkgbuilds checkout --detach \
  0516be29c004aa2a64e8a66ba84a3eda154ccd6e

cd /home/user/lccc
source /home/user/.cache/lccc/agent-toolchains/env.sh
# If LCCC needs building: fastbuild profile (-O1), explicitly two jobs + swap.
LCCC_BUILD_JOBS=2 scripts/build_lccc_fast.sh
JOBS=2 tests/workloads/sqlite-3.54.1/run.sh
# Targeted SQLite varint and guarded-negative-control C regressions:
tests/workloads/sqlite-3.54.1/check_ptr_delta_phi.sh
# Four supported architectures: -Werror/SSA/codegen, plus i686 runtime on
# IA32-compatible hosts. For AArch64/RISC-V runtime, add pinned QEMU from the
# repo's hash-verified harness-cache-8 tag to PATH (no cross sysroot needed):
python3 scripts/restore_agent_toolchain_cache.py --tag harness-cache-8 \
  --only gdb,qemu --prefix /home/user/.cache/lccc/agent-debug-tools
PATH=/home/user/.cache/lccc/agent-debug-tools/bin:$PATH \
  tests/workloads/sqlite-3.54.1/check_ptr_delta_phi_cross.sh
# Interleaved A/B (same compiler, pass enabled vs disabled), with the
# independently checked SQLite decoder and a reproducible 4096-input corpus.
python3 scripts/perf_ab.py --env CCC_DISABLE_PASSES=ptr_delta_phi \
  --only ptr_delta_phi_bench --reps 11 --cflag=-march=x86-64-v3 \
  --json target/sqlite-3.54.1/varint-ab.json \
  tests/workloads/sqlite-3.54.1/ptr_delta_phi_bench.c
# Two backward-step negative controls: Sub(base, 7) and GEP/Add(base, -7)
# are intentionally NOT folded. Experimental variants of each regressed host
# timing despite smaller x86 assembly. Their final on/off code is identical.
python3 scripts/perf_ab.py --env CCC_DISABLE_PASSES=ptr_delta_phi \
  --only ptr_delta_phi_negative_bench --reps 15 --cflag=-march=x86-64-v3 \
  --json target/sqlite-3.54.1/negative-control-ab.json \
  tests/workloads/sqlite-3.54.1/ptr_delta_phi_negative_bench.c
python3 scripts/perf_ab.py --env CCC_DISABLE_PASSES=ptr_delta_phi \
  --only ptr_delta_phi_negative_add_bench --reps 15 --cflag=-march=x86-64-v3 \
  --json target/sqlite-3.54.1/negative-add-control-ab.json \
  tests/workloads/sqlite-3.54.1/ptr_delta_phi_negative_add_bench.c

# Controlled full-project comparison: SAME LCCC binary/C flags/source.
# Keep both builds so the paired gate can run each on fresh databases.
SQLITE_ARTIFACT_DIR=target/sqlite-3.54.1-enabled JOBS=2 tests/workloads/sqlite-3.54.1/run.sh
CCC_DISABLE_PASSES=ptr_delta_phi SQLITE_ARTIFACT_DIR=target/sqlite-3.54.1-disabled \
  JOBS=2 tests/workloads/sqlite-3.54.1/run.sh
python3 tests/workloads/sqlite-3.54.1/paired_speedtest.py \
  --enabled target/sqlite-3.54.1-enabled --disabled target/sqlite-3.54.1-disabled \
  --repeats 7 --out target/sqlite-3.54.1/paired-speedtest.json
# Optional third whole-project reference on the retained, pinned sources:
python3 tests/workloads/sqlite-3.54.1/compare_clang.py \
  --baseline target/sqlite-3.54.1-enabled --out target/sqlite-3.54.1-clang
```

The runner downloads the pinned release ZIP into `$SQLITE_CACHE` (default
`~/source-cache`), validates entries before extracting into a **unique empty**
`$SQLITE_WORK_ROOT/run.XXXXXXXX` (default `~/workloads/sqlite-3.54.1`), and
writes JSON, SQL outputs, configure/build logs, and every unaveraged benchmark
run to `$SQLITE_ARTIFACT_DIR` (default `target/sqlite-3.54.1`). `work-path.txt`
locates the compiled executables. By default `KEEP_WORK=1` retains sources and
builds for inspection, but **verified speedtest databases are removed after
integrity checking** (raw output, hashes and timings remain). `KEEP_WORK=0`
removes the private run directory. Reruns never unzip over existing files or
prompt for overwrite. Everything in `target/` is
scratch and must not enter `ms178-1.patch`.

`SQLITE_CFLAGS` defaults to `-O2 -march=x86-64-v3 -mtune=raptorlake
-fomit-frame-pointer` **for SQLite**. The *compiler itself* is built under the
`fastbuild` preset (`-O1`, `-j2`), independently of these C flags. `GCC` defaults
to `gcc-16`; `LCCC` and `LCCC_LD` default to `target/fastbuild/*`. `JOBS` is
capped at 2. `SPEED_SIZE=25`, `SPEED_REPEATS=3` and
`SPEED_SETS='main cte rtree orm json fp'` are the default bounded upstream
speedtest1 matrix; each GCC/LCCC pair runs on fresh file-backed databases.
The script checks each pair's result hash, test identities, SQL and database
integrity and scans every configure/build/link log for warnings **before**
writing a `PASS` manifest; reruns invalidate stale PASS manifests at startup.
It reports raw per-test times and medians separately, with no
PMU/hardware-speed claim. The separate
`ptr_delta_phi_bench.c` includes the 3.54.1 decoder fixture verbatim, runs the
90,000-case independent oracle before every timing, and then calls the decoder
on a cache-resident, mixed-length corpus. `perf_ab.py` alternates A/B order and
requires zero exit status, empty runtime stderr and identical complete stdout
on *every* sample. Its VM times are a screen, not PMU instruction counts or
proof of end-to-end speedup. For full-project A/B,
`paired_speedtest.py` refuses different compiler hashes, source, flags,
verification hashes or pass settings, alternates the **measurement** order, and
runs `integrity_check` on every fresh database. It saves each raw run and
reports both pooled medians and medians of paired ratios; treat small VM
differences as noise, not as CPU-cycle or native Raptor Lake evidence. The
four-target checker proves the **varint** pointer-subtraction instruction
disappears from x86-64, i686, AArch64 and RISC-V 64 output with the pass.
It also checks that guarded `p -= 7` and `p += -7` pointer-phi **negative
controls** retain their original codegen in both modes: broader folds were
functionally sound but *slower* in separate interleaved x86 host screens, so
static shrinkage is not enough. On compatible Linux/x86-64 hosts, it runs
90,000 varints and 100,000 guarded deltas (50,000 per backward encoding) in
freestanding **i686** binaries built with GCC, Clang and LCCC pass-on/off
against a separate GCC-compiled oracle. When pinned QEMU user-mode emulators
are on PATH, it executes the same cases on **AArch64 and RISC-V 64** with
Clang/LCCC pass-on/off against a separately Clang-compiled oracle.
Their emulated functional results do **not** prove native-target speed.
If QEMU is missing the script explicitly reports those runtimes as skipped.
The optional `compare_clang.py` builds the same retained, patched release with
Clang 23, checks warning-free configure/build/link, compares both SQL cases to
the GCC baseline and verifies all six upstream speedtest hashes, identities
and database integrity. Its single VM run per testset is **not** a performance
claim. Local ICX is not provisioned by this workload; run the repository's
hosted authenticated ICX integration separately when available.

The two deterministic SQL cases cover indexed joins, aggregates, windows,
triggers, rollback, foreign keys, strict tables, upsert, JSON, FTS5, R-tree,
recursive queries and integrity. The upstream `speedtest1 --verify` exercises
six additional workloads. `make quicktest` / the Tcl testfixture are **not**
run on this host (`tclsh` is not installed). This gate therefore does not
establish the correctness of *all* SQLite functionality, only the stated tests.

For a local code-generation comparison without external Compiler Explorer,
`scripts/godbolt.py compare --offline --oracles '' --local-ref gcc16=gcc-16
--local-ref clang23=clang-23 --function FUNCTION SOURCE` compares the same C
source through the repository's Godbolt-derived oracle and records compiler
identities. ICC Classic and ICX are not installed in this sandbox, and the
external CE API is outside the allowed network; no results are attributed to
them. The i686/mold preset is also not a measured native SQLite execution.
