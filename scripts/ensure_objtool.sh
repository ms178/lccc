#!/usr/bin/env bash
# Build objtool, the Linux kernel's object-file validator, without root.
#
# objtool decodes every instruction of an x86-64 object with the kernel's
# decoder and follows every code path with the stack state it implies. Run
# as `objtool --orc --dry-run FILE.o` it reports what a compiler's own tests
# do not ask: a `ret` reached with a moved stack pointer, two paths that meet
# with different stack depths, a function without an ELF size, instructions
# nothing reaches. scripts/objtool_differential.py runs it over the
# regression corpus for lccc and GCC and reports what only lccc draws.
#
# Sources, both from github.com (the only hosts a sandbox may reach):
#   * tools/objtool and what it includes, a sparse blob-less clone of
#     torvalds/linux at $LCCC_OBJTOOL_KERNEL_TAG (default v7.3);
#   * libelf.h and gelf.h of the elfutils release that matches the installed
#     libelf runtime, when the development package is absent.
# Needs git, make, a C compiler and the libelf runtime (libelf.so.1).
#
# Usage: scripts/ensure_objtool.sh [PREFIX]
#   Default prefix: $LCCC_OBJTOOL_PREFIX or $HOME/.cache/lccc/objtool
# Stdout is only the absolute path of the verified objtool binary.
set -euo pipefail

prefix=${1:-${LCCC_OBJTOOL_PREFIX:-$HOME/.cache/lccc/objtool}}
tag=${LCCC_OBJTOOL_KERNEL_TAG:-v7.3}
bin=$prefix/bin/objtool

verified() {
    # The binary answers, and it still tells a broken stack from a sound one.
    [[ -x $bin ]] || return 1
    local probe
    probe=$(mktemp -d)
    # shellcheck disable=SC2064
    trap "rm -rf '$probe'" RETURN
    printf '\t.text\n\t.globl sound\n\t.type sound, @function\nsound:\n\tpushq %%rbx\n\tpopq %%rbx\n\tret\n\t.size sound, .-sound\n' >"$probe/sound.s"
    printf '\t.text\n\t.globl broken\n\t.type broken, @function\nbroken:\n\tpushq %%rbx\n\tret\n\t.size broken, .-broken\n' >"$probe/broken.s"
    "${CC:-cc}" -c "$probe/sound.s" -o "$probe/sound.o" || return 1
    "${CC:-cc}" -c "$probe/broken.s" -o "$probe/broken.o" || return 1
    [[ -z $("$bin" --orc --dry-run "$probe/sound.o" 2>&1) ]] || return 1
    "$bin" --orc --dry-run "$probe/broken.o" 2>&1 | grep -F 'return with modified stack frame' >/dev/null
}

if verified; then
    printf '%s\n' "$bin"
    exit 0
fi

mkdir -p "$prefix"
work=$prefix/src
rm -rf "$work"
mkdir -p "$work"

{
    git clone --quiet --depth 1 --branch "$tag" --filter=blob:none --sparse \
        https://github.com/torvalds/linux.git "$work/linux"
    git -C "$work/linux" sparse-checkout set tools/objtool tools/include tools/lib \
        tools/arch/x86 tools/build tools/scripts arch/x86/lib arch/x86/include/asm

    libelf_flags=()
    if ! printf '#include <gelf.h>\n' | "${CC:-cc}" -E -x c - >/dev/null 2>&1; then
        runtime=$(find /usr/lib /lib -name 'libelf.so.1' 2>/dev/null | head -n 1)
        [[ -n $runtime ]] || { echo "ensure_objtool: no libelf.so.1 on this host" >&2; exit 1; }
        version=$(basename "$(readlink -f "$runtime")")   # libelf-0.188.so
        version=${version#libelf-}
        version=${version%.so}
        git clone --quiet --depth 1 --branch "elfutils-$version" --filter=blob:none --sparse \
            https://github.com/sourceware-org/elfutils.git "$work/elfutils"
        git -C "$work/elfutils" sparse-checkout set libelf
        mkdir -p "$work/sysroot/include" "$work/sysroot/lib"
        cp "$work/elfutils/libelf/libelf.h" "$work/elfutils/libelf/gelf.h" \
            "$work/elfutils/libelf/nlist.h" "$work/sysroot/include/"
        ln -sf "$runtime" "$work/sysroot/lib/libelf.so"
        libelf_flags=(EXTRA_CFLAGS="-I$work/sysroot/include"
            LIBELF_FLAGS="-I$work/sysroot/include"
            LIBELF_LIBS="-L$work/sysroot/lib -lelf")
    fi

    make -C "$work/linux/tools/objtool" -j"$(nproc)" V=0 "${libelf_flags[@]}"
    mkdir -p "$prefix/bin"
    cp "$work/linux/tools/objtool/objtool" "$bin"
    git -C "$work/linux" rev-parse HEAD >"$prefix/KERNEL_COMMIT"
} >"$prefix/build.log" 2>&1 || {
    echo "ensure_objtool: build failed, see $prefix/build.log" >&2
    tail -n 15 "$prefix/build.log" >&2
    exit 1
}
rm -rf "$work"

verified || { echo "ensure_objtool: the built objtool fails its probe" >&2; exit 1; }
printf '%s\n' "$bin"
