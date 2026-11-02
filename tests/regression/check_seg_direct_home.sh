#!/usr/bin/env bash
# Segment-override load: the value must land in its own GPR home.
#
# WHY THIS EXISTS
# ---------------
# A per-CPU access whose result is immediately used as the next pointer step
#
#     p = p->next;                     /* struct node __seg_gs *p */
#
# used to round-trip through %rax:
#
#     movq %gs:24(%rdi), %rax          /* load                                */
#     movq %rax, %rdi                  /* relay into the value's home         */
#
# GCC and Clang emit the single-instruction form
#
#     movq %gs:24(%rdi), %rdi
#
# because the segment load has no reason to be pinned to %rax.  lccc's
# *default-space* twin (`emit_load_with_const_offset`) and the segment
# const-ADDRESS form (`emit_seg_load_const_addr`) both already load straight
# into the destination's register; only the segment base+displacement path
# (`emit_seg_load_with_const_offset`) went through %rax.  That is the kernel's
# hottest per-CPU shape (`this_cpu_ptr(p)->field`, per-CPU counters in the
# softirq/sched paths), so it costs one instruction and one live register per
# access.
#
# This gate is the codegen half of the contract: it checks the emitted shape
# (direct home, no relay) and that the walk loop stays within the oracle's
# instruction count.  The behavioural half is
# tests/correctness/programs/seg_gs_const_offset_fold.c, which installs a real
# GS base and checks the loaded values — a shape-only gate cannot see a wrong
# offset, and a behavioural test cannot see the extra instruction.  Both run.
set -euo pipefail

CCC=${CCC:-./target/fastbuild/lccc}
REPO=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
tmp=${TMPDIR:-/tmp}/lccc-seg-direct-home.$$
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp"
cd "$REPO"

if ! command -v "$CCC" >/dev/null 2>&1; then
    echo "FAIL: compiler '$CCC' not found" >&2
    exit 2
fi

# The kernel's per-CPU pointer walk: three segment loads and a segment load of
# the next pointer per iteration.
cat >"$tmp/walk.c" <<'EOF'
struct node { unsigned long tag; unsigned long hits; unsigned long miss; struct node __seg_gs *next; };
unsigned long walk(struct node __seg_gs *p)
{
    unsigned long acc = 0;
    while (p) {
        acc += p->hits;
        acc ^= p->miss;
        acc += p->tag & 7UL;
        p = p->next;
    }
    return acc;
}
EOF

compile() { # compile <outfile> [extra-env-assignment...]
    local out=$1; shift
    # `export` with no arguments PRINTS the environment, so the extra
    # assignments (if any) are applied only when present.
    if [ "$#" -gt 0 ]; then
        ( export "$@"; "$CCC" -O2 -S "$tmp/walk.c" -o "$out" )
    else
        "$CCC" -O2 -S "$tmp/walk.c" -o "$out"
    fi
}

# The inner loop: the LAST label block that ends in a back edge (the walk has a
# pre-header guard block, so "the first label" would capture both).
verdict() { # verdict <asm-file>  -> 0 = pass, 1 = fail (prints the reason)
    local asm=$1
    awk '
        /^\.LBB[0-9]+:$/ { cur = $0; body = ""; next }
        cur != "" {
            body = body $0 "\n"
            if ($0 ~ /^[[:space:]]*(jne|jmp)[[:space:]]/) { lastc = cur; lastl = body; cur = "" }
        }
        END { if (lastl == "") exit 1; printf "%s%s", lastc, lastl }
    ' "$asm" >"$tmp/loop.txt" || {
        echo "FAIL: no back edge found in the walk function"
        return 1
    }
    if ! grep -q '%gs:' "$tmp/loop.txt"; then
        echo "FAIL: no segment load in the walk loop"
        sed -n '1,80p' "$asm"
        return 1
    fi

    # Regexes live in variables: an unquoted `(`/`)` inside `[[ =~ ... ]]` is
    # parsed by bash as a grouping construct (`[^(]` looks unbalanced) and the
    # script dies with "unexpected EOF while looking for matching `)'".
    local re_load_rax='movq[[:space:]]+%gs:[^(]*\((%[a-z0-9]+)\),[[:space:]]*%rax[[:space:]]*$'
    local re_rax_copy='^[[:space:]]*movq[[:space:]]+%rax,[[:space:]]*%[a-z0-9]+[[:space:]]*$'
    local re_load_home='movq[[:space:]]+%gs:[^(]*\((%[a-z0-9%]+)\),[[:space:]]*(%[a-z0-9]+)[[:space:]]*$'

    local prev="" line relay=0 direct=0 rc=0
    while IFS= read -r line; do
        # (1) No segment load may relay through %rax.
        if [[ $prev =~ $re_load_rax ]] && [[ $line =~ $re_rax_copy ]]; then
            echo "FAIL: segment load still relays through %rax:"
            printf '%s\n%s\n' "$prev" "$line"
            relay=1
        fi
        # (2) At least one load must write the same register the value lives in.
        if [[ $line =~ $re_load_home ]] && [[ ${BASH_REMATCH[1]} == ${BASH_REMATCH[2]} ]]; then
            direct=1
        fi
        prev="$line"
    done <"$tmp/loop.txt"
    [ "$relay" = 0 ] || rc=1
    if [ "$direct" = 0 ]; then
        echo "FAIL: no direct-to-home segment load found"
        cat "$tmp/loop.txt"
        rc=1
    fi

    # (3) The loop must not be longer than the oracle's: GCC and Clang emit
    #     eight instructions for this body (the lea-inplace fold removes one;
    #     no pass may add one).  A leading label may share its line with the
    #     first instruction, so strip it before counting.  `grep -c` exits 1 on
    #     a zero count; under `set -euo pipefail` that must not abort the run.
    local n
    n=$(sed 's/^\.LBB[0-9]*://' "$tmp/loop.txt" | grep -cE '^[[:space:]]*[a-z]' || true)
    n=${n:-0}
    if [ "$n" -gt 8 ]; then
        echo "FAIL: loop body has $n instructions (oracle: 8)"
        cat "$tmp/loop.txt"
        rc=1
    fi
    [ "$rc" = 0 ] && echo "PASS: segment loads land in their own home ($n instructions in the walk loop)"
    return $rc
}

compile "$tmp/walk.s" || { echo "FAIL: compile failed" >&2; exit 1; }
if ! out=$(verdict "$tmp/walk.s"); then
    printf '%s\n' "$out" >&2
    exit 1
fi
printf '%s\n' "$out"

# SELF-TEST: with the lea-inplace fold disabled the accumulator relay returns
# and the loop grows past the oracle's count, so the SAME verdict must fail.
# A gate that cannot fail is not a gate.
compile "$tmp/walk.skip.s" CCC_PEEPHOLE_SKIP=load_lea_inplace
if out=$(verdict "$tmp/walk.skip.s"); then
    echo "SELF-TEST FAIL: the gate still passes with the lea-inplace fold disabled:" >&2
    printf '%s\n' "$out" >&2
    exit 1
fi
echo "self-test: fold disabled => gate fails, as required"
