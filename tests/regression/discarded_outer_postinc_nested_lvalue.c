/* gcc.c-torture/execute/20060929-1.c reduced.
 * A discarded outer post-increment must not make nested post-increments in the
 * lvalue address expression return their new value: (*p++)++ increments *old_p.
 */
extern void abort(void);
void bump(int **p, int *q) {
    **p = *q++;
    (*p++)++;
}
int main(void) {
    int x = 42, y = 0;
    int *p = &x;
    bump(&p, &y);
    if (p - 1 != &x || y != 0 || x != 0) abort();
    return 0;
}
