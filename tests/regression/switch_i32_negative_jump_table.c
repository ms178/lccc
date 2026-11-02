/* A 32-bit switch index must not depend on unspecified upper argument bits. */
__attribute__((noinline)) int negative_dense_switch(int x) {
    switch (x) {
        case -3: return -50;
        case -2: return 10;
        case 0: return 120;
        case 2: return -10;
        case 4: return 200;
        default: return -999;
    }
}

static int expected(int x) {
    if (x == -3) return -50;
    if (x == -2) return 10;
    if (x == 0) return 120;
    if (x == 2) return -10;
    if (x == 4) return 200;
    return -999;
}

int main(void) {
    /* On AArch64 this w-register increment can leave x with a zero upper
       half even while its signed 32-bit value is still negative. */
    for (int x = -8; x <= 8; ++x) {
        if (negative_dense_switch(x) != expected(x)) {
            return 1 + (x + 8);
        }
    }
    return 0;
}
