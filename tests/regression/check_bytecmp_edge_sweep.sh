#!/usr/bin/env bash
# Byte-compare window phase: EXHAUSTIVE page-boundary sweep (x86-64).
#
# `check_bytecmp_vec_guard_page.sh` parks q at ONE safe offset (512).  The
# guard it exercises -- `q + (WIDTH-1) <= (q | 4095)` -- has all of its meaning
# at the page END, so the offsets that decide behaviour are the last WIDTH-1
# bytes of a page, and those are precisely the ones a single safe offset does
# not reach.  This gate sweeps every one of them, and does the same for the p
# side, whose bound is `end` rather than the page.
#
# The dangerous configuration, and the one a naive sweep gets wrong:
#   * p is LONG, so the phase's room test `p + (WIDTH-1) < end` passes and the
#     phase actually runs; and
#   * q's first mismatch is EARLY, so the scalar loop reads only q[0..m-1] and
#     exits; and
#   * q sits near the page end, so the WINDOW crosses into PROT_NONE even
#     though every byte the scalar program touches is legal.
# Tying p's length to m -- the obvious way to write this -- makes the room test
# fail, the phase never runs, and the gate then passes even with the page guard
# deleted outright.  That vacuity was found by mutation: with `page_ok` forced
# always-true this gate must SIGSEGV, and a sweep that does not is testing
# nothing.  Do not "simplify" the p/q length relationship back together.
set -euo pipefail

CCC=${CCC:-./target/release/lccc}
GCC=${GCC:-gcc}
command -v "$GCC" >/dev/null 2>&1 || { echo "SKIP: no $GCC on PATH"; exit 0; }

td=$(mktemp -d)
trap 'rm -rf "$td"' EXIT

cp "$(dirname "$0")/bytecmp_vec/edge_sweep.c" "$td/edge_sweep.c"

cat >"$td/kernel.c" <<'EOF'
const unsigned char *bytecmp_unsigned(const unsigned char *p,
                                      const unsigned char *end,
                                      const unsigned char *q)
{ while (p < end && *p == *q) { p++; q++; } return p; }
EOF

"$GCC" -O2 "$td/edge_sweep.c" "$td/kernel.c" -o "$td/sweep.gcc"
gcc_out=$("$td/sweep.gcc") || { echo "FAIL: the GCC reference itself failed"; exit 1; }
echo "gcc  $gcc_out"

"$CCC" -O2 "$td/edge_sweep.c" "$td/kernel.c" -o "$td/sweep.lccc" ||
    { echo "FAIL lccc -O2: did not link"; exit 1; }
lccc_out=$("$td/sweep.lccc") || rc=$?
if [ "${rc:-0}" -ne 0 ]; then
    echo "FAIL lccc -O2: exited ${rc} (a SIGSEGV here means the page guard let a
         window read past the mapping)"
    exit 1
fi
echo "lccc $lccc_out"

# The gate is only meaningful if it actually vectorizes; otherwise it is
# testing the scalar fallback and would pass against any compiler.
"$CCC" -O2 -S "$td/kernel.c" -o "$td/kernel.s"
if ! grep -qE 'pcmpeqb|pmovmskb' "$td/kernel.s"; then
    echo "FAIL: the kernel did not vectorize, so this gate proves nothing"
    exit 1
fi
echo "PASS bytecmp edge sweep (vectorized kernel, page edges both streams)"
