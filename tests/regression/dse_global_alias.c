/* Different symbol names do not prove different storage. Keep the first
 * store even when GVN/global-address CSE/forwarding are disabled: DSE's own
 * correctness cannot depend on another pass canonicalizing the aliases. */
#include <stdio.h>
int dse_object;
extern int dse_alias_object __attribute__((alias("dse_object")));
__attribute__((noinline)) int dse_read_alias(void) {
    dse_object = 17;
    int observed = dse_alias_object;
    dse_object = 29;
    return observed;
}
int main(void) {
    int observed = dse_read_alias();
    printf("dse-alias %d %d\n", observed, dse_object);
    return observed != 17 || dse_object != 29;
}
