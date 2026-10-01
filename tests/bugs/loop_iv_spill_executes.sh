#!/bin/bash
# End-to-end regression for the loop-IV spill coalescing pass.
#
# The unit tests in loop_iv_spill.rs prove the pass's guards and its emitted
# shape on hand-written assembly. They cannot prove the pass's OUTPUT is right,
# and output is the only thing that matters here: the failure mode is a
# generated label colliding with one already present, which merges two blocks in
# the assembler's view and keeps only the first predecessor set. `as` accepts
# that, `ld` accepts it, and the program returns a plausible wrong answer with
# nothing on any compiler's stderr. Comparing assembly text would not catch it.
#
# So this script goes through the whole chain on the one program in the corpus
# where the pass actually fires (the corpus differential puts it at 1 of 937):
#
#   1. compile with the pass ON and with it OFF, same binary, same flags;
#   2. assemble each arm STANDALONE with `as`, so a malformed rewrite is caught
#      even though lccc's own driver would have papered over it;
#   3. link and RUN both, and require identical output;
#   4. require the hot loop to have actually shrunk.
#
# The pass flag is `CCC_PEEPHOLE_SKIP`, not a rebuild: both arms come from one
# binary, so an A/B here cannot be explained by a different compiler.
set -u
LCCC=${LCCC:-/home/user/lccc/target/fastbuild/lccc}
SRC=${SRC:-/home/user/lccc/tests/benchmark/programs/lz4_compress.c}
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

FLAGS="-O2 -DMATCH_RICH=1 -DPASSES=2048"
EXPECT=29f49acaab81f800
fail() { echo "FAIL: $*" >&2; exit 1; }

for arm in on off; do
  if [ "$arm" = on ]; then
    "$LCCC" $FLAGS -S -o "$WORK/$arm.s" "$SRC" || fail "compile (pass on)"
  else
    CCC_PEEPHOLE_SKIP=loop_iv_spill,__none__ \
      "$LCCC" $FLAGS -S -o "$WORK/$arm.s" "$SRC" || fail "compile (pass off)"
  fi
  as --64 "$WORK/$arm.s" -o "$WORK/$arm.o" 2> "$WORK/$arm.aserr" \
    || fail "assemble ($arm): $(head -3 "$WORK/$arm.aserr")"
  gcc "$WORK/$arm.o" -o "$WORK/$arm.bin" 2>/dev/null \
    || fail "link ($arm)"
  "$WORK/$arm.bin" > "$WORK/$arm.out" || fail "run ($arm)"
done

on=$(cat "$WORK/on.out")
off=$(cat "$WORK/off.out")
[ "$on" = "$off" ] || fail "output differs: on=$on off=$off"
[ "$on" = "$EXPECT" ] || fail "checksum $on, expected $EXPECT"

# The rewrite must have happened, or this script has silently tested nothing.
cmp -s "$WORK/on.s" "$WORK/off.s" && fail "the pass did not fire; the test is vacuous"

# And it must have removed the round trip: exactly one reload of the slot and
# exactly one store back, both outside the loop body.
loads=$(grep -c 'movq 112(%rsp), %rcx' "$WORK/on.s")
stores=$(grep -c 'movq  *%rcx, 112(%rsp)' "$WORK/on.s")
[ "$loads" -eq 1 ] || fail "expected 1 reload in the pass-on arm, found $loads"
[ "$stores" -eq 1 ] || fail "expected 1 store in the pass-on arm, found $stores"
offloads=$(grep -c 'movq 112(%rsp), %rcx' "$WORK/off.s")
[ "$offloads" -ge 1 ] || fail "the pass-off arm has no reload to remove"

echo "PASS: loop-IV spill coalescing, checksum $on, $loads reload / $stores store"
