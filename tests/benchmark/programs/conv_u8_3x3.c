/* 3x3 u8 convolution with saturating pack, checksum main. */
#include <stdio.h>

#define W 64
#define H 64
static unsigned char img[H][W], out[H][W];
static const int k[3][3] = {{-1, -2, -1}, {0, 0, 0}, {1, 2, 1}};

static void conv(void) {
    for (int y = 1; y < H - 1; y++) {
        for (int x = 1; x < W - 1; x++) {
            int s = 0;
            for (int ky = 0; ky < 3; ky++)
                for (int kx = 0; kx < 3; kx++)
                    s += (int)img[y + ky - 1][x + kx - 1] * k[ky][kx];
            if (s < 0) s = -s;
            if (s > 255) s = 255;
            out[y][x] = (unsigned char)s;
        }
    }
}

int main(void) {
    for (int y = 0; y < H; y++)
        for (int x = 0; x < W; x++)
            img[y][x] = (unsigned char)((x * 3 + y * 5 + x * y) & 255);
    conv();
    unsigned long long h = 0;
    for (int y = 0; y < H; y++)
        for (int x = 0; x < W; x++) h = h * 31 + out[y][x];
    printf("%llx\n", h);
    return 0;
}
