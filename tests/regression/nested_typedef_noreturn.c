/* A function typedef supplies the type, not the entity. The attribute on
 * the nested prototype must not mark the same-named file-scope function. */
typedef void callback_t(void);
extern void _Exit(int) __attribute__((noreturn));
static volatile int calls;

__attribute__((noinline)) void tick(void)
{
    ++calls;
}

__attribute__((noinline)) int declare_nested(int take_nested)
{
    auto callback_t tick __attribute__((noreturn));
    void tick(void) { _Exit(77); }
    if (take_nested)
        tick();
    return 23;
}

__attribute__((noinline)) int call_file_scope(void)
{
    tick();
    return 42;
}

int main(int argc, char **argv)
{
    (void)argv;
    return declare_nested(argc > 1) != 23 || call_file_scope() != 42 || calls != 1;
}
