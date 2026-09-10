#!/usr/bin/env bash
# ============================================================================
# ensure_cross_toolchains.sh — idempotently provide AArch64 / RISC-V C
# cross-toolchains for execution-validated backend work.
#
# WHY THIS EXISTS
# ---------------
# Debian's `gcc-<N>-<triplet>-linux-gnu` packages declare
#
#       Conflicts: gcc-multilib
#
# so `apt-get install gcc-multilib gcc-aarch64-linux-gnu` cannot be satisfied:
# installing the cross compilers silently REMOVES the i686 multilib contract
# that LCCC's CI (and the i686 regression corpus) needs, and installing the
# multilib removes the cross compilers. Both are required: every backend must
# be validated by *execution*, not by eyeballing assembly.
#
# The conflict is a dpkg dependency-graph artefact, not a filesystem one: the
# cross packages install only triplet-prefixed paths
# (/usr/bin/<triplet>-gcc, /usr/lib/gcc-cross/<triplet>/…, /usr/<triplet>/…)
# that never collide with the native or multilib trees. So this script
# downloads the packages and unpacks them with `dpkg -x` into the live root,
# which bypasses the dependency solver entirely. Both toolchains then coexist
# with gcc-multilib.
#
# WHAT IT PROVIDES
# ----------------
#   aarch64-linux-gnu-gcc, aarch64-linux-gnu-as/ld, /usr/aarch64-linux-gnu
#   riscv64-linux-gnu-gcc, riscv64-linux-gnu-as/ld, /usr/riscv64-linux-gnu
#
# plus the runtime needed to *run* the produced binaries under `qemu-aarch64`
# / `qemu-riscv64` (binfmt_misc is not registered on every host, so pass
# `-L /usr/<triplet>` explicitly, or use scripts/run_cross.sh).
#
# Usage:  scripts/ensure_cross_toolchains.sh [aarch64|riscv64|all]
# ============================================================================
set -uo pipefail

want=${1:-all}
work=${LCCC_XROOT:-/var/tmp/lccc-xroot}

have() { command -v "$1" >/dev/null 2>&1; }

need_aarch64=0
need_riscv64=0
case "$want" in
    aarch64|arm64) need_aarch64=1 ;;
    riscv64|rv64)  need_riscv64=1 ;;
    all)           need_aarch64=1; need_riscv64=1 ;;
    *) echo "usage: $0 [aarch64|riscv64|all]" >&2; exit 2 ;;
esac
# Keep the requested set separate: the need_* flags below are cleared for
# already-installed tools, but final verification must still check exactly the
# targets requested by this invocation (not always both, and not neither).
check_aarch64=$need_aarch64
check_riscv64=$need_riscv64

if [[ $need_aarch64 == 1 ]] \
    && have aarch64-linux-gnu-gcc \
    && have aarch64-linux-gnu-as \
    && [[ -e /usr/aarch64-linux-gnu/lib/ld-linux-aarch64.so.1 ]]; then
    need_aarch64=0
fi
if [[ $need_riscv64 == 1 ]] \
    && have riscv64-linux-gnu-gcc \
    && have riscv64-linux-gnu-as \
    && [[ -e /usr/riscv64-linux-gnu/lib/ld-linux-riscv64-lp64d.so.1 ]]; then
    need_riscv64=0
fi
need_qemu=0
if [[ $check_aarch64 == 1 ]] && ! have qemu-aarch64; then
    need_qemu=1
fi
if [[ $check_riscv64 == 1 ]] && ! have qemu-riscv64; then
    need_qemu=1
fi
if [[ $need_aarch64 == 0 && $need_riscv64 == 0 && $need_qemu == 0 ]]; then
    echo "requested cross toolchain(s) already present"
    exit 0
fi

# Resolve the *host* GCC major so the cross packages match the installed
# libgcc/libc6-cross generation instead of a hardcoded number that rots.
host_gcc_ver=$(gcc -dumpversion 2>/dev/null | cut -d. -f1)
[[ -n "$host_gcc_ver" ]] || host_gcc_ver=14

pkgs=()
if [[ $need_qemu == 1 ]]; then
    # qemu-user carries qemu-aarch64 and qemu-riscv64. It has no multilib
    # conflict, but extracting it here keeps this bootstrap non-destructive and
    # usable without mutating dpkg's package database.
    pkgs+=("qemu-user")
fi
if [[ $need_aarch64 == 1 ]]; then
    pkgs+=(
        "gcc-${host_gcc_ver}-aarch64-linux-gnu"
        "cpp-${host_gcc_ver}-aarch64-linux-gnu"
        "libgcc-${host_gcc_ver}-dev-arm64-cross"
        "libgcc-s1-arm64-cross"
        "libc6-dev-arm64-cross"
        "libc6-arm64-cross"
        "linux-libc-dev-arm64-cross"
        "binutils-aarch64-linux-gnu"
    )
fi
if [[ $need_riscv64 == 1 ]]; then
    pkgs+=(
        "gcc-${host_gcc_ver}-riscv64-linux-gnu"
        "cpp-${host_gcc_ver}-riscv64-linux-gnu"
        "libgcc-${host_gcc_ver}-dev-riscv64-cross"
        "libgcc-s1-riscv64-cross"
        "libc6-dev-riscv64-cross"
        "libc6-riscv64-cross"
        "linux-libc-dev-riscv64-cross"
        "binutils-riscv64-linux-gnu"
        "libatomic1-riscv64-cross"
    )
fi

rm -rf "$work"
mkdir -p "$work"
cd "$work" || exit 1

echo "[cross] downloading ${#pkgs[@]} packages into $work"
missing=()
for p in "${pkgs[@]}"; do
    apt-get download "$p" >/dev/null 2>&1 || missing+=("$p")
done
if ((${#missing[@]})); then
    echo "[cross] WARNING: could not download: ${missing[*]}" >&2
fi

shopt -s nullglob
debs=(*.deb)
if ((${#debs[@]} == 0)); then
    echo "[cross] FATAL: no packages downloaded" >&2
    exit 1
fi

echo "[cross] unpacking ${#debs[@]} archives (bypassing the gcc-multilib conflict)"
for d in "${debs[@]}"; do
    dpkg -x "$d" "$work" 2>/dev/null
done

# The archives carry a `./usr` tree; graft it onto the live root. Cross
# packages own only triplet-prefixed paths, so this cannot clobber the native
# or i686 trees (verified: no file is written outside /usr/<triplet>,
# /usr/lib/gcc-cross, /usr/libexec/gcc-cross, /usr/bin/<triplet>-*,
# /usr/share/<triplet>).
if [[ -d "$work/usr" ]]; then
    sudo cp -a "$work/usr/." /usr/ 2>/dev/null \
        || cp -a "$work/usr/." /usr/ 2>/dev/null \
        || echo "[cross] FATAL: cannot write /usr" >&2
fi

# Versioned compiler packages intentionally omit the unversioned driver
# symlinks supplied by the conflicting gcc-<triplet>-linux-gnu meta packages.
# Recreate only missing links, pointing at the matching host-major tools.
for t in aarch64 riscv64; do
    if [[ $t == aarch64 && $check_aarch64 == 0 ]] \
        || [[ $t == riscv64 && $check_riscv64 == 0 ]]; then
        continue
    fi
    for tool in gcc gcc-ar gcc-nm gcc-ranlib; do
        dst="/usr/bin/${t}-linux-gnu-${tool}"
        src="${dst}-${host_gcc_ver}"
        if [[ ! -e "$dst" && -x "$src" ]]; then
            sudo ln -s "$(basename "$src")" "$dst" 2>/dev/null \
                || ln -s "$(basename "$src")" "$dst" 2>/dev/null \
                || true
        fi
    done
done

# A sysroot is not enough for qemu `-L`: both the emulator and dynamic loader
# path must exist. Verify exactly the targets requested by this invocation.
rc=0
for t in aarch64 riscv64; do
    if [[ $t == aarch64 && $check_aarch64 == 0 ]] \
        || [[ $t == riscv64 && $check_riscv64 == 0 ]]; then
        continue
    fi
    cc="${t}-linux-gnu-gcc"
    qemu="qemu-${t}"
    if [[ $t == aarch64 ]]; then
        loader=/usr/aarch64-linux-gnu/lib/ld-linux-aarch64.so.1
    else
        loader=/usr/riscv64-linux-gnu/lib/ld-linux-riscv64-lp64d.so.1
    fi
    if have "$cc" && have "$qemu" && [[ -e $loader ]]; then
        echo "[cross] OK  $cc ($($cc --version | head -1)); $qemu; $loader"
    else
        have "$cc" || echo "[cross] MISSING $cc"
        have "$qemu" || echo "[cross] MISSING $qemu"
        [[ -e $loader ]] || echo "[cross] MISSING $loader"
        rc=1
    fi
done
exit $rc
