#!/usr/bin/env bash
# Per-function instruction counts for bit-selection.c on every target LCCC
# lowers, with the single-bit fold on and off (`CCC_NO_BIT_SELECT=1`).
#
#   ./cross-target.sh                 # prints the table
#   ./cross-target.sh > cross-target-counts.txt
#
# The point of the knob-off column is the no-pessimisation contract: selecting
# `BitSet`/`BitClear` must never lengthen a function, including on the targets
# where the recogniser wins nothing (`x | 1 << n` is already two instructions
# on AArch64).  There is no qemu and no 32-bit libc in the development
# sandbox, so instruction counts and the executed probe on x86-64 are the whole
# of the evidence for i686/AArch64/RISC-V.
set -u
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../../.." && pwd)
build="$root/target/fastbuild"
src="$here/bit-selection.c"
names="$here/names.txt"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

targets="x86-64:$build/lccc i686:$build/lccc-i686 aarch64:$build/lccc-arm riscv64:$build/lccc-riscv"

printf '%-8s %-12s %6s %6s %6s %s\n' target function off on delta note
for entry in $targets; do
    t=${entry%%:*}
    bin=${entry#*:}
    for knob in off on; do
        env=""
        [ "$knob" = off ] && env="CCC_NO_BIT_SELECT=1"
        env $env "$bin" -S -O2 "$src" -o "$tmp/$t-$knob.s" || { echo "$t: build failed" >&2; exit 1; }
        # shellcheck disable=SC2046  # one argument per probe name
        python3 "$here/count.py" "$tmp/$t-$knob.s" $(cat "$names") > "$tmp/$t-$knob.counts"
    done
    while read -r fn; do
        [ -z "$fn" ] && continue
        off=$(awk -v f="$fn" '$1 == f { print $2 }' "$tmp/$t-off.counts")
        on=$(awk -v f="$fn" '$1 == f { print $2 }' "$tmp/$t-on.counts")
        [ -z "$off" ] && continue
        delta=$((on - off))
        note=""
        [ "$delta" -gt 0 ] && note="REGRESSION"
        printf '%-8s %-12s %6s %6s %+6d %s\n' "$t" "$fn" "${off:-?}" "${on:-?}" "$delta" "$note"
    done < "$names"
done
