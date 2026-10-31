#!/usr/bin/env bash
# Two instruction-selection contracts, both of which are INVISIBLE in output
# and therefore need a codegen gate rather than a differential test:
#
#   1. `ptr_iv_collapse` — several pointer induction variables that advance by
#      the same constant in a literal-bounded loop share one index register.
#      The reads become `off(%base,%index)` and the loop carries one counter
#      instead of one counter plus one `add` per pointer.  zlib-ng's
#      compare256_64 is the shape: 66.35 -> 42.85 cycles/call on the pinned
#      kernel measurement.
#
#   2. `promote_guarded_counts` (corpus-level zero-guard proof, runs inside
#      CVP) — `__builtin_clz(x | 1)` and `__builtin_ctz(x | 0x80000000)` are
#      provably nonzero by construction, so the baseline lowering must not
#      emit the defined-zero diamond.  byte_bit_length in llvm_word_counts.c
#      goes 14 -> 5 instructions, matching GCC.
#
# Both checks are two-sided: the positive case proves the transform fires, and
# the negative case (an unguarded count, or the pass switched off) proves the
# check can fail.  A gate that cannot fail is decoration.
#
# Usage: CCC=target/fastbuild/lccc bash tests/regression/check_ptr_iv_collapse_codegen.sh
set -euo pipefail

CCC=${CCC:-./target/fastbuild/lccc}
repo=$(cd "$(dirname "$0")/../.." && pwd)
tmp=${TMPDIR:-/tmp}/lccc-ptriv.$$
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp"
fail=0

note() { printf '%s\n' "$*"; }
bad() {
  printf 'FAIL: %s\n' "$*"
  fail=1
}

# Body of one function, from its label to its `.size` directive.
body() { awk -v fn="$2" '$0 ~ ("^" fn ":") { on = 1 } on { print } $0 ~ ("^\\.size " fn ",") { on = 0 }' "$1"; }

# ── 1. pointer IV collapse ──────────────────────────────────────────────────
src="$repo/tests/regression/ptr_iv_collapse_scan.c"
"$CCC" -O2 -S "$src" -o "$tmp/on.s"
CCC_DISABLE_PASSES=ptr_iv_collapse "$CCC" -O2 -S "$src" -o "$tmp/off.s"

# The collapsed loop addresses both arrays through ONE index register:
# `(%rdi,%rdx)` and `(%rsi,%rdx)` share the index.  Extract the memory
# operands of the compare256_64 body and compare the index registers.
indices() {  # indices <file> <function> <base-substring>
  body "$1" "$2" |
    grep -oE "\((%r[a-z0-9]+), %r[a-z0-9]+\)" |
    tr -d '()' | tr ',' '\n' | awk 'NR % 2 == 0' | sort -u | tr '\n' ' '
}

on_idx=$(indices "$tmp/on.s" compare256_64)
if [ "$(echo "$on_idx" | wc -w)" != "1" ]; then
  bad "compare256_64 does not address both arrays through one index register (indices: ${on_idx:-none})"
fi
# Exactly one counter increment in the collapsed body.
on_add=$(body "$tmp/on.s" compare256_64 | grep -cE '^\s+add[lq] \$8,' || true)
if [ "$on_add" != "1" ]; then
  bad "compare256_64 carries $on_add step instructions, expected exactly 1"
fi

# Negative control: the same translation unit with the pass off must NOT have
# the single-index shape, otherwise this gate is measuring nothing.
off_add=$(body "$tmp/off.s" compare256_64 | grep -cE '^\s+add[lq] \$8,' || true)
if [ "$off_add" -le "$on_add" ]; then
  bad "with the pass disabled the loop still has $off_add step instructions (expected more than $on_add)"
fi
if [ "$(indices "$tmp/off.s" compare256_64 | wc -w)" = "1" ]; then
  bad "with the pass disabled compare256_64 still uses a single index (positive control is vacuous)"
fi

# The pass must be semantics-preserving on the whole file, not just this loop.
"$CCC" -O2 -o "$tmp/scan" "$src"
if ! "$tmp/scan" >"$tmp/scan.out"; then
  bad "ptr_iv_collapse_scan failed under the collapsed build"
fi
CCC_DISABLE_PASSES=ptr_iv_collapse "$CCC" -O2 -o "$tmp/scan_off" "$src"
"$tmp/scan_off" >"$tmp/scan_off.out"
if ! cmp -s "$tmp/scan.out" "$tmp/scan_off.out"; then
  bad "collapsed and uncollapsed builds disagree on ptr_iv_collapse_scan output"
fi

# ── 2. zero-guard count promotion ───────────────────────────────────────────
gatesrc="$repo/tests/regression/clz_zero_guard.c"
"$CCC" -O2 -S "$gatesrc" -o "$tmp/counts.s"

# The defined-zero lowering introduces `clz_nz`/`clz_done`/`ctz_nz` labels.
# None may appear in a guarded count.
for fn in clz_guarded ctz_guarded byte_bit_length cap_leading; do
  if body "$tmp/counts.s" "$fn" | grep -qE '^\.L(clz|ctz)_(nz|done)'; then
    bad "$fn still emits the defined-zero diamond"
  fi
done

# Negative control: the UNGUARDED counts must keep a defined-zero fix-up —
# either the diamond or the branch-free zero seed (`movl $63`, `movl $32`).
# Without one of them the positive case above proves nothing about the guard.
for fn in clz_plain ctz_plain; do
  if ! body "$tmp/counts.s" "$fn" |
      grep -qE '^(\.L(clz|ctz)_(nz|done)|\s+mov[lq] \$(32|63), %e?[a-z0-9]+$)'; then
    bad "$fn lost its defined-zero lowering: the negative control is vacuous"
  fi
done
# ...and the guarded forms must not contain one.
for fn in clz_guarded ctz_guarded byte_bit_length; do
  if body "$tmp/counts.s" "$fn" | grep -qE '\s+mov[lq] \$(32|63), %e?[a-z0-9]+$'; then
    bad "$fn still seeds a zero result: the guard proof did not reach the lowering"
  fi
done

# The folded bit-length wrapper is a single BSR and an add, not a scan plus a
# zero test: pin the count so a regression shows up as a number, not a guess.
bb_lines=$(body "$tmp/counts.s" byte_bit_length |
  grep -vE '^\s*(#|\.cfi|\.size|\.type|\.p2align|\.L|\.l)' |
  grep -cE '^\s+[a-z]' || true)
if [ "$bb_lines" -gt 6 ]; then
  bad "byte_bit_length lowered to $bb_lines instructions (expected at most 6)"
fi

if [ "$fail" = 0 ]; then
  note "PASS: pointer IV collapse and zero-guard count promotion are both selected"
  note "      compare256_64 steps: collapsed=$on_add uncollapsed=$off_add"
  note "      byte_bit_length instructions: $bb_lines"
fi
exit "$fail"
