/* Soundness of the aggregate dead-store elimination and the pointer-alias
 * analysis it depends on.
 *
 * `passes/aggregate_copy_forward.rs` produced FIVE independent miscompiles.
 * Every one of them deleted stores that were genuinely read, and every one of
 * them was silent -- the program simply computed the wrong answer:
 *
 *  1. The read-set was built from `Instruction::Load` only, so a vector
 *     intrinsic reading an alloca through `dest_ptr`/`args` did not count as a
 *     read and the initializing stores were deleted.
 *  2. Variable GEP offsets were collapsed to the constant 0, so `a[i]`
 *     registered as a read of `a[0]` alone and every other initializer looked
 *     dead. `int a[4]={10,20,30,40}; for(i) s+=a[i];` printed garbage.
 *  3. Pointer PHIs were not tracked, so after IVSR rewrote `a[i]` into a
 *     pointer recurrence the loop's reads were attributed to nothing.
 *  4. A loop-carried pointer phi is defined in terms of itself; a strict
 *     "unknown input => reject" rule dropped the root entirely.
 *  5. The dominance walk indexed `idom` with the usize::MAX sentinel for
 *     unreachable blocks and panicked.
 *
 * Each function below is a distinct shape that used to break. They are
 * `noinline` so the optimizer must handle the real control flow rather than
 * constant-folding everything away, and the expected values are computed by
 * hand.
 */
#include <stdio.h>

/* 1. Plain array read by VARIABLE index -- the original scalar reproducer. */
__attribute__((noinline)) static int var_index_sum(int n)
{
       int a[4] = { 10, 20, 30, 40 };
       int s = 0;
       for (int i = 0; i < n; i++)
               s += a[i];
       return s;                      /* n=4 -> 100 */
}

/* 2. Array of structs, field read by variable index (IVSR turns this into a
 *    pointer recurrence, which is what exposed the PHI blindness). */
struct S { int i, j, k; };
__attribute__((noinline)) static int struct_array_sum(int n)
{
       struct S arr[3] = { { 1, 2, 3 }, { 4, 5, 6 }, { 7, 8, 9 } };
       int tot = 0;
       for (int i = 0; i < n; i++)
               tot += arr[i].i;
       return tot;                    /* n=3 -> 12 */
}

/* 3. Two loops, each with a block-local array. Loop 1's array must survive
 *    into loop 2 (the block-local slot region reuses offsets across blocks). */
__attribute__((noinline)) static int two_loops(int n)
{
       int acc = 0;
       for (int i = 0; i < n; i++) {
               int a[2] = { i, i + 1 };
               acc += a[0] + a[1];
       }
       for (int i = 0; i < n; i++) {
               int b[2] = { 2 * i, 2 * i + 1 };
               acc += b[0] + b[1];
       }
       return acc;                    /* n=10 -> 100 + 190 = 290 */
}

/* 4. Store-before-load in the same iteration. A "loop-carried recurrence"
 *    promotion that ignores program order shifts every read one iteration
 *    late and makes the first read see uninitialized stack. */
__attribute__((noinline)) static int store_then_load(int n)
{
       int acc = 0;
       for (int i = 0; i < n; i++) {
               int a[2];
               a[0] = i;                  /* store ... */
               a[1] = i + 1;
               acc += a[0] + a[1];        /* ... then load, SAME iteration */
       }
       return acc;                    /* n=10 -> 100 */
}

/* 5. A genuine loop-carried recurrence through memory: the load DOES observe
 *    the previous iteration. Promotion here is legal and must stay correct. */
__attribute__((noinline)) static long carried(int n)
{
       long slot = 1;
       for (int i = 0; i < n; i++)
               slot = slot * 2 + 1;       /* 1,3,7,15,... = 2^(n+1)-1 */
       return slot;
}

/* 6. Unreachable block: the dominance walk must not index the idom sentinel. */
__attribute__((noinline)) static int with_unreachable(int n)
{
       int a[3] = { 5, 6, 7 };
       int s = 0;
       if (n > 0) {
               for (int i = 0; i < 3; i++)
                       s += a[i];
       } else {
               __builtin_unreachable();
       }
       return s;                      /* 18 */
}

/* 7. Nested loops writing a 2-D array by variable index, then summing it.
 *    A collapsed offset makes every row alias row 0. */
__attribute__((noinline)) static int matrix_sum(int n)
{
       int m[3][3];
       for (int i = 0; i < n; i++)
               for (int j = 0; j < n; j++)
                       m[i][j] = i * 10 + j;
       int s = 0;
       for (int i = 0; i < n; i++)
               for (int j = 0; j < n; j++)
                       s += m[i][j];
       return s;                      /* n=3 -> 90 + 9 = 99 */
}

/* 8. Aliasing through a pointer the compiler cannot see past. */
__attribute__((noinline)) static int via_pointer(int *p, int n)
{
       int a[4] = { 1, 2, 3, 4 };
       if (p)
               a[1] = *p;                 /* may alias nothing, must not delete a[0] */
       int s = 0;
       for (int i = 0; i < n; i++)
               s += a[i];
       return s;
}

struct check { const char *name; int got; int want; };

int main(void)
{
       int ext = 20;
       struct check c[] = {
               { "var_index_sum",   var_index_sum(4),      100 },
               { "struct_array_sum", struct_array_sum(3),   12 },
               { "two_loops",       two_loops(10),         290 },
               { "store_then_load", store_then_load(10),   100 },
               { "carried",         (int)carried(4),        31 },
               { "with_unreachable", with_unreachable(1),   18 },
               { "matrix_sum",      matrix_sum(3),          99 },
               { "via_pointer",     via_pointer(&ext, 4),   28 },
               { "via_pointer_null", via_pointer((int *)0, 4), 10 },
       };

       int fail = 0;
       for (unsigned i = 0; i < sizeof(c) / sizeof(c[0]); i++) {
               if (c[i].got != c[i].want) {
                       printf("FAIL %s got=%d want=%d\n",
                              c[i].name, c[i].got, c[i].want);
                       fail = 1;
               }
       }
       if (fail)
               return 1;
       printf("PASS aggregate_dse_soundness\n");
       return 0;
}
