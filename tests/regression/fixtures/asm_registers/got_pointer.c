/* i686, position-independent code: an assembler statement that takes %ebx
   in a function that needs the GOT pointer.

   lccc keeps the GOT pointer in %ebx for the whole function. A statement
   that names the register -- a `"b"` operand, the `"=b"` of a CPUID, a
   clobber, a register variable -- overwrites it, and everything behind the
   statement that reads a global or calls through the PLT went through the
   wrong register: every function below crashed in a PIE or a shared
   object, at every optimization level.

   Each function reads globals before and after the statement and calls a
   libc function behind it (through the PLT in position-independent code). */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

int counter = 5;               /* another module may define it: through the GOT */
static int local_counter = 11; /* relative to the GOT pointer */
char text[16] = "abcdefgh";
unsigned out_a, out_b, out_c, out_d;

#define NOINLINE __attribute__((noinline))

/* an input in %ebx */
NOINLINE int input_in_ebx(int x) {
    int before = counter;
    int seen;
    __asm__ volatile("movl %%ebx, %0" : "=r"(seen) : "b"(x));
    return (int)strlen(text) + before + counter + seen + local_counter;
}

/* an output in %ebx */
NOINLINE unsigned output_in_ebx(unsigned x) {
    unsigned r;
    int before = counter;
    __asm__ volatile("leal 3(%1), %%ebx" : "=b"(r) : "r"(x));
    return r + (unsigned)strlen(text) + before + counter + local_counter;
}

/* a clobber, in three spellings */
NOINLINE int clobbered(int x) {
    int before = counter;
    __asm__ volatile("xorl %%ebx, %%ebx" : : : "ebx");
    before += (int)strlen(text);
    __asm__ volatile("xorl %%ebx, %%ebx" : : : "bl");
    before += counter;
    __asm__ volatile("xorl %%ebx, %%ebx" : : : "%bx");
    return x + before + counter + local_counter + atoi("7");
}

/* four outputs, as CPUID has them, stored to globals */
NOINLINE unsigned four_outputs(unsigned leaf) {
    __asm__ volatile("movl %%eax, %%ebx\n\taddl $1, %%ebx\n\tleal 2(%%eax), %%ecx\n\tleal 3(%%eax), %%edx"
                     : "=a"(out_a), "=b"(out_b), "=c"(out_c), "=d"(out_d)
                     : "a"(leaf));
    return out_a + 10 * out_b + 100 * out_c + 1000 * out_d + (unsigned)strlen(text);
}

/* CPUID itself */
NOINLINE unsigned vendor(char name[13]) {
    unsigned a, b, c, d;
    __asm__ volatile("cpuid" : "=a"(a), "=b"(b), "=c"(c), "=d"(d) : "a"(0), "c"(0));
    memcpy(name, &b, 4);
    memcpy(name + 4, &d, 4);
    memcpy(name + 8, &c, 4);
    name[12] = 0;
    return (unsigned)strlen(name) + (unsigned)counter + (a != 0);
}

/* in and out through %ebx, in a loop that reads and writes globals */
NOINLINE unsigned in_a_loop(unsigned n) {
    unsigned acc = 0;
    for (unsigned i = 0; i < n; i++) {
        unsigned v = i + (unsigned)counter;
        __asm__ volatile("addl $7, %%ebx" : "+b"(v));
        acc += v + (unsigned)local_counter + (unsigned char)text[i & 7];
        counter += 1;
    }
    return acc;
}

/* a register variable */
NOINLINE int register_variable(int x) {
    register int r __asm__("ebx") = x * 3;
    int seen;
    __asm__ volatile("movl %1, %0" : "=r"(seen) : "r"(r));
    return seen + (int)strlen(text) + counter;
}

/* memory operands that name globals, with %ebx taken */
NOINLINE int symbols_with_ebx_taken(int x) {
    int r;
    __asm__ volatile("movl %2, %%ebx\n\taddl %1, %%ebx\n\tmovl %%ebx, %0"
                     : "=&r"(r)
                     : "m"(counter), "m"(local_counter), "b"(x));
    return r + (int)strlen(text);
}

/* the 64-bit compare and exchange, the classical user of %ebx:%ecx */
NOINLINE unsigned long long compare_exchange(unsigned long long *p, unsigned long long old,
                                             unsigned long long new_value) {
    unsigned long long previous;
    __asm__ volatile("lock; cmpxchg8b %1"
                     : "=A"(previous), "+m"(*p)
                     : "b"((unsigned)new_value), "c"((unsigned)(new_value >> 32)), "0"(old)
                     : "memory", "cc");
    return previous + (unsigned)strlen(text) + (unsigned)counter;
}

/* two statements with a call between them */
NOINLINE int two_statements(int x) {
    int a, b;
    __asm__ volatile("movl %%ebx, %0" : "=r"(a) : "b"(x + counter));
    puts("between");
    __asm__ volatile("movl %%ebx, %0" : "=r"(b) : "b"(x + local_counter));
    return a + b + atoi("12");
}

/* the address of a global as the input */
NOINLINE int address_in_ebx(void) {
    int r;
    __asm__ volatile("movl (%%ebx), %0" : "=r"(r) : "b"(&counter));
    return r + (int)strlen(text) + local_counter;
}

int main(void) {
    unsigned long long cell = 0x1122334455667788ull;
    char name[13];
    printf("input_in_ebx %d\n", input_in_ebx(100));
    printf("output_in_ebx %u\n", output_in_ebx(40));
    printf("clobbered %d\n", clobbered(9));
    printf("four_outputs %u\n", four_outputs(1));
    printf("vendor %u", vendor(name));
    printf(" %zu\n", strlen(name));
    printf("in_a_loop %u\n", in_a_loop(5));
    printf("register_variable %d\n", register_variable(4));
    printf("symbols_with_ebx_taken %d\n", symbols_with_ebx_taken(1000));
    printf("compare_exchange %llx", compare_exchange(&cell, 0x1122334455667788ull, 0xaabbccdd00112233ull));
    printf(" %llx\n", cell);
    printf("two_statements %d\n", two_statements(1));
    printf("address_in_ebx %d\n", address_in_ebx());
    printf("counter %d\n", counter);
    return 0;
}
