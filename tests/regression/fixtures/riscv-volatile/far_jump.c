/* The inline-assembly directive makes the forward jump exceed jal's range. */
__attribute__((noinline)) int farjump(int n) {
    if (n) goto target;
    __asm__ volatile (".space 1048576");
target:
    return 42;
}
int main(void) { return farjump(1) != 42; }
