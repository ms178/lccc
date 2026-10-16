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

# GP counterpart: a textual rename cannot replace architectural RAX or CL.
"$compiler" -O2 -Wall -Wextra -Werror "$fixture/gp-driver.c" "$fixture/gp-witness.s" -o "$tmp/gp"
"$tmp/gp"
printf '.text\nshlq %%cl, %%r8\n' > "$tmp/good-shift.s"
"$compiler" -c "$tmp/good-shift.s" -o "$tmp/good-shift.o"
printf '.text\nshlq %%sil, %%r8\n' > "$tmp/bad-shift.s"
if "$compiler" -c "$tmp/bad-shift.s" -o "$tmp/bad-shift.o" 2>"$tmp/bad-shift.err"; then
    echo 'FAIL: assembler accepted illegal rewritten shift count' >&2
    exit 1
fi
echo 'GP fixed-operand controls: PASS (native implicit product + assembler shift rejection)'

# Native AVX-512 controls are conditional; their optimizer tests are hermetic.
"$compiler" -O2 -Wall -Wextra -Werror "$fixture/masked-copy-driver.c" "$fixture/masked-copy-witness.s" -o "$tmp/masked-copy"
"$tmp/masked-copy"

"$compiler" -O2 -mavx2 "$fixture/gather-driver.c" "$fixture/gather-witness.s" -o "$tmp/gather"
"$tmp/gather"
echo 'AVX2 gather mask read/write controls: PASS'
