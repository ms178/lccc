/* Under -fprofile-generate the unit also holds the profile writer: counters,
 * value-profile sites, and the writer's paths and format strings, which
 * are string literals under symbol names. check_symbol_annotations.py
 * reads their symbols. */
int printf(const char *, ...);
static int add1(int x) { return x + 1; }
static int add2(int x) { return x + 2; }
static int (*const ops[2])(int) = {add1, add2};
int pick(int k, int v) {
    switch (k & 3) { case 0: return v; case 1: return v * 3; case 2: return v - 7; default: return ops[v & 1](v); }
}
int main(void) { int s = 0; for (int i = 0; i < 1000; i++) s += pick(i, i) + ops[i & 1](i); printf("%d\n", s); return 0; }
