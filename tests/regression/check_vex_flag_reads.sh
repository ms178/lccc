#!/usr/bin/env bash
# Independent architectural control for the hermetic optimizer regression:
# a removed VEX move changes flags for all eleven flag/string vector test forms.
set -euo pipefail

if [[ $(uname -m) != x86_64 ]] || ! grep -qw avx2 /proc/cpuinfo || ! grep -qw sse4_2 /proc/cpuinfo; then
    echo 'FAIL: VEX flag-read witness requires an AVX2/SSE4.2-capable x86-64 Linux host' >&2
    exit 1
fi

root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
fixture="$root/tests/regression/fixtures/peephole-vex-flags"
compiler=${GCC:-gcc}
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

"$compiler" --version
"$compiler" -O2 -mavx2 "$fixture/driver.c" "$fixture/witness.s" -o "$tmp/witness"
"$tmp/witness"
echo 'VEX flag-read architectural witnesses: PASS (11 original/deleted pairs)'

"$compiler" -O2 -mavx2 "$fixture/implicit-driver.c" "$fixture/implicit-witness.s" -o "$tmp/implicit"
"$tmp/implicit"
echo 'Implicit vector/state architectural witnesses: PASS (SHA conditional on ISA availability)'
