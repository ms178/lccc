/* Standalone, non-kernel ABI fixture: transport an ordinary offset pointer and
 * integer value through separate argument registers into a volatile GS store.
 * No Linux kernel source or headers are compiled by the GCC fixture oracle.
 * The cast through uintptr_t exercises the generic segment-store emitter, not
 * just its direct-GEP fast path. Compare every byte, including untouched bytes.
 */
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <sys/syscall.h>
#include <unistd.h>

#define ARCH_SET_GS 0x1001
#define ARCH_GET_GS 0x1004
static unsigned char area[1024] __attribute__((aligned(64)));
static unsigned char expected[1024];

#define DEFINE_STORE(type, name) \
    __attribute__((noinline)) void name(void *offset, type value) { \
        *(volatile type __seg_gs *)(uintptr_t)offset = value; \
    }
DEFINE_STORE(unsigned char, store8)
DEFINE_STORE(unsigned short, store16)
DEFINE_STORE(unsigned int, store32)
DEFINE_STORE(unsigned long long, store64)
DEFINE_STORE(signed char, stores8)
DEFINE_STORE(short, stores16)
DEFINE_STORE(int, stores32)
DEFINE_STORE(long long, stores64)

static int failures;
#define CHECK_STORE(type, name, input) do { \
    void (*volatile call)(void *, type) = name; \
    for (unsigned long offset = 64; offset <= 256; offset += 64) { \
        type value = (type)(input); \
        memset(area, 0xa5, sizeof(area)); \
        memset(expected, 0xa5, sizeof(expected)); \
        memcpy(expected + offset, &value, sizeof(value)); \
        call((void *)offset, value); \
        if (memcmp(area, expected, sizeof(area))) { \
            printf("FAIL %s offset=%lu\n", #name, offset); \
            ++failures; \
        } \
    } \
} while (0)

int main(void) {
    unsigned long original;
    if (syscall(SYS_arch_prctl, ARCH_GET_GS, &original) ||
        syscall(SYS_arch_prctl, ARCH_SET_GS, area)) {
        puts("FAIL cannot install test GS base");
        return 1;
    }
    /* Zero and small values turn the historical wrong-address write into a
     * deterministic byte mismatch rather than relying solely on a fault. */
    CHECK_STORE(unsigned char, store8, 0);
    CHECK_STORE(unsigned char, store8, 4);
    CHECK_STORE(unsigned short, store16, 8);
    CHECK_STORE(unsigned int, store32, 0);
    CHECK_STORE(unsigned int, store32, 4);
    CHECK_STORE(unsigned long long, store64, 16);
    CHECK_STORE(signed char, stores8, -1);
    CHECK_STORE(short, stores16, -123);
    CHECK_STORE(int, stores32, -123456);
    CHECK_STORE(long long, stores64, -1234567890123LL);
    if (syscall(SYS_arch_prctl, ARCH_SET_GS, original)) {
        puts("FAIL cannot restore GS base");
        return 1;
    }
    printf("%s (%d failures)\n", failures ? "FAILED" : "PASSED", failures);
    return failures != 0;
}
