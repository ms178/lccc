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
#                                                 /var/tmp/lccc-i686-sysroot.
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
        /home/user|/home/user/*)
            die "refusing sysroot path '$canon' inside /home/user (exceeds Arena workspace snapshot quota; use /var/tmp/lccc-i686-sysroot)"
            ;;
    esac
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

# Compile, link, and execute a real 32-bit C (and optional C++) binary.
probe_m32() {
    local sysroot=${1:-}
    local cc_bin=${CC_FOR_M32_PROBE:-gcc}
    command -v "$cc_bin" >/dev/null 2>&1 || return 1

    local tmp
    tmp=$(mktemp -d "${TMPDIR:-/var/tmp}/lccc-m32-probe.XXXXXX")
    cat >"$tmp/probe.c" <<'EOF'
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
    local -a extra_flags=()
    local ld_path="${LD_LIBRARY_PATH:-}"
    if [[ -n "$sysroot" ]]; then
        extra_flags+=(
            "-B$sysroot/usr/lib32"
            "-B$sysroot/lib32"
            "-idirafter" "$sysroot/usr/include"
            "-idirafter" "$sysroot/usr/include/i386-linux-gnu"
            "-L$sysroot/usr/lib32"
            "-L$sysroot/lib32"
            "-L$sysroot/lib/i386-linux-gnu"
            "-L$sysroot/usr/lib/i386-linux-gnu"
        )
        if [[ -e "$sysroot/lib/ld-linux.so.2" ]]; then
            extra_flags+=("-Wl,--dynamic-linker=$sysroot/lib/ld-linux.so.2")
        elif [[ -e "$sysroot/lib32/ld-linux.so.2" ]]; then
            extra_flags+=("-Wl,--dynamic-linker=$sysroot/lib32/ld-linux.so.2")
        fi
        ld_path="$sysroot/lib32:$sysroot/usr/lib32:$sysroot/lib/i386-linux-gnu:$sysroot/usr/lib/i386-linux-gnu${ld_path:+:$ld_path}"
    fi

    local ok=0
    if "$cc_bin" -m32 "${extra_flags[@]}" "$tmp/probe.c" -o "$tmp/probe" >/dev/null 2>&1; then
        if LD_LIBRARY_PATH="$ld_path" "$tmp/probe" >/dev/null 2>&1; then
            ok=1
        fi
    fi
    rm -rf "$tmp"
    [[ $ok -eq 1 ]]
}

stage_isolated_sysroot() {
    local sysroot=$1
    validate_sysroot_path "$sysroot"
    assert_usrmerge_intact /

    command -v apt-get >/dev/null 2>&1 || die "apt-get not available to download isolated multilib packages"
    command -v dpkg-deb >/dev/null 2>&1 || die "dpkg-deb not available to unpack isolated multilib packages"

    local dl_dir
    dl_dir=$(mktemp -d "${TMPDIR:-/var/tmp}/lccc-i686-debs.XXXXXX")
    log "staging isolated rootless i386 sysroot at $sysroot (live / is untouched)"

    local gcc_major
    gcc_major=$(gcc -dumpversion 2>/dev/null | cut -d. -f1 || true)
    local -a pkgs=(libc6-i386 libc6-dev-i386)
    if [[ -n "$gcc_major" ]]; then
        pkgs+=("lib32gcc-${gcc_major}-dev" "lib32gcc-s1" "lib32stdc++-${gcc_major}-dev" "lib32stdc++6")
    fi

    (
        cd "$dl_dir"
        apt-get download "${pkgs[@]}" >/dev/null 2>&1 \
            || apt-get download libc6-i386 libc6-dev-i386 >/dev/null
    ) || {
        rm -rf "$dl_dir"
        die "failed to download biarch packages into staging directory"
    }

    rm -rf "$sysroot"
    mkdir -p "$sysroot"
    local deb
    for deb in "$dl_dir"/*.deb; do
        [[ -f "$deb" ]] || continue
        dpkg-deb -x "$deb" "$sysroot"
    done
    rm -rf "$dl_dir"

    cat >"$sysroot/README.DO_NOT_COPY_TO_ROOT.txt" <<'EOF'
Isolated 32-bit biarch sysroot staged by scripts/ensure_i386_multilib_safe.sh.
NEVER copy or dpkg -x / dpkg -i these files onto `/` — doing so can replace
the usr-merge `/lib -> usr/lib` symlink or skew live amd64 glibc inside the
running container.  Point LCCC_I686_SYSROOT at this directory instead.
EOF

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
        --check-only)
            assert_usrmerge_intact /
            if probe_m32 ""; then
                log "host -m32 C/runtime multilib healthy"
                return 0
            fi
            if [[ -d "$DEFAULT_SYSROOT" ]] && probe_m32 "$DEFAULT_SYSROOT"; then
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
    if [[ -d "$DEFAULT_SYSROOT" ]] && probe_m32 "$DEFAULT_SYSROOT"; then
        log "reusing healthy isolated -m32 sysroot: $DEFAULT_SYSROOT"
        printf 'LCCC_I686_SYSROOT=%s\n' "$DEFAULT_SYSROOT"
        return 0
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
    if probe_m32 "$DEFAULT_SYSROOT"; then
        log "isolated -m32 sysroot verified at $DEFAULT_SYSROOT"
        printf 'LCCC_I686_SYSROOT=%s\n' "$DEFAULT_SYSROOT"
        return 0
    fi

    die "unable to verify -m32 multilib via either safe --no-upgrade apt or isolated sysroot ($DEFAULT_SYSROOT)"
}

main "$@"
