#!/usr/bin/env bash
# AUD-POS-1: a diagnostic's file:line:col is the compiler's contract with
# everything that reads it -- the human, the editor that jumps to it, the build
# log parser.  After a `#include`, LCCC used to report every position in the
# rest of the file one line too high: the preprocessor wrote a
# return-to-parent line marker (which already accounts for the directive's own
# line) and then emitted a newline "to preserve line numbering", so a phantom
# blank line sat between the marker and the parent's next line.
#
# The reference is GCC doing the same thing to the same file: whatever line and
# column it blames, LCCC must blame.  Positions only -- the wording and the
# caret art are the compiler's own.
set -euo pipefail

# The cases run inside a scratch directory, so both compilers must be
# absolute before the cd below.
CCC=$(realpath "${CCC:-./target/fastbuild/lccc}")
GCC=$(command -v "${CCC_GCC:-gcc}")
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
cd "$tmp"

pass=0
fail=0

# first-position <compiler> <file> [flags...] -> "line:col" of the first
# diagnostic that names <file>, or "-" when the compiler reported none.
first_position() {
    local cc=$1 input=$2 file=$3
    shift 3
    "$cc" "$@" -S -o /dev/null "$input" 2>&1 |
        awk -v want="$file" '
            index($0, want ":") == 1 {
                split($0, parts, ":")
                if (parts[2] ~ /^[0-9]+$/ && parts[3] ~ /^[0-9]+$/) {
                    print parts[2] ":" parts[3]
                    exit
                }
            }
            END { if (NR == 0) print "-" }' || true
}

# Every case is checked in both directions: LCCC must agree with GCC, and the
# case must actually produce a diagnostic (an oracle gate that cannot fail is
# no gate).  The expected position is spelled out as well, so a case whose
# fixture no longer fails is a hard error rather than a silent agreement on
# "no diagnostic".
row() {
    local name=$1 expected=$2 file=$3 want_file=$4
    shift 4
    local got want
    got=$(first_position "$CCC" "$file" "$want_file" "$@" || true)
    want=$(first_position "$GCC" "$file" "$want_file" "$@" || true)
    if [ -z "$got" ] || [ -z "$want" ]; then
        echo "FAIL $name: no diagnostic (lccc='${got:-<none>}' gcc='${want:-<none>}')" >&2
        fail=$((fail + 1))
        return 0
    fi
    if [ "$got" != "$want" ]; then
        echo "FAIL $name: lccc blames $got, gcc blames $want" >&2
        fail=$((fail + 1))
        return 0
    fi
    if [ "$expected" != "-" ] && [ "$got" != "$expected" ]; then
        echo "FAIL $name: lccc blames $got, fixture documents $expected" >&2
        fail=$((fail + 1))
        return 0
    fi
    pass=$((pass + 1))
}

printf 'int h;\n' > h.h
printf 'int n;\n' > n.h
printf 'int a;\nint bad = ;\n' > bad.h
printf '#include "h.h"\nint n;\n' > nested.h
printf 'int outer;\n#include "nested.h"\n' > n.h

printf '#include <stddef.h>\nint bad = ;\n' > inc_first.c
printf 'int a;\n#include <stddef.h>\nint bad = ;\n' > inc_middle.c
printf '#include <stddef.h>\n#include <stdint.h>\nint bad = ;\n' > inc_twice.c
printf '#include "h.h"\nint bad = ;\n' > inc_local.c
printf '#include "n.h"\nint bad = ;\n' > inc_nested.c
printf '#include "bad.h"\nint ok;\n' > inc_error_inside.h.c
printf 'int a;\nint bad[;];\n' > no_include.c
printf '#include <stddef.h>\nint a;\nint b;\nint c;\nint d;\nint bad = ;\n' > inc_far.c
printf '#include <stddef.h>\nint a;\n#define M(x) (x)\nint b = M(1);\nint bad = ;\n' > inc_macro.c
printf '#include <stddef.h>\nint a;\n#pragma GCC diagnostic ignored "-Wunused"\nint bad = ;\n' > inc_pragma.c

row "error after include"            2:11 inc_first.c  inc_first.c
row "include on line 2"              3:11 inc_middle.c inc_middle.c
row "two includes"                   3:11 inc_twice.c  inc_twice.c
row "local include"                  2:11 inc_local.c  inc_local.c
row "nested include"                 2:11 inc_nested.c inc_nested.c
row "error inside the header"        2:11 inc_error_inside.h.c bad.h
row "no include at all"              2:9  no_include.c  no_include.c
row "far below an include"           6:11 inc_far.c    inc_far.c
row "after a #define past the include" 5:11 inc_macro.c inc_macro.c
row "after a #pragma past the include" 4:11 inc_pragma.c inc_pragma.c

# The conditional AST retains the colon token rather than scanning backwards
# through preprocessed text. A #line marker between ':' and the third operand
# must not move the diagnostic to the condition or the remapped operand.
cat > conditional_line.c <<'C'
void f(int c, int *p) { (void)(c ? p :
#line 100 "virtual.c"
1); }
C
cat > conditional_nested.c <<'C'
void f(int c, int *p) { (void)(c ? p : (c ? p : 1)); }
C
cat > conditional_gnu.c <<'C'
void f(int c, int *p) { (void)(c ?: p); }
C
cat > conditional_macro.c <<'C'
#define ID(x) x
void f(int c, int *p) { (void)(c ? p : ID(1)); }
C
row "conditional across line marker" - conditional_line.c conditional_line.c -std=gnu17
row "nested conditional colon" - conditional_nested.c conditional_nested.c -std=gnu17
row "GNU conditional colon" - conditional_gnu.c conditional_gnu.c -std=gnu17
row "conditional macro operand" - conditional_macro.c conditional_macro.c -std=gnu17

# C11 6.8.4.2p3 constraints on one switch: no two case constants with the
# same value after conversion to the promoted controlling type, at most one
# default.  LCCC used to accept all of these silently (the later label won or
# lost depending on the dispatch order); GCC blames the later label's
# `case`/`default` keyword.
cat > case_duplicate.c <<'C'
int f(int x) {
    switch (x) { case 1: return 1; case 2: return 2; case 1: return 3; }
    return 0;
}
C
cat > case_in_range.c <<'C'
int f(unsigned char x) {
    switch (x) { case 1 ... 5: return 1; case 3: return 2; }
    return 0;
}
C
cat > case_range_overlap.c <<'C'
int f(long x) {
    switch (x) { case 9: return 1; case 2 ... 10: return 2; }
    return 0;
}
C
cat > case_converted_duplicate.c <<'C'
int f(unsigned x) {
    switch (x) { case -1: return 1; case 0xffffffffu: return 2; }
    return 0;
}
C
cat > default_twice.c <<'C'
int f(int x) {
    switch (x) { default: return 0; case 1: return 1; default: return 2; }
}
C
row "duplicate case value"            2:54 case_duplicate.c case_duplicate.c
row "case inside an earlier range"    2:42 case_in_range.c case_in_range.c
row "range over an earlier case"      2:36 case_range_overlap.c case_range_overlap.c
row "duplicate after conversion"      2:37 case_converted_duplicate.c case_converted_duplicate.c
row "second default label"            2:55 default_twice.c default_twice.c

# The same rules must not reject a valid switch: 128-bit cases that differ
# only above bit 63, a nested switch reusing its parent's values, and the
# unsigned images of distinct negative values.
cat > case_distinct.c <<'C'
long f(__int128 v, int x, unsigned char c) {
    switch (v) {
    case 0: return 1;
    case (__int128)1 << 64: return 2;
    case -1: return 3;
    case 0xffffffffffffffffull: return 4;
    }
    switch (x) {
    case 1: switch (x) { case 1: return 5; default: break; } break;
    case 2 ... 4: return 6;
    }
    switch (c) { case 255: return 7; case -2: return 8; }
    return 0;
}
C
if "$CCC" -S -o /dev/null case_distinct.c 2>case_distinct.err && [ ! -s case_distinct.err ]; then
    pass=$((pass + 1))
else
    echo "FAIL distinct case values: lccc rejected or warned" >&2
    sed 's/^/      /' case_distinct.err >&2
    fail=$((fail + 1))
fi

# The include line itself is still blamed for a missing header -- the phantom
# line was never the "not found" diagnostic's problem, and pinning it here keeps
# the fix from turning into "stop reporting includes at all".
printf '#include "definitely_missing_header.h"\nint ok;\n' > missing.c
got=$("$CCC" -S -o /dev/null missing.c 2>&1 | grep -c 'definitely_missing_header.h' || true)
if [ "$got" -lt 1 ]; then
    echo "FAIL missing-include: no diagnostic mentions the missing header" >&2
    fail=$((fail + 1))
else
    pass=$((pass + 1))
fi

# __LINE__ is the semantic twin of the same bookkeeping: a phantom line between
# the include and the next line must not shift the macro either.
printf '#include "h.h"\nint line_here = __LINE__;\n' > line.c
got=$("$CCC" -E line.c 2>/dev/null | grep -c 'int line_here = 2;' || true)
want=$("$GCC" -E line.c 2>/dev/null | grep -c 'int line_here = 2;' || true)
if [ "$got" -lt 1 ] || [ "$want" -lt 1 ]; then
    echo "FAIL __LINE__ after include: lccc=$got gcc=$want" >&2
    fail=$((fail + 1))
else
    pass=$((pass + 1))
fi

echo "diagnostic positions: $pass passed, $fail failed"
[ "$fail" -eq 0 ]
