#!/usr/bin/env bash
# check_dash_output.sh — `-o -` means stdout where GCC says it does (AUD-DRV-1).
#
# THE GAP THIS PINS
# -----------------
# `lccc -S -o - x.c` used to CREATE A FILE LITERALLY NAMED `-` in the working
# directory, silently discarding the assembly from any pipeline's point of
# view (`lccc -S -o - x.c | grep ...` got nothing).  Build systems, distro
# helper scripts and configure probes use `-o -` as a stream convention.
#
# The GCC 14.2 oracle, measured cell by cell (this file reproduces the table):
#
#   invocation            GCC behavior                     old LCCC behavior
#   --------------------- -------------------------------- -------------------
#   -S -o -               asm -> stdout, no file `-`        file named `-`
#   -E -o -               preprocessed -> stdout            file named `-`
#   -c -o -               fails (assembler: "can't open     file named `-`
#                          a bfd on stdout -")
#   -E -o - -MF -         preprocessed + deps -> stdout,    deps lost into the
#                          dep target is the derived         `-` file
#                          object name (x.o: x.c)
#   -E -o - -MMD          NO .d file created (there is no   (n/a)
#                          output file name to derive from)
#   -MF -                 dep rule -> stdout                file named `-`
#   (full link) -o -      REJECTED: non-zero exit, stderr   same (kept, pinned
#                          diagnostic, no file named `-`     so it stays;
#                          (gcc 14.2 measured 2026-10-14;    S01 row said
#                          the S01 "file named -" reading    "file named -"
#                          was a mis-measurement)            — corrected)
#
# LCCC's integrated assembler cannot reproduce GCC's exact `-c` failure text
# (that text is bfd's), so the `-c` row pins the CONTRACT: non-zero exit, a
# diagnostic on stderr, no file named `-`, and no object silently written.
#
# The dependency rows are load-bearing for the kernel build: fixdep expects
# the .d file to exist next to the object, and the -E path with an explicit
# -MF is exactly how glibc/kernel linker-script preprocessing runs.
set -uo pipefail

ROOT=$(cd "$(dirname "$0")/../.." && pwd)
# The directory the gate was invoked from, captured before the `cd "$WORK"`
# below: the relative-CCC self-test has to reproduce the caller's spelling.
INVOKED_FROM=$PWD
LCCC=${CCC:-$ROOT/target/fastbuild/lccc}
# Resolve a RELATIVE $CCC before the `cd` below.  Dozens of workflow steps spell
# the compiler `env: CCC=target/fastbuild/lccc`, and this gate changes directory:
# the relative spelling then names a path inside the scratch tree, every probe
# fails with "no such file", and the run reads like a codegen regression instead
# of a path bug.  A bare command name stays a name, so a PATH lookup still works.
if [[ $LCCC == */* && $LCCC != /* ]]; then
    LCCC=$PWD/$LCCC
fi
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
cd "$WORK"

fail=0
# step <name> <expression>: the expression is a shell TEST evaluated in this
# shell (eval), so it can reference $out/$rc and use [[ ]]/(( )) naturally.
step() {
    local name=$1 expr=$2
    if eval "$expr"; then
        printf 'PASS %s\n' "$name"
    else
        printf 'FAIL %s\n' "$name"
        fail=1
    fi
}

printf 'int f(int x){ return x*3; }\nint main(void){ return f(1)==3?0:1; }\n' > x.c

# --- self-test ----------------------------------------------------------------
# A fake compiler with the OLD (buggy) behavior: every stdout row must fail,
# which is what makes this gate a mutation test rather than a tautology.
if [[ ${1:-} == --self-test ]]; then
    cat > fake-old-lccc <<'EOF'
#!/usr/bin/env bash
# Old LCCC: -o - is just a filename. Everything lands in ./- ; nothing to stdout.
out=""; mode=""; src=""; mffile=""
while (( $# )); do case $1 in
  -o) out=$2; shift 2;;
  -S) mode=S; shift;;
  -E) mode=E; shift;;
  -c) mode=c; shift;;
  -MF) mffile=$2; shift 2;;
  -MMD|-M) shift;;
  *) [[ -z $src && $1 != -* ]] && src=$1; shift;;
esac; done
src=${src:-x.c}
printf '\t.text\n\t.globl f\n' > "$out"          # asm text into the dash file
[[ -n $mffile ]] && printf '%s.o: %s\n' "${src%.c}" "$src" > "$mffile"
exit 0
EOF
    chmod +x fake-old-lccc
    # Re-invoke by ABSOLUTE path: the gate body already cd'd into its workdir,
    # so a relative $0 would no longer resolve. The output is CAPTURED before
    # grepping: `grep -q` exits on the first match, the gate would die of
    # SIGPIPE mid-stream, and `set -o pipefail` would flip that into a
    # pass-looking failure of the pipeline.
    old_out=$(CCC="$PWD/fake-old-lccc" bash "$ROOT/tests/regression/check_dash_output.sh")
    if grep -q '^FAIL ' <<<"$old_out"; then
        echo "PASS self-test (old behavior is rejected)"
        exit 0
    fi
    echo "FAIL self-test (gate does not bite on the old behavior)"
    exit 1
fi
# --- /self-test ---------------------------------------------------------------

rm -f ./- ./x.d ./deps.txt

# Row 1: -S -o - streams assembly to stdout and creates no file.
out=$("$LCCC" -S -o - x.c 2>"$WORK/err")
step "-S -o -: assembly on stdout"       'grep -q "f:" <<<"$out"'
step "-S -o -: no file named - created"  '[[ ! -e ./- ]]'

# Row 2: -E -o - streams the preprocessed translation unit to stdout.
out=$("$LCCC" -E -o - x.c 2>"$WORK/err")
step "-E -o -: preprocessed on stdout"   'grep -q "int f" <<<"$out"'
step "-E -o -: no file named - created"  '[[ ! -e ./- ]]'

# Row 3: -c -o - fails cleanly (object output cannot go to stdout).
rm -f ./-
"$LCCC" -c -o - x.c >"$WORK/out" 2>"$WORK/err"; rc=$?
step "-c -o -: non-zero exit"            '(( rc != 0 ))'
step "-c -o -: diagnostic on stderr"     'grep -qiE "error|stdout" "$WORK/err"'
step "-c -o -: no file named - created"  '[[ ! -e ./- ]]'

# Row 4: -E -o - -MF -: preprocessed output AND dep rule both on stdout,
# with the GCC-derived rule target (the object name, not the dash).
out=$("$LCCC" -E -o - -MF - -MMD x.c 2>"$WORK/err")
step "-E -o - -MF -: preprocessed on stdout" 'grep -q "int f" <<<"$out"'
step "-E -o - -MF -: dep rule on stdout"     'grep -q "^x\.o: x\.c" <<<"$out"'

# Row 5: -E -o - -MMD creates NO derived .d file (GCC: measured none).
rm -f ./- ./x.d
"$LCCC" -E -o - -MMD x.c >/dev/null 2>&1
step "-E -o - -MMD: no .d created"       '[[ ! -e ./x.d ]]'

# Row 6: -MF - (no -o) sends the dep rule to stdout.
rm -f ./- ./x.d
out=$("$LCCC" -E -MF - -MMD x.c 2>"$WORK/err")
step "-MF -: dep rule on stdout"         'grep -q "^x\.o: x\.c" <<<"$out"'

# Row 7: an explicit -MF <file> still lands next to stdout output.
rm -f ./deps.txt
"$LCCC" -E -o - -MF deps.txt -MMD x.c >/dev/null 2>&1
step "-MF <file>: dep file written"      'grep -q "^x\.o: x\.c" ./deps.txt'

# Row 8: full-link -o - is REJECTED, exactly like GCC 14.2 (measured: exit 1,
# linker diagnostic on stderr, no file named `-`; the assembler driver cannot
# open a BFD on stdout and the collect2 chain fails the same way).  The S01
# measurement behind the old row was wrong — re-measured 2026-10-14.
rm -f ./-
"$LCCC" -o - x.c >"$WORK/out" 2>"$WORK/err"; rc=$?
step "link -o -: non-zero exit (GCC parity)" '(( rc != 0 ))'
step "link -o -: diagnostic on stderr"       '[[ -s "$WORK/err" ]]'
step "link -o -: no file named - created"    '[[ ! -e ./- ]]'

# Row 9 (the pipeline the audit names): -S -o - must work through a pipe.
n=$("$LCCC" -S -o - x.c 2>/dev/null | grep -c 'f:')
step "-S -o -: survives a pipe"          '(( n > 0 ))'

# Row 10 (red-team follow-up): the -x assembler / .S preprocessing path had
# the SAME dash bug the C path had — it wrote a file named `-`.  Pin it.
printf '\t.text\n\t.globl f\nf:\n\tret\n' > sprobe.s
rm -f ./-
out=$("$LCCC" -x assembler -E -o - sprobe.s 2>"$WORK/err")
step "-x assembler -E -o -: asm on stdout"  'grep -q "globl" <<<"$out"'
step "-x assembler -E -o -: no dash file"   '[[ ! -e ./- ]]'
out=$("$LCCC" -E -o - sprobe.s 2>"$WORK/err")
step ".S -E -o -: asm on stdout"            'grep -q "globl" <<<"$out"'
step ".S -E -o -: no dash file"             '[[ ! -e ./- ]]'

# Rows 19-21 (GCC-parity for assembly dep rules, measured gcc 14.2): the .S
# -E path writes dep rules exactly like the C -E path — the rule target is
# the would-be OBJECT (s1.o:), never the dash, and the derived .d name
# follows the -o value when there is one.
printf '\t.text\n\t.globl f\nf:\n\tret\n' > s1.S
out=$("$LCCC" -E -o - -MF - -MMD s1.S 2>"$WORK/err")
step ".S -E -o - -MF -: rule on stdout"   'grep -q "^s1\.o: s1\.S" <<<"$out"'
out=$("$LCCC" -E -MF - -MMD s1.S 2>"$WORK/err")
step ".S -E -MF -: rule on stdout"        'grep -q "^s1\.o: s1\.S" <<<"$out"'
rm -f ./s1.d
"$LCCC" -E -MD s1.S >/dev/null 2>&1
step ".S -E -MD: derived s1.d"            'grep -q "^s1\.o: s1\.S" ./s1.d'

# --- relative-CCC contract ----------------------------------------------------
# Exercise the spelling the pipeline actually uses, from the directory it uses
# it in.  This is the regression the replay of `Codegen and runtime contracts`
# hit: with `CCC=target/fastbuild/lccc` the gate still passed on a developer box
# only because nothing there exports CCC, and CI only passes because that one
# step happens not to set it either -- the next person to add `env: CCC` to it
# would have seen nine "codegen regressions" that were all one missing `$PWD`.
if [[ -z ${DASH_OUTPUT_RELATIVE:-} && -z ${CCC:-} ]]; then
    # Run from the repository root, which is how every workflow step and
    # `scripts/ci_local.sh` invoke a gate; the relative spelling is then
    # `./target/fastbuild/lccc`.
    if [[ $ROOT == "$INVOKED_FROM" ]]; then
        relative_ccc=./target/fastbuild/lccc
    elif [[ $ROOT == "$INVOKED_FROM"/* ]]; then
        relative_ccc=${ROOT#"$INVOKED_FROM"/}/target/fastbuild/lccc
    else
        relative_ccc=""
    fi
    # ...and from the directory the caller used, because that is what makes the
    # spelling relative.
    if [[ -n $relative_ccc ]] \
            && ! ( cd "$INVOKED_FROM" && DASH_OUTPUT_RELATIVE=1 CCC=$relative_ccc \
                   bash "$ROOT/tests/regression/check_dash_output.sh" ) >"$WORK/relative.log" 2>&1; then
        printf 'FAIL self-test (a relative CCC breaks after cd): %s\n' \
            "$(tail -3 "$WORK/relative.log" | tr '\n' ' ')"
        fail=1
    elif [[ -n $relative_ccc ]]; then
        echo "PASS self-test (a relative CCC survives the workdir change)"
    fi
fi

exit $fail
