/* gcc.c-torture/execute/20020720-1.c
 *
 * fabs(x) < 0.0 is always false (NaN-safe: unordered compares are false).
 * The dead call to abort must be eliminated; we keep abort defined so a
 * missed fold is an execute failure rather than a link failure. */
void abort(void);
double fabs(double);

void foo(double x) {
    double p, q;
    p = fabs(x);
    q = 0.0;
    if (p < q)
        abort();
}

int main(void) {
    foo(1.0);
    foo(-1.0);
    foo(0.0);
    return 0;
}
