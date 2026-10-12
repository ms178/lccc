#!/usr/bin/env bash
# ============================================================================
# check_const_trip_loop_inline.sh — pin the constant-trip loop-helper decision.
#
# `const_trip_loop_inline.c` pins run-time RESULTS. This script pins the
# compile-time DECISION, which a run-and-compare test cannot see:
#
#   A loop helper whose exit is bounded by a parameter (expat's sip_round
#   shape, 137 IR instructions) is cloned at a call site that passes a
#   constant 2..8 for that parameter. Its state then stays in registers
#   across rounds. Measured on the SipHash benchmark: 198.9M -> 159.3M
#   Callgrind instructions, against 140.6M for GCC 14.2 (which also inlines
#   every site).
#
# Assertions (x86-64 -O2 -march=x86-64-v3 -fwrapv):
#   1. constant trips 2, 4 (inside a loop) and 8: the helper call is gone
#   2. trips 0, 1, 9, and a non-constant trip: the call remains
#   3. at -Os the tier is disabled, so a constant-trip call remains
#   4. an always_inline twin is NOT restricted to constants: at a
#      non-constant site its call is still inlined
# ============================================================================
set -euo pipefail

here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
CCC=${CCC:-$here/../../target/fastbuild/lccc}
[[ -x $CCC ]] || { echo "check_const_trip_loop_inline: lccc not found at $CCC" >&2; exit 1; }

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

# One TU for the admitted shapes and one for the refused shapes, so a change
# to one cannot mask the other. Both helpers are the sip_round body.
body='
static void round_ct(struct st *s, const int rounds)
{
    for (int i = 0; i < rounds; i++) {
        s->a += s->b; s->b = (s->b << 13) | (s->b >> 51); s->b ^= s->a;
        s->a = (s->a << 32) | (s->a >> 32);
        s->c += s->d; s->d = (s->d << 16) | (s->d >> 48); s->d ^= s->c;
        s->a += s->d; s->d = (s->d << 21) | (s->d >> 43); s->d ^= s->a;
        s->c += s->b; s->b = (s->b << 17) | (s->b >> 47); s->b ^= s->c;
        s->c = (s->c << 32) | (s->c >> 32);
    }
}
'
admitted="$work/admitted.c"
refused="$work/refused.c"
{
  echo '#include <stdint.h>'
  echo 'struct st { uint64_t a, b, c, d; };'
  echo "$body"
  cat <<'EOF'
__attribute__((noinline)) void site_two(struct st *s) { round_ct(s, 2); }
__attribute__((noinline)) void site_four_in_loop(struct st *s)
{
    for (int k = 0; k < 3; k++)
        round_ct(s, 4);
}
__attribute__((noinline)) void site_eight(struct st *s) { round_ct(s, 8); }
EOF
} > "$admitted"
{
  echo '#include <stdint.h>'
  echo 'struct st { uint64_t a, b, c, d; };'
  echo "$body"
  cat <<'EOF'
__attribute__((noinline)) void site_zero(struct st *s) { round_ct(s, 0); }
__attribute__((noinline)) void site_one(struct st *s) { round_ct(s, 1); }
__attribute__((noinline)) void site_nine(struct st *s) { round_ct(s, 9); }
__attribute__((noinline)) void site_variable(struct st *s, int n) { round_ct(s, n); }
EOF
  # always_inline twin of the same body, called at a non-constant site.
  ai_body=${body//round_ct/ai_round}
  attr='__attribute__((always_inline)) static void'
  echo "${ai_body/static void/$attr}"
  echo '__attribute__((noinline)) void site_always_variable(struct st *s, int n) { ai_round(s, n); }'
} > "$refused"

# Print one function's body: from its label up to its .size directive.
extract() { # extract <asm> <fn>
  awk -v fn="$2" '
    $0 == fn ":" { on = 1; next }
    on && $1 == ".size" && $2 == fn "," { exit }
    on { print }' "$1"
}

# Count direct calls to <callee> in one function's body.
calls_to() { # calls_to <asm> <fn> <callee>
  extract "$1" "$2" | grep -c "^[[:space:]]*call[[:space:]]\+$3[[:space:]]*$" || true
}

fail=0
expect_calls() { # expect_calls <asm> <fn> <callee> <count> <label>
  local got
  got=$(calls_to "$1" "$2" "$3")
  if [[ $got != "$4" ]]; then
    echo "FAIL $5: $2 has $got call(s) to $3, expected $4" >&2
    fail=1
  else
    echo "ok   $5: $2 has $got call(s) to $3"
  fi
}

O2="$work/admitted.s"
"$CCC" -O2 -march=x86-64-v3 -fwrapv -S -o "$O2" "$admitted"
expect_calls "$O2" site_two round_ct 0 "constant trip 2"
expect_calls "$O2" site_four_in_loop round_ct 0 "constant trip 4 in a loop"
expect_calls "$O2" site_eight round_ct 0 "constant trip 8"

R2="$work/refused.s"
"$CCC" -O2 -march=x86-64-v3 -fwrapv -S -o "$R2" "$refused"
expect_calls "$R2" site_zero round_ct 1 "trip 0 is refused"
expect_calls "$R2" site_one round_ct 1 "trip 1 is refused"
expect_calls "$R2" site_nine round_ct 1 "trip 9 is refused"
expect_calls "$R2" site_variable round_ct 1 "non-constant trip is refused"
expect_calls "$R2" site_always_variable ai_round 0 "always_inline at a non-constant site is cloned"

OS="$work/admitted_os.s"
"$CCC" -Os -march=x86-64-v3 -fwrapv -S -o "$OS" "$admitted"
expect_calls "$OS" site_two round_ct 1 "-Os keeps the tier disabled (trip 2)"

if [[ $fail -ne 0 ]]; then
  echo "check_const_trip_loop_inline: FAILED" >&2
  exit 1
fi
echo "check_const_trip_loop_inline: all assertions hold"
