void direct(void) { volatile int x = 7; x; x; }
void stores(void) { volatile int x; x = 1; x = 2; }
int parameter(volatile int p) { return p + p; }
void byte_reads(void) { volatile unsigned char c = 3; c; c; }
void wide_reads(void) { volatile long long x = 5; x; x; }
void indirect(volatile int *p) { *p; *p; }
void plain_discard(void) { int x = 7; x; x; }
struct member_fields { volatile int v; int plain; };
void local_field(void) { struct member_fields s = {9, 1}; s.v; s.v; }
void pointer_field(struct member_fields *s) { s->v; s->v; }
int calculation(int p) {
    volatile int x = 9;
    x = x + p;
    return x + x + p;
}
int main(void) {
    struct member_fields s = {9, 1};
    direct(); stores(); byte_reads(); wide_reads(); local_field(); pointer_field(&s);
    return parameter(21) != 42 || calculation(3) != 27;
}
