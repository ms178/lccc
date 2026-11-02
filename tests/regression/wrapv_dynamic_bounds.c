#include <limits.h>
#include <stdio.h>
/* Every memory access is bounded independently of the tested runtime bound. */
__attribute__((noinline)) static unsigned sum(const unsigned *a, int n) {
    unsigned s=0;
    for(int i=0;i<n;i++) {
        s += a[i & 31] + a[(i+1) & 31];
        if(i==65) break;
    }
    return s;
}
__attribute__((noinline)) static void copy(int *a, const int *b, int n) {
    for(int i=0;i<n;i++) a[i]=b[i+1];
}
int main(void) {
    unsigned a[32]; int b[33], c[32];
    for(int i=0;i<32;i++) a[i]=(unsigned)i*123456789u;
    for(int i=0;i<33;i++) b[i]=i*17-9;
    int bounds[]={INT_MIN,-1,0,1,2,31,32,65,66,INT_MAX};
    for(unsigned j=0;j<sizeof(bounds)/sizeof(bounds[0]);j++) printf("%u\n",sum(a,bounds[j]));
    for(int n=-2;n<=32;n++) {
        for(int i=0;i<32;i++) c[i]=-1;
        copy(c,b,n); unsigned h=0;
        for(int i=0;i<32;i++) h=h*33u+(unsigned)c[i];
        printf("%u\n",h);
    }
    return 0;
}
