#!/usr/bin/env bash
# AUD-DRV-1 — `-o -` means standard output (and never a file called `-`).
#
# GCC streams text to stdout for `-S`, `-E` and the `-M*` dependency printers,
# and rejects `-o -` for `-c` and for a link (its assembler cannot open a BFD on
# stdout). lccc created a *file* named `-` in every mode, so
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
#   gcc -dM -E -o - x.c    -> macro definitions on stdout, rc 0
#   gcc -c -o - x.c        -> rc 1 ("can't open a bfd on stdout")
#   gcc -S -o - x.c y.c    -> rc 1 ("cannot specify '-o' ... with multiple files")
#   gcc -MD -E -o - x.c    -> preprocessed on stdout AND a dep file named `-.d`
#
# This is the single gate for that contract: the merge with upstream PR #815
# left two gates for `-o -` (this one and upstream's) asserting two different
# rejection messages for the same `-c -o -`, so exactly one had to survive, and
# the compiler behind them now has one guard. This gate is the superset — the
# streaming rows through a real PIPE
# and the executable-mode row are upstream's, the multi-input row, the derived
# `-.d` row and the system-GCC oracle row are this branch's — and the compiler
# now has exactly one guard behind it (`Pipeline::validate_output_destination`).
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

fail=0
fail() { echo "FAIL: $*" >&2; fail=1; }
note_die() { echo "FAIL: $*" >&2; exit 1; }

printf 'int f(void){return 42;}\n' >x.c
printf 'int g(void){return 7;}\n' >y.c

# 1. -S -o - : assembly on stdout, nothing on disk called `-`
"$CCC" -S -o - x.c >s.out 2>s.err || { cat s.err >&2; note_die "lccc -S -o - failed"; }
[[ -s s.out ]] || fail "-S -o - wrote nothing to stdout"
grep -q '^f:' s.out || fail "-S -o - stdout does not contain the function body"
[[ ! -e ./- ]] || fail "-S -o - created a file named '-'"
[[ ! -e ./-.d ]] || fail "-S -o - left a dep file behind without -MD"

# 2. The same row through a real pipe: this is how consumers use it (a redirect
#    to a file can still pass if the compiler only *appends* on close).
piped=$("$CCC" -S -o - x.c | grep -c '^f:')
[[ $piped = 1 ]] || fail "-S -o - did not stream assembly through a pipe (matches: $piped)"

# 3. -E -o - : preprocessed source on stdout
"$CCC" -E -o - x.c >e.out 2>e.err || { cat e.err >&2; note_die "lccc -E -o - failed"; }
[[ -s e.out ]] || fail "-E -o - wrote nothing to stdout"
grep -q 'int f(void)' e.out || fail "-E -o - stdout is not the preprocessed source"
[[ ! -e ./- ]] || fail "-E -o - created a file named '-'"

# 4. -M -o - and -M -MF - : dependency rules on stdout
"$CCC" -M -o - x.c >m1.out || note_die "lccc -M -o - failed"
grep -Eq '^[^ ]*\.o: .*x\.c' m1.out || { cat m1.out >&2; fail "-M -o - stdout is not a make rule"; }
"$CCC" -M -MF - x.c >m2.out || note_die "lccc -M -MF - failed"
grep -Eq '^[^ ]*\.o: .*x\.c' m2.out || { cat m2.out >&2; fail "-M -MF - stdout is not a make rule"; }
[[ ! -e ./- ]] || fail "-M ... left a file named '-'"

# 5. -dM -E -o - : the macro dump goes to stdout too.
macros=$("$CCC" -dM -E -o - x.c | grep -c '^#define __STDC__')
[[ $macros = 1 ]] || fail "-dM -E -o - did not stream macro definitions (matches: $macros)"
[[ ! -e ./- ]] || fail "-dM -E -o - created a file named '-'"

# 6. -c -o - : rejected up front (GCC errors too), with an actionable message,
#    and no `-` file.
if "$CCC" -c -o - x.c >c.out 2>c.err; then
    fail "-c -o - was accepted (GCC rejects it)"
fi
grep -qi 'standard output' c.err || { cat c.err >&2; fail "-c -o - rejection message is not actionable"; }
[[ ! -e ./- ]] || fail "-c -o - created a file named '-'"

# 7. The link row: an executable cannot be piped either.
if "$CCC" -o - x.c >x.out 2>x.err; then
    fail "-o - on a link was accepted (GCC rejects it: no BFD on stdout)"
fi
grep -qi 'standard output' x.err || { cat x.err >&2; fail "-o - link rejection message is not actionable"; }
[[ ! -e ./- ]] || fail "linking with -o - created a file named '-'"
[[ ! -e ./a.out ]] || fail "linking with -o - still wrote a.out"

# 8. multiple inputs with -o - : rejected, like GCC
if "$CCC" -S -o - x.c y.c >d.out 2>d.err; then
    fail "multiple inputs with -o - were accepted (GCC rejects them)"
fi
[[ ! -s d.out ]] || fail "multiple-input rejection still printed output"
[[ ! -e ./- ]] || fail "multiple inputs with -o - created a file named '-'"

# 9. -MD -E -o - : stdout stream plus GCC's derived `-.d` dep file
"$CCC" -MD -E -o - x.c >md.out 2>md.err || { cat md.err >&2; note_die "lccc -MD -E -o - failed"; }
grep -q 'int f(void)' md.out || fail "-MD -E -o - stdout lost the preprocessed source"
[[ -s ./-.d ]] || fail "-MD -E -o - did not write the derived '-.d' dep file (GCC writes it)"

# 10. Negative control: a real path still writes a real file, so the fix cannot
#     have turned every -o into stdout.
"$CCC" -S -o real.s x.c || note_die "-S -o real.s failed"
[[ -s real.s ]] || fail "-S -o real.s did not write the assembly file"

# 11. Oracle agreement when a system GCC is available: the same rows must have
#     the same accept/reject status, so a future refactor cannot drift back to
#     "writes a file named -" for the rows GCC streams.
if command -v "$GCC" >/dev/null 2>&1; then
    rm -f ./- ./-.d
    gcc -S -o - x.c >gs.out 2>gs.err || note_die "oracle: gcc -S -o - failed; environment problem"
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
    if gcc -o - x.c >/dev/null 2>&1; then
        fail "oracle: this gcc links to stdout; the gate's premise is wrong"
    fi
fi

[[ $fail = 0 ]] || { echo 'o-stdout: FAIL'; exit 1; }
echo "PASS: -o - streams to stdout for -S/-E/-M/-dM (pipes included), is rejected for -c and for a link, and leaves no file named '-'"
