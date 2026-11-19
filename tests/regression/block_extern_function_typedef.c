/* A block-scope declaration names a FUNCTION whenever its type is one, however
 * the type is spelled: a function declarator, a bare function typedef, or
 * `typeof` of a function (the shape of the kernel's symbol_get()).
 *
 * The two `extern` spellings without a declarator were registered as objects.
 * A call through one then loaded the first eight bytes of the callee's machine
 * code and jumped to them: SIGSEGV at every optimization level, on every
 * target, where GCC calls the function.  The plain `typeof(f) f;` spelling
 * became a local variable of function type.
 *
 * The controls pin the neighbours that must NOT change: a typedef with a
 * pointer declarator is an object, and `typeof` takes its operand's type even
 * when a function of the declared name exists at file scope.
 */
typedef int step_type(int);

__attribute__((noinline)) int step(int x) { return x + 1; }
__attribute__((noinline)) int twice(int x) { return 2 * x; }
int later(int);
extern step_type *hook;

__attribute__((noinline)) int by_typedef(int n)
{
    extern step_type step;
    return step(n);
}

__attribute__((noinline)) int by_typeof(int n)
{
    extern __typeof__(twice) twice;
    extern __typeof__(later) later;
    return later(twice(n));
}

/* Without `extern`: still declarations of the file-scope functions. */
__attribute__((noinline)) int plain(int n)
{
    step_type step;
    __typeof__(twice) twice;
    return twice(step(n));
}

/* A local of the same name hides the function; leaving the block restores it. */
__attribute__((noinline)) int hidden_then_restored(int n)
{
    int step = n + 100;
    {
        extern step_type step;
        n = step(n);
    }
    return n + step;
}

/* The pointer declarator makes this an object that holds an address. */
__attribute__((noinline)) int pointer_object(int n)
{
    extern step_type *hook;
    return hook(n);
}

/* `twice` is an int here, so the inner declaration is another int. */
__attribute__((noinline)) int operand_type_decides(int n)
{
    int twice = n + 5;
    {
        __typeof__(twice) twice;
        twice = 3;
        n += twice;
    }
    return n + twice;
}

int later(int x) { return x + 10; }
step_type *hook = step;

int main(void)
{
    if (by_typedef(41) != 42)
        return 1;
    if (by_typeof(16) != 42)
        return 2;
    if (plain(20) != 42)
        return 3;
    if (hidden_then_restored(1) != 2 + 101)
        return 4;
    if (pointer_object(41) != 42)
        return 5;
    if (operand_type_decides(1) != 4 + 6)
        return 6;
    return 0;
}
