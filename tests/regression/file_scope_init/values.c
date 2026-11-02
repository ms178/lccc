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


/* Element and member reads of a const object: the fold must produce the bytes
 * the object actually holds. */
static const int cat[4] = {1, 2, 3, 4};
int ce_a = cat[2];
int ce_b = cat[1] * cat[3];
static const struct co { int x; int y; } co_s = {7, 9};
int ce_c = co_s.y;
static const struct cn { int m[3]; } cn_t = {{4, 5, 6}};
int ce_d = cn_t.m[2];
static const struct bf { int a : 4; unsigned b : 5; } bf_v = {-3, 17};
int ce_e = bf_v.a * 100 + (int)bf_v.b;
static const int cmx[2][2] = {{1, 2}, {3, 4}};
int ce_f = cmx[1][1];
static const double cde[2] = {1.5, 2.5};
double ce_g = cde[1];
static const char cce[3] = "ab";
int ce_h = cce[1];

/* A folded call, and a constant condition's selected operand. */
static const double sq = __builtin_sqrt(4.0);
int fc_a = (int)sq;
int fc_b = 0.5 ? 3 : (int)(long)&g;
int fc_c = 0.0 ? (int)(long)&g : 5;

int main(void) {
    printf("%d %d %d %d %d %d %d %d %d %d %d %d %d %d %lu %d\n", c5, c6, eq, ne, eqh,
           land, lnot, lor, (int)lv, ord, ordle, nul, nul2, diff, ulv, sel == &g);
    printf("%d %d %d %d %d %d %d %d %ld %g %d %.9g\n", cv_a, cv_c, cv_d, cv_e, cv_uc,
           cv_sc, cv_bb, cv_cb, cv_lg, cv_dd, cv_dd2, cv_ff);
    printf("%d %d %d %d %d %d %g %d %d %d %d\n", ce_a, ce_b, ce_c, ce_d, ce_e, ce_f,
           ce_g, ce_h, fc_a, fc_b, fc_c);
    return 0;
}
