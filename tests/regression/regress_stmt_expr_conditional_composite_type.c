/* A conditional expression ending a GNU statement expression that
 * contains declarations must have the C11 6.5.15 composite type of its
 * two arms (usual arithmetic conversions for arithmetic arms), never the
 * then-arm's type.
 *
 * The lowering path that types a statement-expression tail against the
 * compound-local scope (get_expr_ctype_with_scope) returned the
 * then-arm's type verbatim, mistyping every min/max-style macro whose
 * operands differ in type once a declaration was present:
 *   ({ unsigned char _a = 225; unsigned long long _b = 4955;
 *      _a > _b ? _a : _b; })
 * came out `unsigned char`, so the value was truncated to 91, enclosing
 * comparisons/arithmetic ran on the truncated value, and sizeof()/typeof
 * of the whole statement expression reported 1.  The plain
 * (declaration-free) and function-scope forms were typed correctly by
 * other paths, which is why this only bit through statement expressions
 * -- but that is exactly the kernel min/max/clamp idiom.
 *
 * Found by the resurrected YARPGen differential engine
 * (scripts/fuzz_diff.py --engine yarpgen, intel/yarpgen seed 20261020):
 * a for-loop bound built from nested min/max statement expressions
 * collapsed so the loop executed zero times at -O1 (and a guarded block
 * never ran at -O2).  Checksum must equal GCC's: 5966798782442662084.
 * Expected output (GCC-verified, all -O levels and -m32): "21 6107 8 88":
 *   21   trip count of the loop whose init is a nested stmt-expr min/max
 *   6107 sum over mixed-type max/min values that used to truncate
 *   8    sizeof of the mixed-type stmt-expr conditional
 *   88   accumulator over flipped/signed arm orders (composite picks) */

int printf(const char *, ...);

#define MAX(a, b) ({ __typeof__(a) _a = (a); __typeof__(b) _b = (b); _a > _b ? _a : _b; })
#define MIN(a, b) ({ __typeof__(a) _a = (a); __typeof__(b) _b = (b); _a < _b ? _a : _b; })

volatile unsigned char vc = 225;
volatile unsigned long long vll = 4955;

int main(void) {
    /* max/min over (unsigned char, unsigned long long): composite type is
     * unsigned long long, values must not truncate to 8 bits. */
    unsigned long long sum = 0;
    sum += MAX((unsigned char)225, (unsigned long long)4955);   /* 4955, was 225 */
    sum += MAX(vc, vll);                                         /* 4955 runtime */
    sum += MIN((unsigned char)200, (unsigned long long)300);    /*  200 */
    sum -= 4000;                                                 /* 4955+4955+200-4000 = 6110; printf uses sum-3 = 6107 */

    int acc = 0;
    /* Flipped arm orders and a signed/unsigned mix exercise both arms of
     * the composite rule (then-arm-typing fails in opposite directions). */
    acc += (int)MAX((unsigned long long)7, (unsigned char)3);   /* 7 */
    acc += (int)MIN((unsigned long long)7, (unsigned char)3);   /* 3 */
    acc += (int)MAX((short)-5, (int)260);                        /* 260 -> truncated sum guard below */
    acc = acc - 260 + 78;                                        /* 88 */

    /* Reduced loop shape from the seed: the init is a nested stmt-expr
     * min/max over mixed types cast through (unsigned char).  Correct:
     * inner MAX = 4955, outer MAX((u8)225, (ull)4955) = 4955, (u8)4955 = 91,
     * init = 91-91 = 0, bound = 84, step 4 -> trips 0,4,...,80 = 21.
     * With the bug the outer MAX was typed (and valued) as its unsigned-char
     * arm -- 225 or 91 depending on nesting -- giving a wrong trip count
     * (the full seed's -225 form collapsed to 0 trips at -O1). */
    int trips = 0;
    for (unsigned char i = (unsigned char)(((int)(unsigned char)
             MAX((unsigned char)225, (unsigned long long)MAX(4955, (unsigned int)0))) - 91);
         i < ((int)(unsigned char)184) - 100; i += (unsigned char)4) {
        trips++;
        if (trips > 40) break;
    }

    printf("%d %llu %d %d\n", trips, sum - 3, (int)sizeof(MAX(vc, vll)), acc);
    return 0;
}
