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
# Shared host i386-execution probe (see i386_exec.sh for why a link probe
# is not enough).
source "$(dirname "$0")/i386_exec.sh"

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
    # The run leg below executes 32-bit binaries; a host can link -m32
    # (sysroot or multilib) yet refuse to execute the result, so the
    # probe must exercise execution itself (i386_exec.sh). The link-level
    # fallback keeps validating the i686 linker's COMDAT rules — this
    # leg is their only coverage, the i686 linker's dedup code is
    # separate from the x86-64 one — on hosts that cannot run i386.
    if [[ -n "$m" ]] && ! i386_exec_ok "$GCC" "$tmp"; then
        # Can the host link -m32 at all? Same probe taxonomy as the run
        # probe above (i386_exec.sh): this is exactly its link level.
        if i386_link_ok "$GCC"; then
            echo "SKIP-RUN: comdat -m32: host cannot execute i386 (link-level COMDAT rules still asserted)"
        else
            echo "SKIP -m32: no 32-bit toolchain"
            continue
        fi
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
    if ! "$CCC" $m "$tmp/main$m.o" "${objs[@]}" -o "$tmp/t$m" 2>"$tmp/err"; then
        echo "FAIL ($tag): lccc link failed:" >&2
        cat "$tmp/err" >&2
        exit 1
    fi
    # Link-level COMDAT selection law (every host, both modes): the link
    # itself already asserts the structural rules — a wrongly dropped
    # group leaves its global UNDEFINED (unresolved symbol), and a
    # duplicate-signature group that survived to symbol resolution is a
    # duplicate-definition error. For x86-64 the selection is ALSO
    # verified directly: all five globals defined exactly once, and the
    # duplicate-signature rule's first-wins choice is visible in the
    # linked code (fc's body must be the c1 copy: movl $3, not the c2
    # copy's movl $30). The i686 lccc-ld output carries no section
    # headers, so nm/objdump cannot read it — there the structural
    # link-success assertion plus the runtime check below are the
    # coverage.
    if [[ -z "$m" ]]; then
        for sym in fa fb fc fd1 fd2; do
            nm -B "$tmp/t$m" | awk -v s="$sym" '$2 == "T" && $3 == s {n++} END {exit !(n == 1)}' || {
                echo "FAIL ($tag): $sym not defined exactly once after COMDAT selection" >&2
                exit 1
            }
        done
        if ! objdump -d "$tmp/t$m" | awk '
            /<fc>:/ {inf = 1; next}
            inf && /mov/ {
                if ($0 ~ /\$0x3,/)  {ok = 1}
                if ($0 ~ /\$0x1e,/) {bad = 1}
                inf = 0
            }
            END {exit !(ok && !bad)}'; then
            echo "FAIL ($tag): duplicate-signature COMDAT kept the wrong body for fc" >&2
            exit 1
        fi
    fi
    # Runtime confirmation of the full value composition (54321) whenever
    # the host can execute this mode.
    if [[ -z "$m" ]] || i386_exec_ok "$GCC" "$tmp"; then
        "$tmp/ref$m" || { echo "FAIL ($tag): GNU ld reference program failed" >&2; exit 1; }
        "$tmp/t$m" || { echo "FAIL ($tag): wrong COMDAT selection (exit $?)" >&2; exit 1; }
    fi
done
# The PASS line claims exactly what ran (SKIP-RUN vocabulary, like
# copy-alias/notype-code): a restricted host stays visible as reduced
# coverage, not as an unqualified PASS.
if ! i386_link_ok "$GCC"; then
    echo "PASS: COMDAT signature identity (x86-64 only; i386 unavailable on this host)"
elif ! i386_exec_ok "$GCC" "$tmp"; then
    echo "PASS: COMDAT signature identity (x86-64 + i386 link-level; i386 execution unavailable on this host)"
else
    echo "PASS: COMDAT signature identity (x86-64 + i386)"
fi
