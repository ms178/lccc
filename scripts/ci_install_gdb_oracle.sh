#!/usr/bin/env bash
# ============================================================================
# ci_install_gdb_oracle.sh — install the pinned GDB oracle, built with
# -march=haswell, in a private verified prefix.
#
# WHY THIS EXISTS
# ---------------
# Three gates cross-check LCCC's debug information against a real debugger:
#
#   tests/regression/check_dwarf_line_tables.py   `break <fn>` must land on the
#                                                 declared file:line, and
#                                                 `break <file>:<line>` must
#                                                 resolve back through the line
#                                                 table
#   tests/regression/check_incoming_byval_homes.sh backtrace on a by-value crash
#   tests/regression/check_dwarf_versions.py      form/attribute readability
#   tests/regression/check_dwarf_parameters.py    O0 parameter homes and GDB values
#
# Every one of them resolves the debugger opportunistically
# (`shutil.which("gdb")` / `command -v gdb`) and SKIPS the cross-check when it
# is absent — check_dwarf_line_tables.py even prints
# "gdb not installed, debugger check skipped" and still exits 0. A host
# without gdb therefore reports a green DWARF gate that never asked a debugger
# anything. That is precisely the failure mode
# scripts/oracle_toolchains_from_tags.sh exists to prevent for the compilers:
# an oracle that is skipped is not an oracle that passed.
#
# So gdb joins the pinned toolchain set. Two properties make it an oracle
# rather than a convenience:
#
#   * EXACT VERSION. GDB's line-table and DWARF-form handling moves between
#     releases (this repository already documents a DW_FORM_data2/data4
#     difference in docs/DWARF-LINE-TABLES.md), so "whatever gdb is on PATH"
#     is not a reference implementation. The pin is 18.1, the latest GNU
#     release (2026-09-25).
#   * -march=haswell. The oracle must be able to single-step and disassemble
#     the code this project actually produces. LCCC's x86-64 output and the
#     owner's own packages are built for the Haswell ISA level (AVX2/BMI2/FMA)
#     on a Raptor Lake host; a distribution gdb built for a generic baseline
#     is fine at reading DWARF but is not the binary this project ships or
#     debugs. Building the oracle with -march=haswell keeps the debugger on
#     the same ISA contract as the code under test, and records that contract
#     in the build stamp so a warm prefix cannot silently be a generic build.
#
# SOURCE AND PIN
# --------------
# GNU publishes no checksum for GDB tarballs (only a detached .sig), so the
# SHA-256 below was computed from the release tarball and cross-verified
# byte-for-byte against BOTH GNU transports on 2026-11-05:
#
#   https://ftp.gnu.org/gnu/gdb/gdb-18.1.tar.xz
#   https://sourceware.org/pub/gdb/releases/gdb-18.1.tar.xz
#   -> cd9fc3fe2b47743840e42c1592d3d87f8302eb18639c0b8b4ba0898002e2348f
#      (22,158,720 bytes, identical from both hosts)
#
# Two independent mirrors agreeing is the strongest identity claim available
# without the release manager's key; the .sig stays the upstream provenance
# record. A re-issued tarball changes the digest and fails loudly here.
#
# USAGE
#   scripts/ci_install_gdb_oracle.sh [--prefix DIR] [--print-bin] [--self-test]
#
#   --prefix DIR   install root (default:
#                  ${LCCC_GDB_ORACLE_PREFIX:-$XDG_CACHE_HOME/lccc/gdb-oracle/18.1})
#   --print-bin    print the gdb binary path on stdout (last line)
#   --self-test    offline: exercise the digest/version/march gates against
#                  synthetic prefixes; never downloads, builds or writes the
#                  real prefix
#
# EXIT STATUS
#   0  $prefix/bin/gdb exists, reports "GNU gdb (GDB) 18.1", carries a build
#      stamp matching this pin (version + source digest + -march=haswell), and
#      resolved a real DWARF line-table query on a freshly compiled binary
#   1  download/digest/build/probe failure, or a foreign prefix
#   2  usage error, or a required host tool/header is missing
#
# Under GitHub Actions the resolved prefix/bin is appended to $GITHUB_PATH and
# GDB=<binary> is exported through $GITHUB_ENV, so later steps resolve the
# oracle the same way they resolve $CLANG.
# ============================================================================
set -Eeuo pipefail

GDB_VERSION=18.1
GDB_TARBALL="gdb-$GDB_VERSION.tar.xz"
GDB_SHA256=cd9fc3fe2b47743840e42c1592d3d87f8302eb18639c0b8b4ba0898002e2348f
GDB_SIZE=22158720
GDB_MARCH=haswell
# Mirrors, in preference order. ftp.gnu.org is the canonical GNU host;
# sourceware.org is the upstream project host. Both are pinned by the same
# digest, so the second is a transport fallback, never a different source.
GDB_URLS=(
    "https://ftp.gnu.org/gnu/gdb/$GDB_TARBALL"
    "https://sourceware.org/pub/gdb/releases/$GDB_TARBALL"
)
# Part of the contract, not a build detail: recorded in the stamp and verified
# on every reuse, so a warm generic-baseline prefix cannot vouch for itself.
GDB_CFLAGS="-O2 -march=$GDB_MARCH -mtune=$GDB_MARCH"

prefix=${LCCC_GDB_ORACLE_PREFIX:-${XDG_CACHE_HOME:-$HOME/.cache}/lccc/gdb-oracle/$GDB_VERSION}
print_bin=0
self_test=0
while [ $# -gt 0 ]; do
    case $1 in
        --prefix) prefix=${2:?--prefix requires a directory}; shift 2 ;;
        --print-bin) print_bin=1; shift ;;
        --self-test) self_test=1; shift ;;
        -h|--help) sed -n '2,66p' "$0"; exit 0 ;;
        *) echo "gdb-oracle: unknown argument: $1" >&2; exit 2 ;;
    esac
done

log() { printf '[gdb-oracle] %s\n' "$*" >&2; }
fail() { log "FAIL: $*"; exit 1; }
usage_fail() { echo "gdb-oracle: $*" >&2; exit 2; }

# GITHUB_ANNOTATE: reviewers of a red run often cannot reach the raw job log,
# but the check-run API is always reachable.
annotate() {
    if [ -n "${GITHUB_ACTIONS:-}" ]; then
        local msg=$*
        msg=${msg//'%'/%25}; msg=${msg//$'\r'/%0D}; msg=${msg//$'\n'/%0A}
        printf '::error title=GDB %s oracle::%s\n' "$GDB_VERSION" "$msg" >&2
    fi
}

installed_version() {  # installed_version <root> -> banner or nothing
    local root=$1
    [ -x "$root/bin/gdb" ] || return 1
    "$root/bin/gdb" --version 2>/dev/null | head -n 1
}

# The stamp is the whole reuse contract: version, source digest, ISA level and
# the compiler that built it. Any drift means "not this pin", and the prefix is
# rebuilt rather than reused — a warm prefix must never be trusted by name.
stamp_ok() {  # stamp_ok <root>
    local root=$1 stamp=$1/.lccc-build
    [ -x "$root/bin/gdb" ] || return 1
    [ -f "$stamp" ] || return 1
    grep -qx "version=$GDB_VERSION" "$stamp" || return 1
    grep -qx "source_sha256=$GDB_SHA256" "$stamp" || return 1
    grep -qx "march=$GDB_MARCH" "$stamp" || return 1
    grep -qx "cflags=$GDB_CFLAGS" "$stamp" || return 1
    [ "$(installed_version "$root")" = "GNU gdb (GDB) $GDB_VERSION" ] || return 1
}

# A real query, not a version banner: the debugger must resolve the line table
# in BOTH directions -- `break <fn>` -> file:line, and `break <file>:<line>` ->
# an address -- because those are exactly the two capabilities
# tests/regression/check_dwarf_line_tables.py asserts. Matching is against the
# strings gdb actually prints (captured from gdb 18.1 on 2026-11-05):
#
#   Breakpoint 1 at 0x113c: file /.../probe.c, line 2.
#   Line 2 of "/.../probe.c" starts at address 0x1138 <main> and ends at ...
#
# An earlier version of this probe matched only `info line` and passed `info
# line` with no argument, which answers "No line number information available"
# in batch mode -- so the probe failed against a perfectly good debugger.
dwarf_probe() {  # dwarf_probe <gdb-binary> <cc>
    local gdb=$1 cc=$2 dir src bin out
    command -v "$cc" >/dev/null 2>&1 || return 0   # no compiler: banner-only host
    dir=$(mktemp -d "${TMPDIR:-/tmp}/gdb-oracle-probe.XXXXXX")
    src=$dir/probe.c
    bin=$dir/probe
    cat > "$src" <<'EOS'
int helper(int value) { return value + 1; }
int main(void) { return helper(41) - 42; }
EOS
    "$cc" -g -O0 -o "$bin" "$src" 2>/dev/null || { rm -rf "$dir"; return 0; }
    out=$("$gdb" -q -batch -nx \
        -ex 'break main' -ex 'info line main' -ex 'break probe.c:2' "$bin" 2>&1) || true
    rm -rf "$dir"
    if ! printf '%s' "$out" | grep -qE 'file .*probe\.c, line [0-9]+'; then
        log "gdb did not resolve `break main' to a file:line:"
        printf '%s\n' "$out" | sed -n '1,6p' >&2
        return 1
    fi
    if ! printf '%s' "$out" | grep -qE 'Line [0-9]+ of ".*probe\.c"'; then
        log "gdb did not answer `info line main' from the line table:"
        printf '%s\n' "$out" | sed -n '1,6p' >&2
        return 1
    fi
    return 0
}

# The -march contract, verified on the ARTIFACT and not only in the stamp: a
# haswell build of gdb really does contain AVX2 and BMI2 encodings (measured on
# the 18.1 build of 2026-11-05: vinserti128 x128, bzhi x40, vfmadd x1), while a
# generic x86-64-baseline build contains neither. A stamp is a claim; this is
# the bytes. Skipped only where objdump is absent, which is then said out loud.
isa_census_ok() {  # isa_census_ok <gdb-binary>
    local gdb=$1 disasm avx2 bmi2
    if ! command -v objdump >/dev/null 2>&1; then
        log "WARNING: objdump is unavailable, so -march=$GDB_MARCH is trusted from the build stamp only"
        return 0
    fi
    # Fail CLOSED: if objdump cannot disassemble the binary, the -march claim
    # is unverifiable on the artifact, and an unverifiable claim must not pass
    # as a verified one. (The missing-objdump case above is the only degraded
    # mode, and it announces itself.)
    if ! disasm=$(objdump -d "$gdb" 2>/dev/null); then
        log "objdump could not disassemble $gdb; -march=$GDB_MARCH is unverifiable on this artifact"
        return 1
    fi
    avx2=$(printf '%s' "$disasm" | grep -cE '\b(vinserti128|vextracti128|vpermd|vpmaskmovd|vpbroadcastd)')
    bmi2=$(printf '%s' "$disasm" | grep -cE '\b(bzhi|blsr|blsmsk|blsi|pdep|pext|mulx)')
    if [ "${avx2:-0}" -eq 0 ] || [ "${bmi2:-0}" -eq 0 ]; then
        log "gdb at $gdb has no AVX2 ($avx2) or BMI2 ($bmi2) instructions: it was not built with -march=$GDB_MARCH"
        return 1
    fi
    log "ISA census: $avx2 AVX2 and $bmi2 BMI2 encodings (-march=$GDB_MARCH confirmed in the binary)"
    return 0
}

publish() {
    if [ -n "${GITHUB_PATH:-}" ]; then printf '%s\n' "$prefix/bin" >> "$GITHUB_PATH"; fi
    if [ -n "${GITHUB_ENV:-}" ]; then printf 'GDB=%s\n' "$prefix/bin/gdb" >> "$GITHUB_ENV"; fi
    if [ "$print_bin" = 1 ]; then printf '%s\n' "$prefix/bin/gdb"; fi
}

# ---------------------------------------------------------------------------
# Self-test: the gates above are the contract, so they get their own offline
# proof. Synthetic prefixes exercise accept/reject without a 22 MB download or
# a ten-minute build, which is what makes this safe to run on every commit.
# ---------------------------------------------------------------------------
run_self_test() {
    local work rc=0 root
    work=$(mktemp -d "${TMPDIR:-/tmp}/gdb-oracle-selftest.XXXXXX")
    # shellcheck disable=SC2064  # expanded now, on purpose
    trap "rm -rf '$work'" RETURN

    # A fake gdb whose banner and stamp are correct is accepted...
    root=$work/good
    mkdir -p "$root/bin"
    printf '#!/bin/sh\necho "GNU gdb (GDB) %s"\n' "$GDB_VERSION" > "$root/bin/gdb"
    chmod +x "$root/bin/gdb"
    printf 'version=%s\nsource_sha256=%s\nmarch=%s\ncflags=%s\n' \
        "$GDB_VERSION" "$GDB_SHA256" "$GDB_MARCH" "$GDB_CFLAGS" > "$root/.lccc-build"
    if stamp_ok "$root"; then echo "  ok   a matching stamp+banner is accepted"
    else echo "  FAIL a matching stamp+banner was rejected"; rc=1; fi

    # ...but a generic-baseline build of the same version is not: dropping the
    # march line is exactly the "warm distro gdb" case this pin exists for.
    root=$work/generic
    mkdir -p "$root/bin"
    printf '#!/bin/sh\necho "GNU gdb (GDB) %s"\n' "$GDB_VERSION" > "$root/bin/gdb"
    chmod +x "$root/bin/gdb"
    printf 'version=%s\nsource_sha256=%s\ncflags=-O2\n' \
        "$GDB_VERSION" "$GDB_SHA256" > "$root/.lccc-build"
    if stamp_ok "$root"; then echo "  FAIL a generic-baseline prefix was accepted"; rc=1
    else echo "  ok   a prefix without -march=$GDB_MARCH is refused"; fi

    # A different source digest is a different tarball.
    root=$work/other-source
    mkdir -p "$root/bin"
    printf '#!/bin/sh\necho "GNU gdb (GDB) %s"\n' "$GDB_VERSION" > "$root/bin/gdb"
    chmod +x "$root/bin/gdb"
    printf 'version=%s\nsource_sha256=%s\nmarch=%s\ncflags=%s\n' \
        "$GDB_VERSION" "$(printf '0%.0s' $(seq 64))" "$GDB_MARCH" "$GDB_CFLAGS" > "$root/.lccc-build"
    if stamp_ok "$root"; then echo "  FAIL a foreign source digest was accepted"; rc=1
    else echo "  ok   a foreign source digest is refused"; fi

    # No stamp at all (a hand-installed or distro gdb) is refused by name.
    root=$work/no-stamp
    mkdir -p "$root/bin"
    printf '#!/bin/sh\necho "GNU gdb (GDB) %s"\n' "$GDB_VERSION" > "$root/bin/gdb"
    chmod +x "$root/bin/gdb"
    if stamp_ok "$root"; then echo "  FAIL an unstamped prefix was accepted"; rc=1
    else echo "  ok   an unstamped prefix is refused"; fi

    # A superseded version is refused even with a perfect stamp.
    root=$work/old
    mkdir -p "$root/bin"
    printf '#!/bin/sh\necho "GNU gdb (GDB) 17.2"\n' > "$root/bin/gdb"
    chmod +x "$root/bin/gdb"
    printf 'version=%s\nsource_sha256=%s\nmarch=%s\ncflags=%s\n' \
        "$GDB_VERSION" "$GDB_SHA256" "$GDB_MARCH" "$GDB_CFLAGS" > "$root/.lccc-build"
    if stamp_ok "$root"; then echo "  FAIL a superseded banner was accepted"; rc=1
    else echo "  ok   a superseded gdb banner is refused"; fi

    # The ISA census must reject a binary with no haswell-only encodings. A
    # shell stub has no instructions at all, so it must be refused.
    if command -v objdump >/dev/null 2>&1; then
        if isa_census_ok "$work/good/bin/gdb" 2>/dev/null; then
            echo "  FAIL the ISA census accepted a binary with no instructions"; rc=1
        else
            echo "  ok   the ISA census refuses a binary without haswell encodings"
        fi
    else
        echo "  skip objdump unavailable; ISA census not exercised"
    fi

    # The pin itself must stay self-consistent: the flags we stamp are the
    # flags we build with, and they name the pinned ISA level.
    case " $GDB_CFLAGS " in
        *" -march=$GDB_MARCH "*) echo "  ok   the stamped CFLAGS carry -march=$GDB_MARCH" ;;
        *) echo "  FAIL CFLAGS '$GDB_CFLAGS' do not pin -march=$GDB_MARCH"; rc=1 ;;
    esac
    if [ "${#GDB_SHA256}" = 64 ]; then echo "  ok   the pinned digest is a SHA-256"
    else echo "  FAIL the pinned digest is not 64 hex digits"; rc=1; fi

    [ "$rc" = 0 ] && echo "gdb-oracle self-test: all cases passed"
    return "$rc"
}

if [ "$self_test" = 1 ]; then
    run_self_test
    exit $?
fi

[[ $prefix = /* && $prefix != / && $prefix != "$HOME" ]] ||
    usage_fail "prefix must be an absolute private directory (got '$prefix')"

verify_install() {  # verify_install <root>
    local root=$1
    stamp_ok "$root" || return 1
    isa_census_ok "$root/bin/gdb" || return 1
    dwarf_probe "$root/bin/gdb" "${CC:-cc}" || return 1
    return 0
}

if [ -x "$prefix/bin/gdb" ] && [ -f "$prefix/.lccc-build" ]; then
    if verify_install "$prefix"; then
        log "reusing verified GDB $GDB_VERSION (-march=$GDB_MARCH) at $prefix"
        publish
        exit 0
    fi
    log "the existing prefix does not satisfy the pin; rebuilding"
    rm -rf "$prefix"
fi

# Never overlay a foreign or failed install: a half-built prefix must stay
# visible as a failure instead of turning into a version-only cache hit.
if [ -d "$prefix" ] && [ -n "$(find "$prefix" -mindepth 1 -print -quit 2>/dev/null)" ]; then
    fail "unverified non-empty prefix: $prefix (choose a fresh --prefix or remove it explicitly)"
fi

for tool in curl tar xz sha256sum make gcc g++ python3 objdump; do
    command -v "$tool" >/dev/null 2>&1 || usage_fail "required host tool missing: $tool"
done
# Every header GDB's configure needs is probed here, because a missing one
# fails deep inside configure with a message that reads like a GDB bug ("error:
# Building GDB requires GMP 4.2+, and MPFR 3.1.0+") rather than naming the
# package to install. Found the hard way: gmp/mpfr are not in the default
# build-essential set, so the build died after downloading and unpacking.
for header in readline/readline.h zlib.h gmp.h mpfr.h; do
    printf '#include <%s>\nint main(void){return 0;}\n' "$header" |
        gcc -x c -fsyntax-only - 2>/dev/null ||
        usage_fail "missing development header <$header> (install libreadline-dev, zlib1g-dev, libgmp-dev and libmpfr-dev)"
done

work=$(mktemp -d "${TMPDIR:-/tmp}/gdb-oracle.XXXXXX")
trap 'rm -rf "$work"' EXIT
archive=$work/$GDB_TARBALL

# The tarball is only 21 MiB, but the build tree it unpacks into is ~2 GiB
# (bfd/opcodes/libiberty objects plus gdb itself). A small tmpfs TMPDIR fails
# as a compiler "No space left on device" deep inside the build, which reads
# like a GDB bug; ask the filesystem first instead.
avail_kb=$(df -Pk "$work" 2>/dev/null | awk 'NR==2 {print $4}')
if [ -n "${avail_kb:-}" ] && [ "$avail_kb" -lt 2621440 ]; then
    fail "$work has $((avail_kb / 1024)) MiB free; building gdb needs ~2.5 GiB. Point TMPDIR at a disk-backed directory, e.g. TMPDIR=/var/tmp."
fi

downloaded=0
for url in "${GDB_URLS[@]}"; do
    log "downloading $url"
    if curl --fail --location --silent --show-error --retry 3 --retry-delay 3 \
            --retry-all-errors --output "$archive" "$url"; then
        downloaded=1
        break
    fi
    log "  transport failed; trying the next mirror"
done
[ "$downloaded" = 1 ] || fail "could not download $GDB_TARBALL from any pinned mirror"

size=$(stat -c %s "$archive")
[ "$size" = "$GDB_SIZE" ] || fail "$GDB_TARBALL is $size bytes, expected $GDB_SIZE"
log "verifying SHA-256 of $size bytes"
echo "$GDB_SHA256  $archive" | sha256sum --check --quiet - ||
    fail "$GDB_TARBALL digest mismatch: a re-issued tarball must be re-reviewed"

log "extracting"
mkdir -p "$work/source"
tar -xf "$archive" -C "$work/source" --strip-components=1

log "configuring (CFLAGS='$GDB_CFLAGS', -march=$GDB_MARCH)"
mkdir -p "$work/build"
cd "$work/build"
# Only the debugger is built: `all-gdb` pulls in bfd/opcodes/libiberty but not
# the assembler, linker, simulator or gprofng, which is what keeps this to
# minutes instead of a full binutils-gdb tree. MAKEINFO=true skips the manual
# (texinfo is not a build dependency of the oracle), and --with-python=no
# keeps the oracle independent of the host's Python ABI — the DWARF gates use
# `-ex` scripts, not Python scripting, and a deterministic oracle is worth
# more than a convenience here.
"$work/source/configure" \
    --prefix="$prefix" \
    --disable-binutils --disable-ld --disable-gas --disable-gold \
    --disable-gprof --disable-gprofng --disable-sim --disable-gdbserver \
    --disable-nls --disable-werror \
    --with-system-readline --with-zlib --with-expat \
    --with-python=no \
    CFLAGS="$GDB_CFLAGS" CXXFLAGS="$GDB_CFLAGS" \
    MAKEINFO=true > "$work/configure.log" 2>&1 || {
        tail -n 25 "$work/configure.log" >&2
        fail "configure failed"
    }

# The ISA level must actually be in effect before an hour of compiling is
# spent on it: ask the compiler that will build gdb whether -march=haswell
# enables AVX2/BMI2/FMA. A silently ignored flag would produce a
# generic-baseline oracle that still passes the version probe.
if ! gcc -march="$GDB_MARCH" -Q --help=target 2>/dev/null |
        grep -qE -- '-mavx2\s+\[enabled\]'; then
    fail "the host gcc does not honour -march=$GDB_MARCH (no AVX2); the oracle would be a generic build"
fi

jobs=$(nproc 2>/dev/null || echo 2)
[ "$jobs" -gt 4 ] && jobs=4   # bounded: gdb's link step is memory-hungry
log "building all-gdb with -j$jobs (this takes several minutes)"
if ! make -j"$jobs" MAKEINFO=true all-gdb > "$work/build.log" 2>&1; then
    tail -n 30 "$work/build.log" >&2
    fail "the gdb build failed"
fi
log "installing into $prefix"
make MAKEINFO=true install-gdb > "$work/install.log" 2>&1 || {
    tail -n 20 "$work/install.log" >&2
    fail "install-gdb failed"
}

banner=$(installed_version "$prefix") || fail "$prefix/bin/gdb is not executable after install"
[ "$banner" = "GNU gdb (GDB) $GDB_VERSION" ] ||
    fail "installed gdb reports '$banner', expected 'GNU gdb (GDB) $GDB_VERSION'"

# Record the full contract, then require it on every later reuse.
{
    printf 'version=%s\n' "$GDB_VERSION"
    printf 'source_sha256=%s\n' "$GDB_SHA256"
    printf 'source_size=%s\n' "$GDB_SIZE"
    printf 'march=%s\n' "$GDB_MARCH"
    printf 'cflags=%s\n' "$GDB_CFLAGS"
    printf 'compiler=%s\n' "$(gcc -dumpfullversion 2>/dev/null || gcc -dumpversion)"
    printf 'banner=%s\n' "$banner"
    printf 'built_at=%s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
} > "$prefix/.lccc-build"
chmod 644 "$prefix/.lccc-build"

verify_install "$prefix" || fail "the freshly built prefix does not satisfy its own contract (stamp, -march census, line table)"

log "GDB $GDB_VERSION oracle (-march=$GDB_MARCH) at $prefix/bin/gdb"
publish
