#!/usr/bin/env bash
# check_sema_lvalue_gates.sh -- error-contract pins for the C11 6.3.2.1p3
# modifiable-lvalue rule (assignment/increment/decrement of non-modifiable
# lvalues).  The ACCEPTED half lives in tests/regression/
# const_member_modifiability.c (run_regression corpus); this gate pins the
# REJECTED half: each case must fail with exactly GCC 16.2's wording, once,
# and -- when a pinned gcc-16 is on PATH -- its live error text is compared
# byte-for-byte so the two compilers cannot drift apart unnoticed.
#
# Exit status: non-zero on any mismatch.
set -u
DIR="$(cd "$(dirname "$0")" && pwd)"
LCCC_CC="${CCC:-lccc-cc}"
GCC_BIN="${GCC:-gcc-16}"
WORK="${TMPDIR:-/tmp}/lvalue-gates.$$"
mkdir -p "$WORK"
pass=0; fail=0

say() { printf '%s\n' "$*"; }

# case <name> <expected-message> <source>
case_check() {
    local name="$1" want="$2" src="$3"
    printf '%s\n' "$src" > "$WORK/$name.c"
    local out rc errs
    out=$("$LCCC_CC" -c "$WORK/$name.c" -o "$WORK/$name.o" 2>&1)
    rc=$?
    # Count positional diagnostics only; the trailing `ccc: error: N error(s)`
    # summary line is status text, not a diagnostic.
    errs=$(printf '%s\n' "$out" | grep -cE ":[0-9]+:[0-9]+: error:")
    if [ "$rc" -eq 0 ]; then
        say "  FAIL $name: lccc ACCEPTED (rc=0) -- expected rejection"
        fail=$((fail + 1))
        return
    fi
    if ! printf '%s\n' "$out" | grep -qF "$want"; then
        say "  FAIL $name: wrong diagnostic (want: $want)"
        printf '%s\n' "$out" | sed 's/^/      | /' | head -4
        fail=$((fail + 1))
        return
    fi
    if [ "$errs" -ne 1 ]; then
        say "  FAIL $name: $errs errors emitted (NO-SPAM contract: exactly 1)"
        fail=$((fail + 1))
        return
    fi
    # Live GCC comparison when the pinned oracle is available.
    if command -v "$GCC_BIN" > /dev/null 2>&1; then
        local gout grc
        gout=$("$GCC_BIN" -c "$WORK/$name.c" -o "$WORK/$name.g.o" 2>&1)
        grc=$?
        if [ "$grc" -eq 0 ]; then
            say "  FAIL $name: $GCC_BIN accepted the case -- update the pin"
            fail=$((fail + 1))
            return
        fi
        if ! printf '%s\n' "$gout" | grep -qF "$want"; then
            say "  FAIL $name: $GCC_BIN wording diverges (want: $want)"
            printf '%s\n' "$gout" | sed 's/^/      | /' | head -4
            fail=$((fail + 1))
            return
        fi
    fi
    say "  ok   $name: rejected with exact wording, 1 error"
    pass=$((pass + 1))
}

say "== sema modifiable-lvalue gates (C11 6.3.2.1p3)"

case_check structural_member "assignment of read-only member 'n'" '
struct s6 { const int v; };
struct s5 { union { struct s6 n; int z; } u; };
void f(void) {
    struct s5 wu = { 0 };
    struct s6 w = { 1 };
    wu.u.n = w;
}'

case_check whole_aggregate "assignment of read-only variable 'wu'" '
struct s6 { const int v; };
struct s5 { struct s6 n; int z; };
void f(void) {
    struct s5 wu = { 0 }, v = { 0 };
    wu = v;
}'

case_check deref_aggregate "assignment of read-only location" '
struct s6 { const int v; };
struct s5 { struct s6 n; int z; };
void f(struct s5 *p, struct s5 v) {
    *p = v;
}'

case_check const_variable "assignment of read-only variable 'cc'" '
struct clean { int a; };
void f(void) {
    const struct clean cc = { 0 }, o = { 1 };
    cc = o;
}'

case_check deep_const_member "assignment of read-only member 'v'" '
struct s6 { const int v; };
struct s5 { struct s6 n; };
void f(void) {
    struct s5 wu = { 0 };
    wu.n.v = 1;
}'

case_check inc_const_var "increment of read-only variable 'ci'" '
void f(void) {
    const int ci = 0;
    ci++;
}'

case_check dec_const_member "decrement of read-only member 'v'" '
struct s6 { const int v; };
struct s5 { struct s6 n; int z; };
void f(void) {
    struct s5 wu = { 0 };
    wu.n.v--;
}'

# NO-SPAM across a multi-error file: exactly one diagnostic per violation.
multi_src='
struct s6 { const int v; };
struct s5 { struct s6 n; int z; };
void f(void) {
    struct s5 wu = { 0 }, v = { 0 };
    wu = v;
    wu.n = v.n;
    wu = v;
}'
printf '%s\n' "$multi_src" > "$WORK/multi.c"
multi_out=$("$LCCC_CC" -c "$WORK/multi.c" -o "$WORK/multi.o" 2>&1)
multi_rc=$?
multi_errs=$(printf '%s\n' "$multi_out" | grep -cE ":[0-9]+:[0-9]+: error:")
if [ "$multi_rc" -ne 0 ] && [ "$multi_errs" -eq 3 ]; then
    say "  ok   multi_error_no_spam: 3 violations -> exactly 3 errors"
    pass=$((pass + 1))
else
    say "  FAIL multi_error_no_spam: rc=$multi_rc errs=$multi_errs (want 3)"
    fail=$((fail + 1))
fi

rm -rf "$WORK"
say "check_sema_lvalue_gates: $pass passed, $fail failed"
[ "$fail" -eq 0 ]
