/* Regression test: volatile stack-slot markers must be scoped to their own
 * function, and volatile-demoted lines must keep their real destination
 * register.
 *
 * Historical bug (found via sqlite3 miscompile): `# LCCC_VOLATILE_SLOT`
 * markers were collected FILE-WIDE; every line touching the same numeric
 * slot offset in ANY function was demoted to Other{REG_NONE}.  A demoted
 * `movq 32(%rsp), %rax` argument reload looked like "rax not written" to
 * combined_local_pass's rax_is_zero tracking, so the `xorl %eax, %eax`
 * zeroing the NEXT stack argument was removed as redundant and the call
 * received a stale pointer instead of NULL (sqlite3SelectNew got
 * pOrderBy = garbage and crashed walking it).
 *
 * a_func's four volatile locals make the codegen emit markers at
 * 40/32/24/16(%rsp); b_func's callee() argument reload happens to load
 * 32(%rsp) -- the same numeric offset -- inside the zeroing window.
 * The weak definitions keep the test self-contained while preserving the
 * external-call codegen shape.
 */
extern int callee(int a, int b, int c, int d, int e, int f, void *g, void *h, void *i);
extern int pre(void);

__attribute__((weak)) int pre(void){ return 0; }
__attribute__((weak)) int callee(int a,int b,int c,int d,int e,int f,void *g,void *h,void *i){
  if (g != 0) return -1;   /* arg7 must be NULL */
  if (h == 0) return -2;   /* arg8 must be &f (non-NULL) */
  return a+b+c+d+e+f;
}

int a_func(int x){
  volatile int v1 = x;
  volatile int v2 = x + 1;
  volatile int v3 = x + 2;
  volatile int v4 = x + 3;
  return v1 + v2 + v3 + v4;
}

int b_func(int x){
  int f = x + 1;
  int t = pre();                    /* clobber caller-saved regs */
  return callee(1, 2, 3, 4, 5, 6, 0, &f, 0) + t;
}

int main(void){
  /* a_func must not be miscompiled either */
  if (a_func(1) != 10) return 2;    /* 1+2+3+4 */
  /* callee checks g==NULL (arg7) and h!=NULL (arg8=&f); sum = 21 */
  if (b_func(4) != 21) return 1;
  return 0;
}
