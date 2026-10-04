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
# The sources are fetched from kernel.org's cgit first (NOT GitHub, which
# rate-limits raw fetches aggressively and silently returns a 429 HTML body
# that then fails to compile with a confusing error).  When kernel.org is
# unreachable from the build network (403/blocked egress — observed in
# locked-down research sandboxes), the same path is retried ONCE from the
# gregkh/linux stable mirror on GitHub.  The mirror is safe to fall back
# to precisely because every file is pinned by SHA-256 below: a doctored
# or HTML body cannot pass the pin check, so the fallback can only ever
# deliver the exact v6.12 sources or a hard error.
#
# Usage:
#   tests/linker/setup_kernel_tools.sh [--kver v6.12] [--prefix DIR]
#
# The suite picks the tool up from $LCCC_RELOCS_TOOL, defaulting to
# ${LCCC_ORACLE_PREFIX:-$HOME/tools}/bin/relocs; without it the test reports
# SKIP, which `run_linker_tests.py --strict` (CI) turns into a failure.
#
# Every fetched file is pinned by SHA-256 (the v6.12 contents), so a changed
# tag, a proxy or an HTML error page fails here instead of building something
# else.  Another --kver needs its own pins: --no-verify is for exploration
# only, never CI.
# ============================================================================
set -euo pipefail

KVER=${KVER:-v6.12}
PREFIX=${LCCC_ORACLE_PREFIX:-$HOME/tools}
VERIFY=1

while [[ $# -gt 0 ]]; do
  case $1 in
    --kver)   KVER=$2; shift 2 ;;
    --prefix) PREFIX=$2; shift 2 ;;
    --no-verify) VERIFY=0; shift ;;
    *) echo "unknown option: $1" >&2; exit 2 ;;
  esac
done

if [[ $VERIFY == 1 && $KVER != v6.12 ]]; then
  echo "error: no SHA-256 pins for $KVER (pins cover v6.12; --no-verify to explore)" >&2
  exit 1
fi

BIN="$PREFIX/bin"
SRC="$PREFIX/kernel-tools"
mkdir -p "$BIN" "$SRC/tools"

if [[ -x "$BIN/relocs" ]]; then
  echo "relocs already built: $BIN/relocs"
  exit 0
fi

BASE="https://git.kernel.org/pub/scm/linux/kernel/git/stable/linux.git/plain"
MIRROR="https://raw.githubusercontent.com/gregkh/linux"

# fetch <dest> <cgit-path-with-query>: kernel.org first, mirror fallback.
# The SHA-256 pins (and the SPDX first-line sanity check) make the fallback
# tamper-proof; a mirror outage or HTML body dies with the pin error.
fetch() {
  local dest=$1 path=$2
  if ! curl -fsSL --max-time 60 -o "$dest" "$BASE/$path"; then
    echo "note: kernel.org fetch failed for $path; trying the gregkh mirror" >&2
    curl -fsSL --max-time 60 -o "$dest" "$MIRROR/$KVER/${path%%\?*}"
  fi
}

echo "fetching arch/x86/tools/relocs sources at $KVER"
for f in relocs.c relocs.h relocs_common.c relocs_32.c relocs_64.c; do
  fetch "$SRC/$f" "arch/x86/tools/$f?h=$KVER"
done
# relocs.h includes <tools/le_byteshift.h> from the kernel's tools/include.
fetch "$SRC/tools/le_byteshift.h" "tools/include/tools/le_byteshift.h?h=$KVER"

# Sanity: a rate-limited or redirected fetch yields an HTML error page that
# compiles into a wall of nonsense. Catch it here with a clear message.
if ! grep -c 'SPDX-License-Identifier' <(sed -n '1p' "$SRC/relocs.c") >/dev/null; then
  echo "error: relocs.c does not look like kernel source (download blocked?)" >&2
  head -3 "$SRC/relocs.c" >&2
  exit 1
fi

if [[ $VERIFY == 1 ]]; then
  ( cd "$SRC" && sha256sum -c --quiet - ) <<'PINS' || { echo "error: kernel tool sources do not match the v6.12 pins" >&2; exit 1; }
e23d46f4cb098aef1c80862a4aaa176c21b53ba12699576ec23e366428897f8f  relocs.c
61783eeddcf582c6f8c74c63f8f5170df51e34819a8b8dad9af68117f5c185d0  relocs.h
a49f30d39b362c29bdd27f19cadffdcfefefe5ad35511dd4b8f00d002a1cf08f  relocs_common.c
ae8b4ffcfe0367bb457a73b9a9534bf2ec495e2f7d99e23b1257b0009d693680  relocs_32.c
88e45459755585c934a9e24ebb9f6b71fd27f8d10c2fa9cc4a4d51fe3719166b  relocs_64.c
87b40e54fed5b27a9acd784e41521d8bbf95922448d07bea17be6b4bb5fa7925  tools/le_byteshift.h
PINS
fi

echo "building relocs"
( cd "$SRC" && cc -O2 -I. -o relocs relocs_common.c relocs_32.c relocs_64.c )
install -m755 "$SRC/relocs" "$BIN/relocs"

echo "relocs ready: $BIN/relocs ($KVER)"
echo
echo "The linker suite uses it automatically:"
echo "    tests/linker/run_linker_tests.py --filter emit_relocs"
