void user_simd(void) {
    __asm__ volatile("movi v0.16b, #0" ::: "v0");
}
