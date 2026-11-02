// Shared-object identity fixture: keep text fixed and change only catch type.
extern void gf_maythrow();
#ifdef GF_IDENTICAL_CATCH
#define GF_SECOND_TYPE int
#else
#define GF_SECOND_TYPE long
#endif
extern "C" __attribute__((noinline)) int guard_int() {
    try { gf_maythrow(); } catch (int) { return 7; }
    return 0;
}
extern "C" __attribute__((noinline)) int guard_long() {
    try { gf_maythrow(); } catch (GF_SECOND_TYPE) { return 7; }
    return 0;
}
