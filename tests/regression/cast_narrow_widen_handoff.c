/* A narrowing cast followed immediately by a widening cast must materialize
 * the truncation. The x86-64 no-home policy once forwarded the original U32
 * register through U32->U8->U64. zlib-ng's bi_reverse then generated corrupt
 * Huffman codes (bi_reverse(8188,13): 575 instead of 2047). */
typedef unsigned char u8;
typedef unsigned short u16;
typedef unsigned long long u64;

typedef struct code_data {
    union { u16 frequency, code; } fc;
    union { u16 parent, length; } dl;
} code_data;

static u16 reverse_code(unsigned code, int len) {
#define REVERSE_BYTE(b) \
    (u8)((((u8)(b) * 0x80200802ULL) & 0x0884422110ULL) \
         * 0x0101010101ULL >> 32)
    return (REVERSE_BYTE(code >> 8) | (u16)REVERSE_BYTE(code) << 8)
           >> (16 - len);
}

static void generate(code_data *tree, int max_code, u16 *next_code) {
    for (int n = 0; n <= max_code; ++n) {
        int len = tree[n].dl.length;
        if (len != 0)
            tree[n].fc.code = reverse_code(next_code[len]++, len);
    }
}

int main(void) {
    code_data tree[262] = {{0}};
    u16 next_code[16] = {0};
    tree[261].dl.length = 13;
    next_code[13] = 8188;
    generate(tree, 261, next_code);
    if (tree[261].fc.code != 2047 || next_code[13] != 8189)
        return 1;

    /* Exercise the same narrowing handoff with non-constant runtime values. */
    volatile unsigned inputs[] = {0, 255, 256, 8188, 0x1234abcdU};
    const u8 expected[] = {0, 255, 0, 252, 205};
    for (unsigned i = 0; i < sizeof(inputs) / sizeof(inputs[0]); ++i) {
        u64 widened = (u8)inputs[i];
        if (widened != expected[i])
            return 2;
    }
    return 0;
}
