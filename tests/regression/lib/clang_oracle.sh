#!/usr/bin/env bash
# Shared Clang-oracle policy for the gates that compare lccc with Clang
# (`ext_vector_type` layout and by-value ABI: GCC has no such type, so Clang is
# the only reference implementation).
#
# The oracle is Clang 23.1.3, EXACTLY. The version is part of the contract:
#   * Clang's i386 convention for 8-byte vectors changed between releases
#     (Clang 19 returns an `ext_vector_type(2)` of `unsigned` in %eax:%edx,
#     Clang 23 in %xmm0), so "whatever `clang` is on PATH" is not a reference.
#     An Ubuntu runner's default `clang` is several releases older.
#   * Any other Clang is a configuration error, never a silent substitute, and
#     a missing Clang FAILS: a gate that skipped its cross-check would report a
#     comparison that never ran.
#
# Resolution, in order:
#   1. $CLANG, if set. It must report version 23.1.3, otherwise the gate fails
#      (an explicit choice is never replaced).
#   2. clang-23 on PATH, then clang on PATH, each only if it reports 23.1.3.
#
# CI installs the official LLVM 23.1.3 release (.github/workflows/ci.yml,
# "Install the Clang 23.1.3 oracle") and exports CLANG.
#
# Usage (from a gate script, run from the repository root):
#   . tests/regression/lib/clang_oracle.sh
#   require_clang_oracle "$gate_name" || exit 1
#   "$CLANG_ORACLE_BIN" -m64 -O2 -c fixture.c -o out.o
#
# Variable set by require_clang_oracle (read by the sourcing gate):
#   CLANG_ORACLE_BIN  absolute path of the Clang 23.1.3 oracle binary
# shellcheck disable=SC2034  # consumed by the gates that source this file

CLANG_ORACLE_VERSION=23.1.3

# clang_oracle_version <clang-binary> -> its full version, or nothing.
clang_oracle_version() {
    "$1" -dumpversion 2>/dev/null
}

require_clang_oracle() {
    local gate=$1 cand bin ver found=""
    CLANG_ORACLE_BIN=""
    if [ -n "${CLANG:-}" ]; then
        bin=$(command -v "$CLANG" 2>/dev/null || true)
        if [ -z "$bin" ]; then
            echo "FAIL: $gate: CLANG=$CLANG is not an executable; the oracle is Clang $CLANG_ORACLE_VERSION exactly" >&2
            return 1
        fi
        ver=$(clang_oracle_version "$bin")
        if [ "$ver" != "$CLANG_ORACLE_VERSION" ]; then
            echo "FAIL: $gate: CLANG=$CLANG is Clang ${ver:-?}; the oracle is Clang $CLANG_ORACLE_VERSION exactly" >&2
            return 1
        fi
        CLANG_ORACLE_BIN=$bin
        return 0
    fi
    for cand in clang-23 clang; do
        bin=$(command -v "$cand" 2>/dev/null || true)
        [ -n "$bin" ] || continue
        ver=$(clang_oracle_version "$bin")
        if [ "$ver" = "$CLANG_ORACLE_VERSION" ]; then
            CLANG_ORACLE_BIN=$bin
            return 0
        fi
        found="${found:+$found, }$cand=Clang ${ver:-?}"
    done
    echo "FAIL: $gate needs Clang $CLANG_ORACLE_VERSION as the oracle; found: ${found:-none}." >&2
    echo "      Install the official LLVM $CLANG_ORACLE_VERSION release, or set CLANG=<path to clang $CLANG_ORACLE_VERSION>." >&2
    return 1
}
