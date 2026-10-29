/* Pointer stores on both arms, followed by a join reload. Also exercise
   aliasing, intervening writes and volatile operations in the same executable.
   Freestanding so AArch64 and RISC-V can run it without a cross libc. */
struct node {
    struct node *next;
    unsigned key;
};

__attribute__((noinline))
static struct node *choose_and_advance(struct node **slot, struct node **advance,
                                       struct node *a, struct node *b, unsigned take_a) {
    struct node *next;
    if (take_a) {
        *slot = a;
        next = a->next;
    } else {
        *slot = b;
        next = b->next;
    }
    struct node *selected = *slot;
    *advance = next;
    return selected;
}

__attribute__((noinline))
static struct node *clobber_before_join(struct node **slot, struct node **alias,
                                        struct node *a, struct node *b,
                                        struct node *replacement, unsigned take_a) {
    if (take_a)
        *slot = a;
    else
        *slot = b;
    *alias = replacement;
    return *slot;
}

__attribute__((noinline))
static struct node *volatile_join(struct node *volatile *slot, struct node *a,
                                  struct node *b, unsigned take_a) {
    if (take_a)
        *slot = a;
    else
        *slot = b;
    return *slot;
}

int main(void) {
    struct node a = {0, 1}, b = {0, 2}, c = {0, 3}, d = {0, 4};
    a.next = &c;
    b.next = &d;
    c.next = &a;
    d.next = &b;
    for (unsigned n = 0; n < 4096; ++n) {
        unsigned take_a = n & 1u;
        struct node *expected = take_a ? &a : &b;
        struct node *expected_next = take_a ? &c : &d;
        struct node *slot = 0;
        struct node *advance = 0;
        if (choose_and_advance(&slot, &advance, &a, &b, take_a) != expected)
            return 1;
        if (slot != expected || advance != expected_next)
            return 2;
        if (choose_and_advance(&slot, &slot, &a, &b, take_a) != expected)
            return 3;
        if (slot != expected_next)
            return 4;
        if (clobber_before_join(&slot, &slot, &a, &b, &c, take_a) != &c)
            return 5;
        if (clobber_before_join(&slot, &advance, &a, &b, &d, take_a) != expected)
            return 6;
        if (advance != &d)
            return 7;
        struct node *volatile visible = 0;
        if (volatile_join(&visible, &a, &b, take_a) != expected || visible != expected)
            return 8;
    }
    return 0;
}
