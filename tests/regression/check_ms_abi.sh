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
__int128 __attribute__((ms_abi)) f(__int128 x) { return x; }
C
if "$CCC" -c "$tmp/unsupported.c" -o "$tmp/bad.o" > "$tmp/error" 2>&1; then
  echo 'FAIL: accepted unimplemented Microsoft wide-scalar lowering' >&2; exit 1
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
# Non-kernel GNU Microsoft integer128 return ABI: one XMM0 payload, not RDX:RAX.
cat > "$tmp/wide.h" <<'C'
#define MS __attribute__((ms_abi))
typedef unsigned long word;
typedef unsigned __int128 u128;
typedef __int128 s128;
typedef u128 MS (*retfn)(word,word);
extern MS u128 cc_u128(word,word), gcc_u128(word,word), ms_chain(word,word);
extern MS s128 cc_s128(long,long);
extern MS u128 variadic_u128(int,...);
extern MS u128 cc_mixed(word,double,word,float,word,double), gcc_mixed(word,double,word,float,word,double);
extern u128 direct_mixed(word,word);
extern u128 invoke_u128(retfn,word,word,int), direct_u128(word,word);
C
cat > "$tmp/wide-lccc.c" <<'C'
#include "wide.h"
MS u128 cc_u128(word low,word high) { return ((u128)high<<64)|low; }
MS u128 cc_mixed(word low,double x,word high,float y,word mask,double z) {
    return ((u128)(high^(word)y)<<64)|(low^mask^(word)(x+z));
}
u128 direct_mixed(word low,word high) { return gcc_mixed(low,3.0,high,5.0f,0xa5a5UL,7.0); }
MS s128 cc_s128(long low,long high) { return (s128)(((u128)(word)high<<64)|(word)low); }
MS u128 ms_chain(word low,word high) { return gcc_u128(low,high); }
u128 direct_u128(word low,word high) { return gcc_u128(low,high); }
u128 invoke_u128(retfn fn,word low,word high,int which) {
    retfn volatile saved=fn;
    if(which==0)return fn(low,high);
    if(which==1)return (*saved)(low,high);
    return (which==2?fn:cc_u128)(low,high);
}
MS u128 variadic_u128(int tag,...) {
    __builtin_ms_va_list ap; __builtin_ms_va_start(ap,tag);
    word low=__builtin_va_arg(ap,word), high=__builtin_va_arg(ap,word);
    __builtin_ms_va_end(ap);
    return ((u128)high<<64)|low;
}
C
cat > "$tmp/wide-oracle.c" <<'C'
#include "wide.h"
MS u128 gcc_u128(word low,word high) { return ((u128)high<<64)|low; }
MS u128 gcc_mixed(word low,double x,word high,float y,word mask,double z) {
    return ((u128)(high^(word)y)<<64)|(low^mask^(word)(x+z));
}
int main(void) {
    const word words[]={0,1,0xffffffffffffffffUL,0x8000000000000000UL,0x123456789abcdef0UL};
    retfn volatile ptr=cc_u128;
    for(unsigned a=0;a<5;++a)for(unsigned b=0;b<5;++b){
        word low=words[a],high=words[b];u128 expected=((u128)high<<64)|low;
        if(cc_u128(low,high)!=expected||ptr(low,high)!=expected)return 1;
        if(cc_s128((long)low,(long)high)!=(s128)expected)return 2;
        if(direct_u128(low,high)!=expected||ms_chain(low,high)!=expected)return 3;
        for(int which=0;which<4;++which)
            if(invoke_u128(gcc_u128,low,high,which)!=expected)return 4;
        if(variadic_u128(7,low,high)!=expected)return 5;
        u128 mixed=((u128)(high^5UL)<<64)|(low^0xa5a5UL^10UL);
        if(cc_mixed(low,3.0,high,5.0f,0xa5a5UL,7.0)!=mixed||direct_mixed(low,high)!=mixed)return 6;
    }
    return 0;
}
C
"$GCC" -O2 -c "$tmp/wide-oracle.c" -o "$tmp/wide-oracle.o"
for opt in -O0 -O2 -Os; do
  "$CCC" "$opt" -c "$tmp/wide-lccc.c" -o "$tmp/wide-lccc.o"
  "$GCC" "$tmp/wide-lccc.o" "$tmp/wide-oracle.o" -o "$tmp/wide-check"
  "$tmp/wide-check"
  echo "PASS Microsoft signed/unsigned integer128 returns $opt: direct/indirect, callbacks, SysV crossing and variadic definitions"
done
# Isolate integer return transport: mixed-FP functions above must not mask a
# missing XMM-return ISA check.
cat > "$tmp/wide-nosse.c" <<'C'
unsigned __int128 __attribute__((ms_abi)) wide_nosse(unsigned long a,unsigned long b) {
    return ((unsigned __int128)b<<64)|a;
}
C
if "$CCC" -O2 -mno-sse -mno-sse2 -S "$tmp/wide-nosse.c" -o "$tmp/wide-forbidden.s" > "$tmp/wide-error" 2>&1; then
  echo 'FAIL Microsoft integer128 XMM return accepted without SSE' >&2;exit 1
fi
grep -q 'SSE.*disabled' "$tmp/wide-error"
