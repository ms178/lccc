/* A register copy at the end of one arm of a diamond is not a fact after the
 * join.
 *
 * AArch64 -O2/-Os: the address-alias pass renamed `[x8]` onto `[x13]` after
 * `mov x8, x13` by scanning the text forward from the copy -- past the
 * branch back to the loop head and into the join block, which the other arm
 * and the loop exit also reach with x8 holding something else.  Splitting a
 * list into two tail-pointer lists then terminated the wrong list (`*odd_tail
 * = 0` stored through the even list's pointer).  It surfaced when leaf loop
 * state moved into the volatile registers (gcc.c-torture pr39100), but the
 * pass was unsound for any register assignment of this shape.  The cross
 * harness runs this on AArch64 and RV64; the native corpus on x86-64.
 */

struct node {
    int key;
    struct node *next;
};

struct split {
    struct node *even;
    short n_even, n_odd;
};

__attribute__((noinline)) struct split *split_by_parity(struct split *s, struct node *x)
{
    struct node *odd = 0, **odd_tail = &odd;
    struct node *even = 0, **even_tail = &even;
    while (x) {
        struct node *next = x->next;
        if ((x->key & 1) == 1) {
            s->n_odd++;
            *odd_tail = x;
            odd_tail = &(*odd_tail)->next;
        } else {
            s->n_even++;
            *even_tail = x;
            even_tail = &(*even_tail)->next;
        }
        x = next;
    }
    *even_tail = odd;
    *odd_tail = 0;
    s->even = even;
    return s;
}

int main(void)
{
    struct split s = {0, 0, 0};
    struct node n[5] = {{0, &n[1]}, {1, &n[2]}, {2, &n[3]}, {3, &n[4]}, {5, 0}};
    split_by_parity(&s, &n[0]);
    if (s.n_even != 2 || s.n_odd != 3)
        return 1;
    /* even list 0 -> 2, then the odd list 1 -> 3 -> 5, terminated. */
    if (s.even != &n[0] || n[0].next != &n[2] || n[2].next != &n[1])
        return 2;
    if (n[1].next != &n[3] || n[3].next != &n[4] || n[4].next != 0)
        return 3;

    struct split t = {0, 0, 0};
    struct node m[2] = {{0, &m[1]}, {1, 0}};
    split_by_parity(&t, &m[0]);
    if (t.even != &m[0] || m[0].next != &m[1] || m[1].next != 0)
        return 4;
    return 0;
}
