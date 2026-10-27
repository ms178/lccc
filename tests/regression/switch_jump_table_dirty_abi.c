#include <stdint.h>
#include <stdio.h>

/* A direct SIB jump-table index must use the defined low 32 bits, never
 * arbitrary upper argument bits. Large sparse return values keep this switch
 * out of switch_lut so the backend jump-table path is exercised directly. */
__attribute__((noinline)) int jt32(uint32_t x) {
    switch (x) {
    case 0: return 1001;
    case 1: return 2003;
    case 2: return 3007;
    case 3: return 4013;
    case 4: return 5017;
    case 5: return 6019;
    case 6: return 7027;
    case 7: return 8033;
    default: return 9901;
    }
}

#if defined(__x86_64__)
typedef int (*jt32_fn)(uint32_t);
__asm__(
    ".text\n"
    ".globl call_dirty32\n"
    ".type call_dirty32,@function\n"
    "call_dirty32:\n"
    "    subq $8, %rsp\n"
    "    movq %rdi, %rax\n"
    "    movq %rsi, %rdi\n"
    "    movl %edx, %edx\n"
    "    orq %rdx, %rdi\n"
    "    call *%rax\n"
    "    addq $8, %rsp\n"
    "    ret\n"
    ".size call_dirty32, .-call_dirty32\n");
extern int call_dirty32(jt32_fn fn, uint64_t upper, uint32_t low);
#endif

static const int expected[10] = {
    1001, 2003, 3007, 4013, 5017, 6019, 7027, 8033, 9901, 9901
};

int main(void) {
    for (uint32_t x = 0; x < 10; ++x) {
        if (jt32(x) != expected[x])
            return 1;
    }
#if defined(__x86_64__)
    const uint64_t uppers[] = {
        0x1234567800000000ULL, 0xffffffff00000000ULL, 0x8000000000000000ULL
    };
    for (unsigned p = 0; p < 3; ++p) {
        for (uint32_t x = 0; x < 10; ++x) {
            if (call_dirty32(jt32, uppers[p], x) != expected[x])
                return 2;
        }
    }
#endif
    puts("jt32 ok");
    return 0;
}
