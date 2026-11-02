/* Reduced reproducer: LCCC -O2 miscompile in peephole memory folding.
 * setparams() computes &params[i] via a loop; the address flows through a
 * phi, is copied into new_level, and is dereferenced later. The x86 peephole
 * folds the `addq + movq %rax,%r10 + movq 16(%r10)` chain into an indexed
 * load but drops the definition of %r10, which a later phi copy-back
 * (`movq %r10, ...`) still reads -> undefined register -> garbage result.
 *
 * Correct output: 16  (gcc -O2 prints 16; lccc -O2 printed 2122188180)
 */
#include <stdio.h>
typedef struct { int param; void *buf; unsigned long size; int status; } P;
static int prep(P **out, P *p) { if (p->size < sizeof(int)) return -5; *out = p; return 0; }
int setparams(void *strm, P *params, unsigned long count) {
    P *new_level = 0, *new_strategy = 0;
    for (unsigned long i = 0; i < count; i++) {
        switch (params[i].param) {
        case 0: prep(&new_level, &params[i]); break;
        case 1: prep(&new_strategy, &params[i]); break;
        default: break;
        }
    }
    return (new_level ? *(int *)new_level->buf : 0)
         + (new_strategy ? *(int *)new_strategy->buf : 0);
}
int main(void) {
    int lv = 7, st = 9;
    P params[2];
    params[0].param = 0; params[0].buf = &lv; params[0].size = sizeof(int); params[0].status = 0;
    params[1].param = 1; params[1].buf = &st; params[1].size = sizeof(int); params[1].status = 0;
    int r = setparams(0, params, 2);
    printf("%d\n", r);
    return r == 16 ? 0 : 1;
}
