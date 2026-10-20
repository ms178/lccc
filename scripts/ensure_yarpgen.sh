#!/usr/bin/env bash
# Idempotently (re)provision the YARPGen binary the differential harness
# needs (scripts/fuzz_diff.py --engine yarpgen, scripts/yarpgen_mutpair.py).
#
# Why this exists: the yarpgen engine quietly rotted for lack of a
# provisioner -- no CI job could build the generator, so no gate ran it,
# so a harness bug that dropped YARPGen's emitted init.h (every case
# uncompilable) survived unnoticed.  A generator nobody can install is a
# test nobody runs.
#
# Source: intel/yarpgen (the maintained YARPGen home; the VoR0n0k/yarpgen
# repo this project used to link is no longer public), pinned to an exact
# commit.  The pin is part of the test contract: generation is only
# reproducible for a fixed generator revision, so seed N must mean the
# same program on every host.  Raise the pin deliberately, together with
# a smoke run (see below), never by tracking a moving branch.
#
# Trust model (mirrors scripts/ensure_gcc_torture.sh, scoped down): the
# checkout and build live in the snapshot-excluded ~/.cache zone, which a
# harness wipe may partially delete.  The stamp is therefore written
# (temp file + rename) only after the built binary passes an end-to-end
# probe -- generate a case, require driver.c/func.c/init.h, compile it
# with the host cc, run it, require a decimal hash on stdout.  A stamp is
# trusted only when it names the pinned ref AND the binary exists AND a
# fresh probe of that binary still passes (--check re-probes; any
# failure re-provisions automatically).
#
# Usage:
#   scripts/ensure_yarpgen.sh [--force] [--print-bin] [--check]
# Env:  YARPGEN_REF (default: pin below), LCCC_YARPGEN_PREFIX,
#       YARPGEN_BUILD_JOBS (default 2, the research-host policy),
#       CC (probe compiler, default: cc/gcc from PATH).
set -Eeuo pipefail

REF=${YARPGEN_REF:-1adb290f453505838f3aa33dc571f292b0c1810f}
PREFIX=${LCCC_YARPGEN_PREFIX:-"${XDG_CACHE_HOME:-$HOME/.cache}/lccc/yarpgen/${REF:0:12}"}
BIN="$PREFIX/build/yarpgen"
STAMP="$PREFIX/.lccc-provisioned"
JOBS=${YARPGEN_BUILD_JOBS:-2}

print_bin=0 force=0 check_only=0
while [ $# -gt 0 ]; do
    case "$1" in
        --print-bin) print_bin=1 ;;
        --force) force=1 ;;
        --check) check_only=1 ;;
        *) echo "ensure_yarpgen: unknown argument '$1'" >&2; exit 2 ;;
    esac
    shift
done

probe() {  # probe <yarpgen-binary> -> 0 iff generate+compile+run works
    local bin=$1 cc_bin tmp
    [ -x "$bin" ] || return 1
    cc_bin=${CC:-}
    if [ -z "$cc_bin" ]; then
        cc_bin=$(command -v gcc || command -v cc || true)
    fi
    [ -n "$cc_bin" ] || return 1
    tmp=$(mktemp -d) || return 1
    mkdir -p "$tmp/gen"   # yarpgen's -o folder must already exist
    if "$bin" --std=c --seed=123 --max-array-dims=3 -o "$tmp/gen" >/dev/null 2>&1 \
        && [ -f "$tmp/gen/driver.c" ] && [ -f "$tmp/gen/func.c" ] \
        && [ -f "$tmp/gen/init.h" ] \
        && "$cc_bin" -std=c99 -w -O1 "$tmp/gen/driver.c" "$tmp/gen/func.c" \
             -o "$tmp/a.out" >/dev/null 2>&1; then
        local out
        out=$("$tmp/a.out" 2>/dev/null) || { rm -rf "$tmp"; return 1; }
        rm -rf "$tmp"
        case "$out" in (*[!0-9]*|'') return 1 ;; (*) return 0 ;; esac
    fi
    rm -rf "$tmp"
    return 1
}

stamp_ok() {
    [ -x "$BIN" ] && [ -f "$STAMP" ] \
        && grep -qx "ref=$REF" "$STAMP" && grep -qx 'probe=ok' "$STAMP"
}

write_stamp() {
    local tmp="$STAMP.tmp.$$"
    printf 'ref=%s\nprobe=ok\n' "$REF" > "$tmp"
    mv -f "$tmp" "$STAMP"
}

if [ "$force" = 0 ] && stamp_ok && probe "$BIN"; then
    [ "$print_bin" = 1 ] && echo "$BIN"
    exit 0
fi
if [ "$check_only" = 1 ]; then
    echo "ensure_yarpgen: no valid provision at $PREFIX (run without --check to build)" >&2
    exit 1
fi

command -v git >/dev/null || { echo "ensure_yarpgen: git not found" >&2; exit 1; }
command -v cmake >/dev/null || {
    echo "ensure_yarpgen: cmake not found (pip install --user cmake, or apt install cmake)" >&2
    exit 1
}

mkdir -p "$PREFIX"
rm -f "$STAMP"   # a failed/interrupted re-provision must leave no stamp
if [ ! -d "$PREFIX/src/.git" ]; then
    rm -rf "$PREFIX/src"
    git clone -q --no-checkout https://github.com/intel/yarpgen "$PREFIX/src"
fi
git -C "$PREFIX/src" fetch -q origin "$REF" 2>/dev/null || true
git -C "$PREFIX/src" checkout -q "$REF"
actual=$(git -C "$PREFIX/src" rev-parse HEAD)
if [ "$actual" != "$REF" ]; then
    echo "ensure_yarpgen: checkout drift: want $REF, got $actual" >&2
    exit 1
fi

cmake -S "$PREFIX/src" -B "$PREFIX/build" -DCMAKE_BUILD_TYPE=Release >/dev/null
cmake --build "$PREFIX/build" -j "$JOBS" >/dev/null

if ! probe "$BIN"; then
    echo "ensure_yarpgen: built binary failed the generate/compile/run probe: $BIN" >&2
    exit 1
fi
write_stamp
echo "ensure_yarpgen: provisioned $BIN (ref ${REF:0:12}, probe ok)" >&2
[ "$print_bin" = 1 ] && echo "$BIN"
