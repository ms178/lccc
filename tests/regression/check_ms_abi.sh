#!/usr/bin/env bash
# Non-kernel cross-compiler ABI tests. Oracle objects are intentionally built
# with GCC; no boot/kernel target code is involved in this regression.
set -euo pipefail
CCC=${CCC:-./target/fastbuild/lccc}
GCC=${GCC:-gcc}
HERE=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
tmp=$(mktemp -d)
trap 'rm -rf -- "$tmp"' EXIT
"$GCC" -O2 -c "$HERE/fixtures/ms_abi/oracle.c" -o "$tmp/oracle.o"
"$GCC" -c "$HERE/fixtures/ms_abi/probes.S" -o "$tmp/probes.o"
for opt in '-O0' '-O2' '-Os' '-O2 -fstack-clash-protection'; do
  read -r -a flags <<< "$opt"
  "$CCC" "${flags[@]}" -c "$HERE/fixtures/ms_abi/lccc.c" -o "$tmp/lccc.o"
  "$GCC" "$tmp/lccc.o" "$tmp/oracle.o" "$tmp/probes.o" -o "$tmp/check"
  "$tmp/check"
  readelf --debug-dump=frames "$tmp/lccc.o" > "$tmp/frames"
  for reg in rdi rsi xmm6 xmm7 xmm8 xmm9 xmm10 xmm11 xmm12 xmm13 xmm14 xmm15; do
    grep -Eq "DW_CFA_offset: r[0-9]+ \($reg\) at cfa-" "$tmp/frames"
  done
done
cat > "$tmp/kernel.c" <<'C'
typedef long __attribute__((ms_abi)) (*callback)(long,long,long,long,long);
long __attribute__((ms_abi,noinline)) entry(long a,long b,long c,long d,long e) {
  return a+b+c+d+e;
}
long invoke(callback p) { return p(1,2,3,4,5); }
C
"$CCC" -O2 -mno-sse -mno-sse2 -S "$tmp/kernel.c" -o "$tmp/kernel.s"
if grep -Eq '%[xyz]mm[0-9]' "$tmp/kernel.s"; then
  echo 'FAIL Microsoft ABI integer-only/no-SSE emission introduced vector instructions' >&2
  exit 1
fi
# Unsupported classes must still fail closed, never fall through to SysV.
cat > "$tmp/unsupported.c" <<'C'
struct record { long a,b,c; };
struct record __attribute__((ms_abi)) f(struct record x) { return x; }
C
if "$CCC" -c "$tmp/unsupported.c" -o "$tmp/bad.o" > "$tmp/error" 2>&1; then
  echo 'FAIL: accepted unimplemented Microsoft aggregate lowering' >&2; exit 1
fi
grep -q 'refusing an incorrect ABI' "$tmp/error"
for declarations in \
  '__attribute__((ms_abi)) long f(long); __attribute__((sysv_abi)) long f(long);' \
  '__attribute__((ms_abi)) long f(long); __attribute__((sysv_abi)) long f(long x) { return x; }'; do
  printf '%s\n' "$declarations" > "$tmp/conflict.c"
  if "$CCC" -fsyntax-only "$tmp/conflict.c" > "$tmp/error" 2>&1; then
    echo 'FAIL: accepted conflicting explicit calling conventions' >&2; exit 1
  fi
  grep -q 'conflicting calling convention' "$tmp/error"
done
echo 'PASS Microsoft x64 ABI focused interoperability and fail-closed boundaries'
