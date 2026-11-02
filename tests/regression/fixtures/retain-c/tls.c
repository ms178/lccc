/* Compile-only on cross targets; native execution checks the actual TLS image. */
__thread int retained_init __attribute__((retain)) = 17;
__thread int retained_zero __attribute__((retain));
__thread const int retained_const __attribute__((retain)) = 23;
__thread const int retained_const_zero __attribute__((retain));
__thread int custom_init __attribute__((section(".custom_tls_init"), retain)) = 29;
__thread int custom_zero __attribute__((section(".custom_tls_zero"), retain));
__thread int plain_init __attribute__((section(".plain_tls_init"))) = 31;
__thread int plain_zero __attribute__((section(".plain_tls_zero")));
int relocation_target = 59;
int *const retained_ptr __attribute__((retain)) = &relocation_target;

__attribute__((noinline)) int touch_locals(void) {
    static __thread int local_thread __attribute__((retain, used)) = 37;
    static __thread int local_zero __attribute__((retain, used));
    static int local_object __attribute__((retain, used)) = 41,
               local_sibling __attribute__((used)) = 43;
    static __attribute__((retain, used)) int local_spec_a = 47, local_spec_b = 53;
    static int local_after __attribute__((used)) = 61;
    return local_thread + local_object;
}

int main(void) {
    if (retained_init != 17 || retained_zero != 0 || retained_const != 23 ||
        retained_const_zero != 0 || custom_init != 29 || custom_zero != 0 ||
        plain_init != 31 || plain_zero != 0 || *retained_ptr != 59 ||
        touch_locals() != 78)
        return 1;
    retained_zero = 43;
    custom_zero = 47;
    plain_zero = 53;
    return retained_zero != 43 || custom_zero != 47 || plain_zero != 53;
}
