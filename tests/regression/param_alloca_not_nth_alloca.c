/* gcc.c-torture/execute/20180112-1.c reduced.
 * After mem2reg removes a parameter home, emit_store_params must not identify
 * the first remaining local alloca (here volatile `ss`) as the parameter slot.
 */
extern void abort(void);
typedef __UINT32_TYPE__ u32;
__attribute__((noinline)) u32 bug(u32 *result) {
    volatile u32 ss = 0xffffffffu;
    volatile u32 d = 0xeeeeeeeeu;
    u32 tt = d & 0x00800000u;
    u32 r = tt << 8;
    r = (r >> 31) | (r << 1);
    u32 off = (r ^ ss) >> 1;
    *result = tt;
    return off;
}
int main(void) {
    u32 l = 0;
    if (bug(&l) != 0x7fffffffu || l != 0x00800000u) abort();
    return 0;
}
