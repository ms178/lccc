/* A file-scope pointer whose base specifier is an arithmetic type must store
 * the address, not the address converted to that type.  The static
 * initializer's type decisions used the declaration's *base specifier*
 * (`_Decimal64` in `_Decimal64 *p`) instead of the object's type:
 *   _Decimal64 *pd = (_Decimal64 *)64;   stored BID(64) = 3584865303386914880
 *   long double *pl = (long double *)40; stored x87 bits (m64) / 0 (m32)
 *   _Bool *pb = (_Bool *)48;             stored 1
 *   _Complex double *a[2] = {...};       lowered as complex values (8.0's bits)
 * Also: a cast of a folded address relation, `(T *)(uintptr_t)(&g != 0)`,
 * is a constant GCC accepts (it was "initializer element is not constant").
 * Integer-to-pointer conversions are implementation-defined (GCC: value
 * preserving); no pointer below is dereferenced.  Compared with GCC. */
#include <stdint.h>
#include <stdio.h>

typedef long double LD;
_Decimal64 *pd = (_Decimal64 *)64;
_Decimal32 *pd32[2] = {(_Decimal32 *)16, (_Decimal32 *)32};
_Decimal128 *pd128 = (_Decimal128 *)112;
float *pf = (float *)8;
double *pdd = (double *)24;
long double *pl = (long double *)40;
_Bool *pb = (_Bool *)48;
_Complex double *pc = (_Complex double *)72;
_Complex float *pcf = (_Complex float *)80;
_Complex double *pca[2] = {(_Complex double *)8, (_Complex double *)16};
_Complex float *pcf3[3] = {0, (_Complex float *)24};
_Complex long double *(pcl[1]) = {(_Complex long double *)32};
_Complex double ca[2] = {1.5, 2.5}; /* control: a real complex array */
_Decimal64 **cl = (_Decimal64 *[]){(_Decimal64 *)8, (_Decimal64 *)16};
LD **cl3 = &(LD *){(LD *)40};
struct S { _Decimal64 *p; LD *q; _Bool *b; } s = {(_Decimal64 *)88, (LD *)96, (_Bool *)104};

static int obj, obj2;
_Bool rel = &obj != 0;
double drel = &obj != 0;
_Decimal32 decrel = &obj != 0;
long double *prel = (long double *)(uintptr_t)(&obj != 0);
_Decimal64 *pdrel = (_Decimal64 *)(uintptr_t)!(&obj == 0);
float frel = (float)(&obj == 0);
unsigned char crel = (unsigned char)(_Bool)(&obj != 0);
int land = (&obj != 0) && (&obj2 == 0);
int lor = (&obj == 0) || !(&obj2 != 0);
short lnest = !((&obj != &obj2) && (&obj2 != 0));

#define U(p) (unsigned long)(uintptr_t)(p)
int main(void) {
    union { _Decimal32 d; unsigned u; } dr = {decrel};
    printf("%lu %lu %lu %lu %lu %lu %lu %lu\n", U(pd), U(pd32[0]), U(pd32[1]), U(pd128), U(pf), U(pdd),
           U(pl), U(pb));
    printf("%lu %lu %lu %lu %lu\n", U(pc), U(pcf), U(s.p), U(s.q), U(s.b));
    printf("%lu %lu %lu %lu %lu %lu %g %g %lu %lu %lu\n", U(pca[0]), U(pca[1]), U(pcf3[0]), U(pcf3[1]),
           U(pcf3[2]), U(pcl[0]), __real__ ca[1], __imag__ ca[1], U(cl[0]), U(cl[1]), U(*cl3));
    printf("%d %g %x %lu %lu %g %u %d %d %d\n", rel, drel, dr.u, U(prel), U(pdrel), frel, crel, land, lor,
           lnest);
    return 0;
}
