/* Godbolt/oracle kernel for loop rotation (PF-17, opt-in CCC_LOOP_ROTATE=1).
 * Two sequential counted loops so pred-label (`(pre_op, header_label)`)
 * is exercised on the second loop. noinline so CE keeps a named function.
 *
 * Local:
 *   CCC_LOOP_ROTATE=1 CCC_DISABLE_PASSES=vectorize scripts/codegen_oracle.py \
 *     engineering/kernels/loop_rotate_sum.c --function sum_arr \
 *     --local target/fastbuild/lccc --local-flags '-O2' --flags '-O2' \
 *     --oracles gcc16.2,clang23.1,icc,icx
 */
int sum_arr(const int *a, int n) __attribute__((noinline));
int fill_arr(int *a, int n) __attribute__((noinline));

int fill_arr(int *a, int n) {
    int i, s = 0;
    for (i = 0; i < n; i++)
        a[i] = i + 1;
    return s;
}

int sum_arr(const int *a, int n) {
    int i, s = 0;
    for (i = 0; i < n; i++)
        s += a[i];
    return s;
}
