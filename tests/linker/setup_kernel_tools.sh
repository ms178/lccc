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
#   tests/linker/setup_kernel_tools.sh [--kver v7.3] [--prefix DIR]
#
# The suite picks the tool up from $LCCC_RELOCS_TOOL, defaulting to
# ${LCCC_ORACLE_PREFIX:-$HOME/tools}/bin/relocs; without it the test reports
# SKIP, which `run_linker_tests.py --strict` (CI) turns into a failure.
#
# Every fetched file is pinned by SHA-256 (the v7.3 contents — Linux 7.3,
# the latest mainline release at the 2026-10-27 cache refresh; only
# relocs.c differs from the previous v6.12 pins), so a changed tag, a
# proxy or an HTML error page fails here instead of building something
# else.  Another --kver needs its own pins: --no-verify is for exploration
# only, never CI.
#
# The built tool is stamped ($SRC/.lccc-kver: kver + the pins it was
# built from).  A stamp mismatch — the exact state a pin bump leaves a
# warm prefix in — rebuilds instead of reusing the stale binary: the
# previous version of this script exited 0 on ANY existing
# $BIN/relocs, which would have kept a v6.12-built tool in service
# forever after this bump.
# ============================================================================
set -euo pipefail

PINNED_KVER=v7.3
KVER=${KVER:-$PINNED_KVER}
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

if [[ $VERIFY == 1 && $KVER != "$PINNED_KVER" ]]; then
  echo "error: no SHA-256 pins for $KVER (pins cover $PINNED_KVER; --no-verify to explore)" >&2
  exit 1
fi

# The v7.3 contents (cross-checked between the torvalds and stable
# kernel.org trees at the v7.3 tag, 2026-10-27).  One definition feeds
# both the `sha256sum -c` gate and the build stamp, so the two can
# never disagree about what "the pinned sources" means.
PINS='acbeca7b2fb6f84502c96385edf117f5170756d743373a7a581b5b46427803f7  relocs.c
61783eeddcf582c6f8c74c63f8f5170df51e34819a8b8dad9af68117f5c185d0  relocs.h
a49f30d39b362c29bdd27f19cadffdcfefefe5ad35511dd4b8f00d002a1cf08f  relocs_common.c
ae8b4ffcfe0367bb457a73b9a9534bf2ec495e2f7d99e23b1257b0009d693680  relocs_32.c
88e45459755585c934a9e24ebb9f6b71fd27f8d10c2fa9cc4a4d51fe3719166b  relocs_64.c
87b40e54fed5b27a9acd784e41521d8bbf95922448d07bea17be6b4bb5fa7925  tools/le_byteshift.h'
FILES=(relocs.c relocs.h relocs_common.c relocs_32.c relocs_64.c tools/le_byteshift.h)

BIN="$PREFIX/bin"
SRC="$PREFIX/kernel-tools"
STAMP="$SRC/.lccc-kver"
mkdir -p "$BIN" "$SRC/tools"

pins_digest=$(printf '%s\n' "$PINS" | sha256sum | cut -d' ' -f1)
if [[ $VERIFY == 1 ]]; then
  want_stamp="$KVER sha256:$pins_digest"
else
  want_stamp="$KVER no-verify"
fi

verify_sources() {  # 0 iff $SRC holds exactly the pinned files
  [[ $VERIFY == 1 ]] || return 0
  ( cd "$SRC" && printf '%s\n' "$PINS" | sha256sum -c --quiet - ) >/dev/null 2>&1
}

if [[ -x "$BIN/relocs" && -f "$STAMP" && $(cat "$STAMP") == "$want_stamp" ]] && verify_sources; then
  echo "relocs already built: $BIN/relocs ($KVER, stamp verified)"
  exit 0
fi
if [[ -x "$BIN/relocs" ]]; then
  echo "relocs present but stamp/sources do not prove $KVER; rebuilding"
fi

BASE="https://git.kernel.org/pub/scm/linux/kernel/git/torvalds/linux.git/plain"

fetch_sources() {
  echo "fetching arch/x86/tools/relocs sources at $KVER"
  local f
  for f in relocs.c relocs.h relocs_common.c relocs_32.c relocs_64.c; do
    curl -fsSL --retry 3 --retry-delay 2 -o "$SRC/$f" "$BASE/arch/x86/tools/$f?h=$KVER"
  done
  # relocs.h includes <tools/le_byteshift.h> from the kernel's tools/include.
  curl -fsSL --retry 3 --retry-delay 2 -o "$SRC/tools/le_byteshift.h" \
       "$BASE/tools/include/tools/le_byteshift.h?h=$KVER"
}

# Pre-placed sources are adopted instead of refetched (the same offline
# pattern ensure_gas_247.sh and ensure_arm_a64_xml.sh use): a sandbox with
# no route to git.kernel.org stages these files out of band (the
# harness-cache-2 tag carried them for v6.12), and the pin check below
# still rejects anything that is not exactly the pinned release.
have_all=1
for f in "${FILES[@]}"; do
  [[ -s "$SRC/$f" ]] || have_all=0
done
if [[ $have_all == 1 ]]; then
  echo "adopting pre-placed relocs sources in $SRC"
else
  fetch_sources
fi

# Sanity: a rate-limited or redirected fetch yields an HTML error page that
# compiles into a wall of nonsense. Catch it here with a clear message.
if ! grep -c 'SPDX-License-Identifier' <(sed -n '1p' "$SRC/relocs.c") >/dev/null; then
  echo "error: relocs.c does not look like kernel source (download blocked?)" >&2
  head -3 "$SRC/relocs.c" >&2
  exit 1
fi

if ! verify_sources; then
  # The pin-bump case: pre-placed v6.12 sources in a warm prefix.
  # Refetch the pinned release rather than failing forever on bytes
  # this script itself staged under the old pins.
  echo "pre-placed sources do not match the $KVER pins; refetching"
  fetch_sources
  verify_sources || { echo "error: kernel tool sources do not match the $KVER pins" >&2; exit 1; }
fi

echo "building relocs"
( cd "$SRC" && cc -O2 -I. -o relocs relocs_common.c relocs_32.c relocs_64.c )
install -m755 "$SRC/relocs" "$BIN/relocs"
printf '%s\n' "$want_stamp" > "$STAMP"

echo "relocs ready: $BIN/relocs ($KVER)"
echo
echo "The linker suite uses it automatically:"
echo "    tests/linker/run_linker_tests.py --filter emit_relocs"
