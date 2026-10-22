#!/usr/bin/env bash
# Full-width casts expose affine SIB offsets; narrower signedness casts do not.
set -euo pipefail
here=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH='' cd -- "$here/../.." && pwd)
CCC=${CCC:-$root/target/fastbuild/lccc}
. "$here/lib/gcc_oracle.sh"
require_gcc_oracle check_pointer_ring_indexing
work=$(mktemp -d "${TMPDIR:-/tmp}/lccc-pointer-ring.XXXXXX")
trap 'rm -rf "$work"' EXIT
source=$here/pointer_ring_indexing.c
"$GCC_ORACLE_BIN" -O0 -fwrapv -Wall -Wextra -Werror "$source" -o "$work/reference"
timeout 30 "$work/reference" > "$work/expected"
for opt in -O0 -O1 -O2 -O3 -Os -Oz; do
    for route in default classic no-gep-fold; do
        knobs=()
        [[ $route != classic ]] || knobs+=(CCC_NO_MACHINST=1)
        [[ $route != no-gep-fold ]] || knobs+=(CCC_NO_GEP_FOLD=1)
        env "${knobs[@]}" "$CCC" "$opt" -fwrapv "$source" -o "$work/candidate"
        timeout 30 "$work/candidate" > "$work/actual"
        diff -u "$work/expected" "$work/actual"
    done
done
"$CCC" -O2 -S "$source" -o "$work/index.s"
# Pin the displacement, not a particular physical register allocation.
awk '/^unsigned_index:/ { inside=1 } inside { print } inside && /\.size/ { exit }' "$work/index.s" > "$work/body.s"
grep -Eq '8\(%[a-z0-9]+, *%[a-z0-9]+, *8\)' "$work/body.s"
# Compilation is required even without a runnable 32-bit libc.
"$CCC" -m32 -O2 -S "$source" -o "$work/index32.s"
printf 'PASS: pointer-ring indexing, six optimization levels, three lowering routes, affine displacement and ILP32 compilation\n'
