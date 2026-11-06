#!/usr/bin/env bash
# -g must not change generated code: with and without -g, the assembly of a
# source must be identical once debug-only rows are removed.  Debug rows used
# to be visible to the peephole passes as ordinary lines, so window caps, slot
# reuse and textual matchers decided differently under -g (see
# docs/DWARF-LINE-TABLES.md).  D7/D10 also render `.set .Llccc_debug_alias_*`
# and `.section .debug_*` data into `-S`; those are metadata, not code, and the
# filter below removes them while leaving instructions, CFI and non-debug
# sections byte-for-byte comparable.
#
# Checked: every entry of debug_invariance_sentinels.txt (each historical
# divergence, at the target and level where it diverged) plus a breadth
# sample of every STRIDE-th source at OPTS on x86-64 and i686.  AArch64 and
# RISC-V run their own text peepholes and are checked on the sentinels.
#
#   CCC=<lccc>        x86-64 compiler (default target/fastbuild/lccc)
#   CCC_I686=<lccc>   i686 compiler (default lccc-i686 beside $CCC)
#   CCC_ARM=<lccc>    AArch64 compiler (default lccc-arm beside $CCC)
#   CCC_RISCV=<lccc>  RISC-V compiler (default lccc-riscv beside $CCC)
#                     (a target's checks are skipped when it is absent)
#   STRIDE=32 OPTS=-O2
#   (STRIDE=1 OPTS="-O0 -O1 -O2 -O3" is the exhaustive sweep.)
set -u
here=$(cd "$(dirname "$0")" && pwd)
CCC=${CCC:-target/fastbuild/lccc}
CCC_I686=${CCC_I686:-$(dirname "$CCC")/lccc-i686}
CCC_ARM=${CCC_ARM:-$(dirname "$CCC")/lccc-arm}
CCC_RISCV=${CCC_RISCV:-$(dirname "$CCC")/lccc-riscv}
STRIDE=${STRIDE:-32}
OPTS=${OPTS:-"-O2"}
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

# Strip only debug metadata.  A `.section .debug_*` block is skipped until the
# next non-debug section directive; `.loc`, `.file` and the writer's
# `.Llccc_debug_alias_*` absolute symbols are skipped line-wise.  Ordinary
# `.set`, instructions, labels, CFI and data sections are preserved.
strip_debug_rows() {
    awk '
        /^[[:space:]]*\.(loc|file)([[:space:]]|$)/ { next }
        /^[[:space:]]*\.set[[:space:]]+\.Llccc_debug_alias_/ { next }
        /^[[:space:]]*\.section[[:space:]]+\.debug_/ { indebug = 1; next }
        indebug {
            if ($0 ~ /^[[:space:]]*\.section[[:space:]]+/ && $0 !~ /^[[:space:]]*\.section[[:space:]]+\.debug_/) {
                indebug = 0
            } else {
                next
            }
        }
        { print }
    ' "$1"
}

# The filter is load-bearing: prove it ignores D7 metadata but still rejects a
# one-byte instruction change hidden under the same metadata.
filter_self_test() {
    local d="$work/filter-self-test"
    mkdir -p "$d"
    cat > "$d/plain.s" <<'EOF'
	.text
foo:
	add	a0, a0, 1
	ret
	.section	.note.GNU-stack,"",@progbits
EOF
    cat > "$d/debug.s" <<'EOF'
	.file	1 "x.c"
	.text
foo:
	.loc	1 1 0
	add	a0, a0, 1
	ret
	.section	.note.GNU-stack,"",@progbits
	.set	.Llccc_debug_alias_1,.text+0
	.section	.debug_info,"",%progbits
	.byte	1,2,3
	.section	.debug_str,"MS",%progbits,1
	.byte	4
EOF
    strip_debug_rows "$d/plain.s" > "$d/a"
    strip_debug_rows "$d/debug.s" > "$d/b"
    if ! cmp -s "$d/a" "$d/b"; then
        echo "FAIL: debug-row filter self-test rejected metadata-only difference" >&2
        exit 1
    fi
    sed 's/add	a0, a0, 1/add	a0, a0, 2/' "$d/debug.s" > "$d/changed.s"
    strip_debug_rows "$d/changed.s" > "$d/c"
    if cmp -s "$d/a" "$d/c"; then
        echo "FAIL: debug-row filter self-test missed an instruction change" >&2
        exit 1
    fi
}
filter_self_test

# One job per line: <compiler> <level> <source>.
{
    while read -r target level src; do
        case "$target" in
            x86_64) echo "$CCC $level $src" ;;
            i686) [ -x "$CCC_I686" ] && echo "$CCC_I686 $level $src" ;;
            aarch64) [ -x "$CCC_ARM" ] && echo "$CCC_ARM $level $src" ;;
            riscv64) [ -x "$CCC_RISCV" ] && echo "$CCC_RISCV $level $src" ;;
        esac
    done < <(grep -v '^#' "$here/debug_invariance_sentinels.txt")
    index=0
    for src in "$here"/*.c; do
        index=$((index + 1))
        [ $(((index - 1) % STRIDE)) -eq 0 ] || continue
        for level in $OPTS; do
            echo "$CCC $level $(basename "$src")"
            [ -x "$CCC_I686" ] && echo "$CCC_I686 $level $(basename "$src")"
        done
    done
} | sort -u > "$work/jobs"

check() {
    local cc=$1 level=$2 src=$here/$3 out flags=""
    out=$(mktemp -d "$work/job.XXXXXX")
    [ -f "${src%.c}.flags" ] && flags=$(cat "${src%.c}.flags")
    # shellcheck disable=SC2086  # .flags holds a word list
    if ! timeout 60 "$cc" $level $flags -S "$src" -o "$out/plain.s" 2>/dev/null; then
        echo "SKIP $(basename "$cc") $level $3"
        return
    fi
    # shellcheck disable=SC2086
    if ! timeout 60 "$cc" $level $flags -g -S "$src" -o "$out/debug.s" 2>/dev/null; then
        echo "FAIL $(basename "$cc") $level $3: rejected only under -g"
        return
    fi
    strip_debug_rows "$out/plain.s" > "$out/a"
    strip_debug_rows "$out/debug.s" > "$out/b"
    if cmp -s "$out/a" "$out/b"; then
        echo "PASS"
    else
        echo "FAIL $(basename "$cc") $level $3: -g changes the code"
        diff "$out/a" "$out/b" | head -6 | sed 's/^/    /'
    fi
    rm -rf "$out"
}
export -f check strip_debug_rows
export here work

xargs -P "$(nproc)" -L 1 bash -c 'check "$@"' _ < "$work/jobs" > "$work/results"

pass=$(grep -c '^PASS' "$work/results")
skip=$(grep -c '^SKIP' "$work/results")
fail=$(grep -c '^FAIL' "$work/results")
if [ "$fail" -ne 0 ]; then
    grep -v '^PASS\|^SKIP' "$work/results"
    echo "FAIL: -g invariance: $fail of $((pass + fail)) compilations differ (skipped $skip)"
    exit 1
fi
echo "PASS: -g invariance: $pass compilations identical modulo debug rows" \
     "($(grep -vc "^#" "$here/debug_invariance_sentinels.txt") sentinels;" \
     "1 in $STRIDE sources at $OPTS; skipped $skip)"
