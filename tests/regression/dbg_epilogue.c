/* v5 regression: -g (debug info) must not lose function epilogues.
 * gzip -v crashed at -O2 -g because the .loc directive between a tail varargs
 * call and a conditionally-labeled epilogue fragmented basic blocks, and the
 * identical-block merge then mis-fired and deleted a function's epilogue+ret
 * (the function fell through into the next one -> SIGSEGV, seen in
 * `gzip -v`'s treat_file). Guard the class: several functions with identical
 * frames, conditional tails, and varargs calls; their return values must be
 * exact and control flow must not fall through. Compile with -g (see
 * dbg_epilogue.flags). */
#include <stdio.h>

__attribute__((noinline)) static int tf1(FILE *e, int v, int t) {
  int r = 0;
  for (int i = 0; i < v; i++) r += i;
  if (t > 0) {
    fprintf(e, "sum1:%d\n", r);
  }
  fprintf(e, "done1:%5.1f%%\n", 1.0);
  return r;
}
__attribute__((noinline)) static int tf2(FILE *e, int v, int t) {
  int r = 0;
  for (int i = 0; i < v; i++) r += i;
  if (t > 0) {
    fprintf(e, "sum2:%d\n", r);
  }
  fprintf(e, "done2:%5.1f%%\n", 2.0);
  return r;
}
__attribute__((noinline)) static int tf3(FILE *e, int v, int t) {
  int r = 0;
  for (int i = 0; i < v; i++) r += i;
  if (t > 0) {
    fprintf(e, "sum3:%d\n", r);
  }
  fprintf(e, "done3:%5.1f%%\n", 3.0);
  return r;
}
__attribute__((noinline)) static int tf4(FILE *e, int v, int t) {
  int r = 0;
  for (int i = 0; i < v; i++) r += i;
  if (t > 0) {
    fprintf(e, "sum4:%d\n", r);
  }
  fprintf(e, "done4:%5.1f%%\n", 4.0);
  return r;
}

int main(void) {
  int a = tf1(stderr, 4, 1), b = tf2(stderr, 5, 0);
  int c = tf3(stderr, 6, 1), d = tf4(stderr, 7, 0);
  printf("res %d %d %d %d\n", a, b, c, d);
  return (a == 6 && b == 10 && c == 15 && d == 21) ? 0 : 1;
}
