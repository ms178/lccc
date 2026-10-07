/* A real U32 wrap with no signed cast. 16 GiB of VIRTUAL address space,
 * only two pages committed; guard pages turn a wrong pointer bump into a fault.
 * Linux LP64 runtime contract, not an i686 or remote-CE resource requirement. */
#include <stdint.h>
#include <stdio.h>
#include <sys/mman.h>
#include <unistd.h>
__attribute__((noinline)) static unsigned scan(const unsigned *p, uint32_t i, unsigned n) {
    unsigned s=0;
    for(unsigned k=0;k<n;++k,++i) s+=p[i];
    return s;
}
int main(void) {
#if UINTPTR_MAX < 0xFFFFFFFFFFFFFFFFull
    /* 2^32 uint32_t elements is 16 GiB of index space; `(UINT64_C(1)<<32) *
     * sizeof(unsigned)` cannot be represented by an ILP32 size_t and the span
     * collapses to 0, after which the writes go through a wild pointer. Skip
     * loudly rather than compute a bogus mapping. */
    puts("ivsr_unsigned_sparse_wrap: SKIP (needs a 64-bit address space)");
    return 0;
#else
    size_t page=(size_t)sysconf(_SC_PAGESIZE);
    size_t len=(UINT64_C(1)<<32)*sizeof(unsigned);
    char *map=mmap(0,len+2*page,PROT_NONE,MAP_PRIVATE|MAP_ANONYMOUS|MAP_NORESERVE,-1,0);
    if(map==MAP_FAILED) { perror("sparse mmap"); return 1; }
    unsigned *p=(unsigned *)(map+page);
    if(mprotect(p,page,PROT_READ|PROT_WRITE) || mprotect((char *)p+len-page,page,PROT_READ|PROT_WRITE)) return 2;
    p[UINT32_MAX-1]=17; p[UINT32_MAX]=29; p[0]=43; p[1]=71;
    unsigned r=scan(p,UINT32_MAX-1,4);
    munmap(map,len+2*page);
    if(r!=160) return 3;
    puts("ivsr_unsigned_sparse_wrap: OK");
    return 0;
#endif
}
