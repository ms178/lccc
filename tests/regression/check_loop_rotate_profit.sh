#!/usr/bin/env bash
# ============================================================================
# check_loop_rotate_profit.sh -- pin loop rotation's decision.
#
# Rotation turns `while (c) body` into `if (c) do body while (c)`. The
# pipeline bottom-tests loops without it (`loop_invert`), so the one thing it
# adds is the block between the guard and the loop: a place that runs exactly
# when the body does. The default policy (`RotatePolicy::Profit`, -O2 and up)
# rotates a loop only when something then moves into that block that was
# worth moving, and leaves every other loop as the pipeline without it
# compiles it.
#
# `loop_rotate_profit_guarded_motion.c` pins that the result is the same
# program (nothing that moved runs when the loop does not). This script pins
# the DECISION, which is a compile-time property and invisible to a
# run-and-compare test:
#
#   1. It fires where it pays, and what was to leave the loop has left it:
#        a division of loop-invariant operands        (`idiv` out of the loop)
#        `s->count += x`, kept in a register          (no memory access left)
#        `a[i] = a[i-1] + 3`, carried in a register   (the load is gone)
#      Each with its negative control: with rotation off the same thing is
#      still in the loop, so the first half proves something.
#   2. It leaves alone, byte for byte, what does not pay:
#        a loop nothing can leave                     (never tried)
#        a load worth less than 1/32 of a trip        (tried, undone)
#        a load that would hold one register more     (tried, undone)
#   3. The switches mean what the documentation says: `CCC_NO_LOOP_ROTATE`
#      and `CCC_LOOP_ROTATE=0` are off; `CCC_LOOP_ROTATE=1` rotates the loop
#      (2) leaves alone; -O1 and -Os do not rotate.
#
# Assertions read x86-64 assembly.
# ============================================================================
set -euo pipefail

here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
CCC=${LCCC_BIN:-${CCC:-$here/../../target/fastbuild/lccc}}
[[ -x $CCC ]] || { echo "check_loop_rotate_profit: lccc not found at $CCC" >&2; exit 1; }
if [[ $(uname -m) != x86_64 ]]; then
  echo "check_loop_rotate_profit: SKIP (reads x86-64 assembly, host is $(uname -m))"
  exit 0
fi

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

# One translation unit per shape: a decision in one function must not be able
# to move the text of another.
cat > "$work/div.c" <<'C'
long inv_div(long a, long b, long n)
{
    long s = 0;
    for (long i = 0; i < n; i++)
        s += a / b + i;
    return s;
}
C
cat > "$work/acc.c" <<'C'
struct counter { long pad; long count; };
void acc_field(struct counter *s, long n)
{
    for (long i = 0; i < n; i++)
        s->count += i * 3;
}
C
cat > "$work/prefix.c" <<'C'
void prefix(long *a, long n)
{
    for (long i = 1; i < n; i++)
        a[i] = a[i - 1] + 3;
}
C
cat > "$work/plain.c" <<'C'
long sum(const long *a, long n)
{
    long s = 0;
    for (long i = 0; i < n; i++)
        s += a[i];
    return s;
}
C
# The outer loop's `*p` can leave behind a guard, and one trip of that loop is
# a whole inner loop.
cat > "$work/small.c" <<'C'
long small_gain(const long *p, const long *a, long n)
{
    long s = 0;
    for (long i = 0; i < n; i++) {
        s += *p;
        for (long j = 0; j < n; j++)
            s ^= a[j] + j;
    }
    return s;
}
C
# `p` is read again behind the loop, so its register is not free for `*p`.
cat > "$work/reg.c" <<'C'
long costs_a_register(const long *p, const long *a, long n)
{
    long s = 0;
    for (long i = 0; i < n; i++)
        s += a[i] * *p;
    return s + *p;
}
C

fail=0
ok()  { printf 'ok:   %s\n' "$1"; }
bad() { printf 'FAIL: %s\n' "$1" >&2; fail=1; }

# emit <out.s> <src.c> <opt> [VAR=VALUE...]
emit() {
  local out=$1 src=$2 opt=$3; shift 3
  env -u CCC_LOOP_ROTATE -u CCC_NO_LOOP_ROTATE "$@" "$CCC" "$opt" -S "$src" -o "$out"
}

# trace <src.c> [VAR=VALUE...]: the pass's own account of what it did.
trace() {
  local src=$1; shift
  env -u CCC_LOOP_ROTATE -u CCC_NO_LOOP_ROTATE "$@" CCC_DEBUG_LOOP_ROTATE=1 \
    "$CCC" -O2 -S "$src" -o /dev/null 2>&1 | grep -F '[ROT]' || true
}

# The function's loop: from the label its last conditional backward branch
# names to that branch. (Each function measured here has one loop.)
loop_of() { # loop_of <fn> <file.s>
  awk -v fn="$1:" '
    $0 == fn { inf = 1; next }
    inf && /^[[:space:]]*\.size[[:space:]]/ { exit }
    inf { n++; line[n] = $0
          if ($0 ~ /^\.[A-Za-z0-9_.$]+:/) { l = $0; sub(/:.*/, "", l); at[l] = n } }
    END {
      for (i = 1; i <= n; i++) {
        if (line[i] ~ /^[[:space:]]+j[a-z]+[[:space:]]+\./ && line[i] !~ /^[[:space:]]+jmp[[:space:]]/) {
          t = line[i]; sub(/^[[:space:]]+j[a-z]+[[:space:]]+/, "", t); gsub(/[[:space:]]/, "", t)
          if ((t in at) && at[t] < i) { from = at[t]; to = i }
        }
      }
      if (!to) exit 1
      for (i = from; i <= to; i++) print line[i]
    }' "$2"
}

# Instructions of a loop that touch memory (`lea` computes, it does not).
memory_in() { grep -E '^[[:space:]]+[a-z]' | grep -vE '^[[:space:]]+lea' | grep -cE '\(%r' || true; }
count() { grep -cE "$1" || true; }

# expect <what> <got> <want>
expect() {
  if [[ $2 == "$3" ]]; then ok "$1 ($2)"; else bad "$1: got $2, want $3"; fi
}

for shape in div acc prefix plain small reg; do
  emit "$work/$shape.on.s"  "$work/$shape.c" -O2
  emit "$work/$shape.off.s" "$work/$shape.c" -O2 CCC_NO_LOOP_ROTATE=1
done

# ── 1. It fires where it pays ───────────────────────────────────────────────
expect "division: one idiv in the function"        "$(count '^[[:space:]]+idiv' < "$work/div.on.s")" 1
expect "division: none of it in the loop"          "$(loop_of inv_div "$work/div.on.s" | count '^[[:space:]]+idiv')" 0
expect "division, rotation off: still in the loop" "$(loop_of inv_div "$work/div.off.s" | count '^[[:space:]]+idiv')" 1

expect "accumulator: the loop touches no memory"         "$(loop_of acc_field "$work/acc.on.s" | memory_in)" 0
expect "accumulator, rotation off: a load and a store"   "$(loop_of acc_field "$work/acc.off.s" | memory_in)" 2
# One read in front of the loop, one write behind it.
expect "accumulator: the field is read once and written once" \
  "$(grep -E '^[[:space:]]+[a-z]' "$work/acc.on.s" | count '8\(%rdi\)')" 2

expect "recurrence: the loop only stores"                "$(loop_of prefix "$work/prefix.on.s" | memory_in)" 1
expect "recurrence, rotation off: a load and a store"    "$(loop_of prefix "$work/prefix.off.s" | memory_in)" 2

for shape in div acc prefix; do
  if trace "$work/$shape.c" | grep -q 'rotation pays'; then
    ok "$shape: the trace names what left the loop"
  else
    bad "$shape: no 'rotation pays' line in the trace"
  fi
done

# ── 2. It leaves alone what does not pay ────────────────────────────────────
for shape in plain small reg; do
  if cmp -s "$work/$shape.on.s" "$work/$shape.off.s"; then
    ok "$shape: identical to rotation off"
  else
    bad "$shape: the default differs from rotation off"
  fi
done
# The three are left alone for three reasons, and each is the one meant.
if [[ -z $(trace "$work/plain.c") ]]; then
  ok "plain: never a candidate (no trial rotation)"
else
  bad "plain: the pass tried a loop nothing can leave"
fi
if trace "$work/small.c" | grep -q 'does not pay: 1 load left the loop, worth 1 of a trip of'; then
  ok "small: tried, and undone for what it is worth"
else
  bad "small: not refused for its worth: $(trace "$work/small.c" | tail -1)"
fi
if trace "$work/reg.c" | grep -q 'does not pay: 1 load left the loop and the loop holds more registers'; then
  ok "reg: tried, and undone for the register"
else
  bad "reg: not refused for the register: $(trace "$work/reg.c" | tail -1)"
fi

# ── 3. The switches ─────────────────────────────────────────────────────────
emit "$work/div.zero.s" "$work/div.c" -O2 CCC_LOOP_ROTATE=0
emit "$work/div.both.s" "$work/div.c" -O2 CCC_LOOP_ROTATE=1 CCC_NO_LOOP_ROTATE=1
for variant in zero both; do
  if cmp -s "$work/div.$variant.s" "$work/div.off.s"; then
    ok "switch ($variant): off"
  else
    bad "switch ($variant): not the same as CCC_NO_LOOP_ROTATE=1"
  fi
done
for opt in -O1 -Os; do
  emit "$work/div.opt.on.s"  "$work/div.c" "$opt"
  emit "$work/div.opt.off.s" "$work/div.c" "$opt" CCC_NO_LOOP_ROTATE=1
  if cmp -s "$work/div.opt.on.s" "$work/div.opt.off.s"; then
    ok "$opt: no rotation"
  else
    bad "$opt: rotation changed the output"
  fi
done
if trace "$work/plain.c" CCC_LOOP_ROTATE=1 | grep -q 'SUCCESS: rotated'; then
  ok "CCC_LOOP_ROTATE=1 rotates the loop the default leaves alone"
else
  bad "CCC_LOOP_ROTATE=1 did not rotate the plain loop"
fi

if ((fail)); then
  echo "loop rotation profit gate: FAIL"
  exit 1
fi
echo "loop rotation profit gate: PASS"
