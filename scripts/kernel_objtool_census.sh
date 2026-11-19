#!/usr/bin/env bash
# Count objtool warnings for the objects Kbuild actually checks. Read the
# savedcmd_ record in each .*.o.cmd, not every *.o in the tree: most host tools
# and link products have different validation contracts. Reuse the recorded
# flags, with --dry-run so the census cannot rewrite the measured objects.
#
# Usage: kernel_objtool_census.sh [KERNEL_DIR] [--out PREFIX] [--verbose]
# KERNEL_DIR defaults to the configured kernel work tree. Without --out the
# report files get a private temporary directory. Files: .{main,extra}.{cmds,raw}.
# A complete census exits 0 even if it finds warnings. A missing object,
# malformed record, failed objtool invocation, already-processed object or
# empty census makes the counts incomplete and exits nonzero.
set -euo pipefail

here=$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
# shellcheck source=kernel_version.sh
. "$here/kernel_version.sh"

K=${KERNEL_DIR:-$(lccc_kernel_default_dir)}
OUT=
verbose=0
while (($#)); do
    case $1 in
        --out) [[ $# -ge 2 && -n $2 ]] || { echo 'kernel_objtool_census: --out needs a value' >&2; exit 2; }
               OUT=$2; shift 2 ;;
        --verbose) verbose=1; shift ;;
        -h|--help) sed -n '2,/^set -/p' "$0" | sed '$d'; exit 0 ;;
        --*) echo "kernel_objtool_census: unknown argument: $1" >&2; exit 2 ;;
        *) K=$1; shift ;;
    esac
done

objtool=./tools/objtool/objtool
[[ -d $K ]] || { echo "kernel_objtool_census: no such kernel tree: $K" >&2; exit 2; }
[[ -x $K/$objtool ]] || { echo "kernel_objtool_census: missing objtool in $K" >&2; exit 2; }
if [[ -z $OUT ]]; then
    OUT=$(mktemp -d "${TMPDIR:-/tmp}/lccc-objtool-census.XXXXXX")/census
else
    OUT=$(realpath -m -- "$OUT")
    mkdir -p -- "$(dirname -- "$OUT")"
fi
out_prefix=$OUT
# Objtool parses OBJTOOL_ARGS as options before argv. Replay Kbuild flags only.
unset OBJTOOL_ARGS OBJTOOL_VERBOSE
records=$(mktemp "${TMPDIR:-/tmp}/lccc-objtool-records.XXXXXX")
trap 'rm -f -- "$records"' EXIT
cd -- "$K"
# Capture find's status; process substitution would hide an unreadable tree.
find . -path ./tools -prune -o -type f -name '.*.o.cmd' -print0 | LC_ALL=C sort -z >"$records"
for scope in main extra; do
    : >"$out_prefix.$scope.cmds"
    : >"$out_prefix.$scope.raw"
done

# Kbuild 6.18 records this command after a semicolon, followed by flags of
# the form --flag or --flag=value and the build-relative object path. Reject
# other forms rather than shell-evaluating a recorded command or guessing its
# argument boundaries. The object must also match the .cmd filename itself.
marker="; $objtool "
while IFS= read -r -d '' cmd_path; do
    scope=main
    case $cmd_path in
        ./arch/x86/boot/*|./arch/x86/entry/vdso/*|./arch/x86/realmode/*) scope=extra ;;
    esac
    raw=$out_prefix.$scope.raw
    cmds=$out_prefix.$scope.cmds
    # Read only the first line: Kbuild writes savedcmd_ before dependency data.
    IFS= read -r saved <"$cmd_path" || :
    [[ $saved == savedcmd_*' := '* && $saved == *"$marker"* ]] || continue
    obj_name=${cmd_path##*/}
    obj=${cmd_path%/*}/${obj_name:1:${#obj_name}-5}
    obj=${obj#./}
    target=${saved#savedcmd_}
    target=${target%%' := '*}
    command=${saved#*"$marker"}
    read -r -a argv <<< "${command%%;*}"
    if ((${#argv[@]} == 0)) || [[ ${target#./} != "$obj" || ${argv[-1]#./} != "$obj" ||
                                  ! $obj =~ ^[A-Za-z0-9_./+-]+\.o$ ]]; then
        printf 'ERROR\t%s\tinvalid saved objtool target\n' "$cmd_path" >>"$raw"
        continue
    fi
    bad_flag=
    for flag in "${argv[@]:0:${#argv[@]}-1}"; do
        case $flag in
            --hacks=jump_label|--hacks=noinstr|--hacks=skylake|--ibt|--cfi|--mcount|--mnop|--orc|\
            --retpoline|--rethunk|--sls|--stackval|--static-call|--uaccess|--no-unreachable|--Werror|\
            --link|--module) continue ;;
            --prefix=*) [[ ${flag#*=} =~ ^[0-9]+$ ]] && continue ;;
        esac
        bad_flag=$flag
        break
    done
    if [[ -n $bad_flag ]]; then
        printf 'ERROR\t%s\tunsupported Kbuild objtool flag: %s\n' "$cmd_path" "$bad_flag" >>"$raw"
        continue
    fi
    if [[ ! -f $obj || -L $obj ]]; then
        printf 'ERROR\t%s\tmissing/nonregular object\n' "$cmd_path" >>"$raw"
        continue
    fi
    argv[-1]=./$obj
    printf '%s\t%s --dry-run %s\n' "$obj" "$objtool" "${argv[*]}" >>"$cmds"
    rc=0
    output=$("$objtool" --dry-run "${argv[@]}" </dev/null 2>&1) || rc=$?
    if [[ $output == *'already has '*' section, skipping'* ]]; then
        printf 'SKIPPED\t%s\n' "$obj" >>"$raw"
    fi
    if ((rc)); then
        printf 'ERROR\t%s\trc=%s\n' "$obj" "$rc" >>"$raw"
    fi
    while IFS= read -r line; do
        if [[ $line == *warning:* ]]; then
            printf 'WARN\t%s\t%s\n' "$obj" "$line" >>"$raw"
        elif ((rc)) && [[ -n $line ]]; then
            printf 'DIAG\t%s\t%s\n' "$obj" "$line" >>"$raw"
        fi
    done <<< "$output"
    if ((verbose)); then printf '.' >&2; fi
done <"$records"
if ((verbose)); then printf '\n' >&2; fi

printf 'kernel tree: %s\nreports: %s.{main,extra}.{cmds,raw}\n' "$PWD" "$out_prefix"
objects=0 warnings=0 skipped=0 errors=0
report() {
    local label=$1 scope=$2 o w s e counts
    o=$(wc -l <"$out_prefix.$scope.cmds")
    counts=$(awk -F '\t' '$1=="WARN" {w++} $1=="SKIPPED" {s++} $1=="ERROR" {e++}
        END {print w+0, s+0, e+0}' "$out_prefix.$scope.raw")
    read -r w s e <<< "$counts"
    printf '%-28s objects=%-6s warnings=%-4s skipped=%-4s errors=%s\n' "$label" "$o" "$w" "$s" "$e"
    objects=$((objects + o)) warnings=$((warnings + w)) skipped=$((skipped + s)) errors=$((errors + e))
}
report 'Kbuild scope (main sweep)' main
report 'boot/vdso/realmode' extra
printf '%-28s objects=%-6s warnings=%-4s skipped=%-4s errors=%s\n' \
    'COMPARABLE TOTAL' "$objects" "$warnings" "$skipped" "$errors"

raw_reports=("$out_prefix.main.raw" "$out_prefix.extra.raw")
if ((warnings)); then
    printf '\n---- warnings, verbatim ----\n'
    awk -F '\t' '$1=="WARN" {sub(/^[^\t]*\t[^\t]*\t/, ""); print}' "${raw_reports[@]}" | LC_ALL=C sort
fi
if ((objects == 0 || skipped || errors)); then
    printf 'kernel_objtool_census: incomplete census (objects=%s skipped=%s errors=%s)\n' \
        "$objects" "$skipped" "$errors" >&2
    awk -F '\t' '$1=="SKIPPED" || $1=="ERROR" || $1=="DIAG"' "${raw_reports[@]}" >&2
    exit 1
fi
