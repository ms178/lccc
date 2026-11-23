#!/usr/bin/env bash
# The linear-form alias analysis LICM, the memory promoter and loop-carried
# forwarding share: what it may call disjoint, and what it must not.
#
# Each cell is one function `sum += a[0]` in a loop that stores through an
# index. The load of a[0] leaves the loop exactly when the forms prove that
# no trip's store reaches it:
#
#   a[i + 1], int i = 0; i < n           marches away from a[0]      -> hoisted
#   the same, tested at the bottom       (addresses made in the header) -> hoisted
#   a[i], int i = 0; i < n               the first trip writes a[0]  -> stays
#   a[i + 1], int i = 1, n trips         marches away from a[0]      -> hoisted
#   the same under -fwrapv               wraps round to a[0]         -> stays
#   a[i + 1], unsigned i = 1, n trips    wraps round to a[0]         -> stays
#   while (trips < a[0]) a[i + 1] = ..   the same three, with the load
#                                        in the loop's own test
#
# From the fourth on the index is not what bounds the loop, so nothing
# keeps it from running into its wrap. The wrapping ones were hoisted
# until the forms learnt which arithmetic is exact: an index narrower than
# an address has a linear form only where its overflow is undefined.
# tests/regression/lin_form_{unsigned_index_wrap,signed_index_wrapv}.c run
# such loops over a 4 GiB object and count the trips.
#
# The loops are top-tested, so LICM needs a preheader only a loop that runs
# reaches. CCC_LOOP_ROTATE=1 gives every loop one: by default rotation asks
# first whether the hoist pays, and for one load that takes a register for
# the length of the loop it says no (tests/regression/check_loop_rotate_profit.sh
# holds that decision). The question here is what LICM may prove, not
# whether it is worth it.
set -euo pipefail
CCC=${CCC:-./target/fastbuild/lccc}
tmp=$(mktemp -d "${TMPDIR:-/tmp}/lccc-licm-alias.XXXXXX")
trap 'rm -rf "$tmp"' EXIT

cat >"$tmp/t.c" <<'C'
#define NOINLINE __attribute__((noinline))
NOINLINE int disjoint(int *a, int n)
{
    int sum = 0;
    for (int i = 0; i < n; ++i) {
        sum += a[0];
        a[i + 1] = i;
    }
    return sum;
}
NOINLINE int disjoint_bottom(int *a, int n)
{
    int sum = 0, i = 0;
    do {
        sum += a[0];
        a[i + 1] = i;
    } while (++i < n);
    return sum;
}
NOINLINE int first_trip_aliases(int *a, int n)
{
    int sum = 0;
    for (int i = 0; i < n; ++i) {
        sum += a[0];
        a[i] = i + 5;
    }
    return sum;
}
/* The index is not the loop's counter: only its type bounds it. */
NOINLINE int signed_index(int *a, unsigned long n)
{
    int sum = 0, i = 1;
    for (unsigned long t = 0; t < n; ++t) {
        sum += a[0];
        a[i + 1] = 7;
        i += 1;
    }
    return sum;
}
NOINLINE int unsigned_index(int *a, unsigned long n)
{
    int sum = 0;
    unsigned i = 1;
    for (unsigned long t = 0; t < n; ++t) {
        sum += a[0];
        a[i + 1] = 7;
        i += 1;
    }
    return sum;
}
/* The load is the loop's own test, so it needs no guard to be hoisted:
 * the store alone decides. */
NOINLINE unsigned long signed_index_bound(int *a)
{
    unsigned long trips = 0;
    int i = 1;
    while (trips < (unsigned long)a[0]) {
        a[i + 1] = 7;
        i += 1;
        trips++;
    }
    return trips;
}
NOINLINE unsigned long unsigned_index_bound(int *a)
{
    unsigned long trips = 0;
    unsigned i = 1;
    while (trips < (unsigned long)a[0]) {
        a[i + 1] = 7;
        i += 1;
        trips++;
    }
    return trips;
}
int main(void)
{
    int a[20] = {3}, b[20] = {3}, c[20] = {3}, d[20] = {3}, e[20] = {3};
    int f[20] = {3}, g[20] = {3};
    if (signed_index_bound(f) != 3 || unsigned_index_bound(g) != 3) return 7;
    if (disjoint(a, 10) != 30) return 1;
    if (disjoint_bottom(b, 10) != 30) return 2;
    /* 3, then the 5 the first trip stored, nine times. */
    if (first_trip_aliases(c, 10) != 3 + 9 * 5) return 3;
    if (signed_index(d, 10) != 30) return 4;
    if (unsigned_index(e, 10) != 30) return 5;
    if (disjoint(a, 0) != 0 || signed_index(d, 0) != 0 || unsigned_index(e, 0) != 0) return 6;
    return 0;
}
C

fail=0
ok() { echo "  ok:   $*"; }
bad() {
    echo "  FAIL: $*" >&2
    fail=1
}

# The number of loads from memory inside the loop of function $2 in
# assembly file $1: the instructions with a memory source in the blocks
# that lie on a cycle of the function's control flow. (Where a block is in
# the file says nothing: the layout puts cold blocks behind a loop and
# jumps back from them, and a top-tested loop closes with a `jmp`.)
loads_in_loop() {
    python3 - "$1" "$2" <<'PY'
import re
import sys

path, fn = sys.argv[1], sys.argv[2]
lines, inside = [], False
for raw in open(path, encoding="utf-8", errors="replace"):
    text = raw.rstrip("\n")
    if text.startswith(fn + ":"):
        inside = True
        continue
    if inside and re.match(r"\s*\.size\s", text):
        break
    if inside and text.strip() and not re.match(r"\s*(#|\.[a-z0-9_]+\b(?!:))", text):
        lines.append(text.strip())

# Blocks: a label starts one, a branch ends one.
blocks, names = [[]], {}
for text in lines:
    label = re.match(r"(\.L[A-Za-z0-9_.$]*):", text)
    if label:
        if blocks[-1]:
            blocks.append([])
        names[label.group(1)] = len(blocks) - 1
        continue
    blocks[-1].append(text)
    if re.match(r"(j[a-z]+|ret[a-z]*|ud2)\b", text):
        blocks.append([])
succ = []
for i, block in enumerate(blocks):
    out = []
    last = block[-1] if block else ""
    jump = re.match(r"(j[a-z]+)\s+(\S+)", last)
    if jump and jump.group(2) in names:
        out.append(names[jump.group(2)])
    falls = not re.match(r"(jmp|ret[a-z]*|ud2)\b", last)
    if falls and i + 1 < len(blocks):
        out.append(i + 1)
    succ.append(out)

def reaches(start):
    seen, work = set(), list(succ[start])
    while work:
        b = work.pop()
        if b not in seen:
            seen.add(b)
            work.extend(succ[b])
    return seen

in_loop = [i for i in range(len(blocks)) if i in reaches(i)]
if not in_loop:
    print("no-loop")
    sys.exit(0)
# AT&T syntax: a memory operand in front of a comma is a source.
load = re.compile(r"[a-z0-9]+\s+[^,]*\([^)]*\)\s*,")
print(sum(1 for i in in_loop for text in blocks[i]
          if load.match(text) and not text.startswith("lea")))
PY
}

expect() { # name, mode, flags, function, loads wanted in the loop, why
    local name=$1 mode=$2 flags=$3 fn=$4 want=$5 why=$6 got
    got=$(loads_in_loop "$tmp/$name.s" "$fn")
    if [[ $got == "$want" ]]; then
        ok "$fn ($mode${flags:+, $flags}): $got load(s) in the loop -- $why"
    else
        bad "$fn ($mode${flags:+, $flags}): $got load(s) in the loop, want $want -- $why"
        sed -n "/^$fn:/,/^[[:space:]]*\.size[[:space:]]/p" "$tmp/$name.s" >&2
    fi
}

build() { # name, flags..., with the environment of the caller
    local name=$1
    shift
    "$CCC" -O2 "$@" -S "$tmp/t.c" -o "$tmp/$name.s"
    "$CCC" -O2 "$@" "$tmp/t.c" -o "$tmp/$name"
    if "$tmp/$name"; then
        ok "$name: runs and returns the sums"
    else
        bad "$name: wrong result (exit $?)"
    fi
}

echo "== every loop rotated: what LICM can prove decides"
CCC_LOOP_ROTATE=1 build rot
expect rot "rotated" "" disjoint 0 "a[i + 1] never reaches a[0]"
expect rot "rotated" "" disjoint_bottom 0 "the same with the addresses made in the loop's header"
expect rot "rotated" "" first_trip_aliases 1 "a[i] is a[0] on the first trip"
expect rot "rotated" "" signed_index 0 "a signed index does not wrap: it marches away"
expect rot "rotated" "" unsigned_index 1 "an unsigned index wraps round to a[0]"

echo "== -fwrapv: a signed index wraps as well"
CCC_LOOP_ROTATE=1 build rot-wrapv -fwrapv
expect rot-wrapv "rotated" "-fwrapv" signed_index 1 "the index wraps round to a[0]"
expect rot-wrapv "rotated" "-fwrapv" unsigned_index 1 "an unsigned index wraps round to a[0]"
expect rot-wrapv "rotated" "-fwrapv" first_trip_aliases 1 "a[i] is a[0] on the first trip"
expect rot-wrapv "rotated" "-fwrapv" signed_index_bound 1 "the loop's test reads a[0], the index wraps round to it"

echo "== -fwrapv, the default pipeline"
build wrapv -fwrapv
expect wrapv "default" "-fwrapv" signed_index_bound 1 "the loop's test reads a[0], the index wraps round to it"
expect wrapv "default" "-fwrapv" unsigned_index_bound 1 "the loop's test reads a[0], an unsigned index wraps round to it"

echo "== the default pipeline"
build default
expect default "default" "" signed_index_bound 0 "the loop's test reads a[0], a signed index marches away"
expect default "default" "" unsigned_index_bound 1 "the loop's test reads a[0], an unsigned index wraps round to it"
expect default "default" "" disjoint_bottom 0 "a bottom-tested loop needs no rotation: its header runs"
expect default "default" "" first_trip_aliases 1 "a[i] is a[0] on the first trip"
expect default "default" "" unsigned_index 1 "an unsigned index wraps round to a[0]"

echo "== no rotation at all"
CCC_NO_LOOP_ROTATE=1 build norot
expect norot "no rotation" "" disjoint 1 "nowhere to put the load that a zero-trip call does not reach"
expect norot "no rotation" "" disjoint_bottom 0 "a bottom-tested loop needs no rotation"
expect norot "no rotation" "" unsigned_index 1 "an unsigned index wraps round to a[0]"

if [[ $fail -ne 0 ]]; then
    echo "check_licm_alias_disjoint: FAIL" >&2
    exit 1
fi
echo "check_licm_alias_disjoint: PASS"
