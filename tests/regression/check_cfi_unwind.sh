#!/usr/bin/env bash
# Run-time witness that libgcc's unwinder walks through lccc-compiled frames
# on every target: the integrated assemblers lower `.cfi_*` into
# `.eh_frame` (src/backend/cfi_lower.rs; RV-AS-CFI/A64-AS-CFI: AArch64 and
# RISC-V dropped every CFI directive, so no unwinder -- pthread_cancel,
# C++ exceptions through C callbacks, backtrace(), gdb -- got past the first
# lccc frame). cfi_unwind/unwind.c fails unless _Unwind_Backtrace reports
# the FDE of every function on the chain main -> f1 -> lib_entry -> f2 ->
# f3, built as one executable and split into executable + shared library
# (the unwinder reaches a library's FDEs only through its PT_GNU_EH_FRAME:
# RV-LD-EHFRAMEHDR, lccc-ld wrote that header pointing at address 0 in
# executables and none at all in RISC-V shared libraries).
#
# Legs: x86-64 (bfd, lccc-ld, mold, lld), i686, and AArch64/RISC-V under
# qemu when the cross toolchain and sysroot are installed (also linked by
# the builtin lccc-ld), each at -O0 and -O2, whole and split.
set -uo pipefail
here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
CCC=${CCC:-$here/../../target/fastbuild/lccc}
bin=$(cd "$(dirname "$CCC")" && pwd)
src=$here/cfi_unwind/unwind.c
work=$(mktemp -d "${TMPDIR:-/tmp}/cfiunw.XXXXXX")
trap 'rm -rf "$work"' EXIT
fail=0
n=0

# leg LABEL CC "LINK-DRIVER" "LINK-FLAGS" [RUNNER...]
leg() {
    local label=$1 cc=$2 drv=$3 lflags=$4; shift 4
    case " $lflags " in
    *" -B$shim "*) shim_live || { echo "SKIP [$label: lccc-ld not built]"; return; } ;;
    esac
    local o
    for o in 0 2; do
        local d=$work/$((n += 1))
        mkdir -p "$d"
        if ! "$cc" -O$o -c "$src" -o "$d/u.o"; then
            echo "FAIL [$label -O$o]: compile"; fail=1; continue
        fi
        # The object itself must carry unwind tables.
        if ! readelf -SW "$d/u.o" | grep -Ec '\.eh_frame +PROGBITS +[0-9a-f]+ +[0-9a-f]+ +0*[1-9a-f]' >/dev/null; then
            echo "FAIL [$label -O$o]: no .eh_frame in the object"; fail=1; continue
        fi
        if ! $drv $lflags "$d/u.o" -o "$d/u"; then
            echo "FAIL [$label -O$o]: link"; fail=1; continue
        fi
        run "$label -O$o" "$d/u" "$@"
        if ! "$cc" -O$o -fPIC -DUNWIND_LIB -c "$src" -o "$d/l.o" ||
           ! "$cc" -O$o -DUNWIND_MAIN -c "$src" -o "$d/m.o"; then
            echo "FAIL [$label -O$o shared]: compile"; fail=1; continue
        fi
        if ! $drv $lflags -shared "$d/l.o" -o "$d/libunw.so" ||
           ! $drv $lflags "$d/m.o" -L"$d" -lunw -Wl,-rpath,'$ORIGIN' -o "$d/m"; then
            echo "FAIL [$label -O$o shared]: link"; fail=1; continue
        fi
        run "$label -O$o shared" "$d/m" "$@"
    done
}

# run LABEL EXE [RUNNER...]
run() {
    local label=$1 exe=$2; shift 2
    local got rc
    got=$(timeout 60 "$@" "$exe" 2>&1); rc=$?
    if [ "$rc" = 0 ]; then
        echo "PASS [$label] $got"
    else
        echo "FAIL [$label] rc=$rc: $got"; fail=1
    fi
}

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
    if command -v "ld.$l" >/dev/null 2>&1; then
        leg "x86-64 $l" "$CCC" gcc "-fuse-ld=$l"
    else
        echo "SKIP [x86-64 $l: ld.$l not found]"
    fi
done
if [ -x "$bin/lccc-i686" ] && echo 'int main(void){return 0;}' | gcc -m32 -x c - -o "$work/m32" 2>/dev/null; then
    leg "i686 bfd" "$bin/lccc-i686" "gcc -m32" ""
    # lccc-ld writes i386 executables as ET_EXEC only (it refuses -pie).
    leg "i686 lccc-ld (non-PIE)" "$bin/lccc-i686" "gcc -m32" "-no-pie -B$shim"
else
    echo "SKIP [i686: lccc-i686 or gcc -m32 unavailable]"
fi
for t in aarch64:arm riscv64:riscv; do
    tri=${t%%:*}; cc=$bin/lccc-${t##*:}
    if [ -x "$cc" ] && command -v "$tri-linux-gnu-gcc" >/dev/null 2>&1 &&
       command -v "qemu-$tri" >/dev/null 2>&1 && [ -d "/usr/$tri-linux-gnu" ]; then
        leg "$tri bfd" "$cc" "$tri-linux-gnu-gcc" "" "qemu-$tri" -L "/usr/$tri-linux-gnu"
        # The lccc driver links with its builtin linker by default.
        leg "$tri lccc-ld" "$cc" "$cc" "" "qemu-$tri" -L "/usr/$tri-linux-gnu"
    else
        echo "SKIP [$tri: cross gcc, qemu-$tri or sysroot missing]"
    fi
done
[ "$fail" = 0 ] && echo "check_cfi_unwind: PASS" || { echo "check_cfi_unwind: FAIL"; exit 1; }
