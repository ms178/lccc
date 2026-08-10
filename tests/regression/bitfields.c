#include <stdio.h>
struct Bf { unsigned a:3; signed b:5; unsigned long d:40; };
int main(void){ struct Bf f; f.a=5; f.b=-7; f.d=0x1234567890UL;
  printf("%u %d %lu\n", f.a, f.b, (unsigned long)f.d);
  return (f.a==5 && f.b==-7 && (unsigned long)f.d==0x1234567890UL)?0:1; }
