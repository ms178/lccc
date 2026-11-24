#!/usr/bin/env bash
# Every input file is compiled, whatever is wrong with the one before it.
#
# GCC and Clang go on with the inputs behind one that fails:
#
#     gcc -c a.c bad.c c.c              -> a.o and c.o, bad.c's errors, rc 1
#     gcc -S a.c bad.c c.c              -> a.s and c.s, rc 1
#     gcc -E stop.c c.c                 -> c.c preprocessed on stdout, rc 1
#     gcc -fsyntax-only bad.c worse.c   -> the errors of both, rc 1
#     gcc main.c bad.c worse.c -o prog  -> the errors of both, no prog, rc 1
#
# (measured with GCC 16.2 and Clang 23.1.5, for C and for assembly inputs).
# lccc ended the run at the first failure: `c.o` was not written and what was
# wrong with a second bad input was not said, so a build that hands the driver
# several inputs needed a run for each error.
#
# Each failure is reported when it happens, under the diagnostics of its own
# input, and among several inputs it says which one: the assembler's
# diagnostics do not name their file. One input alone says what it always
# said. The objects a run leaves are compared with GCC's where `$GCC` is
# installed; the rows themselves do not need it.
set -euo pipefail

CCC=${CCC:-./target/fastbuild/lccc}
if [[ ! -x $CCC ]]; then
    echo "SKIP: $CCC not built" >&2
    exit 1
fi
CCC=$(cd "$(dirname -- "$CCC")" && pwd)/$(basename -- "$CCC")
GCC=${GCC:-gcc}

tmp=$(mktemp -d "${TMPDIR:-/tmp}/lccc-every-input.XXXXXX")
trap 'rm -rf "$tmp"' EXIT
cd "$tmp"

fail=0
fail() { echo "FAIL: $*" >&2; fail=1; }

# C inputs and assembly inputs under different stems: `-S a.c` writes `a.s`.
printf 'int first(void) { return 1; }\n' >a.c
printf 'int bad(void) { return undeclared_name; }\n' >bad.c
printf 'int third(void) { return 3; }\n' >c.c
printf 'int worse(void) { return another_undeclared; }\n' >worse.c
printf '#error stop here\nint never;\n' >stop.c
printf '\t.text\n\tnop\n' >x.s
printf '\t.text\n\tbt $256, %%ecx\n' >ybad.s
printf '\t.text\n\tret\n' >z.s
printf '\t.text\n\tint $256\n' >zworse.s

# The objects a run left, in order.
objects() { find . -maxdepth 1 -name '*.o' -printf '%f\n' | sort | tr '\n' ' '; }
reset() { rm -f -- *.o a.s bad.s c.s err.txt out.txt; }

# -c, C inputs: the objects of the good ones, the error of the bad one, rc 1.
reset
rc=0; "$CCC" -c a.c bad.c c.c 2>err.txt || rc=$?
[[ $rc -eq 1 ]] || fail "-c with a bad input: rc $rc, expected 1"
[[ $(objects) == "a.o c.o " ]] || fail "-c a.c bad.c c.c left '$(objects)', expected 'a.o c.o '"
grep -q 'undeclared_name' err.txt || fail "-c: bad.c's diagnostic is missing"

# -c, assembly inputs.
reset
rc=0; "$CCC" -c x.s ybad.s z.s 2>err.txt || rc=$?
[[ $rc -eq 1 ]] || fail "-c with a bad assembly input: rc $rc, expected 1"
[[ $(objects) == "x.o z.o " ]] || fail "-c x.s ybad.s z.s left '$(objects)', expected 'x.o z.o '"
[[ $(cat err.txt) == "ccc: error: ybad.s: operand type mismatch for \`bt'" ]] ||
    fail "-c: one line that names the bad input, got: $(cat err.txt)"

# Two bad inputs: both are reported, in the order of the command line, and
# the good one between them is assembled.
reset
rc=0; "$CCC" -c ybad.s z.s zworse.s 2>err.txt || rc=$?
[[ $rc -eq 1 ]] || fail "-c with two bad inputs: rc $rc, expected 1"
[[ $(objects) == "z.o " ]] || fail "-c ybad.s z.s zworse.s left '$(objects)', expected 'z.o '"
[[ $(wc -l <err.txt) -eq 2 ]] ||
    fail "-c: two lines for two bad inputs, got: $(cat err.txt)"
[[ $(sed -n 1p err.txt) == "ccc: error: ybad.s: "*"\`bt'"* &&
    $(sed -n 2p err.txt) == "ccc: error: zworse.s: "*"\`int'"* ]] ||
    fail "-c: each input's error under its name, in their order; got: $(cat err.txt)"

# One input alone fails as it always did: the same line, rc 1, nothing written.
reset
rc=0; "$CCC" -c ybad.s 2>err.txt || rc=$?
[[ $rc -eq 1 && -z $(objects) && $(cat err.txt) == "ccc: error: operand type mismatch for \`bt'" ]] ||
    fail "-c ybad.s alone: rc $rc, left '$(objects)', said: $(cat err.txt)"

# -S: the assembly of the good ones.
reset
rc=0; "$CCC" -S a.c bad.c c.c 2>err.txt || rc=$?
[[ $rc -eq 1 ]] || fail "-S with a bad input: rc $rc, expected 1"
[[ -s a.s && -s c.s && ! -e bad.s ]] || fail "-S a.c bad.c c.c: a.s and c.s expected, bad.s not"
grep -q 'third' c.s || fail "-S: c.s is not the assembly of c.c"

# -E: the inputs behind the bad one are preprocessed.
reset
rc=0; "$CCC" -E stop.c c.c >out.txt 2>err.txt || rc=$?
[[ $rc -ne 0 ]] || fail "-E with a bad input: rc 0"
grep -q 'third' out.txt || fail "-E stop.c c.c did not preprocess c.c"

# -fsyntax-only: what is wrong with each, and each failure under the
# diagnostics of its own input.
reset
rc=0; "$CCC" -fsyntax-only bad.c worse.c 2>err.txt || rc=$?
[[ $rc -ne 0 ]] || fail "-fsyntax-only with bad inputs: rc 0"
order=$(grep -n -o -e 'undeclared_name. undeclared' -e 'another_undeclared. undeclared' \
    -e '^ccc: error: bad\.c: ' -e '^ccc: error: worse\.c: ' err.txt | cut -d: -f2- | tr '\n' '|')
[[ $order == "undeclared_name' undeclared|ccc: error: bad.c: |another_undeclared' undeclared|ccc: error: worse.c: |" ]] ||
    fail "-fsyntax-only bad.c worse.c: order of the reports is '$order' in: $(cat err.txt)"

# A link: every input is compiled and reported, and nothing is linked.
reset
rm -f prog
printf 'int main(void) { return 0; }\n' >main.c
rc=0; "$CCC" main.c bad.c worse.c -o prog 2>err.txt || rc=$?
[[ $rc -eq 1 && ! -e prog ]] || fail "a link with bad inputs: rc $rc, prog $(ls prog 2>/dev/null)"
if ! grep -q 'undeclared_name' err.txt || ! grep -q 'another_undeclared' err.txt; then
    fail "main.c bad.c worse.c -o prog did not report both: $(cat err.txt)"
fi
rc=0; "$CCC" main.c a.c c.c -o prog 2>err.txt || rc=$?
[[ $rc -eq 0 && -x prog ]] || fail "a link of good inputs: rc $rc: $(cat err.txt)"
rm -f prog

# The same objects as GCC leaves.
if command -v "$GCC" >/dev/null 2>&1; then
    reset
    "$GCC" -c a.c bad.c c.c 2>/dev/null || true
    theirs=$(objects)
    reset
    "$CCC" -c a.c bad.c c.c 2>/dev/null || true
    [[ $(objects) == "$theirs" ]] || fail "-c leaves '$(objects)', $GCC leaves '$theirs'"
else
    echo "note: $GCC not installed; the rows above stand without the oracle" >&2
fi

if [[ $fail -ne 0 ]]; then
    exit 1
fi
echo "every-input: PASS"
