/* Unwinding through lccc-compiled frames: every function on the chain
   main -> f1 -> lib_entry -> f2 -> f3 must be found by libgcc's unwinder,
   identified by the start address of its FDE.

   Built whole, or split: -DUNWIND_LIB (cb, f2, f3, lib_entry; as a shared
   library, where only PT_GNU_EH_FRAME leads the unwinder to the FDEs) and
   -DUNWIND_MAIN (f1, main). f2/f3 are hidden in the split build so the
   library records their real addresses (a canonical function address seen
   from the executable may be a PLT stub). */
extern int _Unwind_Backtrace(int (*)(void *, void *), void *);
extern unsigned long _Unwind_GetRegionStart(void *);
extern int printf(const char *, ...);

struct st {
    unsigned long want[5];
    int found;
    int depth;
};

int lib_entry(struct st *s, int x);

#ifndef UNWIND_MAIN
#ifdef UNWIND_LIB
#define LOCAL __attribute__((visibility("hidden")))
#else
#define LOCAL
#endif

static int cb(void *ctx, void *arg) {
    struct st *s = arg;
    unsigned long r = _Unwind_GetRegionStart(ctx);
    for (int i = 0; i < 5; i++)
        if (r == s->want[i])
            s->found |= 1 << i;
    s->depth++;
    return 0; /* _URC_NO_REASON */
}

volatile int lib_sink;

LOCAL __attribute__((noinline)) int f3(struct st *s, int x) {
    volatile char big[5000]; /* frame larger than a 12-bit immediate */
    big[x] = (char)x;
    _Unwind_Backtrace(cb, s);
    return s->found + big[x];
}

LOCAL __attribute__((noinline)) int f2(struct st *s, int x) {
    int r = f3(s, x + 1);
    lib_sink = r;
    return r + x;
}

__attribute__((noinline)) int lib_entry(struct st *s, int x) {
    s->want[0] = (unsigned long)f3;
    s->want[1] = (unsigned long)f2;
    s->want[2] = (unsigned long)lib_entry;
    int r = f2(s, x);
    lib_sink = r;
    return r - 2;
}
#endif

#ifndef UNWIND_LIB
volatile int sink;

__attribute__((noinline)) int f1(struct st *s, int x) {
    int r = lib_entry(s, x * 2);
    sink = r;
    return r - 1;
}

int main(void) {
    struct st s = {{0, 0, 0, (unsigned long)f1, (unsigned long)main}, 0, 0};
    int r = f1(&s, 3);
    printf("found=%x depth=%d r=%d\n", s.found, s.depth, r);
    return s.found == 31 ? 0 : 1;
}
#endif
