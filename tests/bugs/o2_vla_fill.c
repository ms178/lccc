#include <stdio.h>
#include <stdint.h>
__attribute__((noinline)) void barrier(void){}
unsigned long long g;
int main(void){
  unsigned n = 4;
  unsigned long long a[n];
  unsigned int b[n];
  for (unsigned i = 0; i < n; i++) { a[i] = 5ull*0x9e3779b97f4a7c15ULL + 7ULL + i; b[i] = 6u*2654435761u + i; }
  barrier();
  for (unsigned i = 0; i < n; i++) g = g*33 + (a[i] + b[i]);
  printf("%llu\n", g);
  return 0;
}
