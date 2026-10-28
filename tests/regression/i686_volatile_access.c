void direct(void) { volatile int x = 7; x; x; }
void stores(void) { volatile int x; x = 1; x = 2; }
int parameter(volatile int p) { return p + p; }
void byte_reads(void) { volatile unsigned char c = 3; c; c; }
void wide_reads(void) { volatile long long x = 5; x; x; }
void indirect(volatile int *p) { *p; *p; }
void plain_discard(void) { int x = 7; x; x; }
int calculation(int p) {
    volatile int x = 9;
    x = x + p;
    return x + x + p;
}
int main(void) {
    direct(); stores(); byte_reads(); wide_reads();
    return parameter(21) != 42 || calculation(3) != 27;
}
