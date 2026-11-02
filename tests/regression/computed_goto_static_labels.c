/* Computed goto through static local initializers (cluster B fix): the
 * label names baked into static data must survive the global block
 * renumbering (gcc.c-torture 20071220-1 shape). */

extern void abort(void);
extern void exit(int);

int run(int i) {
    static const void *table[] = { &&one, &&two, &&three };
    goto *table[i];
one:
    return 1;
two:
    return 2;
three:
    return 3;
}

int main(void) {
    if (run(0) != 1) return 1;
    if (run(1) != 2) return 2;
    if (run(2) != 3) return 3;
    /* A stale (pre-renumber) label would alias targets: verify distinct. */
    if (run(0) + run(1) + run(2) != 6) return 4;
    return 0;
}
