/* globaldse_alias_kept.c — alias escape must keep stores (F1).

   `static int out[4]; extern int pub_out[4] __attribute__((alias("out")));`
   Other TUs can read `out` through `pub_out`. The Phase 11a DSE must NOT
   delete stores to `out` when an alias exists. If it did, reading through
   the alias would see zero instead of the written values.

   This is the regression for PR #617 F1.
*/
#include <stdio.h>

static int out[4] = {0};
extern int pub_out[4] __attribute__((alias("out")));

__attribute__((noinline)) void write_out(void) {
    out[0] = 11;
    out[1] = 22;
    out[2] = 33;
    out[3] = 44;
}

int main(void) {
    write_out();
    /* Read through the alias — must see the stores */
    if (pub_out[0] != 11 || pub_out[1] != 22 || pub_out[2] != 33 || pub_out[3] != 44) {
        printf("FAIL alias_kept got %d %d %d %d want 11 22 33 44\n",
               pub_out[0], pub_out[1], pub_out[2], pub_out[3]);
        return 1;
    }
    printf("PASS globaldse_alias_kept %d %d %d %d\n",
           pub_out[0], pub_out[1], pub_out[2], pub_out[3]);
    return 0;
}
