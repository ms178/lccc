#!/usr/bin/env bash
# ============================================================================
# offline_deb_extract.sh — extract offline .deb files into an ISOLATED sysroot.
#
# INCIDENT (2026-10-28): `dpkg -x pkg.deb /` let archive paths resolve through
# the usrmerge symlink /lib64 -> usr/lib64 and clobber the amd64 loader
# /lib64/ld-linux-x86-64.so.2, killing every exec and file write in the
# sandbox.  Extracting packages onto the system root is FORBIDDEN.
#
# This wrapper refuses any destination outside /home/user or /tmp, refuses to
# run as root, and prints the environment recipe for consuming the sysroot.
#
# Usage:
#   scripts/offline_deb_extract.sh <dest-dir> <pkg.deb> [more.deb ...]
#
# Offline multilib example (cached debs from the harness tag cache):
#   scripts/offline_deb_extract.sh /home/user/i386-sysroot \
#       ~/.cache/lccc/agent-toolchains/debs/libc6-dev-i386_*.deb \
#       ~/.cache/lccc/agent-toolchains/debs/libc6-i386_*.deb
#   export C_INCLUDE_PATH=/home/user/i386-sysroot/usr/include/i386-linux-gnu
#   export LIBRARY_PATH=/home/user/i386-sysroot/usr/lib32:/home/user/i386-sysroot/lib32
#   export LD_LIBRARY_PATH=/home/user/i386-sysroot/lib32
#   gcc -B/home/user/i386-sysroot/usr/lib32/ …   # only if crt startup files are needed
# ============================================================================
set -euo pipefail

dest=${1:?usage: offline_deb_extract.sh <dest-dir> <pkg.deb> ...}
shift
[[ $# -ge 1 ]] || { echo 'error: at least one .deb required' >&2; exit 2; }

# Hard destination boundary: workspace or temp only.
case "$(realpath -m "$dest")" in
  /home/user/*|/tmp/*) ;;
  *) echo "error: refusing destination outside /home/user or /tmp: $dest" >&2; exit 1 ;;
esac

# Never as root: prefix extraction does not need privileges, and root is how
# the previous incident reached system paths.
[[ "$(id -u)" -ne 0 ]] || { echo 'error: run unprivileged (no sudo)' >&2; exit 1; }

mkdir -p "$dest"
for deb in "$@"; do
  [[ -f "$deb" ]] || { echo "error: no such deb: $deb" >&2; exit 1; }
  dpkg -x "$deb" "$dest"     # into the PREFIX only — never /
done

echo "extracted into sysroot: $dest"
echo 'consume with (adjust subdirs after inspecting the prefix):'
echo "  export C_INCLUDE_PATH=\"$dest/usr/include/i386-linux-gnu\${C_INCLUDE_PATH:+:\$C_INCLUDE_PATH}\""
echo "  export LIBRARY_PATH=\"$dest/usr/lib32:$dest/lib32\${LIBRARY_PATH:+:\$LIBRARY_PATH}\""
echo "  export LD_LIBRARY_PATH=\"$dest/lib32:$dest/usr/lib32\${LD_LIBRARY_PATH:+:\$LD_LIBRARY_PATH}\""
echo '  # crt/startup files if a link fails: add -B'"$dest"'/usr/lib32/'
