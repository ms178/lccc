#!/usr/bin/env bash
# Translation phases 2 and 3: a physical newline inside a block comment
# cannot terminate a directive when a later splice joins the closing line.
set -euo pipefail
root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
CCC=${CCC:-$root/target/fastbuild/lccc}
GCC=${GCC:-gcc-16}
tmp=$(mktemp -d)
trap 'rm -rf -- "$tmp"' EXIT
cat > "$tmp/ordinary.c" <<'SRC'
#define VALUE (1 + \
 /* this comment spans
    multiple physical lines */\
 (2))
#define SPLIT (10 + \
 /\
* an opening delimiter formed by splicing
   is still a comment */\
 (20))
#if (1 + \
 /* preprocessor expressions have
    the same ordering */\
 (2)) != 3
#error broken preprocessing
#endif
#line 100
static int source_line = __LINE__;
int main(void) { return VALUE != 3 || SPLIT != 30 || source_line != 100; }
SRC
# No adjacent slash-star anywhere in the raw source: phase 2 constructs it
# across multiple splices. The conservative guard must still detect it.
cat > "$tmp/split-only.c" <<'SRC'
#define VALUE (1 + \
 /\
\
* comment
  end */\
 (2))
int main(void) { return VALUE != 3; }
SRC
# A C23 digit separator may straddle either side of a splice. A physical-line
# lexer that mistakes it for a character-literal quote can miss the comment.
cat > "$tmp/c23.c" <<'SRC'
#define X (1\
'0 + \
 /* block
 end */\
 2)
#define Y (1'\
0 + \
 /* block
 end */\
 3)
int main(void) { return X != 12 || Y != 13; }
SRC
# Keep tracking the directive *after* the comment closes, including a
# digraph formed across a splice and a comment before the introducer.
cat > "$tmp/directive-chain.c" <<'SRC'
%\
:define VALUE (1 + \
 /* first
 second */\
 2 + \
 3 + \
 4)
/* prefix
 */ #define OTHER (2 + \
 /* first
 second */\
 3 + \
 4)
int main(void) { return VALUE != 10 || OTHER != 9; }
SRC
# The escape immediately before the first splice applies to the quote on
# the next physical line; a later splice must not become a string newline.
cat > "$tmp/escape.c" <<'SRC'
const char *s = "a\\
" b \
 c";
int main(void) {
    const char expected[] = { 'a', '"', ' ', 'b', ' ', ' ', 'c', 0 };
    for (int i = 0; i < 8; ++i) if (s[i] != expected[i]) return 1;
    return 0;
}
SRC
# Returning to ordinary code after a bridged block comment can preserve a
# safe line break, but a closing delimiter *formed* across a splice must not
# be broken. The __LINE__ values pin both forms to physical source positions.
cat > "$tmp/closing-comment.c" <<'SRC'
#line 200
int closed = 1 + /* ordinary comment
*/ 2 + \
 __LINE__;
#line 300
int split_close = 1 + /* terminator formed by phase two
 *\
/ 2 + \
 __LINE__;
int main(void) { return closed != 205 || split_close != 306; }
SRC
for bits in 32 64; do
    for compiler in "$GCC" "$CCC"; do
        for opt in 0 2; do
            for fixture in ordinary split-only c23 escape directive-chain closing-comment; do
                std=gnu11; [[ $fixture == c23 ]] && std=gnu23
                "$compiler" -std="$std" -m"$bits" -O"$opt" "$tmp/$fixture.c" -o "$tmp/test"
                "$tmp/test" || { echo "FAIL: $fixture $compiler -m$bits -O$opt" >&2; exit 1; }
            done
        done
    done
done
# A safe splice after a separator split over physical lines must preserve
# the position of the token on the following line, not merely its value.
cat > "$tmp/position.c" <<'SRC'
int x = 1\
'0 + \
 l3c2;
SRC
cat > "$tmp/split-close-position.c" <<'SRC'
int x = 1 + /* comment
 *\
/ 2 + \
 l4c2;
SRC
for compiler in "$GCC" "$CCC"; do
    for fixture in position split-close-position; do
        want=3; [[ $fixture == split-close-position ]] && want=4
        if "$compiler" -std=gnu23 -fsyntax-only "$tmp/$fixture.c" >"$tmp/position.err" 2>&1; then
            echo "FAIL: $compiler accepted undeclared position marker in $fixture" >&2; exit 1
        fi
        if ! grep -Fq "$tmp/$fixture.c:$want:2:" "$tmp/position.err"; then
            echo "FAIL: $compiler did not report $fixture at physical line $want column 2" >&2
            cat "$tmp/position.err" >&2; exit 1
        fi
    done
done
echo 'multiline comment splice: PASS (GCC/LCCC, 32/64-bit, O0/O2, C23, positions)'
