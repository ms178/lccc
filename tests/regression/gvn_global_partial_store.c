#include <stdio.h>
unsigned g = 0x11223344u;
__attribute__((noinline)) unsigned f(void) {
    unsigned before = g;
    ((unsigned char *)&g)[0] = 0xaau;
    unsigned after = g;
    return after ^ before;
}
int main(void) { unsigned r=f(); printf("%08x %08x\n",g,r); return !(g==0x112233aau && r==0xeeu); }
