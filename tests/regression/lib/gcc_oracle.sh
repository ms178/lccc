#!/usr/bin/env bash
# Shared GCC-oracle policy for the profile/initializer/named-space gates.
#
# Those gates compare *lccc's verdict or output* against a reference compiler,
# and the reference's identity is part of the contract: the tables under
# tests/regression/{seg_conv,static_local,file_scope_init,designator_init}
# were measured against a GCC whose MAJOR VERSION is recorded here, and an
# oracle that is not that version can disagree with a row for reasons that have
# nothing to do with lccc (GCC 13.3 rejects `long __seg_gs *p = gobj.a;` as a
# non-constant initializer, which GCC >= 14 accepts).
#
# Resolution, in order:
#   1. $GCC, if set. It must name an executable reporting $GCC_ORACLE_MAJOR,
#      otherwise the gate fails: an explicit choice is never replaced.
#   2. gcc-$GCC_ORACLE_MAJOR on PATH (the hosted workflow installs exactly
#      this: the ubuntu-24.04 archive ships gcc-14, and the CI step installs it).
#   3. gcc on PATH, only if it reports $GCC_ORACLE_MAJOR.
#
# The default major is the version the fixtures were measured against AND the
# one the hosted runner can install from its own archive (14). An environment
# without it (an older workstation, a bench box) may set
# LCCC_GCC_ORACLE_MAJOR=<major>, which is accepted ONLY with a loud banner: a
# comparison against a substitute oracle must never look like the pinned one.
#
# Policy when no usable oracle is found: FAIL, locally and under CI. A gate that
# silently skipped its cross-check would print PASSED for a comparison that
# never ran, so the absence is reported as the configuration error it is.
#
# Usage (from a gate script, run from the repository root):
#   . tests/regression/lib/gcc_oracle.sh
#   require_gcc_oracle "$gate_name" || exit 1
#   gcc_oracle_positive_control "$CCC" "$tmp" || exit 1
#   gcc_oracle_positive_control "$GCC_ORACLE_BIN" "$tmp" || exit 1
#   "$GCC_ORACLE_BIN" -w -c -o "$tmp/g.o" fixture.c   # the oracle invocation
#
# Variables set by require_gcc_oracle (read by the sourcing gate):
#   GCC_ORACLE_BIN      absolute path of the oracle binary
#   GCC_ORACLE_VERSION  its `gcc --version` first line (for PASS summaries)
#   GCC_ORACLE_MAJOR    the major version it reported
#   GCC_ORACLE          always 1 after a successful require (kept for gates
#                       that branch on it; a missing oracle fails, never 0)
# shellcheck disable=SC2034  # all four are consumed by the gates that source this

GCC_ORACLE_MAJOR=${LCCC_GCC_ORACLE_MAJOR:-14}
GCC_ORACLE_SUBSTITUTE=0

# gcc_oracle_major <gcc-binary> -> the major version, or nothing.
gcc_oracle_major() {
    "$1" -dumpfullversion 2>/dev/null | cut -d. -f1
}

gcc_oracle_accept() {
    # accept <bin> <major>: record the oracle, banner a substituted major.
    GCC_ORACLE=1
    GCC_ORACLE_BIN=$1
    GCC_ORACLE_MAJOR=$2
    GCC_ORACLE_VERSION=$("$1" --version 2>/dev/null | head -1)
    if [ "$GCC_ORACLE_SUBSTITUTE" = 1 ]; then
        echo "NOTE: oracle substituted by LCCC_GCC_ORACLE_MAJOR=$2" >&2
        echo "      pinned oracle major is 14; this run compares against: $GCC_ORACLE_VERSION" >&2
    fi
    return 0
}

require_gcc_oracle() {
    local gate=$1 bin major found=""
    GCC_ORACLE=0
    GCC_ORACLE_BIN=""
    if [ -n "${LCCC_GCC_ORACLE_MAJOR:-}" ]; then
        GCC_ORACLE_SUBSTITUTE=1
    fi
    if [ -n "${GCC:-}" ]; then
        # An explicit oracle that cannot be used is a configuration error,
        # never a silent fallback to another compiler.
        bin=$(command -v "$GCC" 2>/dev/null || true)
        if [ -z "$bin" ]; then
            echo "FAIL: $gate: GCC=$GCC is not an executable; the oracle is GCC $GCC_ORACLE_MAJOR (this gate requires GCC $GCC_ORACLE_MAJOR exactly)" >&2
            return 1
        fi
        major=$(gcc_oracle_major "$bin")
        if [ "$major" != "$GCC_ORACLE_MAJOR" ]; then
            echo "FAIL: $gate: GCC=$GCC is GCC ${major:-?}; the oracle is GCC $GCC_ORACLE_MAJOR (this gate requires GCC $GCC_ORACLE_MAJOR exactly)" >&2
            return 1
        fi
        gcc_oracle_accept "$bin" "$major"
        return 0
    fi
    for cand in "gcc-$GCC_ORACLE_MAJOR" gcc; do
        bin=$(command -v "$cand" 2>/dev/null || true)
        [ -n "$bin" ] || continue
        major=$(gcc_oracle_major "$bin")
        if [ "$major" = "$GCC_ORACLE_MAJOR" ]; then
            gcc_oracle_accept "$bin" "$major"
            return 0
        fi
        found="${found:+$found, }$cand=GCC ${major:-?}"
    done
    echo "FAIL: $gate needs GCC $GCC_ORACLE_MAJOR as the verdict oracle; found: ${found:-none}." >&2
    echo "      Install it (apt install gcc-$GCC_ORACLE_MAJOR), set GCC=<path to gcc-$GCC_ORACLE_MAJOR>," >&2
    echo "      or -- on a host that cannot install it -- run with LCCC_GCC_ORACLE_MAJOR=<major>" >&2
    echo "      after checking that host's GCC agrees with the fixture tables." >&2
    return 1
}

# gcc_oracle_positive_control <compiler> <tmpdir>
# A named-space semantic positive control: the compiler must ACCEPT a valid
# program that uses a __seg_gs object through a constant-address pointer. A
# broken compiler or toolchain must not make the negative fixtures look like
# valid rejections, so every compiler in a gate is checked this way first.
gcc_oracle_positive_control() {
    local compiler=$1 tmp=$2
    cat > "$tmp/control.c" <<'C'
struct S { long a[3]; };
extern struct S __seg_gs object;
long __seg_gs *p = object.a + 1;
C
    if ! "$compiler" -std=gnu11 -c "$tmp/control.c" -o "$tmp/control.o" >"$tmp/control.err" 2>&1; then
        echo "FAIL: $compiler failed the named-space positive control" >&2
        cat "$tmp/control.err" >&2
        return 1
    fi
    return 0
}

# gcc_oracle_require_m32 <gate> -> 0 if $GCC_ORACLE_BIN can link AND run an
# i686 program, else prints a configuration FAIL and returns 1. Gates that use
# the oracle as an i686 reference (`"$ref" -m32`) need the 32-bit crt objects
# and libgcc of THAT GCC, which the distribution `gcc-multilib` does not supply
# for a secondary GCC; the CI step installs gcc-14-multilib for exactly this.
gcc_oracle_require_m32() {
    local gate=$1 td
    [ -n "${GCC_ORACLE_BIN:-}" ] || { echo "FAIL: $gate: gcc_oracle_require_m32 called before require_gcc_oracle" >&2; return 1; }
    td=$(mktemp -d)
    printf 'int main(void){return 0;}\n' > "$td/m32.c"
    if ! "$GCC_ORACLE_BIN" -m32 "$td/m32.c" -o "$td/m32" >"$td/m32.err" 2>&1 || ! "$td/m32"; then
        echo "FAIL: $gate: the GCC $GCC_ORACLE_MAJOR oracle ($GCC_ORACLE_BIN) cannot link and run -m32 programs." >&2
        echo "      Install its 32-bit runtime: apt install gcc-$GCC_ORACLE_MAJOR-multilib lib32gcc-$GCC_ORACLE_MAJOR-dev." >&2
        head -5 "$td/m32.err" >&2
        rm -rf "$td"
        return 1
    fi
    rm -rf "$td"
    return 0
}
