#include <limits.h>
#include <stdbool.h>
#include <stddef.h>
#include <stdio.h>
#include <string.h>
#include <sys/wait.h>
#include <unistd.h>

#if !__has_attribute(no_sanitize)
#error "missing __has_attribute(no_sanitize)"
#endif

static int trapped(void (*fn)(void)) {
    pid_t pid = fork();
    if (pid == 0) {
        fn();
        _exit(0);
    }
    int status = 0;
    waitpid(pid, &status, 0);
    return WIFSIGNALED(status) ? 1 : 0;
}

volatile int v_max = INT_MAX;
volatile int v_min = INT_MIN;
volatile int v_zero = 0;
volatile int v_neg1 = -1;
volatile int v_32 = 32;
volatile int v_4 = 4;
volatile int v_5 = 5;
volatile double vd_zero = 0.0;
volatile int *vp_null = 0;
struct Rec { int head; int val; int flex[1]; };
volatile struct Rec *vrec_null = 0;

static void t_sadd(void) { volatile int r = v_max + 1; (void)r; }
static void t_ssub(void) { volatile int r = v_min - 1; (void)r; }
static void t_smul(void) { volatile int r = v_max * 3; (void)r; }
static void t_sneg(void) { volatile int r = -v_min; (void)r; }
static void t_idiv0(void) { volatile int r = 99 / v_zero; (void)r; }
static void t_imin_div(void) { volatile int r = v_min / v_neg1; (void)r; }
static void t_shl_exp(void) { volatile int r = 1 << v_32; (void)r; }
static void t_shl_base(void) { volatile int r = v_neg1 << 1; (void)r; }
static void ok_shr_neg(void) { volatile int r = v_neg1 >> 2; (void)r; }
static void t_bounds(void) { int a[4] = {1, 2, 3, 4}; volatile int r = a[v_4]; (void)r; }
static void ok_one_past(void) { int a[4] = {1, 2, 3, 4}; volatile int *p = &a[v_4]; (void)p; }
static void t_two_past(void) { int a[4] = {1, 2, 3, 4}; volatile int *p = &a[v_5]; (void)p; }
static void ok_flex_tail(void) {
    char raw[sizeof(struct Rec) + 3 * sizeof(int)];
    memset(raw, 0, sizeof(raw));
    struct Rec *r = (struct Rec *)raw;
    volatile int x = r->flex[2];
    (void)x;
}
static void t_null_load(void) { volatile int r = *vp_null; (void)r; }
static void ok_addr_deref_null(void) { volatile int *p = &*vp_null; (void)p; }
static void ok_addr_sub_null(void) { volatile int *p = &vp_null[2]; (void)p; }
static void t_addr_member_null(void) { volatile int *p = (volatile int *)&vrec_null->val; (void)p; }
static void t_bool_load(void) {
    bool b;
    unsigned char bad = 3;
    memcpy(&b, &bad, 1);
    volatile int x = b;
    (void)x;
}
static void t_vla_bound(void) {
    int n = v_zero;
    int v[n];
    v[0] = 1;
    (void)v[0];
}
static void t_fdiv0(void) { volatile double r = 3.5 / vd_zero; (void)r; }

__attribute__((no_sanitize("undefined")))
static int allow_wrap(int x) { return x + 1; }

int main(void) {
    if (!trapped(t_sadd)) return 1;
    if (!trapped(t_ssub)) return 2;
    if (!trapped(t_smul)) return 3;
    if (!trapped(t_sneg)) return 4;
    if (!trapped(t_idiv0)) return 5;
    if (!trapped(t_imin_div)) return 6;
    if (!trapped(t_shl_exp)) return 7;
    if (!trapped(t_shl_base)) return 8;
    if (trapped(ok_shr_neg)) return 9;
    if (!trapped(t_bounds)) return 10;
    if (trapped(ok_one_past)) return 11;
    if (!trapped(t_two_past)) return 12;
    if (trapped(ok_flex_tail)) return 13;
    if (!trapped(t_null_load)) return 14;
    if (trapped(ok_addr_deref_null)) return 15;
    if (trapped(ok_addr_sub_null)) return 16;
    if (!trapped(t_addr_member_null)) return 17;
    if (!trapped(t_bool_load)) return 18;
    if (!trapped(t_vla_bound)) return 19;
    if (!trapped(t_fdiv0)) return 20;
    if (offsetof(struct Rec, val) != 4) return 21;
    (void)allow_wrap(v_max);
    puts("ubsan-trap-ok");
    return 0;
}
