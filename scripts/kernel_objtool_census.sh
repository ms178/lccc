#!/usr/bin/env bash
# ============================================================================
# kernel_objtool_census.sh — count objtool warnings over exactly the objects
# Kbuild itself objtools, so the number is comparable between builds.
#
# WHY THIS EXISTS
# ---------------
# A naive `find … -name '*.o' | xargs objtool check` over a built kernel tree
# reports ~121 warnings on this tree and is meaningless: it includes host
# objects (`scripts/kconfig/*`, `scripts/mod/*`, `arch/x86/tools/relocs_*`,
# objtool's own build under `tools/`) that were compiled with `HOSTCC=gcc` and
# that Kbuild never objtools, plus `vmlinux.o`, the vDSO image objects, the
# decompressor and the realmode tree, plus re-run artefacts. Only the objects
# whose `.cmd` file records an objtool invocation are the set Kbuild checks, and
# that set is what a before/after comparison must use.
#
# SCOPE
# -----
# Every `.cmd` file containing the kernel-object spelling
# `; ./tools/objtool/objtool …`, with the flags exactly as recorded there (they
# vary: `--hacks=jump_label --hacks=noinstr --static-call --uaccess` and more).
# On Linux 6.18.63 with the VM config this is 1350 objects, plus 7 more under
# `arch/x86/boot/startup`, `arch/x86/entry/vdso` and `arch/x86/realmode` that
# this script measures and reports separately, for a comparable total of 1357.
#
# TRAPS THIS SCRIPT HANDLES
# -------------------------
# * An object that already carries `.static_call_sites` makes objtool print
#   `file already has .static_call_sites section, skipping` and report NO
#   warnings. Such an object would silently undercount, so skipped objects are
#   counted and listed; a warning total is only trustworthy if none of the
#   interesting objects is among them. Re-measure those by recompiling the TU
#   from its `savedcmd_` line first.
# * objtool's warning count IS idempotent (a known 2-warning object gives 2, 2,
#   2 across runs), so re-running over in-tree objects is safe.
#
# Usage:
#   kernel_objtool_census.sh [KERNEL_DIR] [--out PREFIX] [--verbose]
#
# Environment:
#   KERNEL_DIR   kernel tree to census (default: scripts/kernel_version.sh's
#                default tree)
# Exit status is 0 when the census ran, whatever it found; the counts are the
# output. Compare against artifacts/kernel-61863/OBJTOOL_CENSUS.md.
# ============================================================================
set -uo pipefail

here=$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
# shellcheck source=kernel_version.sh
. "$here/kernel_version.sh"

K=${KERNEL_DIR:-$(lccc_kernel_default_dir)}
OUT=/tmp/objtool-census
verbose=0
while [[ $# -gt 0 ]]; do
    case $1 in
        --out) [[ $# -ge 2 ]] || { echo "kernel_objtool_census: --out needs a value" >&2; exit 2; }
               OUT=$2; shift 2 ;;
        --verbose) verbose=1; shift ;;
        -h|--help) sed -n '3,/^# ====/p' "$0" | sed '$d'; exit 0 ;;
        --*) echo "kernel_objtool_census: unknown argument: $1" >&2; exit 2 ;;
        *) K=$1; shift ;;
    esac
done

[[ -d $K ]] || { echo "kernel_objtool_census: no such kernel tree: $K" >&2; exit 2; }
[[ -x $K/tools/objtool/objtool ]] || {
    echo "kernel_objtool_census: $K/tools/objtool/objtool is missing — build the kernel first" >&2
    exit 2
}

cd "$K" || exit 1

# Directories whose objects Kbuild objtools but which the main sweep skips, so
# that the sweep stays cheap; they are measured separately and added to the
# total. On 6.18.63 that is 7 objects contributing 0 warnings.
EXTRA_DIRS=(./arch/x86/boot ./arch/x86/entry/vdso ./arch/x86/realmode)

collect() {  # collect <outfile> <find-args...>
    local out=$1; shift
    : > "$out"
    local f cmd obj
    while IFS= read -r f; do
        case $f in ./tools/*) continue ;; esac
        cmd=$(grep -oE '; \./tools/objtool/objtool [^;]*' "$f" 2>/dev/null | head -1 | sed 's/^; //; s/[[:space:]]*$//')
        [[ -n $cmd ]] || continue
        obj=$(printf '%s\n' "$cmd" | awk '{print $NF}')
        [[ -f $obj ]] || continue
        printf '%s\t%s\n' "$obj" "$cmd" >> "$out"
    done < <(find "$@" -name '.*.o.cmd' 2>/dev/null | sort)
}

exclude_args=()
for d in "${EXTRA_DIRS[@]}"; do exclude_args+=(-not -path "$d/*"); done
collect "$OUT.main.cmds" . "${exclude_args[@]}" -not -path './tools/*'
collect "$OUT.extra.cmds" "${EXTRA_DIRS[@]}"

run_scope() {  # run_scope <cmds-file> <raw-out>
    local cmds=$1 raw=$2 obj cmd out rc
    : > "$raw"
    # Sequential on purpose: objtool costs ~0.1 s per object, so 1357 objects are
    # a couple of minutes, and a parallel runner would have to thread each
    # object path and its recorded command line through subshell quoting for no
    # useful gain.
    while IFS=$'\t' read -r obj cmd; do
        [[ -n $obj ]] || continue
        out=$(eval "$cmd" 2>&1); rc=$?
        if printf '%s\n' "$out" | grep -q 'already has .* section, skipping'; then
            printf 'SKIPPED\t%s\n' "$obj" >> "$raw"
            continue
        fi
        printf '%s\n' "$out" | grep -E 'warning:' | while IFS= read -r w; do
            printf 'WARN\t%s\t%s\n' "$obj" "$w" >> "$raw"
        done
        [[ $rc -ne 0 ]] && printf 'RC\t%s\trc=%s\n' "$obj" "$rc" >> "$raw"
        [[ $verbose == 1 ]] && printf '.' >&2
    done < "$cmds"
    [[ $verbose == 1 ]] && printf '\n' >&2
}

echo "kernel tree: $K"
echo "objtool:     $K/tools/objtool/objtool"
run_scope "$OUT.main.cmds"  "$OUT.main.raw"
run_scope "$OUT.extra.cmds" "$OUT.extra.raw"

# report <label> <cmds> <raw>: print one scope's counts and leave them in
# R_OBJS / R_WARNS / R_SKIP for the caller to total.
report() {
    local label=$1 cmds=$2 raw=$3 nonzero
    R_OBJS=$(wc -l < "$cmds")
    R_WARNS=$(grep -c '^WARN' "$raw" 2>/dev/null || true);    R_WARNS=${R_WARNS:-0}
    R_SKIP=$(grep -c '^SKIPPED' "$raw" 2>/dev/null || true);  R_SKIP=${R_SKIP:-0}
    nonzero=$(grep -c '^RC' "$raw" 2>/dev/null || true);      nonzero=${nonzero:-0}
    printf '%-28s objects=%-6s warnings=%-4s skipped=%-4s nonzero_rc=%s\n' \
        "$label" "$R_OBJS" "$R_WARNS" "$R_SKIP" "$nonzero"
}
report "Kbuild scope (main sweep)" "$OUT.main.cmds" "$OUT.main.raw"
m_o=$R_OBJS; m_w=$R_WARNS; m_s=$R_SKIP
report "boot/vdso/realmode"        "$OUT.extra.cmds" "$OUT.extra.raw"
e_o=$R_OBJS; e_w=$R_WARNS; e_s=$R_SKIP
printf '%-28s objects=%-6s warnings=%-4s skipped=%-4s\n' "COMPARABLE TOTAL" \
    "$((m_o + e_o))" "$((m_w + e_w))" "$((m_s + e_s))"

echo
echo "---- warnings, verbatim ----"
cat "$OUT.main.raw" "$OUT.extra.raw" | grep '^WARN' | cut -f3- | sort || echo "(none)"
echo
echo "---- warnings, grouped by message ----"
cat "$OUT.main.raw" "$OUT.extra.raw" | grep '^WARN' | cut -f3- \
  | sed -E 's/^.*warning: objtool: //; s/\+0x[0-9a-f]+/+0xN/' | sort | uniq -c | sort -rn
if [[ $((m_s + e_s)) -gt 0 ]]; then
    echo
    echo "NOTE: $((m_s + e_s)) object(s) were skipped as already processed and report"
    echo "no warnings at all; the total above is only complete if none of them would"
    echo "have warned. Recompiling those TUs from their savedcmd_ line re-measures them."
    cat "$OUT.main.raw" "$OUT.extra.raw" | grep '^SKIPPED' | cut -f2 | sed 's/^/  /'
fi
