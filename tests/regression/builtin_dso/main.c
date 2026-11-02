/* Linked as `main.o -L. -ldso -Wl,-rpath,$ORIGIN`: libdso.so must be found
   through DT_RUNPATH and searched before libc (gcc's -l order). atexit is
   libc_nonshared.a's (named by the libc.so linker script) in executables
   too; handlers run in reverse registration order, so this one runs before
   the one libdso's constructor registered. */
extern int printf(const char *, ...);
extern int puts(const char *);
extern char *getenv(const char *);
extern int atexit(void (*)(void));
int dso_value(void);

static void main_bye(void) { puts("main atexit"); }

int main(void) {
    atexit(main_bye);
    printf("main %s %d\n", getenv("HOME"), dso_value());
    return 0;
}
