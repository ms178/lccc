/* Tentative definitions of incomplete arrays (C11 6.9.2p2, 6.2.7p3).
 *
 *   int x[];  ...  int x[300];
 *
 * The two tentative definitions have the composite type int[300]. lccc
 * kept the FIRST one, allocated with a fixed 256-element placeholder, and
 * skipped the completing declaration: x got 1024 bytes of storage, so
 * x[256..299] overwrote whatever the linker placed next (likewise when an
 * `extern int y[280];` completes the type).  An array that
 * stays incomplete to the end of the unit has ONE element (GCC: "array
 * 'one' assumed to have one element"); lccc gave it 256 (sqlite's
 * per-file build emitted a 2048-byte common sqlite3StdType vs GCC's 48).
 */
#include <stdio.h>

int before[64];
int x[];
int between[64];
int x[300];
int after[64];
int y[];
int between2[64];
extern int y[280];   /* an extern declaration completes it too */
int after2[64];
long w[][3];
long w[4][3];
int one[];

static void fill(int *p, int n, int v) {
  for (int i = 0; i < n; i++) p[i] = v + i;
}

static int check(const int *p, int n, int v) {
  for (int i = 0; i < n; i++)
    if (p[i] != v + i) return 0;
  return 1;
}

int main(void) {
  fill(before, 64, 1000);
  fill(between, 64, 2000);
  fill(after, 64, 3000);
  fill(between2, 64, 4000);
  fill(after2, 64, 5000);
  fill(x, 300, 0);
  fill(y, 280, 7);
  for (int i = 0; i < 4; i++)
    for (int j = 0; j < 3; j++) w[i][j] = 10 * i + j;
  one[0] = 42;
  int ok = check(before, 64, 1000) && check(between, 64, 2000) &&
           check(after, 64, 3000) && check(between2, 64, 4000) &&
           check(after2, 64, 5000) && check(x, 300, 0) && check(y, 280, 7);
  long ws = 0;
  for (int i = 0; i < 4; i++)
    for (int j = 0; j < 3; j++) ws += w[i][j];
  printf("neighbours intact: %d\n", ok);
  printf("x[299]=%d y[279]=%d w-sum=%ld one=%d\n", x[299], y[279], ws, one[0]);
  return ok ? 0 : 1;
}
