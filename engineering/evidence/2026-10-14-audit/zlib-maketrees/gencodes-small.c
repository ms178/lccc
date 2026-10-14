/* Altered standalone zlib-ng 2.3.3 reproducer, extracted 2026-10-14.
 * Original maketrees/trees implementation: Jean-loup Gailly, Mark Adler,
 * and zlib-ng contributors. See LICENSE.txt in this directory.
 * Headers replaced with the exact required constants/layout; tracing and
 * disabled assertions omitted. This is NOT an upstream zlib-ng source file.
 */
#include <stdio.h>
#include <stdint.h>
#define L_CODES 286
#define D_CODES 30
#define MAX_BITS 15
#define LENGTH_CODES 29
#define DIST_CODE_LEN 512
#define STD_MAX_MATCH 258
#define STD_MIN_MATCH 3
#define Assert(a,b) ((void)0)
#define Z_INTERNAL
typedef struct { union { uint16_t freq, code; } fc; union { uint16_t dad, len; } dl; } ct_data;
#define Code fc.code
#define Len dl.len
static const int extra_lbits[29]={0,0,0,0,0,0,0,0,1,1,1,1,2,2,2,2,3,3,3,3,4,4,4,4,5,5,5,5,0};
static const int extra_dbits[30]={0,0,0,0,1,1,2,2,3,3,4,4,5,5,6,6,7,7,8,8,9,9,10,10,11,11,12,12,13,13};
void gen_codes(ct_data *, int, uint16_t *);
static inline uint16_t bi_reverse(unsigned code, int len) {
    /* code: the value to invert */
    /* len: its bit length */
    Assert(len >= 1 && len <= 15, "code length must be 1-15");
#define bitrev8(b) \
    (uint8_t)((((uint8_t)(b) * 0x80200802ULL) & 0x0884422110ULL) * 0x0101010101ULL >> 32)
    return (bitrev8(code >> 8) | (uint16_t)bitrev8(code) << 8) >> (16 - len);
}
Z_INTERNAL void gen_codes(ct_data *tree, int max_code, uint16_t *bl_count) {
    /* tree: the tree to decorate */
    /* max_code: largest code with non zero frequency */
    /* bl_count: number of codes at each bit length */
    uint16_t next_code[MAX_BITS+1];  /* next code value for each bit length */
    unsigned int code = 0;           /* running code value */
    int bits;                        /* bit index */
    int n;                           /* code index */

    /* The distribution counts are first used to generate the code values
     * without bit reversal.
     */
    for (bits = 1; bits <= MAX_BITS; bits++) {
        code = (code + bl_count[bits-1]) << 1;
        next_code[bits] = (uint16_t)code;
    }
    /* Check that the bit counts in bl_count are consistent. The last code
     * must be all ones.
     */
    Assert(code + bl_count[MAX_BITS]-1 == (1 << MAX_BITS)-1, "inconsistent bit counts");


    for (n = 0;  n <= max_code; n++) {
        int len = tree[n].Len;
        if (len == 0)
            continue;
        /* Now reverse the bits */
        tree[n].Code = bi_reverse(next_code[len]++, len);


    }
}
