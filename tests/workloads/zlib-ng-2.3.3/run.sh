#!/usr/bin/env bash
# Pinned zlib-ng 2.3.3 + ms178 archpkgbuilds patch end-to-end gate.
#
# This is a correctness gate, not a benchmark. Every potentially hanging test
# has a short timeout and the whole CTest invocation has a hard deadline. The
# script refuses to build without active swap on constrained Arena workers.
set -euo pipefail

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd)
LCCC=${LCCC:-$ROOT/target/fastbuild/lccc}
ARTIFACT_DIR=${ARTIFACT_DIR:-$ROOT/zlib-ng-validation}
WORK_DIR=${WORK_DIR:-$(mktemp -d)}
KEEP_WORK=${KEEP_WORK:-0}
TEST_TIMEOUT=${TEST_TIMEOUT:-30}
SUITE_TIMEOUT=${SUITE_TIMEOUT:-180}
JOBS=${JOBS:-2}
TAG=2.3.3
TAG_COMMIT=12731092979c6d07f42da27da673a9f6c7b13586
ARCHPKG_COMMIT=0533f5f714381744cad35ace14b74e37d3caa137
PATCH_SHA256=40a317e1ac64e458bab6133ef259288edeab1a9054249dece3c7048fade3c2cc

cleanup() {
    if [[ $KEEP_WORK != 1 ]]; then rm -rf "$WORK_DIR"; fi
}
trap cleanup EXIT INT TERM
mkdir -p "$ARTIFACT_DIR" "$WORK_DIR"

if [[ ! -x $LCCC ]]; then
    echo "error: LCCC binary not executable: $LCCC" >&2
    exit 2
fi
if [[ $(wc -l </proc/swaps) -le 1 ]]; then
    # Unprivileged containers (no CAP_SYS_ADMIN) cannot swapon at all. The
    # swap gate exists to protect constrained VMs from OOM during the build;
    # on such boxes the equivalent protection is the userspace memory
    # watchdog (scripts/memwatch.sh) plus -j2 discipline. Allow an explicit,
    # documented waiver rather than silently disabling the gate.
    if [[ ${LCCC_SWAP_WAIVER:-0} == 1 ]]; then
        echo "warning: no active swap; proceeding under LCCC_SWAP_WAIVER (watchdog + -j2 discipline)" >&2
    else
        echo "error: no active swap; run scripts/ensure_swap.sh first (or set LCCC_SWAP_WAIVER=1 in a swapless container with the memwatch watchdog)" >&2
        exit 2
    fi
fi
for tool in git cmake ninja timeout sha256sum python3 curl gzip cmp; do
    command -v "$tool" >/dev/null || { echo "error: missing tool: $tool" >&2; exit 2; }
done
/sbin/swapon --show >"$ARTIFACT_DIR/swap.txt"

SRC="$WORK_DIR/zlib-ng-$TAG"
git clone -q --depth 1 --branch "$TAG" https://github.com/zlib-ng/zlib-ng.git "$SRC"
actual=$(git -C "$SRC" rev-parse HEAD)
[[ $actual == "$TAG_COMMIT" ]] || {
    echo "error: tag moved: expected $TAG_COMMIT, got $actual" >&2; exit 2;
}
PATCH="$WORK_DIR/ms178-1.patch"
curl -fsSL \
  "https://raw.githubusercontent.com/ms178/archpkgbuilds/$ARCHPKG_COMMIT/packages/zlib-ng/ms178-1.patch" \
  -o "$PATCH"
echo "$PATCH_SHA256  $PATCH" | sha256sum -c -
git -C "$SRC" apply --check "$PATCH"
git -C "$SRC" apply "$PATCH"

# Raptor Lake has no AVX-512. Keeping those source files out also avoids
# mistaking an unavailable-ISA failure for a compiler correctness failure.
cmake -S "$SRC" -B "$SRC/build" -G Ninja \
  -DCMAKE_BUILD_TYPE=Release \
  -DCMAKE_C_COMPILER="$LCCC" \
  -DCMAKE_C_FLAGS_RELEASE='-O2 -DNDEBUG' \
  -DZLIB_COMPAT=ON -DBUILD_TESTING=ON -DWITH_GTEST=OFF \
  -DWITH_AVX512=OFF -DWITH_AVX512VNNI=OFF -DWITH_VPCLMULQDQ=OFF \
  >"$ARTIFACT_DIR/configure.log" 2>&1
cmake --build "$SRC/build" -j "$JOBS" >"$ARTIFACT_DIR/build.log" 2>&1

set +e
timeout --signal=TERM --kill-after=5s "${SUITE_TIMEOUT}s" \
  ctest --test-dir "$SRC/build" --output-on-failure \
        --timeout "$TEST_TIMEOUT" -j "$JOBS" \
  >"$ARTIFACT_DIR/ctest.log" 2>&1
ctest_status=$?
set -e
printf '%s\n' "$ctest_status" >"$ARTIFACT_DIR/ctest.status"

# Independent deterministic binary/text round trips. Bound compression and
# decompression separately so a generated-code infinite loop cannot consume a
# session. System gzip validates the stream independently of zlib-ng inflate.
python3 - "$WORK_DIR/input.bin" <<'PY'
from pathlib import Path
import random, sys
r = random.Random(0x178)
p = Path(sys.argv[1])
with p.open("wb") as f:
    for _ in range(2):
        f.write(r.randbytes(1 << 20))
PY
roundtrip_status=0
for level in 1 2 6 9; do
    if ! timeout 10 "$SRC/build/minigzip" -"$level" -c "$WORK_DIR/input.bin" \
          >"$WORK_DIR/input.$level.gz"; then
        echo "level $level compression failed/timed out" >>"$ARTIFACT_DIR/roundtrip.log"
        roundtrip_status=1; continue
    fi
    if ! timeout 10 "$SRC/build/minigzip" -d -c "$WORK_DIR/input.$level.gz" \
          >"$WORK_DIR/output.$level.bin" \
       || ! cmp -s "$WORK_DIR/input.bin" "$WORK_DIR/output.$level.bin"; then
        echo "level $level self-roundtrip failed" >>"$ARTIFACT_DIR/roundtrip.log"
        roundtrip_status=1
    fi
    if ! gzip -t "$WORK_DIR/input.$level.gz" 2>>"$ARTIFACT_DIR/roundtrip.log"; then
        echo "level $level system-gzip validation failed" >>"$ARTIFACT_DIR/roundtrip.log"
        roundtrip_status=1
    fi
done
printf 'ctest=%s roundtrip=%s\n' "$ctest_status" "$roundtrip_status" \
  | tee "$ARTIFACT_DIR/summary.txt"

[[ $ctest_status == 0 && $roundtrip_status == 0 ]]
