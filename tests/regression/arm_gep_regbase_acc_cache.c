// Regression: ARM emit_reg_to_acc must invalidate the accumulator cache.
// At -O2, LICM hoists GlobalAddrs into the entry block where they get
// register homes; the default emit_gep general path then does
// emit_reg_to_acc(base); emit_acc_to_secondary(); emit_load_operand(offset).
// Without invalidate_acc(), the stale acc cache claimed x0 already held the
// offset and skipped the load, computing address = base+base -> SIGSEGV.
// Expected output: 1998000
int printf(const char*,...);
void *malloc(unsigned long);
#define TABLE_SIZE 64
#define NUM_OPS 2000
typedef struct Entry { unsigned int key; int value; struct Entry *next; } Entry;
static Entry *table[TABLE_SIZE];
static unsigned int hash(unsigned int k) { return k & (TABLE_SIZE - 1); }
static void insert(unsigned int key, int value) {
    unsigned int h = hash(key);
    Entry *e = table[h];
    while (e) { if (e->key == key) { e->value = value; return; } e = e->next; }
    e = (Entry *)malloc(sizeof(Entry));
    e->key = key; e->value = value; e->next = table[h]; table[h] = e;
}
static int lookup(unsigned int key) {
    unsigned int h = hash(key);
    for (Entry *e = table[h]; e; e = e->next) if (e->key == key) return e->value;
    return -1;
}
int main(void) {
    unsigned int seed = 12345; long sum = 0;
    for (int i = 0; i < NUM_OPS; i++) { seed = seed*1664525u + 1013904223u; insert(seed, i); }
    seed = 12345;
    for (int i = 0; i < NUM_OPS; i++) { seed = seed*1664525u + 1013904223u; sum += lookup(seed); }
    for (int i = 0; i < NUM_OPS; i++) {
        seed = seed*1664525u + 1013904223u;
        if (i & 1) insert(seed, i); else sum += lookup(seed);
    }
    printf("%ld\n", sum);
    return 0;
}
