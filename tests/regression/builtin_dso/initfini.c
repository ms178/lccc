/* Legacy .init/.fini fragments (check_builtin_dso_link.sh).  crti.o's
   _init/_fini frame encloses them, so they run iff DT_INIT/DT_FINI record
   _init/_fini -- in the executable and in the DSO alike.  RISC-V glibc has
   no crti frame (GNU ld emits no DT_INIT there either), so none run.
   Built twice: as libif.so (TAG "dso") and into the program (TAG "exe").
   The DSO also calls getc, which glibc defines STB_WEAK: its import must
   still be STB_GLOBAL (the binding describes this module's references). */
extern int puts(const char *);
extern int getc(void *);
#define CAT2(a, b) a##b
#define CAT(a, b) CAT2(a, b)
#define HOOK(k) CAT(CAT(k, _hook_), ID)
__attribute__((visibility("hidden"), used)) void HOOK(init)(void) { puts(TAG " .init"); }
__attribute__((visibility("hidden"), used)) void HOOK(fini)(void) { puts(TAG " .fini"); }
#define STR2(x) #x
#define STR(x) STR2(x)
#if defined(__aarch64__)
#define CALL "bl "
#else
#define CALL "call "
#endif
#if !defined(__riscv)
__asm__(".section .init,\"ax\"\n " CALL STR(HOOK(init)) "\n"
        ".section .fini,\"ax\"\n " CALL STR(HOOK(fini)) "\n.text\n");
#endif
#ifdef WITH_MAIN
int if_value(void);
int main(void) { puts("main"); return if_value() != 7; }
#else
int if_value(void) { return 7; }
int if_strong_weakdef(void *f) { return getc(f); }
#endif
