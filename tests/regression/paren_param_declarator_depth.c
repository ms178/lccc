/* A parameter declarator whose parenthesised group nests a group of plain
 * stars, `int (*(*p))[3]`, has every star: p is a pointer to a pointer to
 * an array (`int (**)[3]`). The parameter parser used to re-scan the nested
 * group for its name only, dropping its stars, so p was typed `int (*)[3]`:
 * `sizeof *p` was 12 and `(*p)[1]` read the array instead of a pointer.
 * Locals were unaffected. Checked against GCC 16.2, which agrees on every
 * value below. */
#include <stdio.h>

static int rows[2][3] = { { 1, 2, 3 }, { 4, 5, 6 } };
static int hits;

static void bump(int n) { hits += n; }

static int via_pp(int (*(*pp))[3]) {
    if (sizeof *pp != sizeof(void *) || sizeof **pp != 3 * sizeof(int))
        return -1;
    return (**pp)[2] + (*pp)[1][0];
}

static int via_ppp(int (*(*(*ppp)))[3]) {
    if (sizeof *ppp != sizeof(void *) || sizeof ***ppp != 3 * sizeof(int))
        return -1;
    return (***ppp)[1];
}

static void via_fpp(void (*(*fpp))(int), int n) { (*fpp)(n); }

static int via_plain(int (*(*q)))  { return sizeof *q == sizeof(void *) ? **q : -1; }

int main(void) {
    int (*row)[3] = rows;
    int (**prow)[3] = &row;
    void (*fp)(int) = bump;
    int value = 7, *pv = &value;
    int r1 = via_pp(&row);
    int r2 = via_ppp(&prow);
    via_fpp(&fp, 5);
    int r3 = via_plain(&pv);
    if (r1 != 3 + 4 || r2 != 2 || hits != 5 || r3 != 7) {
        printf("FAIL r1=%d r2=%d hits=%d r3=%d\n", r1, r2, hits, r3);
        return 1;
    }
    printf("PASS paren_param_declarator_depth\n");
    return 0;
}
