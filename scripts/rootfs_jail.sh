#!/usr/bin/env bash
# Privilege-dropping jail for provisioning commands: the live root filesystem
# becomes structurally unwritable, and `sudo` stops working, for the payload.
#
# Why this exists (measured, not assumed)
# -------------------------------------
# Three separate agent sessions destroyed the sandbox by running
#
#     sudo dpkg-deb -x libc6-i386_<bookworm>.deb /
#
# on a usr-merged Debian root.  The .deb carries `./lib/ld-linux.so.2`, so
# dpkg-deb replaces the `/lib -> usr/lib` *symlink* with a real directory that
# contains only the 32-bit loader.  `/lib/x86_64-linux-gnu/libc.so.6` disappears
# from every ld.so cache path, and the next `execve` of a dynamic binary -- including
# the harness's own `/bin/sh` -- fails with ENOENT.  The session is unrecoverable
# without a harness reset.  Prose warnings in three documents did not prevent it.
#
# What this jail does instead
# ---------------------------
# `unshare --user --map-root-user` runs the payload in a fresh user namespace
# whose uid 0 maps to the *caller's unprivileged uid*.  Measured on the Arena
# host (Debian 13, kernel unprivileged userns enabled):
#
#   * `id -u` inside is 0, but the kernel's capable() checks use the outer uid,
#     so root-owned paths are not writable: `touch /canary` -> EACCES;
#   * `sudo -n true` -> exit 1 (`/etc/sudo.conf` is owned by the unmapped
#     overflow uid), so the payload cannot escalate back to real root;
#   * the historical catastrophe command `dpkg-deb -x libc6-i386… /` exits 2
#     and `/lib` is still a symlink afterwards;
#   * caller-writable trees (the workspace, /var/tmp) stay writable, so staging
#     an isolated sysroot inside the jail keeps full functionality.
#
# The jail is therefore not a warning: it removes the capability.
#
# Why no `--mount` and no `--fork` (measured, 20 invocations each):
#
#   --user --map-root-user --mount --fork   19.39 ms/run
#   --user --map-root-user --fork           17.16 ms/run
#   --user --map-root-user                   8.63 ms/run
#
# The protection comes entirely from the user namespace: with neither flag the
# uid_map is still `0 <caller-uid> 1` and `: > /canary` still fails with EACCES.
# Dropping them is 2.25x faster, removes the mount-namespace capability
# requirement (so *more* hosts get an effective jail, not fewer), and lets
# unshare `exec` the payload directly instead of staying alive as a forwarding
# parent -- one process fewer and native signal semantics.
#
# Contract
# --------
#   rootfs_jail.sh --self-test          offline capability + refusal proofs
#   rootfs_jail.sh --available          exit 0 iff an effective jail can be built
#   rootfs_jail.sh -- exec <argv...>    run argv inside the jail (fail closed)
#
# `exec` NEVER falls back to unprotected execution.  If the kernel refuses
# unprivileged user namespaces (some hardened CI images do), it exits 3 with an
# actionable message; the caller decides whether it has an independently safe
# path (e.g. an already path-validated sysroot extraction).
#
# A payload that itself exits 3 is indistinguishable from a refusal by exit
# status alone.  Callers that must tell the two apart ask `jail_available`
# first, which is exactly what the provisioning script does; `jail_run` also
# re-probes on failure so an unshare-level error is always reported as 3 rather
# than as a payload status.
#
# There is deliberately no override variable.  A jail that can be switched off
# by an environment variable is a jail that gets switched off.

set -euo pipefail

log() { printf '[rootfs-jail] %s\n' "$*"; }
die() { printf '[rootfs-jail] ERROR: %s\n' "$*" >&2; exit 1; }

# Exit codes are part of the contract: 1 = usage/payload failure,
# 3 = jail unavailable or ineffective (never silently degraded).
JAIL_UNAVAILABLE=3

have_unshare() { command -v unshare >/dev/null 2>&1; }

# The in-jail preflight.  Runs as the mapped uid 0 and proves three properties
# before the payload is allowed to start.  Any failure aborts the whole jail.
#
# $1 is a root-owned canary path whose *parent* is root-owned and not writable
# by the outer uid (default: /).  The probe only ever attempts to create a
# dot-file there; on a correctly jailed host that attempt fails, and on a
# misconfigured one the file is removed again immediately.
# The in-jail guard.  It runs *in the same namespace as the payload* and then
# `exec`s it, so the properties it proves are the properties the payload gets.
# Verifying in one namespace and then opening a second one for the payload
# would prove nothing about the process that actually runs.
#
# $1 is a root-owned canary path whose *parent* is root-owned and not writable
# by the outer uid (default: /); the remaining arguments are the payload argv.
# The probe only ever attempts to create a dot-file there, inside a subshell:
# a failed redirection on the `:` special builtin terminates a non-interactive
# POSIX shell (measured on dash), so the attempt must not run in the guard
# itself or a correct refusal would kill the guard before it could report.
JAIL_GUARD='
set -eu
parent=$1
shift
if [ "$#" -eq 0 ]; then
    printf "[rootfs-jail] no payload given\n" >&2
    exit 3
fi
# Exactly one mapping line, "0 <outer> 1".  A multi-line map means the namespace
# grants more than the caller identity, and a count other than 1 means the same.
# `read` is used instead of `set --` because the payload argv must survive.
map=$(cat /proc/self/uid_map)
newline="
"
case $map in
    *"$newline"*)
        printf "[rootfs-jail] uid_map has more than one mapping: %s\n" "$map" >&2
        exit 3
        ;;
esac
read -r inner outer count <<EOF_UID_MAP
$map
EOF_UID_MAP
if [ "$inner" != "0" ] || [ "$count" != "1" ] || [ -z "$outer" ]; then
    printf "[rootfs-jail] uid_map is not a single self-mapping: %s\n" "$map" >&2
    exit 3
fi
# A canary parent that does not exist would make the write fail for the wrong
# reason and the guard would certify a jail it never actually tested.
if [ ! -d "$parent" ]; then
    printf "[rootfs-jail] canary parent %s is not a directory\n" "$parent" >&2
    exit 3
fi
canary="$parent/.lccc-rootfs-jail-canary.$$"
if (set -C; : > "$canary") 2>/dev/null; then
    rm -f "$canary"
    printf "[rootfs-jail] root-owned %s is writable inside the jail\n" "$parent" >&2
    exit 3
fi
if command -v sudo >/dev/null 2>&1 && sudo -n true 2>/dev/null; then
    printf "[rootfs-jail] sudo still escalates inside the jail\n" >&2
    exit 3
fi
exec "$@"
'

# Build one namespace, prove the properties inside it, and exec the payload
# there.  The argv is passed as argv, never through a shell string, so quoting
# cannot smuggle a second command past the guard.
jail_run() {
    if [[ ${1:-} != -- ]]; then
        die "jail_run: expected -- before the payload argv"
    fi
    shift
    [[ $# -gt 0 ]] || die "jail_run: no payload given"
    have_unshare || {
        log "unshare(1) is missing; refusing to run the payload unprotected"
        return "$JAIL_UNAVAILABLE"
    }
    local canary_parent=${LCCC_JAIL_CANARY_PARENT:-/}
    local status=0
    unshare --user --map-root-user \
        /bin/sh -c "$JAIL_GUARD" lccc-jail "$canary_parent" "$@" || status=$?
    if [[ $status -ne 0 && $status -ne $JAIL_UNAVAILABLE ]]; then
        # Only on a failure: distinguish "the payload failed" from "the jail
        # could not be built".  The happy path pays nothing for this.
        if ! jail_available; then
            log "user namespace creation failed (status $status); payload was NOT started"
            return "$JAIL_UNAVAILABLE"
        fi
    elif [[ $status -eq $JAIL_UNAVAILABLE ]]; then
        log "jail guard rejected this host; payload was NOT started"
    fi
    return "$status"
}

# `--available` answers the question a caller needs *before* it commits to a
# provisioning strategy.  It runs the real guard with a no-op payload, so there
# is exactly one implementation of the protection test.  As a predicate it
# returns 0/non-0; the CLI maps non-0 to JAIL_UNAVAILABLE.
jail_available() {
    have_unshare || return 1
    local canary_parent=${LCCC_JAIL_CANARY_PARENT:-/}
    unshare --user --map-root-user \
        /bin/sh -c "$JAIL_GUARD" lccc-jail "$canary_parent" /bin/true >/dev/null 2>&1
}

self_test() {
    local tmp
    tmp=$(mktemp -d "${TMPDIR:-/var/tmp}/lccc-jail-selftest.XXXXXX")
    # Interpolate now and trap EXIT, not RETURN: a RETURN trap fires after the
    # function's locals are gone (a deferred "$tmp" would trip `set -u`), and it
    # does not fire at all when `die` exits, which would leak the directory on
    # the very path where a clean tree matters most.
    trap "rm -rf '$tmp'" EXIT

    have_unshare || die "self-test requires unshare(1)"

    # 1. A caller-writable directory can host the canary parent, so the same
    #    guard logic is exercised without depending on / being root-owned.
    mkdir -p "$tmp/writable"
    if LCCC_JAIL_CANARY_PARENT="$tmp/writable" jail_available; then
        die "guard accepted a writable canary parent (guard has no teeth)"
    fi
    log "PASS: guard rejects a jail whose canary parent is writable"

    # 2. A canary parent that does not exist must be refused, not treated as a
    #    successful write refusal.
    if LCCC_JAIL_CANARY_PARENT="$tmp/absent" jail_available; then
        die "guard accepted a non-existent canary parent (fail-open)"
    fi
    log "PASS: guard rejects a non-existent canary parent"

    # 3. The real host guard must succeed here, otherwise the remaining proofs
    #    cannot run and the caller must know the jail is unavailable.
    if ! jail_available; then
        log "SKIP: this host cannot build an effective jail (unprivileged userns disabled)"
        return "$JAIL_UNAVAILABLE"
    fi
    log "PASS: effective jail available (single self uid_map, root unwritable, sudo dead)"

    # 4. Root-owned write attempts fail inside the jail.
    if jail_run -- /bin/sh -c ': > /.lccc-jail-selftest-should-not-exist' 2>/dev/null; then
        rm -f /.lccc-jail-selftest-should-not-exist
        die "jail allowed a write to / (guard has no teeth)"
    fi
    [[ ! -e /.lccc-jail-selftest-should-not-exist ]] || die "jail left a file at /"
    log "PASS: writes to / are refused inside the jail"

    # 5. Caller-writable trees keep working: no functionality is lost.
    jail_run -- /bin/sh -c "printf ok > '$tmp/writable/proof'"
    [[ $(cat "$tmp/writable/proof") == ok ]] || die "jail broke writes to a caller-owned tree"
    log "PASS: caller-owned trees stay writable inside the jail"

    # 6. The payload runs in the guarded namespace, not a sibling of it: its
    #    own uid_map is the single self-mapping the guard checked.
    local payload_map
    payload_map=$(jail_run -- /bin/sh -c 'cat /proc/self/uid_map')
    # Command substitution strips the trailing newline, so the single-line
    # property is "contains no newline", not "wc -l == 1".
    [[ $payload_map != *$'\n'* ]] || die "payload uid_map is multi-line: $payload_map"
    [[ $(printf '%s' "$payload_map" | awk '{print $1, $3}') == "0 1" ]] \
        || die "payload is not in the guarded mapping: $payload_map"
    log "PASS: the payload itself runs inside the guarded namespace"

    # 7. The payload's exit status and argv survive the guard unchanged.
    local status=0
    jail_run -- /bin/sh -c 'exit 7' || status=$?
    [[ $status -eq 7 ]] || die "payload exit 7 became $status"
    jail_run -- /bin/sh -c 'printf "%s" "$1" > "$2"' _ 'a b;touch injected' "$tmp/writable/argv"
    [[ $(cat "$tmp/writable/argv") == 'a b;touch injected' ]] || die "payload argv was re-interpreted"
    [[ ! -e injected ]] || die "payload argv injection created a file"
    log "PASS: payload exit status and argv pass through untouched"

    # 8. sudo cannot escalate.
    if command -v sudo >/dev/null 2>&1; then
        if jail_run -- /bin/sh -c 'sudo -n true' >/dev/null 2>&1; then
            die "jail allowed sudo escalation"
        fi
        log "PASS: sudo escalation is refused inside the jail"
    fi

    # 9. Fail closed when unshare is unavailable: an empty PATH must produce
    #    JAIL_UNAVAILABLE and must NOT run the payload.
    status=0
    PATH="$tmp/empty" jail_run -- /bin/sh -c "printf ran > '$tmp/writable/ran'" || status=$?
    [[ $status -eq $JAIL_UNAVAILABLE ]] || die "missing unshare returned $status, expected $JAIL_UNAVAILABLE"
    [[ ! -e "$tmp/writable/ran" ]] || die "payload ran without a jail"
    log "PASS: missing unshare fails closed without running the payload"

    rm -rf "$tmp"
    trap - EXIT
    log "self-test PASSED"
}

usage() {
    sed -n '2,60p' "$0" | sed 's/^# \{0,1\}//'
}

main() {
    case "${1:-}" in
        --self-test) self_test ;;
        --available)
            if jail_available; then
                log "effective privilege-dropping jail is available"
            else
                log "no effective jail on this host"
                return "$JAIL_UNAVAILABLE"
            fi
            ;;
        --)
            shift
            [[ ${1:-} == exec ]] || die "expected: rootfs_jail.sh -- exec <argv...>"
            shift
            [[ $# -gt 0 ]] || die "no payload given"
            jail_run -- "$@"
            ;;
        -h|--help) usage ;;
        *) die "unknown option: ${1:-} (see --help)" ;;
    esac
}

# Sourcing exposes jail_run/jail_available to the provisioning scripts without
# running main, so there is exactly one implementation of the guard.
if [[ ${BASH_SOURCE[0]} == "$0" ]]; then
    main "$@"
fi
