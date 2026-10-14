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
    local cc=$1 file=$2
    shift 2
    "$cc" "$@" -S -o /dev/null "$file" 2>&1 |
        awk -v want="$file" '
            index($0, want ":") == 1 {
                split($0, parts, ":")
                print parts[2] ":" parts[3]
                exit
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
    got=$(first_position "$CCC" "$want_file" "$@" || true)
    want=$(first_position "$GCC" "$want_file" "$@" || true)
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
