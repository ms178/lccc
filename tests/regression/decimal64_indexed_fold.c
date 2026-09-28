#include <stdio.h>

#define N 64
static _Decimal64 a[N], b[N];

int main(void) {
    /* Recurrence fill: pure _Decimal64 arithmetic, so the loop body is a
       D64 load plus a D64 store through differently-scaled GEPs. */
    a[0] = 1.5DD;
    for (int i = 0; i < N - 1; ++i)
        a[i + 1] = a[i] + 0.25DD;
    for (int i = 0; i < N; ++i)
        b[i] = a[i] * 2.0DD;
    /* Integer-domain checksum: each element is computed by single-step
       _Decimal64 arithmetic (value-exact), truncated once to long long or
       int (quantum-immune: equal values truncate equally whatever the
       quantum), and accumulated exactly. Comparing _Decimal64 BITS here
       would be a fake contract — quantum drift across 64 accumulations
       is legal decimal behavior (lccc pads trailing zeros; gcc does
       not), so bit-exactness would fail a correct compiler. */
    long long chk = 0;
    for (int i = 0; i < N; ++i)
        chk += (long long)a[i] + (long long)b[i];
    printf(
        "chk=%lld a0=%d b0=%d alast=%d blast=%d\n",
        chk,
        (int)a[0],
        (int)b[0],
        (int)a[N - 1],
        (int)b[N - 1]
    );
    return 0;
}
