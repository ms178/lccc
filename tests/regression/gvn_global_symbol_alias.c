#include <stdio.h>
int g=1;
extern int ga __attribute__((alias("g")));
__attribute__((noinline)) int f(void){int x=g;ga=7;return g-x;}
int main(void){int r=f();printf("%d %d\n",g,r);return !(g==7&&r==6);}
