/* PR target/92904 + GCC 4.6 ABI: MEMORY-class varargs whose natural
 * alignment exceeds 16 bytes are placed at slots aligned to their FULL
 * alignment. GCC's callee-side va_arg overflow walk aligns dynamically
 * (addq $A-1; andq $-A), so the CALLER must realign %rsp dynamically —
 * a static 16-aligned push sequence lands the argument at %rsp mod 32/64
 * half the time and the callee reads past it (ASLR makes the abort look
 * nondeterministic; ~50% of runs aborted before the fix at every -O level).
 * Pinned by gcc.c-torture/execute/pr92904.c (which also exercises the
 * named-param side); this is the minimal variadic shape.
 */
#include <stdarg.h>

struct __attribute__((aligned (32))) V { double a, b, c, d; };
struct V sink;

__attribute__((noinline)) void
take (int x, ...)
{
  va_list ap;
  va_start (ap, x);
  while (x--)
    va_arg (ap, double);
  sink = va_arg (ap, struct V);
  va_end (ap);
}

struct V src = { 1.5, 2.5, 3.5, 4.5 };

int main (void)
{
  /* x=0: struct is the first stack argument (anchor itself). */
  take (0, src);
  if (sink.a != 1.5 || sink.b != 2.5 || sink.c != 3.5 || sink.d != 4.5)
    return 1;
  /* x=9: 8 doubles in XMM registers + one stack double ahead of the
   * struct — the anchor is the stack double and the struct must still
   * land on a 32-byte boundary (24 bytes of inter-arg padding). */
  take (9, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, src);
  if (sink.a != 1.5 || sink.b != 2.5 || sink.c != 3.5 || sink.d != 4.5)
    return 2;
  /* 5 leading stack doubles: anchor parity flips, struct must follow the
   * 8-byte slots at the next 32-byte boundary. */
  take (5, 2.0, 2.0, 2.0, 2.0, 2.0, src);
  if (sink.a != 1.5 || sink.b != 2.5 || sink.c != 3.5 || sink.d != 4.5)
    return 3;
  return 0;
}
