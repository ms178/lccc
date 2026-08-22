/* C scoping: a local object shadows a file-scope function prototype.
 *
 * glibc nss/nss_module.c declares a function parameter
 * `void (*bind)(nss_module_functions_untyped)` while <sys/socket.h>
 * declares the 3-argument socket bind(). Sema's call-arity check resolved
 * the 1-argument call against the GLOBAL prototype:
 *   "too few arguments to function 'bind' (expected 3, have 1)".
 *
 * The check must resolve through the scoped symbol table first; genuine
 * arity errors against the real prototype must still be caught (verified
 * by tests in the sema unit suite, not here — this file must simply
 * compile and run).
 */
#include <stdio.h>

struct sockaddr;
extern int bind(int, const struct sockaddr *, unsigned); /* sys/socket.h */

typedef void *fnu[32];

static int called_with;

static void my_binder(fnu funcs)
{
    called_with = (int)(((void **)funcs)[0] != 0);
}

/* Parameter named `bind` shadows the socket prototype. */
static void module_load_full(void *module, void (*bind)(fnu))
{
    bind(module); /* 1 argument — the parameter, not socket bind() */
}

/* Local VARIABLE named `bind` shadows it too. */
static int local_var_shadow(void *module)
{
    void (*bind)(fnu) = my_binder;
    bind(module);
    return called_with;
}

int main(void)
{
    void *fake[32] = { (void *)main };
    module_load_full(fake, my_binder);
    int a = called_with;
    int b = local_var_shadow(fake);
    printf("shadow:%s\n", (a == 1 && b == 1) ? "ok" : "MISMATCH");
    return (a == 1 && b == 1) ? 0 : 1;
}
