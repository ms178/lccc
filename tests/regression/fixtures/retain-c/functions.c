/* The linker, not the frontend, must keep these unreferenced sections. */
__attribute__((retain)) int keep_default(void) { return 11; }
__attribute__((used, retain)) static int keep_static(void) { return 19; }
/* Retain alone does not prevent frontend DCE at optimization levels above O0. */
__attribute__((retain)) static int unused_retain_only(void) { return 23; }
static int unused_retain_data __attribute__((retain)) = 71;
__attribute__((used, section(".discard_fn"))) static int used_not_retain(void) { return 29; }

int retain_prototype(void) __attribute__((retain));
int retain_prototype(void) { return 31; }
int retain_after(void) { return 37; }
int retain_after(void) __attribute__((retain));
static int used_prototype(void) __attribute__((used, retain));
static int used_prototype(void) { return 41; }
static int used_after(void) { return 43; }
static int used_after(void) __attribute__((used, retain));

__attribute__((retain, section(".custom_fn"))) int keep_custom(void) { return 47; }
__attribute__((retain, cold)) int keep_cold(void) { return 67; }

int helper_a(int x);
int helper_b(int x);
__attribute__((used, retain, noinline)) int jump_fn(int x) {
    switch (x) {
        case 0: return helper_b(1);
        case 1: return helper_a(4);
        case 2: return helper_b(7);
        case 3: return helper_a(10);
        case 4: return helper_b(13);
        case 5: return helper_a(16);
        case 6: return helper_b(19);
        case 7: return helper_a(22);
        case 8: return helper_b(25);
        case 9: return helper_a(28);
        case 10: return helper_b(31);
        case 11: return helper_a(34);
        default: return -1;
    }
}
__attribute__((noinline)) int helper_a(int x) { return x + 1; }
__attribute__((noinline)) int helper_b(int x) { return x - 1; }
int main(void) { return 0; }
