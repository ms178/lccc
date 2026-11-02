/* lccc extension: an `asm goto` output object is defined on *every* outgoing
 * edge, including the template's branch edges.
 *
 * GCC defines ordinary `=r` outputs on the label edge but leaves `=@cc`
 * outputs unspecified there ("valid immediately after the asm"), and Clang
 * defines neither on the taken edge. lccc materialises both, which is a strict
 * superset: no program with defined behaviour under GCC changes meaning, and
 * the kernel-style "branch on a flag without materialising a bool" idiom keeps
 * working on the label path. This file pins the extension so it cannot regress
 * silently; asm_goto_outputs.c stays strictly GCC-compatible.
 */
#include <stdio.h>

static volatile int observed;

/* Flag output read on the label edge. */
__attribute__((noinline)) static int ext_label_flag(int x)
{
    _Bool zero = 0;
    asm goto volatile("testl %1, %1; jz %l[target]"
                      : "=@ccz"(zero)
                      : "r"(x)
                      : "cc"
                      : target);
    return zero ? 2 : 0;
target:
    return zero ? 3 : -1;
}

/* Tied `+r` output and a flag output read together on the label edge. */
__attribute__((noinline)) static int ext_label_tied(int x)
{
    unsigned value = 7;
    _Bool zero = 0;
    asm goto volatile("addl %2, %0; testl %2, %2; jz %l[target]"
                      : "+r"(value), "=@ccz"(zero)
                      : "r"(x)
                      : "cc"
                      : target);
    return value == (unsigned)(7 + x) && !zero ? 24 : -1;
target:
    return value == 7 && zero ? 42 : -2;
}

/* Plain register output and a flag output, both read on the label edge. */
__attribute__((noinline)) static int ext_label_two_outputs(int x)
{
    unsigned value = 0;
    _Bool zero = 0;
    asm goto volatile("movl $42, %0; testl %2, %2; jz %l[target]"
                      : "=r"(value), "=@ccz"(zero)
                      : "r"(x)
                      : "cc"
                      : target);
    return value == 42 ? 24 : -1;
target:
    return value == 42 && zero ? 42 : -2;
}

/* Two template branches: each target reads the flag on its own edge. */
__attribute__((noinline)) static int ext_label_two_targets(int x)
{
    _Bool sign = 0;
    _Bool zero = 0;
    asm goto volatile("testl %2, %2; jz %l[is_zero]; js %l[is_negative]"
                      : "=@ccz"(zero), "=@ccs"(sign)
                      : "r"(x)
                      : "cc"
                      : is_zero, is_negative);
    return 0;
is_zero:
    return zero ? 1 : -1;
is_negative:
    return sign ? 2 : -2;
}

int main(void)
{
    static const int values[] = { 0, 1, -1, 2, -2, 0x100, -0x100 };
    for (unsigned i = 0; i < sizeof(values) / sizeof(values[0]); ++i) {
        int x = values[i];
        int want_flag = x == 0 ? 3 : 0;
        if (ext_label_flag(x) != want_flag) {
            printf("FAIL label flag x=%d\n", x);
            return 1;
        }
        if (ext_label_tied(x) != (x == 0 ? 42 : 24)) {
            printf("FAIL label tied x=%d\n", x);
            return 1;
        }
        if (ext_label_two_outputs(x) != (x == 0 ? 42 : 24)) {
            printf("FAIL label two outputs x=%d\n", x);
            return 1;
        }
        int want = x == 0 ? 1 : (x < 0 ? 2 : 0);
        if (ext_label_two_targets(x) != want) {
            printf("FAIL label two targets x=%d\n", x);
            return 1;
        }
    }
    observed = 0;
    puts("PASS asm_goto_outputs_ext");
    return 0;
}
