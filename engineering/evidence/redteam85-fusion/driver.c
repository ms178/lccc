#include <stdio.h>
#include <limits.h>
extern int orig(int,int,long*), transformed(int,int,long*);
int main(void){int v[]={INT_MIN,-2,-1,0,1,2,127,INT_MAX},n=0;for(unsigned i=0;i<8;i++)for(unsigned j=0;j<8;j++){long a=7,b=7;int x=orig(v[i],v[j],&a),y=transformed(v[i],v[j],&b);if(x!=y || a!=b)n++;}printf("%d\n",n);return 0;}
