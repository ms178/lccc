/* v5 regression (Agent B audit): a file-scope asm label that is NOT a register
 * name is a linker-symbol redirect, never a register variable. Before the fix
 * both trees emitted `movl %abccb, %abccb` self-moves (register-global path
 * fired for any asm label); register pinning (`__asm__("rbx")`) must keep
 * working. */
#include <stdio.h>
int renamed __asm__("asm_renamed_sym") = 42;
extern int renamed_ref __asm__("asm_renamed_sym");
int get_renamed(void) { return renamed_ref; }
int main(void) {
  if (get_renamed() != 42) { printf("FAIL renamed\n"); return 1; }
  printf("OK asm_symbol_rename\n");
  return 0;
}
