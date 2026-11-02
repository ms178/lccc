/* The stack-clash probe has TWO shapes, split at 64 KiB.
 *
 * Below the boundary the prologue unrolls one `subq $4096` + `orl $0, (%rsp)`
 * pair per page: no scratch register, no branch, no loop bookkeeping, and the
 * whole sequence stays in one instruction-cache line per eight pages. Above it
 * the unrolled form is a code-size liability (a 3 GiB frame would emit 786 432
 * pairs), so the prologue walks the pages with a loop instead and closes it
 * with the register-form `subq %r11, %rsp`. That tail is not an optimization:
 * `subq r64, imm32` cannot encode a frame above 2 GiB at all, so the register
 * form is the only encoding that reaches the bottom of a giant frame.
 *
 * Nothing here is called. This is an assembly-production fixture: it exists so
 * `check_narrow_int_codegen.py` can assert both shapes in the emitted text,
 * including that the loop keeps its CFA in %rbp (the reason the loop is legal
 * at all -- with %rbp established the unwinder never needs a static %rsp).
 *
 * The pages are written through `volatile` so the frame cannot be shrunk away,
 * and every page is touched so the probe's own stores are not the only ones the
 * optimizer might reorder around.
 */

#define PAGE 4096

/* 3 pages: below the unroll boundary -> three probe pairs, no loop. */
__attribute__((noinline)) int probe_unrolled_three_pages(int seed)
{
    volatile unsigned char frame[3 * PAGE];
    frame[0] = (unsigned char)seed;
    frame[PAGE] = (unsigned char)(seed + 1);
    frame[2 * PAGE] = (unsigned char)(seed + 2);
    return frame[0] + frame[PAGE] + frame[2 * PAGE];
}

/* 64 pages: above the boundary -> loop + register-form tail. */
__attribute__((noinline)) int probe_loop_sixty_four_pages(int seed)
{
    volatile unsigned char frame[64 * PAGE];
    for (int page = 0; page < 64; page++) {
        frame[page * PAGE] = (unsigned char)(seed + page);
    }
    return frame[0] + frame[63 * PAGE];
}

/* Every `tests/regression/*.c` is compiled AND linked as a standalone program by
 * run_regression.py, so this file needs an entry point even though nothing here
 * calls the two fixtures: their external linkage is what keeps their prologues
 * in the object, and `main` is what keeps the corpus link from failing on a
 * fixture that has no `main` by design. */
int main(void)
{
    return 0;
}
