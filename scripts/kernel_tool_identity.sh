#!/usr/bin/env bash
# Invalidate Kbuild products when LCCC tool binaries change.
#
# Kbuild fingerprints command lines, not the executable content behind CC/LD.
# A compiler rebuilt at the same path can therefore leave every old object
# looking current.  Compiler and linker changes have deliberately different
# invalidation domains: compiler changes require Kbuild's complete `clean`,
# while linker-only changes retain expensive, semantically valid object files.
#
# Usage: kernel_tool_identity.sh KERNEL_DIR LCCC LCCC_LD
set -euo pipefail

if (($# != 3)); then
  echo "usage: $0 KERNEL_DIR LCCC LCCC_LD" >&2
  exit 2
fi
K=$1
LCCC=$2
LCCC_LD=$3
[[ -d $K ]] || { echo "kernel_tool_identity: not a directory: $K" >&2; exit 2; }
[[ -f $LCCC ]] || { echo "kernel_tool_identity: compiler not found: $LCCC" >&2; exit 2; }
[[ -f $LCCC_LD ]] || { echo "kernel_tool_identity: linker not found: $LCCC_LD" >&2; exit 2; }

cd "$K"
compiler_stamp=.lccc-compiler-stamp
linker_stamp=.lccc-linker-stamp
compiler_hash=$(sha256sum "$LCCC" | cut -d' ' -f1)
linker_hash=$(sha256sum "$LCCC_LD" | cut -d' ' -f1)
compiler_changed=0

if [[ -f $compiler_stamp && $(cat "$compiler_stamp") != "$compiler_hash" ]]; then
  echo "kernel_tool_identity: lccc changed; cleaning all Kbuild products"
  # This is the authoritative product inventory.  A hand-written object glob
  # inevitably misses generated assembly, archives, and architecture products.
  # `clean` preserves .config, which the following olddefconfig refresh needs.
  make ARCH=x86_64 clean >/dev/null
  compiler_changed=1
fi

if [[ $compiler_changed == 0 && -f $linker_stamp && $(cat "$linker_stamp") != "$linker_hash" ]]; then
  echo "kernel_tool_identity: lccc-ld changed; purging link-only products"
  rm -f arch/x86/entry/vdso/vdso*.so* \
        arch/x86/entry/vdso/vdso-image-*.c \
        arch/x86/boot/setup.elf arch/x86/boot/setup.bin \
        arch/x86/boot/compressed/vmlinux* \
        arch/x86/boot/bzImage vmlinux .tmp_vmlinux* vmlinux.symvers vmlinux.map
fi

# Atomic publication is fail-safe: an interruption retains an older complete
# hash and causes a harmless extra clean, never trust in stale products.
write_stamp() {
  local path=$1 value=$2 tmp
  tmp=$(mktemp "${path}.tmp.XXXXXX")
  printf '%s\n' "$value" > "$tmp"
  mv -f "$tmp" "$path"
}
write_stamp "$compiler_stamp" "$compiler_hash"
write_stamp "$linker_stamp" "$linker_hash"
# Retire the old combined stamp.  It could not distinguish compiler changes
# from linker-only changes and therefore invalidated too little.
rm -f .lccc-tools-stamp
