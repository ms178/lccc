__attribute__((naked)) void nk(void) {
    __asm__ volatile("movi v0.16b, #0\n\tret");
}
