/* GCC 16.2 defaults to GNU C23. This runs without an explicit -std= flag so
 * the regression corpus compares the driver defaults, not only `-std=c23`. */
#if !defined(__STDC_VERSION__) || __STDC_VERSION__ != 202311L
#error "the default C dialect must be GNU C23"
#endif
#ifdef __STRICT_ANSI__
#error "the default must retain GNU extensions"
#endif
#if !true || false
#error "C23 true/false must work in preprocessing"
#endif

constexpr int answer = 42;
static int no_args() { return answer; }
typedef int (*no_args_fn)();
static no_args_fn no_args_pointer = no_args;

int main(void) {
    bool ok = true;
    auto deduced = answer;
    void *empty = nullptr;
    return !(ok && deduced == 42 && empty == nullptr && no_args() == 42 &&
             no_args_pointer() == 42);
}
