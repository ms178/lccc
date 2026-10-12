#!/usr/bin/env bash
# Shared GCC-oracle policy for the named-address-space gates.
#
# The fixtures' verdicts were recorded against GCC 14.2, and that is the oracle
# every seg gate compares against. The MAJOR VERSION is part of that contract:
#   * GCC 13.3 (Ubuntu 24.04's default `gcc`, i.e. what `ubuntu-latest` runs)
#     rejects `long __seg_gs *p = gobj.a;` as a non-constant initializer that
#     GCC 14 accepts, and only warns on `qq = pp` for a pointee-chain space
#     change that GCC 14 errors on. PR #803's Test Suite failed exactly this
#     way: `gcc` on the runner was 13, the gate expected 14's verdict.
#   * So a gate never runs against "whatever `gcc` is on PATH", and never
#     against a newer GCC either: a later release may change a verdict, and the
#     pin keeps the comparison reproducible.
#
# Resolution, in order:
#   1. $GCC, if set. It must name an executable that reports major version 14,
#      otherwise the gate fails (an explicit choice is never replaced).
#   2. gcc-14 on PATH (the hosted workflow installs it explicitly).
#   3. gcc on PATH, only if it reports major version 14.
#
# Policy when no GCC 14 oracle is found: FAIL, locally and under CI. A gate that
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
#   GCC_ORACLE_BIN  absolute path of the GCC 14 oracle binary
#   GCC_ORACLE      always 1 after a successful require (kept for gates that
#                   branch on it; a missing oracle now fails instead of 0)
# shellcheck disable=SC2034  # both are consumed by the gates that source this file

GCC_ORACLE_MAJOR=14

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
    for cand in gcc-14 gcc; do
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
    echo "      Install gcc-14 (apt install gcc-14), or set GCC=<path to gcc-14>." >&2
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
