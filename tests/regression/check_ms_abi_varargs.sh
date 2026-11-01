#!/usr/bin/env bash
# GCC builds only the non-kernel oracle; exercise MS and SysV lists in one TU.
set -euo pipefail
CCC=${CCC:-./target/fastbuild/lccc}
GCC=${GCC:-gcc}
tmp=$(mktemp -d)
trap 'rm -rf -- "$tmp"' EXIT
cat > "$tmp/types.h" <<'C'
#define MS __attribute__((ms_abi))
struct small { unsigned char bytes[3]; };
struct big { long a,b,c; };
typedef long vector2 __attribute__((vector_size(16)));
extern MS long variadic(int,...), fifth(long,long,long,long,long,...);
extern MS unsigned long wide(int,...), wide_extra(int,...);
extern MS struct big variadic_return(int,...);
extern long sysv_variadic(int,...);
C
cat > "$tmp/lccc.c" <<'C'
#include "types.h"
_Static_assert(sizeof(__builtin_va_list)==24,"SysV list size");
_Static_assert(_Alignof(__builtin_va_list)==8,"SysV list alignment");
_Static_assert(sizeof(__builtin_ms_va_list)==8,"Microsoft cursor size");
_Static_assert(__builtin_types_compatible_p(__builtin_ms_va_list,char*),"GCC cursor type");
static long consume(__builtin_ms_va_list *p) { return __builtin_va_arg(*p,long); }
MS long variadic(int tag,...) {
    __builtin_ms_va_list ap, saved;
    __builtin_ms_va_start(ap,tag);
    __builtin_ms_va_copy(saved,ap);
    double a=__builtin_va_arg(ap,double);
    long b=consume(&ap);
    double c=__builtin_va_arg(ap,double);
    double d=__builtin_va_arg(ap,double);
    long e=__builtin_va_arg(ap,long);
    double original=__builtin_va_arg(saved,double);
    __builtin_ms_va_end(saved);
    __builtin_ms_va_end(ap);
    return tag+(long)(a*10)+b+(long)(c*100)+(long)(d*10)+e+(a!=original);
}
MS long fifth(long a,long b,long c,long d,long e,...) {
    __builtin_ms_va_list ap;
    __builtin_ms_va_start(ap,e);
    long f=__builtin_va_arg(ap,long);
    double g=__builtin_va_arg(ap,double);
    __builtin_ms_va_end(ap);
    return a+b+c+d+e+f+(long)(g*10);
}
MS unsigned long wide(int tag,...) {
    __builtin_ms_va_list ap;
    __builtin_ms_va_start(ap,tag);
    unsigned __int128 x=__builtin_va_arg(ap,unsigned __int128);
    long double y=__builtin_va_arg(ap,long double);
    struct small z=__builtin_va_arg(ap,struct small);
    __builtin_ms_va_end(ap);
    return tag+(unsigned long)x+(unsigned long)(x>>64)+(unsigned long)y+z.bytes[0]+z.bytes[1]+z.bytes[2];
}
MS unsigned long wide_extra(int tag,...) {
    __builtin_ms_va_list ap;
    __builtin_ms_va_start(ap,tag);
    _Float128 q=__builtin_va_arg(ap,_Float128);
    float _Complex cf=__builtin_va_arg(ap,float _Complex);
    double _Complex cd=__builtin_va_arg(ap,double _Complex);
    long double _Complex cld=__builtin_va_arg(ap,long double _Complex);
    vector2 v=__builtin_va_arg(ap,vector2);
    __builtin_ms_va_end(ap);
    return tag+(unsigned long)q+(long)__real__ cf+(long)__imag__ cf+
      (long)__real__ cd+(long)__imag__ cd+(long)__real__ cld+(long)__imag__ cld+v[0]+v[1];
}
MS struct big variadic_return(int tag,...) {
    __builtin_ms_va_list ap;
    __builtin_ms_va_start(ap,tag);
    struct big r={tag,__builtin_va_arg(ap,long),__builtin_va_arg(ap,long)};
    __builtin_ms_va_end(ap);
    return r;
}
long sysv_variadic(int tag,...) {
    __builtin_va_list ap;
    __builtin_va_start(ap,tag);
    long a=__builtin_va_arg(ap,long);
    double b=__builtin_va_arg(ap,double);
    __builtin_va_end(ap);
    return tag+a+(long)(b*10);
}
C
cat > "$tmp/oracle.c" <<'C'
#include "types.h"
int main(void) {
    if(variadic(7,2.5,11L,4.25,6.5,19L)!=552)return 1;
    if(fifth(1,2,3,4,5,6L,7.5)!=96)return 2;
    struct small z={{17,29,31}};
    unsigned __int128 x=((unsigned __int128)1<<90)+5;
    if(wide(3,x,13.75L,z)!=3+5+(1UL<<26)+13+17+29+31)return 3;
    vector2 v={17,19};
    if(wide_extra(1,13.5Q,2.25f+4.5fi,11.5+3.25i,7.25L+9.5Li,v)!=86)return 6;
    struct big r=variadic_return(13,17L,19L);
    if(r.a!=13||r.b!=17||r.c!=19)return 4;
    if(sysv_variadic(1,9L,2.5)!=35)return 5;
    return 0;
}
C
"$GCC" -O2 -c "$tmp/oracle.c" -o "$tmp/oracle.o"
for opt in -O0 -O2 -Os; do
  "$CCC" "$opt" -c "$tmp/lccc.c" -o "$tmp/lccc.o"
  "$GCC" "$tmp/lccc.o" "$tmp/oracle.o" -o "$tmp/check"
  "$tmp/check"
  echo "PASS Microsoft variadic definitions $opt: FP, copy, helper, wide/record payloads, fifth fixed parameter, sret and SysV coexistence"
done

for case in wrong_start_type wrong_copy_type extra_end missing_start sysv_context fixed_context sysv_start_in_ms; do
  case "$case" in
    wrong_start_type) body='int ap; __builtin_ms_va_start(ap,n);' ;;
    wrong_copy_type) body='__builtin_ms_va_list ap; int b; __builtin_ms_va_copy(ap,b);' ;;
    extra_end) body='__builtin_ms_va_list ap; __builtin_ms_va_end(ap,ap);' ;;
    missing_start) body='__builtin_ms_va_start();' ;;
    sysv_context) body='__builtin_ms_va_list ap; __builtin_ms_va_start(ap,n);' ;;
    fixed_context) body='__builtin_ms_va_list ap; __builtin_ms_va_start(ap,n);' ;;
    sysv_start_in_ms) body='__builtin_va_list ap; __builtin_va_start(ap,n);' ;;
  esac
  signature='__attribute__((ms_abi)) void bad(int n,...)'
  [[ "$case" != sysv_context ]] || signature='void bad(int n,...)'
  [[ "$case" != fixed_context ]] || signature='__attribute__((ms_abi)) void bad(int n)'
  printf '%s { %s }\n' "$signature" "$body" > "$tmp/bad.c"
  for mode in -fsyntax-only -c; do
    if "$CCC" "$mode" "$tmp/bad.c" -o "$tmp/bad.o" > "$tmp/bad.log" 2>&1; then
      echo "FAIL accepted $case $mode" >&2; exit 1
    fi
    grep -Eq 'Microsoft|argument count|__builtin_ms_va_start' "$tmp/bad.log"
  done
done
echo 'PASS Microsoft variadic malformed cursors/arity/context rejected in syntax-only and code-generation modes'
