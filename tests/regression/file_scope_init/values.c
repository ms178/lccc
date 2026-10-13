/* Runtime values of folded file-scope initializers. The gate compiles this
 * with lccc and with GCC 16.2, links both, and compares the printed values. */
#include <stdio.h>

static int g, h;
static int arr[4];

char c5 = "abc"[1];
char c6 = "abc"[3];
int eq = &g == &g;
int ne = &g != &h;
int eqh = &g == &h;
int land = &g && 1;
int lnot = !&g;
int lor = 0 || &arr[2];
long lv = (long)&g != 0;
int ord = &arr[1] > &arr[0];
int ordle = &arr[3] <= &arr[3];
int nul = &g == 0;
int nul2 = 0 < &g;
int diff = &arr[3] - &arr[0] == 3;
unsigned long ulv = &g == &g;
int *sel = _Generic(0, int: &g, default: 0);

/* Folded const objects: a value is converted to the object's type. */
const int cc = 5;
int cv_a = cc;
int cv_c = cc == 5;
int cv_d = 5 == cc;
int cv_e = cc * 2;
const unsigned char uc = 300;
int cv_uc = uc;
const signed char sc = 200;
int cv_sc = sc;
const _Bool bb = 5;
int cv_bb = bb;
const int cb = {7};
int cv_cb = cb;
const long lg = -3;
long cv_lg = lg + 1;
const double dd = 1.5;
double cv_dd = dd;
int cv_dd2 = (int)(dd * 2);
const float ff = 0.1f;
float cv_ff = ff * 3;

int main(void) {
    printf("%d %d %d %d %d %d %d %d %d %d %d %d %d %d %lu %d\n", c5, c6, eq, ne, eqh,
           land, lnot, lor, (int)lv, ord, ordle, nul, nul2, diff, ulv, sel == &g);
    printf("%d %d %d %d %d %d %d %d %ld %g %d %.9g\n", cv_a, cv_c, cv_d, cv_e, cv_uc,
           cv_sc, cv_bb, cv_cb, cv_lg, cv_dd, cv_dd2, cv_ff);
    return 0;
}
