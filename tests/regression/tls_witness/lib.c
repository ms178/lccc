/* TLS witness library: built -fPIC -shared, so tv is Global-Dynamic and the
   static/hidden ones are Local-Dynamic (both the descriptor/GD sequence). */
__thread int tv = 7;
static __thread int sv = 5;
__attribute__((visibility("hidden"))) __thread int hv = 11;
int get(void) { return tv + sv + hv; }
int *addr_tv(void) { return &tv; }
int bump(void) { sv++; hv += 2; return sv * 100 + hv; }
/* Many values live across a dynamic TLS access: on AArch64/RISC-V the access
   is a call, so none may sit in a caller-saved register across it. */
long press(long a, long b, long c, long d, long e, long f, long g) {
    long x = a * b, y = c * d, z = e * f, w = g * a;
    int t = tv;
    return x + y + z + w + t + a + b + c + d + e + f + g;
}
