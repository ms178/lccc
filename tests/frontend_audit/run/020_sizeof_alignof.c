#include <stdio.h>
struct S1 { char a; int b; char c; };
struct S2 { char a; long long b; short c; };
struct S3 { char a; char b; short c; int d; };
struct S4 { double d; char c; };
struct S5 { char c; double d; };
union U1 { char a; int b; double c; };
struct S6 { struct S1 s; char c; };
struct __attribute__((packed)) P1 { char a; int b; };
struct B1 { unsigned a : 3; unsigned b : 5; unsigned c : 24; };
struct B2 { char a; unsigned b : 7; };
struct B3 { unsigned a : 1; unsigned long long b : 63; };
struct F1 { int n; char flex[]; };
#define P(e) printf("%-38s %lld\n", #e, (long long)(e))
int main(void) {
    P(sizeof(struct S1)); P(sizeof(struct S2)); P(sizeof(struct S3));
    P(sizeof(struct S4)); P(sizeof(struct S5)); P(sizeof(union U1));
    P(sizeof(struct S6)); P(sizeof(struct P1)); P(sizeof(struct B1));
    P(sizeof(struct B2)); P(sizeof(struct B3)); P(sizeof(struct F1));
    P(_Alignof(struct S2)); P(_Alignof(double)); P(_Alignof(struct P1));
    P(sizeof(char[17])); P(sizeof(int[3][4]));
    P(sizeof(long double)); P(_Alignof(long double));
    P(sizeof(1 ? (char)1 : (short)2));
    P(sizeof(void *)); P(_Alignof(void (*)(void)));
    return 0;
}
