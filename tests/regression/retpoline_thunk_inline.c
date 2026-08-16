/* -mindirect-branch=thunk-inline: full retpoline expanded at each site.
 *
 * The kernel vDSO needs this form — it is userspace code that cannot
 * reference the kernel's __x86_indirect_thunk_* symbols, so thunk-extern is
 * not linkable there. The inline sequence must be semantically a plain
 * indirect call/jump (same results, same stack discipline) while never
 * executing a naked `call *%reg`/`jmp *%reg`.
 *
 * Runtime checks: indirect calls through function pointers (call form),
 * a switch dense enough for a jump table (indirect jmp form), deep
 * call chains (RSB discipline: the rewritten return addresses must not
 * corrupt the real ones), and correct argument/return passing throughout.
 */
#include <stdio.h>

__attribute__((noinline)) static int add1(int x) { return x + 1; }
__attribute__((noinline)) static int mul2(int x) { return x * 2; }
__attribute__((noinline)) static int sub3(int x) { return x - 3; }

static int (*table[3])(int) = { add1, mul2, sub3 };

__attribute__((noinline)) static int dispatch(int which, int v)
{
       /* Dense switch: eligible for a jump-table lowering, whose dispatch is
        * an indirect JMP that must also be retpolined. */
       switch (which & 7) {
       case 0: v += 10; break;
       case 1: v += 20; break;
       case 2: v += 30; break;
       case 3: v += 40; break;
       case 4: v += 50; break;
       case 5: v += 60; break;
       case 6: v += 70; break;
       case 7: v += 80; break;
       }
       return v;
}

__attribute__((noinline)) static int chain(int depth, int v)
{
       if (depth == 0)
               return table[v % 3](v);
       return chain(depth - 1, v) + 1;
}

int main(void)
{
       int v = 0;
       /* Call form through every table slot. */
       for (int i = 0; i < 3; i++)
               v += table[i](10);      /* 11 + 20 + 7 = 38 */
       if (v != 38) {
               printf("FAIL calls v=%d\n", v);
               return 1;
       }
       /* Jump-table form. */
       int s = 0;
       for (int i = 0; i < 8; i++)
               s += dispatch(i, 1);    /* 8*1 + (10+..+80) = 368 */
       if (s != 368) {
               printf("FAIL switch s=%d\n", s);
               return 2;
       }
       /* Return-address integrity through 64 stacked retpolines. */
       int c = chain(64, 9);           /* add1? 9%3=0 -> add1(9)=10; +64 */
       if (c != 74) {
               printf("FAIL chain c=%d\n", c);
               return 3;
       }
       printf("PASS retpoline_thunk_inline\n");
       return 0;
}
