/* Regression: C23 [[fallthrough]] statement attribute must parse (glibc
 * cpu-features.c uses it). Expected output "111 102 -1". */
#include <stdio.h>
int f(int x) {
  switch (x) {
  case 1: x += 10; [[fallthrough]];
  case 2: x += 100; break;
  default: x = -1;
  }
  return x;
}
int main(void) { printf("%d %d %d\n", f(1), f(2), f(3)); return 0; }
