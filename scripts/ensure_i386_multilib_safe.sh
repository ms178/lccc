#!/usr/bin/env bash
# Safely verify or provision 32-bit i386/i686 (-m32) multilib support on an
# x86_64 Debian/Ubuntu container WITHOUT ever mutating live amd64 glibc or
# breaking usr-merge root symlinks.
#
# Why this script exists (three harness-fatal -m32 libc traps):
#   1. Extracting Bookworm glibc 2.36 .debs (e.g. debs-bookworm from
#      harness-cache-*) onto a Debian 13 Trixie (glibc 2.41) usr-merged `/`
#      with `dpkg -x ... /` replaces the `/lib -> usr/lib` symlink with a
#      plain directory or mismatches libc.so.6 vs ld-linux, severing execve()
#      for bash, sudo, and the Arena harness daemon instantly.
#   2. Running `apt-get install gcc-multilib libc6-dev-i386` WITHOUT
#      `--no-upgrade` upgrades live `libc6:amd64` in-place whenever mirror
#      security updates lead the container base image (`libc6-dev-i386` ->
#      `libc6-i386 (= V)` -> `libc6 (= V)`), replacing `/lib/x86_64-linux-gnu`
#      shared objects underneath long-lived container supervisor processes.
#      Likewise `dpkg --add-architecture i386` + `linux-libc-dev:i386` fails
#      or churns multiarch state on Debian Trixie where `linux-libc-dev` is
#      `Architecture: all` and biarch `libc6-dev-i386:amd64` needs no foreign
#      dpkg architecture.
#   3. Unpacking fallback sysroots under `/home/user` blows Arena's 128 MiB /
#      10,000-file workspace snapshot cap.
#
# Modes:
#   ensure_i386_multilib_safe.sh                  Verify host -m32; if missing,
#                                                 run safe --no-upgrade apt or
#                                                 stage isolated sysroot under
#                                                 /var/tmp/lccc-i686-sysroot
#                                                 from verified local .debs
#                                                 (offline) or apt download,
#                                                 then emit m32-cc/m32-c++
#                                                 wrappers plus env.sh.
#   ensure_i386_multilib_safe.sh --offline-debs DIR
#                                                 Stage the sysroot from DIR's
#                                                 verified biarch .debs only;
#                                                 never touches the network or
#                                                 the live root.
#   ensure_i386_multilib_safe.sh --print-env      Print a sourceable environment
#                                                 recipe for an already staged
#                                                 sysroot (LCCC_M32_CC/CXX).
#   ensure_i386_multilib_safe.sh --check-only     Compile+run 32-bit C/C++ probe
#                                                 only; never invoke apt/dpkg.
#   ensure_i386_multilib_safe.sh --audit-apt-sim FILE
#                                                 Exit 0 iff apt -s output FILE
#                                                 upgrades/removes no protected
#                                                 host libc/toolchain package.
#   ensure_i386_multilib_safe.sh --validate-sysroot-path DIR
#                                                 Exit 0 iff DIR is safe for an
#                                                 isolated rootless unpack.
#   ensure_i386_multilib_safe.sh --self-test      Run offline safety unit checks.

set -euo pipefail

# Debian sbin tools (`dpkg`, `ldconfig`) are omitted from unprivileged default
# PATHs in some container images.
export PATH="/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin:${PATH:-}"

DEFAULT_SYSROOT="${LCCC_I686_SYSROOT:-/var/tmp/lccc-i686-sysroot}"
SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
# One implementation of the privilege-dropping guard, shared with every other
# provisioning entry point.  Sourcing does not run its main().
# shellcheck source=scripts/rootfs_jail.sh
source "$SCRIPT_DIR/rootfs_jail.sh"

log() {
    printf '[i386-multilib] %s\n' "$*"
}

die() {
    printf '[i386-multilib] ERROR: %s\n' "$*" >&2
    exit 1
}

# Verify usr-merge root symlinks (`/lib`, `/lib64`) have not been replaced by
# plain directories.  Called before and after any package/sysroot action.
assert_usrmerge_intact() {
    local root_dir=${1:-/}
    if [[ -e "$root_dir/usr/lib/x86_64-linux-gnu/libc.so.6" ]]; then
        if [[ -e "$root_dir/lib" && ! -L "$root_dir/lib" && ! -e "$root_dir/lib/x86_64-linux-gnu/libc.so.6" ]]; then
            die "broken usr-merge layout under $root_dir: /lib is not a symlink and lacks x86_64 libc.so.6"
        fi
    fi
    if [[ -e "$root_dir/usr/lib64/ld-linux-x86-64.so.2" ]]; then
        if [[ -e "$root_dir/lib64" && ! -L "$root_dir/lib64" && ! -e "$root_dir/lib64/ld-linux-x86-64.so.2" ]]; then
            die "broken usr-merge layout under $root_dir: /lib64 is not a symlink and lacks ld-linux-x86-64.so.2"
        fi
    fi
}

# Reject any target directory that could clobber the live OS root or bloat the
# persisted /home/user workspace snapshot.
validate_sysroot_path() {
    local raw=${1:?sysroot path required}
    local canon
    canon=$(python3 - "$raw" <<'PY'
import os, sys
p = sys.argv[1]
if not p.startswith('/'):
    raise SystemExit(2)
print(os.path.realpath(p))
PY
    ) || die "sysroot path must be an absolute path (got: $raw)"

    case "$canon" in
        /|/bin|/sbin|/etc|/lib|/lib32|/lib64|/libx32|/usr|/usr/*|/bin/*|/sbin/*|/lib/*|/lib32/*|/lib64/*|/etc/*)
            die "refusing unsafe sysroot path '$canon' (points into live OS root)"
            ;;
        /dev|/dev/*|/proc|/proc/*|/sys|/sys/*|/boot|/boot/*|/run|/run/*)
            die "refusing sysroot path '$canon' (kernel, device or boot tree)"
            ;;
        /home/user|/home/user/*)
            die "refusing sysroot path '$canon' inside /home/user (exceeds Arena workspace snapshot quota; use /var/tmp/lccc-i686-sysroot)"
            ;;
    esac

    # A denylist alone is not enough: staging *replaces* this directory, so a
    # shared tree that nobody thought to deny (`/tmp`, `/var`, `/opt`, `/srv`)
    # would be deleted wholesale.  Require a dedicated directory at least two
    # components deep, which every legitimate sysroot path satisfies
    # (`/var/tmp/lccc-i686-sysroot`, `$RUNNER_TEMP/i686-sysroot`).
    local depth=${canon#/}
    depth=${depth//[^\/]/}
    if [[ ${#depth} -lt 1 ]]; then
        die "refusing sysroot path '$canon': a direct child of / is a shared tree, not a dedicated sysroot"
    fi
}

# Staging replaces the sysroot, so it may only ever replace something that is
# provably ours: absent, empty, or carrying the marker this script writes.  A
# populated directory without the marker is somebody else's data.
assert_replaceable_sysroot() {
    local sysroot=$1
    validate_sysroot_path "$sysroot"
    [[ -e $sysroot ]] || return 0
    [[ -d $sysroot && ! -L $sysroot ]] || die "sysroot exists and is not a real directory: $sysroot"
    if [[ -f $sysroot/README.DO_NOT_COPY_TO_ROOT.txt ]]; then
        return 0
    fi
    if [[ -z $(ls -A "$sysroot" 2>/dev/null) ]]; then
        return 0
    fi
    die "refusing to replace populated directory '$sysroot' that this script did not create (no README.DO_NOT_COPY_TO_ROOT.txt marker)"
}

# Parse `apt-get -s` output and reject ANY upgrade or removal of live host
# libc / dynamic loader / core runtime packages.
audit_apt_simulation() {
    local sim_file=${1:?simulation output file required}
    [[ -r "$sim_file" ]] || die "cannot read apt simulation output: $sim_file"
    python3 - "$sim_file" <<'PY'
import re, sys

text = open(sys.argv[1], encoding="utf-8", errors="replace").read()
protected_exact = {
    "libc6",
    "libc6:amd64",
    "libc-bin",
    "libc-bin:amd64",
    "libc-dev-bin",
    "libc-dev-bin:amd64",
    "libc6-dev",
    "libc6-dev:amd64",
    "libgcc-s1",
    "libgcc-s1:amd64",
    "libstdc++6",
    "libstdc++6:amd64",
    "base-files",
    "usr-is-merged",
}

violations = []
for raw in text.splitlines():
    line = raw.strip()
    m_remv = re.match(r"^Remv\s+(\S+)", line)
    if m_remv:
        pkg = m_remv.group(1)
        violations.append(f"apt plans package removal: {line}")
        continue
    # Inst <pkg> [<old_ver>] (<new_ver> ...) indicates an in-place upgrade of an
    # already-installed package; fresh installs omit the `[old_ver]` bracket.
    m_upg = re.match(r"^Inst\s+(\S+)\s+\[([^\]]+)\]\s+\(([^ )]+)", line)
    if m_upg:
        pkg, old_v, new_v = m_upg.group(1), m_upg.group(2), m_upg.group(3)
        base = pkg.split(":")[0]
        if pkg in protected_exact or base in protected_exact or pkg.endswith(":i386"):
            violations.append(
                f"apt plans live upgrade of protected package {pkg} ({old_v} -> {new_v})"
            )
        continue
    m_inst = re.match(r"^Inst\s+(\S+)\s+\(", line)
    if m_inst:
        pkg = m_inst.group(1)
        if pkg.endswith(":i386"):
            violations.append(
                f"apt plans foreign :i386 multiarch package {pkg}; use amd64 biarch (*-i386) only"
            )

if violations:
    for v in violations:
        sys.stderr.write(f"[i386-multilib] UNSAFE APT PLAN: {v}\n")
    raise SystemExit(1)
PY
}

# The one canonical flag recipe for compiling against an isolated biarch
# sysroot.  Every consumer (the health probe and the generated wrappers) reads
# it from here, so the recipe cannot drift between "what we verified" and "what
# a gate actually uses".  Each flag exists for a measured reason:
#
#   -I<sysroot>/usr/include        Bookworm's libc6-dev-i386 places `gnu/stubs-32.h`
#                                  and `gnu/lib-names-32.h` behind a `gnu ->`
#                                  symlink; it must be searched *before* the host
#                                  headers or -m32 picks the 64-bit stubs.
#   -idirafter <host multiarch>    The biarch package deliberately does NOT ship
#                                  the arch-independent `bits/*`; those come from
#                                  the live amd64 libc6-dev, which is correct for
#                                  -m32 because bits/wordsize.h keys off __x86_64__.
#   -B / -L <sysroot libdirs>      crt1.o, libgcc and libc.so come from the sysroot.
#   -Wl,--sysroot=<sysroot>        libc.so is a *linker script* with absolute
#                                  /lib32 and /usr/lib32 members; without this the
#                                  linker resolves them against the live root.
#   -Wl,--dynamic-linker=<loader>  PT_INTERP points at the staged 32-bit loader, so
#                                  the ELF never needs a host /lib/ld-linux.so.2.
#   -Wl,-rpath,<libdirs>           The staged loader resolves libc/libgcc at run
#                                  time with no LD_LIBRARY_PATH, so `./probe`
#                                  executes directly and execute coverage is kept.
m32_sysroot_flags() {
    local sysroot=$1 cc=${2:-gcc}
    local interp="" candidate
    for candidate in "$sysroot/lib32/ld-linux.so.2" "$sysroot/lib/ld-linux.so.2"; do
        if [[ -e $candidate ]]; then interp=$candidate; break; fi
    done
    [[ -n $interp ]] || return 1
    local multiarch
    multiarch=$("$cc" -print-multiarch 2>/dev/null || true)
    [[ -n $multiarch ]] || multiarch=x86_64-linux-gnu
    printf '%s\n' "-m32" "-I$sysroot/usr/include"
    if [[ -d /usr/include/$multiarch ]]; then
        printf '%s\n' "-idirafter" "/usr/include/$multiarch"
    fi
    printf '%s\n' "-B$sysroot/usr/lib32" "-B$sysroot/lib32" \
        "-L$sysroot/usr/lib32" "-L$sysroot/lib32" \
        "-Wl,--sysroot=$sysroot" "-Wl,--dynamic-linker=$interp" \
        "-Wl,-rpath,$sysroot/lib32:$sysroot/usr/lib32"
}

# Compile, link, and execute a real 32-bit binary against the host multilib
# (no sysroot) or an isolated sysroot.  Execution is part of the contract: a
# link-only check would have hidden both the PT_INTERP and the linker-script
# defects that this recipe exists to solve.  $2 selects the language so C and
# C++ coverage are proven independently.
probe_m32() {
    local sysroot=${1:-} lang=${2:-c}
    local cc_bin=${CC_FOR_M32_PROBE:-gcc}
    local source_name=probe.c
    if [[ $lang == c++ ]]; then
        cc_bin=${CXX_FOR_M32_PROBE:-g++}
        source_name=probe.cpp
    fi
    command -v "$cc_bin" >/dev/null 2>&1 || return 1

    local tmp
    tmp=$(mktemp -d "${TMPDIR:-/var/tmp}/lccc-m32-probe.XXXXXX")
    if [[ $lang == c++ ]]; then
        cat >"$tmp/$source_name" <<'EOF'
#include <string>
#include <vector>
int main() {
    std::vector<std::string> v{"m32ok"};
    return (sizeof(void *) == 4 && v[0][0] == 'm') ? 0 : 1;
}
EOF
    else
        cat >"$tmp/$source_name" <<'EOF'
#include <linux/types.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>
int main(void) {
    __u32 k = 42u;
    char buf[8];
    memcpy(buf, "m32ok", 6);
    return (sizeof(void *) == 4 && k == 42u && buf[0] == 'm') ? 0 : 1;
}
EOF
    fi

    local -a flags=(-m32)
    if [[ -n $sysroot ]]; then
        flags=()
        local line
        while IFS= read -r line; do flags+=("$line"); done \
            < <(m32_sysroot_flags "$sysroot" "$cc_bin")
        if [[ ${#flags[@]} -eq 0 ]]; then
            rm -rf "$tmp"
            return 1
        fi
    fi

    local ok=0
    # Deliberately no LD_LIBRARY_PATH: the rpath recipe must stand on its own,
    # because gates execute the produced binaries in their own environment.
    if "$cc_bin" "${flags[@]}" "$tmp/$source_name" -o "$tmp/probe" >/dev/null 2>&1 \
        && "$tmp/probe" >/dev/null 2>&1; then
        ok=1
    fi
    rm -rf "$tmp"
    [[ $ok -eq 1 ]]
}

# Verify a staged sysroot end to end.  C is mandatory; C++ is mandatory whenever
# a C++ compiler is present, so "the C probe passed" can never mask a broken
# 32-bit libstdc++.
verify_staged_sysroot() {
    local sysroot=$1
    probe_m32 "$sysroot" c || return 1
    if command -v "${CXX_FOR_M32_PROBE:-g++}" >/dev/null 2>&1; then
        probe_m32 "$sysroot" c++ || return 1
    fi
    return 0
}

# Biarch packages only.  The agent cache also ships amd64 payloads (valgrind,
# libc6-dbg, python3-yaml); dragging an amd64 glibc into a 32-bit sysroot would
# shadow host headers for no benefit, so selection is by control-field Package
# and never by filename glob alone.
BIARCH_PACKAGE_RE='^(libc6-i386|libc6-dev-i386|lib32gcc-[0-9]+-dev|lib32gcc-s1|lib32stdc\+\+-[0-9]+-dev|lib32stdc\+\+6|lib32atomic1|lib32gomp1|lib32itm1|lib32quadmath0|lib32ubsan1|lib32asan8)$'

# Where a restored agent toolchain cache keeps its verified .debs.  The restore
# tool checks every payload against the tag's MANIFEST.sha256 before it lands
# here, so this directory is already content-addressed trust.
default_deb_dirs() {
    local prefix=${LCCC_AGENT_CACHE_PREFIX:-${HOME:-/home/user}/.cache/lccc/agent-toolchains}
    printf '%s\n' "$prefix/debs"
    printf '%s\n' "${HOME:-/home/user}/.cache/lccc-toolchains/debs"
}

# Extract one .deb into an already-validated isolated sysroot.  The destination
# is passed as a single argv element and re-validated here, so no caller can
# smuggle `/` in through a variable.  When the kernel supports unprivileged user
# namespaces the extraction additionally runs inside the privilege-dropping
# jail: even a bug in this function could not then write to the live root.
extract_deb_to_sysroot() {
    local deb=$1 sysroot=$2
    validate_sysroot_path "$sysroot"
    [[ -f $deb ]] || die "not a file: $deb"
    [[ -d $sysroot ]] || die "sysroot does not exist: $sysroot"
    if jail_available; then
        jail_run -- dpkg-deb -x "$deb" "$sysroot"
    else
        log "jail unavailable; extracting to the path-validated isolated sysroot only: $sysroot"
        dpkg-deb -x "$deb" "$sysroot"
    fi
}

# Write the wrapper toolchain that makes the isolated sysroot usable by any
# gate without a single write outside it.  Two details carry the coverage:
#
#   * -Wl,--dynamic-linker=<sysroot>/lib32/ld-linux.so.2 pins PT_INTERP to the
#     staged 32-bit loader, so the produced ELF does not depend on a host
#     /lib/ld-linux.so.2 that intentionally does not exist;
#   * -Wl,-rpath,<sysroot libdirs> lets that loader resolve libc/libgcc at run
#     time, so `./probe` runs directly -- no LD_LIBRARY_PATH wrapper, no
#     skipped execute legs, no lost runtime coverage.
write_m32_wrappers() {
    local sysroot=$1 cc=${2:-${CC:-gcc}} cxx=${3:-${CXX:-g++}}
    validate_sysroot_path "$sysroot"
    mkdir -p "$sysroot/bin"
    local name real line resolved
    for name in m32-cc m32-c++; do
        real=$cc
        [[ $name == m32-c++ ]] && real=$cxx
        # Embed the *resolved absolute* driver path.  A wrapper that says
        # `exec gcc-16` only works while the toolchain's bin directory is on
        # PATH -- and the whole point of this wrapper is that a gate can run it
        # from its own environment.  Measured failure before this fix:
        # `m32-cc: line 4: exec: gcc-16: not found`.
        # `type -P` searches PATH for an executable file only.  `command -v`
        # can answer with a shell builtin, function or alias (e.g. `true`),
        # which would be turned into a bogus "$PWD/true" driver path.
        resolved=$(type -P "$real" 2>/dev/null || true)
        [[ -n $resolved ]] || die "compiler not found on PATH: $real"
        case $resolved in /*) ;; *) resolved=$(pwd)/$resolved ;; esac
        real=$resolved
        local -a flags=()
        while IFS= read -r line; do flags+=("$line"); done < <(m32_sysroot_flags "$sysroot" "$real")
        [[ ${#flags[@]} -gt 0 ]] || die "staged sysroot has no 32-bit loader: $sysroot"
        {
            printf '#!/usr/bin/env bash\n'
            printf '# Generated by scripts/ensure_i386_multilib_safe.sh -- do not edit.\n'
            printf '# Isolated 32-bit sysroot toolchain: no root, no live-root writes.\n'
            printf 'exec %q' "$real"
            printf ' %q' "${flags[@]}"
            printf ' "$@"\n'
        } >"$sysroot/bin/$name"
        chmod +x "$sysroot/bin/$name"
    done
    cat >"$sysroot/env.sh" <<EOF
# Source this to give any gate full 32-bit compile/link/run coverage.
export LCCC_I686_SYSROOT="$sysroot"
export LCCC_M32_CC="$sysroot/bin/m32-cc"
export LCCC_M32_CXX="$sysroot/bin/m32-c++"
export PATH="$sysroot/bin:\$PATH"
EOF
}

# Debian biarch .debs contain *absolute* symlinks (e.g. /lib/ld-linux.so.2 ->
# /lib32/ld-linux.so.2 and usr/lib32/libm.so -> /lib32/libm.so.6).  Inside a
# staged sysroot those targets must be re-rooted, or the linker and ld.so
# resolve them against the LIVE root -- which is both a correctness bug and
# exactly the kind of live-root dependency this script exists to avoid.
# Measured: 9 such links in the Bookworm biarch set; without this fixup the
# link fails with "cannot find /lib/ld-linux.so.2 inside <sysroot>".
reroot_absolute_symlinks() {
    local sysroot=$1
    validate_sysroot_path "$sysroot"
    local link target candidate rel fixed=0
    while IFS= read -r -d '' link; do
        target=$(readlink "$link")
        [[ $target == /* ]] || continue
        candidate="$sysroot${target}"
        [[ -e $candidate ]] || continue
        rel=$(realpath --relative-to="$(dirname "$link")" "$candidate")
        ln -sfn "$rel" "$link"
        fixed=$((fixed + 1))
    done < <(find "$sysroot" -type l -print0)
    log "re-rooted $fixed absolute intra-sysroot symlinks"
}

stage_isolated_sysroot() {
    local sysroot=$1 debs_dir=${2:-}
    validate_sysroot_path "$sysroot"
    assert_usrmerge_intact /

    command -v dpkg-deb >/dev/null 2>&1 || die "dpkg-deb not available to unpack isolated multilib packages"

    local -a debs=()
    local -a search=()
    if [[ -n $debs_dir ]]; then
        search+=("$debs_dir")
    else
        while IFS= read -r dir; do search+=("$dir"); done < <(default_deb_dirs)
    fi

    local dir deb package
    for dir in "${search[@]}"; do
        [[ -d $dir ]] || continue
        for deb in "$dir"/*.deb; do
            [[ -f $deb ]] || continue
            package=$(dpkg-deb -f "$deb" Package 2>/dev/null || true)
            [[ $package =~ $BIARCH_PACKAGE_RE ]] || continue
            debs+=("$deb")
        done
        [[ ${#debs[@]} -eq 0 ]] || break
    done

    local dl_dir=""
    if [[ ${#debs[@]} -eq 0 ]]; then
        # No verified local payload: fall back to the network, but only into a
        # private staging directory.  Arena's allowlist blocks Debian mirrors,
        # so this is expected to fail there -- and the diagnostic names the
        # exact offline remedy instead of inviting an improvised live install.
        command -v apt-get >/dev/null 2>&1 || die "no local biarch .debs and no apt-get; restore the agent toolchain cache (scripts/restore_agent_toolchain_cache.py --tag harness-cache-8) or pass --offline-debs DIR"
        dl_dir=$(mktemp -d "${TMPDIR:-/var/tmp}/lccc-i686-debs.XXXXXX")
        local gcc_major
        gcc_major=$(gcc -dumpversion 2>/dev/null | cut -d. -f1 || true)
        local -a pkgs=(libc6-i386 libc6-dev-i386)
        if [[ -n $gcc_major ]]; then
            pkgs+=("lib32gcc-${gcc_major}-dev" "lib32gcc-s1" "lib32stdc++-${gcc_major}-dev" "lib32stdc++6")
        fi
        if ! (cd "$dl_dir" && apt-get download "${pkgs[@]}" >/dev/null 2>&1); then
            rm -rf "$dl_dir"
            die "could not download biarch packages (offline host?). Restore the verified cache with scripts/restore_agent_toolchain_cache.py --tag harness-cache-8, or pass --offline-debs DIR. NEVER extract a .deb onto / -- use this script."
        fi
        for deb in "$dl_dir"/*.deb; do
            [[ -f $deb ]] || continue
            debs+=("$deb")
        done
    fi

    [[ ${#debs[@]} -gt 0 ]] || die "no biarch .debs selected from: ${search[*]}"

    assert_replaceable_sysroot "$sysroot"
    log "staging isolated rootless i386 sysroot at $sysroot from ${#debs[@]} verified .debs (live / is untouched)"
    # Stage beside the target and swap only once the tree is complete.  The
    # previous shape removed the working sysroot first, so an extraction that
    # failed half way left the caller with neither the old tree nor a new one.
    local parent base stage
    parent=$(dirname "$sysroot")
    base=$(basename "$sysroot")
    mkdir -p "$parent"
    stage=$(mktemp -d "$parent/.${base}.staging.XXXXXX")
    for deb in "${debs[@]}"; do
        if ! extract_deb_to_sysroot "$deb" "$stage"; then
            rm -rf "$stage"
            [[ -z $dl_dir ]] || rm -rf "$dl_dir"
            die "extraction failed for $deb; the previous sysroot is untouched"
        fi
    done
    [[ -z $dl_dir ]] || rm -rf "$dl_dir"

    reroot_absolute_symlinks "$stage"
    write_m32_wrappers "$stage" "${CC_FOR_M32_PROBE:-${CC:-gcc}}" "${CXX_FOR_M32_PROBE:-${CXX:-g++}}"

    cat >"$stage/README.DO_NOT_COPY_TO_ROOT.txt" <<'EOF'
Isolated 32-bit biarch sysroot staged by scripts/ensure_i386_multilib_safe.sh.
NEVER copy or dpkg -x / dpkg -i these files onto `/` -- doing so replaces the
usr-merge `/lib -> usr/lib` symlink and kills execve for every dynamic binary,
including the harness shell.  Source `env.sh` here instead, or run
`scripts/rootfs_jail.sh -- exec <cmd>` if you must run a provisioning command
that would otherwise need root.
EOF

    rm -rf "$sysroot"
    mv "$stage" "$sysroot"
    # The wrappers and env.sh embed their own absolute path, so they are
    # regenerated at the final location: a wrapper built for `.staging.XXXXXX`
    # would point every -I/-L/rpath at a directory that no longer exists.
    write_m32_wrappers "$sysroot" "${CC_FOR_M32_PROBE:-${CC:-gcc}}" "${CXX_FOR_M32_PROBE:-${CXX:-g++}}"

    assert_usrmerge_intact /
}

run_self_test() {
    local tmp
    tmp=$(mktemp -d "${TMPDIR:-/var/tmp}/lccc-m32-selftest.XXXXXX")

    # 1. Safe apt simulation (fresh biarch installs only, zero upgrades/removals).
    cat >"$tmp/safe.sim" <<'EOF'
Reading package lists...
Building dependency tree...
Inst libc6-i386 (2.41-12+deb13u4 Debian:13.1/stable [amd64])
Inst libc6-dev-i386 (2.41-12+deb13u4 Debian:13.1/stable [amd64])
Inst lib32gcc-14-dev (14.2.0-19 Debian:13.1/stable [amd64])
Inst gcc-multilib (4:14.2.0-1 Debian:13.1/stable [amd64])
Conf libc6-i386 (2.41-12+deb13u4 Debian:13.1/stable [amd64])
EOF
    audit_apt_simulation "$tmp/safe.sim"

    # 2. Unsafe live libc6 upgrade simulation MUST fail closed.
    cat >"$tmp/upgrade_libc.sim" <<'EOF'
Inst libc6 [2.41-12] (2.41-12+deb13u4 Debian-Security:13/stable-security [amd64])
Inst libc6-i386 (2.41-12+deb13u4 Debian-Security:13/stable-security [amd64])
EOF
    if audit_apt_simulation "$tmp/upgrade_libc.sim" 2>/dev/null; then
        rm -rf "$tmp"
        die "self-test failed: live libc6 upgrade was not rejected"
    fi

    # 3. Unsafe foreign :i386 install or package removal MUST fail closed.
    cat >"$tmp/foreign_i386.sim" <<'EOF'
Inst libc6:i386 (2.41-12+deb13u4 Debian:13.1/stable [i386])
EOF
    if audit_apt_simulation "$tmp/foreign_i386.sim" 2>/dev/null; then
        rm -rf "$tmp"
        die "self-test failed: foreign :i386 install was not rejected"
    fi

    cat >"$tmp/remv.sim" <<'EOF'
Remv gcc-aarch64-linux-gnu [4:14.2.0-1]
Inst gcc-multilib (4:14.2.0-1 Debian:13.1/stable [amd64])
EOF
    if audit_apt_simulation "$tmp/remv.sim" 2>/dev/null; then
        rm -rf "$tmp"
        die "self-test failed: package removal was not rejected"
    fi

    # 4. Unsafe sysroot paths (`/`, `/usr`, `/lib`, `/home/user/...`) MUST fail.
    local bad_path
    for bad_path in / /usr /lib /lib64 /home/user /home/user/.cache/sysroot; do
        if (validate_sysroot_path "$bad_path") 2>/dev/null; then
            rm -rf "$tmp"
            die "self-test failed: unsafe sysroot path '$bad_path' was accepted"
        fi
    done
    validate_sysroot_path "$tmp/isolated-sysroot"

    # 5. Broken usr-merge simulation MUST fail.
    mkdir -p "$tmp/fake_root/usr/lib/x86_64-linux-gnu" "$tmp/fake_root/lib"
    touch "$tmp/fake_root/usr/lib/x86_64-linux-gnu/libc.so.6"
    if (assert_usrmerge_intact "$tmp/fake_root") 2>/dev/null; then
        rm -rf "$tmp"
        die "self-test failed: clobbered /lib directory was not detected"
    fi

    # 6. The jail guard must be wired in, and its own proofs must pass when the
    #    host supports user namespaces.  A jail that is merely documented is the
    #    failure mode this repository already lived through three times.
    if ! declare -F jail_run >/dev/null; then
        rm -rf "$tmp"
        die "self-test failed: rootfs_jail.sh is not sourced"
    fi
    if jail_available; then
        bash "$SCRIPT_DIR/rootfs_jail.sh" --self-test >/dev/null \
            || { rm -rf "$tmp"; die "self-test failed: jail self-test"; }
    fi

    # 7. Package selection is by control field, never by filename: an amd64
    #    payload that happens to live in the same cache directory must not be
    #    dragged into a 32-bit sysroot.
    local pkg
    for pkg in libc6-i386 libc6-dev-i386 lib32gcc-12-dev lib32gcc-s1 lib32stdc++-12-dev; do
        [[ $pkg =~ $BIARCH_PACKAGE_RE ]] \
            || { rm -rf "$tmp"; die "self-test failed: biarch package $pkg rejected"; }
    done
    for pkg in libc6 libc6-dev libc6-dbg valgrind python3-yaml libc6:i386; do
        if [[ $pkg =~ $BIARCH_PACKAGE_RE ]]; then
            rm -rf "$tmp"
            die "self-test failed: non-biarch package $pkg accepted"
        fi
    done

    # 8. Absolute intra-sysroot symlinks are re-rooted; a link whose target does
    #    not exist inside the sysroot is left alone (it may be a legitimate
    #    dangling host link, and rewriting it would invent a file).
    mkdir -p "$tmp/sr/lib32" "$tmp/sr/lib" "$tmp/sr/usr/lib32"
    : >"$tmp/sr/lib32/ld-linux.so.2"
    ln -s /lib32/ld-linux.so.2 "$tmp/sr/lib/ld-linux.so.2"
    ln -s /nonexistent-lccc-probe "$tmp/sr/usr/lib32/dangling.so"
    reroot_absolute_symlinks "$tmp/sr" >/dev/null
    [[ $(readlink "$tmp/sr/lib/ld-linux.so.2") == ../lib32/ld-linux.so.2 ]] \
        || { rm -rf "$tmp"; die "self-test failed: absolute loader link not re-rooted"; }
    [[ -e "$tmp/sr/lib/ld-linux.so.2" ]] \
        || { rm -rf "$tmp"; die "self-test failed: re-rooted link does not resolve"; }
    [[ $(readlink "$tmp/sr/usr/lib32/dangling.so") == /nonexistent-lccc-probe ]] \
        || { rm -rf "$tmp"; die "self-test failed: dangling link was rewritten"; }

    # 9. The generated wrapper carries the three load-bearing linker flags.
    write_m32_wrappers "$tmp/sr" /usr/bin/true /usr/bin/true
    local wrapper
    wrapper=$(cat "$tmp/sr/bin/m32-cc")
    # printf %q escapes commas (-Wl\,-rpath\,...), so assert the semantic
    # flag names rather than an incidental quoting detail.
    [[ $(sed -n '4p' "$tmp/sr/bin/m32-cc") == exec\ /* ]] \
        || { rm -rf "$tmp"; die "self-test failed: wrapper does not exec an absolute driver path"; }
    for needle in '-m32' '--sysroot=' '--dynamic-linker=' 'rpath'; do
        [[ $wrapper == *"$needle"* ]] \
            || { rm -rf "$tmp"; die "self-test failed: wrapper lost $needle"; }
    done

    rm -rf "$tmp"
    log "self-test PASSED"
}

main() {
    case "${1:-}" in
        --self-test)
            run_self_test
            return 0
            ;;
        --audit-apt-sim)
            audit_apt_simulation "${2:?simulation file required}"
            return 0
            ;;
        --validate-sysroot-path)
            validate_sysroot_path "${2:?sysroot path required}"
            return 0
            ;;
        --offline-debs)
            local dir=${2:?directory of verified biarch .debs required}
            [[ -d $dir ]] || die "not a directory: $dir"
            stage_isolated_sysroot "$DEFAULT_SYSROOT" "$dir"
            if verify_staged_sysroot "$DEFAULT_SYSROOT"; then
                log "isolated -m32 sysroot verified at $DEFAULT_SYSROOT (offline, jailed, zero live-root writes)"
                printf 'LCCC_I686_SYSROOT=%s\n' "$DEFAULT_SYSROOT"
                printf 'source %s/env.sh\n' "$DEFAULT_SYSROOT"
                return 0
            fi
            die "offline sysroot staged but the -m32 compile/link/run probe failed"
            ;;
        --print-env)
            [[ -f $DEFAULT_SYSROOT/env.sh ]] || die "no staged sysroot env at $DEFAULT_SYSROOT/env.sh"
            cat "$DEFAULT_SYSROOT/env.sh"
            return 0
            ;;
        --check-only)
            assert_usrmerge_intact /
            if probe_m32 ""; then
                log "host -m32 C/runtime multilib healthy"
                return 0
            fi
            if [[ -d "$DEFAULT_SYSROOT" ]] && verify_staged_sysroot "$DEFAULT_SYSROOT"; then
                log "isolated -m32 sysroot healthy at $DEFAULT_SYSROOT"
                return 0
            fi
            die "no working -m32 multilib environment found (--check-only)"
            ;;
        "")
            ;;
        *)
            die "unknown option: $1"
            ;;
    esac

    assert_usrmerge_intact /

    # Fast path: host -m32 already compiles, links, and executes cleanly.
    if probe_m32 ""; then
        log "host -m32 multilib already healthy (no apt/dpkg action needed)"
        return 0
    fi

    # Second fast path: an isolated sysroot was already staged earlier.
    if [[ -d "$DEFAULT_SYSROOT" ]] && verify_staged_sysroot "$DEFAULT_SYSROOT"; then
        log "reusing healthy isolated -m32 sysroot: $DEFAULT_SYSROOT"
        printf 'LCCC_I686_SYSROOT=%s\n' "$DEFAULT_SYSROOT"
        return 0
    fi

    # Third fast path: a verified local biarch payload (restored agent cache or
    # LCCC_I686_DEBS).  This is the path an offline sandbox must take: it needs
    # neither root nor network, so there is never a reason to improvise a live
    # `dpkg -i` / `dpkg-deb -x ... /`.
    local offline_dir=${LCCC_I686_DEBS:-}
    if [[ -z $offline_dir ]]; then
        local candidate
        while IFS= read -r candidate; do
            if [[ -d $candidate ]] && compgen -G "$candidate/*.deb" >/dev/null; then
                offline_dir=$candidate
                break
            fi
        done < <(default_deb_dirs)
    fi
    if [[ -n $offline_dir ]]; then
        log "verified local biarch .debs found at $offline_dir; staging offline (no root, no network)"
        stage_isolated_sysroot "$DEFAULT_SYSROOT" "$offline_dir"
        if verify_staged_sysroot "$DEFAULT_SYSROOT"; then
            log "isolated -m32 sysroot verified at $DEFAULT_SYSROOT"
            printf 'LCCC_I686_SYSROOT=%s\n' "$DEFAULT_SYSROOT"
            printf 'source %s/env.sh\n' "$DEFAULT_SYSROOT"
            return 0
        fi
        log "offline sysroot did not satisfy the probe; continuing with the audited apt path"
    fi

    # Refresh apt metadata if sudo/root is available, without ever adding
    # foreign `i386` dpkg architecture.
    local -a sudo_cmd=()
    if [[ $(id -u) -ne 0 ]]; then
        if command -v sudo >/dev/null 2>&1 && sudo -n true 2>/dev/null; then
            sudo_cmd=(sudo)
        fi
    fi

    local sim_file
    sim_file=$(mktemp "${TMPDIR:-/var/tmp}/lccc-apt-m32-sim.XXXXXX")
    if [[ ${#sudo_cmd[@]} -gt 0 || $(id -u) -eq 0 ]]; then
        "${sudo_cmd[@]}" apt-get update -qq >/dev/null 2>&1 || true
    fi

    if apt-get -s --no-upgrade --no-install-recommends install \
        gcc-multilib g++-multilib libc6-dev-i386 >"$sim_file" 2>&1 \
        && audit_apt_simulation "$sim_file" \
        && [[ ${#sudo_cmd[@]} -gt 0 || $(id -u) -eq 0 ]]; then
        log "apt dry-run verified safe (zero live libc6/toolchain upgrades or removals); installing biarch multilib"
        rm -f "$sim_file"
        "${sudo_cmd[@]}" env DEBIAN_FRONTEND=noninteractive apt-get install -y \
            --no-install-recommends --no-upgrade \
            gcc-multilib g++-multilib libc6-dev-i386 >/dev/null
        assert_usrmerge_intact /
        if probe_m32 ""; then
            log "host -m32 multilib installed and verified safely"
            return 0
        fi
    else
        rm -f "$sim_file"
        log "live apt-get --no-upgrade multilib install declined (would mutate live libc6 or lacks privileges); falling back to isolated sysroot"
    fi

    stage_isolated_sysroot "$DEFAULT_SYSROOT"
    if verify_staged_sysroot "$DEFAULT_SYSROOT"; then
        log "isolated -m32 sysroot verified at $DEFAULT_SYSROOT"
        printf 'LCCC_I686_SYSROOT=%s\n' "$DEFAULT_SYSROOT"
        return 0
    fi

    die "unable to verify -m32 multilib via either safe --no-upgrade apt or isolated sysroot ($DEFAULT_SYSROOT)"
}

# Sourcing exposes the helpers to the contract tests without provisioning a
# sysroot, consulting apt, or requiring an already-staged env.sh.  The nested
# rootfs_jail.sh has the same guard; this outer script must not undo it by
# unconditionally running main with the caller's positional parameters.
if [[ ${BASH_SOURCE[0]} == "$0" ]]; then
    main "$@"
fi
