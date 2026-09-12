#!/usr/bin/env bash
# differential_corpus.sh — prove a compiler change is (or is not) output-neutral
# across a whole corpus, instead of arguing it from the shape of the patch.
#
#   scripts/differential_corpus.sh <reference-lccc> <candidate-lccc> [corpus-root]
#
# Compiles every .c under CORPUS-ROOT (default: tests/) with both compilers at
# -O2 -S and reports three things separately:
#
#   1. exit-status differences   — the candidate broke (or fixed) a compile
#   2. one-sided failures        — a file only one compiler can handle
#   3. assembly differences      — among the files BOTH compile, byte comparison
#
# Why byte comparison and not instruction counts: a refactor that claims to be
# behaviour-preserving has exactly one honest test, and it is that the machine
# code is identical.  Counting instructions would let a semantic change hide
# behind a compensating one.  Conversely, when the arms are byte-identical a
# runtime A/B is uninformative by construction — scripts/paired_ab.py exits 3 on
# identical binaries — so the win from the reference build carries over exactly
# and re-measuring it would be theatre.  This script is how you establish that.
#
# Determinism: each translation unit is compiled in its own invocation, so
# neither compiler carries state between files.  A difference reported here is a
# real difference in codegen, not an artefact of ordering.
#
# Exit status: 0 = no differences of any kind, 1 = differences found (listed),
# 2 = bad usage or a compiler that will not run.
set -uo pipefail

usage() { echo "usage: $0 <reference-lccc> <candidate-lccc> [corpus-root]" >&2; exit 2; }
[ $# -ge 2 ] || usage
REF="$1"; CAND="$2"; ROOT="${3:-tests}"

for c in "$REF" "$CAND"; do
  [ -x "$c" ] || { echo "not an executable: $c" >&2; exit 2; }
  "$c" --version >/dev/null 2>&1 || "$c" --help >/dev/null 2>&1 || {
    echo "compiler does not respond: $c" >&2; exit 2; }
done
[ -d "$ROOT" ] || { echo "no such corpus root: $ROOT" >&2; exit 2; }

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
mkdir -p "$WORK/ref" "$WORK/cand"
find "$ROOT" -name '*.c' | LC_ALL=C sort > "$WORK/list.txt"
N=$(wc -l < "$WORK/list.txt")
[ "$N" -gt 0 ] || { echo "corpus is empty: $ROOT" >&2; exit 2; }

compile_all() { # <lccc> <outdir>
  local cc="$1" out="$2" f n
  while IFS= read -r f; do
    n="${f//\//_}"
    timeout 120 "$cc" -O2 -S -o "$out/$n.s" "$f" >/dev/null 2>&1
    echo "$?" > "$out/$n.rc"
  done < "$WORK/list.txt"
}

compile_all "$REF"  "$WORK/ref"
compile_all "$CAND" "$WORK/cand"

LIST="$WORK/list.txt" REFDIR="$WORK/ref" CANDDIR="$WORK/cand" python3 - <<'PY'
import hashlib, os
lst = [l.strip() for l in open(os.environ['LIST']) if l.strip()]
ref, cand = os.environ['REFDIR'], os.environ['CANDDIR']
rc_diff, one_sided, asm_diff = [], [], []
ok = fail_both = 0
for f in lst:
    n = f.replace('/', '_')
    a = open(f'{ref}/{n}.rc').read().strip()
    b = open(f'{cand}/{n}.rc').read().strip()
    if a != b:
        rc_diff.append((f, a, b))
    if a != '0' or b != '0':
        if a != '0' and b != '0':
            fail_both += 1
        else:
            one_sided.append((f, a, b))
        continue
    ok += 1
    ha = hashlib.sha256(open(f'{ref}/{n}.s', 'rb').read()).hexdigest()
    hb = hashlib.sha256(open(f'{cand}/{n}.s', 'rb').read()).hexdigest()
    if ha != hb:
        asm_diff.append(f)

print(f"corpus           : {len(lst)} translation units")
print(f"compiled by both : {ok}")
print(f"failed by both   : {fail_both}  (pre-existing, not attributable)")
print(f"exit-status diffs: {len(rc_diff)}")
for f, a, b in rc_diff[:20]:
    print(f"    {f}: ref rc={a} cand rc={b}")
print(f"one-sided failures: {len(one_sided)}")
for f, a, b in one_sided[:20]:
    print(f"    {f}: ref rc={a} cand rc={b}")
print(f"asm diffs        : {len(asm_diff)} of {ok}")
for f in asm_diff[:60]:
    print(f"    {f}")

bad = rc_diff or one_sided or asm_diff
if not bad:
    print("\nVERDICT: output-neutral across the corpus "
          "(identical exit statuses, identical failure set, byte-identical asm)")
else:
    print("\nVERDICT: the candidate CHANGES codegen for the files listed above")
raise SystemExit(1 if bad else 0)
PY
