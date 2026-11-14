#!/usr/bin/env bash
# LICM must treat an intrinsic's `dest_ptr` as a value operand.
#
# The regression this pins: `Setzero128 { dest_ptr: %slot, args: [] }`
# reported ZERO operands to licm's invariance walk, so it hoisted into the
# preheader while the in-loop `Alloca %slot` that defines its result slot
# stayed behind — `CCC_VERIFY_IR=1` flagged 260 dominance violations on
# simd_crc_adler alone (`v37`/`v44` used in block #0, defined in block #2),
# re-reported after every later pass.  The fix has two halves, and this
# gate pins BOTH:
#
#   1. fail-closed: `dest_ptr` joins the invariance requirement, so the
#      intrinsic can never again hoist above its slot definition
#      (=> zero `[ir-verify]` output on the three TUs that showed it);
#   2. no regression in the won codegen: fixed-size allocas are themselves
#      hoistable (static frame-slot declarations — every backend's
#      `calculate_stack_space` walks ALL blocks), so slot + intrinsic BOTH
#      leave the loop and the zero-materialization stays out of the hot
#      16-byte chunk loop (=> no `pxor` inside any backward-branch region
#      of `adler32_simd`).
#
# The companion unit tests live in src/passes/licm.rs
# (`intrinsic_result_slot_hoists_with_its_alloca`,
#  `intrinsic_with_variant_result_slot_stays_in_the_loop`).
set -euo pipefail

repo=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
cd "$repo"

ccc=${CCC:-target/fastbuild/lccc}
[ -x "$ccc" ] || { echo "SKIP: $ccc not built" >&2; exit 0; }

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

fail() { echo "FAIL: $1" >&2; exit 1; }

# 1) The three TUs that exposed the dominance violations compile verify-clean.
check_verify_clean() {
    local src=$1; shift
    local name
    name=$(basename "$src" .c)
    if ! CCC_VERIFY_IR=1 "$ccc" -O2 "$@" "$src" -o "$tmp/$name" 2>"$tmp/$name.err"; then
        fail "$name failed to build: $(head -2 "$tmp/$name.err" | tr '\n' ' ')"
    fi
    if grep -q '\[ir-verify\]' "$tmp/$name.err"; then
        grep '\[ir-verify\]' "$tmp/$name.err" | sed -n '1,3p' >&2
        fail "$name: licm/ivsr dominance violations are back (intrinsic dest_ptr hoisted without its slot)"
    fi
}
check_verify_clean tests/regression/simd_crc_adler.c -msse4.2 -mpclmul
check_verify_clean tests/regression/simd_maskfacts_widen_average.c
check_verify_clean tests/regression/vector_defer_multidef_slot.c -mssse3

# 2) The self-checking binary still passes at runtime.
out=$("$tmp/simd_crc_adler")
[ "$out" = "OK simd_crc_adler" ] || fail "simd_crc_adler runtime check: got '$out'"

# 3) The slot-and-intrinsic pair still hoists: no zero-materialization
#    (pxor) inside any backward-branch region of adler32_simd.  A stranded
#    Setzero128 would be the conservative-only half of the fix.
"$ccc" -O2 -msse4.2 -mpclmul -S tests/regression/simd_crc_adler.c -o "$tmp/sca.s"
python3 - "$tmp/sca.s" <<'EOF'
import re, sys
text = open(sys.argv[1]).read()
m = re.search(r'^adler32_simd:.*?^\.size adler32_simd', text, re.S | re.M)
assert m, "adler32_simd not found in assembly"
lines = m.group(0).splitlines()
labels = {}
for i, l in enumerate(lines):
    lm = re.match(r'^(\.L\w+):', l)
    if lm:
        labels[lm.group(1)] = i
in_loop = 0
for i, l in enumerate(lines):
    jm = re.search(r'\tj\w+\s+(\.L\w+)|^\s*j\w+\s+(\.L\w+)', l)
    tgt = jm and (jm.group(1) or jm.group(2))
    if tgt and tgt in labels and labels[tgt] < i:
        in_loop += sum('pxor' in b for b in lines[labels[tgt]:i + 1])
if in_loop:
    sys.exit(f"pxor inside a loop region of adler32_simd ({in_loop} site(s)): "
             "the result-slot hoist regressed to the stranded form")
EOF

echo "PASS: intrinsic result slots hoist with their allocas; CCC_VERIFY_IR clean on all three reproducer TUs"
