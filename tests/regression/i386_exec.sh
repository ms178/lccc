#!/usr/bin/env bash
# tests/regression/i386_exec.sh — shared host i386-execution probe.
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
# unavailable_i386_interpreter and SIGSYS skips are the model; so are the
# inline probes in check_copy_alias_sizes.sh / check_linker_notype_code.sh
# / check_nocfi_peephole_parity.sh, which predate this helper).
#
# i386_exec_ok <compiler> <scratch-dir>
#   Builds and runs a trivial i386 program with <compiler> (the host gcc,
#   or the compiler under test). Returns 0 when the program runs to
#   completion; non-zero when the link or the execution is impossible on
#   this host. The probe binary lands in <scratch-dir>.
i386_exec_ok() { # $1 = compiler command, $2 = scratch dir
    local cc=$1 dir=$2 rc=0
    echo 'int main(void){return 0;}' \
        | "$cc" -m32 -x c -o "$dir/i386-exec-probe" - >/dev/null 2>&1 || return 1
    "$dir/i386-exec-probe" >/dev/null 2>&1 || rc=$?
    [[ $rc -eq 0 ]]
}
