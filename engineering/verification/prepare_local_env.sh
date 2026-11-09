#!/usr/bin/env bash
# Idempotent sandbox-local repairs (tracked files untouched, no network).
#
# The harness-cache-8 toolchain is a RELOCATED Debian toolchain: GCC 16.2 and
# binutils 2.47 were extracted below /home/user/.cache, while the sandbox
# root filesystem has only a partial glibc.  A plain `gcc-16 -m32` therefore
# cannot find sys/cdefs.h, gnu/stubs.h, crt1.o, libgcc.a or ld-linux.so.2,
# and the cached `ld` has no Debian multiarch SEARCH_DIR for crt1.o.  CI
# runners have those paths; this script reproduces them with symlinks that
# point ONLY at the verified offline Debian extraction.
set -euo pipefail
P="${LCCC_AGENT_CACHE_PREFIX:-/home/user/.cache/lccc/agent-tools}"
D="$P/debs-root"
B="$P/binutils-2.47-x86_64-linux/bin"
link() { # link TARGET LINK_NAME
  local target=$1 name=$2
  if [[ -L $name ]]; then
    [[ $(readlink -f "$name") == $(readlink -f "$target") ]] && return 0
    sudo rm -f -- "$name"
  elif [[ -e $name ]]; then
    echo "prepare_local_env.sh: $name exists and is not a symlink; refusing to replace" >&2
    return 1
  fi
  sudo mkdir -p -- "$(dirname "$name")"
  sudo ln -s -- "$target" "$name"
}
# Valgrind needs ld.so's strlen redirection; publish libc6-dbg's build-id tree.
link "$D/usr/lib/debug" /usr/lib/debug
# glibc multiarch headers.  /usr/include/bits was the first symptom; the
# -m32 GCC/LCCC rows additionally need sys/cdefs.h and gnu/stubs.h.
link /usr/include/x86_64-linux-gnu/bits /usr/include/bits
link /usr/include/x86_64-linux-gnu/sys /usr/include/sys
link /usr/include/x86_64-linux-gnu/gnu /usr/include/gnu
link /usr/include/x86_64-linux-gnu /usr/include/i386-linux-gnu
# glibc's generated stubs.h selects stubs-32.h for ILP32.  The 64-bit
# multiarch directory only ships stubs-64.h; the offline libc6-dev-i386
# extraction supplies the missing 32-bit header.  Without this, LCCC -m32
# fails preprocessing at gnu/stubs.h:7 in every runtime gate.
link "$D/usr/include/x86_64-linux-gnu/gnu/stubs-32.h" /usr/include/x86_64-linux-gnu/gnu/stubs-32.h
# 32-bit CRT/libc/libgcc and the ELF interpreter.  The Debian libc.so linker
# script names these as ABSOLUTE paths, so -L alone cannot relocate them.
link "$D/lib32" /lib32
link "$D/usr/lib32" /usr/lib32
link "$D/usr/lib32" /usr/lib/i386-linux-gnu
link "$D/lib/ld-linux.so.2" /lib/ld-linux.so.2
link "$D/usr/lib/gcc/x86_64-linux-gnu/12" /usr/lib/gcc/i686-linux-gnu/12
# Pinned-oracle gates hardcode this path; the cached bin/as is a wrapper that
# resolves libexec relative to its own prefix, so mirror the WHOLE prefix.
G="$HOME/.cache/gas-2.47-x86_64-linux-gnu"
if [[ ! -L $G ]]; then
  rm -rf -- "$G"
  mkdir -p -- "$(dirname "$G")"
  ln -s -- "$(dirname "$B")" "$G"
fi
echo "PREPARE_LOCAL_ENV_OK debug=$(readlink -f /usr/lib/debug) sys=$(readlink -f /usr/include/sys) gas=$G/bin/as"
