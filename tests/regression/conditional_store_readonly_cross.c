/* No libc: false guarded stores must not write read-only objects. */
static const int read_only_words[257] = {1};
static const unsigned char read_only_bytes[257] = {1};
static volatile unsigned count = 257;

__attribute__((noinline))
static void clamp_negative(int *p, unsigned n) {
    for (unsigned i = 0; i < n; ++i) {
        int v = p[i];
        if (v < 0) p[i] = 0;
    }
}

__attribute__((noinline))
static void fold_upper(unsigned char *p, unsigned n) {
    for (unsigned i = 0; i < n; ++i) {
        unsigned char c = p[i];
        if (c >= 'A' && c <= 'Z') p[i] = (unsigned char)(c + 32);
    }
}

__attribute__((noinline))
static void replace_high(unsigned char *p, unsigned n) {
    for (unsigned i = 0; i < n; ++i) {
        unsigned char c = p[i];
        if (c == 255 || c > 250) p[i] = 0;
    }
}

int main(void) {
    unsigned n = count;
    /* The casts are valid: the guarded writes are never executed. */
    clamp_negative((int *)read_only_words, n);
    fold_upper((unsigned char *)read_only_bytes, n);
    replace_high((unsigned char *)read_only_bytes, n);
    int words[37];
    unsigned char bytes[37];
    for (unsigned i = 0; i < 37; ++i) {
        words[i] = (int)i - 18;
        bytes[i] = (unsigned char)(i + 50);
    }
    clamp_negative(words, 35);
    fold_upper(bytes, 35);
    for (unsigned i = 0; i < 37; ++i) {
        int w = (int)i - 18;
        unsigned char b = (unsigned char)(i + 50);
        if (i < 35 && w < 0) w = 0;
        if (i < 35 && b >= 'A' && b <= 'Z') b = (unsigned char)(b + 32);
        if (words[i] != w || bytes[i] != b) return 1;
    }
    return 0;
}
