#!/usr/bin/env bash
# Codegen gate for direct narrow stores of wide unsigned constants.
#
# A store of 3041712678u (above i32::MAX) to a 32-bit destination must be a
# single `movl $3041712678, <addr>` — the raw imm32 field of movl encodes the
# full unsigned range. The `movabsq $imm, %rax; movl %eax, addr` relay (the
# pre-fix accumulator staging) must not appear for:
#   * a register-homed pointer destination  (p[1] = 3041712678u)
#   * a global symbol destination           (ga[0] = 3041712678u)
# Runtime semantics are covered differentially by wide_const_imm_stores.c.
set -euo pipefail

CCC=${CCC:-./target/fastbuild/lccc}
td=$(mktemp -d)
trap 'rm -rf "$td"' EXIT

cat >"$td/w.c" <<'EOF'
unsigned ga[8];
void f(unsigned *p) { p[0] = 0xFFFFFFFFu; p[1] = 3041712678u; p[2] = 2147483648u; }
void g(void) { ga[0] = 3041712678u; ga[1] = 4294967295u; }
EOF

"$CCC" -O2 -S "$td/w.c" -o "$td/w.s"
body() { sed -n "/^$1:/,/^\.size[[:space:]]*$1/p" "$td/w.s"; }
f_body=$(body f)
g_body=$(body g)

if grep -qF 'movabsq $' <<<"$f_body"; then
    echo "FAIL: f() staged a constant through %rax; want direct movl immediates:" >&2
    echo "$f_body" >&2
    exit 1
fi
if grep -qF 'movabsq $' <<<"$g_body"; then
    echo "FAIL: g() staged a constant through %rax for a global store; want direct movl \$imm, sym(%rip):" >&2
    echo "$g_body" >&2
    exit 1
fi
n_direct=$(grep -cE 'movl \$[0-9]+,' <<<"$f_body" || true)
if [ "$n_direct" -lt 3 ]; then
    echo "FAIL: f() should hold three direct imm32 movl stores; got:" >&2
    echo "$f_body" >&2
    exit 1
fi
if ! grep -qF 'movl $3041712678, ga(%rip)' <<<"$g_body" \
   || ! grep -qF 'movl $4294967295, ga+4(%rip)' <<<"$g_body"; then
    echo "FAIL: g() should store both constants directly to ga(%rip)/ga+4(%rip); got:" >&2
    echo "$g_body" >&2
    exit 1
fi
echo "OK wide_const_imm_stores"
