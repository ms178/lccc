/* Runtime oracle for class-aware GlobalAddr CSE.
 *
 * Mixes RIP-foldable loads of a file-scope table with a must-materialize
 * use (the address is passed to a noinline helper). Both classes must
 * observe the same bytes; mixing them in RA is forbidden, but the values
 * are still the same linker constant. */
static int table[8] = {1, 2, 3, 4, 5, 6, 7, 8};

__attribute__((noinline))
static int consume_ptr(int *p, int i) {
    return p[i];
}

__attribute__((noinline))
int foldable_sum(void) {
    int s = 0;
    for (int i = 0; i < 8; i++)
        s += table[i];
    return s;
}

__attribute__((noinline))
int materialized_sum(void) {
    int s = 0;
    for (int i = 0; i < 8; i++)
        s += consume_ptr(table, i);
    return s;
}

int main(void) {
    return (foldable_sum() + materialized_sum()) != 72;
}
