/* `drm_exec_until_all_locked()` (include/drm/drm_exec.h) restarts its loop
 * through a pointer that can only hold one label:
 *
 *     retry:
 *         for (void *retry_ptr; ({ retry_ptr = &&retry; cleanup(exec); });)
 *             ... if (contended) goto *retry_ptr;
 *
 * GCC 16.2 and Clang 23.1.5 emit `jmp retry`.  LCCC emitted `leaq`/`jmpq *`,
 * an indirect branch in the middle of a function, which objtool reads as a
 * tail call out of a live frame (eighteen functions of amdgpu).  The values
 * are checked here; the direct jump by check_kernel_driver_shapes.py.
 */
#include <stdio.h>

static int cleanups, contentions;

__attribute__((noinline)) static int cleanup(int *locked)
{
    cleanups++;
    if (*locked >= 3)
        return 0;
    return 1;
}
__attribute__((noinline)) static int contended(int attempt)
{
    return attempt == 1 || attempt == 4;
}

__attribute__((noinline)) int lock_all(int n)
{
    int locked = 0, attempt = 0, work = 0;
retry:
    for (void *retry_ptr; ({
             retry_ptr = &&retry;
             (void)retry_ptr;
             cleanup(&locked);
         });) {
        work += n;
        if (__builtin_expect(contended(attempt++), 0)) {
            contentions++;
            locked = 0;
            goto *retry_ptr;
        }
        locked++;
    }
    return work * 100 + locked;
}

/* A dispatch through a table keeps its indirect jump and its semantics. */
__attribute__((noinline)) int interpret(const unsigned char *code, int acc)
{
    static const void *const table[] = { &&op_inc, &&op_double, &&op_neg, &&op_end };
    goto *table[*code++];
op_inc:
    acc += 1;
    goto *table[*code++];
op_double:
    acc *= 2;
    goto *table[*code++];
op_neg:
    acc = -acc;
    goto *table[*code++];
op_end:
    return acc;
}

/* Two labels behind one pointer: still a computed goto. */
__attribute__((noinline)) int either(int which, int x)
{
    void *where = which ? &&high : &&low;
    goto *where;
low:
    return x - 100;
high:
    return x + 100;
}

int main(void)
{
    static const unsigned char program[] = { 0, 1, 0, 2, 1, 3 };
    /* One call per statement: argument evaluation order is not specified. */
    int locked = lock_all(7);
    printf("lock_all %d cleanups %d contentions %d\n", locked, cleanups, contentions);
    printf("interpret %d\n", interpret(program, 5));
    int low = either(0, 1);
    int high = either(1, 1);
    printf("either %d %d\n", low, high);
    return 0;
}
