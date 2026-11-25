#!/usr/bin/env bash
# SQLite 3.54.1: pinned source, archpkgbuilds configuration, differential SQL
# and upstream speedtest1. No downloaded/generated source is stored in Git.
set -euo pipefail

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd)
LCCC=${LCCC:-$ROOT/target/fastbuild/lccc}
LCCC_LD=${LCCC_LD:-$(dirname "$LCCC")/lccc-ld}
GCC=${GCC:-gcc-16}
RECIPE_DIR=${SQLITE_RECIPE_DIR:-/home/user/archpkgbuilds/packages/sqlite}
CACHE=${SQLITE_CACHE:-$HOME/source-cache}
WORK_ROOT=${SQLITE_WORK_ROOT:-$HOME/workloads/sqlite-3.54.1}
ARTIFACT_DIR=${SQLITE_ARTIFACT_DIR:-$ROOT/target/sqlite-3.54.1}
KEEP_WORK=${KEEP_WORK:-1}
JOBS=${JOBS:-2}
SPEED_SIZE=${SPEED_SIZE:-25}
SPEED_REPEATS=${SPEED_REPEATS:-3}
SPEED_SETS=${SPEED_SETS:-'main cte rtree orm json fp'}
# -O1 applies to building LCCC itself, not to the C code being evaluated.
SQLITE_CFLAGS=${SQLITE_CFLAGS:--O2 -march=x86-64-v3 -mtune=raptorlake -fomit-frame-pointer}
VERSION=3.54.1
TAG=version-$VERSION
ARCHIVE_SHA256=a9b05054c19e5649f11d8af849cb1fc1b2311eb6f211b96b244fb5d5e8368cef
FOSSIL_ID=555e07ef33f69a0957d24d8e555b702464a131d6f1607f4c3c1e4551e8e6ca76
RECIPE_COMMIT=0516be29c004aa2a64e8a66ba84a3eda154ccd6e
RECIPE_SHA256=74e7677d9728114cf6a6d2caca42942f40454824e839dabf0922a80995212ed1
RECIPE_PATCH_SHA256=55746d93b0df4b349c4aa4f09535746dac3530f9fd6de241c9f38e2c92e8ee97

for tool in curl unzip patch sha256sum python3 make timeout "$GCC"; do
    command -v "$tool" >/dev/null || { echo "missing required command: $tool" >&2; exit 2; }
done
[[ -x $LCCC && -x $LCCC_LD ]] || { echo 'LCCC and lccc-ld must be executable' >&2; exit 2; }
[[ $(wc -l </proc/swaps) -gt 1 ]] || { echo 'an active swap file is required' >&2; exit 2; }
[[ $JOBS =~ ^[12]$ && $SPEED_SIZE =~ ^[1-9][0-9]*$ && $SPEED_REPEATS =~ ^[1-9][0-9]*$ ]] || {
    echo 'JOBS must be 1 or 2, SPEED_SIZE and SPEED_REPEATS positive integers' >&2; exit 2;
}
[[ $KEEP_WORK == 0 || $KEEP_WORK == 1 ]] || { echo 'KEEP_WORK must be 0 or 1' >&2; exit 2; }
[[ -f $RECIPE_DIR/PKGBUILD && -f $RECIPE_DIR/sqlite-lemon-system-template.patch ]] || {
    echo 'clone ms178/archpkgbuilds (sparse packages/sqlite) and set SQLITE_RECIPE_DIR' >&2; exit 2;
}
check_sha() {
    local actual
    actual=$(sha256sum "$2" | cut -d' ' -f1)
    [[ $actual == "$1" ]] || { echo "checksum mismatch: $2: $actual (expected $1)" >&2; exit 1; }
}
check_sha "$RECIPE_SHA256" "$RECIPE_DIR/PKGBUILD"
check_sha "$RECIPE_PATCH_SHA256" "$RECIPE_DIR/sqlite-lemon-system-template.patch"
if [[ -d $RECIPE_DIR/../../.git ]]; then
    [[ $(git -C "$RECIPE_DIR/../.." rev-parse HEAD) == "$RECIPE_COMMIT" ]] || {
        echo "archpkgbuilds HEAD differs from pinned $RECIPE_COMMIT" >&2; exit 1;
    }
fi
# Recipe flags are extracted from the exact hashed archpkgbuilds PKGBUILD,
# rather than retyped in this runner. Its PGO/BOLT stages are intentionally not
# used: those are for Clang, not for compiling SQLite with LCCC.
CPPFLAGS=$(awk '/^_sqlite_cppflags=\(/,/^\)/ { if ($1 ~ /^-D/) printf "%s ", $1 }' "$RECIPE_DIR/PKGBUILD")
[[ $CPPFLAGS == *SQLITE_ENABLE_STAT4* ]] || { echo 'recipe flags could not be read' >&2; exit 1; }
mkdir -p "$CACHE" "$WORK_ROOT" "$ARTIFACT_DIR"
if ! python3 "$ROOT/tests/workloads/sqlite-3.54.1/test_summarize.py" \
    > "$ARTIFACT_DIR/summary-selftest.log" 2>&1; then
    cat "$ARTIFACT_DIR/summary-selftest.log" >&2
    exit 1
fi
archive="$CACHE/sqlite-$TAG.zip"
if [[ ! -f $archive ]]; then
    tmp=$(mktemp "$CACHE/.sqlite-$TAG.XXXXXX")
    trap 'rm -f -- "$tmp"' EXIT
    curl -fL --retry 2 --connect-timeout 20 --max-time 180 \
        "https://github.com/sqlite/sqlite/archive/refs/tags/$TAG.zip" -o "$tmp"
    check_sha "$ARCHIVE_SHA256" "$tmp"
    mv -- "$tmp" "$archive"
    trap - EXIT
fi
check_sha "$ARCHIVE_SHA256" "$archive"
# Verify the archive before extraction (including duplicate entries, links,
# escaping paths and an upper bound on decompressed size). An empty unique work
# directory means unzip can never prompt for overwrite even on a rerun.
python3 - "$archive" "sqlite-$TAG" <<'PY'
import pathlib, stat, sys, zipfile
archive, prefix = sys.argv[1:]
with zipfile.ZipFile(archive) as z:
    members = z.infolist()
    if not members or len(members) >= 15000 or sum(i.file_size for i in members) >= 400_000_000:
        raise SystemExit('SQLite source archive exceeds resource bounds')
    names = set()
    for item in members:
        p = pathlib.PurePosixPath(item.filename)
        mode = stat.S_IFMT(item.external_attr >> 16)
        if (not p.parts or p.parts[0] != prefix or p.is_absolute()
                or '..' in p.parts or '\\' in item.filename
                or item.filename in names
                or mode not in (0, stat.S_IFDIR, stat.S_IFREG)):
            raise SystemExit('unsafe archive entry: ' + item.filename)
        names.add(item.filename)
PY
WORK=$(mktemp -d "$WORK_ROOT/run.XXXXXXXX")
printf '%s\n' "$WORK" > "$ARTIFACT_DIR/work-path.txt"
cleanup() { [[ $KEEP_WORK == 1 ]] || rm -rf -- "$WORK"; }
trap cleanup EXIT
trap 'rc=$?; echo "SQLite workload FAILED (exit $rc; work: $WORK; artifacts: $ARTIFACT_DIR)" >&2; for f in "$ARTIFACT_DIR"/build-*.log "$ARTIFACT_DIR"/configure-*.log; do if [[ -f $f ]]; then echo "--- $f ---" >&2; tail -25 "$f" >&2; fi; done' ERR
unzip -q "$archive" -d "$WORK"
SRC="$WORK/sqlite-$TAG"
[[ $(cat "$SRC/VERSION") == "$VERSION" && $(cat "$SRC/manifest.uuid") == "$FOSSIL_ID" ]] || {
    echo 'SQLite release identity mismatch' >&2; exit 1;
}
# archpkgbuilds' Lemon patch targets an earlier source layout. Require exact
# dry-run success for the pinned, semantically identical release-specific fix.
COMPAT_PATCH="$ROOT/tests/workloads/sqlite-3.54.1/lemon-system-template.patch"
if patch --dry-run -Np1 --fuzz=0 -i "$RECIPE_DIR/sqlite-lemon-system-template.patch" -d "$SRC" > "$ARTIFACT_DIR/recipe-patch-dry-run.log" 2>&1; then
    PATCH="$RECIPE_DIR/sqlite-lemon-system-template.patch"
    PATCH_STATUS=recipe-applied
else
    PATCH="$COMPAT_PATCH"
    PATCH_STATUS=recipe-stale-compat-applied
    patch --dry-run -Np1 --fuzz=0 -i "$PATCH" -d "$SRC" > "$ARTIFACT_DIR/compat-patch-dry-run.log"
fi
patch -Np1 --fuzz=0 -i "$PATCH" -d "$SRC" > "$ARTIFACT_DIR/lemon-patch.log"

for key in lccc gcc; do
    cc=$LCCC; [[ $key == gcc ]] && cc=$GCC
    build="$WORK/build-$key"
    mkdir -p "$build"
    echo "Configuring and building SQLite $VERSION with $key ($cc)"
    (
        cd "$build"
        CC="$cc" CPPFLAGS="$CPPFLAGS" CFLAGS="$SQLITE_CFLAGS" \
          LDFLAGS='-Wl,-z,relro -Wl,-z,now' \
          "$SRC/configure" --disable-shared --disable-tcl --disable-readline \
          --fts5 --rtree --column-metadata --session \
          > "$ARTIFACT_DIR/configure-$key.log" 2>&1
        timeout --kill-after=10s 1200 make -j"$JOBS" sqlite3 libsqlite3.a \
          > "$ARTIFACT_DIR/build-$key.log" 2>&1
        # speedtest1 gates its R-tree cases on this preprocessor symbol;
        # linking an R-tree-enabled library alone does not enable the tests.
        "$cc" $SQLITE_CFLAGS $CPPFLAGS -DSQLITE_ENABLE_RTREE -I. -I"$SRC" \
          "$SRC/test/speedtest1.c" libsqlite3.a -o speedtest1 \
          -ldl -lpthread -lm > "$ARTIFACT_DIR/speedtest-build-$key.log" 2>&1
    )
done
# The second CLI uses precisely the LCCC amalgamation, but links it through
# the standalone linker and system CRT (not the compiler driver's link path).
(
    cd "$WORK/build-lccc"
    "$LCCC" -DNDEBUG -fPIC $SQLITE_CFLAGS $CPPFLAGS \
        -D_HAVE_SQLITE_CONFIG_H -DBUILD_sqlite -I. -I"$SRC/src" \
        -I"$SRC/ext/rtree" -I"$SRC/ext/icu" -I"$SRC/ext/fts3" \
        -I"$SRC/ext/session" -I"$SRC/ext/misc" \
        -c shell.c -o shell-lccc-ld.o \
        > "$ARTIFACT_DIR/shell-ld-build.log" 2>&1
    gcc_dir=$(dirname "$($GCC -print-file-name=libgcc.a)")
    gcc_s_dir=$(dirname "$($GCC -print-file-name=libgcc_s.so)")
    "$LCCC_LD" -o sqlite3-lccc-ld \
        /usr/lib/x86_64-linux-gnu/Scrt1.o /usr/lib/x86_64-linux-gnu/crti.o \
        "$($GCC -print-file-name=crtbeginS.o)" shell-lccc-ld.o sqlite3.o \
        --dynamic-linker=/lib64/ld-linux-x86-64.so.2 \
        -L"$gcc_dir" -L"$gcc_s_dir" -L/usr/lib/x86_64-linux-gnu \
        -ldl -lpthread -lm -lgcc -lgcc_s -lc \
        "$($GCC -print-file-name=crtendS.o)" /usr/lib/x86_64-linux-gnu/crtn.o \
        > "$ARTIFACT_DIR/ld-build.log" 2>&1
)
for test in relational features; do
    input="$ROOT/tests/workloads/sqlite-3.54.1/$test.sql"
    for key in lccc gcc ld; do
        bin="$WORK/build-$key/sqlite3"
        [[ $key == ld ]] && bin="$WORK/build-lccc/sqlite3-lccc-ld"
        timeout --kill-after=5s 120 "$bin" -batch -bail :memory: < "$input" \
            > "$ARTIFACT_DIR/sql-$test-$key.txt" 2> "$ARTIFACT_DIR/sql-$test-$key.err"
        grep -q '^PASS|' "$ARTIFACT_DIR/sql-$test-$key.txt" || {
            echo "missing SQL invariants for $test/$key" >&2; exit 1;
        }
        ! grep -q '^FAIL|' "$ARTIFACT_DIR/sql-$test-$key.txt" || {
            echo "incorrect SQL invariants for $test/$key" >&2; exit 1;
        }
        grep -qx ok "$ARTIFACT_DIR/sql-$test-$key.txt" || {
            echo "integrity_check did not report ok for $test/$key" >&2; exit 1;
        }
    done
    diff -u "$ARTIFACT_DIR/sql-$test-gcc.txt" "$ARTIFACT_DIR/sql-$test-lccc.txt" \
        > "$ARTIFACT_DIR/sql-$test-gcc-vs-lccc.diff" || {
            echo "GCC/LCCC SQL mismatch ($test); see artifact diff" >&2; exit 1;
        }
    diff -u "$ARTIFACT_DIR/sql-$test-lccc.txt" "$ARTIFACT_DIR/sql-$test-ld.txt" \
        > "$ARTIFACT_DIR/sql-$test-driver-vs-ld.diff" || {
            echo "LCCC driver/standalone-linker SQL mismatch ($test)" >&2; exit 1;
        }
done
# Upstream's own speedtest1 provides a verification hash independent of elapsed
# time. Interleave GCC and LCCC to reduce drift; keep *raw* per-run times and
# reject a mismatch before interpreting timings. All DBs are independent.
for testset in $SPEED_SETS; do
    [[ $testset =~ ^[a-z0-9]+$ ]] || { echo "invalid testset: $testset" >&2; exit 2; }
    for ((rep=1; rep<=SPEED_REPEATS; rep++)); do
        for key in gcc lccc; do
            out="$ARTIFACT_DIR/bench-$testset-$rep-$key.txt"
            db="$WORK/speed-$testset-$rep-$key.db"
            timeout --kill-after=5s 180 "$WORK/build-$key/speedtest1" \
                --size "$SPEED_SIZE" --nosync --testset "$testset" --verify \
                "$db" > "$out" 2> "$ARTIFACT_DIR/bench-$testset-$rep-$key.err"
            grep -q '^Verification Hash: ' "$out" || { echo "no speedtest verification: $out" >&2; exit 1; }
            [[ $("$WORK/build-$key/sqlite3" -batch -bail "$db" 'PRAGMA integrity_check;') == ok ]] || {
                echo "speedtest DB failed integrity_check: $db" >&2; exit 1;
            }
        done
    done
done
python3 "$ROOT/tests/workloads/sqlite-3.54.1/summarize.py" \
    "$ARTIFACT_DIR" "$VERSION" "$ARCHIVE_SHA256" "$FOSSIL_ID" \
    "$RECIPE_COMMIT" "$RECIPE_SHA256" "$RECIPE_PATCH_SHA256" \
    "$PATCH_STATUS" "$(sha256sum "$PATCH" | cut -d' ' -f1)" \
    "$LCCC" "$LCCC_LD" "$GCC" "$SQLITE_CFLAGS" "$CPPFLAGS" "$JOBS" "$SPEED_SIZE" "$SPEED_REPEATS" "$SPEED_SETS"
printf 'SQLite %s PASS: GCC/LCCC/lccc-ld SQL, speedtest1 --verify, integrity_check\n' "$VERSION"
printf 'Manifest: %s/manifest.json; raw logs: %s\n' "$ARTIFACT_DIR" "$ARTIFACT_DIR"
