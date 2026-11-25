# SQLite 3.54.1 full-project workload

This is a *bounded* end-to-end gate, not a claim that the entire SQLite/Tcl
suite ran or that VM timings predict a Raptor Lake CPU. It uses the official
SQLite release **tag** `version-3.54.1` from the project's GitHub mirror,
checks the pinned ZIP SHA-256 and the upstream `manifest.uuid`, and builds the
SQLite CLI and static library with **both** LCCC and GCC 16.2 at the same C
optimization/target flags. The `lccc-ld` binary independently links a third
CLI from LCCC objects. The active swap file is mandatory.

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

cd /home/user/lccc
source /home/user/.cache/lccc/agent-toolchains/env.sh
# If LCCC needs building: fastbuild profile (-O1), explicitly two jobs + swap.
LCCC_BUILD_JOBS=2 scripts/build_lccc_fast.sh
JOBS=2 tests/workloads/sqlite-3.54.1/run.sh
# Targeted SQLite varint C and code-shape regression (fast, independent):
tests/workloads/sqlite-3.54.1/check_ptr_delta_phi.sh
```

The runner downloads the pinned release ZIP into `$SQLITE_CACHE` (default
`~/source-cache`), validates entries before extracting into a **unique empty**
`$SQLITE_WORK_ROOT/run.XXXXXXXX` (default `~/workloads/sqlite-3.54.1`), and
writes JSON, SQL outputs, configure/build logs, and every unaveraged benchmark
run to `$SQLITE_ARTIFACT_DIR` (default `target/sqlite-3.54.1`). `work-path.txt`
locates the compiled executables. By default `KEEP_WORK=1` retains builds for
inspection; `KEEP_WORK=0` removes only the private run directory. Reruns never
unzip over existing files or prompt for overwrite. Everything in `target/` is
scratch and must not enter `ms178-1.patch`.

`SQLITE_CFLAGS` defaults to `-O2 -march=x86-64-v3 -mtune=raptorlake
-fomit-frame-pointer` **for SQLite**. The *compiler itself* is built under the
`fastbuild` preset (`-O1`, `-j2`), independently of these C flags. `GCC` defaults
to `gcc-16`; `LCCC` and `LCCC_LD` default to `target/fastbuild/*`. `JOBS` is
capped at 2. `SPEED_SIZE=25`, `SPEED_REPEATS=3` and
`SPEED_SETS='main cte rtree orm json fp'` are the default bounded upstream
speedtest1 matrix; each GCC/LCCC pair runs on fresh file-backed databases.
The script checks each pair's result hash, test identities, SQL and database
integrity **before** writing a `PASS` manifest. It reports raw per-test times
and medians separately, with no PMU/hardware-speed claim.

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
