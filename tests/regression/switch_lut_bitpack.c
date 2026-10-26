#include <stdio.h>

__attribute__((noinline)) int is_vowel(char c) {
    switch (c) {
        case 'a': case 'e': case 'i': case 'o': case 'u':
        case 'A': case 'E': case 'I': case 'O': case 'U':
            return 1;
        default:
            return 0;
    }
}

__attribute__((noinline)) int two_val_custom(int x) {
    switch (x) {
        case 10: case 12: case 15: case 20: return 42;
        case 11: case 14: return 7;
        default: return -1;
    }
}

__attribute__((noinline)) int nibble_lut(int x) {
    switch (x) {
        case 0: return 100;
        case 1: return 103;
        case 2: return 107;
        case 3: return 115;
        case 5: return 109;
        case 8: return 112;
        default: return 999;
    }
}

__attribute__((noinline)) int byte_neg_lut(int x) {
    switch (x) {
        case -3: return -50;
        case -2: return 10;
        case  0: return 120;
        case  2: return -10;
        case  4: return 200;
        default: return -999;
    }
}

__attribute__((noinline)) int join_phi_with_ext_pred(int flag, int x) {
    int r;
    if (flag) {
        r = 777;
    } else {
        switch (x) {
            case 1: r = 10; break;
            case 2: r = 11; break;
            case 4: r = 13; break;
            case 5: r = 12; break;
            default: r = 10; break;
        }
    }
    return r * 3 + 1;
}

int main(void) {
    unsigned long long h = 1469598103934665603ULL;
    for (int x = -128; x <= 260; x++) {
        h = (h ^ (unsigned long long)(unsigned int)is_vowel((char)x)) * 1099511628211ULL;
        h = (h ^ (unsigned long long)(unsigned int)two_val_custom(x)) * 1099511628211ULL;
        h = (h ^ (unsigned long long)(unsigned int)nibble_lut(x)) * 1099511628211ULL;
        h = (h ^ (unsigned long long)(unsigned int)byte_neg_lut(x)) * 1099511628211ULL;
        h = (h ^ (unsigned long long)(unsigned int)join_phi_with_ext_pred(x & 1, x >> 1)) * 1099511628211ULL;
    }
    printf("%016llx\n", h);
    return 0;
}
