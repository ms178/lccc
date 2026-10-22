/* Distinct GlobalAddr names are not an alias proof: a GNU alias can redirect
 * both names to the same array. Destination is one element ahead, so scalar
 * execution is a recurrence. `(&shifted[1])[i]` preserves the analyzer's
 * canonical zero-based IV while producing a GlobalAddr-rooted destination;
 * this exercises module-level symbol facts and the runtime dependence guard.
 * On this call the pointers overlap and the guard must select the scalar path.
 */
#define N 64

static int backing[N + 1];
extern int shifted[N + 1] __attribute__((alias("backing")));

__attribute__((noinline)) static void shift_add(int n)
{
    for (int i = 0; i < n; ++i)
        (&shifted[1])[i] = backing[i] + 2;
}

int main(void)
{
    for (int i = 0; i <= N; ++i)
        backing[i] = i;

    shift_add(N);

    if (backing[0] != 0)
        return 1;
    for (int i = 1; i <= N; ++i) {
        if (backing[i] != 2 * i)
            return 2 + i;
    }
    return 0;
}
