/* Every spelling of a general register GCC takes in a clobber list.

   Each function keeps five values alive across a statement that destroys
   one register and says so in one spelling: `"bx"`, `"bl"`, `"bh"`, `"ebx"`
   and `"rbx"` all name %ebx (GCC takes the 64-bit name in 32-bit code too).
   A compiler that does not know a spelling keeps one of the five values in
   the destroyed register, or does not save the register for its caller.

   Both are checked: the result is printed, and the function is called
   through `call_checked`, which loads every callee-saved register with a
   known value first and reports the ones that came back changed.

   Compiled with the frame pointer omitted (-O1 and up), where the frame
   pointer register is one more callee-saved register; with a frame pointer
   GCC refuses its spellings ("bp cannot be used in 'asm' here"), which
   `frame_pointer_refused.c` holds lccc to. */
#include <stdio.h>

#define NOINLINE __attribute__((noinline))

#if defined(__x86_64__)
typedef long word;
#else
typedef int word;
#endif

typedef word (*function)(word, word);

/* Calls f(x, y) with known values in the callee-saved registers. Stores the
   result and returns a mask of the registers f did not preserve. */
unsigned call_checked(function f, word x, word y, word *result);

#if defined(__x86_64__)
__asm__(".text\n"
        ".type call_checked, @function\n"
        "call_checked:\n"
        "    pushq %rbp\n"
        "    pushq %rbx\n"
        "    pushq %r12\n"
        "    pushq %r13\n"
        "    pushq %r14\n"
        "    pushq %r15\n"
        "    pushq %rcx\n"
        "    movabsq $0x1111111111111111, %rbx\n"
        "    movabsq $0x2222222222222222, %r12\n"
        "    movabsq $0x3333333333333333, %r13\n"
        "    movabsq $0x4444444444444444, %r14\n"
        "    movabsq $0x5555555555555555, %r15\n"
        "    movabsq $0x6666666666666666, %rbp\n"
        "    movq %rdi, %rax\n"
        "    movq %rsi, %rdi\n"
        "    movq %rdx, %rsi\n"
        "    call *%rax\n"
        "    popq %rcx\n"
        "    movq %rax, (%rcx)\n"
        "    xorl %eax, %eax\n"
        "    movabsq $0x1111111111111111, %rdx\n"
        "    cmpq %rdx, %rbx\n"
        "    je 1f\n"
        "    orl $1, %eax\n"
        "1:  movabsq $0x2222222222222222, %rdx\n"
        "    cmpq %rdx, %r12\n"
        "    je 2f\n"
        "    orl $2, %eax\n"
        "2:  movabsq $0x3333333333333333, %rdx\n"
        "    cmpq %rdx, %r13\n"
        "    je 3f\n"
        "    orl $4, %eax\n"
        "3:  movabsq $0x4444444444444444, %rdx\n"
        "    cmpq %rdx, %r14\n"
        "    je 4f\n"
        "    orl $8, %eax\n"
        "4:  movabsq $0x5555555555555555, %rdx\n"
        "    cmpq %rdx, %r15\n"
        "    je 5f\n"
        "    orl $16, %eax\n"
        "5:  movabsq $0x6666666666666666, %rdx\n"
        "    cmpq %rdx, %rbp\n"
        "    je 6f\n"
        "    orl $32, %eax\n"
        "6:  popq %r15\n"
        "    popq %r14\n"
        "    popq %r13\n"
        "    popq %r12\n"
        "    popq %rbx\n"
        "    popq %rbp\n"
        "    ret\n"
        ".size call_checked, .-call_checked\n");
#else
__asm__(".text\n"
        ".type call_checked, @function\n"
        "call_checked:\n"
        "    pushl %ebp\n"
        "    pushl %ebx\n"
        "    pushl %esi\n"
        "    pushl %edi\n"
        "    subl $4, %esp\n"
        "    movl $0x11111111, %ebx\n"
        "    movl $0x22222222, %esi\n"
        "    movl $0x33333333, %edi\n"
        "    movl $0x44444444, %ebp\n"
        "    pushl 32(%esp)\n"
        "    pushl 32(%esp)\n"
        "    call *32(%esp)\n"
        "    addl $12, %esp\n"
        "    movl 32(%esp), %ecx\n"
        "    movl %eax, (%ecx)\n"
        "    xorl %eax, %eax\n"
        "    cmpl $0x11111111, %ebx\n"
        "    je 1f\n"
        "    orl $1, %eax\n"
        "1:  cmpl $0x22222222, %esi\n"
        "    je 2f\n"
        "    orl $2, %eax\n"
        "2:  cmpl $0x33333333, %edi\n"
        "    je 3f\n"
        "    orl $4, %eax\n"
        "3:  cmpl $0x44444444, %ebp\n"
        "    je 4f\n"
        "    orl $8, %eax\n"
        "4:  popl %edi\n"
        "    popl %esi\n"
        "    popl %ebx\n"
        "    popl %ebp\n"
        "    ret\n"
        ".size call_checked, .-call_checked\n");
#endif

/* X(function name, the instruction that destroys the register, the clobber) */
#define COMMON(X)                                    \
    X(ax_eax, "xorl %%eax, %%eax", "eax")           \
    X(ax_ax, "xorl %%eax, %%eax", "ax")             \
    X(ax_al, "xorl %%eax, %%eax", "al")             \
    X(ax_ah, "xorl %%eax, %%eax", "ah")             \
    X(ax_rax, "xorl %%eax, %%eax", "rax")           \
    X(ax_percent, "xorl %%eax, %%eax", "%eax")      \
    X(bx_ebx, "xorl %%ebx, %%ebx", "ebx")           \
    X(bx_bx, "xorl %%ebx, %%ebx", "bx")             \
    X(bx_bl, "xorl %%ebx, %%ebx", "bl")             \
    X(bx_bh, "xorl %%ebx, %%ebx", "bh")             \
    X(bx_rbx, "xorl %%ebx, %%ebx", "rbx")           \
    X(bx_percent, "xorl %%ebx, %%ebx", "%ebx")      \
    X(bx_percent_bl, "xorl %%ebx, %%ebx", "%bl")    \
    X(cx_ecx, "xorl %%ecx, %%ecx", "ecx")           \
    X(cx_cx, "xorl %%ecx, %%ecx", "cx")             \
    X(cx_cl, "xorl %%ecx, %%ecx", "cl")             \
    X(cx_ch, "xorl %%ecx, %%ecx", "ch")             \
    X(cx_rcx, "xorl %%ecx, %%ecx", "rcx")           \
    X(dx_edx, "xorl %%edx, %%edx", "edx")           \
    X(dx_dx, "xorl %%edx, %%edx", "dx")             \
    X(dx_dl, "xorl %%edx, %%edx", "dl")             \
    X(dx_dh, "xorl %%edx, %%edx", "dh")             \
    X(dx_rdx, "xorl %%edx, %%edx", "rdx")           \
    X(si_esi, "xorl %%esi, %%esi", "esi")           \
    X(si_si, "xorl %%esi, %%esi", "si")             \
    X(si_sil, "xorl %%esi, %%esi", "sil")           \
    X(si_rsi, "xorl %%esi, %%esi", "rsi")           \
    X(si_percent, "xorl %%esi, %%esi", "%si")       \
    X(di_edi, "xorl %%edi, %%edi", "edi")           \
    X(di_di, "xorl %%edi, %%edi", "di")             \
    X(di_dil, "xorl %%edi, %%edi", "dil")           \
    X(di_rdi, "xorl %%edi, %%edi", "rdi")           \
    X(bp_ebp, "xorl %%ebp, %%ebp", "ebp")           \
    X(bp_bp, "xorl %%ebp, %%ebp", "bp")             \
    X(bp_bpl, "xorl %%ebp, %%ebp", "bpl")           \
    X(bp_rbp, "xorl %%ebp, %%ebp", "rbp")           \
    X(bp_percent, "xorl %%ebp, %%ebp", "%ebp")

/* GCC knows the extended registers by their 64-bit names only. */
#if defined(__x86_64__)
#define EXTENDED(X)                                  \
    X(r8, "xorl %%r8d, %%r8d", "r8")                \
    X(r9, "xorl %%r9d, %%r9d", "r9")                \
    X(r10, "xorl %%r10d, %%r10d", "r10")            \
    X(r11, "xorl %%r11d, %%r11d", "r11")            \
    X(r12, "xorl %%r12d, %%r12d", "r12")            \
    X(r13, "xorl %%r13d, %%r13d", "r13")            \
    X(r14, "xorl %%r14d, %%r14d", "r14")            \
    X(r15, "xorl %%r15d, %%r15d", "r15")            \
    X(r12_percent, "xorl %%r12d, %%r12d", "%r12")
#else
#define EXTENDED(X)
#endif

#define DEFINE(id, destroy, clobber)                                              \
    NOINLINE word clobber_##id(word x, word y) {                                  \
        word a = x * 3 + y, b = x - y, c = x ^ y, d = x + 7 * y, e = y - 5 * x;   \
        __asm__ volatile(destroy : : : clobber);                                  \
        return a + 2 * b + 3 * c + 5 * d + 7 * e;                                 \
    }
COMMON(DEFINE)
EXTENDED(DEFINE)

/* Several registers in one statement, each by another spelling. */
NOINLINE word clobber_three(word x, word y) {
    word a = x * 3 + y, b = x - y, c = x ^ y, d = x + 7 * y, e = y - 5 * x;
    __asm__ volatile("xorl %%ebx, %%ebx\n\txorl %%esi, %%esi\n\txorl %%edi, %%edi" : : : "bl", "si", "%edi");
    return a + 2 * b + 3 * c + 5 * d + 7 * e;
}

/* A register variable in a callee-saved register is an operand in it: the
   statement reads the register by name, so the value has to be there. */
NOINLINE word register_variable(word x, word y) {
    word a = x * 3 + y, b = x - y, c = x ^ y, d = x + 7 * y;
    register word in_bx __asm__("ebx") = y - 5 * x;
    word e;
    __asm__ volatile("movl %%ebx, %k0" : "=r"(e) : "r"(in_bx));
    return a + 2 * b + 3 * c + 5 * d + 7 * (int)e;
}

#define ENTRY(id, destroy, clobber) { #id " as " clobber, clobber_##id },

static const struct {
    const char *name;
    function f;
} table[] = {
    COMMON(ENTRY)
    EXTENDED(ENTRY)
    { "three", clobber_three },
    { "register variable", register_variable },
};

int main(void) {
    for (unsigned i = 0; i < sizeof table / sizeof *table; i++) {
        word result = 0;
        unsigned lost = call_checked(table[i].f, 1000 + (word)i, 77 - (word)i, &result);
        printf("%s: %ld, callee-saved registers lost %#x\n", table[i].name, (long)result, lost);
    }
    return 0;
}
