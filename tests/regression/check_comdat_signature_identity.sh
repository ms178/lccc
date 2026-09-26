#!/usr/bin/env bash
# COMDAT groups are identified by their signature alone (gABI; GNU ld).
#
# The i686 linker additionally dropped every group member whose section
# NAME an earlier kept member had used: two distinct groups sharing a name
# (`.text` in `,comdat` groups with different signatures, which assemblers
# emit whenever the section name is not made unique) lost the second
# group's code, and plain (non-COMDAT) groups -- never deduplicated by the
# gABI -- were merged the same way.  An unnamed signature also collapsed all
# such groups into one.  The x86-64 linker (linker_common::comdat) already
# had the signature rule; both are pinned here against GNU as objects:
#
#   1. distinct signatures, same section name: both groups are kept;
#   2. one signature twice: the first group is kept and the second dropped
#      before symbol resolution (a strong global defined in both copies is
#      not a duplicate definition, and the first copy's body wins);
#   3. plain groups sharing a name are both kept.
set -euo pipefail
CCC=${CCC:-./target/fastbuild/lccc}
GCC=${GCC_BIN:-gcc}
tmp=${TMPDIR:-/tmp}/lccc-comdat-sig.$$
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp"

# `movl $N, %eax; ret` assembles identically for both targets.
group() { # file section flags-with-group-args symbol value
    cat >"$tmp/$1" <<S
    .section $2,"axG",@progbits,$3
    .globl $4
    .type $4, @function
$4:
    movl \$$5, %eax
    ret
    .size $4, .-$4
S
}
group a.s .text sigA,comdat fa 1
group b.s .text sigB,comdat fb 2
group c1.s .text.fc sigC,comdat fc 3
group c2.s .text.fc sigC,comdat fc 30
group d1.s .text.plain gD1 fd1 4
group d2.s .text.plain gD2 fd2 5
cat >"$tmp/main.c" <<'C'
int fa(void), fb(void), fc(void), fd1(void), fd2(void);
int main(void)
{
    /* 1 + 2*10 + 3*100 + 4*1000 + 5*10000 */
    return fa() + fb() * 10 + fc() * 100 + fd1() * 1000 + fd2() * 10000 == 54321 ? 0 : 1;
}
C

for m in "" -m32; do
    if [[ -n "$m" ]] && ! echo 'int main(void){return 0;}' | "$GCC" -m32 -x c - -o "$tmp/probe" 2>/dev/null; then
        echo "SKIP -m32: no 32-bit toolchain"
        continue
    fi
    tag=${m:-x86-64}
    objs=()
    for s in a b c1 c2 d1 d2; do
        "$GCC" $m -c "$tmp/$s.s" -o "$tmp/$s$m.o"
        objs+=("$tmp/$s$m.o")
    done
    "$CCC" $m -O2 -c "$tmp/main.c" -o "$tmp/main$m.o"
    # Reference: GNU ld accepts the set and the program returns 0.
    "$GCC" $m "$tmp/main$m.o" "${objs[@]}" -o "$tmp/ref$m"
    "$tmp/ref$m" || { echo "FAIL ($tag): GNU ld reference program failed" >&2; exit 1; }
    if ! "$CCC" $m "$tmp/main$m.o" "${objs[@]}" -o "$tmp/t$m" 2>"$tmp/err"; then
        echo "FAIL ($tag): lccc link failed:" >&2
        cat "$tmp/err" >&2
        exit 1
    fi
    "$tmp/t$m" || { echo "FAIL ($tag): wrong COMDAT selection (exit $?)" >&2; exit 1; }
done
echo "PASS: COMDAT groups are identified by signature"
