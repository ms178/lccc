#!/usr/bin/env bash
# Runtime contract for x86 L->L self-reload store forwarding.  Assembly unit
# tests pin the rewrite itself; this executes the C shape whose unsigned return
# value needs the reload's upper-half zeroing at every -O level on both targets.
set -euo pipefail
root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
CCC=${CCC:-$root/target/fastbuild/lccc}
CCC_I686=${CCC_I686:-$root/target/fastbuild/lccc-i686}
[[ $CCC = /* ]] || CCC=$root/$CCC
[[ $CCC_I686 = /* ]] || CCC_I686=$root/$CCC_I686
[[ -x $CCC ]] || { echo "FAIL: compiler not built: $CCC" >&2; exit 1; }
[[ -x $CCC_I686 ]] || { echo "FAIL: i686 compiler not built: $CCC_I686" >&2; exit 1; }
src=$root/tests/regression/store_forwarding_dword_self_reload.c
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

for arch in x86-64 i686; do
    cc=$CCC
    [[ $arch == i686 ]] && cc=$CCC_I686
    for opt in -O0 -O1 -O2 -O3 -Os; do
        exe=$work/run-$arch$opt
        env -u CCC_NO_PEEPHOLE -u CCC_PEEPHOLE_SKIP -u CCC_NO_REDUNDANT_EXT \
            -u CCC_NO_ZEXT_NOP "$cc" "$opt" "$src" -o "$exe"
        timeout 10 "$exe"
    done
done

echo "store-forwarding dword self-reload: PASS (x86-64 + i686 runtime at -O0/-O1/-O2/-O3/-Os)"
