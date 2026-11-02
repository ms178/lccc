#!/usr/bin/env bash
# 64-bit division width bypass — emitted-assembly contract (x86-64).
#
# `X86Tune::bypass_div64()` is a *tuning* decision, so it has two failure modes
# that the C regression fixture cannot see:
#
#   * SILENTLY OFF.  The guard disappears for a row whose measured divider ratio
#     says it should be there (e.g. someone "fixes" `div64_latency_min` to the
#     published minimum, which flips the ≥2× predicate off for every row — see
#     `engineering/subsystems/cpu-model.md` §5).  `cpu_model_div64_bypass_skylake`
#     still passes: without the guard, `divq` is simply the old, slower path.
#   * SILENTLY ON.  The guard appears where it must not (an off row, or with
#     `CCC_NO_DIV64_BYPASS=1` set), changing codegen for a CPU whose divider does
#     not benefit.  Same test still passes, same reason.
#
# Both are assembly facts, so this gate asserts them on the emitted text.  The
# three probes share one binary and one source:
#
#   POSITIVE        -mtune=skylake      → guard + `divl` fast path + `divq` slow
#   NEGATIVE (row)  -mtune=raptorlake   → no guard
#   NEGATIVE (knob) -mtune=skylake + CCC_NO_DIV64_BYPASS=1 → no guard
#
# plus a runtime differential so the gate also fails if the guarded form is ever
# wrong (not only missing).
set -euo pipefail

CCC=${CCC:-./target/fastbuild/lccc}
GCC=${GCC:-gcc}
command -v "$GCC" >/dev/null 2>&1 || { echo "SKIP: no $GCC on PATH"; exit 0; }

td=$(mktemp -d)
trap 'rm -rf "$td"' EXIT

cat >"$td/div64.c" <<'EOF'
#include <stdint.h>
/* One 64-bit divide per flavour, no 32-bit divides: `divl` in the output can
   only come from the bypass. */
uint64_t u64div(uint64_t a, uint64_t b) { return a / b; }
int64_t  s64div(int64_t a, int64_t b)   { return a / b; }
uint64_t u64rem(uint64_t a, uint64_t b) { return a % b; }
EOF

"$CCC" -O2 -mtune=skylake -S "$td/div64.c" -o "$td/skl.s"
"$CCC" -O2 -mtune=raptorlake -S "$td/div64.c" -o "$td/rpl.s"
CCC_NO_DIV64_BYPASS=1 "$CCC" -O2 -mtune=skylake -S "$td/div64.c" -o "$td/skl.knob.s"

status=0
python3 - "$td/skl.s" "$td/rpl.s" "$td/skl.knob.s" <<'PY' || status=$?
import re
import sys

skl, rpl, knob = (open(p).read() for p in sys.argv[1:4])

GUARD = re.compile(r"shrq\s+\$32")
FAST = re.compile(r"^\s+divl\b", re.M)
SLOW = re.compile(r"^\s+divq\b", re.M)
SIGNED_SLOW = re.compile(r"^\s+idivq\b", re.M)
LABEL = re.compile(r"^\.Ldiv64_(slow|done)_\d+:", re.M)

def report(name, ok, detail):
    print(f"  {'ok  ' if ok else 'FAIL'} {name}: {detail}")
    return ok

ok = True
# POSITIVE: the guard, both paths, and the shared label allocator's names.
ok &= report(
    "skylake emits the bypass",
    bool(GUARD.search(skl)) and bool(FAST.search(skl)) and len(set(LABEL.findall(skl))) == 2,
    f"guard={bool(GUARD.search(skl))} divl={bool(FAST.search(skl))} "
    f"labels={sorted(set(LABEL.findall(skl)))}",
)
# The source has three divides: two unsigned (`divq` in their slow paths) and
# one signed (`idivq`).  Both slow paths must survive on every probe.
ok &= report(
    "skylake keeps the 64-bit slow path",
    len(SLOW.findall(skl)) >= 2 and len(SIGNED_SLOW.findall(skl)) >= 1,
    f"divq sites={len(SLOW.findall(skl))} (2 unsigned), "
    f"idivq sites={len(SIGNED_SLOW.findall(skl))} (1 signed)",
)
# NEGATIVE CONTROLS, same binary: an off row and the kill switch.
ok &= report(
    "raptorlake does not emit the bypass",
    not GUARD.search(rpl) and not FAST.search(rpl) and not LABEL.search(rpl),
    "no guard, no divl, no .Ldiv64_* labels",
)
ok &= report(
    "CCC_NO_DIV64_BYPASS=1 wins on a bypass row",
    not GUARD.search(knob) and not FAST.search(knob) and not LABEL.search(knob),
    "no guard, no divl, no .Ldiv64_* labels",
)
# The kill switch must return the row to *exactly* the off-row text: if these
# ever differ, one of the two paths has drifted from "no bypass".
ok &= report(
    "knob output == off-row output",
    knob == rpl,
    "byte-identical text sections",
)
sys.exit(0 if ok else 1)
PY

# Runtime differential: the guarded form must agree with GCC on both sides of the
# 32-bit boundary (fast path), on wide operands (slow path), and for signed
# negatives (slow path).
cat >"$td/run.c" <<'EOF'
#include <stdint.h>
#include <stdio.h>
uint64_t u64div(uint64_t, uint64_t);
uint64_t u64rem(uint64_t, uint64_t);
int64_t  s64div(int64_t, int64_t);
static const uint64_t vals[] = {
    0, 1, 7, 100, 0x7fffffffu, 0x80000000u, 0xffffffffu, 0x100000000ull,
    0x100000001ull, 0x7fffffffffffffffull, 0x8000000000000000ull,
};
int main(void)
{
    for (unsigned i = 0; i < sizeof vals / sizeof vals[0]; i++) {
        for (unsigned j = 0; j < sizeof vals / sizeof vals[0]; j++) {
            uint64_t a = vals[i], b = vals[j];
            if (b == 0) continue;
            printf("%llu %llu %lld\n", (unsigned long long)u64div(a, b),
                   (unsigned long long)u64rem(a, b),
                   (long long)s64div((int64_t)a, (int64_t)b));
        }
    }
    return 0;
}
EOF

"$GCC" -O2 "$td/div64.c" "$td/run.c" -o "$td/ref"          # GCC: reference
"$CCC" -O2 -mtune=skylake "$td/div64.c" "$td/run.c" -o "$td/skl"
CCC_NO_DIV64_BYPASS=1 "$CCC" -O2 -mtune=skylake "$td/div64.c" "$td/run.c" -o "$td/knob"

"$td/ref" > "$td/ref.out"
"$td/skl" > "$td/skl.out"
"$td/knob" > "$td/knob.out"

if ! cmp -s "$td/ref.out" "$td/skl.out"; then
    echo "FAIL runtime: guarded form disagrees with GCC"
    diff "$td/ref.out" "$td/skl.out" | sed -n '1,10p'
    status=1
else
    echo "  ok   runtime: guarded form matches GCC over the boundary grid ($(wc -l <"$td/ref.out") cases)"
fi
if ! cmp -s "$td/ref.out" "$td/knob.out"; then
    echo "FAIL runtime: CCC_NO_DIV64_BYPASS=1 form disagrees with GCC"
    diff "$td/ref.out" "$td/knob.out" | sed -n '1,10p'
    status=1
fi

[ "$status" = 0 ] || exit "$status"
echo "ok: div64 width bypass emitted on bypass rows only, and correct where emitted"
