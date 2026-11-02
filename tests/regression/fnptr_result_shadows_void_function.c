/* C11 6.2.1p4: a visible object binding shadows a file-scope function for the
 * call RESULT type as well as the call target. Linux timer_migration.c names
 * its bool-returning callback `up`, shadowing the semaphore's void up().
 * The call target and argument checker already chose the callback; typing the
 * result from the hidden void function rejected `if (up(...))` or silently
 * converted an FP/integer return through the wrong ABI.
 *
 * Also exercises nested-block shadowing, pointer-to-function-pointer calls,
 * explicit dereference calls, and restoration of the file-scope entity after
 * the shadowing scope ends.
 */
struct lock;
extern void up(struct lock *semaphore);

typedef _Bool (*up_f)(int *, int *, int *);

static int calls;

static _Bool less(int *a, int *b, int *data)
{
    ++calls;
    *data += *a < *b;
    return *a < *b;
}

static int visit(up_f up, int *a, int *b, int *data)
{
    int result = 0;

    if (up(a, b, data))
        result |= 1;
    {
        up_f down = less;
        if (down(b, a, data))
            result |= 2;
    }
    if (up(b, a, data))
        result |= 4;
    return result;
}

static int via_pointer_pointer(up_f *slot, int *a, int *b, int *data)
{
    /* `slot` is an object, not the hidden void up(); the call result is bool. */
    up_f up = *slot;
    if ((*up)(a, b, data))
        return 1;
    return 0;
}

static int result;

void up(struct lock *semaphore)
{
    (void)semaphore;
    result = 42;
}

int main(void)
{
    int a = 1, b = 2, data = 0;
    up_f slot = less;

    if (visit(less, &a, &b, &data) != 1 || data != 1 || calls != 3)
        return 1;
    if (via_pointer_pointer(&slot, &a, &b, &data) != 1 || data != 2 || calls != 4)
        return 2;
    up((struct lock *)0);
    return result != 42;
}
