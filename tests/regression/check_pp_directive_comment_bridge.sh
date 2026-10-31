#!/usr/bin/env bash
# Comment-bridged directives must preprocess to GCC's token sequence.
#
# C11 5.1.1.2 orders the translation phases: splicing (2) runs before comment
# replacement (3), which runs before directives are recognized (4). A block
# comment that opens on a `#define` line and is still open at that line's end
# therefore swallows the line's final newline from the *comment's* point of
# view: the next physical line merges into the same directive. The kernel
# hits this with fs/nfs/callback_xdr.c CB_OP_GETATTR_RES_MAXSZ:
#
#     #define CB_OP_GETATTR_RES_MAXSZ (CB_OP_HDR + \
#                                      /* time, deleg */\
#                                      (2 + 2 + ...) * 4)
#
# The second splice is preceded by a comment that internally contains a plain
# (unspliced) newline. Two properties are asserted against GCC 16.2 as the
# oracle, over `gcc -E -P` vs `$CCC -E -P` normalized to a single token
# stream:
#
#   1. byte-identical token sequence after the comment closes — the macro
#      body includes the arithmetic after `*/`, and nothing leaks as stray
#      text (the pre-fix lccc truncated the #define at the kept newline and
#      emitted `(2 + 2 + ...) * 4)` as top-level garbage);
#   2. for the kernel shape, the macro evaluates to the documented constant
#      (84) — a value check that survives even if both preprocessors agreed
#      on a broken reading.
#
# Variants pin the edges: quotes and `//` inside the bridged comment (they
# are comment text, not literals or line comments, while the bridge is
# open), a bridge without any trailing splice (worked before the fix, must
# keep working), a close-then-reopen chain inside one directive, a later
# ordinary splice after the bridge closed (the keep-break elision must come
# back — directive context must not leak), the `%:` digraph introducer, and
# a comment left open at end of file (GCC: the rest of the file is comment
# text inside the directive and disappears).
#
# Usage:  env CCC=... [GCC=gcc-16] bash tests/regression/check_pp_directive_comment_bridge.sh
set -euo pipefail
root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
CCC=${CCC:-$root/target/fastbuild/lccc}
GCC=${GCC:-gcc-16}
command -v "$CCC" >/dev/null || { echo "missing lccc: $CCC" >&2; exit 1; }
if ! command -v "$GCC" >/dev/null; then GCC=gcc; fi
command -v "$GCC" >/dev/null || { echo "missing oracle: $GCC" >&2; exit 1; }
tmp=$(mktemp -d)
trap 'rm -rf -- "$tmp"' EXIT

# Collapse all white space so a comparison is token-sequence-only (the
# keep-break elision gives lccc finer physical layout than GCC's always-splice
# output; tokens are the contract this gate pins).
norm() { tr '\n\t' '  ' <"$1" | tr -s ' ' | sed 's/^ //; s/ $//'; }

fail=0
check() {
  local name=$1 src=$2
  printf '%s' "$src" > "$tmp/$name.c"
  "$GCC" -E -P "$tmp/$name.c" >"$tmp/$name.gcc" 2>"$tmp/$name.gcc.err" || {
    echo "FAIL[$name]: gcc -E rejected the fixture" >&2
    cat "$tmp/$name.gcc.err" >&2
    fail=1
    return
  }
  if ! "$CCC" -E -P "$tmp/$name.c" >"$tmp/$name.lccc" 2>"$tmp/$name.lccc.err"; then
    echo "FAIL[$name]: lccc -E failed" >&2
    cat "$tmp/$name.lccc.err" >&2
    fail=1
    return
  fi
  norm "$tmp/$name.gcc" >"$tmp/$name.gcc.n"
  norm "$tmp/$name.lccc" >"$tmp/$name.lccc.n"
  if ! diff -u "$tmp/$name.gcc.n" "$tmp/$name.lccc.n" >"$tmp/$name.diff"; then
    echo "FAIL[$name]: token sequence diverges from $GCC -E -P:" >&2
    cat "$tmp/$name.diff" >&2
    fail=1
    return
  fi
  echo "ok[$name]"
}

# 1. The kernel shape: two splices around a comment holding a plain newline.
kernel_src='#define CB_OP_HDR ((2 * 4))
#define CB_OP_GETATTR_RES_MAXSZ (CB_OP_HDR + \
                                 /* time, deleg */\
                                 (2 + 2 + 3 + 3 + 3 + 3 + 3) * 4)
int arr[CB_OP_GETATTR_RES_MAXSZ];
'
check kernel_shape "$kernel_src"

# Value pin: (2*4) + (2+2+3+3+3+3+3)*4 = 8 + 76 = 84. Read the array size
# out of lccc's own expansion and evaluate it here, independent of GCC.
"$CCC" -E -P "$tmp/kernel_shape.c" | tr -d ' \n' | sed -n 's/.*intarr\[\(.*\)\];/\1/p' >"$tmp/expr"
if [ ! -s "$tmp/expr" ]; then
  echo "FAIL[kernel_value]: no 'int arr[...]' in lccc output" >&2
  fail=1
else
  val=$(python3 - "$tmp/expr" <<'PYEOF'
import sys, re
expr = open(sys.argv[1]).read().strip()
if not re.fullmatch(r"[0-9+*() -]+", expr):
    sys.exit(1)
print(eval(expr))
PYEOF
) || val=""
  if [ "$val" != "84" ]; then
    echo "FAIL[kernel_value]: macro evaluates to '$val', want 84 (expr: $(cat "$tmp/expr"))" >&2
    fail=1
  else
    echo "ok[kernel_value]"
  fi
fi

# 2. Quotes and '//' inside the bridged comment are comment text.
check quoted_comment_text '#define RES (1 + \
  /* it'"'"'s not // code
   * still comment */ 2 + \
  3)
int arr[RES];
'

# 3. Bridge without any trailing splice (pre-existing correct path).
check bridge_no_trailing_splice '#define D 1 + /* x
 y */ + 2
int use = D;
'

# 4. Close-and-reopen chain inside one directive, splice after the second
#    close (two carries in a row).
check chained_bridges '#define CH (1 + \
  /* a
   * b */ + /* c
   * d */ 2 + \
  3)
int arr[CH];
'

# 5. Bridge closes, then an ordinary-line splice further down: tokens must
#    still match GCC (which always splices), and the ordinary keep-break
#    elision is pinned by the unit tests, not here.
check ordinary_splice_after_bridge '#define D (1 + /*
  c */ 2)
int a = D;
int b = 1 + \
 2;
'

# 6. The %: digraph introduces the directive the same way.
check digraph_directive '%:define DG (1 + \
  /* x
   * y */ 2 + \
  3)
int arr[DG];
'

# 7. Comment spanning to the last line of the file inside a directive: GCC's
#    phase 3 deletes the whole comment including its newlines, so the
#    directive ends at EOF and the swallowed lines never appear. (A comment
#    left UNCLOSED at EOF is a hard GCC error — "unterminated comment" — so
#    the oracle fixture closes it on the final line.)
check comment_to_end_of_file '#define E 1 /*
int swallow;
int also_swallow;
*/
'

if [ "$fail" -ne 0 ]; then
  echo "directive comment-bridge: FAIL" >&2
  exit 1
fi
echo "directive comment-bridge: PASS (7 fixtures vs $GCC -E -P)"
