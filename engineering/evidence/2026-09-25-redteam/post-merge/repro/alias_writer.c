/* Compiled as its own translation unit: no local code reads hidden. */
static int hidden;
extern int published __attribute__((alias("hidden")));
__attribute__((noinline)) void update(int v) { hidden = v; }
