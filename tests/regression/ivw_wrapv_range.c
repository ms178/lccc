/* Deliberate signed wrapping: compile with -fwrapv. Every index is masked. */
#include <stdio.h>
#include <stdint.h>
__attribute__((noinline)) static unsigned f0(const unsigned *p) { unsigned sum=0; int trips=0; for(int i=(0);i<=(63);i+=(1)) { sum+=p[(unsigned)(i*(4))&63U]; if(++trips==32) break; } return sum; }
__attribute__((noinline)) static unsigned f1(const unsigned *p) { unsigned sum=0; int trips=0; for(int i=(0);i<=(63);i+=(2)) { sum+=p[(unsigned)(i*(3))&63U]; if(++trips==32) break; } return sum; }
__attribute__((noinline)) static unsigned f2(const unsigned *p) { unsigned sum=0; int trips=0; for(int i=(3);i<=(63);i+=(4)) { sum+=p[(unsigned)(i*(8))&63U]; if(++trips==32) break; } return sum; }
__attribute__((noinline)) static unsigned f3(const unsigned *p) { unsigned sum=0; int trips=0; for(int i=(0);i<=(7);i+=(1)) { sum+=p[(unsigned)(i*(1000000000))&63U]; if(++trips==32) break; } return sum; }
__attribute__((noinline)) static unsigned f4(const unsigned *p) { unsigned sum=0; int trips=0; for(int i=(2147483640);i<=(2147483647);i+=(1)) { sum+=p[(unsigned)(i*(4))&63U]; if(++trips==32) break; } return sum; }
__attribute__((noinline)) static unsigned f5(const unsigned *p) { unsigned sum=0; int trips=0; for(int i=(2147483644);i<=(2147483647);i+=(2)) { sum+=p[(unsigned)(i*(1))&63U]; if(++trips==32) break; } return sum; }
__attribute__((noinline)) static unsigned f6(const unsigned *p) { unsigned sum=0; int trips=0; for(int i=(-2147483648);i<=(31);i+=(1)) { sum+=p[(unsigned)(i*(4))&63U]; if(++trips==32) break; } return sum; }
__attribute__((noinline)) static unsigned f7(const unsigned *p) { unsigned sum=0; int trips=0; for(int i=(1073741824);i<=(1073741827);i+=(1)) { sum+=p[(unsigned)(i*(4))&63U]; if(++trips==32) break; } return sum; }
int main(void) { unsigned p[64]; for(unsigned i=0;i<64;++i)p[i]=i*37+11; unsigned sum=f0(p)+f1(p)+f2(p)+f3(p)+f4(p)+f5(p)+f6(p)+f7(p); if(sum!=206604U)return 1; puts("ivw_wrapv_range: PASS"); return 0; }
