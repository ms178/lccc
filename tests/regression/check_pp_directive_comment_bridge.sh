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
# The same merging happens when the comment opens on an ORDINARY line and
# closes on a line whose remaining tokens are a `#define` — the phase-4
# context of a bridged line is decided by its own tokens, never by the line
# that opened the comment:
#
#     /* header
#        comment */ #define A 1 + \
#       2
#
# Both shapes truncate the macro body if the joiner keeps a physical break
# inside the directive, and both leak the tail as top-level tokens.
#
# Two properties are asserted against GCC 16.2 as the oracle, over
# `gcc -E -P` vs `$CCC -E -P` normalized to a single token stream:
#
#   1. byte-identical token sequence after the comment closes — the macro
#      body includes the arithmetic after `*/`, and nothing leaks as stray
#      text;
#   2. for the kernel shape, the macro evaluates to the documented constant
#      (84) — a value check that survives even if both preprocessors agreed
#      on a broken reading.
#
# Coverage, in three legs:
#
#   * fixtures — the shapes reasoned about by hand: the kernel shape, quotes
#     and `//` inside a bridged comment (they are comment text, not literals
#     or line comments, while the bridge is open), a bridge without any
#     trailing splice, a close-then-reopen chain inside one directive, a
#     later ordinary splice after the bridge closed (the keep-break elision
#     must come back — directive context must not leak), the `%:` digraph
#     introducer, a directive reached from an ordinary line (with and without
#     space after `*/`), a bridged comment that swallows following code into
#     the macro body, and a comment left open at end of file. Each fixture is
#     run through EVERY frontend built beside $CCC (x86-64, i686, AArch64,
#     RISC-V): the joiner lives in shared frontend code and a target-specific
#     surprise is exactly the kind of thing a single-target gate hides.
#   * a deterministic sweep — a fixed-seed generator over the same grammar
#     with randomized comment bodies, depths, spacing and suffixes. Seeded so
#     a failure is reproducible from the seed alone; the two shapes above were
#     found by exactly this sweep once their templates were encoded.
#   * a value pin — the kernel macro's numeric value, computed here.
#
# The sweep is a knob so one gate serves both duties: CI runs the default
# width in about a second, and a release audit widens it without editing code —
#
#     LCCC_PP_FUZZ_CASES=400 bash tests/regression/check_pp_directive_comment_bridge.sh
#
# reproduces the 400-case x 5-frontend audit recorded in
# engineering/journal/2026-10-31-comment-bridged-directives.md (2000 compiler
# invocations, ~10 s on two cores). LCCC_PP_FUZZ_SEED changes the corpus.
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

# Every frontend the build produced beside $CCC. The CI artifact ships all
# six binaries, so this leg runs there; a packaged single-binary setup skips
# it with a note instead of failing.
ccc_path=$CCC
if [[ $ccc_path != */* ]]; then ccc_path=$(command -v -- "$CCC"); fi
compiler_dir=$(CDPATH= cd -- "$(dirname -- "$ccc_path")" && pwd)
frontends=("$ccc_path")
for sibling in lccc-x86 lccc-i686 lccc-arm lccc-riscv; do
  if [[ -x $compiler_dir/$sibling ]]; then frontends+=("$compiler_dir/$sibling"); fi
done
if (( ${#frontends[@]} == 1 )); then
  echo "note: no sibling frontends beside $ccc_path; single-target leg only" >&2
fi

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
  norm "$tmp/$name.gcc" >"$tmp/$name.gcc.n"
  local cc
  for cc in "${frontends[@]}"; do
    if ! "$cc" -E -P "$tmp/$name.c" >"$tmp/$name.out" 2>"$tmp/$name.err"; then
      echo "FAIL[$name@$(basename -- "$cc")]: -E failed" >&2
      cat "$tmp/$name.err" >&2
      fail=1
      continue
    fi
    norm "$tmp/$name.out" >"$tmp/$name.n"
    if ! diff -u "$tmp/$name.gcc.n" "$tmp/$name.n" >"$tmp/$name.diff"; then
      echo "FAIL[$name@$(basename -- "$cc")]: token sequence diverges from $GCC -E -P:" >&2
      cat "$tmp/$name.diff" >&2
      fail=1
      continue
    fi
    echo "ok[$name@$(basename -- "$cc")]"
  done
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

# 8. The comment opens on an ORDINARY line and closes on a line whose
#    remaining tokens introduce a directive, which then splices. The phase-4
#    context belongs to the closing line: `A` is `1 + 2`.
check bridge_from_ordinary_line '/* header
   comment */ #define A 1 + \
  2
int arr[A];
'

# 9. Same, with no space after the terminator (`*/#define`).
check bridge_from_ordinary_line_tight '/* header
   comment */#define B 2 * \
  3
int arr[B];
'

# 10. The destructive half of phase 3: the comment's newline is deleted, so
#     `int swallowed …;` is part of the directive and never reaches the
#     output, while the macro body keeps its spliced continuation.
check bridge_swallows_following_code '#define D 1 /* c
*/ int swallowed = 1 \
  + 2;
int after;
'

# ---- 11. deterministic differential sweep -----------------------------------
# The fixtures above are the shapes reasoned about by hand; this is the space
# around them, generated from a fixed seed so a failure reproduces exactly.
# The sweep runs against $CCC only: every frontend above shares this frontend
# code, and the fixture leg proves that on all of them.
sweep_cases_wanted=${LCCC_PP_FUZZ_CASES:-24}
sweep_seed=${LCCC_PP_FUZZ_SEED:-0x5EED1234}
python3 - "$tmp/sweep" "$sweep_cases_wanted" "$sweep_seed" <<'PYEOF'
import os
import random
import sys

out = sys.argv[1]
wanted = int(sys.argv[2], 0)
seed = int(sys.argv[3], 0)
os.makedirs(out, exist_ok=True)
rng = random.Random(seed)

WORDS = ("change", "size", "atime", "mtime", "deleg", "not // code", "it's", '"quoted"', "%", "%:")


def body(depth, indent="   "):
    """Comment body lines; may contain quotes, `//` and `%:` but never `*/`."""
    lines = []
    for i in range(depth):
        word = rng.choice(WORDS)
        lines.append(f"{indent}* {word} {rng.randrange(100)} line {i}")
    return "\n".join(lines)


def ints():
    return [rng.randrange(1, 9) for _ in range(4)]


def case_a(a, b, c, d):
    return (
        f"#define M{a} (({a}) + \\\n"
        f"  /* alpha\n{body(rng.randrange(1, 4))} */\\\n"
        f"  ({b} + {c}) * {d})\n"
        f"int arr[M{a}];\n"
    )


def case_b(a, b, c, d):
    gap = " " * rng.randrange(0, 3)
    return (
        f"/* alpha\n{body(rng.randrange(1, 4))} */{gap}#define M{a} {b} + \\\n"
        f"  {c}\n"
        f"int arr[M{a}];\n"
    )


def case_c(a, b, c, d):
    return (
        f"#define M{a} (({b}) + \\\n"
        f"  /* one\n{body(rng.randrange(1, 3), '   ')} */ + /* three\n"
        f"{body(rng.randrange(1, 3), '   ')} */ {c} + \\\n"
        f"  {d})\n"
        f"int arr[M{a}];\n"
    )


def case_d(a, b, c, d):
    return (
        f"#define M{a} {b} /* alpha\n"
        f"{body(rng.randrange(1, 3), '')}*/ int swallowed{a} = {c} \\\n"
        f"  + {d};\n"
        f"int after{a};\n"
    )


def case_e(a, b, c, d):
    return (
        f"int v{a} = {b} + /* alpha\n"
        f"{body(rng.randrange(1, 3), '')}*/ {c} + \\\n"
        f"  {d};\n"
        f"int w{a} = v{a} + \\\n"
        f"  {b};\n"
    )


def case_f(a, b, c, d):
    return (
        f"%:define M{a} ({b} + \\\n"
        f"  /* alpha\n{body(rng.randrange(1, 4))} */ {c})\n"
        f"int arr[M{a}];\n"
    )


templates = (case_a, case_b, case_c, case_d, case_e, case_f)
index = 0
while index < wanted:
    for template in templates:
        if index >= wanted:
            break
        a, b, c, d = ints()
        a = a * 100 + index  # unique macro/binding names per case
        with open(os.path.join(out, f"case{index:03d}.c"), "w") as handle:
            handle.write(template(a, b, c, d))
        index += 1
print(index)
PYEOF
sweep_cases=$(ls "$tmp/sweep"/*.c | wc -l)
for case in "$tmp/sweep"/*.c; do
  name=$(basename -- "$case" .c)
  "$GCC" -E -P "$case" >"$tmp/$name.gcc" 2>"$tmp/$name.gcc.err" || {
    echo "FAIL[sweep:$name]: $GCC rejected a generated fixture (generator bug)" >&2
    cat "$tmp/$name.gcc.err" >&2
    fail=1
    continue
  }
  "$CCC" -E -P "$case" >"$tmp/$name.out" 2>"$tmp/$name.err" || {
    echo "FAIL[sweep:$name]: $CCC -E failed" >&2
    cat "$tmp/$name.err" >&2
    fail=1
    continue
  }
  norm "$tmp/$name.gcc" >"$tmp/$name.gcc.n"
  norm "$tmp/$name.out" >"$tmp/$name.n"
  if ! diff -u "$tmp/$name.gcc.n" "$tmp/$name.n" >"$tmp/$name.diff"; then
    echo "FAIL[sweep:$name]: token sequence diverges from $GCC -E -P:" >&2
    cat "$tmp/$name.diff" >&2
    fail=1
  fi
done
if (( fail == 0 )); then
  echo "ok[sweep] ($sweep_cases generated cases, seed $sweep_seed)"
fi

if [ "$fail" -ne 0 ]; then
  echo "directive comment-bridge: FAIL" >&2
  exit 1
fi
echo "directive comment-bridge: PASS (10 fixtures x ${#frontends[@]} frontends, $sweep_cases seeded cases, value pin, vs $GCC -E -P)"
