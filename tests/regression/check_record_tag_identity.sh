#!/usr/bin/env bash
# Record tag identity: a nested same-tag definition must not alias the outer one.
#
# WHY THIS EXISTS
# ---------------
# lccc keyed struct/union types by tag alone while struct layouts were
# scope-aware, so an inner `struct S { int c; }` and an outer
# `struct S { char c; }` were the SAME CType. No compatibility check could fire,
# and the call path did not compare record types at all -- so the program below
# compiled and printed 44 where the correct answer is 300, while GCC rejects it
# outright:
#
#     struct S { char c; };
#     unsigned f(struct S p) { return (unsigned)(unsigned char)p.c; }
#     int main(void) { struct S { int c; } s = { 300 }; printf("%u\n", f(s)); }
#
# A regression corpus cannot catch this: it compares runtime output against
# GCC, and GCC refuses to build the test at all. So the oracle here is
# accept/reject, not output. Tracked as TAG-ID-1 in backlog.md.
#
# This gate is a differential against the host GCC, which is the point: the
# expectations are not hand-written, they are read off the oracle at run time.
# A row that lccc and GCC disagree on fails the gate.
#
# KNOWN DIVERGENCE (asserted, not hidden)
# ---------------------------------------
# One row is recorded as a known divergence and is verified to be *exactly*
# that, so it cannot silently widen:
#
#   pos_n3037 @ -std=c17 : GCC rejects, lccc accepts.
#
# C 6.7.2.3 makes an inner-scope definition a new type incompatible with the
# outer one, so GCC is right. lccc gives two definitions with *corresponding*
# members the same key, which is what C23 (N3037) requires; pre-C23 it is too
# permissive. It cannot miscompile -- corresponding members means identical
# layouts, so the values are interchangeable in memory -- and closing it needs
# the C standard threaded into sema (which currently has no notion of -std)
# plus a mode-gated N3037 compatibility relation. Making keys always distinct
# without that relation would reject valid C23, which is worse.

set -uo pipefail

CCC=${CCC:-target/fastbuild/lccc}
GCC=${GCC:-gcc}
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

if [[ ! -x "$CCC" ]]; then
    echo "check_record_tag_identity: FAIL: compiler not found at $CCC" >&2
    exit 1
fi
if ! command -v "$GCC" >/dev/null 2>&1; then
    echo "check_record_tag_identity: FAIL: no oracle ($GCC) -- this gate is" \
         "meaningless without one and must not silently pass" >&2
    exit 1
fi

fail=0
declare -a RESULTS=()
ok()   { RESULTS+=("ok   $*"); }
bad()  { RESULTS+=("FAIL $*"); fail=1; }

# write_case <name> <body>
write_case() { printf '%s\n' "$2" > "$WORK/$1.c"; }

# --- negative rows: members do NOT correspond, so both must reject -----------
write_case neg_member_type '
struct S { char c; };
unsigned f(struct S p) { return (unsigned)(unsigned char)p.c; }
int main(void) { struct S { int c; } s = { 300 }; return (int)f(s); }'

write_case neg_member_name '
struct S { int i; };
void f(struct S p);
void g(void) { struct S { int j; } s = { 42 }; f(s); }'

write_case neg_member_count '
struct S { int i; };
void f(struct S p);
void g(void) { struct S { int i; int k; } s = { 42, 1 }; f(s); }'

write_case neg_bitfield_width '
struct S { int b : 3; };
void f(struct S p);
void g(void) { struct S { int b : 5; } s = { 1 }; f(s); }'

write_case neg_bitfield_signedness '
struct S { signed int b : 3; };
void f(struct S p);
void g(void) { struct S { unsigned int b : 3; } s = { 1 }; f(s); }'

# --- positive rows ----------------------------------------------------------
# N3037: corresponding members. C23 accepts; pre-C23 the two are distinct.
write_case pos_n3037 '
struct S { int i; };
void f(struct S p);
void g(void) { struct S { int i; } s = { 42 }; f(s); }'

# Ordinary same-type by-value passing, fully linked, with a checked result.
write_case runtime_same_type '
#include <stdio.h>
struct P { int x, y; };
static int sum(struct P p) { return p.x + p.y; }
int main(void) { struct P q = { 20, 22 }; printf("%d\n", sum(q)); return 0; }'

# An elaborated `struct S b;` AFTER an inner definition must denote the INNER
# record, not the outer one -- this is the path where the type comes from the
# tag alone rather than from a member list, so it exercises the alias map.
write_case runtime_elaborated_after_inner '
#include <stdio.h>
struct S { char c; };
int main(void) {
    struct S { int c; } a = { 300 };
    struct S b;
    b.c = 12345;
    printf("%d %d %zu\n", a.c, b.c, sizeof b);
    return 0;
}'

# A nested definition that shadows with corresponding members, then is used
# consistently inside its own scope: must build and run correctly.
write_case runtime_nested_consistent '
#include <stdio.h>
struct Q { int v; };
static int outer(struct Q q) { return q.v; }
int main(void) {
    struct Q { int v; } inner = { 7 };
    printf("%d %d\n", inner.v, outer(inner));
    return 0;
}'

verdict() { # verdict <compiler...> <file> -> accept|REJECT
    if "$@" >/dev/null 2>&1; then echo accept; else echo REJECT; fi
}

compare_row() { # compare_row <case> <std>
    local case=$1 std=$2 want got
    want=$(verdict "$GCC" -std="$std" -fsyntax-only "$WORK/$case.c")
    got=$(verdict "$CCC" -std="$std" -fsyntax-only "$WORK/$case.c")
    if [[ "$want" == "$got" ]]; then
        ok "$case @$std: both $want"
    else
        bad "$case @$std: gcc=$want lccc=$got"
    fi
}

# Every negative row must match the oracle in both modes.
for case in neg_member_type neg_member_name neg_member_count \
            neg_bitfield_width neg_bitfield_signedness; do
    compare_row "$case" c17
    compare_row "$case" c23
done

# C23 N3037 must match the oracle.
compare_row pos_n3037 c23

# The one known divergence, pinned so it cannot widen unnoticed.
pre=$(verdict "$GCC" -std=c17 -fsyntax-only "$WORK/pos_n3037.c")
ours=$(verdict "$CCC" -std=c17 -fsyntax-only "$WORK/pos_n3037.c")
if [[ "$pre" == REJECT && "$ours" == accept ]]; then
    ok "pos_n3037 @c17: known divergence unchanged (gcc=REJECT lccc=accept; diagnostics only, layouts identical)"
else
    bad "pos_n3037 @c17: divergence changed shape (gcc=$pre lccc=$ours) -- update this gate deliberately"
fi

# Runtime rows: the value must be right, not merely accepted.
if "$CCC" -std=c23 "$WORK/runtime_same_type.c" -o "$WORK/rt1" >/dev/null 2>&1; then
    out=$("$WORK/rt1" 2>&1)
    if [[ "$out" == "42" ]]; then ok "runtime_same_type: 42"; else
        bad "runtime_same_type: printed '$out', expected 42"; fi
else
    bad "runtime_same_type: failed to build"
fi

if "$CCC" -std=c23 "$WORK/runtime_nested_consistent.c" -o "$WORK/rt2" >/dev/null 2>&1; then
    out=$("$WORK/rt2" 2>&1)
    if [[ "$out" == "7 7" ]]; then ok "runtime_nested_consistent: 7 7"; else
        bad "runtime_nested_consistent: printed '$out', expected '7 7'"; fi
else
    bad "runtime_nested_consistent: failed to build"
fi

# The original miscompile must stay rejected, and must never produce a binary.
cat > "$WORK/tagid1.c" <<'EOF'
#include <stdio.h>
struct S { char c; };
unsigned f(struct S p) { return (unsigned)(unsigned char)p.c; }
int main(void) { struct S { int c; } s = { 300 }; printf("%u\n", f(s)); return 0; }
EOF
if "$CCC" -std=c23 "$WORK/tagid1.c" -o "$WORK/tagid1" >/dev/null 2>&1; then
    ran=$("$WORK/tagid1" 2>&1)
    bad "TAG-ID-1 regressed: compiled and printed '$ran' (correct answer 300, gcc rejects)"
else
    ok "TAG-ID-1: rejected, no binary produced"
fi

if "$CCC" -std=c23 "$WORK/runtime_elaborated_after_inner.c" -o "$WORK/rt3" >/dev/null 2>&1; then
    out=$("$WORK/rt3" 2>&1)
    # GCC prints the same; the point is that `b` is the INNER 4-byte record.
    want=$("$GCC" -std=c23 "$WORK/runtime_elaborated_after_inner.c" -o "$WORK/rt3g" >/dev/null 2>&1 && "$WORK/rt3g")
    if [[ "$out" == "$want" && "$out" == "300 12345 4" ]]; then
        ok "runtime_elaborated_after_inner: $out"
    else
        bad "runtime_elaborated_after_inner: lccc='$out' gcc='$want' expected='300 12345 4'"
    fi
else
    bad "runtime_elaborated_after_inner: failed to build"
fi

for line in "${RESULTS[@]}"; do printf '%s\n' "$line"; done
if (( fail != 0 )); then
    echo "check_record_tag_identity: FAILED" >&2
    exit 1
fi
echo "check_record_tag_identity: PASS (${#RESULTS[@]} rows vs $GCC oracle)"
