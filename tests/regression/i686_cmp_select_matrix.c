/* i686 fused Cmp->Select matrix (emit_fused_cmp_select_impl): every
 * predicate x width x signedness x operand shape (register pair, immediate,
 * memory lhs, constant lhs), with int, long long and double arms, over the
 * boundary values of each type.  Each function folds its results into a
 * hash; stdout is compared byte-for-byte with GCC.  The exhaustive
 * unrolled variant (64152 checks) is artifacts/pr833/csel_stress.py.  */
#include <stdio.h>

static const signed char v_sc[] = {-128, -127, -1, 0, 1, 126, 127};
__attribute__((noinline)) static int rr_sc_eq(signed char a, signed char b) { return a == b ? 11 : -7; }
__attribute__((noinline)) static long long ri_sc_eq(signed char a) { return a == (signed char)(126) ? 0x100000003LL : -5LL; }
__attribute__((noinline)) static double mr_sc_eq(const signed char *p, signed char b) { return *p == b ? 1.5 : -2.25; }
__attribute__((noinline)) static int ir_sc_eq(signed char b) { return (signed char)0 == b ? 3 : 4; }
__attribute__((noinline)) static int rr_sc_ne(signed char a, signed char b) { return a != b ? 11 : -7; }
__attribute__((noinline)) static long long ri_sc_ne(signed char a) { return a != (signed char)(126) ? 0x100000003LL : -5LL; }
__attribute__((noinline)) static double mr_sc_ne(const signed char *p, signed char b) { return *p != b ? 1.5 : -2.25; }
__attribute__((noinline)) static int ir_sc_ne(signed char b) { return (signed char)0 != b ? 3 : 4; }
__attribute__((noinline)) static int rr_sc_lt(signed char a, signed char b) { return a < b ? 11 : -7; }
__attribute__((noinline)) static long long ri_sc_lt(signed char a) { return a < (signed char)(126) ? 0x100000003LL : -5LL; }
__attribute__((noinline)) static double mr_sc_lt(const signed char *p, signed char b) { return *p < b ? 1.5 : -2.25; }
__attribute__((noinline)) static int ir_sc_lt(signed char b) { return (signed char)0 < b ? 3 : 4; }
__attribute__((noinline)) static int rr_sc_le(signed char a, signed char b) { return a <= b ? 11 : -7; }
__attribute__((noinline)) static long long ri_sc_le(signed char a) { return a <= (signed char)(126) ? 0x100000003LL : -5LL; }
__attribute__((noinline)) static double mr_sc_le(const signed char *p, signed char b) { return *p <= b ? 1.5 : -2.25; }
__attribute__((noinline)) static int ir_sc_le(signed char b) { return (signed char)0 <= b ? 3 : 4; }
__attribute__((noinline)) static int rr_sc_gt(signed char a, signed char b) { return a > b ? 11 : -7; }
__attribute__((noinline)) static long long ri_sc_gt(signed char a) { return a > (signed char)(126) ? 0x100000003LL : -5LL; }
__attribute__((noinline)) static double mr_sc_gt(const signed char *p, signed char b) { return *p > b ? 1.5 : -2.25; }
__attribute__((noinline)) static int ir_sc_gt(signed char b) { return (signed char)0 > b ? 3 : 4; }
__attribute__((noinline)) static int rr_sc_ge(signed char a, signed char b) { return a >= b ? 11 : -7; }
__attribute__((noinline)) static long long ri_sc_ge(signed char a) { return a >= (signed char)(126) ? 0x100000003LL : -5LL; }
__attribute__((noinline)) static double mr_sc_ge(const signed char *p, signed char b) { return *p >= b ? 1.5 : -2.25; }
__attribute__((noinline)) static int ir_sc_ge(signed char b) { return (signed char)0 >= b ? 3 : 4; }
static const unsigned char v_uc[] = {0, 1, 127, 128, 254, 255};
__attribute__((noinline)) static int rr_uc_eq(unsigned char a, unsigned char b) { return a == b ? 11 : -7; }
__attribute__((noinline)) static long long ri_uc_eq(unsigned char a) { return a == (unsigned char)(254) ? 0x100000003LL : -5LL; }
__attribute__((noinline)) static double mr_uc_eq(const unsigned char *p, unsigned char b) { return *p == b ? 1.5 : -2.25; }
__attribute__((noinline)) static int ir_uc_eq(unsigned char b) { return (unsigned char)0 == b ? 3 : 4; }
__attribute__((noinline)) static int rr_uc_ne(unsigned char a, unsigned char b) { return a != b ? 11 : -7; }
__attribute__((noinline)) static long long ri_uc_ne(unsigned char a) { return a != (unsigned char)(254) ? 0x100000003LL : -5LL; }
__attribute__((noinline)) static double mr_uc_ne(const unsigned char *p, unsigned char b) { return *p != b ? 1.5 : -2.25; }
__attribute__((noinline)) static int ir_uc_ne(unsigned char b) { return (unsigned char)0 != b ? 3 : 4; }
__attribute__((noinline)) static int rr_uc_lt(unsigned char a, unsigned char b) { return a < b ? 11 : -7; }
__attribute__((noinline)) static long long ri_uc_lt(unsigned char a) { return a < (unsigned char)(254) ? 0x100000003LL : -5LL; }
__attribute__((noinline)) static double mr_uc_lt(const unsigned char *p, unsigned char b) { return *p < b ? 1.5 : -2.25; }
__attribute__((noinline)) static int ir_uc_lt(unsigned char b) { return (unsigned char)0 < b ? 3 : 4; }
__attribute__((noinline)) static int rr_uc_le(unsigned char a, unsigned char b) { return a <= b ? 11 : -7; }
__attribute__((noinline)) static long long ri_uc_le(unsigned char a) { return a <= (unsigned char)(254) ? 0x100000003LL : -5LL; }
__attribute__((noinline)) static double mr_uc_le(const unsigned char *p, unsigned char b) { return *p <= b ? 1.5 : -2.25; }
__attribute__((noinline)) static int ir_uc_le(unsigned char b) { return (unsigned char)0 <= b ? 3 : 4; }
__attribute__((noinline)) static int rr_uc_gt(unsigned char a, unsigned char b) { return a > b ? 11 : -7; }
__attribute__((noinline)) static long long ri_uc_gt(unsigned char a) { return a > (unsigned char)(254) ? 0x100000003LL : -5LL; }
__attribute__((noinline)) static double mr_uc_gt(const unsigned char *p, unsigned char b) { return *p > b ? 1.5 : -2.25; }
__attribute__((noinline)) static int ir_uc_gt(unsigned char b) { return (unsigned char)0 > b ? 3 : 4; }
__attribute__((noinline)) static int rr_uc_ge(unsigned char a, unsigned char b) { return a >= b ? 11 : -7; }
__attribute__((noinline)) static long long ri_uc_ge(unsigned char a) { return a >= (unsigned char)(254) ? 0x100000003LL : -5LL; }
__attribute__((noinline)) static double mr_uc_ge(const unsigned char *p, unsigned char b) { return *p >= b ? 1.5 : -2.25; }
__attribute__((noinline)) static int ir_uc_ge(unsigned char b) { return (unsigned char)0 >= b ? 3 : 4; }
static const short v_ss[] = {-32768, -256, -1, 0, 1, 255, 256, 32767};
__attribute__((noinline)) static int rr_ss_eq(short a, short b) { return a == b ? 11 : -7; }
__attribute__((noinline)) static long long ri_ss_eq(short a) { return a == (short)(256) ? 0x100000003LL : -5LL; }
__attribute__((noinline)) static double mr_ss_eq(const short *p, short b) { return *p == b ? 1.5 : -2.25; }
__attribute__((noinline)) static int ir_ss_eq(short b) { return (short)0 == b ? 3 : 4; }
__attribute__((noinline)) static int rr_ss_ne(short a, short b) { return a != b ? 11 : -7; }
__attribute__((noinline)) static long long ri_ss_ne(short a) { return a != (short)(256) ? 0x100000003LL : -5LL; }
__attribute__((noinline)) static double mr_ss_ne(const short *p, short b) { return *p != b ? 1.5 : -2.25; }
__attribute__((noinline)) static int ir_ss_ne(short b) { return (short)0 != b ? 3 : 4; }
__attribute__((noinline)) static int rr_ss_lt(short a, short b) { return a < b ? 11 : -7; }
__attribute__((noinline)) static long long ri_ss_lt(short a) { return a < (short)(256) ? 0x100000003LL : -5LL; }
__attribute__((noinline)) static double mr_ss_lt(const short *p, short b) { return *p < b ? 1.5 : -2.25; }
__attribute__((noinline)) static int ir_ss_lt(short b) { return (short)0 < b ? 3 : 4; }
__attribute__((noinline)) static int rr_ss_le(short a, short b) { return a <= b ? 11 : -7; }
__attribute__((noinline)) static long long ri_ss_le(short a) { return a <= (short)(256) ? 0x100000003LL : -5LL; }
__attribute__((noinline)) static double mr_ss_le(const short *p, short b) { return *p <= b ? 1.5 : -2.25; }
__attribute__((noinline)) static int ir_ss_le(short b) { return (short)0 <= b ? 3 : 4; }
__attribute__((noinline)) static int rr_ss_gt(short a, short b) { return a > b ? 11 : -7; }
__attribute__((noinline)) static long long ri_ss_gt(short a) { return a > (short)(256) ? 0x100000003LL : -5LL; }
__attribute__((noinline)) static double mr_ss_gt(const short *p, short b) { return *p > b ? 1.5 : -2.25; }
__attribute__((noinline)) static int ir_ss_gt(short b) { return (short)0 > b ? 3 : 4; }
__attribute__((noinline)) static int rr_ss_ge(short a, short b) { return a >= b ? 11 : -7; }
__attribute__((noinline)) static long long ri_ss_ge(short a) { return a >= (short)(256) ? 0x100000003LL : -5LL; }
__attribute__((noinline)) static double mr_ss_ge(const short *p, short b) { return *p >= b ? 1.5 : -2.25; }
__attribute__((noinline)) static int ir_ss_ge(short b) { return (short)0 >= b ? 3 : 4; }
static const unsigned short v_us[] = {0, 1, 255, 256, 32768, 65535};
__attribute__((noinline)) static int rr_us_eq(unsigned short a, unsigned short b) { return a == b ? 11 : -7; }
__attribute__((noinline)) static long long ri_us_eq(unsigned short a) { return a == (unsigned short)(32768) ? 0x100000003LL : -5LL; }
__attribute__((noinline)) static double mr_us_eq(const unsigned short *p, unsigned short b) { return *p == b ? 1.5 : -2.25; }
__attribute__((noinline)) static int ir_us_eq(unsigned short b) { return (unsigned short)0 == b ? 3 : 4; }
__attribute__((noinline)) static int rr_us_ne(unsigned short a, unsigned short b) { return a != b ? 11 : -7; }
__attribute__((noinline)) static long long ri_us_ne(unsigned short a) { return a != (unsigned short)(32768) ? 0x100000003LL : -5LL; }
__attribute__((noinline)) static double mr_us_ne(const unsigned short *p, unsigned short b) { return *p != b ? 1.5 : -2.25; }
__attribute__((noinline)) static int ir_us_ne(unsigned short b) { return (unsigned short)0 != b ? 3 : 4; }
__attribute__((noinline)) static int rr_us_lt(unsigned short a, unsigned short b) { return a < b ? 11 : -7; }
__attribute__((noinline)) static long long ri_us_lt(unsigned short a) { return a < (unsigned short)(32768) ? 0x100000003LL : -5LL; }
__attribute__((noinline)) static double mr_us_lt(const unsigned short *p, unsigned short b) { return *p < b ? 1.5 : -2.25; }
__attribute__((noinline)) static int ir_us_lt(unsigned short b) { return (unsigned short)0 < b ? 3 : 4; }
__attribute__((noinline)) static int rr_us_le(unsigned short a, unsigned short b) { return a <= b ? 11 : -7; }
__attribute__((noinline)) static long long ri_us_le(unsigned short a) { return a <= (unsigned short)(32768) ? 0x100000003LL : -5LL; }
__attribute__((noinline)) static double mr_us_le(const unsigned short *p, unsigned short b) { return *p <= b ? 1.5 : -2.25; }
__attribute__((noinline)) static int ir_us_le(unsigned short b) { return (unsigned short)0 <= b ? 3 : 4; }
__attribute__((noinline)) static int rr_us_gt(unsigned short a, unsigned short b) { return a > b ? 11 : -7; }
__attribute__((noinline)) static long long ri_us_gt(unsigned short a) { return a > (unsigned short)(32768) ? 0x100000003LL : -5LL; }
__attribute__((noinline)) static double mr_us_gt(const unsigned short *p, unsigned short b) { return *p > b ? 1.5 : -2.25; }
__attribute__((noinline)) static int ir_us_gt(unsigned short b) { return (unsigned short)0 > b ? 3 : 4; }
__attribute__((noinline)) static int rr_us_ge(unsigned short a, unsigned short b) { return a >= b ? 11 : -7; }
__attribute__((noinline)) static long long ri_us_ge(unsigned short a) { return a >= (unsigned short)(32768) ? 0x100000003LL : -5LL; }
__attribute__((noinline)) static double mr_us_ge(const unsigned short *p, unsigned short b) { return *p >= b ? 1.5 : -2.25; }
__attribute__((noinline)) static int ir_us_ge(unsigned short b) { return (unsigned short)0 >= b ? 3 : 4; }
static const int v_si[] = {-2147483648, -1, 0, 1, 2147483647};
__attribute__((noinline)) static int rr_si_eq(int a, int b) { return a == b ? 11 : -7; }
__attribute__((noinline)) static long long ri_si_eq(int a) { return a == (int)(1) ? 0x100000003LL : -5LL; }
__attribute__((noinline)) static double mr_si_eq(const int *p, int b) { return *p == b ? 1.5 : -2.25; }
__attribute__((noinline)) static int ir_si_eq(int b) { return (int)0 == b ? 3 : 4; }
__attribute__((noinline)) static int rr_si_ne(int a, int b) { return a != b ? 11 : -7; }
__attribute__((noinline)) static long long ri_si_ne(int a) { return a != (int)(1) ? 0x100000003LL : -5LL; }
__attribute__((noinline)) static double mr_si_ne(const int *p, int b) { return *p != b ? 1.5 : -2.25; }
__attribute__((noinline)) static int ir_si_ne(int b) { return (int)0 != b ? 3 : 4; }
__attribute__((noinline)) static int rr_si_lt(int a, int b) { return a < b ? 11 : -7; }
__attribute__((noinline)) static long long ri_si_lt(int a) { return a < (int)(1) ? 0x100000003LL : -5LL; }
__attribute__((noinline)) static double mr_si_lt(const int *p, int b) { return *p < b ? 1.5 : -2.25; }
__attribute__((noinline)) static int ir_si_lt(int b) { return (int)0 < b ? 3 : 4; }
__attribute__((noinline)) static int rr_si_le(int a, int b) { return a <= b ? 11 : -7; }
__attribute__((noinline)) static long long ri_si_le(int a) { return a <= (int)(1) ? 0x100000003LL : -5LL; }
__attribute__((noinline)) static double mr_si_le(const int *p, int b) { return *p <= b ? 1.5 : -2.25; }
__attribute__((noinline)) static int ir_si_le(int b) { return (int)0 <= b ? 3 : 4; }
__attribute__((noinline)) static int rr_si_gt(int a, int b) { return a > b ? 11 : -7; }
__attribute__((noinline)) static long long ri_si_gt(int a) { return a > (int)(1) ? 0x100000003LL : -5LL; }
__attribute__((noinline)) static double mr_si_gt(const int *p, int b) { return *p > b ? 1.5 : -2.25; }
__attribute__((noinline)) static int ir_si_gt(int b) { return (int)0 > b ? 3 : 4; }
__attribute__((noinline)) static int rr_si_ge(int a, int b) { return a >= b ? 11 : -7; }
__attribute__((noinline)) static long long ri_si_ge(int a) { return a >= (int)(1) ? 0x100000003LL : -5LL; }
__attribute__((noinline)) static double mr_si_ge(const int *p, int b) { return *p >= b ? 1.5 : -2.25; }
__attribute__((noinline)) static int ir_si_ge(int b) { return (int)0 >= b ? 3 : 4; }
static const unsigned v_ui[] = {0, 1, 2147483647, 2147483648u, 4294967295u};
__attribute__((noinline)) static int rr_ui_eq(unsigned a, unsigned b) { return a == b ? 11 : -7; }
__attribute__((noinline)) static long long ri_ui_eq(unsigned a) { return a == (unsigned)(2147483648u) ? 0x100000003LL : -5LL; }
__attribute__((noinline)) static double mr_ui_eq(const unsigned *p, unsigned b) { return *p == b ? 1.5 : -2.25; }
__attribute__((noinline)) static int ir_ui_eq(unsigned b) { return (unsigned)0 == b ? 3 : 4; }
__attribute__((noinline)) static int rr_ui_ne(unsigned a, unsigned b) { return a != b ? 11 : -7; }
__attribute__((noinline)) static long long ri_ui_ne(unsigned a) { return a != (unsigned)(2147483648u) ? 0x100000003LL : -5LL; }
__attribute__((noinline)) static double mr_ui_ne(const unsigned *p, unsigned b) { return *p != b ? 1.5 : -2.25; }
__attribute__((noinline)) static int ir_ui_ne(unsigned b) { return (unsigned)0 != b ? 3 : 4; }
__attribute__((noinline)) static int rr_ui_lt(unsigned a, unsigned b) { return a < b ? 11 : -7; }
__attribute__((noinline)) static long long ri_ui_lt(unsigned a) { return a < (unsigned)(2147483648u) ? 0x100000003LL : -5LL; }
__attribute__((noinline)) static double mr_ui_lt(const unsigned *p, unsigned b) { return *p < b ? 1.5 : -2.25; }
__attribute__((noinline)) static int ir_ui_lt(unsigned b) { return (unsigned)0 < b ? 3 : 4; }
__attribute__((noinline)) static int rr_ui_le(unsigned a, unsigned b) { return a <= b ? 11 : -7; }
__attribute__((noinline)) static long long ri_ui_le(unsigned a) { return a <= (unsigned)(2147483648u) ? 0x100000003LL : -5LL; }
__attribute__((noinline)) static double mr_ui_le(const unsigned *p, unsigned b) { return *p <= b ? 1.5 : -2.25; }
__attribute__((noinline)) static int ir_ui_le(unsigned b) { return (unsigned)0 <= b ? 3 : 4; }
__attribute__((noinline)) static int rr_ui_gt(unsigned a, unsigned b) { return a > b ? 11 : -7; }
__attribute__((noinline)) static long long ri_ui_gt(unsigned a) { return a > (unsigned)(2147483648u) ? 0x100000003LL : -5LL; }
__attribute__((noinline)) static double mr_ui_gt(const unsigned *p, unsigned b) { return *p > b ? 1.5 : -2.25; }
__attribute__((noinline)) static int ir_ui_gt(unsigned b) { return (unsigned)0 > b ? 3 : 4; }
__attribute__((noinline)) static int rr_ui_ge(unsigned a, unsigned b) { return a >= b ? 11 : -7; }
__attribute__((noinline)) static long long ri_ui_ge(unsigned a) { return a >= (unsigned)(2147483648u) ? 0x100000003LL : -5LL; }
__attribute__((noinline)) static double mr_ui_ge(const unsigned *p, unsigned b) { return *p >= b ? 1.5 : -2.25; }
__attribute__((noinline)) static int ir_ui_ge(unsigned b) { return (unsigned)0 >= b ? 3 : 4; }

int main(void) {
    unsigned long long h = 1469598103934665603ULL, all = 0;
    for (int i = 0; i < 7; i++) for (int j = 0; j < 7; j++) {
        h = (h ^ (unsigned)rr_sc_eq(v_sc[i], v_sc[j])) * 1099511628211ULL;
        h = (h ^ (unsigned long long)ri_sc_eq(v_sc[i])) * 1099511628211ULL;
        h = (h ^ (unsigned long long)(long long)(4 * mr_sc_eq(&v_sc[i], v_sc[j]))) * 1099511628211ULL;
        h = (h ^ (unsigned)ir_sc_eq(v_sc[j])) * 1099511628211ULL;
    }
    printf("sc_eq %016llx\n", h); all ^= h;
    for (int i = 0; i < 7; i++) for (int j = 0; j < 7; j++) {
        h = (h ^ (unsigned)rr_sc_ne(v_sc[i], v_sc[j])) * 1099511628211ULL;
        h = (h ^ (unsigned long long)ri_sc_ne(v_sc[i])) * 1099511628211ULL;
        h = (h ^ (unsigned long long)(long long)(4 * mr_sc_ne(&v_sc[i], v_sc[j]))) * 1099511628211ULL;
        h = (h ^ (unsigned)ir_sc_ne(v_sc[j])) * 1099511628211ULL;
    }
    printf("sc_ne %016llx\n", h); all ^= h;
    for (int i = 0; i < 7; i++) for (int j = 0; j < 7; j++) {
        h = (h ^ (unsigned)rr_sc_lt(v_sc[i], v_sc[j])) * 1099511628211ULL;
        h = (h ^ (unsigned long long)ri_sc_lt(v_sc[i])) * 1099511628211ULL;
        h = (h ^ (unsigned long long)(long long)(4 * mr_sc_lt(&v_sc[i], v_sc[j]))) * 1099511628211ULL;
        h = (h ^ (unsigned)ir_sc_lt(v_sc[j])) * 1099511628211ULL;
    }
    printf("sc_lt %016llx\n", h); all ^= h;
    for (int i = 0; i < 7; i++) for (int j = 0; j < 7; j++) {
        h = (h ^ (unsigned)rr_sc_le(v_sc[i], v_sc[j])) * 1099511628211ULL;
        h = (h ^ (unsigned long long)ri_sc_le(v_sc[i])) * 1099511628211ULL;
        h = (h ^ (unsigned long long)(long long)(4 * mr_sc_le(&v_sc[i], v_sc[j]))) * 1099511628211ULL;
        h = (h ^ (unsigned)ir_sc_le(v_sc[j])) * 1099511628211ULL;
    }
    printf("sc_le %016llx\n", h); all ^= h;
    for (int i = 0; i < 7; i++) for (int j = 0; j < 7; j++) {
        h = (h ^ (unsigned)rr_sc_gt(v_sc[i], v_sc[j])) * 1099511628211ULL;
        h = (h ^ (unsigned long long)ri_sc_gt(v_sc[i])) * 1099511628211ULL;
        h = (h ^ (unsigned long long)(long long)(4 * mr_sc_gt(&v_sc[i], v_sc[j]))) * 1099511628211ULL;
        h = (h ^ (unsigned)ir_sc_gt(v_sc[j])) * 1099511628211ULL;
    }
    printf("sc_gt %016llx\n", h); all ^= h;
    for (int i = 0; i < 7; i++) for (int j = 0; j < 7; j++) {
        h = (h ^ (unsigned)rr_sc_ge(v_sc[i], v_sc[j])) * 1099511628211ULL;
        h = (h ^ (unsigned long long)ri_sc_ge(v_sc[i])) * 1099511628211ULL;
        h = (h ^ (unsigned long long)(long long)(4 * mr_sc_ge(&v_sc[i], v_sc[j]))) * 1099511628211ULL;
        h = (h ^ (unsigned)ir_sc_ge(v_sc[j])) * 1099511628211ULL;
    }
    printf("sc_ge %016llx\n", h); all ^= h;
    for (int i = 0; i < 6; i++) for (int j = 0; j < 6; j++) {
        h = (h ^ (unsigned)rr_uc_eq(v_uc[i], v_uc[j])) * 1099511628211ULL;
        h = (h ^ (unsigned long long)ri_uc_eq(v_uc[i])) * 1099511628211ULL;
        h = (h ^ (unsigned long long)(long long)(4 * mr_uc_eq(&v_uc[i], v_uc[j]))) * 1099511628211ULL;
        h = (h ^ (unsigned)ir_uc_eq(v_uc[j])) * 1099511628211ULL;
    }
    printf("uc_eq %016llx\n", h); all ^= h;
    for (int i = 0; i < 6; i++) for (int j = 0; j < 6; j++) {
        h = (h ^ (unsigned)rr_uc_ne(v_uc[i], v_uc[j])) * 1099511628211ULL;
        h = (h ^ (unsigned long long)ri_uc_ne(v_uc[i])) * 1099511628211ULL;
        h = (h ^ (unsigned long long)(long long)(4 * mr_uc_ne(&v_uc[i], v_uc[j]))) * 1099511628211ULL;
        h = (h ^ (unsigned)ir_uc_ne(v_uc[j])) * 1099511628211ULL;
    }
    printf("uc_ne %016llx\n", h); all ^= h;
    for (int i = 0; i < 6; i++) for (int j = 0; j < 6; j++) {
        h = (h ^ (unsigned)rr_uc_lt(v_uc[i], v_uc[j])) * 1099511628211ULL;
        h = (h ^ (unsigned long long)ri_uc_lt(v_uc[i])) * 1099511628211ULL;
        h = (h ^ (unsigned long long)(long long)(4 * mr_uc_lt(&v_uc[i], v_uc[j]))) * 1099511628211ULL;
        h = (h ^ (unsigned)ir_uc_lt(v_uc[j])) * 1099511628211ULL;
    }
    printf("uc_lt %016llx\n", h); all ^= h;
    for (int i = 0; i < 6; i++) for (int j = 0; j < 6; j++) {
        h = (h ^ (unsigned)rr_uc_le(v_uc[i], v_uc[j])) * 1099511628211ULL;
        h = (h ^ (unsigned long long)ri_uc_le(v_uc[i])) * 1099511628211ULL;
        h = (h ^ (unsigned long long)(long long)(4 * mr_uc_le(&v_uc[i], v_uc[j]))) * 1099511628211ULL;
        h = (h ^ (unsigned)ir_uc_le(v_uc[j])) * 1099511628211ULL;
    }
    printf("uc_le %016llx\n", h); all ^= h;
    for (int i = 0; i < 6; i++) for (int j = 0; j < 6; j++) {
        h = (h ^ (unsigned)rr_uc_gt(v_uc[i], v_uc[j])) * 1099511628211ULL;
        h = (h ^ (unsigned long long)ri_uc_gt(v_uc[i])) * 1099511628211ULL;
        h = (h ^ (unsigned long long)(long long)(4 * mr_uc_gt(&v_uc[i], v_uc[j]))) * 1099511628211ULL;
        h = (h ^ (unsigned)ir_uc_gt(v_uc[j])) * 1099511628211ULL;
    }
    printf("uc_gt %016llx\n", h); all ^= h;
    for (int i = 0; i < 6; i++) for (int j = 0; j < 6; j++) {
        h = (h ^ (unsigned)rr_uc_ge(v_uc[i], v_uc[j])) * 1099511628211ULL;
        h = (h ^ (unsigned long long)ri_uc_ge(v_uc[i])) * 1099511628211ULL;
        h = (h ^ (unsigned long long)(long long)(4 * mr_uc_ge(&v_uc[i], v_uc[j]))) * 1099511628211ULL;
        h = (h ^ (unsigned)ir_uc_ge(v_uc[j])) * 1099511628211ULL;
    }
    printf("uc_ge %016llx\n", h); all ^= h;
    for (int i = 0; i < 8; i++) for (int j = 0; j < 8; j++) {
        h = (h ^ (unsigned)rr_ss_eq(v_ss[i], v_ss[j])) * 1099511628211ULL;
        h = (h ^ (unsigned long long)ri_ss_eq(v_ss[i])) * 1099511628211ULL;
        h = (h ^ (unsigned long long)(long long)(4 * mr_ss_eq(&v_ss[i], v_ss[j]))) * 1099511628211ULL;
        h = (h ^ (unsigned)ir_ss_eq(v_ss[j])) * 1099511628211ULL;
    }
    printf("ss_eq %016llx\n", h); all ^= h;
    for (int i = 0; i < 8; i++) for (int j = 0; j < 8; j++) {
        h = (h ^ (unsigned)rr_ss_ne(v_ss[i], v_ss[j])) * 1099511628211ULL;
        h = (h ^ (unsigned long long)ri_ss_ne(v_ss[i])) * 1099511628211ULL;
        h = (h ^ (unsigned long long)(long long)(4 * mr_ss_ne(&v_ss[i], v_ss[j]))) * 1099511628211ULL;
        h = (h ^ (unsigned)ir_ss_ne(v_ss[j])) * 1099511628211ULL;
    }
    printf("ss_ne %016llx\n", h); all ^= h;
    for (int i = 0; i < 8; i++) for (int j = 0; j < 8; j++) {
        h = (h ^ (unsigned)rr_ss_lt(v_ss[i], v_ss[j])) * 1099511628211ULL;
        h = (h ^ (unsigned long long)ri_ss_lt(v_ss[i])) * 1099511628211ULL;
        h = (h ^ (unsigned long long)(long long)(4 * mr_ss_lt(&v_ss[i], v_ss[j]))) * 1099511628211ULL;
        h = (h ^ (unsigned)ir_ss_lt(v_ss[j])) * 1099511628211ULL;
    }
    printf("ss_lt %016llx\n", h); all ^= h;
    for (int i = 0; i < 8; i++) for (int j = 0; j < 8; j++) {
        h = (h ^ (unsigned)rr_ss_le(v_ss[i], v_ss[j])) * 1099511628211ULL;
        h = (h ^ (unsigned long long)ri_ss_le(v_ss[i])) * 1099511628211ULL;
        h = (h ^ (unsigned long long)(long long)(4 * mr_ss_le(&v_ss[i], v_ss[j]))) * 1099511628211ULL;
        h = (h ^ (unsigned)ir_ss_le(v_ss[j])) * 1099511628211ULL;
    }
    printf("ss_le %016llx\n", h); all ^= h;
    for (int i = 0; i < 8; i++) for (int j = 0; j < 8; j++) {
        h = (h ^ (unsigned)rr_ss_gt(v_ss[i], v_ss[j])) * 1099511628211ULL;
        h = (h ^ (unsigned long long)ri_ss_gt(v_ss[i])) * 1099511628211ULL;
        h = (h ^ (unsigned long long)(long long)(4 * mr_ss_gt(&v_ss[i], v_ss[j]))) * 1099511628211ULL;
        h = (h ^ (unsigned)ir_ss_gt(v_ss[j])) * 1099511628211ULL;
    }
    printf("ss_gt %016llx\n", h); all ^= h;
    for (int i = 0; i < 8; i++) for (int j = 0; j < 8; j++) {
        h = (h ^ (unsigned)rr_ss_ge(v_ss[i], v_ss[j])) * 1099511628211ULL;
        h = (h ^ (unsigned long long)ri_ss_ge(v_ss[i])) * 1099511628211ULL;
        h = (h ^ (unsigned long long)(long long)(4 * mr_ss_ge(&v_ss[i], v_ss[j]))) * 1099511628211ULL;
        h = (h ^ (unsigned)ir_ss_ge(v_ss[j])) * 1099511628211ULL;
    }
    printf("ss_ge %016llx\n", h); all ^= h;
    for (int i = 0; i < 6; i++) for (int j = 0; j < 6; j++) {
        h = (h ^ (unsigned)rr_us_eq(v_us[i], v_us[j])) * 1099511628211ULL;
        h = (h ^ (unsigned long long)ri_us_eq(v_us[i])) * 1099511628211ULL;
        h = (h ^ (unsigned long long)(long long)(4 * mr_us_eq(&v_us[i], v_us[j]))) * 1099511628211ULL;
        h = (h ^ (unsigned)ir_us_eq(v_us[j])) * 1099511628211ULL;
    }
    printf("us_eq %016llx\n", h); all ^= h;
    for (int i = 0; i < 6; i++) for (int j = 0; j < 6; j++) {
        h = (h ^ (unsigned)rr_us_ne(v_us[i], v_us[j])) * 1099511628211ULL;
        h = (h ^ (unsigned long long)ri_us_ne(v_us[i])) * 1099511628211ULL;
        h = (h ^ (unsigned long long)(long long)(4 * mr_us_ne(&v_us[i], v_us[j]))) * 1099511628211ULL;
        h = (h ^ (unsigned)ir_us_ne(v_us[j])) * 1099511628211ULL;
    }
    printf("us_ne %016llx\n", h); all ^= h;
    for (int i = 0; i < 6; i++) for (int j = 0; j < 6; j++) {
        h = (h ^ (unsigned)rr_us_lt(v_us[i], v_us[j])) * 1099511628211ULL;
        h = (h ^ (unsigned long long)ri_us_lt(v_us[i])) * 1099511628211ULL;
        h = (h ^ (unsigned long long)(long long)(4 * mr_us_lt(&v_us[i], v_us[j]))) * 1099511628211ULL;
        h = (h ^ (unsigned)ir_us_lt(v_us[j])) * 1099511628211ULL;
    }
    printf("us_lt %016llx\n", h); all ^= h;
    for (int i = 0; i < 6; i++) for (int j = 0; j < 6; j++) {
        h = (h ^ (unsigned)rr_us_le(v_us[i], v_us[j])) * 1099511628211ULL;
        h = (h ^ (unsigned long long)ri_us_le(v_us[i])) * 1099511628211ULL;
        h = (h ^ (unsigned long long)(long long)(4 * mr_us_le(&v_us[i], v_us[j]))) * 1099511628211ULL;
        h = (h ^ (unsigned)ir_us_le(v_us[j])) * 1099511628211ULL;
    }
    printf("us_le %016llx\n", h); all ^= h;
    for (int i = 0; i < 6; i++) for (int j = 0; j < 6; j++) {
        h = (h ^ (unsigned)rr_us_gt(v_us[i], v_us[j])) * 1099511628211ULL;
        h = (h ^ (unsigned long long)ri_us_gt(v_us[i])) * 1099511628211ULL;
        h = (h ^ (unsigned long long)(long long)(4 * mr_us_gt(&v_us[i], v_us[j]))) * 1099511628211ULL;
        h = (h ^ (unsigned)ir_us_gt(v_us[j])) * 1099511628211ULL;
    }
    printf("us_gt %016llx\n", h); all ^= h;
    for (int i = 0; i < 6; i++) for (int j = 0; j < 6; j++) {
        h = (h ^ (unsigned)rr_us_ge(v_us[i], v_us[j])) * 1099511628211ULL;
        h = (h ^ (unsigned long long)ri_us_ge(v_us[i])) * 1099511628211ULL;
        h = (h ^ (unsigned long long)(long long)(4 * mr_us_ge(&v_us[i], v_us[j]))) * 1099511628211ULL;
        h = (h ^ (unsigned)ir_us_ge(v_us[j])) * 1099511628211ULL;
    }
    printf("us_ge %016llx\n", h); all ^= h;
    for (int i = 0; i < 5; i++) for (int j = 0; j < 5; j++) {
        h = (h ^ (unsigned)rr_si_eq(v_si[i], v_si[j])) * 1099511628211ULL;
        h = (h ^ (unsigned long long)ri_si_eq(v_si[i])) * 1099511628211ULL;
        h = (h ^ (unsigned long long)(long long)(4 * mr_si_eq(&v_si[i], v_si[j]))) * 1099511628211ULL;
        h = (h ^ (unsigned)ir_si_eq(v_si[j])) * 1099511628211ULL;
    }
    printf("si_eq %016llx\n", h); all ^= h;
    for (int i = 0; i < 5; i++) for (int j = 0; j < 5; j++) {
        h = (h ^ (unsigned)rr_si_ne(v_si[i], v_si[j])) * 1099511628211ULL;
        h = (h ^ (unsigned long long)ri_si_ne(v_si[i])) * 1099511628211ULL;
        h = (h ^ (unsigned long long)(long long)(4 * mr_si_ne(&v_si[i], v_si[j]))) * 1099511628211ULL;
        h = (h ^ (unsigned)ir_si_ne(v_si[j])) * 1099511628211ULL;
    }
    printf("si_ne %016llx\n", h); all ^= h;
    for (int i = 0; i < 5; i++) for (int j = 0; j < 5; j++) {
        h = (h ^ (unsigned)rr_si_lt(v_si[i], v_si[j])) * 1099511628211ULL;
        h = (h ^ (unsigned long long)ri_si_lt(v_si[i])) * 1099511628211ULL;
        h = (h ^ (unsigned long long)(long long)(4 * mr_si_lt(&v_si[i], v_si[j]))) * 1099511628211ULL;
        h = (h ^ (unsigned)ir_si_lt(v_si[j])) * 1099511628211ULL;
    }
    printf("si_lt %016llx\n", h); all ^= h;
    for (int i = 0; i < 5; i++) for (int j = 0; j < 5; j++) {
        h = (h ^ (unsigned)rr_si_le(v_si[i], v_si[j])) * 1099511628211ULL;
        h = (h ^ (unsigned long long)ri_si_le(v_si[i])) * 1099511628211ULL;
        h = (h ^ (unsigned long long)(long long)(4 * mr_si_le(&v_si[i], v_si[j]))) * 1099511628211ULL;
        h = (h ^ (unsigned)ir_si_le(v_si[j])) * 1099511628211ULL;
    }
    printf("si_le %016llx\n", h); all ^= h;
    for (int i = 0; i < 5; i++) for (int j = 0; j < 5; j++) {
        h = (h ^ (unsigned)rr_si_gt(v_si[i], v_si[j])) * 1099511628211ULL;
        h = (h ^ (unsigned long long)ri_si_gt(v_si[i])) * 1099511628211ULL;
        h = (h ^ (unsigned long long)(long long)(4 * mr_si_gt(&v_si[i], v_si[j]))) * 1099511628211ULL;
        h = (h ^ (unsigned)ir_si_gt(v_si[j])) * 1099511628211ULL;
    }
    printf("si_gt %016llx\n", h); all ^= h;
    for (int i = 0; i < 5; i++) for (int j = 0; j < 5; j++) {
        h = (h ^ (unsigned)rr_si_ge(v_si[i], v_si[j])) * 1099511628211ULL;
        h = (h ^ (unsigned long long)ri_si_ge(v_si[i])) * 1099511628211ULL;
        h = (h ^ (unsigned long long)(long long)(4 * mr_si_ge(&v_si[i], v_si[j]))) * 1099511628211ULL;
        h = (h ^ (unsigned)ir_si_ge(v_si[j])) * 1099511628211ULL;
    }
    printf("si_ge %016llx\n", h); all ^= h;
    for (int i = 0; i < 5; i++) for (int j = 0; j < 5; j++) {
        h = (h ^ (unsigned)rr_ui_eq(v_ui[i], v_ui[j])) * 1099511628211ULL;
        h = (h ^ (unsigned long long)ri_ui_eq(v_ui[i])) * 1099511628211ULL;
        h = (h ^ (unsigned long long)(long long)(4 * mr_ui_eq(&v_ui[i], v_ui[j]))) * 1099511628211ULL;
        h = (h ^ (unsigned)ir_ui_eq(v_ui[j])) * 1099511628211ULL;
    }
    printf("ui_eq %016llx\n", h); all ^= h;
    for (int i = 0; i < 5; i++) for (int j = 0; j < 5; j++) {
        h = (h ^ (unsigned)rr_ui_ne(v_ui[i], v_ui[j])) * 1099511628211ULL;
        h = (h ^ (unsigned long long)ri_ui_ne(v_ui[i])) * 1099511628211ULL;
        h = (h ^ (unsigned long long)(long long)(4 * mr_ui_ne(&v_ui[i], v_ui[j]))) * 1099511628211ULL;
        h = (h ^ (unsigned)ir_ui_ne(v_ui[j])) * 1099511628211ULL;
    }
    printf("ui_ne %016llx\n", h); all ^= h;
    for (int i = 0; i < 5; i++) for (int j = 0; j < 5; j++) {
        h = (h ^ (unsigned)rr_ui_lt(v_ui[i], v_ui[j])) * 1099511628211ULL;
        h = (h ^ (unsigned long long)ri_ui_lt(v_ui[i])) * 1099511628211ULL;
        h = (h ^ (unsigned long long)(long long)(4 * mr_ui_lt(&v_ui[i], v_ui[j]))) * 1099511628211ULL;
        h = (h ^ (unsigned)ir_ui_lt(v_ui[j])) * 1099511628211ULL;
    }
    printf("ui_lt %016llx\n", h); all ^= h;
    for (int i = 0; i < 5; i++) for (int j = 0; j < 5; j++) {
        h = (h ^ (unsigned)rr_ui_le(v_ui[i], v_ui[j])) * 1099511628211ULL;
        h = (h ^ (unsigned long long)ri_ui_le(v_ui[i])) * 1099511628211ULL;
        h = (h ^ (unsigned long long)(long long)(4 * mr_ui_le(&v_ui[i], v_ui[j]))) * 1099511628211ULL;
        h = (h ^ (unsigned)ir_ui_le(v_ui[j])) * 1099511628211ULL;
    }
    printf("ui_le %016llx\n", h); all ^= h;
    for (int i = 0; i < 5; i++) for (int j = 0; j < 5; j++) {
        h = (h ^ (unsigned)rr_ui_gt(v_ui[i], v_ui[j])) * 1099511628211ULL;
        h = (h ^ (unsigned long long)ri_ui_gt(v_ui[i])) * 1099511628211ULL;
        h = (h ^ (unsigned long long)(long long)(4 * mr_ui_gt(&v_ui[i], v_ui[j]))) * 1099511628211ULL;
        h = (h ^ (unsigned)ir_ui_gt(v_ui[j])) * 1099511628211ULL;
    }
    printf("ui_gt %016llx\n", h); all ^= h;
    for (int i = 0; i < 5; i++) for (int j = 0; j < 5; j++) {
        h = (h ^ (unsigned)rr_ui_ge(v_ui[i], v_ui[j])) * 1099511628211ULL;
        h = (h ^ (unsigned long long)ri_ui_ge(v_ui[i])) * 1099511628211ULL;
        h = (h ^ (unsigned long long)(long long)(4 * mr_ui_ge(&v_ui[i], v_ui[j]))) * 1099511628211ULL;
        h = (h ^ (unsigned)ir_ui_ge(v_ui[j])) * 1099511628211ULL;
    }
    printf("ui_ge %016llx\n", h); all ^= h;
    return all == 0;
}
