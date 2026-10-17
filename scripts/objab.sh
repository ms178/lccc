#!/usr/bin/env bash
# Object A/B: compile every tests/regression C fixture with two compiler
# binaries and compare the object files byte for byte.
#
#   scripts/objab.sh OLD_BIN_DIR NEW_BIN_DIR OUTDIR BIN [CFLAGS...]
#
# BIN names the driver inside both directories (lccc, lccc-i686, lccc-arm,
# lccc-riscv). CFLAGS default to -O2. Writes OUTDIR/{same,diff,status}.txt
# and prints one summary line; exits 1 if any fixture's compile status
# changed (one side compiles, the other refuses), 0 otherwise. A byte
# difference is reported, not failed: it is the reviewer's evidence.
set -u
if [ $# -lt 4 ]; then
    sed -n '2,11p' "$0" >&2
    exit 2
fi
old=$1/$4 new=$2/$4 out=$3
shift 4
[ $# -gt 0 ] || set -- -O2
for b in "$old" "$new"; do
    [ -x "$b" ] || { echo "objab: $b is not executable" >&2; exit 2; }
done
repo=$(cd "$(dirname "$0")/.." && pwd)
mkdir -p "$out/old" "$out/new"
: > "$out/same.txt"; : > "$out/diff.txt"; : > "$out/status.txt"
export old new out
one() {
    src=$1 base=$(basename "$1" .c) flags=$2
    # shellcheck disable=SC2086
    "$old" $flags -c "$src" -o "$out/old/$base.o" > /dev/null 2>&1; ro=$?
    # shellcheck disable=SC2086
    "$new" $flags -c "$src" -o "$out/new/$base.o" > /dev/null 2>&1; rn=$?
    if [ $ro -ne 0 ] || [ $rn -ne 0 ]; then
        [ $((ro == 0)) -eq $((rn == 0)) ] || echo "$base old=$ro new=$rn" >> "$out/status.txt"
    elif cmp -s "$out/old/$base.o" "$out/new/$base.o"; then
        echo "$base" >> "$out/same.txt"
    else
        echo "$base" >> "$out/diff.txt"
    fi
}
export -f one
ls "$repo"/tests/regression/*.c | xargs -P "${OBJAB_JOBS:-2}" -I{} bash -c 'one "$1" "$2"' _ {} "$*"
for f in same diff status; do sort -o "$out/$f.txt" "$out/$f.txt"; done
total=$(ls "$repo"/tests/regression/*.c | wc -l)
echo "objab $(basename "$new"): $(wc -l < "$out/same.txt") identical, $(wc -l < "$out/diff.txt") differ," \
     "$(wc -l < "$out/status.txt") status changes, of $total fixtures"
[ ! -s "$out/status.txt" ]
