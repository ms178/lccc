/* C23 constexpr: named constants are usable in constant expressions (6.6p8). */
int printf(const char *, ...);
constexpr int N = 4;
constexpr unsigned char UC = 200;
constexpr double D = 2.5;
constexpr float F = 0.5f;
constexpr long L = N * 3;
constexpr _Bool B = N == 4;
constexpr char S[] = "hi";
struct P { int x, y; };
constexpr struct P PT = { N, 7 };
constexpr int *NP = nullptr;
enum { E = N + 1 };
_Static_assert(N == 4 && L == 12 && B && E == 5 && UC == 200, "named constants");
static int arr[N];                         /* not a VLA */
static double sd = D * 2;                  /* arithmetic constant init */
int main(void) {
  constexpr int M = N << 1;
  int k = 8; int r = 0;
  switch (k) { case M: r = 1; break; case N: r = 2; break; }
  _Static_assert(M == 8, "block-scope constexpr");
  _Static_assert(sizeof arr == 4 * sizeof(int), "array, not VLA");
  printf("%d %d %d %g %g %ld %d %s %d %d %d %g %d\n", N, UC, r, D, (double)F, L, B, S, PT.x, PT.y,
         _Generic(&N, const int *: 1, default: 0), sd, NP == 0);
  return 0;
}
