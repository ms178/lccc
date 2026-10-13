#!/usr/bin/env bash
# Shared GCC-oracle policy for the named-address-space gates.
#
# The fixtures' verdicts are compared against GCC 16.2, the correctness oracle
# for every seg gate. The MAJOR VERSION is part of that contract:
#   * GCC 13.3 (Ubuntu 24.04's default `gcc`, i.e. what `ubuntu-latest` runs)
#     disagrees with newer GCC on named-address-space verdicts (for example it
#     rejects `long __seg_gs *p = gobj.a;` as a non-constant initializer).
#   * So a gate never runs against "whatever `gcc` is on PATH", and never
#     against an older or different GCC: the pin keeps the comparison
#     reproducible. Any other GCC is a configuration error, never a silent
#     substitute.
#
# Resolution, in order:
#   1. $GCC, if set. It must name an executable that reports major version 16,
#      otherwise the gate fails (an explicit choice is never replaced).
#   2. gcc-16 on PATH (the hosted workflow installs it explicitly).
#   3. gcc on PATH, only if it reports major version 16.
#
# Policy when no GCC 16 oracle is found: FAIL, locally and under CI. A gate that
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
#   GCC_ORACLE_BIN  absolute path of the GCC 16 oracle binary
#   GCC_ORACLE      always 1 after a successful require (kept for gates that
#                   branch on it; a missing oracle now fails instead of 0)
# shellcheck disable=SC2034  # both are consumed by the gates that source this file

GCC_ORACLE_MAJOR=16

# gcc_oracle_major <gcc-binary> -> the major version, or nothing.
gcc_oracle_major() {
    "$1" -dumpfullversion 2>/dev/null | cut -d. -f1
}

require_gcc_oracle() {
    local gate=$1 cand bin major found=""
    GCC_ORACLE=0
    GCC_ORACLE_BIN=""
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
        GCC_ORACLE=1
        GCC_ORACLE_BIN=$bin
        return 0
    fi
    for cand in gcc-16 gcc; do
        bin=$(command -v "$cand" 2>/dev/null || true)
        [ -n "$bin" ] || continue
        major=$(gcc_oracle_major "$bin")
        if [ "$major" = "$GCC_ORACLE_MAJOR" ]; then
            GCC_ORACLE=1
            GCC_ORACLE_BIN=$bin
            return 0
        fi
        found="${found:+$found, }$cand=GCC ${major:-?}"
    done
    echo "FAIL: $gate needs GCC $GCC_ORACLE_MAJOR as the verdict oracle; found: ${found:-none}." >&2
    echo "      Install gcc-16 (apt install gcc-16), or set GCC=<path to gcc-16>." >&2
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
# and libgcc of THAT GCC, which the distribution `gcc-multilib` does not supply for gcc-16.
gcc_oracle_require_m32() {
    local gate=$1 td
    [ -n "${GCC_ORACLE_BIN:-}" ] || { echo "FAIL: $gate: gcc_oracle_require_m32 called before require_gcc_oracle" >&2; return 1; }
    td=$(mktemp -d)
    printf 'int main(void){return 0;}\n' > "$td/m32.c"
    if ! "$GCC_ORACLE_BIN" -m32 "$td/m32.c" -o "$td/m32" >"$td/m32.err" 2>&1 || ! "$td/m32"; then
        echo "FAIL: $gate: the GCC $GCC_ORACLE_MAJOR oracle ($GCC_ORACLE_BIN) cannot link and run -m32 programs." >&2
        echo "      Install its 32-bit runtime: apt install gcc-16-multilib lib32gcc-16-dev (the CI step does exactly this)." >&2
        head -5 "$td/m32.err" >&2
        rm -rf "$td"
        return 1
    fi
    rm -rf "$td"
    return 0
}
