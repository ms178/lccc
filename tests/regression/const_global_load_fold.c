/* gcc.c-torture/execute/20030216-1.c
 *
 * Loads of const scalar globals must fold so `(int)one == 1` is a
 * compile-time true and the dead abort is DCE'd. */
void abort(void);
const double one = 1.0;

int main(void) {
    if ((int)one != 1)
        abort();
    return 0;
}
