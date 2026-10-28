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
#
# CACHING
# -------
# `apt-get download` is the only network-bound step here, and it dominated the
# hosted pipeline: run 39752334266 paid 331 s in the assembler/linker lane,
# 165 s in contract shard b and 23 s in shard a for the identical 17-package
# set, in three jobs that all start at the same moment. LCCC_CROSS_DEB_DIR
# (default ~/.cache/lccc-cross-debs) keeps the downloaded archives, so a lane
# that restores the cache only pays the unpack. The package filenames are
# resolved through `apt-get download --print-uris`, which reports the exact
# archive apt would fetch *without* fetching it -- a cached file is therefore
# selected by the same key apt would use, not by a guessable name glob, and a
# package upgrade simply misses and re-downloads. Anything unexpected (no
# index, unparsable output) falls back to the unconditional download path, so
# the cache can only ever be an optimization, never a new failure mode.
# ============================================================================
set -uo pipefail

want=${1:-all}
work=${LCCC_XROOT:-/var/tmp/lccc-xroot}
deb_cache=${LCCC_CROSS_DEB_DIR:-$HOME/.cache/lccc-cross-debs}

have() { command -v "$1" >/dev/null 2>&1; }
runnable() {
    have "$1" && "$1" --version >/dev/null 2>&1
}

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
    && runnable aarch64-linux-gnu-gcc \
    && runnable aarch64-linux-gnu-as \
    && runnable aarch64-linux-gnu-ld \
    && [[ -e /usr/aarch64-linux-gnu/lib/ld-linux-aarch64.so.1 ]]; then
    need_aarch64=0
fi
if [[ $need_riscv64 == 1 ]] \
    && runnable riscv64-linux-gnu-gcc \
    && runnable riscv64-linux-gnu-as \
    && runnable riscv64-linux-gnu-ld \
    && [[ -e /usr/riscv64-linux-gnu/lib/ld-linux-riscv64-lp64d.so.1 ]]; then
    need_riscv64=0
fi
need_qemu=0
if [[ $check_aarch64 == 1 ]] && ! runnable qemu-aarch64; then
    need_qemu=1
fi
if [[ $check_riscv64 == 1 ]] && ! runnable qemu-riscv64; then
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

# The archive NAME apt would use for each package, resolved without a download.
# `--print-uris` prints "'<uri>' <filename> <size> SHA256:<digest>" per package.
# Taking the last field selects the digest, misses every cached archive and
# downloads the same package again into the work directory.
apt_opts=(
    -o Acquire::http::Timeout=15
    -o Acquire::https::Timeout=15
    -o Acquire::Retries=2
)
archive_name() {  # archive_name <package> -> basename, or empty
    apt-get "${apt_opts[@]}" download --print-uris "$1" 2>/dev/null \
        | awk 'NR == 1 { print $2 }' \
        | sed -e "s/^'//" -e "s/'$//" -e 's|.*/||'
}

echo "[cross] provisioning ${#pkgs[@]} packages into $work (cache: $deb_cache)"
missing=()
cached=0
fetched=0
[[ -n $deb_cache ]] && { mkdir -p "$deb_cache" 2>/dev/null || deb_cache=""; }
to_fetch=()
declare -A pkg_archive=()
for p in "${pkgs[@]}"; do
    name=$(archive_name "$p")
    pkg_archive["$p"]=$name
    if [[ -n $deb_cache && -n $name && -s $deb_cache/$name ]]; then
        cached=$((cached + 1))
    else
        to_fetch+=("$p")
    fi
done
if ((${#to_fetch[@]} > 0)); then
    dl_dir=${deb_cache:-$work}
    if ! ( cd "$dl_dir" && timeout 180 apt-get "${apt_opts[@]}" download "${to_fetch[@]}" >/dev/null 2>&1 ); then
        for p in "${to_fetch[@]}"; do
            name=${pkg_archive["$p"]:-}
            [[ -n $name && -s $dl_dir/$name ]] && continue
            ( cd "$dl_dir" && timeout 45 apt-get "${apt_opts[@]}" download "$p" >/dev/null 2>&1 ) || true
        done
    fi
fi
for p in "${pkgs[@]}"; do
    name=${pkg_archive["$p"]:-}
    if [[ -n $deb_cache && -n $name && -s $deb_cache/$name ]]; then
        if [[ " ${to_fetch[*]} " == *" $p "* ]]; then
            fetched=$((fetched + 1))
        fi
        ln "$deb_cache/$name" "$work/$name" 2>/dev/null \
            || cp "$deb_cache/$name" "$work/$name" 2>/dev/null \
            || missing+=("$p")
    elif [[ -z $deb_cache && -n $name && -s $work/$name ]]; then
        fetched=$((fetched + 1))
    else
        timeout 45 apt-get "${apt_opts[@]}" download "$p" >/dev/null 2>&1 || missing+=("$p")
    fi
done
echo "[cross] archives: $cached reused, $fetched fetched"
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

# `dpkg -x` deliberately bypasses package maintainer scripts. Cross binutils
# executables can therefore be present while their host-side shared libraries
# (for example libopcodes-*-arm64.so) are absent from the dynamic-loader cache.
# Rebuild it after grafting /usr; otherwise an `as --version` probe succeeds
# only on machines where every library happens to be in a built-in search dir.
loader_cache_failed=0
# Minimal root/agent environments often omit /sbin and /usr/sbin from PATH.
# Discovery must not report a missing tool when the standard absolute binary
# is present and sudo can execute it (the earlier PATH-only check did exactly
# that after successfully unpacking every cross package).
# The search roots are overridable so a test can exercise this discovery
# without touching /sbin; production always uses the standard two.
CROSS_SBIN_DIRS=("${CROSS_SBIN_DIR:-/sbin}" "${CROSS_USR_SBIN_DIR:-/usr/sbin}")

find_ldconfig() {  # -> absolute path on stdout, empty when the host has none
    local found dir
    found=$(command -v ldconfig 2>/dev/null || true)
    if [[ -z $found ]]; then
        for dir in "${CROSS_SBIN_DIRS[@]}"; do
            if [[ -x $dir/ldconfig ]]; then
                found=$dir/ldconfig
                break
            fi
        done
    fi
    printf '%s' "$found"
}

ldconfig_bin=$(find_ldconfig)
if [[ -n $ldconfig_bin ]]; then
    if [[ ${EUID:-$(id -u)} -eq 0 ]]; then
        "$ldconfig_bin" || loader_cache_failed=1
    elif have sudo; then
        sudo -n "$ldconfig_bin" || loader_cache_failed=1
    else
        echo "[cross] ERROR: ldconfig needs root after dpkg -x cross-toolchain restore" >&2
        loader_cache_failed=1
    fi
else
    echo "[cross] ERROR: ldconfig is required after dpkg -x cross-toolchain restore" >&2
    loader_cache_failed=1
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
# path must exist. Verify exactly the targets requested by this invocation,
# including that the cross binutils can actually load their shared libraries.
rc=$loader_cache_failed
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
    if runnable "$cc" && runnable "$qemu" && [[ -e $loader ]]; then
        echo "[cross] OK  $cc ($($cc --version | head -1)); $qemu; $loader"
    else
        runnable "$cc" || echo "[cross] MISSING or not runnable $cc"
        runnable "$qemu" || echo "[cross] MISSING or not runnable $qemu"
        [[ -e $loader ]] || echo "[cross] MISSING $loader"
        rc=1
    fi
    for tool in as ld; do
        host_tool="${t}-linux-gnu-${tool}"
        if runnable "$host_tool"; then
            echo "[cross] OK  $host_tool"
        else
            echo "[cross] MISSING or not runnable $host_tool" >&2
            rc=1
        fi
    done
done
exit $rc
