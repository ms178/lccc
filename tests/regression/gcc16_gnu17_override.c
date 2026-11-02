/* An explicit older dialect remains an escape hatch from the gnu23 default. */
#if !defined(__STDC_VERSION__) || __STDC_VERSION__ != 201710L
#error "-std=gnu17 must select the C17 version macro"
#endif
#ifdef __STRICT_ANSI__
#error "-std=gnu17 must remain a GNU dialect"
#endif

static int unspecified_args() { return 42; }
typedef int (*unspecified_args_fn)();
static unspecified_args_fn unspecified_args_pointer = unspecified_args;

int main(void) {
    int value = 42;
    return value != 42 || unspecified_args(1) != 42 ||
           unspecified_args_pointer(1) != 42;
}
