/* No-remainder fast path (vectorize): a constant-trip reduction whose trip
   count divides evenly by the vector width omits the scalar remainder loop
   and routes outside uses (accumulators, escaping loop counter) to the
   horizontal-reduce results. Exercises the skip (divisible trips) and the
   retained remainder (non-divisible trip); every case has a closed-form
   answer so a wrong skip/rewire aborts. Arrays hold a[i] = i + 1,
   b[i] = 3 (see init). */
extern void abort(void);

#define N_DIV 64
#define N_REM 67
#define N_ARR 128

static int a[N_ARR], b[N_ARR];

static void init(void) {
    for (int i = 0; i < N_ARR; i++) {
        a[i] = i + 1;
        b[i] = 3;
    }
}

/* Divisible dot product (64 % 8 == 0: skip fires) with escaping IV use. */
static int dot_div(int *iv_out) {
    int s = 0, i;
    for (i = 0; i < N_DIV; i++)
        s += a[i] * b[i];
    *iv_out = i;
    return s;
}

/* Non-divisible dot product (67 % 8 == 3: remainder retained). */
static int dot_rem(int *iv_out) {
    int s = 0, i;
    for (i = 0; i < N_REM; i++)
        s += a[i] * b[i];
    *iv_out = i;
    return s;
}

/* Divisible multi-reduction: two accumulators share one vector loop. */
static int multi_red(int *second_out) {
    int s1 = 0, s2 = 0;
    for (int i = 0; i < N_DIV; i++) {
        s1 += a[i];
        s2 += a[i] * b[i];
    }
    *second_out = s2;
    return s1;
}

/* Divisible guarded sum: only elements above 32 accumulate. */
static int guarded_sum(void) {
    int s = 0;
    for (int i = 0; i < N_DIV; i++)
        if (a[i] > 32)
            s += a[i];
    return s;
}

/* Divisible max reduction. */
static int max_red(void) {
    int mx = 0;
    for (int i = 0; i < N_DIV; i++)
        mx = a[i] > mx ? a[i] : mx;
    return mx;
}

/* Zero-trip reduction: the h-reduce identity must reach outside uses. */
static int zero_trip(int *iv_out) {
    int s = 0, i;
    for (i = 0; i < 0; i++)
        s += a[i] * b[i];
    *iv_out = i;
    return s;
}

int main(void) {
    int iv, second;
    init();

    /* 3 * sum(1..64) = 3 * 2080 = 6240; final counter 64. */
    if (dot_div(&iv) != 6240 || iv != 64)
        abort();
    /* 3 * sum(1..67) = 3 * 2278 = 6834; final counter 67. */
    if (dot_rem(&iv) != 6834 || iv != 67)
        abort();
    /* sum(1..64) = 2080; 3 * that = 6240. */
    if (multi_red(&second) != 2080 || second != 6240)
        abort();
    /* sum(33..64) = (33 + 64) * 32 / 2 = 1552. */
    if (guarded_sum() != 1552)
        abort();
    /* max(1..64) = 64. */
    if (max_red() != 64)
        abort();
    if (zero_trip(&iv) != 0 || iv != 0)
        abort();
    return 0;
}
