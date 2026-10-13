#!/usr/bin/env bash
# AUD-DRV-1 — `-o -` means standard output (and never a file called `-`).
#
# GCC streams text to stdout for `-S`, `-E` and the `-M*` dependency printers,
# and rejects `-o -` for `-c` (its assembler cannot open a BFD on stdout). lccc
# created a *file* named `-` in every mode, so
#
#     lccc -S -o - t.c | grep ret        # silently empty, `-` left behind
#     lccc -E -o - t.c > t.i             # t.i empty, `-` left behind
#
# broke every pipeline and build-system probe that relies on the convention.
# The oracle for each row below was measured against GCC 12.2 locally:
#
#   gcc -S -o - x.c        -> assembly on stdout, rc 0
#   gcc -E -o - x.c        -> preprocessed source on stdout, rc 0
#   gcc -M -o - x.c        -> make rule on stdout, rc 0
#   gcc -M -MF - x.c       -> make rule on stdout, rc 0
#   gcc -c -o - x.c        -> rc 1 ("can't open a bfd on stdout")
#   gcc -S -o - x.c y.c    -> rc 1 ("cannot specify '-o' ... with multiple files")
#   gcc -MD -E -o - x.c    -> preprocessed on stdout AND a dep file named `-.d`
set -euo pipefail

CCC=${CCC:-./target/fastbuild/lccc}
if [[ ! -x $CCC ]]; then
    echo "SKIP: $CCC not built" >&2
    exit 1
fi
CCC=$(cd "$(dirname -- "$CCC")" && pwd)/$(basename -- "$CCC")
GCC=${GCC:-gcc}

tmp=$(mktemp -d "${TMPDIR:-/tmp}/lccc-o-stdout.XXXXXX")
trap 'rm -rf "$tmp"' EXIT
cd "$tmp"

fail() { echo "FAIL: $*" >&2; exit 1; }

printf 'int f(void){return 42;}\n' >x.c
printf 'int g(void){return 7;}\n' >y.c

# 1. -S -o - : assembly on stdout, nothing on disk called `-`
"$CCC" -S -o - x.c >s.out 2>s.err || { cat s.err >&2; fail "lccc -S -o - failed"; }
[[ -s s.out ]] || fail "-S -o - wrote nothing to stdout"
grep -q '^f:' s.out || fail "-S -o - stdout does not contain the function body"
[[ ! -e ./- ]] || fail "-S -o - created a file named '-'"
[[ ! -e ./-.d ]] || fail "-S -o - left a dep file behind without -MD"

# 2. -E -o - : preprocessed source on stdout
"$CCC" -E -o - x.c >e.out 2>e.err || { cat e.err >&2; fail "lccc -E -o - failed"; }
[[ -s e.out ]] || fail "-E -o - wrote nothing to stdout"
grep -q 'int f(void)' e.out || fail "-E -o - stdout is not the preprocessed source"
[[ ! -e ./- ]] || fail "-E -o - created a file named '-'"

# 3. -M -o - and -M -MF - : dependency rules on stdout
"$CCC" -M -o - x.c >m1.out || fail "lccc -M -o - failed"
grep -Eq '^[^ ]*\.o: .*x\.c' m1.out || { cat m1.out >&2; fail "-M -o - stdout is not a make rule"; }
"$CCC" -M -MF - x.c >m2.out || fail "lccc -M -MF - failed"
grep -Eq '^[^ ]*\.o: .*x\.c' m2.out || { cat m2.out >&2; fail "-M -MF - stdout is not a make rule"; }
[[ ! -e ./- ]] || fail "-M ... left a file named '-'"

# 4. -c -o - : rejected up front (GCC errors too), and no `-` file
if "$CCC" -c -o - x.c >c.out 2>c.err; then
    fail "-c -o - was accepted (GCC rejects it)"
fi
grep -qi 'standard output' c.err || { cat c.err >&2; fail "-c -o - rejection message is not actionable"; }
[[ ! -e ./- ]] || fail "-c -o - created a file named '-'"

# 5. multiple inputs with -o - : rejected, like GCC
if "$CCC" -S -o - x.c y.c >d.out 2>d.err; then
    fail "multiple inputs with -o - were accepted (GCC rejects them)"
fi
[[ ! -s d.out ]] || fail "multiple-input rejection still printed output"
[[ ! -e ./- ]] || fail "multiple inputs with -o - created a file named '-'"

# 6. -MD -E -o - : stdout stream plus GCC's derived `-.d` dep file
"$CCC" -MD -E -o - x.c >md.out 2>md.err || { cat md.err >&2; fail "lccc -MD -E -o - failed"; }
grep -q 'int f(void)' md.out || fail "-MD -E -o - stdout lost the preprocessed source"
[[ -s ./-.d ]] || fail "-MD -E -o - did not write the derived '-.d' dep file (GCC writes it)"

# 7. -dM -E -o - : macro definitions on stdout (GCC's -dM printer).
"$CCC" -dM -E -o - x.c >dm.out 2>dm.err || { cat dm.err >&2; fail "lccc -dM -E -o - failed"; }
grep -q '^#define __STDC__' dm.out || fail "-dM -E -o - stdout lost the macro definitions"
[[ ! -e ./- ]] || fail "-dM -E -o - created a file named '-'"

# 8. -MF - with -c: the dependency rule streams to stdout while the object goes
#    to a real path.  This is the compile-and-assemble driver path, not the -M
#    printer, so it pins the `-MF -` contract where build systems use it.
"$CCC" -c x.c -o /dev/null -MMD -MF - >mf.out 2>mf.err || { cat mf.err >&2; fail "lccc -c -MMD -MF - failed"; }
grep -q 'x\.c' mf.out || fail "-MF - did not stream the dependency rule"

# 9. Oracle agreement when a system GCC is available: the same rows must have
#    the same accept/reject status, so a future refactor cannot drift back to
#    "writes a file named -" for the rows GCC streams.
if command -v "$GCC" >/dev/null 2>&1; then
    rm -f ./- ./-.d
    gcc -S -o - x.c >gs.out 2>gs.err || fail "oracle: gcc -S -o - failed; environment problem"
    gcc -c -o - x.c >/dev/null 2>gc.err || true
    if "$CCC" -S -o - x.c >cs.out 2>/dev/null && gcc -S -o - x.c >gs2.out 2>/dev/null; then
        # Both stream assembly: the *presence* of the body must agree. (The
        # instruction selection itself is compared elsewhere.)
        grep -q '^f:' cs.out || fail "oracle: lccc stdout lost the function body"
        grep -q '^f:' gs2.out || fail "oracle: gcc stdout lost the function body"
    else
        fail "oracle: -S -o - acceptance disagrees with gcc"
    fi
    if gcc -c -o - x.c >/dev/null 2>&1; then
        fail "oracle: this gcc accepts -c -o -; the gate's premise is wrong"
    fi
fi

# 10. Link mode is the documented GCC *wart*, pinned as parity: GCC 12 (and
#     every release measured for this gate) does NOT stream an executable —
#     `gcc -o - m.c` exits 0 and creates a file literally named `-`.  lccc
#     matches that byte for byte (rc 0, file `-`, empty stdout), which is why
#     the driver must not "helpfully" reject `-o -` here: a build script that
#     passes `-o -` to a link step would then fail where GCC succeeds.
if command -v "$GCC" >/dev/null 2>&1; then
    rm -f ./- ./-.d
    printf 'int main(void){return 0;}\n' >mm.c
    gcc -o - mm.c >gcc_link.out 2>gcc_link.err
    gcc_rc=$?
    gcc_streamed=$(wc -c <gcc_link.out)
    rm -f ./-
    if "$CCC" -o - mm.c >lccc_link.out 2>lccc_link.err; then
        lccc_rc=0
    else
        lccc_rc=$?
    fi
    lccc_streamed=$(wc -c <lccc_link.out)
    [[ $lccc_rc = "$gcc_rc" ]] || fail "link-mode -o -: lccc rc=$lccc_rc, gcc rc=$gcc_rc"
    [[ $lccc_streamed = "$gcc_streamed" ]] || fail "link-mode -o -: lccc wrote $lccc_streamed bytes to stdout, gcc $gcc_streamed"
    [[ -x ./- ]] || fail "link-mode -o -: lccc did not produce the literal '-' executable GCC produces"
fi

echo "PASS: -o - streams to stdout for -S/-E/-M/-dM/-MF, is rejected for -c and multi-input, and leaves no file named '-' (link mode keeps GCC's literal-'-' behaviour)"
