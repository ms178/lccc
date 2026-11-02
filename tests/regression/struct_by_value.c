/* Struct by-value ABI: return-in-memory + pass-by-value. */
#include <stdio.h>
struct Big { int a[8]; long b; double d; };
struct Big mk(void){ struct Big x; for(int i=0;i<8;i++)x.a[i]=i*i; x.b=77; x.d=2.5; return x; }
struct Big id(struct Big x){ x.a[0]=1234; return x; }
int main(void){ struct Big b=mk(); struct Big c=id(b);
  printf("%d %ld %.1f %d\n", b.a[0], b.b, b.d, c.a[0]);
  return (b.b==77 && c.a[0]==1234)?0:1; }
