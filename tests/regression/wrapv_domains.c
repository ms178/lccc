/* Defined only with -fwrapv: arithmetic overflow, not out-of-bounds access. */
#include <limits.h>
#include <stdio.h>
#include <stdlib.h>
__attribute__((noinline)) int address(const int *p, int start, int n) {
    int s=0;
    for(int i=start;i<start+n;i++) s+=p[i*4];
    return s;
}
__attribute__((noinline)) int triangular(int start, int n) {
    int s=0;
    for(int j=0;j<n;j++) s+=(j+start)*(j+start+1)/2;
    return s;
}
__attribute__((noinline)) int affine_guard(void) {
    int count=0;
    for(int i=0;i+INT_MAX>=0;i++) { if(++count==4) break; }
    return count;
}
__attribute__((noinline)) unsigned latch_wrap(const int *p) {
    unsigned sum=0;
    for(int i=INT_MAX-1;i!=INT_MIN+2;i++) sum+=(unsigned)p[(unsigned)i&3U];
    return sum;
}
int main(int argc,char **argv) {
    int p[64];for(int i=0;i<64;i++) p[i]=i+1;
    int which=argc>1?atoi(argv[1]):0;
    switch(which) {
      case 0: printf("%d\n",address(p,1073741824,4));break;
      case 1: printf("%d\n",triangular(46340,3));break;
      case 2: printf("%d\n",affine_guard());break;
      case 3: printf("%u\n",latch_wrap(p));break;
      default:return 2;
    }
    return 0;
}
