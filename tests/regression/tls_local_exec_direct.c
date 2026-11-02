/* PF-TLS-1: Local-Exec `__thread` accesses must fold into ONE segment-
   relative instruction with the TPOFF relocation in the memory operand
   (`movq %rdi, %fs:a@tpoff`), the form GCC and Clang emit outside
   `-fPIC`/`-shared`.  Every width the fold supports is exercised, plain
   and through a constant-offset GEP into a TLS array, plus an extern
   (Initial-Exec) symbol that must KEEP its GOT-based sequence.
   The addend-carrying form (`%fs:arr@TPOFF+16`) is what the assembler's
   `sym@MOD+N` parsing exists for: parsed as a plain `symbol+offset` it
   degrades to an absolute relocation and links the wrong address.  */
#include <stdio.h>

static __thread unsigned long ul, arr[4];
static __thread long sl;
static __thread unsigned int ui;
static __thread int si;
static __thread unsigned short us;
static __thread short ss;
static __thread unsigned char uc;
static __thread signed char sc;
static __thread unsigned long *ptr_slot;

__attribute__((noinline)) static void set_all(unsigned long v) {
    ul = v;
    sl = (long)v + 1;
    ui = (unsigned int)(v + 2);
    si = (int)(v + 3);
    us = (unsigned short)(v + 4);
    ss = (short)(v + 5);
    uc = (unsigned char)(v + 6);
    sc = (signed char)(v + 7);
    arr[0] = v + 8;
    arr[1] = v + 9;
    arr[2] = v + 10;
    arr[3] = v + 11;
}

__attribute__((noinline)) static unsigned long sum_all(void) {
    unsigned long acc = 0;
    acc += ul;
    acc += (unsigned long)sl;
    acc += ui;
    acc += (unsigned long)si;
    acc += us;
    acc += (unsigned long)ss;
    acc += uc;
    acc += (unsigned long)sc;
    acc += arr[0] + arr[1] + arr[2] + arr[3];
    return acc;
}

int main(void) {
    unsigned long base = 0x1122334455667700UL;
    set_all(base);
    /* A TLS pointer slot: the stored address must be the calling thread's
       instance, whose contents are then read back through the pointer. */
    ptr_slot = &arr[2];
    arr[2] = base + 100;
    unsigned long through = *ptr_slot;
    printf("%lu %lu %d\n", sum_all(), through, ptr_slot == &arr[2]);
    return 0;
}
