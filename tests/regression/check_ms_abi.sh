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
_Float128 __attribute__((ms_abi)) f(_Float128 x) { return x; }
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

# Fixed integer128 arguments retain semantic IR types until backend transport.
cat > "$tmp/wide-args.h" <<'C'
#define MS __attribute__((ms_abi))
typedef unsigned __int128 U;
typedef unsigned long W;
typedef U MS (*wide_cb)(U,W,U,W,U,U);
extern MS U cc_gather(U,W,U,W,U,U), gcc_gather(U,W,U,W,U,U);
extern MS W cc_fold(U), gcc_smash(U), cc_mixwide(U,double,U,float,U);
extern MS __int128 cc_signed_arg(__int128);
extern U cc_inline_wide(U);
extern MS W gcc_check_alignment(U,W,U,W,U,U);
extern W cc_alignment(U,U,U,U);
extern MS U cc_wide_vadef(U,int,...), gcc_wide_vadef(int,...);
extern U cc_wide_vacall(U,U,U,U), clang_wide_self(U,U,U,U);
struct wide_record { U bits; W tag; };
extern MS struct wide_record cc_wide_sret(U,int,...);
extern U cc_cross(U,U,U,U), cc_isolate(U), cc_indirect(wide_cb,U,U,U,U);
C
cat > "$tmp/wide-args-lccc.c" <<'C'
#include "wide-args.h"
MS U cc_gather(U a,W m,U b,W n,U c,U d) { a+=b; return a+c+d+m+n; }
MS W cc_fold(U a) { return (W)a^(W)(a>>64); }
MS __int128 cc_signed_arg(__int128 a) { return a^1; }
MS W cc_mixwide(U a,double x,U b,float y,U c) {
    return (W)a^(W)(b>>64)^(W)c^(W)(x+y);
}
static MS __attribute__((always_inline)) inline U inline_wide(U a) { return a^1; }
U cc_inline_wide(U a) { return inline_wide(a); }
U cc_cross(U a,U b,U c,U d) { return gcc_gather(a,3,b,5,c,d)^a; }
W cc_alignment(U a,U b,U c,U d) { return gcc_check_alignment(a,3,b,5,c,d); }
U cc_isolate(U a) { gcc_smash(a); return a; }
MS U cc_wide_vadef(U fixed,int n,...) {
    __builtin_ms_va_list ap; __builtin_ms_va_start(ap,n);
    U sum=fixed;
    while(n--)sum+=__builtin_va_arg(ap,U);
    __builtin_ms_va_end(ap); return sum;
}
U cc_wide_vacall(U a,U b,U c,U d) { return gcc_wide_vadef(4,a,b,c,d); }
MS struct wide_record cc_wide_sret(U fixed,int n,...) {
    __builtin_ms_va_list ap; __builtin_ms_va_start(ap,n);
    U sum=fixed;
    for(int i=0;i<n;++i)sum+=__builtin_va_arg(ap,U);
    __builtin_ms_va_end(ap);
    struct wide_record out={sum,(W)n}; return out;
}
U cc_indirect(wide_cb cb,U a,U b,U c,U d) { return cb(a,3,b,5,c,d)^a; }
C
cat > "$tmp/wide-args-oracle.c" <<'C'
#include "wide-args.h"
MS U gcc_gather(U a,W m,U b,W n,U c,U d) { a+=b; return a+c+d+m+n; }

/* A callee may overwrite its caller-owned by-value image, never the source. */
__asm__(".text\n.globl gcc_smash\ngcc_smash:\nmovq $0,(%rcx)\nmovq $0,8(%rcx)\nxorl %eax,%eax\nret\n");
__asm__(".text\n.globl gcc_check_alignment\ngcc_check_alignment:\nmovq %rcx,%rax\norq %r8,%rax\norq 40(%rsp),%rax\norq 48(%rsp),%rax\nandq $15,%rax\nret\n");
int main(void) {
    W words[]={0,1,0xffffffffffffffffUL,0x8000000000000000UL,0x123456789abcdef0UL};
    wide_cb volatile callback=cc_gather;
    for(int i=0;i<5;++i)for(int j=0;j<5;++j){
        U a=((U)words[i]<<64)|words[j],b=~a,c=a^((U)1<<100),d=a+37;
        U expected=a+b+c+d+8;
        if(cc_gather(a,3,b,5,c,d)!=expected||callback(a,3,b,5,c,d)!=expected)return 1;
        if(cc_cross(a,b,c,d)!=(expected^a))return 2;
        if(cc_indirect(gcc_gather,a,b,c,d)!=(expected^a))return 3;
        if(cc_isolate(a)!=a)return 4;
        if(cc_alignment(a,b,c,d))return 9;
        if(clang_wide_self(a,b,c,d)!=a+b+c+d)return 12;
        if(cc_wide_vadef(a,3,b,c,d)!=a+b+c+d)return 10;
        if(cc_wide_vacall(a,b,c,d)!=a+b+c+d)return 11;
        struct wide_record record=cc_wide_sret(a,3,b,c,d);
        if(record.bits!=a+b+c+d||record.tag!=3)return 13;
        if(cc_fold(a)!=(words[i]^words[j]))return 5;
        if((U)cc_signed_arg((__int128)a)!=(a^1))return 6;
        if(cc_inline_wide(a)!=(a^1))return 7;
        if(cc_mixwide(a,3.0,b,5.0f,c)!=((W)a^(W)(b>>64)^(W)c^8UL))return 8;
    }
    return 0;
}
C
# GCC 16.2 fails its own __builtin_ms_va_list / integer128 self-check:
# its caller passes an indirect image, but va_arg reads inline aligned bytes.
# Use the pinned, self-checked Clang receiver for this direction, not a broken
# reference. GCC still produces all callers and the fixed-signature oracle.
. "$HERE/lib/clang_oracle.sh"
require_clang_oracle check_ms_abi_wide_variadic
cat > "$tmp/wide-args-clang.c" <<'C'
#include "wide-args.h"
MS U gcc_wide_vadef(int n,...) {
    __builtin_ms_va_list ap; __builtin_ms_va_start(ap,n);
    U sum=0;
    while(n--)sum+=__builtin_va_arg(ap,U);
    __builtin_ms_va_end(ap); return sum;
}
U clang_wide_self(U a,U b,U c,U d) { return gcc_wide_vadef(4,a,b,c,d); }
C
"$CLANG_ORACLE_BIN" -O2 -c "$tmp/wide-args-clang.c" -o "$tmp/wide-args-clang.o"
"$GCC" -O2 -c "$tmp/wide-args-oracle.c" -o "$tmp/wide-args-oracle.o"
for opt in -O0 -O2 -Os '-O2 -mpreferred-stack-boundary=3'; do
  read -r -a flags <<< "$opt"
  "$CCC" "${flags[@]}" -c "$tmp/wide-args-lccc.c" -o "$tmp/wide-args-lccc.o"
  "$GCC" "$tmp/wide-args-lccc.o" "$tmp/wide-args-oracle.o" "$tmp/wide-args-clang.o" -o "$tmp/wide-args-check"
  "$tmp/wide-args-check"
  echo "PASS Microsoft integer128 arguments $opt: four copies, overflow, indirect, mutation isolation and full-width results"
done
cat > "$tmp/wide-args-nosse.c" <<'C'
unsigned long __attribute__((ms_abi)) cc_fold(unsigned __int128 a) {
    return (unsigned long)a^(unsigned long)(a>>64);
}
C
"$CCC" -O2 -mno-sse -mno-sse2 -S "$tmp/wide-args-nosse.c" -o "$tmp/wide-args-nosse.s"
if grep -Eq '%[xyz]mm[0-9]' "$tmp/wide-args-nosse.s"; then
  echo 'FAIL wide integer arguments introduced forbidden vector registers' >&2; exit 1
fi
