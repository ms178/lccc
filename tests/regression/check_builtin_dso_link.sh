#!/usr/bin/env bash
# Shared libraries and dynamic executables linked by lccc's builtin linker
# (each target's lccc driver links with it by default), against gcc's link
# semantics:
#   - `-shared` links crti/crtbeginS/crtendS/crtn and -lgcc -lgcc_s -lc
#     (the DSO's constructor registers atexit: __dso_handle + libc_nonshared);
#   - the user's -l libraries precede the defaults in DT_NEEDED, i.e. in the
#     loader's search order (libdso's getenv interposes libc's);
#   - `-rpath` becomes DT_RUNPATH (the program finds libdso.so through
#     $ORIGIN, no LD_LIBRARY_PATH);
#   - the driver's implicit libm is not NEEDED by a program using no libm;
#   - atexit (libc_nonshared.a, via the libc.so linker script) resolves in
#     the executable as in the DSO;
#   - a missing user -l library fails the link with GNU ld's diagnostic.
#   - DT_INIT/DT_FINI exist iff _init/_fini are defined: legacy .init/.fini
#     fragments run in the program and its DSO (none on RISC-V, whose glibc
#     has no crti frame -- as with GNU ld), and a `-shared -nostartfiles`
#     object with a bare .init gets neither tag;
#   - `-nostartfiles` links no crt object, `-nostdlib`/`-nodefaultlibs` no
#     default library (an unresolved puts fails the link, as with GCC);
#   - an import is STB_WEAK iff every reference is weak: crtbeginS.o's
#     __cxa_finalize WEAK, a strong getc GLOBAL although glibc defines it weak.
#   - `--defsym=ALIAS=TARGET` with an object-defined TARGET binds ALIAS to
#     it even when a linked library exports ALIAS: no dynamic relocation for
#     ALIAS (RISC-V once kept libc's JUMP_SLOT).
# Legs: x86-64, i686 (with gcc -m32 libraries), and AArch64/RISC-V under
# qemu when cross gcc and sysroot are installed. One compile+link per leg.
#
# Usage: [CCC=target/fastbuild/lccc] bash tests/regression/check_builtin_dso_link.sh
set -u
here=$(cd "$(dirname "$0")" && pwd)
CCC=${CCC:-$here/../../target/fastbuild/lccc}
[ -x "$CCC" ] || { echo "check_builtin_dso_link: SKIP ($CCC not built)"; exit 0; }
bin=$(cd "$(dirname "$CCC")" && pwd)
src=$here/builtin_dso
work=$(mktemp -d "${TMPDIR:-/tmp}/dsolink.XXXXXX")
trap 'rm -rf "$work"' EXIT
fail=0
expect=$'dso ctor\nmain interposed 42\nmain atexit\ndso atexit'
missing='cannot find -lno_such_lib_dsolink: No such file or directory'

# leg LABEL CC READELF [RUNNER...]
leg() {
    local label=$1 cc=$2 re=$3; shift 3
    local d=$work/${label// /_}
    mkdir -p "$d"
    if ! "$cc" -O2 -fPIC -c "$src/lib.c" -o "$d/lib.o" ||
       ! "$cc" -O2 -c "$src/main.c" -o "$d/main.o"; then
        echo "FAIL [$label]: compile"; fail=1; return
    fi
    if ! "$cc" -shared "$d/lib.o" -o "$d/libdso.so" ||
       ! "$cc" "$d/main.o" -L"$d" -ldso -Wl,-rpath,'$ORIGIN' -o "$d/main"; then
        echo "FAIL [$label]: link"; fail=1; return
    fi
    local dyn_exe dyn_so got err why=""
    err=$("$cc" "$d/main.o" -L"$d" -lno_such_lib_dsolink -o "$d/bad" 2>&1) &&
        why+=" missing-lib-linked"
    grep -qF "$missing" <<<"$err" || why+=" missing-lib-diagnostic=[${err//$'\n'/|}]"
    dyn_exe=$("$re" -dW "$d/main")
    dyn_so=$("$re" -dW "$d/libdso.so")
    grep -q 'RUNPATH.*\[\$ORIGIN\]' <<<"$dyn_exe" || why+=" no-RUNPATH"
    grep -q 'NEEDED.*\[libc\.so' <<<"$dyn_so" || why+=" dso-without-libc-NEEDED"
    grep -q 'NEEDED.*\[libm\.so' <<<"$dyn_exe" && why+=" unused-libm-NEEDED"
    got=$(cd / && timeout 60 "$@" "$d/main" 2>&1)
    [ "$got" = "$expect" ] || why+=" output=[${got//$'\n'/|}]"
    # .init/.fini fragments, import binding.
    local if_expect=$'dso .init\nexe .init\nmain\nexe .fini\ndso .fini'
    [ "$label" = riscv64 ] && if_expect=main
    if "$cc" -O2 -fPIC -shared -DTAG='"dso"' -DID=dso "$src/initfini.c" -o "$d/libif.so" &&
       "$cc" -O2 -DTAG='"exe"' -DID=exe -DWITH_MAIN "$src/initfini.c" -L"$d" -lif \
           -Wl,-rpath,'$ORIGIN' -o "$d/ifmain"; then
        got=$(cd / && timeout 60 "$@" "$d/ifmain" 2>&1)
        [ "$got" = "$if_expect" ] || why+=" initfini=[${got//$'\n'/|}]"
        got=$("$re" -sW --dyn-syms "$d/libif.so" |
            awk '{n=$8; sub(/@.*/, "", n)} n=="getc"||n=="__cxa_finalize" {print n "=" $5}' |
            sort | tr '\n' ' ')
        [ "$got" = "__cxa_finalize=WEAK getc=GLOBAL " ] || why+=" import-binding=[$got]"
    else
        why+=" initfini-link"
    fi
    # -nostartfiles: no crt object, no DT_INIT for a bare .init.
    if "$cc" -O2 -fPIC -shared -nostartfiles "$src/bare.c" -o "$d/libbare.so"; then
        "$re" -sW --dyn-syms "$d/libbare.so" | grep -q '__cxa_finalize' &&
            why+=" nostartfiles-linked-crt"
        "$re" -dW "$d/libbare.so" | grep -qE '\((INIT|FINI)\)' &&
            why+=" bare-init-tagged"
    else
        why+=" nostartfiles-link"
    fi
    # --defsym over a same-named library export.
    if "$cc" -O2 "$src/defsym.c" -Wl,--defsym=getpagesize=my_getpagesize -o "$d/defsym"; then
        got=$(cd / && timeout 60 "$@" "$d/defsym" 2>&1)
        [ "$got" = 12345 ] || why+=" defsym-output=[${got//$'\n'/|}]"
        "$re" -rW "$d/defsym" | grep -q 'getpagesize' && why+=" defsym-dynamic-reloc"
    else
        why+=" defsym-link"
    fi
    # -nostdlib / -nodefaultlibs: no default library resolves puts.
    for f in -nostdlib -nodefaultlibs; do
        "$cc" "$src/nolibc.c" "$f" -o "$d/nolibc" >/dev/null 2>&1 && why+=" $f-linked-libc"
    done
    if [ -z "$why" ]; then
        echo "PASS [$label]"
    else
        echo "FAIL [$label]:$why"; fail=1
    fi
}

leg "x86-64" "$CCC" readelf
if [ -x "$bin/lccc-i686" ] && echo 'int main(void){return 0;}' | gcc -m32 -x c - -o "$work/m32" 2>/dev/null; then
    leg "i686" "$bin/lccc-i686" readelf
else
    echo "SKIP [i686: lccc-i686 or gcc -m32 unavailable]"
fi
for t in aarch64:arm riscv64:riscv; do
    tri=${t%%:*}; cc=$bin/lccc-${t##*:}
    if [ -x "$cc" ] && command -v "$tri-linux-gnu-gcc" >/dev/null 2>&1 &&
       command -v "qemu-$tri" >/dev/null 2>&1 && [ -d "/usr/$tri-linux-gnu" ]; then
        leg "$tri" "$cc" "$tri-linux-gnu-readelf" "qemu-$tri" -L "/usr/$tri-linux-gnu"
    else
        echo "SKIP [$tri: cross gcc, qemu-$tri or sysroot missing]"
    fi
done
[ "$fail" = 0 ] && echo "check_builtin_dso_link: PASS" || { echo "check_builtin_dso_link: FAIL"; exit 1; }
