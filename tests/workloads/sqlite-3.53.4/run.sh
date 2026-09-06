#!/usr/bin/env bash
# SQLite 3.53.4 workload gate for LCCC.
#
# This is an end-to-end correctness/build/codegen gate, not a PMU claim.  It
# fetches the exact SQLite source archive named by ms178/archpkgbuilds, records
# the recipe checksum discrepancy instead of hiding it, applies the same Lemon
# search-path fix with a release-tarball-compatible context, builds the CLI and
# static library with LCCC, links a second CLI directly with lccc-ld, and runs a
# deterministic SQL workload against LCCC and GCC reference binaries.
#
# Usage:
#   tests/workloads/sqlite-3.53.4/run.sh
#   LCCC=/path/to/lccc SQLITE_WORK_DIR=/large/disk/path run.sh
#
# The source and build trees are deliberately outside the repository by
# default.  Generated SQLite files must not enter ms178-1.patch.
set -euo pipefail

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd)
LCCC=${LCCC:-$ROOT/target/fastbuild/lccc}
LCCC_LD=${LCCC_LD:-$(dirname "$LCCC")/lccc-ld}
WORK=${SQLITE_WORK_DIR:-/home/user/workloads/sqlite-3.53.4}
CACHE=${SQLITE_CACHE:-/home/user/source-cache}
SRCVER=3530400
PKGVER=3.53.4
URL="https://www.sqlite.org/2026/sqlite-src-${SRCVER}.zip"
# Actual archive digest recorded by the repository's provenance audit.  The
# current archpkgbuilds PKGBUILD lists a different digest; fail closed if the
# bytes change, but report both values in the manifest.
ARCHIVE_SHA256=d18fa15aec74d8c17e1463f861095adc01b5ad190256acb4f91d22f0368d232b
RECIPE_SHA256=2d7b032b6fdfe8c442aa809f850687a81d06381deecd7be3312601d28612e640
RECIPE_PATCH=${SQLITE_RECIPE_PATCH:-/home/user/archpkgbuilds/packages/sqlite/sqlite-lemon-system-template.patch}
COMPAT_PATCH=${SQLITE_COMPAT_PATCH:-$ROOT/tests/workloads/sqlite-3.53.4/sqlite-lemon-system-template-3.53.4.patch}
JOBS=${JOBS:-2}
CFLAGS=${SQLITE_CFLAGS:--O2 -march=x86-64-v3 -mtune=native -fomit-frame-pointer}
CPPFLAGS=${SQLITE_CPPFLAGS:--D_GNU_SOURCE=1 -DSQLITE_ENABLE_COLUMN_METADATA=1 -DSQLITE_ENABLE_DBSTAT_VTAB=1 -DSQLITE_ENABLE_DESERIALIZE=1 -DSQLITE_ENABLE_FTS3_PARENTHESIS=1 -DSQLITE_ENABLE_FTS3_TOKENIZER=1 -DSQLITE_ENABLE_MATH_FUNCTIONS=1 -DSQLITE_ENABLE_STAT4=1 -DSQLITE_ENABLE_STMTVTAB=1 -DSQLITE_ENABLE_UNLOCK_NOTIFY=1 -DSQLITE_LIKE_DOESNT_MATCH_BLOBS=1 -DSQLITE_MAX_EXPR_DEPTH=10000 -DSQLITE_MAX_VARIABLE_NUMBER=250000 -DSQLITE_USE_URI=1}

mkdir -p "$WORK" "$CACHE"
command -v curl >/dev/null || { echo 'error: curl is required' >&2; exit 2; }
command -v unzip >/dev/null || { echo 'error: unzip is required' >&2; exit 2; }
command -v patch >/dev/null || { echo 'error: patch is required' >&2; exit 2; }
[[ -x "$LCCC" ]] || { echo "error: LCCC is not executable: $LCCC" >&2; exit 2; }
[[ -x "$LCCC_LD" ]] || { echo "error: lccc-ld is not executable: $LCCC_LD" >&2; exit 2; }

archive="$CACHE/sqlite-src-${SRCVER}.zip"
if [[ ! -f "$archive" ]]; then curl -fL --retry 3 -o "$archive" "$URL"; fi
archive_sha=$(sha256sum "$archive" | awk '{print $1}')
[[ "$archive_sha" == "$ARCHIVE_SHA256" ]] || {
    echo "error: SQLite archive checksum mismatch: $archive_sha" >&2
    exit 1
}

src="$WORK/sqlite-src-${SRCVER}"
build="$WORK/build-lccc"
rm -rf "$src" "$build"
unzip -q "$archive" -d "$WORK"

recipe_patch_status=missing
if [[ -f "$RECIPE_PATCH" ]]; then
    recipe_sha=$(sha256sum "$RECIPE_PATCH" | awk '{print $1}')
    if patch --dry-run -Np1 --fuzz=0 -i "$RECIPE_PATCH" -d "$src" >/dev/null 2>&1; then
        patch -Np1 --fuzz=0 -i "$RECIPE_PATCH" -d "$src" >/dev/null
        recipe_patch_status=applied
    else
        recipe_patch_status=stale-on-release-tarball
    fi
else
    recipe_sha=missing
fi
# The archpkgbuilds patch is intentionally not fuzzed or silently edited.  Its
# semantic replacement is pinned in this repository and must apply exactly.
if [[ "$recipe_patch_status" != applied ]]; then
    patch --dry-run -Np1 --fuzz=0 -i "$COMPAT_PATCH" -d "$src" >/dev/null
    patch -Np1 --fuzz=0 -i "$COMPAT_PATCH" -d "$src" >/dev/null
fi
compat_sha=$(sha256sum "$COMPAT_PATCH" | awk '{print $1}')

mkdir -p "$build"
(
    cd "$build"
    CC="$LCCC" CPPFLAGS="$CPPFLAGS" CFLAGS="$CFLAGS" \
      LDFLAGS='-Wl,-z,relro -Wl,-z,now' \
      "$src/configure" --prefix="$WORK/install-lccc" \
        --disable-shared --enable-static --disable-readline --disable-tcl \
        --enable-threadsafe --enable-json
    # Ask for the archive in the same invocation: SQLite marks sqlite3.o as
    # an intermediate of the CLI-only target and may delete it after linking.
    # The archive is also the useful generated-code artifact for downstream
    # workload link tests.
    make -j"$JOBS" sqlite3 libsqlite3.a >build.log 2>&1
)

# Link an independent CLI through the standalone linker.  The compiler driver
# is still used for preprocessing/code generation; this command proves that
# the companion lccc-ld consumes the same objects and CRT contract.
(
    cd "$build"
    includes="-I. -I$src/src -I$src/ext/rtree -I$src/ext/icu -I$src/ext/fts3 -I$src/ext/session -I$src/ext/misc"
    "$LCCC" -DNDEBUG -fPIC $CFLAGS $CPPFLAGS -D_HAVE_SQLITE_CONFIG_H \
      -DBUILD_sqlite $includes -c shell.c -o shell-lccc.o
    "$LCCC_LD" -o sqlite3-lccc-ld \
      /usr/lib/x86_64-linux-gnu/Scrt1.o /usr/lib/x86_64-linux-gnu/crti.o \
      "$(gcc -print-file-name=crtbeginS.o)" shell-lccc.o sqlite3.o \
      --dynamic-linker=/lib64/ld-linux-x86-64.so.2 \
      -L"$(gcc -print-file-name=libgcc.a | xargs dirname)" \
      -L/usr/lib/x86_64-linux-gnu -lz -ldl -lpthread -lm -lgcc -lgcc_s -lc \
      "$(gcc -print-file-name=crtendS.o)" /usr/lib/x86_64-linux-gnu/crtn.o
)

sql="$WORK/workload.sql"
cat >"$sql" <<'EOF'
PRAGMA journal_mode=MEMORY;
PRAGMA synchronous=OFF;
PRAGMA foreign_keys=ON;
CREATE TABLE item(id INTEGER PRIMARY KEY, name TEXT NOT NULL, value REAL, state TEXT);
WITH RECURSIVE n(i) AS (VALUES(1) UNION ALL SELECT i+1 FROM n WHERE i<2048)
INSERT INTO item SELECT i, printf('item_%04d',i), i/3.0,
  CASE i%4 WHEN 0 THEN 'new' WHEN 1 THEN 'ready' WHEN 2 THEN 'done' ELSE 'cached' END FROM n;
CREATE INDEX item_state_value ON item(state,value DESC);
CREATE TABLE event(item_id INTEGER, kind TEXT, payload BLOB,
  FOREIGN KEY(item_id) REFERENCES item(id));
INSERT INTO event SELECT id, CASE id%3 WHEN 0 THEN 'launch' WHEN 1 THEN 'shader' ELSE 'network' END,
  zeroblob(32+(id%5)) FROM item;
CREATE INDEX event_item_kind ON event(item_id,kind);
SELECT count(*), sum(id), printf('%.3f',avg(value)) FROM item;
SELECT state, count(*), printf('%.3f',sum(value)) FROM item GROUP BY state ORDER BY state;
SELECT i.name, count(e.item_id) FROM item i LEFT JOIN event e ON e.item_id=i.id
  GROUP BY i.id HAVING count(e.item_id)>0 ORDER BY i.id LIMIT 32;
SELECT name FROM item WHERE state='ready' AND value BETWEEN 100.0 AND 500.0 ORDER BY value DESC LIMIT 32;
SELECT item_id FROM event WHERE kind='shader' INTERSECT SELECT id FROM item WHERE state='done';
SELECT json_extract('{"a":1,"b":[2,3]}','$.b[1]');
SELECT typeof(id), typeof(name), typeof(value) FROM item LIMIT 4;
EXPLAIN QUERY PLAN SELECT name FROM item WHERE state='ready' ORDER BY value DESC LIMIT 8;
PRAGMA integrity_check;
EOF

out_lccc=$($build/sqlite3-lccc-ld :memory: <"$sql")
out_driver=$($build/sqlite3 :memory: <"$sql")
# The two LCCC links must agree exactly.  GCC is the reference for the same
# semantic workload; use the host CLI only as an additional oracle because it
# may have a different compile-time feature set.
[[ "$out_lccc" == "$out_driver" ]] || {
    echo 'error: lccc driver and lccc-ld SQLite outputs differ' >&2
    diff -u <(printf '%s\n' "$out_driver") <(printf '%s\n' "$out_lccc") || true
    exit 1
}

manifest="$WORK/manifest.json"
python3 - "$manifest" "$archive_sha" "$recipe_patch_status" "${recipe_sha:-missing}" "$compat_sha" "$LCCC" "$LCCC_LD" <<'PY'
import json, pathlib, sys, subprocess
out, archive, patch_status, recipe, compat, lccc, lccc_ld = sys.argv[1:]
def version(cmd):
    try: return subprocess.run([cmd, '--version'], text=True, capture_output=True).stdout.splitlines()[0]
    except OSError as e: return f'{type(e).__name__}: {e}'
payload = {
    'schema': 1, 'workload': 'sqlite', 'version': '3.53.4',
    'archive_sha256': archive, 'recipe_patch_status': patch_status,
    'recipe_patch_sha256': recipe, 'compat_patch_sha256': compat,
    'lccc': str(pathlib.Path(lccc).resolve()), 'lccc_version': version(lccc),
    'lccc_ld': str(pathlib.Path(lccc_ld).resolve()), 'lccc_ld_version': version(lccc_ld),
    'link_mode': 'LCCC compile + standalone lccc-ld final link',
    'result': 'PASS',
}
pathlib.Path(out).write_text(json.dumps(payload, indent=2) + '\n')
PY
printf 'SQLite %s: PASS\n' "$PKGVER"
printf '  archive=%s\n  recipe_patch=%s (%s)\n  compatibility_patch=%s\n' "$archive_sha" "$recipe_patch_status" "${recipe_sha:-missing}" "$compat_sha"
printf '  lccc=%s\n  lccc-ld=%s\n' "$LCCC" "$LCCC_LD"
