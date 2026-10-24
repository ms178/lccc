float g(float a, float b);
void mixed(float a, float b) {
    __asm__ volatile("movi v0.16b, #0" ::: "v0");
    g(a, b);
}
