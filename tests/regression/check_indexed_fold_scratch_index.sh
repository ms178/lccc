#!/usr/bin/env bash
# Indexed SIB fold with a scratch-homed index (S13 codegen-gate fix).
#
# The x86-64 `indexed_fold_ok` decider must accept a SIB base/index homed in
# the emitter's %rdx/%r11 scratch set: the indexed path never writes either
# register (write-confinement proof in the decider's doc comment), so
# refusing the fold only forces a redundant LEA rematerialisation. The
# historical over-strict decider refused sqlite's 9-byte put-arm store and
# emitted `leaq 8(%rsp), %rcx` + `movb %r9b, (%rdx, %rcx)` instead of the
# single `movb %r9b, 8(%rsp, %rdx)` (+1 insn, +1 stackmem on the golden
# sqlite_varint gate; the expat moves regression shared the root cause and
# stays pinned by the gate's metrics).
#
# Pins the fold structurally (no remat LEA inside sqlite_put_varint, folded
# %rdx-indexed SIB store present) and differentially (shrunk-corpus binary
# agrees with gcc -O2 on the checksum).
set -eu

CCC=${CCC:-./target/fastbuild/lccc}
dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
bench="$dir/../benchmark/programs/sqlite_varint.c"
tmp="${TMPDIR:-/tmp}/lccc-indexed-fold-scratch.$$"
trap 'rm -rf "$tmp"' EXIT HUP INT TERM
mkdir -p "$tmp"

"$CCC" -O2 -S "$bench" -o "$tmp/out.s"

# Isolate sqlite_put_varint (up to the next .type directive).
awk 'BEGIN{on=0} /^\.type sqlite_put_varint, @function/{on=1;next} /^\.type /{on=0} on' \
    "$tmp/out.s" > "$tmp/put.s"
test -s "$tmp/put.s"

# No rematerialisation LEA may survive inside the function: pre-fix this
# held exactly `leaq 8(%rsp), %rcx`.
if grep -qE '^[[:space:]]+leaq' "$tmp/put.s"; then
    echo "FAIL: rematerialisation LEA inside sqlite_put_varint:" >&2
    grep -E '^[[:space:]]+leaq' "$tmp/put.s" >&2
    exit 1
fi

# The %rdx-homed index must fold into a stack-slot SIB store
# (`movb %r9b, 8(%rsp, %rdx)`; value register and displacement left loose).
grep -qE 'movb %r[0-9]+b, [0-9]+\(%rsp, %rdx\)' "$tmp/put.s"

# Differential execution with a shrunk corpus (defaults are 2^18 x 24).
"$CCC" -O2 -DVALUE_COUNT=4096 -DPASSES=2 "$bench" -o "$tmp/test"
"$tmp/test" > "$tmp/run.log"
grep -Eq '^[0-9a-f]+$' "$tmp/run.log"
gcc -O2 -DVALUE_COUNT=4096 -DPASSES=2 "$bench" -o "$tmp/ref"
"$tmp/ref" > "$tmp/ref.log"
cmp -s "$tmp/run.log" "$tmp/ref.log"

echo "OK indexed-fold-scratch-index"
