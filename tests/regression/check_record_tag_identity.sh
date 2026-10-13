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
# This gate is a differential: the expectations are read off a real compiler at
# run time, not hand-written, and a row the two disagree on fails the gate.
#
# THE ORACLE IS PROBED, NOT ASSUMED
# ---------------------------------
# PR #737 failed on this gate and the red was the gate's fault, not the
# compiler's. ubuntu-latest ships GCC 13, which does not know -std=c23; the old
# `GCC=${GCC:-gcc}` never checked, and its verdict helper returned REJECT for
# ANY non-zero exit -- including "unrecognized command-line option". Five
# negative rows therefore "agreed" with lccc by both rejecting (GCC rejecting
# the FLAG, lccc rejecting the PROGRAM), one positive row went red, and the
# failure read as a miscompile. A gate whose evidence depends on the runner
# image is a gate that silently changes strength between runs, so the oracle is
# now selected by capability and named in the output.
#
# KNOWN DIVERGENCE (asserted, not hidden)
# ---------------------------------------
# Two rows are recorded as known divergences and verified to be *exactly* that,
# so they cannot silently widen:
#
#   pos_n3037       @ -std=c17 : GCC rejects, lccc accepts.
#   pos_anon_member @ -std=c17 : GCC rejects, lccc accepts.
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
WORK=$(mktemp -d) || {
    echo "check_record_tag_identity: FAIL: cannot create a work directory" >&2
    exit 1
}
trap 'rm -rf "$WORK"' EXIT

if [[ ! -x "$CCC" ]]; then
    echo "check_record_tag_identity: FAIL: compiler not found at $CCC" >&2
    exit 1
fi

# --- oracle selection -------------------------------------------------------
probe() { # probe <compiler> <std> -> 0 if it compiles a trivial TU in that dialect
    printf 'int main(void){return 0;}\n' > "$WORK/probe.c"
    "$1" -std="$2" -fsyntax-only "$WORK/probe.c" >/dev/null 2>&1
}

if [[ -n "${GCC:-}" ]]; then
    candidates=("$GCC")
else
    # Newest first: the c23 rows need GCC >= 14, and a pinned newer toolchain
    # beats the image default when both are present. CI installs gcc-14
    # explicitly so this list resolves on a hosted runner too.
    candidates=(gcc-16 gcc-15 gcc-14 gcc)
fi
GCC=''
for cand in "${candidates[@]}"; do
    command -v "$cand" >/dev/null 2>&1 || continue
    if probe "$cand" c23 && probe "$cand" c17; then GCC=$cand; break; fi
done
if [[ -z "$GCC" ]]; then
    echo "check_record_tag_identity: FAIL: no usable oracle among" \
         "${candidates[*]}" >&2
    echo "  an oracle must accept both -std=c17 and -std=c23 (GCC >= 14);" \
         >&2
    echo "  without one the c23 rows are meaningless and the c17 rows" \
         >&2
    echo "  compare a rejection of the FLAG against a rejection of the" \
         >&2
    echo "  PROGRAM.  Install gcc-14 or point GCC= at a suitable compiler." >&2
    exit 1
fi
ORACLE_VERSION=$("$GCC" --version 2>/dev/null)
ORACLE_VERSION=${ORACLE_VERSION%%$'\n'*}

# Positive control: both compilers must accept a trivially valid TU in each
# dialect, or every "both REJECT" below is uninterpretable. Run before any row
# so the failure is reported as an environment problem, not a compiler defect.
for std in c17 c23; do
    for who in "$GCC" "$CCC"; do
        if ! "$who" -std="$std" -fsyntax-only "$WORK/probe.c" >/dev/null 2>&1; then
            echo "check_record_tag_identity: FAIL: positive control -- $who" \
                 "cannot compile a trivial TU at -std=$std" >&2
            exit 1
        fi
    done
done

fail=0
rows=0
declare -a RESULTS=()
ok()  { RESULTS+=("ok   $*"); rows=$((rows + 1)); }
bad() { RESULTS+=("FAIL $*"); fail=1; rows=$((rows + 1)); }
note() { RESULTS+=("        $*"); }

write_case() { printf '%s\n' "$2" > "$WORK/$1.c"; }

# --- cases ------------------------------------------------------------------
# Members do NOT correspond, so both compilers must reject.
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

# The SAME shape spelled with the tag only after the inner definition. This is
# the arm of resolve_struct_or_union that takes its type from the tag rather
# than from a member list, so it is the spelling that regressed to "still
# accepted" when the non-defining path bypassed the alias map.
write_case neg_tagonly_call '
struct S { char c; };
void f(struct S p);
void g(void) { struct S { int c; } s; f(s); }'

# --- pointers to records ----------------------------------------------------
# TAG-ID-1 was closed for by-value records only. These are the same defect
# reached through a pointer, which the record check's `_ => return` arm let
# through: two distinct same-tag records have distinct pointer types too, and
# C23 6.5.2.2p7 makes that a constraint rather than a warning.
write_case neg_ptr_inner_vs_outer '
struct S { char c; };
void f(struct S *p);
void g(void) { struct S { int c; } s; f(&s); }'

# An array parameter decays to a pointer, so the array spelling of the same
# mistake has to be caught by the same rule.
write_case neg_array_of_inner '
struct S { char c; };
void f(struct S a[4]);
void g(void) { struct S { int c; } a[4]; f(a); }'

write_case neg_ptr_diff_tag '
struct A { int x; };
struct B { int x; };
void f(struct A *p);
void g(void) { struct B b; f(&b); }'

# Recursion: the pointee comparison has to reach the ultimate pointee, or
# `struct S **` would be the one spelling that still slips through.
write_case neg_ptr_ptr_record '
struct S { int x; };
void f(struct S **p);
void g(void) { struct S { int y; } *q; f(&q); }'

write_case neg_ptr_scalar '
void f(char *p);
void g(void) { int x; f(&x); }'

write_case neg_funptr_sig '
void f(int (*p)(void));
void g(void) { long (*q)(void); f(q); }'

# --- positives: valid C that both must accept -------------------------------
# Regression guard for the inverse failure: one scope, two spellings of ONE
# type. `struct S b;` after the inner definition denotes the inner record, so
# `b = a` is valid C and must compile. Routing the non-defining arm back to the
# base key made this a hard "incompatible types (have 'struct S' but expected
# 'struct S')" -- a false rejection of valid code that GCC accepts.
write_case pos_tagonly_assign '
struct S { char c; };
int h(void) { struct S { int c; } a; struct S b; b = a; return b.c; }'

# sizeof/alignof must see the inner record too. These are separate sema lookup
# sites from the ones that build CTypes, so they need their own row.
# sizeof must see the inner record. This goes through sema's const-eval, which
# resolves the tag through the alias map. The _Alignof counterpart does NOT
# work and is pinned separately as gap_alignof_static_assert below -- asserting
# both here hid that, because a row that fails for a second reason still looks
# like one row.
write_case pos_sizeof_inner '
struct S { char c; };
int h(void) {
    struct S { int c; } a;
    _Static_assert(sizeof(struct S) == sizeof(int), "sizeof must see the inner record");
    return (int)sizeof a;
}'

# A transparent_union parameter accepts any member type; the record argument
# check must not start rejecting those.
write_case pos_transparent_union '
union U { int i; float f; } __attribute__((transparent_union));
void f(union U u);
void g(void) { f(1); f(1.5f); }'

# Pointer positives: the compatibility rule must not reject what C allows.
write_case pos_ptr_same '
struct S { int x; };
void f(struct S *p);
void g(void) { struct S s = { 1 }; f(&s); }'

# 6.3.2.3p2 in both directions, and the null pointer constant.
write_case pos_void_ptr '
void takes_void(void *p);
void takes_int(int *p);
void g(void) { int x; void *v = &x; takes_void(&x); takes_int(v); takes_int(0); }'

# `char *` against `unsigned char *` is -Wpointer-sign, a warning and not a
# constraint violation, so it must stay acceptable or existing code breaks.
write_case pos_ptr_sign '
void f(unsigned char *p);
void g(void) { char b[4]; f(b); }'

# Array-to-pointer decay at the argument.
write_case pos_array_decay '
void f(int *p);
void g(void) { int a[4]; f(a); }'

# N3037: corresponding members. C23 accepts; pre-C23 the two are distinct.
write_case pos_n3037 '
struct S { int i; };
void f(struct S p);
void g(void) { struct S { int i; } s = { 42 }; f(s); }'

# C23 6.2.7 makes two anonymous members compatible when their members
# correspond. Each anonymous `struct { ... }` conversion mints a fresh
# __anon_struct_N key, so comparing member types by key alone rejected this
# valid C23 program outright.
write_case pos_anon_member '
struct S { struct { int i; }; };
void f(struct S p);
void g(void) { struct S { struct { int i; }; } s; f(s); }'

# PARSER-ALIGN-1, tracked in backlog.md. `_Static_assert` is evaluated by the
# PARSER (parse_static_assert), which resolves `_Alignof(struct S)` through its
# own flat `struct_tag_alignments` map. That map is keyed by bare tag and is
# never unwound at block scope -- the parser has no block scoping at all -- so
# inside a shadowing scope it still holds the OUTER record's alignment and the
# assertion reads 1 where the answer is 4. sizeof is unaffected because it
# resolves in sema, and the same `_Alignof` is correct in an array bound and in
# an enum constant, which do go through sema. Fixing it means giving the parser
# block-scope tracking; pinning it here is what stops it widening silently.
write_case gap_alignof_static_assert '
struct S { char c; };
int h(void) {
    struct S { int c; } a;
    (void)a;
    _Static_assert(_Alignof(struct S) == _Alignof(int), "alignof must see the inner record");
    return 0;
}'

# --- runtime rows -----------------------------------------------------------
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

# A variant key must survive into codegen, where no layout is registered under
# the base key any more. A compound literal reaches lowering annotated by sema.
write_case runtime_compound_literal '
#include <stdio.h>
struct S { char c; };
int main(void) {
    struct S { int c; } a = { 300 };
    struct S b = (struct S){ .c = 42 };
    printf("%d %d\n", a.c, b.c);
    return 0;
}'

# Same, through an array of the inner record: element access and sizeof(arr)
# both have to resolve against the variant layout.
write_case runtime_array_of_inner '
#include <stdio.h>
struct S { char c; };
int main(void) {
    struct S { int c; } arr[2] = { { 300 }, { 7 } };
    printf("%d %d %zu\n", arr[0].c, arr[1].c, sizeof arr);
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

# Passing a function name where a function pointer is expected -- the callback
# idiom, and the shape that broke when function-type compatibility compared
# parameter names (C23 6.7.6.3p15 keeps them out of the type).
write_case runtime_callback '
#include <stdio.h>
static int dbl(int x) { return x * 2; }
static int call(int (*p)(int)) { return p(21); }
int main(void) { printf("%d\n", call(dbl)); return 0; }'

# The original miscompile. Must be rejected and must leave no binary behind.
cat > "$WORK/tagid1.c" <<'EOF'
#include <stdio.h>
struct S { char c; };
unsigned f(struct S p) { return (unsigned)(unsigned char)p.c; }
int main(void) { struct S { int c; } s = { 300 }; printf("%u\n", f(s)); return 0; }
EOF

# --- the row table ----------------------------------------------------------
# One entry per assertion, and the driver below is the only code that knows how
# to run one. EXPECTED_ROWS is derived from the length of this table rather
# than quoted in prose, because the prose had already drifted three times
# (15, 16, 28) while the gate ran something else; a row that stops existing now
# shrinks the table and fails the gate instead of passing with less coverage.
#
#   verdict|<case>|<std>|<oracle reason>|<lccc reason>
#       Both compilers must reach the same verdict, and when that verdict is
#       REJECT both must reach it for the stated reason. A reason is what stops
#       an oracle rejecting its own command line from scoring as agreement.
#       Use "-" where the expected verdict is accept.
#   divergence|<case>|<std>|<oracle verdict>|<lccc verdict>|<why>
#       A pinned, KNOWN disagreement in a stated direction. Fails if the shape
#       changes either way, so fixing the underlying defect turns this row red
#       and forces it to be promoted rather than left recording a stale bug.
#   runtime|<case>|<expected stdout>
#       lccc must build it, run it and print exactly that; the oracle must
#       print the same, so an expectation cannot drift away from the oracle.
#   no-binary|<case>
#       lccc must refuse it and must not leave an executable behind.
PTR_GCC='incompatible pointer type'
PTR_LCCC='incompatible pointer type'
REC_GCC='incompatible type'
REC_LCCC='incompatible type for argument'

ROWS=()
for case in neg_member_type neg_member_name neg_member_count \
            neg_bitfield_width neg_bitfield_signedness neg_tagonly_call; do
    for std in c17 c23; do
        ROWS+=("verdict|$case|$std|$REC_GCC|$REC_LCCC")
    done
done
for case in neg_ptr_inner_vs_outer neg_array_of_inner neg_ptr_diff_tag \
            neg_ptr_ptr_record neg_ptr_scalar neg_funptr_sig; do
    for std in c17 c23; do
        ROWS+=("verdict|$case|$std|$PTR_GCC|$PTR_LCCC")
    done
done
for case in pos_tagonly_assign pos_sizeof_inner pos_transparent_union \
            pos_ptr_same pos_void_ptr pos_ptr_sign pos_array_decay; do
    for std in c17 c23; do
        ROWS+=("verdict|$case|$std|-|-")
    done
done
ROWS+=("verdict|pos_n3037|c23|-|-")
ROWS+=("verdict|pos_anon_member|c23|-|-")
ROWS+=("divergence|pos_n3037|c17|REJECT|accept|C23 N3037 correspondence is unconditional; diagnostics only, layouts identical")
ROWS+=("divergence|pos_anon_member|c17|REJECT|accept|same class as pos_n3037")
ROWS+=("divergence|gap_alignof_static_assert|c23|accept|REJECT|PARSER-ALIGN-1: _Alignof in _Static_assert uses the parser's flat, non-scoped tag map")
ROWS+=("runtime|runtime_same_type|42")
ROWS+=("runtime|runtime_nested_consistent|7 7")
ROWS+=("runtime|runtime_elaborated_after_inner|300 12345 4")
ROWS+=("runtime|runtime_compound_literal|300 42")
ROWS+=("runtime|runtime_array_of_inner|300 7 8")
ROWS+=("runtime|runtime_callback|42")
ROWS+=("no-binary|tagid1")

EXPECTED_ROWS=${#ROWS[@]}

# --- drivers ----------------------------------------------------------------
VERDICT=''; LAST_ERR=''
# verdict_of <compiler...> -- sets VERDICT (accept|REJECT) and LAST_ERR.
# Sets globals rather than echoing: a `$(verdict ...)` command substitution
# runs in a subshell, so stderr captured inside it would not survive to the
# caller that needs it for the reason check and the failure report.
verdict_of() {
    local out rc
    out=$("$@" 2>&1 >/dev/null); rc=$?
    LAST_ERR=$out
    if (( rc == 0 )); then VERDICT=accept; else VERDICT=REJECT; fi
}

# Every verdict row must agree on the VERDICT and, when rejecting, on the
# REASON. Agreement on accept/reject alone is what made five rows worthless on
# hosted CI. A red row prints both diagnostics, because a differential that
# reports only "disagree" sends the next engineer back to reproducing it by hand.
run_verdict() { # run_verdict <case> <std> <oracle reason> <lccc reason>
    local case=$1 std=$2 greason=$3 lreason=$4 want got gerr lerr
    verdict_of "$GCC" -std="$std" -fsyntax-only "$WORK/$case.c"; want=$VERDICT; gerr=$LAST_ERR
    verdict_of "$CCC" -std="$std" -fsyntax-only "$WORK/$case.c"; got=$VERDICT; lerr=$LAST_ERR
    if [[ "$want" != "$got" ]]; then
        bad "$case @$std: gcc=$want lccc=$got"
        note "gcc  stderr: ${gerr:-<none>}"
        note "lccc stderr: ${lerr:-<none>}"
        return
    fi
    if [[ "$want" == REJECT ]]; then
        if [[ "$gerr" != *"$greason"* ]]; then
            bad "$case @$std: the ORACLE rejected for an unrelated reason, so" \
                "this row proves nothing about lccc (wanted '$greason')"
            note "gcc  stderr: ${gerr:-<none>}"
            return
        fi
        if [[ "$lerr" != *"$lreason"* ]]; then
            bad "$case @$std: lccc rejected for an unrelated reason (a parse" \
                "error here would still score as agreement; wanted '$lreason')"
            note "lccc stderr: ${lerr:-<none>}"
            return
        fi
    fi
    ok "$case @$std: both $want, reasons checked"
}

# A pinned divergence: the two compilers are KNOWN to disagree here, in a
# stated direction, and the row asserts that exact shape. It fails if the shape
# changes in either direction -- including if lccc starts agreeing, which means
# someone fixed it and the row should be promoted to a verdict row rather than
# left recording a defect that no longer exists.
run_divergence() { # run_divergence <case> <std> <want-gcc> <want-lccc> <note>
    local case=$1 std=$2 want_gcc=$3 want_lccc=$4 why=$5 pre ours
    verdict_of "$GCC" -std="$std" -fsyntax-only "$WORK/$case.c"; pre=$VERDICT
    verdict_of "$CCC" -std="$std" -fsyntax-only "$WORK/$case.c"; ours=$VERDICT
    if [[ "$pre" == "$want_gcc" && "$ours" == "$want_lccc" ]]; then
        ok "$case @$std: known divergence unchanged (gcc=$pre lccc=$ours) -- $why"
    else
        bad "$case @$std: divergence changed shape (gcc=$pre lccc=$ours," \
            "expected gcc=$want_gcc lccc=$want_lccc) -- update this gate deliberately"
        note "$why"
    fi
}

# The value must be right, not merely accepted, and the expectation is
# cross-checked against the oracle's own output so it cannot quietly drift into
# asserting something the oracle disagrees with.
run_runtime() { # run_runtime <case> <expected>
    local case=$1 expected=$2 out want
    if ! "$CCC" -std=c23 "$WORK/$case.c" -o "$WORK/$case.lccc" >/dev/null 2>&1; then
        bad "$case: lccc failed to build"
        return
    fi
    out=$("$WORK/$case.lccc" 2>&1)
    if "$GCC" -std=c23 "$WORK/$case.c" -o "$WORK/$case.gcc" >/dev/null 2>&1; then
        want=$("$WORK/$case.gcc" 2>&1)
    else
        want='<oracle could not build>'
    fi
    if [[ "$out" == "$expected" && "$want" == "$expected" ]]; then
        ok "$case: $out"
    else
        bad "$case: lccc='$out' gcc='$want' expected='$expected'"
    fi
}

run_no_binary() { # run_no_binary <case>
    local case=$1 ran
    if "$CCC" -std=c23 "$WORK/$case.c" -o "$WORK/$case.bin" >/dev/null 2>&1; then
        ran=$("$WORK/$case.bin" 2>&1)
        bad "$case regressed: compiled and printed '$ran'" \
            "(correct answer 300, gcc rejects)"
    else
        ok "$case: rejected, no binary produced"
    fi
}

for spec in "${ROWS[@]}"; do
    IFS='|' read -r -a f <<< "$spec"
    case "${f[0]}" in
        verdict)    run_verdict    "${f[1]}" "${f[2]}" "${f[3]}" "${f[4]}" ;;
        divergence) run_divergence "${f[1]}" "${f[2]}" "${f[3]}" "${f[4]}" "${f[5]}" ;;
        runtime)    run_runtime    "${f[1]}" "${f[2]}" ;;
        no-binary)  run_no_binary  "${f[1]}" ;;
        *)          bad "internal: unknown row kind '${f[0]}' in '$spec'" ;;
    esac
done

for line in "${RESULTS[@]}"; do printf '%s\n' "$line"; done

if (( fail != 0 )); then
    echo "check_record_tag_identity: FAILED ($rows rows vs oracle: $ORACLE_VERSION)" >&2
    exit 1
fi
# The count is an assertion in its own right: it catches a driver that silently
# stops dispatching a kind, which the table alone would not.
if (( rows != EXPECTED_ROWS )); then
    echo "check_record_tag_identity: FAILED -- ran $rows rows, expected" \
         "$EXPECTED_ROWS, so a row stopped running" >&2
    exit 1
fi
echo "check_record_tag_identity: PASS ($rows rows vs oracle: $ORACLE_VERSION)"
