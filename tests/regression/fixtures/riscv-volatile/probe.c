/* Assembly and freestanding runtime fixture: each volatile access is visible. */
void local_reads(void) { volatile int x = 7; x; x; }
void local_stores(int a, int b) { volatile int x; x = a; x = b; }
int parameter_reads(volatile int p) { return p + p; }
void byte_reads(void) { volatile unsigned char x = 3; x; x; }
void wide_reads(void) { volatile long long x = 5; x; x; }
void pointer_reads(volatile int *p) { *p; *p; }
void pointer_stores(volatile int *p, int a, int b) { *p = a; *p = b; }
volatile int global;
void global_reads(void) { global; global; }
struct fields { volatile int v; int plain; };
void local_field_reads(void) { struct fields f = {9, 1}; f.v; f.v; }
void pointer_field_reads(struct fields *p) { p->v; p->v; }
void plain_discard(void) { int x = 7; x; x; }
int ordinary(int n) { int x = n + 1; return x * 3; }
int main(void) {
    struct fields f = {9, 1};
    volatile int n = 4;
    local_reads(); local_stores(1, 2); byte_reads(); wide_reads();
    pointer_reads(&n); pointer_stores(&n, 5, 8); global_reads();
    local_field_reads(); pointer_field_reads(&f); plain_discard();
    return n != 8 || f.v != 9 || parameter_reads(21) != 42 || ordinary(4) != 15;
}
