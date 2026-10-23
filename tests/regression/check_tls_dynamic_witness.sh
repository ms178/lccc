#!/usr/bin/env bash
# Run-time witness of the dynamic TLS models on every target LCCC emits.
#
# tls_witness/lib.c and dl.c are compiled by LCCC with -fPIC (Global- and
# Local-Dynamic: x86 TLS descriptors, AArch64 descriptors, RISC-V
# __tls_get_addr); main.c is a PIE that reads lib's TLS through Initial-Exec.
# The program checks values per thread, the address identity between the
# executable's IE access and the library's GD access, a function with many
# values live across a dynamic access (a call on AArch64/RISC-V: the register
# allocator must not keep them in caller-saved registers), and dlopen of a
# library with a 1 MiB TLS block, which only a dynamic model can load (the
# fixture self-check proves an Initial-Exec build of it fails).
#
# Legs: x86-64 (bfd, lccc-ld, mold, lld when present; plus GCC gnu2 objects
# through lccc-ld: the _TLS_MODULE_BASE_ anchor), i686 (bfd, mold:
# PIE, @GOTNTPOFF; lccc-ld: non-PIE, @INDNTPOFF), AArch64 and RISC-V under qemu-user when the cross toolchain exists.
# A missing optional tool is reported as SKIP, never as a pass.
set -uo pipefail
here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
CCC=${CCC:-$here/../../target/fastbuild/lccc}
bin=$(cd "$(dirname "$CCC")" && pwd)
src=$here/tls_witness
work=$(mktemp -d "${TMPDIR:-/tmp}/tlswit.XXXXXX")
trap 'rm -rf "$work"' EXIT
want=$'27 1 613\n59614 27\n284\n101 102'
fail=0
n=0

# leg <label> <lccc> <driver> <extra link flags> [runner...]
# MAIN=nopie builds the executable -fno-pie/-no-pie (i686 lccc-ld emits
# ET_EXEC only and refuses -pie explicitly); default is a PIE.
# LEG_CFLAGS adds compile flags; NEED_SYM fails the leg unless lib.o and
# dl.o reference that symbol (the leg must exercise what it claims to).
leg() {
    local label=$1 cc=$2 drv=$3 lflags=$4; shift 4
    case " $lflags " in
    *" -B$shim "*) shim_live || { echo "SKIP [$label: lccc-ld not built]"; return; } ;;
    esac
    local mc=-fPIE ml=-pie
    [ "${MAIN:-pie}" = nopie ] && mc=-fno-pie ml=-no-pie
    local d=$work/$((n += 1))
    mkdir -p "$d"
    local cf=${LEG_CFLAGS:-}
    # shellcheck disable=SC2086 # $cf is a flag list
    if ! "$cc" $cf -O1 -fPIC -c "$src/lib.c" -o "$d/lib.o" ||
       ! "$cc" $cf -O1 -fPIC -c "$src/dl.c" -o "$d/dl.o" ||
       ! "$cc" $cf -O1 "$mc" -c "$src/main.c" -o "$d/main.o"; then
        echo "FAIL [$label]: compile"; fail=1; return
    fi
    if [ -n "${NEED_SYM:-}" ] && ! { nm "$d/lib.o" | grep -c " $NEED_SYM\$" >/dev/null &&
                                      nm "$d/dl.o" | grep -c " $NEED_SYM\$" >/dev/null; }; then
        echo "FAIL [$label]: objects do not reference $NEED_SYM"; fail=1; return
    fi
    # shellcheck disable=SC2086 # $lflags is a flag list
    if ! $drv $lflags -shared "$d/lib.o" -o "$d/libtw.so" ||
       ! $drv $lflags -shared "$d/dl.o" -o "$d/libdl_tw.so" ||
       ! $drv $lflags $ml "$d/main.o" -L"$d" -ltw -ldl -lpthread \
             -Wl,-rpath,'$ORIGIN' -o "$d/main"; then
        echo "FAIL [$label]: link"; fail=1; return
    fi
    local got
    got=$(cd "$d" && timeout 60 "$@" ./main 2>&1)
    if [ "$got" = "$want" ]; then
        echo "PASS [$label]"
    else
        echo "FAIL [$label]: got: $(tr '\n' '|' <<<"$got")"; fail=1
    fi
}

# Fixture self-check: Initial-Exec cannot dlopen the 1 MiB TLS library.
if gcc -O1 -fPIC -ftls-model=initial-exec -shared "$src/dl.c" -o "$work/libdl_tw.so" &&
   printf '#include <dlfcn.h>\n#include <stdio.h>\nint main(void){puts(dlopen("./libdl_tw.so",RTLD_NOW)?"loaded":"refused");return 0;}\n' >"$work/m.c" &&
   gcc "$work/m.c" -o "$work/m" -ldl && [ "$(cd "$work" && ./m)" = "refused" ]; then
    echo "PASS [fixture: IE build of dl.c is refused by dlopen]"
else
    echo "SKIP [fixture self-check: this libc loads the IE build; the dlopen leg cannot discriminate]"
fi

shim=$work/shim
mkdir -p "$shim"
ln -s "$bin/lccc-ld" "$shim/ld"
# gcc falls back to the system ld without a word when `-B` finds no usable
# `ld`, so a missing lccc-ld would turn every lccc-ld leg into a second,
# passing GNU ld leg. Those legs run only when gcc provably resolves the shim.
shim_live() { [ "$(gcc -B"$shim" -print-prog-name=ld 2>/dev/null)" = "$shim/ld" ]; }
leg "x86-64 bfd" "$CCC" gcc ""
leg "x86-64 lccc-ld" "$CCC" gcc "-B$shim"
for l in mold lld; do
    if command -v "ld.$l" >/dev/null 2>&1; then leg "x86-64 $l" "$CCC" gcc "-fuse-ld=$l"; else echo "SKIP [x86-64 $l: ld.$l not found]"; fi
done
# A Local-Exec reference to TLS a shared library defines has no link-time
# value: lccc-ld must refuse it like GNU ld ("unresolvable R_X86_64_TPOFF32"),
# not write an offset into the executable's own block.
# refuse_le <label> <lccc> <driver>
refuse_le() {
    shim_live || { echo "SKIP [$1 LE vs DSO: lccc-ld not built]"; return; }
    local d=$work/le$((n += 1))
    mkdir -p "$d"
    printf '__thread int x = 5;\n' >"$d/l.c"
    printf 'extern __thread int x;\nint main(void){return x;}\n' >"$d/m.c"
    $3 -fPIC -shared "$d/l.c" -o "$d/libl.so" &&
        "$2" -O1 -fno-pie -ftls-model=local-exec -c "$d/m.c" -o "$d/m.o" || {
        echo "FAIL [$1 LE vs DSO]: setup"; fail=1; return; }
    local err
    if err=$($3 -no-pie "-B$shim" "$d/m.o" -L"$d" -ll -o "$d/m" 2>&1); then
        echo "FAIL [$1 LE vs DSO]: lccc-ld linked it"; fail=1
    elif grep -q "unresolvable R_.*shared-library TLS symbol" <<<"$err"; then
        echo "PASS [$1 LE vs DSO refused by lccc-ld]"
    else
        echo "FAIL [$1 LE vs DSO]: wrong diagnostic: $err"; fail=1
    fi
}
refuse_le x86-64 "$CCC" gcc

# GCC's gnu2 Local-Dynamic code anchors every local TLS access on the
# linker-defined `_TLS_MODULE_BASE_` (one descriptor call, then
# `%fs:v@dtpoff(%rax)`).  lccc-ld must define it -- hidden, at the TLS block
# start in a DSO and at its end (the TP) in an executable -- or the link fails
# (executable) or the DSO dies at load time with an undefined dynamic symbol.
LEG_CFLAGS=-mtls-dialect=gnu2 NEED_SYM=_TLS_MODULE_BASE_ \
    leg "x86-64 lccc-ld, GCC gnu2 objects (_TLS_MODULE_BASE_)" gcc gcc "-B$shim"

if [ -x "$bin/lccc-i686" ] && echo 'int main(void){return 0;}' | gcc -m32 -x c - -o "$work/m32" 2>/dev/null; then
    leg "i686 bfd" "$bin/lccc-i686" "gcc -m32" ""
    refuse_le i686 "$bin/lccc-i686" "gcc -m32"
    MAIN=nopie leg "i686 lccc-ld (non-PIE: @INDNTPOFF)" "$bin/lccc-i686" "gcc -m32" "-B$shim"
    LEG_CFLAGS="-m32 -mtls-dialect=gnu2" NEED_SYM=_TLS_MODULE_BASE_ MAIN=nopie \
        leg "i686 lccc-ld, GCC gnu2 objects (_TLS_MODULE_BASE_)" gcc "gcc -m32" "-B$shim"
    if command -v ld.mold >/dev/null 2>&1; then leg "i686 mold" "$bin/lccc-i686" "gcc -m32" "-fuse-ld=mold"; else echo "SKIP [i686 mold]"; fi
else
    echo "SKIP [i686: lccc-i686 or gcc -m32 unavailable]"
fi
for t in aarch64:arm riscv64:riscv; do
    tri=${t%%:*}; cc=$bin/lccc-${t##*:}
    if [ -x "$cc" ] && command -v "$tri-linux-gnu-gcc" >/dev/null 2>&1 &&
       command -v "qemu-$tri" >/dev/null 2>&1 && [ -d "/usr/$tri-linux-gnu" ]; then
        leg "$tri" "$cc" "$tri-linux-gnu-gcc" "" "qemu-$tri" -L "/usr/$tri-linux-gnu"
    else
        echo "SKIP [$tri: cross gcc, qemu-$tri or sysroot missing]"
    fi
done
[ "$fail" = 0 ] && echo "check_tls_dynamic_witness: PASS" || { echo "check_tls_dynamic_witness: FAIL"; exit 1; }
