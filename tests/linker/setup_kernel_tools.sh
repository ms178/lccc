#!/usr/bin/env bash
# ============================================================================
# Build the Linux kernel host tools the linker test-suite validates against.
#
# Currently: arch/x86/tools/relocs — the *real* consumer of `--emit-relocs`.
#
# Why this matters more than a structural check: `--emit-relocs` exists so that
# CONFIG_RELOCATABLE / CONFIG_RANDOMIZE_BASE (KASLR) kernels can be slid to a
# random base at boot.  A linker can emit .rela sections that look perfectly
# well-formed to readelf and still be useless — wrong symbol indices, wrong
# addend convention, missing section symbols — and the only failure signal
# would be a kernel that builds cleanly and then does not boot.
#
# Running the kernel's own tool over the linked image and comparing the
# relocation set it derives against the set it derives from GNU ld's image is
# the strongest available check short of booting a kernel in QEMU.
#
# The sources are fetched from kernel.org's cgit (NOT GitHub, which rate-limits
# raw fetches aggressively and silently returns a 429 HTML body that then fails
# to compile with a confusing error).
#
# Usage:
#   tests/linker/setup_kernel_tools.sh [--kver v6.12] [--prefix DIR]
#
# The suite picks the tool up from $LCCC_RELOCS_TOOL, defaulting to
# <prefix>/bin/relocs, and SKIPs cleanly when it is absent.
# ============================================================================
set -euo pipefail

KVER=${KVER:-v6.12}
PREFIX=${LCCC_ORACLE_PREFIX:-/home/user/tools}

while [[ $# -gt 0 ]]; do
  case $1 in
    --kver)   KVER=$2; shift 2 ;;
    --prefix) PREFIX=$2; shift 2 ;;
    *) echo "unknown option: $1" >&2; exit 2 ;;
  esac
done

BIN="$PREFIX/bin"
SRC="$PREFIX/kernel-tools"
mkdir -p "$BIN" "$SRC/tools"

if [[ -x "$BIN/relocs" ]]; then
  echo "relocs already built: $BIN/relocs"
  exit 0
fi

BASE="https://git.kernel.org/pub/scm/linux/kernel/git/stable/linux.git/plain"

echo "fetching arch/x86/tools/relocs sources at $KVER"
for f in relocs.c relocs.h relocs_common.c relocs_32.c relocs_64.c; do
  curl -fsSL -o "$SRC/$f" "$BASE/arch/x86/tools/$f?h=$KVER"
done
# relocs.h includes <tools/le_byteshift.h> from the kernel's tools/include.
curl -fsSL -o "$SRC/tools/le_byteshift.h" \
     "$BASE/tools/include/tools/le_byteshift.h?h=$KVER"

# Sanity: a rate-limited or redirected fetch yields an HTML error page that
# compiles into a wall of nonsense. Catch it here with a clear message.
if ! head -1 "$SRC/relocs.c" | grep -q "SPDX-License-Identifier"; then
  echo "error: relocs.c does not look like kernel source (download blocked?)" >&2
  head -3 "$SRC/relocs.c" >&2
  exit 1
fi

echo "building relocs"
( cd "$SRC" && cc -O2 -I. -o relocs relocs_common.c relocs_32.c relocs_64.c )
install -m755 "$SRC/relocs" "$BIN/relocs"

echo "relocs ready: $BIN/relocs ($KVER)"
echo
echo "The linker suite uses it automatically:"
echo "    tests/linker/run_linker_tests.py --filter emit_relocs"
