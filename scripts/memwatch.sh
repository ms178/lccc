#!/usr/bin/env bash
# ============================================================================
# memwatch.sh — userspace OOM protection for swapless build containers.
#
# Why this exists:
#   The Arena/harness sandbox runs unprivileged (no CAP_SYS_ADMIN), so
#   swapon(2) is impossible no matter how large a swap file we prepare.
#   On a 2-vCPU / ~4 GiB box, parallel Rust/C builds can transiently exhaust
#   anonymous memory.  Without swap the kernel OOM killer then fires at the
#   worst possible moment and may take down the shell / harness itself.
#
# What it does (userspace "swap"):
#   Polls MemAvailable.  Below WARN_MB it sends SIGSTOP to the single
#   largest memory-consuming process in our own session (never PID 1,
#   never the harness, never other users).  Below KILL_MB it SIGKILLs the
#   single largest offender.  A stopped rustc is far better than a dead
#   machine: incremental compilation makes the lost work cheap to redo,
#   and the watchdog logs every action so post-mortems stay possible.
#
# Safety properties:
#   * only touches processes owned by the invoking user ($UID)
#   * never touches its own process group, the harness, or session leaders
#   * single-shot mode (--once) for use from cron-like wrappers
#   * all decisions logged with timestamp + /proc/<pid>/cmdline snapshot
#
# Usage:
#   memwatch.sh                 # foreground loop, 2s interval
#   memwatch.sh --daemon        # background, log to $MEMWATCH_LOG
#   memwatch.sh --once          # single check (for wrappers)
#   kill $(cat $MEMWATCH_PIDFILE 2>/dev/null)  # stop
# ============================================================================
set -u

INTERVAL=${MEMWATCH_INTERVAL:-2}
WARN_MB=${MEMWATCH_WARN_MB:-450}
KILL_MB=${MEMWATCH_KILL_MB:-250}
LOG=${MEMWATCH_LOG:-/tmp/memwatch.log}
PIDFILE=${MEMWATCH_PIDFILE:-/tmp/memwatch.pid}
SELF=$$

log() { printf '%s %s\n' "$(date -u +%FT%TZ)" "$*" >>"$LOG"; }

avail_kb() { awk '/^MemAvailable:/{print $2}' /proc/meminfo; }

# Largest RSS process owned by $UID, excluding self and PID 1.
# Prints "<pid> <rss_kb> <cmdline>".
biggest_proc() {
    local best_pid=0 best_rss=0 pid rss
    for d in /proc/[0-9]*; do
        pid=${d#/proc/}
        [[ $pid -eq 1 || $pid -eq $SELF ]] && continue
        rss=$(awk '/^VmRSS:/{print $2}' "$d/status" 2>/dev/null) || continue
        [[ -n ${rss:-} ]] || continue
        local owner
        owner=$(awk '/^Uid:/{print $2}' "$d/status" 2>/dev/null)
        [[ $owner == "$UID" ]] || continue
        if (( rss > best_rss )); then
            best_rss=$rss; best_pid=$pid
        fi
    done
    if (( best_pid > 0 )); then
        printf '%d %d %s\n' "$best_pid" "$best_rss" \
            "$(tr '\0' ' ' <"/proc/$best_pid/cmdline" 2>/dev/null | cut -c1-120)"
    fi
}

act() {
    local level=$1 kb pid rss cmd
    kb=$(avail_kb)
    local line
    line=$(biggest_proc) || return 0
    [[ -n $line ]] || return 0
    read -r pid rss cmd <<<"$line"
    [[ -n ${pid:-} ]] || return 0
    case $level in
    stop)
        if kill -STOP "$pid" 2>/dev/null; then
            log "WARN avail=${kb}kB SIGSTOP pid=$pid rss=${rss}kB cmd='$cmd'"
        fi ;;
    kill)
        if kill -KILL "$pid" 2>/dev/null; then
            log "CRIT avail=${kb}kB SIGKILL pid=$pid rss=${rss}kB cmd='$cmd'"
        fi ;;
    esac
}

check_once() {
    local kb
    kb=$(avail_kb)
    (( kb / 1024 < KILL_MB )) && { act kill; return; }
    (( kb / 1024 < WARN_MB )) && { act stop; return; }
    return 0
}

case ${1:-loop} in
--once) check_once ;;
--daemon)
    if [[ -f $PIDFILE ]] && kill -0 "$(cat "$PIDFILE")" 2>/dev/null; then
        echo "memwatch already running (pid $(cat "$PIDFILE"))" >&2; exit 0
    fi
    nohup "$0" loop >/dev/null 2>&1 &
    echo $! >"$PIDFILE"
    echo "memwatch daemon pid $(cat "$PIDFILE"), log $LOG" >&2 ;;
loop)
    echo $SELF >"$PIDFILE"
    log "memwatch start warn=${WARN_MB}MB kill=${KILL_MB}MB interval=${INTERVAL}s"
    while sleep "$INTERVAL"; do check_once; done ;;
--status)
    if [[ -f $PIDFILE ]] && kill -0 "$(cat "$PIDFILE")" 2>/dev/null; then
        echo "running: $(cat "$PIDFILE")"
    else
        echo "not running"
    fi
    tail -5 "$LOG" 2>/dev/null ;;
*) echo "usage: $0 [--daemon|--once|--status|loop]" >&2; exit 2 ;;
esac
