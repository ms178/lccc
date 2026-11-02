/* The probe for §16 of ../../llvm-missed/SCALAR-PREDICATE-FOLDS.md: every
   comparison whose operand interval decides it, asked one at a time.  A fold
   that needs a sibling compare to fire regresses silently when the sibling
   folds first (that is what CI's range_fold_branch gate caught), so each of
   these stands alone.

   Build for a differential read:
     lccc -O2 -S intervals.c -o intervals-lccc.s
     gcc  -O2 -S intervals.c -o intervals-gcc.s     (GCC 16.2 in CI)
   Every function must be a `mov $0/$1` or an `xor` and nothing else. */
#include <limits.h>

/* A zero-extension cannot leave [0, 2^n - 1]: the <= and < spellings are true
   for every int, the > and == spellings false. */
int uchar_le(int x) { return (unsigned char)x <= 255; }
int uchar_lt(int x) { return (unsigned char)x < 256; }
int uchar_gt(int x) { return (unsigned char)x > 255; }
int uchar_eq(int x) { return (unsigned char)x == 300; }
int ushort_le(int x) { return (unsigned short)x <= 65535; }
int uint_le(long x) { return (unsigned int)x <= 4294967295u; }

/* A sign-extension cannot leave [-2^(n-1), 2^(n-1)-1]. */
int schar_lt(int x) { return (signed char)x < 128; }
int schar_gt(int x) { return (signed char)x > 127; }
int schar_ge(int x) { return (signed char)x >= -128; }
int schar_eq(int x) { return (signed char)x == -200; }
int sshort_le(int x) { return (short)x <= 40000; }

/* Without a cast, the compare width's own domain decides -- in the reading the
   predicate implies.  The unsigned reading of -1 is 0xffffffff. */
int int_gt_max(int x) { return x > INT_MAX; }
int int_lt_min(int x) { return x < INT_MIN; }
int uint_le_max(unsigned x) { return x <= 4294967295u; }
int uint_gt_max(unsigned x) { return x > 4294967295u; }
int int_ule_neg1(int x) { return (unsigned)x <= (unsigned)-1; }

/* Refusals: inside the interval nothing is decided, the unsigned reading of a
   sign-extended value is not an interval, and equality without a cast is not
   decided by the bits. */
int keep_uchar_lt(int x) { return (unsigned char)x < 100; }
int keep_schar_gt(int x) { return (signed char)x > -100; }
int keep_schar_ult(int x) { return (unsigned)(signed char)x < 300; }
int keep_uint_ge_max(unsigned x) { return x >= 4294967295u; }
int keep_int_eq(int x) { return x == 5; }
