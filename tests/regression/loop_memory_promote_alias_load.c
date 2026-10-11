/* Salvaged from levkropp/lccc a9987d0; strengthened with executable checks. */
extern int printf(const char *, ...);
extern void *memset(void *, int, unsigned long);
extern char *strcpy(char *, const char *);

/* Regression: loop_memory_promote must not promote a loop-carried store when
 * another loop load reaches the same address through a different pointer value
 * (e.g. duplicate GEPs hoisted by LICM). The unpromoted loads kept reading
 * stale memory, so the decrement never became visible and the loop spun
 * forever (sqlite3FpDecode trailing-zero trim, sqlite workload hang).
 * Correct output: n=0 (three trailing zeros trimmed; n==0 stops it). */
typedef struct {
    int flags;
    int n;
    char *z;
    char zBuf[24];
} FpD;

__attribute__((noinline)) void trim_zeros(FpD *p) {
    p->z = &p->zBuf[1];
    while (p->n > 0 && p->z[p->n - 1] == '0') {
        p->n--;
    }
}

int main(void) {
    FpD p;
    memset(&p, 0, sizeof(p));
    p.n = 3;
    strcpy(p.zBuf, "x000");
    trim_zeros(&p);
    printf("n=%d\n", p.n);
    if (p.n != 0) return 1;
    /* Include zero-trip, all-zero, no-zero and partially trimmed strings. */
    for (int n = 0; n < 20; n++) {
        for (int keep = 0; keep <= n; keep++) {
            p.n = n;
            for (int i = 0; i < n; i++) p.zBuf[i+1] = i < keep ? 'x' : '0';
            trim_zeros(&p);
            if (p.n != keep || p.z != p.zBuf + 1) return 2;
        }
    }
    return 0;
}
