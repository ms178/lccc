/* Cross-compiler atomic interaction on shared aligned objects. This is a
 * sequential ABI/value test, not a contention or memory-order litmus test. */
#include "atomic_interop.h"

#define CHECK(id, condition) do { if (!(condition)) return (id); } while (0)

int main(void)
{
    struct { u32 before, value, after; } word = { 0x12345678u, 0xfffffffeu, 0x87654321u };
    u32 expected;
    CHECK(1, interop_add32(&word.value, 3) == 0xfffffffeu);
    CHECK(2, __atomic_load_n(&word.value, __ATOMIC_ACQUIRE) == 1);
    expected = 2;
    CHECK(3, !interop_cas32(&word.value, &expected, 7));
    CHECK(4, expected == 1 && word.value == 1);
    expected = 1;
    CHECK(5, interop_cas32(&word.value, &expected, 0x80000001u));
    CHECK(6, expected == 1 && word.value == 0x80000001u);
    CHECK(7, word.before == 0x12345678u && word.after == 0x87654321u);

    u64 value64 = 0x123456789abcdef0ull;
    CHECK(8, interop_xchg64(&value64, 0xfedcba9876543210ull) == 0x123456789abcdef0ull);
    CHECK(9, __atomic_load_n(&value64, __ATOMIC_SEQ_CST) == 0xfedcba9876543210ull);

#if defined(__aarch64__)
    /* Carry out of bit 63, and CAS whose low half matches but high does not. */
    u128 value128 = ((u128)0x1122334455667788ull << 64) | 0xffffffffffffffffull;
    u128 old = value128;
    CHECK(10, interop_add128(&value128, 1) == old);
    u128 next = (u128)0x1122334455667789ull << 64;
    CHECK(11, __atomic_load_n(&value128, __ATOMIC_SEQ_CST) == next);
    u128 expected128 = (u128)0x1122334455667788ull << 64;
    CHECK(12, !interop_cas128(&value128, &expected128, 5));
    CHECK(13, expected128 == next && __atomic_load_n(&value128, __ATOMIC_RELAXED) == next);
    u128 desired = ((u128)0xdeadbeefcafebabeull << 64) | 0x123456789abcdef0ull;
    CHECK(14, interop_cas128(&value128, &expected128, desired));
    CHECK(15, expected128 == next);
    CHECK(16, interop_xchg128(&value128, old) == desired);
    CHECK(17, __atomic_load_n(&value128, __ATOMIC_ACQUIRE) == old);
#endif
    return 0;
}
