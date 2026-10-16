#!/usr/bin/env bash
# All accesses stay inside the source array; no fabricated out-of-object base.
set -euo pipefail
root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
ccc=${CCC:-$root/target/fastbuild/lccc}
gcc=${GCC:-gcc}
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
src="$root/tests/regression/ivsr_signed_affine_step.c"
for mode in -m64 -m32; do
    for opt in -O0 -O1 -O2 -O3 -Os -Oz; do
        "$gcc" "$mode" "$opt" -fwrapv -Wall -Wextra -Werror "$src" -o "$tmp/ref"
        CCC_VALIDATE_SSA=1 "$ccc" "$mode" "$opt" -fwrapv "$src" -o "$tmp/mine"
        "$tmp/ref" > "$tmp/expected"
        "$tmp/mine" > "$tmp/actual"
        cmp "$tmp/expected" "$tmp/actual"
        grep -qx 'ivsr_signed_affine_step: PASS (2184 checks)' "$tmp/actual"
    done
done
echo 'PASS: signed affine-step m64/m32, six optimization levels, 2184 checks per arm'
# Independent backend controls prevent either emitter masking the other's bug.
src="$root/tests/regression/x86_signed_index_home.c"
for mode in -m64 -m32; do
    for opt in -O0 -O1 -O2 -O3 -Os -Oz; do
        "$gcc" "$mode" "$opt" -fwrapv -Wall -Wextra -Werror "$src" -o "$tmp/ref"
        "$tmp/ref" > "$tmp/expected"
        for route in default classic no-peephole; do
            knobs=()
            [[ $route != classic ]] || knobs+=(CCC_NO_MACHINST=1)
            [[ $route != no-peephole ]] || knobs+=(CCC_NO_PEEPHOLE=1)
            env CCC_VALIDATE_SSA=1 "${knobs[@]}" "$ccc" "$mode" "$opt" -fwrapv "$src" -o "$tmp/mine"
            "$tmp/mine" > "$tmp/actual"
            cmp "$tmp/expected" "$tmp/actual"
            grep -qx 'x86_signed_index_home: PASS (2150 checks)' "$tmp/actual"
        done
    done
done
echo 'PASS: signed index homes, two widths, six optimization levels, three routes, 2150 checks per arm'
