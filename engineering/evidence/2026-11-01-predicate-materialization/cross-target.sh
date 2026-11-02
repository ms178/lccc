#!/usr/bin/env bash
# Per-target instruction counts for the probe, fold off vs fold on.
#
#   ./cross-target.sh [lccc] [probefile]
#
# The knob is the only difference between the two builds of each row, so a
# function that grows on a target is a pessimisation of this fold on that
# target.  Usage from the evidence directory:
#
#   scripts/build_lccc_fast.sh                      # x86-64 + the cross bins
#   ./cross-target.sh ../../../target/fastbuild/lccc predicate-materialize.c
set -euo pipefail
CCC=${1:-../../../target/fastbuild/lccc}
SRC=${2:-predicate-materialize.c}
dir=$(CDPATH= cd -- "$(dirname -- "$CCC")" && pwd)
tmp=${TMPDIR:-/tmp}/lccc-pred-cross.$$
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp"
count() { # $1 = asm file, $2 = output (function, count per line, sorted)
    awk -v out="$2" '
        /^[a-z_][A-Za-z0-9_]*:/ { f = $0; sub(":", "", f); cur = f; n[f] += 0; next }
        /^[ \t]*\.size/ { cur = "" }
        cur != "" && /^[ \t]+[a-z]/ && !/^[ \t]+\./ && !/^[ \t]+#/ { n[cur]++ }
        END { for (f in n) printf "%s %d\n", f, n[f] }' "$1" | sort > "$2"
}
for target in x86-64 i686 arm riscv; do
    case $target in
        x86-64) bin=$CCC ;;
        *) bin=$dir/lccc-$target ;;
    esac
    [ -x "$bin" ] || { echo "# $target: $bin not built, skipped" >&2; continue; }
    CCC_NO_PREDICATE_MATERIALIZE=1 "$bin" -O2 -S "$SRC" -o "$tmp/$target-off.s"
    "$bin" -O2 -S "$SRC" -o "$tmp/$target-on.s"
    count "$tmp/$target-off.s" "$tmp/$target-off.txt"
    count "$tmp/$target-on.s" "$tmp/$target-on.txt"
    echo "# $target: function off -> on (unchanged rows omitted)"
    join -a1 -a2 -e0 -o 0,1.2,2.2 "$tmp/$target-off.txt" "$tmp/$target-on.txt" |
        awk '$2 != $3 { print "  " $1, $2, "->", $3; changed++ }
             $2 == $3 { same++ }
             END { if (same) printf "# %d of %d function(s) unchanged on this target\n", same, same + changed + 0 }'
done
