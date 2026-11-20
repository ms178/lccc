/* A switch that names every value its selector can have, each case returning.
 *
 * `(attr >> 3) & 7` is one of eight values and the inner switch has eight
 * cases, so its default is dead, and with it the `break` behind the switch
 * and the edge that carries `val`'s initial value to the join.  This is
 * `atom_get_src_int` of Linux's drivers/gpu/drm/amd/amdgpu/atom.c.
 *
 * The x86-64 backend drops the jump table's range check on that bound.  The
 * IR kept the default edge, so the relay that loads 0xCDCDCDCD and jumps to
 * the join was emitted with no way in:
 *
 *     movl $3452816845, %r9d
 *     jmp .LBB20
 *
 * and objtool reported `atom_get_src_int+0x44: unreachable instruction`.
 *
 * The values are what this file checks; that nothing unreachable is emitted
 * is held by tests/regression/check_kernel_driver_shapes.py (at every level,
 * and its self-test turns the fix off to see the relay come back).
 */
#include <stdio.h>
#include <stdint.h>
struct ctx { uint32_t reg[8]; uint32_t ws[8]; uint32_t divmul[2]; const uint8_t *bios; };
__attribute__((noinline)) static uint32_t read_reg(struct ctx *c, uint32_t i) { return c->reg[i & 7] + i; }
__attribute__((noinline))
uint32_t get_src(struct ctx *c, uint8_t attr, int *ptr, uint32_t *saved, int print)
{
    uint32_t idx, val = 0xCDCDCDCD, align, arg;
    arg = attr & 7;
    align = (attr >> 3) & 7;
    switch (arg) {
    case 0:
        idx = c->bios[*ptr] | (c->bios[*ptr + 1] << 8);
        (*ptr) += 2;
        val = read_reg(c, idx);
        break;
    case 1:
        idx = c->bios[*ptr];
        (*ptr)++;
        val = c->ws[idx & 7];
        break;
    case 2:
        idx = c->bios[*ptr];
        (*ptr)++;
        switch (idx) {
        case 0x40: val = c->divmul[0]; break;
        case 0x41: val = c->divmul[1]; break;
        default: val = c->ws[idx & 7];
        }
        break;
    case 5:
        switch (align) {
        case 0:
            val = c->bios[*ptr] | (c->bios[*ptr + 1] << 8) | (c->bios[*ptr + 2] << 16) | ((uint32_t)c->bios[*ptr + 3] << 24);
            (*ptr) += 4;
            return val;
        case 1:
        case 2:
        case 3:
            val = c->bios[*ptr] | (c->bios[*ptr + 1] << 8);
            (*ptr) += 2;
            return val;
        case 4:
        case 5:
        case 6:
        case 7:
            val = c->bios[*ptr];
            (*ptr)++;
            return val;
        }
        break;
    case 6:
        val = c->reg[c->bios[*ptr] & 7] * 3;
        (*ptr)++;
        break;
    }
    if (saved)
        *saved = val;
    val &= 0xffffffffu >> (align * 3);
    if (print)
        val += 1;
    return val;
}
int main(void)
{
    static const uint8_t bios[16] = {1, 2, 3, 4, 0x40, 0x41, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16};
    struct ctx c = {{1, 2, 3, 4, 5, 6, 7, 8}, {11, 12, 13, 14, 15, 16, 17, 18}, {100, 200}, bios};
    uint32_t sum = 0, saved = 0;
    for (int attr = 0; attr < 256; attr++) {
        int ptr = attr & 7;
        uint32_t v = get_src(&c, (uint8_t)attr, &ptr, &saved, attr & 1);
        sum = sum * 31 + v + (uint32_t)ptr + saved;
    }
    printf("%u\n", sum);
    return 0;
}
