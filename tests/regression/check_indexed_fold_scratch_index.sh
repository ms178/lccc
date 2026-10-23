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
# SIB store present; a %rdx-born remainder index folded in put_rem) and
# differentially (shrunk-corpus binary
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
# held exactly `leaq 8(%rsp), %rcx`. The ban is the remat SIGNATURE
# (frame-anchored LEA), not every LEA: a future legitimate non-frame LEA
# (alloca arithmetic, PIC) must not trip the fold pin; the positive SIB
# pin below carries the fold proof either way.
if grep -qE '^[[:space:]]+leaq +-?[0-9]*\(%(rsp|rbp)\)' "$tmp/put.s"; then
    echo "FAIL: rematerialisation LEA inside sqlite_put_varint:" >&2
    grep -E '^[[:space:]]+leaq +-?[0-9]*\(%(rsp|rbp)\)' "$tmp/put.s" >&2
    exit 1
fi

# The digit-buffer index must fold into a stack-slot SIB store
# (`movb %r9b, 8(%rsp, %rdx)`; value register, displacement and index home
# left loose: which register the allocator picks is not this gate's subject).
if ! grep -qE 'movb %[a-z0-9]+, -?[0-9]*\(%rsp, %r[0-9a-z]+\)' "$tmp/put.s"; then
    echo "FAIL: sqlite_put_varint lacks the folded stack-slot SIB store" >&2
    exit 1
fi

# Scratch-homed index witness, independent of the allocator's other choices:
# a remainder is BORN in %rdx (`divq`), and the %rdx wave admits a value born
# at its clobber point, so the index of `buf[v % d]` is %rdx-homed by
# construction.  The decider must fold it (`movb %r13b, (%rsp, %rdx)`), never
# rematerialise the slot address into a register first.
cat > "$tmp/rem.c" <<'CSRC'
typedef unsigned long u64;
__attribute__((noinline)) unsigned put_rem(u64 v, u64 d, unsigned char x, const unsigned char *in) {
    unsigned char buf[64];
    __builtin_memcpy(buf, in, 64);
    buf[v % d] = x;
    return buf[0] + buf[63] * 3u + buf[(v + 1) & 63] * 7u;
}
CSRC
"$CCC" -O2 -S "$tmp/rem.c" -o "$tmp/rem.s"
rem=$(awk '/^put_rem:/{on=1} on; /^\.size[[:space:]]+put_rem,/{on=0}' "$tmp/rem.s")
if ! grep -qE 'divq' <<<"$rem" \
    || ! grep -qE '^[[:space:]]+movb %[a-z0-9]+, -?[0-9]*\(%rsp, %rdx\)$' <<<"$rem"; then
    echo "FAIL: put_rem's %rdx-born index is not folded into a (%rsp, %rdx) SIB store:" >&2
    printf '%s\n' "$rem" >&2
    exit 1
fi

# Differential execution with a shrunk corpus (defaults are 2^18 x 24).
"$CCC" -O2 -DVALUE_COUNT=4096 -DPASSES=2 "$bench" -o "$tmp/test"
"$tmp/test" > "$tmp/run.log"
grep -Eq '^[0-9a-f]+$' "$tmp/run.log" || { echo "FAIL: no checksum line:" >&2; cat "$tmp/run.log" >&2; exit 1; }
gcc -O2 -DVALUE_COUNT=4096 -DPASSES=2 "$bench" -o "$tmp/ref"
"$tmp/ref" > "$tmp/ref.log"
cmp -s "$tmp/run.log" "$tmp/ref.log" || { echo "FAIL: checksum differs from gcc -O2" >&2; exit 1; }

echo "OK indexed-fold-scratch-index"
