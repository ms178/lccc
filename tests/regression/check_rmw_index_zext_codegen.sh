#!/usr/bin/env bash
# ============================================================================
# check_rmw_index_zext_codegen.sh -- `C[idx]++` on a 32-bit-indexed table is
# ONE memory read-modify-write.
#
# The x86-64 emitter spells the shapes in rmw_index_zext.c as
#
#     movl %esi, %esi                 <- index zero-extension
#     movl (%rdi, %rsi, 4), %r9d      <- load
#     leal 1(%r9), %eax               <- 32-bit increment, ADDRESS-width base
#     movl %esi, %esi                 <- redundant re-zero-extension
#     movl %eax, (%rdi, %rsi, 4)      <- store
#
# and GCC 16.2 / Clang 23.1 emit one `addl $1, (%rdi, %rsi, 4)`.  Two defects
# kept the trio alive: `memory_fold::parse_const_increment` compared the LEA
# base `%r9` to the loaded `%r9d` textually, and the redundant self-zext that
# separates increment and store is only deleted by the global phase, after
# Phase 1's last `mem_rmw` run.
#
# Contracts:
#   1. SEMANTICS.  The fixture's stdout equals the GCC oracle's at -O0..-O3,
#      with the fold enabled and with `CCC_PEEPHOLE_SKIP=mem_rmw`.  The skip
#      name is passed to the COMPILER (it is a compile-time knob; exporting it
#      to the compiled program would test the default build twice).
#   2. THE FOLD FIRES.  At -O2 every positive function holds exactly the
#      expected fused instruction and no load/increment/store relay.
#   3. NEGATIVE CONTROL.  inc_and_return returns the new value: it keeps its
#      load and must not be fused.
#   4. NARROW IMMEDIATES are in range for the operand width (`addb $44`, not
#      a `addb $300` the assembler has to truncate).
#   5. inc_alias: the index's zero-extension survives a memory-destination
#      add, so exactly one `movl %edx, %edx` remains.
#   6. MUTATION.  With `CCC_PEEPHOLE_SKIP=mem_rmw` contract 2's checker must
#      FAIL; otherwise it would pass on a tree where the fold never runs.
# ============================================================================
set -euo pipefail

here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH= cd -- "$here/../.." && pwd)
CCC=${CCC:-$root/target/fastbuild/lccc}
GCC=${GCC:-gcc}
[[ -x $CCC ]] || { echo "check_rmw_index_zext_codegen: lccc not found at $CCC" >&2; exit 1; }
command -v "$GCC" >/dev/null 2>&1 || { echo "check_rmw_index_zext_codegen: no gcc oracle" >&2; exit 1; }

src=$here/rmw_index_zext.c
work=$(mktemp -d "${TMPDIR:-/tmp}/lccc-rmw-zext.XXXXXX")
trap 'rm -rf "$work"' EXIT
fail=0
bad() { printf '  FAIL: %s\n' "$*" >&2; fail=1; }

echo "  contract 1: oracle parity, -O0..-O3 x {default, CCC_PEEPHOLE_SKIP=mem_rmw}"
"$GCC" -O2 -w "$src" -o "$work/ref"
expected=$("$work/ref")
for opt in -O0 -O1 -O2 -O3; do
    for skip in "" mem_rmw; do
        exe="$work/t${opt}${skip:+-$skip}"
        if ! env ${skip:+CCC_PEEPHOLE_SKIP=$skip} "$CCC" "$opt" "$src" -o "$exe" >"$exe.log" 2>&1; then
            bad "$opt [${skip:-default}]: lccc failed to build"; continue
        fi
        got=$("$exe") || { bad "$opt [${skip:-default}]: non-zero exit"; continue; }
        [[ $got == "$expected" ]] || bad "$opt [${skip:-default}]: stdout differs from the GCC oracle"
    done
done

"$CCC" -O2 -S "$src" -o "$work/on.s"
CCC_PEEPHOLE_SKIP=mem_rmw "$CCC" -O2 -S "$src" -o "$work/off.s"

# Contracts 2-5 on one assembly file; exit status 0 iff all hold.
check() {
    python3 - "$1" <<'PY'
import re
import sys

text = open(sys.argv[1], encoding="utf-8").read()


def body(name):
    m = re.search(rf"(?ms)^{re.escape(name)}:\n(.*?)^\s*\.size\s+{re.escape(name)},", text)
    if not m:
        raise SystemExit(f"  FAIL: no assembly body for {name}")
    return [
        line.strip()
        for line in m.group(1).splitlines()
        if line.strip() and not line.strip().startswith((".", "#"))
    ]


failures = []
MEM = r"\(%rdi, %r[a-z0-9]+, \d\)"
positive = {
    "inc_shr24": "addl $1,",
    "inc_byte": "addl $1,",
    "inc_shr8": "addl $1,",
    "inc_mask": "addl $1,",
    "inc_param": "addl $1,",
    "add_param": "addl $7,",
    "sub_param": "subl $3,",
}
for fn, want in positive.items():
    lines = body(fn)
    fused = [l for l in lines if re.fullmatch(rf"{re.escape(want)} {MEM}", l)]
    loads = [l for l in lines if re.fullmatch(rf"movl {MEM}, %\w+", l)]
    stores = [l for l in lines if re.fullmatch(rf"movl %\w+, {MEM}", l)]
    if len(fused) != 1 or loads or stores:
        failures.append(f"{fn}: want one `{want} (mem)` and no load/store relay; got {lines}")

# Contract 3: the returned new value keeps its load.
lines = body("inc_and_return")
if not any(re.fullmatch(rf"movl {MEM}, %\w+", l) for l in lines):
    failures.append(f"inc_and_return: the load is gone -- a live value was fused away: {lines}")

# Contract 4: narrow immediates within the operand width.
for fn, want in {
    "add_byte_300": r"addb \$44, ",
    "sub_byte_200": r"subb \$200, ",
    "add_word_70000": r"addw \$4464, ",
}.items():
    lines = body(fn)
    if not any(re.match(want, l) for l in lines):
        failures.append(f"{fn}: want `{want.replace(chr(92), '')}(mem)`: {lines}")
for l in re.findall(r"(?m)^\s*(?:add|sub)([bw]) \$(-?\d+),", text):
    width, imm = l[0], int(l[1])
    bits = 8 if width == "b" else 16
    if not -(1 << (bits - 1)) <= imm < (1 << bits):
        failures.append(f"out-of-range {bits}-bit immediate ${imm}")

# Contract 5: one zero-extension of the shared index.
lines = body("inc_alias")
n = sum(1 for l in lines if l == "movl %edx, %edx")
adds = sum(1 for l in lines if re.fullmatch(r"addl \$1, \(%r[a-z0-9]+, %rdx, 4\)", l))
if n != 1 or adds != 2:
    failures.append(f"inc_alias: want 2 memory adds and 1 `movl %edx, %edx`; got {lines}")

for f in failures:
    print("  FAIL:", f, file=sys.stderr)
sys.exit(1 if failures else 0)
PY
}

echo "  contracts 2-5: fused RMW per shape, live-value control, narrow immediates, one zext"
check "$work/on.s" || bad "codegen contracts (see above)"

echo "  contract 6: the same checker fails with the fold skipped"
if check "$work/off.s" 2>/dev/null; then
    bad "CCC_PEEPHOLE_SKIP=mem_rmw still satisfies contract 2: the checker or the knob is inert"
fi

if [[ $fail -ne 0 ]]; then
    echo "check_rmw_index_zext_codegen: FAILED" >&2
    exit 1
fi
echo "check_rmw_index_zext_codegen: PASS (oracle parity -O0..-O3 x skip, fused RMW on 7 shapes, live-value control, narrow immediates, mutation arm)"
