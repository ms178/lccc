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
P="${LCCC_AGENT_CACHE_PREFIX:-${HOME}/.cache/lccc/agent-toolchains}"
D="$P/debs-root"
B="$P/binutils-2.47-x86_64-linux/bin"
link() { # link TARGET LINK_NAME
  local target=$1 name=$2
  if [[ -L $name ]]; then
    [[ $(readlink -f "$name") == $(readlink -f "$target") ]] && return 0
    sudo rm -f -- "$name"
  elif [[ -e $name ]]; then
    # A multiarch directory the distribution itself owns is a REAL directory,
    # not something this script may replace: replacing /usr/include/
    # i386-linux-gnu with a symlink to the 64-bit multiarch tree would silently
    # drop linux-libc-dev's 32-bit <asm/*.h> (a -m32 include collapse), and on
    # hosts where the same path is a symlink swapoff/ln -s would be a no-op
    # churn.  Reconcile the MEMBERS instead of the container: every entry the
    # offline extraction provides is published as a symlink INSIDE the existing
    # directory, and pre-existing members are never deleted.  This keeps the
    # 32-bit glibc headers reachable without touching distribution state.
    if [[ -d $name && -d $target ]]; then
      local member target_real rel dst
      target_real=$(readlink -f -- "$target") || return 1
      while IFS= read -r -d '' member; do
        # `find` enumerates the canonical target. Strip that same canonical
        # root—not the caller's possibly relative/symlink spelling—or a
        # symlinked target leaves the absolute source path embedded under
        # `$name`, creating an incorrect nested destination.
        case "$member" in
          "$target_real"/*) rel=${member#"$target_real"/} ;;
          *) echo "prepare_local_env.sh: target member escaped canonical root" >&2; return 1 ;;
        esac
        dst="$name/$rel"
        [[ -e $dst || -L $dst ]] && continue
        sudo mkdir -p -- "$(dirname "$dst")"
        sudo ln -s -- "$member" "$dst"
      done < <(find "$target_real" -mindepth 1 -maxdepth 1 -print0)
      return 0
    fi
    echo "prepare_local_env.sh: $name exists and is not a symlink; refusing to replace" >&2
    return 1
  fi
  sudo mkdir -p -- "$(dirname "$name")"
  sudo ln -s -- "$target" "$name"
}
# The restore script unpacks the pinned Debian archive as a directory of
# .deb files, but agent-env.sh and the 32-bit/GDB/Valgrind gates need an
# EXTRACTED root.  Rebuild it from the SAME immutable tag bytes, verify the
# archive against MANIFEST.sha256, and extract every .deb with dpkg-deb -x.
materialise_debs_root() {
  local ROOT TAG want got tmp deb
  ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
  TAG=${LCCC_AGENT_CACHE_TAG:-harness-cache-8}
  if [[ -x "$D/usr/bin/valgrind" && -d "$D/usr/lib/debug/.build-id" ]]; then
    return 0
  fi
  if ! git -C "$ROOT" cat-file -e "$TAG:MANIFEST.sha256.part00" 2>/dev/null; then
    git -C "$ROOT" fetch origin tag "$TAG" --quiet || true
  fi
  want=$(git -C "$ROOT" cat-file blob "$TAG:MANIFEST.sha256.part00" |
    awk '$2 == "debs-bookworm.tar.xz" { print $1; exit }')
  [[ -n $want ]] || { echo "prepare_local_env.sh: debs-bookworm pin missing" >&2; return 1; }
  tmp=$(mktemp -d)
  trap 'rm -rf -- "$tmp"' RETURN
  git -C "$ROOT" cat-file blob "$TAG:debs-bookworm.tar.xz.part00" >"$tmp/debs.tar.xz"
  got=$(sha256sum "$tmp/debs.tar.xz" | awk '{print $1}')
  [[ $got == "$want" ]] || { echo "prepare_local_env.sh: debs archive digest mismatch" >&2; return 1; }
  # Reject absolute or parent-traversing members before extracting trusted bytes.
  if tar -tJf "$tmp/debs.tar.xz" | grep -Eq '(^/|(^|/)\.\.(/|$))'; then
    echo "prepare_local_env.sh: unsafe member in debs archive" >&2
    return 1
  fi
  tar -xJf "$tmp/debs.tar.xz" -C "$tmp"
  mkdir -p -- "$D"
  while IFS= read -r -d '' deb; do
    dpkg-deb -x "$deb" "$D"
  done < <(find "$tmp/debs" -maxdepth 1 -type f -name '*.deb' -print0)
  rm -rf -- "$tmp"
  trap - RETURN
  if [[ ! -L "$P/pyyaml-root" && ! -e "$P/pyyaml-root" ]]; then
    ln -s -- "$D/usr/lib/python3/dist-packages" "$P/pyyaml-root"
  fi
  echo "DEBS_ROOT_OK valgrind=$D/usr/bin/valgrind debug=$D/usr/lib/debug/.build-id"
}
materialise_debs_root

# Valgrind needs ld.so's strlen redirection; publish libc6-dbg's build-id tree.
link "$D/usr/lib/debug" /usr/lib/debug
# Valgrind's tool binaries (memcheck-amd64-linux, the *-valgrind-*.xml CPU
# description files) live in usr/libexec/valgrind, and the compiled-in search
# path is the ABSOLUTE /usr/libexec/valgrind.  Without this link the driver
# starts, then dies with "failed to start tool 'memcheck' for platform
# 'amd64-linux'", which `check_hex_float_oracles.py` correctly reports as a
# FAIL of the hexadecimal-literal oracle rather than skipping Memcheck.
link "$D/usr/libexec/valgrind" /usr/libexec/valgrind
# Debian's suppression file (--default-suppressions resolves it through the
# same absolute prefix).  /usr/lib/valgrind is a REAL directory here, so the
# member is published instead of the container (see `link`).
link "$D/usr/lib/valgrind/debian.supp" /usr/lib/valgrind/debian.supp
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

# ─── Offline pinned-oracle adoption ───────────────────────────────────────
# The pinned GAS/A64-XML/kernel-relocs provisioners accept a pre-placed
# tarball/sources in $HOME/.cache/dl and re-verify its SHA-256 against their
# own pins.  A sandbox with no route to ftp.gnu.org, developer.arm.com or
# git.kernel.org otherwise fails those gates even though the exact immutable
# bytes are in the harness-cache-8 tag.  Adopt them from the Git object store
# (the ONLY network host this script may ever contact is the Git remote used
# to fetch the tag), never from an unpinned mirror, and re-check every digest.
adopt_offline_oracles() {
  local ROOT TAG DL manifest name part dest want got tmp prefix src
  ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
  TAG=${LCCC_AGENT_CACHE_TAG:-harness-cache-8}
  DL=${GAS_DL_DIR:-${ARM_XML_DL_DIR:-$HOME/.cache/dl}}
  mkdir -p -- "$DL"
  if ! git -C "$ROOT" cat-file -e "$TAG:MANIFEST.sha256.part00" 2>/dev/null; then
    git -C "$ROOT" fetch origin tag "$TAG" --quiet || true
  fi
  manifest_digest() {
    git -C "$ROOT" cat-file blob "$TAG:MANIFEST.sha256.part00" |
      awk -v n="$1" '$2 == n { print $1; exit }'
  }
  adopt_blob() {
    local part dest name want got tmp
    part=$1; dest=$2; name=$3
    want=$(manifest_digest "$name")
    [[ -n $want ]] || { echo "prepare_local_env.sh: no manifest pin for $name" >&2; return 1; }
    if [[ -f $dest ]] && [[ $(sha256sum "$dest" | awk '{print $1}') == "$want" ]]; then
      return 0
    fi
    tmp="$dest.tmp.$$"
    git -C "$ROOT" cat-file blob "$TAG:$part" >"$tmp" || { rm -f -- "$tmp"; return 1; }
    got=$(sha256sum "$tmp" | awk '{print $1}')
    if [[ $got != "$want" ]]; then
      rm -f -- "$tmp"
      echo "prepare_local_env.sh: $name digest mismatch (got $got, want $want)" >&2
      return 1
    fi
    mv -f -- "$tmp" "$dest"
  }
  # Names are exactly what scripts/ensure_gas_247.sh and
  # scripts/ensure_arm_a64_xml.sh look for before they try a mirror.
  adopt_blob "binutils-2.47-source.tar.xz.part00" \
             "$DL/binutils-2.47.tar.xz" "binutils-2.47-source.tar.xz"
  adopt_blob "ISA_A64_xml_A_profile-2026-09_md.tar.gz.part00" \
             "$DL/ISA_A64_xml_A_profile-2026-09_md.tar.gz" \
             "ISA_A64_xml_A_profile-2026-09_md.tar.gz"
  # tests/linker/setup_kernel_tools.sh adopts pre-placed SOURCES, not the
  # prebuilt relocs binary, and re-checks all six per-file pins itself.
  prefix="$HOME/.cache/lccc-kernel-tools/kernel-tools"
  mkdir -p -- "$prefix/tools"
  tmp=$(mktemp -d)
  trap 'rm -rf -- "$tmp"' RETURN
  adopt_blob "kernel-relocs-v7.3.1-x86_64.tar.xz.part00" \
             "$tmp/kernel-relocs.tar.xz" "kernel-relocs-v7.3.1-x86_64.tar.xz"
  tar -xJf "$tmp/kernel-relocs.tar.xz" -C "$tmp"
  src="$tmp/kernel-relocs-v7.3.1-x86_64/src"
  for name in relocs.c relocs.h relocs_common.c relocs_32.c relocs_64.c; do
    cp -f -- "$src/$name" "$prefix/$name"
  done
  cp -f -- "$src/tools/le_byteshift.h" "$prefix/tools/le_byteshift.h"
  rm -rf -- "$tmp"
  trap - RETURN
  echo "OFFLINE_ORACLES_OK dl=$DL kernel=$prefix tag=$TAG"
}
adopt_offline_oracles
echo "PREPARE_LOCAL_ENV_OK debug=$(readlink -f /usr/lib/debug) sys=$(readlink -f /usr/include/sys) gas=$G/bin/as"
