/* A live value in the pre-coloured %xmm1 scratch alongside a 16-byte segment
   constant store. The store's constant form materialises its high half
   through %xmm1; if the "is the scratch free" test only looks at %xmm0, the
   live value is destroyed and this program computes the wrong result. */
#include <stdio.h>
#include <stdlib.h>
#include <sys/syscall.h>
#include <unistd.h>
#include <stdint.h>
#include <string.h>
#include <sys/mman.h>
#include <errno.h>
#define DISP ((uintptr_t)1<<32)
static __int128 dst[2] __attribute__((aligned(16)));
static void mapm(const void*g,size_t n){uintptr_t lo=(uintptr_t)g+DISP,e=lo+n;
 for(uintptr_t p=lo&~(uintptr_t)4095;p<e;p+=4096){void*w=(void*)p;
  void*m=mmap(w,4096,PROT_READ|PROT_WRITE,MAP_PRIVATE|MAP_ANONYMOUS|MAP_FIXED_NOREPLACE,-1,0);
  if(m==MAP_FAILED){if(errno==EEXIST)continue;return;}
  if(m!=w){munmap(m,4096);if(mmap(w,4096,PROT_READ|PROT_WRITE,MAP_PRIVATE|MAP_ANONYMOUS|MAP_FIXED,-1,0)!=w)return;}}}
__attribute__((noinline)) static long two_wide(__int128 a, __int128 b) {
    /* both arguments must survive; the store's staging is the hazard */
    __int128 __seg_gs *p = (__int128 __seg_gs *)dst;
    *p = ((__int128)0x1122334455667788LL << 64) | 0x99aabbccddeeff00ULL;
    return (long)(a >> 64) + (long)b + (long)(*p >> 64);
}
int main(void){
    mapm(dst, sizeof dst);
    if(syscall(SYS_arch_prctl,0x1001,(void*)DISP)!=0){perror("gs");return 2;}
    __int128 a = ((__int128)11<<64)|1, b = 22;
    long want = 11 + 22 + 0x1122334455667788L;
    long got = two_wide(a,b);
    printf("%s got=%ld want=%ld\n", got==want?"ok":"WRONG", got, want);
    if (got != want) { printf("FAILED (1 failures)\n"); return 1; }
    printf("PASSED (0 failures)\n");
    return 0;
}
