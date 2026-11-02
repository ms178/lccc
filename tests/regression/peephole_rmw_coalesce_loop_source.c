/* Regression (torture execute/builtin-bitops-1.c @O2, minimized): the
 * peephole's copy-into-RMW coalescing retargeted `movq %rdi,%r9` (+
 * `andq %r8,%r9`) into `andq %r8,%rdi`, destroying the loop-invariant
 * input register on every iteration past the first.
 *
 * Guard 1 (the consumer's write clobbers the copy source, so the source
 * must be dead) used whole-function textual uniqueness, which is unsound
 * when the copy sits in a loop: the source is mentioned only there yet
 * re-read every iteration. Dataflow (loop-aware) or block-local death only.
 *
 * The helper MUST stay non-static: when it inlines into main the
 * allocator never forms the copy-into-RMW shape and the test passes
 * vacuously (the first reduction of this test made exactly that mistake:
 * it passed with the guard reverted). Proven: aborts with Guard 1
 * reverted (SIGABRT), passes with it.
 */
extern void abort(void);

int my_popcountll(unsigned long long x) {
    int i;
    int count = 0;
    for (i = 0; i < 64; i++)
        if (x & (1ULL << i))
            count++;
    return count;
}

int main(void) {
    if (my_popcountll(0x8000000000000000ULL) != 1)
        abort();
    if (my_popcountll(2ULL) != 1)
        abort();
    if (my_popcountll(0xa5a5a5a5a5a5a5a5ULL) != 32)
        abort();
    if (my_popcountll(0xffffffffffffffffULL) != 64)
        abort();
    return 0;
}
