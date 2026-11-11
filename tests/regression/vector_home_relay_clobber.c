typedef float vf8 __attribute__((vector_size(32)));
static __attribute__((noinline)) void clobber(vf8 *p) {
    *p = (vf8){7,7,7,7,7,7,7,7};
}
static __attribute__((noinline)) void relay(vf8 *out, const vf8 *in, vf8 hi) {
    vf8 t = __builtin_ia32_minps256(*in, hi);
    clobber(out);
    *out = t;
}
int main(void) {
    vf8 in = {5,5,5,5,5,5,5,5};
    vf8 hi = {3,3,3,3,3,3,3,3};
    vf8 out = {0,0,0,0,0,0,0,0};
    relay(&out, &in, hi);
    for (int i = 0; i < 8; ++i) if (out[i] != 3.0f) return 1;
    return 0;
}
