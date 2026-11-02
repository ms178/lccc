/* Frame-pointer builtins next to a dynamic-TLS access.
 *
 * On RISC-V `__builtin_return_address` lowers to `ld t0, -8(s0)` and
 * `__builtin_frame_address` to `mv t0, s0`. Both read s0, so a function using
 * either must ESTABLISH s0 -- which rules out the minimal RA-only frame that a
 * lone dynamic-TLS resolver would otherwise permit. Selecting that frame on IR
 * alone let the builtin read the CALLER's frame pointer, and the load could
 * fault outright when the caller keeps no frame pointer at all.
 *
 * `probe` below is exactly the shape that used to trigger it: the builtin's
 * result is discarded, so nothing forces a callee-saved home, and the frame
 * stays at the 16-byte minimum. The gate asserts statically that any function
 * reading s0 also sets it up. */
extern __thread unsigned long external_tls;

__attribute__((noinline)) void *probe(void) {
    unsigned long *p = &external_tls;
    __builtin_return_address(0);
    return p;
}

__attribute__((noinline)) void *frame_probe(void) {
    unsigned long *p = &external_tls;
    __builtin_frame_address(0);
    return p;
}
