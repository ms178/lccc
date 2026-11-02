extern __thread unsigned long external_tls;
__thread unsigned long local_tls;

__attribute__((noinline)) unsigned long *external_address(void) {
    return &external_tls;
}

__attribute__((noinline)) unsigned long *local_address(void) {
    return &local_tls;
}

__attribute__((noinline)) unsigned long external_load(void) {
    return external_tls;
}

__attribute__((noinline)) void external_store(unsigned long value) {
    external_tls = value;
}
