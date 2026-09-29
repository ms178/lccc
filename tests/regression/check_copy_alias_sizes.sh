#!/usr/bin/env bash
# COPY-alias size discipline (S22): same-address DSO data exports carrying
# different st_size must ALL read fully. i686 shares one R_386_COPY per
# group (the rep must be the largest member or the tail under-copies and
# stays zero); x86-64 shares one BSS slot per group (sized for the largest
# or a later-larger alias overflows into its neighbor). The DSO is built by
# the host linker (not under test); the executable is linked by lccc-ld via
# a gcc -B shim, isolating the linkers under test. The DSO exports are
# deliberately STT_NOTYPE (no .type: hand-written asm in the wild), which
# also pins the NOTYPE-in-data COPY routing.
set -euo pipefail
CCC=${CCC:-target/fastbuild/lccc}
LD=${LD:-$(dirname "$CCC")/lccc-ld}
tmp=$(mktemp -d "${TMPDIR:-/tmp}/lccc-copyalias.XXXXXX")
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp/shim"
ln -s "$(cd "$(dirname "$LD")" && pwd)/$(basename "$LD")" "$tmp/shim/ld"
# One 8-byte object, two aliases with different declared sizes.
cat >"$tmp/aliased.s" <<'S'
    .data
    .globl alias4
    .globl alias8
    .p2align 3
alias4:
alias8:
    .quad 0x1122334455667788
    .size alias4, 4
    .size alias8, 8
S
cat >"$tmp/use.c" <<'C'
#include <stdio.h>
extern int alias4;
extern long long alias8;
int main(void) {
    printf("%d %lld\n", alias4, alias8);
    return 0;
}
C
# Little-endian: low 4 bytes 0x55667788, full 8 bytes 0x1122334455667788.
expected="1432778632 1234605616436508552"
for m in "" -m32; do
    # i686 lccc-ld is ET_EXEC-only: -pie output is not implemented there.
    pie=()
    if [[ "$m" == "-m32" ]]; then pie=(-no-pie); fi
    gcc $m -shared -fPIC -o "$tmp/libaliased.so" "$tmp/aliased.s"
    "$CCC" $m -c "$tmp/use.c" -o "$tmp/use.o"
    # shellcheck disable=SC2086
    gcc $m "${pie[@]}" -B"$tmp/shim" "$tmp/use.o" -L"$tmp" -laliased -o "$tmp/use"
    got=$(LD_LIBRARY_PATH="$tmp" "$tmp/use")
    if [[ "$got" != "$expected" ]]; then
        echo "FAIL${m:+ ($m)}: got '$got', expected '$expected'" >&2
        exit 1
    fi
done
echo "PASS: copy-alias sizes (x86-64 + i386)"
