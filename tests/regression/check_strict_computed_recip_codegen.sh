#!/usr/bin/env bash
# PERF-41 structural guard: AVX2 computes reciprocal/products packed, while
# scalar VADDSD retains source lane order. SSE2 uses two packets and ADDSD;
# neither spelling may exceed the resolved target's ISA permission.
set -euo pipefail
CCC=${CCC:-./target/fastbuild/lccc}
dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
tmp=$(mktemp -d "${TMPDIR:-/tmp}/lccc-strict-recip.XXXXXX")
trap 'rm -rf "$tmp"' EXIT
src="$dir/vectorize_strict_computed_recip.c"

"$CCC" -O2 -march=x86-64-v3 -S "$src" -o "$tmp/v3.s"
# The VEX pack must be correct at the emitter, not depend on the optional
# final-text promotion pass to remove a costly AVX/SSE transition.
CCC_NO_VEX_PROMOTE=1 "$CCC" -O2 -march=x86-64-v3 -S "$src" -o "$tmp/no-promote.s"
"$CCC" -O2 -march=x86-64 -S "$src" -o "$tmp/baseline.s"
"$CCC" -O2 -S "$src" -o "$tmp/default.s"
"$CCC" -O2 -mno-avx2 -S "$src" -o "$tmp/no-avx2.s"
if "$CCC" -O2 -mno-sse2 -msse -S "$src" -o "$tmp/sse1.s" 2>"$tmp/sse1.err"; then
    echo "SSE1-only target unexpectedly accepted this F64 kernel" >&2
    exit 1
fi
grep -q 'requires SSE2' "$tmp/sse1.err"
CCC_NO_XMM_REGALLOC=1 "$CCC" -O2 -S "$src" -o "$tmp/sse2-stack.s"
# The four-wide AVX2 spelling still requires SSE4.1. AVX-only may use
# the two-packet SSE2 spelling; AVX2 with an explicit SSE4.1 denial retains
# the original scalar fallback rather than mixing legacy and YMM code.
"$CCC" -O2 -march=x86-64-v2 -mavx -msse4.1 -S "$src" -o "$tmp/avx-only.s"
"$CCC" -O2 -mavx2 -mno-sse4.1 -S "$src" -o "$tmp/no-sse41.s"
# Exercise the private-allocation fallback too: no scalar XMM homes means the
# packed product must spill once and be read as offsets 0, 8, 16, 24.
CCC_NO_XMM_REGALLOC=1 "$CCC" -O2 -march=x86-64-v3 -S "$src" -o "$tmp/stack.s"

python3 - "$tmp/v3.s" "$tmp/baseline.s" "$tmp/avx-only.s" "$tmp/no-sse41.s" "$tmp/stack.s" "$tmp/no-promote.s" "$tmp/default.s" "$tmp/no-avx2.s" "$tmp/sse1.err" "$tmp/sse2-stack.s" <<'PY'
import re
import sys

v3 = open(sys.argv[1], encoding="utf-8").read()
baseline = open(sys.argv[2], encoding="utf-8").read()
avx_only = open(sys.argv[3], encoding="utf-8").read()
no_sse41 = open(sys.argv[4], encoding="utf-8").read()
stack = open(sys.argv[5], encoding="utf-8").read()
no_promote = open(sys.argv[6], encoding="utf-8").read()
default = open(sys.argv[7], encoding="utf-8").read()
no_avx2 = open(sys.argv[8], encoding="utf-8").read()


def body(text, name):
    match = re.search(rf"(?ms)^{re.escape(name)}:\n(.*?)^\.size {re.escape(name)},", text)
    if not match:
        raise SystemExit(f"missing assembly body for {name}")
    return match.group(1)


packed = body(v3, "ordered_recip_sum")
if "vcvtdq2pd" not in packed or "vdivpd" not in packed or "vmulpd" not in packed:
    raise SystemExit("v3 strict reciprocal loop lost packed conversion/division/multiply")
# The four i32 lanes are built entirely with VEX. Legacy movd/pinsrd
# between VEX-256 instructions was correct but ~28x slower on the measured
# Xeon VM; the optional final peephole is not an authority for this contract.
for name, text in (("normal", v3), ("no final VEX promotion", no_promote),
                   ("no XMM register allocation", stack)):
    pack_body = body(text, "ordered_recip_sum")
    pack = re.search(r"(?s)\bvmovd %\w+, %xmm0\n(.*?)\bvcvtdq2pd %xmm0, %ymm0", pack_body)
    if not pack or len(re.findall(r"\bvpinsrd \$[123], %\w+, %xmm0, %xmm0", pack.group(1))) != 3:
        raise SystemExit(f"{name}: strict reciprocal pack lacks VEX movd + three in-place VEX inserts")
    if re.search(r"(?<![\w])(?:movd|pinsrd)\b", pack_body):
        raise SystemExit(f"{name}: legacy SSE lane pack reintroduced in AVX2 loop")
adds = list(re.finditer(r"\bvaddsd\b", packed))
if len(adds) < 4:
    raise SystemExit("v3 strict reciprocal loop lost ordered scalar lane adds")
if "vaddpd" in packed:
    raise SystemExit("strict reciprocal loop formed a reassociated packed add")
for name, text in (("baseline", baseline), ("default (plain x86-64)", default), ("AVX2 denial", no_avx2),
                   ("AVX-only", avx_only), ("AVX2 without SSE4.1", no_sse41)):
    if any(op in text for op in ("vcvtdq2pd", "vdivpd", "vmulpd")):
        raise SystemExit(f"strict reciprocal lowering escaped its AVX2+SSE4.1 gate ({name})")

for name, text in (("baseline", baseline), ("default", default), ("AVX2 denied", no_avx2),
                   ("SSE2 stack", open(sys.argv[10], encoding="utf-8").read())):
    code = body(text, "ordered_recip_sum")
    if len(re.findall(r"\bdivpd\b", code)) < 2 or len(re.findall(r"\bcvtdq2pd\b", code)) < 2:
        raise SystemExit(f"{name}: SSE2 two-packet prefix missing")
    if re.search(r"\b(?:addpd|pinsrd|pmulld)\b|%ymm", code):
        raise SystemExit(f"{name}: reassociation or a post-SSE2 instruction")
if "requires SSE2" not in open(sys.argv[9], encoding="utf-8").read():
    raise SystemExit("SSE1 permission did not diagnose unsupported F64")

stack_body = body(stack, "ordered_recip_sum")
if not re.search(r"\bvmovupd\s+%ymm0,\s*\(%rax\)", stack_body):
    raise SystemExit("no-XMM fallback did not spill the packed product to its private alloca")
ordered_spill_adds = re.search(
    r"(?ms)\bvaddsd\s+\(%rax\),\s*%xmm0,\s*%xmm0\n"
    r"\s*vaddsd\s+8\(%rax\),\s*%xmm0,\s*%xmm0\n"
    r"\s*vaddsd\s+16\(%rax\),\s*%xmm0,\s*%xmm0\n"
    r"\s*vaddsd\s+24\(%rax\),\s*%xmm0,\s*%xmm0",
    stack_body,
)
if not ordered_spill_adds:
    raise SystemExit("no-XMM fallback lost ordered scalar adds from scratch offsets 0/8/16/24")
PY
