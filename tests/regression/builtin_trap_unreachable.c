/* trap/unreachable must not depend on libc abort(); compile+run smoke. */
int main(void) {
    if (0) { __builtin_trap(); }
    if (0) { __builtin_unreachable(); }
    return 0;
}
