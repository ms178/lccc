#!/usr/bin/env bash
# Shared GDB-oracle policy for the gates that cross-check LCCC's DWARF with a
# real debugger (`tests/regression/check_dwarf_line_tables.py`, and the failure
# forensics in `tests/regression/check_incoming_byval_homes.sh`).
#
# The oracle is GNU gdb 18.1, EXACTLY, as installed by
# `scripts/ci_install_gdb_oracle.sh` (SHA-256-pinned source, -march=haswell).
# GDB's line-table and DWARF-form handling moves between releases, so
# "whatever gdb is on PATH" is not a reference implementation. Before this
# policy existed both consumers resolved gdb opportunistically and returned
# success when it was absent: a green gate that never asked a debugger
# anything. This library makes that failure mode impossible by default.
#
# Resolution, in order:
#   1. $GDB, if set. It must report exactly "GNU gdb (GDB) 18.1", otherwise
#      the gate FAILS -- an explicitly selected debugger is never replaced by
#      a substitute, and never downgraded to a skip.
#   2. `gdb` on PATH, only if it reports exactly the same banner.
#
# A debugger that is absent, or present only at another version, is a FAILURE
# (status 1), not a pass. Hosts that genuinely cannot provide the pin may opt
# into an explicit skip with --allow-missing-debugger; the skip is announced
# on stderr and returned as status 3, so a green run cannot hide it.
#
# Usage from a shell gate (sourceable; no argv parsing or exit when sourced):
#   . tests/regression/lib/gdb_oracle.sh
#   gdb_rc=0
#   require_gdb_oracle check_my_gate --allow-missing-debugger || gdb_rc=$?
#   case $gdb_rc in 0) "$GDB_ORACLE_BIN" ... ;; 3) : explicit skip ;; *) exit 1 ;; esac
#
# Usage from a Python gate (executable CLI, one shared policy implementation):
#   tests/regression/lib/gdb_oracle.sh --print [--allow-missing-debugger]
#   status 0 -> absolute path on stdout; 3 -> explicit skip; 1 -> hard failure.
#
# Variable set by require_gdb_oracle:
#   GDB_ORACLE_BIN  absolute path of the pinned debugger; empty on skip/failure.
# shellcheck disable=SC2034  # consumed by the gates that source this file

GDB_ORACLE_VERSION=18.1
GDB_ORACLE_BANNER="GNU gdb (GDB) $GDB_ORACLE_VERSION"
GDB_ORACLE_BIN=""

# First line of `<gdb> --version`. Command substitution plus parameter
# expansion avoids a `head` subprocess and the SIGPIPE / pipefail interaction
# that a truncated pipeline creates under `set -o pipefail` (a long GDB
# copyright banner would otherwise kill the producer with SIGPIPE).
gdb_oracle_banner() {
    local out
    out=$("$1" --version 2>/dev/null) || return 1
    printf '%s\n' "${out%%$'\n'*}"
}

# gdb_oracle_resolve <gate> <allow-missing> -> 0 ok / 3 explicit skip / 1 fail.
# Stdout stays clean so the CLI's --print output is machine-readable; every
# diagnostic goes to stderr.
gdb_oracle_resolve() {
    local gate=$1 allow_missing=$2 bin banner found
    GDB_ORACLE_BIN=""
    if [ -n "${GDB:-}" ]; then
        bin=$(command -v -- "$GDB" 2>/dev/null) || bin=""
        if [ -z "$bin" ] || [ ! -x "$bin" ]; then
            printf 'FAIL: %s: GDB=%s is not an executable; the oracle is %s\n' \
                "$gate" "$GDB" "$GDB_ORACLE_BANNER" >&2
            printf '      Provision it with scripts/ci_install_gdb_oracle.sh.\n' >&2
            return 1
        fi
        banner=$(gdb_oracle_banner "$bin") || banner=""
        if [ "$banner" != "$GDB_ORACLE_BANNER" ]; then
            printf 'FAIL: %s: GDB=%s reports "%s"; the oracle is %s exactly.\n' \
                "$gate" "$GDB" "${banner:-?}" "$GDB_ORACLE_BANNER" >&2
            printf '      An explicitly selected debugger is never substituted.\n' >&2
            return 1
        fi
        GDB_ORACLE_BIN=$bin
        return 0
    fi
    bin=$(command -v -- gdb 2>/dev/null) || bin=""
    if [ -n "$bin" ] && [ -x "$bin" ]; then
        banner=$(gdb_oracle_banner "$bin") || banner=""
        if [ "$banner" = "$GDB_ORACLE_BANNER" ]; then
            GDB_ORACLE_BIN=$bin
            return 0
        fi
        found="gdb=${banner:-?} (unpinned; not a substitute)"
    else
        found="no gdb on PATH"
    fi
    if [ "$allow_missing" = 1 ]; then
        printf 'SKIP: %s: %s is not the pinned %s; --allow-missing-debugger was passed explicitly, so the debugger cross-check will NOT run.\n' \
            "$gate" "$found" "$GDB_ORACLE_BANNER" >&2
        return 3
    fi
    printf 'FAIL: %s needs %s as the debugger oracle; found: %s.\n' \
        "$gate" "$GDB_ORACLE_BANNER" "$found" >&2
    printf '      Install it with scripts/ci_install_gdb_oracle.sh, or set GDB=<path to gdb %s>.\n' \
        "$GDB_ORACLE_VERSION" >&2
    printf '      Pass --allow-missing-debugger only when the cross-check is intentionally skipped.\n' >&2
    return 1
}

# require_gdb_oracle <gate> [--allow-missing-debugger]
require_gdb_oracle() {
    local gate=${1:?require_gdb_oracle needs a gate name} allow_missing=0 arg
    shift
    for arg in "$@"; do
        case $arg in
            --allow-missing-debugger) allow_missing=1 ;;
            *) printf 'gdb-oracle: unknown option: %s\n' "$arg" >&2; return 1 ;;
        esac
    done
    gdb_oracle_resolve "$gate" "$allow_missing"
}

# CLI mode only when executed directly; sourcing must never parse argv or exit.
if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
    _gate=check_dwarf_line_tables
    _allow=0
    _print=0
    _opts=()
    while [ $# -gt 0 ]; do
        case $1 in
            --print) _print=1; shift ;;
            --allow-missing-debugger) _allow=1; shift ;;
            --gate) _gate=${2:?--gate requires a name}; shift 2 ;;
            -h|--help) sed -n '2,40p' "$0"; exit 0 ;;
            *) printf 'gdb-oracle: unknown argument: %s\n' "$1" >&2; exit 2 ;;
        esac
    done
    if [ "$_print" != 1 ]; then
        printf 'gdb-oracle: CLI requires --print (or source this library)\n' >&2
        exit 2
    fi
    if [ "$_allow" = 1 ]; then
        _opts+=(--allow-missing-debugger)
    fi
    require_gdb_oracle "$_gate" "${_opts[@]}"
    _rc=$?
    if [ "$_rc" = 0 ]; then
        printf '%s\n' "$GDB_ORACLE_BIN"
    fi
    exit "$_rc"
fi
