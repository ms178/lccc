/* A pragma where a statement is expected.
 *
 * GCC takes the pragma and then parses the statement that follows it as the
 * statement that was expected: in
 *
 *     if (x)
 *     #pragma pack(push, 1)
 *         hits += 1;
 *
 * the addition is the body of the `if`, and the pragma has taken effect.
 * lccc handled its pragma markers between the items of a block only, so
 * the same lines were "expected expression before '#pragma pack'" -- for
 * `#pragma pack` and `#pragma GCC visibility` always, and for
 * `#pragma GCC push_options` / `pop_options` / `reset_options` from the
 * moment they stopped being dropped.
 *
 * Every guarded statement below adds a distinct power of ten, so the
 * printed sums say which statements ran under which condition; the
 * structure sizes say whether each `pack` pragma took effect. GCC 16.2 and
 * Clang 23 print the same three numbers. */
#include <stdio.h>

static int walk(int x) {
    int hits = 0;
    if (x)
#pragma GCC push_options
        hits += 1;
    else
#pragma GCC pop_options
        hits += 10;
    for (int i = 0; i < 3; i++)
#pragma pack(push, 1)
        hits += 100;
    struct packed {
        char c;
        int i;
    };
    while (x-- > 0)
#pragma pack(pop)
        hits += 1000;
    struct natural {
        char c;
        int i;
    };
    do
#pragma GCC visibility push(hidden)
        hits += 10000;
    while (0);
#pragma GCC visibility pop
    switch (x) {
#pragma GCC reset_options
    default:
#pragma GCC push_options
        hits += 100000;
#pragma GCC pop_options
    }
    goto done;
done:
#pragma pack()
    hits += (int)(sizeof(struct packed) * 1000000 + sizeof(struct natural) * 10000000);
    return hits;
}

int main(void) {
    printf("%d %d %d\n", walk(0), walk(1), walk(4));
    return 0;
}
