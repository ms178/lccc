/* Run-time witness that code alignment and code-span data survive linker
 * relaxation (see check_riscv_relax_layout.sh).  Freestanding: no libc, the
 * exit status is the number of failed checks. */
typedef unsigned long uptr;

static __attribute__((noreturn)) void sys_exit(long code)
{
    register long a0 __asm__("a0") = code;
    register long a7 __asm__("a7") = 93;
    __asm__ volatile("ecall" : : "r"(a0), "r"(a7));
    for (;;) {
    }
}

static int failures;
static void check(int ok) { failures += !ok; }

__attribute__((noinline)) int callee(int x) { return x * 3 + 1; }

/* Relaxable calls in front of a dense switch: lccc's jump table is
 * `.word .LBBn - .Ljt`, ADD/SUB pairs that must follow the shrinking code. */
__attribute__((noinline)) int sw(int x)
{
    switch (x) {
    case 0: return callee(1);
    case 1: return callee(2) + 7;
    case 2: return callee(x) + 2;
    case 3: return 11 + callee(3);
    case 4: return 13;
    case 5: return callee(5) - 17;
    case 6: return 19;
    case 7: return callee(7) * 2;
    default: return -1;
    }
}

/* An over-aligned function after relaxable code: its `.p2align 6` is an
 * R_RISCV_ALIGN run the linker must trim to exactly the boundary. */
__attribute__((noinline)) int before_aligned(int x) { return callee(x) + callee(x + 1); }
__attribute__((aligned(64), noinline)) int aligned_fn(int x) { return x + 5; }
__attribute__((noinline)) int after_aligned(int x) { return callee(x) - 1; }

/* Computed goto: a table of lone code labels (`.quad .Ln`). */
__attribute__((noinline)) int cgoto(int i)
{
    static void *const tbl[] = {&&l0, &&l1, &&l2};
    int acc = callee(i);
    goto *tbl[i];
l0: return acc + 100;
l1: return acc + 200;
l2: return acc + 300;
}

/* Hand-written spans over relaxable code: `.word`, `.uleb128` and a `. -`
 * difference must equal the distance in the final image. */
__asm__(".text\n"
        ".globl span_start\n.globl span_end\n.globl span_mid\n"
        "span_start:\n"
        "1:  call callee\n"
        "    tail callee\n"
        "    .p2align 4\n"
        "span_mid:\n"
        "2:  call callee\n"
        "span_end:\n"
        "    ret\n"
        "3:  .word 1b - .\n"
        ".pushsection .rodata\n"
        ".globl span_word\n.globl span_uleb\n.globl span_dot_at\n"
        "span_word: .word 2b - 1b\n"
        "span_dot_at: .word 3b - span_start\n"
        "span_uleb: .uleb128 span_end - 1b\n"
        "           .uleb128 2b - 1b + 3\n"
        ".popsection\n");
extern char span_start[], span_mid[], span_end[];
extern const int span_word, span_dot_at;
extern const unsigned char span_uleb[];

static uptr uleb(const unsigned char **p)
{
    uptr v = 0;
    unsigned shift = 0;
    unsigned char b;
    do {
        b = *(*p)++;
        v |= (uptr)(b & 0x7f) << shift;
        shift += 7;
    } while (b & 0x80);
    return v;
}

/* The entry point sets gp as crt1 does, so a linker that relaxes accesses to
 * gp-relative form (GNU ld defines __global_pointer$) is exercised too; weak
 * for linkers that define no global pointer (then gp is 0 and unused). */
__asm__(".text\n.globl _start\n.weak __global_pointer$\n"
        "_start:\n"
        ".option push\n.option norelax\n"
        "    la gp, __global_pointer$\n"
        ".option pop\n"
        "    tail start_c\n");

void start_c(void)
{
    static const int want[] = {4, 14, 9, 21, 13, -1, 19, 44};
    volatile int n = 8;
    for (int i = 0; i < n; i++)
        check(sw(i) == want[i]);
    check(sw(9) == -1);
    check(((uptr)&aligned_fn & 63) == 0);
    check(aligned_fn(1) == 6 && before_aligned(1) == 11 && after_aligned(2) == 6);
    check(cgoto(0) == 101 && cgoto(1) == 204 && cgoto(2) == 307);
    check(((uptr)span_mid & 15) == 0);
    check(span_word == span_mid - span_start);
    const char *at3 = span_start + span_dot_at;
    check(*(const int *)at3 == (int)(span_start - at3));
    const unsigned char *p = span_uleb;
    check(uleb(&p) == (uptr)(span_end - span_start));
    check(uleb(&p) == (uptr)(span_mid - span_start) + 3);
    sys_exit(failures);
}
