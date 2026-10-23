/* A shared library linked by lccc's builtin linker (check_builtin_dso_link.sh).
   The constructor's atexit() needs crtbeginS.o's __dso_handle and
   libc_nonshared.a's atexit; getenv() interposes libc's for every module
   whose lookup scope reaches this library before libc. */
extern int printf(const char *, ...);
extern int atexit(void (*)(void));

static void bye(void) { printf("dso atexit\n"); }

__attribute__((constructor)) static void init(void) {
    printf("dso ctor\n");
    atexit(bye);
}

char *getenv(const char *name) {
    (void)name;
    return "interposed";
}

int dso_value(void) { return 42; }
