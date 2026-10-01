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
GCC=${GCC_BIN:-gcc}
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

# Host i386 capability probe: the shared i386_exec.sh taxonomy (this
# gate used to carry a private third copy; the 3-level model lives in the
# helper once, memoized per compiler). A host can refuse -m32 at the
# LINK (no 32-bit CRT/libgcc anywhere gcc looks), at the EXEC (the link
# succeeds through a sysroot, but there is no /lib/ld-linux.so.2 to exec,
# or a seccomp policy SIGSYSes the ia32 syscall gateway — rc 159), or not
# at all. Link success is therefore never evidence the run leg can work;
# the runtime comparison below is gated on the exact capability it needs
# (the twin logic is run_regression.py's unavailable_i386_interpreter and
# SIGSYS skips, and check_i686_tls_ie_relax.sh). The link-level alias laws
# are asserted on every host that can link.
source "$(dirname "$0")/i386_exec.sh"
i386_cap=$(i386_capability "$GCC")

# Link-level COPY-alias law, asserted for every mode the host can link
# (execution-independent). The two architectures bind the alias group
# differently and the law is stated per mode:
#   x86-64: lccc's codegen reaches DSO data directly (%rip), so every
#     member of the alias group is copy-relocated into the executable.
#     Two correct physical layouts exist — lccc-ld dedups the group into
#     ONE shared slot (both definitions at one address, verified:
#     0x4a10/0x4a10), while GNU ld emits adjacent per-symbol slots
#     (verified: 0x4018+8 == 0x4020) — and the runtime check below passes
#     under both. The implementation-independent laws are therefore:
#       1. every member carries a COPY relocation;
#       2. every member exports exactly its declared size (alias8=8,
#          alias4=4) — an under-sized alias8 is the tail-under-copy bug
#          (low 4 bytes read fine, high 4 stay zero) in the shared model,
#          and a short copy in the split model;
#       3. the [address, size) ranges never PARTIALLY overlap: either the
#          anchors coincide (shared slot) or the ranges are disjoint
#          (split slots). A partial overlap means one copy overwrites
#          another's bytes and both read corrupt data.
#   i686: lccc's i386 codegen reaches DSO data GOT-indirectly
#     (R_386_GOT32 in the object), so the aliases are bound by GLOB_DAT
#     GOT entries, not by copies; the link law is that every member of
#     the group carries its GOT binding.
# Hosts that cannot execute i386 keep the full runtime check for x86-64
# and the link law for both modes.
copy_alias_law() { # $1 = executable, $2 = mode label
    local exe=$1 m=$2 relocs
    # --use-dynamic: the i386 lccc-ld output carries no section headers,
    # so a section-driven readelf shows nothing (same reason as
    # check_linker_notype_code.sh).
    relocs=$(readelf -rW --use-dynamic "$exe")
    if [[ -z "$m" ]]; then
        for a in alias8 alias4; do
            awk -v sym="$a" '$3 ~ /COPY$/ && $5 == sym {found=1} END {exit !found}' \
                <<<"$relocs" || {
                echo "FAIL${m:+ ($m)}: no COPY relocation for $a" >&2
                exit 1
            }
        done
        # dyn-syms columns: Num: Value Size Type Bind Vis Ndx Name. Value is
        # hex without 0x, Size decimal for these small values; bash base
        # arithmetic (16#/10#) parses both without gawk-only strtonum.
        # One readelf + one awk pass replaces four readelf|awk|head
        # pipelines (12 processes -> 2, one ELF parse instead of four).
        # awk reads to EOF, so readelf can never take SIGPIPE under
        # `set -o pipefail` (check_pipefail_sigpipe.py); it keeps the FIRST
        # row per name exactly like the old `head -1`, and the END block
        # always emits four lines (empty for a missing symbol), so the four
        # `read`s assign positional empties without field shifting. A
        # readelf or awk failure still fails the assignment and trips set
        # -e, so a real producer error stays red.
        local ad8 ad4 sz8 sz4 a8 a4 syms
        syms=$(readelf --dyn-syms -W "$exe" | awk '
            $8 == "alias8" && !a8 { ad8 = $2; sz8 = $3; a8 = 1 }
            $8 == "alias4" && !a4 { ad4 = $2; sz4 = $3; a4 = 1 }
            END { print ad8; print sz8; print ad4; print sz4 }')
        { read -r ad8; read -r sz8; read -r ad4; read -r sz4; } <<<"$syms"
        if [[ -z $ad8 || -z $ad4 ]]; then
            echo "FAIL${m:+ ($m)}: alias definitions missing from .dynsym" >&2
            exit 1
        fi
        sz8=$((10#$sz8)) sz4=$((10#$sz4)) a8=$((16#$ad8)) a4=$((16#$ad4))
        if [[ $sz8 -ne 8 || $sz4 -ne 4 ]]; then
            echo "FAIL${m:+ ($m)}: exported alias sizes wrong (alias8=$sz8 want 8, alias4=$sz4 want 4)" >&2
            exit 1
        fi
        if [[ $a8 -ne $a4 \
              && $((a8 + sz8)) -gt $a4 && $((a4 + sz4)) -gt $a8 ]]; then
            printf 'FAIL%s: alias ranges partially overlap (alias8@0x%x+8, alias4@0x%x+4)\n' \
                "${m:+ ($m)}" "$a8" "$a4" >&2
            exit 1
        fi
    else
        local a
        for a in alias8 alias4; do
            awk -v sym="$a" '$3 == "R_386_GLOB_DAT" && $5 == sym {found=1} END {exit !found}' \
                <<<"$relocs" || {
                echo "FAIL${m:+ ($m)}: no GLOB_DAT GOT binding for $a" >&2
                exit 1
            }
        done
    fi
}

for m in "" -m32; do
    if [[ -n "$m" && "$i386_cap" == none ]]; then
        echo "SKIP (-m32): host cannot link i386 at all"
        # Machine-readable reduced-coverage marker (run_regression.py's
        # SKIP-RUN vocabulary): a restricted host must stay visible as
        # less coverage, never as a clean PASS.
        echo "SKIP-RUN: copy-alias -m32: host cannot link i386 at all"
        m32_mode_skipped=1
        continue
    fi
    # i686 lccc-ld is ET_EXEC-only: -pie output is not implemented there.
    pie=()
    if [[ "$m" == "-m32" ]]; then pie=(-no-pie); fi
    gcc $m -shared -fPIC -o "$tmp/libaliased.so" "$tmp/aliased.s"
    "$CCC" $m -c "$tmp/use.c" -o "$tmp/use.o"
    # shellcheck disable=SC2086
    gcc $m "${pie[@]}" -B"$tmp/shim" "$tmp/use.o" -L"$tmp" -laliased -o "$tmp/use"
    copy_alias_law "$tmp/use" "$m"
    if [[ -z "$m" || "$i386_cap" == run ]]; then
        got=$(LD_LIBRARY_PATH="$tmp" "$tmp/use")
        if [[ "$got" != "$expected" ]]; then
            echo "FAIL${m:+ ($m)}: got '$got', expected '$expected'" >&2
            exit 1
        fi
    else
        echo "SKIP${m:+ ($m)}: host cannot execute i386; link-level alias law still asserted"
        echo "SKIP-RUN: copy-alias -m32: host cannot execute i386 (link-level law still asserted)"
        m32_run_skipped=1
    fi
done
# The PASS line claims exactly what ran — never more (audit F5).
if [[ "${m32_mode_skipped:-0}" == 1 ]]; then
    echo "PASS: copy-alias sizes (x86-64 only; i386 unavailable on this host)"
elif [[ "${m32_run_skipped:-0}" == 1 ]]; then
    echo "PASS: copy-alias sizes (x86-64 + i386 link-level; i386 execution unavailable on this host)"
else
    echo "PASS: copy-alias sizes (x86-64 + i386)"
fi
