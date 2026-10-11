#include <stdio.h>
#define FN __attribute__((noinline))
FN unsigned same(unsigned c, unsigned a, unsigned b, unsigned d, unsigned e) {
    unsigned x=c?a:b, y=c?d:e;
    return c?x:y;
}
FN unsigned mixed(unsigned c, unsigned a, unsigned b, unsigned d, unsigned e) {
    unsigned x=(unsigned char)c?a:b, y=c?d:e;
    return c?x:y;
}
int main(void) {
    unsigned state=7;
    for (unsigned i=0;i<10000;i++) {
        state=state*1664525u+1013904223u;
        unsigned c=i<257?i:state;
        if (same(c,state,i,state+1,i+1)!=(c?state:i+1) ||
            mixed(c,state,i,state+1,i+1)!=(c?((unsigned char)c?state:i):i+1)) return 1;
    }
    puts("PASS: nested-select polarity and truncated-predicate witness");
    return 0;
}
