#!/usr/bin/env bash
# tests/regression/i386_exec.sh — shared host i386 capability probe.
#
# Source, don't execute:
#   source "$(dirname "$0")/i386_exec.sh"
#   if i386_exec_ok gcc "$tmp"; then ... run 32-bit binaries ... fi
#
# A host can link -m32 yet refuse to execute the result: no
# /lib/ld-linux.so.2 (ENOENT at exec), or a seccomp policy that SIGSYSes
# the ia32 syscall gateway (rc 159) while -m32 links fine through a
# sysroot. Gates that RUN 32-bit binaries must probe the exact capability
# the run leg needs — link success is not evidence the run can work, and
# a sandboxed host must skip the leg instead of reporting a codegen or
# linker failure that never happened (run_regression.py's
# unavailable_i386_interpreter and SIGSYS skips are the model).
#
# The probe measures HOST capability, never the compiler under test: a
# probe driven by the artifact under test turns that artifact's own
# regression into a silent SKIP of the leg the gate exists to guard
# (check_nocfi_peephole_parity.sh, pre-fix). Call it with the host gcc.
#
# One probe, three levels, memoized per compiler:
#   i386_capability <compiler>          prints run | link | none
#     run  — compiled, linked AND executed: 32-bit run legs may go ahead
#     link — compiled and linked, execution refused on this host: assert
#            link-level laws, skip the run legs
#     none — the -m32 link itself is impossible: skip the mode entirely
#   i386_exec_ok <compiler> <scratch-dir>   — capability == run
#   i386_link_ok <compiler>                — capability != none
# The richer 3-level model lives here once; the two boolean helpers are
# thin wrappers, and gates stop carrying private probe copies. Memoizing
# per compiler: the answer is a property of the host+compiler pair, so
# repeated calls (comdat's mode loop, eh_frame's link re-probe) build and
# run one probe binary per process, not one per call.
declare -A _I386_CAP_MEMO=()
# Populate the memo IN THE CURRENT SHELL. Every $(...)-free helper below
# calls this first; assignments made inside a $( ) command substitution
# would land in the forked child and be discarded (the original
# implementation memoized exactly there, so the parent array stayed
# empty and every call re-probed -- mktemp, gcc -m32 and all).
# Gates that capture the value with i386_cap=$(i386_capability "$GCC")
# must run _i386_ensure_memo "$GCC" FIRST, or the capture re-probes once
# in its own subshell.
_i386_ensure_memo() { # $1 = compiler command
    local cc=$1
    [[ -n "${_I386_CAP_MEMO[$cc]+x}" ]] && return 0
    local cap=none rc=0 dir
    dir=$(mktemp -d "${TMPDIR:-/tmp}/i386-cap-probe.XXXXXX")
    if echo 'int main(void){return 0;}' \
        | "$cc" -m32 -x c -o "$dir/probe" - >/dev/null 2>&1; then
        cap=link
        "$dir/probe" >/dev/null 2>&1 || rc=$?
        if [[ $rc -eq 0 ]]; then cap=run; fi
    fi
    rm -rf "$dir"
    _I386_CAP_MEMO[$cc]=$cap
}
i386_capability() { # $1 = compiler command; prints run | link | none
    _i386_ensure_memo "$1"
    printf '%s\n' "${_I386_CAP_MEMO[$1]}"
}
i386_exec_ok() { # $1 = compiler command, $2 = scratch dir (legacy param: the
    # probe manages its own scratch; the signature is kept for the four
    # existing call sites, all of which pass their gate tmp dir.)
    _i386_ensure_memo "$1"
    [[ "${_I386_CAP_MEMO[$1]}" == run ]]
}
i386_link_ok() { # $1 = compiler command: the -m32 LINK works (exec may not)
    _i386_ensure_memo "$1"
    [[ "${_I386_CAP_MEMO[$1]}" != none ]]
}
