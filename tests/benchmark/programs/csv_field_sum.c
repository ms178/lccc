/* CSV integer-field summer with checksum main. */
#include <stdio.h>

static long sum_fields(const char *p, int field) {
    long total = 0;
    int col = 0;
    long cur = 0;
    int neg = 0, in = 0;
    for (;; p++) {
        char c = *p;
        if (c >= '0' && c <= '9') {
            cur = cur * 10 + (c - '0');
            in = 1;
        } else {
            if (in && col == field) total += neg ? -cur : cur;
            cur = 0;
            neg = 0;
            in = 0;
            if (c == ',') {
                col++;
            } else if (c == '\n' || c == '\0') {
                col = 0;
                if (c == '\0') break;
            } else if (c == '-') {
                neg = 1;
            }
        }
    }
    return total;
}

int main(void) {
    static char buf[4096];
    int o = 0;
    for (int r = 0; r < 64; r++) {
        for (int c = 0; c < 6; c++) {
            if (c) buf[o++] = ',';
            int v = (r * 131 + c * 17) % 1000 - 500;
            if (v < 0) {
                buf[o++] = '-';
                v = -v;
            }
            char tmp[8];
            int t = 0;
            if (v == 0) tmp[t++] = '0';
            while (v > 0) {
                tmp[t++] = (char)('0' + v % 10);
                v /= 10;
            }
            while (t > 0) buf[o++] = tmp[--t];
        }
        buf[o++] = '\n';
    }
    buf[o] = '\0';
    printf("%ld %ld %ld\n", sum_fields(buf, 0), sum_fields(buf, 3),
           sum_fields(buf, 5));
    return 0;
}
