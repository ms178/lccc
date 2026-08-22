/* gcc.c-torture/execute/20021127-1.c
 *
 * With -fbuiltin (LCCC default), calls to llabs use the builtin even when
 * the user later defines llabs. The user body aborts; the call must not
 * reach it. */
long long a = -1;
long long llabs(long long);
void abort(void);

int main(void) {
    if (llabs(a) != 1)
        abort();
    return 0;
}

long long llabs(long long b) {
    abort();
    return b;
}
