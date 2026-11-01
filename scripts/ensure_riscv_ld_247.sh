#!/usr/bin/env bash
# ensure_riscv_ld_247.sh -- (re)provision GNU ld (bfd) 2.47 for
# riscv64-linux-gnu: the oracle the RISC-V linker-relaxation gate
# (tests/regression/check_riscv_relax_bfd.py) compares lccc-ld against.
#
# Why a locally built ld: the distro's riscv64-linux-gnu-ld is whatever
# binutils the runner image ships (Debian trixie: 2.44), and relaxation is
# exactly the area where bfd's output moves between releases.  The project
# pins binutils 2.47 (scripts/ensure_gas_247.sh); a relaxation claim made
# against another release is not a claim about the pin.
#
# The tarball is the one ensure_gas_247.sh pins: the SAME digest
# (GAS_TARBALL_SHA256, read from that script so there is one pin, not two),
# the SAME download cache (GAS_DL_DIR) and the SAME lock, so the two
# provisioners never race over the shared tarball or source tree.
#
# Usage: scripts/ensure_riscv_ld_247.sh [install-prefix]
#   Installs <prefix>/bin/ld (default prefix
#   ~/.cache/bfd-ld-2.47-riscv64-linux-gnu) and prints its version line.
#   A prefix is reused only when its provenance marker binds the installed
#   bytes to the current pin and the binary reports the pinned version and
#   the elf64lriscv emulation.
set -euo pipefail

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
pin=$(sed -n 's/^GAS_TARBALL_SHA256=\([0-9a-f]\{64\}\)$/\1/p' "$here/ensure_gas_247.sh")
[[ ${#pin} -eq 64 ]] || { echo "FATAL: no GAS_TARBALL_SHA256 pin in ensure_gas_247.sh" >&2; exit 1; }

ver=2.47
target=riscv64-linux-gnu
prefix=${1:-${HOME}/.cache/bfd-ld-${ver}-${target}}
ld="$prefix/bin/ld"
marker="$prefix/.lccc-bfd-ld-provenance"

sha() { local s; s=$(sha256sum "$1") && printf '%s\n' "${s%% *}"; }

# The version token is the last field of --version's first line, anchored
# (2.47 or a dated 2.47.YYYYMMDD snapshot stamp, which the release tarball's
# own build produces), and the binary must actually offer the emulation.
usable() {
    [[ -x "$ld" && -f "$marker" ]] || return 1
    local tok
    tok=$("$ld" --version 2>/dev/null | sed -n '1p' | awk '{print $NF}')
    [[ "$tok" =~ ^2\.47(\.[0-9]{8})?$ ]] || return 1
    "$ld" -V 2>/dev/null | grep -qx '   elf64lriscv' || return 1
    [[ "$(sed -n 's/^tarball_sha256=//p' "$marker")" == "$pin" ]] || return 1
    [[ "$(sed -n 's/^ld_sha256=//p' "$marker")" == "$(sha "$ld")" ]] || return 1
}

if usable; then
    "$ld" --version | sed -n '1p'
    exit 0
fi

dl_dir=${GAS_DL_DIR:-${HOME}/.cache/dl}
cache=${GAS_CACHE:-${HOME}/.cache}
tarball="$dl_dir/binutils-$ver.tar.xz"
mkdir -p "$dl_dir" "$cache"
if command -v flock >/dev/null 2>&1; then
    exec 9>"$dl_dir/.ensure_gas_247.lock"
    flock 9
fi
if usable; then
    "$ld" --version | sed -n '1p'
    exit 0
fi

if [[ ! -f "$tarball" ]]; then
    for base in \
        "https://ftp.gnu.org/gnu/binutils" \
        "https://mirrors.kernel.org/gnu/binutils" \
        "https://mirror.csclub.uwaterloo.ca/gnu/binutils"; do
        if curl --connect-timeout 10 --max-time 300 -sSLo "$tarball.part" \
            "$base/binutils-$ver.tar.xz"; then
            mv -f "$tarball.part" "$tarball"
            break
        fi
        rm -f "$tarball.part"
    done
fi
[[ -f "$tarball" ]] || { echo "FATAL: no mirror reachable for binutils-$ver" >&2; exit 1; }
if [[ "$(sha "$tarball")" != "$pin" ]]; then
    rm -f "$tarball"
    echo "FATAL: $tarball does not hash to the pinned $pin -- removed" >&2
    exit 1
fi

src="$cache/binutils-$ver"
if [[ ! -f "$src/configure" ]]; then
    rm -rf "$src" "$src.extracting"
    mkdir -p "$src.extracting"
    tar -xJf "$tarball" -C "$src.extracting"
    mv "$src.extracting/binutils-$ver" "$src"
    rm -rf "$src.extracting"
fi
build="$src-build-ld-${target//-/_}"
rm -rf "$build"
mkdir -p "$build"
cd "$build"
# Linux defaults (relro, hash style, separate code) come from the target
# triple exactly as for the distro's cross ld; nothing is overridden.
"$src/configure" --target="$target" --prefix="$prefix" \
    --disable-gdb --disable-gdbserver --disable-sim --disable-gprofng \
    --disable-nls --disable-werror --disable-gas --disable-gold \
    --disable-binutils >configure.log 2>&1
make -j"${LCCC_ENSURE_JOBS:-2}" all-ld >make.log 2>&1
mkdir -p "$prefix/bin"
install -m 0755 ld/ld-new "$ld.tmp"
mv -f "$ld.tmp" "$ld"
printf 'target=%s\ntarball_version=%s\ntarball_sha256=%s\nld_sha256=%s\n' \
    "$target" "$ver" "$pin" "$(sha "$ld")" >"$marker.tmp"
mv -f "$marker.tmp" "$marker"
cd /
rm -rf "$build"
usable || { echo "FATAL: the freshly built ld fails validation" >&2; exit 1; }
"$ld" --version | sed -n '1p'
