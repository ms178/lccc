#!/usr/bin/env bash
# GCC 16.2 diagnostic-severity parity for the C99-era default-error class
# and the read-only mutation class, measured against gcc-16.2.0.
#
# The contract pinned here (every expectation below was recorded from
# gcc-16.2.0 before being encoded):
#
#   * `-Wimplicit-int`, `-Wimplicit-function-declaration`,
#     `-Wreturn-mismatch`, `-Wint-conversion` and
#     `-Wincompatible-pointer-types` are ERRORS from C99 on (gnu17, gnu23)
#     and silent-or-warnings in gnu90 exactly where GCC 16.2 says so,
#   * the four GCC message shapes of `-Wint-conversion` (initialization,
#     assignment, returning, passing argument) and of
#     `-Wincompatible-pointer-types` appear VERBATIM, at GCC's columns,
#   * the dialect default prints `[-W<flag>]`, an explicit
#     `-Werror=<flag>` promotion keeps GCC's `[-Werror=<flag>]` label,
#     `-Wno-error=<flag>` demotes back to a warning, `-Wno-<flag>`
#     silences, and pre-C99 default suppression never swallows an
#     explicitly requested `-Wimplicit-int`/`-Wimplicit-function-declaration`
#     (and `-Wreturn-mismatch` stays silent in gnu90 even when named),
#   * every read-only mutation (variable / location / member x
#     assignment / increment / decrement) produces GCC's verbatim message,
#     the legal forms stay legal, and nothing ever panics,
#   * GCC 16 itself, when available, agrees with the encoded exit table
#     (so the table cannot rot into self-referential expectations), and
#   * a valid translation unit still compiles (a gate that only asserted
#     rejections would pass with every construct deleted).
set -euo pipefail
# A bare `set -e` abort names nothing in the CI check UI. The ERR trap turns the
# failing command into an annotation, so a red step explains itself without
# anyone having to open a log.
trap 'rc=$?; echo "::error::severity gate aborted (exit $rc) at: $BASH_COMMAND"' ERR
redteam_runner=$(cd "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)/check_diagnostic_redteam.py

CCC=${CCC:-./target/fastbuild/lccc}
if [[ ! -x $CCC ]]; then
    echo "SKIP: $CCC not built" >&2
    exit 1
fi
CCC=$(cd "$(dirname -- "$CCC")" && pwd)/$(basename -- "$CCC")
GCC=${GCC:-gcc-16}

tmp=$(mktemp -d "${TMPDIR:-/tmp}/lccc-gcc162-severity.XXXXXX")
trap 'rm -rf "$tmp"' EXIT
cd "$tmp"

# The message also becomes a GitHub annotation, so a red step names its own
# failure in the check UI instead of only in a log the branch cannot read.
fail() { echo "FAIL: $*" >&2; echo "::error::$*"; exit 1; }

# run_exit <compiler> <file> <std> -> prints the exit code, stderr in <file>.err
run_exit() {
    local compiler=$1 file=$2 std=$3
    set +e
    "$compiler" -std="$std" -fsyntax-only -c "$file" 2>"$file.err"
    local rc=$?
    set -e
    echo "$rc"
}

# expect_exit <std> <file> <expected>
expect_exit() {
    local std=$1 file=$2 want=$3 got
    got=$(run_exit "$CCC" "$file" "$std")
    [[ $got == "$want" ]] \
        || { cat "$file.err" >&2; fail "-std=$std $file: exit $got, measured gcc-16.2 contract wants $want"; }
    # A bad program must never be answered with an ICE.
    ! grep -qE "panicked at|internal compiler error|RUST_BACKTRACE" "$file.err" \
        || { cat "$file.err" >&2; fail "-std=$std $file: ICE instead of a diagnostic"; }
}

# expect_first <std> <file> <regex> : the first diagnostic line must match
expect_first() {
    local std=$1 file=$2 pattern=$3 line
    line=$(grep -E ': (warning|error):' "$file.err" | head -1 || true)
    [[ -n $line ]] || { cat "$file.err" >&2; fail "-std=$std $file: no diagnostic"; }
    [[ $line =~ $pattern ]] \
        || { echo "got:    $line" >&2; echo "wanted: $pattern" >&2; fail "-std=$std $file: first diagnostic mismatch"; }
}

# expect_silent <std> <file>
expect_silent() {
    local std=$1 file=$2
    if grep -qE "^${file}:[0-9]+:[0-9]+: (warning|error):" "$file.err"; then
        cat "$file.err" >&2
        fail "-std=$std $file: expected silence, got a diagnostic"
    fi
}

# ---------------------------------------------------------------- fixtures --
cat > assign_const.c <<'C'
void f(void){ int *const p = 0; p = (int*)1; }
C
cat > bad_cast.c <<'C'
int main(void){ return (int)main; }
C
cat > empty_body.c <<'C'
int f(void){ } int main(void){ return 0; }
C
cat > implicit_fn.c <<'C'
int main(void){ return foo(); }
C
cat > implicit_int.c <<'C'
main(void){ return 0; }
C
cat > implicit_void_ret.c <<'C'
int main(void){ return; }
C
cat > incompat_ptr.c <<'C'
void g(int*); void h(void){ g((double*)0); }
C
cat > int_conv.c <<'C'
void f(void){ int *p = 1; (void)p; }
C
cat > old_style.c <<'C'
int f(a) int a; { return a; } int main(void){ return f(1); }
C
cat > ptr_to_int.c <<'C'
int main(void){ return (int)main(); }
C
cat > ret_mismatch.c <<'C'
int f(void){ return; }
C
cat > void_arith.c <<'C'
int f(void *v){ return (int)((char*)v + 1 - (char*)v); }
C
cat > ro.c <<'C'
struct S { int i; };
void a1(void){ const int x=1; x=2; }
void a2(void){ const int a[2]={1,2}; a[0]=5; }
void a3(void){ const struct S s={0}; s.i=1; }
void a4(void){ int *const p=0; p=(int*)1; }
void a5(void){ const int *q=0; *q=1; }
void a6(void){ const int c=1; ++c; }
void a7(void){ const int c=1; --c; }
void a8(void){ const struct S s={0}; ++s.i; }
void a9(void){ const int a[2]={1,2}; a[0]++; }
void a10(void){ const int c=1; c+=2; }
void a11(void){ int *const p=0; *p=1; }
void a12(void){ int x=1; int *const q=&x; ++(*q); }
void a13(void){ struct S {int i;} s; const struct S *p=&s; p->i=1; }
C

# --------------------------------------------------- exit-code parity table --
# Columns: gnu90 gnu17 gnu23 (gcc-16.2.0 measured; oracle cross-check below).
declare -A want90=(
    [assign_const]=1 [bad_cast]=0 [empty_body]=0 [implicit_fn]=0
    [implicit_int]=0 [implicit_void_ret]=0 [incompat_ptr]=0 [int_conv]=0
    [old_style]=0 [ptr_to_int]=0 [ret_mismatch]=0 [ro]=1 [void_arith]=0
)
declare -A want17=(
    [assign_const]=1 [bad_cast]=0 [empty_body]=0 [implicit_fn]=1
    [implicit_int]=1 [implicit_void_ret]=1 [incompat_ptr]=1 [int_conv]=1
    [old_style]=0 [ptr_to_int]=0 [ret_mismatch]=1 [ro]=1 [void_arith]=0
)
for file in "${!want90[@]}"; do
    expect_exit gnu90 "$file.c" "${want90[$file]}"
    expect_exit gnu17 "$file.c" "${want17[$file]}"
    expect_exit gnu23 "$file.c" "${want17[$file]}"
done

# --------------------------------------- read-only messages, verbatim lines --
expect_first gnu23 ro.c 'ro\.c:2:32: error: assignment of read-only variable .x.'
# every measured message (line:col: severity: text) from one pass:
run_exit "$CCC" ro.c gnu23 >/dev/null
for frag in \
    "assignment of read-only variable 'x'" \
    "assignment of read-only location 'a[0]'" \
    "assignment of member 'i' in read-only object" \
    "assignment of read-only variable 'p'" \
    "assignment of read-only location '*q'" \
    "increment of read-only variable 'c'" \
    "decrement of read-only variable 'c'" \
    "increment of member 'i' in read-only object" \
    "increment of read-only location 'a[0]'" \
    "assignment of read-only variable 'c'" \
    "assignment of member 'i' in read-only object"
do
    grep -qF "$frag" ro.c.err \
        || { cat ro.c.err >&2; fail "ro.c: missing verbatim message '$frag'"; }
done
# The legal forms (a11, a12) must not produce diagnostics of their own; with
# 13 lines and 11 errors, exactly two statements are silent -- assert the
# count so a blanket-reject regression cannot hide here.
n=$(grep -cE "^ro\.c:[0-9]+:[0-9]+: error:" ro.c.err || true)
[[ $n == 11 ]] || { cat ro.c.err >&2; fail "ro.c: $n errors, measured contract wants 11"; }

# ------------------------------------------ -Wint-conversion: 4 contexts ----
cat > ic_init.c <<'C'
void f(void){ int *p = 1; (void)p; }
C
cat > ic_assign.c <<'C'
void f(void){ int *p; p = 2; (void)p; }
C
cat > ic_ret.c <<'C'
int *f(void){ return 3; }
C
cat > ic_arg.c <<'C'
void g(int*); void f(void){ g(4); }
C
for spec in \
    "ic_init.c:1:24:error:initialization of 'int \*' from 'int' makes pointer from integer without a cast \[-Wint-conversion\]" \
    "ic_assign.c:1:25:error:assignment to 'int \*' from 'int' makes pointer from integer without a cast \[-Wint-conversion\]" \
    "ic_ret.c:1:22:error:returning 'int' from a function with return type 'int \*' makes pointer from integer without a cast \[-Wint-conversion\]" \
    "ic_arg.c:1:31:error:passing argument 1 of 'g' makes pointer from integer without a cast \[-Wint-conversion\]"
do
    IFS=: read -r file line col sev rest <<<"$spec"
    run_exit "$CCC" "$file" gnu17 >/dev/null
    grep -qE "^${file}:${line}:${col}: ${sev}: ${rest}" "$file.err" \
        || { cat "$file.err" >&2; fail "$file: int-conversion context/col mismatch"; }
done

# ------------------------ -Wincompatible-pointer-types: 4 contexts + init ---
cat > ip_init.c <<'C'
void f(void){ char c; int *p = &c; (void)p; }
C
cat > ip_assign.c <<'C'
void f(void){ char c; int *p; p = &c; (void)p; }
C
cat > ip_ret.c <<'C'
int *f(void){ return (char*)0; }
C
cat > ip_arg.c <<'C'
void g(int*); void f(void){ g((double*)0); }
C
for std in gnu90 gnu17; do
    sev=warning
    if [[ $std == gnu17 ]]; then sev=error; fi
    for spec in \
        "ip_init.c:1:32:initialization of 'int \*' from incompatible pointer type 'char \*'" \
        "ip_assign.c:1:33:assignment to 'int \*' from incompatible pointer type 'char \*'" \
        "ip_ret.c:1:22:returning 'char \*' from a function with incompatible return type 'int \*'" \
        "ip_arg.c:1:31:passing argument 1 of 'g' from incompatible pointer type"
    do
        IFS=: read -r file line col rest <<<"$spec"
        run_exit "$CCC" "$file" "$std" >/dev/null
        grep -qE "^${file}:${line}:${col}: ${sev}: ${rest} \[-Wincompatible-pointer-types\]" "$file.err" \
            || { cat "$file.err" >&2; fail "$file@$std: incompatible-pointer context/col/severity mismatch"; }
    done
done

# --------------------------------- flag override battery (implicit_int) -----
run_exit "$CCC" implicit_int.c gnu17 >/dev/null
expect_first gnu17 implicit_int.c \
    'implicit_int\.c:1:1: error: return type defaults to .int. \[-Wimplicit-int\]'
# At gnu17 the diagnostic is already an error by dialect default, so GCC
# keeps the plain label even under an explicit -Werror=implicit-int
# (measured); the -Werror= label appears when the promotion is what makes
# it an error, i.e. in gnu90 (measured).
set +e
"$CCC" -std=gnu17 -Werror=implicit-int -fsyntax-only -c implicit_int.c 2>impl_werror.err
set -e
grep -q "error: return type defaults to 'int' \[-Wimplicit-int\]" impl_werror.err \
    || { cat impl_werror.err >&2; fail "gnu17 -Werror=implicit-int label mismatch"; }
set +e
"$CCC" -std=gnu90 -Werror=implicit-int -fsyntax-only -c implicit_int.c 2>impl_werror90.err
set -e
grep -q "error: return type defaults to 'int' \[-Werror=implicit-int\]" impl_werror90.err \
    || { cat impl_werror90.err >&2; fail "gnu90 -Werror=implicit-int label mismatch"; }
set +e
"$CCC" -std=gnu17 -Wno-error=implicit-int -fsyntax-only -c implicit_int.c 2>impl_no_werror.err
rc=$?
set -e
[[ $rc == 0 ]] || fail "-Wno-error=implicit-int must demote to a warning (exit $rc)"
grep -q "warning: return type defaults to 'int' \[-Wimplicit-int\]" impl_no_werror.err \
    || { cat impl_no_werror.err >&2; fail "-Wno-error=implicit-int demotion label mismatch"; }
set +e
"$CCC" -std=gnu17 -Wno-implicit-int -fsyntax-only -c implicit_int.c 2>impl_off.err
set -e
if grep -qE ': (warning|error):' impl_off.err; then
    cat impl_off.err >&2
    fail "-Wno-implicit-int must silence the diagnostic"
fi

# ------------------------ pre-C99 posture (gnu90), measured one by one -------
run_exit "$CCC" implicit_int.c gnu90 >/dev/null
expect_silent gnu90 implicit_int.c
run_exit "$CCC" implicit_fn.c gnu90 >/dev/null
expect_silent gnu90 implicit_fn.c
run_exit "$CCC" ret_mismatch.c gnu90 >/dev/null
expect_silent gnu90 ret_mismatch.c
set +e
"$CCC" -std=gnu90 -Wreturn-mismatch -fsyntax-only -c ret_mismatch.c 2>rm90.err
set -e
if grep -qE "^[a-z0-9_]+\.c:[0-9]+:[0-9]+: (warning|error):" rm90.err; then
    cat rm90.err >&2
    fail "gnu90: -Wreturn-mismatch must stay silent even when named (gcc-16.2 measured)"
fi
set +e
"$CCC" -std=gnu90 -Wimplicit-int -fsyntax-only -c implicit_int.c 2>ii90.err
set -e
grep -q "warning: return type defaults to 'int'" ii90.err \
    || { cat ii90.err >&2; fail "gnu90: explicit -Wimplicit-int must still warn"; }
set +e
"$CCC" -std=gnu90 -Werror -fsyntax-only -c implicit_int.c 2>we90.err
set -e
if grep -qE "^[a-z0-9_]+\.c:[0-9]+:[0-9]+: (warning|error):" we90.err; then
    cat we90.err >&2
    fail "gnu90: -Werror must not promote the silent implicit-int default"
fi
run_exit "$CCC" int_conv.c gnu90 >/dev/null
expect_first gnu90 int_conv.c 'warning: initialization of .int \*. from .int. makes pointer from integer without a cast'
set +e
"$CCC" -std=gnu90 -Werror -fsyntax-only -c int_conv.c 2>ic90w.err
set -e
grep -q "error: initialization of 'int \*' from 'int' makes pointer from integer without a cast \[-Werror=int-conversion\]" ic90w.err \
    || { cat ic90w.err >&2; fail "gnu90 -Werror=int-conversion label mismatch"; }

# ---------------------------- flag kills / demotes for the pointer class -----
set +e
"$CCC" -std=gnu17 -Wno-incompatible-pointer-types -fsyntax-only -c ip_arg.c 2>ip_off.err
set -e
if grep -qE "^[a-z0-9_]+\.c:[0-9]+:[0-9]+: (warning|error):" ip_off.err; then
    cat ip_off.err >&2
    fail "-Wno-incompatible-pointer-types must silence the diagnostic"
fi
set +e
"$CCC" -std=gnu17 -Wno-error=incompatible-pointer-types -fsyntax-only -c ip_arg.c 2>ip_demote.err
set -e
grep -q "warning: passing argument 1 of 'g' from incompatible pointer type" ip_demote.err \
    || { cat ip_demote.err >&2; fail "-Wno-error=incompatible-pointer-types must demote to a warning"; }

# ------------------------------------------------------- positive control ----
cat > valid.c <<'C'
int add(int a, int b){ return a + b; }
int main(void){ const int c = 3; int *p = 0; (void)p; return add(c, 4) - 7; }
C
for std in gnu90 gnu17 gnu23; do
    run_exit "$CCC" valid.c "$std" >/dev/null
    [[ $(cat valid.c.err) == "" ]] || { cat valid.c.err >&2; fail "valid.c rejected under -std=$std"; }
    got=$(run_exit "$CCC" valid.c "$std")
    [[ $got == 0 ]] || fail "valid.c exit $got under -std=$std"
done

# ---------------------------------------------- oracle cross-check (gcc-16) --
if command -v "$GCC" >/dev/null 2>&1; then
    ver=$("$GCC" -dumpfullversion 2>/dev/null || "$GCC" -dumpversion)
    if [[ $ver == 16.* ]]; then
        for file in "${!want90[@]}"; do
            for std in gnu90 gnu17 gnu23; do
                want=${want17[$file]}
                if [[ $std == gnu90 ]]; then want=${want90[$file]}; fi
                got=$(run_exit "$GCC" "$file.c" "$std")
                [[ $got == "$want" ]] \
                    || fail "oracle drift: gcc $ver -std=$std $file.c exits $got, table says $want"
            done
        done
        echo "ok: gcc $ver agrees with the full 39-entry exit table"
    else
        echo "note: $GCC is $ver, not 16.x -- oracle cross-check skipped" >&2
    fi
else
    echo "note: $GCC not found -- oracle cross-check skipped" >&2
fi

echo "PASS: gcc-16.2 diagnostic severity, message and column parity"

if ! redteam=$(CCC="$CCC" GCC="$GCC" python3 "$redteam_runner" 2>&1); then
    printf '%s\n' "$redteam" >&2
    printf '::error::%s\n' "$(printf '%s\n' "$redteam" \
        | grep -m1 -E 'FAIL|Traceback|error:' || printf 'redteam matrix failed')"
    exit 1
fi
printf '%s\n' "$redteam"

"$CCC" -O2 "${redteam_runner%/*}/diagnostic_const_legal_execute.c" -o "$tmp/const-legal"
"$tmp/const-legal"
